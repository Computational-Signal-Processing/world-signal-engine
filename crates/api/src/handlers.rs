//! HTTP handlers.
//!
//! Every handler is a thin read over the engine. Responses are the domain
//! types serialized directly, so the API and the model can never drift apart.

use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use wse_model::{EntityId, EventId, ObservationId, SignalId, SignalType, SourceId};
use wse_storage::{
    EventStore, ObservationQuery, ObservationStore, SignalQuery, SignalStore, SourceStore,
};

use crate::state::AppState;

/// A uniform error body.
#[derive(Debug, Serialize)]
pub struct ApiError {
    pub error: String,
}

/// Convert any handler failure into a JSON response.
fn not_found(what: impl std::fmt::Display) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(ApiError {
            error: format!("{what} not found"),
        }),
    )
        .into_response()
}

fn bad_request(message: impl std::fmt::Display) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(ApiError {
            error: message.to_string(),
        }),
    )
        .into_response()
}

fn internal(message: impl std::fmt::Display) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiError {
            error: message.to_string(),
        }),
    )
        .into_response()
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub sources: usize,
    pub observations: usize,
    pub events: usize,
    pub signals: usize,
}

/// `GET /health`
pub async fn health(State(state): State<AppState>) -> Response {
    let engine = state.read().await;
    let store = engine.store();
    let (sources, observations, events, signals) = (
        store.all_sources().map(|s| s.len()).unwrap_or(0),
        store.observation_count().unwrap_or(0),
        store.event_count().unwrap_or(0),
        store.signal_count().unwrap_or(0),
    );
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        sources,
        observations,
        events,
        signals,
    })
    .into_response()
}

/// `GET /metrics`
pub async fn metrics(State(state): State<AppState>) -> Response {
    let engine = state.read().await;
    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4")],
        engine.metrics().render(),
    )
        .into_response()
}

/// Query parameters accepted by `GET /signals`.
#[derive(Debug, Deserialize)]
pub struct SignalParams {
    pub category: Option<String>,
    pub entity: Option<String>,
    #[serde(rename = "type")]
    pub signal_type: Option<String>,
    pub lens: Option<String>,
    pub active: Option<bool>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

fn parse_signal_type(raw: &str) -> Option<SignalType> {
    match raw.to_ascii_uppercase().as_str() {
        "NOW" => Some(SignalType::Now),
        "ANOMALY" => Some(SignalType::Anomaly),
        "EARLY_SIGNAL" => Some(SignalType::EarlySignal),
        "CONVERGENCE" => Some(SignalType::Convergence),
        "IMPACT" => Some(SignalType::Impact),
        _ => None,
    }
}

/// `GET /signals`
pub async fn list_signals(
    State(state): State<AppState>,
    Query(params): Query<SignalParams>,
) -> Response {
    let signal_type = match params.signal_type.as_deref() {
        Some(raw) => match parse_signal_type(raw) {
            Some(ty) => Some(ty),
            None => return bad_request(format!("unknown signal type: {raw}")),
        },
        None => None,
    };

    let query = SignalQuery {
        category: params.category,
        entity_id: params.entity,
        signal_type,
        lens_id: params.lens,
        active_only: params.active.unwrap_or(false),
        range: None,
        limit: params.limit,
        offset: params.offset,
    };

    let engine = state.read().await;
    match engine.store().query_signals(&query) {
        Ok(page) => Json(page).into_response(),
        Err(err) => internal(err),
    }
}

/// `GET /signals/:id`
pub async fn get_signal(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let engine = state.read().await;
    match engine.signal(&SignalId::new(id.clone())) {
        Some(signal) => Json(signal).into_response(),
        None => not_found(format!("signal {id}")),
    }
}

/// `GET /events/:id`
pub async fn get_event(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let engine = state.read().await;
    match engine.event(&EventId::new(id.clone())) {
        Some(event) => Json(event).into_response(),
        None => not_found(format!("event {id}")),
    }
}

/// `GET /observations/:id`
pub async fn get_observation(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let engine = state.read().await;
    match engine.observation(&ObservationId::new(id.clone())) {
        Some(observation) => Json(observation).into_response(),
        None => not_found(format!("observation {id}")),
    }
}

/// `GET /observations/:id/raw`
///
/// The last step of the drill-down: the bytes the source actually returned.
pub async fn get_observation_raw(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let engine = state.read().await;
    let Some(observation) = engine.observation(&ObservationId::new(id.clone())) else {
        return not_found(format!("observation {id}"));
    };
    let hash = &observation.raw.hash;
    match engine.raw_payload(hash) {
        Some(payload) => {
            let content_type = payload
                .reference
                .content_type
                .clone()
                .unwrap_or_else(|| "application/octet-stream".to_string());
            ([(header::CONTENT_TYPE, content_type)], payload.body.clone()).into_response()
        }
        // The reference is still meaningful even when the body was not
        // retained (for example data ingested before this endpoint existed),
        // so answer with what we know rather than a bare 404.
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "raw payload not retained",
                "observation_id": id,
                "raw": observation.raw,
            })),
        )
            .into_response(),
    }
}

