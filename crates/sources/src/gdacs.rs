//! GDACS — the Global Disaster Alert and Coordination System.
//!
//! GDACS is a joint UN/European Commission platform that publishes official
//! multi-hazard alerts (earthquakes, cyclones, floods, volcanoes, wildfires,
//! droughts) with a severity *alert level* (Green/Orange/Red) and score. It is
//! independent of NASA EONET and USGS: EONET observes natural events, GDACS
//! *assesses their impact*, so a Red cyclone alert is a different fact from a
//! storm appearing in EONET.
//!
//! The measurement is the *count of active alerts per hazard type and alert
//! level* over the recent window. A jump in Red alerts is a real escalation in
//! what is happening to people, and it feeds the DISASTERS and HUMANITARIAN
//! lenses with official severity rather than news chatter.
//!
//! API docs: <https://www.gdacs.org/Knowledge/API.aspx>
//!
//! Authentication: none. License: GDACS data is free to use with attribution.

use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use wse_collector::CollectorError;
use wse_model::{EntityId, Observation, RawReference, Source, SourceId};

pub const SOURCE_ID: &str = "gdacs_disasters";
pub const COLLECTOR_TYPE: &str = "gdacs_disasters";

/// The event-list search endpoint. `fromDate`/`toDate` are filled at collection
/// time so a long-running process does not freeze its window.
pub const API_BASE: &str = "https://www.gdacs.org/gdacsapi/api/events/geteventlist/SEARCH";

/// The hazard types tracked, as GDACS codes.
pub const HAZARDS: &[(&str, &str)] = &[
    ("EQ", "earthquake"),
    ("TC", "tropical_cyclone"),
    ("FL", "flood"),
    ("VO", "volcano"),
    ("WF", "wildfire"),
    ("DR", "drought"),
];

/// The alert levels, weakest first.
pub const LEVELS: [&str; 3] = ["Green", "Orange", "Red"];

/// Build the search URL for a window ending at `now`.
pub fn window_url(now: DateTime<Utc>, days: i64) -> String {
    let to = now.date_naive();
    let from = to - chrono::Duration::days(days);
    format!(
        "{API_BASE}?eventlist={}&fromDate={}&toDate={}",
        HAZARDS.iter().map(|h| h.0).collect::<Vec<_>>().join(","),
        from.format("%Y-%m-%d"),
        to.format("%Y-%m-%d")
    )
}

