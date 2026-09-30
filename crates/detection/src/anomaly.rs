//! Anomaly detection: deviation from a rolling baseline.
//!
//! Emits an [`AnomalyCandidate`] — never a signal. Both a robust z-score
//! (MAD-based, resistant to outliers) and a classical z-score are computed;
//! either can trigger, and the candidate records which one did.

use wse_model::{
    robust_z_score, AnomalyCandidate, BaselineSnapshot, CandidateDirection, CandidateKind,
    DetectionMethod, ObservationId,
};

use crate::config::DetectorConfig;
use crate::series::SeriesTracker;

/// A detector that flags large deviations from the rolling baseline.
#[derive(Debug, Clone, Default)]
pub struct AnomalyDetector {
    config: DetectorConfig,
}

impl AnomalyDetector {
    pub fn new(config: DetectorConfig) -> Self {
        Self { config }
    }
}

impl crate::Detector for AnomalyDetector {
    fn name(&self) -> &'static str {
        "anomaly"
    }

    fn detect(&self, tracker: &SeriesTracker) -> Vec<AnomalyCandidate> {
        let mut candidates = detect_anomaly_with(tracker, &self.config);
        candidates.retain(|_| true);
        candidates
    }
}

/// Convenience entry point using the tracker's own configuration.
pub fn detect_anomaly(tracker: &SeriesTracker) -> Vec<AnomalyCandidate> {
    detect_anomaly_with(tracker, tracker.config())
}

/// Core anomaly logic.
pub fn detect_anomaly_with(
    tracker: &SeriesTracker,
    config: &DetectorConfig,
) -> Vec<AnomalyCandidate> {
    if !tracker.is_ready() {
        return Vec::new();
    }
    let Some(current) = tracker.latest_value() else {
        return Vec::new();
    };
    let Some(observed_at) = tracker.latest_at() else {
        return Vec::new();
    };
    let Some(observation_id) = tracker.latest_observation().cloned() else {
        return Vec::new();
    };

    let baseline = tracker.baseline_before_latest();
    if baseline.sample_size == 0 {
        return Vec::new();
    }

    let robust = robust_z_score(current, baseline.median, baseline.mad);
    let classical = classical_z(current, &baseline);

    // A zero-MAD baseline means the history is perfectly flat. Any movement is
    // then infinitely surprising, which is not a useful score; fall back to the
    // classical z-score and let that decide.
    let (score, method) = if baseline.mad > f64::EPSILON {
        if robust.abs() >= classical.abs() {
            (robust, DetectionMethod::RobustZScore)
        } else {
            (classical, DetectionMethod::ZScore)
        }
    } else {
        (classical, DetectionMethod::ZScore)
    };

    let robust_triggered = baseline.mad > f64::EPSILON && robust.abs() >= config.robust_z_threshold;
    let classical_triggered = classical.abs() >= config.z_threshold;
    if !robust_triggered && !classical_triggered {
        return Vec::new();
    }

    let deviation = current - baseline.mean;
    let mut candidate = AnomalyCandidate::new(
        tracker.series_key(),
        observation_id,
        observed_at,
        baseline.clone(),
        current,
    );
    candidate.source_id = tracker.source_id().clone();
    candidate.entity_id = tracker.entity_id().cloned();
    candidate.metric = tracker.metric().to_string();
    candidate.unit = tracker.unit().to_string();
    candidate.deviation = deviation;
    candidate.score = score;
    candidate.method = method;
    candidate.kind = CandidateKind::Anomaly;
    candidate.direction = CandidateDirection::from_delta(deviation);
    candidate.confidence = confidence_from_score(score.abs(), config.robust_z_threshold);
    candidate.latitude = tracker.latitude();
    candidate.longitude = tracker.longitude();
    candidate.duration_seconds = 0;
    candidate.identity = tracker.latest_identity().map(str::to_string);
    candidate.record_label = tracker.latest_record_label().map(str::to_string);

    vec![candidate]
}

/// Classical z-score: `(x - mean) / std_dev`. Zero when the series is flat.
pub fn classical_z(current: f64, baseline: &BaselineSnapshot) -> f64 {
    if baseline.std_dev <= f64::EPSILON {
        0.0
    } else {
        (current - baseline.mean) / baseline.std_dev
    }
}

