//! HTTP handlers.
//!
//! Every handler is a thin read over the engine. Responses are the domain
//! types serialized directly, so the API and the model can never drift apart.
//!
//! A handful of handlers are writes rather than reads — the Control screen's
//! collection switch, per-source enable/disable and "run now". They touch only
//! [`RuntimeState`](wse_engine::RuntimeState), never detector state, so they
//! cannot corrupt the pipeline they control.

use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    Json,
};
use futures_util::stream::{self, Stream};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::time::Duration;
use wse_engine::runtime::{ActivityKind, DiskSummary, LatencySummary, SourceControl};
use wse_engine::ControlSnapshot;
use wse_model::{
    Cadence, EntityId, EventId, Observation, ObservationId, SignalId, SignalType, Source,
    SourceHealth, SourceId,
};
use wse_storage::{MaintenanceStore, ObservationQuery, SignalQuery};

use crate::state::AppState;

/// A cadence rendered for display, e.g. `15m`, `1h`, `event`, `daily@06Z`.
///
/// The UI shows this next to a source so the reader can judge how fresh an
/// observation *should* be. It is derived from the catalog, never from how
/// often we happen to have polled, so a mis-scheduled source is visible rather
/// than hidden.
pub fn cadence_label(cadence: Cadence) -> String {
    match cadence {
        Cadence::Event => "event".to_string(),
        Cadence::Interval { seconds } => human_seconds(seconds),
        Cadence::Daily { hour_utc } => format!("daily@{hour_utc:02}Z"),
        Cadence::Irregular => "irregular".to_string(),
    }
}

fn human_seconds(seconds: u64) -> String {
    if seconds.is_multiple_of(86_400) && seconds >= 86_400 {
        format!("{}d", seconds / 86_400)
    } else if seconds.is_multiple_of(3_600) && seconds >= 3_600 {
        format!("{}h", seconds / 3_600)
    } else if seconds.is_multiple_of(60) && seconds >= 60 {
        format!("{}m", seconds / 60)
    } else {
        format!("{seconds}s")
    }
}

/// An observation as the UI needs it: the measurement plus the derived values
/// (series key, source lag) that make the drill-down readable.
#[derive(Debug, Serialize)]
pub struct ObservationView {
    #[serde(flatten)]
    pub observation: Observation,
    /// The series this observation belongs to; the handle for `/timeline`.
    pub series_key: String,
    /// Source-side lag in milliseconds (`received_at - observed_at`).
    ///
    /// Non-zero lag is the difference between "the world changed" and "we just
    /// heard about it", and the reader must be able to see which one it is.
    pub lag_ms: i64,
}

impl ObservationView {
    pub fn new(observation: Observation) -> Self {
        Self {
            series_key: observation.series_key(),
            lag_ms: observation.lag_ms(),
            observation,
        }
    }
}

/// A source plus its display cadence and current health.
#[derive(Debug, Serialize)]
pub struct SourceView {
    #[serde(flatten)]
    pub source: Source,
    /// Human-readable cadence, derived from the catalog.
    pub cadence_label: String,
    pub health: Option<SourceHealth>,
}

impl SourceView {
    pub fn new(source: Source, health: Option<SourceHealth>) -> Self {
        let cadence_label = cadence_label(source.cadence);
        Self {
            cadence_label,
            source,
            health,
        }
    }
}

/// A uniform error body.
#[derive(Debug, Serialize)]
pub struct ApiError {
    pub error: String,
}

/// Convert any handler failure into a JSON response.
fn not_found(what: impl std::fmt::Display) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(ApiError {
            error: format!("{what} not found"),
        }),
    )
        .into_response()
}

fn bad_request(message: impl std::fmt::Display) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(ApiError {
            error: message.to_string(),
        }),
    )
        .into_response()
}

fn internal(message: impl std::fmt::Display) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiError {
            error: message.to_string(),
        }),
    )
        .into_response()
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub sources: usize,
    pub observations: usize,
    pub events: usize,
    pub signals: usize,
}

/// `GET /health`
pub async fn health<S: wse_storage::Store>(State(state): State<AppState<S>>) -> Response {
    let engine = state.read().await;
    let store = engine.store();
    let (sources, observations, events, signals) = (
        store.all_sources().map(|s| s.len()).unwrap_or(0),
        store.observation_count().unwrap_or(0),
        store.event_count().unwrap_or(0),
        store.signal_count().unwrap_or(0),
    );
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        sources,
        observations,
        events,
        signals,
    })
    .into_response()
}

