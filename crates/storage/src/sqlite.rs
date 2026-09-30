//! SQLite storage backend.
//!
//! One file, one process, no server. That is the right shape for a single-node
//! deployment: the engine is one binary on one VM, and a database that needs its
//! own daemon would be more moving parts than the problem has.
//!
//! Three things this backend exists to guarantee:
//!
//! * **A restart is not a cold start.** Observations, events, signals, source
//!   health and baselines are all on disk, so stopping and starting the engine
//!   does not throw away the history the detector learned from.
//! * **Raw bytes do not live in RAM.** Payloads are content-addressed files
//!   under `raw/<prefix>/<hash>`; the database keeps only the metadata. An
//!   engine that runs for months must not hold every payload it ever fetched.
//! * **Retention is possible.** `observations` and `raw_payloads` carry
//!   timestamps and sizes, so old rows and files can be pruned deliberately
//!   instead of the process growing until the VM dies.
//!
//! ## Layout
//!
//! ```text
//! data/
//!   world-signal-engine.db
//!   raw/ab/abcdef...
//! ```
//!
//! ## Schema versioning
//!
//! Migrations are an ordered list of SQL batches, applied inside a transaction
//! and tracked with SQLite's own `user_version`. Each one runs exactly once, so
//! startup is idempotent. See `docs/decisions/0008-sqlite-persistence.md`.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};

use wse_model::{
    BaselineSnapshot, Event, EventId, Observation, ObservationId, Signal, SignalId, Source,
    SourceHealth, SourceId,
};

use crate::query::{ObservationQuery, Page, SignalQuery, TimeRange};
use crate::raw::{RawStore, StoredPayload};
use crate::store::{
    BaselineStore, DiskUsage, EventStore, MaintenanceStore, ObservationStore, SignalStore,
    SourceStore, StorageError,
};

/// Where the database and the raw payloads live.
#[derive(Debug, Clone)]
pub struct SqliteConfig {
    /// Path to the database file. Parent directories are created.
    pub db_path: PathBuf,
    /// Directory the content-addressed raw payloads are written under.
    pub raw_dir: PathBuf,
}

impl SqliteConfig {
    pub fn new(db_path: impl Into<PathBuf>, raw_dir: impl Into<PathBuf>) -> Self {
        Self {
            db_path: db_path.into(),
            raw_dir: raw_dir.into(),
        }
    }
}

/// The ordered schema migrations.
///
/// Append-only: an existing database records how many have run in
/// `PRAGMA user_version`, and only the missing ones are applied. Never edit a
/// released entry — a deployed database has already run it.
const MIGRATIONS: &[&str] = &[
    // v1 — the initial schema.
    r#"
    CREATE TABLE sources (
        id       TEXT PRIMARY KEY,
        priority INTEGER NOT NULL,
        category TEXT NOT NULL,
        json     TEXT NOT NULL
    );

    CREATE TABLE source_health (
        source_id TEXT PRIMARY KEY,
        status    TEXT NOT NULL,
        json      TEXT NOT NULL
    );

    CREATE TABLE observations (
        id                TEXT PRIMARY KEY,
        source_id         TEXT NOT NULL,
        entity_id         TEXT,
        metric            TEXT NOT NULL,
        series_key        TEXT NOT NULL,
        observed_at_nanos INTEGER NOT NULL,
        received_at_nanos INTEGER NOT NULL,
        value             REAL NOT NULL,
        json              TEXT NOT NULL
    );
    CREATE INDEX idx_observations_series
        ON observations(series_key, observed_at_nanos);
    CREATE INDEX idx_observations_source
        ON observations(source_id);
    CREATE INDEX idx_observations_entity
        ON observations(entity_id);
    CREATE INDEX idx_observations_observed_at
        ON observations(observed_at_nanos);

    CREATE TABLE events (
        id              TEXT PRIMARY KEY,
        group_key       TEXT NOT NULL,
        first_seen_nanos INTEGER NOT NULL,
        last_seen_nanos  INTEGER NOT NULL,
        state           TEXT NOT NULL,
        json            TEXT NOT NULL
    );
    CREATE INDEX idx_events_first_seen ON events(first_seen_nanos);
    CREATE INDEX idx_events_last_seen  ON events(last_seen_nanos);

    CREATE TABLE signals (
        id                TEXT PRIMARY KEY,
        event_id          TEXT NOT NULL,
        first_seen_nanos  INTEGER NOT NULL,
        last_updated_nanos INTEGER NOT NULL,
        rank              REAL NOT NULL,
        json              TEXT NOT NULL
    );
    CREATE INDEX idx_signals_rank
        ON signals(rank DESC, last_updated_nanos DESC);
    CREATE INDEX idx_signals_event ON signals(event_id);

    CREATE TABLE signal_categories (
        signal_id TEXT NOT NULL,
        category  TEXT NOT NULL
    );
    CREATE INDEX idx_signal_categories ON signal_categories(category, signal_id);

    CREATE TABLE signal_entities (
        signal_id TEXT NOT NULL,
        entity    TEXT NOT NULL
    );
    CREATE INDEX idx_signal_entities ON signal_entities(entity, signal_id);

    CREATE TABLE signal_lenses (
        signal_id TEXT NOT NULL,
        lens_id   TEXT NOT NULL
    );
    CREATE INDEX idx_signal_lenses ON signal_lenses(lens_id, signal_id);

    CREATE TABLE baselines (
        series_key TEXT PRIMARY KEY,
        at_nanos   INTEGER NOT NULL,
        json       TEXT NOT NULL
    );

    CREATE TABLE raw_payloads (
        hash              TEXT PRIMARY KEY,
        source_id         TEXT,
        locator           TEXT NOT NULL,
        content_type      TEXT,
        size              INTEGER NOT NULL,
        received_at_nanos INTEGER NOT NULL,
        path              TEXT NOT NULL
    );
    CREATE INDEX idx_raw_received_at ON raw_payloads(received_at_nanos);
    "#,
];

/// A persistent backend over one SQLite file plus a directory of raw payloads.
pub struct SqliteStore {
    /// rusqlite's `Connection` is `Send` but not `Sync`, and the engine shares
    /// one store behind an `RwLock`. The mutex makes the store shareable; the
    /// outer lock already serializes writers, so this never contends in
    /// practice. Without it the store could not be used from a served process
    /// at all.
    conn: std::sync::Mutex<Connection>,
    raw: FilesystemRawStore,
}

