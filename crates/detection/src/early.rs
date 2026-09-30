//! Early-signal detection.
//!
//! The canonical case from the brief:
//!
//! ```text
//! Day 1  +0.3σ
//! Day 2  +0.5σ
//! Day 3  +0.8σ
//! Day 4  +1.2σ
//! Day 5  +1.6σ
//! ```
//!
//! No single point is anomalous. The *combination* of a small deviation that
//! is persistent, directional and accelerating is what matters. This detector
//! looks for exactly that shape.

use wse_model::{
    robust_z_score, AnomalyCandidate, BaselineSnapshot, CandidateDirection, CandidateKind,
    DetectionMethod,
};

use crate::config::DetectorConfig;
use crate::series::{snapshot, SeriesTracker};

/// A detector for small-but-persistent directional drift.
#[derive(Debug, Clone, Default)]
pub struct EarlySignalDetector {
    config: DetectorConfig,
}

impl EarlySignalDetector {
    pub fn new(config: DetectorConfig) -> Self {
        Self { config }
    }
}

impl crate::Detector for EarlySignalDetector {
    fn name(&self) -> &'static str {
        "early_signal"
    }

    fn detect(&self, tracker: &SeriesTracker) -> Vec<AnomalyCandidate> {
        detect_early_signal_with(tracker, &self.config)
    }
}

/// Convenience entry point using the tracker's own configuration.
pub fn detect_early_signal(tracker: &SeriesTracker) -> Vec<AnomalyCandidate> {
    detect_early_signal_with(tracker, tracker.config())
}

/// Minimum absolute deviation for a point to count as part of a drift run.
///
/// Below this the point is indistinguishable from baseline noise, and including
/// it would let a flat series masquerade as a persistent trend.
const RUN_MEMBERSHIP_SIGMA: f64 = 0.1;

/// Core early-signal logic.
pub fn detect_early_signal_with(
    tracker: &SeriesTracker,
    config: &DetectorConfig,
) -> Vec<AnomalyCandidate> {
    if !tracker.is_ready() {
        return Vec::new();
    }
    let Some(observed_at) = tracker.latest_at() else {
        return Vec::new();
    };
    let Some(observation_id) = tracker.latest_observation().cloned() else {
        return Vec::new();
    };
    let samples = tracker.window().samples();
    let n = samples.len();
    if n < config.early_signal_min_points + 1 {
        return Vec::new();
    }

    // Compare the recent tail against a baseline drawn from *before* it, so the
    // drift we are hunting cannot hide inside its own reference. The probe
    // starts generous and doubles while the run fills it, which is how a drift
    // longer than the initial probe is still captured.
    let max_probe = n.saturating_sub(2).max(1);
    let mut probe = (n / 4)
        .max(config.early_signal_min_points)
        .clamp(1, max_probe);
    let mut chosen = None;
    for _ in 0..4 {
        let (baseline, sigmas) = baseline_and_sigmas(&samples[..n - probe], &samples[n - probe..]);
        if baseline.mad <= f64::EPSILON {
            return Vec::new();
        }
        let run = trailing_run_len(&sigmas);
        if run == 0 {
            return Vec::new();
        }
        if run < probe {
            chosen = Some((baseline, sigmas, run));
            break;
        }
        // The run fills the whole probe: it probably started earlier.
        let next = (probe * 2).min(max_probe);
        if next == probe {
            chosen = Some((baseline, sigmas, run));
            break;
        }
        probe = next;
    }
    let Some((baseline, sigmas, run_len)) = chosen else {
        return Vec::new();
    };

    if run_len < config.early_signal_min_points {
        return Vec::new();
    }

    let run = &samples[n - run_len..];
    let run_sigmas = &sigmas[sigmas.len() - run_len..];
    let duration_seconds = run[run.len() - 1].t - run[0].t;
    if duration_seconds < config.early_signal_min_duration_seconds {
        return Vec::new();
    }

    let latest_sigma = *run_sigmas.last().expect("run is non-empty");
    let magnitude = latest_sigma.abs();
    if magnitude < config.early_signal_min_sigma {
        return Vec::new();
    }
    // Already a full anomaly: leave it to the anomaly detector so the two
    // detectors do not describe the same point twice.
    if magnitude >= config.robust_z_threshold {
        return Vec::new();
    }

    if !is_accelerating(run_sigmas, config.early_signal_accel_tolerance) {
        return Vec::new();
    }

    let direction = CandidateDirection::from_delta(latest_sigma);
    let current = run[run.len() - 1].value;
    let mut candidate = AnomalyCandidate::new(
        tracker.series_key(),
        observation_id,
        observed_at,
        baseline,
        current,
    );
    candidate.source_id = tracker.source_id().clone();
    candidate.entity_id = tracker.entity_id().cloned();
    candidate.metric = tracker.metric().to_string();
    candidate.unit = tracker.unit().to_string();
    candidate.deviation = current - candidate.baseline.median;
    candidate.score = latest_sigma;
    candidate.method = DetectionMethod::PersistenceDrift;
    candidate.kind = CandidateKind::EarlySignal;
    candidate.direction = direction;
    candidate.duration_seconds = duration_seconds;
    candidate.confidence = early_confidence(run_len, config, magnitude, run_sigmas);
    candidate.latitude = tracker.latitude();
    candidate.longitude = tracker.longitude();
    vec![candidate]
}

