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
//! The MVP implementation is deterministic: entity, geography, time window,
//! category and direction. Graph and ML methods can be layered on later.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use wse_model::{AnomalyId, CandidateDirection, EntityId, SourceId};

/// Tunables for convergence detection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConvergenceConfig {
    /// Candidates further apart than this cannot belong to one group.
    pub window_seconds: i64,
    /// Minimum distinct sources that must agree.
    pub min_sources: usize,
    /// Minimum distinct series (metrics) that must agree.
    pub min_series: usize,
}

impl Default for ConvergenceConfig {
    fn default() -> Self {
        Self {
            window_seconds: 6 * 3600,
            min_sources: 2,
            min_series: 2,
        }
    }
}

/// A set of independent candidates that describe the same change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConvergenceGroup {
    /// Entity the group is about, when the candidates share one.
    pub entity_id: Option<EntityId>,
    /// Key used for grouping when there is no entity (the series key).
    pub group_key: String,
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
}

/// Detect convergence among a batch of candidates.
///
/// The input is the same candidate set the event engine sees; convergence is a
/// property *across* candidates, so it is computed before signals are formed.
pub fn detect_convergence(
    candidates: &[wse_model::AnomalyCandidate],
    config: &ConvergenceConfig,
) -> Vec<ConvergenceGroup> {
    // Group by entity when present; otherwise by series key. Grouping by
    // series key for entity-less candidates prevents unrelated metrics from
    // being declared "convergent" just because they are numerically similar.
    let mut buckets: HashMap<String, Vec<&wse_model::AnomalyCandidate>> = HashMap::new();
    for c in candidates {
        let key = c
            .entity_id
            .as_ref()
            .map(|e| e.as_str().to_string())
            .unwrap_or_else(|| c.series_key.clone());
        buckets.entry(key).or_default().push(c);
    }

    let mut groups = Vec::new();
    for (key, mut items) in buckets {
        items.sort_by_key(|c| c.observed_at);

        // Sliding window over time. A window is closed once the next candidate
        // is more than `window_seconds` past the window's first candidate.
        let mut start = 0;
        while start < items.len() {
            let window_start = items[start].observed_at;
            let mut end = start;
            while end < items.len()
                && (items[end].observed_at - window_start).num_seconds() <= config.window_seconds
            {
                end += 1;
            }
            if let Some(group) = build_group(&key, &items[start..end], config) {
                groups.push(group);
            }
            start = end;
        }
    }

    groups.sort_by(|a, b| {
        b.strength
            .partial_cmp(&a.strength)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.first_seen.cmp(&b.first_seen))
    });
    groups
}

fn build_group(
    key: &str,
    window: &[&wse_model::AnomalyCandidate],
    config: &ConvergenceConfig,
) -> Option<ConvergenceGroup> {
    let mut sources: Vec<SourceId> = Vec::new();
    let mut series: Vec<String> = Vec::new();
    let mut ids = Vec::new();
    for c in window {
        if !sources.contains(&c.source_id) {
            sources.push(c.source_id.clone());
        }
        if !series.contains(&c.series_key) {
            series.push(c.series_key.clone());
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
        entity_id: window.first().and_then(|c| c.entity_id.clone()),
        group_key: key.to_string(),
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
}
