//! Signal generation.
//!
//! A signal is an event promoted to human attention. This module decides which
//! events deserve that, attaches evidence that traces back to observations, and
//! records *why* the signal exists so the UI can explain it instead of saying
//! "the AI found this important".
//!
//! The five types from the brief are produced here:
//!
//! | Type | Produced when |
//! |---|---|
//! | `NOW` | a candidate was observed within the recency window |
//! | `ANOMALY` | any candidate is a full deviation |
//! | `EARLY_SIGNAL` | any candidate is a persistence drift |
//! | `CONVERGENCE` | independent sources point at the same change |
//! | `IMPACT` | the change touches a configured lens/entity |

pub mod event;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use wse_correlation::{ConvergenceConfig, ConvergenceGroup};
use wse_model::lens::Lens;
use wse_model::{
    CandidateDirection, CandidateKind, Event, Evidence, Signal, SignalQuality, SignalType,
};

use crate::event::EventEngine;

/// Tunables for signal promotion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignalConfig {
    /// A candidate observed within this window makes a signal `NOW`.
    pub now_window_seconds: i64,
    /// An event needs at least this many observations to be promoted.
    pub min_observations: usize,
    /// Minimum confidence for promotion.
    pub min_confidence: f64,
    /// Categories that count as "impactful" (e.g. energy, finance).
    pub impact_categories: Vec<String>,
    /// Entities that count as "impactful" (e.g. a strait, a port).
    pub impact_entities: Vec<String>,
}

impl Default for SignalConfig {
    fn default() -> Self {
        Self {
            now_window_seconds: 30 * 60,
            min_observations: 1,
            min_confidence: 0.0,
            impact_categories: Vec::new(),
            impact_entities: Vec::new(),
        }
    }
}

/// The signal engine.
#[derive(Debug, Clone)]
pub struct SignalEngine {
    config: SignalConfig,
    convergence: ConvergenceConfig,
    /// The lenses signals are matched against. Empty by default: a lens is a
    /// view, so with none configured every signal simply has no lens matches.
    lenses: Vec<Lens>,
}

impl SignalEngine {
    pub fn new(config: SignalConfig) -> Self {
        Self {
            config,
            convergence: ConvergenceConfig::default(),
            lenses: Vec::new(),
        }
    }

    pub fn with_convergence(mut self, convergence: ConvergenceConfig) -> Self {
        self.convergence = convergence;
        self
    }

    /// The lenses every formed signal is matched against.
    pub fn with_lenses(mut self, lenses: Vec<Lens>) -> Self {
        self.lenses = lenses;
        self
    }

    pub fn lenses(&self) -> &[Lens] {
        &self.lenses
    }

    pub fn config(&self) -> &SignalConfig {
        &self.config
    }

    /// Turn events (plus their supporting candidates) into signals.
    ///
    /// Convergence is computed here from `self.convergence` rather than passed
    /// in, so the configuration that decides how sources are matched and the
    /// code that forms the signals cannot drift apart.
    ///
    /// Returns one signal per event that meets the promotion rules.
    pub fn form_signals(
        &self,
        events: &[Event],
        candidates: &[wse_model::AnomalyCandidate],
        now: DateTime<Utc>,
    ) -> Vec<Signal> {
        let convergence = wse_correlation::detect_convergence(candidates, &self.convergence);
        let convergence = convergence.as_slice();
        let mut signals = Vec::new();
        for event in events {
            if event.observation_count() < self.config.min_observations {
                continue;
            }
            let event_candidates: Vec<&wse_model::AnomalyCandidate> = candidates
                .iter()
                .filter(|c| event.anomalies.contains(&c.id))
                .collect();
            if event_candidates.is_empty() {
                continue;
            }

            let mut signal = Signal::new(event.id.clone(), event.first_seen);
            signal.series_key = dominant_series(&event_candidates);
            signal.id = Signal::stable_id(&event.id, &signal.series_key, event.direction);
            signal.last_updated = event.last_seen;
            signal.duration_seconds = event.duration_seconds();
            signal.entities = event.entities.clone();
            signal.categories = event.categories.clone();
            signal.location = event.location.clone();
            signal.direction = event.direction;
            signal.evidence = evidence_for(&event_candidates);
            signal.confidence = mean_confidence(&event_candidates);

            self.assign_types(&mut signal, &event_candidates, convergence, now);
            if signal.types.is_empty() {
                continue;
            }
            if signal.confidence < self.config.min_confidence {
                continue;
            }

            signal.title = title_for(&signal, event);
            signal.summary = summarize(&signal);
            signal.reasons = reasons_for(&signal, &event_candidates);
            self.assign_lens_matches(&mut signal);
            signal.quality = quality_for(&signal, &event_candidates);
            signals.push(signal);
        }
        signals
    }

