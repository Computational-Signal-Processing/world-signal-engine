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
    let app = router_with_web_dir(
        state,
        "/nonexistent-web-dir-for-tests",
        wse_api::SecurityConfig::default(),
    );
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
async fn every_signal_carries_a_human_narrative() {
    let base = serve().await;
    let (status, page) = get_json(&base, "/signals?limit=50").await;
    assert_eq!(status, 200);
    let items = page["items"].as_array().expect("items");
    assert!(!items.is_empty(), "expected signals: {page}");

    for signal in items {
        let narrative = &signal["narrative"];
        assert!(
            narrative.is_object(),
            "signal has no narrative object: {signal}"
        );
        let headline = narrative["headline"].as_str().unwrap_or("");
        assert!(!headline.is_empty(), "empty headline: {signal}");
        // The headline is the human title, and it must not be the old machine
        // form. This is the product-level promise: detection output is language.
        assert!(
            !headline.starts_with("Anomaly:")
                && !headline.starts_with("Early signal:")
                && !headline.contains("_"),
            "headline is machine jargon: {headline}"
        );
        assert_eq!(
            signal["title"].as_str().unwrap(),
            headline,
            "title must equal the narrative headline"
        );
        assert!(
            !narrative["unknowns"].as_array().unwrap().is_empty(),
            "every signal must state its unknowns"
        );
        assert!(
            signal["status"].is_string(),
            "signal has no status: {signal}"
        );
        // This suite runs on the synthetic world, so the origin is reported as
        // such and the narrative says so rather than implying a live feed.
        assert_eq!(signal["data_origin"], "SYNTHETIC");
        assert!(
            narrative["unknowns"]
                .as_array()
                .unwrap()
                .iter()
                .any(|u| u.as_str().unwrap_or("").contains("synthetic")),
            "synthetic data must be disclosed: {narrative}"
        );
    }
}

#[tokio::test]
async fn the_status_filter_is_applied() {
    let base = serve().await;
    let (status, page) = get_json(&base, "/signals?status=confirmed").await;
    assert_eq!(status, 200);
    for signal in page["items"].as_array().unwrap() {
        assert_eq!(signal["status"], "Confirmed", "leaked: {signal}");
    }
    let (status, _) = get_json(&base, "/signals?status=not_a_status").await;
    assert_eq!(status, 400);
}

