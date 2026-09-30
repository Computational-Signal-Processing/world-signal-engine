//! Backtesting: scoring the detector against data whose outcome is known.
//!
//! The brief is blunt about this — replay exists so that false positives, false
//! negatives, detection latency and signal persistence can be *measured*. A
//! detector that has only ever been watched by eye has not been measured.
//!
//! The method here is deliberately plain, because the numbers have to be
//! trustworthy rather than flattering:
//!
//! * The stream is replayed through the real pipeline, one arrival batch at a
//!   time, with the engine clock pinned to each batch's `received_at`. The
//!   detector therefore sees exactly what it saw live.
//! * Detection latency is `signal.first_seen - earliest evidence observed_at`.
//!   It answers "how long after the world changed did we say so?" and needs no
//!   labels to compute.
//! * Persistence is the signal's own span. A signal that appears and vanishes
//!   in one cycle and one that is sustained for three days are not the same
//!   finding, and this is where that shows up.
//! * False positives and false negatives are only reported when labels are
//!   supplied. With no ground truth the report says so rather than inventing a
//!   precision number, because an unlabelled run cannot distinguish "wrong"
//!   from "not yet known to be right".

use std::collections::BTreeSet;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use wse_collector::Stream;
use wse_model::{Observation, Signal, Source};
use wse_scheduler::SharedReplayClock;
use wse_storage::SignalStore;

use crate::{Engine, EngineConfig};

/// A change that is known to have happened, used as ground truth.
///
/// Identity is `(source_id, metric)`: the series the change occurred in. That
/// is the coarsest identity that is still meaningful — an anomaly in
/// `usgs_earthquakes / magnitude` must not be credited for a signal about
/// `github_rust_activity / commits`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LabeledEvent {
    pub source_id: String,
    pub metric: String,
    /// When the change began, per the source's own clock.
    pub start: DateTime<Utc>,
    /// When it ended. Equal to `start` for an instantaneous spike.
    pub end: DateTime<Utc>,
    /// Free-form label, e.g. `drift`, `spike`, `outage`.
    pub label: String,
}

impl LabeledEvent {
    pub fn new(
        source_id: impl Into<String>,
        metric: impl Into<String>,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        label: impl Into<String>,
    ) -> Self {
        Self {
            source_id: source_id.into(),
            metric: metric.into(),
            start,
            end,
            label: label.into(),
        }
    }

    pub fn contains(&self, at: DateTime<Utc>) -> bool {
        at >= self.start && at <= self.end
    }

    /// Whether the labeled window and `[from, to]` overlap.
    fn overlaps(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> bool {
        from <= self.end && to >= self.start
    }
}

/// One signal the backtest observed, with its measured numbers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BacktestDetection {
    pub signal_id: String,
    pub types: Vec<String>,
    pub title: String,
    pub first_seen: DateTime<Utc>,
    pub last_updated: DateTime<Utc>,
    pub duration_seconds: i64,
    pub evidence_count: usize,
    /// Distinct sources backing this signal.
    pub sources: Vec<String>,
    pub entities: Vec<String>,
    /// `first_seen - earliest evidence observed_at`.
    pub latency_seconds: Option<i64>,
    /// The label of the ground-truth event this signal matched, if any.
    pub matched_label: Option<String>,
}

/// What a backtest run produced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BacktestReport {
    pub observations_replayed: usize,
    pub cycles: usize,
    pub signals_total: usize,
    pub detections: Vec<BacktestDetection>,
    /// Number of ground-truth events supplied. Zero means unlabelled.
    pub truth_total: usize,
    pub matched: usize,
    pub false_positives: usize,
    pub false_negatives: Vec<LabeledEvent>,
    /// Mean detection latency over detections that have evidence.
    pub mean_latency_seconds: Option<f64>,
    pub mean_persistence_seconds: Option<f64>,
    pub max_persistence_seconds: Option<i64>,
    /// Span between the first and last observation in the stream.
    pub stream_span_seconds: Option<i64>,
}

