//! CAP-2A — a cumulative level becomes a derived delta series.
//!
//! arXiv reports a cumulative `preprint_total`, which only ever grows. A level
//! z-score on it is close to meaningless; the quantity whose change matters is
//! the increment. This drives the real engine (storage, derivation, detection,
//! merge — not the evaluator in isolation) and asserts the *stored* result:
//!
//! * the raw level is stored but never detected on;
//! * the derived `preprint_new` series is what detection sees;
//! * a missing predecessor and a counter reset emit nothing;
//! * the derived id is deterministic, so re-ingesting de-duplicates;
//! * the provenance links the derived point to both raw inputs;
//! * the interval is explicit, not inferred from polling time.

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use wse_baseline::Derived;
use wse_collector::CollectionMode;
use wse_detection::DetectorConfig;
use wse_engine::{Engine, EngineConfig};
use wse_model::{
    Derivation, DerivationKind, DerivationProvenance, Observation, ObservationId, RawReference,
    Source, SourceId,
};
use wse_scheduler::Clock;
use wse_signals::event::EventConfig;
use wse_signals::SignalConfig;
use wse_storage::{BaselineStore, ObservationStore};

const SOURCE: &str = "arxiv_submissions";
const RAW: &str = "preprint_total";
const DERIVED: &str = "preprint_new";

fn origin() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).unwrap()
}

/// A clock pinned to a fixed instant, so nothing in the test depends on the
/// wall clock and a replay is byte-for-byte.
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

fn engine(clock_at: DateTime<Utc>) -> Engine {
    let config = EngineConfig {
        detector: DetectorConfig::synthetic(),
        event: EventConfig::default(),
        signal: SignalConfig {
            now_window_seconds: i64::MAX,
            ..SignalConfig::default()
        },
        convergence: wse_engine::ConvergenceConfig::default(),
        lenses: Vec::new(),
    };
    let mut engine = Engine::with_clock(config, Arc::new(FixedClock(clock_at)));
    // The catalog is the source of truth for the derivation and for the
    // detection gate, so the test registers the real arXiv entry.
    engine
        .register_source(
            Source::new(SourceId::new(SOURCE), "arXiv", "arxiv_category_total")
                .with_measurement(wse_model::MeasurementSemantics::StableSeries)
                .deriving(Derivation::delta(RAW, DERIVED)),
        )
        .unwrap();
    engine
}

/// One raw cumulative observation for a category, at a given time.
fn raw(total: f64, at: DateTime<Utc>) -> Observation {
    Observation::new(
        SourceId::new(SOURCE),
        Some(wse_model::EntityId::new("arxiv_cs_ai")),
        RAW,
        total,
        "preprints",
        at,
        RawReference::new(
            "https://export.arxiv.org/api/query?cat:cs.AI",
            format!("h{total}"),
        ),
    )
    .with_received_at(at)
}

fn derived_observations(engine: &Engine) -> Vec<Observation> {
    let key = wse_model::series_key(&SourceId::new(SOURCE), "arxiv_cs_ai", DERIVED, "preprints");
    let mut points = engine
        .store()
        .latest_observations(&key, 100)
        .expect("store read");
    points.reverse();
    points
}

fn raw_observations(engine: &Engine) -> Vec<Observation> {
    let key = wse_model::series_key(&SourceId::new(SOURCE), "arxiv_cs_ai", RAW, "preprints");
    engine
        .store()
        .latest_observations(&key, 100)
        .expect("store read")
}

/// The pure evaluator's own contract, exercised directly for the first four
/// semantics so a failure points at the rule rather than the plumbing.
#[test]
fn delta_rules() {
    // No predecessor.
    assert_eq!(evaluate(None, 100.0), Derived::NoPredecessor);
    // Normal increase.
    assert_eq!(evaluate(Some(100.0), 107.0), Derived::Value(7.0));
    // Zero is a measurement, distinct from a missing predecessor.
    assert_eq!(evaluate(Some(100.0), 100.0), Derived::Value(0.0));
    assert_ne!(evaluate(Some(100.0), 100.0), evaluate(None, 100.0));
    // A reset is not a negative delta.
    assert_eq!(evaluate(Some(107.0), 20.0), Derived::Reset);
}

