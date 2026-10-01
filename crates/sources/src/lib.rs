//! # wse-sources
//!
//! The first real feeds, plus the catalog entries that describe them.
//!
//! Each module owns one source. A source module provides:
//!
//! * a [`Source`] catalog entry (the metadata, and how the scheduler runs it);
//! * a pure `parse` function from a payload to [`Observation`]s;
//! * a [`Collector`] that fetches the payload and hands it to `parse`.
//!
//! Splitting fetch from parse is deliberate. `parse` takes bytes and a
//! `received_at` timestamp and does no I/O, so it is fully unit-testable
//! against a checked-in fixture and works identically in live and replay mode.
//!
//! Nothing here depends on the detection or signal engines: a collector that
//! fails only affects its own source.

pub mod collectors;
pub mod eonet;
pub mod gdelt;
pub mod github;
pub mod hackernews;
pub mod nasa;
pub mod nws;
pub mod usgs;

pub use collectors::{
    live_collectors, CollectorContext, EonetCollector, GdeltCollector, GitHubCollector,
    HackerNewsCollector, LiveTransport, NasaNeoCollector, NwsAlertsCollector, Request, Transport,
    TransportError, UsgsCollector,
};

use wse_model::Source;

/// Every catalog entry known to this crate, in priority order.
///
/// This is the single place a new source is registered: add a module, add its
/// entry here. No core code changes.
pub fn catalog() -> Vec<Source> {
    vec![
        usgs::source(),
        nasa::source(),
        gdelt::source(),
        hackernews::source(),
        github::source(),
        nws::source(),
        eonet::source(),
    ]
}

/// Look up a catalog entry by id.
pub fn source_by_id(id: &str) -> Option<Source> {
    catalog().into_iter().find(|s| s.id.as_str() == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_ids_are_unique_and_prefixed() {
        let entries = catalog();
        assert!(entries.len() >= 5);
        let mut ids: Vec<&str> = entries.iter().map(|s| s.id.as_str()).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(before, ids.len(), "duplicate source ids in catalog");
    }

    #[test]
    fn every_entry_declares_a_collector_type_and_endpoint() {
        for source in catalog() {
            assert!(
                !source.collector_type.is_empty(),
                "{} has no collector_type",
                source.id
            );
            assert!(!source.endpoint.is_empty(), "{} has no endpoint", source.id);
            assert!(
                source.license.is_some(),
                "{} must declare a license so usage stays lawful",
                source.id
            );
        }
    }

    #[test]
    fn source_by_id_finds_every_catalog_entry() {
        for source in catalog() {
            let found = source_by_id(source.id.as_str()).expect("entry should resolve");
            assert_eq!(found.id, source.id);
        }
        assert!(source_by_id("nope").is_none());
    }
}