/// `GET /metrics`
pub async fn metrics<S: wse_storage::Store>(State(state): State<AppState<S>>) -> Response {
    let engine = state.read().await;
    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4")],
        engine.metrics().render(),
    )
        .into_response()
}

/// Query parameters accepted by `GET /signals`.
#[derive(Debug, Deserialize)]
pub struct SignalParams {
    pub category: Option<String>,
    pub entity: Option<String>,
    #[serde(rename = "type")]
    pub signal_type: Option<String>,
    pub lens: Option<String>,
    pub active: Option<bool>,
    pub status: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

fn parse_signal_type(raw: &str) -> Option<SignalType> {
    match raw.to_ascii_uppercase().as_str() {
        "NOW" => Some(SignalType::Now),
        "ANOMALY" => Some(SignalType::Anomaly),
        "EARLY_SIGNAL" => Some(SignalType::EarlySignal),
        "CONVERGENCE" => Some(SignalType::Convergence),
        "IMPACT" => Some(SignalType::Impact),
        _ => None,
    }
}

fn parse_status(raw: &str) -> Option<wse_model::SignalStatus> {
    use wse_model::SignalStatus;
    match raw.to_ascii_lowercase().as_str() {
        "new" => Some(SignalStatus::New),
        "developing" => Some(SignalStatus::Developing),
        "confirmed" => Some(SignalStatus::Confirmed),
        "stable" => Some(SignalStatus::Stable),
        "fading" => Some(SignalStatus::Fading),
        "resolved" => Some(SignalStatus::Resolved),
        _ => None,
    }
}

/// `GET /signals`
pub async fn list_signals<S: wse_storage::Store>(
    State(state): State<AppState<S>>,
    Query(params): Query<SignalParams>,
) -> Response {
    let signal_type = match params.signal_type.as_deref() {
        Some(raw) => match parse_signal_type(raw) {
            Some(ty) => Some(ty),
            None => return bad_request(format!("unknown signal type: {raw}")),
        },
        None => None,
    };

    let status = match params.status.as_deref() {
        Some(raw) => match parse_status(raw) {
            Some(st) => Some(st),
            None => return bad_request(format!("unknown signal status: {raw}")),
        },
        None => None,
    };

    let query = SignalQuery {
        category: params.category,
        entity_id: params.entity,
        signal_type,
        lens_id: params.lens,
        active_only: params.active.unwrap_or(false),
        status,
        range: None,
        limit: params.limit,
        offset: params.offset,
    };

    let engine = state.read().await;
    match engine.store().query_signals(&query) {
        Ok(page) => Json(page).into_response(),
        Err(err) => internal(err),
    }
}

/// `GET /signals/:id`
pub async fn get_signal<S: wse_storage::Store>(
    State(state): State<AppState<S>>,
    Path(id): Path<String>,
) -> Response {
    let engine = state.read().await;
    match engine.signal(&SignalId::new(id.clone())) {
        Some(signal) => Json(signal).into_response(),
        None => not_found(format!("signal {id}")),
    }
}

/// `GET /events/:id`
pub async fn get_event<S: wse_storage::Store>(
    State(state): State<AppState<S>>,
    Path(id): Path<String>,
) -> Response {
    let engine = state.read().await;
    match engine.event(&EventId::new(id.clone())) {
        Some(event) => Json(event).into_response(),
        None => not_found(format!("event {id}")),
    }
}

/// `GET /observations/:id`
pub async fn get_observation<S: wse_storage::Store>(
    State(state): State<AppState<S>>,
    Path(id): Path<String>,
) -> Response {
    let engine = state.read().await;
    match engine.observation(&ObservationId::new(id.clone())) {
        Some(observation) => Json(ObservationView::new(observation)).into_response(),
        None => not_found(format!("observation {id}")),
    }
}

/// `GET /observations/:id/raw`
///
/// The last step of the drill-down: the bytes the source actually returned.
pub async fn get_observation_raw<S: wse_storage::Store>(
    State(state): State<AppState<S>>,
    Path(id): Path<String>,
) -> Response {
    let engine = state.read().await;
    let Some(observation) = engine.observation(&ObservationId::new(id.clone())) else {
        return not_found(format!("observation {id}"));
    };
    let hash = &observation.raw.hash;
    match engine.raw_payload(hash) {
        Some(payload) => {
            let content_type = payload
                .reference
                .content_type
                .clone()
                .unwrap_or_else(|| "application/octet-stream".to_string());
            ([(header::CONTENT_TYPE, content_type)], payload.body.clone()).into_response()
        }
        // The reference is still meaningful even when the body was not
        // retained (for example data ingested before this endpoint existed),
        // so answer with what we know rather than a bare 404.
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "raw payload not retained",
                "observation_id": id,
                "raw": observation.raw,
            })),
        )
            .into_response(),
    }
}

