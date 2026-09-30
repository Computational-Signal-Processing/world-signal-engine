//! # wse-correlation
//!
//! The convergence engine: do independent sources point at the same change?
//!
//! The brief's example:
//!
//! ```text
//! oil_price ↑
//! shipping_delay ↑
//! port_activity ↓
//! news_mentions ↑
//! ```
//!
//! Three of those move up and one moves down, so a naive "same direction"
//! rule would miss the story. The engine therefore groups by *entity and time
//! window* first, and reports which directions are involved, leaving the
//! interpretation to the signal engine and the human.
//!
//! The implementation is deterministic: entity, geography, time window,
//! category and direction. No graph, no ML.
//!
//! ## How entities are matched
//!
//! Exact equality was the original rule, and it was too strict to be useful.
//! Real entity ids are prefixed slugs produced by the collectors
//! (`region_san_francisco_bay_area`, `region_san_francisco`, `topic_oil_price`),
//! so the same place or topic arriving from two providers under slightly
//! different names never converged — which is precisely the case convergence
//! exists to catch.
//!
//! Two entities are now related when one's canonical segment set is a *subset*
//! of the other's, sharing at least two segments:
//!
//! ```text
//! {region, san, francisco}          ⊆ {region, san, francisco, bay, area}  related
//! {region, south, fiji}             ⊄ {region, south, tonga}                distinct
//! ```
//!
//! Subset rather than plain overlap, because overlap would declare
//! `region_south_fiji` and `region_south_tonga` the same place on the strength
//! of the shared word "south". Requiring two shared segments stops the bare
//! prefix (`region`) from matching everything.
//!
//! ## How places are matched
//!
//! Candidates carry coordinates, and the brief names geography as a grouping
//! dimension. When entities are not enough, candidates within
//! [`ConvergenceConfig::radius_km`] of each other can converge on location —
//! but only when [`ConvergenceConfig::merge_mode`] is [`MergeMode::Related`].
//! At the default [`MergeMode::Exact`] a missing entity means a candidate
//! groups by series alone, exactly as before.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use wse_model::geo::haversine_km;
use wse_model::{AnomalyId, CandidateDirection, EntityId, SourceId};

/// How aggressively candidates are allowed to be considered "about the same
/// thing".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeMode {
    /// Two candidates share an entity only when the ids are identical, and
    /// geography is not used for grouping at all.
    ///
    /// This is the default, so existing behaviour is unchanged: entity-less
    /// candidates group by series and nothing else.
    #[default]
    Exact,
    /// Entity ids may be related rather than identical, and candidates that
    /// share no entity may still converge on geography.
    Related,
}

/// Tunables for convergence detection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConvergenceConfig {
    /// Candidates further apart than this cannot belong to one group.
    pub window_seconds: i64,
    /// Minimum distinct sources that must agree.
    pub min_sources: usize,
    /// Minimum distinct series (metrics) that must agree.
    pub min_series: usize,
    /// How entities and places are allowed to match.
    #[serde(default)]
    pub merge_mode: MergeMode,
    /// Radius, in kilometres, within which two candidates count as sharing a
    /// location. Only consulted under [`MergeMode::Related`].
    pub radius_km: f64,
    /// Minimum segments two entity ids must share to be related.
    pub min_shared_segments: usize,
}

impl Default for ConvergenceConfig {
    fn default() -> Self {
        Self {
            window_seconds: 6 * 3600,
            min_sources: 2,
            min_series: 2,
            merge_mode: MergeMode::Exact,
            radius_km: 250.0,
            min_shared_segments: 2,
        }
    }
}

impl ConvergenceConfig {
    /// A configuration that also matches related entities and places.
    pub fn related() -> Self {
        Self {
            merge_mode: MergeMode::Related,
            ..Self::default()
        }
    }

    pub fn with_radius_km(mut self, radius_km: f64) -> Self {
        self.radius_km = radius_km;
        self
    }

