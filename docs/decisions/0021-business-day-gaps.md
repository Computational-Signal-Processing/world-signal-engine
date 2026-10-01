# 0021 — Business-day gaps are absence, not zero (ECB)

**Status:** accepted

## Context

`ecb_exchange_rates` tracks the USD/EUR reference rate, which the ECB publishes
on TARGET business days only. Weekends and holidays are **absent** from the
series, not zero.

`docs/source-semantic-audit.md` (section 10, F8) flagged the risk that this gap
"reads as staleness" and that the missing days are treated as flat. The
cross-cutting rule from the brief is explicit: *a collector failing, or a source
not publishing, must never be read as "world activity = 0"*.

## Decision

**No code change. Prove the guarantee with a regression test.**

The concern was that a time-elapsed baseline would misread the gap. Investigation
shows detection is **not** time-based:

- the rolling window is trimmed by *count* (`min_samples`) and age, and its
  statistics (`mean`, `median`, `std_dev`, `mad`, z-scores) are computed over the
  values in the window, with no term that scales a deviation by elapsed time;
- `trend_per_second` exists in the snapshot but no detector branches on it;
- the ECB collector emits an observation **only** for days the API returns, so a
  missing day inserts nothing — there is no synthetic zero anywhere in the path.

A normal move across a weekend therefore produces no anomaly, and a genuinely
large move is still caught.

Rather than add a gap-aware baseline the engine does not need, the guarantee is
locked by `crates/engine/tests/business_day_gaps.rs`, which drives the real
engine over a business-day-only series:

1. a normal move across a weekend gap yields **no** anomaly candidate;
2. a genuinely large move across the same gap **is** detected (non-vacuous);
3. a missing day stores **no** observation — and a zero is stored only when a
   collector actually supplies one, so "no data" can never become "data = 0".

## Consequences

- ECB keeps `DETECTABLE_WITH_CONSTRAINTS` in the audit only because of the
  shared id contract (F1, now done); gap handling needs no further work.
- If a future detector introduces a time-based rate (e.g. velocity in units per
  second), it must handle gaps explicitly — the test above is the guard that
  will fail first if a gap starts to matter.

## Verification

`crates/engine/tests/business_day_gaps.rs` (3 tests) passes on the real engine.
The non-vacuity of the "absence, not zero" claim is shown by supplying a real
zero for the missing day and observing that the store does record it.
