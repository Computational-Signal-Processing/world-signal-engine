//! # wse-engine
//!
//! The pipeline, end to end:
//!
//! ```text
//! collector -> observations -> storage -> baseline -> detection
//!           -> event -> correlation -> signal -> storage
//! ```
//!
//! [`Engine`] owns the per-series detector state, the event engine, the signal
//! engine and the storage backend. It is deliberately synchronous after the
//! async collection step: everything downstream of an observation is CPU-bound
//! and deterministic, which keeps it easy to test and replay.
//!
//! Two rules from the brief are enforced structurally:
//!
//! * **Data absence is not an event.** A failed collection records source
//!   health and returns early; it never feeds zeros into detection.
//! * **Every signal is explainable.** Signals carry evidence and reasons that
//!   trace back to observation ids, and the raw reference is stored alongside.

pub mod backtest;
pub mod metrics;
pub mod runtime;

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use wse_collector::{CollectionResult, Collector, CollectorError};
use wse_detection::{DetectorConfig, SeriesTracker};
use wse_model::{Event, Observation, Signal, Source, SourceHealth, SourceId};
use wse_scheduler::{Clock, LiveClock};
use wse_signals::event::EventEngine;
use wse_signals::SignalEngine;
use wse_storage::InMemoryStore;
use wse_storage::{StorageError, Store};

pub use backtest::{
    run_backtest, truth_from_windows, BacktestDetection, BacktestReport, LabeledEvent,
};
pub use metrics::Metrics;
pub use runtime::{
    Activity, ActivityKind, ControlSnapshot, DiskSummary, RuntimeState, ScheduleView, SourceControl,
};
pub use wse_correlation::{ConvergenceConfig, MergeMode};

/// Everything one pipeline cycle produced, for reporting and tests.
#[derive(Debug, Default)]
pub struct CycleOutcome {
    pub observations_ingested: usize,
    pub observations_new: usize,
    pub candidates: usize,
    pub events: Vec<Event>,
    pub signals: Vec<Signal>,
    pub source_failed: bool,
}

/// Engine configuration. Every stage defaults to its own defaults.
#[derive(Debug, Clone, Default)]
pub struct EngineConfig {
    pub detector: DetectorConfig,
    pub event: wse_signals::event::EventConfig,
    pub signal: wse_signals::SignalConfig,
    /// How convergence decides that independent candidates are about the same
    /// thing. Defaults to exact entity matching; `ConvergenceConfig::related()`
    /// also matches related entity names and geography.
    pub convergence: wse_correlation::ConvergenceConfig,
    /// Lenses that signals are matched against. Empty means no lens filtering,
    /// which is a valid deployment: lenses are views, not storage.
    pub lenses: Vec<wse_model::lens::Lens>,
}

/// The pipeline engine.
///
/// Generic over the storage backend so the same code runs against the in-memory
/// store (synthetic world, tests) and SQLite (a real deployment). The default
/// type parameter keeps `Engine::new(config)` working for the common case.
pub struct Engine<S: Store = InMemoryStore> {
    config: EngineConfig,
    store: S,
    trackers: HashMap<String, SeriesTracker>,
    event_engine: EventEngine,
    signal_engine: SignalEngine,
    metrics: Metrics,
    /// Source of "now" for signal formation and health timestamps. Live by
    /// default; replay swaps in a clock that advances through historical time,
    /// which is what makes a replayed run reproduce byte-for-byte.
    clock: Arc<dyn Clock>,
    /// Operational state (collection switch, source controls, activity). Shared
    /// with the API so the Control screen and the live stream read the same
    /// truth. Not part of detector state: replay must not reproduce it.
    runtime: RuntimeState,
}

impl Engine<InMemoryStore> {
    pub fn new(config: EngineConfig) -> Self {
        Self::build(config, InMemoryStore::new(), Arc::new(LiveClock))
    }

    /// Build an in-memory engine whose sense of "now" comes from `clock`.
    ///
    /// Replay uses this: the clock advances through historical time so the
    /// detector sees the same batches, at the same times, that live collection
    /// produced.
    pub fn with_clock(config: EngineConfig, clock: Arc<dyn Clock>) -> Self {
        Self::build(config, InMemoryStore::new(), clock)
    }
}

impl<S: Store> Engine<S> {
    /// Build an engine over an explicit store.
    pub fn with_store(config: EngineConfig, store: S) -> Self {
        Self::build(config, store, Arc::new(LiveClock))
    }

