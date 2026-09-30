//! End-to-end replay: capture a world, write it to a stream, read it back and
//! score the detector against it.
//!
//! This is the Phase 10 acceptance test. It exercises the whole chain the brief
//! asks for:
//!
//! ```text
//! observations -> stream file -> read back -> replay -> detect -> signals
//!                                                        -> backtest scores
//! ```
//!
//! Nothing here touches the network, and every assertion is on a number rather
//! than on "it seemed to work".

use std::io::Cursor;

use chrono::{DateTime, Duration, Utc};
use wse_collector::replay::{read_stream, write_stream, ReplayCollector, StreamHeader};
use wse_collector::synthetic::SyntheticWorld;
use wse_detection::DetectorConfig;
use wse_engine::ConvergenceConfig;
use wse_engine::{run_backtest, truth_from_windows, Engine, EngineConfig, LabeledEvent};
use wse_model::{SignalType, Source};
use wse_signals::event::EventConfig;
use wse_signals::SignalConfig;
use wse_storage::SignalStore;

fn origin() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).unwrap()
}

/// The same permissive configuration the acceptance test uses: the synthetic
/// series are short, so the sample minimum is low.
fn config() -> EngineConfig {
    EngineConfig {
        detector: DetectorConfig::synthetic(),
        event: EventConfig::default(),
        signal: SignalConfig {
            now_window_seconds: i64::MAX,
            ..SignalConfig::default()
        },
        convergence: ConvergenceConfig::related(),
        lenses: Vec::new(),
    }
}

fn source() -> Source {
    Source::new(
        wse_model::SourceId::new("synthetic_sensor"),
        "Synthetic Sensor",
        "synthetic",
    )
    .with_category("geophysics")
}

/// The acceptance world's script, as ground truth.
///
/// The world injects a drift at 100..120 and a spike at 200. The labels describe
/// what *should* be found; the detector never sees them.
fn truth() -> Vec<LabeledEvent> {
    truth_from_windows(
        "synthetic_sensor",
        "sensor",
        origin(),
        3600,
        &[
            (100, 120, "drift".to_string()),
            (200, 201, "spike".to_string()),
        ],
    )
}

/// Capture the acceptance world into an in-memory stream file.
fn captured_stream(steps: usize) -> Vec<u8> {
    let world = SyntheticWorld::acceptance(origin());
    let observations = world.generate(steps);
    let header = StreamHeader::new(observations.len() as u64, vec![source()]);
    let mut buffer = Vec::new();
    write_stream(&mut buffer, &header, &observations).unwrap();
    buffer
}

#[test]
fn a_captured_stream_round_trips_through_the_file_format() {
    let bytes = captured_stream(205);
    let stream = read_stream(Cursor::new(bytes)).unwrap();

    assert_eq!(stream.observations.len(), 205);
    assert_eq!(stream.header.as_ref().unwrap().sources.len(), 1);
    assert_eq!(
        stream.observed_span().unwrap().1 - stream.observed_span().unwrap().0,
        Duration::seconds(204 * 3600)
    );
}

#[test]
fn replaying_a_captured_stream_finds_the_scripted_drift_and_spike() {
    let bytes = captured_stream(205);
    let stream = read_stream(Cursor::new(bytes)).unwrap();

    let report = run_backtest(&stream, &truth(), config());

    assert_eq!(report.observations_replayed, 205);
    assert!(
        report.signals_total > 0,
        "the scripted changes must be surfaced"
    );
    assert_eq!(
        report.false_negatives.len(),
        0,
        "both scripted changes should be found, missed: {:?}",
        report.false_negatives
    );
    assert!(
        report.matched >= 2,
        "drift and spike are two distinct changes"
    );
    assert_eq!(report.recall(), Some(1.0));
}

#[test]
fn backtest_is_deterministic_across_runs() {
    let bytes = captured_stream(205);
    let stream = read_stream(Cursor::new(bytes)).unwrap();

    let first = run_backtest(&stream, &truth(), config());
    let second = run_backtest(&stream, &truth(), config());

    assert_eq!(first.signals_total, second.signals_total);
    assert_eq!(first.detections, second.detections);
    assert_eq!(first.matched, second.matched);
    assert_eq!(first.mean_latency_seconds, second.mean_latency_seconds);

    // Signal ids are content-derived, so a replay must reproduce them. This is
    // what lets a replayed run update existing records instead of minting new
    // ones for a change that never changed.
    let ids_first: Vec<&String> = first.detections.iter().map(|d| &d.signal_id).collect();
    let ids_second: Vec<&String> = second.detections.iter().map(|d| &d.signal_id).collect();
    assert_eq!(ids_first, ids_second);
    assert!(!ids_first.is_empty());
}

#[test]
fn an_unlabelled_backtest_withholds_precision_rather_than_inventing_it() {
    let bytes = captured_stream(205);
    let stream = read_stream(Cursor::new(bytes)).unwrap();

    let report = run_backtest(&stream, &[], config());
    assert!(!report.is_labelled());
    assert_eq!(report.precision(), None);
    assert_eq!(report.recall(), None);
    // Latency and persistence need no labels, so they are still reported.
    assert!(report.signals_total > 0);
    assert!(report.mean_persistence_seconds.is_some());
}

#[tokio::test]
async fn the_replay_collector_drives_the_pipeline_like_a_live_one() {
    let bytes = captured_stream(205);
    let stream = read_stream(Cursor::new(bytes)).unwrap();
    let expected_cycles = stream.cycles().len();

    let collector = ReplayCollector::from_stream(&stream).unwrap();
    assert_eq!(collector.total_cycles(), expected_cycles);

    let mut engine = Engine::new(config());
    engine.register_source(source()).unwrap();

    let mut cycles = 0;
    while !collector.is_exhausted() {
        let outcome = engine.run_collector(&collector).await;
        assert!(!outcome.source_failed, "replay must not report a failure");
        cycles += 1;
    }
    assert_eq!(cycles, expected_cycles);

    // Running past the end is an empty success, never a failure: "no more data"
    // must not be confused with "the source broke".
    let after_end = engine.run_collector(&collector).await;
    assert!(!after_end.source_failed);
    assert!(after_end.observations_ingested == 0);

    assert_eq!(engine.metrics().observations_total, 205);
    let signals = engine
        .store()
        .query_signals(&wse_storage::SignalQuery::default())
        .unwrap();
    assert!(signals
        .items
        .iter()
        .any(|s| s.has_type(SignalType::Anomaly)));
}

#[test]
fn the_same_stream_scores_the_same_through_the_collector_and_the_backtest() {
    // Two entry points into replay — the `Collector` interface and the backtest
    // driver — must agree, otherwise "replay" means two different things.
    let bytes = captured_stream(205);
    let stream = read_stream(Cursor::new(bytes)).unwrap();

    let report = run_backtest(&stream, &truth(), config());
    let via_collector = {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let collector = ReplayCollector::from_stream(&stream).unwrap();
            let mut engine = Engine::new(config());
            engine.register_source(source()).unwrap();
            while !collector.is_exhausted() {
                engine.run_collector(&collector).await;
            }
            engine
                .store()
                .query_signals(&wse_storage::SignalQuery::default())
                .unwrap()
                .total
        })
    };

    assert_eq!(report.signals_total, via_collector);
}
