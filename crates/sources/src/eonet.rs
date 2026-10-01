//! NASA EONET — open natural events (wildfires, storms, volcanoes).
//!
//! EONET publishes a continuously updated catalogue of natural events, each
//! with a category and a position. It needs no key.
//!
//! The measurement is the *number of open events in a category*. A burst of
//! wildfires or storms is a real change in the world, and it is geospatial:
//! every event carries a coordinate, so the map has something true to show.
//!
//! The feed is a snapshot (a count can fall as events close), and an event
//! with no magnitude is still a countable event — its absence of a magnitude
//! is not an absence of the event.
//!
//! API docs: <https://eonet.gsfc.nasa.gov/docs/v3>

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use wse_collector::CollectorError;
use wse_model::{EntityId, Observation, RawReference, Source, SourceId};

pub const SOURCE_ID: &str = "nasa_eonet";
pub const COLLECTOR_TYPE: &str = "nasa_eonet";

/// Open natural events, most recent first.
pub const EVENTS_ENDPOINT: &str = "https://eonet.gsfc.nasa.gov/api/v3/events?status=open";

/// The catalog entry.
pub fn source() -> Source {
    Source {
        id: SourceId::new(SOURCE_ID),
        name: "NASA EONET Natural Events".to_string(),
        provider: "NASA Earth Observatory".to_string(),
        category: "earth".to_string(),
        subcategory: Some("natural_events".to_string()),
        endpoint: EVENTS_ENDPOINT.to_string(),
        protocol: wse_model::Protocol::Https,
        format: wse_model::DataFormat::Json,
        cadence: wse_model::Cadence::Interval { seconds: 1800 },
        timezone: Some("UTC".to_string()),
        license: Some("Public domain (NASA)".to_string()),
        authentication: wse_model::AuthKind::None,
        cost: wse_model::Cost::Free,
        historical_available: true,
        realtime_available: true,
        geospatial: true,
        entities: vec!["natural_events".to_string(), "earth".to_string()],
        priority: 25,
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
pub struct EventsResponse {
    #[serde(default)]
    pub events: Vec<Event>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Event {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    /// `None` while the event is open; an RFC 3339 timestamp once it closes.
    #[serde(default)]
    pub closed: Option<String>,
    #[serde(default)]
    pub categories: Vec<Category>,
    #[serde(default)]
    pub geometry: Vec<Geometry>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Category {
    #[serde(default)]
    pub id: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Geometry {
    #[serde(default, rename = "type")]
    pub geometry_type: Option<String>,
    /// `[longitude, latitude]` for a Point.
    #[serde(default)]
    pub coordinates: Option<serde_json::Value>,
    #[serde(default)]
    pub date: Option<String>,
}

/// Parse an EONET payload into observations.
///
/// Pure: no I/O and no clock reads beyond the supplied `received_at`.
///
/// Emits one observation per category: the count of open events, with the
/// newest event's position attached so the map has a real coordinate. A
/// category with no open events is emitted as 0 rather than omitted — "no
/// wildfires open" is a measurement the baseline needs.
pub fn parse(body: &[u8], received_at: DateTime<Utc>) -> Result<Vec<Observation>, CollectorError> {
    let response: EventsResponse = serde_json::from_slice(body)
        .map_err(|e| CollectorError::Parse(format!("eonet events: {e}")))?;

    // Only open events count; a closed event is no longer happening.
    let open: Vec<&Event> = response
        .events
        .iter()
        .filter(|e| e.closed.is_none())
        .collect();

    // Count per category, and remember the newest position in each.
    let mut per_category: std::collections::BTreeMap<String, (usize, Option<(f64, f64)>)> =
        Default::default();
    for event in &open {
        for category in &event.categories {
            if category.id.is_empty() {
                continue;
            }
            let entry = per_category.entry(category.id.clone()).or_default();
            entry.0 += 1;
            if entry.1.is_none() {
                entry.1 = point_of(event);
            }
        }
    }

    let source_id = SourceId::new(SOURCE_ID);
    let hash = wse_model::fnv1a_hex(&String::from_utf8_lossy(body));
    let bytes = body.len() as u64;

    // A stable set of categories, so a category falling to zero is visible as
    // a series rather than simply disappearing.
    let known = known_categories();
    let mut categories: Vec<String> = per_category.keys().cloned().collect();
    for category in known {
        if !categories.iter().any(|c| c == category) {
            categories.push(category.to_string());
        }
    }
    categories.sort();

    let mut observations = Vec::with_capacity(categories.len());
    for category in categories {
        let (count, location) = per_category.get(&category).cloned().unwrap_or((0, None));
        let entity = EntityId::new(format!("natural_{}", wse_model::canonicalize(&category)));
        let raw = RawReference {
            locator: EVENTS_ENDPOINT.to_string(),
            hash: hash.clone(),
            content_type: Some("application/json".to_string()),
            bytes: Some(bytes),
        };
        let mut observation = Observation::new(
            source_id.clone(),
            Some(entity),
            "open_natural_events",
            count as f64,
            "events",
            received_at,
            raw,
        )
        .with_received_at(received_at)
        .with_dimension("category", category.clone());
        if let Some((lat, lon)) = location {
            observation = observation.with_location(lat, lon);
        }
        observations.push(observation);
    }

    Ok(observations)
}

/// The categories EONET uses, so each is tracked even when it has no events.
fn known_categories() -> &'static [&'static str] {
    &[
        "drought",
        "dustHaze",
        "earthquakes",
        "floods",
        "landslides",
        "manmade",
        "seaLakeIce",
        "severeStorms",
        "snow",
        "tempExtremes",
        "volcanoes",
        "waterColor",
        "wildfires",
    ]
}

/// The newest position of an event, as `(latitude, longitude)`.
fn point_of(event: &Event) -> Option<(f64, f64)> {
    let geometry = event.geometry.last()?;
    if geometry.geometry_type.as_deref() != Some("Point") {
        return None;
    }
    let pair = geometry.coordinates.as_ref()?.as_array()?;
    let lon = pair.first()?.as_f64()?;
    let lat = pair.get(1)?.as_f64()?;
    Some((lat, lon))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn fixture() -> Vec<u8> {
        include_bytes!("../../../tests/fixtures/eonet_events.json").to_vec()
    }

    fn received() -> DateTime<Utc> {
        Utc.timestamp_millis_opt(1_700_000_000_000)
            .single()
            .unwrap()
    }

    fn find<'a>(observations: &'a [Observation], category: &str) -> &'a Observation {
        observations
            .iter()
            .find(|o| o.dimensions.get("category").map(String::as_str) == Some(category))
            .expect("observation for category")
    }

    #[test]
    fn counts_open_events_per_category() {
        let observations = parse(&fixture(), received()).unwrap();
        assert_eq!(find(&observations, "wildfires").value, 2.0);
        assert_eq!(find(&observations, "severeStorms").value, 1.0);
        assert_eq!(find(&observations, "volcanoes").value, 1.0);
        assert_eq!(
            find(&observations, "wildfires").metric,
            "open_natural_events"
        );
        assert_eq!(find(&observations, "wildfires").unit, "events");
    }

    #[test]
    fn a_closed_event_is_not_counted() {
        // The fixture has a fifth, closed wildfire; only two wildfires are open.
        let observations = parse(&fixture(), received()).unwrap();
        assert_eq!(find(&observations, "wildfires").value, 2.0);
    }

    #[test]
    fn a_category_with_no_open_events_is_recorded_as_zero() {
        let observations = parse(&fixture(), received()).unwrap();
        assert_eq!(find(&observations, "floods").value, 0.0);
    }

    #[test]
    fn the_newest_position_is_attached_for_the_map() {
        let observations = parse(&fixture(), received()).unwrap();
        let storms = find(&observations, "severeStorms");
        assert_eq!(storms.latitude, Some(36.6));
        assert_eq!(storms.longitude, Some(-50.4));
        // A category with no events carries no location, and that is honest.
        assert_eq!(find(&observations, "floods").latitude, None);
    }

    #[test]
    fn series_keys_are_unique_per_category() {
        let observations = parse(&fixture(), received()).unwrap();
        let keys: std::collections::HashSet<String> =
            observations.iter().map(|o| o.series_key()).collect();
        assert_eq!(keys.len(), observations.len());
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
        assert_eq!(source.category, "earth");
    }
}
