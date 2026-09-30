//! Event formation.
//!
//! Anomaly candidates are grouped into events. The rule is deliberately
//! deterministic and explainable:
//!
//! ```text
//! same entity (or series)  +  same direction  +  within the time window
//!     -> one event
//! ```
//!
//! Events persist across collection cycles: a new candidate that fits an
//! active event extends it rather than creating a second one. That is what
//! gives an event its `first_seen`/`last_seen` span and its lifecycle.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use wse_model::{AnomalyCandidate, CandidateDirection, Event, EventState, ObservationId};

/// Tunables for event formation and lifecycle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventConfig {
    /// Maximum gap between a candidate and an event's last update for the
    /// candidate to join that event.
    pub window_seconds: i64,
    /// Candidates required before an event is created.
    pub min_candidates: usize,
    /// Idle time after which an event is resolved.
    pub resolve_after_seconds: i64,
}

impl Default for EventConfig {
    fn default() -> Self {
        Self {
            window_seconds: 6 * 3600,
            min_candidates: 1,
            resolve_after_seconds: 24 * 3600,
        }
    }
}

/// Groups candidates into events and maintains the lifecycle.
#[derive(Debug, Clone)]
pub struct EventEngine {
    config: EventConfig,
    /// Events still open to new candidates.
    active: Vec<Event>,
    /// Most recent candidate per active event, used for direction matching.
    directions: HashMap<String, CandidateDirection>,
}

impl EventEngine {
    pub fn new(config: EventConfig) -> Self {
        Self {
            config,
            active: Vec::new(),
            directions: HashMap::new(),
        }
    }

    pub fn config(&self) -> &EventConfig {
        &self.config
    }

    /// Events currently open.
    pub fn active_events(&self) -> &[Event] {
        &self.active
    }

    /// Re-admit an event that was open when the process last stopped.
    ///
    /// Without this, a restart would leave every in-flight change orphaned: the
    /// next candidate would not find its event, so it would start a second one
    /// with a fresh id and the original signal would stop accumulating. The
    /// event's own `group_key` and `last_seen` are what the matcher needs, so
    /// they are restored as-is rather than recomputed.
    pub fn adopt(&mut self, event: Event) {
        if event.state == EventState::Resolved {
            return;
        }
        if self.active.iter().any(|e| e.id == event.id) {
            return;
        }
        self.directions
            .insert(event.id.as_str().to_string(), event.direction);
        self.active.push(event);
    }

    /// Ingest a batch of candidates and return every event that was created or
    /// updated by this batch.
    ///
    /// `category_of` maps a source to its catalog category, so events can carry
    /// categories for filtering and lenses.
    pub fn ingest(
        &mut self,
        candidates: &[AnomalyCandidate],
        category_of: &dyn Fn(&str) -> Option<String>,
    ) -> Vec<Event> {
        let mut touched: Vec<String> = Vec::new();
        let mut sorted: Vec<&AnomalyCandidate> = candidates.iter().collect();
        sorted.sort_by_key(|c| c.observed_at);

        for candidate in sorted {
            let group_key = group_key(candidate);
            match self.find_slot(&group_key, candidate) {
                Some(idx) => {
                    let event = &mut self.active[idx];
                    event.observe(candidate.observed_at);
                    if !event.observations.contains(&candidate.observation_id) {
                        event.observations.push(candidate.observation_id.clone());
                    }
                    if !event.anomalies.contains(&candidate.id) {
                        event.anomalies.push(candidate.id.clone());
                    }
                    if let Some(entity) = &candidate.entity_id {
                        if !event.entities.contains(entity) {
                            event.entities.push(entity.clone());
                        }
                    }
                    if let Some(category) = category_of(candidate.source_id.as_str()) {
                        if !event.categories.contains(&category) {
                            event.categories.push(category);
                        }
                    }
                    if event.location.is_none() {
                        event.location = location_of(candidate);
                    }
                    event.source_count = distinct_sources(event, candidates);
                    if event.observation_count() > 1 {
                        event.state = EventState::Changing;
                    }
                    touched.push(event.id.as_str().to_string());
                }
                None => {
                    let mut event = Event::new_for(
                        group_key.clone(),
                        title_for(candidate),
                        candidate.observed_at,
                    );
                    event.observations.push(candidate.observation_id.clone());
                    event.anomalies.push(candidate.id.clone());
                    if let Some(entity) = &candidate.entity_id {
                        event.entities.push(entity.clone());
                    }
                    if let Some(category) = category_of(candidate.source_id.as_str()) {
                        event.categories.push(category);
                    }
                    event.location = location_of(candidate);
                    event.direction = candidate.direction;
                    event.source_count = 1;
                    event.state = EventState::Active;
                    self.directions.insert(group_key, candidate.direction);
                    touched.push(event.id.as_str().to_string());
                    self.active.push(event);
                }
            }
        }

        let touched_set: std::collections::HashSet<&str> =
            touched.iter().map(|s| s.as_str()).collect();
        self.active
            .iter()
            .filter(|e| touched_set.contains(e.id.as_str()))
            .cloned()
            .collect()
    }

