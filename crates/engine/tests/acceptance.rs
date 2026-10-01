//! The MVP acceptance test from the brief.
//!
//! It drives the *entire* pipeline with no real network dependency:
//!
//! ```text
//! source -> new observation -> normalize -> store -> baseline -> deviation
//!        -> anomaly candidate -> event -> signal -> API -> UI
//! ```
//!
//! and then proves the human can walk back down:
//!
//! ```text
//! SIGNAL -> EVENT -> OBSERVATIONS -> SOURCE -> RAW DATA
//! ```
//!
//! The synthetic world scripts three things that must be found:
//!
//! * a gradual drift that is *not* individually anomalous  -> `EARLY_SIGNAL`
//! * a large spike                                          -> `ANOMALY`
//! * several independent streams moving together            -> `CONVERGENCE`

use chrono::{DateTime, Utc};
use wse_collector::synthetic::{SyntheticCollector, SyntheticWorld};
use wse_detection::DetectorConfig;
use wse_engine::ConvergenceConfig;
use wse_engine::{Engine, EngineConfig};
use wse_model::{SignalType, Source};
use wse_signals::event::EventConfig;
use wse_signals::SignalConfig;
use wse_storage::{ObservationStore, SignalStore, SourceStore};

fn origin() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).unwrap()
}

/// A permissive engine: the synthetic series are short, so the sample minimum
/// is low and the persistence window is zero.
fn engine() -> Engine {
    Engine::new(EngineConfig {
        detector: DetectorConfig::synthetic(),
        event: EventConfig::default(),
        signal: SignalConfig {
            now_window_seconds: i64::MAX,
            ..SignalConfig::default()
        },
        convergence: ConvergenceConfig::related(),
        lenses: Vec::new(),
    })
}

async fn drive(engine: &mut Engine, world: SyntheticWorld, steps: usize) {
    let collector = SyntheticCollector::new(world);
    for _ in 0..steps {
        engine.run_collector(&collector).await;
    }
}

#[tokio::test]
async fn spike_produces_an_anomaly_signal() {
    let mut engine = engine();
    engine
        .register_source(
            Source::new(
                wse_model::SourceId::new("synthetic_sensor"),
                "Synthetic Sensor",
                "synthetic",
            )
            .with_category("geophysics"),
        )
        .unwrap();

    // 205 points: 0..100 normal, 100..120 drift, 120..200 normal, 200 spike.
    drive(&mut engine, SyntheticWorld::acceptance(origin()), 205).await;

    let signals = engine
        .store()
        .query_signals(&wse_storage::SignalQuery::default())
        .unwrap();
    assert!(
        signals
            .items
            .iter()
            .any(|s| s.has_type(SignalType::Anomaly)),
        "a large spike must produce an ANOMALY signal; got {:?}",
        signals.items.iter().map(|s| &s.types).collect::<Vec<_>>()
    );
    // The signal is quantitative, not a verdict.
    let anomaly = signals
        .items
        .iter()
        .find(|s| s.has_type(SignalType::Anomaly))
        .unwrap();
    assert!(anomaly.summary.contains('σ'));
    assert!(!anomaly.reasons.is_empty());
    assert!(!anomaly.evidence.is_empty());
}