    pub fn with_window_seconds(mut self, window_seconds: i64) -> Self {
        self.window_seconds = window_seconds;
        self
    }
}

/// Why a group's members were considered to be about the same thing.
///
/// Reported rather than assumed, so a signal can say *how* its sources agree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchKind {
    /// The candidates share an identical entity id.
    ExactEntity,
    /// The candidates' entity ids overlap (`region_san_francisco` and
    /// `region_san_francisco_bay_area`).
    RelatedEntity,
    /// The candidates carry coordinates within the configured radius.
    Geography,
    /// No shared entity or place: grouped by series key.
    Series,
}

impl MatchKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            MatchKind::ExactEntity => "exact_entity",
            MatchKind::RelatedEntity => "related_entity",
            MatchKind::Geography => "geography",
            MatchKind::Series => "series",
        }
    }
}

/// A set of independent candidates that describe the same change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConvergenceGroup {
    /// Entity the group is about, when the candidates share one.
    pub entity_id: Option<EntityId>,
    /// Every distinct entity id in the group. More than one when the members
    /// matched as related rather than identical.
    pub entity_ids: Vec<EntityId>,
    /// Key used for grouping when there is no entity (the series key).
    pub group_key: String,
    /// How the members were judged to be about the same thing.
    pub match_kinds: Vec<MatchKind>,
    /// Directions observed within the group, most common first.
    pub directions: Vec<CandidateDirection>,
    /// The single direction shared by the majority, if any.
    pub dominant_direction: CandidateDirection,
    pub source_ids: Vec<SourceId>,
    pub series_keys: Vec<String>,
    pub candidate_ids: Vec<AnomalyId>,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    /// `0..=1`: how strong the agreement is.
    pub strength: f64,
}

impl ConvergenceGroup {
    pub fn source_count(&self) -> usize {
        self.source_ids.len()
    }

    pub fn series_count(&self) -> usize {
        self.series_keys.len()
    }

    pub fn duration_seconds(&self) -> i64 {
        (self.last_seen - self.first_seen).num_seconds().max(0)
    }

    /// Whether the group matched on something beyond the series key.
    pub fn is_entity_or_geographic(&self) -> bool {
        self.match_kinds
            .iter()
            .any(|k| !matches!(k, MatchKind::Series))
    }
}

/// Detect convergence among a batch of candidates.
///
/// The input is the same candidate set the event engine sees; convergence is a
/// property *across* candidates, so it is computed before signals are formed.
pub fn detect_convergence(
    candidates: &[wse_model::AnomalyCandidate],
    config: &ConvergenceConfig,
) -> Vec<ConvergenceGroup> {
    let mut groups = match config.merge_mode {
        MergeMode::Exact => detect_by_exact_key(candidates, config),
        MergeMode::Related => detect_by_related_key(candidates, config),
    };
    groups.extend(detect_geographic(candidates, config));

    // Deterministic ordering. The tiebreaker on `group_key` matters: without
    // it, groups of equal strength came out in hash order, which varies per
    // run and would make a replayed backtest non-reproducible.
    groups.sort_by(|a, b| {
        b.strength
            .partial_cmp(&a.strength)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.first_seen.cmp(&b.first_seen))
            .then_with(|| a.group_key.cmp(&b.group_key))
    });
    groups
}

/// The original grouping: entity when present, series key otherwise.
fn detect_by_exact_key(
    candidates: &[wse_model::AnomalyCandidate],
    config: &ConvergenceConfig,
) -> Vec<ConvergenceGroup> {
    // BTreeMap, not HashMap: iteration order must not depend on hashing.
    let mut buckets: BTreeMap<String, Vec<&wse_model::AnomalyCandidate>> = BTreeMap::new();
    for c in candidates {
        let key = c
            .entity_id
            .as_ref()
            .map(|e| e.as_str().to_string())
            .unwrap_or_else(|| c.series_key.clone());
        buckets.entry(key).or_default().push(c);
    }

    let mut groups = Vec::new();
    for (key, items) in buckets {
        let kind = if items[0].entity_id.is_some() {
            MatchKind::ExactEntity
        } else {
            MatchKind::Series
        };
        groups.extend(windows_for(&key, &items, config, kind));
    }
    groups
}

