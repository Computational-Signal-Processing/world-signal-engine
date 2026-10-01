# 0019 — Crossref: measure one completed day, not a window ending today

**Status:** accepted

## Context

`crossref_works` counted works registered in a **trailing two-day window**
(`from-created-date: now-2d`, `until-created-date: today`), sampled once a day.
The semantic audit flagged two structural problems (risk #7, remediation F7):

- **Window overlap.** A daily poll over a two-day window shares one day with the
  previous poll, so consecutive values are autocorrelated and a daily z-score is
  noisy (weaker than KEV's seven-day overlap, but real).
- **The newest point is partially deposited.** The window ends *today*, and
  Crossref deposits lag; the most recent day's registrations are still arriving
  when the poll runs, so the latest point is structurally depressed. A "drop" is
  an artifact of deposit lag, not a change in scholarly output.

## Decision

**Measure one completed day per poll: the day before collection.**

- `window(now)` returns `(yesterday, yesterday)`; because Crossref's
  `until-created-date` is exclusive, `[from, until)` is exactly that one day.
- The window is stable across the collection day, so a re-poll measures the same
  day.
- The observation's `observed_at` is the measured day's UTC midnight (not the
  poll time), and the day is the record key, so a re-poll of the same day keeps
  one identity and de-duplicates.
- The measured day is carried as the `day` attribute for drill-down.

This removes both defects at once: consecutive daily polls share no day (no
overlap), and a completed day is fully deposited (no partial latest point). No
detector change is required — the series is now a genuine, non-overlapping daily
count, so a daily z-score is meaningful.

### Why not a window-overlap-aware baseline

The audit's constraint suggested a "window-overlap-aware baseline". As with
F5/`kev_added` (`0018`), the overlap does not need to be modelled in the
detector: the collector can simply measure a non-overlapping period. That keeps
the detection core unchanged.

## Consequences

- `WINDOW_DAYS` is `1`; the `window_days` source parameter reflects that.
- Each observation represents "works registered on day X", not "in the last two
  days" — the vocabulary and audit wording are updated to match.
- The count is still *registration*, not *publication*, and the query is still a
  fuzzy `query.bibliographic` match; those caveats are unchanged.
- A day with no registrations is emitted as `0` — a real measurement, distinct
  from a collector failure.

## Verification

`crates/sources/src/crossref.rs` unit tests: the window is a single day equal to
yesterday and never today; it is stable across the collection day; and a re-poll
of the same day keeps its id while the next day is a new record. The F7 spec in
`crates/sources/tests/semantic_regression.rs` drives the shipped helpers and
fails against the old two-day-window code.
