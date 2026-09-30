//! Signals: what a human actually looks at.
//!
//! A signal is an event promoted to human attention, carrying the evidence
//! needed to explain *why* it appeared. The model never stores a single opaque
//! "importance" number; quality is multi-dimensional (see [`SignalQuality`]).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::anomaly::CandidateDirection;
use crate::geo::Location;
use crate::ids::{EntityId, EventId, LensId, ObservationId, SignalId, SourceId};

/// Signal types. These are *not* an importance ranking and may co-occur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SignalType {
    /// A meaningful change happening right now.
    Now,
    /// A clear departure from normal behaviour.
    Anomaly,
    /// Small but persistent, directional, accelerating change.
    EarlySignal,
    /// Independent sources pointing at the same change.
    Convergence,
    /// A change with meaningful potential impact for a lens/entity.
    Impact,
}

impl SignalType {
    pub fn as_str(self) -> &'static str {
        match self {
            SignalType::Now => "NOW",
            SignalType::Anomaly => "ANOMALY",
            SignalType::EarlySignal => "EARLY_SIGNAL",
            SignalType::Convergence => "CONVERGENCE",
            SignalType::Impact => "IMPACT",
        }
    }
}

/// Backwards-compatible alias; direction lives with candidates.
pub type Direction = CandidateDirection;

/// One piece of supporting evidence, always traceable to an observation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    pub source_id: SourceId,
    pub observation_id: ObservationId,
    pub metric: String,
    pub unit: String,
    /// Human-readable statement, e.g. "temperature +4.1σ over 18 min".
    pub statement: String,
    pub observed_at: DateTime<Utc>,
    pub value: f64,
    /// Robust z-score or equivalent deviation measure, when available.
    pub deviation_sigma: Option<f64>,
}

/// Multi-dimensional quality. The UI renders these separately rather than
/// collapsing them into one "importance" score.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignalQuality {
    /// `0..=1`: how new this change is relative to recent history.
    pub novelty: f64,
    /// `0..=1`: magnitude of deviation.
    pub strength: f64,
    /// `0..=1`: how long it has been sustained.
    pub persistence: f64,
    /// `0..=1`: detector confidence.
    pub confidence: f64,
    /// `0..=1`: how many distinct entities/categories it touches.
    pub breadth: f64,
    /// `0..=1`: how many independent sources agree.
    pub convergence: f64,
    /// `0..=1`: relevance to the active lens set.
    pub relevance: f64,
}

impl Default for SignalQuality {
    fn default() -> Self {
        Self {
            novelty: 0.0,
            strength: 0.0,
            persistence: 0.0,
            confidence: 0.0,
            breadth: 0.0,
            convergence: 0.0,
            relevance: 0.0,
        }
    }
}

impl SignalQuality {
    /// Deterministic ordering key for feeds. Deliberately *not* exposed as
    /// "importance" to the user.
    pub fn rank(&self) -> f64 {
        0.25 * self.strength
            + 0.20 * self.persistence
            + 0.20 * self.confidence
            + 0.15 * self.convergence
            + 0.10 * self.novelty
            + 0.10 * self.relevance
    }
}

/// A signal presented to humans, always linked back to an event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Signal {
    pub id: SignalId,
    pub event_id: EventId,
    pub types: Vec<SignalType>,
    pub title: String,
    pub summary: String,
    pub first_seen: DateTime<Utc>,
    pub last_updated: DateTime<Utc>,
    pub duration_seconds: i64,
    pub confidence: f64,
    pub evidence: Vec<Evidence>,
    pub entities: Vec<EntityId>,
    pub categories: Vec<String>,
    pub location: Option<Location>,
    pub lens_matches: Vec<LensId>,
    pub quality: SignalQuality,
    pub direction: CandidateDirection,
    /// Explainability: why the engine emitted this signal.
    pub reasons: Vec<String>,
    /// The series this signal is about. Together with `direction` it forms the
    /// signal's identity, which is what makes a signal *persistent* across
    /// collection cycles rather than a new one every minute.
    pub series_key: String,
}

impl Signal {
    pub fn new(event_id: EventId, at: DateTime<Utc>) -> Self {
        Self {
            id: SignalId::generate(),
            event_id,
            types: Vec::new(),
            title: String::new(),
            summary: String::new(),
            first_seen: at,
            last_updated: at,
            duration_seconds: 0,
            confidence: 0.0,
            evidence: Vec::new(),
            entities: Vec::new(),
            categories: Vec::new(),
            location: None,
            lens_matches: Vec::new(),
            quality: SignalQuality::default(),
            direction: CandidateDirection::Flat,
            reasons: Vec::new(),
            series_key: String::new(),
        }
    }

    /// A stable, content-derived id.
    ///
    /// Using a deterministic id (rather than a random UUID) means re-forming
    /// the same signal after a restart updates the existing record instead of
    /// creating a duplicate.
    pub fn stable_id(
        event_id: &EventId,
        series_key: &str,
        direction: CandidateDirection,
    ) -> SignalId {
        let hash = crate::ids::fnv1a_hex(&format!(
            "{}|{}|{}",
            event_id.as_str(),
            series_key,
            direction.as_str()
        ));
        SignalId::new(format!("sig_{hash}"))
    }

    pub fn add_type(&mut self, ty: SignalType) {
        if !self.types.contains(&ty) {
            self.types.push(ty);
        }
    }

    pub fn has_type(&self, ty: SignalType) -> bool {
        self.types.contains(&ty)
    }

    /// Distinct sources represented in the evidence.
    pub fn distinct_sources(&self) -> usize {
        let mut ids: Vec<&str> = self.evidence.iter().map(|e| e.source_id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        ids.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_are_deduplicated() {
        let mut s = Signal::new(EventId::new("evt_1"), Utc::now());
        s.add_type(SignalType::Anomaly);
        s.add_type(SignalType::Anomaly);
        s.add_type(SignalType::Convergence);
        assert_eq!(s.types.len(), 2);
        assert!(s.has_type(SignalType::Convergence));
    }

    #[test]
    fn distinct_sources_counts_unique_source_ids() {
        let mut s = Signal::new(EventId::new("evt_1"), Utc::now());
        for (src, obs) in [("src_a", "obs_1"), ("src_a", "obs_2"), ("src_b", "obs_3")] {
            s.evidence.push(Evidence {
                source_id: SourceId::new(src),
                observation_id: ObservationId::new(obs),
                metric: "m".into(),
                unit: "u".into(),
                statement: "s".into(),
                observed_at: Utc::now(),
                value: 0.0,
                deviation_sigma: None,
            });
        }
        assert_eq!(s.distinct_sources(), 2);
    }

    #[test]
    fn rank_is_bounded_by_weights() {
        let q = SignalQuality {
            novelty: 1.0,
            strength: 1.0,
            persistence: 1.0,
            confidence: 1.0,
            breadth: 1.0,
            convergence: 1.0,
            relevance: 1.0,
        };
        assert!((q.rank() - 1.0).abs() < 1e-9);
        assert_eq!(SignalQuality::default().rank(), 0.0);
    }

    #[test]
    fn signal_round_trips_through_serde() {
        let mut s = Signal::new(EventId::new("evt_1"), Utc::now());
        s.add_type(SignalType::Now);
        s.title = "Oil moving".into();
        let json = serde_json::to_string(&s).unwrap();
        let back: Signal = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }
}