    /// Find an active event this candidate belongs to.
    fn find_slot(&self, group_key: &str, candidate: &AnomalyCandidate) -> Option<usize> {
        self.active.iter().position(|e| {
            if e.state == EventState::Resolved {
                return false;
            }
            let same_group = event_group_key(e) == group_key;
            let gap = (candidate.observed_at - e.last_seen).num_seconds().abs();
            let within_window = gap <= self.config.window_seconds;
            let same_direction = e.direction == candidate.direction;
            same_group && within_window && same_direction
        })
    }

    /// Advance lifecycle for all active events against a clock.
    ///
    /// Events that have been idle past `resolve_after_seconds` are resolved.
    /// Returns every event whose state changed.
    pub fn refresh(&mut self, now: DateTime<Utc>) -> Vec<Event> {
        let mut changed = Vec::new();
        for event in &mut self.active {
            let before = event.state;
            event.refresh_state(now, self.config.resolve_after_seconds);
            if event.state != before {
                changed.push(event.clone());
            }
        }
        self.active.retain(|e| e.state != EventState::Resolved);
        changed
    }

    /// Drop resolved events from the active set (they live on in storage).
    pub fn clear_resolved(&mut self) {
        self.active.retain(|e| e.state != EventState::Resolved);
    }
}

/// The grouping key for a candidate: its entity, or its series if entity-less.
pub fn group_key(candidate: &AnomalyCandidate) -> String {
    candidate
        .entity_id
        .as_ref()
        .map(|e| format!("entity:{}", e.as_str()))
        .unwrap_or_else(|| format!("series:{}", candidate.series_key))
}

/// The grouping key for an event.
///
/// This is the key the event was formed under, kept on the event itself. It
/// used to be re-derived from `entities.first()` and then `anomalies.first()`,
/// which meant an event with entities grouped under `entity:…` while one
/// without grouped under `anomaly:…` — two different keys for the same
/// ongoing change, so it never merged and a new event was started each cycle.
pub fn event_group_key(event: &Event) -> String {
    if !event.group_key.is_empty() {
        return event.group_key.clone();
    }
    // Fall back for records built by `Event::new` (tests, hand-built data).
    match event.entities.first() {
        Some(entity) => format!("entity:{}", entity.as_str()),
        None => event
            .anomalies
            .first()
            .map(|a| format!("anomaly:{}", a.as_str()))
            .unwrap_or_else(|| format!("event:{}", event.id.as_str())),
    }
}

fn title_for(candidate: &AnomalyCandidate) -> String {
    let metric = if candidate.metric.is_empty() {
        candidate.series_key.as_str()
    } else {
        candidate.metric.as_str()
    };
    let direction = match candidate.direction {
        CandidateDirection::Up => "rising",
        CandidateDirection::Down => "falling",
        CandidateDirection::Flat => "unusual",
    };
    format!("{metric} {direction}")
}

fn location_of(candidate: &AnomalyCandidate) -> Option<wse_model::Location> {
    match (candidate.latitude, candidate.longitude) {
        (Some(lat), Some(lon)) => Some(wse_model::Location::new(lat, lon)),
        _ => None,
    }
}

/// Count distinct sources contributing to an event.
///
/// Event observations may come from several sources; the count is derived from
/// the candidates that referenced them.
fn distinct_sources(event: &Event, candidates: &[AnomalyCandidate]) -> usize {
    let mut sources: Vec<&str> = candidates
        .iter()
        .filter(|c| event.anomalies.contains(&c.id))
        .map(|c| c.source_id.as_str())
        .collect();
    sources.sort_unstable();
    sources.dedup();
    sources.len().max(1)
}

