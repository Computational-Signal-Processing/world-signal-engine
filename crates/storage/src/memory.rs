//! In-memory storage backend.
//!
//! Deliberately simple: hash maps for lookup, sorted vectors for range
//! queries. It exists so the entire pipeline can run and be tested on a single
//! machine with zero operational cost, which is exactly what the MVP needs.

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Utc};
use wse_model::{
    BaselineSnapshot, Event, EventId, Observation, ObservationId, Signal, SignalId, Source,
    SourceHealth, SourceId,
};

use crate::query::{ObservationQuery, Page, SignalQuery, TimeRange};
use crate::raw::{MemoryRawStore, RawStore, StoredPayload};
use crate::store::{
    BaselineStore, DiskUsage, EventStore, MaintenanceStore, ObservationStore, SignalStore,
    SourceStore, StorageError,
};

/// A complete in-memory implementation of every store.
#[derive(Debug, Default)]
pub struct InMemoryStore {
    observations: HashMap<ObservationId, Observation>,
    /// series key -> observation ids, oldest first.
    series: HashMap<String, Vec<ObservationId>>,
    events: HashMap<EventId, Event>,
    signals: HashMap<SignalId, Signal>,
    sources: HashMap<SourceId, Source>,
    health: HashMap<SourceId, SourceHealth>,
    baselines: HashMap<String, (DateTime<Utc>, BaselineSnapshot)>,
    raw: MemoryRawStore,
}

impl InMemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Observations for a series, ordered oldest first.
    fn series_observations(&self, series_key: &str) -> Vec<Observation> {
        self.series
            .get(series_key)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.observations.get(id).cloned())
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl ObservationStore for InMemoryStore {
    fn put_observation(&mut self, observation: Observation) -> Result<(), StorageError> {
        let key = observation.series_key();
        let id = observation.id.clone();
        self.series.entry(key).or_default().push(id.clone());
        // Keep each series sorted by observation time so range queries and
        // "latest" lookups are correct even if data arrives out of order.
        if let Some(ids) = self.series.get_mut(&observation.series_key()) {
            let observations = &self.observations;
            ids.sort_by_key(|i| {
                observations
                    .get(i)
                    .map(|o| o.observed_at)
                    .unwrap_or_else(Utc::now)
            });
        }
        self.observations.insert(id, observation);
        Ok(())
    }

    fn get_observation(&self, id: &ObservationId) -> Result<Option<Observation>, StorageError> {
        Ok(self.observations.get(id).cloned())
    }

    fn contains_observation(&self, id: &ObservationId) -> Result<bool, StorageError> {
        Ok(self.observations.contains_key(id))
    }

    fn query_observations(
        &self,
        query: &ObservationQuery,
    ) -> Result<Page<Observation>, StorageError> {
        let mut items: Vec<Observation> = self
            .observations
            .values()
            .filter(|o| {
                query
                    .source_id
                    .as_ref()
                    .is_none_or(|s| o.source_id.as_str() == s)
                    && query
                        .entity_id
                        .as_ref()
                        .is_none_or(|e| o.entity_id.as_ref().is_some_and(|id| id.as_str() == e))
                    && query.metric.as_ref().is_none_or(|m| &o.metric == m)
                    && query
                        .series_key
                        .as_ref()
                        .is_none_or(|k| &o.series_key() == k)
                    && query.range.is_none_or(|r| r.contains(o.observed_at))
            })
            .cloned()
            .collect();

        items.sort_by_key(|o| o.observed_at);
        if query.newest_first {
            items.reverse();
        }

        let total = items.len();
        let offset = query.offset.unwrap_or(0);
        let limit = query.limit.unwrap_or(total.max(1));
        let page: Vec<Observation> = items.into_iter().skip(offset).take(limit).collect();
        Ok(Page::new(page, total, limit, offset))
    }

    fn latest_observations(
        &self,
        series_key: &str,
        limit: usize,
    ) -> Result<Vec<Observation>, StorageError> {
        let mut obs = self.series_observations(series_key);
        obs.reverse();
        obs.truncate(limit);
        Ok(obs)
    }

    fn observation_count(&self) -> Result<usize, StorageError> {
        Ok(self.observations.len())
    }

    fn series_keys(&self) -> Result<Vec<String>, StorageError> {
        let mut keys: Vec<String> = self.series.keys().cloned().collect();
        keys.sort();
        Ok(keys)
    }
}

