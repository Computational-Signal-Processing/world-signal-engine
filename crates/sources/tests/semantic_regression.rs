//! Regression specs for the semantic bugs found in `docs/source-semantic-audit.md`.
//!
//! All specs are active: they pass under the fixed contracts (per-record
//! observation identity for F1, a committed universe for F2, a coherent daily
//! count for F4, a non-overlapping daily count for F5, a single completed day
//! for F7, a per-repository series for F9) and fail without them.

use chrono::{DateTime, Utc};

fn received() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_120, 0).unwrap()
}

/// F1 — an observation's id must be derived from the *record*, not the whole
/// payload. Re-collecting the same record inside a changed feed must keep its id
/// so the engine de-duplicates it instead of inserting a duplicate point.
#[test]
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
    // The newly appearing quake gets its own id, distinct from the rest.
    let new = b.iter().find(|o| o.value == 3.3).unwrap();
    assert!(a.iter().all(|o| o.id != new.id));
}

/// F1 — AFAD's window slides between polls; an event retained in both windows
/// must keep its id.
#[test]
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
/// both windows must keep its id. The window is simulated the way the API does
/// it: append the newest day, then drop the oldest, so retained days are not
/// re-indexed.
#[test]
fn ecb_a_window_retained_rate_keeps_its_id() {
    let base = include_bytes!("../../../tests/fixtures/ecb_exr.json").to_vec();
    let a = wse_sources::ecb::parse(&base, received()).unwrap();

    let mut v: serde_json::Value = serde_json::from_slice(&base).unwrap();
    let series = v["dataSets"][0]["series"]["0:0:0:0:0"]["observations"]
        .as_object_mut()
        .unwrap();
    // Shift the retained window up by one index (append newest, drop oldest).
    let v2 = series["2"].clone();
    let v1 = series["1"].clone();
    series.insert(
        "2".to_string(),
        serde_json::json!([1.1320, 0, 0, null, null]),
    );
    series.insert("1".to_string(), v2);
    series.insert("0".to_string(), v1);
    series.remove("3");
    v["structure"]["dimensions"]["observation"][0]["values"] = serde_json::json!([
        {"id": "2026-09-29", "name": "2026-09-29"},
        {"id": "2026-09-30", "name": "2026-09-30"},
        {"id": "2026-10-01", "name": "2026-10-01"}
    ]);
    let body2 = serde_json::to_vec(&v).unwrap();
    let b = wse_sources::ecb::parse(&body2, received()).unwrap();

    // 2026-09-29 (1.1378) and 2026-09-30 (1.1355) are retained across the slide.
    for value in [1.1378, 1.1355] {
        let first_a = a.iter().find(|o| o.value == value).unwrap();
        let first_b = b.iter().find(|o| o.value == value).unwrap();
        assert_eq!(
            first_a.id, first_b.id,
            "an unchanged ECB observation ({value}) must keep its id across a window slide"
        );
    }
    // 2026-09-28 (1.1403) was dropped; it must not appear in the new window.
    assert!(b.iter().all(|o| o.value != 1.1403));
}

