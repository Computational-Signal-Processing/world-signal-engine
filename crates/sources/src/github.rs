//! GitHub repository activity — a **fixed universe** of major open-source
//! projects.
//!
//! ## Why the population is fixed
//!
//! An earlier version searched `language:rust sort:updated per_page:50` and
//! summed the stars of whatever came back. That is not a time series: the
//! top-50 "recently updated" set churns every collection, so the aggregate
//! moved when *membership* changed, not when the world did. A repository
//! dropping out of the page read as "the ecosystem lost stars".
//!
//! This source instead declares a **fixed universe** of named repositories
//! (below) and re-measures the same set every collection. The aggregate is then
//! a real, comparable measurement: how the stars of a known set of projects
//! changed. Membership is stable by construction — the list only changes when
//! someone edits the code.
//!
//! ## Why the universe is not one language
//!
//! An earlier version tracked only Rust repositories. That is a sensor of one
//! corner of software, not of "the software world": a shift in developer
//! attention from one ecosystem to another is exactly the kind of change the
//! engine exists to surface, and a single-language universe is blind to it. The
//! universe now spans the major language ecosystems, foundational
//! infrastructure, and AI/ML, so the aggregate is a measurement of *the open
//! source ecosystem* rather than of one community. See
//! `docs/decisions/0024-github-universe-spans-ecosystems.md`.
//!
//! The catalog marks this source `measurement: fixed_universe` and tier 3
//! (community/platform signal), so the engine treats it honestly.
//!
//! ## Why each repository is its own series
//!
//! All repositories share the entity `ecosystem_open_source`, metric
//! `repo_stars` and unit `stars`. Without more, they would share one
//! `series_key` and therefore one rolling baseline, pooling unrelated
//! repositories: a repository's first appearance would be scored against the
//! others' star counts and fire a meaningless cold-start deviation (~1527σ was
//! recorded this way, see `docs/reality-audit.md` finding 6). Each observation
//! therefore carries `dimension: repo = owner/name`, so the baseline is per
//! repository — exactly the "rolling statistics per repository" the semantic
//! audit asks for. See `docs/decisions/0020-github-per-repo-series.md`.
//!
//! API docs: <https://docs.github.com/en/rest/repos/repos>
//!
//! Authentication: works unauthenticated at 60 requests/hour (the universe is
//! ~20 repositories, collected hourly, so it fits); a token raises the limit.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use wse_collector::CollectorError;
use wse_model::{EntityId, Observation, RawReference, Source, SourceId};

pub const SOURCE_ID: &str = "github_repo_universe";
pub const COLLECTOR_TYPE: &str = "github_repo_universe";

/// Repository detail endpoint; `{repo}` is `owner/name`.
pub const REPO_ENDPOINT: &str = "https://api.github.com/repos/{repo}";

/// The fixed universe of open-source repositories, spanning the major language
/// ecosystems, foundational infrastructure and AI/ML.
///
/// Membership is deliberately stable: the same set is re-measured every hour so
/// a change in the aggregate is a change in these projects, never churn in a
/// search result. Changing this list is a deliberate, reviewable act. Each entry
/// must be the canonical `owner/name` (renamed repositories 301-redirect, which
/// the transport does not follow).
pub const UNIVERSE: &[&str] = &[
    // Language ecosystems
    "rust-lang/rust",
    "golang/go",
    "python/cpython",
    "nodejs/node",
    "denoland/deno",
    "vuejs/core",
    "vercel/next.js",
    "react/react",
    // Foundational infrastructure
    "kubernetes/kubernetes",
    "moby/moby",
    "hashicorp/terraform",
    "redis/redis",
    "postgres/postgres",
    "apache/kafka",
    "grafana/grafana",
    "prometheus/prometheus",
    // AI / ML
    "pytorch/pytorch",
    "huggingface/transformers",
    "langchain-ai/langchain",
    "ollama/ollama",
    // Developer tooling
    "microsoft/vscode",
    "neovim/neovim",
    "astral-sh/ruff",
    "duckdb/duckdb",
];

/// The catalog entry.
pub fn source() -> Source {
    let mut parameters = std::collections::BTreeMap::new();
    parameters.insert("universe".to_string(), UNIVERSE.join(","));
    parameters.insert("universe_size".to_string(), UNIVERSE.len().to_string());

    Source {
        id: SourceId::new(SOURCE_ID),
        name: "GitHub Open-Source Repository Universe".to_string(),
        provider: "GitHub".to_string(),
        category: "technology".to_string(),
        subcategory: Some("software_ecosystem".to_string()),
        endpoint: REPO_ENDPOINT.to_string(),
        protocol: wse_model::Protocol::Https,
        format: wse_model::DataFormat::Json,
        cadence: wse_model::Cadence::Interval { seconds: 3600 },
        timezone: Some("UTC".to_string()),
        license: Some("Public API; GitHub Acceptable Use terms apply".to_string()),
        authentication: wse_model::AuthKind::Token,
        cost: wse_model::Cost::FreeWithRegistration,
        historical_available: false,
        realtime_available: true,
        geospatial: false,
        entities: vec!["software".to_string()],
        priority: 50,
        enabled: true,
        collector_type: COLLECTOR_TYPE.to_string(),
        parameters,
        tier: wse_model::SourceTier::Tier3,
        // The population is the fixed universe above, so the aggregate is a
        // comparable measurement rather than a churning search result.
        measurement: wse_model::MeasurementSemantics::FixedUniverse,
        feeds_lenses: vec!["lens_software".to_string()],
        derivations: Vec::new(),
    }
}

/// A single repository, reduced to the fields we measure.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Repo {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub full_name: String,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub stargazers_count: f64,
    #[serde(default)]
    pub forks_count: f64,
    #[serde(default)]
    pub open_issues_count: f64,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub language: Option<String>,
}

