//! IMPACT is produced, not just declared.
//!
//! The fifth signal type had no producer: `SignalConfig::impact_categories` /
//! `impact_entities` existed and were unit-tested, but no shipped configuration
//! ever declared a scope, so a served engine could never emit `IMPACT`. This
//! test closes that gap end to end against the **real shipped config** and the
//! **real source catalog**:
//!
//! 1. the shipped `config/impact/*.yaml` declares a non-empty scope,
//! 2. a spike on a `finance` source (ECB) produces a signal typed `IMPACT`,
//! 3. the signal's reason names the matched term, so it is checkable,
//! 4. a source outside the scope (EONET, `earth`) is *not* marked `IMPACT`.
//!
//! The failure this guards against is silent: the type would look implemented
//! (it is tested in isolation) while no running engine could ever produce it.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use wse_engine::{Engine, EngineConfig};
use wse_model::{EntityId, Observation, RawReference, Signal, SignalType, SourceId};
use wse_signals::event::EventConfig;
use wse_signals::SignalConfig;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn origin() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).unwrap()
}

/// The shipped impact scope, exactly as the CLI loads it for a served engine.
fn shipped_impact() -> (Vec<String>, Vec<String>) {
    let catalog = wse_config::load_impact(root().join("config/impact")).expect("impact loads");
    assert!(
        catalog.problems.is_empty(),
        "shipped impact scope must load: {:?}",
        catalog.problems
    );
    (catalog.scope.categories, catalog.scope.entities)
}

fn engine() -> Engine {
    let (impact_categories, impact_entities) = shipped_impact();
    Engine::new(EngineConfig {
        detector: wse_detection::DetectorConfig::synthetic(),
        event: EventConfig::default(),
        signal: SignalConfig {
            now_window_seconds: i64::MAX,
            impact_categories,
            impact_entities,
            ..SignalConfig::default()
        },
        convergence: wse_engine::ConvergenceConfig::default(),
        lenses: Vec::new(),
    })
}

/// An observation shaped exactly like the ECB collector's `exchange_rate`.
fn ecb_reading(entity: &str, value: f64, at: DateTime<Utc>) -> Observation {
    Observation::new(
        SourceId::new(wse_sources::ecb::SOURCE_ID),
        Some(EntityId::new(entity)),
        "exchange_rate",
        value,
        "rate",
        at,
        RawReference::new("ecb:exchange_rate", format!("ecb:{value}")),
    )
    .with_received_at(at)
}

/// A quiet baseline of alternating small values, then a spike. The alternation
/// gives the baseline a non-zero MAD, so the spike is a real deviation.
fn drive_spike(engine: &mut Engine, entity: &str) -> Vec<Signal> {
    let mut signals = Vec::new();
    for i in 0..12 {
        let at = origin() + chrono::Duration::seconds(i * 600);
        let value = if i % 2 == 0 { 1.0 } else { 1.1 };
        signals.extend(
            engine
                .ingest_observations(vec![ecb_reading(entity, value, at)])
                .1,
        );
    }
    let at = origin() + chrono::Duration::seconds(12 * 600);
    signals.extend(
        engine
            .ingest_observations(vec![ecb_reading(entity, 5.0, at)])
            .1,
    );
    signals
}

#[test]
fn the_shipped_engine_declares_a_non_empty_impact_scope() {
    let (categories, entities) = shipped_impact();
    assert!(
        !categories.is_empty(),
        "an empty scope makes IMPACT unreachable; the shipped engine must declare one"
    );
    assert!(
        categories.iter().any(|c| c == "finance"),
        "the shipped scope must cover finance, or the ECB producer cannot fire: {categories:?}"
    );
    assert!(!entities.is_empty(), "entities: {entities:?}");
}

#[test]
fn a_finance_spike_produces_an_impact_signal() {
    let mut engine = engine();
    engine
        .register_source(wse_sources::ecb::source())
        .expect("register the shipped ECB source");

    let signals = drive_spike(&mut engine, "fx_usd_eur");

    let impact = signals
        .iter()
        .find(|s| s.has_type(SignalType::Impact))
        .unwrap_or_else(|| {
            panic!(
                "a finance spike must be IMPACT; got types {:?}",
                signals.iter().map(|s| &s.types).collect::<Vec<_>>()
            )
        });

    // The reason names the term, so a reader can check why — not just that it
    // was "important".
    assert!(
        impact
            .reasons
            .iter()
            .any(|r| r.contains("impact scope") && r.contains("finance")),
        "the impact reason must name the matched term: {:?}",
        impact.reasons
    );
    // Impact is additive: the same change is still an ANOMALY.
    assert!(
        impact.has_type(SignalType::Anomaly),
        "IMPACT must not replace the detection types: {:?}",
        impact.types
    );
    // The category came from the source catalog, not a rewrite.
    assert_eq!(impact.categories, vec!["finance".to_string()]);
}

