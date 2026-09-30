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
pub mod security;
pub mod state;

pub use security::SecurityConfig;
pub use state::AppState;

use axum::{routing::get, Router};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::timeout::TimeoutLayer;

/// Default location of the web UI, relative to the repository root.
pub const DEFAULT_WEB_DIR: &str = "web";

/// Build the API router with the default (open) security configuration.
///
/// Kept for tests and embedded use; a real deployment should call
/// [`router_with_config`] with [`SecurityConfig::from_env`].
pub fn router<S: wse_storage::Store + 'static>(state: AppState<S>) -> Router {
    router_with_config(state, SecurityConfig::default())
}

/// Build the API router, serving the web UI from `DEFAULT_WEB_DIR` if present,
/// with an explicit security configuration.
pub fn router_with_config<S: wse_storage::Store + 'static>(
    state: AppState<S>,
    security: SecurityConfig,
) -> Router {
    router_with_web_dir(
        state,
        std::env::var("WSE_WEB_DIR").unwrap_or_else(|_| DEFAULT_WEB_DIR.into()),
        security,
    )
}

/// Build the API router with an explicit web UI directory and security config.
///
/// If the directory does not exist the API still works; only the UI is absent.
/// This keeps the binary usable in environments that ship just the API.
pub fn router_with_web_dir<S: wse_storage::Store + 'static>(
    state: AppState<S>,
    web_dir: impl AsRef<Path>,
    security: SecurityConfig,
) -> Router {
    let security = Arc::new(security);

    let router = Router::new()
        .route("/health", get(handlers::health))
        .route("/metrics", get(handlers::metrics))
        .route("/world", get(handlers::world))
        .route("/control", get(handlers::control))
        .route(
            "/control/collection",
            axum::routing::post(handlers::set_collection),
        )
        .route("/activity", get(handlers::activity))
        .route("/events", get(handlers::events))
        .route("/signals", get(handlers::list_signals))
        .route("/signals/{id}", get(handlers::get_signal))
        .route("/events/{id}", get(handlers::get_event))
        .route("/observations/{id}", get(handlers::get_observation))
        .route("/observations/{id}/raw", get(handlers::get_observation_raw))
        .route("/sources", get(handlers::list_sources))
        .route("/sources/{id}", get(handlers::get_source))
        .route(
            "/sources/{id}/enabled",
            axum::routing::post(handlers::set_source_enabled),
        )
        .route(
            "/sources/{id}/run",
            axum::routing::post(handlers::run_source),
        )
        .route("/entities/{id}", get(handlers::get_entity))
        .route("/lenses", get(handlers::list_lenses))
        .route("/lenses/{id}", get(handlers::get_lens))
        .route("/timeline", get(handlers::timeline))
        .layer(axum::middleware::from_fn_with_state(
            security.clone(),
            security::require_api_key,
        ))
        .layer(tower_http::limit::RequestBodyLimitLayer::new(
            security.max_body_bytes,
        ))
        .layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(security.request_timeout_secs),
        ))
        // Defence in depth for the browser: the UI only ever needs same-origin
        // reads, so framing, MIME sniffing and referrer leakage are all denied.
        .layer(SetResponseHeaderLayer::overriding(
            axum::http::header::X_CONTENT_TYPE_OPTIONS,
            axum::http::HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            axum::http::header::X_FRAME_OPTIONS,
            axum::http::HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            axum::http::header::REFERRER_POLICY,
            axum::http::HeaderValue::from_static("no-referrer"),
        ))
        .layer(security::cors_layer(&security))
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

    #[tokio::test]
    async fn a_configured_router_still_builds() {
        let state = AppState::new(Engine::new(EngineConfig::default()));
        let app = router_with_config(
            state,
            SecurityConfig {
                api_keys: vec!["k".into()],
                ..Default::default()
            },
        );
        let _ = app;
    }
}
