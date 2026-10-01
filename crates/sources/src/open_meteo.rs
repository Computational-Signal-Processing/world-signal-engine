//! Open-Meteo — global weather and air quality for a fixed city set.
//!
//! Open-Meteo is a free, keyless weather API with a genuinely global model
//! grid. Two products are tracked here:
//!
//! * **weather** — surface temperature and precipitation (`open_meteo_weather`);
//! * **air quality** — PM2.5 and PM10 particulates (`open_meteo_air_quality`).
//!
//! Both are the *same sensor network* asked two questions, and both are
//! independent of the official alert feeds (NWS): a shift in the measured
//! temperature or particulate level is a physical change, not a bulletin.
//!
//! ## Why a fixed city set, not a search
//!
//! The measurement is per city. The city list is fixed in code (below), so each
//! city is its own stable series — the same discipline as
//! `github_repo_universe`. A city's temperature is compared against that city's
//! own history, never pooled with another city's. Adding or removing a city is a
//! deliberate, reviewable edit, not churn in a query result.
//!
//! ## Why these two products and not the forecast
//!
//! A forecast is a prediction, and the brief is explicit that the engine
//! measures the world rather than predicting it (§39). The `current` block is
//! the measured-now value, which is what a baseline can be formed against.
//!
//! API docs: <https://open-meteo.com/en/docs>
//!
//! Authentication: none. License: Open-Meteo data is CC-BY 4.0; free for
//! non-commercial use.

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use wse_collector::CollectorError;
use wse_model::{EntityId, Observation, RawReference, Source, SourceId};

pub const WEATHER_SOURCE_ID: &str = "open_meteo_weather";
pub const AIR_SOURCE_ID: &str = "open_meteo_air_quality";
pub const COLLECTOR_TYPE: &str = "open_meteo_current";

pub const WEATHER_ENDPOINT: &str = "https://api.open-meteo.com/v1/forecast";
pub const AIR_ENDPOINT: &str = "https://air-quality-api.open-meteo.com/v1/air-quality";

/// A city in the fixed measurement set: `(slug, display name, country, lat, lon)`.
///
/// The set spans continents and climate zones so that "the world" is not one
/// hemisphere, and it includes Istanbul so the TURKEY lens has a weather sensor
/// inside its bounding box.
pub const CITIES: &[(&str, &str, &str, f64, f64)] = &[
    ("istanbul", "Istanbul", "TR", 41.01, 28.98),
    ("london", "London", "GB", 51.51, -0.13),
    ("new_york", "New York", "US", 40.71, -74.01),
    ("los_angeles", "Los Angeles", "US", 34.05, -118.24),
    ("mexico_city", "Mexico City", "MX", 19.43, -99.13),
    ("sao_paulo", "Sao Paulo", "BR", -23.55, -46.63),
    ("lagos", "Lagos", "NG", 6.52, 3.38),
    ("cairo", "Cairo", "EG", 30.04, 31.24),
    ("johannesburg", "Johannesburg", "ZA", -26.20, 28.05),
    ("moscow", "Moscow", "RU", 55.75, 37.62),
    ("delhi", "Delhi", "IN", 28.61, 77.21),
    ("beijing", "Beijing", "CN", 39.90, 116.41),
    ("singapore", "Singapore", "SG", 1.35, 103.82),
    ("tokyo", "Tokyo", "JP", 35.68, 139.69),
    ("sydney", "Sydney", "AU", -33.87, 151.21),
];

/// Build the request URL for a set of coordinates.
fn url(base: &str, fields: &str) -> String {
    let lats: Vec<String> = CITIES.iter().map(|c| format!("{:.2}", c.3)).collect();
    let lons: Vec<String> = CITIES.iter().map(|c| format!("{:.2}", c.4)).collect();
    format!(
        "{base}?latitude={}&longitude={}&current={fields}&timezone=UTC",
        lats.join(","),
        lons.join(",")
    )
}

/// The weather endpoint for the whole city set.
pub fn weather_url() -> String {
    url(WEATHER_ENDPOINT, "temperature_2m,precipitation")
}

/// The air-quality endpoint for the whole city set.
pub fn air_quality_url() -> String {
    url(AIR_ENDPOINT, "pm2_5,pm10")
}

