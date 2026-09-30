//! GitHub repository activity — public software-ecosystem signals.
//!
//! Searches public repositories in a language and reports each repository's
//! star count. A change here is a change in the *whole ecosystem*: it takes a
//! coordinated move across many repositories to move the aggregate, which is
//! exactly the kind of slow, persistent shift the early-signal engine is for.
//!
//! API docs: <https://docs.github.com/en/rest/search/search>
//!
//! Authentication: works unauthenticated at a low rate limit; a token raises
//! the limit. The catalog entry says so rather than pretending otherwise.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use wse_collector::CollectorError;
use wse_model::{EntityId, Observation, RawReference, Source, SourceId};

pub const SOURCE_ID: &str = "github_rust_activity";
pub const COLLECTOR_TYPE: &str = "github_repo_search";

/// Repository search endpoint.
pub const API_ENDPOINT: &str = "https://api.github.com/search/repositories";

/// Language whose ecosystem we track.
pub const LANGUAGE: &str = "rust";

/// The catalog entry.
pub fn source() -> Source {
    let mut parameters = std::collections::BTreeMap::new();
    parameters.insert("language".to_string(), LANGUAGE.to_string());
    parameters.insert("sort".to_string(), "updated".to_string());
    parameters.insert("per_page".to_string(), "50".to_string());

    Source {
        id: SourceId::new(SOURCE_ID),
        name: "GitHub Rust Ecosystem Activity".to_string(),
        provider: "GitHub".to_string(),
        category: "technology".to_string(),
        subcategory: Some("software_ecosystem".to_string()),
        endpoint: API_ENDPOINT.to_string(),
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
    }
}

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
    pub pushed_at: Option<String>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub language: Option<String>,
}

/// The search API wraps results in an envelope; the raw list endpoint does not.
/// Accept both so the collector survives either endpoint being swapped in.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RepoResponse {
    Envelope {
        #[serde(default)]
        items: Vec<Repo>,
    },
    Bare(Vec<Repo>),
}

impl RepoResponse {
    fn into_repos(self) -> Vec<Repo> {
        match self {
            RepoResponse::Envelope { items } => items,
            RepoResponse::Bare(repos) => repos,
        }
    }
}

