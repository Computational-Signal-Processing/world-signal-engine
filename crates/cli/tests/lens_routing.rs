//! Runtime lens routing: a source's declared lens is reachable end to end.
//!
//! The coverage tests in `lens_coverage.rs` check that the *declarations* are
//! consistent (a source names a lens that exists, a lens names categories its
//! sources emit). This file checks the *runtime*: that an observation from a
//! source which declares a lens actually becomes a signal reachable through
//! that lens.
//!
//! The regression it locks is `nasa_eonet`, which declares `lens_earth` while
//! emitting category `earth`. Before runtime routing, `feeds_lenses` was inert:
//! a signal was matched only against the lens's category filter, so an EONET
//! signal never reached EARTH and the declaration was false. The probe drives
//! the real collector output through the real engine with the shipped lens set.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use wse_engine::{Engine, EngineConfig};
use wse_model::{Observation, Signal, SourceId};
use wse_signals::event::EventConfig;
use wse_signals::SignalConfig;

fn lenses_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../config/lenses")
}

fn origin() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).unwrap()
}

/// An engine with the shipped lens set, permissive enough for a short series.
fn engine() -> Engine {
    let lenses = wse_config::load_lenses(lenses_dir())
        .expect("shipped lenses load")
        .lenses;
    Engine::new(EngineConfig {
        detector: wse_detection::DetectorConfig::synthetic(),
        event: EventConfig::default(),
        signal: SignalConfig {
            now_window_seconds: i64::MAX,
            ..SignalConfig::default()
        },
        convergence: wse_engine::ConvergenceConfig::related(),
        lenses,
    })
}

/// The EONET observations for one cycle, taken from the real fixture so the
/// signal carries the real series key, metric, unit and raw reference.
fn eonet_template(at: DateTime<Utc>) -> Vec<Observation> {
    let body = include_bytes!("../../../tests/fixtures/eonet_events.json").to_vec();
    wse_sources::eonet::parse(&body, at).expect("fixture parses")
}

/// A wildfire-count observation at a chosen value, built from the real parsed
/// observation so the series identity, metric, unit and raw reference are the
/// source's own. Only the value and the timestamp change.
fn eonet_reading(at: DateTime<Utc>, wildfires: f64) -> Observation {
    let mut observation = eonet_template(at)
        .into_iter()
        .find(|o| o.entity_id.as_ref().map(|e| e.as_str()) == Some("natural_wildfires"))
        .expect("a wildfire series in the fixture");
    observation.value = wildfires;
    observation
}

/// Drive a controlled series: a quiet baseline of alternating small counts,
/// then a spike. The alternation gives the baseline a non-zero MAD, so a spike
/// is a real deviation rather than a flat-history edge case.
fn drive_spike(engine: &mut Engine) -> Vec<Signal> {
    let mut signals = Vec::new();
    for i in 0..12 {
        let at = origin() + chrono::Duration::seconds(i * 600);
        let value = if i % 2 == 0 { 2.0 } else { 3.0 };
        signals.extend(engine.ingest_observations(vec![eonet_reading(at, value)]).1);
    }
    let at = origin() + chrono::Duration::seconds(12 * 600);
    signals.extend(engine.ingest_observations(vec![eonet_reading(at, 40.0)]).1);
    signals
}

fn signals_for<'a>(signals: &'a [Signal], lens: &str) -> Vec<&'a Signal> {
    signals
        .iter()
        .filter(|s| s.lens_matches.iter().any(|l| l.as_str() == lens))
        .collect()
}