#[test]
fn a_source_outside_the_scope_is_not_impact() {
    // The negative control. `earth` is a real category the engine detects, but
    // the shipped scope does not name it, so a spike there must produce a
    // signal that is *not* IMPACT. Without this, `has_impact` returning `true`
    // unconditionally would still pass the positive tests.
    let (categories, entities) = shipped_impact();
    assert!(
        !categories.iter().any(|c| c == "earth"),
        "the control is only valid if earth is out of scope: {categories:?}"
    );
    assert!(
        !entities
            .iter()
            .any(|e| e.eq_ignore_ascii_case("natural_wildfires")),
        "the control entity must be out of scope: {entities:?}"
    );

    let mut engine = engine();
    engine
        .register_source(
            wse_model::Source::new(SourceId::new("earth_probe"), "Earth Probe", "test")
                .with_category("earth"),
        )
        .expect("register probe");

    let mut signals = Vec::new();
    for i in 0..12 {
        let at = origin() + chrono::Duration::seconds(i * 600);
        let value = if i % 2 == 0 { 2.0 } else { 3.0 };
        let mut o = Observation::new(
            SourceId::new("earth_probe"),
            Some(EntityId::new("natural_wildfires")),
            "open_events",
            value,
            "count",
            at,
            RawReference::new("probe:open_events", format!("probe:{value}")),
        );
        o.received_at = at;
        signals.extend(engine.ingest_observations(vec![o]).1);
    }
    let at = origin() + chrono::Duration::seconds(12 * 600);
    let mut o = Observation::new(
        SourceId::new("earth_probe"),
        Some(EntityId::new("natural_wildfires")),
        "open_events",
        40.0,
        "count",
        at,
        RawReference::new("probe:open_events", "probe:40.0"),
    );
    o.received_at = at;
    signals.extend(engine.ingest_observations(vec![o]).1);

    assert!(
        signals.iter().any(|s| s.has_type(SignalType::Anomaly)),
        "the out-of-scope spike must still be detected: {:?}",
        signals.iter().map(|s| &s.types).collect::<Vec<_>>()
    );
    assert!(
        signals.iter().all(|s| !s.has_type(SignalType::Impact)),
        "an out-of-scope change must not be IMPACT: {:?}",
        signals
            .iter()
            .map(|s| (&s.types, &s.categories))
            .collect::<Vec<_>>()
    );
}

#[test]
fn an_entity_in_scope_is_impact_even_without_a_listed_category() {
    // The shipped scope names the Hormuz entity. A source with an unlisted
    // category but an in-scope entity must still be IMPACT — the entity path.
    let (categories, entities) = shipped_impact();
    assert!(
        entities.iter().any(|e| e.eq_ignore_ascii_case("hormuz")),
        "the shipped scope must name Hormuz for the entity path: {entities:?}"
    );

    let mut engine = engine();
    // A source category deliberately outside the scope.
    engine
        .register_source(
            wse_model::Source::new(SourceId::new("shipping_probe"), "Shipping Probe", "test")
                .with_category("transport"),
        )
        .expect("register probe");

    let mut signals = Vec::new();
    for i in 0..12 {
        let at = origin() + chrono::Duration::seconds(i * 600);
        let value = if i % 2 == 0 { 1.0 } else { 1.1 };
        let mut o = Observation::new(
            SourceId::new("shipping_probe"),
            Some(EntityId::new("region_hormuz")),
            "shipping_delay",
            value,
            "hours",
            at,
            RawReference::new("probe:shipping_delay", format!("probe:{value}")),
        );
        o.received_at = at;
        signals.extend(engine.ingest_observations(vec![o]).1);
    }
    let at = origin() + chrono::Duration::seconds(12 * 600);
    let mut o = Observation::new(
        SourceId::new("shipping_probe"),
        Some(EntityId::new("region_hormuz")),
        "shipping_delay",
        9.0,
        "hours",
        at,
        RawReference::new("probe:shipping_delay", "probe:9.0"),
    );
    o.received_at = at;
    signals.extend(engine.ingest_observations(vec![o]).1);

    let impact = signals
        .iter()
        .find(|s| s.has_type(SignalType::Impact))
        .unwrap_or_else(|| {
            panic!(
                "an in-scope entity must be IMPACT; got types {:?}",
                signals.iter().map(|s| &s.types).collect::<Vec<_>>()
            )
        });
    assert!(
        impact
            .reasons
            .iter()
            .any(|r| r.contains("impact scope") && r.contains("entity") && r.contains("Hormuz")),
        "the entity impact reason must name the term: {:?}",
        impact.reasons
    );
    // `transport` is not in the shipped category list; only the entity matched.
    assert!(
        !categories.iter().any(|c| c == "transport"),
        "the control is only valid if transport is out of scope"
    );
}
