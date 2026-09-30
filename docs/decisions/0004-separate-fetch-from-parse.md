# 0004 — Separate fetch from parse in collectors

**Status:** accepted

## Context

A collector could be written as one function that fetches a URL and returns
observations. That is fewer moving parts, and it is how most of these are
written.

## Decision

Each source module provides a pure `parse(bytes, received_at) -> Vec<Observation>`
and a separate `Collector` that fetches the bytes and calls it.

## Consequences

- `parse` is fully unit-testable against a checked-in fixture, with no network.
  The test suite runs offline, which keeps it fast and reliable in CI.
- The same parsing code path is exercised in live and replay mode, so a replay
  test is genuinely testing what production runs.
- Fixtures double as documentation of the source's actual response shape,
  including the awkward parts — the GitHub search envelope, GDELT's plain-text
  rate-limit notice.
- `parse` must reject malformed input explicitly rather than returning an empty
  list. This is enforced by review and tested, because returning empty on a bad
  response is indistinguishable from "the world reported nothing" — precisely the
  confusion the engine exists to prevent.
- A small amount of ceremony per source: two entry points instead of one. This is
  the price of offline testability and is judged worth it.
