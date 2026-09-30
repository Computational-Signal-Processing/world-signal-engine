//! Observations: the atomic measurement the whole engine is built on.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{EntityId, ObservationId, SourceId};
use crate::quality::Quality;

/// A pointer back to the untouched source payload.
///
/// Drill-down (`SIGNAL -> EVENT -> OBSERVATION -> SOURCE -> RAW DATA`) depends
/// on this. The raw payload may live on disk, in object storage, or simply be
/// re-fetchable from the endpoint; the engine only needs a stable reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawReference {
    /// Where the raw payload can be found (file path, object key, or URL).
    pub locator: String,
    /// Fingerprint of the payload, used for duplicate detection.
    pub hash: String,
    pub content_type: Option<String>,
    pub bytes: Option<u64>,
}

impl RawReference {
    pub fn new(locator: impl Into<String>, hash: impl Into<String>) -> Self {
        Self {
            locator: locator.into(),
            hash: hash.into(),
            content_type: None,
            bytes: None,
        }
    }
}

/// A single normalized measurement from a source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    pub id: ObservationId,
    pub source_id: SourceId,
    /// When the world produced this value, per the source.
    pub observed_at: DateTime<Utc>,
    /// When we received it. `observed_at != received_at` reveals lag.
    pub received_at: DateTime<Utc>,
    pub entity_id: Option<EntityId>,
    pub metric: String,
    pub value: f64,
    pub unit: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub quality: Quality,
    pub raw: RawReference,
    /// Extra grouping dimensions, e.g. `{"station": "Kadikoy"}`.
    pub dimensions: BTreeMap<String, String>,
    /// Additional non-numeric attributes from the source record.
    pub attributes: BTreeMap<String, String>,
    /// Per-record discriminator within the series, e.g. a repository name or a
    /// story id.
    ///
    /// Some sources emit several independent records per series *per
    /// timestamp* — GitHub search returns many repositories, Hacker News many
    /// stories. Without a discriminator those records share an id (same series,
    /// same `observed_at`, same payload hash) and de-duplication silently drops
    /// all but one. `identity` is that discriminator: it is part of the
    /// observation's id and **never** part of its [`series_key`](Self::series_key),
    /// so the records stay independently observable while still forming one
    /// series for baseline and detection.
    pub identity: Option<String>,
}

impl Observation {
    /// Build an observation with a deterministic id derived from its series
    /// key, timestamp and payload hash.
    pub fn new(
        source_id: SourceId,
        entity_id: Option<EntityId>,
        metric: impl Into<String>,
        value: f64,
        unit: impl Into<String>,
        observed_at: DateTime<Utc>,
        raw: RawReference,
    ) -> Self {
        let metric = metric.into();
        let unit = unit.into();
        let entity_key = entity_id.as_ref().map(|e| e.as_str()).unwrap_or("-");
        let key = series_key(&source_id, entity_key, &metric, &unit);
        // No `identity` yet: the id is finalized by `with_identity`, which is
        // what a source with several records per timestamp calls.
        let id = ObservationId::deterministic(&key, &observed_at.to_rfc3339(), &raw.hash, None);
        Self {
            id,
            source_id,
            observed_at,
            received_at: Utc::now(),
            entity_id,
            metric,
            value,
            unit,
            latitude: None,
            longitude: None,
            quality: Quality::pristine(),
            raw,
            dimensions: BTreeMap::new(),
            attributes: BTreeMap::new(),
            identity: None,
        }
    }

    pub fn with_location(mut self, latitude: f64, longitude: f64) -> Self {
        self.latitude = Some(latitude);
        self.longitude = Some(longitude);
        self
    }

    /// Set the per-record discriminator and re-derive the deterministic id.
    ///
    /// Call this for any source that emits more than one record per series per
    /// timestamp. The discriminator must be stable for the same record across
    /// collections (a repository name, a story id), so that re-collecting an
    /// unchanged payload still produces the same id and is de-duplicated.
    pub fn with_identity(mut self, identity: impl Into<String>) -> Self {
        let identity = identity.into();
        let entity_key = self.entity_id.as_ref().map(|e| e.as_str()).unwrap_or("-");
        let key = series_key(&self.source_id, entity_key, &self.metric, &self.unit);
        self.id = ObservationId::deterministic(
            &key,
            &self.observed_at.to_rfc3339(),
            &self.raw.hash,
            Some(&identity),
        );
        self.identity = Some(identity);
        self
    }