/// `GET /sources`
pub async fn list_sources<S: wse_storage::Store>(State(state): State<AppState<S>>) -> Response {
    let engine = state.read().await;
    match engine.store().all_sources() {
        Ok(sources) => {
            let views: Vec<SourceView> = sources
                .into_iter()
                .map(|source| {
                    let health = engine.source_health(&source.id);
                    SourceView::new(source, health)
                })
                .collect();
            Json(views).into_response()
        }
        Err(err) => internal(err),
    }
}

/// `GET /sources/:id`
pub async fn get_source<S: wse_storage::Store>(
    State(state): State<AppState<S>>,
    Path(id): Path<String>,
) -> Response {
    let engine = state.read().await;
    let source_id = SourceId::new(id.clone());
    match engine.store().get_source(&source_id) {
        Ok(Some(source)) => {
            let health = engine.source_health(&source_id);
            Json(SourceView::new(source, health)).into_response()
        }
        Ok(None) => not_found(format!("source {id}")),
        Err(err) => internal(err),
    }
}

#[derive(Debug, Serialize)]
pub struct EntityDetail {
    pub entity_id: String,
    pub signals: Vec<wse_model::Signal>,
    pub observations: Vec<ObservationView>,
}

/// `GET /entities/:id`
///
/// Entities are not a first-class store in the MVP; this endpoint resolves the
/// entity by searching the signals and observations that reference it.
pub async fn get_entity<S: wse_storage::Store>(
    State(state): State<AppState<S>>,
    Path(id): Path<String>,
) -> Response {
    let engine = state.read().await;
    let store = engine.store();
    let signals = match store.query_signals(&SignalQuery {
        entity_id: Some(id.clone()),
        ..SignalQuery::default()
    }) {
        Ok(page) => page.items,
        Err(err) => return internal(err),
    };
    let observations = match store.query_observations(&ObservationQuery {
        entity_id: Some(id.clone()),
        limit: Some(100),
        ..ObservationQuery::default()
    }) {
        Ok(page) => page.items,
        Err(err) => return internal(err),
    };
    if signals.is_empty() && observations.is_empty() {
        return not_found(format!("entity {id}"));
    }
    Json(EntityDetail {
        entity_id: id,
        signals,
        observations: observations.into_iter().map(ObservationView::new).collect(),
    })
    .into_response()
}