impl SqliteStore {
    /// Open (creating if needed) the database and raw directory.
    pub fn open(config: &SqliteConfig) -> Result<Self, StorageError> {
        if let Some(parent) = config.db_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    StorageError::Backend(format!("cannot create {}: {e}", parent.display()))
                })?;
            }
        }
        std::fs::create_dir_all(&config.raw_dir).map_err(|e| {
            StorageError::Backend(format!("cannot create {}: {e}", config.raw_dir.display()))
        })?;

        let conn = Connection::open(&config.db_path)
            .map_err(|e| StorageError::Backend(format!("cannot open database: {e}")))?;

        // WAL lets readers proceed while a collection cycle writes, which is
        // what keeps the API responsive during a slow ingest. `synchronous =
        // NORMAL` is the usual companion: durable against process crashes, and
        // it does not fsync on every commit.
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| StorageError::Backend(format!("cannot set WAL mode: {e}")))?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(|e| StorageError::Backend(format!("cannot set synchronous: {e}")))?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|e| StorageError::Backend(format!("cannot enable foreign keys: {e}")))?;
        // Wait rather than fail when another connection holds the write lock.
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| StorageError::Backend(format!("cannot set busy timeout: {e}")))?;

        let mut store = Self {
            conn: std::sync::Mutex::new(conn),
            raw: FilesystemRawStore::new(config.raw_dir.clone()),
        };
        store.migrate()?;
        Ok(store)
    }

    /// Borrow the connection. A poisoned lock is recovered rather than
    /// propagated: the data is still consistent (SQLite rolls back the failed
    /// transaction), and refusing every later request would turn one panic into
    /// a permanently dead process.
    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Apply any migrations the database has not seen yet.
    fn migrate(&mut self) -> Result<(), StorageError> {
        let applied: i64 = self
            .conn()
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(backend)?;

        for (index, sql) in MIGRATIONS.iter().enumerate() {
            let version = index as i64 + 1;
            if version <= applied {
                continue;
            }
            let mut conn = self.conn();
            let tx = conn.transaction().map_err(backend)?;
            tx.execute_batch(sql)
                .map_err(|e| StorageError::Backend(format!("migration {version} failed: {e}")))?;
            // `user_version` does not take a bound parameter, but `version` is
            // a loop counter over a compile-time constant list, never input.
            tx.pragma_update(None, "user_version", version)
                .map_err(backend)?;
            tx.commit().map_err(backend)?;
            tracing::info!(version, "applied database migration");
        }
        Ok(())
    }

    /// The schema version the database is at, for `/health` and tests.
    pub fn schema_version(&self) -> Result<i64, StorageError> {
        self.conn()
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(backend)
    }

    /// The raw payload store, for the drill-down's last step.
    pub fn raw(&self) -> &FilesystemRawStore {
        &self.raw
    }

    pub fn raw_mut(&mut self) -> &mut FilesystemRawStore {
        &mut self.raw
    }

    /// Load the raw index from the database so a restart can serve payloads
    /// written by a previous run.
    pub fn load_raw_index(&mut self) -> Result<(), StorageError> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT hash, path, size, received_at_nanos, content_type
                     FROM raw_payloads ORDER BY received_at_nanos ASC",
            )
            .map_err(backend)?;
        let entries: Vec<(String, String, i64, i64, Option<String>)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })
            .map_err(backend)?
            .collect::<Result<_, _>>()
            .map_err(backend)?;
        drop(stmt);
        drop(conn);
        self.raw.load_index(entries)
    }

    /// Delete raw payloads, oldest first, until the store is at or below
    /// `max_bytes`. Returns how many were removed.
    ///
    /// Oldest first: a payload fetched last week is more likely to be
    /// re-fetchable than one from a minute ago, and the recent bytes are the
    /// ones a drill-down is most likely to ask for.
    pub fn prune_raw_to(&mut self, max_bytes: u64) -> Result<usize, StorageError> {
        let mut removed = 0usize;
        let mut used = self.raw.bytes_used();
        if used <= max_bytes {
            return Ok(0);
        }

        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT hash, size FROM raw_payloads
                 ORDER BY received_at_nanos ASC, hash ASC",
            )
            .map_err(backend)?;
        let candidates: Vec<(String, i64)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(backend)?
            .collect::<Result<_, _>>()
            .map_err(backend)?;
        drop(stmt);
        // Release the guard before the loop: each iteration re-acquires it, and
        // the mutex is not reentrant.
        drop(conn);

        for (hash, size) in candidates {
            if used <= max_bytes {
                break;
            }
            self.raw.remove(&hash)?;
            self.conn()
                .execute("DELETE FROM raw_payloads WHERE hash = ?1", params![hash])
                .map_err(backend)?;
            used = used.saturating_sub(size.max(0) as u64);
            removed += 1;
        }
        Ok(removed)
    }

    /// Bytes on disk: the database file plus the retained raw payloads.
    pub fn disk_usage(&self) -> Result<DiskUsage, StorageError> {
        let db_bytes = database_bytes(&self.conn())?;
        let raw_bytes = self.raw.bytes_used();
        Ok(DiskUsage {
            db_bytes,
            raw_bytes,
            raw_files: self.raw.len() as u64,
        })
    }

    fn replace_signal_facets(tx: &Transaction<'_>, signal: &Signal) -> Result<(), StorageError> {
        let facets: [(&str, &str, Vec<String>); 2] = [
            (
                "signal_categories",
                "category",
                signal.categories.iter().map(|c| c.to_string()).collect(),
            ),
            (
                "signal_entities",
                "entity",
                signal
                    .entities
                    .iter()
                    .map(|e| e.as_str().to_string())
                    .collect(),
            ),
        ];
        for (table, column, values) in facets {
            tx.execute(
                &format!("DELETE FROM {table} WHERE signal_id = ?1"),
                params![signal.id.as_str()],
            )
            .map_err(backend)?;
            let sql = format!("INSERT INTO {table} (signal_id, {column}) VALUES (?1, ?2)");
            let mut stmt = tx.prepare(&sql).map_err(backend)?;
            for value in &values {
                stmt.execute(params![signal.id.as_str(), value])
                    .map_err(backend)?;
            }
        }

        tx.execute(
            "DELETE FROM signal_lenses WHERE signal_id = ?1",
            params![signal.id.as_str()],
        )
        .map_err(backend)?;
        {
            let mut stmt = tx
                .prepare("INSERT INTO signal_lenses (signal_id, lens_id) VALUES (?1, ?2)")
                .map_err(backend)?;
            for lens in &signal.lens_matches {
                stmt.execute(params![signal.id.as_str(), lens.as_str()])
                    .map_err(backend)?;
            }
        }
        Ok(())
    }
}