    fn assign_types(
        &self,
        signal: &mut Signal,
        candidates: &[&wse_model::AnomalyCandidate],
        convergence: &[ConvergenceGroup],
        now: DateTime<Utc>,
    ) {
        if candidates
            .iter()
            .any(|c| (now - c.observed_at).num_seconds() <= self.config.now_window_seconds)
        {
            signal.add_type(SignalType::Now);
        }
        if candidates.iter().any(|c| c.kind == CandidateKind::Anomaly) {
            signal.add_type(SignalType::Anomaly);
        }
        if candidates
            .iter()
            .any(|c| c.kind == CandidateKind::EarlySignal)
        {
            signal.add_type(SignalType::EarlySignal);
        }
        if convergence.iter().any(|g| {
            g.candidate_ids
                .iter()
                .any(|id| candidates.iter().any(|c| &c.id == id))
        }) {
            signal.add_type(SignalType::Convergence);
        }
        if self.has_impact(signal) {
            signal.add_type(SignalType::Impact);
        }
    }

    /// Record which lenses currently show this signal.
    ///
    /// This is what makes `GET /signals?lens=` able to return anything: the
    /// filter reads `lens_matches`, so leaving it empty would make every lens
    /// query return nothing. It is deliberately *not* part of the signal's
    /// identity — a lens is a view, and changing one must not rewrite history.
    fn assign_lens_matches(&self, signal: &mut Signal) {
        if self.lenses.is_empty() {
            return;
        }
        let text = format!("{} {}", signal.title, signal.summary);
        let location = signal.location.as_ref().map(|l| (l.latitude, l.longitude));
        let entities: Vec<String> = signal
            .entities
            .iter()
            .map(|e| e.as_str().to_string())
            .collect();
        signal.lens_matches = self
            .lenses
            .iter()
            .filter(|lens| lens.matches(&signal.categories, &entities, &text, location))
            .map(|lens| lens.id.clone())
            .collect();
    }

    fn has_impact(&self, signal: &Signal) -> bool {
        let category_hit = signal.categories.iter().any(|c| {
            self.config
                .impact_categories
                .iter()
                .any(|want| want.eq_ignore_ascii_case(c))
        });
        let entity_hit = signal.entities.iter().any(|e| {
            let segments = entity_segments(e.as_str());
            self.config.impact_entities.iter().any(|want| {
                let wanted = wse_model::canonicalize(want);
                !wanted.is_empty() && segments.contains(&wanted)
            })
        });
        category_hit || entity_hit
    }
}

/// Split an entity id into canonical, comparable segments.
///
/// Entity ids are slug-like (`ent_route_hormuz`), so matching a configured
/// impact term against whole segments avoids the substring false positives that
/// a naive `contains` would produce.
fn entity_segments(entity_id: &str) -> Vec<String> {
    entity_id
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(wse_model::canonicalize)
        .collect()
}

