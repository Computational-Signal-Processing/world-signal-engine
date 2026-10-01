//! Backtesting the detector against a real, hand-checked history.
//!
//! Phase 10 built the backtest mechanism but scored it only against the
//! synthetic world's *scripted* changes — ground truth we invented, which
//! cannot catch a detector that is calibrated to itself. This test supplies
//! ground truth from the world instead: the M7.0+ earthquakes in the checked-in
//! USGS catalog (`tests/fixtures/usgs_9mo_2026.geojson`, the real feed over
//! 2026-01-01..2026-09-30, M5.0+). Those ten quakes are objectively notable,
//! and the labels were written by reading the catalog, not by running the
//! detector.
//!
//! The stream is reconstructed the way a live monitor would have seen it: each
//! quake arrives at the next hourly poll after it occurred, and every poll
//! re-serves the hour's records (de-duplicated by record key), so the detector
//! sees real arrival batching rather than one observation per quake.
//!
//! The numbers are deliberately asserted as *measurements*, not as targets. The
//! point is that precision and recall are now computed against reality and
//! reported honestly — a low precision here is a finding about the detector,
//! not a reason to move the label.

use chrono::DateTime;
use wse_engine::{run_backtest, EngineConfig, LabeledEvent};
use wse_sources::usgs;

fn fixture() -> Vec<u8> {
    include_bytes!("../../../tests/fixtures/usgs_9mo_2026.geojson").to_vec()
}

fn labels() -> Vec<LabeledEvent> {
    serde_json::from_slice(include_bytes!(
        "../../../tests/fixtures/usgs_m7_labels_2026.json"
    ))
    .expect("the shipped label file must parse")
}

/// The captured stream, with its catalog header and arrival-ordered records.
fn stream() -> wse_collector::Stream {
    wse_collector::Stream {
        header: Some(wse_collector::StreamHeader::new(0, vec![usgs::source()])),
        observations: replayed_observations(),
    }
}

/// Rebuild the stream the way an hourly poller would have observed it.
fn replayed_observations() -> Vec<wse_model::Observation> {
    let parsed_at = DateTime::from_timestamp(1_767_225_600, 0).unwrap(); // 2026-01-01
    let mut observations = usgs::parse(&fixture(), parsed_at).expect("the fixture must parse");
    for observation in &mut observations {
        // A quake appears in the feed at the first poll after it happened.
        let next_hour = (observation.observed_at.timestamp() / 3600 + 1) * 3600;
        observation.received_at = DateTime::from_timestamp(next_hour, 0).unwrap();
    }
    observations.sort_by_key(|o| o.received_at);
    observations
}

#[test]
fn the_shipped_history_produces_a_labelled_backtest() {
    let stream = stream();
    assert!(
        stream.observations.len() > 1000,
        "the fixture is the real 9-month M5+ catalog: {}",
        stream.observations.len()
    );
    let truth = labels();
    assert_eq!(
        wse_engine::backtest::labeled_series_present(&stream, &truth).len(),
        truth.len(),
        "every label must name a series the stream actually carries"
    );
    let report = run_backtest(&stream, &truth, EngineConfig::default());

    assert!(report.is_labelled());
    assert_eq!(report.truth_total, 10, "ten M7+ quakes are labelled");
    assert!(
        report.signals_total > 0,
        "a real 9-month earthquake history must produce signals"
    );
    // A labelled report computes precision and recall rather than withholding.
    assert!(report.precision().is_some());
    assert!(report.recall().is_some());
    assert!(
        report.matched > 0,
        "at least one M7+ quake must be detected: {:#?}",
        report.false_negatives
    );

    // The measured numbers, pinned so detector drift is visible and reviewed
    // rather than silent. This detector currently catches half of the labelled
    // M7+ events and emits four unlabelled signals for each one it catches.
    // Those are the facts; improving them is the work, not this assertion.
    assert_eq!(report.matched, 5, "M7+ events detected");
    assert_eq!(report.recall(), Some(0.5));
    assert_eq!(report.precision(), Some(0.2));
}

/// The measurement is reproducible: the same stream and labels score the same.
///
/// A detector whose precision wobbles between identical runs cannot be
/// improved, because there is no fixed number to move.
#[test]
fn the_measurement_is_deterministic() {
    let stream = stream();
    let truth = labels();
    let first = run_backtest(&stream, &truth, EngineConfig::default());
    let second = run_backtest(&stream, &truth, EngineConfig::default());
    assert_eq!(first, second);
}

/// Latency needs no labels: it answers "how long after the world changed did we
/// say so?" even for a quake we did not detect.
#[test]
fn detection_latency_is_reported_for_real_events() {
    let report = run_backtest(&stream(), &labels(), EngineConfig::default());
    let latency = report
        .mean_latency_seconds
        .expect("detections with evidence report a latency");
    // A quake is observed at its own time but arrives at the next hourly poll,
    // so latency is bounded by the polling interval plus detection.
    assert!(
        (0.0..=2.0 * 3600.0).contains(&latency),
        "hourly polling bounds latency to ~an hour, got {latency}s"
    );
}

/// Guard the fixture's provenance: it must stay the real catalog, not a stub.
#[test]
fn the_fixture_is_the_real_catalog() {
    let parsed_at = DateTime::from_timestamp(1_767_225_600, 0).unwrap();
    let observations = usgs::parse(&fixture(), parsed_at).unwrap();
    let magnitudes: Vec<f64> = observations.iter().map(|o| o.value).collect();
    assert!(magnitudes.iter().all(|m| *m >= 5.0 - 1e-9));
    assert!(
        magnitudes.iter().cloned().fold(f64::MIN, f64::max) >= 7.0,
        "the catalog contains M7+ events"
    );
    let regions: std::collections::BTreeSet<&str> = observations
        .iter()
        .filter_map(|o| o.entity_id.as_ref().map(|e| e.as_str()))
        .collect();
    assert!(
        regions.len() > 50,
        "a global catalog touches many regions, got {}",
        regions.len()
    );
}
