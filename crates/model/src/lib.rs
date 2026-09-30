//! # wse-model
//!
//! Domain model for the World Signal Engine.
//!
//! The model deliberately separates the pipeline stages described in
//! `DATA_MODEL.md` and `SIGNAL_MODEL.md`:
//!
//! ```text
//! Observation -> Change -> AnomalyCandidate -> Event -> Signal
//! ```
//!
//! Nothing in this crate performs I/O or detection; it only describes the
//! vocabulary that the rest of the engine shares.

pub mod anomaly;
pub mod entity;
pub mod event;
pub mod geo;
pub mod ids;
pub mod lens;
pub mod observation;
pub mod quality;
pub mod signal;
pub mod source;

pub use anomaly::{
    robust_z_score, AnomalyCandidate, BaselineSnapshot, CandidateDirection, CandidateKind,
    DetectionMethod,
};
pub use entity::{canonicalize, Entity, EntityKind};
pub use event::{Event, EventState};
pub use geo::{haversine_km, Location};
pub use ids::{fnv1a_hex, AnomalyId, EntityId, EventId, LensId, ObservationId, SignalId, SourceId};
pub use lens::Lens;
pub use observation::{series_key, Observation, RawReference};
pub use quality::{DataPresence, Quality, QualityFlag};
pub use signal::{Direction, Evidence, Signal, SignalQuality, SignalType};
pub use source::{
    AuthKind, Cadence, Cost, DataFormat, HealthStatus, Protocol, Source, SourceHealth,
};