#[derive(Debug, Deserialize)]
pub struct TimelineParams {
    pub series: String,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct TimelineResponse {
    pub series_key: String,
    pub observations: Vec<ObservationView>,
    pub baseline: Option<wse_model::BaselineSnapshot>,
}

/// `GET /timeline?series=&limit=`
///
/// The raw material for the `NORMAL ────╮ ╰──● NOW` visualisation: the recent
/// points of one series plus the baseline they were compared against.
pub async fn timeline<S: wse_storage::Store>(
    State(state): State<AppState<S>>,
    Query(params): Query<TimelineParams>,
) -> Response {
    let limit = params.limit.unwrap_or(200).min(5_000);
    let engine = state.read().await;
    let store = engine.store();

    let observations = match store.latest_observations(&params.series, limit) {
        Ok(o) => o,
        Err(err) => return internal(err),
    };
    if observations.is_empty() {
        return not_found(format!("series {}", params.series));
    }
    let baseline = wse_storage::BaselineStore::get_baseline(store, &params.series)
        .ok()
        .flatten()
        .map(|(_, snapshot)| snapshot);

    Json(TimelineResponse {
        series_key: params.series,
        observations: observations.into_iter().map(ObservationView::new).collect(),
        baseline,
    })
    .into_response()
}

/// A lens plus how many signals it currently shows.
///
/// The count is included so the UI can distinguish "this lens matches nothing
/// yet" from "this lens is misconfigured" — a lens whose categories no collector
/// emits is legitimately empty, and that is worth seeing rather than guessing.
#[derive(Debug, Serialize)]
pub struct LensSummary {
    #[serde(flatten)]
    pub lens: wse_model::lens::Lens,
    pub matching_signals: usize,
}

/// `GET /lenses`
pub async fn list_lenses<S: wse_storage::Store>(State(state): State<AppState<S>>) -> Response {
    let engine = state.read().await;
    let signals = match engine.store().query_signals(&SignalQuery::default()) {
        Ok(page) => page.items,
        Err(err) => return internal(err),
    };
    let summaries: Vec<LensSummary> = engine
        .lenses()
        .iter()
        .map(|lens| LensSummary {
            matching_signals: signals
                .iter()
                .filter(|s| s.lens_matches.contains(&lens.id))
                .count(),
            lens: lens.clone(),
        })
        .collect();
    Json(summaries).into_response()
}

/// `GET /lenses/:id`
pub async fn get_lens<S: wse_storage::Store>(
    State(state): State<AppState<S>>,
    Path(id): Path<String>,
) -> Response {
    let engine = state.read().await;
    let Some(lens) = engine.lenses().iter().find(|l| l.id.as_str() == id) else {
        return not_found(format!("lens {id}"));
    };
    let signals = match engine.store().query_signals(&SignalQuery {
        lens_id: Some(id),
        ..SignalQuery::default()
    }) {
        Ok(page) => page.items,
        Err(err) => return internal(err),
    };
    Json(LensSummary {
        matching_signals: signals.len(),
        lens: lens.clone(),
    })
    .into_response()
}

/// Parse an entity id, exposed for tests.
pub fn entity_id(raw: &str) -> EntityId {
    EntityId::new(raw)
}

/* --------------------------------------------------------------- WORLD -- */

/// The one-screen answer to "what is changing in the world right now?".
///
/// The WORLD view previously had to fetch every signal and count in the client.
/// This composes the answer from the store and the runtime, so the screen opens
/// with a single request and the numbers on it come from the same place the
/// feed does.
#[derive(Debug, Serialize)]
pub struct WorldSummary {
    pub generated_at: chrono::DateTime<chrono::Utc>,
    /// Signals whose event has not been resolved.
    pub active_signals: usize,
    pub signals_total: usize,
    pub events_total: usize,
    pub observations_total: usize,
    /// Active signals per signal type, so the header can show the mix.
    pub by_type: Vec<TypeCount>,
    /// Active signals per lifecycle status.
    pub by_status: Vec<StatusCount>,
    /// The freshest active signals, best first — the "NOW" strip.
    pub now: Vec<wse_model::Signal>,
    /// How many sources are connected and how many are currently healthy.
    pub sources_total: usize,
    pub sources_healthy: usize,
    /// Whether the world is being watched right now. `false` means the page is
    /// a static snapshot, and the UI must say so rather than imply monitoring.
    pub monitoring: bool,
    /// Whether a collection loop exists at all, paused or not.
    pub collector_active: bool,
    /// Whether collection is paused by an operator.
    pub collection_enabled: bool,
    /// Per-source health, so the World header can show coverage honestly
    /// without a second request.
    pub sources: Vec<SourceStatus>,
    /// When the most recent observation from any source arrived, and how stale
    /// that is. `None` until the first collection.
    pub last_collection_at: Option<chrono::DateTime<chrono::Utc>>,
    pub data_age_seconds: Option<i64>,
    /// Real latency telemetry, the same figures `/control` reports.
    pub latency: LatencySummary,
}

/// A source's health as the World header needs it.
#[derive(Debug, Serialize)]
pub struct SourceStatus {
    pub source_id: String,
    pub name: String,
    pub category: String,
    /// `healthy`, `degraded`, `down`, `rate_limited`, `unknown`.
    pub status: String,
    /// Seconds since the last successful collection, when there has been one.
    pub age_seconds: Option<i64>,
    /// Whether the source has never produced a successful collection yet.
    pub never_collected: bool,
}

#[derive(Debug, Serialize)]
pub struct TypeCount {
    #[serde(rename = "type")]
    pub signal_type: String,
    pub count: usize,
}

#[derive(Debug, Serialize)]
pub struct StatusCount {
    pub status: String,
    pub count: usize,
}

/// `GET /world`
///
/// A composed view rather than a new store: it reads the same signals the feed
/// does, so it cannot drift from them.
pub async fn world<S: wse_storage::Store>(State(state): State<AppState<S>>) -> Response {
    let engine = state.read().await;
    let store = engine.store();

    let all = match store.query_signals(&SignalQuery::default()) {
        Ok(page) => page.items,
        Err(err) => return internal(err),
    };
    let active = match store.query_signals(&SignalQuery::active()) {
        Ok(page) => page.items,
        Err(err) => return internal(err),
    };

    let type_order = [
        SignalType::Now,
        SignalType::Anomaly,
        SignalType::EarlySignal,
        SignalType::Convergence,
        SignalType::Impact,
    ];
    let by_type = type_order
        .iter()
        .map(|ty| TypeCount {
            signal_type: ty.as_str().to_string(),
            count: active.iter().filter(|s| s.types.contains(ty)).count(),
        })
        .collect();

    let status_order = [
        wse_model::SignalStatus::New,
        wse_model::SignalStatus::Developing,
        wse_model::SignalStatus::Confirmed,
        wse_model::SignalStatus::Stable,
        wse_model::SignalStatus::Fading,
        wse_model::SignalStatus::Resolved,
    ];
    let by_status = status_order
        .iter()
        .map(|st| StatusCount {
            status: st.as_str().to_string(),
            count: all.iter().filter(|s| s.status == *st).count(),
        })
        .collect();

    let sources = store.all_sources().unwrap_or_default();
    let schedules = engine.runtime().schedules();
    let now = chrono::Utc::now();

    // Per-source health, rendered from the real health record rather than the
    // schedule alone: "never collected" and "collected an hour ago" are
    // different, and neither is the same as "the world is quiet".
    let source_status: Vec<SourceStatus> = sources
        .iter()
        .map(|source| {
            let id = source.id.as_str().to_string();
            let health = store.get_health(&source.id).ok().flatten();
            let status = health
                .as_ref()
                .map(|h| h.status)
                .unwrap_or(wse_model::HealthStatus::Unknown);
            let last_success = health.as_ref().and_then(|h| h.last_success);
            SourceStatus {
                source_id: id,
                name: source.name.clone(),
                category: source.category.clone(),
                status: status.as_str().to_string(),
                age_seconds: last_success.map(|at| (now - at).num_seconds()),
                never_collected: last_success.is_none(),
            }
        })
        .collect();

    let sources_healthy = source_status
        .iter()
        .filter(|s| s.status == "healthy")
        .count();

    // Data freshness: when did the newest observation actually arrive?
    let last_collection_at = schedules
        .values()
        .filter_map(|s| s.last_success)
        .max()
        .or_else(|| {
            store
                .query_observations(&ObservationQuery {
                    limit: Some(1),
                    ..Default::default()
                })
                .ok()
                .and_then(|page| page.items.into_iter().map(|o| o.received_at).max())
        });
    let data_age_seconds = last_collection_at.map(|at| (now - at).num_seconds());

    // The store already orders by rank then recency; the strip shows the head.
    let now_strip: Vec<wse_model::Signal> = active.iter().take(8).cloned().collect();

    Json(WorldSummary {
        generated_at: now,
        active_signals: active.len(),
        signals_total: all.len(),
        events_total: store.event_count().unwrap_or(0),
        observations_total: store.observation_count().unwrap_or(0),
        by_type,
        by_status,
        now: now_strip,
        sources_total: sources.len(),
        sources_healthy,
        monitoring: engine.runtime().monitoring(),
        collector_active: engine.runtime().collector_active(),
        collection_enabled: engine.runtime().collection_enabled(),
        sources: source_status,
        last_collection_at,
        data_age_seconds,
        latency: latency_summary(&engine),
    })
    .into_response()
}

/* ------------------------------------------------------- OBSERVATORY -- */

/// The categories the observatory board shows, in priority order.
///
/// Ordered by how fast a change in each matters to a human watching the world:
/// the ground and the sky first, then the systems people depend on. The board
/// only shows categories that some source actually emits — an entry here with no
/// source behind it would be a permanently empty card, which is noise, so
/// [`observatory`] intersects this list with the catalog.
pub const OBSERVATORY_CATEGORIES: &[&str] = &[
    "geophysics",
    "space",
    "weather",
    "environment",
    "disasters",
    "global_events",
    "cyber",
    "technology",
    "markets",
    "health",
    "science",
];

/// The board's category list: the preferred order, restricted to categories the
/// catalog actually populates, with any remaining catalog category appended so a
/// newly added source's category appears on the board without a code change.
pub fn observatory_categories(sources: &[wse_model::Source]) -> Vec<String> {
    let present: std::collections::BTreeSet<&str> =
        sources.iter().map(|s| s.category.as_str()).collect();
    let mut ordered: Vec<String> = OBSERVATORY_CATEGORIES
        .iter()
        .filter(|c| present.contains(**c))
        .map(|c| c.to_string())
        .collect();
    for category in present {
        if !ordered.iter().any(|c| c == category) {
            ordered.push(category.to_string());
        }
    }
    ordered
}

/// `GET /observatory?window=24h|7d`
///
/// The single-screen control room's whole board in one response: per-category
/// rollups with their real series, a severity-ordered feed, the breaking ticker,
/// and the one alert worth interrupting for.
///
/// A composed read over the existing stores, like `/world`; it cannot drift from
/// the feed because it reads the same signals.
#[derive(Debug, Deserialize)]
pub struct ObservatoryParams {
    /// `24h` (default) or `7d`; the activity chart's window.
    pub window: Option<String>,
}

pub async fn observatory<S: wse_storage::Store>(
    State(state): State<AppState<S>>,
    Query(params): Query<ObservatoryParams>,
) -> Response {
    let engine = state.read().await;
    let window = crate::observatory::ActivityWindow::parse(params.window.as_deref());
    let sources = engine.store().all_sources().unwrap_or_default();
    let categories = observatory_categories(&sources);
    let order: Vec<&str> = categories.iter().map(String::as_str).collect();
    let snapshot = crate::observatory::build_snapshot(&engine, &order, &window, chrono::Utc::now());
    Json(snapshot).into_response()
}

/* ------------------------------------------------------------- CONTROL -- */

/// Build the Control screen's snapshot from real state: the store's counts and
/// disk usage plus the runtime's collection switch and schedules.
///
/// Nothing here is hard-coded. A field the Control screen shows and this does
/// not compute is a bug, not a placeholder.
pub fn control_snapshot<S: wse_storage::Store>(
    engine: &wse_engine::Engine<S>,
    version: &'static str,
) -> ControlSnapshot {
    let runtime = engine.runtime();
    let store = engine.store();

    let sources = store.all_sources().unwrap_or_default();
    let schedules = runtime.schedules();
    let sources: Vec<SourceControl> = sources
        .into_iter()
        .map(|source| {
            let id = source.id.as_str().to_string();
            let schedule = schedules.get(&id).cloned().unwrap_or_default();
            SourceControl {
                enabled: runtime.is_source_enabled(&id),
                running: runtime.is_running(&id),
                source_id: id,
                name: source.name,
                category: source.category,
                schedule,
            }
        })
        .collect();

    let signals_total = store.signal_count().unwrap_or(0);
    let active_query = SignalQuery {
        active_only: true,
        ..Default::default()
    };
    let signals_active = store
        .query_signals(&active_query)
        .map(|page| page.total)
        .unwrap_or(0);

    let disk = match MaintenanceStore::disk_usage(store) {
        Ok(usage) => DiskSummary {
            database_bytes: usage.db_bytes,
            raw_bytes: usage.raw_bytes,
            raw_files: usage.raw_files,
            total_bytes: usage.total_bytes(),
        },
        Err(_) => DiskSummary {
            database_bytes: 0,
            raw_bytes: 0,
            raw_files: 0,
            total_bytes: 0,
        },
    };

    let latency = latency_summary(engine);

    ControlSnapshot {
        status: "live",
        version,
        started_at: runtime.started_at(),
        uptime_seconds: runtime.uptime_seconds(),
        collector_active: runtime.collector_active(),
        collection_enabled: runtime.collection_enabled(),
        monitoring: runtime.monitoring(),
        sources,
        signals_active,
        signals_total,
        observations_total: store.observation_count().unwrap_or(0),
        events_total: store.event_count().unwrap_or(0),
        disk,
        latency,
    }
}

/// Compute the latency telemetry from what is actually stored.
///
/// Nothing is estimated. Each figure is derived from timestamps the pipeline
/// already records, so it can be checked against the raw data.
fn latency_summary<S: wse_storage::Store>(engine: &wse_engine::Engine<S>) -> LatencySummary {
    let store = engine.store();
    let now = chrono::Utc::now();

    // Source-side lag over the most recent observations.
    let observation_lag_ms = store
        .query_observations(&ObservationQuery {
            limit: Some(200),
            ..Default::default()
        })
        .ok()
        .filter(|page| !page.items.is_empty())
        .map(|page| {
            let total: i64 = page.items.iter().map(|o| o.lag_ms()).sum();
            total / page.items.len() as i64
        });

    // Detection latency: signal formation versus the evidence's own timestamp.
    let signals = store
        .query_signals(&SignalQuery::default())
        .ok()
        .map(|page| page.items)
        .unwrap_or_default();

    let detection_ms = {
        let mut latencies: Vec<i64> = signals
            .iter()
            .filter_map(|s| {
                s.evidence
                    .iter()
                    .map(|e| e.observed_at)
                    .min()
                    .map(|earliest| (s.first_seen - earliest).num_milliseconds())
            })
            .collect();
        if latencies.is_empty() {
            None
        } else {
            latencies.sort_unstable();
            Some(latencies[latencies.len() / 2])
        }
    };

    let newest_signal_age_ms = signals
        .iter()
        .map(|s| s.first_seen)
        .max()
        .map(|newest| (now - newest).num_milliseconds());

    LatencySummary {
        observation_lag_ms,
        collector_ms: engine.metrics().collector_latency_ms,
        detection_ms,
        newest_signal_age_ms,
    }
}

/// `GET /control`
///
/// The engine's real operational state. Authenticated like every other API
/// route, because it exposes source schedules and storage footprint.
pub async fn control<S: wse_storage::Store>(State(state): State<AppState<S>>) -> Response {
    let engine = state.read().await;
    Json(control_snapshot(&engine, env!("CARGO_PKG_VERSION"))).into_response()
}

#[derive(Debug, Deserialize)]
pub struct CollectionBody {
    pub enabled: bool,
}

/// `POST /control/collection` — pause or resume continuous collection.
pub async fn set_collection<S: wse_storage::Store>(
    State(state): State<AppState<S>>,
    Json(body): Json<CollectionBody>,
) -> Response {
    let engine = state.read().await;
    engine.runtime().set_collection_enabled(body.enabled);
    Json(serde_json::json!({ "collection_enabled": body.enabled })).into_response()
}

#[derive(Debug, Deserialize)]
pub struct SourceEnabledBody {
    pub enabled: bool,
}

/// `POST /sources/:id/enabled` — enable or disable one source.
pub async fn set_source_enabled<S: wse_storage::Store>(
    State(state): State<AppState<S>>,
    Path(id): Path<String>,
    Json(body): Json<SourceEnabledBody>,
) -> Response {
    let engine = state.read().await;
    // Refuse an id that is not in the catalog: enabling a source that does not
    // exist would silently do nothing and look like it worked.
    match engine.store().get_source(&SourceId::new(id.clone())) {
        Ok(Some(_)) => {
            engine.runtime().set_source_enabled(&id, body.enabled);
            Json(serde_json::json!({ "source_id": id, "enabled": body.enabled })).into_response()
        }
        Ok(None) => not_found(format!("source {id}")),
        Err(err) => internal(err),
    }
}

/// `POST /sources/:id/run` — run one source at the next scheduler pass.
///
/// Returns `202 Accepted`: the run is queued, not performed here. The scheduler
/// coalesces duplicate requests and refuses to start a source that is already
/// running, so this cannot overlap the scheduled run or double-poll a source.
pub async fn run_source<S: wse_storage::Store>(
    State(state): State<AppState<S>>,
    Path(id): Path<String>,
) -> Response {
    let engine = state.read().await;
    match engine.store().get_source(&SourceId::new(id.clone())) {
        Ok(Some(_)) => {
            if engine.runtime().is_running(&id) {
                return (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({
                        "error": "source is already running",
                        "source_id": id,
                    })),
                )
                    .into_response();
            }
            engine.runtime().request_run_now(&id);
            (
                StatusCode::ACCEPTED,
                Json(serde_json::json!({ "queued": true, "source_id": id })),
            )
                .into_response()
        }
        Ok(None) => not_found(format!("source {id}")),
        Err(err) => internal(err),
    }
}

