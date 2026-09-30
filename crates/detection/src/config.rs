//! Detector thresholds. All are explicit so a signal can be explained as
//! "this crossed *this* configured threshold".

use serde::{Deserialize, Serialize};

/// Tunables for the classical detectors.
///
/// Defaults are conservative: they favour missing a weak signal over flooding
/// the feed with false positives, because the whole point of the system is to
/// surface a *small* number of things worth investigating.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DetectorConfig {
    /// Minimum samples before any deviation is judged.
    pub min_samples: usize,
    /// Absolute robust z-score that counts as an anomaly.
    pub robust_z_threshold: f64,
    /// Absolute classical z-score that counts as an anomaly.
    pub z_threshold: f64,
    /// Minimum relative change versus the previous point (0.05 = 5%).
    pub relative_change_threshold: f64,
    /// Minimum absolute change versus the previous point.
    pub absolute_change_threshold: f64,
    /// Minimum velocity, in baseline standard deviations per step.
    pub velocity_sigma_threshold: f64,
    /// Consecutive points required for an early signal.
    pub early_signal_min_points: usize,
    /// Minimum latest magnitude (in robust sigma) for an early signal.
    pub early_signal_min_sigma: f64,
    /// Minimum span, in seconds, an early-signal run must cover.
    pub early_signal_min_duration_seconds: i64,
    /// Maximum allowed downward wobble when checking acceleration.
    pub early_signal_accel_tolerance: f64,
}

impl Default for DetectorConfig {
    fn default() -> Self {
        Self {
            min_samples: 20,
            robust_z_threshold: 3.5,
            z_threshold: 3.0,
            relative_change_threshold: 0.05,
            absolute_change_threshold: 0.0,
            velocity_sigma_threshold: 3.0,
            early_signal_min_points: 4,
            early_signal_min_sigma: 0.4,
            early_signal_min_duration_seconds: 3 * 24 * 3600,
            early_signal_accel_tolerance: 0.05,
        }
    }
}

impl DetectorConfig {
    /// A configuration suited to dense, fast synthetic or high-frequency
    /// series: shorter persistence windows and smaller sample minimums.
    pub fn synthetic() -> Self {
        Self {
            min_samples: 10,
            early_signal_min_points: 4,
            early_signal_min_duration_seconds: 0,
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_conservative() {
        let c = DetectorConfig::default();
        assert!(c.robust_z_threshold >= 3.0);
        assert!(c.min_samples >= 10);
    }

    #[test]
    fn synthetic_requires_fewer_samples() {
        assert!(DetectorConfig::synthetic().min_samples < DetectorConfig::default().min_samples);
    }
}