    /// Build an engine over a persistent store, with an injectable clock.
    pub fn with_store_and_clock(config: EngineConfig, store: S, clock: Arc<dyn Clock>) -> Self {
        Self::build(config, store, clock)
    }

    fn build(config: EngineConfig, store: S, clock: Arc<dyn Clock>) -> Self {
        // Read the source→lens declarations from the catalog. The engine is the
        // only layer that holds both the sources and the signal engine, so this
        // is where the declaration becomes runtime routing. Nothing here names
        // a source or a category: the catalog is the source of truth.
        let source_lenses: Vec<(String, String)> = store
            .all_sources()
            .unwrap_or_default()
            .into_iter()
            .flat_map(|source| {
                source
                    .feeds_lenses
                    .into_iter()
                    .map(move |lens| (source.id.as_str().to_string(), lens))
            })
            .collect();

        Self {
            event_engine: EventEngine::new(config.event.clone()),
            signal_engine: SignalEngine::new(config.signal.clone())
                .with_convergence(config.convergence.clone())
                .with_lenses(config.lenses.clone())
                .with_source_lenses(source_lenses),
            config,
            store,
            trackers: HashMap::new(),
            metrics: Metrics::default(),
            clock,
            runtime: RuntimeState::new(),
        }
    }

    /// The operational state, for the API and the live stream.
    pub fn runtime(&self) -> &RuntimeState {
        &self.runtime
    }

    /// Rebuild detector state from the stored history.
    ///
    /// This is what makes a restart not a cold start. Without it the engine
    /// would come back with empty rolling windows, spend the next N cycles
    /// re-learning "normal", and silently stop being able to detect anything in
    /// the meantime. Baselines are restored from the cache; the recent points of
    /// each series are replayed through the trackers so the windows are warm.
    ///
    /// Returns how many series were rehydrated.
    pub fn rehydrate(&mut self, history_per_series: usize) -> Result<usize, StorageError> {
        let keys = self.store.series_keys()?;
        let mut restored = 0usize;
        for key in &keys {
            // Newest first from the store; the tracker wants oldest first.
            let mut points = self.store.latest_observations(key, history_per_series)?;
            if points.is_empty() {
                continue;
            }
            points.reverse();
            let tracker = self.trackers.entry(key.clone()).or_insert_with(|| {
                SeriesTracker::from_observation(&points[0], self.config.detector.clone())
            });
            for point in points {
                tracker.push(
                    point.observed_at,
                    point.value,
                    point.id.clone(),
                    point.source_id.clone(),
                );
            }
            restored += 1;
        }

        // The event engine's active set is also rebuilt, so a change that was
        // mid-flight before the restart keeps accumulating into the same event
        // instead of starting a second one.
        let events = self.store.all_events()?;
        for event in events {
            if event.state != wse_model::EventState::Resolved {
                self.event_engine.adopt(event);
            }
        }
        Ok(restored)
    }

    /// The current time, as the engine sees it.
    pub fn now(&self) -> DateTime<Utc> {
        self.clock.now()
    }

    /// The retained raw payloads, for the drill-down's final step.
    pub fn raw_store(&self) -> &S {
        &self.store
    }

    /// Retrieve a raw payload by the hash carried in an observation reference.
    pub fn raw_payload(&self, hash: &str) -> Option<wse_storage::StoredPayload> {
        self.store.get(hash).ok().flatten()
    }

    pub fn store(&self) -> &S {
        &self.store
    }

    pub fn store_mut(&mut self) -> &mut S {
        &mut self.store
    }

    pub fn metrics(&self) -> &Metrics {
        &self.metrics
    }

    /// The lenses signals are matched against.
    pub fn lenses(&self) -> &[wse_model::lens::Lens] {
        &self.config.lenses
    }

    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// Register a source in the catalog.
    pub fn register_source(&mut self, source: Source) -> Result<(), StorageError> {
        self.metrics.sources_registered += 1;
        self.store.put_source(source)?;
        // The source catalog is the source of truth for lens routing, so a
        // source registered after construction must take effect too: otherwise
        // a served engine (which registers its catalog after `build`) would
        // route nothing.
        self.refresh_source_lenses();
        Ok(())
    }

    /// Re-read the source→lens declarations from the stored catalog.
    fn refresh_source_lenses(&mut self) {
        let pairs: Vec<(String, String)> = self
            .store
            .all_sources()
            .unwrap_or_default()
            .into_iter()
            .flat_map(|source| {
                source
                    .feeds_lenses
                    .into_iter()
                    .map(move |lens| (source.id.as_str().to_string(), lens))
            })
            .collect();
        self.signal_engine.set_source_lenses(pairs);
    }

