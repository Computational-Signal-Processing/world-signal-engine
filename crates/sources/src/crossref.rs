//! Crossref — scholarly output as a **stable count** per fixed topic.
//!
//! Crossref's `/works` endpoint returns `total-results` for a query, i.e. how
//! many works match. Used with a fixed time window it becomes a stable series:
//! "how many works were registered on day X matching topic Y". The query and
//! the window are fixed; only the window's contents change, which is exactly
//! the world change we want to measure.
//!
//! The window is a **single, non-overlapping day** — the day before collection.
//! A trailing multi-day window sampled daily would share days between
//! consecutive polls (autocorrelated series), and a window ending *today* would
//! count a day whose deposits are still arriving (the newest point structurally
//! depressed). Both are avoided by measuring one completed day at a time. See
//! `docs/decisions/0019-crossref-single-day-window.md`.
//!
//! This is the SCIENCE lens's institutional backbone and, via the AI topic, the
//! AI lens's research-velocity sensor.
//!
//! Docs: <https://api.crossref.org/swagger-ui/index.html>
//!
//! Authentication: none for polite use. Crossref asks for a `mailto` so heavy
//! users can be contacted; the collector sends one from `CROSSREF_MAILTO` when
//! set. License: Crossref REST API terms; metadata is open.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use wse_collector::CollectorError;
use wse_model::{EntityId, Observation, RawReference, Source, SourceId};

pub const SOURCE_ID: &str = "crossref_works";
pub const COLLECTOR_TYPE: &str = "crossref_works_count";

/// Works search endpoint.
pub const API_ENDPOINT: &str = "https://api.crossref.org/works";

/// The tracked topics: `(slug, human label, bibliographic query)`.
///
/// A fixed list, re-measured every collection. Each is a count of works
/// registered in the recent window matching the query.
pub const TOPICS: &[(&str, &str, &str)] = &[
    (
        "artificial_intelligence",
        "artificial intelligence",
        "artificial intelligence",
    ),
    ("machine_learning", "machine learning", "machine learning"),
    ("climate_change", "climate change", "climate change"),
    ("crispr", "CRISPR", "crispr gene editing"),
    (
        "quantum_computing",
        "quantum computing",
        "quantum computing",
    ),
];

/// The measurement window, in days. One completed day per poll: the day is the
/// record, so consecutive polls never overlap and today's still-arriving
/// deposits are never counted.
pub const WINDOW_DAYS: i64 = 1;

/// The catalog entry.
pub fn source() -> Source {
    let mut parameters = BTreeMap::new();
    parameters.insert("window_days".to_string(), WINDOW_DAYS.to_string());
    parameters.insert(
        "topics".to_string(),
        TOPICS.iter().map(|t| t.0).collect::<Vec<_>>().join(","),
    );

    Source {
        id: SourceId::new(SOURCE_ID),
        name: "Crossref Scholarly Works".to_string(),
        provider: "Crossref".to_string(),
        category: "science".to_string(),
        subcategory: Some("publication_velocity".to_string()),
        endpoint: API_ENDPOINT.to_string(),
        protocol: wse_model::Protocol::Https,
        format: wse_model::DataFormat::Json,
        cadence: wse_model::Cadence::Interval { seconds: 86_400 },
        timezone: Some("UTC".to_string()),
        license: Some("Crossref REST API terms; metadata is open".to_string()),
        authentication: wse_model::AuthKind::None,
        cost: wse_model::Cost::Free,
        historical_available: true,
        realtime_available: false,
        geospatial: false,
        entities: vec!["research".to_string()],
        priority: 45,
        enabled: true,
        collector_type: COLLECTOR_TYPE.to_string(),
        parameters,
        tier: wse_model::SourceTier::Tier2,
        measurement: wse_model::MeasurementSemantics::StableSeries,
        feeds_lenses: vec!["lens_science".to_string(), "lens_ai".to_string()],
        derivations: Vec::new(),
    }
}

/// The `message` envelope of a works response.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Message {
    #[serde(default, rename = "total-results")]
    pub total_results: u64,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct WorksResponse {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub message: Message,
}

/// Build the works URL for one topic and window.
pub fn works_url(query: &str, from: &str, until: &str, mailto: Option<&str>) -> String {
    let q = query.replace(' ', "+");
    let mut url = format!(
        "{API_ENDPOINT}?rows=0&query.bibliographic={q}&filter=from-created-date:{from},until-created-date:{until}"
    );
    if let Some(mailto) = mailto {
        url.push_str(&format!("&mailto={mailto}"));
    }
    url
}

/// Parse a works response into the total count.
pub fn parse_count(body: &[u8]) -> Result<u64, CollectorError> {
    let response: WorksResponse = serde_json::from_slice(body)
        .map_err(|e| CollectorError::Parse(format!("crossref works: {e}")))?;
    if response.status != "ok" {
        return Err(CollectorError::Parse(format!(
            "crossref returned status {:?}",
            response.status
        )));
    }
    Ok(response.message.total_results)
}