impl EventStore for InMemoryStore {
    fn put_event(&mut self, event: Event) -> Result<(), StorageError> {
        self.events.insert(event.id.clone(), event);
        Ok(())
    }

    fn get_event(&self, id: &EventId) -> Result<Option<Event>, StorageError> {
        Ok(self.events.get(id).cloned())
    }

    fn events_in_range(&self, range: TimeRange) -> Result<Vec<Event>, StorageError> {
        let mut events: Vec<Event> = self
            .events
            .values()
            .filter(|e| e.last_seen >= range.from && e.first_seen < range.to)
            .cloned()
            .collect();
        events.sort_by_key(|e| e.first_seen);
        Ok(events)
    }

    fn all_events(&self) -> Result<Vec<Event>, StorageError> {
        let mut events: Vec<Event> = self.events.values().cloned().collect();
        events.sort_by_key(|e| e.first_seen);
        Ok(events)
    }

    fn event_count(&self) -> Result<usize, StorageError> {
        Ok(self.events.len())
    }
}

impl SignalStore for InMemoryStore {
    fn put_signal(&mut self, signal: Signal) -> Result<(), StorageError> {
        self.signals.insert(signal.id.clone(), signal);
        Ok(())
    }

    fn get_signal(&self, id: &SignalId) -> Result<Option<Signal>, StorageError> {
        Ok(self.signals.get(id).cloned())
    }

    fn query_signals(&self, query: &SignalQuery) -> Result<Page<Signal>, StorageError> {
        let mut items: Vec<Signal> = self
            .signals
            .values()
            .filter(|s| {
                query
                    .category
                    .as_ref()
                    .is_none_or(|c| s.categories.iter().any(|sc| sc.eq_ignore_ascii_case(c)))
                    && query
                        .entity_id
                        .as_ref()
                        .is_none_or(|e| s.entities.iter().any(|se| se.as_str() == e))
                    && query.signal_type.is_none_or(|t| s.types.contains(&t))
                    && query
                        .lens_id
                        .as_ref()
                        .is_none_or(|l| s.lens_matches.iter().any(|lm| lm.as_str() == l))
                    && query.range.is_none_or(|r| r.contains(s.last_updated))
                    // `active_only` means "the event behind this signal has not
                    // been resolved". This was previously declared on the query
                    // and ignored here, so `?active=true` returned everything;
                    // the SQLite backend and this one must agree on the filter.
                    && (!query.active_only
                        || self
                            .events
                            .get(&s.event_id)
                            .is_none_or(|e| e.state != wse_model::EventState::Resolved))
            })
            .cloned()
            .collect();

        items.sort_by(|a, b| {
            b.quality
                .rank()
                .partial_cmp(&a.quality.rank())
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.last_updated.cmp(&a.last_updated))
        });

        let total = items.len();
        let offset = query.offset.unwrap_or(0);
        let limit = query.limit.unwrap_or(total.max(1));
        let page: Vec<Signal> = items.into_iter().skip(offset).take(limit).collect();
        Ok(Page::new(page, total, limit, offset))
    }

    fn signals_for_event(&self, event_id: &EventId) -> Result<Vec<Signal>, StorageError> {
        let mut signals: Vec<Signal> = self
            .signals
            .values()
            .filter(|s| &s.event_id == event_id)
            .cloned()
            .collect();
        signals.sort_by_key(|s| s.first_seen);
        Ok(signals)
    }

    fn signal_count(&self) -> Result<usize, StorageError> {
        Ok(self.signals.len())
    }
}

impl SourceStore for InMemoryStore {
    fn put_source(&mut self, source: Source) -> Result<(), StorageError> {
        self.sources.insert(source.id.clone(), source);
        Ok(())
    }

    fn get_source(&self, id: &SourceId) -> Result<Option<Source>, StorageError> {
        Ok(self.sources.get(id).cloned())
    }

    fn all_sources(&self) -> Result<Vec<Source>, StorageError> {
        let mut sources: Vec<Source> = self.sources.values().cloned().collect();
        sources.sort_by(|a, b| a.priority.cmp(&b.priority).then_with(|| a.id.cmp(&b.id)));
        Ok(sources)
    }