    /// Run one collector and push its output through the pipeline.
    ///
    /// A collector failure is recorded as source health and returns a failed
    /// outcome. It is *not* converted into an observation.
    pub async fn run_collector(&mut self, collector: &dyn Collector) -> CycleOutcome {
        let source_id = collector.source_id();
        let started = Utc::now();
        let result = collector.collect().await;
        self.apply_collection(&source_id, started, result)
    }

    /// Apply an already-completed collection run.
    ///
    /// Split out from [`run_collector`](Self::run_collector) so a live driver
    /// can perform the network fetch **without holding the engine lock** and
    /// take the lock only for the fast, in-memory ingest. Otherwise a slow or
    /// timing-out source would block every HTTP reader for the duration.
    pub fn apply_collection(
        &mut self,
        source_id: &SourceId,
        started: DateTime<Utc>,
        result: Result<CollectionResult, CollectorError>,
    ) -> CycleOutcome {
        let mut outcome = CycleOutcome::default();

        match result {
            Ok(result) => {
                self.record_success(source_id, &result, started);
                outcome.observations_ingested = result.observations.len();
                // Retain the raw bytes before anything else looks at the
                // observations: the drill-down's last step must not depend on
                // the source still being reachable later.
                for payload in &result.raw_payloads {
                    if let Err(err) = self
                        .store
                        .put(payload.reference.clone(), payload.body.clone())
                    {
                        tracing::error!(error = %err, "failed to retain raw payload");
                    }
                }
                let (events, signals, candidates) = self.ingest_observations(result.observations);
                outcome.observations_new = self.metrics.observations_new_last_cycle;
                outcome.candidates = candidates;
                outcome.events = events;
                outcome.signals = signals;
                self.publish_cycle(source_id, &outcome);
            }
            Err(err) => {
                self.record_failure(source_id, &err);
                if err.kind() == wse_model::FailureKind::RateLimited {
                    self.metrics.collector_rate_limited_total += 1;
                } else {
                    self.metrics.collector_failure_total += 1;
                }
                outcome.source_failed = true;
                let kind = if err.kind() == wse_model::FailureKind::RateLimited {
                    ActivityKind::SourceRateLimited
                } else {
                    ActivityKind::SourceFailed
                };
                self.runtime.record(
                    Activity::new(kind, format!("{source_id}: {err}"))
                        .for_source(source_id.as_str()),
                );
                tracing::warn!(source = %source_id, error = %err, "collector failed");
            }
        }
        outcome
    }

    /// Turn one cycle's outcome into activity lines and broadcast them.
    ///
    /// Only real changes are published: a cycle that received nothing new, or
    /// that produced no signal, is silence rather than noise. This is what
    /// keeps the live stream meaningful instead of a per-second heartbeat.
    fn publish_cycle(&self, source_id: &SourceId, outcome: &CycleOutcome) {
        if outcome.source_failed {
            return;
        }
        if outcome.observations_new > 0 {
            self.runtime.record(
                Activity::new(
                    ActivityKind::Observation,
                    format!(
                        "{source_id}: {} new observation(s)",
                        outcome.observations_new
                    ),
                )
                .for_source(source_id.as_str()),
            );
        }
        if outcome.candidates > 0 {
            self.runtime.record(
                Activity::new(
                    ActivityKind::Anomaly,
                    format!("{source_id}: {} anomaly candidate(s)", outcome.candidates),
                )
                .for_source(source_id.as_str()),
            );
        }
        for event in &outcome.events {
            self.runtime.record(
                Activity::new(ActivityKind::Event, format!("event: {}", event.title))
                    .for_event(event.id.as_str()),
            );
        }
        for signal in &outcome.signals {
            self.runtime.record(
                Activity::new(ActivityKind::Signal, format!("signal: {}", signal.title))
                    .for_signal(signal.id.as_str()),
            );
        }
    }

