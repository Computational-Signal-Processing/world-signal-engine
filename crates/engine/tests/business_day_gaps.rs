//! F8 — business-day gaps (absence is not zero).
//!
//! The ECB publishes on TARGET business days only: weekends and holidays are
//! **absent**, not zero. The concern recorded in `docs/source-semantic-audit.md`
//! is that the gap reads as staleness and that the series is treated as if the
//! missing days were flat.
//!
//! This drives the real engine and proves the opposite holds:
//!
//! * a normal move across a weekend gap produces no anomaly (detection is
//!   count-based over the window, not a time-elapsed rate);
//! * a genuinely large move is still caught, so the guarantee is not vacuous;
//! * a missing day inserts no observation at all — it is never recorded as a
//!   zero, so "no data" cannot be mistaken for "data = 0".

use std::sync::Arc;

use chrono::{DateTime, Datelike, Duration, Utc};
use wse_collector::CollectionMode;
use wse_detection::DetectorConfig;
use wse_engine::{Engine, EngineConfig};
use wse_model::{EntityId, Observation, RawReference, Source, SourceId};
use wse_scheduler::Clock;
use wse_signals::event::EventConfig;
use wse_signals::SignalConfig;
use wse_storage::ObservationStore;

const SOURCE: &str = "ecb_exchange_rates";

/// A clock pinned to a fixed instant, so a replay is byte-for-byte.
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

fn engine(at: DateTime<Utc>) -> Engine {
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
    let mut engine = Engine::with_clock(config, Arc::new(FixedClock(at)));
    engine
        .register_source(
            Source::new(SourceId::new(SOURCE), "ECB", "ecb_sdmx_series")
                .with_measurement(wse_model::MeasurementSemantics::StableSeries),
        )
        .unwrap();
    engine
}

fn obs(value: f64, at: DateTime<Utc>, i: usize) -> Observation {
    Observation::new(
        SourceId::new(SOURCE),
        Some(EntityId::new("fx_usd_eur")),
        "exchange_rate",
        value,
        "rate",
        at,
        RawReference::new(format!("ecb://{i}"), format!("h{i}")),
    )
}

/// The business days from `start`, each paired with its UTC midnight.
fn business_days(start: DateTime<Utc>, count: usize) -> Vec<DateTime<Utc>> {
    let mut days = Vec::new();
    let mut day = start;
    while days.len() < count {
        if day.weekday().num_days_from_monday() < 5 {
            days.push(day);
        }
        day += Duration::days(1);
    }
    days
}

/// ~50 business days oscillating around 1.14 with a small, repeating wobble.
fn baseline_days(start: DateTime<Utc>) -> Vec<DateTime<Utc>> {
    business_days(start, 50)
}

fn wobble(i: usize) -> f64 {
    ((i % 5) as f64 - 2.0) * 0.001
}

#[test]
fn a_normal_move_across_a_weekend_gap_is_not_anomalous() {
    let start = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
    let days = baseline_days(start);
    let mut engine = engine(start);

    // Feed the history up to the last business day of the week.
    let mut i = 0usize;
    for (n, at) in days.iter().take(49).enumerate() {
        engine.ingest_observations(vec![obs(1.14 + wobble(n), *at, i)]);
        i += 1;
    }

    // The final point lands after the weekend gap (a Monday). Its value is a
    // normal-sized move, +0.002. Detection is count-based, so the three-day
    // wall-clock gap does not inflate the deviation.
    let last = *days.last().unwrap();
    let (_, _, candidates) =
        engine.ingest_observations(vec![obs(1.14 + wobble(49) + 0.002, last, i)]);
    assert_eq!(
        candidates, 0,
        "a normal move across a business-day gap must not read as an anomaly"
    );
}

#[test]
fn a_genuine_move_across_a_gap_is_still_caught() {
    // The same shape, but a move far outside the wobble: the guarantee above
    // must not be vacuous.
    let start = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
    let days = baseline_days(start);
    let mut engine = engine(start);

    let mut i = 0usize;
    for (n, at) in days.iter().take(49).enumerate() {
        engine.ingest_observations(vec![obs(1.14 + wobble(n), *at, i)]);
        i += 1;
    }

    let last = *days.last().unwrap();
    let (_, _, candidates) = engine.ingest_observations(vec![obs(1.40, last, i)]);
    assert!(
        candidates > 0,
        "a real move across a gap must still be detected"
    );
}

#[test]
fn a_missing_business_day_is_absent_not_zero() {
    let start = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
    let days = business_days(start, 10);
    let mut engine = engine(start);

    // Store every business day except one.
    let missing = days[5];
    for (i, at) in days.iter().filter(|d| **d != missing).enumerate() {
        engine.ingest_observations(vec![obs(1.14 + wobble(i), *at, i)]);
    }

    // Nothing is stored for the skipped day — no zero-valued observation exists
    // to be mistaken for "the rate was 0".
    let series = format!("{SOURCE}::fx_usd_eur::exchange_rate::rate");
    let stored = engine.store().latest_observations(&series, 100).unwrap();
    assert_eq!(
        stored.len(),
        9,
        "exactly the fed business days are stored, not one per calendar day"
    );
    assert!(
        stored.iter().all(|o| o.observed_at != missing),
        "the skipped day must have no observation at all"
    );
    assert!(
        stored.iter().all(|o| o.value > 1.0),
        "a missing day must never appear as a zero-valued observation"
    );

    // Non-vacuous: the store *does* record a zero when one is actually
    // supplied, so the absence above is because nothing was emitted for the
    // missing day — not because the store silently drops zeros.
    engine.ingest_observations(vec![obs(0.0, missing, 99)]);
    let stored = engine.store().latest_observations(&series, 100).unwrap();
    assert!(
        stored
            .iter()
            .any(|o| o.observed_at == missing && o.value == 0.0),
        "a genuinely supplied zero is stored, proving absence is a collector concern"
    );
}
