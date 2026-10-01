//! CISA Known Exploited Vulnerabilities — measurable security activity.
//!
//! Not cybersecurity news: the KEV catalog is the authoritative list of
//! vulnerabilities **known to be exploited in the wild**, maintained by the US
//! Cybersecurity and Infrastructure Security Agency. It is a stable series:
//!
//! * `kev_added` — how many vulnerabilities were added **that UTC day**. The
//!   catalog is polled once a day and the count is a genuine, non-overlapping
//!   daily quantity: consecutive points share no members, so a daily z-score is
//!   meaningful. (An earlier version emitted a trailing 7-day sum sampled
//!   daily; consecutive points then overlapped by six days, which made the
//!   series autocorrelated and a daily z-score structurally misleading. See
//!   `docs/decisions/0018-kev-daily-additions.md`.)
//! * `kev_catalog_total` — the size of the whole catalog. It only ever grows, so
//!   a level z-score on it is close to meaningless. The catalog therefore
//!   declares a `Delta` derivation to `kev_catalog_growth` — how many
//!   vulnerabilities were added since the previous poll — which is what
//!   detection runs on. The raw total is stored and drill-downable but
//!   **evidence-only**. See `docs/decisions/0014-derived-metrics.md`.
//!
//! `kev_added` (from the authoritative `dateAdded`) and `kev_catalog_growth`
//! (from the total's interval difference) are two independent measurements of
//! the same daily additions, so they cross-check each other.
//!
//! Feed: <https://www.cisa.gov/known-exploited-vulnerabilities-catalog>
//!
//! Authentication: none. License: public domain (US Government).

use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use wse_collector::CollectorError;
use wse_model::{Derivation, EntityId, Observation, RawReference, Source, SourceId};

pub const SOURCE_ID: &str = "cisa_kev";
pub const COLLECTOR_TYPE: &str = "cisa_kev_catalog";

/// The raw cumulative catalog size the collector emits.
pub const RAW_TOTAL_METRIC: &str = "kev_catalog_total";
/// The derived per-interval series detection runs on.
pub const DERIVED_GROWTH_METRIC: &str = "kev_catalog_growth";

/// The machine-readable catalog.
pub const API_ENDPOINT: &str =
    "https://www.cisa.gov/sites/default/files/feeds/known_exploited_vulnerabilities.json";

