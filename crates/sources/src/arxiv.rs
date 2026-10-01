//! arXiv — preprint submission velocity per fixed category.
//!
//! The arXiv API returns `<opensearch:totalResults>` for a category query: the
//! total number of preprints in that category. The total only ever grows, so a
//! level z-score on it is close to meaningless. The collector therefore emits
//! the **raw cumulative level** (`preprint_total`), and the catalog declares a
//! `Delta` derivation from it to `preprint_new` — the number of new preprints
//! since the previous poll, which is the submission-velocity series the name
//! promises.
//!
//! The raw level is stored and drill-downable but **evidence-only**: it is not
//! detected on. Only `preprint_new` is. See `docs/decisions/0014-derived-metrics.md`.
//!
//! Docs: <https://info.arxiv.org/help/api/index.html>
//!
//! Authentication: none. arXiv asks that clients not exceed one request every
//! three seconds; the daily cadence is far inside that. License: arXiv API
//! terms; metadata is open.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use wse_collector::CollectorError;
use wse_model::{Derivation, EntityId, Observation, RawReference, Source, SourceId};

use crate::xml;

pub const SOURCE_ID: &str = "arxiv_submissions";
pub const COLLECTOR_TYPE: &str = "arxiv_category_total";

/// The raw cumulative level the collector emits.
pub const RAW_METRIC: &str = "preprint_total";
/// The derived per-interval series detection runs on.
pub const DERIVED_METRIC: &str = "preprint_new";

/// The query endpoint.
pub const API_ENDPOINT: &str = "https://export.arxiv.org/api/query";

/// Tracked arXiv categories: `(slug, label, category code)`.
pub const CATEGORIES: &[(&str, &str, &str)] = &[
    ("cs_ai", "cs.AI", "cs.AI"),
    ("cs_lg", "cs.LG", "cs.LG"),
    ("cs_cl", "cs.CL", "cs.CL"),
    ("cs_cv", "cs.CV", "cs.CV"),
];

/// The catalog entry.
pub fn source() -> Source {
    let mut parameters = BTreeMap::new();
    parameters.insert(
        "categories".to_string(),
        CATEGORIES.iter().map(|c| c.0).collect::<Vec<_>>().join(","),
    );

    Source {
        id: SourceId::new(SOURCE_ID),
        name: "arXiv Preprint Velocity".to_string(),
        provider: "arXiv (Cornell University)".to_string(),
        category: "science".to_string(),
        subcategory: Some("preprint_velocity".to_string()),
        endpoint: API_ENDPOINT.to_string(),
        protocol: wse_model::Protocol::Https,
        format: wse_model::DataFormat::Atom,
        cadence: wse_model::Cadence::Interval { seconds: 86_400 },
        timezone: Some("UTC".to_string()),
        license: Some("arXiv API terms; metadata is open".to_string()),
        authentication: wse_model::AuthKind::None,
        cost: wse_model::Cost::Free,
        historical_available: true,
        realtime_available: false,
        geospatial: false,
        entities: vec!["research".to_string()],
        priority: 46,
        enabled: true,
        collector_type: COLLECTOR_TYPE.to_string(),
        parameters,
        tier: wse_model::SourceTier::Tier2,
        measurement: wse_model::MeasurementSemantics::StableSeries,
        feeds_lenses: vec!["lens_science".to_string(), "lens_ai".to_string()],
        derivations: vec![Derivation::delta(RAW_METRIC, DERIVED_METRIC)],
    }
}

/// Build the query URL for one category.
///
/// `max_results=1` is the smallest value arXiv accepts; `0` returns HTTP 500.
/// The feed still reports the full `opensearch:totalResults`, which is the
/// measurement, so one result is fetched and discarded.
pub fn category_url(category: &str) -> String {
    format!("{API_ENDPOINT}?search_query=cat:{category}&max_results=1")
}

/// Parse the total number of results from an arXiv Atom feed.
pub fn parse_total(body: &[u8]) -> Result<u64, CollectorError> {
    let text = std::str::from_utf8(body)
        .map_err(|e| CollectorError::Parse(format!("arxiv not utf-8: {e}")))?;
    let raw = xml::text(text, "opensearch:totalResults")
        .ok_or_else(|| CollectorError::Parse("arxiv: no totalResults".to_string()))?;
    raw.trim()
        .parse::<u64>()
        .map_err(|e| CollectorError::Parse(format!("arxiv totalResults {raw:?}: {e}")))
}

/// The `updated` timestamp the feed reports, when present.
pub fn parse_updated(body: &[u8]) -> Option<DateTime<Utc>> {
    let text = std::str::from_utf8(body).ok()?;
    let raw = xml::text(text, "updated")?;
    DateTime::parse_from_rfc3339(raw.trim())
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

/// Turn one category's total into an observation.
pub fn observation_for(
    slug: &str,
    label: &str,
    total: u64,
    observed_at: DateTime<Utc>,
    body: &[u8],
) -> Observation {
    let source_id = SourceId::new(SOURCE_ID);
    let entity = EntityId::new(format!("arxiv_{slug}"));
    let hash = wse_model::fnv1a_hex(&String::from_utf8_lossy(body));
    let raw = RawReference {
        locator: category_url(label),
        hash,
        content_type: Some("application/atom+xml".to_string()),
        bytes: Some(body.len() as u64),
    };
    Observation::new(
        source_id,
        Some(entity),
        RAW_METRIC,
        total as f64,
        "preprints",
        observed_at,
        raw,
    )
    .with_received_at(observed_at)
    .with_attribute("category", label.to_string())
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
        include_bytes!("../../../tests/fixtures/arxiv_query.xml").to_vec()
    }

    #[test]
    fn parses_the_total_result_count() {
        assert_eq!(parse_total(&fixture()).unwrap(), 203523);
    }

    #[test]
    fn parses_the_feed_updated_time() {
        assert_eq!(
            parse_updated(&fixture()).unwrap().to_rfc3339(),
            "2026-10-01T01:35:48+00:00"
        );
    }

    #[test]
    fn a_feed_without_a_total_is_a_parse_error() {
        assert!(matches!(
            parse_total(b"<feed></feed>"),
            Err(CollectorError::Parse(_))
        ));
    }

    #[test]
    fn each_category_is_its_own_series() {
        let ai = observation_for("cs_ai", "cs.AI", 203523, received(), &fixture());
        let lg = observation_for("cs_lg", "cs.LG", 100000, received(), &fixture());
        assert_ne!(ai.series_key(), lg.series_key());
        assert_eq!(ai.metric, RAW_METRIC);
        assert_eq!(ai.unit, "preprints");
    }

    #[test]
    fn the_catalog_is_science_and_ai() {
        let source = source();
        assert_eq!(source.format, wse_model::DataFormat::Atom);
        assert_eq!(source.tier, wse_model::SourceTier::Tier2);
        assert!(source.feeds_lenses.contains(&"lens_ai".to_string()));
        assert!(source.measurement.is_comparable());
    }

    #[test]
    fn the_catalog_declares_the_velocity_derivation() {
        // The collector emits the raw level; the catalog is what says the
        // detection series is its increment. Without this the source would be
        // detected on a monotonic total.
        let source = source();
        let derivation = source
            .derivation_for(RAW_METRIC)
            .expect("arxiv must derive from its cumulative level");
        assert_eq!(derivation.to_metric, DERIVED_METRIC);
        assert_eq!(derivation.kind, wse_model::DerivationKind::Delta);
    }
}
