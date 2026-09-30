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

pub mod metrics;

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use wse_collector::{CollectionResult, Collector};
use wse_detection::{DetectorConfig, SeriesTracker};
use wse_model::{Event, Observation, Signal, Source, SourceHealth, SourceId};
use wse_signals::event::EventEngine;
use wse_signals::SignalEngine;
use wse_storage::InMemoryStore;
use wse_storage::{
    BaselineStore, EventStore, ObservationStore, RawStore, SignalStore, SourceStore,
};

pub use metrics::Metrics;

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

/// Engine configuration. All three stages default to their own defaults.
#[derive(Debug, Clone, Default)]
pub struct EngineConfig {
    pub detector: DetectorConfig,
    pub event: wse_signals::event::EventConfig,
    pub signal: wse_signals::SignalConfig,
}

/// The pipeline engine.
pub struct Engine {
    config: EngineConfig,
    store: InMemoryStore,
    raw_store: RawStore,
    trackers: HashMap<String, SeriesTracker>,
    event_engine: EventEngine,
    signal_engine: SignalEngine,
    metrics: Metrics,
}

impl Engine {
    pub fn new(config: EngineConfig) -> Self {
        Self {
            event_engine: EventEngine::new(config.event.clone()),
            signal_engine: SignalEngine::new(config.signal.clone()),
            config,
            store: InMemoryStore::new(),
            raw_store: RawStore::new(),
            trackers: HashMap::new(),
            metrics: Metrics::default(),
        }
    }

    /// The retained raw payloads, for the drill-down's final step.
    pub fn raw_store(&self) -> &RawStore {
        &self.raw_store
    }

    /// Retrieve a raw payload by the hash carried in an observation reference.
    pub fn raw_payload(&self, hash: &str) -> Option<&wse_storage::StoredPayload> {
        self.raw_store.get(hash)
    }

    pub fn store(&self) -> &InMemoryStore {
        &self.store
    }

    pub fn store_mut(&mut self) -> &mut InMemoryStore {
        &mut self.store
    }

    pub fn metrics(&self) -> &Metrics {
        &self.metrics
    }

    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// Register a source in the catalog.
    pub fn register_source(&mut self, source: Source) -> Result<(), wse_storage::StorageError> {
        self.metrics.sources_registered += 1;
        self.store.put_source(source)
    }

    /// Run one collector and push its output through the pipeline.
    ///
    /// A collector failure is recorded as source health and returns a failed
    /// outcome. It is *not* converted into an observation.
    pub async fn run_collector(&mut self, collector: &dyn Collector) -> CycleOutcome {
        let source_id = collector.source_id();
        let started = Utc::now();
        let mut outcome = CycleOutcome::default();

        match collector.collect().await {
            Ok(result) => {
                self.record_success(&source_id, &result, started);
                outcome.observations_ingested = result.observations.len();
                // Retain the raw bytes before anything else looks at the
                // observations: the drill-down's last step must not depend on
                // the source still being reachable later.
                for payload in &result.raw_payloads {
                    if let Err(err) = self
                        .raw_store
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
            }
            Err(err) => {
                self.record_failure(&source_id, started);
                self.metrics.collector_failure_total += 1;
                outcome.source_failed = true;
                tracing::warn!(source = %source_id, error = %err, "collector failed");
            }
        }
        outcome
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

            let series_key = observation.series_key();
            let tracker = self.trackers.entry(series_key.clone()).or_insert_with(|| {
                SeriesTracker::from_observation(&observation, self.config.detector.clone())
            });
            tracker.push(
                observation.observed_at,
                observation.value,
                observation.id.clone(),
                observation.source_id.clone(),
            );

            candidates.extend(wse_detection::anomaly::detect_anomaly(tracker));
            candidates.extend(wse_detection::early::detect_early_signal(tracker));

            let baseline = tracker.baseline_before_latest();
            let _ = self
                .store
                .put_baseline(&series_key, observation.observed_at, baseline);

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

        let now = Utc::now();
        let events = self.event_engine.ingest(&candidates, &category_of);
        let groups = wse_correlation::detect_convergence(
            &candidates,
            &wse_correlation::ConvergenceConfig::default(),
        );
        let signals = self
            .signal_engine
            .form_signals(&events, &candidates, &groups, now);

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
                Ok(Some(existing)) => merge_signals(existing, signal),
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
            Utc::now(),
            latency,
            result.records_received,
            result.records_changed,
            result.records_duplicate,
        );
        let _ = self.store.put_health(health);
    }

    fn record_failure(&mut self, source_id: &SourceId, _started: DateTime<Utc>) {
        let mut health = self
            .store
            .get_health(source_id)
            .ok()
            .flatten()
            .unwrap_or_else(|| SourceHealth::new(source_id.clone()));
        health.record_failure(Utc::now());
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

/// Fold a freshly formed signal into the one already stored under the same id.
///
/// The signal keeps its original `first_seen` and its type set only grows, so
/// "how long has this been going on?" stays answerable. Evidence is unioned by
/// observation id so the trail to raw data never loses a link.
fn merge_signals(mut existing: Signal, fresh: Signal) -> Signal {
    existing.last_updated = existing.last_updated.max(fresh.last_updated);
    existing.duration_seconds = (existing.last_updated - existing.first_seen)
        .num_seconds()
        .max(0);
    existing.title = fresh.title;
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
    // The summary describes the accumulated span, so recompute it after the
    // union rather than keeping the single-cycle version.
    existing.summary = wse_signals::summarize(&existing);
    existing
}
