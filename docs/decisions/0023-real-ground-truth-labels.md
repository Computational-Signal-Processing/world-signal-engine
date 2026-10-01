# 0023 — Ground truth for the real sources is the world, not the detector

Status: accepted (2026-10-01)

## Context

Phase 10 built backtesting so that false positives, false negatives, detection
latency and persistence could be *measured* rather than eyeballed. The mechanism
shipped, but its only ground truth was the synthetic world's scripted changes —
drifts and spikes the test itself injected. That is circular: a detector scored
against changes it was tuned to find cannot reveal that it misses real ones or
invents fake ones. The reality audit recorded the consequence as finding 10:
"no labelled ground truth for real sources; backtest precision/recall is
unlabelled", and the roadmap named widening the labels as the next step.

The temptation is to make the labels by running the detector and calling what it
finds "events". That would launder the detector's own errors into truth and
produce a precision of 1.0 that means nothing.

## Decision

Ground truth for a real source is read out of the world's own record, before and
independently of the detector, and checked in as a fixture.

For the first real labels we use the USGS earthquake catalog. The checked-in
`tests/fixtures/usgs_9mo_2026.geojson` is the **real feed** over
2026-01-01..2026-09-30 for M5.0+ (1494 events), trimmed to the fields the
normalizer reads and fetched directly from the USGS FDSN API. The ten M7.0+
earthquakes in that window are the labels
(`tests/fixtures/usgs_m7_labels_2026.json`); a magnitude threshold is an
objective, source-defined criterion, not a judgment about what the detector did.

Three rules keep the labels honest:

1. **Written by reading the catalog, not by running the engine.** If a label and
   a detection ever disagree, the label is what the world says, and the detector
   is what is wrong.
2. **The stream is reconstructed the way a live poller would have seen it.** A
   quake arrives at the first hourly poll after it occurred, so the detector
   sees real arrival batching (many quakes per cycle), not one observation per
   event. Feeding a perfectly de-batched stream would flatter the detector.
3. **The numbers are asserted as measurements, not targets.** The test pins the
   current precision and recall so drift is visible and reviewed. It does not
   assert that they are good.

## What the measurement says

Over the nine months, with the production detector profile:

| metric | value |
| --- | --- |
| labelled events | 10 (M7.0+) |
| signals emitted | 25 |
| matched | 5 |
| recall | 0.50 |
| precision | 0.20 |

Half of the M7+ events were detected, and four unlabelled signals were emitted
for each labelled one that was. This is a real, unflattering result and is the
point: the detector now has a number to improve, computed against reality.

The misses are informative. Several labelled days had a real M7+ quake that was
**not** an outlier against that region's own M5+ distribution — a single M7.4 in
a region that routinely sees M5–6 is not a 3.5σ deviation in *magnitude*. The
detector is asking the right question of the wrong quantity for those events.
That is a detection-design question for later phases, recorded here rather than
hidden by moving the label.

## Consequences

- `crates/engine/tests/real_backtest.rs` scores the detector against the real,
  hand-checked history and proves the measurement is deterministic.
- The USGS catalog fixture is ~400 KB of checked-in real data. It is the price of
  measuring against reality; the alternative (fetch at test time) would make the
  suite depend on the network and the clock.
- Precision and recall for the real sources are no longer "unlabelled" — reality
  audit finding 10 is closed. Other sources still have no labels; the same
  method (source-defined, objective, read before the detector) is how they get
  them.
