//! NOAA SWPC planetary K-index — geomagnetic activity.
//!
//! The Kp index is the standard global measure of geomagnetic disturbance: a
//! rise means a geomagnetic storm, which affects satellites, power grids and
//! radio. It is a real, official measurement, not a news proxy.
//!
//! The product feed returns a rolling window of 3-hourly Kp values. The latest
//! value is the observation; the time series is the Kp history.
//!
//! Product: <https://services.swpc.noaa.gov/products/noaa-planetary-k-index.json>
//!
//! Authentication: none. License: public domain (US Government / NOAA).

use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use wse_collector::CollectorError;
use wse_model::{EntityId, Observation, RawReference, Source, SourceId};

pub const SOURCE_ID: &str = "noaa_kp_index";
pub const COLLECTOR_TYPE: &str = "noaa_kp_index";

/// The planetary K-index product.
pub const API_ENDPOINT: &str =
    "https://services.swpc.noaa.gov/products/noaa-planetary-k-index.json";

/// The catalog entry.
pub fn source() -> Source {
    let mut parameters = BTreeMap::new();
    parameters.insert("product".to_string(), "planetary_k_index".to_string());

    Source {
        id: SourceId::new(SOURCE_ID),
        name: "NOAA Planetary K-index".to_string(),
        provider: "NOAA Space Weather Prediction Center".to_string(),
        category: "space".to_string(),
        subcategory: Some("geomagnetic_activity".to_string()),
        endpoint: API_ENDPOINT.to_string(),
        protocol: wse_model::Protocol::Https,
        format: wse_model::DataFormat::Json,
        cadence: wse_model::Cadence::Interval { seconds: 3600 },
        timezone: Some("UTC".to_string()),
        license: Some("Public domain (US Government)".to_string()),
        authentication: wse_model::AuthKind::None,
        cost: wse_model::Cost::Free,
        historical_available: true,
        realtime_available: true,
        geospatial: false,
        entities: vec!["space_weather".to_string()],
        priority: 25,
        enabled: true,
        collector_type: COLLECTOR_TYPE.to_string(),
        parameters,
        tier: wse_model::SourceTier::Tier1,
        measurement: wse_model::MeasurementSemantics::StableSeries,
        feeds_lenses: vec!["lens_space".to_string()],
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct KpPoint {
    #[serde(default, rename = "time_tag")]
    pub time_tag: String,
    #[serde(default, rename = "Kp")]
    pub kp: f64,
    #[serde(default)]
    pub a_running: Option<f64>,
    #[serde(default)]
    pub station_count: Option<f64>,
}

/// Parse the Kp product into observations.
///
/// One observation per 3-hourly point. The timestamp is the point's own
/// `time_tag` (UTC, no zone suffix), so the series is the real Kp history
/// rather than a sequence of collection times.
pub fn parse(body: &[u8], received_at: DateTime<Utc>) -> Result<Vec<Observation>, CollectorError> {
    let points: Vec<KpPoint> =
        serde_json::from_slice(body).map_err(|e| CollectorError::Parse(format!("noaa kp: {e}")))?;

    let source_id = SourceId::new(SOURCE_ID);
    let entity = EntityId::new("geomagnetic_kp");
    let hash = wse_model::fnv1a_hex(&String::from_utf8_lossy(body));
    let mut observations = Vec::new();

    for point in points {
        let Some(observed_at) = parse_time_tag(&point.time_tag) else {
            continue;
        };
        let raw = RawReference {
            locator: format!("{API_ENDPOINT}#{}", point.time_tag),
            hash: hash.clone(),
            content_type: Some("application/json".to_string()),
            bytes: Some(body.len() as u64),
        };
        let mut observation = Observation::new(
            source_id.clone(),
            Some(entity.clone()),
            "kp_index",
            point.kp,
            "kp",
            observed_at,
            raw,
        )
        .with_received_at(received_at)
        // The point's own `time_tag` is its natural key: the product is a
        // rolling window, so a retained 3-hourly point must keep one id.
        .with_record_key(&point.time_tag)
        .with_attribute("time_tag", point.time_tag.clone());
        if let Some(a) = point.a_running {
            observation = observation.with_attribute("a_running", format!("{a:.0}"));
        }
        if let Some(c) = point.station_count {
            observation = observation.with_attribute("station_count", format!("{c:.0}"));
        }
        observations.push(observation);
    }

    observations.sort_by_key(|o| o.observed_at);
    Ok(observations)
}

/// Parse the feed's `YYYY-MM-DDTHH:MM:SS` (no zone) as UTC.
fn parse_time_tag(raw: &str) -> Option<DateTime<Utc>> {
    NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S")
        .ok()
        .map(|n| n.and_utc())
        .or_else(|| {
            DateTime::parse_from_rfc3339(raw)
                .ok()
                .map(|d| d.with_timezone(&Utc))
        })
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
        include_bytes!("../../../tests/fixtures/noaa_kp_index.json").to_vec()
    }

    #[test]
    fn parses_every_kp_point() {
        let observations = parse(&fixture(), received()).unwrap();
        assert_eq!(observations.len(), 3);
        assert_eq!(observations[0].metric, "kp_index");
        assert_eq!(observations[0].unit, "kp");
        assert_eq!(observations[0].value, 3.0);
    }

    #[test]
    fn observed_at_comes_from_the_point_timestamp() {
        let observations = parse(&fixture(), received()).unwrap();
        assert_eq!(
            observations[0].observed_at.to_rfc3339(),
            "2026-09-24T00:00:00+00:00"
        );
        assert!(observations[0].observed_at < observations[0].received_at);
    }

    #[test]
    fn all_points_share_one_series() {
        let observations = parse(&fixture(), received()).unwrap();
        let keys: Vec<String> = observations.iter().map(|o| o.series_key()).collect();
        assert!(keys.iter().all(|k| k == &keys[0]));
    }

    #[test]
    fn malformed_payload_is_a_parse_error() {
        assert!(matches!(
            parse(b"{\"not\":\"a list\"}", received()),
            Err(CollectorError::Parse(_))
        ));
    }

    #[test]
    fn the_catalog_is_institutional_space() {
        let source = source();
        assert_eq!(source.category, "space");
        assert_eq!(source.tier, wse_model::SourceTier::Tier1);
        assert!(source.feeds_lenses.contains(&"lens_space".to_string()));
    }
}