/// Helper for tests and callers that only have observation ids.
pub fn observation_ids(event: &Event) -> Vec<ObservationId> {
    event.observations.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wse_model::{
        AnomalyCandidate, BaselineSnapshot, CandidateKind, DetectionMethod, EntityId,
        ObservationId, SourceId,
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
        secs: i64,
    ) -> AnomalyCandidate {
        let mut c = AnomalyCandidate::new(
            series,
            ObservationId::new(format!("obs_{series}_{secs}")),
            at(secs),
            snap(),
            120.0,
        );
        c.source_id = SourceId::new(source);
        c.entity_id = entity.map(EntityId::new);
        c.metric = series.split("::").nth(2).unwrap_or("metric").to_string();
        c.direction = direction;
        c.score = 4.0;
        c.kind = CandidateKind::Anomaly;
        c.method = DetectionMethod::RobustZScore;
        c
    }

    fn no_category(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn candidates_on_same_entity_and_direction_form_one_event() {
        let mut engine = EventEngine::new(EventConfig::default());
        let candidates = vec![
            candidate(
                "src_a",
                "a::oil::price::usd",
                Some("ent_hormuz"),
                CandidateDirection::Up,
                0,
            ),
            candidate(
                "src_b",
                "b::oil::volume::bbl",
                Some("ent_hormuz"),
                CandidateDirection::Up,
                60,
            ),
        ];
        let events = engine.ingest(&candidates, &no_category);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].observation_count(), 2);
        assert_eq!(events[0].anomalies.len(), 2);
    }

    #[test]
    fn opposite_directions_form_separate_events() {
        let mut engine = EventEngine::new(EventConfig::default());
        let candidates = vec![
            candidate(
                "src_a",
                "a::oil::price::usd",
                Some("ent_hormuz"),
                CandidateDirection::Up,
                0,
            ),
            candidate(
                "src_b",
                "b::oil::price::usd",
                Some("ent_hormuz"),
                CandidateDirection::Down,
                60,
            ),
        ];
        let events = engine.ingest(&candidates, &no_category);
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn candidates_outside_the_window_form_separate_events() {
        let mut engine = EventEngine::new(EventConfig::default());
        let candidates = vec![
            candidate(
                "src_a",
                "a::oil::price::usd",
                Some("ent_hormuz"),
                CandidateDirection::Up,
                0,
            ),
            candidate(
                "src_b",
                "b::oil::volume::bbl",
                Some("ent_hormuz"),
                CandidateDirection::Up,
                100_000,
            ),
        ];
        let events = engine.ingest(&candidates, &no_category);
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn repeated_ingest_extends_an_existing_event() {
        let mut engine = EventEngine::new(EventConfig::default());
        let first = vec![candidate(
            "src_a",
            "a::oil::price::usd",
            Some("ent_hormuz"),
            CandidateDirection::Up,
            0,
        )];
        engine.ingest(&first, &no_category);
        let second = vec![candidate(
            "src_b",
            "b::oil::volume::bbl",
            Some("ent_hormuz"),
            CandidateDirection::Up,
            120,
        )];
        let events = engine.ingest(&second, &no_category);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].observation_count(), 2);
        assert_eq!(events[0].duration_seconds(), 120);
    }

    #[test]
    fn category_is_attached_from_the_source_lookup() {
        let mut engine = EventEngine::new(EventConfig::default());
        let candidates = vec![candidate(
            "src_a",
            "a::oil::price::usd",
            Some("ent_hormuz"),
            CandidateDirection::Up,
            0,
        )];
        let events = engine.ingest(&candidates, &|s| {
            (s == "src_a").then(|| "energy".to_string())
        });
        assert_eq!(events[0].categories, vec!["energy"]);
    }

    #[test]
    fn idle_events_are_resolved_and_removed() {
        let mut engine = EventEngine::new(EventConfig {
            resolve_after_seconds: 60,
            ..EventConfig::default()
        });
        let candidates = vec![candidate(
            "src_a",
            "a::oil::price::usd",
            Some("ent_hormuz"),
            CandidateDirection::Up,
            0,
        )];
        engine.ingest(&candidates, &no_category);
        assert_eq!(engine.active_events().len(), 1);
        let changed = engine.refresh(at(10_000));
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].state, EventState::Resolved);
        assert!(engine.active_events().is_empty());
    }

    #[test]
    fn observation_ids_helper_returns_the_evidence_trail() {
        let mut engine = EventEngine::new(EventConfig::default());
        let candidates = vec![candidate(
            "src_a",
            "a::oil::price::usd",
            Some("ent_hormuz"),
            CandidateDirection::Up,
            0,
        )];
        let events = engine.ingest(&candidates, &no_category);
        assert_eq!(observation_ids(&events[0]).len(), 1);
    }
}
