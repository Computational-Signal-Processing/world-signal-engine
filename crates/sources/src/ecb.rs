//! ECB exchange rates — an official daily reference rate series.
//!
//! The European Central Bank publishes daily euro reference rates. This source
//! tracks the USD/EUR rate as a stable, institutional financial series: an
//! abnormal move is a real market change, and it is an independent sensor for
//! the FINANCE and TURKEY lenses.
//!
//! Data: <https://data.ecb.europa.eu/> (SDMX 2.1 REST API)
//!
//! Authentication: none. License: ECB data terms; free reuse with attribution.

use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use wse_collector::CollectorError;
use wse_model::{EntityId, Observation, RawReference, Source, SourceId};

pub const SOURCE_ID: &str = "ecb_exchange_rates";
pub const COLLECTOR_TYPE: &str = "ecb_sdmx_series";

/// USD/EUR daily reference rate, SDMX key `D.USD.EUR.SP00.A`.
pub const API_ENDPOINT: &str =
    "https://data-api.ecb.europa.eu/service/data/EXR/D.USD.EUR.SP00.A?format=jsondata&lastNObservations=7";

/// The catalog entry.
pub fn source() -> Source {
    let mut parameters = BTreeMap::new();
    parameters.insert("series".to_string(), "D.USD.EUR.SP00.A".to_string());
    parameters.insert("last_n".to_string(), "7".to_string());

    Source {
        id: SourceId::new(SOURCE_ID),
        name: "ECB Euro Reference Rates".to_string(),
        provider: "European Central Bank".to_string(),
        category: "finance".to_string(),
        subcategory: Some("exchange_rate".to_string()),
        endpoint: API_ENDPOINT.to_string(),
        protocol: wse_model::Protocol::Https,
        format: wse_model::DataFormat::Json,
        cadence: wse_model::Cadence::Interval { seconds: 86_400 },
        timezone: Some("UTC".to_string()),
        license: Some("ECB data terms; free reuse with attribution".to_string()),
        authentication: wse_model::AuthKind::None,
        cost: wse_model::Cost::Free,
        historical_available: true,
        realtime_available: false,
        geospatial: false,
        entities: vec!["usd_eur".to_string()],
        priority: 44,
        enabled: true,
        collector_type: COLLECTOR_TYPE.to_string(),
        parameters,
        tier: wse_model::SourceTier::Tier1,
        measurement: wse_model::MeasurementSemantics::StableSeries,
        feeds_lenses: vec!["lens_finance".to_string()],
    }
}

/// A minimal SDMX-JSON envelope: the time axis plus the series values.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SdmxJson {
    #[serde(default, rename = "dataSets")]
    pub data_sets: Vec<DataSet>,
    #[serde(default)]
    pub structure: Option<Structure>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct DataSet {
    #[serde(default)]
    pub series: BTreeMap<String, Series>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Series {
    /// Observation index (as a string) mapped to a value array whose first
    /// element is the measurement.
    #[serde(default)]
    pub observations: BTreeMap<String, Vec<Option<f64>>>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Structure {
    #[serde(default)]
    pub dimensions: Dimensions,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Dimensions {
    #[serde(default)]
    pub observation: Vec<Dimension>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Dimension {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub values: Vec<DimensionValue>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct DimensionValue {
    #[serde(default)]
    pub id: String,
}

/// Parse an ECB SDMX-JSON response into observations.
///
/// Each observation's timestamp comes from the SDMX time axis, so the series is
/// the real reference-rate history rather than collection times.
pub fn parse(body: &[u8], received_at: DateTime<Utc>) -> Result<Vec<Observation>, CollectorError> {
    let envelope: SdmxJson = serde_json::from_slice(body)
        .map_err(|e| CollectorError::Parse(format!("ecb sdmx: {e}")))?;

    let times: Vec<String> = envelope
        .structure
        .as_ref()
        .and_then(|s| s.dimensions.observation.first())
        .map(|d| d.values.iter().map(|v| v.id.clone()).collect())
        .unwrap_or_default();
    if times.is_empty() {
        return Err(CollectorError::Parse("ecb: no time dimension".to_string()));
    }

    let source_id = SourceId::new(SOURCE_ID);
    let entity = EntityId::new("fx_usd_eur");
    let hash = wse_model::fnv1a_hex(&String::from_utf8_lossy(body));
    let mut observations = Vec::new();

    for dataset in &envelope.data_sets {
        for series in dataset.series.values() {
            for (index, values) in &series.observations {
                let Some(value) = values.first().copied().flatten() else {
                    continue;
                };
                let Ok(index) = index.parse::<usize>() else {
                    continue;
                };
                let Some(date) = times.get(index) else {
                    continue;
                };
                let Some(observed_at) = parse_date(date) else {
                    continue;
                };
                let raw = RawReference {
                    locator: format!("{API_ENDPOINT}#{date}"),
                    hash: hash.clone(),
                    content_type: Some("application/vnd.sdmx.data+json".to_string()),
                    bytes: Some(body.len() as u64),
                };
                observations.push(
                    Observation::new(
                        source_id.clone(),
                        Some(entity.clone()),
                        "exchange_rate",
                        value,
                        "rate",
                        observed_at,
                        raw,
                    )
                    .with_received_at(received_at)
                    // The SDMX period is the observation's natural key: a
                    // rolling `lastNObservations` window re-fetches the same
                    // days, and each day must keep one id.
                    .with_record_key(date)
                    .with_attribute("pair", "USD/EUR".to_string())
                    .with_attribute("period", date.clone()),
                );
            }
        }
    }

    observations.sort_by_key(|o| o.observed_at);
    Ok(observations)
}

/// Parse an SDMX daily period (`YYYY-MM-DD`) as UTC midnight.
fn parse_date(raw: &str) -> Option<DateTime<Utc>> {
    NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .ok()
        .map(|d| d.and_hms_opt(0, 0, 0).unwrap().and_utc())
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
        include_bytes!("../../../tests/fixtures/ecb_exr.json").to_vec()
    }

    #[test]
    fn parses_the_rate_series() {
        let observations = parse(&fixture(), received()).unwrap();
        assert_eq!(observations.len(), 3);
        assert_eq!(observations[0].metric, "exchange_rate");
        assert_eq!(observations[0].unit, "rate");
        assert_eq!(observations[0].value, 1.1403);
    }

    #[test]
    fn timestamps_come_from_the_time_axis() {
        let observations = parse(&fixture(), received()).unwrap();
        assert_eq!(
            observations[0].observed_at.to_rfc3339(),
            "2026-09-28T00:00:00+00:00"
        );
    }

    #[test]
    fn every_point_shares_one_series() {
        let observations = parse(&fixture(), received()).unwrap();
        let keys: Vec<String> = observations.iter().map(|o| o.series_key()).collect();
        assert!(keys.iter().all(|k| k == &keys[0]));
    }

    #[test]
    fn a_response_without_a_time_axis_is_a_parse_error() {
        assert!(matches!(
            parse(br#"{"dataSets":[],"structure":null}"#, received()),
            Err(CollectorError::Parse(_))
        ));
    }

    #[test]
    fn the_catalog_is_institutional_finance() {
        let source = source();
        assert_eq!(source.category, "finance");
        assert_eq!(source.tier, wse_model::SourceTier::Tier1);
        assert!(source.feeds_lenses.contains(&"lens_finance".to_string()));
    }
}
