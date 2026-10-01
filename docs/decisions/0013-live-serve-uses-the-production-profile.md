# 0013 — A live serve runs the production detector profile

**Status:** accepted

## Context

`wse serve` built its engine from `synthetic_engine_config()`:

```rust
EngineConfig {
    detector: DetectorConfig::synthetic(),   // min_samples = 10, early duration = 0
    signal: SignalConfig { now_window_seconds: i64::MAX, .. },
    ..
}
```

The name says "synthetic", and the synthetic world is what `--synthetic` drives,
but the function was used for *every* served instance — so the live product ran
the demo calibration. The two profiles differ where it matters:

| | synthetic | production |
| --- | --- | --- |
| `min_samples` | 10 | 20 |
| `early_signal_min_duration_seconds` | 0 | 3 days |
| `now_window_seconds` | `i64::MAX` | 30 min |

The synthetic thresholds are tuned for the dense, well-behaved series the demo
world emits. On a real feed they judge deviation from too little history, let an
"early signal" fire with no required persistence span, and — because the NOW
window was infinite — rendered every signal as NOW. The world would be reported
as if it were the demo world. The mistake was invisible in tests because every
test builds its own `EngineConfig`; only the served binary picked the demo one.

## Decision

The served engine's configuration is chosen in one place, `serve_engine_config`:

```rust
fn serve_engine_config(synthetic: bool) -> EngineConfig {
    if synthetic { return synthetic_engine_config(); }
    EngineConfig { detector: DetectorConfig::default(), signal: SignalConfig::default(), .. }
}
```

Both serve paths (in-memory and SQLite) call it, so they cannot drift. A normal
`serve` is a real-world monitor and uses the production profile; `--synthetic`
drives the demo world and keeps the synthetic profile. `demo` and `replay` keep
using `synthetic_engine_config()` unchanged, and `collect`, `replay-stream` and
`backtest` keep using `EngineConfig::default()` unchanged.

## Consequences

- A live serve detects on the production thresholds. The product claim — a
  continuously running real-world detector — matches what actually runs.
- `--synthetic` is the only path to the demo calibration while serving, and it
  is the path that also registers the synthetic world.
- The regression test `a_live_serve_uses_the_production_detector_profile` calls
  the same selection function the binary uses and asserts the production values,
  so a silent revert fails in CI rather than shipping.
