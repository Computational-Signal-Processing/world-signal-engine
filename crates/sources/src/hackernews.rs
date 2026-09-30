//! Hacker News front-page activity — the technology/software source.
//!
//! Uses the public Algolia HN Search API, which needs no key. The measured
//! quantity is the score of front-page stories in a tag: a jump in the score
//! distribution or in the number of stories is a real change in what the
//! software world is paying attention to.
//!
//! API docs: <https://hn.algolia.com/api>

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use wse_collector::CollectorError;
use wse_model::{EntityId, Observation, RawReference, Source, SourceId};

pub const SOURCE_ID: &str = "hackernews_frontpage";
pub const COLLECTOR_TYPE: &str = "hackernews_search";

/// Front-page stories, newest first.
pub const API_ENDPOINT: &str = "https://hn.algolia.com/api/v1/search_by_date";

/// The catalog entry.
pub fn source() -> Source {
    let mut parameters = std::collections::BTreeMap::new();
    parameters.insert("tags".to_string(), "front_page".to_string());
    parameters.insert("hits_per_page".to_string(), "50".to_string());

    Source {
        id: SourceId::new(SOURCE_ID),
        name: "Hacker News Front Page".to_string(),
        provider: "Y Combinator / Algolia".to_string(),
        category: "technology".to_string(),
        subcategory: Some("developer_attention".to_string()),
        endpoint: API_ENDPOINT.to_string(),
        protocol: wse_model::Protocol::Https,
        format: wse_model::DataFormat::Json,
        cadence: wse_model::Cadence::Interval { seconds: 600 },
        timezone: Some("UTC".to_string()),
        license: Some("Public API, no key required (Algolia HN Search)".to_string()),
        authentication: wse_model::AuthKind::None,
        cost: wse_model::Cost::Free,
        historical_available: true,
        realtime_available: true,
        geospatial: false,
        entities: vec!["software".to_string()],
        priority: 40,
        enabled: true,
        collector_type: COLLECTOR_TYPE.to_string(),
        parameters,
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SearchResponse {
    #[serde(default)]
    pub hits: Vec<Hit>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Hit {
    #[serde(rename = "objectID", default)]
    pub object_id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub points: Option<f64>,
    #[serde(default)]
    pub num_comments: Option<f64>,
    /// Epoch seconds.
    #[serde(default)]
    pub created_at_i: Option<i64>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(rename = "_tags", default)]
    pub tags: Vec<String>,
}

/// Parse an Algolia HN response into observations.
///
/// One observation per story, measuring its score. The entity is the software
/// ecosystem as a whole: the series is "how much attention software stories
/// are getting", and it only becomes a signal when the distribution shifts.
pub fn parse(body: &[u8], received_at: DateTime<Utc>) -> Result<Vec<Observation>, CollectorError> {
    let response: SearchResponse = serde_json::from_slice(body)
        .map_err(|e| CollectorError::Parse(format!("hackernews search: {e}")))?;

    let source_id = SourceId::new(SOURCE_ID);
    let entity = EntityId::new("software_ecosystem");
    let hash = wse_model::fnv1a_hex(&String::from_utf8_lossy(body));
    let mut observations = Vec::new();

    for hit in response.hits {
        // Comments have no score and no title; they are not front-page stories.
        if hit.tags.iter().any(|t| t == "comment") {
            continue;
        }
        let (Some(title), Some(points)) = (hit.title.clone(), hit.points) else {
            continue;
        };
        let Some(observed_at) = hit
            .created_at_i
            .and_then(|s| Utc.timestamp_opt(s, 0).single())
            .or_else(|| {
                hit.created_at
                    .as_deref()
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|d| d.with_timezone(&Utc))
            })
        else {
            continue;
        };

        let locator = hit
            .url
            .clone()
            .unwrap_or_else(|| format!("https://news.ycombinator.com/item?id={}", hit.object_id));
        let raw = RawReference {
            locator,
            hash: hash.clone(),
            content_type: Some("application/json".to_string()),
            bytes: Some(body.len() as u64),
        };

        let mut observation = Observation::new(
            source_id.clone(),
            Some(entity.clone()),
            "story_score",
            points,
            "points",
            observed_at,
            raw,
        )
        .with_received_at(received_at)
        .with_attribute("story_id", hit.object_id.clone())
        .with_attribute("title", title);

        if let Some(author) = &hit.author {
            observation = observation.with_attribute("author", author.clone());
        }
        if let Some(comments) = hit.num_comments {
            observation = observation.with_attribute("comments", format!("{comments:.0}"));
        }

        observations.push(observation);
    }

    observations.sort_by_key(|o| o.observed_at);
    Ok(observations)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        include_bytes!("../../../tests/fixtures/hackernews_search.json").to_vec()
    }

    fn received() -> DateTime<Utc> {
        Utc.timestamp_millis_opt(1_700_001_000_000)
            .single()
            .unwrap()
    }

    #[test]
    fn parses_stories_and_ignores_comments() {
        let observations = parse(&fixture(), received()).unwrap();
        assert_eq!(observations.len(), 3, "the comment must be dropped");
        assert_eq!(observations[0].metric, "story_score");
        assert_eq!(observations[0].unit, "points");
    }

    #[test]
    fn epoch_seconds_become_utc_timestamps() {
        let observations = parse(&fixture(), received()).unwrap();
        assert_eq!(observations[0].observed_at.timestamp(), 1_700_000_100);
    }

    #[test]
    fn stories_share_one_series_key_so_attention_can_be_tracked() {
        let observations = parse(&fixture(), received()).unwrap();
        let keys: Vec<String> = observations.iter().map(|o| o.series_key()).collect();
        assert!(keys.iter().all(|k| k == &keys[0]), "{keys:?}");
    }

    #[test]
    fn the_story_title_is_kept_for_explanation() {
        let observations = parse(&fixture(), received()).unwrap();
        assert_eq!(
            observations[0].attributes.get("title").map(String::as_str),
            Some("Show HN: A tiny deterministic signal engine in Rust")
        );
        assert!(observations[0].raw.locator.starts_with("https://"));
    }

    #[test]
    fn a_story_without_points_is_skipped() {
        let body = br#"{"hits":[{"objectID":"1","title":"t","created_at_i":1700000000,
            "points":null,"_tags":["story"]}]}"#;
        assert!(parse(body, received()).unwrap().is_empty());
    }

    #[test]
    fn malformed_payload_is_a_parse_error() {
        assert!(matches!(
            parse(b"nope", received()),
            Err(CollectorError::Parse(_))
        ));
    }

    #[test]
    fn catalog_entry_needs_no_auth() {
        let source = source();
        assert_eq!(source.authentication, wse_model::AuthKind::None);
        assert_eq!(source.cost, wse_model::Cost::Free);
    }
}
