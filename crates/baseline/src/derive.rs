//! Derived metrics: turning a raw measurement into the quantity worth
//! detecting on.
//!
//! A collector emits what the source reports. Sometimes that is not the thing
//! whose change matters: a cumulative registry size only ever grows, and a level
//! z-score on it is close to meaningless — the change is the *increment*. This
//! module is the small, pure evaluator that computes such a derived value. It
//! holds no state and does no I/O, so it is deterministic and can be tested on
//! its own; the engine is responsible for supplying the predecessor.
//!
//! The rules are deliberately conservative and are the whole point:
//!
//! * **No predecessor is not zero.** A first observation produces no derived
//!   value; emitting `0` would claim "nothing happened" when the truth is "we
//!   have nothing to compare against".
//! * **A counter reset is not a large negative change.** When `current <
//!   previous` on a cumulative level the sequence restarted; no derived value is
//!   emitted and the next observation establishes the new predecessor.
//! * **A genuine zero is a measurement.** `current == previous` emits `0`, and
//!   that zero is distinguishable from "no predecessor" because it is present at
//!   all.

use wse_model::DerivationKind;

/// The outcome of evaluating a derivation against a predecessor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Derived {
    /// A derived value was computed.
    Value(f64),
    /// No predecessor exists, so nothing can be computed. Never zero.
    NoPredecessor,
    /// The cumulative sequence went backwards. No value is emitted; the next
    /// observation becomes the new predecessor.
    Reset,
}

impl Derived {
    /// The computed value, if there is one.
    pub fn value(self) -> Option<f64> {
        match self {
            Derived::Value(v) => Some(v),
            Derived::NoPredecessor | Derived::Reset => None,
        }
    }
}

/// Evaluate `kind` for `current` against an optional `previous` value.
///
/// `previous` is `None` when the series has no comparable earlier point yet.
/// This is the only place the arithmetic lives; callers must not re-implement
/// it, so the reset and missing-predecessor rules cannot drift apart between
/// the live path and a test.
pub fn evaluate(kind: DerivationKind, previous: Option<f64>, current: f64) -> Derived {
    let Some(previous) = previous else {
        return Derived::NoPredecessor;
    };
    match kind {
        DerivationKind::Delta => {
            if current < previous {
                // A cumulative counter that went backwards is a reset (or a
                // correction), not a collapse. Manufacturing `current - previous`
                // here would invent a huge negative anomaly.
                Derived::Reset
            } else {
                Derived::Value(current - previous)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_predecessor_emits_nothing() {
        assert_eq!(
            evaluate(DerivationKind::Delta, None, 100.0),
            Derived::NoPredecessor
        );
        assert_eq!(evaluate(DerivationKind::Delta, None, 100.0).value(), None);
    }

    #[test]
    fn a_normal_increase_is_the_difference() {
        assert_eq!(
            evaluate(DerivationKind::Delta, Some(100.0), 107.0),
            Derived::Value(7.0)
        );
    }

    #[test]
    fn an_unchanged_value_is_a_genuine_zero() {
        let zero = evaluate(DerivationKind::Delta, Some(100.0), 100.0);
        assert_eq!(zero, Derived::Value(0.0));
        assert_ne!(
            zero,
            evaluate(DerivationKind::Delta, None, 100.0),
            "a real zero must be distinguishable from a missing predecessor"
        );
    }

    #[test]
    fn a_decrease_is_a_reset_not_a_negative_delta() {
        assert_eq!(
            evaluate(DerivationKind::Delta, Some(107.0), 20.0),
            Derived::Reset
        );
        assert_eq!(
            evaluate(DerivationKind::Delta, Some(107.0), 20.0).value(),
            None
        );
    }
}
