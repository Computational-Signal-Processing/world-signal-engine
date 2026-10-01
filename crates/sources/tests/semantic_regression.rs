//! Regression specs for the semantic bugs found in `docs/source-semantic-audit.md`.
//!
//! Each test encodes the behaviour the source *should* have. They are marked
//! `#[ignore]` because the bug is still present: the point is to have the fix
//! turn them green, one at a time, rather than to break the suite today.
//!
//! Run them with `cargo test -p wse-sources --test semantic_regression -- --ignored`.

use chrono::{DateTime, Utc};

fn received() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_120, 0).unwrap()
}

/// F1 — an observation's id must be derived from the *record*, not the whole
/// payload. Re-collecting the same record inside a changed feed must keep its id
/// so the engine de-duplicates it instead of inserting a duplicate point.
#[test]
#[ignore = "F1: observation id is seeded with the whole-payload hash; see docs/source-semantic-audit.md"]
fn usgs_a_re_polled_quake_keeps_its_id() {
    let base = include_bytes!("../../../tests/fixtures/usgs_all_hour.geojson").to_vec();
    let a = wse_sources::usgs::parse(&base, received()).unwrap();

    // The same feed with one additional quake appended.
    let text = String::from_utf8(base).unwrap();
    let extra = r#"{"type":"Feature","id":"new0000001","properties":{"mag":3.3,"place":"1 km N of Somewhere, CA","time":1700000119000},"geometry":{"type":"Point","coordinates":[1.0,2.0,3.0]}},"#;
    let body2 = text
        .replace("\"features\": [", &format!("\"features\": [{extra}"))
        .into_bytes();
    let b = wse_sources::usgs::parse(&body2, received()).unwrap();

    let first_a = a.iter().find(|o| o.value == 5.4).unwrap();
    let first_b = b.iter().find(|o| o.value == 5.4).unwrap();
    assert_eq!(
        first_a.id, first_b.id,
        "the same earthquake must keep one id across feeds"
    );
}

/// F1 — AFAD's window slides between polls; an event retained in both windows
/// must keep its id.
#[test]
#[ignore = "F1: observation id is seeded with the whole-payload hash; see docs/source-semantic-audit.md"]
fn afad_a_window_retained_event_keeps_its_id() {
    let base = include_bytes!("../../../tests/fixtures/afad_events.json").to_vec();
    let a = wse_sources::afad::parse(&base, received()).unwrap();

    let text = String::from_utf8(base).unwrap();
    let extra = r#"{"eventID":"9999999","location":"Test","latitude":"39.0","longitude":"35.0","depth":"5.0","type":"ML","magnitude":"3.1","province":"Ankara","district":"","neighborhood":"","date":"2026-09-03T00:00:00","isEventUpdate":false}"#;
    let body2 = text.replace('[', &format!("[{extra},")).into_bytes();
    let b = wse_sources::afad::parse(&body2, received()).unwrap();

    let first_a = a
        .iter()
        .find(|o| o.attributes.get("event_id").map(String::as_str) == Some("727420"))
        .unwrap();
    let first_b = b
        .iter()
        .find(|o| o.attributes.get("event_id").map(String::as_str) == Some("727420"))
        .unwrap();
    assert_eq!(
        first_a.id, first_b.id,
        "the same AFAD event must keep one id across window slides"
    );
}

/// F1 — ECB returns a rolling `lastNObservations` window; a rate retained in
/// both windows must keep its id.
#[test]
#[ignore = "F1: observation id is seeded with the whole-payload hash; see docs/source-semantic-audit.md"]
fn ecb_a_window_retained_rate_keeps_its_id() {
    let base = include_bytes!("../../../tests/fixtures/ecb_exr.json").to_vec();
    let a = wse_sources::ecb::parse(&base, received()).unwrap();

    let text = String::from_utf8(base).unwrap();
    // Append one extra day to the time axis and one value, mimicking a slid window.
    let body2 = text
        .replace(
            r#"{"id": "2026-09-30", "name": "2026-09-30"}"#,
            r#"{"id": "2026-09-30", "name": "2026-09-30"}, {"id": "2026-10-01", "name": "2026-10-01"}"#,
        )
        .replace(
            r#""2": [1.1355, 0, 0, null, null]"#,
            r#""2": [1.1355, 0, 0, null, null], "3": [1.1320, 0, 0, null, null]"#,
        )
        .into_bytes();
    let b = wse_sources::ecb::parse(&body2, received()).unwrap();

    let first_a = a.iter().find(|o| o.value == 1.1403).unwrap();
    let first_b = b.iter().find(|o| o.value == 1.1403).unwrap();
    assert_eq!(
        first_a.id, first_b.id,
        "an unchanged ECB observation must keep its id across a window slide"
    );
}