/// Grouping where entity ids may be *related* rather than identical.
///
/// Candidates are bucketed by their canonical entity segments, so
/// `region_san_francisco` and `region_san_francisco_bay_area` land in the same
/// bucket and are then confirmed by the subset rule. Candidates whose entity
/// has no related partner fall back to their series key.
fn detect_by_related_key(
    candidates: &[wse_model::AnomalyCandidate],
    config: &ConvergenceConfig,
) -> Vec<ConvergenceGroup> {
    // Union-find over related entities, then group candidates by representative.
    let entities: Vec<EntityId> = {
        let mut ids: Vec<EntityId> = candidates
            .iter()
            .filter_map(|c| c.entity_id.clone())
            .collect();
        ids.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        ids.dedup();
        ids
    };

    let mut parent: Vec<usize> = (0..entities.len()).collect();
    for i in 0..entities.len() {
        for j in (i + 1)..entities.len() {
            if entities_related(&entities[i], &entities[j], config.min_shared_segments) {
                union(&mut parent, i, j);
            }
        }
    }

    let mut buckets: BTreeMap<String, Vec<&wse_model::AnomalyCandidate>> = BTreeMap::new();
    for c in candidates {
        let key = match c.entity_id.as_ref() {
            Some(entity) => {
                let index = entities
                    .iter()
                    .position(|e| e == entity)
                    .expect("entity came from the candidate list");
                let root = find(&mut parent, index);
                format!("entity:{}", entities[root].as_str())
            }
            None => format!("series:{}", c.series_key),
        };
        buckets.entry(key).or_default().push(c);
    }

    let mut groups = Vec::new();
    for (key, items) in buckets {
        let distinct_entities = items
            .iter()
            .filter_map(|c| c.entity_id.as_ref())
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        let kind = if items[0].entity_id.is_none() {
            MatchKind::Series
        } else if distinct_entities > 1 {
            MatchKind::RelatedEntity
        } else {
            MatchKind::ExactEntity
        };
        groups.extend(windows_for(&key, &items, config, kind));
    }
    groups
}

/// Candidates that share no entity may still converge on where they happened.
fn detect_geographic(
    candidates: &[wse_model::AnomalyCandidate],
    config: &ConvergenceConfig,
) -> Vec<ConvergenceGroup> {
    if config.merge_mode != MergeMode::Related {
        return Vec::new();
    }

    let located: Vec<&wse_model::AnomalyCandidate> = candidates
        .iter()
        .filter(|c| c.latitude.is_some() && c.longitude.is_some())
        .collect();
    if located.len() < 2 {
        return Vec::new();
    }

    // Cluster by connected components: two candidates are in the same place if
    // they are within the radius. Transitive, so a chain of nearby points forms
    // one place rather than an arbitrary split.
    let mut parent: Vec<usize> = (0..located.len()).collect();
    for i in 0..located.len() {
        for j in (i + 1)..located.len() {
            let (Some(lat1), Some(lon1)) = (located[i].latitude, located[i].longitude) else {
                continue;
            };
            let (Some(lat2), Some(lon2)) = (located[j].latitude, located[j].longitude) else {
                continue;
            };
            if haversine_km(lat1, lon1, lat2, lon2) <= config.radius_km {
                union(&mut parent, i, j);
            }
        }
    }

    let mut clusters: BTreeMap<usize, Vec<&wse_model::AnomalyCandidate>> = BTreeMap::new();
    for (index, candidate) in located.iter().enumerate() {
        let root = find(&mut parent, index);
        clusters.entry(root).or_default().push(candidate);
    }

    let mut groups = Vec::new();
    for (root, items) in clusters {
        if items.len() < 2 {
            continue;
        }
        // Name the cluster after its first coordinate so the key is stable
        // across runs regardless of how the cluster was built.
        let (Some(lat), Some(lon)) = (items[0].latitude, items[0].longitude) else {
            continue;
        };
        let key = format!("geo:{root}:{lat:.3},{lon:.3}");
        groups.extend(windows_for(&key, &items, config, MatchKind::Geography));
    }
    groups
}

