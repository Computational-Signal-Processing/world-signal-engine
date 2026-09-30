# 0007 — Event ids are derived, not random

**Status:** accepted

## Context

`Event::new` generated a random UUID for the event id. `Signal::stable_id` is
built on top of it:

```rust
pub fn stable_id(event_id: &EventId, series_key: &str, direction: CandidateDirection) -> SignalId
```

with a doc comment promising that a deterministic id "means re-forming the same
signal after a restart updates the existing record instead of creating a
duplicate."

That promise was false. The signal id was deterministic *given the event id*,
but the event id was random, so re-forming the same signal produced a different
signal id every time. The bug was invisible while every run was a live run with
wall-clock timestamps: nothing was ever replayed, so nothing was ever compared.
Replay made it immediate — two runs over the same stream produced different
signal ids, and `backtest_is_deterministic` failed.

## Decision

`Event::new_for(group_key, title, at)` derives the id from the group key and the
event's start time:

```rust
EventId::new(format!("evt_{}", fnv1a_hex(&format!("{}|{}", group_key, at.to_rfc3339()))))
```

`Event::new` is retained with a random id for records that are not expected to
recur, and is documented as such. The pipeline uses `new_for`.

## Consequences

- Replaying the same stream reproduces the same event and signal ids. A replay
  updates existing records instead of minting new ones, which is what the
  `stable_id` doc comment always claimed.
- `backtest` reports are diffable between runs and safe to assert on in CI.
- Two genuinely distinct events that start at the same instant under the same
  grouping key would collide. They cannot be distinguished by content, so an
  identical id is the honest answer rather than an accident.