#[tokio::test]
async fn the_world_summary_answers_what_is_changing() {
    let base = serve().await;
    let (status, world) = get_json(&base, "/world").await;
    assert_eq!(status, 200);
    assert!(
        world["active_signals"].as_u64().unwrap() > 0,
        "world summary should report active signals: {world}"
    );
    assert!(world["observations_total"].as_u64().unwrap() > 0);
    assert_eq!(world["sources_total"], 1);
    // The NOW strip carries the same human signals the feed does.
    let now = world["now"].as_array().expect("now strip");
    assert!(!now.is_empty(), "NOW strip should not be empty: {world}");
    assert!(
        now[0]["narrative"]["headline"].as_str().is_some(),
        "NOW strip entries must be described for people"
    );
    // The type breakdown covers every type, even at zero.
    let by_type = world["by_type"].as_array().unwrap();
    assert_eq!(by_type.len(), 5, "all five signal types must be reported");
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
    // The source is flattened into the view, with its cadence rendered for
    // display next to it.
    assert_eq!(source["id"], source_id);
    assert!(
        source["cadence_label"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "source view must carry a display cadence: {source}"
    );

    // The observation view exposes the series handle and source lag, which are
    // what the Timeline and drill-down screens read.
    assert!(
        observation["series_key"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "observation view must carry a series_key: {observation}"
    );
    assert!(
        observation["lag_ms"].is_i64(),
        "observation view must carry lag_ms: {observation}"
    );

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

    let app = router_with_web_dir(
        state,
        (*web_dir).clone(),
        wse_api::SecurityConfig::default(),
    );
    let base = serve_app(app).await;

    let response = reqwest::get(format!("{base}/")).await.unwrap();
    assert_eq!(response.status(), 200);
    let body = response.text().await.unwrap();
    assert!(body.contains("WSE"), "index.html was not served: {body}");
}

/// A served engine with authentication turned on.
async fn serve_with_keys(keys: Vec<String>) -> String {
    let state = AppState::new(Engine::new(EngineConfig::default()));
    let app = router_with_web_dir(
        state,
        "/nonexistent-web-dir-for-tests",
        wse_api::SecurityConfig {
            api_keys: keys,
            ..wse_api::SecurityConfig::default()
        },
    );
    serve_app(app).await
}

#[tokio::test]
async fn a_configured_key_is_required_for_every_data_route() {
    let base = serve_with_keys(vec!["s3cret".into()]).await;

    // Without a key: rejected.
    let (status, _) = get_json(&base, "/signals").await;
    assert_eq!(status, 401, "an unauthenticated read must not succeed");

    // With a wrong key: rejected. A near-miss must not be treated as a match.
    let wrong = reqwest::Client::new()
        .get(format!("{base}/signals"))
        .bearer_auth("s3cre")
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status().as_u16(), 401);

    // With the right key: served.
    let ok = reqwest::Client::new()
        .get(format!("{base}/signals"))
        .bearer_auth("s3cret")
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status().as_u16(), 200);

    // The `X-API-Key` header is accepted too, for clients that cannot set
    // `Authorization`.
    let via_header = reqwest::Client::new()
        .get(format!("{base}/signals"))
        .header("x-api-key", "s3cret")
        .send()
        .await
        .unwrap();
    assert_eq!(via_header.status().as_u16(), 200);
}

#[tokio::test]
async fn health_stays_open_but_metrics_does_not() {
    let base = serve_with_keys(vec!["s3cret".into()]).await;

    // A load balancer must be able to probe health without credentials.
    let (status, _) = get_json(&base, "/health").await;
    assert_eq!(status, 200);

    // Metrics expose operational detail and stay behind the key.
    let (status, _) = get_json(&base, "/metrics").await;
    assert_eq!(status, 401);

    let ok = reqwest::Client::new()
        .get(format!("{base}/metrics"))
        .bearer_auth("s3cret")
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status().as_u16(), 200);
}

#[tokio::test]
async fn an_unauthenticated_router_emits_no_cors_headers() {
    // The default configuration has no allowed origins. A cross-origin request
    // must not be told it is welcome, or any page could read the API through a
    // visitor's browser.
    let base = serve().await;
    let response = reqwest::Client::new()
        .get(format!("{base}/health"))
        .header("origin", "https://evil.example")
        .send()
        .await
        .unwrap();
    assert!(
        response
            .headers()
            .get("access-control-allow-origin")
            .is_none(),
        "an unconfigured API must not send CORS headers"
    );
}

#[tokio::test]
async fn a_configured_origin_is_the_only_one_allowed() {
    let state = AppState::new(Engine::new(EngineConfig::default()));
    let app = router_with_web_dir(
        state,
        "/nonexistent-web-dir-for-tests",
        wse_api::SecurityConfig {
            cors_origins: vec!["https://app.example".into()],
            ..wse_api::SecurityConfig::default()
        },
    );
    let base = serve_app(app).await;

    let allowed = reqwest::Client::new()
        .get(format!("{base}/health"))
        .header("origin", "https://app.example")
        .send()
        .await
        .unwrap();
    assert_eq!(
        allowed
            .headers()
            .get("access-control-allow-origin")
            .map(|v| v.to_str().unwrap()),
        Some("https://app.example")
    );

    let denied = reqwest::Client::new()
        .get(format!("{base}/health"))
        .header("origin", "https://evil.example")
        .send()
        .await
        .unwrap();
    assert!(denied
        .headers()
        .get("access-control-allow-origin")
        .is_none());
}