#[tokio::test]
async fn drift_produces_an_early_signal() {
    let mut engine = engine();
    drive(&mut engine, SyntheticWorld::acceptance(origin()), 130).await;

    let signals = engine
        .store()
        .query_signals(&wse_storage::SignalQuery::default())
        .unwrap();
    assert!(
        signals
            .items
            .iter()
            .any(|s| s.has_type(SignalType::EarlySignal)),
        "a persistent drift must produce an EARLY_SIGNAL; got {:?}",
        signals.items.iter().map(|s| &s.types).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn aligned_independent_streams_produce_convergence() {
    let mut engine = engine();
    // Three independent sources, same entity, same direction, same window.
    drive(&mut engine, SyntheticWorld::convergence(origin()), 130).await;

    let signals = engine
        .store()
        .query_signals(&wse_storage::SignalQuery::default())
        .unwrap();
    assert!(
        signals
            .items
            .iter()
            .any(|s| s.has_type(SignalType::Convergence)),
        "independent streams moving together must produce CONVERGENCE; got {:?}",
        signals.items.iter().map(|s| &s.types).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn differently_named_entities_for_one_place_converge_when_related_matching_is_on() {
    // Three providers, one place, three different entity slugs. This is what
    // Phase 11 widened matching for; under exact matching it produces nothing.
    let mut related = engine();
    drive(
        &mut related,
        SyntheticWorld::related_entities(origin()),
        130,
    )
    .await;
    let signals = related
        .store()
        .query_signals(&wse_storage::SignalQuery::default())
        .unwrap();
    assert!(
        signals
            .items
            .iter()
            .any(|s| s.has_type(SignalType::Convergence)),
        "related entity names must converge under MergeMode::Related; got {:?}",
        signals.items.iter().map(|s| &s.types).collect::<Vec<_>>()
    );

    // The same world under exact matching: no convergence, because the ids
    // differ. This is the behaviour Phase 11 changed, asserted on both sides.
    let mut exact = Engine::new(EngineConfig {
        detector: DetectorConfig::synthetic(),
        event: EventConfig::default(),
        signal: SignalConfig {
            now_window_seconds: i64::MAX,
            ..SignalConfig::default()
        },
        convergence: ConvergenceConfig::default(),
        lenses: Vec::new(),
    });
    drive(&mut exact, SyntheticWorld::related_entities(origin()), 130).await;
    let exact_signals = exact
        .store()
        .query_signals(&wse_storage::SignalQuery::default())
        .unwrap();
    assert!(
        !exact_signals
            .items
            .iter()
            .any(|s| s.has_type(SignalType::Convergence)),
        "exact matching must not converge differently named entities; got {:?}",
        exact_signals
            .items
            .iter()
            .map(|s| &s.types)
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn independent_sources_at_one_place_converge_geographically() {
    let mut engine = engine();
    // Distinct entities, identical coordinates: only geography can relate them.
    drive(
        &mut engine,
        SyntheticWorld::geographic_convergence(origin()),
        130,
    )
    .await;

    let signals = engine
        .store()
        .query_signals(&wse_storage::SignalQuery::default())
        .unwrap();
    assert!(
        signals
            .items
            .iter()
            .any(|s| s.has_type(SignalType::Convergence)),
        "independent sources at one place must converge geographically; got {:?}",
        signals.items.iter().map(|s| &s.types).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn signal_can_be_traced_back_to_raw_data() {
    let mut engine = engine();
    engine
        .register_source(
            Source::new(
                wse_model::SourceId::new("synthetic_sensor"),
                "Synthetic Sensor",
                "synthetic",
            )
            .with_category("geophysics"),
        )
        .unwrap();
    drive(&mut engine, SyntheticWorld::acceptance(origin()), 205).await;

    let signals = engine
        .store()
        .query_signals(&wse_storage::SignalQuery::default())
        .unwrap();
    let signal = signals
        .items
        .iter()
        .find(|s| s.has_type(SignalType::Anomaly))
        .expect("an anomaly signal");

    // SIGNAL -> EVENT
    let event = engine.event(&signal.event_id).expect("event is stored");
    assert_eq!(event.id, signal.event_id);

    // EVENT -> OBSERVATIONS
    assert!(!event.observations.is_empty());
    let observation = engine
        .observation(&event.observations[0])
        .expect("observation is stored");

    // OBSERVATION -> SOURCE
    let source = engine
        .store()
        .get_source(&observation.source_id)
        .unwrap()
        .expect("source is stored");
    assert_eq!(source.category, "geophysics");

    // OBSERVATION -> RAW DATA
    assert!(!observation.raw.locator.is_empty());
    assert!(!observation.raw.hash.is_empty());
    assert!(observation.raw.locator.starts_with("synthetic://"));
}

#[tokio::test]
async fn replay_is_deterministic() {
    // Two identical runs must produce identical observations: replay and
    // synthetic tests depend on it.
    let mut a = engine();
    let mut b = engine();
    drive(&mut a, SyntheticWorld::acceptance(origin()), 150).await;
    drive(&mut b, SyntheticWorld::acceptance(origin()), 150).await;

    let keys_a = a.store().series_keys().unwrap();
    let keys_b = b.store().series_keys().unwrap();
    assert_eq!(keys_a, keys_b);

    let obs_a = a.store().latest_observations(&keys_a[0], 5).unwrap();
    let obs_b = b.store().latest_observations(&keys_b[0], 5).unwrap();
    let values_a: Vec<f64> = obs_a.iter().map(|o| o.value).collect();
    let values_b: Vec<f64> = obs_b.iter().map(|o| o.value).collect();
    assert_eq!(values_a, values_b);
}

#[tokio::test]
async fn duplicate_collection_does_not_create_duplicate_observations() {
    let mut engine = engine();
    let collector = SyntheticCollector::new(SyntheticWorld::acceptance(origin()));

    // Collect the same logical point twice by replaying an identical world.
    let first = engine.run_collector(&collector).await;
    assert_eq!(first.observations_new, 1);

    // Re-ingesting the same observations is a no-op.
    let obs = engine
        .store()
        .query_observations(&wse_storage::ObservationQuery::default())
        .unwrap();
    let before = obs.total;
    let (_, _, candidates) = engine.ingest_observations(obs.items);
    assert_eq!(candidates, 0);
    assert_eq!(
        engine
            .store()
            .query_observations(&wse_storage::ObservationQuery::default())
            .unwrap()
            .total,
        before
    );
}

#[tokio::test]
async fn metrics_reflect_pipeline_activity() {
    let mut engine = engine();
    drive(&mut engine, SyntheticWorld::acceptance(origin()), 205).await;
    let m = engine.metrics();
    assert_eq!(m.observations_total, 205);
    assert!(m.collector_success_total == 205);
    assert!(m.anomalies_total >= 1);
    assert!(m.signals_total >= 1);
    assert!(!m.render().is_empty());
}

/// A collector failure, classified the same way the live loop does.
///
/// Driving `apply_collection` directly keeps this a pure engine test: no
/// network, no extra collector type, just the classification under test.
#[tokio::test]
async fn a_throttled_source_reads_as_rate_limited_not_down() {
    let mut engine = engine();
    let source_id = wse_model::SourceId::new("src_throttled");
    engine
        .register_source(Source::new(source_id.clone(), "Throttled", "test"))
        .unwrap();

    for _ in 0..3 {
        let outcome = engine.apply_collection(
            &source_id,
            Utc::now(),
            Err(wse_collector::CollectorError::RateLimited(
                "HTTP 429".into(),
            )),
        );
        assert!(outcome.source_failed);
        // The failure must never fabricate an observation.
        assert_eq!(outcome.observations_ingested, 0);
    }

    let health = engine.source_health(&source_id).unwrap();
    assert_eq!(health.status, wse_model::HealthStatus::RateLimited);
    assert_eq!(health.rate_limit_count, 3);
    assert_eq!(engine.metrics().collector_rate_limited_total, 3);
    assert_eq!(engine.metrics().collector_failure_total, 0);
    assert_eq!(engine.metrics().observations_total, 0);
}

#[tokio::test]
async fn a_real_failure_escalates_to_down() {
    let mut engine = engine();
    let source_id = wse_model::SourceId::new("src_broken");
    engine
        .register_source(Source::new(source_id.clone(), "Broken", "test"))
        .unwrap();

    for _ in 0..3 {
        engine.apply_collection(
            &source_id,
            Utc::now(),
            Err(wse_collector::CollectorError::Transport(
                "connection refused".into(),
            )),
        );
    }

    let health = engine.source_health(&source_id).unwrap();
    assert_eq!(health.status, wse_model::HealthStatus::Down);
    assert_eq!(engine.metrics().collector_failure_total, 3);
    assert_eq!(engine.metrics().collector_rate_limited_total, 0);
}

#[tokio::test]
async fn apply_collection_ingests_a_precomputed_result() {
    // The live loop fetches without the lock and applies under it; this is the
    // path it uses, so it must ingest exactly like `run_collector`.
    use wse_collector::Collector as _;
    let mut engine = engine();
    let collector = SyntheticCollector::new(SyntheticWorld::acceptance(origin()));
    let result = collector.collect().await;
    let outcome = engine.apply_collection(&collector.source_id(), Utc::now(), result);
    assert_eq!(outcome.observations_ingested, 1);
    assert_eq!(outcome.observations_new, 1);
    assert!(!outcome.source_failed);
}

/// A source that declares `unstable_population` must be stored for evidence but
/// never detected on.
///
/// The scenario is the worst case for a naive engine: the population itself
/// jumps, so a metric that would look like a 20σ anomaly is really just a
/// different set of members being measured. The engine must record the
/// observations and refuse to form a signal from them.
#[tokio::test]
async fn an_unstable_population_is_stored_but_never_detected_on() {
    use wse_model::MeasurementSemantics;

    let mut churning = engine();
    churning
        .register_source(
            Source::new(
                wse_model::SourceId::new("synthetic_churn"),
                "Churning Search",
                "synthetic",
            )
            .with_category("technology")
            .with_measurement(MeasurementSemantics::UnstablePopulation),
        )
        .unwrap();

    let world =
        SyntheticWorld::new(origin()).with_stream(wse_collector::synthetic::ScriptedStream::new(
            wse_collector::synthetic::SyntheticStream::new("churn", 100.0, 2.0),
            vec![wse_collector::synthetic::Phase::spike(50, 400.0)],
        ));
    drive(&mut churning, world, 55).await;

    // The spike was recorded...
    let observations = churning
        .store()
        .query_observations(&wse_storage::ObservationQuery::default())
        .unwrap();
    assert_eq!(observations.total, 55);

    // ...but produced no signal, because the source is not comparable.
    let signals = churning
        .store()
        .query_signals(&wse_storage::SignalQuery::default())
        .unwrap();
    assert!(
        signals.items.is_empty(),
        "an unstable population must never produce a signal; got {:?}",
        signals.items.iter().map(|s| &s.types).collect::<Vec<_>>()
    );

    // Control: the same scripted spike on a comparable source *does* signal, so
    // the assertion above is about the measurement semantics, not about the
    // spike being too small to notice.
    let mut control = engine();
    control
        .register_source(
            Source::new(
                wse_model::SourceId::new("synthetic_churn"),
                "Churning Search",
                "synthetic",
            )
            .with_category("technology")
            .with_measurement(MeasurementSemantics::StableSeries),
        )
        .unwrap();
    let world =
        SyntheticWorld::new(origin()).with_stream(wse_collector::synthetic::ScriptedStream::new(
            wse_collector::synthetic::SyntheticStream::new("churn", 100.0, 2.0),
            vec![wse_collector::synthetic::Phase::spike(50, 400.0)],
        ));
    drive(&mut control, world, 55).await;
    let control_signals = control
        .store()
        .query_signals(&wse_storage::SignalQuery::default())
        .unwrap();
    assert!(
        !control_signals.items.is_empty(),
        "the control (a stable series with the same spike) must signal"
    );
}
