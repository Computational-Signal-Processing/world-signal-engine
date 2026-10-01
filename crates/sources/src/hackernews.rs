//! Hacker News front-page activity — the technology/software source.
//!
//! ## Why the population is fixed
//!
//! The public Algolia HN Search endpoint returns *whatever* is on the front
//! page right now. Two collections return different stories, so a change in the
//! summed score is largely a change in *which stories we happened to sample*,
//! not a change in software attention. Interpreting that as a world measurement
//! is exactly the mistake this source used to make.
//!
//! This source instead tracks a **fixed universe**: it resolves the current
//! top-N story ids once, then re-measures the *same* ids on every later
//! collection by their item ids, which never change. A story's score rising is
//! then a real, comparable measurement of attention on a known story.
//!
//! ## How the fixed universe is discovered
//!
//! The HN Firebase API exposes the current top-story id list
//! (`v0/topstories.json`). The ids are the durable identity; the score behind
//! each id is what we track. On each collection the collector fetches the list,
//! reuses any ids already in its universe, and admits only enough new ids to
//! keep the universe at `UNIVERSE_SIZE`. Membership therefore grows slowly and
//! deliberately rather than churning every poll.
//!
//! API docs: <https://github.com/HackerNews/API>
//!
//! Authentication: none. The catalog marks this tier 3 (community signal) and
//! `fixed_universe`.

use std::collections::BTreeMap;

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use wse_collector::CollectorError;
use wse_model::{EntityId, Observation, RawReference, Source, SourceId};

pub const SOURCE_ID: &str = "hackernews_frontpage";
pub const COLLECTOR_TYPE: &str = "hackernews_universe";

/// The current top-story id list.
pub const TOP_STORIES_ENDPOINT: &str = "https://hacker-news.firebaseio.com/v0/topstories.json";
/// One story item, by id.
pub const ITEM_ENDPOINT: &str = "https://hacker-news.firebaseio.com/v0/item/{id}.json";

/// How many stories the fixed universe tracks.
pub const UNIVERSE_SIZE: usize = 30;

/// The catalog entry.
pub fn source() -> Source {
    let mut parameters = std::collections::BTreeMap::new();
    parameters.insert("universe_size".to_string(), UNIVERSE_SIZE.to_string());

    Source {
        id: SourceId::new(SOURCE_ID),
        name: "Hacker News Front Page".to_string(),
        provider: "Y Combinator / Algolia".to_string(),
        category: "technology".to_string(),
        subcategory: Some("developer_attention".to_string()),
        endpoint: TOP_STORIES_ENDPOINT.to_string(),
        protocol: wse_model::Protocol::Https,
        format: wse_model::DataFormat::Json,
        cadence: wse_model::Cadence::Interval { seconds: 600 },
        timezone: Some("UTC".to_string()),
        license: Some("Public API, no key required (Hacker News)".to_string()),
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
        tier: wse_model::SourceTier::Tier3,
        measurement: wse_model::MeasurementSemantics::FixedUniverse,
        feeds_lenses: vec!["lens_software".to_string()],
    }
}

/// A story item as the Firebase API returns it.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Item {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub by: Option<String>,
    #[serde(default)]
    pub score: Option<f64>,
    #[serde(default)]
    pub descendants: Option<f64>,
    /// Epoch seconds.
    #[serde(default)]
    pub time: Option<i64>,
    #[serde(default, rename = "type")]
    pub item_type: Option<String>,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub dead: bool,
}

/// Parse the top-story id list.
pub fn parse_ids(body: &[u8]) -> Result<Vec<i64>, CollectorError> {
    serde_json::from_slice(body)
        .map_err(|e| CollectorError::Parse(format!("hackernews topstories: {e}")))
}

/// Parse one story item into at most one observation.
///
/// Returns `Ok(None)` for a comment, a deleted/dead item, or one with no score
/// — none of which is a front-page story measurement.
pub fn parse_item(
    body: &[u8],
    observed_at: DateTime<Utc>,
) -> Result<Option<Observation>, CollectorError> {
    let item: Item = serde_json::from_slice(body)
        .map_err(|e| CollectorError::Parse(format!("hackernews item: {e}")))?;

    if item.deleted || item.dead {
        return Ok(None);
    }
    if item.item_type.as_deref() == Some("comment") {
        return Ok(None);
    }
    let (Some(title), Some(score)) = (item.title.clone(), item.score) else {
        return Ok(None);
    };
    let Some(observed_at_item) = item.time.and_then(|s| Utc.timestamp_opt(s, 0).single()) else {
        return Ok(None);
    };

    let source_id = SourceId::new(SOURCE_ID);
    let entity = EntityId::new("software_ecosystem");
    let hash = wse_model::fnv1a_hex(&String::from_utf8_lossy(body));
    let locator = item
        .url
        .clone()
        .unwrap_or_else(|| format!("https://news.ycombinator.com/item?id={}", item.id));
    let raw = RawReference {
        locator,
        hash,
        content_type: Some("application/json".to_string()),
        bytes: Some(body.len() as u64),
    };

    let mut observation = Observation::new(
        source_id,
        Some(entity),
        "story_score",
        score,
        "points",
        // The score is the current value; observed_at is when we looked, so the
        // series' timestamps track collection rather than the story's birthday.
        observed_at,
        raw,
    )
    .with_received_at(observed_at)
    .with_identity(item.id.to_string())
    .with_attribute("story_id", item.id.to_string())
    .with_attribute("title", title)
    .with_attribute("posted_at", observed_at_item.to_rfc3339());

    if let Some(author) = &item.by {
        observation = observation.with_attribute("author", author.clone());
    }
    if let Some(comments) = item.descendants {
        observation = observation.with_attribute("comments", format!("{comments:.0}"));
    }

    Ok(Some(observation))
}

