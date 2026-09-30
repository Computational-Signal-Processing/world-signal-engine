//! # wse-presentation
//!
//! The layer between detection and a person.
//!
//! Everything upstream of this crate speaks in series keys, sigma and
//! candidate kinds. Everything a person reads is built here: the headline, the
//! "what changed" sentence, the magnitude in words, the stated unknowns, and
//! the lifecycle status. Keeping it in one crate means the API and the web
//! client cannot disagree about what a signal says, and the translation is
//! tested like any other logic rather than living in the UI as string
//! concatenation.
//!
//! It reads only the signal's own record. It never queries the store, never
//! invents a cause, and never states a fact the evidence does not carry — what
//! is not known is listed in [`SignalNarrative::unknowns`].

pub mod narrative;
pub mod vocabulary;

pub use narrative::{narrative_for, status_for};
pub use vocabulary::{category_label, entity_vocab, metric_vocab, source_label};

use wse_model::Signal;

/// Fill a signal's human-facing fields from its own record.
///
/// Idempotent: calling it again recomputes the same narrative, which is what
/// lets a merged signal re-derive its text after its evidence has grown.
pub fn describe(signal: &mut Signal) {
    signal.narrative = narrative::narrative_for(signal);
}
