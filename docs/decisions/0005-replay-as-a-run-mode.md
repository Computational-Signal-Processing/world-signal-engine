# 0005 — Replay is a first-class run mode, not a test helper

**Status:** accepted

## Context

The brief requires two run modes, `LIVE` and `REPLAY`, and explains why:
without replaying real historical data there is no way to measure false
positives, false negatives, detection latency or signal persistence. A detector
that has only ever been watched by eye has not been measured.

The tempting shortcut is to make replay a test-only concern: build the engine
against `Utc::now()` and let tests fake time some other way. That shortcut is
what the brief warns against, and it has a concrete cost — the detector would
be exercised under a clock that never matches the data it is scoring.

## Decision

The engine takes an injectable clock. `Engine::with_clock` accepts any
`Clock`, and the scheduler's `SharedReplayClock` is a shareable, advanceable
clock that the replay driver pins to each arrival batch's `received_at`.

Replay is grouped by `received_at`, not by `observed_at`. Observations that
arrived together are fed to the pipeline in the same cycle, so the detector
sees the same batches, in the same order, that live collection produced.
Detection latency is then `signal.first_seen - earliest evidence observed_at`,
which is measurable without any labels.

Captured streams are newline-delimited JSON: a `header` record then one
`observation` per line. The format is deliberately plain so `jq`, `grep` and
`wc -l` work on it, and an observation can be read without the engine.

## Consequences

- Replay is deterministic. The same stream and the same configuration produce
  byte-identical reports, including signal ids. `backtest` is safe to run in CI
  and to diff between runs.
- Precision and recall are only reported when labels are supplied. An unlabelled
  run still reports latency and persistence, but withholds precision rather than
  inventing a number — an unlabelled run cannot distinguish "wrong" from "not
  yet known to be right".
- The clock injection surfaced two latent identity bugs (see ADR 0006 and 0007)
  that were invisible while every run used the wall clock.
