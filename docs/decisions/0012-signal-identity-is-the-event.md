# 0012 — A signal's identity is its event, not its series

**Status:** accepted

## Context

`Signal::stable_id` hashed three things together:

```rust
fnv1a_hex(&format!("{}|{}|{}", event_id, series_key, direction))
```

The `series_key` came from `dominant_series`, which picks the candidate the event
saw *most recently*. That is a fine choice for display, but it is not stable.

An event accumulates candidates from several series as independent sources
converge on one entity. Whichever source fired last becomes "dominant", so the
series can differ from one cycle to the next:

```text
cycle 1: candidate on a::ent_x::m1  ->  series_key = a::ent_x::m1
cycle 2: candidate on b::ent_x::m2  ->  series_key = b::ent_x::m2   (same event)
```

Because the signal id was keyed on the series, the two cycles produced two
different ids. The merge in `Engine::ingest_observations` matches by id, so it
missed, and the engine stored a **second** signal for one ongoing change while
the first froze — exactly the "a new signal every minute" failure the design
warns against, and the same class of bug as 0006 (event key) and 0007 (event
id), one layer up.

## Decision

The signal id is derived from the event and the direction only:

```rust
pub fn stable_id(event_id: &EventId, direction: CandidateDirection) -> SignalId {
    let hash = fnv1a_hex(&format!("{}|{}", event_id.as_str(), direction.as_str()));
    SignalId::new(format!("sig_{hash}"))
}
```

The event is the thing that persists across cycles, and 0007 already made its id
deterministic. `series_key` stays on the signal, but only as display — it names
the series a reader is looking at, not the signal's identity.

## Consequences

- One ongoing change is one stored signal. A converging event no longer forks
  into a second signal when a different source becomes the most recent.
- The signal id no longer depends on candidate arrival order within a cycle, so
  it is stable under replay as well.
- Two distinct changes that share an event id and direction still share a signal.
  That is correct: they are the same event, which is the unit of "one change".
- `Signal::series_key` can change over a signal's life while its id does not.
  This is intentional; consumers that keyed on the series for grouping must key
  on `id`/`event_id` instead.