/// F1 — NOAA Kp returns a rolling window of 3-hourly points; a point retained in
/// both windows must keep its id. The extra point is appended, so the retained
/// points keep their timestamps.
#[test]
fn noaa_kp_a_window_retained_point_keeps_its_id() {
    let base = include_bytes!("../../../tests/fixtures/noaa_kp_index.json").to_vec();
    let a = wse_sources::noaa_kp::parse(&base, received()).unwrap();

    let text = String::from_utf8(base).unwrap();
    let extra = r#"{"time_tag": "2026-09-24T09:00:00", "Kp": 5.0}"#;
    let body2 = text.replacen(']', &format!(",{extra}]"), 1).into_bytes();
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
/// windows must keep its id. A stale bucket is dropped and a new one appended,
/// so the retained buckets keep their timestamps.
#[test]
fn gdelt_a_window_retained_bucket_keeps_its_id() {
    let base = include_bytes!("../../../tests/fixtures/gdelt_timelinevol.json").to_vec();
    let a = wse_sources::gdelt::parse(&base, received()).unwrap();

    let mut v: serde_json::Value = serde_json::from_slice(&base).unwrap();
    let data = v["timeline"][0]["data"].as_array_mut().unwrap();
    data.remove(0); // drop the oldest bucket
    data.push(serde_json::json!({"date": "20231118000000", "value": 0.95}));
    let body2 = serde_json::to_vec(&v).unwrap();
    let b = wse_sources::gdelt::parse(&body2, received()).unwrap();

    // 20231115120000 (0.455) is retained across the slide.
    let first_a = a.iter().find(|o| o.value == 0.455).unwrap();
    let first_b = b.iter().find(|o| o.value == 0.455).unwrap();
    assert_eq!(
        first_a.id, first_b.id,
        "an unchanged GDELT bucket must keep its id across a window slide"
    );
}

/// F1 — reordering a feed must not change any record's id (identity is not
/// positional).
#[test]
fn usgs_reordering_the_feed_does_not_change_ids() {
    let base = include_bytes!("../../../tests/fixtures/usgs_all_hour.geojson").to_vec();
    let a = wse_sources::usgs::parse(&base, received()).unwrap();

    let mut v: serde_json::Value = serde_json::from_slice(&base).unwrap();
    v["features"].as_array_mut().unwrap().reverse();
    let b = wse_sources::usgs::parse(&serde_json::to_vec(&v).unwrap(), received()).unwrap();

    let ids = |obs: &[wse_model::Observation]| {
        let mut ids: Vec<String> = obs.iter().map(|o| o.id.as_str().to_string()).collect();
        ids.sort();
        ids
    };
    assert_eq!(ids(&a), ids(&b), "reordering must not change ids");
}

/// F1 — removing an unrelated record must not change the ids of the records that
/// remain.
#[test]
fn usgs_removing_an_unrelated_quake_does_not_change_ids() {
    let base = include_bytes!("../../../tests/fixtures/usgs_all_hour.geojson").to_vec();
    let a = wse_sources::usgs::parse(&base, received()).unwrap();

    // Drop the last feature (hv73000001, magnitude 1.8).
    let mut v: serde_json::Value = serde_json::from_slice(&base).unwrap();
    v["features"].as_array_mut().unwrap().pop();
    let b = wse_sources::usgs::parse(&serde_json::to_vec(&v).unwrap(), received()).unwrap();

    assert_eq!(b.len(), a.len() - 1);
    for kept in &b {
        let same = a.iter().find(|o| o.value == kept.value).unwrap();
        assert_eq!(
            kept.id, same.id,
            "removing an unrelated record must not change the remaining ids"
        );
    }
}

/// F1 — a revised measurement for the same upstream record keeps its identity.
/// A magnitude revision is a change in the *value*, not a new earthquake.
#[test]
fn usgs_a_revised_magnitude_keeps_identity() {
    let base = include_bytes!("../../../tests/fixtures/usgs_all_hour.geojson").to_vec();
    let a = wse_sources::usgs::parse(&base, received()).unwrap();
    let original = a.iter().find(|o| o.value == 5.4).unwrap();

    let mut v: serde_json::Value = serde_json::from_slice(&base).unwrap();
    v["features"][0]["properties"]["mag"] = serde_json::json!(5.7);
    let b = wse_sources::usgs::parse(&serde_json::to_vec(&v).unwrap(), received()).unwrap();
    let revised = b.iter().find(|o| o.value == 5.7).unwrap();

    assert_eq!(
        original.id, revised.id,
        "a revised magnitude for the same quake must keep one identity"
    );
    assert_eq!(original.series_key(), revised.series_key());
}

/// F2 — the catalog declares Hacker News a `fixed_universe`, so the collector
/// must re-measure the *same* story ids every poll. The top list changes
/// completely between the two polls here; the second poll must still re-measure
/// the ids the first poll committed to. A story leaving the front page is not a
/// world change and must not churn the universe.
#[tokio::test]
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

/// F4 — NASA NEO must not pool every object into one distance series. The feed
/// interleaves unrelated rocks, so a distance series is incoherent, and a
/// symmetric detector would flag a *far* pass as anomalous. The coherent series
/// is the daily count of close approaches; the day's closest object stays as
/// drill-down attributes.
#[test]
fn nasa_neo_counts_approaches_per_day_rather_than_pooling_distances() {
    let body = include_bytes!("../../../tests/fixtures/nasa_neo_feed.json").to_vec();
    let observations = wse_sources::nasa::parse(&body, received()).unwrap();

    // One point per UTC day, each the day's approach count — not one point per
    // object carrying a raw miss distance.
    assert_eq!(observations.len(), 2);
    assert!(
        observations
            .iter()
            .all(|o| o.metric == "neo_close_approaches"),
        "the series must be a count, not a pooled distance"
    );
    assert_eq!(observations[0].value, 2.0);
    assert_eq!(observations[1].value, 1.0);

    // All points share one series, so the count is comparable day to day.
    let keys: Vec<String> = observations.iter().map(|o| o.series_key()).collect();
    assert!(
        keys.iter().all(|k| k == &keys[0]),
        "one coherent series: {keys:?}"
    );

    // The day's closest object is preserved for drill-down.
    assert!(observations[0]
        .attributes
        .contains_key("closest_object_name"));
}

/// F5 — CISA `kev_added` must be a non-overlapping daily count, not a trailing
/// 7-day sum sampled daily. The overlapping window made consecutive points share
/// six of seven days, so a daily z-score was structurally misleading. The count
/// is now the additions dated to the collection day, so consecutive polls never
/// share a member and the day is the record key.
#[test]
fn cisa_kev_counts_only_the_collection_day() {
    let body = include_bytes!("../../../tests/fixtures/cisa_kev.json").to_vec();
    // The fixture's catalogue version is 2026.09.30, with one entry dated that
    // day; the poll is on that day.
    let day = DateTime::parse_from_rfc3339("2026-09-30T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let observations = wse_sources::cisa_kev::parse(&body, day).unwrap();
    let added = observations
        .iter()
        .find(|o| o.metric == "kev_added")
        .unwrap();

    // One entry dated the collection day — not the trailing-window total.
    assert_eq!(added.value, 1.0);
    assert_eq!(
        added.attributes.get("day").map(String::as_str),
        Some("2026-09-30")
    );

    // The next day counts its own additions, not a shared window.
    let next = day + chrono::Duration::days(1);
    let next_added = wse_sources::cisa_kev::parse(&body, next)
        .unwrap()
        .into_iter()
        .find(|o| o.metric == "kev_added")
        .unwrap();
    assert_eq!(next_added.value, 0.0);
    assert_ne!(
        added.id, next_added.id,
        "each day is its own record, never a sliding window"
    );
}

/// F7 — Crossref must measure a single completed day, not a trailing window
/// ending today. A 2-day window sampled daily shares a day between consecutive
/// polls, and a window ending today counts a day whose deposits are still
/// arriving (the newest point structurally depressed). The window is now
/// `[yesterday, yesterday)`: one day, never today.
#[test]
fn crossref_measures_one_completed_day_never_today() {
    use wse_sources::crossref::{observation_for, window};

    let morning = DateTime::parse_from_rfc3339("2026-10-01T06:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let (from, until) = window(morning);
    // A single day, and it is yesterday, not the collection day.
    assert_eq!(from, "2026-09-30");
    assert_eq!(until, "2026-09-30");
    assert_ne!(until, "2026-10-01", "today must never be measured");

    // The window is stable across the collection day, so a re-poll keeps one id.
    let evening = DateTime::parse_from_rfc3339("2026-10-01T23:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let a = observation_for("crispr", "CRISPR", 100, morning, b"{}");
    let b = observation_for("crispr", "CRISPR", 100, evening, b"{}");
    assert_eq!(a.id, b.id, "the same measured day keeps one identity");
    // The observation is timestamped at the measured day, not the poll time.
    assert_eq!(
        a.observed_at,
        DateTime::parse_from_rfc3339("2026-09-30T00:00:00Z").unwrap()
    );
}

/// F9 — GitHub repositories must not share one baseline. All repositories use
/// entity `ecosystem_rust` / metric `repo_stars` / unit `stars`; without a
/// discriminator they pool into one series, so a repository's first appearance
/// is scored against the others' star counts and fires a meaningless cold-start
/// deviation (~1527σ, `docs/reality-audit.md` finding 6). The `repo` dimension
/// gives each repository its own series, so the baseline is that repository's
/// own history and the sample minimum is a per-repo cold-start guard.
#[test]
fn github_repositories_do_not_share_one_baseline() {
    let fixture = include_bytes!("../../../tests/fixtures/github_repo.json").to_vec();
    let at = DateTime::parse_from_rfc3339("2026-10-01T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);

    let tokio = wse_sources::github::parse_repo(&fixture, at)
        .unwrap()
        .unwrap();
    let serde = wse_sources::github::parse_repo(
        br#"{"id":2,"full_name":"serde-rs/serde","stargazers_count":5}"#,
        at,
    )
    .unwrap()
    .unwrap();

    // Distinct series, distinguished by the repository, not pooled.
    assert_ne!(tokio.series_key(), serde.series_key());
    assert!(tokio.series_key().contains("repo=tokio-rs/tokio"));
    assert!(serde.series_key().contains("repo=serde-rs/serde"));

    // The same repository across polls stays on one series.
    let tokio_again = wse_sources::github::parse_repo(&fixture, at)
        .unwrap()
        .unwrap();
    assert_eq!(tokio.series_key(), tokio_again.series_key());
}
