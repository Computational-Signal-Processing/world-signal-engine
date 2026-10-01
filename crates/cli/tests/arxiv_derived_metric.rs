//! arXiv end to end through the real shipped catalog.
//!
//! `crates/engine/tests/derived_metrics.rs` proves the derivation mechanism
//! against a hand-built source. This closes the loop with the **real** catalog
//! entry: `wse_sources::arxiv::source()` must actually declare the delta, so the
//! shipped arXiv source is detected on `preprint_new` and never on the raw
//! cumulative `preprint_total`.
//!
//! If someone deletes the declaration from `arxiv.rs`, the engine stops deriving
//! and this test fails — which is the point.

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use wse_collector::CollectionMode;
use wse_engine::{Engine, EngineConfig};
use wse_model::{Observation, SourceId};
use wse_scheduler::Clock;
use wse_signals::event::EventConfig;
use wse_signals::SignalConfig;
use wse_storage::{BaselineStore, ObservationStore};

#[derive(Debug)]
struct FixedClock(DateTime<Utc>);

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }

    fn mode(&self) -> CollectionMode {
        CollectionMode::Replay
    }
}

fn origin() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).unwrap()
}

/// The shipped arXiv observation constructor, driven with a controlled total.
fn raw(total: u64, at: DateTime<Utc>) -> Observation {
    wse_sources::arxiv::observation_for("cs_ai", "cs.AI", total, at, b"<feed/>")
}

#[test]
fn the_shipped_arxiv_source_is_detected_on_its_derived_velocity() {
    let t0 = origin();
    let mut engine = Engine::with_clock(
        EngineConfig {
            detector: wse_detection::DetectorConfig::synthetic(),
            event: EventConfig::default(),
            signal: SignalConfig {
                now_window_seconds: i64::MAX,
                ..SignalConfig::default()
            },
            convergence: wse_engine::ConvergenceConfig::default(),
            lenses: Vec::new(),
        },
        Arc::new(FixedClock(t0)),
    );
    engine
        .register_source(wse_sources::arxiv::source())
        .expect("the shipped arxiv source registers");

    // Two polls: 203523 -> 203600 is +77 new preprints.
    engine.ingest_observations(vec![raw(203523, t0)]);
    engine.ingest_observations(vec![raw(203600, t0 + Duration::days(1))]);

    let entity = "arxiv_cs_ai";
    let derived_key = wse_model::series_key(
        &SourceId::new(wse_sources::arxiv::SOURCE_ID),
        entity,
        wse_sources::arxiv::DERIVED_METRIC,
        "preprints",
    );
    let raw_key = wse_model::series_key(
        &SourceId::new(wse_sources::arxiv::SOURCE_ID),
        entity,
        wse_sources::arxiv::RAW_METRIC,
        "preprints",
    );

    // The raw cumulative level is stored but never detection-tracked.
    assert_eq!(
        engine
            .store()
            .latest_observations(&raw_key, 10)
            .unwrap()
            .len(),
        2
    );
    assert!(engine.store().get_baseline(&raw_key).unwrap().is_none());
    // The derived velocity is detection-tracked.
    assert!(engine.store().get_baseline(&derived_key).unwrap().is_some());

    let derived = engine
        .store()
        .latest_observations(&derived_key, 10)
        .unwrap();
    assert_eq!(derived.len(), 1);
    assert_eq!(derived[0].value, 77.0);
}