/// Parse one repository-detail response into at most one observation.
///
/// `observed_at` is the collection time, not a source-side timestamp: the
/// measurement is "the star count right now", and taking the timestamp from the
/// repository's `pushed_at` would make the series' timestamps churn with
/// activity rather than with when we actually looked.
///
/// Returns `Ok(None)` for a repository that cannot be measured (archived, or
/// missing an identity), so a retired project is skipped rather than reported
/// as zero.
pub fn parse_repo(
    body: &[u8],
    observed_at: DateTime<Utc>,
) -> Result<Option<Observation>, CollectorError> {
    let repo: Repo = serde_json::from_slice(body)
        .map_err(|e| CollectorError::Parse(format!("github repo: {e}")))?;

    if repo.archived || repo.full_name.is_empty() {
        return Ok(None);
    }

    let source_id = SourceId::new(SOURCE_ID);
    let entity = EntityId::new("ecosystem_open_source");
    let hash = wse_model::fnv1a_hex(&String::from_utf8_lossy(body));
    let raw = RawReference {
        locator: if repo.html_url.is_empty() {
            format!("github:{}", repo.full_name)
        } else {
            repo.html_url.clone()
        },
        hash,
        content_type: Some("application/json".to_string()),
        bytes: Some(body.len() as u64),
    };

    let observation = Observation::new(
        source_id,
        Some(entity),
        "repo_stars",
        repo.stargazers_count,
        "stars",
        observed_at,
        raw,
    )
    .with_received_at(observed_at)
    // The repository is the record identity; it is stable across collections,
    // so an unchanged star count de-duplicates instead of re-inserting.
    .with_identity(&repo.full_name)
    // The repository is also the series dimension, so its baseline is its own
    // history rather than a pool of unrelated repositories' star counts.
    .with_dimension("repo", repo.full_name.clone())
    .with_attribute("repo", repo.full_name.clone())
    .with_attribute("repo_id", repo.id.to_string())
    .with_attribute("forks", format!("{:.0}", repo.forks_count))
    .with_attribute("open_issues", format!("{:.0}", repo.open_issues_count));

    Ok(Some(observation))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn received() -> DateTime<Utc> {
        Utc.timestamp_millis_opt(1_700_500_000_000)
            .single()
            .unwrap()
    }

    fn fixture() -> Vec<u8> {
        include_bytes!("../../../tests/fixtures/github_repo.json").to_vec()
    }

    #[test]
    fn parses_a_repository_into_one_observation() {
        let observation = parse_repo(&fixture(), received()).unwrap().unwrap();
        assert_eq!(observation.metric, "repo_stars");
        assert_eq!(observation.unit, "stars");
        assert_eq!(observation.source_id.as_str(), SOURCE_ID);
        // Observed at collection time, not a source-side push time.
        assert_eq!(observation.observed_at, received());
    }

    #[test]
    fn the_repository_is_the_record_identity() {
        let observation = parse_repo(&fixture(), received()).unwrap().unwrap();
        assert_eq!(observation.identity.as_deref(), Some("tokio-rs/tokio"));
        assert_eq!(
            observation.attributes.get("repo").map(String::as_str),
            Some("tokio-rs/tokio")
        );
        assert!(observation.raw.locator.starts_with("https://github.com/"));
    }

    #[test]
    fn each_repository_is_its_own_series() {
        // Two different repositories must not share a baseline: they are
        // distinguished by the `repo` dimension, not pooled under one key.
        let tokio = parse_repo(&fixture(), received()).unwrap().unwrap();
        let other = parse_repo(
            br#"{"id":2,"full_name":"serde-rs/serde","stargazers_count":5}"#,
            received(),
        )
        .unwrap()
        .unwrap();
        assert_ne!(tokio.series_key(), other.series_key());
        assert!(tokio.series_key().contains("repo=tokio-rs/tokio"));
        assert!(other.series_key().contains("repo=serde-rs/serde"));
    }

    #[test]
    fn the_same_repository_shares_one_series_across_polls() {
        // Two polls of the same repository stay on one series, so the rolling
        // baseline is that repository's own history.
        let a = parse_repo(&fixture(), received()).unwrap().unwrap();
        let b = parse_repo(&fixture(), received()).unwrap().unwrap();
        assert_eq!(a.series_key(), b.series_key());
        assert!(a.series_key().contains("ecosystem_open_source"));
    }

    #[test]
    fn an_archived_repository_is_skipped_not_zeroed() {
        let body = br#"{"id":1,"full_name":"a/b","stargazers_count":5,"archived":true}"#;
        assert!(parse_repo(body, received()).unwrap().is_none());
    }

    #[test]
    fn re_collecting_the_same_repository_de_duplicates() {
        let first = parse_repo(&fixture(), received()).unwrap().unwrap();
        let second = parse_repo(&fixture(), received()).unwrap().unwrap();
        assert_eq!(first.id, second.id);
    }

    #[test]
    fn malformed_payload_is_a_parse_error() {
        assert!(matches!(
            parse_repo(br#"{"id":"oops"}"#, received()),
            Err(CollectorError::Parse(_))
        ));
    }

    #[test]
    fn the_catalog_declares_a_fixed_universe() {
        let source = source();
        assert_eq!(
            source.measurement,
            wse_model::MeasurementSemantics::FixedUniverse
        );
        assert!(source.measurement.is_comparable());
        assert_eq!(source.tier, wse_model::SourceTier::Tier3);
        assert!(source.feeds_lenses.contains(&"lens_software".to_string()));
        // Every entry is a well-formed `owner/name`.
        for repo in UNIVERSE {
            assert_eq!(repo.matches('/').count(), 1, "bad universe entry {repo}");
        }
    }
}
