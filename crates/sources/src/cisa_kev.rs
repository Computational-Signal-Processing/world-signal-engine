//! CISA Known Exploited Vulnerabilities — measurable security activity.
//!
//! Not cybersecurity news: the KEV catalog is the authoritative list of
//! vulnerabilities **known to be exploited in the wild**, maintained by the US
//! Cybersecurity and Infrastructure Security Agency. It is a stable series:
//!
//! * `kev_added_7d` — how many vulnerabilities were added in the last seven
//!   days. A rise is a real increase in newly-exploited vulnerabilities.
//! * `kev_catalog_total` — the size of the whole catalog, a slow monotonic
//!   series that makes an unusual acceleration visible.
//!
//! Feed: <https://www.cisa.gov/known-exploited-vulnerabilities-catalog>
//!
//! Authentication: none. License: public domain (US Government).

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use wse_collector::CollectorError;
use wse_model::{EntityId, Observation, RawReference, Source, SourceId};

pub const SOURCE_ID: &str = "cisa_kev";
pub const COLLECTOR_TYPE: &str = "cisa_kev_catalog";

/// The machine-readable catalog.
pub const API_ENDPOINT: &str =
    "https://www.cisa.gov/sites/default/files/feeds/known_exploited_vulnerabilities.json";

/// The trailing window for the "recently added" count.
pub const RECENT_DAYS: i64 = 7;