/// Total size of the database file plus its WAL and shared-memory sidecars.
fn database_bytes(conn: &Connection) -> Result<u64, StorageError> {
    let path: String = conn
        .query_row("PRAGMA database_list", [], |row| row.get::<_, String>(2))
        .map_err(backend)?;
    if path.is_empty() {
        return Ok(0);
    }
    let mut total = 0u64;
    for suffix in ["", "-wal", "-shm"] {
        let candidate = format!("{path}{suffix}");
        if let Ok(meta) = std::fs::metadata(&candidate) {
            total += meta.len();
        }
    }
    Ok(total)
}

fn backend(err: rusqlite::Error) -> StorageError {
    StorageError::Backend(err.to_string())
}

fn nanos(at: DateTime<Utc>) -> Result<i64, StorageError> {
    at.timestamp_nanos_opt()
        .ok_or_else(|| StorageError::InvalidQuery(format!("timestamp out of range: {at}")))
}

fn from_nanos(value: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(
        value.div_euclid(1_000_000_000),
        value.rem_euclid(1_000_000_000) as u32,
    )
    .unwrap_or_else(Utc::now)
}

fn to_json<T: serde::Serialize>(value: &T) -> Result<String, StorageError> {
    serde_json::to_string(value).map_err(|e| StorageError::Backend(e.to_string()))
}

fn from_json<T: serde::de::DeserializeOwned>(raw: &str) -> Result<T, StorageError> {
    serde_json::from_str(raw).map_err(|e| StorageError::Backend(e.to_string()))
}

impl ObservationStore for SqliteStore {
    fn put_observation(&mut self, observation: Observation) -> Result<(), StorageError> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(backend)?;
        insert_observation(&tx, &observation)?;
        tx.commit().map_err(backend)
    }

    /// Insert many observations in one transaction.
    ///
    /// Overridden rather than left to the trait's default because the default
    /// issues a `contains` query per row; for a collector returning hundreds of
    /// records that is hundreds of round trips and a slow fsync per insert.
    fn put_observations(&mut self, observations: Vec<Observation>) -> Result<usize, StorageError> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(backend)?;
        let mut inserted = 0usize;
        for observation in observations {
            let exists: Option<i64> = tx
                .query_row(
                    "SELECT 1 FROM observations WHERE id = ?1",
                    params![observation.id.as_str()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(backend)?;
            if exists.is_some() {
                continue;
            }
            insert_observation(&tx, &observation)?;
            inserted += 1;
        }
        tx.commit().map_err(backend)?;
        Ok(inserted)
    }

    fn get_observation(&self, id: &ObservationId) -> Result<Option<Observation>, StorageError> {
        let raw: Option<String> = self
            .conn()
            .query_row(
                "SELECT json FROM observations WHERE id = ?1",
                params![id.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(backend)?;
        raw.map(|raw| from_json(&raw)).transpose()
    }

    fn contains_observation(&self, id: &ObservationId) -> Result<bool, StorageError> {
        let found: Option<i64> = self
            .conn()
            .query_row(
                "SELECT 1 FROM observations WHERE id = ?1",
                params![id.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(backend)?;
        Ok(found.is_some())
    }

    fn query_observations(
        &self,
        query: &ObservationQuery,
    ) -> Result<Page<Observation>, StorageError> {
        let mut sql = String::from("SELECT json FROM observations WHERE 1=1");
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(source_id) = &query.source_id {
            sql.push_str(" AND source_id = ?");
            args.push(Box::new(source_id.clone()));
        }
        if let Some(entity_id) = &query.entity_id {
            sql.push_str(" AND entity_id = ?");
            args.push(Box::new(entity_id.clone()));
        }
        if let Some(metric) = &query.metric {
            sql.push_str(" AND metric = ?");
            args.push(Box::new(metric.clone()));
        }
        if let Some(series_key) = &query.series_key {
            sql.push_str(" AND series_key = ?");
            args.push(Box::new(series_key.clone()));
        }
        if let Some(range) = &query.range {
            sql.push_str(" AND observed_at_nanos >= ? AND observed_at_nanos < ?");
            args.push(Box::new(nanos(range.from)?));
            args.push(Box::new(nanos(range.to)?));
        }

        let count_sql = sql.replacen("SELECT json", "SELECT COUNT(*)", 1);
        let conn = self.conn();
        let total: i64 = conn
            .query_row(
                &count_sql,
                rusqlite::params_from_iter(args.iter().map(|a| a.as_ref())),
                |row| row.get(0),
            )
            .map_err(backend)?;

        sql.push_str(if query.newest_first {
            " ORDER BY observed_at_nanos DESC, id ASC"
        } else {
            " ORDER BY observed_at_nanos ASC, id ASC"
        });
        let limit = query.limit.unwrap_or(usize::MAX);
        sql.push_str(" LIMIT ? OFFSET ?");
        args.push(Box::new(limit as i64));
        args.push(Box::new(query.offset.unwrap_or(0) as i64));

        let mut stmt = conn.prepare(&sql).map_err(backend)?;
        let rows = stmt
            .query_map(
                rusqlite::params_from_iter(args.iter().map(|a| a.as_ref())),
                |row| row.get::<_, String>(0),
            )
            .map_err(backend)?;

        let mut items = Vec::new();
        for row in rows {
            items.push(from_json(&row.map_err(backend)?)?);
        }

        Ok(Page::new(
            items,
            total as usize,
            limit,
            query.offset.unwrap_or(0),
        ))
    }

    fn latest_observations(
        &self,
        series_key: &str,
        limit: usize,
    ) -> Result<Vec<Observation>, StorageError> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT json FROM observations WHERE series_key = ?1
                     ORDER BY observed_at_nanos DESC, id ASC LIMIT ?2",
            )
            .map_err(backend)?;
        let rows = stmt
            .query_map(params![series_key, limit as i64], |row| {
                row.get::<_, String>(0)
            })
            .map_err(backend)?;
        let mut items = Vec::new();
        for row in rows {
            items.push(from_json(&row.map_err(backend)?)?);
        }
        Ok(items)
    }

    fn observation_count(&self) -> Result<usize, StorageError> {
        let conn = self.conn();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM observations", [], |row| row.get(0))
            .map_err(backend)?;
        Ok(count as usize)
    }

    fn series_keys(&self) -> Result<Vec<String>, StorageError> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT DISTINCT series_key FROM observations ORDER BY series_key")
            .map_err(backend)?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(backend)?;
        let mut keys = Vec::new();
        for row in rows {
            keys.push(row.map_err(backend)?);
        }
        Ok(keys)
    }
}

fn insert_observation(tx: &Transaction<'_>, observation: &Observation) -> Result<(), StorageError> {
    tx.execute(
        "INSERT OR REPLACE INTO observations
           (id, source_id, entity_id, metric, series_key,
            observed_at_nanos, received_at_nanos, value, json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            observation.id.as_str(),
            observation.source_id.as_str(),
            observation.entity_id.as_ref().map(|e| e.as_str()),
            observation.metric,
            observation.series_key(),
            nanos(observation.observed_at)?,
            nanos(observation.received_at)?,
            observation.value,
            to_json(observation)?,
        ],
    )
    .map_err(backend)?;
    Ok(())
}

impl EventStore for SqliteStore {
    fn put_event(&mut self, event: Event) -> Result<(), StorageError> {
        self.conn()
            .execute(
                "INSERT OR REPLACE INTO events
                   (id, group_key, first_seen_nanos, last_seen_nanos, state, json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    event.id.as_str(),
                    event.group_key,
                    nanos(event.first_seen)?,
                    nanos(event.last_seen)?,
                    format!("{:?}", event.state),
                    to_json(&event)?,
                ],
            )
            .map_err(backend)?;
        Ok(())
    }

    fn get_event(&self, id: &EventId) -> Result<Option<Event>, StorageError> {
        let raw: Option<String> = self
            .conn()
            .query_row(
                "SELECT json FROM events WHERE id = ?1",
                params![id.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(backend)?;
        raw.map(|raw| from_json(&raw)).transpose()
    }

    fn events_in_range(&self, range: TimeRange) -> Result<Vec<Event>, StorageError> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT json FROM events
                     WHERE last_seen_nanos >= ?1 AND first_seen_nanos < ?2
                     ORDER BY first_seen_nanos ASC",
            )
            .map_err(backend)?;
        let rows = stmt
            .query_map(params![nanos(range.from)?, nanos(range.to)?], |row| {
                row.get::<_, String>(0)
            })
            .map_err(backend)?;
        let mut events = Vec::new();
        for row in rows {
            events.push(from_json(&row.map_err(backend)?)?);
        }
        Ok(events)
    }

    fn all_events(&self) -> Result<Vec<Event>, StorageError> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT json FROM events ORDER BY first_seen_nanos ASC")
            .map_err(backend)?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(backend)?;
        let mut events = Vec::new();
        for row in rows {
            events.push(from_json(&row.map_err(backend)?)?);
        }
        Ok(events)
    }

    fn event_count(&self) -> Result<usize, StorageError> {
        let conn = self.conn();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
            .map_err(backend)?;
        Ok(count as usize)
    }
}