impl BacktestReport {
    /// Whether ground truth was supplied, and therefore whether precision and
    /// recall mean anything.
    pub fn is_labelled(&self) -> bool {
        self.truth_total > 0
    }

    /// Fraction of reported signals that matched a labeled event.
    ///
    /// `None` when unlabelled: an unlabelled run cannot report precision.
    pub fn precision(&self) -> Option<f64> {
        if !self.is_labelled() || self.signals_total == 0 {
            return None;
        }
        Some(self.matched as f64 / self.signals_total as f64)
    }

    /// Fraction of labeled events that were detected.
    pub fn recall(&self) -> Option<f64> {
        if !self.is_labelled() || self.truth_total == 0 {
            return None;
        }
        Some(self.matched as f64 / self.truth_total as f64)
    }
}

/// Run a captured stream through the pipeline and score the result.
///
/// The engine clock is pinned to each arrival batch, so this is deterministic:
/// the same stream and the same configuration produce the same report.
pub fn run_backtest(
    stream: &Stream,
    truth: &[LabeledEvent],
    config: EngineConfig,
) -> BacktestReport {
    let start = stream
        .observations
        .iter()
        .map(|o| o.received_at)
        .min()
        .unwrap_or_else(Utc::now);
    let clock = SharedReplayClock::new(start, Duration::zero());
    let mut engine: Engine = Engine::with_clock(config, clock.handle());

    for source in catalog_for(stream) {
        let _ = engine.register_source(source);
    }

    let cycles = stream.cycles();
    for cycle in &cycles {
        if let Some(first) = cycle.first() {
            clock.set(first.received_at);
        }
        engine.ingest_observations(cycle.clone());
    }

    let signals = engine
        .store()
        .query_signals(&wse_storage::SignalQuery::default())
        .map(|page| page.items)
        .unwrap_or_default();

    let detections = signals
        .iter()
        .map(|signal| detection_for(signal, truth))
        .collect::<Vec<_>>();

    let matched_labels: BTreeSet<String> = detections
        .iter()
        .filter_map(|d| d.matched_label.clone())
        .collect();

    let false_negatives: Vec<LabeledEvent> = truth
        .iter()
        .filter(|event| {
            !detections
                .iter()
                .any(|d| d.matched_label.as_deref() == Some(event.label.as_str()))
        })
        .cloned()
        .collect();

    let false_positives = if truth.is_empty() {
        0
    } else {
        detections
            .iter()
            .filter(|d| d.matched_label.is_none())
            .count()
    };

    let latencies: Vec<i64> = detections
        .iter()
        .filter_map(|d| d.latency_seconds)
        .collect();
    let durations: Vec<i64> = detections.iter().map(|d| d.duration_seconds).collect();

    BacktestReport {
        observations_replayed: stream.observations.len(),
        cycles: cycles.len(),
        signals_total: detections.len(),
        detections,
        truth_total: truth.len(),
        matched: matched_labels.len(),
        false_positives,
        false_negatives,
        mean_latency_seconds: mean(&latencies),
        mean_persistence_seconds: mean(&durations),
        max_persistence_seconds: durations.iter().copied().max(),
        stream_span_seconds: stream
            .observed_span()
            .map(|(from, to)| (to - from).num_seconds()),
    }
}

/// Build a detection from a signal, matching it against ground truth.
fn detection_for(signal: &Signal, truth: &[LabeledEvent]) -> BacktestDetection {
    let series: BTreeSet<(String, String)> = signal
        .evidence
        .iter()
        .map(|e| (e.source_id.as_str().to_string(), e.metric.clone()))
        .collect();

    let earliest = signal.evidence.iter().map(|e| e.observed_at).min();
    let latency_seconds = earliest.map(|at| (signal.first_seen - at).num_seconds());

    // A signal matches a labeled event when it is about the same series and its
    // span intersects the labeled window. Matching on the series rather than on
    // the label is what keeps the check honest: the engine never sees the
    // labels.
    let matched_label = truth
        .iter()
        .find(|event| {
            series.contains(&(event.source_id.clone(), event.metric.clone()))
                && event.overlaps(signal.first_seen, signal.last_updated)
        })
        .map(|event| event.label.clone());

    BacktestDetection {
        signal_id: signal.id.as_str().to_string(),
        types: signal
            .types
            .iter()
            .map(|t| t.as_str().to_string())
            .collect(),
        title: signal.title.clone(),
        first_seen: signal.first_seen,
        last_updated: signal.last_updated,
        duration_seconds: signal.duration_seconds,
        evidence_count: signal.evidence.len(),
        sources: signal
            .evidence
            .iter()
            .map(|e| e.source_id.as_str().to_string())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        entities: signal
            .entities
            .iter()
            .map(|e| e.as_str().to_string())
            .collect(),
        latency_seconds,
        matched_label,
    }
}

