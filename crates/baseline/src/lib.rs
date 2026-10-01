//! # wse-baseline
//!
//! Baseline statistics: the "what is normal" half of detection.
//!
//! Everything here is deliberately classical and explainable. No machine
//! learning is required for the MVP; the brief calls for rolling mean/median,
//! standard deviation, MAD, z-scores, rate of change, percentiles and EWMA.
//!
//! ```
//! use wse_baseline::RollingWindow;
//!
//! let mut window = RollingWindow::new(50, 86_400);
//! for i in 0..10 {
//!     window.push(1_700_000_000 + i * 60, i as f64);
//! }
//! let snapshot = window.snapshot();
//! assert_eq!(snapshot.sample_size, 10);
//! ```

mod derive;
mod stats;

pub use derive::{evaluate, Derived};
pub use stats::{
    ewma, first_differences, mean, median, median_absolute_deviation, percentile, std_dev,
    trend_per_second, volatility,
};

use std::collections::VecDeque;

use chrono::{DateTime, Utc};
use wse_model::BaselineSnapshot;

/// One point in a rolling window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    /// Seconds since the Unix epoch (UTC).
    pub t: i64,
    pub value: f64,
}

impl Sample {
    pub fn new(t: i64, value: f64) -> Self {
        Self { t, value }
    }

    pub fn from_datetime(at: DateTime<Utc>, value: f64) -> Self {
        Self {
            t: at.timestamp(),
            value,
        }
    }
}

/// A fixed-size, time-bounded window of recent samples for one series.
///
/// The window is intentionally simple: a ring buffer trimmed by both a maximum
/// length and a maximum age. This keeps a single machine able to track many
/// series without a database round trip per observation.
#[derive(Debug, Clone)]
pub struct RollingWindow {
    samples: VecDeque<Sample>,
    max_len: usize,
    max_age_seconds: i64,
}

impl RollingWindow {
    /// `max_len` caps memory; `max_age_seconds` caps relevance.
    pub fn new(max_len: usize, max_age_seconds: i64) -> Self {
        assert!(max_len > 0, "rolling window must hold at least one sample");
        Self {
            samples: VecDeque::with_capacity(max_len),
            max_len,
            max_age_seconds,
        }
    }

    /// Append a sample, dropping the oldest samples that fall outside the
    /// length or age bounds.
    pub fn push(&mut self, t: i64, value: f64) {
        self.samples.push_back(Sample::new(t, value));
        while self.samples.len() > self.max_len {
            self.samples.pop_front();
        }
        let cutoff = t - self.max_age_seconds;
        while self.samples.front().is_some_and(|s| s.t < cutoff) {
            self.samples.pop_front();
        }
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn latest(&self) -> Option<Sample> {
        self.samples.back().copied()
    }

    pub fn previous(&self) -> Option<Sample> {
        let n = self.samples.len();
        if n < 2 {
            None
        } else {
            self.samples.get(n - 2).copied()
        }
    }

    /// Chronologically ordered samples.
    pub fn samples(&self) -> Vec<Sample> {
        self.samples.iter().copied().collect()
    }

    pub fn values(&self) -> Vec<f64> {
        self.samples.iter().map(|s| s.value).collect()
    }

    /// Mean of the window, or `None` when empty.
    pub fn mean(&self) -> Option<f64> {
        if self.samples.is_empty() {
            None
        } else {
            Some(mean(&self.values()))
        }
    }

    /// Full statistical description of the window.
    ///
    /// Returns an all-zero snapshot for an empty window so callers can treat it
    /// uniformly; use [`RollingWindow::is_ready`] to gate detection instead of
    /// inspecting the numbers.
    pub fn snapshot(&self) -> BaselineSnapshot {
        let values = self.values();
        let n = values.len();
        if n == 0 {
            return BaselineSnapshot {
                sample_size: 0,
                mean: 0.0,
                median: 0.0,
                std_dev: 0.0,
                mad: 0.0,
                p05: 0.0,
                p95: 0.0,
                ewma: 0.0,
                trend_per_second: 0.0,
                volatility: 0.0,
            };
        }
        let samples = self.samples();
        let ts: Vec<i64> = samples.iter().map(|s| s.t).collect();
        let med = median(&values);
        BaselineSnapshot {
            sample_size: n,
            mean: mean(&values),
            median: med,
            std_dev: std_dev(&values),
            mad: median_absolute_deviation(&values),
            p05: percentile(&values, 0.05),
            p95: percentile(&values, 0.95),
            ewma: ewma(&values, ewma_alpha(n)),
            trend_per_second: trend_per_second(&ts, &values),
            volatility: volatility(&values),
        }
    }

    /// Whether the window has enough history to judge deviation.
    pub fn is_ready(&self, min_samples: usize) -> bool {
        self.samples.len() >= min_samples
    }
}

/// Standard EWMA smoothing factor for a window of `n` samples.
pub fn ewma_alpha(n: usize) -> f64 {
    if n == 0 {
        return 1.0;
    }
    2.0 / (n as f64 + 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_trims_by_length() {
        let mut w = RollingWindow::new(3, 1_000_000);
        for i in 0..5 {
            w.push(100 + i, i as f64);
        }
        assert_eq!(w.len(), 3);
        assert_eq!(w.values(), vec![2.0, 3.0, 4.0]);
    }

    #[test]
    fn window_trims_by_age() {
        let mut w = RollingWindow::new(100, 100);
        w.push(0, 1.0);
        w.push(50, 2.0);
        w.push(200, 3.0);
        // cutoff = 200 - 100 = 100, so only the sample at t=200 survives.
        assert_eq!(w.len(), 1);
        assert_eq!(w.values(), vec![3.0]);
    }

    #[test]
    fn latest_and_previous_are_ordered() {
        let mut w = RollingWindow::new(10, 1000);
        assert!(w.latest().is_none());
        w.push(1, 10.0);
        w.push(2, 20.0);
        assert_eq!(w.latest().unwrap().value, 20.0);
        assert_eq!(w.previous().unwrap().value, 10.0);
    }

    #[test]
    fn snapshot_of_constant_series_is_degenerate() {
        let mut w = RollingWindow::new(10, 1000);
        for i in 0..10 {
            w.push(i, 5.0);
        }
        let s = w.snapshot();
        assert_eq!(s.sample_size, 10);
        assert_eq!(s.mean, 5.0);
        assert_eq!(s.median, 5.0);
        assert_eq!(s.std_dev, 0.0);
        assert_eq!(s.mad, 0.0);
        assert_eq!(s.trend_per_second, 0.0);
    }

    #[test]
    fn empty_snapshot_is_all_zero() {
        let w = RollingWindow::new(10, 1000);
        let s = w.snapshot();
        assert_eq!(s.sample_size, 0);
        assert_eq!(s.mean, 0.0);
        assert!(!w.is_ready(1));
    }

    #[test]
    fn readiness_gate_works() {
        let mut w = RollingWindow::new(10, 1000);
        w.push(0, 1.0);
        assert!(!w.is_ready(3));
        w.push(1, 1.0);
        w.push(2, 1.0);
        assert!(w.is_ready(3));
    }
}