    /// Push a batch of observations through detection, events and signals.
    ///
    /// Returns the events and signals produced, plus the number of anomaly
    /// candidates detected.
    pub fn ingest_observations(
        &mut self,
        observations: Vec<Observation>,
    ) -> (Vec<Event>, Vec<Signal>, usize) {
        let mut new_count = 0usize;
        let mut candidates = Vec::new();

        // Which sources emit a quantity that may be read as a world change.
        // A source whose population churns between collections (a top-N
        // search result) is stored for evidence but never detected on: its
        // aggregate is membership churn, not a change in the world. The
        // judgement is declared in the catalog, not hard-coded here.
        let comparable: HashMap<String, bool> = self
            .store
            .all_sources()
            .unwrap_or_default()
            .into_iter()
            .map(|s| (s.id.as_str().to_string(), s.measurement.is_comparable()))
            .collect();

        for observation in observations {
            // De-duplication happens before anything else: the same payload
            // collected twice must not produce a second observation.
            if self
                .store
                .contains_observation(&observation.id)
                .unwrap_or(false)
            {
                self.metrics.observations_duplicate_total += 1;
                continue;
            }

            self.metrics.observations_total += 1;
            new_count += 1;

            let detectable = comparable
                .get(observation.source_id.as_str())
                .copied()
                // An unknown source (a test, a replay of an unregistered feed)
                // is treated as measurable: the catalog is the place to mark a
                // source as unstable, and absence of an entry is not a verdict.
                .unwrap_or(true);

            if detectable {
                let series_key = observation.series_key();
                let tracker = self.trackers.entry(series_key.clone()).or_insert_with(|| {
                    SeriesTracker::from_observation(&observation, self.config.detector.clone())
                });
                tracker.push_observation(&observation);

                candidates.extend(wse_detection::anomaly::detect_anomaly(tracker));
                candidates.extend(wse_detection::early::detect_early_signal(tracker));

                let baseline = tracker.baseline_before_latest();
                let _ = self
                    .store
                    .put_baseline(&series_key, observation.observed_at, baseline);
            }

            if let Err(err) = self.store.put_observation(observation) {
                tracing::error!(error = %err, "failed to store observation");
            }
        }

        self.metrics.observations_new_last_cycle = new_count;
        self.metrics.anomalies_total += candidates.len() as u64;

        if candidates.is_empty() {
            return (Vec::new(), Vec::new(), 0);
        }

        let mut categories: HashMap<String, String> = HashMap::new();
        for source in self.store.all_sources().unwrap_or_default() {
            categories.insert(source.id.as_str().to_string(), source.category);
        }
        let category_of = |source: &str| categories.get(source).cloned();

        // "Now" comes from the engine clock, not the wall clock, so a replayed
        // run forms exactly the signals it formed the first time.
        let now = self.clock.now();
        let events = self.event_engine.ingest(&candidates, &category_of);
        let signals = self.signal_engine.form_signals(&events, &candidates, now);

        for event in &events {
            if let Err(err) = self.store.put_event(event.clone()) {
                tracing::error!(error = %err, "failed to store event");
            }
        }
        // Merge each freshly formed signal into any existing one with the same
        // stable id. Without this, a slow drift would create a new signal every
        // cycle instead of one signal that persists and grows.
        let mut persisted = Vec::with_capacity(signals.len());
        for signal in signals {
            let merged = match self.store.get_signal(&signal.id) {
                Ok(Some(existing)) => merge_signals(existing, signal, now),
                _ => signal,
            };
            if let Err(err) = self.store.put_signal(merged.clone()) {
                tracing::error!(error = %err, "failed to store signal");
            }
            persisted.push(merged);
        }

        self.metrics.events_total += events.len() as u64;
        self.metrics.signals_total += persisted.len() as u64;
        for signal in &persisted {
            for ty in &signal.types {
                *self
                    .metrics
                    .signal_types_total
                    .entry(ty.as_str().to_string())
                    .or_insert(0) += 1;
            }
        }

        (events, persisted, candidates.len())
    }

    /// Advance the lifecycle of active events against a clock.
    pub fn tick_lifecycle(&mut self, now: DateTime<Utc>) -> Vec<Event> {
        let changed = self.event_engine.refresh(now);
        for event in &changed {
            let _ = self.store.put_event(event.clone());
        }
        changed
    }

    /// Mark an active event as stabilized and persist it.
    pub fn stabilize_event(&mut self, event_id: &wse_model::EventId) -> Option<Event> {
        let events = self.event_engine.active_events().to_vec();
        let mut found = None;
        for mut event in events {
            if &event.id == event_id {
                event.stabilize();
                let _ = self.store.put_event(event.clone());
                found = Some(event);
            }
        }
        found
    }