/// The series that best represents the event: the most recently observed one.
///
/// A signal is keyed on a single series so its identity is stable across
/// cycles even when the underlying event accumulates candidates from several.
fn dominant_series(candidates: &[&wse_model::AnomalyCandidate]) -> String {
    candidates
        .iter()
        .max_by_key(|c| c.observed_at)
        .map(|c| c.series_key.clone())
        .unwrap_or_default()
}

fn evidence_for(candidates: &[&wse_model::AnomalyCandidate]) -> Vec<Evidence> {
    let mut evidence: Vec<Evidence> = candidates
        .iter()
        .map(|c| Evidence {
            source_id: c.source_id.clone(),
            observation_id: c.observation_id.clone(),
            metric: if c.metric.is_empty() {
                c.series_key.clone()
            } else {
                c.metric.clone()
            },
            unit: c.unit.clone(),
            statement: statement_for(c),
            observed_at: c.observed_at,
            value: c.current,
            deviation_sigma: Some(c.robust_z()),
        })
        .collect();
    evidence.sort_by_key(|e| e.observed_at);
    evidence
}

/// A human-readable, quantitative statement about one candidate.
///
/// The wording is deliberately specific — "temperature +4.1σ over 18 min" —
/// rather than qualitative, so a signal never claims importance it cannot
/// justify.
fn statement_for(c: &wse_model::AnomalyCandidate) -> String {
    let metric = if c.metric.is_empty() {
        c.series_key.as_str()
    } else {
        c.metric.as_str()
    };
    let unit = if c.unit.is_empty() {
        String::new()
    } else {
        format!(" {}", c.unit)
    };
    match c.kind {
        CandidateKind::Anomaly => format!(
            "{metric} {value:.2}{unit} ({sigma:+.1}σ vs baseline, {method:?})",
            value = c.current,
            sigma = c.score,
            method = c.method
        ),
        CandidateKind::EarlySignal => format!(
            "{metric} drifting {dir} ({sigma:+.1}σ, {minutes} min, persistent)",
            dir = direction_word(c.direction),
            sigma = c.score,
            minutes = c.duration_seconds / 60
        ),
    }
}

fn direction_word(direction: CandidateDirection) -> &'static str {
    match direction {
        CandidateDirection::Up => "up",
        CandidateDirection::Down => "down",
        CandidateDirection::Flat => "sideways",
    }
}

fn mean_confidence(candidates: &[&wse_model::AnomalyCandidate]) -> f64 {
    if candidates.is_empty() {
        return 0.0;
    }
    candidates.iter().map(|c| c.confidence).sum::<f64>() / candidates.len() as f64
}

fn title_for(signal: &Signal, event: &Event) -> String {
    let kind = if signal.has_type(SignalType::EarlySignal) && !signal.has_type(SignalType::Anomaly)
    {
        "Early signal"
    } else if signal.has_type(SignalType::Convergence) {
        "Convergence"
    } else if signal.has_type(SignalType::Anomaly) {
        "Anomaly"
    } else {
        "Change"
    };
    format!("{kind}: {}", event.title)
}

/// Build the quantitative summary from the signal's own accumulated evidence.
///
/// Deriving it from `signal.evidence` (rather than from this cycle's
/// candidates) is what lets a merged signal keep reporting the full span it
/// covers: "35 observations from 2 sources, 3.5σ, sustained 2340 min".
pub fn summarize(signal: &Signal) -> String {
    let count = signal.evidence.len();
    let sources = signal.distinct_sources();
    let max_sigma = signal
        .evidence
        .iter()
        .filter_map(|e| e.deviation_sigma)
        .map(f64::abs)
        .fold(0.0_f64, f64::max);
    let duration_minutes = signal.duration_seconds / 60;
    format!(
        "{count} observation(s) from {sources} source(s); largest deviation {max_sigma:.1}σ; sustained {duration_minutes} min"
    )
}