/// The catalog entry for the weather product.
pub fn weather_source() -> Source {
    let mut parameters = std::collections::BTreeMap::new();
    parameters.insert("cities".to_string(), CITIES.len().to_string());
    Source {
        id: SourceId::new(WEATHER_SOURCE_ID),
        name: "Open-Meteo Weather".to_string(),
        provider: "Open-Meteo".to_string(),
        category: "weather".to_string(),
        subcategory: Some("surface_observation".to_string()),
        endpoint: WEATHER_ENDPOINT.to_string(),
        protocol: wse_model::Protocol::Https,
        format: wse_model::DataFormat::Json,
        cadence: wse_model::Cadence::Interval { seconds: 1800 },
        timezone: Some("UTC".to_string()),
        license: Some("CC-BY 4.0 (Open-Meteo)".to_string()),
        authentication: wse_model::AuthKind::None,
        cost: wse_model::Cost::Free,
        historical_available: true,
        realtime_available: true,
        geospatial: true,
        entities: vec!["weather".to_string()],
        priority: 22,
        enabled: true,
        collector_type: COLLECTOR_TYPE.to_string(),
        parameters,
        tier: wse_model::SourceTier::Tier2,
        measurement: wse_model::MeasurementSemantics::StableSeries,
        feeds_lenses: vec!["lens_earth".to_string(), "lens_agriculture".to_string()],
        derivations: Vec::new(),
    }
}

/// The catalog entry for the air-quality product.
pub fn air_quality_source() -> Source {
    let mut parameters = std::collections::BTreeMap::new();
    parameters.insert("cities".to_string(), CITIES.len().to_string());
    Source {
        id: SourceId::new(AIR_SOURCE_ID),
        name: "Open-Meteo Air Quality".to_string(),
        provider: "Open-Meteo / CAMS".to_string(),
        category: "environment".to_string(),
        subcategory: Some("air_quality".to_string()),
        endpoint: AIR_ENDPOINT.to_string(),
        protocol: wse_model::Protocol::Https,
        format: wse_model::DataFormat::Json,
        cadence: wse_model::Cadence::Interval { seconds: 3600 },
        timezone: Some("UTC".to_string()),
        license: Some("CC-BY 4.0 (Open-Meteo / Copernicus CAMS)".to_string()),
        authentication: wse_model::AuthKind::None,
        cost: wse_model::Cost::Free,
        historical_available: true,
        realtime_available: true,
        geospatial: true,
        entities: vec!["air_quality".to_string()],
        priority: 26,
        enabled: true,
        collector_type: COLLECTOR_TYPE.to_string(),
        parameters,
        tier: wse_model::SourceTier::Tier2,
        measurement: wse_model::MeasurementSemantics::StableSeries,
        feeds_lenses: vec!["lens_earth".to_string(), "lens_agriculture".to_string()],
        derivations: Vec::new(),
    }
}

/// One location block in an Open-Meteo multi-location response.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Location {
    #[serde(default)]
    pub latitude: f64,
    #[serde(default)]
    pub longitude: f64,
    /// The request index; absent for the first location.
    #[serde(default)]
    pub location_id: Option<usize>,
    #[serde(default)]
    pub current: Current,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Current {
    #[serde(default)]
    pub time: String,
    #[serde(default)]
    pub temperature_2m: Option<f64>,
    #[serde(default)]
    pub precipitation: Option<f64>,
    #[serde(default)]
    pub pm2_5: Option<f64>,
    #[serde(default)]
    pub pm10: Option<f64>,
}

/// One metric in a product: `(metric name, unit, reader)`.
type Metric = (&'static str, &'static str, fn(&Current) -> Option<f64>);

/// Parse a multi-location response. `emit` decides which metrics to read and
/// their units, so the same envelope serves both products.
fn parse_with(
    body: &[u8],
    received_at: DateTime<Utc>,
    entity: &str,
    metrics: &[Metric],
) -> Result<Vec<Observation>, CollectorError> {
    // A single-coordinate response is an object; a multi-coordinate one is an
    // array. Accept both so a one-city request still parses.
    let locations: Vec<Location> = match serde_json::from_slice::<serde_json::Value>(body) {
        Ok(serde_json::Value::Array(_)) => serde_json::from_slice(body)
            .map_err(|e| CollectorError::Parse(format!("open-meteo: {e}")))?,
        Ok(value @ serde_json::Value::Object(_)) => vec![serde_json::from_value(value)
            .map_err(|e| CollectorError::Parse(format!("open-meteo: {e}")))?],
        _ => return Err(CollectorError::Parse("open-meteo: not JSON".to_string())),
    };

    let source_id = SourceId::new(if entity == "air_quality" {
        AIR_SOURCE_ID
    } else {
        WEATHER_SOURCE_ID
    });
    let hash = wse_model::fnv1a_hex(&String::from_utf8_lossy(body));
    let bytes = body.len() as u64;
    let mut observations = Vec::new();

    for location in &locations {
        // The index maps back to the fixed city list; without it we cannot say
        // which city this is, so we skip rather than mislabel.
        let index = location.location_id.unwrap_or(0);
        let Some((slug, name, country, _, _)) = CITIES.get(index) else {
            continue;
        };
        let Some(observed_at) = parse_time(&location.current.time) else {
            continue;
        };
        for (metric, unit, read) in metrics {
            let Some(value) = read(&location.current) else {
                continue;
            };
            let raw = RawReference {
                locator: format!("{WEATHER_ENDPOINT}#{slug}"),
                hash: hash.clone(),
                content_type: Some("application/json".to_string()),
                bytes: Some(bytes),
            };
            observations.push(
                Observation::new(
                    source_id.clone(),
                    Some(EntityId::new(entity)),
                    *metric,
                    value,
                    *unit,
                    observed_at,
                    raw,
                )
                .with_received_at(received_at)
                // The city is the record identity and the series dimension:
                // stable across polls, and each city its own baseline.
                .with_record_key(*slug)
                .with_dimension("city", *slug)
                .with_attribute("city", *name)
                .with_attribute("country", *country)
                .with_location(location.latitude, location.longitude),
            );
        }
    }

    if observations.is_empty() {
        return Err(CollectorError::Parse(
            "open-meteo: no readable locations".to_string(),
        ));
    }
    observations.sort_by_key(|a| a.series_key());
    Ok(observations)
}

