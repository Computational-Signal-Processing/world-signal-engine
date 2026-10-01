//! NWS active weather alerts — a real-time public-safety source.
//!
//! The National Weather Service publishes every active alert as GeoJSON,
//! needing no key. Each alert has a severity, so the quantity we track is the
//! *count of active alerts at each severity level*, in total and per US state.
//! A jump in Extreme or Severe alerts is a real change in what is happening to
//! people right now, and it is exactly the kind of thing the engine exists to
//! surface.
//!
//! Unlike a per-event source, the feed is a *snapshot*: the count can fall as
//! well as rise, which makes it a good test of the baseline engine in both
//! directions.
//!
//! API docs: <https://www.weather.gov/documentation/services-web-api>

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use wse_collector::CollectorError;
use wse_model::{EntityId, Observation, RawReference, Source, SourceId};

pub const SOURCE_ID: &str = "nws_alerts";
pub const COLLECTOR_TYPE: &str = "nws_alerts";

/// Active, actual alerts. `status=actual` drops test and exercise messages.
pub const ALERTS_ENDPOINT: &str =
    "https://api.weather.gov/alerts/active?status=actual&message_type=alert";

/// The four severities the NWS uses, in the order we emit them.
const SEVERITIES: [(&str, &str); 4] = [
    ("Extreme", "extreme"),
    ("Severe", "severe"),
    ("Moderate", "moderate"),
    ("Minor", "minor"),
];