/// `GET /activity` — the recent activity stream, newest first.
pub async fn activity<S: wse_storage::Store>(
    State(state): State<AppState<S>>,
    Query(params): Query<ActivityParams>,
) -> Response {
    let engine = state.read().await;
    let limit = params
        .limit
        .unwrap_or(50)
        .min(wse_engine::runtime::ACTIVITY_LIMIT);
    Json(serde_json::json!({
        "items": engine.runtime().recent_activity(limit),
    }))
    .into_response()
}

#[derive(Debug, Deserialize)]
pub struct ActivityParams {
    pub limit: Option<usize>,
}

/// `GET /events` (SSE) — the live activity stream.
///
/// Server-Sent Events rather than WebSockets: the flow is one-way
/// (server→browser), SSE reconnects automatically, and it rides plain HTTP so
/// the existing auth middleware, reverse proxy and TLS story are unchanged.
///
/// Each event carries an `id` (the activity timestamp in nanos) so a
/// reconnecting browser can send `Last-Event-ID`; the client uses it to drop
/// anything it has already seen, which is what keeps reconnect from duplicating
/// entries. A periodic comment line keeps proxies from closing an idle stream,
/// and the client treats a gap in keep-alives as "connection lost".
pub async fn events<S: wse_storage::Store>(
    State(state): State<AppState<S>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let runtime = {
        let engine = state.read().await;
        engine.runtime().clone()
    };
    let receiver = runtime.subscribe();

    let stream = stream::unfold(receiver, |mut receiver| async move {
        loop {
            match receiver.recv().await {
                Ok(activity) => {
                    let id = activity
                        .at
                        .timestamp_nanos_opt()
                        .unwrap_or_default()
                        .to_string();
                    let data = serde_json::to_string(&activity).unwrap_or_else(|_| "{}".into());
                    let event = Event::default()
                        .id(id)
                        .event(event_name(activity.kind))
                        .data(data);
                    return Some((Ok(event), receiver));
                }
                // A lagging subscriber missed a burst. Skipping is correct: the
                // UI re-syncs its full state on the next event and on reconnect,
                // so replaying a stale backlog would only add duplicates.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    });

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}

fn event_name(kind: ActivityKind) -> &'static str {
    match kind {
        ActivityKind::Started => "started",
        ActivityKind::Observation => "observation",
        ActivityKind::Anomaly => "anomaly",
        ActivityKind::Event => "event",
        ActivityKind::Signal => "signal",
        ActivityKind::SourceRecovered => "source_recovered",
        ActivityKind::SourceFailed => "source_failed",
        ActivityKind::SourceRateLimited => "source_rate_limited",
        ActivityKind::Control => "control",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_type_parsing_is_case_insensitive() {
        assert_eq!(parse_signal_type("anomaly"), Some(SignalType::Anomaly));
        assert_eq!(
            parse_signal_type("EARLY_SIGNAL"),
            Some(SignalType::EarlySignal)
        );
        assert_eq!(parse_signal_type("nope"), None);
    }

    #[test]
    fn entity_id_is_passthrough() {
        assert_eq!(entity_id("ent_x").as_str(), "ent_x");
    }

    #[test]
    fn cadences_render_in_the_units_a_reader_thinks_in() {
        assert_eq!(cadence_label(Cadence::Interval { seconds: 900 }), "15m");
        assert_eq!(cadence_label(Cadence::Interval { seconds: 3_600 }), "1h");
        assert_eq!(cadence_label(Cadence::Interval { seconds: 86_400 }), "1d");
        assert_eq!(cadence_label(Cadence::Interval { seconds: 45 }), "45s");
        // 90 minutes is not a whole number of hours, so it stays in minutes.
        assert_eq!(cadence_label(Cadence::Interval { seconds: 5_400 }), "90m");
        assert_eq!(cadence_label(Cadence::Event), "event");
        assert_eq!(cadence_label(Cadence::Daily { hour_utc: 6 }), "daily@06Z");
        assert_eq!(cadence_label(Cadence::Irregular), "irregular");
    }

    #[test]
    fn an_observation_view_exposes_series_key_and_lag() {
        use wse_model::{Observation, RawReference, SourceId};
        let observed_at = chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let mut observation = Observation::new(
            SourceId::new("src_a"),
            None,
            "temperature",
            21.5,
            "celsius",
            observed_at,
            RawReference::new("raw/a", "hash"),
        );
        observation.received_at = observed_at + chrono::Duration::seconds(90);

        let view = ObservationView::new(observation);
        assert_eq!(view.lag_ms, 90_000);
        assert_eq!(view.series_key, "src_a::-::temperature::celsius");
    }
}
