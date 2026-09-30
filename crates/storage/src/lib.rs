//! # wse-storage
//!
//! Storage is expressed as traits so the application is never locked to one
//! database. Two backends implement them:
//!
//! * [`InMemoryStore`] — hash maps. Runs the whole pipeline and the tests with
//!   zero operational cost, which is what the synthetic world needs.
//! * `SqliteStore` (feature `sqlite`) — a single-file database with the raw
//!   payloads on disk, for an engine that has to survive a restart.
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

#[cfg(feature = "sqlite")]
mod sqlite;

pub use memory::InMemoryStore;
pub use query::{ObservationQuery, Page, SignalQuery, TimeRange};
pub use raw::{MemoryRawStore, RawStore, StoredPayload};
#[cfg(feature = "sqlite")]
pub use sqlite::{FilesystemRawStore, SqliteConfig, SqliteStore};
pub use store::{
    BaselineStore, DiskUsage, EventStore, MaintenanceStore, ObservationStore, SignalStore,
    SourceStore, StorageError, Store,
};