/// The catalog entry.
pub fn source() -> Source {
    let mut parameters = BTreeMap::new();
    parameters.insert("hazards".to_string(), HAZARDS.len().to_string());
    Source {
        id: SourceId::new(SOURCE_ID),
        name: "GDACS Disaster Alerts".to_string(),
        provider: "GDACS (UN OCHA / European Commission JRC)".to_string(),
        category: "disasters".to_string(),
        subcategory: Some("multi_hazard".to_string()),
        endpoint: API_BASE.to_string(),
        protocol: wse_model::Protocol::Https,
        format: wse_model::DataFormat::GeoJson,
        cadence: wse_model::Cadence::Interval { seconds: 1800 },
        timezone: Some("UTC".to_string()),
        license: Some("GDACS data; free to use with attribution".to_string()),
        authentication: wse_model::AuthKind::None,
        cost: wse_model::Cost::Free,
        historical_available: true,
        realtime_available: true,
        geospatial: true,
        entities: vec!["disasters".to_string()],
        priority: 28,
        enabled: true,
        collector_type: COLLECTOR_TYPE.to_string(),
        parameters,
        tier: wse_model::SourceTier::Tier1,
        measurement: wse_model::MeasurementSemantics::StableSeries,
        feeds_lenses: vec!["lens_earth".to_string(), "lens_humanitarian".to_string()],
        derivations: Vec::new(),
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct FeatureCollection {
    #[serde(default)]
    pub features: Vec<Feature>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Feature {
    #[serde(default)]
    pub properties: Properties,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Properties {
    #[serde(default)]
    pub eventtype: String,
    #[serde(default)]
    pub alertlevel: String,
}

/// Parse a GDACS event list into counts.
///
/// Emits one observation per `(hazard, level)` pair, including zeros for the
/// combinations with no alerts: "no Red floods in the window" is a measurement
/// the baseline needs, and a level falling to zero must be visible.
pub fn parse(body: &[u8], received_at: DateTime<Utc>) -> Result<Vec<Observation>, CollectorError> {
    let collection: FeatureCollection =
        serde_json::from_slice(body).map_err(|e| CollectorError::Parse(format!("gdacs: {e}")))?;

    let mut counts: BTreeMap<(String, String), usize> = BTreeMap::new();
    for feature in &collection.features {
        let hazard = feature.properties.eventtype.to_uppercase();
        let level = normalize_level(&feature.properties.alertlevel);
        if hazard.is_empty() || level.is_empty() {
            continue;
        }
        *counts.entry((hazard, level)).or_default() += 1;
    }

    let source_id = SourceId::new(SOURCE_ID);
    let entity = EntityId::new("disasters");
    let hash = wse_model::fnv1a_hex(&String::from_utf8_lossy(body));
    let bytes = body.len() as u64;
    let mut observations = Vec::new();

    for (code, label) in HAZARDS {
        for level in LEVELS {
            let count = counts
                .get(&(code.to_string(), level.to_string()))
                .copied()
                .unwrap_or(0);
            let raw = RawReference {
                locator: API_BASE.to_string(),
                hash: hash.clone(),
                content_type: Some("application/geo+json".to_string()),
                bytes: Some(bytes),
            };
            observations.push(
                Observation::new(
                    source_id.clone(),
                    Some(entity.clone()),
                    "active_alerts",
                    count as f64,
                    "alerts",
                    received_at,
                    raw,
                )
                .with_received_at(received_at)
                .with_dimension("hazard", *code)
                .with_dimension("level", level)
                .with_attribute("hazard", *label),
            );
        }
    }

    Ok(observations)
}

/// GDACS alert levels are capitalised inconsistently; normalise to the canonical
/// `Green`/`Orange`/`Red`, and drop anything else rather than guess.
fn normalize_level(raw: &str) -> String {
    LEVELS
        .iter()
        .find(|l| l.eq_ignore_ascii_case(raw.trim()))
        .map(|l| l.to_string())
        .unwrap_or_default()
}

/// Parse a GDACS timestamp (`YYYY-MM-DDTHH:MM:SS`, no zone; it is UTC).
#[allow(dead_code)]
fn parse_time(raw: &str) -> Option<DateTime<Utc>> {
    NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S")
        .ok()
        .map(|t| t.and_utc())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn received() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-01T05:30:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn fixture() -> Vec<u8> {
        include_bytes!("../../../tests/fixtures/gdacs_disasters.json").to_vec()
    }

    fn find<'a>(observations: &'a [Observation], hazard: &str, level: &str) -> &'a Observation {
        observations
            .iter()
            .find(|o| {
                o.dimensions.get("hazard").map(String::as_str) == Some(hazard)
                    && o.dimensions.get("level").map(String::as_str) == Some(level)
            })
            .expect("observation for hazard/level")
    }

    #[test]
    fn counts_alerts_per_hazard_and_level() {
        let observations = parse(&fixture(), received()).unwrap();
        // Six hazards times three levels.
        assert_eq!(observations.len(), HAZARDS.len() * LEVELS.len());
        assert_eq!(observations[0].metric, "active_alerts");
        assert_eq!(observations[0].unit, "alerts");
    }

    #[test]
    fn a_missing_combination_is_recorded_as_zero() {
        let observations = parse(&fixture(), received()).unwrap();
        // No drought alerts in the fixture window.
        assert_eq!(find(&observations, "DR", "Red").value, 0.0);
    }

    #[test]
    fn every_series_is_distinct() {
        let observations = parse(&fixture(), received()).unwrap();
        let keys: std::collections::HashSet<String> =
            observations.iter().map(|o| o.series_key()).collect();
        assert_eq!(keys.len(), observations.len());
    }

    #[test]
    fn malformed_payload_is_a_parse_error() {
        assert!(matches!(
            parse(b"nope", received()),
            Err(CollectorError::Parse(_))
        ));
    }

    #[test]
    fn an_unknown_alert_level_is_not_counted() {
        let body = br#"{"type":"FeatureCollection","features":[{"properties":{"eventtype":"EQ","alertlevel":"Purple"}}]}"#;
        let observations = parse(body, received()).unwrap();
        assert!(observations.iter().all(|o| o.value == 0.0));
    }

    #[test]
    fn catalog_is_institutional_disasters() {
        let source = source();
        assert_eq!(source.category, "disasters");
        assert_eq!(source.tier, wse_model::SourceTier::Tier1);
        assert!(source
            .feeds_lenses
            .contains(&"lens_humanitarian".to_string()));
    }
}