/// Explicit, machine-checkable reasons the signal exists.
fn reasons_for(signal: &Signal, candidates: &[&wse_model::AnomalyCandidate]) -> Vec<String> {
    let mut reasons = Vec::new();
    if signal.has_type(SignalType::Anomaly) {
        let best = candidates
            .iter()
            .filter(|c| c.kind == CandidateKind::Anomaly)
            .max_by(|a, b| a.score.abs().partial_cmp(&b.score.abs()).unwrap());
        if let Some(best) = best {
            reasons.push(format!(
                "deviation {sigma:+.1}σ from baseline (median {median:.2}, MAD {mad:.2}) via {method:?}",
                sigma = best.score,
                median = best.baseline.median,
                mad = best.baseline.mad,
                method = best.method
            ));
        }
    }
    if signal.has_type(SignalType::EarlySignal) {
        let drift = candidates
            .iter()
            .find(|c| c.kind == CandidateKind::EarlySignal);
        if let Some(drift) = drift {
            reasons.push(format!(
                "persistent directional drift for {} min, {sigma:+.1}σ and still growing",
                drift.duration_seconds / 60,
                sigma = drift.score
            ));
        }
    }
    if signal.has_type(SignalType::Convergence) {
        reasons.push(format!(
            "{} independent sources point at the same change",
            signal.distinct_sources()
        ));
    }
    if signal.has_type(SignalType::Now) {
        reasons.push("change observed within the current window".to_string());
    }
    if signal.has_type(SignalType::Impact) {
        reasons.push(format!(
            "touches configured impact scope ({} categories, {} entities)",
            signal.categories.len(),
            signal.entities.len()
        ));
    }
    reasons
}

/// Multi-dimensional quality. Never a single opaque "importance" number.
fn quality_for(signal: &Signal, candidates: &[&wse_model::AnomalyCandidate]) -> SignalQuality {
    let max_sigma = candidates
        .iter()
        .map(|c| c.score.abs())
        .fold(0.0_f64, f64::max);
    let strength = (max_sigma / 6.0).clamp(0.0, 1.0);
    let persistence = (signal.duration_seconds as f64 / (6.0 * 3600.0)).clamp(0.0, 1.0);
    let novelty = if signal.has_type(SignalType::Now) {
        1.0
    } else {
        0.4
    };
    let convergence = (signal.distinct_sources() as f64 / 4.0).clamp(0.0, 1.0);
    let breadth = (signal.categories.len() as f64 / 3.0).clamp(0.0, 1.0);
    let relevance = if signal.has_type(SignalType::Impact) {
        1.0
    } else {
        0.5
    };
    SignalQuality {
        novelty,
        strength,
        persistence,
        confidence: signal.confidence,
        breadth,
        convergence,
        relevance,
    }
}

