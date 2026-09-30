//! Shared application state.

use std::sync::Arc;

use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use wse_engine::Engine;

/// The engine behind an async reader/writer lock.
///
/// An async lock (rather than `std::sync::RwLock`) is required because
/// collection is awaited while the write guard is held: a blocking guard is
/// not `Send` and could not cross the await point.
#[derive(Clone)]
pub struct AppState {
    engine: Arc<RwLock<Engine>>,
}

impl AppState {
    pub fn new(engine: Engine) -> Self {
        Self {
            engine: Arc::new(RwLock::new(engine)),
        }
    }

    pub async fn read(&self) -> RwLockReadGuard<'_, Engine> {
        self.engine.read().await
    }

    pub async fn write(&self) -> RwLockWriteGuard<'_, Engine> {
        self.engine.write().await
    }
}
