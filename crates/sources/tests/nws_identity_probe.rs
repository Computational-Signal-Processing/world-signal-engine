//! F1.1 — NWS observation identity probe.
//!
//! NWS emits one count per (entity, severity). The id is seeded with the
//! *base* series key (`source::entity::metric::unit`), which does **not**
//! include dimensions, so several counts that differ only by severity can
//! share an id. This probe prints the collision structure and asserts the
//! invariant that distinct series must have distinct ids.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

fn received() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).unwrap()
}

#[test]
fn nws_distinct_series_must_have_distinct_ids() {
    let body = include_bytes!("../../../tests/fixtures/nws_active_alerts.json").to_vec();
    let observations = wse_sources::nws::parse(&body, received()).unwrap();

    // Group by id, keeping the fields that make each observation distinct.
    let mut by_id: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for o in &observations {
        let entity = o.entity_id.as_ref().map(|e| e.as_str()).unwrap_or("-");
        let severity = o
            .dimensions
            .get("severity")
            .map(String::as_str)
            .unwrap_or("-");
        let state = o.dimensions.get("state").map(String::as_str).unwrap_or("-");
        by_id
            .entry(o.id.as_str().to_string())
            .or_default()
            .push(format!(
                "entity={entity} severity={severity} state={state} value={}",
                o.value
            ));
    }

    let distinct_series: std::collections::BTreeSet<String> =
        observations.iter().map(|o| o.series_key()).collect();

    println!("total observations : {}", observations.len());
    println!("distinct ids       : {}", by_id.len());
    println!("distinct series    : {}", distinct_series.len());
    for (id, members) in &by_id {
        println!("  id {id} -> {} record(s)", members.len());
        for m in members {
            println!("      {m}");
        }
    }

    assert_eq!(
        by_id.len(),
        distinct_series.len(),
        "every distinct (entity, severity) series must have its own observation id"
    );

    // Identity must be stable across repeated polls, so an unchanged count
    // de-duplicates instead of being re-inserted.
    let again = wse_sources::nws::parse(&body, received()).unwrap();
    let ids_a: std::collections::BTreeSet<&str> =
        observations.iter().map(|o| o.id.as_str()).collect();
    let ids_b: std::collections::BTreeSet<&str> = again.iter().map(|o| o.id.as_str()).collect();
    assert_eq!(ids_a, ids_b, "ids must be deterministic across polls");
}
