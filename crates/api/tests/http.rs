//! HTTP-level integration tests.
//!
//! These drive the real router over a real socket (rather than calling
//! handlers directly) so routing, extractors, status codes and the JSON shape
//! are all exercised. The engine is fed by the synthetic world, so the test is
//! deterministic and needs no network.

use std::net::SocketAddr;
use std::sync::Arc;

use chrono::DateTime;
use serde_json::Value;
use tokio::net::TcpListener;

use wse_api::{router_with_web_dir, AppState};
use wse_collector::synthetic::{SyntheticCollector, SyntheticWorld};
use wse_engine::{Engine, EngineConfig};
use wse_model::Source;

/// Boot an engine on the synthetic acceptance world and serve the API.
///
/// Returns the base URL of a server running in a background task.
async fn serve() -> String {
    serve_with(EngineConfig::default()).await
}

/// The same, but with the repository's shipped lens set loaded.
///
/// The path is resolved from the crate manifest, not the working directory:
/// `cargo test` runs with the crate as CWD, and a relative path would silently
/// find nothing (a missing lens directory is not an error).
async fn serve_with_lenses() -> String {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let lenses = wse_config::load_lenses(root.join("config/lenses"))
        .expect("shipped lenses must load")
        .lenses;
    assert!(!lenses.is_empty(), "the shipped lens set must not be empty");
    serve_with(EngineConfig {
        lenses,
        ..EngineConfig::default()
    })
    .await
}

async fn serve_with(config: EngineConfig) -> String {
    let origin = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
    let mut engine = Engine::new(config);
    engine
        .register_source(Source::new(
            wse_model::SourceId::new("synthetic_sensor"),
            "Synthetic Sensor",
            "synthetic",
        ))
        .unwrap();

    let collector = SyntheticCollector::new(SyntheticWorld::acceptance(origin));
    for _ in 0..205 {
        engine.run_collector(&collector).await;
    }

    let state = AppState::new(engine);
    // Point the router at a directory that does not exist: these tests are
    // about the API, and the UI fallback is covered separately.
    let app = router_with_web_dir(state, "/nonexistent-web-dir-for-tests");
    serve_app(app).await
}

async fn serve_app(app: axum::Router) -> String {
    let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}")
}

async fn get_json(base: &str, path: &str) -> (u16, Value) {
    let response = reqwest::get(format!("{base}{path}"))
        .await
        .expect("request failed");
    let status = response.status().as_u16();
    let body: Value = response.json().await.unwrap_or(Value::Null);
    (status, body)
}

#[tokio::test]
async fn health_reports_a_populated_engine() {
    let base = serve().await;
    let (status, body) = get_json(&base, "/health").await;
    assert_eq!(status, 200);
    assert_eq!(body["status"], "ok");
    assert_eq!(body["sources"], 1);
    assert!(
        body["observations"].as_u64().unwrap() > 0,
        "expected observations: {body}"
    );
}

#[tokio::test]
async fn signals_are_listed_and_then_resolved_individually() {
    let base = serve().await;
    let (status, page) = get_json(&base, "/signals?limit=50").await;
    assert_eq!(status, 200);

    let items = page["items"].as_array().expect("items array");
    assert!(!items.is_empty(), "expected at least one signal: {page}");

    for signal in items {
        let id = signal["id"].as_str().unwrap();
        let (status, detail) = get_json(&base, &format!("/signals/{id}")).await;
        assert_eq!(status, 200, "signal {id} not retrievable");
        assert_eq!(detail["id"], id);
    }
}

#[tokio::test]
async fn signal_type_filter_is_applied() {
    let base = serve().await;
    let (status, page) = get_json(&base, "/signals?type=ANOMALY").await;
    assert_eq!(status, 200);
    for signal in page["items"].as_array().unwrap() {
        let types = signal["types"].as_array().unwrap();
        assert!(
            types.iter().any(|t| t == "ANOMALY"),
            "signal leaked past the type filter: {signal}"
        );
    }

    let (status, _) = get_json(&base, "/signals?type=NOT_A_TYPE").await;
    assert_eq!(status, 400);
}