/// Map an absolute score onto `0.0..=1.0` confidence.
///
/// Reaches `1.0` at roughly three times the trigger threshold, which keeps
/// confidence monotonic in score without ever saturating immediately.
pub fn confidence_from_score(score: f64, threshold: f64) -> f64 {
    if threshold <= 0.0 {
        return 0.0;
    }
    let ratio = score / (threshold * 3.0);
    ratio.clamp(0.0, 1.0)
}

/// Helper used by tests and the signal engine to build a candidate by hand.
pub fn candidate_from(
    tracker: &SeriesTracker,
    observation_id: ObservationId,
    baseline: BaselineSnapshot,
    current: f64,
    score: f64,
) -> AnomalyCandidate {
    let mut c = AnomalyCandidate::new(
        tracker.series_key(),
        observation_id,
        tracker.latest_at().unwrap_or_else(chrono::Utc::now),
        baseline,
        current,
    );
    c.score = score;
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::series::SeriesTracker;
    use chrono::{DateTime, Utc};
    use wse_model::{ObservationId, SourceId};

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
    }

    fn tracker(history: &[f64], final_value: f64) -> SeriesTracker {
        let mut t = SeriesTracker::new("s::e::m::u", DetectorConfig::synthetic());
        for (i, v) in history.iter().enumerate() {
            t.push(
                at(i as i64),
                *v,
                ObservationId::new(format!("o{i}")),
                SourceId::new("s"),
            );
        }
        t.push(
            at(history.len() as i64),
            final_value,
            ObservationId::new("of"),
            SourceId::new("s"),
        );
        t
    }

    #[test]
    fn classical_z_is_zero_for_flat_baseline() {
        let b = BaselineSnapshot {
            sample_size: 5,
            mean: 1.0,
            median: 1.0,
            std_dev: 0.0,
            mad: 0.0,
            p05: 1.0,
            p95: 1.0,
            ewma: 1.0,
            trend_per_second: 0.0,
            volatility: 0.0,
        };
        assert_eq!(classical_z(50.0, &b), 0.0);
    }

    #[test]
    fn spike_is_flagged_with_high_score() {
        let history: Vec<f64> = (0..60).map(|i| 50.0 + (i % 7) as f64 * 0.2).collect();
        let t = tracker(&history, 900.0);
        let candidates = detect_anomaly(&t);
        assert_eq!(candidates.len(), 1);
        let c = &candidates[0];
        assert_eq!(c.kind, CandidateKind::Anomaly);
        assert_eq!(c.direction, CandidateDirection::Up);
        assert!(c.score > 10.0, "score was {}", c.score);
        assert!(c.confidence > 0.9);
        assert_eq!(c.metric, "");
    }

    #[test]
    fn downward_spike_has_down_direction() {
        let history: Vec<f64> = (0..60).map(|i| 50.0 + (i % 7) as f64 * 0.2).collect();
        let t = tracker(&history, -900.0);
        let c = &detect_anomaly(&t)[0];
        assert_eq!(c.direction, CandidateDirection::Down);
        assert!(c.score < 0.0);
    }

    #[test]
    fn flat_series_never_produces_an_anomaly() {
        let history: Vec<f64> = vec![10.0; 60];
        // Even a modest move off a perfectly flat baseline is judged classically,
        // but a value equal to the baseline must never trigger.
        let t = tracker(&history, 10.0);
        assert!(detect_anomaly(&t).is_empty());
    }

    #[test]
    fn insufficient_history_produces_nothing() {
        let t = tracker(&[1.0, 1.0, 1.0], 999.0);
        assert!(detect_anomaly(&t).is_empty());
    }

    #[test]
    fn confidence_is_monotonic_and_bounded() {
        assert_eq!(confidence_from_score(0.0, 3.5), 0.0);
        let low = confidence_from_score(3.5, 3.5);
        let mid = confidence_from_score(7.0, 3.5);
        let high = confidence_from_score(50.0, 3.5);
        assert!(low < mid && mid < high);
        assert!(high <= 1.0);
    }
}