/// The source catalog to register: the stream's own header if it carries one,
/// otherwise the minimum needed to rebuild categories from the observations.
fn catalog_for(stream: &Stream) -> Vec<Source> {
    if let Some(header) = &stream.header {
        if !header.sources.is_empty() {
            return header.sources.clone();
        }
    }
    stream
        .source_ids()
        .into_iter()
        .map(|id| Source::new(id.clone(), id.as_str().to_string(), "unknown"))
        .collect()
}

/// Ground-truth events derived from the scripted changes in a stream.
///
/// The synthetic world knows exactly where its drift and spike were injected,
/// so the acceptance scenario can be scored without a hand-written label file.
pub fn truth_from_windows(
    source_id: &str,
    metric: &str,
    origin: DateTime<Utc>,
    step_seconds: i64,
    windows: &[(usize, usize, String)],
) -> Vec<LabeledEvent> {
    windows
        .iter()
        .map(|(from, to, label)| {
            let start = origin + Duration::seconds(*from as i64 * step_seconds);
            let end = origin + Duration::seconds((*to as i64 - 1).max(*from as i64) * step_seconds);
            LabeledEvent::new(source_id, metric, start, end, label.clone())
        })
        .collect()
}

/// Observations whose series the labeled events refer to.
///
/// Used by the CLI to sanity-check that a label file actually matches the
/// stream it is about, rather than silently scoring nothing.
pub fn labeled_series_present(stream: &Stream, truth: &[LabeledEvent]) -> Vec<LabeledEvent> {
    truth
        .iter()
        .filter(|event| {
            stream.observations.iter().any(|o: &Observation| {
                o.source_id.as_str() == event.source_id && o.metric == event.metric
            })
        })
        .cloned()
        .collect()
}

