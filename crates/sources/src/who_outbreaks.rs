//! WHO Disease Outbreak News — official outbreak announcements.
//!
//! The World Health Organization publishes Disease Outbreak News (DON): official
//! statements about outbreaks of international concern (Ebola, cholera, measles,
//! novel influenza, ...). It is the authoritative health signal, independent of
//! any news proxy.
//!
//! The measurement is the *number of DON items published in the last 24 hours*,
//! a genuine, non-overlapping daily count (the same discipline as
//! `cisa_kev`'s daily additions): consecutive daily polls share no members, so a
//! daily z-score is meaningful. Most days the count is zero; a day with one or
//! more is a real announcement, which is exactly what the baseline should flag.
//!
//! API: <https://www.who.int/api/news/diseaseoutbreaknews>
//!
//! Authentication: none. License: WHO content terms; free to use with
//! attribution.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use wse_collector::CollectorError;
use wse_model::{EntityId, Observation, RawReference, Source, SourceId};

pub const SOURCE_ID: &str = "who_outbreaks";
pub const COLLECTOR_TYPE: &str = "who_outbreaks";

/// Recent DON items, newest first.
pub const API_ENDPOINT: &str =
    "https://www.who.int/api/news/diseaseoutbreaknews?$orderby=PublicationDate%20desc&$top=25";

/// The catalog entry.
pub fn source() -> Source {
    Source {
        id: SourceId::new(SOURCE_ID),
        name: "WHO Disease Outbreak News".to_string(),
        provider: "World Health Organization".to_string(),
        category: "health".to_string(),
        subcategory: Some("outbreak_news".to_string()),
        endpoint: API_ENDPOINT.to_string(),
        protocol: wse_model::Protocol::Https,
        format: wse_model::DataFormat::Json,
        cadence: wse_model::Cadence::Interval { seconds: 86_400 },
        timezone: Some("UTC".to_string()),
        license: Some("WHO content terms; free reuse with attribution".to_string()),
        authentication: wse_model::AuthKind::None,
        cost: wse_model::Cost::Free,
        historical_available: true,
        realtime_available: false,
        geospatial: false,
        entities: vec!["health".to_string()],
        priority: 42,
        enabled: true,
        collector_type: COLLECTOR_TYPE.to_string(),
        parameters: std::collections::BTreeMap::new(),
        tier: wse_model::SourceTier::Tier1,
        measurement: wse_model::MeasurementSemantics::StableSeries,
        feeds_lenses: vec!["lens_health".to_string(), "lens_humanitarian".to_string()],
        derivations: Vec::new(),
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Response {
    #[serde(default, rename = "value")]
    pub value: Vec<Item>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Item {
    #[serde(default, rename = "Id")]
    pub id: String,
    #[serde(default, rename = "PublicationDate")]
    pub publication_date: String,
    #[serde(default, rename = "Title")]
    pub title: String,
}

/// Parse a DON response into a single daily-count observation.
///
/// Emits one observation: how many items were published in the 24 hours ending
/// at `received_at`. The record key is the UTC day, so re-polling the same day
/// de-duplicates.
pub fn parse(body: &[u8], received_at: DateTime<Utc>) -> Result<Vec<Observation>, CollectorError> {
    let response: Response =
        serde_json::from_slice(body).map_err(|e| CollectorError::Parse(format!("who don: {e}")))?;

    let since = received_at - Duration::days(1);
    let recent: Vec<&Item> = response
        .value
        .iter()
        .filter(|item| {
            parse_date(&item.publication_date)
                .map(|d| d > since && d <= received_at)
                .unwrap_or(false)
        })
        .collect();

    let day = received_at.format("%Y-%m-%d").to_string();
    let raw = RawReference {
        locator: API_ENDPOINT.to_string(),
        hash: wse_model::fnv1a_hex(&String::from_utf8_lossy(body)),
        content_type: Some("application/json".to_string()),
        bytes: Some(body.len() as u64),
    };
    let mut observation = Observation::new(
        SourceId::new(SOURCE_ID),
        Some(EntityId::new("health")),
        "outbreak_news",
        recent.len() as f64,
        "items",
        received_at,
        raw,
    )
    .with_received_at(received_at)
    // The UTC day is the natural key: one count per day.
    .with_record_key(day.clone())
    .with_attribute("day", day);
    if let Some(first) = recent.first() {
        observation = observation.with_attribute("latest", first.title.clone());
    }

    Ok(vec![observation])
}

fn parse_date(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        include_bytes!("../../../tests/fixtures/who_outbreaks.json").to_vec()
    }

    #[test]
    fn counts_publications_in_the_last_day() {
        let received = DateTime::parse_from_rfc3339("2026-09-26T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let observations = parse(&fixture(), received).unwrap();
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].metric, "outbreak_news");
        assert_eq!(observations[0].unit, "items");
        // The fixture has a 2026-09-25 publication, inside the 24h window.
        assert_eq!(observations[0].value, 1.0);
    }

    #[test]
    fn a_quiet_day_is_recorded_as_zero() {
        let received = DateTime::parse_from_rfc3339("2026-10-01T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let observations = parse(&fixture(), received).unwrap();
        assert_eq!(observations[0].value, 0.0);
    }

    #[test]
    fn re_polling_the_same_day_de_duplicates() {
        let received = DateTime::parse_from_rfc3339("2026-09-26T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let a = parse(&fixture(), received).unwrap();
        let b = parse(&fixture(), received).unwrap();
        assert_eq!(a[0].id, b[0].id);
    }

    #[test]
    fn malformed_payload_is_a_parse_error() {
        let received = DateTime::parse_from_rfc3339("2026-10-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert!(matches!(
            parse(b"nope", received),
            Err(CollectorError::Parse(_))
        ));
    }

    #[test]
    fn catalog_is_institutional_health() {
        let source = source();
        assert_eq!(source.category, "health");
        assert_eq!(source.tier, wse_model::SourceTier::Tier1);
        assert!(source.feeds_lenses.contains(&"lens_health".to_string()));
    }
}