#[tokio::test]
async fn drill_down_reaches_raw_data() {
    let base = serve().await;
    let (_, page) = get_json(&base, "/signals?limit=1").await;
    let signal = &page["items"][0];

    // SIGNAL -> EVENT
    let event_id = signal["event_id"].as_str().unwrap();
    let (status, event) = get_json(&base, &format!("/events/{event_id}")).await;
    assert_eq!(status, 200);

    // EVENT -> OBSERVATION
    let observation_id = event["observations"][0].as_str().unwrap();
    let (status, observation) = get_json(&base, &format!("/observations/{observation_id}")).await;
    assert_eq!(status, 200);

    // OBSERVATION -> SOURCE
    let source_id = observation["source_id"].as_str().unwrap();
    let (status, source) = get_json(&base, &format!("/sources/{source_id}")).await;
    assert_eq!(status, 200);
    assert_eq!(source["source"]["id"], source_id);

    // OBSERVATION -> RAW DATA
    assert!(
        !observation["raw"]["locator"].as_str().unwrap().is_empty(),
        "raw reference must be present"
    );
    assert!(
        !observation["raw"]["hash"].as_str().unwrap().is_empty(),
        "raw reference must carry a hash"
    );

    // ...and the raw bytes themselves must be retrievable, not just referenced.
    let response = reqwest::get(format!("{base}/observations/{observation_id}/raw"))
        .await
        .expect("raw request failed");
    assert_eq!(response.status(), 200);
    assert!(
        response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .is_some(),
        "raw payload must declare its content type"
    );
    let body = response.text().await.unwrap();
    assert!(
        !body.is_empty(),
        "raw payload body must be the source's actual bytes"
    );
}

#[tokio::test]
async fn raw_payload_of_an_unknown_observation_is_404() {
    let base = serve().await;
    let (status, body) = get_json(&base, "/observations/obs_does_not_exist/raw").await;
    assert_eq!(status, 404);
    assert!(body["error"].as_str().unwrap().contains("observation"));
}

#[tokio::test]
async fn timeline_returns_the_series_and_its_baseline() {
    let base = serve().await;
    let (_, page) = get_json(&base, "/signals?limit=1").await;
    let series = page["items"][0]["series_key"].as_str().unwrap();

    let (status, body) = get_json(&base, &format!("/timeline?series={series}&limit=50")).await;
    assert_eq!(status, 200);
    assert_eq!(body["series_key"], series);
    assert!(!body["observations"].as_array().unwrap().is_empty());
    assert!(
        body["baseline"].is_object(),
        "baseline should be reported: {body}"
    );
}

#[tokio::test]
async fn unknown_ids_return_404_with_a_json_error() {
    let base = serve().await;
    for path in [
        "/signals/sig_missing",
        "/events/evt_missing",
        "/observations/obs_missing",
        "/sources/src_missing",
    ] {
        let (status, body) = get_json(&base, path).await;
        assert_eq!(status, 404, "{path} should be 404");
        assert!(
            body["error"].is_string(),
            "{path} should carry an error body"
        );
    }
}

#[tokio::test]
async fn metrics_are_prometheus_shaped() {
    let base = serve().await;
    let response = reqwest::get(format!("{base}/metrics")).await.unwrap();
    assert_eq!(response.status(), 200);
    let text = response.text().await.unwrap();
    assert!(text.contains("wse_observations_total"), "{text}");
    assert!(text.contains("wse_signals_total"), "{text}");
}

#[tokio::test]
async fn source_health_distinguishes_failure_from_zero_activity() {
    let base = serve().await;
    let (status, detail) = get_json(&base, "/sources/synthetic_sensor").await;
    assert_eq!(status, 200);
    let health = &detail["health"];
    assert!(health.is_object(), "expected health: {detail}");
    assert_eq!(health["status"], "healthy");
    assert!(
        health["records_received"].as_u64().unwrap() > 0,
        "a successful collector must report received records"
    );
}