/// Compute a robust baseline from `history` and the signed sigma of each point
/// in `tail` relative to it.
fn baseline_and_sigmas(
    history: &[wse_baseline::Sample],
    tail: &[wse_baseline::Sample],
) -> (BaselineSnapshot, Vec<f64>) {
    let values: Vec<f64> = history.iter().map(|s| s.value).collect();
    let ts: Vec<i64> = history.iter().map(|s| s.t).collect();
    let baseline = snapshot(&values, &ts);
    let sigmas = tail
        .iter()
        .map(|s| robust_z_score(s.value, baseline.median, baseline.mad))
        .collect();
    (baseline, sigmas)
}

/// Length of the trailing run of same-sign, above-floor deviations.
///
/// "Same sign" is taken from the most recent point: an early signal is about
/// where the series is *going*.
fn trailing_run_len(sigmas: &[f64]) -> usize {
    let Some(&last) = sigmas.last() else {
        return 0;
    };
    let sign = last.signum();
    if sign == 0.0 {
        return 0;
    }
    let mut len = 0;
    for sigma in sigmas.iter().rev() {
        if sigma.signum() != sign || sigma.abs() < RUN_MEMBERSHIP_SIGMA {
            break;
        }
        len += 1;
    }
    // A run of one is not persistence.
    if len >= 2 {
        len
    } else {
        0
    }
}

/// Whether magnitude grows across the run.
///
/// Uses the least-squares slope of `|sigma|` rather than strict monotonicity:
/// real series wobble, and demanding point-by-point growth would miss every
/// genuine drift that is not perfectly smooth. Requires a positive slope *and*
/// net growth from the start of the run to the end.
fn is_accelerating(sigmas: &[f64], tolerance: f64) -> bool {
    if sigmas.len() < 2 {
        return false;
    }
    let n = sigmas.len() as f64;
    let mean_x = (n - 1.0) / 2.0;
    let mean_y = sigmas.iter().map(|s| s.abs()).sum::<f64>() / n;
    let mut num = 0.0;
    let mut den = 0.0;
    for (i, sigma) in sigmas.iter().enumerate() {
        let dx = i as f64 - mean_x;
        num += dx * (sigma.abs() - mean_y);
        den += dx * dx;
    }
    let slope = if den <= f64::EPSILON { 0.0 } else { num / den };
    let grew = sigmas.last().map(|s| s.abs()).unwrap_or(0.0)
        > sigmas.first().map(|s| s.abs()).unwrap_or(0.0);
    slope > tolerance && grew
}