fn evaluate(previous: Option<f64>, current: f64) -> Derived {
    wse_baseline::evaluate(DerivationKind::Delta, previous, current)
}

#[test]
fn test_1_first_observation_emits_no_derived_point() {
    let mut engine = engine(origin());
    let (_, _, candidates) = engine.ingest_observations(vec![raw(100.0, origin())]);
    assert!(derived_observations(&engine).is_empty());
    assert_eq!(candidates, 0);
    // The raw level is still stored.
    assert_eq!(raw_observations(&engine).len(), 1);
}

#[test]
fn test_2_normal_delta_is_stored_as_its_own_series() {
    let mut engine = engine(origin());
    let t0 = origin();
    let t1 = t0 + Duration::days(1);
    engine.ingest_observations(vec![raw(100.0, t0)]);
    engine.ingest_observations(vec![raw(107.0, t1)]);

    let derived = derived_observations(&engine);
    assert_eq!(derived.len(), 1);
    assert_eq!(derived[0].metric, DERIVED);
    assert_eq!(derived[0].value, 7.0);
    assert!(derived[0].series_key().contains(DERIVED));
}

#[test]
fn test_3_zero_delta_is_emitted_and_distinct_from_missing() {
    let mut engine = engine(origin());
    let t0 = origin();
    let t1 = t0 + Duration::days(1);
    engine.ingest_observations(vec![raw(100.0, t0)]);
    engine.ingest_observations(vec![raw(100.0, t1)]);

    let derived = derived_observations(&engine);
    assert_eq!(derived.len(), 1, "an unchanged total is a real zero");
    assert_eq!(derived[0].value, 0.0);
}

#[test]
fn test_4_reset_emits_no_negative_delta_and_restarts() {
    let mut engine = engine(origin());
    let t0 = origin();
    engine.ingest_observations(vec![raw(100.0, t0)]);
    // Reset: the counter went backwards. No derived point.
    engine.ingest_observations(vec![raw(20.0, t0 + Duration::days(1))]);
    assert!(derived_observations(&engine).is_empty());

    // The next observation establishes the new predecessor: 20 -> 25 is +5.
    engine.ingest_observations(vec![raw(25.0, t0 + Duration::days(2))]);
    let derived = derived_observations(&engine);
    assert_eq!(derived.len(), 1);
    assert_eq!(derived[0].value, 5.0);
}

#[test]
fn test_5_identity_is_deterministic_across_reprocessing() {
    let t0 = origin();
    let t1 = t0 + Duration::days(1);

    let id = |engine: &mut Engine| {
        engine.ingest_observations(vec![raw(100.0, t0)]);
        engine.ingest_observations(vec![raw(107.0, t1)]);
        derived_observations(engine)[0].id.clone()
    };

    let first = id(&mut engine(origin()));
    let second = id(&mut engine(origin()));
    assert_eq!(first, second);
}

#[test]
fn test_6_reingesting_the_same_observations_does_not_duplicate() {
    let mut engine = engine(origin());
    let t0 = origin();
    let t1 = t0 + Duration::days(1);
    engine.ingest_observations(vec![raw(100.0, t0)]);
    engine.ingest_observations(vec![raw(107.0, t1)]);
    assert_eq!(derived_observations(&engine).len(), 1);

    // Re-ingest both raw points: the raw and the derived ids are already seen.
    engine.ingest_observations(vec![raw(100.0, t0)]);
    engine.ingest_observations(vec![raw(107.0, t1)]);
    assert_eq!(derived_observations(&engine).len(), 1);
}

#[test]
fn test_7_provenance_links_both_raw_inputs() {
    let mut engine = engine(origin());
    let t0 = origin();
    let t1 = t0 + Duration::days(1);
    let before = raw(100.0, t0);
    let after = raw(107.0, t1);
    let before_id = before.id.clone();
    let after_id = after.id.clone();
    engine.ingest_observations(vec![before]);
    engine.ingest_observations(vec![after]);

    let derived = derived_observations(&engine);
    let provenance = derived[0]
        .derivation
        .as_ref()
        .expect("a derived point must carry provenance");
    assert_eq!(provenance.kind, DerivationKind::Delta);
    assert_eq!(provenance.inputs, vec![before_id.clone(), after_id.clone()]);
    assert_eq!(provenance.formula, "current - previous");
    assert_eq!(
        provenance,
        &DerivationProvenance {
            kind: DerivationKind::Delta,
            inputs: vec![before_id, after_id],
            interval_start: t0,
            interval_end: t1,
            formula: "current - previous".to_string(),
        }
    );
}