/// The catalog entry.
pub fn source() -> Source {
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
        parameters: BTreeMap::new(),
        tier: wse_model::SourceTier::Tier1,
        measurement: wse_model::MeasurementSemantics::StableSeries,
        feeds_lenses: vec!["lens_cyber".to_string()],
        derivations: vec![Derivation::delta(RAW_TOTAL_METRIC, DERIVED_GROWTH_METRIC)],
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
/// Two observations: the additions dated to that UTC day and the catalog size.
/// Both share the entity `cyber_kev` and differ by metric, so each is its own
/// series.
pub fn parse(body: &[u8], observed_at: DateTime<Utc>) -> Result<Vec<Observation>, CollectorError> {
    let catalog: Catalog = serde_json::from_slice(body)
        .map_err(|e| CollectorError::Parse(format!("cisa kev: {e}")))?;

    let source_id = SourceId::new(SOURCE_ID);
    let entity = EntityId::new("cyber_kev");
    let hash = wse_model::fnv1a_hex(&String::from_utf8_lossy(body));
    // The additions *of the collection day*. `dateAdded` is the authoritative
    // day, so the count is non-overlapping across consecutive daily polls.
    let day = observed_at.date_naive();

    let mut added_today = 0u64;
    let mut added_cves: Vec<&str> = Vec::new();
    for v in &catalog.vulnerabilities {
        if NaiveDate::parse_from_str(&v.date_added, "%Y-%m-%d") == Ok(day) {
            added_today += 1;
            if added_cves.len() < 5 {
                added_cves.push(&v.cve_id);
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
        added_today as f64,
        "vulnerabilities",
        observed_at,
        raw(format!("{API_ENDPOINT}#added-{day}")),
    )
    .with_received_at(observed_at)
    // The day is the record, so a re-poll of the same day keeps one identity.
    .with_record_key(day.format("%Y-%m-%d").to_string())
    .with_attribute("day", day.format("%Y-%m-%d").to_string())
    .with_attribute("catalog_version", catalog.catalog_version.clone())
    .with_attribute("added_cves", added_cves.join(","));

    let size = Observation::new(
        source_id,
        Some(entity),
        RAW_TOTAL_METRIC,
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
        DateTime::parse_from_rfc3339("2026-09-30T00:00:00Z")
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
        assert_eq!(added.value, 1.0, "one entry dated the collection day");
        assert_eq!(added.unit, "vulnerabilities");
        assert_eq!(
            added.attributes.get("day").map(String::as_str),
            Some("2026-09-30")
        );
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
    fn only_the_collection_day_counts() {
        // One old entry and one dated the collection day: only the latter
        // counts, so consecutive daily polls never share a member.
        let body = br#"{"catalogVersion":"2026.01.01","count":2,"vulnerabilities":[
            {"cveID":"CVE-2020-0001","dateAdded":"2020-01-01"},
            {"cveID":"CVE-2026-1001","dateAdded":"2026-09-30"}]}"#;
        let observations = parse(body, received()).unwrap();
        let added = observations
            .iter()
            .find(|o| o.metric == "kev_added")
            .unwrap();
        assert_eq!(added.value, 1.0);
    }

    #[test]
    fn a_day_with_no_additions_is_a_genuine_zero() {
        // The catalog was polled on a day with nothing added. "Nothing added"
        // is a real measurement, emitted as 0 — not the same as no data.
        let body = br#"{"catalogVersion":"2026.01.01","count":1,"vulnerabilities":[
            {"cveID":"CVE-2020-0001","dateAdded":"2020-01-01"}]}"#;
        let observations = parse(body, received()).unwrap();
        let added = observations
            .iter()
            .find(|o| o.metric == "kev_added")
            .unwrap();
        assert_eq!(added.value, 0.0);
    }

    #[test]
    fn the_day_is_the_record_key() {
        // Re-polling the same day keeps one identity; the next day is a new
        // record. Without the day key, a day re-polled would be re-minted.
        let a = parse(&fixture(), received()).unwrap();
        let b = parse(&fixture(), received()).unwrap();
        let added_a = a.iter().find(|o| o.metric == "kev_added").unwrap();
        let added_b = b.iter().find(|o| o.metric == "kev_added").unwrap();
        assert_eq!(added_a.id, added_b.id);

        let next_day = DateTime::parse_from_rfc3339("2026-10-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let c = parse(&fixture(), next_day).unwrap();
        let added_c = c.iter().find(|o| o.metric == "kev_added").unwrap();
        assert_ne!(added_a.id, added_c.id, "a new day is a new record");
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

    #[test]
    fn the_catalog_declares_the_growth_derivation() {
        // The collector emits the raw cumulative size; the catalog is what says
        // the detection series is its increment. Without this the source would
        // be detected on a monotonic level.
        let source = source();
        let declaration = source
            .derivations
            .iter()
            .find(|d| d.from_metric == RAW_TOTAL_METRIC)
            .expect("a growth derivation is declared");
        assert_eq!(declaration.to_metric, DERIVED_GROWTH_METRIC);
        assert_eq!(declaration.kind, wse_model::DerivationKind::Delta);
        assert_eq!(
            source
                .derivation_for(RAW_TOTAL_METRIC)
                .map(|d| d.to_metric.as_str()),
            Some(DERIVED_GROWTH_METRIC),
        );
    }

    #[test]
    fn the_emitted_total_metric_matches_the_declared_input() {
        // The derivation reads `RAW_TOTAL_METRIC`; the collector must emit
        // exactly that metric or the derivation never fires.
        let observations = parse(&fixture(), received()).unwrap();
        let total = observations
            .iter()
            .find(|o| o.metric == RAW_TOTAL_METRIC)
            .expect("the raw total is emitted under the declared metric name");
        assert_eq!(total.value, 4.0);
    }
}