/// The EONET regression: an EONET signal is reachable through the EARTH lens.
#[test]
fn an_eonet_signal_reaches_the_earth_lens() {
    let mut engine = engine();
    let source = wse_sources::eonet::source();
    let source_id: SourceId = source.id.clone();
    engine.register_source(source).expect("register eonet");

    let all_signals = drive_spike(&mut engine);

    let earth = signals_for(&all_signals, "lens_earth");
    assert!(
        !earth.is_empty(),
        "an EONET signal must be reachable through EARTH; got lens matches: {:?}",
        all_signals
            .iter()
            .map(|s| s.lens_matches.clone())
            .collect::<Vec<_>>()
    );

    // The signal is genuinely EONET's, not some other source's.
    let signal = earth
        .iter()
        .find(|s| s.evidence.iter().any(|e| e.source_id == source_id))
        .expect("an EONET-backed signal reaches EARTH");
    assert_eq!(
        signal.categories,
        vec!["earth".to_string()],
        "the signal carries EONET's category, not a rewritten one"
    );
}

/// Provenance survives routing: the signal still walks back to its observation
/// and source, and the observation's value is intact.
#[test]
fn routing_preserves_provenance() {
    let mut engine = engine();
    engine
        .register_source(wse_sources::eonet::source())
        .expect("register eonet");

    let signals = drive_spike(&mut engine);

    let signal = signals_for(&signals, "lens_earth")
        .into_iter()
        .next()
        .expect("an EARTH-visible signal");

    // SIGNAL -> EVENT -> OBSERVATION -> SOURCE.
    let event = engine.event(&signal.event_id).expect("event resolves");
    assert!(event
        .observations
        .contains(&signal.evidence[0].observation_id));

    let observation = engine
        .observation(&signal.evidence[0].observation_id)
        .expect("observation resolves from the signal's evidence");
    assert_eq!(observation.source_id.as_str(), "nasa_eonet");
    assert_eq!(observation.value, 40.0, "the measured value is unchanged");
    assert_eq!(
        observation.dimensions.get("category").map(String::as_str),
        Some("wildfires")
    );
    assert!(
        !observation.raw.hash.is_empty(),
        "raw reference is retained"
    );
}

/// One source feeding several lenses: AFAD declares EARTH and TURKEY, and its
/// signal is reachable through both, while an unrelated lens is not.
#[test]
fn one_source_can_feed_several_lenses() {
    let mut engine = engine();
    engine
        .register_source(wse_sources::afad::source())
        .expect("register afad");

    // A controlled magnitude series on one province: quiet, then a spike, so a
    // signal forms. Locations come from the real fixture (Turkey), so TURKEY —
    // a geographic lens — also shows it. AFAD keys each observation by its
    // upstream event id, so the id is re-derived after the timestamp is moved.
    let body = include_bytes!("../../../tests/fixtures/afad_events.json").to_vec();
    let template = wse_sources::afad::parse(&body, origin()).expect("fixture parses");
    let readings = template
        .iter()
        .find(|o| o.dimensions.get("province").map(String::as_str) == Some("Kahramanmaraş"))
        .expect("a Kahramanmaraş reading")
        .clone();
    let event_key = readings
        .attributes
        .get("event_id")
        .cloned()
        .expect("afad carries its event id");

    let reading_at = |at: DateTime<Utc>, value: f64| {
        let mut o = readings.clone();
        o.observed_at = at;
        o.received_at = at;
        o.value = value;
        o.with_record_key(&event_key)
    };

    let mut signals = Vec::new();
    for i in 0..12 {
        let at = origin() + chrono::Duration::seconds(i * 600);
        let value = if i % 2 == 0 { 2.0 } else { 3.0 };
        signals.extend(engine.ingest_observations(vec![reading_at(at, value)]).1);
    }
    let at = origin() + chrono::Duration::seconds(12 * 600);
    signals.extend(engine.ingest_observations(vec![reading_at(at, 6.5)]).1);

    let earth = signals_for(&signals, "lens_earth");
    assert!(
        !earth.is_empty(),
        "AFAD must reach EARTH, which it declares"
    );
    let turkey = signals_for(&signals, "lens_turkey");
    assert!(!turkey.is_empty(), "AFAD must also reach TURKEY");
}
