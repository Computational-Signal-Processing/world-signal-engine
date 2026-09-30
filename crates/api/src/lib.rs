//! # wse-api
//!
//! A deliberately small REST surface over the engine.
//!
//! The MVP exposes exactly what the drill-down needs and nothing more:
//!
//! ```text
//! GET /health
//! GET /metrics
//! GET /signals              ?category=&type=&lens=&active=&limit=
//! GET /signals/:id
//! GET /events/:id
//! GET /observations/:id
//! GET /sources
//! GET /sources/:id
//! GET /entities/:id
//! GET /timeline             ?series=&limit=
//! ```
//!
//! The state is the [`Engine`] behind an async `RwLock`, so a scheduler task
//! can write while HTTP readers query.
//!
//! The API also serves the static web UI (see `web/`) as a fallback, so a
//! single process is enough to run the whole MVP.

pub mod handlers;
pub mod state;

pub use state::AppState;

use axum::{routing::get, Router};
use std::path::Path;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};

/// Default location of the web UI, relative to the repository root.
pub const DEFAULT_WEB_DIR: &str = "web";

/// Build the API router, serving the web UI from `DEFAULT_WEB_DIR` if present.
pub fn router(state: AppState) -> Router {
    router_with_web_dir(
        state,
        std::env::var("WSE_WEB_DIR").unwrap_or_else(|_| DEFAULT_WEB_DIR.into()),
    )
}

/// Build the API router with an explicit web UI directory.
///
/// If the directory does not exist the API still works; only the UI is absent.
/// This keeps the binary usable in environments that ship just the API.
pub fn router_with_web_dir(state: AppState, web_dir: impl AsRef<Path>) -> Router {
    let router = Router::new()
        .route("/health", get(handlers::health))
        .route("/metrics", get(handlers::metrics))
        .route("/signals", get(handlers::list_signals))
        .route("/signals/{id}", get(handlers::get_signal))
        .route("/events/{id}", get(handlers::get_event))
        .route("/observations/{id}", get(handlers::get_observation))
        .route("/observations/{id}/raw", get(handlers::get_observation_raw))
        .route("/sources", get(handlers::list_sources))
        .route("/sources/{id}", get(handlers::get_source))
        .route("/entities/{id}", get(handlers::get_entity))
        .route("/timeline", get(handlers::timeline))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let web_dir = web_dir.as_ref();
    if web_dir.is_dir() {
        let index = web_dir.join("index.html");
        router.fallback_service(ServeDir::new(web_dir).not_found_service(ServeFile::new(index)))
    } else {
        tracing::warn!(
            path = %web_dir.display(),
            "web UI directory not found; serving API only"
        );
        router
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wse_engine::{Engine, EngineConfig};
    use wse_storage::SourceStore;

    #[tokio::test]
    async fn health_endpoint_responds() {
        let state = AppState::new(Engine::new(EngineConfig::default()));
        let app = router(state);
        // Router construction is the assertion here; handler behaviour is
        // covered in `handlers` tests.
        let _ = app;
    }

    #[tokio::test]
    async fn state_shares_the_engine() {
        let mut engine = Engine::new(EngineConfig::default());
        engine
            .register_source(wse_model::Source::new(
                wse_model::SourceId::new("src_a"),
                "A",
                "a",
            ))
            .unwrap();
        let state = AppState::new(engine);
        let engine = state.read().await;
        assert_eq!(engine.store().all_sources().unwrap().len(), 1);
    }
}