/// Confidence in `0.0..=1.0`, derived from run length, magnitude and slope.
fn early_confidence(
    run_len: usize,
    config: &DetectorConfig,
    magnitude: f64,
    sigmas: &[f64],
) -> f64 {
    let length_score =
        (run_len as f64 / (config.early_signal_min_points as f64 * 2.0)).clamp(0.0, 1.0);
    let magnitude_score = (magnitude / config.robust_z_threshold).clamp(0.0, 1.0);
    let growth = match (sigmas.first(), sigmas.last()) {
        (Some(first), Some(last)) if first.abs() > f64::EPSILON => {
            ((last.abs() - first.abs()) / first.abs()).clamp(0.0, 1.0)
        }
        _ => 0.0,
    };
    (0.4 * length_score + 0.4 * magnitude_score + 0.2 * growth).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use wse_model::{ObservationId, SourceId};

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
    }

    /// A baseline with real spread: values cycle over five distinct levels so
    /// the median never lands exactly on a single level (which would make MAD
    /// zero and the series undecidable).
    fn baseline_values(n: usize) -> Vec<f64> {
        (0..n).map(|i| 100.0 + (i % 5) as f64 * 0.1 - 0.2).collect()
    }

    fn tracker(history: &[f64], tail: &[f64]) -> SeriesTracker {
        let mut t = SeriesTracker::new("s::e::m::u", DetectorConfig::synthetic());
        for (i, v) in history.iter().enumerate() {
            t.push(
                at(i as i64),
                *v,
                ObservationId::new(format!("o{i}")),
                SourceId::new("s"),
            );
        }
        for (j, v) in tail.iter().enumerate() {
            t.push(
                at((history.len() + j) as i64),
                *v,
                ObservationId::new(format!("t{j}")),
                SourceId::new("s"),
            );
        }
        t
    }

    #[test]
    fn trailing_run_is_same_sign_and_above_floor() {
        assert_eq!(trailing_run_len(&[0.3, 0.5, 0.8]), 3);
        assert_eq!(trailing_run_len(&[0.3, 0.5, -0.8]), 0); // sign break -> run of 1 -> 0
        assert_eq!(trailing_run_len(&[0.3, 0.5, 0.01]), 0); // last is below floor
    }

    #[test]
    fn accelerating_requires_growth() {
        assert!(is_accelerating(&[0.3, 0.5, 0.8, 1.2, 1.6], 0.05));
        assert!(!is_accelerating(&[0.8, 0.8, 0.8], 0.05)); // flat: no slope, no net growth
        assert!(!is_accelerating(&[1.2, 0.8, 0.5], 0.05)); // shrinking
        assert!(!is_accelerating(&[1.0], 0.05)); // single point is not a run
                                                 // Wobbly but clearly rising overall.
        assert!(is_accelerating(&[0.3, 0.6, 0.55, 0.9, 1.1], 0.05));
    }

    #[test]
    fn gradual_drift_is_flagged_as_early_signal() {
        // Tight baseline with real spread, then a small persistent ramp whose
        // final magnitude is well below the anomaly threshold.
        let history = baseline_values(60);
        let tail = [100.2, 100.3, 100.4, 100.5, 100.6];
        let t = tracker(&history, &tail);
        let candidates = detect_early_signal(&t);
        assert_eq!(
            candidates.len(),
            1,
            "drift should yield exactly one early signal"
        );
        let c = &candidates[0];
        assert_eq!(c.kind, CandidateKind::EarlySignal);
        assert_eq!(c.method, DetectionMethod::PersistenceDrift);
        assert_eq!(c.direction, CandidateDirection::Up);
        assert!(c.score > 0.0);
        assert!(c.score < DetectorConfig::synthetic().robust_z_threshold);
        assert!(c.confidence > 0.0);
        assert!(c.duration_seconds > 0);
    }

    #[test]
    fn downward_drift_is_detected_too() {
        let history = baseline_values(60);
        let tail = [99.8, 99.7, 99.6, 99.5, 99.4];
        let t = tracker(&history, &tail);
        let c = &detect_early_signal(&t)[0];
        assert_eq!(c.direction, CandidateDirection::Down);
    }

    #[test]
    fn pure_noise_is_not_an_early_signal() {
        let history = baseline_values(60);
        let tail = [100.0, 100.1, 99.9, 100.05, 100.0];
        let t = tracker(&history, &tail);
        assert!(detect_early_signal(&t).is_empty());
    }

    #[test]
    fn a_full_anomaly_is_left_to_the_anomaly_detector() {
        let history = baseline_values(60);
        let tail = [110.0, 120.0, 140.0, 170.0, 210.0];
        let t = tracker(&history, &tail);
        assert!(
            detect_early_signal(&t).is_empty(),
            "an obvious anomaly should not also be reported as an early signal"
        );
    }

    #[test]
    fn non_monotonic_recovery_is_not_early_signal() {
        let history = baseline_values(60);
        // Rises then falls back: persistent but not accelerating.
        let tail = [100.3, 100.7, 101.1, 100.5, 100.1];
        let t = tracker(&history, &tail);
        assert!(detect_early_signal(&t).is_empty());
    }
}
