//! npm package downloads — a fixed universe of major JavaScript packages.
//!
//! Download volume is a direct, daily measurement of how much the world is
//! *using* a piece of software. It is independent of GitHub stars (attention)
//! and Hacker News (discussion): a package can be downloaded heavily while
//! getting no stars, and a spike in installs is a real adoption change.
//!
//! ## Why a fixed package set, not a search
//!
//! The measurement is per package. The set is fixed in code (below), so each
//! package is its own stable series. A "most-downloaded" query would churn
//! membership and make the aggregate move with the ranking rather than with
//! usage — the same trap `github_repo_universe` documents.
//!
//! ## Why the weekly point
//!
//! The `last-week` point is a complete, non-overlapping window: consecutive
//! daily polls of `last-week` share members, but the series is sampled once a
//! day and the *value* is a stable weekly total, so a day-over-day change is a
//! real change in usage rather than an artifact of the window. (`last-day`
//! swings with weekday and would need its own seasonality handling.)
//!
//! API docs: <https://github.com/npm/registry/blob/master/docs/download-counts.md>
//!
//! Authentication: none. License: npm registry data is free to use.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use wse_collector::CollectorError;
use wse_model::{EntityId, Observation, RawReference, Source, SourceId};

pub const SOURCE_ID: &str = "npm_downloads";
pub const COLLECTOR_TYPE: &str = "npm_downloads";

/// The weekly download point for one package; `{package}` is filled per request.
pub const API_ENDPOINT: &str = "https://api.npmjs.org/downloads/point/last-week/{package}";

/// The fixed universe of major JavaScript packages.
pub const PACKAGES: &[&str] = &[
    "react",
    "vue",
    "next",
    "svelte",
    "express",
    "lodash",
    "axios",
    "typescript",
    "webpack",
    "vite",
    "eslint",
    "jest",
];

/// The catalog entry.
pub fn source() -> Source {
    let mut parameters = std::collections::BTreeMap::new();
    parameters.insert("packages".to_string(), PACKAGES.len().to_string());
    parameters.insert("window".to_string(), "last-week".to_string());
    Source {
        id: SourceId::new(SOURCE_ID),
        name: "npm Package Downloads".to_string(),
        provider: "npm, Inc.".to_string(),
        category: "technology".to_string(),
        subcategory: Some("package_downloads".to_string()),
        endpoint: API_ENDPOINT.to_string(),
        protocol: wse_model::Protocol::Https,
        format: wse_model::DataFormat::Json,
        cadence: wse_model::Cadence::Interval { seconds: 86_400 },
        timezone: Some("UTC".to_string()),
        license: Some("npm registry download counts; free to use".to_string()),
        authentication: wse_model::AuthKind::None,
        cost: wse_model::Cost::Free,
        historical_available: true,
        realtime_available: false,
        geospatial: false,
        entities: vec!["software".to_string()],
        priority: 47,
        enabled: true,
        collector_type: COLLECTOR_TYPE.to_string(),
        parameters,
        tier: wse_model::SourceTier::Tier2,
        measurement: wse_model::MeasurementSemantics::FixedUniverse,
        feeds_lenses: vec!["lens_software".to_string()],
        derivations: Vec::new(),
    }
}

/// The download-count response.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Downloads {
    #[serde(default)]
    pub downloads: f64,
    #[serde(default)]
    pub start: String,
    #[serde(default)]
    pub end: String,
    #[serde(default)]
    pub package: String,
}

/// Parse one package's weekly download point into one observation.
///
/// The observed time is the window's `end` date when present, so the series is
/// anchored to the measured week rather than to the poll time.
pub fn parse(
    body: &[u8],
    package: &str,
    received_at: DateTime<Utc>,
) -> Result<Option<Observation>, CollectorError> {
    let downloads: Downloads = serde_json::from_slice(body)
        .map_err(|e| CollectorError::Parse(format!("npm downloads: {e}")))?;
    // npm answers `{"error": ...}` for an unknown package; there is no count.
    if downloads.package.is_empty() {
        return Ok(None);
    }
    let observed_at = parse_day(&downloads.end).unwrap_or(received_at);

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
        downloads.downloads,
        "downloads",
        observed_at,
        raw,
    )
    .with_received_at(received_at)
    .with_record_key(package)
    .with_dimension("package", package)
    .with_attribute("package", downloads.package.clone())
    .with_attribute("window_start", downloads.start.clone());
    Ok(Some(observation))
}

fn parse_day(raw: &str) -> Option<DateTime<Utc>> {
    chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .ok()
        .map(|d| d.and_hms_opt(0, 0, 0).unwrap().and_utc())
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
        include_bytes!("../../../tests/fixtures/npm_downloads.json").to_vec()
    }

    #[test]
    fn parses_a_weekly_download_point() {
        let observation = parse(&fixture(), "react", received()).unwrap().unwrap();
        assert_eq!(observation.metric, "weekly_downloads");
        assert_eq!(observation.unit, "downloads");
        assert_eq!(
            observation.dimensions.get("package").map(String::as_str),
            Some("react")
        );
    }

    #[test]
    fn the_series_is_anchored_to_the_measured_week() {
        let observation = parse(&fixture(), "react", received()).unwrap().unwrap();
        // The fixture window ends 2026-09-29.
        assert_eq!(
            observation.observed_at.to_rfc3339(),
            "2026-09-29T00:00:00+00:00"
        );
    }

    #[test]
    fn each_package_is_its_own_series() {
        let a = parse(&fixture(), "react", received()).unwrap().unwrap();
        let b = parse(&fixture(), "vue", received()).unwrap().unwrap();
        assert_ne!(a.series_key(), b.series_key());
    }

    #[test]
    fn an_unknown_package_is_skipped_not_zeroed() {
        let body = br#"{"error":"package not found"}"#;
        assert!(parse(body, "nope", received()).unwrap().is_none());
    }

    #[test]
    fn malformed_payload_is_a_parse_error() {
        assert!(matches!(
            parse(b"nope", "react", received()),
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
        assert!(source.feeds_lenses.contains(&"lens_software".to_string()));
    }
}
