//! NOAA GOES X-ray flux — solar flares.
//!
//! The GOES satellites measure solar X-ray flux continuously, and the
//! background flux is the standard indicator of solar activity: a jump is a
//! solar flare, which affects radio, satellites and power grids. This is the
//! second independent space-weather sensor alongside `noaa_kp_index` (Kp is
//! geomagnetic disturbance *at Earth*; the X-ray flux is the solar driver), so
//! SPACE has two genuinely independent measurements and convergence has real
//! material.
//!
//! The feed is a rolling window of 1-minute flux points. The measurement is the
//! *latest* flux in the 0.1–0.8 nm channel, timestamped at the point's own time
//! so the series is the real flux history.
//!
//! Product: <https://services.swpc.noaa.gov/json/goes/primary/xrays-1-day.json>
//!
//! Authentication: none. License: public domain (US Government / NOAA).

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use wse_collector::CollectorError;
use wse_model::{EntityId, Observation, RawReference, Source, SourceId};

pub const SOURCE_ID: &str = "noaa_goes_xray";
pub const COLLECTOR_TYPE: &str = "noaa_goes_xray";

/// The 1-minute X-ray flux product.
pub const API_ENDPOINT: &str = "https://services.swpc.noaa.gov/json/goes/primary/xrays-1-day.json";

/// The long-wavelength channel: the standard solar-flare indicator.
const CHANNEL: &str = "0.1-0.8nm";

/// The catalog entry.
pub fn source() -> Source {
    let mut parameters = BTreeMap::new();
    parameters.insert("product".to_string(), "goes_xray_flux".to_string());
    parameters.insert("channel".to_string(), CHANNEL.to_string());

    Source {
        id: SourceId::new(SOURCE_ID),
        name: "NOAA GOES X-ray Flux".to_string(),
        provider: "NOAA Space Weather Prediction Center".to_string(),
        category: "space".to_string(),
        subcategory: Some("solar_activity".to_string()),
        endpoint: API_ENDPOINT.to_string(),
        protocol: wse_model::Protocol::Https,
        format: wse_model::DataFormat::Json,
        cadence: wse_model::Cadence::Interval { seconds: 900 },
        timezone: Some("UTC".to_string()),
        license: Some("Public domain (US Government)".to_string()),
        authentication: wse_model::AuthKind::None,
        cost: wse_model::Cost::Free,
        historical_available: true,
        realtime_available: true,
        geospatial: false,
        entities: vec!["space_weather".to_string()],
        priority: 27,
        enabled: true,
        collector_type: COLLECTOR_TYPE.to_string(),
        parameters,
        tier: wse_model::SourceTier::Tier1,
        measurement: wse_model::MeasurementSemantics::StableSeries,
        feeds_lenses: vec!["lens_space".to_string()],
        derivations: Vec::new(),
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct XrayPoint {
    #[serde(default)]
    pub time_tag: String,
    #[serde(default)]
    pub flux: f64,
    #[serde(default)]
    pub energy: String,
}

/// Parse the X-ray product into observations.
///
/// Emits one observation per point in the long-wavelength channel, timestamped
/// at the point's own `time_tag`. The short channel is ignored: it is a
/// different measurement, and mixing the two would make one series out of two
/// quantities.
pub fn parse(body: &[u8], received_at: DateTime<Utc>) -> Result<Vec<Observation>, CollectorError> {
    let points: Vec<XrayPoint> = serde_json::from_slice(body)
        .map_err(|e| CollectorError::Parse(format!("goes xray: {e}")))?;

    let source_id = SourceId::new(SOURCE_ID);
    let entity = EntityId::new("space_weather");
    let hash = wse_model::fnv1a_hex(&String::from_utf8_lossy(body));
    let bytes = body.len() as u64;
    let mut observations = Vec::new();

    for point in &points {
        if point.energy != CHANNEL {
            continue;
        }
        let Some(observed_at) = parse_time(&point.time_tag) else {
            continue;
        };
        let raw = RawReference {
            locator: format!("{API_ENDPOINT}#{}", point.time_tag),
            hash: hash.clone(),
            content_type: Some("application/json".to_string()),
            bytes: Some(bytes),
        };
        observations.push(
            Observation::new(
                source_id.clone(),
                Some(entity.clone()),
                "xray_flux",
                point.flux,
                "W/m^2",
                observed_at,
                raw,
            )
            .with_received_at(received_at)
            // The timestamp is the record's natural key: the rolling window
            // re-serves the same minutes, and each minute keeps one id.
            .with_record_key(point.time_tag.clone())
            .with_attribute("channel", CHANNEL.to_string()),
        );
    }

    if observations.is_empty() {
        return Err(CollectorError::Parse(
            "goes xray: no points in the long channel".to_string(),
        ));
    }
    observations.sort_by_key(|o| o.observed_at);
    Ok(observations)
}

fn parse_time(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn received() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-01T11:05:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn fixture() -> Vec<u8> {
        include_bytes!("../../../tests/fixtures/noaa_goes_xray.json").to_vec()
    }

    #[test]
    fn parses_flux_points_in_the_long_channel() {
        let observations = parse(&fixture(), received()).unwrap();
        assert!(!observations.is_empty());
        assert!(observations.iter().all(|o| o.metric == "xray_flux"));
        assert!(observations.iter().all(|o| o.unit == "W/m^2"));
    }

    #[test]
    fn points_share_one_series_and_are_ordered() {
        let observations = parse(&fixture(), received()).unwrap();
        let keys: std::collections::HashSet<String> =
            observations.iter().map(|o| o.series_key()).collect();
        assert_eq!(keys.len(), 1, "one sensor, one series");
        let times: Vec<_> = observations.iter().map(|o| o.observed_at).collect();
        assert!(times.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn the_same_minute_keeps_its_identity_across_polls() {
        let a = parse(&fixture(), received()).unwrap();
        let b = parse(&fixture(), received()).unwrap();
        assert_eq!(a[0].id, b[0].id);
    }

    #[test]
    fn malformed_payload_is_a_parse_error() {
        assert!(matches!(
            parse(b"nope", received()),
            Err(CollectorError::Parse(_))
        ));
    }

    #[test]
    fn a_payload_with_only_the_short_channel_is_a_parse_error() {
        let body = br#"[{"time_tag":"2026-10-01T11:00:00Z","flux":1e-9,"energy":"0.05-0.4nm"}]"#;
        assert!(matches!(
            parse(body, received()),
            Err(CollectorError::Parse(_))
        ));
    }

    #[test]
    fn catalog_is_institutional_space() {
        let source = source();
        assert_eq!(source.category, "space");
        assert_eq!(source.tier, wse_model::SourceTier::Tier1);
        assert!(source.feeds_lenses.contains(&"lens_space".to_string()));
    }
}