/// `GET /sources`
pub async fn list_sources(State(state): State<AppState>) -> Response {
    let engine = state.read().await;
    match engine.store().all_sources() {
        Ok(sources) => Json(sources).into_response(),
        Err(err) => internal(err),
    }
}

#[derive(Debug, Serialize)]
pub struct SourceDetail {
    pub source: wse_model::Source,
    pub health: Option<wse_model::SourceHealth>,
}

/// `GET /sources/:id`
pub async fn get_source(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let engine = state.read().await;
    let source_id = SourceId::new(id.clone());
    match engine.store().get_source(&source_id) {
        Ok(Some(source)) => Json(SourceDetail {
            health: engine.source_health(&source_id),
            source,
        })
        .into_response(),
        Ok(None) => not_found(format!("source {id}")),
        Err(err) => internal(err),
    }
}

#[derive(Debug, Serialize)]
pub struct EntityDetail {
    pub entity_id: String,
    pub signals: Vec<wse_model::Signal>,
    pub observations: Vec<wse_model::Observation>,
}

/// `GET /entities/:id`
///
/// Entities are not a first-class store in the MVP; this endpoint resolves the
/// entity by searching the signals and observations that reference it.
pub async fn get_entity(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let engine = state.read().await;
    let store = engine.store();
    let signals = match store.query_signals(&SignalQuery {
        entity_id: Some(id.clone()),
        ..SignalQuery::default()
    }) {
        Ok(page) => page.items,
        Err(err) => return internal(err),
    };
    let observations = match store.query_observations(&ObservationQuery {
        entity_id: Some(id.clone()),
        limit: Some(100),
        ..ObservationQuery::default()
    }) {
        Ok(page) => page.items,
        Err(err) => return internal(err),
    };
    if signals.is_empty() && observations.is_empty() {
        return not_found(format!("entity {id}"));
    }
    Json(EntityDetail {
        entity_id: id,
        signals,
        observations,
    })
    .into_response()
}

#[derive(Debug, Deserialize)]
pub struct TimelineParams {
    pub series: String,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct TimelineResponse {
    pub series_key: String,
    pub observations: Vec<wse_model::Observation>,
    pub baseline: Option<wse_model::BaselineSnapshot>,
}

/// `GET /timeline?series=&limit=`
///
/// The raw material for the `NORMAL ────╮ ╰──● NOW` visualisation: the recent
/// points of one series plus the baseline they were compared against.
pub async fn timeline(
    State(state): State<AppState>,
    Query(params): Query<TimelineParams>,
) -> Response {
    let limit = params.limit.unwrap_or(200).min(5_000);
    let engine = state.read().await;
    let store = engine.store();

    let observations = match store.latest_observations(&params.series, limit) {
        Ok(o) => o,
        Err(err) => return internal(err),
    };
    if observations.is_empty() {
        return not_found(format!("series {}", params.series));
    }
    let baseline = wse_storage::BaselineStore::get_baseline(store, &params.series)
        .ok()
        .flatten()
        .map(|(_, snapshot)| snapshot);

    Json(TimelineResponse {
        series_key: params.series,
        observations,
        baseline,
    })
    .into_response()
}

/// Parse an entity id, exposed for tests.
pub fn entity_id(raw: &str) -> EntityId {
    EntityId::new(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_type_parsing_is_case_insensitive() {
        assert_eq!(parse_signal_type("anomaly"), Some(SignalType::Anomaly));
        assert_eq!(
            parse_signal_type("EARLY_SIGNAL"),
            Some(SignalType::EarlySignal)
        );
        assert_eq!(parse_signal_type("nope"), None);
    }

    #[test]
    fn entity_id_is_passthrough() {
        assert_eq!(entity_id("ent_x").as_str(), "ent_x");
    }
}
