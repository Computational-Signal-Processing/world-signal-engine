//! # wse-collector
//!
//! The collector contract and the synthetic world.
//!
//! A collector knows how to talk to one source. It does **not** know anything
//! about detection: it fetches, de-duplicates, normalizes and hands
//! [`Observation`]s to the pipeline. This separation is what lets a new source
//! be added with "a new collector plus a source config" and no core changes.
//!
//! [`synthetic`] provides a deterministic world so the whole pipeline can be
//! exercised without depending on any real feed.

pub mod rng;
pub mod synthetic;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use wse_model::{Observation, RawReference, SourceId};

pub use synthetic::{SyntheticCollector, SyntheticStream, SyntheticWorld};

#[derive(Debug, Error)]
pub enum CollectorError {
    #[error("transport failure: {0}")]
    Transport(String),
    #[error("failed to parse source payload: {0}")]
    Parse(String),
    #[error("source returned an unexpected status: {0}")]
    Status(String),
    #[error("collector is not configured: {0}")]
    Configuration(String),
    #[error("other collector error: {0}")]
    Other(String),
}

/// How often a collector should run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Schedule {
    /// Run whenever the source emits; the scheduler polls at `poll_seconds`.
    Event { poll_seconds: u64 },
    /// Fixed interval.
    Interval { seconds: u64 },
    /// Only run when explicitly triggered (tests, replay).
    Manual,
}

impl Schedule {
    /// Default poll interval in seconds, used by the scheduler.
    pub fn poll_seconds(&self) -> Option<u64> {
        match self {
            Schedule::Event { poll_seconds } => Some(*poll_seconds),
            Schedule::Interval { seconds } => Some(*seconds),
            Schedule::Manual => None,
        }
    }
}

/// Live or replay. The brief requires both so detection can be back-tested on
/// historical data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CollectionMode {
    Live,
    Replay,
}

/// The untouched payload, kept so a human can drill down to raw data.
#[derive(Debug, Clone, PartialEq)]
pub struct RawPayload {
    pub reference: RawReference,
    pub body: Vec<u8>,
}

impl RawPayload {
    /// Build a payload, computing its fingerprint and size.
    pub fn new(locator: impl Into<String>, body: Vec<u8>, content_type: &str) -> Self {
        let hash = wse_model::fnv1a_hex(&String::from_utf8_lossy(&body));
        let reference = RawReference {
            locator: locator.into(),
            hash,
            content_type: Some(content_type.to_string()),
            bytes: Some(body.len() as u64),
        };
        Self { reference, body }
    }

    pub fn as_str(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.body)
    }
}

/// What one collection run produced.
#[derive(Debug, Clone, Default)]
pub struct CollectionResult {
    pub source_id: Option<SourceId>,
    pub observations: Vec<Observation>,
    pub raw_payloads: Vec<RawPayload>,
    /// Number of records the source returned, before de-duplication.
    pub records_received: u64,
    /// Number of records that were new.
    pub records_changed: u64,
    /// Number of records recognised as duplicates.
    pub records_duplicate: u64,
    pub errors: Vec<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl CollectionResult {
    pub fn new(source_id: SourceId) -> Self {
        Self {
            source_id: Some(source_id),
            ..Self::default()
        }
    }

    /// A failed run: no observations, and the failure recorded explicitly.
    ///
    /// Note that this is *not* the same as a successful run that found no
    /// records. Callers must keep the two apart (see `docs/philosophy.md`).
    pub fn failure(source_id: SourceId, error: impl Into<String>) -> Self {
        Self {
            source_id: Some(source_id),
            errors: vec![error.into()],
            ..Self::default()
        }
    }

    pub fn is_failure(&self) -> bool {
        !self.errors.is_empty() && self.observations.is_empty()
    }

    pub fn latency_ms(&self) -> Option<u64> {
        match (self.started_at, self.finished_at) {
            (Some(a), Some(b)) => Some((b - a).num_milliseconds().max(0) as u64),
            _ => None,
        }
    }
}

/// The contract every collector implements.
#[async_trait]
pub trait Collector: Send + Sync {
    /// The catalog entry this collector serves.
    fn source_id(&self) -> SourceId;

    /// How the scheduler should run this collector.
    fn schedule(&self) -> Schedule;

    /// Whether the collector can be replayed from history.
    fn mode(&self) -> CollectionMode {
        CollectionMode::Live
    }

    /// Perform one collection run.
    async fn collect(&self) -> Result<CollectionResult, CollectorError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_payload_fingerprints_its_body() {
        let a = RawPayload::new("raw/a", b"{\"x\":1}".to_vec(), "application/json");
        let b = RawPayload::new("raw/a", b"{\"x\":1}".to_vec(), "application/json");
        let c = RawPayload::new("raw/a", b"{\"x\":2}".to_vec(), "application/json");
        assert_eq!(a.reference.hash, b.reference.hash);
        assert_ne!(a.reference.hash, c.reference.hash);
        assert_eq!(a.reference.bytes, Some(7));
    }

    #[test]
    fn failure_is_distinct_from_empty_success() {
        let src = SourceId::new("src_a");
        let empty = CollectionResult::new(src.clone());
        assert!(!empty.is_failure());
        let failed = CollectionResult::failure(src, "connection refused");
        assert!(failed.is_failure());
        assert!(failed.observations.is_empty());
    }

    #[test]
    fn schedule_poll_intervals() {
        assert_eq!(Schedule::Interval { seconds: 30 }.poll_seconds(), Some(30));
        assert_eq!(Schedule::Event { poll_seconds: 5 }.poll_seconds(), Some(5));
        assert_eq!(Schedule::Manual.poll_seconds(), None);
    }
}
