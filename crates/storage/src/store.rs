//! Store traits: the contract every storage backend must satisfy.

use chrono::{DateTime, Utc};
use thiserror::Error;
use wse_model::{
    BaselineSnapshot, Event, EventId, Observation, ObservationId, Signal, SignalId, Source,
    SourceHealth, SourceId,
};

use crate::query::{ObservationQuery, Page, SignalQuery, TimeRange};

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("storage backend failure: {0}")]
    Backend(String),
    #[error("invalid query: {0}")]
    InvalidQuery(String),
}

/// The time-series store.
pub trait ObservationStore {
    fn put_observation(&mut self, observation: Observation) -> Result<(), StorageError>;

    /// Insert many observations, returning how many were newly stored.
    ///
    /// Duplicates (same deterministic id) are ignored, not errors: re-running a
    /// collector must be safe.
    fn put_observations(&mut self, observations: Vec<Observation>) -> Result<usize, StorageError> {
        let mut inserted = 0;
        for o in observations {
            if !self.contains_observation(&o.id)? {
                self.put_observation(o)?;
                inserted += 1;
            }
        }
        Ok(inserted)
    }

    fn get_observation(&self, id: &ObservationId) -> Result<Option<Observation>, StorageError>;

    fn contains_observation(&self, id: &ObservationId) -> Result<bool, StorageError>;

    fn query_observations(
        &self,
        query: &ObservationQuery,
    ) -> Result<Page<Observation>, StorageError>;

    /// Most recent observations for a series, newest first.
    fn latest_observations(
        &self,
        series_key: &str,
        limit: usize,
    ) -> Result<Vec<Observation>, StorageError>;

    fn observation_count(&self) -> Result<usize, StorageError>;

    /// Distinct series keys known to the store.
    fn series_keys(&self) -> Result<Vec<String>, StorageError>;
}

/// The event store.
pub trait EventStore {
    fn put_event(&mut self, event: Event) -> Result<(), StorageError>;
    fn get_event(&self, id: &EventId) -> Result<Option<Event>, StorageError>;
    fn events_in_range(&self, range: TimeRange) -> Result<Vec<Event>, StorageError>;
    fn all_events(&self) -> Result<Vec<Event>, StorageError>;
    fn event_count(&self) -> Result<usize, StorageError>;
}

/// The signal store.
pub trait SignalStore {
    fn put_signal(&mut self, signal: Signal) -> Result<(), StorageError>;
    fn get_signal(&self, id: &SignalId) -> Result<Option<Signal>, StorageError>;
    fn query_signals(&self, query: &SignalQuery) -> Result<Page<Signal>, StorageError>;
    fn signals_for_event(&self, event_id: &EventId) -> Result<Vec<Signal>, StorageError>;
    fn signal_count(&self) -> Result<usize, StorageError>;
}

/// The source catalog and health store.
pub trait SourceStore {
    fn put_source(&mut self, source: Source) -> Result<(), StorageError>;
    fn get_source(&self, id: &SourceId) -> Result<Option<Source>, StorageError>;
    fn all_sources(&self) -> Result<Vec<Source>, StorageError>;
    fn put_health(&mut self, health: SourceHealth) -> Result<(), StorageError>;
    fn get_health(&self, id: &SourceId) -> Result<Option<SourceHealth>, StorageError>;
}

/// Cached baseline snapshots, keyed by series.
pub trait BaselineStore {
    fn put_baseline(
        &mut self,
        series_key: &str,
        at: DateTime<Utc>,
        snapshot: BaselineSnapshot,
    ) -> Result<(), StorageError>;
    fn get_baseline(
        &self,
        series_key: &str,
    ) -> Result<Option<(DateTime<Utc>, BaselineSnapshot)>, StorageError>;
}

/// Umbrella trait for a complete storage backend.
pub trait Store: ObservationStore + EventStore + SignalStore + SourceStore + BaselineStore {}
impl<T> Store for T where
    T: ObservationStore + EventStore + SignalStore + SourceStore + BaselineStore
{
}
