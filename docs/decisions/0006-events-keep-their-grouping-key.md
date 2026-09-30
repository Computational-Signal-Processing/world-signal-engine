# 0006 — Events keep their grouping key

**Status:** accepted

## Context

The event engine groups anomaly candidates by a key: `entity:…` when the
candidate names an entity, `series:…` otherwise. To decide whether a new
candidate belongs to an existing event, it compared that key against
`event_group_key(event)`, which was *re-derived* from the event's accumulated
state:

```rust
match event.entities.first() {
    Some(entity) => format!("entity:{}", entity.as_str()),
    None => event.anomalies.first().map(|a| format!("anomaly:{}", a.as_str()))…,
}
```

Two things are wrong with that. The fallback produces `anomaly:…` for a
candidate whose key was `series:…`, so the two never match. And the key depends
on which of `entities` / `anomalies` happens to be populated and in what order.

The result: an event with an entity groups under `entity:…`, a candidate for the
same series groups under `series:…`, they never merge, and a new event is
started every cycle for one ongoing change. Because each new event had a random
id, this was also invisible — nothing could be compared across cycles.

## Decision

The grouping key is computed once, when the event is created, and stored on the
event as `Event::group_key`. `event_group_key` returns it directly, falling back
to the old derivation only for records built by `Event::new` (tests and
hand-built data).

## Consequences

- One ongoing change is one event. An event accumulates observations across
  cycles instead of being replaced each cycle, so `duration_seconds` and
  `observation_count` mean what they say.
- Replay is reproducible, because the key no longer depends on vector order.
- `Event::group_key` is part of the serialized record; a stream or store written
  before this change reads back with an empty key and uses the fallback.