/// The catalog entry.
pub fn source() -> Source {
    Source {
        id: SourceId::new(SOURCE_ID),
        name: "NWS Active Weather Alerts".to_string(),
        provider: "U.S. National Weather Service".to_string(),
        category: "weather".to_string(),
        subcategory: Some("public_safety".to_string()),
        endpoint: ALERTS_ENDPOINT.to_string(),
        protocol: wse_model::Protocol::Https,
        format: wse_model::DataFormat::GeoJson,
        cadence: wse_model::Cadence::Interval { seconds: 600 },
        timezone: Some("UTC".to_string()),
        license: Some("Public domain (US Government)".to_string()),
        authentication: wse_model::AuthKind::None,
        cost: wse_model::Cost::Free,
        historical_available: true,
        realtime_available: true,
        geospatial: true,
        entities: vec!["weather".to_string(), "united_states".to_string()],
        priority: 20,
        enabled: true,
        collector_type: COLLECTOR_TYPE.to_string(),
        parameters: std::collections::BTreeMap::new(),
        tier: wse_model::SourceTier::Tier1,
        measurement: wse_model::MeasurementSemantics::StableSeries,
        feeds_lenses: vec!["lens_earth".to_string()],
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
    #[serde(default, rename = "areaDesc")]
    pub area_desc: Option<String>,
    #[serde(default)]
    pub severity: Option<String>,
}

/// Parse an NWS alerts payload into observations.
///
/// Pure: no I/O and no clock reads beyond the supplied `received_at`, so a
/// fixture produces identical results in a test and in replay.
///
/// Emits one observation per severity level: the count of active alerts, both
/// nationally (`entity=weather_united_states`) and per state
/// (`entity=weather_us_tx`). A zero count is a real measurement — "there are no
/// Extreme alerts right now" — and is recorded, not omitted. An alert whose
/// severity is not one of the four known levels is skipped: it is not a level
/// we track, and folding it into another would corrupt the distribution.
pub fn parse(body: &[u8], received_at: DateTime<Utc>) -> Result<Vec<Observation>, CollectorError> {
    let collection: FeatureCollection = serde_json::from_slice(body)
        .map_err(|e| CollectorError::Parse(format!("nws alerts: {e}")))?;

    // (severity label, named states) for every countable active alert.
    let alerts: Vec<(String, Vec<String>)> = collection
        .features
        .iter()
        .filter_map(|f| {
            let severity = f.properties.severity.clone()?;
            if !SEVERITIES.iter().any(|(label, _)| *label == severity) {
                return None;
            }
            let area = f.properties.area_desc.clone().unwrap_or_default();
            Some((severity, states_of(&area)))
        })
        .collect();

    let source_id = SourceId::new(SOURCE_ID);
    let hash = wse_model::fnv1a_hex(&String::from_utf8_lossy(body));
    let bytes = body.len() as u64;
    let mut observations = Vec::new();

    // Total active alerts, per severity.
    for (label, dim) in SEVERITIES {
        let count = alerts.iter().filter(|(s, _)| s == label).count() as f64;
        observations.push(count_observation(
            &source_id,
            &hash,
            bytes,
            EntityId::new("weather_united_states"),
            count,
            dim,
            None,
            received_at,
        ));
    }

    // The same counts, per state, so a regional burst is not hidden inside the
    // national total. An alert naming several states counts in each.
    let mut per_state: std::collections::BTreeMap<&str, [usize; 4]> = Default::default();
    for (severity, states) in &alerts {
        for state in states {
            let bucket = per_state.entry(state.as_str()).or_default();
            for (i, (label, _)) in SEVERITIES.iter().enumerate() {
                if severity == label {
                    bucket[i] += 1;
                }
            }
        }
    }
    for (state, counts) in per_state {
        let entity = EntityId::new(format!("weather_us_{}", wse_model::canonicalize(state)));
        for (i, (_, dim)) in SEVERITIES.iter().enumerate() {
            observations.push(count_observation(
                &source_id,
                &hash,
                bytes,
                entity.clone(),
                counts[i] as f64,
                dim,
                Some(state),
                received_at,
            ));
        }
    }

    Ok(observations)
}

#[allow(clippy::too_many_arguments)]
fn count_observation(
    source_id: &SourceId,
    hash: &str,
    bytes: u64,
    entity: EntityId,
    count: f64,
    severity_dim: &str,
    state: Option<&str>,
    received_at: DateTime<Utc>,
) -> Observation {
    let raw = RawReference {
        locator: ALERTS_ENDPOINT.to_string(),
        hash: hash.to_string(),
        content_type: Some("application/geo+json".to_string()),
        bytes: Some(bytes),
    };

    // The observation time is the collection time: this is a snapshot count,
    // not a per-alert event, so it belongs to the moment it was measured.
    //
    // The record key is the full series key (base series *plus* dimensions).
    // Every count is distinguished by its severity — and, for a state, by the
    // state — so a base-key id would collide across the four severities of one
    // entity and silently drop three of them. `with_record_key` folds the
    // dimensions into the id without putting them in the series key, so the
    // series keep their own dimensions while the ids stay distinct.
    let mut observation = Observation::new(
        source_id.clone(),
        Some(entity),
        "active_weather_alerts",
        count,
        "alerts",
        received_at,
        raw,
    )
    .with_received_at(received_at)
    .with_dimension("severity", severity_dim.to_string());

    if let Some(state) = state {
        observation = observation.with_dimension("state", state.to_string());
    }
    let record_key = observation.series_key();
    observation.with_record_key(record_key)
}

/// US state/territory codes, so an area description can be reduced to states.
const STATE_CODES: [&str; 59] = [
    "AL", "AK", "AZ", "AR", "CA", "CO", "CT", "DE", "FL", "GA", "HI", "ID", "IL", "IN", "IA", "KS",
    "KY", "LA", "ME", "MD", "MA", "MI", "MN", "MS", "MO", "MT", "NE", "NV", "NH", "NJ", "NM", "NY",
    "NC", "ND", "OH", "OK", "OR", "PA", "RI", "SC", "SD", "TN", "TX", "UT", "VT", "VA", "WA", "WV",
    "WI", "WY", "DC", "PR", "VI", "GU", "AS", "MP", "UM", "FM", "MH",
];

/// Extract the states named in an area description.
///
/// `areaDesc` looks like `"Crockett, TX; Sutton, TX"` or `"Coastal Waters of
/// Cape Cod, MA"`. The two-letter code after the last comma of each segment is
/// the state. A segment with no recognisable code contributes nothing.
fn states_of(area: &str) -> Vec<String> {
    let mut states = Vec::new();
    for segment in area.split(';') {
        let Some((_, tail)) = segment.trim().rsplit_once(',') else {
            continue;
        };
        let code = tail.trim().to_uppercase();
        if STATE_CODES.contains(&code.as_str()) && !states.contains(&code) {
            states.push(code);
        }
    }
    states
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn fixture() -> Vec<u8> {
        include_bytes!("../../../tests/fixtures/nws_active_alerts.json").to_vec()
    }

    fn received() -> DateTime<Utc> {
        Utc.timestamp_millis_opt(1_700_000_000_000)
            .single()
            .unwrap()
    }

    fn national<'a>(observations: &'a [Observation], severity: &str) -> &'a Observation {
        observations
            .iter()
            .find(|o| {
                o.entity_id.as_ref().unwrap().as_str() == "weather_united_states"
                    && o.dimensions.get("severity").map(String::as_str) == Some(severity)
            })
            .expect("national observation for severity")
    }

    #[test]
    fn counts_every_severity_nationally() {
        let observations = parse(&fixture(), received()).unwrap();
        assert_eq!(national(&observations, "extreme").value, 1.0);
        assert_eq!(national(&observations, "severe").value, 1.0);
        assert_eq!(national(&observations, "moderate").value, 1.0);
        assert_eq!(national(&observations, "minor").value, 1.0);
        assert_eq!(
            national(&observations, "severe").metric,
            "active_weather_alerts"
        );
        assert_eq!(national(&observations, "severe").unit, "alerts");
    }

    #[test]
    fn a_severity_with_no_alerts_is_recorded_as_zero() {
        // "No Extreme alerts right now" is a measurement the baseline needs,
        // so a missing level is emitted as 0 rather than omitted.
        let body = br#"{"features":[{"properties":{"severity":"Minor","areaDesc":"A, CA"}}]}"#;
        let observations = parse(body, received()).unwrap();
        assert_eq!(national(&observations, "extreme").value, 0.0);
        assert_eq!(national(&observations, "minor").value, 1.0);
    }

    #[test]
    fn an_unknown_severity_is_not_counted() {
        let observations = parse(&fixture(), received()).unwrap();
        let total: f64 = observations
            .iter()
            .filter(|o| o.entity_id.as_ref().unwrap().as_str() == "weather_united_states")
            .map(|o| o.value)
            .sum();
        assert_eq!(total, 4.0, "the Unknown-severity alert must not be counted");
    }

    #[test]
    fn per_state_counts_are_emitted_for_named_states() {
        let observations = parse(&fixture(), received()).unwrap();
        let tx_severe = observations
            .iter()
            .find(|o| {
                o.entity_id.as_ref().unwrap().as_str() == "weather_us_tx"
                    && o.dimensions.get("severity").map(String::as_str) == Some("severe")
            })
            .expect("TX severe observation");
        assert_eq!(tx_severe.value, 1.0);
        assert_eq!(
            tx_severe.dimensions.get("state").map(String::as_str),
            Some("TX")
        );
        // Each state emits all four severity series, so a drop to zero is seen.
        let tx_count = observations
            .iter()
            .filter(|o| o.entity_id.as_ref().unwrap().as_str() == "weather_us_tx")
            .count();
        assert_eq!(tx_count, 4);
    }

    #[test]
    fn national_and_state_series_never_collide() {
        let observations = parse(&fixture(), received()).unwrap();
        let keys: std::collections::HashSet<String> =
            observations.iter().map(|o| o.series_key()).collect();
        assert_eq!(
            keys.len(),
            observations.len(),
            "every count must have its own series"
        );
    }

    #[test]
    fn every_series_has_its_own_id() {
        // The four severities of one entity share a base series key and differ
        // only by the `severity` dimension. The id must include that dimension,
        // or three of the four counts would de-duplicate away and only one
        // severity would ever reach storage or the baseline.
        let observations = parse(&fixture(), received()).unwrap();
        let ids: std::collections::HashSet<&str> =
            observations.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(
            ids.len(),
            observations.len(),
            "each (entity, severity) series must have a distinct observation id"
        );
    }

    #[test]
    fn malformed_payload_is_a_parse_error_not_an_empty_result() {
        assert!(matches!(
            parse(b"nope", received()),
            Err(CollectorError::Parse(_))
        ));
    }

    #[test]
    fn catalog_entry_needs_no_auth_and_is_geospatial() {
        let source = source();
        assert_eq!(source.authentication, wse_model::AuthKind::None);
        assert_eq!(source.cost, wse_model::Cost::Free);
        assert!(source.geospatial);
        assert_eq!(source.category, "weather");
    }

    #[test]
    fn states_are_extracted_from_area_descriptions() {
        assert_eq!(states_of("Crockett, TX; Sutton, TX"), vec!["TX"]);
        assert_eq!(states_of("Franklin, NE; Harlan, NE"), vec!["NE"]);
        assert_eq!(states_of("Coastal Waters of Cape Cod, MA"), vec!["MA"]);
        assert!(states_of("No comma here").is_empty());
    }
}