/// Parse a weather payload into temperature and precipitation observations.
pub fn parse_weather(
    body: &[u8],
    received_at: DateTime<Utc>,
) -> Result<Vec<Observation>, CollectorError> {
    parse_with(
        body,
        received_at,
        "weather",
        &[
            ("temperature_2m", "°C", |c| c.temperature_2m),
            ("precipitation", "mm", |c| c.precipitation),
        ],
    )
}

/// Parse an air-quality payload into PM2.5 and PM10 observations.
pub fn parse_air_quality(
    body: &[u8],
    received_at: DateTime<Utc>,
) -> Result<Vec<Observation>, CollectorError> {
    parse_with(
        body,
        received_at,
        "air_quality",
        &[
            ("pm2_5", "µg/m³", |c| c.pm2_5),
            ("pm10", "µg/m³", |c| c.pm10),
        ],
    )
}

/// Parse Open-Meteo's local-time string (`YYYY-MM-DDTHH:MM`, UTC because we ask
/// for `timezone=UTC`).
fn parse_time(raw: &str) -> Option<DateTime<Utc>> {
    NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M")
        .ok()
        .map(|t| t.and_utc())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn received() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-01T11:05:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn weather_fixture() -> Vec<u8> {
        include_bytes!("../../../tests/fixtures/open_meteo_weather.json").to_vec()
    }

    fn air_fixture() -> Vec<u8> {
        include_bytes!("../../../tests/fixtures/open_meteo_air_quality.json").to_vec()
    }

    #[test]
    fn parses_temperature_and_precipitation() {
        let observations = parse_weather(&weather_fixture(), received()).unwrap();
        let temp = observations
            .iter()
            .find(|o| o.metric == "temperature_2m")
            .unwrap();
        assert_eq!(temp.unit, "°C");
        assert_eq!(
            temp.dimensions.get("city").map(String::as_str),
            Some("istanbul")
        );
        assert!(temp.value > -60.0 && temp.value < 60.0);
    }

    #[test]
    fn the_measured_time_is_the_observation_time() {
        let observations = parse_weather(&weather_fixture(), received()).unwrap();
        // The fixture's `current.time` is 2026-10-01T11:00.
        assert_eq!(
            observations[0].observed_at.to_rfc3339(),
            "2026-10-01T11:00:00+00:00"
        );
    }

    #[test]
    fn each_city_is_its_own_series() {
        // A single-coordinate response has one city; two metrics stay distinct.
        let observations = parse_weather(&weather_fixture(), received()).unwrap();
        let keys: std::collections::HashSet<String> =
            observations.iter().map(|o| o.series_key()).collect();
        assert_eq!(keys.len(), observations.len());
        assert!(keys.iter().any(|k| k.contains("city=istanbul")));
    }

    #[test]
    fn the_same_city_across_polls_shares_one_series() {
        let a = parse_weather(&weather_fixture(), received()).unwrap();
        let b = parse_weather(&weather_fixture(), received()).unwrap();
        assert_eq!(a[0].series_key(), b[0].series_key());
        assert_eq!(a[0].id, b[0].id, "the same record de-duplicates");
    }

    #[test]
    fn air_quality_reads_particulates() {
        let observations = parse_air_quality(&air_fixture(), received()).unwrap();
        let pm = observations.iter().find(|o| o.metric == "pm2_5").unwrap();
        assert_eq!(pm.unit, "µg/m³");
        assert_eq!(
            pm.entity_id.as_ref().map(|e| e.as_str()),
            Some("air_quality")
        );
    }

    #[test]
    fn observations_carry_a_location_for_the_map() {
        let observations = parse_weather(&weather_fixture(), received()).unwrap();
        assert!(observations[0].latitude.is_some());
        assert!(observations[0].longitude.is_some());
    }

    #[test]
    fn malformed_payload_is_a_parse_error() {
        assert!(matches!(
            parse_weather(b"nope", received()),
            Err(CollectorError::Parse(_))
        ));
    }

    #[test]
    fn the_city_set_includes_turkey() {
        // The TURKEY lens needs a weather sensor inside its box.
        assert!(CITIES.iter().any(|c| c.2 == "TR"));
    }
}