impl SignalStore for SqliteStore {
    fn put_signal(&mut self, signal: Signal) -> Result<(), StorageError> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(backend)?;
        tx.execute(
            "INSERT OR REPLACE INTO signals
               (id, event_id, first_seen_nanos, last_updated_nanos, rank, json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                signal.id.as_str(),
                signal.event_id.as_str(),
                nanos(signal.first_seen)?,
                nanos(signal.last_updated)?,
                signal.quality.rank(),
                to_json(&signal)?,
            ],
        )
        .map_err(backend)?;
        SqliteStore::replace_signal_facets(&tx, &signal)?;
        tx.commit().map_err(backend)
    }

    fn get_signal(&self, id: &SignalId) -> Result<Option<Signal>, StorageError> {
        let raw: Option<String> = self
            .conn()
            .query_row(
                "SELECT json FROM signals WHERE id = ?1",
                params![id.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(backend)?;
        raw.map(|raw| from_json(&raw)).transpose()
    }

    fn query_signals(&self, query: &SignalQuery) -> Result<Page<Signal>, StorageError> {
        let mut sql = String::from("SELECT DISTINCT s.json FROM signals s WHERE 1=1");
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(category) = &query.category {
            sql.push_str(
                " AND EXISTS (SELECT 1 FROM signal_categories c
                   WHERE c.signal_id = s.id AND c.category = ? COLLATE NOCASE)",
            );
            args.push(Box::new(category.clone()));
        }
        if let Some(entity) = &query.entity_id {
            sql.push_str(
                " AND EXISTS (SELECT 1 FROM signal_entities e
                   WHERE e.signal_id = s.id AND e.entity = ?)",
            );
            args.push(Box::new(entity.clone()));
        }
        if let Some(ty) = query.signal_type {
            // Signal types are stored inside the serialized signal, so this
            // stays a JSON probe rather than a join table: the type set is
            // small and fixed, and a table would only mirror it.
            sql.push_str(" AND s.json LIKE ?");
            args.push(Box::new(format!("%\"{}\"%", ty.as_str())));
        }
        if let Some(lens) = &query.lens_id {
            sql.push_str(
                " AND EXISTS (SELECT 1 FROM signal_lenses l
                   WHERE l.signal_id = s.id AND l.lens_id = ?)",
            );
            args.push(Box::new(lens.clone()));
        }
        if query.active_only {
            // "Active" means the event behind the signal has not been resolved.
            // A signal whose event is gone is history, not something to
            // investigate now.
            sql.push_str(
                " AND EXISTS (SELECT 1 FROM events ev
                   WHERE ev.id = s.event_id AND ev.state != 'Resolved')",
            );
        }
        if let Some(range) = &query.range {
            sql.push_str(" AND s.last_updated_nanos >= ? AND s.last_updated_nanos < ?");
            args.push(Box::new(nanos(range.from)?));
            args.push(Box::new(nanos(range.to)?));
        }

        let count_sql = sql.replacen("SELECT DISTINCT s.json", "SELECT COUNT(*)", 1);
        let conn = self.conn();
        let total: i64 = conn
            .query_row(
                &count_sql,
                rusqlite::params_from_iter(args.iter().map(|a| a.as_ref())),
                |row| row.get(0),
            )
            .map_err(backend)?;

        sql.push_str(" ORDER BY s.rank DESC, s.last_updated_nanos DESC, s.id ASC");
        let limit = query.limit.unwrap_or(usize::MAX);
        sql.push_str(" LIMIT ? OFFSET ?");
        args.push(Box::new(limit as i64));
        args.push(Box::new(query.offset.unwrap_or(0) as i64));

        let mut stmt = conn.prepare(&sql).map_err(backend)?;
        let rows = stmt
            .query_map(
                rusqlite::params_from_iter(args.iter().map(|a| a.as_ref())),
                |row| row.get::<_, String>(0),
            )
            .map_err(backend)?;
        let mut items = Vec::new();
        for row in rows {
            items.push(from_json(&row.map_err(backend)?)?);
        }
        Ok(Page::new(
            items,
            total as usize,
            limit,
            query.offset.unwrap_or(0),
        ))
    }

    fn signals_for_event(&self, event_id: &EventId) -> Result<Vec<Signal>, StorageError> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT json FROM signals WHERE event_id = ?1
                     ORDER BY first_seen_nanos ASC",
            )
            .map_err(backend)?;
        let rows = stmt
            .query_map(params![event_id.as_str()], |row| row.get::<_, String>(0))
            .map_err(backend)?;
        let mut signals = Vec::new();
        for row in rows {
            signals.push(from_json(&row.map_err(backend)?)?);
        }
        Ok(signals)
    }

    fn signal_count(&self) -> Result<usize, StorageError> {
        let conn = self.conn();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM signals", [], |row| row.get(0))
            .map_err(backend)?;
        Ok(count as usize)
    }
}

