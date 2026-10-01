//! A signal's identity is the event, not the series.
//!
//! An event is one ongoing change, keyed on its entity (or series) and start
//! time, and its id is stable across cycles. Its *candidates*, though, can come
//! from several series — an event accumulates them as independent sources
//! converge — and the "dominant" series (the most recently observed) can differ
//! from cycle to cycle.
//!
//! The signal engine must therefore derive the signal id from the event, not
//! from the dominant series. Keying it on the series made one ongoing change
//! mint a new signal id whenever the dominant series flipped: the merge in
//! `Engine::ingest_observations` matches by id, so it missed, and the engine
//! stored a *second* signal for the same change while the first froze. That is
//! the "a new signal every minute" failure the design explicitly warns against.
//!
//! These tests drive the real `process_batch` path, not a mock.

use chrono::{DateTime, Utc};
use wse_model::{
    AnomalyCandidate, BaselineSnapshot, CandidateDirection, CandidateKind, DetectionMethod,
    EntityId, ObservationId, SourceId,
};
use wse_signals::event::EventEngine;
use wse_signals::{process_batch, SignalConfig, SignalEngine};

fn at(secs: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
}

fn snap() -> BaselineSnapshot {
    BaselineSnapshot {
        sample_size: 50,
        mean: 100.0,
        median: 100.0,
        std_dev: 1.0,
        mad: 1.0,
        p05: 98.0,
        p95: 102.0,
        ewma: 100.0,
        trend_per_second: 0.0,
        volatility: 1.0,
    }
}

fn candidate(source: &str, series: &str, entity: &str, secs: i64) -> AnomalyCandidate {
    let mut c = AnomalyCandidate::new(
        series,
        ObservationId::new(format!("obs_{series}_{secs}")),
        at(secs),
        snap(),
        104.0,
    );
    c.source_id = SourceId::new(source);
    c.entity_id = Some(EntityId::new(entity));
    c.metric = series.split("::").nth(2).unwrap_or("metric").to_string();
    c.unit = "unit".into();
    c.kind = CandidateKind::Anomaly;
    c.direction = CandidateDirection::Up;
    c.score = 4.0;
    c.confidence = 0.9;
    c.method = DetectionMethod::RobustZScore;
    c
}

fn no_category(_: &str) -> Option<String> {
    None
}

fn engine() -> SignalEngine {
    SignalEngine::new(SignalConfig {
        now_window_seconds: i64::MAX,
        ..SignalConfig::default()
    })
}

/// Two sources fire on the same entity in successive cycles. The event persists
/// (one id), so the signal must too, even though the dominant series flips from
/// `a::…` to `b::…`.
#[test]
fn the_signal_id_survives_the_dominant_series_changing() {
    let mut events = EventEngine::new(Default::default());
    let engine = engine();

    let first = process_batch(
        &mut events,
        &engine,
        &[candidate("src_a", "a::ent_x::m1::u", "ent_x", 0)],
        &no_category,
        at(10),
    );
    let second = process_batch(
        &mut events,
        &engine,
        &[candidate("src_b", "b::ent_x::m2::u", "ent_x", 60)],
        &no_category,
        at(70),
    );

    assert_eq!(
        first[0].event_id, second[0].event_id,
        "the two cycles must describe one ongoing event"
    );
    assert_ne!(
        first[0].series_key, second[0].series_key,
        "the dominant series genuinely changed, which is the trap"
    );
    assert_eq!(
        first[0].id, second[0].id,
        "one ongoing change must keep one signal id across cycles"
    );
}

/// The same convergence shape in a single cycle: three sources, one entity. The
/// event is one; the signal is one; and a later cycle re-forms the same id even
/// when a different source is the most recent.
#[test]
fn a_convergence_signal_keeps_one_id_across_cycles() {
    let mut events = EventEngine::new(Default::default());
    let engine = engine();

    let converge = process_batch(
        &mut events,
        &engine,
        &[
            candidate("src_a", "a::oil::price::usd", "ent_hormuz", 0),
            candidate("src_b", "b::shipping::delay::h", "ent_hormuz", 30),
            candidate("src_c", "c::news::mentions::n", "ent_hormuz", 60),
        ],
        &no_category,
        at(90),
    );
    assert_eq!(converge.len(), 1);

    // Next cycle: only the shipping source fires again. Same event, so the same
    // signal id — even though the dominant series is now the shipping one.
    let next = process_batch(
        &mut events,
        &engine,
        &[candidate(
            "src_b",
            "b::shipping::delay::h",
            "ent_hormuz",
            120,
        )],
        &no_category,
        at(130),
    );
    assert_eq!(next.len(), 1);
    assert_eq!(
        converge[0].id, next[0].id,
        "a converging event must not fork into a second signal"
    );
}
