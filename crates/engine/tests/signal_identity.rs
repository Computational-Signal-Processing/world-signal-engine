//! One ongoing change is one stored signal.
//!
//! The signal engine keys a signal on its event, so a change that persists
//! across collection cycles updates the existing signal instead of creating a
//! new one. This drives the real engine (storage + merge, not the signal engine
//! in isolation) and asserts the *stored* result: the failure mode this guards
//! is a second signal row for the same change while the first freezes.
//!
//! The trap is a change whose candidates come from different series on
//! different cycles — the convergence shape. If the signal id were keyed on the
//! series, the id would flip and the merge would miss.

use chrono::{DateTime, Utc};
use wse_detection::DetectorConfig;
use wse_engine::{Engine, EngineConfig};
use wse_model::{Observation, RawReference, Signal, SignalType, Source, SourceId};
use wse_signals::event::EventConfig;
use wse_signals::SignalConfig;
use wse_storage::{SignalQuery, SignalStore};

fn origin() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).unwrap()
}

fn engine() -> Engine {
    Engine::new(EngineConfig {
        detector: DetectorConfig::synthetic(),
        event: EventConfig::default(),
        signal: SignalConfig {
            now_window_seconds: i64::MAX,
            ..SignalConfig::default()
        },
        convergence: wse_engine::ConvergenceConfig::default(),
        lenses: Vec::new(),
    })
}

fn source(id: &str, category: &str) -> Source {
    Source::new(SourceId::new(id), id, "test").with_category(category)
}

/// One observation on a named series, with a controlled value.
fn obs(source: &str, entity: &str, metric: &str, value: f64, at: DateTime<Utc>) -> Observation {
    let mut o = Observation::new(
        SourceId::new(source),
        Some(wse_model::EntityId::new(entity)),
        metric,
        value,
        "unit",
        at,
        RawReference::new(
            format!("{source}:{metric}"),
            format!("{source}:{metric}:{value}"),
        ),
    );
    o.received_at = at;
    o
}

fn stored_signals(engine: &Engine) -> Vec<Signal> {
    engine
        .store()
        .query_signals(&SignalQuery::default())
        .expect("query signals")
        .items
}

/// Two independent sources fire on the same entity in successive cycles. Both
/// cycles describe one event, so the engine must store exactly one signal, and
/// the merged signal must carry evidence from both sources.
#[test]
fn two_cycles_one_entity_store_exactly_one_signal() {
    let mut engine = engine();
    engine
        .register_source(source("src_a", "geophysics"))
        .unwrap();
    engine
        .register_source(source("src_b", "geophysics"))
        .unwrap();

    // A few quiet points per source to establish a baseline (alternating so the
    // MAD is non-zero and a spike is a real deviation).
    for i in 0..12 {
        let at = origin() + chrono::Duration::seconds(i * 600);
        let value = if i % 2 == 0 { 2.0 } else { 3.0 };
        engine.ingest_observations(vec![
            obs("src_a", "ent_shared", "metric_a", value, at),
            obs("src_b", "ent_shared", "metric_b", value, at),
        ]);
    }

    // Cycle 1: source A spikes.
    let t1 = origin() + chrono::Duration::seconds(12 * 600);
    engine.ingest_observations(vec![obs("src_a", "ent_shared", "metric_a", 40.0, t1)]);

    // Cycle 2: source B spikes on the same entity, within the event window.
    let t2 = origin() + chrono::Duration::seconds(13 * 600);
    engine.ingest_observations(vec![obs("src_b", "ent_shared", "metric_b", 40.0, t2)]);

    let signals = stored_signals(&engine);
    assert_eq!(
        signals.len(),
        1,
        "one ongoing change must be one stored signal, not one per series: {:?}",
        signals
            .iter()
            .map(|s| (&s.id, &s.series_key))
            .collect::<Vec<_>>()
    );

    let signal = &signals[0];
    assert!(
        signal.distinct_sources() >= 2,
        "the one signal must carry evidence from both sources, got {}",
        signal.distinct_sources()
    );
}

/// Two sources spiking on the same entity *in the same cycle* is convergence,
/// and it is still a single stored signal.
#[test]
fn same_cycle_convergence_is_one_signal() {
    let mut engine = engine();
    engine
        .register_source(source("src_a", "geophysics"))
        .unwrap();
    engine
        .register_source(source("src_b", "geophysics"))
        .unwrap();

    for i in 0..12 {
        let at = origin() + chrono::Duration::seconds(i * 600);
        let value = if i % 2 == 0 { 2.0 } else { 3.0 };
        engine.ingest_observations(vec![
            obs("src_a", "ent_shared", "metric_a", value, at),
            obs("src_b", "ent_shared", "metric_b", value, at),
        ]);
    }

    // Both sources spike together, on the same entity.
    let t = origin() + chrono::Duration::seconds(12 * 600);
    engine.ingest_observations(vec![
        obs("src_a", "ent_shared", "metric_a", 40.0, t),
        obs("src_b", "ent_shared", "metric_b", 40.0, t),
    ]);

    let signals = stored_signals(&engine);
    assert_eq!(
        signals.len(),
        1,
        "convergence is one signal, not one per source"
    );
    assert!(
        signals[0].has_type(SignalType::Convergence),
        "independent sources on one entity is a CONVERGENCE signal"
    );
    assert!(signals[0].distinct_sources() >= 2);
}
