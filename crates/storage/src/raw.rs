//! Raw payload retention.
//!
//! The drill-down ends at `... -> SOURCE -> RAW DATA`, so the raw bytes a
//! collector fetched have to be retrievable, not just referenced. This store
//! keeps them keyed by the same [`RawReference`] hash the observations carry,
//! which is what makes "show me the evidence" a real promise rather than a
//! pointer to a URL that may since have changed.
//!
//! Two implementations exist, behind [`RawStore`]:
//!
//! * [`MemoryRawStore`] — a hash map. Used by the synthetic world and the tests,
//!   where a run is short and nothing needs to outlive the process.
//! * `SqliteRawStore` (in the SQLite backend) — content-addressed files on disk,
//!   so an engine that runs for months does not hold every payload in RAM.

use std::collections::HashMap;
use std::path::PathBuf;

use wse_model::RawReference;

use crate::store::StorageError;

/// A retained payload together with where it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredPayload {
    pub reference: RawReference,
    pub body: Vec<u8>,
}

/// How the raw bytes behind an observation are retained.
///
/// The engine only ever needs "keep these bytes under this hash" and "give me
/// the bytes for this hash", so the backend stays swappable and the storage
/// choice never leaks into the pipeline.
pub trait RawStore: Send + Sync {
    fn put(&mut self, reference: RawReference, body: Vec<u8>) -> Result<(), StorageError>;

    fn get(&self, hash: &str) -> Result<Option<StoredPayload>, StorageError>;

    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Total bytes held by the store, for `/metrics`.
    fn bytes_used(&self) -> u64 {
        0
    }

    /// Path of the file holding `hash`, when the backend keeps files.
    ///
    /// `None` for a backend that has no files, so callers must not assume a
    /// path exists.
    fn location(&self, _hash: &str) -> Option<PathBuf> {
        None
    }

    /// Delete retained payloads, oldest reference first, until the store is at
    /// or below `max_bytes`. Returns how many were removed.
    ///
    /// Only the raw bytes are dropped; the [`RawReference`] on the observation
    /// stays, so the drill-down still reports *what* the payload was and why it
    /// is no longer retained. Silently losing the reference would be data loss;
    /// losing the bytes is a documented retention policy.
    fn prune_to(&mut self, _max_bytes: u64) -> Result<usize, StorageError> {
        Ok(0)
    }
}

/// Raw payloads held in memory, addressable by their content hash.
#[derive(Debug, Default)]
pub struct MemoryRawStore {
    payloads: HashMap<String, StoredPayload>,
}

impl MemoryRawStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl RawStore for MemoryRawStore {
    /// Retain a payload. Re-storing the same bytes is a no-op, so repeated
    /// collection cycles do not grow memory without bound.
    fn put(&mut self, reference: RawReference, body: Vec<u8>) -> Result<(), StorageError> {
        if body.len() as u64 != reference.bytes.unwrap_or(body.len() as u64) {
            return Err(StorageError::InvalidQuery(format!(
                "raw payload length {} does not match its reference ({})",
                body.len(),
                reference.bytes.unwrap_or_default()
            )));
        }
        self.payloads
            .entry(reference.hash.clone())
            .or_insert(StoredPayload { reference, body });
        Ok(())
    }

    fn get(&self, hash: &str) -> Result<Option<StoredPayload>, StorageError> {
        Ok(self.payloads.get(hash).cloned())
    }

    fn len(&self) -> usize {
        self.payloads.len()
    }

    fn bytes_used(&self) -> u64 {
        self.payloads.values().map(|p| p.body.len() as u64).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(hash: &str, bytes: u64) -> RawReference {
        RawReference {
            locator: "https://example.test/feed".to_string(),
            hash: hash.to_string(),
            content_type: Some("application/json".to_string()),
            bytes: Some(bytes),
        }
    }

    #[test]
    fn a_payload_is_retrievable_by_its_reference_hash() {
        let mut store = MemoryRawStore::new();
        let body = b"{\"a\":1}".to_vec();
        let hash = wse_model::fnv1a_hex("{\"a\":1}");
        store
            .put(reference(&hash, body.len() as u64), body.clone())
            .unwrap();
        assert_eq!(store.get(&hash).unwrap().unwrap().body, body);
    }

    #[test]
    fn re_storing_the_same_payload_does_not_duplicate_it() {
        let mut store = MemoryRawStore::new();
        let body = b"same".to_vec();
        let hash = wse_model::fnv1a_hex("same");
        store.put(reference(&hash, 4), body.clone()).unwrap();
        store.put(reference(&hash, 4), body).unwrap();
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn a_length_mismatch_is_rejected_rather_than_stored_wrong() {
        let mut store = MemoryRawStore::new();
        assert!(matches!(
            store.put(reference("h", 99), b"short".to_vec()),
            Err(StorageError::InvalidQuery(_))
        ));
        assert!(store.is_empty());
    }

    #[test]
    fn bytes_used_tracks_the_retained_payloads() {
        let mut store = MemoryRawStore::new();
        store.put(reference("h1", 3), b"abc".to_vec()).unwrap();
        store.put(reference("h2", 2), b"de".to_vec()).unwrap();
        assert_eq!(store.bytes_used(), 5);
    }
}
