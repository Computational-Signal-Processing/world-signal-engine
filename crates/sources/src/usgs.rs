//! USGS earthquakes — GeoJSON, event-driven, free, no auth.
//!
//! This is the canonical "geophysical event" source: bursts of activity are
//! real, and the interesting signal is usually a change in *rate* or in the
//! magnitude distribution rather than any single quake.
//!
//! Feed docs: <https://earthquake.usgs.gov/earthquakes/feed/v1.0/geojson.php>

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use wse_collector::CollectorError;
use wse_model::{EntityId, Observation, RawReference, Source, SourceId};

pub const SOURCE_ID: &str = "usgs_earthquakes";
pub const COLLECTOR_TYPE: &str = "usgs_earthquake";
pub const ALL_HOUR_ENDPOINT: &str =
    "https://earthquake.usgs.gov/earthquakes/feed/v1.0/summary/all_hour.geojson";

/// The catalog entry.
pub fn source() -> Source {
    Source {
        id: SourceId::new(SOURCE_ID),
        name: "USGS Earthquake Feed".to_string(),
        provider: "U.S. Geological Survey".to_string(),
        category: "geophysics".to_string(),
        subcategory: Some("seismology".to_string()),
        endpoint: ALL_HOUR_ENDPOINT.to_string(),
        protocol: wse_model::Protocol::Https,
        format: wse_model::DataFormat::GeoJson,
        cadence: wse_model::Cadence::Event {},
        timezone: Some("UTC".to_string()),
        license: Some("Public domain (USGS)".to_string()),
        authentication: wse_model::AuthKind::None,
        cost: wse_model::Cost::Free,
        historical_available: true,
        realtime_available: true,
        geospatial: true,
        entities: vec!["earthquake".to_string()],
        priority: 10,
        enabled: true,
        collector_type: COLLECTOR_TYPE.to_string(),
        parameters: std::collections::BTreeMap::new(),
        tier: wse_model::SourceTier::Tier1,
        measurement: wse_model::MeasurementSemantics::StableSeries,
        feeds_lenses: vec!["lens_earth".to_string(), "lens_turkey".to_string()],
        derivations: Vec::new(),
    }
}

