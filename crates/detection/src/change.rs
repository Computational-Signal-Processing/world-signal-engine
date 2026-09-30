//! Change detection: the simplest, most explainable layer.
//!
//! A change is *not* an anomaly. It only answers "did the value move, and by
//! how much?" — absolute, relative, velocity and acceleration.

use serde::{Deserialize, Serialize};

/// A quantified change between two consecutive points.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Change {
    pub from: f64,
    pub to: f64,
    /// `to - from`.
    pub absolute: f64,
    /// `(to - from) / |from|`, or `0.0` when `from == 0`.
    pub relative: f64,
    /// Seconds between the two points.
    pub elapsed_seconds: i64,
    /// `absolute / elapsed_seconds`, in units per second.
    pub velocity: f64,
}

impl Change {
    pub fn between(from: f64, to: f64, elapsed_seconds: i64) -> Self {
        let absolute = to - from;
        let relative = if from.abs() < f64::EPSILON {
            0.0
        } else {
            absolute / from.abs()
        };
        let velocity = if elapsed_seconds > 0 {
            absolute / elapsed_seconds as f64
        } else {
            0.0
        };
        Self {
            from,
            to,
            absolute,
            relative,
            elapsed_seconds,
            velocity,
        }
    }

    pub fn is_meaningful(&self, relative_threshold: f64, absolute_threshold: f64) -> bool {
        self.relative.abs() >= relative_threshold || self.absolute.abs() >= absolute_threshold
    }
}

/// Acceleration between two consecutive changes, in units per second squared.
pub fn acceleration(previous: &Change, current: &Change) -> f64 {
    let dt = current.elapsed_seconds as f64;
    if dt <= 0.0 {
        return 0.0;
    }
    (current.velocity - previous.velocity) / dt
}

/// Absolute difference between two values.
pub fn absolute_change(from: f64, to: f64) -> f64 {
    to - from
}

/// Relative difference between two values, guarding against a zero base.
pub fn relative_change(from: f64, to: f64) -> f64 {
    if from.abs() < f64::EPSILON {
        0.0
    } else {
        (to - from) / from.abs()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_quantifies_absolute_and_relative() {
        let c = Change::between(100.0, 105.0, 60);
        assert_eq!(c.absolute, 5.0);
        assert!((c.relative - 0.05).abs() < 1e-12);
        assert!((c.velocity - (5.0 / 60.0)).abs() < 1e-12);
    }

    #[test]
    fn relative_change_of_zero_base_is_zero() {
        assert_eq!(relative_change(0.0, 10.0), 0.0);
        assert_eq!(Change::between(0.0, 10.0, 1).relative, 0.0);
    }

    #[test]
    fn velocity_is_zero_for_zero_elapsed() {
        assert_eq!(Change::between(1.0, 2.0, 0).velocity, 0.0);
    }

    #[test]
    fn meaningful_respects_either_threshold() {
        let tiny_abs_but_large_rel = Change::between(1.0, 1.2, 1);
        assert!(tiny_abs_but_large_rel.is_meaningful(0.05, 100.0));
        let large_abs_but_tiny_rel = Change::between(1000.0, 1005.0, 1);
        assert!(large_abs_but_tiny_rel.is_meaningful(0.5, 1.0));
        let nothing = Change::between(100.0, 100.0, 1);
        assert!(!nothing.is_meaningful(0.05, 1.0));
    }

    #[test]
    fn acceleration_detects_speeding_up() {
        let a = Change::between(0.0, 1.0, 1); // velocity 1
        let b = Change::between(1.0, 3.0, 1); // velocity 2
        assert!((acceleration(&a, &b) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn acceleration_of_steady_motion_is_zero() {
        let a = Change::between(0.0, 2.0, 1);
        let b = Change::between(2.0, 4.0, 1);
        assert_eq!(acceleration(&a, &b), 0.0);
    }
}