/// Slide a time window across one bucket and build a group per window.
fn windows_for(
    key: &str,
    items: &[&wse_model::AnomalyCandidate],
    config: &ConvergenceConfig,
    kind: MatchKind,
) -> Vec<ConvergenceGroup> {
    let mut sorted = items.to_vec();
    sorted.sort_by_key(|c| c.observed_at);

    let mut groups = Vec::new();
    let mut start = 0;
    while start < sorted.len() {
        let window_start = sorted[start].observed_at;
        let mut end = start;
        while end < sorted.len()
            && (sorted[end].observed_at - window_start).num_seconds() <= config.window_seconds
        {
            end += 1;
        }
        if let Some(group) = build_group(key, &sorted[start..end], config, kind) {
            groups.push(group);
        }
        start = end;
    }
    groups
}

fn build_group(
    key: &str,
    window: &[&wse_model::AnomalyCandidate],
    config: &ConvergenceConfig,
    kind: MatchKind,
) -> Option<ConvergenceGroup> {
    let mut sources: Vec<SourceId> = Vec::new();
    let mut series: Vec<String> = Vec::new();
    let mut entities: Vec<EntityId> = Vec::new();
    let mut ids = Vec::new();
    for c in window {
        if !sources.contains(&c.source_id) {
            sources.push(c.source_id.clone());
        }
        if !series.contains(&c.series_key) {
            series.push(c.series_key.clone());
        }
        if let Some(entity) = &c.entity_id {
            if !entities.contains(entity) {
                entities.push(entity.clone());
            }
        }
        ids.push(c.id.clone());
    }

    if sources.len() < config.min_sources || series.len() < config.min_series {
        return None;
    }

    let mut directions: Vec<CandidateDirection> = Vec::new();
    for c in window {
        if !directions.contains(&c.direction) {
            directions.push(c.direction);
        }
    }
    let dominant = dominant_direction(window);

    // Strength grows with source agreement and with the magnitude of the
    // deviations involved, and is capped at 1.
    let agreement = (sources.len() as f64 / (config.min_sources as f64 * 2.0)).min(1.0);
    let magnitude = mean_abs_score(window);
    let magnitude_score = (magnitude / 4.0).min(1.0);
    let strength = (0.6 * agreement + 0.4 * magnitude_score).clamp(0.0, 1.0);

    let first_seen = window.first()?.observed_at;
    let last_seen = window.last()?.observed_at;

    Some(ConvergenceGroup {
        entity_id: entities.first().cloned(),
        entity_ids: entities,
        group_key: key.to_string(),
        match_kinds: vec![kind],
        directions,
        dominant_direction: dominant,
        source_ids: sources,
        series_keys: series,
        candidate_ids: ids,
        first_seen,
        last_seen,
        strength,
    })
}

/// Whether two entity ids name the same thing closely enough to converge.
///
/// One segment set must be a subset of the other, and they must share at least
/// `min_shared` segments. The subset requirement is what keeps
/// `region_south_fiji` and `region_south_tonga` apart despite the shared word.
pub fn entities_related(a: &EntityId, b: &EntityId, min_shared: usize) -> bool {
    let left = entity_segments(a.as_str());
    let right = entity_segments(b.as_str());
    if left.is_empty() || right.is_empty() {
        return false;
    }
    let shared = left.iter().filter(|s| right.contains(s)).count();
    if shared < min_shared {
        return false;
    }
    let left_subset = left.iter().all(|s| right.contains(s));
    let right_subset = right.iter().all(|s| left.contains(s));
    left_subset || right_subset
}