/// The catalog entry.
pub fn source() -> Source {
    let mut parameters = BTreeMap::new();
    parameters.insert("recent_days".to_string(), RECENT_DAYS.to_string());

    Source {
        id: SourceId::new(SOURCE_ID),
        name: "CISA Known Exploited Vulnerabilities".to_string(),
        provider: "U.S. Cybersecurity and Infrastructure Security Agency".to_string(),
        category: "cyber".to_string(),
        subcategory: Some("exploited_vulnerabilities".to_string()),
        endpoint: API_ENDPOINT.to_string(),
        protocol: wse_model::Protocol::Https,
        format: wse_model::DataFormat::Json,
        cadence: wse_model::Cadence::Interval { seconds: 86_400 },
        timezone: Some("UTC".to_string()),
        license: Some("Public domain (US Government)".to_string()),
        authentication: wse_model::AuthKind::None,
        cost: wse_model::Cost::Free,
        historical_available: true,
        realtime_available: false,
        geospatial: false,
        entities: vec!["cyber".to_string()],
        priority: 35,
        enabled: true,
        collector_type: COLLECTOR_TYPE.to_string(),
        parameters,
        tier: wse_model::SourceTier::Tier1,
        measurement: wse_model::MeasurementSemantics::StableSeries,
        feeds_lenses: vec!["lens_cyber".to_string()],
        derivations: Vec::new(),
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Vulnerability {
    #[serde(default, rename = "cveID")]
    pub cve_id: String,
    #[serde(default, rename = "dateAdded")]
    pub date_added: String,
    #[serde(default, rename = "vendorProject")]
    pub vendor_project: String,
    #[serde(default)]
    pub product: String,
    #[serde(default, rename = "knownRansomwareCampaignUse")]
    pub ransomware: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Catalog {
    #[serde(default, rename = "catalogVersion")]
    pub catalog_version: String,
    #[serde(default)]
    pub count: u64,
    #[serde(default)]
    pub vulnerabilities: Vec<Vulnerability>,
}

/// Parse the catalog into observations.
///
/// Two observations: the trailing-window addition count and the catalog size.
/// Both share the entity `cyber_kev` and differ by metric, so each is its own
/// series.
pub fn parse(body: &[u8], observed_at: DateTime<Utc>) -> Result<Vec<Observation>, CollectorError> {
    let catalog: Catalog = serde_json::from_slice(body)
        .map_err(|e| CollectorError::Parse(format!("cisa kev: {e}")))?;

    let source_id = SourceId::new(SOURCE_ID);
    let entity = EntityId::new("cyber_kev");
    let hash = wse_model::fnv1a_hex(&String::from_utf8_lossy(body));
    let cutoff = (observed_at - Duration::days(RECENT_DAYS)).date_naive();

    let mut recent = 0u64;
    let mut recent_cves: Vec<&str> = Vec::new();
    for v in &catalog.vulnerabilities {
        if let Ok(date) = NaiveDate::parse_from_str(&v.date_added, "%Y-%m-%d") {
            if date >= cutoff {
                recent += 1;
                if recent_cves.len() < 5 {
                    recent_cves.push(&v.cve_id);
                }
            }
        }
    }

    let raw = |locator: String| RawReference {
        locator,
        hash: hash.clone(),
        content_type: Some("application/json".to_string()),
        bytes: Some(body.len() as u64),
    };

    // The catalog size the source itself reports, falling back to the list
    // length when the header count is absent.
    let total = if catalog.count > 0 {
        catalog.count
    } else {
        catalog.vulnerabilities.len() as u64
    };

    let added = Observation::new(
        source_id.clone(),
        Some(entity.clone()),
        "kev_added",
        recent as f64,
        "vulnerabilities",
        observed_at,
        raw(format!("{API_ENDPOINT}#recent-{RECENT_DAYS}d")),
    )
    .with_received_at(observed_at)
    .with_attribute("window_days", RECENT_DAYS.to_string())
    .with_attribute("catalog_version", catalog.catalog_version.clone())
    .with_attribute("recent_cves", recent_cves.join(","));

    let size = Observation::new(
        source_id,
        Some(entity),
        "kev_catalog_total",
        total as f64,
        "vulnerabilities",
        observed_at,
        raw(API_ENDPOINT.to_string()),
    )
    .with_received_at(observed_at)
    .with_attribute("catalog_version", catalog.catalog_version);

    Ok(vec![added, size])
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
        include_bytes!("../../../tests/fixtures/cisa_kev.json").to_vec()
    }

    #[test]
    fn parses_both_series() {
        let observations = parse(&fixture(), received()).unwrap();
        assert_eq!(observations.len(), 2);
        let added = observations
            .iter()
            .find(|o| o.metric == "kev_added")
            .unwrap();
        let total = observations
            .iter()
            .find(|o| o.metric == "kev_catalog_total")
            .unwrap();
        assert_eq!(total.value, 4.0, "catalog count from the header");
        assert_eq!(added.value, 2.0, "two entries within the last 7 days");
        assert_eq!(added.unit, "vulnerabilities");
    }

    #[test]
    fn the_two_series_are_distinct() {
        let observations = parse(&fixture(), received()).unwrap();
        assert_ne!(observations[0].series_key(), observations[1].series_key());
        for o in &observations {
            assert_eq!(o.entity_id.as_ref().unwrap().as_str(), "cyber_kev");
        }
    }

    #[test]
    fn the_recent_window_excludes_old_entries() {
        let body = br#"{"catalogVersion":"2026.01.01","count":1,
            "vulnerabilities":[{"cveID":"CVE-2020-0001","dateAdded":"2020-01-01"}]}"#;
        let observations = parse(body, received()).unwrap();
        let added = observations
            .iter()
            .find(|o| o.metric == "kev_added")
            .unwrap();
        assert_eq!(added.value, 0.0);
    }

    #[test]
    fn a_missing_count_falls_back_to_the_list_length() {
        let body = br#"{"vulnerabilities":[
            {"cveID":"CVE-2026-1","dateAdded":"2026-09-30"},
            {"cveID":"CVE-2026-2","dateAdded":"2026-09-29"}]}"#;
        let observations = parse(body, received()).unwrap();
        let total = observations
            .iter()
            .find(|o| o.metric == "kev_catalog_total")
            .unwrap();
        assert_eq!(total.value, 2.0);
    }

    #[test]
    fn malformed_payload_is_a_parse_error() {
        assert!(matches!(
            parse(b"nope", received()),
            Err(CollectorError::Parse(_))
        ));
    }

    #[test]
    fn the_catalog_is_institutional_cyber() {
        let source = source();
        assert_eq!(source.category, "cyber");
        assert_eq!(source.tier, wse_model::SourceTier::Tier1);
        assert!(source.feeds_lenses.contains(&"lens_cyber".to_string()));
        assert!(source.measurement.is_comparable());
    }
}