impl SourceStore for SqliteStore {
    fn put_source(&mut self, source: Source) -> Result<(), StorageError> {
        self.conn()
            .execute(
                "INSERT OR REPLACE INTO sources (id, priority, category, json)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    source.id.as_str(),
                    source.priority as i64,
                    source.category,
                    to_json(&source)?,
                ],
            )
            .map_err(backend)?;
        Ok(())
    }

    fn get_source(&self, id: &SourceId) -> Result<Option<Source>, StorageError> {
        let raw: Option<String> = self
            .conn()
            .query_row(
                "SELECT json FROM sources WHERE id = ?1",
                params![id.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(backend)?;
        raw.map(|raw| from_json(&raw)).transpose()
    }

    fn all_sources(&self) -> Result<Vec<Source>, StorageError> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT json FROM sources ORDER BY priority ASC, id ASC")
            .map_err(backend)?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(backend)?;
        let mut sources = Vec::new();
        for row in rows {
            sources.push(from_json(&row.map_err(backend)?)?);
        }
        Ok(sources)
    }

    fn put_health(&mut self, health: SourceHealth) -> Result<(), StorageError> {
        self.conn()
            .execute(
                "INSERT OR REPLACE INTO source_health (source_id, status, json)
                 VALUES (?1, ?2, ?3)",
                params![
                    health.source_id.as_str(),
                    format!("{:?}", health.status),
                    to_json(&health)?,
                ],
            )
            .map_err(backend)?;
        Ok(())
    }

    fn get_health(&self, id: &SourceId) -> Result<Option<SourceHealth>, StorageError> {
        let raw: Option<String> = self
            .conn()
            .query_row(
                "SELECT json FROM source_health WHERE source_id = ?1",
                params![id.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(backend)?;
        raw.map(|raw| from_json(&raw)).transpose()
    }
}

impl BaselineStore for SqliteStore {
    fn put_baseline(
        &mut self,
        series_key: &str,
        at: DateTime<Utc>,
        snapshot: BaselineSnapshot,
    ) -> Result<(), StorageError> {
        self.conn()
            .execute(
                "INSERT OR REPLACE INTO baselines (series_key, at_nanos, json)
                 VALUES (?1, ?2, ?3)",
                params![series_key, nanos(at)?, to_json(&snapshot)?],
            )
            .map_err(backend)?;
        Ok(())
    }

    fn get_baseline(
        &self,
        series_key: &str,
    ) -> Result<Option<(DateTime<Utc>, BaselineSnapshot)>, StorageError> {
        let row: Option<(i64, String)> = self
            .conn()
            .query_row(
                "SELECT at_nanos, json FROM baselines WHERE series_key = ?1",
                params![series_key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(backend)?;
        match row {
            Some((at, json)) => Ok(Some((from_nanos(at), from_json(&json)?))),
            None => Ok(None),
        }
    }
}

impl RawStore for SqliteStore {
    /// Retain a payload: write the bytes to a content-addressed file, then
    /// record the metadata row. The row is what lets a later run find the file
    /// and what retention orders by, so the two writes belong together.
    fn put(
        &mut self,
        reference: wse_model::RawReference,
        body: Vec<u8>,
    ) -> Result<(), StorageError> {
        let hash = reference.hash.clone();
        self.raw.put(reference.clone(), body.clone())?;
        let path = self
            .raw
            .location(&hash)
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        self.conn()
            .execute(
                "INSERT OR REPLACE INTO raw_payloads
                   (hash, source_id, locator, content_type, size, received_at_nanos, path)
                 VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?6)",
                params![
                    hash,
                    reference.locator,
                    reference.content_type,
                    body.len() as i64,
                    nanos(Utc::now())?,
                    path,
                ],
            )
            .map_err(backend)?;
        Ok(())
    }

    fn get(&self, hash: &str) -> Result<Option<StoredPayload>, StorageError> {
        self.raw.get(hash)
    }

    fn len(&self) -> usize {
        self.raw.len()
    }

    fn bytes_used(&self) -> u64 {
        self.raw.bytes_used()
    }
}

impl MaintenanceStore for SqliteStore {
    /// Delete observations older than `cutoff`.
    ///
    /// Events and signals are deliberately **not** deleted here. They are the
    /// human-readable history; an observation is a raw measurement and can be
    /// aged out, but removing the signal a person was investigating because its
    /// underlying samples expired would be losing the conclusion to save the
    /// working. Signals age out on their own schedule, or not at all.
    fn delete_observations_before(&mut self, cutoff: DateTime<Utc>) -> Result<usize, StorageError> {
        let deleted = self
            .conn()
            .execute(
                "DELETE FROM observations WHERE observed_at_nanos < ?1",
                params![nanos(cutoff)?],
            )
            .map_err(backend)?;
        Ok(deleted)
    }

    fn prune_raw_to(&mut self, max_bytes: u64) -> Result<usize, StorageError> {
        SqliteStore::prune_raw_to(self, max_bytes)
    }

    fn disk_usage(&self) -> Result<DiskUsage, StorageError> {
        SqliteStore::disk_usage(self)
    }
}

/// Content-addressed raw payloads on the filesystem.
///
/// The hash *is* the address, so storing the same payload twice costs nothing
/// and the bytes for a hash can be verified by re-hashing them. The database
/// holds the metadata; the bytes live here.
pub struct FilesystemRawStore {
    root: PathBuf,
    index: std::collections::HashMap<String, StoredPayload>,
    /// Sizes of payloads known to the store, for `bytes_used` without stat-ing
    /// the whole directory on every metrics scrape.
    known: std::collections::HashMap<String, (PathBuf, u64, DateTime<Utc>)>,
}

impl FilesystemRawStore {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            index: std::collections::HashMap::new(),
            known: std::collections::HashMap::new(),
        }
    }

    /// Where a hash's bytes live: `raw/<first two chars>/<hash>`.
    ///
    /// Sharding on the prefix keeps any one directory from holding every file,
    /// which matters on filesystems that degrade with large flat directories.
    pub fn path_for(&self, hash: &str) -> PathBuf {
        let prefix = hash.get(0..2).unwrap_or("00");
        self.root.join(prefix).join(hash)
    }

    /// Rebuild the in-memory index from the database's metadata.
    ///
    /// Called once at startup so `bytes_used` and pruning are correct for
    /// payloads written by a previous run, without walking the filesystem.
    pub fn load_index(
        &mut self,
        entries: Vec<(String, String, i64, i64, Option<String>)>,
    ) -> Result<(), StorageError> {
        for (hash, path, size, received_at, content_type) in entries {
            let path = PathBuf::from(path);
            let size = size.max(0) as u64;
            // Metadata-only entries: the body is read lazily on `get`, so a
            // restart does not load gigabytes of payloads into RAM.
            self.index
                .entry(hash.clone())
                .or_insert_with(|| StoredPayload {
                    reference: wse_model::RawReference {
                        locator: path.display().to_string(),
                        hash: hash.clone(),
                        content_type,
                        bytes: Some(size),
                    },
                    body: Vec::new(),
                });
            self.known
                .insert(hash.clone(), (path, size, from_nanos(received_at)));
        }
        Ok(())
    }

    fn write_file(&self, hash: &str, body: &[u8]) -> Result<PathBuf, StorageError> {
        let path = self.path_for(hash);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                StorageError::Backend(format!("cannot create {}: {e}", parent.display()))
            })?;
        }
        // Write to a temporary file and rename, so a crash mid-write cannot
        // leave a truncated file under a hash that promises specific bytes.
        let temp = path.with_extension("tmp");
        std::fs::write(&temp, body)
            .map_err(|e| StorageError::Backend(format!("cannot write {}: {e}", temp.display())))?;
        std::fs::rename(&temp, &path)
            .map_err(|e| StorageError::Backend(format!("cannot rename into place: {e}")))?;
        Ok(path)
    }

    /// Drop a payload: delete its file and forget it.
    ///
    /// Both halves matter. Deleting only the file leaves the size in `known`,
    /// so `bytes_used` keeps counting bytes that are gone and retention keeps
    /// pruning until the store looks empty. Missing files are not an error:
    /// retention running twice must be safe.
    pub fn remove(&mut self, hash: &str) -> Result<(), StorageError> {
        if let Some((path, _, _)) = self.known.remove(hash) {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(StorageError::Backend(format!(
                        "cannot remove {}: {e}",
                        path.display()
                    )))
                }
            }
        }
        self.index.remove(hash);
        Ok(())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl RawStore for FilesystemRawStore {
    fn put(
        &mut self,
        reference: wse_model::RawReference,
        body: Vec<u8>,
    ) -> Result<(), StorageError> {
        if body.len() as u64 != reference.bytes.unwrap_or(body.len() as u64) {
            return Err(StorageError::InvalidQuery(format!(
                "raw payload length {} does not match its reference ({})",
                body.len(),
                reference.bytes.unwrap_or_default()
            )));
        }
        let hash = reference.hash.clone();
        if self.known.contains_key(&hash) {
            return Ok(());
        }
        let path = self.write_file(&hash, &body)?;
        let size = body.len() as u64;
        self.known
            .insert(hash.clone(), (path.clone(), size, Utc::now()));
        self.index.insert(hash, StoredPayload { reference, body });
        Ok(())
    }

    fn get(&self, hash: &str) -> Result<Option<StoredPayload>, StorageError> {
        // Prefer the cached body (a payload stored this run), and fall back to
        // reading the file (a payload stored by a previous run).
        if let Some(payload) = self.index.get(hash) {
            if !payload.body.is_empty() {
                return Ok(Some(payload.clone()));
            }
        }
        let Some((path, size, _)) = self.known.get(hash) else {
            return Ok(None);
        };
        let body = match std::fs::read(path) {
            Ok(body) => body,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(StorageError::Backend(format!(
                    "cannot read {}: {e}",
                    path.display()
                )))
            }
        };
        let reference = self
            .index
            .get(hash)
            .map(|p| p.reference.clone())
            .unwrap_or_else(|| wse_model::RawReference {
                locator: path.display().to_string(),
                hash: hash.to_string(),
                content_type: None,
                bytes: Some(*size),
            });
        Ok(Some(StoredPayload { reference, body }))
    }

    fn len(&self) -> usize {
        self.known.len()
    }

    fn bytes_used(&self) -> u64 {
        self.known.values().map(|(_, size, _)| *size).sum()
    }

    fn location(&self, hash: &str) -> Option<PathBuf> {
        self.known.get(hash).map(|(path, _, _)| path.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wse_model::{Evidence, SignalQuality, SignalType};

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
    }

    fn temp_config(name: &str) -> SqliteConfig {
        let root = std::env::temp_dir().join(format!(
            "wse-test-{name}-{}-{}",
            std::process::id(),
            // A counter is enough; tests are the only caller.
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        SqliteConfig::new(root.join("wse.db"), root.join("raw"))
    }

    fn obs(suffix: &str, secs: i64, value: f64) -> Observation {
        Observation::new(
            SourceId::new("src_a"),
            None,
            format!("metric_{suffix}"),
            value,
            "unit",
            at(secs),
            wse_model::RawReference::new("raw", format!("h{secs}")),
        )
    }

    #[test]
    fn migrations_are_applied_once_and_are_idempotent() {
        let config = temp_config("migrate");
        let store = SqliteStore::open(&config).unwrap();
        assert_eq!(store.schema_version().unwrap(), MIGRATIONS.len() as i64);
        drop(store);
        // Reopening must not re-run the migration and must not fail.
        let store = SqliteStore::open(&config).unwrap();
        assert_eq!(store.schema_version().unwrap(), MIGRATIONS.len() as i64);
        std::fs::remove_dir_all(config.db_path.parent().unwrap()).ok();
    }

    #[test]
    fn observations_survive_a_reopen() {
        let config = temp_config("reopen");
        {
            let mut store = SqliteStore::open(&config).unwrap();
            store
                .put_observations(vec![obs("x", 0, 1.0), obs("x", 60, 2.0)])
                .unwrap();
        }
        let store = SqliteStore::open(&config).unwrap();
        assert_eq!(store.observation_count().unwrap(), 2);
        let key = obs("x", 0, 1.0).series_key();
        let latest = store.latest_observations(&key, 1).unwrap();
        assert_eq!(latest[0].value, 2.0);
        std::fs::remove_dir_all(config.db_path.parent().unwrap()).ok();
    }

    #[test]
    fn duplicate_observation_ids_are_not_stored_twice() {
        let config = temp_config("dedup");
        let mut store = SqliteStore::open(&config).unwrap();
        let o = obs("x", 0, 1.0);
        assert_eq!(
            store.put_observations(vec![o.clone(), o.clone()]).unwrap(),
            1
        );
        assert_eq!(store.observation_count().unwrap(), 1);
        std::fs::remove_dir_all(config.db_path.parent().unwrap()).ok();
    }

    #[test]
    fn observation_range_and_filters_are_respected() {
        let config = temp_config("query");
        let mut store = SqliteStore::open(&config).unwrap();
        for i in 0..10 {
            store.put_observation(obs("x", i * 60, i as f64)).unwrap();
        }
        let page = store
            .query_observations(&ObservationQuery::default().with_limit(3))
            .unwrap();
        assert_eq!(page.total, 10);
        assert_eq!(page.items.len(), 3);
        assert_eq!(page.items[0].value, 0.0);

        let range = TimeRange::new(at(120), at(300));
        let windowed = store
            .query_observations(&ObservationQuery::default().in_range(range))
            .unwrap();
        assert_eq!(windowed.total, 3);

        let newest = store
            .query_observations(&ObservationQuery::default().newest_first())
            .unwrap();
        assert_eq!(newest.items[0].value, 9.0);
        std::fs::remove_dir_all(config.db_path.parent().unwrap()).ok();
    }

    #[test]
    fn signals_persist_with_their_facets_and_rank_order() {
        let config = temp_config("signals");
        let mut store = SqliteStore::open(&config).unwrap();

        let mut low = Signal::new(EventId::new("evt_1"), at(0));
        low.add_type(SignalType::Now);
        low.categories = vec!["technology".into()];
        low.entities = vec![wse_model::EntityId::new("ecosystem_rust")];
        low.lens_matches = vec![wse_model::LensId::new("lens_global")];
        low.quality = SignalQuality::default();

        let mut high = Signal::new(EventId::new("evt_2"), at(0));
        high.add_type(SignalType::Anomaly);
        high.categories = vec!["technology".into()];
        high.quality.strength = 1.0;
        high.quality.confidence = 1.0;

        store.put_signal(low).unwrap();
        store.put_signal(high).unwrap();

        let all = store.query_signals(&SignalQuery::default()).unwrap();
        assert_eq!(all.total, 2);
        assert_eq!(all.items[0].types, vec![SignalType::Anomaly]);

        let by_category = store
            .query_signals(&SignalQuery::default().with_category("Technology"))
            .unwrap();
        assert_eq!(by_category.total, 2, "category match is case-insensitive");

        let by_type = store
            .query_signals(&SignalQuery::default().with_type(SignalType::Now))
            .unwrap();
        assert_eq!(by_type.total, 1);

        let by_lens = store
            .query_signals(&SignalQuery {
                lens_id: Some("lens_global".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(by_lens.total, 1);

        let by_entity = store
            .query_signals(&SignalQuery {
                entity_id: Some("ecosystem_rust".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(by_entity.total, 1);

        std::fs::remove_dir_all(config.db_path.parent().unwrap()).ok();
    }

    #[test]
    fn re_storing_a_signal_replaces_its_facets_rather_than_duplicating_them() {
        let config = temp_config("facets");
        let mut store = SqliteStore::open(&config).unwrap();
        let mut signal = Signal::new(EventId::new("evt_1"), at(0));
        signal.categories = vec!["technology".into()];
        signal.lens_matches = vec![wse_model::LensId::new("lens_global")];
        store.put_signal(signal.clone()).unwrap();

        signal.categories = vec!["space".into()];
        signal.lens_matches = vec![wse_model::LensId::new("lens_space")];
        store.put_signal(signal.clone()).unwrap();

        let by_old = store
            .query_signals(&SignalQuery::default().with_category("technology"))
            .unwrap();
        assert_eq!(by_old.total, 0, "stale facet rows must not linger");
        let by_new = store
            .query_signals(&SignalQuery::default().with_category("space"))
            .unwrap();
        assert_eq!(by_new.total, 1);
        std::fs::remove_dir_all(config.db_path.parent().unwrap()).ok();
    }

    #[test]
    fn active_only_excludes_signals_whose_event_is_resolved() {
        let config = temp_config("active");
        let mut store = SqliteStore::open(&config).unwrap();

        let mut active = Event::new_for("k1", "active", at(0));
        active.state = wse_model::EventState::Changing;
        let mut resolved = Event::new_for("k2", "resolved", at(0));
        resolved.state = wse_model::EventState::Resolved;
        store.put_event(active.clone()).unwrap();
        store.put_event(resolved.clone()).unwrap();

        store
            .put_signal(Signal::new(active.id.clone(), at(0)))
            .unwrap();
        store
            .put_signal(Signal::new(resolved.id.clone(), at(0)))
            .unwrap();

        let all = store.query_signals(&SignalQuery::default()).unwrap();
        assert_eq!(all.total, 2);
        let only_active = store.query_signals(&SignalQuery::active()).unwrap();
        assert_eq!(only_active.total, 1);
        std::fs::remove_dir_all(config.db_path.parent().unwrap()).ok();
    }

    #[test]
    fn events_and_sources_round_trip() {
        let config = temp_config("events");
        let mut store = SqliteStore::open(&config).unwrap();
        let mut event = Event::new_for("k", "quake", at(100));
        event.observe(at(200));
        store.put_event(event.clone()).unwrap();
        assert_eq!(store.get_event(&event.id).unwrap().unwrap(), event);
        assert_eq!(
            store
                .events_in_range(TimeRange::new(at(0), at(300)))
                .unwrap()
                .len(),
            1
        );

        let source = Source::new(SourceId::new("src_a"), "A", "a");
        store.put_source(source.clone()).unwrap();
        assert_eq!(store.get_source(&source.id).unwrap().unwrap(), source);
        assert!(store.get_health(&source.id).unwrap().is_none());
        let mut health = SourceHealth::new(source.id.clone());
        health.record_failure(at(0));
        store.put_health(health.clone()).unwrap();
        assert_eq!(store.get_health(&source.id).unwrap().unwrap(), health);
        std::fs::remove_dir_all(config.db_path.parent().unwrap()).ok();
    }

    #[test]
    fn baselines_survive_a_reopen() {
        let config = temp_config("baselines");
        let snapshot = BaselineSnapshot {
            sample_size: 10,
            mean: 1.0,
            median: 1.0,
            std_dev: 0.5,
            mad: 0.4,
            p05: 0.2,
            p95: 1.8,
            ewma: 1.1,
            trend_per_second: 0.01,
            volatility: 0.3,
        };
        {
            let mut store = SqliteStore::open(&config).unwrap();
            store.put_baseline("k", at(0), snapshot.clone()).unwrap();
        }
        let store = SqliteStore::open(&config).unwrap();
        let (when, got) = store.get_baseline("k").unwrap().unwrap();
        assert_eq!(when, at(0));
        assert_eq!(got, snapshot);
        std::fs::remove_dir_all(config.db_path.parent().unwrap()).ok();
    }

    #[test]
    fn evidence_survives_for_the_drill_down() {
        let config = temp_config("evidence");
        let mut store = SqliteStore::open(&config).unwrap();
        let mut signal = Signal::new(EventId::new("evt_1"), at(0));
        signal.evidence.push(Evidence {
            source_id: SourceId::new("src_a"),
            observation_id: ObservationId::new("obs_1"),
            metric: "m".into(),
            unit: "u".into(),
            statement: "statement".into(),
            observed_at: at(0),
            value: 1.0,
            deviation_sigma: Some(4.1),
        });
        store.put_signal(signal.clone()).unwrap();
        let got = store.get_signal(&signal.id).unwrap().unwrap();
        assert_eq!(got.evidence[0].observation_id.as_str(), "obs_1");
        std::fs::remove_dir_all(config.db_path.parent().unwrap()).ok();
    }

    #[test]
    fn retention_deletes_only_old_observations() {
        let config = temp_config("retention");
        let mut store = SqliteStore::open(&config).unwrap();
        for i in 0..10 {
            store.put_observation(obs("x", i * 3600, i as f64)).unwrap();
        }
        let deleted = store.delete_observations_before(at(5 * 3600)).unwrap();
        assert_eq!(deleted, 5);
        assert_eq!(store.observation_count().unwrap(), 5);
        std::fs::remove_dir_all(config.db_path.parent().unwrap()).ok();
    }

    #[test]
    fn raw_retention_prunes_oldest_payloads_until_under_the_cap() {
        let config = temp_config("raw-retention");
        let mut store = SqliteStore::open(&config).unwrap();

        // Four payloads of 100 bytes each, written oldest first.
        for i in 0..4 {
            let mut reference = wse_model::RawReference::new("raw", format!("h{i}"));
            reference.bytes = Some(100);
            store.put(reference, vec![i as u8; 100]).unwrap();
        }
        assert_eq!(store.bytes_used(), 400);

        // Cap at 250 bytes: dropping the oldest payload (400 -> 300) is not
        // enough, so the second oldest goes too (300 -> 200). Two removals.
        let removed = store.prune_raw_to(250).unwrap();
        assert_eq!(removed, 2);
        assert_eq!(store.len(), 2);
        // The newest two survived; the oldest are gone.
        assert!(store.get("h2").unwrap().is_some());
        assert!(store.get("h3").unwrap().is_some());
        assert!(store.get("h0").unwrap().is_none());

        // Pruning under the cap is a no-op, not an error.
        assert_eq!(store.prune_raw_to(1_000).unwrap(), 0);

        // A reopened store must still see the surviving payloads, which proves
        // the metadata rows were updated alongside the files.
        drop(store);
        let mut reopened = SqliteStore::open(&config).unwrap();
        reopened.load_raw_index().unwrap();
        assert_eq!(reopened.len(), 2);
        assert_eq!(reopened.get("h3").unwrap().unwrap().body.len(), 100);
        std::fs::remove_dir_all(config.db_path.parent().unwrap()).ok();
    }
}
