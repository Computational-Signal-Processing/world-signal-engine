//! PyPI package downloads — a fixed universe of major Python packages.
//!
//! The Python counterpart to `npm_downloads`: download volume is a direct
//! measurement of software *usage*, independent of GitHub attention and news.
//! Together the two give the SOFTWARE lens four independent sensors (stars,
//! discussion, and two download-volume series), which is what makes convergence
//! meaningful rather than decorative.
//!
//! ## Why a fixed package set, not a search
//!
//! The measurement is per package, and the set is fixed in code (below), so each
//! package is its own stable series.
//!
//! ## Why the recent total and not a single day
//!
//! `pypistats` `recent` returns `last_day`, `last_week` and `last_month`. The
//! `last_week` total is the stable, weekday-insensitive quantity, so that is
//! what the series carries; `last_day` is attached as a drill-down attribute for
//! investigation but is not the detection series.
//!
//! API docs: <https://pypistats.org/api/>
//!
//! Authentication: none. License: PyPI stats are free to use.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use wse_collector::CollectorError;
use wse_model::{EntityId, Observation, RawReference, Source, SourceId};

pub const SOURCE_ID: &str = "pypi_downloads";
pub const COLLECTOR_TYPE: &str = "pypi_downloads";

/// The recent-downloads endpoint for one package.
pub const API_ENDPOINT: &str = "https://pypistats.org/api/packages/{package}/recent";

/// The fixed universe of major Python packages.
pub const PACKAGES: &[&str] = &[
    "requests",
    "numpy",
    "pandas",
    "scipy",
    "flask",
    "django",
    "fastapi",
    "pydantic",
    "pytest",
    "torch",
    "transformers",
    "aiohttp",
];

/// The catalog entry.
pub fn source() -> Source {
    let mut parameters = std::collections::BTreeMap::new();
    parameters.insert("packages".to_string(), PACKAGES.len().to_string());
    parameters.insert("window".to_string(), "last_week".to_string());
    Source {
        id: SourceId::new(SOURCE_ID),
        name: "PyPI Package Downloads".to_string(),
        provider: "Python Package Index".to_string(),
        category: "technology".to_string(),
        subcategory: Some("package_downloads".to_string()),
        endpoint: API_ENDPOINT.to_string(),
        protocol: wse_model::Protocol::Https,
        format: wse_model::DataFormat::Json,
        cadence: wse_model::Cadence::Interval { seconds: 86_400 },
        timezone: Some("UTC".to_string()),
        license: Some("PyPI download statistics; free to use".to_string()),
        authentication: wse_model::AuthKind::None,
        cost: wse_model::Cost::Free,
        historical_available: true,
        realtime_available: false,
        geospatial: false,
        entities: vec!["software".to_string()],
        priority: 48,
        enabled: true,
        collector_type: COLLECTOR_TYPE.to_string(),
        parameters,
        tier: wse_model::SourceTier::Tier2,
        measurement: wse_model::MeasurementSemantics::FixedUniverse,
        feeds_lenses: vec!["lens_software".to_string()],
        derivations: Vec::new(),
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Response {
    #[serde(default)]
    pub data: Data,
    #[serde(default)]
    pub package: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Data {
    #[serde(default)]
    pub last_day: f64,
    #[serde(default)]
    pub last_week: f64,
    #[serde(default)]
    pub last_month: f64,
}

/// Parse one package's recent-downloads response into one observation.
///
/// `observed_at` is the collection time: pypistats reports rolling windows with
/// no anchor date, so the honest timestamp is when we looked.
pub fn parse(
    body: &[u8],
    package: &str,
    observed_at: DateTime<Utc>,
) -> Result<Option<Observation>, CollectorError> {
    let response: Response = serde_json::from_slice(body)
        .map_err(|e| CollectorError::Parse(format!("pypi downloads: {e}")))?;
    if response.package.is_empty() {
        return Ok(None);
    }

    let raw = RawReference {
        locator: API_ENDPOINT.replace("{package}", package),
        hash: wse_model::fnv1a_hex(&String::from_utf8_lossy(body)),
        content_type: Some("application/json".to_string()),
        bytes: Some(body.len() as u64),
    };
    let observation = Observation::new(
        SourceId::new(SOURCE_ID),
        Some(EntityId::new("software")),
        "weekly_downloads",
        response.data.last_week,
        "downloads",
        observed_at,
        raw,
    )
    .with_received_at(observed_at)
    .with_record_key(package)
    .with_dimension("package", package)
    .with_attribute("package", response.package.clone())
    .with_attribute("last_day", format!("{:.0}", response.data.last_day))
    .with_attribute("last_month", format!("{:.0}", response.data.last_month));
    Ok(Some(observation))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn received() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-01T02:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn fixture() -> Vec<u8> {
        include_bytes!("../../../tests/fixtures/pypi_downloads.json").to_vec()
    }

    #[test]
    fn parses_the_weekly_total() {
        let observation = parse(&fixture(), "requests", received()).unwrap().unwrap();
        assert_eq!(observation.metric, "weekly_downloads");
        assert_eq!(observation.unit, "downloads");
        assert_eq!(observation.value, 292_028_863.0);
    }

    #[test]
    fn the_day_total_is_kept_as_a_drill_down_attribute() {
        let observation = parse(&fixture(), "requests", received()).unwrap().unwrap();
        assert_eq!(
            observation.attributes.get("last_day").map(String::as_str),
            Some("49362057")
        );
    }

    #[test]
    fn each_package_is_its_own_series() {
        let a = parse(&fixture(), "requests", received()).unwrap().unwrap();
        let b = parse(&fixture(), "numpy", received()).unwrap().unwrap();
        assert_ne!(a.series_key(), b.series_key());
    }

    #[test]
    fn an_unknown_package_is_skipped_not_zeroed() {
        let body = br#"{"data":{"last_week":0},"package":""}"#;
        assert!(parse(body, "nope", received()).unwrap().is_none());
    }

    #[test]
    fn malformed_payload_is_a_parse_error() {
        assert!(matches!(
            parse(b"nope", "requests", received()),
            Err(CollectorError::Parse(_))
        ));
    }

    #[test]
    fn catalog_declares_a_fixed_universe() {
        let source = source();
        assert_eq!(
            source.measurement,
            wse_model::MeasurementSemantics::FixedUniverse
        );
    }
}