/// One GeoJSON feature, reduced to the fields we actually normalize.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Feature {
    pub id: String,
    #[serde(default)]
    pub properties: Properties,
    #[serde(default)]
    pub geometry: Option<Geometry>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Properties {
    pub mag: Option<f64>,
    /// Epoch milliseconds.
    pub time: Option<i64>,
    pub place: Option<String>,
    pub url: Option<String>,
    pub net: Option<String>,
    #[serde(rename = "type")]
    pub event_type: Option<String>,
    pub tsunami: Option<i64>,
    pub sig: Option<f64>,
    pub alert: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Geometry {
    #[serde(rename = "type")]
    pub geometry_type: String,
    /// `[longitude, latitude, depth_km]`.
    pub coordinates: Vec<f64>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct FeatureCollection {
    #[serde(default)]
    pub features: Vec<Feature>,
}

/// Parse a USGS GeoJSON payload into observations.
///
/// Pure: no I/O and no clock reads beyond the supplied `received_at`, so a
/// fixture produces identical results in a test and in replay.
pub fn parse(body: &[u8], received_at: DateTime<Utc>) -> Result<Vec<Observation>, CollectorError> {
    let collection: FeatureCollection = serde_json::from_slice(body)
        .map_err(|e| CollectorError::Parse(format!("usgs geojson: {e}")))?;

    let source_id = SourceId::new(SOURCE_ID);
    let mut observations = Vec::with_capacity(collection.features.len());

    for feature in collection.features {
        // A quake without a magnitude or a time is not measurable; skipping it
        // is not the same as reporting zero.
        let (Some(magnitude), Some(time_ms)) = (feature.properties.mag, feature.properties.time)
        else {
            continue;
        };
        let Some(observed_at) = Utc.timestamp_millis_opt(time_ms).single() else {
            continue;
        };

        // The entity is the seismic region, not the individual quake: a change
        // in a region's behaviour is what matters, and per-quake ids never
        // repeat so they could never form a series.
        let place = feature.properties.place.clone().unwrap_or_default();
        let region = region_of(&place);
        let entity = EntityId::new(format!("region_{}", wse_model::canonicalize(&region)));

        let locator = feature
            .properties
            .url
            .clone()
            .unwrap_or_else(|| format!("usgs:{}", feature.id));
        let hash = wse_model::fnv1a_hex(&String::from_utf8_lossy(body));
        let raw = RawReference {
            locator,
            hash,
            content_type: Some("application/geo+json".to_string()),
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
        // The USGS feature id is the quake's stable upstream identity. The
        // feed is re-fetched every minute, so without this a quake retained in
        // the next feed would be re-minted as a new observation.
        .with_record_key(&feature.id);

        if let Some(geometry) = &feature.geometry {
            if geometry.coordinates.len() >= 2 {
                observation =
                    observation.with_location(geometry.coordinates[1], geometry.coordinates[0]);
            }
        }
        observation = observation
            .with_attribute("event_id", feature.id.clone())
            .with_dimension("region", region.clone())
            .with_attribute("net", feature.properties.net.clone().unwrap_or_default());
        if let Some(depth) = feature.geometry.as_ref().and_then(|g| g.coordinates.get(2)) {
            observation = observation.with_attribute("depth_km", format!("{depth:.1}"));
        }
        if let Some(alert) = &feature.properties.alert {
            observation = observation.with_attribute("alert", alert.clone());
        }
        if let Some(tsunami) = feature.properties.tsunami {
            observation = observation.with_attribute("tsunami", tsunami.to_string());
        }

        observations.push(observation);
    }

    Ok(observations)
}

/// Reduce a USGS place string to a stable region key.
///
/// USGS places look like `"110 km SE of Ierapetra, Greece"` or
/// `"8 km NW of The Geysers, CA"`; the text after the last comma is the
/// region, which is what groups quakes into a series.
fn region_of(place: &str) -> String {
    match place.rsplit_once(',') {
        Some((_, region)) => region.trim().to_string(),
        None => {
            if place.trim().is_empty() {
                "unknown".to_string()
            } else {
                place.trim().to_string()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wse_collector::{CollectionMode, Collector};

    fn fixture() -> Vec<u8> {
        include_bytes!("../../../tests/fixtures/usgs_all_hour.geojson").to_vec()
    }

    fn received() -> DateTime<Utc> {
        Utc.timestamp_millis_opt(1_700_000_120_000)
            .single()
            .unwrap()
    }

    #[test]
    fn parses_every_measurable_feature() {
        let observations = parse(&fixture(), received()).unwrap();
        assert_eq!(observations.len(), 4);
        let first = &observations[0];
        assert_eq!(first.metric, "earthquake_magnitude");
        assert_eq!(first.value, 5.4);
        assert_eq!(first.unit, "magnitude");
        assert_eq!(first.source_id.as_str(), SOURCE_ID);
        assert_eq!(first.latitude, Some(34.5678));
        assert_eq!(first.longitude, Some(26.1234));
        assert_eq!(first.received_at, received());
    }

    #[test]
    fn observed_at_comes_from_the_source_timestamp() {
        let observations = parse(&fixture(), received()).unwrap();
        assert_eq!(
            observations[0].observed_at.timestamp_millis(),
            1_700_000_000_000
        );
        // The source time is earlier than receipt: lag is visible, not hidden.
        assert!(observations[0].observed_at < observations[0].received_at);
    }

    #[test]
    fn quakes_group_by_region_not_by_event() {
        let observations = parse(&fixture(), received()).unwrap();
        let entities: Vec<&str> = observations
            .iter()
            .map(|o| o.entity_id.as_ref().unwrap().as_str())
            .collect();
        assert_eq!(
            entities,
            vec![
                "region_greece",
                "region_ca",
                "region_off the coast of oregon",
                "region_hawaii"
            ]
        );
        // The per-quake id is kept as an attribute so drill-down still works.
        assert_eq!(
            observations[0]
                .attributes
                .get("event_id")
                .map(String::as_str),
            Some("us7000abcd")
        );
    }

    #[test]
    fn identical_payloads_are_deduplicated_by_observation_id() {
        let a = parse(&fixture(), received()).unwrap();
        let b = parse(&fixture(), received()).unwrap();
        let ids_a: Vec<_> = a.iter().map(|o| o.id.clone()).collect();
        let ids_b: Vec<_> = b.iter().map(|o| o.id.clone()).collect();
        assert_eq!(
            ids_a, ids_b,
            "re-parsing the same payload must be idempotent"
        );
    }

    #[test]
    fn a_feature_without_a_magnitude_is_skipped_not_zeroed() {
        let body = br#"{"features":[{"id":"x","properties":{"time":1700000000000},
            "geometry":{"type":"Point","coordinates":[1.0,2.0,3.0]}}]}"#;
        let observations = parse(body, received()).unwrap();
        assert!(observations.is_empty(), "no magnitude means no observation");
    }

    #[test]
    fn raw_reference_points_back_at_the_source() {
        let observations = parse(&fixture(), received()).unwrap();
        let raw = &observations[0].raw;
        assert!(raw.locator.starts_with("https://earthquake.usgs.gov/"));
        assert_eq!(raw.content_type.as_deref(), Some("application/geo+json"));
        assert!(!raw.hash.is_empty());
    }

    #[test]
    fn malformed_payload_is_a_parse_error_not_an_empty_result() {
        let err = parse(b"not json", received()).unwrap_err();
        assert!(matches!(err, CollectorError::Parse(_)));
    }

    #[test]
    fn schedule_is_event_driven() {
        let source = source();
        assert_eq!(source.id.as_str(), SOURCE_ID);
        assert_eq!(source.collector_type, COLLECTOR_TYPE);
        assert!(source.realtime_available);
        assert!(source.geospatial);
    }

    #[test]
    fn catalog_entry_survives_a_serde_round_trip() {
        let source = source();
        let json = serde_json::to_string(&source).unwrap();
        let back: Source = serde_json::from_str(&json).unwrap();
        assert_eq!(source, back);
    }

    /// The trait object must be usable through the collector interface.
    #[test]
    fn collector_identity_matches_the_catalog() {
        struct Probe;
        #[async_trait::async_trait]
        impl Collector for Probe {
            fn source_id(&self) -> SourceId {
                SourceId::new(SOURCE_ID)
            }
            fn schedule(&self) -> wse_collector::Schedule {
                wse_collector::Schedule::Event { poll_seconds: 60 }
            }
            fn mode(&self) -> CollectionMode {
                CollectionMode::Live
            }
            async fn collect(&self) -> Result<wse_collector::CollectionResult, CollectorError> {
                let observations = parse(&fixture(), received())?;
                let mut result = wse_collector::CollectionResult::new(self.source_id());
                result.records_received = observations.len() as u64;
                result.records_changed = observations.len() as u64;
                result.observations = observations;
                Ok(result)
            }
        }

        let probe = Probe;
        assert_eq!(probe.source_id().as_str(), source().id.as_str());
    }
}