#[tokio::test]
async fn entities_endpoint_resolves_referencing_signals() {
    let base = serve().await;
    let (_, page) = get_json(&base, "/signals?limit=1").await;
    let entity = page["items"][0]["entities"][0].as_str().unwrap();

    let (status, body) = get_json(&base, &format!("/entities/{entity}")).await;
    assert_eq!(status, 200);
    assert_eq!(body["entity_id"], entity);
    assert!(!body["signals"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn the_engine_never_confuses_absence_with_zero() {
    // A source that has never run has no health record; that is reported as
    // "not found", never as a healthy source reporting zero activity.
    let base = serve().await;
    let (status, _) = get_json(&base, "/sources/never_seen_source").await;
    assert_eq!(status, 404);
}

/// Lenses are listed with their match counts, and an unknown one is 404.
#[tokio::test]
async fn lenses_are_listed_and_resolved_individually() {
    let base = serve_with_lenses().await;

    let (status, body) = get_json(&base, "/lenses").await;
    assert_eq!(status, 200);
    let lenses = body.as_array().expect("an array of lenses");
    assert!(
        lenses.len() >= 9,
        "expected the shipped lens set, got {}",
        lenses.len()
    );

    // WORLD is the empty lens: it shows every signal.
    let world = lenses
        .iter()
        .find(|l| l["id"] == "lens_global")
        .expect("lens_global must be present");
    assert_eq!(world["name"], "WORLD");
    assert!(
        world["matching_signals"].as_u64().unwrap() > 0,
        "WORLD must show the synthetic signals"
    );

    // A lens whose categories no source emits is present and honestly empty.
    let energy = lenses
        .iter()
        .find(|l| l["id"] == "lens_energy")
        .expect("lens_energy must be present");
    assert_eq!(energy["matching_signals"], 0);

    let (status, one) = get_json(&base, "/lenses/lens_global").await;
    assert_eq!(status, 200);
    assert_eq!(one["id"], "lens_global");
    assert_eq!(one["matching_signals"], world["matching_signals"]);

    let (status, _) = get_json(&base, "/lenses/lens_does_not_exist").await;
    assert_eq!(status, 404);
}

/// `?lens=` filters on the matches the engine recorded at formation.
///
/// Without this the filter would read an always-empty field and every lens
/// query would return nothing, which is the bug this asserts against.
#[tokio::test]
async fn the_lens_filter_returns_the_signals_that_lens_shows() {
    let base = serve_with_lenses().await;

    let (status, all) = get_json(&base, "/signals?limit=200").await;
    assert_eq!(status, 200);
    let all = all["items"].as_array().unwrap();
    assert!(!all.is_empty(), "the synthetic world must produce signals");

    let (status, world) = get_json(&base, "/signals?lens=lens_global&limit=200").await;
    assert_eq!(status, 200);
    assert_eq!(
        world["total"].as_u64().unwrap(),
        all.len() as u64,
        "WORLD shows everything, so it must return every signal"
    );

    // Every returned signal must actually name the lens, and the lens must be
    // in the signal's own match list — not merely inferred by the query.
    for signal in world["items"].as_array().unwrap() {
        let matches = signal["lens_matches"].as_array().unwrap();
        assert!(matches.iter().any(|m| m == "lens_global"));
    }

    let (status, energy) = get_json(&base, "/signals?lens=lens_energy&limit=200").await;
    assert_eq!(status, 200);
    assert_eq!(energy["total"], 0, "ENERGY has no source yet");
}

/// The router must also serve the UI when the directory exists.
#[tokio::test]
async fn web_ui_is_served_from_the_configured_directory() {
    let state = AppState::new(Engine::new(EngineConfig::default()));
    let web_dir = Arc::new(std::env::temp_dir().join("wse-web-test"));
    std::fs::create_dir_all(&*web_dir).unwrap();
    std::fs::write(
        web_dir.join("index.html"),
        "<!doctype html><title>WSE</title>",
    )
    .unwrap();

    let app = router_with_web_dir(state, (*web_dir).clone());
    let base = serve_app(app).await;

    let response = reqwest::get(format!("{base}/")).await.unwrap();
    assert_eq!(response.status(), 200);
    let body = response.text().await.unwrap();
    assert!(body.contains("WSE"), "index.html was not served: {body}");
}