    fn put_health(&mut self, health: SourceHealth) -> Result<(), StorageError> {
        self.health.insert(health.source_id.clone(), health);
        Ok(())
    }

    fn get_health(&self, id: &SourceId) -> Result<Option<SourceHealth>, StorageError> {
        Ok(self.health.get(id).cloned())
    }
}

impl BaselineStore for InMemoryStore {
    fn put_baseline(
        &mut self,
        series_key: &str,
        at: DateTime<Utc>,
        snapshot: BaselineSnapshot,
    ) -> Result<(), StorageError> {
        self.baselines
            .insert(series_key.to_string(), (at, snapshot));
        Ok(())
    }

    fn get_baseline(
        &self,
        series_key: &str,
    ) -> Result<Option<(DateTime<Utc>, BaselineSnapshot)>, StorageError> {
        Ok(self.baselines.get(series_key).cloned())
    }
}

/// Convenience accessor used by the API for the "timeline" view.
impl InMemoryStore {
    /// Count of observations per series, useful for health/metrics endpoints.
    pub fn series_sizes(&self) -> BTreeMap<String, usize> {
        self.series
            .iter()
            .map(|(k, v)| (k.clone(), v.len()))
            .collect()
    }
}

impl RawStore for InMemoryStore {
    fn put(
        &mut self,
        reference: wse_model::RawReference,
        body: Vec<u8>,
    ) -> Result<(), StorageError> {
        self.raw.put(reference, body)
    }

    fn get(&self, hash: &str) -> Result<Option<StoredPayload>, StorageError> {
        self.raw.get(hash)
    }

    fn len(&self) -> usize {
        self.raw.len()
    }

    fn bytes_used(&self) -> u64 {
        self.raw.bytes_used()
    }
}

impl MaintenanceStore for InMemoryStore {
    /// Drop observations older than `cutoff`.
    ///
    /// Present so the retention loop has a uniform interface, and so a test can
    /// exercise retention without opening a database. In-memory data is
    /// discarded at exit, so a deployment would not rely on this.
    fn delete_observations_before(&mut self, cutoff: DateTime<Utc>) -> Result<usize, StorageError> {
        let before = self.observations.len();
        self.observations.retain(|_, o| o.observed_at >= cutoff);
        for ids in self.series.values_mut() {
            ids.retain(|id| self.observations.contains_key(id));
        }
        self.series.retain(|_, ids| !ids.is_empty());
        Ok(before - self.observations.len())
    }

    /// Nothing to prune: in-memory payloads vanish with the process.
    fn prune_raw_to(&mut self, _max_bytes: u64) -> Result<usize, StorageError> {
        Ok(0)
    }

