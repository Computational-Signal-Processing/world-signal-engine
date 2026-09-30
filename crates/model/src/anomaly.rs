//! Anomaly candidates: detection output that is *not yet* a signal.
//!
//! The brief is explicit that `score > threshold -> signal` is too naive.
//! Detectors therefore emit [`AnomalyCandidate`] values; the event and signal
//! engines decide what deserves human attention.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{AnomalyId, EntityId, ObservationId, SourceId};

/// The statistical description of "normal" that a candidate deviates from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BaselineSnapshot {
    /// Number of points the baseline was computed from.
    pub sample_size: usize,
    pub mean: f64,
    pub median: f64,
    pub std_dev: f64,
    pub mad: f64,
    pub p05: f64,
    pub p95: f64,
    pub ewma: f64,
    pub trend_per_second: f64,
    pub volatility: f64,
}

/// Which detector produced a candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DetectionMethod {
    /// Absolute/relative change from the previous point.
    Change,
    /// Robust z-score against a rolling baseline.
    RobustZScore,
    /// Classical z-score against a rolling baseline.
    ZScore,
    /// Rate of change (velocity) anomaly.
    Velocity,
    /// Sustained directional drift (early-signal precursor).
    PersistenceDrift,
    /// Direction change / regime shift.
    RegimeShift,
}

/// What kind of attention a candidate is asking for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CandidateKind {
    /// A large, immediate deviation.
    Anomaly,
    /// A small but persistent, directional, accelerating change.
    EarlySignal,
}

/// Direction of a change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateDirection {
    Up,
    Down,
    Flat,
}

impl CandidateDirection {
    pub fn from_delta(delta: f64) -> Self {
        if delta > 0.0 {
            CandidateDirection::Up
        } else if delta < 0.0 {
            CandidateDirection::Down
        } else {
            CandidateDirection::Flat
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            CandidateDirection::Up => "up",
            CandidateDirection::Down => "down",
            CandidateDirection::Flat => "flat",
        }
    }
}

/// A detection result awaiting event/signal formation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnomalyCandidate {
    pub id: AnomalyId,
    pub source_id: SourceId,
    pub entity_id: Option<EntityId>,
    pub series_key: String,
    pub metric: String,
    pub unit: String,
    pub observation_id: ObservationId,
    pub observed_at: DateTime<Utc>,
    pub baseline: BaselineSnapshot,
    pub current: f64,
    /// Signed deviation from the baseline, in baseline units.
    pub deviation: f64,
    /// Robust z-score when the detector could compute one.
    pub score: f64,
    pub method: DetectionMethod,
    pub kind: CandidateKind,
    pub direction: CandidateDirection,
    /// How long the deviation has persisted, in seconds.
    pub duration_seconds: i64,
    /// Detector confidence in `0.0..=1.0`.
    pub confidence: f64,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

impl AnomalyCandidate {
    pub fn new(
        series_key: impl Into<String>,
        observation_id: ObservationId,
        observed_at: DateTime<Utc>,
        baseline: BaselineSnapshot,
        current: f64,
    ) -> Self {
        Self {
            id: AnomalyId::generate(),
            source_id: SourceId::new(""),
            entity_id: None,
            series_key: series_key.into(),
            metric: String::new(),
            unit: String::new(),
            observation_id,
            observed_at,
            deviation: current - baseline.mean,
            baseline,
            current,
            score: 0.0,
            method: DetectionMethod::Change,
            kind: CandidateKind::Anomaly,
            direction: CandidateDirection::Flat,
            duration_seconds: 0,
            confidence: 0.0,
            latitude: None,
            longitude: None,
        }
    }

    /// Signed deviation expressed in robust (MAD) units.
    pub fn robust_z(&self) -> f64 {
        robust_z_score(self.current, self.baseline.median, self.baseline.mad)
    }
}

/// Robust z-score: `0.6745 * (x - median) / MAD`.
///
/// Returns `0.0` when MAD is zero (a perfectly flat baseline carries no
/// information about scale) unless the value also equals the median, in which
/// case it is genuinely zero. Callers should treat a zero-MAD baseline as
/// "cannot judge" rather than "normal".
pub fn robust_z_score(x: f64, median: f64, mad: f64) -> f64 {
    if mad <= f64::EPSILON {
        return 0.0;
    }
    0.6745 * (x - median) / mad
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn robust_z_is_zero_when_mad_is_zero() {
        assert_eq!(robust_z_score(10.0, 5.0, 0.0), 0.0);
    }

    #[test]
    fn robust_z_matches_hand_calculation() {
        // 0.6745 * (10 - 5) / 2 = 1.68625
        let z = robust_z_score(10.0, 5.0, 2.0);
        assert!((z - 1.68625).abs() < 1e-9);
    }

    #[test]
    fn direction_follows_sign_of_delta() {
        assert_eq!(CandidateDirection::from_delta(1.0), CandidateDirection::Up);
        assert_eq!(
            CandidateDirection::from_delta(-1.0),
            CandidateDirection::Down
        );
        assert_eq!(
            CandidateDirection::from_delta(0.0),
            CandidateDirection::Flat
        );
    }

    #[test]
    fn candidate_round_trips_through_serde() {
        let snap = BaselineSnapshot {
            sample_size: 100,
            mean: 1.0,
            median: 1.0,
            std_dev: 0.5,
            mad: 0.4,
            p05: 0.2,
            p95: 1.8,
            ewma: 1.0,
            trend_per_second: 0.0,
            volatility: 0.5,
        };
        let c = AnomalyCandidate::new(
            "src::ent::m::u",
            ObservationId::new("obs_1"),
            Utc::now(),
            snap,
            4.0,
        );
        let json = serde_json::to_string(&c).unwrap();
        let back: AnomalyCandidate = serde_json::from_str(&json).unwrap();
        assert_eq!(back, c);
    }
}
