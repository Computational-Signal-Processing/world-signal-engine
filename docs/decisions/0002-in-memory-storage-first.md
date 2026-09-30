# 0002 — In-memory storage first

**Status:** accepted

## Context

The engine needs time-series storage with historical queries. The obvious move is
to pick a database — a time-series database, or Postgres with an extension — and
build against it.

## Decision

Storage is defined as traits (`ObservationStore`, `EventStore`, `SignalStore`,
`SourceStore`, `BaselineStore`) and the first implementation is in memory. The
application code depends on the traits, never on a specific database.

## Consequences

- The whole pipeline runs and is tested on a single machine with zero operational
  cost, which is what the MVP needs.
- The engine is not locked to one database; a persistent backend can be added
  without touching detection, signals or the API.
- The trait set is forced to be honest, because the in-memory implementation
  cannot paper over a badly shaped interface with a query language.
- Data does not survive a restart yet. This is acceptable for the current stage
  and is the main thing a persistent backend will fix.
- Range queries are handled by keeping each series sorted by observation time, so
  "latest" and windowed lookups are correct even when data arrives out of order.