    /// Override the receipt timestamp.
    ///
    /// Live collection uses `Utc::now()`; replay and synthetic data set this
    /// explicitly so that lag and clock-drift behaviour are reproducible.
    pub fn with_received_at(mut self, received_at: DateTime<Utc>) -> Self {
        self.received_at = received_at;
        self
    }

    pub fn with_quality(mut self, quality: Quality) -> Self {
        self.quality = quality;
        self
    }

    pub fn with_dimension(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.dimensions.insert(key.into(), value.into());
        self
    }

    /// Free-form context that is **not** part of the series identity.
    ///
    /// Dimensions are folded into [`series_key`](Self::series_key), so a
    /// per-record dimension would give every record its own one-point series
    /// and nothing could ever be compared. Per-record detail — a story title,
    /// a repository name, an event id — belongs here instead: it is preserved
    /// for drill-down but keeps the series intact.
    pub fn with_attribute(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(key.into(), value.into());
        self
    }

    /// The identity of the time series this observation belongs to.
    ///
    /// Baseline and detection operate on series, not on individual points.
    pub fn series_key(&self) -> String {
        let entity_key = self.entity_id.as_ref().map(|e| e.as_str()).unwrap_or("-");
        let mut key = series_key(&self.source_id, entity_key, &self.metric, &self.unit);
        for (k, v) in &self.dimensions {
            key.push('|');
            key.push_str(k);
            key.push('=');
            key.push_str(v);
        }
        key
    }

    /// Source-side lag between observation and receipt.
    pub fn lag_ms(&self) -> i64 {
        (self.received_at - self.observed_at).num_milliseconds()
    }
}

/// Canonical series key: `source::entity::metric::unit`.
pub fn series_key(source_id: &SourceId, entity: &str, metric: &str, unit: &str) -> String {
    format!("{}::{}::{}::{}", source_id.as_str(), entity, metric, unit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::EntityId;

    fn ts(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn obs(value: f64, hash: &str) -> Observation {
        Observation::new(
            SourceId::new("src_a"),
            Some(EntityId::new("ent_x")),
            "temperature",
            value,
            "celsius",
            ts("2026-01-01T00:00:00Z"),
            RawReference::new("raw/a.json", hash),
        )
    }

    #[test]
    fn identical_payloads_produce_identical_ids() {
        assert_eq!(obs(1.0, "h1").id, obs(1.0, "h1").id);
    }

    #[test]
    fn different_payload_hash_produces_different_id() {
        assert_ne!(obs(1.0, "h1").id, obs(1.0, "h2").id);
    }

    #[test]
    fn series_key_includes_dimensions() {
        let a = obs(1.0, "h").with_dimension("station", "kadikoy");
        let b = obs(1.0, "h").with_dimension("station", "besiktas");
        assert_ne!(a.series_key(), b.series_key());
        assert!(a
            .series_key()
            .starts_with("src_a::ent_x::temperature::celsius"));
    }

    #[test]
    fn lag_is_measured_from_timestamps() {
        let mut o = obs(1.0, "h");
        o.observed_at = ts("2026-01-01T00:00:00Z");
        o.received_at = ts("2026-01-01T00:00:02Z");
        assert_eq!(o.lag_ms(), 2000);
    }

    #[test]
    fn observation_round_trips_through_serde() {
        let o = obs(42.5, "h").with_location(41.0, 29.0);
        let json = serde_json::to_string(&o).unwrap();
        let back: Observation = serde_json::from_str(&json).unwrap();
        assert_eq!(back, o);
    }

    #[test]
    fn identity_separates_records_but_keeps_the_series_intact() {
        // The GitHub/HN shape: many records, one series, one timestamp.
        let a = obs(10.0, "h").with_identity("repo_a");
        let b = obs(20.0, "h").with_identity("repo_b");
        assert_ne!(a.id, b.id, "independent records must not collide");
        assert_eq!(
            a.series_key(),
            b.series_key(),
            "identity must never leak into the series key"
        );
    }

    #[test]
    fn identity_is_stable_across_collections() {
        // The same record collected twice keeps its id, so it de-duplicates.
        assert_eq!(
            obs(10.0, "h").with_identity("repo_a").id,
            obs(10.0, "h").with_identity("repo_a").id
        );
    }

    #[test]
    fn records_without_an_identity_still_collide() {
        // Backwards compatibility: a single-record source is unchanged.
        assert_eq!(obs(10.0, "h").id, obs(10.0, "h").id);
        assert!(obs(10.0, "h").identity.is_none());
    }
}