#[tokio::test]
async fn the_ui_stays_reachable_when_authentication_is_on() {
    // The key field lives in the UI, so the UI cannot itself be behind the key:
    // a browser would have nowhere to type one. The middleware guards the API
    // routes only, and this is what proves it.
    let web_dir = Arc::new(std::env::temp_dir().join("wse-web-auth-test"));
    std::fs::create_dir_all(&*web_dir).unwrap();
    std::fs::write(
        web_dir.join("index.html"),
        "<!doctype html><title>WSE</title>",
    )
    .unwrap();

    let state = AppState::new(Engine::new(EngineConfig::default()));
    let app = router_with_web_dir(
        state,
        (*web_dir).clone(),
        wse_api::SecurityConfig {
            api_keys: vec!["s3cret".into()],
            ..wse_api::SecurityConfig::default()
        },
    );
    let base = serve_app(app).await;

    let page = reqwest::get(format!("{base}/")).await.unwrap();
    assert_eq!(page.status(), 200, "the UI must load without a key");

    // The data routes behind it still do not.
    let (status, _) = get_json(&base, "/signals").await;
    assert_eq!(status, 401);
}

/* --------------------------------------------------------- control plane */

/// Boot the synthetic world and serve with a key, so the authenticated control
/// routes can be exercised the way a real deployment reaches them.
async fn serve_authenticated(key: &str) -> String {
    let origin = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
    let mut engine = Engine::new(EngineConfig::default());
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
    let app = router_with_web_dir(
        state,
        "/nonexistent-web-dir-for-tests",
        wse_api::SecurityConfig {
            api_keys: vec![key.into()],
            ..wse_api::SecurityConfig::default()
        },
    );
    serve_app(app).await
}

async fn post_json(base: &str, path: &str, key: &str, body: Value) -> (u16, Value) {
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{base}{path}"))
        .bearer_auth(key)
        .json(&body)
        .send()
        .await
        .expect("request failed");
    let status = response.status().as_u16();
    let body: Value = response.json().await.unwrap_or(Value::Null);
    (status, body)
}

async fn get_json_auth(base: &str, path: &str, key: &str) -> (u16, Value) {
    let client = reqwest::Client::new();
    let response = client
        .get(format!("{base}{path}"))
        .bearer_auth(key)
        .send()
        .await
        .expect("request failed");
    let status = response.status().as_u16();
    let body: Value = response.json().await.unwrap_or(Value::Null);
    (status, body)
}

#[tokio::test]
async fn control_snapshot_reports_real_state() {
    let base = serve().await;
    let (status, body) = get_json(&base, "/control").await;
    assert_eq!(status, 200);
    assert_eq!(body["status"], "live");
    assert_eq!(body["collection_enabled"], true);

    let sources = body["sources"].as_array().expect("sources array");
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0]["source_id"], "synthetic_sensor");
    assert_eq!(sources[0]["enabled"], true);

    // The counts are read from the store, not hard-coded.
    assert!(
        body["observations_total"].as_u64().unwrap() > 0,
        "control must report real observation counts: {body}"
    );
    assert!(body["disk"]["total_bytes"].as_u64().is_some());
}

