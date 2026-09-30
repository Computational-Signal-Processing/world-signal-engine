# 0003 — Retain raw payloads, keyed by content hash

**Status:** accepted

## Context

The drill-down ends at `SIGNAL → EVENT → OBSERVATION → SOURCE → RAW DATA`. The
observation model already carried a `RawReference` (locator, hash, content type,
size), and it was tempting to treat that as sufficient: the locator is a URL, and
the raw data can be fetched from it when someone asks.

## Decision

The raw bytes are retained, keyed by the same content hash the observation's
reference carries.

## Consequences

- The last step of the drill-down is real. A user asking "show me the evidence"
  gets the bytes the source actually returned, not a URL that may since have
  changed, expired, rate-limited us, or started returning something different.
- Evidence is stable over time. This matters for a system whose whole purpose is
  historical context: a signal from three weeks ago must still be checkable
  against what was seen then.
- Memory grows with retained payloads, so re-storing identical bytes is a no-op
  and payloads are addressed by hash rather than appended.
- The hash in the reference and the hash of the retained body must agree, or the
  lookup dead-ends. Where a producer builds both — as the synthetic collector
  does — they are derived from one shared function so they cannot drift. A
  mismatch is rejected at store time rather than accepted and discovered later.
- When a payload genuinely was not retained, the API answers with the reference
  and an explicit "not retained" rather than a bare 404, because the reference is
  still meaningful.
