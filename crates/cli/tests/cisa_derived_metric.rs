//! CAP-2B — the CISA catalog size is a cumulative level; detect its growth.
//!
//! `kev_catalog_total` only ever grows, so a level z-score on it fires on the
//! fact that the catalog exists, not on anything that changed. The quantity
//! worth detecting is how many vulnerabilities were *added* since the previous
//! poll. This drives the **real shipped catalog** through the real engine and
//! asserts the stored result: the raw total is evidence-only, and the derived
//! `kev_catalog_growth` series is what detection sees.
//!
//! Mirrors `arxiv_derived_metric.rs`; if someone removes the declaration from
//! `cisa_kev.rs`, the engine stops deriving and this test fails.

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use wse_collector::CollectionMode;
use wse_engine::{Engine, EngineConfig};
use wse_model::{EntityId, Observation, RawReference, SourceId};
use wse_scheduler::Clock;
use wse_signals::event::EventConfig;
use wse_signals::SignalConfig;
use wse_storage::{BaselineStore, ObservationStore};

const ENTITY: &str = "cyber_kev";
const RAW_METRIC: &str = "kev_catalog_total";
const DERIVED_METRIC: &str = "kev_catalog_growth";

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

fn engine(at: DateTime<Utc>) -> Engine {
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
        Arc::new(FixedClock(at)),
    );
    engine
        .register_source(wse_sources::cisa_kev::source())
        .expect("the shipped cisa_kev source registers");
    engine
}

/// A raw cumulative catalog-size observation, shaped exactly like the
/// collector's `kev_catalog_total`.
fn raw(total: u64, at: DateTime<Utc>) -> Observation {
    Observation::new(
        SourceId::new(wse_sources::cisa_kev::SOURCE_ID),
        Some(EntityId::new(ENTITY)),
        RAW_METRIC,
        total as f64,
        "vulnerabilities",
        at,
        RawReference::new(
            "https://www.cisa.gov/known-exploited-vulnerabilities-catalog",
            "h",
        ),
    )
    .with_received_at(at)
}

fn key(metric: &str) -> String {
    wse_model::series_key(
        &SourceId::new(wse_sources::cisa_kev::SOURCE_ID),
        ENTITY,
        metric,
        "vulnerabilities",
    )
}

#[test]
fn the_shipped_cisa_source_is_detected_on_its_derived_growth() {
    let t0 = origin();
    let t1 = t0 + Duration::days(1);
    let mut engine = engine(t0);

    // 1000 -> 1012: twelve vulnerabilities added in the interval.
    engine.ingest_observations(vec![raw(1000, t0)]);
    engine.ingest_observations(vec![raw(1012, t1)]);

    // The raw cumulative level is stored but never detection-tracked.
    assert_eq!(
        engine
            .store()
            .latest_observations(&key(RAW_METRIC), 10)
            .unwrap()
            .len(),
        2
    );
    assert!(
        engine
            .store()
            .get_baseline(&key(RAW_METRIC))
            .unwrap()
            .is_none(),
        "the raw catalog size must not be detection-tracked"
    );
    // The derived growth is detection-tracked.
    assert!(
        engine
            .store()
            .get_baseline(&key(DERIVED_METRIC))
            .unwrap()
            .is_some(),
        "the derived growth series must be detection-tracked"
    );

    let derived = engine
        .store()
        .latest_observations(&key(DERIVED_METRIC), 10)
        .unwrap();
    assert_eq!(derived.len(), 1);
    assert_eq!(derived[0].value, 12.0);
    assert_eq!(derived[0].metric, DERIVED_METRIC);
    // Provenance links the two raw totals and the measured interval.
    let provenance = derived[0].derivation.as_ref().expect("provenance");
    assert_eq!(provenance.interval_start, t0);
    assert_eq!(provenance.interval_end, t1);
    assert_eq!(provenance.inputs.len(), 2);
}

#[test]
fn a_single_catalog_poll_emits_no_growth() {
    let mut engine = engine(origin());
    engine.ingest_observations(vec![raw(1000, origin())]);
    assert!(
        engine
            .store()
            .latest_observations(&key(DERIVED_METRIC), 10)
            .unwrap()
            .is_empty(),
        "no predecessor is not zero growth"
    );
}

#[test]
fn an_unchanged_catalog_is_a_genuine_zero_growth() {
    // KEV frequently goes days without an addition. "Nothing was added" is a
    // real measurement and must be emitted as 0 — distinct from the first poll,
    // which emits nothing at all because there is nothing to compare against.
    let t0 = origin();
    let mut engine = engine(t0);
    engine.ingest_observations(vec![raw(1000, t0)]);
    engine.ingest_observations(vec![raw(1000, t0 + Duration::days(1))]);

    let derived = engine
        .store()
        .latest_observations(&key(DERIVED_METRIC), 10)
        .unwrap();
    assert_eq!(derived.len(), 1);
    assert_eq!(derived[0].value, 0.0);
}