/// Convenience: run the event and signal engines together over one batch.
///
/// This is the vertical slice the MVP acceptance test exercises.
pub fn process_batch(
    event_engine: &mut EventEngine,
    signal_engine: &SignalEngine,
    candidates: &[wse_model::AnomalyCandidate],
    category_of: &dyn Fn(&str) -> Option<String>,
    now: DateTime<Utc>,
) -> Vec<Signal> {
    let events = event_engine.ingest(candidates, category_of);
    if events.is_empty() {
        return Vec::new();
    }
    signal_engine.form_signals(&events, candidates, now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wse_model::{
        AnomalyCandidate, BaselineSnapshot, DetectionMethod, EntityId, EventId, LensId,
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

    #[allow(clippy::too_many_arguments)]
    fn candidate(
        source: &str,
        series: &str,
        entity: Option<&str>,
        kind: CandidateKind,
        direction: CandidateDirection,
        score: f64,
        confidence: f64,
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
        c.metric = series.split("::").nth(2).unwrap_or("metric").to_string();
        c.unit = "unit".to_string();
        c.kind = kind;
        c.direction = direction;
        c.score = score;
        c.confidence = confidence;
        c.method = match kind {
            CandidateKind::Anomaly => DetectionMethod::RobustZScore,
            CandidateKind::EarlySignal => DetectionMethod::PersistenceDrift,
        };
        c.duration_seconds = if kind == CandidateKind::EarlySignal {
            1800
        } else {
            0
        };
        c
    }

    fn engine() -> SignalEngine {
        SignalEngine::new(SignalConfig::default())
    }

    fn no_category(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn anomaly_candidate_produces_an_anomaly_signal_with_evidence() {
        let mut events = EventEngine::new(Default::default());
        let candidates = vec![candidate(
            "src_a",
            "a::oil::price::usd",
            Some("ent_hormuz"),
            CandidateKind::Anomaly,
            CandidateDirection::Up,
            4.1,
            0.9,
            0,
        )];
        let signals = process_batch(&mut events, &engine(), &candidates, &no_category, at(30));
        assert_eq!(signals.len(), 1);
        let s = &signals[0];
        assert!(s.has_type(SignalType::Anomaly));
        assert!(s.has_type(SignalType::Now));
        assert_eq!(s.evidence.len(), 1);
        assert!(s.evidence[0].statement.contains("4.1σ"));
        assert!(!s.reasons.is_empty());
        assert!(s.title.starts_with("Anomaly"));
    }

    #[test]
    fn early_signal_candidate_produces_an_early_signal() {
        let mut events = EventEngine::new(Default::default());
        let candidates = vec![candidate(
            "src_a",
            "a::sensor::temp::c",
            Some("ent_sensor"),
            CandidateKind::EarlySignal,
            CandidateDirection::Up,
            1.6,
            0.6,
            0,
        )];
        let signals = process_batch(&mut events, &engine(), &candidates, &no_category, at(30));
        assert_eq!(signals.len(), 1);
        assert!(signals[0].has_type(SignalType::EarlySignal));
        assert!(!signals[0].has_type(SignalType::Anomaly));
        assert!(signals[0].title.starts_with("Early signal"));
    }

    #[test]
    fn independent_sources_produce_a_convergence_signal() {
        let mut events = EventEngine::new(Default::default());
        let candidates = vec![
            candidate(
                "src_a",
                "a::oil::price::usd",
                Some("ent_hormuz"),
                CandidateKind::Anomaly,
                CandidateDirection::Up,
                4.0,
                0.9,
                0,
            ),
            candidate(
                "src_b",
                "b::shipping::delay::h",
                Some("ent_hormuz"),
                CandidateKind::Anomaly,
                CandidateDirection::Up,
                3.5,
                0.9,
                30,
            ),
            candidate(
                "src_c",
                "c::news::mentions::n",
                Some("ent_hormuz"),
                CandidateKind::Anomaly,
                CandidateDirection::Up,
                3.0,
                0.9,
                60,
            ),
        ];
        let signals = process_batch(&mut events, &engine(), &candidates, &no_category, at(90));
        assert_eq!(signals.len(), 1);
        let s = &signals[0];
        assert!(s.has_type(SignalType::Convergence));
        assert_eq!(s.distinct_sources(), 3);
        assert!(s.reasons.iter().any(|r| r.contains("independent sources")));
    }

    #[test]
    fn impact_type_requires_configured_scope() {
        let engine = SignalEngine::new(SignalConfig {
            impact_entities: vec!["Hormuz".to_string()],
            ..SignalConfig::default()
        });
        let mut events = EventEngine::new(Default::default());
        let candidates = vec![candidate(
            "src_a",
            "a::oil::price::usd",
            Some("ent_hormuz"),
            CandidateKind::Anomaly,
            CandidateDirection::Up,
            4.0,
            0.9,
            0,
        )];
        let signals = process_batch(
            &mut events,
            &engine,
            &candidates,
            &|_| Some("energy".into()),
            at(30),
        );
        assert!(signals[0].has_type(SignalType::Impact));
    }

    #[test]
    fn a_quiet_event_produces_no_signal() {
        // No candidates at all -> no events -> no signals.
        let mut events = EventEngine::new(Default::default());
        let signals = process_batch(&mut events, &engine(), &[], &no_category, at(0));
        assert!(signals.is_empty());
    }

    #[test]
    fn quality_dimensions_are_reported_separately() {
        let mut events = EventEngine::new(Default::default());
        let candidates = vec![candidate(
            "src_a",
            "a::oil::price::usd",
            Some("ent_hormuz"),
            CandidateKind::Anomaly,
            CandidateDirection::Up,
            6.0,
            1.0,
            0,
        )];
        let signals = process_batch(&mut events, &engine(), &candidates, &no_category, at(30));
        let q = &signals[0].quality;
        assert!(q.strength > 0.9);
        assert!(q.confidence > 0.9);
        assert!(q.novelty > 0.0);
        assert!(q.convergence > 0.0);
        // The summary is quantitative, not a verdict.
        assert!(signals[0].summary.contains("σ"));
    }

    #[test]
    fn signal_links_back_to_its_event() {
        let mut events = EventEngine::new(Default::default());
        let candidates = vec![candidate(
            "src_a",
            "a::oil::price::usd",
            Some("ent_hormuz"),
            CandidateKind::Anomaly,
            CandidateDirection::Up,
            4.0,
            0.9,
            0,
        )];
        let signals = process_batch(&mut events, &engine(), &candidates, &no_category, at(30));
        assert_eq!(signals[0].event_id, events.active_events()[0].id);
        assert_ne!(signals[0].event_id, EventId::new("evt_other"));
    }

    /// A lens is matched against the signal, and the match is *recorded* on it.
    #[test]
    fn a_lens_that_covers_the_signal_is_recorded_on_it() {
        let mut events = EventEngine::new(Default::default());
        let candidates = vec![candidate(
            "src_a",
            "a::oil::price::usd",
            Some("ent_hormuz"),
            CandidateKind::Anomaly,
            CandidateDirection::Up,
            4.0,
            0.9,
            0,
        )];

        let mut lens = Lens::new(LensId::new("lens_global"), "WORLD");
        lens.entities.push("ent_hormuz".into());
        let engine = SignalEngine::new(SignalConfig::default()).with_lenses(vec![lens]);

        let signals = process_batch(&mut events, &engine, &candidates, &no_category, at(30));
        assert_eq!(
            signals[0].lens_matches,
            vec![LensId::new("lens_global")],
            "a matching lens must be recorded on the signal"
        );
    }

    /// A lens whose filter the signal does not satisfy must not appear.
    #[test]
    fn a_lens_that_does_not_cover_the_signal_is_not_recorded() {
        let mut events = EventEngine::new(Default::default());
        let candidates = vec![candidate(
            "src_a",
            "a::oil::price::usd",
            Some("ent_hormuz"),
            CandidateKind::Anomaly,
            CandidateDirection::Up,
            4.0,
            0.9,
            0,
        )];

        let mut lens = Lens::new(LensId::new("lens_software"), "SOFTWARE");
        lens.categories.push("technology".into());
        let engine = SignalEngine::new(SignalConfig::default()).with_lenses(vec![lens]);

        let signals = process_batch(&mut events, &engine, &candidates, &no_category, at(30));
        assert!(
            signals[0].lens_matches.is_empty(),
            "an unrelated lens must not match: {:?}",
            signals[0].lens_matches
        );
    }

    /// With no lenses configured, matching is a no-op rather than a panic.
    #[test]
    fn no_lenses_configured_records_nothing() {
        let mut events = EventEngine::new(Default::default());
        let candidates = vec![candidate(
            "src_a",
            "a::oil::price::usd",
            None,
            CandidateKind::Anomaly,
            CandidateDirection::Up,
            4.0,
            0.9,
            0,
        )];
        let signals = process_batch(&mut events, &engine(), &candidates, &no_category, at(30));
        assert!(signals[0].lens_matches.is_empty());
    }
}
