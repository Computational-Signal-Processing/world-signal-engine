//! Strongly-typed identifiers used across the engine.

use serde::{Deserialize, Serialize};

macro_rules! string_id {
    ($(#[$meta:meta])* $name:ident, $prefix:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Stable prefix used when generating new identifiers.
            pub const PREFIX: &'static str = $prefix;

            pub fn new(id: impl Into<String>) -> Self {
                Self(id.into())
            }

            /// Generate a fresh, random identifier.
            pub fn generate() -> Self {
                Self(format!("{}_{}", $prefix, uuid::Uuid::new_v4().simple()))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_string())
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

string_id!(
    /// Identifies a configured data source from the source catalog.
    SourceId,
    "src"
);
string_id!(
    /// Identifies a single observation.
    ObservationId,
    "obs"
);
string_id!(
    /// Identifies a canonical entity.
    EntityId,
    "ent"
);
string_id!(
    /// Identifies an event formed from anomalies.
    EventId,
    "evt"
);
string_id!(
    /// Identifies a signal presented to humans.
    SignalId,
    "sig"
);
string_id!(
    /// Identifies an anomaly candidate.
    AnomalyId,
    "anm"
);
string_id!(
    /// Identifies a lens.
    LensId,
    "lens"
);

impl ObservationId {
    /// Deterministic identifier derived from the observation identity and the
    /// hash of its raw payload.
    ///
    /// Determinism makes duplicate detection trivial: collecting the same
    /// payload twice yields the same id, so the store can reject it without
    /// comparing every field.
    ///
    /// `identity` is the per-record discriminator (a repository name, a story
    /// id). Sources that emit several records per series per timestamp must
    /// pass one, or those records collide on `(series, timestamp, payload)` and
    /// are silently de-duplicated down to a single survivor. A `None` identity
    /// keeps the original id, so single-record sources are unaffected.
    pub fn deterministic(
        series_key: &str,
        observed_at: &str,
        payload_hash: &str,
        identity: Option<&str>,
    ) -> Self {
        let seed = match identity {
            Some(identity) => format!("{series_key}|{observed_at}|{payload_hash}|{identity}"),
            // No discriminator: keep the original seed format, so ids for
            // single-record sources are unchanged by this addition.
            None => format!("{series_key}|{observed_at}|{payload_hash}"),
        };
        Self(format!("obs_{}", fnv1a_hex(&seed)))
    }
}

/// Stable, dependency-free FNV-1a 64-bit hash rendered as lowercase hex.
///
/// Used for payload fingerprints and deterministic identifiers. It is *not*
/// a cryptographic hash and must never be used for security purposes.
pub fn fnv1a_hex(input: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_ids_carry_their_prefix() {
        let id = SourceId::generate();
        assert!(id.as_str().starts_with("src_"));
        assert_eq!(id.as_str().len(), 4 + 32);
    }

    #[test]
    fn ids_round_trip_through_serde() {
        let id = EntityId::new("ent_istanbul");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"ent_istanbul\"");
        let back: EntityId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn deterministic_observation_ids_are_stable_and_distinct() {
        let a = ObservationId::deterministic("s::m::u", "2026-01-01T00:00:00Z", "abc", None);
        let b = ObservationId::deterministic("s::m::u", "2026-01-01T00:00:00Z", "abc", None);
        let c = ObservationId::deterministic("s::m::u", "2026-01-01T00:00:01Z", "abc", None);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn a_record_discriminator_separates_records_sharing_a_timestamp() {
        // Two records, same series, same timestamp, same payload hash — exactly
        // the GitHub/HN shape. The discriminator is what keeps them apart.
        let repo_a =
            ObservationId::deterministic("s::m::u", "2026-01-01T00:00:00Z", "abc", Some("a/b"));
        let repo_b =
            ObservationId::deterministic("s::m::u", "2026-01-01T00:00:00Z", "abc", Some("c/d"));
        assert_ne!(repo_a, repo_b);
        // Same record re-collected is still the same id, so it de-duplicates.
        let repo_a_again =
            ObservationId::deterministic("s::m::u", "2026-01-01T00:00:00Z", "abc", Some("a/b"));
        assert_eq!(repo_a, repo_a_again);
    }

    #[test]
    fn a_missing_discriminator_keeps_the_original_id() {
        // Backwards compatibility: a source with one record per timestamp must
        // not see its ids change just because the discriminator was added.
        let no_identity =
            ObservationId::deterministic("s::m::u", "2026-01-01T00:00:00Z", "abc", None);
        let legacy = ObservationId::new(format!(
            "obs_{}",
            fnv1a_hex("s::m::u|2026-01-01T00:00:00Z|abc")
        ));
        assert_eq!(no_identity, legacy);
    }

    #[test]
    fn fnv1a_is_deterministic_and_hex_encoded() {
        assert_eq!(fnv1a_hex("hello"), fnv1a_hex("hello"));
        assert_ne!(fnv1a_hex("hello"), fnv1a_hex("world"));
        assert_eq!(fnv1a_hex("hello").len(), 16);
    }
}
