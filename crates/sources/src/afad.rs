//! AFAD — Turkey's own earthquake catalogue.
//!
//! AFAD (Disaster and Emergency Management Presidency) publishes Turkey's
//! national seismic events. It is Turkey's authoritative geophysical feed and
//! the TURKEY lens's primary sensor: it reports events USGS may threshold away,
//! so it is genuinely independent, not a duplicate of the USGS feed.
//!
//! The measured quantity is the rate of events per region (an *increasing*
//! series, so a burst of aftershocks is a real change), re-measured over a
//! fixed recent window.
//!
//! Service: <https://deprem.afad.gov.tr/event-service> (public event filter)
//!
//! Authentication: none. License: AFAD open data terms.

use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use wse_collector::CollectorError;
use wse_model::{EntityId, Observation, RawReference, Source, SourceId};

pub const SOURCE_ID: &str = "afad_earthquakes";
pub const COLLECTOR_TYPE: &str = "afad_event_filter";

/// The public event-filter service.
pub const API_ENDPOINT: &str = "https://deprem.afad.gov.tr/apiv2/event/filter";

/// The catalog entry.
pub fn source() -> Source {
    let mut parameters = BTreeMap::new();
    parameters.insert("country".to_string(), "Türkiye".to_string());
    parameters.insert("min_magnitude".to_string(), "2.0".to_string());

    Source {
        id: SourceId::new(SOURCE_ID),
        name: "AFAD Turkey Earthquake Catalogue".to_string(),
        provider: "AFAD (Disaster and Emergency Management Presidency)".to_string(),
        category: "geophysics".to_string(),
        subcategory: Some("seismology".to_string()),
        endpoint: API_ENDPOINT.to_string(),
        protocol: wse_model::Protocol::Https,
        format: wse_model::DataFormat::Json,
        cadence: wse_model::Cadence::Interval { seconds: 3600 },
        timezone: Some("Europe/Istanbul".to_string()),
        license: Some("AFAD open data terms".to_string()),
        authentication: wse_model::AuthKind::None,
        cost: wse_model::Cost::Free,
        historical_available: true,
        realtime_available: true,
        geospatial: true,
        entities: vec!["earthquake".to_string()],
        priority: 15,
        enabled: true,
        collector_type: COLLECTOR_TYPE.to_string(),
        parameters,
        tier: wse_model::SourceTier::Tier1,
        measurement: wse_model::MeasurementSemantics::StableSeries,
        feeds_lenses: vec!["lens_earth".to_string(), "lens_turkey".to_string()],
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Event {
    #[serde(default, rename = "eventID")]
    pub event_id: String,
    #[serde(default)]
    pub location: String,
    #[serde(default)]
    pub latitude: String,
    #[serde(default)]
    pub longitude: String,
    #[serde(default)]
    pub depth: String,
    #[serde(default, rename = "type")]
    pub event_type: String,
    #[serde(default)]
    pub magnitude: String,
    #[serde(default)]
    pub province: String,
    #[serde(default)]
    pub district: String,
    /// Local time, `YYYY-MM-DDTHH:MM:SS`.
    #[serde(default)]
    pub date: String,
}

/// Build the event-filter URL for a time window.
pub fn filter_url(start: &str, end: &str) -> String {
    format!("{API_ENDPOINT}?start={start}&end={end}&minmag=2&limit=100")
}

/// Parse an AFAD event list into observations.
///
/// The entity is the province, so each province's seismicity is its own series
/// and a local swarm is visible as a change in that province rather than being
/// diluted into a national average.
pub fn parse(body: &[u8], received_at: DateTime<Utc>) -> Result<Vec<Observation>, CollectorError> {
    // AFAD answers an empty result set with a zero-length body, not `[]`.
    if body.iter().all(|b| b.is_ascii_whitespace()) {
        return Ok(Vec::new());
    }
    let events: Vec<Event> = serde_json::from_slice(body)
        .map_err(|e| CollectorError::Parse(format!("afad events: {e}")))?;

    let source_id = SourceId::new(SOURCE_ID);
    let hash = wse_model::fnv1a_hex(&String::from_utf8_lossy(body));
    let mut observations = Vec::new();

    for event in events {
        // A magnitude and a time are what make the event measurable.
        let (Ok(magnitude), Some(observed_at)) = (
            event.magnitude.parse::<f64>(),
            parse_local_time(&event.date),
        ) else {
            continue;
        };

        let province = if event.province.trim().is_empty() {
            event.location.trim()
        } else {
            event.province.trim()
        };
        let entity = EntityId::new(format!("province_{}", wse_model::canonicalize(province)));

        let raw = RawReference {
            locator: format!("afad:{}", event.event_id),
            hash: hash.clone(),
            content_type: Some("application/json".to_string()),
            bytes: Some(body.len() as u64),
        };

        let mut observation = Observation::new(
            source_id.clone(),
            Some(entity),
            "earthquake_magnitude",
            magnitude,
            "magnitude",
            observed_at,
            raw,
        )
        .with_received_at(received_at)
        // AFAD's event id is the stable upstream identity; the query window
        // slides between polls, so the same event must keep one id.
        .with_record_key(&event.event_id)
        .with_attribute("event_id", event.event_id.clone())
        .with_attribute("place", event.location.clone())
        .with_dimension("province", province.to_string());

        if let (Ok(lat), Ok(lon)) = (
            event.latitude.parse::<f64>(),
            event.longitude.parse::<f64>(),
        ) {
            observation = observation.with_location(lat, lon);
        }
        if let Ok(depth) = event.depth.parse::<f64>() {
            observation = observation.with_attribute("depth_km", format!("{depth:.1}"));
        }
        if !event.district.trim().is_empty() {
            observation = observation.with_attribute("district", event.district.trim().to_string());
        }
        if !event.event_type.trim().is_empty() {
            observation =
                observation.with_attribute("mag_type", event.event_type.trim().to_string());
        }

        observations.push(observation);
    }

    observations.sort_by_key(|o| o.observed_at);
    Ok(observations)
}

/// Parse AFAD's local-time string as UTC.
///
/// AFAD reports `date` in Turkey local time (UTC+3) with no zone suffix. Turkey
/// has used a fixed UTC+3 offset year-round since 2016, so the conversion is
/// unambiguous; we subtract three hours and label the result UTC.
fn parse_local_time(raw: &str) -> Option<DateTime<Utc>> {
    NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S")
        .ok()
        .map(|n| n.and_utc() - chrono::Duration::hours(3))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn received() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn fixture() -> Vec<u8> {
        include_bytes!("../../../tests/fixtures/afad_events.json").to_vec()
    }

    #[test]
    fn parses_every_measurable_event() {
        let observations = parse(&fixture(), received()).unwrap();
        assert_eq!(observations.len(), 3);
        assert_eq!(observations[0].metric, "earthquake_magnitude");
        assert_eq!(observations[0].unit, "magnitude");
        assert_eq!(observations[0].value, 2.0);
    }

    #[test]
    fn local_time_is_converted_to_utc() {
        let observations = parse(&fixture(), received()).unwrap();
        // 2026-09-01T01:00:44 local (UTC+3) -> 2026-08-31T22:00:44Z.
        assert_eq!(
            observations[0].observed_at.to_rfc3339(),
            "2026-08-31T22:00:44+00:00"
        );
    }

    #[test]
    fn events_group_by_province() {
        let observations = parse(&fixture(), received()).unwrap();
        let entities: Vec<&str> = observations
            .iter()
            .map(|o| o.entity_id.as_ref().unwrap().as_str())
            .collect();
        assert!(entities.iter().any(|e| e.contains("kahramanmaras")));
        // Location and the event id are kept for drill-down.
        assert!(observations[0].latitude.is_some());
        assert!(!observations[0]
            .attributes
            .get("event_id")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn an_empty_body_is_no_events_not_a_failure() {
        assert!(parse(b"", received()).unwrap().is_empty());
        assert!(parse(b"  ", received()).unwrap().is_empty());
    }

    #[test]
    fn malformed_payload_is_a_parse_error() {
        assert!(matches!(
            parse(b"nope", received()),
            Err(CollectorError::Parse(_))
        ));
    }

    #[test]
    fn the_catalog_is_institutional_and_feeds_turkey() {
        let source = source();
        assert_eq!(source.tier, wse_model::SourceTier::Tier1);
        assert!(source.feeds_lenses.contains(&"lens_turkey".to_string()));
        assert!(source.feeds_lenses.contains(&"lens_earth".to_string()));
        assert!(source.geospatial);
    }
}