fn mean(values: &[i64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    Some(values.iter().sum::<i64>() as f64 / values.len() as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wse_collector::StreamHeader;
    use wse_model::{RawReference, SourceId};

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
    }

    fn obs(source: &str, metric: &str, value: f64, observed: i64, received: i64) -> Observation {
        Observation::new(
            SourceId::new(source),
            None,
            metric,
            value,
            "unit",
            at(observed),
            RawReference::new(
                format!("https://example.test/{observed}"),
                format!("h{observed}"),
            ),
        )
        .with_received_at(at(received))
    }

    /// A stream with 20 quiet points, then 10 sharply elevated ones.
    fn stream_with_spike() -> Stream {
        let mut observations = Vec::new();
        for i in 0..20 {
            observations.push(obs("src_a", "metric", 100.0, i * 60, i * 60));
        }
        for i in 20..30 {
            observations.push(obs("src_a", "metric", 500.0, i * 60, i * 60));
        }
        Stream {
            header: Some(StreamHeader::new(observations.len() as u64, Vec::new())),
            observations,
        }
    }

    #[test]
    fn a_replayed_spike_is_detected_and_scored_against_truth() {
        let stream = stream_with_spike();
        let truth = vec![LabeledEvent::new(
            "src_a",
            "metric",
            at(20 * 60),
            at(29 * 60),
            "spike",
        )];

        let report = run_backtest(&stream, &truth, EngineConfig::default());

        assert_eq!(report.observations_replayed, 30);
        assert!(report.signals_total > 0, "the spike must be surfaced");
        assert!(report.is_labelled());
        assert_eq!(report.false_negatives.len(), 0, "the spike was found");
        assert!(report.matched >= 1);
        assert_eq!(report.precision(), Some(1.0));
        assert_eq!(report.recall(), Some(1.0));
    }

    #[test]
    fn an_unlabelled_run_reports_no_precision_rather_than_guessing() {
        let stream = stream_with_spike();
        let report = run_backtest(&stream, &[], EngineConfig::default());

        assert!(!report.is_labelled());
        assert_eq!(report.precision(), None);
        assert_eq!(report.recall(), None);
        assert_eq!(report.false_positives, 0);
    }

    #[test]
    fn a_label_the_detector_missed_is_reported_as_a_false_negative() {
        let stream = stream_with_spike();
        // The detector has no chance: this series does not exist in the stream.
        let truth = vec![LabeledEvent::new(
            "src_missing",
            "metric",
            at(0),
            at(600),
            "phantom",
        )];

        let report = run_backtest(&stream, &truth, EngineConfig::default());
        assert_eq!(report.matched, 0);
        assert_eq!(report.false_negatives.len(), 1);
        assert_eq!(report.false_negatives[0].label, "phantom");
        assert_eq!(report.recall(), Some(0.0));
    }

    #[test]
    fn replay_is_deterministic() {
        let stream = stream_with_spike();
        let truth = vec![LabeledEvent::new(
            "src_a",
            "metric",
            at(20 * 60),
            at(29 * 60),
            "spike",
        )];

        let first = run_backtest(&stream, &truth, EngineConfig::default());
        let second = run_backtest(&stream, &truth, EngineConfig::default());

        assert_eq!(first.signals_total, second.signals_total);
        assert_eq!(first.detections, second.detections);
        assert_eq!(first.matched, second.matched);
        // Signal ids are content-derived, so a replay must reproduce them.
        let ids_first: Vec<_> = first.detections.iter().map(|d| &d.signal_id).collect();
        let ids_second: Vec<_> = second.detections.iter().map(|d| &d.signal_id).collect();
        assert_eq!(ids_first, ids_second);
    }

    #[test]
    fn latency_is_measured_from_the_earliest_evidence() {
        let stream = stream_with_spike();
        let report = run_backtest(&stream, &[], EngineConfig::default());
        for detection in &report.detections {
            if let Some(latency) = detection.latency_seconds {
                assert!(
                    latency >= 0,
                    "latency cannot be negative: a signal cannot precede its evidence"
                );
            }
        }
        assert!(report.mean_latency_seconds.is_some());
    }

    #[test]
    fn the_engine_clock_follows_the_stream_not_the_wall_clock() {
        // If the engine used `Utc::now()`, every signal would be stamped near
        // 2026 and this span would be seconds rather than the stream's 29
        // minutes.
        let stream = stream_with_spike();
        let report = run_backtest(&stream, &[], EngineConfig::default());
        assert_eq!(report.stream_span_seconds, Some(29 * 60));
        for detection in &report.detections {
            assert!(
                detection.first_seen < Utc::now() - Duration::days(1),
                "signal {} was stamped with wall-clock time",
                detection.signal_id
            );
        }
    }

    #[test]
    fn truth_windows_convert_indices_to_timestamps() {
        let events = truth_from_windows(
            "src_a",
            "metric",
            at(0),
            60,
            &[(20, 30, "spike".to_string())],
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].start, at(20 * 60));
        assert_eq!(events[0].end, at(29 * 60));
        assert!(events[0].contains(at(25 * 60)));
        assert!(!events[0].contains(at(30 * 60)));
    }

    #[test]
    fn labels_that_do_not_match_the_stream_are_visible() {
        let stream = stream_with_spike();
        let truth = vec![
            LabeledEvent::new("src_a", "metric", at(0), at(60), "real"),
            LabeledEvent::new("src_missing", "metric", at(0), at(60), "typo"),
        ];
        let present = labeled_series_present(&stream, &truth);
        assert_eq!(present.len(), 1);
        assert_eq!(present[0].label, "real");
    }
}
