//! NASA near-Earth objects (NeoWs) — JSON, daily cadence, free.
//!
//! Space weather for rocks: the interesting signal is a change in the *rate*
//! of close approaches, or a single unusually close or unusually large object.
//!
//! API docs: <https://api.nasa.gov/> (the NEO feed lives under NeoWs).

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use wse_collector::CollectorError;
use wse_model::{EntityId, Observation, RawReference, Source, SourceId};

pub const SOURCE_ID: &str = "nasa_neo";
pub const COLLECTOR_TYPE: &str = "nasa_neo";

/// A seven-day window ending today; the API takes explicit dates.
pub const FEED_ENDPOINT: &str = "https://api.nasa.gov/neo/rest/v1/feed";

/// The catalog entry.
pub fn source() -> Source {
    Source {
        id: SourceId::new(SOURCE_ID),
        name: "NASA Near-Earth Object Feed".to_string(),
        provider: "NASA / JPL".to_string(),
        category: "space".to_string(),
        subcategory: Some("near_earth_objects".to_string()),
        endpoint: FEED_ENDPOINT.to_string(),
        protocol: wse_model::Protocol::Https,
        format: wse_model::DataFormat::Json,
        cadence: wse_model::Cadence::Daily { hour_utc: 6 },
        timezone: Some("UTC".to_string()),
        license: Some("Public domain (NASA)".to_string()),
        authentication: wse_model::AuthKind::ApiKey,
        cost: wse_model::Cost::FreeWithRegistration,
        historical_available: true,
        realtime_available: false,
        geospatial: false,
        entities: vec!["near_earth_object".to_string()],
        priority: 20,
        enabled: true,
        collector_type: COLLECTOR_TYPE.to_string(),
        parameters: std::collections::BTreeMap::new(),
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Feed {
    #[serde(default)]
    pub element_count: usize,
    #[serde(default)]
    pub near_earth_objects: std::collections::BTreeMap<String, Vec<Neo>>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Neo {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub estimated_diameter: Option<Diameter>,
    #[serde(default)]
    pub is_potentially_hazardous_asteroid: bool,
    #[serde(default)]
    pub close_approach_data: Vec<Approach>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Diameter {
    pub kilometers: Option<DiameterRange>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct DiameterRange {
    pub estimated_diameter_min: f64,
    pub estimated_diameter_max: f64,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Approach {
    /// Epoch milliseconds of closest approach.
    pub epoch_date_close_approach: Option<i64>,
    pub relative_velocity: Option<Velocity>,
    pub miss_distance: Option<MissDistance>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Velocity {
    /// The API sends this as a string.
    pub kilometers_per_second: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct MissDistance {
    pub kilometers: String,
    pub lunar: String,
}

/// Parse a NeoWs feed into observations.
///
/// One observation per close approach, keyed on the *miss distance* — the
/// quantity whose distribution actually changes when the neighbourhood gets
/// busier. Diameter and velocity ride along as dimensions.
pub fn parse(body: &[u8], received_at: DateTime<Utc>) -> Result<Vec<Observation>, CollectorError> {
    let feed: Feed = serde_json::from_slice(body)
        .map_err(|e| CollectorError::Parse(format!("nasa neo: {e}")))?;

    let source_id = SourceId::new(SOURCE_ID);
    let hash = wse_model::fnv1a_hex(&String::from_utf8_lossy(body));
    let mut observations = Vec::new();

    for (date, neos) in &feed.near_earth_objects {
        for neo in neos {
            let Some(approach) = neo.close_approach_data.first() else {
                continue;
            };
            let Some(epoch_ms) = approach.epoch_date_close_approach else {
                continue;
            };
            let Some(observed_at) = Utc.timestamp_millis_opt(epoch_ms).single() else {
                continue;
            };
            let Some(distance_km) = approach
                .miss_distance
                .as_ref()
                .and_then(|d| d.kilometers.parse::<f64>().ok())
            else {
                continue;
            };

            let raw = RawReference {
                locator: format!("nasa:neo:{}", neo.id),
                hash: hash.clone(),
                content_type: Some("application/json".to_string()),
                bytes: Some(body.len() as u64),
            };

            // The entity is the object class, not the object: individual rocks
            // never recur, so a per-object series could never accumulate.
            let entity = EntityId::new("neo_class_all");
            let mut observation = Observation::new(
                source_id.clone(),
                Some(entity),
                "neo_miss_distance",
                distance_km,
                "km",
                observed_at,
                raw,
            )
            .with_received_at(received_at)
            .with_attribute("object_id", neo.id.clone())
            .with_attribute("object_name", neo.name.clone())
            .with_attribute("approach_date", date.clone())
            .with_attribute(
                "hazardous",
                neo.is_potentially_hazardous_asteroid.to_string(),
            );

            if let Some(diameter) = neo
                .estimated_diameter
                .as_ref()
                .and_then(|d| d.kilometers.as_ref())
            {
                let mean =
                    (diameter.estimated_diameter_min + diameter.estimated_diameter_max) / 2.0;
                observation = observation.with_attribute("diameter_km", format!("{mean:.3}"));
            }
            if let Some(velocity) = approach.relative_velocity.as_ref() {
                observation = observation
                    .with_attribute("velocity_kps", velocity.kilometers_per_second.clone());
            }
            if let Some(lunar) = approach.miss_distance.as_ref() {
                observation =
                    observation.with_attribute("miss_distance_lunar", lunar.lunar.clone());
            }

            observations.push(observation);
        }
    }

    // Stable ordering: BTreeMap iteration is by date, but within a date the
    // API order is not guaranteed.
    observations.sort_by_key(|o| o.observed_at);
    Ok(observations)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        include_bytes!("../../../tests/fixtures/nasa_neo_feed.json").to_vec()
    }

    fn received() -> DateTime<Utc> {
        Utc.timestamp_millis_opt(1_700_010_000_000)
            .single()
            .unwrap()
    }

    #[test]
    fn parses_every_close_approach() {
        let observations = parse(&fixture(), received()).unwrap();
        assert_eq!(observations.len(), 3);
        assert_eq!(observations[0].metric, "neo_miss_distance");
        assert_eq!(observations[0].unit, "km");
        // The earliest approach in the fixture, by close-approach epoch.
        assert_eq!(observations[0].value, 25_940_000.0);
    }

    #[test]
    fn observations_are_ordered_by_approach_time() {
        let observations = parse(&fixture(), received()).unwrap();
        let times: Vec<i64> = observations
            .iter()
            .map(|o| o.observed_at.timestamp_millis())
            .collect();
        let mut sorted = times.clone();
        sorted.sort_unstable();
        assert_eq!(times, sorted);
    }

    #[test]
    fn hazards_and_dimensions_are_preserved_for_drill_down() {
        let observations = parse(&fixture(), received()).unwrap();
        let hazardous = observations
            .iter()
            .find(|o| o.attributes.get("hazardous").map(String::as_str) == Some("true"))
            .expect("fixture contains a hazardous object");
        assert_eq!(
            hazardous.attributes.get("object_name").map(String::as_str),
            Some("(2023 WX1)")
        );
        assert!(hazardous.attributes.contains_key("diameter_km"));
        assert_eq!(
            hazardous.attributes.get("velocity_kps").map(String::as_str),
            Some("8.02")
        );
    }

    #[test]
    fn all_objects_share_one_series_so_rate_can_be_measured() {
        let observations = parse(&fixture(), received()).unwrap();
        let keys: Vec<String> = observations.iter().map(|o| o.series_key()).collect();
        let first = keys.first().unwrap();
        assert!(
            keys.iter().all(|k| k == first),
            "close approaches must accumulate into one series: {keys:?}"
        );
    }

    #[test]
    fn malformed_payload_is_a_parse_error() {
        // A JSON array is valid JSON but not a NeoWs feed object.
        assert!(matches!(
            parse(br#"{"near_earth_objects":"oops"}"#, received()),
            Err(CollectorError::Parse(_))
        ));
    }

    #[test]
    fn catalog_entry_declares_a_key_and_a_daily_cadence() {
        let source = source();
        assert_eq!(source.authentication, wse_model::AuthKind::ApiKey);
        assert_eq!(source.cadence, wse_model::Cadence::Daily { hour_utc: 6 });
        assert!(source.license.is_some());
    }
}