#[test]
fn test_8_interval_is_explicit_and_not_the_polling_time() {
    let mut engine = engine(origin());
    let t0 = origin();
    let t1 = t0 + Duration::days(1);
    engine.ingest_observations(vec![raw(100.0, t0)]);
    engine.ingest_observations(vec![raw(107.0, t1)]);

    let derived = derived_observations(&engine);
    let provenance = derived[0].derivation.as_ref().unwrap();
    assert_eq!(provenance.interval_start, t0);
    assert_eq!(provenance.interval_end, t1);
    assert_eq!(
        derived[0].observed_at, t1,
        "observed_at is the interval end"
    );
    assert_eq!(derived[0].received_at, t1);
}

#[test]
fn test_9_raw_series_is_not_detected_but_derived_is_eligible() {
    let mut engine = engine(origin());
    let t0 = origin();
    // A realistic cumulative series: noisy increments, then a large jump. The
    // raw level rises smoothly and would not look anomalous as a level; the
    // *delta* series is where the spike lives, so detection must attribute the
    // anomaly to `preprint_new`, never to the raw `preprint_total`.
    let increments = [
        10.0, 11.0, 9.0, 12.0, 10.0, 11.0, 9.0, 12.0, 10.0, 11.0, 9.0,
    ];
    let mut total = 100.0;
    engine.ingest_observations(vec![raw(total, t0)]);
    for (i, step) in increments.iter().enumerate() {
        total += step;
        engine.ingest_observations(vec![raw(total, t0 + Duration::days(i as i64 + 1))]);
    }
    // The spike: +200 in one interval.
    total += 200.0;
    engine.ingest_observations(vec![raw(total, t0 + Duration::days(12))]);

    // The derived series was fed to the detector: it has a warm baseline.
    let derived_key =
        wse_model::series_key(&SourceId::new(SOURCE), "arxiv_cs_ai", DERIVED, "preprints");
    assert!(
        engine.store().get_baseline(&derived_key).unwrap().is_some(),
        "the derived series must be detection-tracked"
    );
    // The raw series was not: no tracker means no baseline was ever written.
    let raw_key = wse_model::series_key(&SourceId::new(SOURCE), "arxiv_cs_ai", RAW, "preprints");
    assert!(
        engine.store().get_baseline(&raw_key).unwrap().is_none(),
        "the raw cumulative level must not be detection-tracked"
    );

    let derived = derived_observations(&engine);
    assert_eq!(
        derived.len(),
        increments.len() + 1,
        "one delta per transition after the first observation"
    );
    assert_eq!(derived.last().unwrap().value, 200.0);
}

#[test]
fn test_10_replay_is_deterministic() {
    let t0 = origin();
    let script = [100.0, 107.0, 109.0];

    let run = || {
        let mut engine = engine(t0);
        for (i, value) in script.iter().enumerate() {
            engine.ingest_observations(vec![raw(*value, t0 + Duration::days(i as i64))]);
        }
        derived_observations(&engine)
    };

    let first = run();
    let second = run();
    assert_eq!(first, second);
    // 100 -> 107 -> 109 yields no delta, +7, +2.
    assert_eq!(
        first.iter().map(|o| o.value).collect::<Vec<_>>(),
        vec![7.0, 2.0]
    );
}

#[test]
fn test_11_raw_observations_remain_queryable_for_drill_down() {
    let mut engine = engine(origin());
    let t0 = origin();
    let t1 = t0 + Duration::days(1);
    engine.ingest_observations(vec![raw(100.0, t0)]);
    engine.ingest_observations(vec![raw(107.0, t1)]);

    // Both raw points are stored, so the drill-down can reach them by id.
    let raws = raw_observations(&engine);
    assert_eq!(raws.len(), 2);
    let ids: Vec<ObservationId> = raws.iter().map(|o| o.id.clone()).collect();
    for id in ids {
        assert!(engine.store().contains_observation(&id).unwrap());
    }
}