#[tokio::test]
async fn collection_can_be_paused_and_resumed() {
    let base = serve().await;

    let (status, body) = post_json(
        &base,
        "/control/collection",
        "",
        serde_json::json!({"enabled": false}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body["collection_enabled"], false);

    let (_, control) = get_json(&base, "/control").await;
    assert_eq!(control["collection_enabled"], false);

    let (status, _) = post_json(
        &base,
        "/control/collection",
        "",
        serde_json::json!({"enabled": true}),
    )
    .await;
    assert_eq!(status, 200);
    let (_, control) = get_json(&base, "/control").await;
    assert_eq!(control["collection_enabled"], true);
}

#[tokio::test]
async fn a_source_can_be_disabled_then_enabled() {
    let base = serve().await;

    let (status, _) = post_json(
        &base,
        "/sources/synthetic_sensor/enabled",
        "",
        serde_json::json!({"enabled": false}),
    )
    .await;
    assert_eq!(status, 200);

    let (_, control) = get_json(&base, "/control").await;
    assert_eq!(control["sources"][0]["enabled"], false);

    // An unknown source is refused rather than silently accepted.
    let (status, _) = post_json(
        &base,
        "/sources/does_not_exist/enabled",
        "",
        serde_json::json!({"enabled": true}),
    )
    .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn run_now_is_queued_and_deduplicated() {
    let base = serve().await;

    let (status, body) = post_json(
        &base,
        "/sources/synthetic_sensor/run",
        "",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, 202);
    assert_eq!(body["queued"], true);

    // A second request for the same source is coalesced, not queued twice.
    let (status, _) = post_json(
        &base,
        "/sources/synthetic_sensor/run",
        "",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, 202);

    let (status, _) = post_json(&base, "/sources/nope/run", "", serde_json::json!({})).await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn the_activity_stream_reflects_what_happened() {
    let base = serve().await;
    let (status, body) = get_json(&base, "/activity?limit=20").await;
    assert_eq!(status, 200);
    let items = body["items"].as_array().expect("items array");
    assert!(
        !items.is_empty(),
        "expected activity after 205 cycles: {body}"
    );
    // Newest first, and every entry carries a real kind. The kind is the
    // wire contract the web client switches on (icons, filters, and the
    // World screen's live refresh), so it must stay SCREAMING_SNAKE_CASE.
    const KNOWN_KINDS: &[&str] = &[
        "STARTED",
        "OBSERVATION",
        "ANOMALY",
        "EVENT",
        "SIGNAL",
        "SOURCE_RECOVERED",
        "SOURCE_FAILED",
        "SOURCE_RATE_LIMITED",
        "CONTROL",
    ];
    for item in items {
        let kind = item["kind"].as_str().expect("kind is a string");
        assert!(
            KNOWN_KINDS.contains(&kind),
            "activity kind {kind:?} is not one the client knows; the wire format changed"
        );
        assert!(item["message"].as_str().is_some());
    }
}

#[tokio::test]
async fn the_events_sse_endpoint_streams_activity() {
    let base = serve().await;
    // A live subscriber must receive the next activity line. We trigger one by
    // pausing collection, then read the first SSE frame off the socket.
    let client = reqwest::Client::new();
    let mut stream = client
        .get(format!("{base}/events"))
        .header("accept", "text/event-stream")
        .send()
        .await
        .expect("connect to SSE");
    assert_eq!(stream.status(), 200);

    // Queue a control action after subscribing so it is broadcast to us.
    let _ = post_json(
        &base,
        "/control/collection",
        "",
        serde_json::json!({"enabled": false}),
    )
    .await;

    let mut collected = String::new();
    for _ in 0..10 {
        match tokio::time::timeout(std::time::Duration::from_secs(5), stream.chunk()).await {
            Ok(Ok(Some(chunk))) => {
                collected.push_str(&String::from_utf8_lossy(&chunk));
                if collected.contains("collection paused") {
                    break;
                }
            }
            _ => break,
        }
    }
    assert!(
        collected.contains("event: control") && collected.contains("collection paused"),
        "expected the control activity on the SSE stream, got: {collected}"
    );
}

#[tokio::test]
async fn the_control_plane_requires_a_key_when_one_is_configured() {
    let base = serve_authenticated("s3cret").await;

    // No key: every control route is refused.
    let (status, _) = get_json(&base, "/control").await;
    assert_eq!(status, 401);
    let (status, _) = post_json(
        &base,
        "/control/collection",
        "",
        serde_json::json!({"enabled": false}),
    )
    .await;
    assert_eq!(status, 401);

    // With the key: it works.
    let (status, _) = get_json_auth(&base, "/control", "s3cret").await;
    assert_eq!(status, 200);
    let (status, _) = post_json(
        &base,
        "/control/collection",
        "s3cret",
        serde_json::json!({"enabled": false}),
    )
    .await;
    assert_eq!(status, 200);
}
