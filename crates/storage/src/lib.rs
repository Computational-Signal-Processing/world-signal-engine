//! # wse-storage
//!
//! Storage is expressed as traits so the application is never locked to one
//! database. The MVP ships an in-memory implementation that is good enough to
//! run the whole pipeline and the tests; a persistent backend can be added
//! later without touching the engine.
//!
//! The five stores mirror the domain:
//!
//! * [`ObservationStore`] — the time series.
//! * [`EventStore`] — grouped anomalies.
//! * [`SignalStore`] — what humans read.
//! * [`SourceStore`] — the catalog.
//! * [`BaselineStore`] — cached baseline snapshots.
//! * [`RawStore`] — the raw payloads behind every observation, so drill-down
//!   can actually reach the bytes the source returned.

mod memory;
mod query;
mod raw;
mod store;

pub use memory::InMemoryStore;
pub use query::{ObservationQuery, Page, SignalQuery, TimeRange};
pub use raw::{RawStore, StoredPayload};
pub use store::{
    BaselineStore, EventStore, ObservationStore, SignalStore, SourceStore, StorageError, Store,
};
