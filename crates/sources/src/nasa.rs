//! NASA near-Earth objects (NeoWs) — JSON, daily cadence, free.
//!
//! Space weather for rocks: the interesting signal is a change in the *rate*
//! of close approaches. One observation is one UTC day's count of close
//! approaches, not a single object's miss distance: the feed pools unrelated
//! rocks, so a distance series interleaves values that do not belong together,
//! and a symmetric detector would flag a *far* pass as anomalous. A count of
//! approaches per day is coherent; the day's closest object rides along as
//! attributes for drill-down. A day with no approaches is emitted as 0, because
//! "nothing came close today" is a measurement, not a missing value.
//!
//! API docs: <https://api.nasa.gov/> (the NEO feed lives under NeoWs).

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use wse_collector::CollectorError;
use wse_model::{EntityId, Observation, RawReference, Source, SourceId};

pub const SOURCE_ID: &str = "nasa_neo";
pub const COLLECTOR_TYPE: &str = "nasa_neo";

/// The number of close approaches the feed records on a given UTC day. The feed
/// is *itself* the close-approach feed, so every entry counts — the coherent
/// series is the day's traffic, not any single object's distance.
pub const METRIC: &str = "neo_close_approaches";
pub const UNIT: &str = "approaches";

/// The entity is the whole near-Earth-object population: individual rocks never
/// recur, so a per-object series could never accumulate.
pub const ENTITY: &str = "neo_class_all";

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
        entities: vec![ENTITY.to_string()],
        priority: 20,
        enabled: true,
        collector_type: COLLECTOR_TYPE.to_string(),
        parameters: std::collections::BTreeMap::new(),
        tier: wse_model::SourceTier::Tier1,
        measurement: wse_model::MeasurementSemantics::StableSeries,
        feeds_lenses: vec!["lens_space".to_string()],
        derivations: Vec::new(),
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
/// One observation per UTC day that appears in the feed: the count of close
/// approaches that day. The day's closest object rides along as attributes, so
/// the interesting near miss is still drill-downable without making it the
/// series. Keying on the day (not the object) means a day retained across polls
/// keeps one identity, and the count is a real per-day quantity.
pub fn parse(body: &[u8], received_at: DateTime<Utc>) -> Result<Vec<Observation>, CollectorError> {
    let feed: Feed = serde_json::from_slice(body)
        .map_err(|e| CollectorError::Parse(format!("nasa neo: {e}")))?;

    let source_id = SourceId::new(SOURCE_ID);
    let hash = wse_model::fnv1a_hex(&String::from_utf8_lossy(body));
    let bytes = body.len() as u64;

    // Group the feed's approaches by UTC day. Each entry is (id, name,
    // hazardous, distance_km, velocity_kps).
    struct Approach {
        id: String,
        name: String,
        hazardous: bool,
        distance_km: f64,
        velocity_kps: Option<String>,
    }
    let mut per_day: std::collections::BTreeMap<NaiveDate, Vec<Approach>> = Default::default();

    for (date, neos) in &feed.near_earth_objects {
        let Ok(day) = NaiveDate::parse_from_str(date, "%Y-%m-%d") else {
            continue;
        };
        for neo in neos {
            let Some(approach) = neo.close_approach_data.first() else {
                continue;
            };
            let Some(distance_km) = approach
                .miss_distance
                .as_ref()
                .and_then(|d| d.kilometers.parse::<f64>().ok())
            else {
                continue;
            };
            per_day.entry(day).or_default().push(Approach {
                id: neo.id.clone(),
                name: neo.name.clone(),
                hazardous: neo.is_potentially_hazardous_asteroid,
                distance_km,
                velocity_kps: approach
                    .relative_velocity
                    .as_ref()
                    .map(|v| v.kilometers_per_second.clone()),
            });
        }
    }

    let mut observations = Vec::with_capacity(per_day.len());
    for (day, approaches) in per_day {
        let observed_at = Utc.from_utc_datetime(&day.and_hms_opt(0, 0, 0).expect("midnight"));
        let raw = RawReference {
            locator: format!("nasa:neo:{}", day.format("%Y-%m-%d")),
            hash: hash.clone(),
            content_type: Some("application/json".to_string()),
            bytes: Some(bytes),
        };

        let mut observation = Observation::new(
            source_id.clone(),
            Some(EntityId::new(ENTITY)),
            METRIC,
            approaches.len() as f64,
            UNIT,
            observed_at,
            raw,
        )
        .with_received_at(received_at)
        // The day is the record: a day retained across polls keeps its id.
        .with_record_key(day.format("%Y-%m-%d").to_string());

        // Drill-down: the closest object that day, if the feed names one.
        if let Some(closest) = approaches
            .iter()
            .min_by(|a, b| a.distance_km.total_cmp(&b.distance_km))
        {
            observation = observation
                .with_attribute("closest_object_id", closest.id.clone())
                .with_attribute("closest_object_name", closest.name.clone())
                .with_attribute("closest_distance_km", format!("{:.0}", closest.distance_km))
                .with_attribute("closest_hazardous", closest.hazardous.to_string());
            if let Some(velocity) = &closest.velocity_kps {
                observation = observation.with_attribute("closest_velocity_kps", velocity.clone());
            }
        }
        observations.push(observation);
    }

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
    fn counts_close_approaches_per_day() {
        let observations = parse(&fixture(), received()).unwrap();
        // The fixture spans two UTC days: two approaches on 2023-11-15, one on
        // 2023-11-17.
        assert_eq!(observations.len(), 2);
        assert!(observations.iter().all(|o| o.metric == METRIC));
        assert!(observations.iter().all(|o| o.unit == UNIT));
        assert_eq!(observations[0].value, 2.0);
        assert_eq!(observations[1].value, 1.0);
    }

    #[test]
    fn one_observation_per_day_so_the_series_is_coherent() {
        let observations = parse(&fixture(), received()).unwrap();
        let keys: Vec<String> = observations.iter().map(|o| o.series_key()).collect();
        let first = keys.first().unwrap();
        assert!(
            keys.iter().all(|k| k == first),
            "every day must share one series so the count is comparable: {keys:?}"
        );
        // And no two observations share a timestamp: one point per day.
        let mut times: Vec<i64> = observations
            .iter()
            .map(|o| o.observed_at.timestamp())
            .collect();
        times.dedup();
        assert_eq!(times.len(), observations.len());
    }

    #[test]
    fn the_closest_object_is_kept_for_drill_down() {
        let observations = parse(&fixture(), received()).unwrap();
        // 2023-11-15's closest is (2023 WX1) at 6 013 000 km, a hazardous rock.
        let day = &observations[0];
        assert_eq!(
            day.attributes
                .get("closest_object_name")
                .map(String::as_str),
            Some("(2023 WX1)")
        );
        assert_eq!(
            day.attributes
                .get("closest_distance_km")
                .map(String::as_str),
            Some("6013000")
        );
        assert_eq!(
            day.attributes.get("closest_hazardous").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            day.attributes
                .get("closest_velocity_kps")
                .map(String::as_str),
            Some("8.02")
        );
    }

    #[test]
    fn a_day_retained_across_polls_keeps_its_id() {
        let a = parse(&fixture(), received()).unwrap();
        // A later poll whose feed still carries 2023-11-15 but adds a day.
        let later = br#"{"element_count":1,"near_earth_objects":{"2023-11-15":[{"id":"3542519","name":"542519 (2013 HV18)","is_potentially_hazardous_asteroid":false,"close_approach_data":[{"epoch_date_close_approach":1700000000000,"relative_velocity":{"kilometers_per_second":"6.10"},"miss_distance":{"kilometers":"25940000","lunar":"67.4"}}]}]}}"#;
        let b = parse(later, received()).unwrap();
        let first_a = a.iter().find(|o| o.value == 2.0).unwrap();
        let first_b = b.iter().find(|o| o.value == 1.0).unwrap();
        assert_eq!(
            first_a.id, first_b.id,
            "the same UTC day must keep one identity across polls"
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
