//! Raw payload retention.
//!
//! The drill-down ends at `... -> SOURCE -> RAW DATA`, so the raw bytes a
//! collector fetched have to be retrievable, not just referenced. This store
//! keeps them keyed by the same [`RawReference`] hash the observations carry,
//! which is what makes "show me the evidence" a real promise rather than a
//! pointer to a URL that may since have changed.

use std::collections::HashMap;

use wse_model::RawReference;

use crate::store::StorageError;

/// Raw payloads, addressable by their content hash.
#[derive(Debug, Default)]
pub struct RawStore {
    payloads: HashMap<String, StoredPayload>,
}

/// A retained payload together with where it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredPayload {
    pub reference: RawReference,
    pub body: Vec<u8>,
}

impl RawStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Retain a payload. Re-storing the same bytes is a no-op, so repeated
    /// collection cycles do not grow memory without bound.
    pub fn put(&mut self, reference: RawReference, body: Vec<u8>) -> Result<(), StorageError> {
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

    pub fn get(&self, hash: &str) -> Option<&StoredPayload> {
        self.payloads.get(hash)
    }

    pub fn len(&self) -> usize {
        self.payloads.len()
    }

    pub fn is_empty(&self) -> bool {
        self.payloads.is_empty()
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
        let mut store = RawStore::new();
        let body = b"{\"a\":1}".to_vec();
        let hash = wse_model::fnv1a_hex("{\"a\":1}");
        store
            .put(reference(&hash, body.len() as u64), body.clone())
            .unwrap();
        assert_eq!(store.get(&hash).unwrap().body, body);
    }

    #[test]
    fn re_storing_the_same_payload_does_not_duplicate_it() {
        let mut store = RawStore::new();
        let body = b"same".to_vec();
        let hash = wse_model::fnv1a_hex("same");
        store.put(reference(&hash, 4), body.clone()).unwrap();
        store.put(reference(&hash, 4), body).unwrap();
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn a_length_mismatch_is_rejected_rather_than_stored_wrong() {
        let mut store = RawStore::new();
        assert!(matches!(
            store.put(reference("h", 99), b"short".to_vec()),
            Err(StorageError::InvalidQuery(_))
        ));
        assert!(store.is_empty());
    }
}
