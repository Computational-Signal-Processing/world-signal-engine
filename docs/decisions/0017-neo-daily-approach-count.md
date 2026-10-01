# 0017 — NASA NEO: a daily approach count, not a pooled distance

**Status:** accepted

## Context

The `nasa_neo` collector emitted one observation per close approach, as a miss
distance in km, all on the single entity `neo_class_all` and the metric
`neo_miss_distance`. The semantic audit called the result incoherent (risk #4,
remediation F4):

- The entity collapses **every** object into one series, so the series
  interleaves the miss distances of unrelated rocks. A change in it is not a
  physical change in anything.
- The detector's deviation direction is **symmetric**, so a *far* pass — a rock
  that happened to miss by a wide margin — reads as an anomaly. The interesting
  values are the *near* ones, so the detector is pointed the wrong way.
- Individual objects never recur, so a per-object series could never accumulate
  a baseline.

The declared intent in the module doc was "a change in the rate of close
approaches". The code never computed a rate.

## Decision

**The coherent series is the daily count of close approaches.** One observation
per UTC day that appears in the feed:

- metric `neo_close_approaches`, unit `approaches`, entity `neo_class_all`
  (the population, which is the right subject for a rate);
- value is the number of approaches the feed records that day;
- `observed_at` is that day's UTC midnight, and the day is the **record key**,
  so a day retained across polls keeps one identity and de-duplicates;
- the day's **closest object** (id, name, distance, hazardous flag, velocity)
  is carried as attributes, so the interesting near miss is still
  drill-downable without making it the series.

This keeps `measurement: stable_series` and requires **no engine change**: a
per-day count is exactly the kind of quantity `stable_series` means, so the
detector now runs in the correct (positive) direction on a coherent series.

### Zero-fill, and the absence rule

A day with no approaches is emitted as `0`, not omitted — the same reasoning as
EONET's per-category counts: "nothing came close today" is a measurement the
baseline needs, and a source that only reports busy days would bias the mean
upward. This is distinct from a **collector failure**, which is not a zero: the
engine's source-health path returns early and never feeds zeros into detection
(`docs/decisions` on source health). A missing feed is no data; an empty day is
data.

## Consequences

- The entity declared in the catalog is now `neo_class_all` (it previously said
  `near_earth_object`, which never matched what the collector emitted — a latent
  contradiction fixed in passing).
- The `neo_miss_distance` vocabulary is replaced by `neo_close_approaches`; the
  vocabulary completeness test is updated, so a collector emitting an unmapped
  metric still fails loudly.
- The feed's window is fixed at seven days (`start_date = today - 6`), so the
  distinct day keys are stable across polls; today's partial count is a known
  edge (a day's approaches can still arrive later), which the baseline absorbs
  as normal low-end noise rather than a special case.

## Limits

The count treats every entry in the close-approach feed as one approach. A
finer series (e.g. approaches below a distance threshold) would need a threshold
the source does not declare; the raw distances remain available in the payload
and as the closest-object attribute for anyone who wants to re-derive one.

## Verification

`crates/sources/src/nasa.rs` unit tests: the fixture spans two UTC days and
yields two observations (counts `2` and `1`); every day shares one series; the
closest object is preserved for drill-down; and a day retained across polls
keeps its id. `crates/sources/src/collectors.rs` drives the collector through
its fixture transport. The old tests asserting one observation per approach and
a `neo_miss_distance` metric were removed because they encoded the bug.
