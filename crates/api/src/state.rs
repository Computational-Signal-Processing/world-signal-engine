//! Shared application state.

use std::sync::Arc;

use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use wse_engine::Engine;
use wse_storage::{InMemoryStore, Store};

/// The engine behind an async reader/writer lock.
///
/// Generic over the storage backend so the same API can be served from the
/// in-memory store (demos, tests) or the persistent SQLite store (deployments)
/// without a second set of handlers. The default keeps `AppState` in existing
/// signatures meaning "the in-memory engine".
///
/// An async lock (rather than `std::sync::RwLock`) is required because
/// collection is awaited while the write guard is held: a blocking guard is
/// not `Send` and could not cross the await point.
pub struct AppState<S: Store = InMemoryStore> {
    engine: Arc<RwLock<Engine<S>>>,
}

/// Cloned by sharing the `Arc`, not by cloning the store.
///
/// A derived `Clone` would demand `S: Clone`, which the SQLite store is not
/// (and should not be — two connections to one file is not a clone). The lock
/// is already behind an `Arc`, so cloning the handle is all axum needs.
impl<S: Store> Clone for AppState<S> {
    fn clone(&self) -> Self {
        Self {
            engine: Arc::clone(&self.engine),
        }
    }
}

impl<S: Store> AppState<S> {
    pub fn new(engine: Engine<S>) -> Self {
        Self {
            engine: Arc::new(RwLock::new(engine)),
        }
    }

    pub async fn read(&self) -> RwLockReadGuard<'_, Engine<S>> {
        self.engine.read().await
    }

    pub async fn write(&self) -> RwLockWriteGuard<'_, Engine<S>> {
        self.engine.write().await
    }
}