/// Split an entity id into canonical, comparable segments.
fn entity_segments(entity_id: &str) -> Vec<String> {
    entity_id
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(wse_model::canonicalize)
        .collect()
}

fn find(parent: &mut [usize], mut index: usize) -> usize {
    while parent[index] != index {
        parent[index] = parent[parent[index]];
        index = parent[index];
    }
    index
}

fn union(parent: &mut [usize], a: usize, b: usize) {
    let (ra, rb) = (find(parent, a), find(parent, b));
    if ra != rb {
        // Keep the lower index as the root so cluster naming is deterministic.
        let (low, high) = if ra < rb { (ra, rb) } else { (rb, ra) };
        parent[high] = low;
    }
}

/// The direction held by the most candidates, ties broken by `Flat`.
fn dominant_direction(window: &[&wse_model::AnomalyCandidate]) -> CandidateDirection {
    let mut up = 0;
    let mut down = 0;
    for c in window {
        match c.direction {
            CandidateDirection::Up => up += 1,
            CandidateDirection::Down => down += 1,
            CandidateDirection::Flat => {}
        }
    }
    match up.cmp(&down) {
        std::cmp::Ordering::Greater => CandidateDirection::Up,
        std::cmp::Ordering::Less => CandidateDirection::Down,
        std::cmp::Ordering::Equal => CandidateDirection::Flat,
    }
}

