//! # wse-detection
//!
//! Turns a series of observations into [`AnomalyCandidate`]s.
//!
//! Detection is split into small, independently testable pieces:
//!
//! * [`change`] — did the value change at all, and by how much?
//! * [`anomaly`] — is the value far from its rolling baseline?
//! * [`early`] — is a *small* deviation persistent, directional and accelerating?
//!
//! The detectors never decide what a human should see. They emit candidates;
//! the event and signal engines (see `wse-signals`) decide.

pub mod anomaly;
pub mod change;
pub mod config;
pub mod early;
pub mod series;

pub use anomaly::detect_anomaly;
pub use change::{absolute_change, relative_change, Change};
pub use config::DetectorConfig;
pub use early::detect_early_signal;
pub use series::SeriesTracker;

use wse_model::AnomalyCandidate;

/// Anything that can turn a stream of points into candidates.
pub trait Detector: Send + Sync {
    fn name(&self) -> &'static str;

    /// Inspect the tracker state after a new point has been pushed and return
    /// zero or more candidates.
    fn detect(&self, tracker: &SeriesTracker) -> Vec<AnomalyCandidate>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use wse_model::{ObservationId, SourceId};

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
    }

    /// Build a tracker with `n` points from a generator, then a final point.
    fn tracker_with(cfg: DetectorConfig, history: &[f64], final_value: f64) -> SeriesTracker {
        let mut t = SeriesTracker::new("src::ent::m::u", cfg);
        for (i, v) in history.iter().enumerate() {
            t.push(
                at(i as i64 * 60),
                *v,
                ObservationId::new(format!("obs_{i}")),
                SourceId::new("src"),
            );
        }
        t.push(
            at(history.len() as i64 * 60),
            final_value,
            ObservationId::new("obs_final"),
            SourceId::new("src"),
        );
        t
    }

    #[test]
    fn anomaly_detector_flags_a_spike() {
        // Tight normal cloud around 100, then a huge spike.
        let history: Vec<f64> = (0..50).map(|i| 100.0 + (i % 5) as f64 * 0.1).collect();
        let tracker = tracker_with(DetectorConfig::synthetic(), &history, 500.0);
        let candidates = detect_anomaly(&tracker);
        assert!(
            !candidates.is_empty(),
            "spike should be detected as an anomaly"
        );
        assert!(candidates[0].score >= tracker.config().robust_z_threshold);
    }

    #[test]
    fn anomaly_detector_ignores_noise() {
        let history: Vec<f64> = (0..50).map(|i| 100.0 + (i % 5) as f64 * 0.1).collect();
        let tracker = tracker_with(DetectorConfig::synthetic(), &history, 100.2);
        assert!(detect_anomaly(&tracker).is_empty());
    }

    #[test]
    fn detector_trait_is_object_safe() {
        let detectors: Vec<Box<dyn Detector>> = vec![
            Box::new(anomaly::AnomalyDetector::default()),
            Box::new(early::EarlySignalDetector::default()),
        ];
        assert_eq!(detectors.len(), 2);
        assert_eq!(detectors[0].name(), "anomaly");
        assert_eq!(detectors[1].name(), "early_signal");
    }
}
