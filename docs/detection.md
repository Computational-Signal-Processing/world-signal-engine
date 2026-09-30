# Detection

Detection turns a time series into `AnomalyCandidate`s. It never produces a
signal; that decision belongs to the signal engine.

```text
time series → baseline → change → anomaly / early signal → AnomalyCandidate
```

## Baselines

A baseline is what normal looks like for a series *at the moment of measurement*.
It is computed from the values before the point being judged, so the point cannot
hide inside its own reference.

`BaselineSnapshot` carries:

```text
sample_size, mean, median, std_dev, mad
p05, p95
ewma
trend_per_second
volatility
```

The statistics available (`wse-baseline`):

| Method | Purpose |
| --- | --- |
| Rolling mean | Central tendency, sensitive to outliers. |
| Rolling median | Central tendency, robust. |
| Standard deviation | Spread, sensitive to outliers. |
| MAD | Median absolute deviation — the robust spread. |
| EWMA | Recent-weighted level, tracks regime changes. |
| Percentiles (`p05`, `p95`) | Distribution shape, without assuming normality. |
| Rate of change | Velocity. |
| Volatility | How much the series moves per step. |

No ML model is needed for the MVP, and none is used.

## Change detection

Comparing absolute values is not detection:

```text
current = 105
previous = 100
```

That is a change. Whether it matters depends on the series. The engine computes:

```text
absolute_change
relative_change
velocity
acceleration
deviation_from_baseline
persistence
frequency
```

`DetectorConfig` sets the thresholds:

```text
min_samples                    20    samples before any judgement
robust_z_threshold             3.5
z_threshold                    3.0
relative_change_threshold      0.05  (5%)
absolute_change_threshold      0.0
velocity_sigma_threshold       3.0
```

## Anomaly detection

Both a robust z-score and a classical z-score are computed for the latest point
against its baseline:

```text
robust    = 0.6745 * (x - median) / MAD
classical = (x - mean) / std_dev
```

The larger magnitude wins, and the candidate records which method produced it
(`RobustZScore` or `ZScore`). Robust z-score is preferred because real-world
feeds contain outliers that would otherwise inflate `std_dev` and mask genuine
deviations.

### The flat-baseline case

If MAD is zero the history is perfectly flat. Any movement is then infinitely
surprising, which is not a useful score, so the detector falls back to the
classical z-score and lets that decide.

### The output is a candidate

An anomaly produces an `AnomalyCandidate`, carrying:

```text
baseline, current, deviation, score, method, kind
direction, duration_seconds, confidence
```

Nothing here decides the candidate matters. That is deliberate: a detector
threshold is not the same decision as "a human should look at this".

## Early-signal detection

The case the rest of the system would miss:

```text
Day 1  +0.3σ
Day 2  +0.5σ
Day 3  +0.8σ
Day 4  +1.2σ
Day 5  +1.6σ
```

No single point is a large anomaly. The combination of a small deviation that is
persistent, directional and accelerating is the signal.

The detector looks for a trailing run of points, each of which is:

- **persistent** — at least `early_signal_min_points` consecutive points,
- **directional** — all in the same direction,
- **meaningful** — at least `early_signal_min_sigma` in magnitude,
- **accelerating** — within `early_signal_accel_tolerance`,
- **sustained** — spanning at least `early_signal_min_duration_seconds`.

```text
early_signal_min_points             4
early_signal_min_sigma              0.4
early_signal_min_duration_seconds   3 days
early_signal_accel_tolerance        0.05
```

### Why the baseline excludes the run

The run being hunted is compared against a baseline drawn from *before* it. If
the baseline included the run, a slow drift would raise its own reference and
hide itself.

### Growing the probe

A drift longer than the initial probe window still has to be captured. The probe
starts at a quarter of the window and doubles while the trailing run fills it
entirely — a sign the run probably started earlier — up to a bounded number of
attempts.

### Why the run threshold exists

Points below `RUN_MEMBERSHIP_SIGMA` (0.1σ) are indistinguishable from baseline
noise. Including them would let a flat series masquerade as a persistent trend,
so a run ends at the first such point.

## Synthetic configurations

`DetectorConfig::synthetic()` shortens the persistence windows and sample
minimums, for dense, fast synthetic and high-frequency series. It is what makes
the pipeline visible in seconds during `demo` and `serve --synthetic`.

This is a demonstration configuration. It is permissive on purpose and makes no
claims about the real world.

## Replay

Detection is deterministic given a series, which is what makes replay possible:
feed a historical stream through the same code path as though it were arriving
now, and compare the signals against what actually happened. That is how false
positives, false negatives and detection latency get measured rather than
guessed.