/// Turn one topic's count into an observation.
///
/// The metric is a count of works registered on the measured day; the topic is
/// the entity, so each topic is its own series and the AI topics can be told
/// apart. `received_at` is the collection time; the observation's `observed_at`
/// is the measured day's UTC midnight, so a re-poll of the same day yields the
/// same id and is de-duplicated.
pub fn observation_for(
    slug: &str,
    label: &str,
    count: u64,
    received_at: DateTime<Utc>,
    body: &[u8],
) -> Observation {
    let source_id = SourceId::new(SOURCE_ID);
    let entity = EntityId::new(format!("research_{slug}"));
    let hash = wse_model::fnv1a_hex(&String::from_utf8_lossy(body));
    let raw = RawReference {
        locator: format!("{API_ENDPOINT}?query.bibliographic={slug}"),
        hash,
        content_type: Some("application/json".to_string()),
        bytes: Some(body.len() as u64),
    };
    let day = window(received_at).0;
    let observed_at = NaiveDate::parse_from_str(&day, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|d| Utc.from_utc_datetime(&d))
        .unwrap_or(received_at);
    Observation::new(
        source_id,
        Some(entity),
        "works_registered",
        count as f64,
        "works",
        observed_at,
        raw,
    )
    .with_received_at(received_at)
    // The measured day is the record, so a day re-polled keeps its identity.
    .with_record_key(day.clone())
    .with_attribute("day", day)
    .with_attribute("topic", label.to_string())
    .with_attribute("window_days", WINDOW_DAYS.to_string())
}

/// The window `(from, until)` for a collection at `now`.
///
/// One completed day: the day before collection. `until` is exclusive in the
/// Crossref filter, so `[from, until)` is exactly that single day, and it is
/// the same window for every poll within the collection day.
pub fn window(now: DateTime<Utc>) -> (String, String) {
    let day = now - Duration::days(WINDOW_DAYS);
    let day = day.format("%Y-%m-%d").to_string();
    (day.clone(), day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn received() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn parses_the_total_count() {
        let body =
            br#"{"status":"ok","message-type":"work-list","message":{"total-results":24805}}"#;
        assert_eq!(parse_count(body).unwrap(), 24805);
    }

    #[test]
    fn a_non_ok_status_is_a_parse_error() {
        let body = br#"{"status":"failed","message":{"total-results":0}}"#;
        assert!(matches!(parse_count(body), Err(CollectorError::Parse(_))));
    }

    #[test]
    fn malformed_payload_is_a_parse_error() {
        assert!(parse_count(b"nope").is_err());
    }

    #[test]
    fn the_url_carries_the_window_and_topic() {
        let url = works_url("quantum computing", "2026-09-01", "2026-09-30", None);
        assert!(url.contains("query.bibliographic=quantum+computing"));
        assert!(url.contains("from-created-date:2026-09-01"));
        assert!(url.contains("until-created-date:2026-09-30"));
        assert!(url.contains("rows=0"));
    }

    #[test]
    fn the_window_is_a_single_completed_day() {
        // A collection at any time on 2026-10-01 measures 2026-09-30, and only
        // that day: `from == until` (Crossref's `until` is exclusive), so
        // consecutive daily polls never share a day and today's still-arriving
        // deposits are never counted.
        let morning = DateTime::parse_from_rfc3339("2026-10-01T06:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let evening = DateTime::parse_from_rfc3339("2026-10-01T23:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            window(morning),
            ("2026-09-30".to_string(), "2026-09-30".to_string())
        );
        assert_eq!(
            window(evening),
            ("2026-09-30".to_string(), "2026-09-30".to_string()),
            "the measured day does not move within a collection day"
        );
    }

    #[test]
    fn a_re_poll_of_the_same_day_keeps_its_id() {
        let morning = DateTime::parse_from_rfc3339("2026-10-01T06:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let evening = DateTime::parse_from_rfc3339("2026-10-01T23:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let a = observation_for("crispr", "CRISPR", 100, morning, b"{}");
        let b = observation_for("crispr", "CRISPR", 100, evening, b"{}");
        assert_eq!(a.id, b.id, "the same measured day keeps one identity");
        // The next collection day measures a different day, a new record.
        let next = DateTime::parse_from_rfc3339("2026-10-02T06:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let c = observation_for("crispr", "CRISPR", 100, next, b"{}");
        assert_ne!(a.id, c.id);
    }

    #[test]
    fn the_mailto_is_optional_and_never_in_the_body() {
        let with = works_url("ai", "a", "b", Some("me@example.com"));
        assert!(with.contains("mailto=me@example.com"));
        let without = works_url("ai", "a", "b", None);
        assert!(!without.contains("mailto"));
    }

    #[test]
    fn each_topic_is_its_own_series() {
        let ai = observation_for(
            "artificial_intelligence",
            "artificial intelligence",
            100,
            received(),
            b"{}",
        );
        let ml = observation_for(
            "machine_learning",
            "machine learning",
            200,
            received(),
            b"{}",
        );
        assert_ne!(ai.series_key(), ml.series_key());
        assert!(ai
            .entity_id
            .as_ref()
            .unwrap()
            .as_str()
            .contains("artificial_intelligence"));
        assert_eq!(ai.metric, "works_registered");
    }

    #[test]
    fn the_catalog_is_institutional_science() {
        let source = source();
        assert_eq!(source.category, "science");
        assert_eq!(source.tier, wse_model::SourceTier::Tier2);
        assert!(source.measurement.is_comparable());
        assert!(source.feeds_lenses.contains(&"lens_science".to_string()));
        assert!(source.feeds_lenses.contains(&"lens_ai".to_string()));
    }
}