/// Choose the next fixed universe of story ids.
///
/// Ids already tracked are kept (their scores remain comparable across
/// collections); only enough new ids from `candidates` are admitted to refill
/// the universe to `size`. Order is preserved so the universe is deterministic.
pub fn next_universe(current: &[i64], candidates: &[i64], size: usize) -> Vec<i64> {
    let mut universe: Vec<i64> = current.iter().copied().take(size).collect();
    let known: std::collections::HashSet<i64> = universe.iter().copied().collect();
    for id in candidates {
        if universe.len() >= size {
            break;
        }
        if !known.contains(id) {
            universe.push(*id);
        }
    }
    universe
}

/// Parse a batch of items keyed by story id into observations.
///
/// Used by the collector once it has resolved the fixed universe.
pub fn parse_items(
    items: &BTreeMap<i64, Vec<u8>>,
    observed_at: DateTime<Utc>,
) -> Result<Vec<Observation>, CollectorError> {
    let mut observations = Vec::new();
    for body in items.values() {
        if let Some(observation) = parse_item(body, observed_at)? {
            observations.push(observation);
        }
    }
    observations.sort_by_key(|o| o.observed_at);
    Ok(observations)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn received() -> DateTime<Utc> {
        Utc.timestamp_millis_opt(1_700_001_000_000)
            .single()
            .unwrap()
    }

    fn item_fixture() -> Vec<u8> {
        include_bytes!("../../../tests/fixtures/hackernews_item.json").to_vec()
    }

    #[test]
    fn parses_a_story_item() {
        let observation = parse_item(&item_fixture(), received()).unwrap().unwrap();
        assert_eq!(observation.metric, "story_score");
        assert_eq!(observation.unit, "points");
        assert_eq!(observation.observed_at, received());
    }

    #[test]
    fn the_story_id_is_the_record_identity() {
        let observation = parse_item(&item_fixture(), received()).unwrap().unwrap();
        assert_eq!(observation.identity.as_deref(), Some("41000001"));
        assert!(observation.raw.locator.starts_with("https://"));
    }

    #[test]
    fn comments_and_dead_items_are_skipped() {
        let comment = br#"{"id":1,"type":"comment","score":5,"time":1700000000}"#;
        assert!(parse_item(comment, received()).unwrap().is_none());
        let dead =
            br#"{"id":1,"type":"story","title":"t","score":5,"time":1700000000,"dead":true}"#;
        assert!(parse_item(dead, received()).unwrap().is_none());
        let no_score = br#"{"id":1,"type":"story","title":"t","time":1700000000}"#;
        assert!(parse_item(no_score, received()).unwrap().is_none());
    }

    #[test]
    fn the_top_story_list_parses() {
        let ids = parse_ids(b"[41000001,41000002,41000003]").unwrap();
        assert_eq!(ids, vec![41000001, 41000002, 41000003]);
    }

    #[test]
    fn the_universe_keeps_known_ids_and_fills_the_rest() {
        // Two tracked ids stay; the universe is topped up from the candidates.
        let current = vec![10, 11];
        let candidates = vec![11, 12, 13, 14];
        let next = next_universe(&current, &candidates, 4);
        assert_eq!(next, vec![10, 11, 12, 13]);
    }

    #[test]
    fn the_universe_never_exceeds_its_size() {
        let current: Vec<i64> = (0..30).collect();
        let next = next_universe(&current, &(100..200).collect::<Vec<_>>(), 30);
        assert_eq!(next.len(), 30);
        assert_eq!(next, current, "a full universe is not churned");
    }

    #[test]
    fn malformed_payload_is_a_parse_error() {
        assert!(matches!(
            parse_item(b"nope", received()),
            Err(CollectorError::Parse(_))
        ));
        assert!(parse_ids(b"nope").is_err());
    }

    #[test]
    fn the_catalog_declares_a_fixed_universe_and_no_auth() {
        let source = source();
        assert_eq!(source.authentication, wse_model::AuthKind::None);
        assert_eq!(
            source.measurement,
            wse_model::MeasurementSemantics::FixedUniverse
        );
        assert_eq!(source.tier, wse_model::SourceTier::Tier3);
        assert!(source.feeds_lenses.contains(&"lens_software".to_string()));
    }
}
