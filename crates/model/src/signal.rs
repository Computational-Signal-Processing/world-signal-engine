//! Signals: what a human actually looks at.
//!
//! A signal is an event promoted to human attention, carrying the evidence
//! needed to explain *why* it appeared. The model never stores a single opaque
//! "importance" number; quality is multi-dimensional (see [`SignalQuality`]).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::anomaly::{BaselineSnapshot, CandidateDirection};
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

/// Where a signal is in its life.
///
/// This is deliberately separate from [`SignalType`]. A type says *what kind of
/// change* this is; the status says *how far along it is*. The UI needs both:
/// a signal can be an `ANOMALY` that is `CONFIRMED`, or an `EARLY_SIGNAL` that
/// is still `NEW`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SignalStatus {
    /// Seen for the first time in the most recent cycle.
    New,
    /// Observed across more than one cycle and still changing.
    Developing,
    /// Corroborated: more than one independent source, or a sustained span.
    Confirmed,
    /// No longer changing, but not yet over.
    Stable,
    /// Past its peak and receding toward normal.
    Fading,
    /// Back to normal.
    Resolved,
}

impl SignalStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            SignalStatus::New => "NEW",
            SignalStatus::Developing => "DEVELOPING",
            SignalStatus::Confirmed => "CONFIRMED",
            SignalStatus::Stable => "STABLE",
            SignalStatus::Fading => "FADING",
            SignalStatus::Resolved => "RESOLVED",
        }
    }
}

/// Whether a signal's backing data comes from a live source or a synthetic one.
///
/// The brief is explicit that a person must never mistake seeded/demo data for
/// the real world. The flag is carried on the signal so the UI can label it,
/// rather than being inferred from a source id by convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DataOrigin {
    /// Collected from a real, external source.
    #[default]
    Live,
    /// Produced by the deterministic synthetic world (demo, tests, replay of
    /// captured fixtures).
    Synthetic,
}

/// A human-readable answer to "what changed, in the world's terms".
///
/// Detection speaks in series keys and sigma. Nobody reads those. This is the
/// translation, kept as data (not prose baked into the UI) so the API, the web
/// client and any future client all say the same thing, and so the mapping can
/// be tested like any other logic.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SignalNarrative {
    /// One sentence a person can read: "Software attention is rising sharply".
    pub headline: String,
    /// What changed, named in human terms ("Attention on Hacker News stories").
    pub subject: String,
    /// Where, when the data actually supports a place. `None` when it does not.
    pub where_text: Option<String>,
    /// What the metric did, phrased for a person: "rose 8.4% to 623 points".
    pub what_changed: String,
    /// The direction as a person reads it: "rising", "falling", "accelerating".
    pub direction_text: String,
    /// A sentence describing the magnitude without a raw sigma.
    pub magnitude_text: String,
    /// One line on why the engine surfaced it — still evidence, not a verdict.
    pub why_signal: String,
    /// How many independent sources stand behind it.
    pub evidence_sources: usize,
    /// What the engine does *not* know. Always populated.
    ///
    /// The brief asks the system to know what it does not know. This is where
    /// that is stated plainly instead of left to the reader to infer.
    pub unknowns: Vec<String>,
}

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
    /// What the series looked like immediately before this point.
    ///
    /// Carried on the evidence so the reader can compare "what it was" with
    /// "what it is" without a second request, and so a replayed or restored
    /// signal explains itself from its own record rather than from the current
    /// state of the store.
    #[serde(default)]
    pub baseline: Option<BaselineSnapshot>,
    /// The observation's per-record discriminator, when the source set one.
    ///
    /// This is where a human name for the record lives for sources that emit
    /// many records per series — a Hacker News story title, a repository name.
    /// It is what lets the UI say "the story *X* is getting unusual attention"
    /// instead of naming the metric `story_score`.
    #[serde(default)]
    pub identity: Option<String>,
    /// A human name for the specific record this observation measured.
    ///
    /// For a source that emits many records per series, this is the thing the
    /// reader recognises — a story title, a repository name, an asteroid name.
    /// It is filled from the observation's attributes, so the signal can say
    /// *which* story is getting attention rather than only that "attention"
    /// moved.
    #[serde(default)]
    pub record_label: Option<String>,
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
    /// Where this signal is in its life.
    ///
    /// Recomputed on every merge from how long the signal has been observed and
    /// how many independent sources back it, so the feed does not show a
    /// three-day-old change as if it were new.
    #[serde(default = "default_status")]
    pub status: SignalStatus,
    /// Whether the data behind this signal is live or synthetic.
    #[serde(default)]
    pub data_origin: DataOrigin,
    /// The human-readable translation of the detection output.
    #[serde(default)]
    pub narrative: SignalNarrative,
}

fn default_status() -> SignalStatus {
    SignalStatus::Developing
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
            status: SignalStatus::Developing,
            data_origin: DataOrigin::Live,
            narrative: SignalNarrative::default(),
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
        self.distinct_source_ids().len()
    }

    /// The distinct source ids behind the signal, sorted and de-duplicated.
    ///
    /// Sorted so a caller that routes by provenance writes lens matches in a
    /// stable order; a replayed run must produce the same signal bytes.
    pub fn distinct_source_ids(&self) -> Vec<&str> {
        let mut ids: Vec<&str> = self.evidence.iter().map(|e| e.source_id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        ids
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
                baseline: None,
                identity: None,
                record_label: None,
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
