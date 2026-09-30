//! Per-series state: the rolling window plus the metadata detectors need to
//! describe a deviation in human terms.

use chrono::{DateTime, Utc};
use wse_baseline::RollingWindow;
use wse_model::{BaselineSnapshot, Observation, ObservationId, SourceId};

use crate::config::DetectorConfig;

/// Tracks one time series over time.
///
/// The tracker owns no detection logic; it is the shared state that the
/// detectors read. This keeps detectors pure functions of `&SeriesTracker`,
/// which makes them trivial to test with synthetic input.
#[derive(Debug, Clone)]
pub struct SeriesTracker {
    series_key: String,
    config: DetectorConfig,
    window: RollingWindow,
    source_id: SourceId,
    entity_id: Option<wse_model::EntityId>,
    metric: String,
    unit: String,
    latitude: Option<f64>,
    longitude: Option<f64>,
    latest_observation: Option<ObservationId>,
    latest_at: Option<DateTime<Utc>>,
    points_seen: u64,
}

impl SeriesTracker {
    pub fn new(series_key: impl Into<String>, config: DetectorConfig) -> Self {
        let window = RollingWindow::new(
            // Keep a generous window so baseline statistics stay meaningful,
            // bounded by the configured minimum plus headroom.
            config.min_samples.max(64) * 8,
            90 * 24 * 3600,
        );
        Self {
            series_key: series_key.into(),
            config,
            window,
            source_id: SourceId::new(""),
            entity_id: None,
            metric: String::new(),
            unit: String::new(),
            latitude: None,
            longitude: None,
            latest_observation: None,
            latest_at: None,
            points_seen: 0,
        }
    }

    /// Build a tracker directly from an observation, capturing its metadata.
    pub fn from_observation(obs: &Observation, config: DetectorConfig) -> Self {
        let mut tracker = Self::new(obs.series_key(), config);
        tracker.source_id = obs.source_id.clone();
        tracker.entity_id = obs.entity_id.clone();
        tracker.metric = obs.metric.clone();
        tracker.unit = obs.unit.clone();
        tracker.latitude = obs.latitude;
        tracker.longitude = obs.longitude;
        tracker
    }

    pub fn config(&self) -> &DetectorConfig {
        &self.config
    }

    pub fn series_key(&self) -> &str {
        &self.series_key
    }

    pub fn source_id(&self) -> &SourceId {
        &self.source_id
    }

    pub fn entity_id(&self) -> Option<&wse_model::EntityId> {
        self.entity_id.as_ref()
    }

    pub fn metric(&self) -> &str {
        &self.metric
    }

    pub fn unit(&self) -> &str {
        &self.unit
    }

    pub fn latitude(&self) -> Option<f64> {
        self.latitude
    }

    pub fn longitude(&self) -> Option<f64> {
        self.longitude
    }

    pub fn latest_observation(&self) -> Option<&ObservationId> {
        self.latest_observation.as_ref()
    }

    pub fn latest_at(&self) -> Option<DateTime<Utc>> {
        self.latest_at
    }

    pub fn points_seen(&self) -> u64 {
        self.points_seen
    }

    pub fn window(&self) -> &RollingWindow {
        &self.window
    }

    /// Feed a new point. The window is updated *before* detection runs, so
    /// detectors see the current point as part of the history; they use
    /// [`SeriesTracker::baseline_before_latest`] when they need a baseline that
    /// excludes it.
    pub fn push(
        &mut self,
        at: DateTime<Utc>,
        value: f64,
        observation_id: ObservationId,
        source_id: SourceId,
    ) {
        self.window.push(at.timestamp(), value);
        self.latest_observation = Some(observation_id);
        self.latest_at = Some(at);
        self.points_seen += 1;
        if self.source_id.as_str().is_empty() {
            self.source_id = source_id;
        }
    }

    pub fn latest_value(&self) -> Option<f64> {
        self.window.latest().map(|s| s.value)
    }

    pub fn previous_value(&self) -> Option<f64> {
        self.window.previous().map(|s| s.value)
    }

    /// Baseline computed over every point *except* the latest one.
    ///
    /// This is the statistically honest baseline: comparing a value against a
    /// window that already contains it dilutes the deviation.
    pub fn baseline_before_latest(&self) -> BaselineSnapshot {
        let samples = self.window.samples();
        if samples.len() < 2 {
            return self.window.snapshot();
        }
        let history = &samples[..samples.len() - 1];
        let values: Vec<f64> = history.iter().map(|s| s.value).collect();
        let ts: Vec<i64> = history.iter().map(|s| s.t).collect();
        snapshot(&values, &ts)
    }

    /// Baseline over the whole window.
    pub fn baseline(&self) -> BaselineSnapshot {
        self.window.snapshot()
    }

    pub fn is_ready(&self) -> bool {
        self.window.is_ready(self.config.min_samples)
    }
}

/// Compute a [`BaselineSnapshot`] from raw values and timestamps.
pub fn snapshot(values: &[f64], timestamps: &[i64]) -> BaselineSnapshot {
    if values.is_empty() {
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
    use wse_baseline::{
        ewma, mean, median, median_absolute_deviation, percentile, std_dev, trend_per_second,
        volatility,
    };
    BaselineSnapshot {
        sample_size: values.len(),
        mean: mean(values),
        median: median(values),
        std_dev: std_dev(values),
        mad: median_absolute_deviation(values),
        p05: percentile(values, 0.05),
        p95: percentile(values, 0.95),
        ewma: ewma(values, 2.0 / (values.len() as f64 + 1.0)),
        trend_per_second: trend_per_second(timestamps, values),
        volatility: volatility(values),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
    }

    #[test]
    fn tracker_accumulates_points() {
        let mut t = SeriesTracker::new("k", DetectorConfig::synthetic());
        for i in 0..5 {
            t.push(
                at(i),
                i as f64,
                ObservationId::new(format!("o{i}")),
                SourceId::new("s"),
            );
        }
        assert_eq!(t.points_seen(), 5);
        assert_eq!(t.latest_value(), Some(4.0));
        assert_eq!(t.previous_value(), Some(3.0));
    }

    #[test]
    fn baseline_before_latest_excludes_the_spike() {
        let mut t = SeriesTracker::new("k", DetectorConfig::synthetic());
        for i in 0..30 {
            t.push(
                at(i),
                10.0,
                ObservationId::new(format!("o{i}")),
                SourceId::new("s"),
            );
        }
        t.push(
            at(31),
            1000.0,
            ObservationId::new("spike"),
            SourceId::new("s"),
        );
        let before = t.baseline_before_latest();
        let whole = t.baseline();
        assert_eq!(before.median, 10.0);
        assert!(
            whole.mean > before.mean,
            "including the spike should raise the mean"
        );
    }

    #[test]
    fn baseline_before_latest_handles_tiny_history() {
        let mut t = SeriesTracker::new("k", DetectorConfig::synthetic());
        t.push(at(0), 1.0, ObservationId::new("o0"), SourceId::new("s"));
        // Only one point: no "before latest" history exists, fall back to whole window.
        assert_eq!(t.baseline_before_latest().sample_size, 1);
    }
}