/// Parse a GitHub repository search response into observations.
///
/// One observation per repository, measuring stars. The entity is the language
/// ecosystem; the per-repo name is a dimension so drill-down stays possible.
pub fn parse(body: &[u8], received_at: DateTime<Utc>) -> Result<Vec<Observation>, CollectorError> {
    let response: RepoResponse = serde_json::from_slice(body)
        .map_err(|e| CollectorError::Parse(format!("github repos: {e}")))?;
    let repos = response.into_repos();

    let source_id = SourceId::new(SOURCE_ID);
    let entity = EntityId::new(format!("ecosystem_{LANGUAGE}"));
    let hash = wse_model::fnv1a_hex(&String::from_utf8_lossy(body));
    let mut observations = Vec::new();

    for repo in repos {
        // Archived repositories do not move; including them would dilute the
        // aggregate with dead weight.
        if repo.archived || repo.full_name.is_empty() {
            continue;
        }
        // GitHub has no single "observed at" for a search hit. The push time is
        // the closest honest proxy, and a missing one means we cannot place the
        // point on a timeline, so we skip rather than invent a timestamp.
        let Some(observed_at) = repo
            .pushed_at
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.with_timezone(&Utc))
        else {
            continue;
        };

        let raw = RawReference {
            locator: repo.html_url.clone(),
            hash: hash.clone(),
            content_type: Some("application/json".to_string()),
            bytes: Some(body.len() as u64),
        };

        let observation = Observation::new(
            source_id.clone(),
            Some(entity.clone()),
            "repo_stars",
            repo.stargazers_count,
            "stars",
            observed_at,
            raw,
        )
        .with_received_at(received_at)
        // Two repositories can share a `pushed_at` to the second. Without a
        // per-repository discriminator they would share an observation id and
        // all but one would be silently de-duplicated away. The repository is
        // the record's identity; the ecosystem remains the series.
        .with_identity(&repo.full_name)
        .with_attribute("repo", repo.full_name.clone())
        .with_attribute("repo_id", repo.id.to_string())
        .with_attribute("forks", format!("{:.0}", repo.forks_count))
        .with_attribute("open_issues", format!("{:.0}", repo.open_issues_count));

        observations.push(observation);
    }

    observations.sort_by_key(|o| o.observed_at);
    Ok(observations)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn fixture() -> Vec<u8> {
        include_bytes!("../../../tests/fixtures/github_repos_search.json").to_vec()
    }

    fn received() -> DateTime<Utc> {
        // After the most recent push in the fixture.
        Utc.timestamp_millis_opt(1_700_500_000_000)
            .single()
            .unwrap()
    }

    #[test]
    fn parses_active_repositories_only() {
        let observations = parse(&fixture(), received()).unwrap();
        assert_eq!(observations.len(), 2, "archived repo must be dropped");
        assert_eq!(observations[0].metric, "repo_stars");
        assert_eq!(observations[0].unit, "stars");
    }

    #[test]
    fn observed_at_comes_from_the_last_push() {
        let observations = parse(&fixture(), received()).unwrap();
        assert_eq!(
            observations[0].observed_at.to_rfc3339(),
            "2023-11-19T22:04:00+00:00"
        );
        assert!(observations[0].observed_at < observations[0].received_at);
    }

    #[test]
    fn repository_names_are_kept_for_drill_down() {
        let observations = parse(&fixture(), received()).unwrap();
        assert_eq!(
            observations[0].attributes.get("repo").map(String::as_str),
            Some("tokio-rs/tokio")
        );
        assert!(observations[0]
            .raw
            .locator
            .starts_with("https://github.com/"));
    }

    #[test]
    fn the_whole_ecosystem_is_one_series() {
        let observations = parse(&fixture(), received()).unwrap();
        let keys: Vec<String> = observations.iter().map(|o| o.series_key()).collect();
        assert!(keys.iter().all(|k| k == &keys[0]), "{keys:?}");
        assert!(keys[0].contains("ecosystem_rust"));
    }

    #[test]
    fn a_repo_without_a_push_time_is_skipped_not_timestamped_now() {
        let body = br#"[{"id":1,"full_name":"a/b","stargazers_count":5,"archived":false}]"#;
        assert!(parse(body, received()).unwrap().is_empty());
    }

    #[test]
    fn malformed_payload_is_a_parse_error() {
        // Valid JSON, wrong shape entirely.
        assert!(matches!(
            parse(br#"{"total_count":"oops","items":42}"#, received()),
            Err(CollectorError::Parse(_))
        ));
    }

    #[test]
    fn a_bare_repository_array_is_also_accepted() {
        // The raw list endpoint returns an array; the search endpoint an
        // envelope. Both must parse, so swapping endpoints cannot silently
        // produce zero observations.
        let body = br#"[{"id":1,"full_name":"a/b","html_url":"https://github.com/a/b",
            "stargazers_count":5,"pushed_at":"2023-11-19T22:04:00Z","archived":false}]"#;
        let observations = parse(body, received()).unwrap();
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].value, 5.0);
    }

    #[test]
    fn repositories_sharing_a_push_time_stay_distinct() {
        // The real GitHub shape that caused silent data loss: two repositories
        // pushed in the same second must produce two observations, not one.
        let body = br#"[
            {"id":1,"full_name":"a/one","html_url":"https://github.com/a/one",
             "stargazers_count":10,"pushed_at":"2023-11-19T22:04:00Z","archived":false},
            {"id":2,"full_name":"a/two","html_url":"https://github.com/a/two",
             "stargazers_count":20,"pushed_at":"2023-11-19T22:04:00Z","archived":false}
        ]"#;
        let observations = parse(body, received()).unwrap();
        assert_eq!(observations.len(), 2, "both repositories must be retained");
        assert_ne!(
            observations[0].id, observations[1].id,
            "same timestamp must not collapse two repositories into one id"
        );
        assert_eq!(
            observations[0].series_key(),
            observations[1].series_key(),
            "both are still one ecosystem series"
        );
    }

    #[test]
    fn an_unchanged_repository_still_de_duplicates() {
        // Identity must be stable, or every collection would re-insert.
        let body = br#"[{"id":1,"full_name":"a/one","html_url":"https://github.com/a/one",
            "stargazers_count":10,"pushed_at":"2023-11-19T22:04:00Z","archived":false}]"#;
        let first = parse(body, received()).unwrap();
        let second = parse(body, received()).unwrap();
        assert_eq!(first[0].id, second[0].id);
    }

    #[test]
    fn catalog_entry_is_hourly_and_token_authenticated() {
        let source = source();
        assert_eq!(
            source.cadence,
            wse_model::Cadence::Interval { seconds: 3600 }
        );
        assert_eq!(source.authentication, wse_model::AuthKind::Token);
    }
}
