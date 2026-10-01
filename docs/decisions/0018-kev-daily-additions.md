# 0018 — CISA `kev_added`: a non-overlapping daily count

**Status:** accepted

## Context

`cisa_kev` emitted `kev_added` as the number of vulnerabilities added in a
**trailing seven-day window**, sampled once a day. The semantic audit flagged
this (risk #5, remediation F5): consecutive daily points share six of seven days,
so the series is heavily autocorrelated and a daily z-score on it is structurally
misleading — a day that is genuinely flat can still look like a rise because the
window has not yet rolled off, and a real one-day batch is smeared across the
next six points.

`docs/decisions/0015-cisa-derived-growth.md` left F5 open and named the shape it
would need — a `WindowCount` derivation kind — without committing to it. The
F5 note in `0015` is now superseded: the problem does not need a new derivation
kind at all.

## Decision

**Emit `kev_added` as the additions dated to the collection day.** The count is
over the entries whose `dateAdded` equals the poll's UTC day:

- metric `kev_added`, unit `vulnerabilities`, entity `cyber_kev`, unchanged;
- `observed_at` is the poll time (the day's boundary is carried as the `day`
  attribute and as the record key);
- consecutive daily polls share **no** members, so a daily z-score is now
  meaningful and no detector change is required.

The record key is the day (`with_record_key(day)`), so re-polling the same day
keeps one identity and a new day is a new record.

### Why not a `WindowCount` derivation kind

`WindowCount` was the originally imagined fix: keep the overlapping window and
teach the engine to account for the overlap. That is a detection-core change
(new derivation kind, overlap-aware predecessor lookup) to *recover* a daily
series from a window. The collector can simply emit the daily series directly —
`dateAdded` is the authoritative day — which is simpler and keeps the core
untouched. A `WindowCount` kind may still be worth adding if a future source can
only expose a window; it is not needed here.

### `kev_added` and `kev_catalog_growth`

These are now two independent measurements of the same quantity: additions per
day.

- `kev_catalog_growth = Delta(kev_catalog_total)` is the interval difference of
  the catalogue size (CAP-2B, `0015`);
- `kev_added` is the count of entries dated to the day.

They should agree, and where they diverge that is itself informative (a
backdated edit, a revised `dateAdded`). Both remain detection series.

## Consequences

- No detector, model, or engine change: only the collector and its docs.
- The `recent_days` source parameter is removed; it no longer describes the
  emitted series.
- The collection-day count is complete for a once-daily poll that runs after the
  day's additions have landed. A poll that runs early would count a partial day;
  the shipped schedule runs daily, and a late addition is counted by the poll on
  the day it is dated.
- A day with no additions is emitted as `0` — a real measurement, distinct from
  a collector failure (no data), matching the absence rule elsewhere.

## Verification

`crates/sources/src/cisa_kev.rs` unit tests: the fixture yields one addition on
its collection day (not the seven-day total); only the collection day counts;
a quiet day is a genuine `0`; and the day is the record key (a re-poll keeps its
id, the next day is a new record). The F5 spec in
`crates/sources/tests/semantic_regression.rs` drives the shipped collector
through the fixture and fails against the old trailing-window code.