fn mean_abs_score(window: &[&wse_model::AnomalyCandidate]) -> f64 {
    if window.is_empty() {
        return 0.0;
    }
    window.iter().map(|c| c.score.abs()).sum::<f64>() / window.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use wse_model::{
        AnomalyCandidate, BaselineSnapshot, CandidateKind, DetectionMethod, EntityId, ObservationId,
    };

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
    }

    fn snap() -> BaselineSnapshot {
        BaselineSnapshot {
            sample_size: 50,
            mean: 100.0,
            median: 100.0,
            std_dev: 1.0,
            mad: 1.0,
            p05: 98.0,
            p95: 102.0,
            ewma: 100.0,
            trend_per_second: 0.0,
            volatility: 1.0,
        }
    }

    fn candidate(
        source: &str,
        series: &str,
        entity: Option<&str>,
        direction: CandidateDirection,
        score: f64,
        secs: i64,
    ) -> AnomalyCandidate {
        let mut c = AnomalyCandidate::new(
            series,
            ObservationId::new(format!("obs_{series}_{secs}")),
            at(secs),
            snap(),
            100.0 + score,
        );
        c.source_id = SourceId::new(source);
        c.entity_id = entity.map(EntityId::new);
        c.direction = direction;
        c.score = score;
        c.kind = CandidateKind::Anomaly;
        c.method = DetectionMethod::RobustZScore;
        c
    }

    fn located(mut c: AnomalyCandidate, lat: f64, lon: f64) -> AnomalyCandidate {
        c.latitude = Some(lat);
        c.longitude = Some(lon);
        c
    }

    #[test]
    fn independent_sources_on_same_entity_converge() {
        let candidates = vec![
            candidate(
                "src_a",
                "a::oil::price::usd",
                Some("ent_hormuz"),
                CandidateDirection::Up,
                4.0,
                0,
            ),
            candidate(
                "src_b",
                "b::shipping::delay::h",
                Some("ent_hormuz"),
                CandidateDirection::Up,
                3.5,
                60,
            ),
            candidate(
                "src_c",
                "c::news::mentions::n",
                Some("ent_hormuz"),
                CandidateDirection::Up,
                3.0,
                120,
            ),
        ];
        let groups = detect_convergence(&candidates, &ConvergenceConfig::default());
        assert_eq!(groups.len(), 1);
        let g = &groups[0];
        assert_eq!(g.source_count(), 3);
        assert_eq!(g.series_count(), 3);
        assert_eq!(g.dominant_direction, CandidateDirection::Up);
        assert!(g.strength > 0.5);
        assert_eq!(g.match_kinds, vec![MatchKind::ExactEntity]);
    }

    #[test]
    fn a_single_source_does_not_converge() {
        let candidates = vec![
            candidate(
                "src_a",
                "a::oil::price::usd",
                Some("ent_hormuz"),
                CandidateDirection::Up,
                4.0,
                0,
            ),
            candidate(
                "src_a",
                "a::oil::volume::bbl",
                Some("ent_hormuz"),
                CandidateDirection::Up,
                3.5,
                60,
            ),
        ];
        assert!(detect_convergence(&candidates, &ConvergenceConfig::default()).is_empty());
    }

    #[test]
    fn sources_outside_the_window_do_not_converge() {
        let candidates = vec![
            candidate(
                "src_a",
                "a::oil::price::usd",
                Some("ent_hormuz"),
                CandidateDirection::Up,
                4.0,
                0,
            ),
            candidate(
                "src_b",
                "b::shipping::delay::h",
                Some("ent_hormuz"),
                CandidateDirection::Up,
                4.0,
                100_000,
            ),
        ];
        assert!(detect_convergence(&candidates, &ConvergenceConfig::default()).is_empty());
    }

    #[test]
    fn mixed_directions_still_group_but_report_a_dominant() {
        // The brief's example: three up, one down.
        let candidates = vec![
            candidate(
                "src_a",
                "a::oil::price::usd",
                Some("ent_hormuz"),
                CandidateDirection::Up,
                4.0,
                0,
            ),
            candidate(
                "src_b",
                "b::shipping::delay::h",
                Some("ent_hormuz"),
                CandidateDirection::Up,
                4.0,
                10,
            ),
            candidate(
                "src_c",
                "c::news::mentions::n",
                Some("ent_hormuz"),
                CandidateDirection::Up,
                4.0,
                20,
            ),
            candidate(
                "src_d",
                "d::port::activity::teu",
                Some("ent_hormuz"),
                CandidateDirection::Down,
                4.0,
                30,
            ),
        ];
        let groups = detect_convergence(&candidates, &ConvergenceConfig::default());
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].dominant_direction, CandidateDirection::Up);
        assert_eq!(groups[0].directions.len(), 2);
    }

    #[test]
    fn entity_less_candidates_group_by_series_not_globally() {
        let candidates = vec![
            candidate("src_a", "a::x::m::u", None, CandidateDirection::Up, 4.0, 0),
            candidate("src_b", "b::y::m::u", None, CandidateDirection::Up, 4.0, 10),
        ];
        // Different series, no shared entity: not convergent.
        assert!(detect_convergence(&candidates, &ConvergenceConfig::default()).is_empty());
    }

    #[test]
    fn empty_input_is_safe() {
        assert!(detect_convergence(&[], &ConvergenceConfig::default()).is_empty());
    }

    // --- Phase 11: related entity matching ---------------------------------

    #[test]
    fn a_qualified_name_related_to_its_bare_name() {
        // The case exact matching missed: two providers, same place, one more
        // specific.
        let candidates = vec![
            candidate(
                "src_a",
                "a::quake::count::n",
                Some("region_san_francisco"),
                CandidateDirection::Up,
                4.0,
                0,
            ),
            candidate(
                "src_b",
                "b::shipping::delay::h",
                Some("region_san_francisco_bay_area"),
                CandidateDirection::Up,
                4.0,
                60,
            ),
        ];

        // Exact mode: different ids, so no convergence.
        assert!(detect_convergence(&candidates, &ConvergenceConfig::default()).is_empty());

        // Related mode: same place, so they converge.
        let groups = detect_convergence(&candidates, &ConvergenceConfig::related());
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].source_count(), 2);
        assert_eq!(groups[0].match_kinds, vec![MatchKind::RelatedEntity]);
        assert_eq!(groups[0].entity_ids.len(), 2);
        assert!(groups[0].is_entity_or_geographic());
    }

    #[test]
    fn unrelated_places_sharing_a_word_do_not_converge() {
        // `region_south_fiji` and `region_south_tonga` share "region" and
        // "south". Neither is a subset of the other, so they stay apart.
        let candidates = vec![
            candidate(
                "src_a",
                "a::quake::count::n",
                Some("region_south_fiji"),
                CandidateDirection::Up,
                4.0,
                0,
            ),
            candidate(
                "src_b",
                "b::shipping::delay::h",
                Some("region_south_tonga"),
                CandidateDirection::Up,
                4.0,
                60,
            ),
        ];
        assert!(detect_convergence(&candidates, &ConvergenceConfig::related()).is_empty());
    }

    #[test]
    fn a_shared_prefix_alone_is_not_enough() {
        // `region_fiji` and `region_tonga` share only the bare prefix.
        let candidates = vec![
            candidate(
                "src_a",
                "a::quake::count::n",
                Some("region_fiji"),
                CandidateDirection::Up,
                4.0,
                0,
            ),
            candidate(
                "src_b",
                "b::shipping::delay::h",
                Some("region_tonga"),
                CandidateDirection::Up,
                4.0,
                60,
            ),
        ];
        assert!(detect_convergence(&candidates, &ConvergenceConfig::related()).is_empty());
    }

    #[test]
    fn three_related_names_form_one_group_not_a_chain_of_pairs() {
        let candidates = vec![
            candidate(
                "src_a",
                "a::x::m::u",
                Some("region_hormuz"),
                CandidateDirection::Up,
                4.0,
                0,
            ),
            candidate(
                "src_b",
                "b::y::m::u",
                Some("region_strait_hormuz"),
                CandidateDirection::Up,
                4.0,
                10,
            ),
            candidate(
                "src_c",
                "c::z::m::u",
                Some("region_strait_of_hormuz"),
                CandidateDirection::Up,
                4.0,
                20,
            ),
        ];
        let groups = detect_convergence(&candidates, &ConvergenceConfig::related());
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].source_count(), 3);
        assert_eq!(groups[0].entity_ids.len(), 3);
    }

    #[test]
    fn entity_relations_are_symmetric_and_segment_based() {
        assert!(entities_related(
            &EntityId::new("region_san_francisco"),
            &EntityId::new("region_san_francisco_bay_area"),
            2
        ));
        assert!(entities_related(
            &EntityId::new("region_san_francisco_bay_area"),
            &EntityId::new("region_san_francisco"),
            2
        ));
        assert!(entities_related(
            &EntityId::new("topic_oil_price"),
            &EntityId::new("topic_oil_price_brent"),
            2
        ));
        assert!(!entities_related(
            &EntityId::new("region_fiji"),
            &EntityId::new("region_tonga"),
            2
        ));
        // Identical ids are related (equality is a subset of itself).
        assert!(entities_related(
            &EntityId::new("ent_hormuz"),
            &EntityId::new("ent_hormuz"),
            2
        ));
    }

    // --- Phase 11: geographic matching -------------------------------------

    #[test]
    fn nearby_candidates_without_entities_converge_on_place() {
        // Two providers, no shared entity, but the same coordinates.
        let candidates = vec![
            located(
                candidate(
                    "src_a",
                    "a::quake::count::n",
                    None,
                    CandidateDirection::Up,
                    4.0,
                    0,
                ),
                35.6,
                139.7,
            ),
            located(
                candidate(
                    "src_b",
                    "b::news::mentions::n",
                    None,
                    CandidateDirection::Up,
                    4.0,
                    60,
                ),
                35.65,
                139.75,
            ),
        ];

        // Exact mode ignores geography entirely.
        assert!(detect_convergence(&candidates, &ConvergenceConfig::default()).is_empty());

        let groups = detect_convergence(&candidates, &ConvergenceConfig::related());
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].match_kinds, vec![MatchKind::Geography]);
        assert_eq!(groups[0].source_count(), 2);
    }

    #[test]
    fn distant_candidates_do_not_converge_on_place() {
        let candidates = vec![
            located(
                candidate(
                    "src_a",
                    "a::quake::count::n",
                    None,
                    CandidateDirection::Up,
                    4.0,
                    0,
                ),
                35.6,
                139.7,
            ),
            located(
                candidate(
                    "src_b",
                    "b::news::mentions::n",
                    None,
                    CandidateDirection::Up,
                    4.0,
                    60,
                ),
                41.0,
                29.0,
            ),
        ];
        assert!(detect_convergence(&candidates, &ConvergenceConfig::related()).is_empty());
    }

    #[test]
    fn the_radius_is_respected() {
        // ~110 km apart: inside a 250 km radius, outside a 50 km one.
        let candidates = vec![
            located(
                candidate(
                    "src_a",
                    "a::quake::count::n",
                    None,
                    CandidateDirection::Up,
                    4.0,
                    0,
                ),
                35.6,
                139.7,
            ),
            located(
                candidate(
                    "src_b",
                    "b::news::mentions::n",
                    None,
                    CandidateDirection::Up,
                    4.0,
                    60,
                ),
                36.6,
                139.7,
            ),
        ];
        assert_eq!(
            detect_convergence(
                &candidates,
                &ConvergenceConfig::related().with_radius_km(250.0)
            )
            .len(),
            1
        );
        assert!(detect_convergence(
            &candidates,
            &ConvergenceConfig::related().with_radius_km(50.0)
        )
        .is_empty());
    }

    #[test]
    fn geography_needs_two_distinct_series() {
        // Same place, same source and series: that is one signal, not
        // convergence.
        let candidates = vec![
            located(
                candidate(
                    "src_a",
                    "a::quake::count::n",
                    None,
                    CandidateDirection::Up,
                    4.0,
                    0,
                ),
                35.6,
                139.7,
            ),
            located(
                candidate(
                    "src_a",
                    "a::quake::count::n",
                    None,
                    CandidateDirection::Up,
                    4.0,
                    60,
                ),
                35.65,
                139.75,
            ),
        ];
        assert!(detect_convergence(&candidates, &ConvergenceConfig::related()).is_empty());
    }

    #[test]
    fn candidates_without_coordinates_are_skipped_by_geography() {
        let candidates = vec![
            candidate(
                "src_a",
                "a::quake::count::n",
                None,
                CandidateDirection::Up,
                4.0,
                0,
            ),
            candidate(
                "src_b",
                "b::news::mentions::n",
                None,
                CandidateDirection::Up,
                4.0,
                60,
            ),
        ];
        assert!(detect_convergence(&candidates, &ConvergenceConfig::related()).is_empty());
    }

    #[test]
    fn ordering_is_deterministic_across_shuffles() {
        // Equal-strength groups used to come out in hash order, which varies
        // per run and would break replay.
        let mut candidates = Vec::new();
        for i in 0..8 {
            candidates.push(candidate(
                "src_a",
                &format!("a::m{i}::x::u"),
                Some(&format!("ent_{i}")),
                CandidateDirection::Up,
                4.0,
                0,
            ));
            candidates.push(candidate(
                "src_b",
                &format!("b::m{i}::y::u"),
                Some(&format!("ent_{i}")),
                CandidateDirection::Up,
                4.0,
                10,
            ));
        }

        let forward = detect_convergence(&candidates, &ConvergenceConfig::related());
        candidates.reverse();
        let backward = detect_convergence(&candidates, &ConvergenceConfig::related());

        let keys_forward: Vec<&String> = forward.iter().map(|g| &g.group_key).collect();
        let keys_backward: Vec<&String> = backward.iter().map(|g| &g.group_key).collect();
        assert_eq!(keys_forward, keys_backward);
        assert_eq!(forward.len(), 8);
    }
}