    fn record_success(
        &mut self,
        source_id: &SourceId,
        result: &CollectionResult,
        started: DateTime<Utc>,
    ) {
        self.metrics.collector_success_total += 1;
        let latency = result
            .latency_ms()
            .unwrap_or_else(|| (Utc::now() - started).num_milliseconds().max(0) as u64);
        self.metrics.collector_latency_ms = Some(latency);
        let mut health = self
            .store
            .get_health(source_id)
            .ok()
            .flatten()
            .unwrap_or_else(|| SourceHealth::new(source_id.clone()));
        health.record_success(
            self.clock.now(),
            latency,
            result.records_received,
            result.records_changed,
            result.records_duplicate,
        );
        let _ = self.store.put_health(health);
    }

    fn record_failure(&mut self, source_id: &SourceId, err: &wse_collector::CollectorError) {
        let mut health = self
            .store
            .get_health(source_id)
            .ok()
            .flatten()
            .unwrap_or_else(|| SourceHealth::new(source_id.clone()));
        health.record_failure_with(self.clock.now(), err.kind(), Some(short_error(err)));
        let _ = self.store.put_health(health);
    }

    /// Health for a source, if it has ever run.
    pub fn source_health(&self, source_id: &SourceId) -> Option<SourceHealth> {
        self.store.get_health(source_id).ok().flatten()
    }

    /// Read a signal back out, for the API.
    pub fn signal(&self, id: &wse_model::SignalId) -> Option<Signal> {
        self.store.get_signal(id).ok().flatten()
    }

    /// Read an event back out.
    pub fn event(&self, id: &wse_model::EventId) -> Option<Event> {
        self.store.get_event(id).ok().flatten()
    }

    /// Read an observation back out.
    pub fn observation(&self, id: &wse_model::ObservationId) -> Option<Observation> {
        self.store.get_observation(id).ok().flatten()
    }

    /// All series currently tracked.
    pub fn tracked_series(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.trackers.keys().cloned().collect();
        keys.sort();
        keys
    }
}

/// A short, credential-free description of a collector failure.
///
/// Transport errors carry the response body, which can be long; source health
/// only needs enough to tell the reader what happened.
fn short_error(err: &wse_collector::CollectorError) -> String {
    const MAX: usize = 160;
    let text = err.to_string();
    if text.chars().count() <= MAX {
        return text;
    }
    let truncated: String = text.chars().take(MAX).collect();
    format!("{truncated}…")
}

/// Fold a freshly formed signal into the one already stored under the same id.
///
/// The signal keeps its original `first_seen` and its type set only grows, so
/// "how long has this been going on?" stays answerable. Evidence is unioned by
/// observation id so the trail to raw data never loses a link.
fn merge_signals(mut existing: Signal, fresh: Signal, now: DateTime<Utc>) -> Signal {
    existing.last_updated = existing.last_updated.max(fresh.last_updated);
    existing.duration_seconds = (existing.last_updated - existing.first_seen)
        .num_seconds()
        .max(0);
    existing.summary = fresh.summary;
    existing.reasons = fresh.reasons;
    existing.confidence = fresh.confidence;
    existing.quality = fresh.quality;
    existing.location = fresh.location.clone().or(existing.location);
    for ty in fresh.types {
        existing.add_type(ty);
    }
    for entity in fresh.entities {
        if !existing.entities.contains(&entity) {
            existing.entities.push(entity);
        }
    }
    for category in fresh.categories {
        if !existing.categories.contains(&category) {
            existing.categories.push(category);
        }
    }
    for item in fresh.evidence {
        if !existing
            .evidence
            .iter()
            .any(|e| e.observation_id == item.observation_id)
        {
            existing.evidence.push(item);
        }
    }
    existing.evidence.sort_by_key(|e| e.observed_at);
    // Lens matches are unioned rather than replaced. The union is what a merge
    // means for a *view*: a signal that accumulated more categories over its
    // life can only have gained lenses, and dropping a match would make a
    // `?lens=` query lose a signal it had already returned.
    for lens in fresh.lens_matches {
        if !existing.lens_matches.contains(&lens) {
            existing.lens_matches.push(lens);
        }
    }
    existing.lens_matches.sort();
    // The summary describes the accumulated span, so recompute it after the
    // union rather than keeping the single-cycle version.
    existing.summary = wse_signals::summarize(&existing);
    // Re-derive the human-facing fields from the grown record. The narrative
    // must describe the whole span the signal now covers, and the status must
    // move with it, or a signal that has been running for hours would keep
    // reading as if it had just appeared.
    existing.data_origin = fresh.data_origin;
    wse_presentation::describe(&mut existing);
    existing.title = existing.narrative.headline.clone();
    existing.status = wse_presentation::status_for(
        existing.first_seen,
        existing.last_updated,
        now,
        existing.distinct_sources(),
        existing.duration_seconds,
    );
    existing
}