    fn disk_usage(&self) -> Result<DiskUsage, StorageError> {
        // Nothing is on disk. Reporting zero is the honest answer, and it is
        // distinguishable from "the backend failed to measure".
        Ok(DiskUsage::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wse_model::{Evidence, RawReference, SignalQuality, SignalType};

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
    }

    fn obs(series_suffix: &str, secs: i64, value: f64) -> Observation {
        Observation::new(
            SourceId::new("src_a"),
            None,
            format!("metric_{series_suffix}"),
            value,
            "unit",
            at(secs),
            RawReference::new("raw", format!("h{secs}")),
        )
    }

    #[test]
    fn duplicate_observations_are_ignored() {
        let mut store = InMemoryStore::new();
        let o = obs("x", 0, 1.0);
        assert_eq!(
            store.put_observations(vec![o.clone(), o.clone()]).unwrap(),
            1
        );
        assert_eq!(store.observation_count().unwrap(), 1);
    }

    #[test]
    fn series_queries_return_newest_first() {
        let mut store = InMemoryStore::new();
        store
            .put_observations(vec![
                obs("x", 0, 1.0),
                obs("x", 60, 2.0),
                obs("x", 120, 3.0),
            ])
            .unwrap();
        let key = obs("x", 0, 1.0).series_key();
        let latest = store.latest_observations(&key, 2).unwrap();
        assert_eq!(latest.len(), 2);
        assert_eq!(latest[0].value, 3.0);
        assert_eq!(latest[1].value, 2.0);
    }

    #[test]
    fn observation_query_filters_and_pages() {
        let mut store = InMemoryStore::new();
        for i in 0..10 {
            store.put_observation(obs("x", i * 60, i as f64)).unwrap();
        }
        let page = store
            .query_observations(&ObservationQuery::default().with_limit(3))
            .unwrap();
        assert_eq!(page.total, 10);
        assert_eq!(page.items.len(), 3);
        assert_eq!(page.items[0].value, 0.0);
    }

    #[test]
    fn time_range_query_is_respected() {
        let mut store = InMemoryStore::new();
        for i in 0..10 {
            store.put_observation(obs("x", i * 60, i as f64)).unwrap();
        }
        let range = TimeRange::new(at(120), at(300));
        let page = store
            .query_observations(&ObservationQuery::default().in_range(range))
            .unwrap();
        assert_eq!(page.total, 3); // t=120,180,240
    }

    #[test]
    fn signal_query_sorts_by_rank_and_filters_by_type() {
        let mut store = InMemoryStore::new();
        let mut low = Signal::new(EventId::new("evt_1"), at(0));
        low.add_type(SignalType::Now);
        low.quality = SignalQuality::default();
        let mut high = Signal::new(EventId::new("evt_2"), at(0));
        high.add_type(SignalType::Anomaly);
        high.quality.strength = 1.0;
        high.quality.confidence = 1.0;
        store.put_signal(low).unwrap();
        store.put_signal(high).unwrap();

        let all = store.query_signals(&SignalQuery::default()).unwrap();
        assert_eq!(all.total, 2);
        assert_eq!(all.items[0].types, vec![SignalType::Anomaly]);

        let only_now = store
            .query_signals(&SignalQuery::default().with_type(SignalType::Now))
            .unwrap();
        assert_eq!(only_now.total, 1);
    }

    #[test]
    fn event_round_trip_and_range() {
        let mut store = InMemoryStore::new();
        let mut e = Event::new("quake", at(100));
        e.observe(at(200));
        store.put_event(e.clone()).unwrap();
        assert_eq!(store.event_count().unwrap(), 1);
        assert_eq!(store.get_event(&e.id).unwrap().unwrap(), e);
        assert_eq!(
            store
                .events_in_range(TimeRange::new(at(0), at(300)))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            store
                .events_in_range(TimeRange::new(at(400), at(500)))
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn source_catalog_and_health_are_separate() {
        let mut store = InMemoryStore::new();
        let src = Source::new(SourceId::new("src_a"), "A", "a");
        store.put_source(src.clone()).unwrap();
        assert_eq!(store.get_source(&src.id).unwrap().unwrap(), src);
        assert!(store.get_health(&src.id).unwrap().is_none());

        let mut health = SourceHealth::new(src.id.clone());
        health.record_failure(at(0));
        store.put_health(health.clone()).unwrap();
        assert_eq!(store.get_health(&src.id).unwrap().unwrap(), health);
    }

    #[test]
    fn baselines_are_cached_per_series() {
        let mut store = InMemoryStore::new();
        assert!(store.get_baseline("k").unwrap().is_none());
        let snap = BaselineSnapshot {
            sample_size: 10,
            mean: 1.0,
            median: 1.0,
            std_dev: 0.0,
            mad: 0.0,
            p05: 1.0,
            p95: 1.0,
            ewma: 1.0,
            trend_per_second: 0.0,
            volatility: 0.0,
        };
        store.put_baseline("k", at(0), snap.clone()).unwrap();
        let (when, got) = store.get_baseline("k").unwrap().unwrap();
        assert_eq!(when, at(0));
        assert_eq!(got, snap);
    }

    #[test]
    fn signal_evidence_is_preserved_for_drill_down() {
        let mut store = InMemoryStore::new();
        let mut s = Signal::new(EventId::new("evt_1"), at(0));
        s.evidence.push(Evidence {
            source_id: SourceId::new("src_a"),
            observation_id: ObservationId::new("obs_1"),
            metric: "m".into(),
            unit: "u".into(),
            statement: "statement".into(),
            observed_at: at(0),
            value: 1.0,
            deviation_sigma: Some(4.1),
        });
        store.put_signal(s.clone()).unwrap();
        let got = store.get_signal(&s.id).unwrap().unwrap();
        assert_eq!(got.evidence.len(), 1);
        assert_eq!(got.evidence[0].observation_id.as_str(), "obs_1");
    }
}