/// F1 — NOAA Kp returns a rolling window of 3-hourly points; a point retained in
/// both windows must keep its id.
#[test]
#[ignore = "F1: observation id is seeded with the whole-payload hash; see docs/source-semantic-audit.md"]
fn noaa_kp_a_window_retained_point_keeps_its_id() {
    let base = include_bytes!("../../../tests/fixtures/noaa_kp_index.json").to_vec();
    let a = wse_sources::noaa_kp::parse(&base, received()).unwrap();

    let text = String::from_utf8(base).unwrap();
    let extra = r#"{"time_tag": "2026-09-24T09:00:00", "Kp": 5.0}"#;
    let body2 = text.replacen('[', &format!("[{extra},"), 1).into_bytes();
    let b = wse_sources::noaa_kp::parse(&body2, received()).unwrap();

    let first_a = &a[0];
    let first_b = b
        .iter()
        .find(|o| o.observed_at == first_a.observed_at)
        .unwrap();
    assert_eq!(
        first_a.id, first_b.id,
        "an unchanged Kp observation must keep its id across a window slide"
    );
}

/// F1 — GDELT returns a 1-day window every poll; a bucket retained in both
/// windows must keep its id.
#[test]
#[ignore = "F1: observation id is seeded with the whole-payload hash; see docs/source-semantic-audit.md"]
fn gdelt_a_window_retained_bucket_keeps_its_id() {
    let base = include_bytes!("../../../tests/fixtures/gdelt_timelinevol.json").to_vec();
    let a = wse_sources::gdelt::parse(&base, received()).unwrap();

    let text = String::from_utf8(base).unwrap();
    let extra = r#"{"date": "20260929T000000Z", "value": 1.0}"#;
    let body2 = text
        .replace("\"data\": [", &format!("\"data\": [{extra}, "))
        .into_bytes();
    let b = wse_sources::gdelt::parse(&body2, received()).unwrap();

    let first_a = &a[0];
    let first_b = b
        .iter()
        .find(|o| o.observed_at == first_a.observed_at)
        .unwrap();
    assert_eq!(
        first_a.id, first_b.id,
        "an unchanged GDELT bucket must keep its id across a window slide"
    );
}

/// F2 — the catalog declares Hacker News a `fixed_universe`, but the collector
/// samples the current top-30 each poll, so the population churns between
/// collections. A fixed universe must re-measure the *same* story ids; here the
/// top list changes completely between two polls, and the second poll must still
/// re-measure the ids the first poll committed to (or the catalog must stop
/// claiming `fixed_universe`).
#[tokio::test]
#[ignore = "F2: catalog says fixed_universe, collector churns the top-30; see docs/source-semantic-audit.md"]
async fn hackernews_re_measures_a_stable_universe_across_polls() {
    use std::sync::{Arc, Mutex};
    use wse_collector::{CollectionMode, Collector};
    use wse_scheduler::{Clock, LiveClock};
    use wse_sources::collectors::{
        CollectorContext, HackerNewsCollector, Request, Transport, TransportError,
    };

    /// Returns a different top-story list on every call, and a body per item.
    struct ScriptedTransport {
        calls: Mutex<usize>,
    }

    impl Transport for ScriptedTransport {
        fn fetch(&self, request: &Request) -> Result<Vec<u8>, TransportError> {
            if request.url.contains("topstories") {
                let mut n = self.calls.lock().unwrap();
                *n += 1;
                // Poll 1 returns ids 1..=3, poll 2 returns a disjoint 4..=6.
                let ids: Vec<i64> = if *n == 1 {
                    vec![1, 2, 3]
                } else {
                    vec![4, 5, 6]
                };
                return Ok(serde_json::to_vec(&ids).unwrap());
            }
            // Item endpoint: /item/{id}.json
            let id: i64 = request
                .url
                .rsplit('/')
                .next()
                .and_then(|s| s.split('.').next())
                .and_then(|s| s.parse().ok())
                .expect("item url");
            let body = serde_json::json!({
                "id": id, "type": "story", "title": format!("story {id}"),
                "score": 10.0, "time": 1_700_000_000
            });
            Ok(serde_json::to_vec(&body).unwrap())
        }
    }

    let transport = Arc::new(ScriptedTransport {
        calls: Mutex::new(0),
    });
    let context = CollectorContext::new(transport, Arc::new(LiveClock) as Arc<dyn Clock>);
    let _ = CollectionMode::Live;

    let collector = HackerNewsCollector::with_context(context);
    let first = collector.collect().await.unwrap();
    let second = collector.collect().await.unwrap();

    let first_ids: Vec<String> = first
        .observations
        .iter()
        .filter_map(|o| o.identity.clone())
        .collect();
    let second_ids: Vec<String> = second
        .observations
        .iter()
        .filter_map(|o| o.identity.clone())
        .collect();

    assert_eq!(
        first_ids, second_ids,
        "a fixed universe must re-measure the same story ids every poll"
    );
}
