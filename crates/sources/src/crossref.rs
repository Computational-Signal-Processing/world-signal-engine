//! Crossref — scholarly output as a **stable count** per fixed topic.
//!
//! Crossref's `/works` endpoint returns `total-results` for a query, i.e. how
//! many works match. Used with a fixed time window it becomes a stable series:
//! "how many works were registered in the last day matching topic X". The
//! query and the window are fixed; only the window's contents change, which is
//! exactly the world change we want to measure.
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

use chrono::{DateTime, Duration, Utc};
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

/// The measurement window, in days, ending at collection time.
pub const WINDOW_DAYS: i64 = 2;

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
/// The metric is a count of works registered in the window; the topic is the
/// entity, so each topic is its own series and the AI topics can be told apart.
pub fn observation_for(
    slug: &str,
    label: &str,
    count: u64,
    observed_at: DateTime<Utc>,
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
    Observation::new(
        source_id,
        Some(entity),
        "works_registered",
        count as f64,
        "works",
        observed_at,
        raw,
    )
    .with_received_at(observed_at)
    .with_attribute("topic", label.to_string())
    .with_attribute("window_days", WINDOW_DAYS.to_string())
}

/// The default window `(from, until)` for a collection at `now`.
pub fn window(now: DateTime<Utc>) -> (String, String) {
    let from = now - Duration::days(WINDOW_DAYS);
    (
        from.format("%Y-%m-%d").to_string(),
        now.format("%Y-%m-%d").to_string(),
    )
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
