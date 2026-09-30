# Architecture

## The pipeline

```text
SOURCE CATALOG
      ↓
COLLECTOR            fetch, de-duplicate, retain raw bytes
      ↓
NORMALIZATION        source-specific shape → Observation
      ↓
OBSERVATION          one measurement, at one time, from one source
      ↓
TIME SERIES          observations grouped by series key
      ↓
BASELINE             what normal looks like for that series
      ↓
CHANGE DETECTION     absolute, relative, velocity, acceleration
      ↓
ANOMALY DETECTION    AnomalyCandidate (never a Signal)
      ↓
EVENT FORMATION      candidates grouped by entity, window, direction
      ↓
ENTITY / CONTEXT LINKING
      ↓
CORRELATION          independent sources pointing at the same change
      ↓
SIGNAL FORMATION     what a human should look at
      ↓
LENS / RELEVANCE     visibility, not storage
      ↓
API / WEB UI
      ↓
DRILL DOWN           signal → event → observation → source → raw data
```

Each arrow is a real boundary in the code. A detector cannot see a `Source`; a
collector cannot see a baseline. That keeps new sources and new detectors from
reaching into each other.

## Crates

The workspace is split so each stage can be tested on its own.

| Crate | Responsibility |
| --- | --- |
| `wse-model` | The vocabulary: `Source`, `Observation`, `Entity`, `Event`, `AnomalyCandidate`, `Signal`, `Lens`, `Quality`, `RawReference`. No I/O, no detection. |
| `wse-collector` | The `Collector` contract, `CollectionResult`, `RawPayload`, and the deterministic `SyntheticCollector`/`SyntheticWorld`. |
| `wse-sources` | The real collectors and the source catalog. Each source is one module. |
| `wse-normalize` | Turning source-specific payloads into observations, plus data-quality helpers. |
| `wse-storage` | Storage traits (`ObservationStore`, `EventStore`, `SignalStore`, `SourceStore`, `BaselineStore`) and the in-memory implementation, including `RawStore`. |
| `wse-baseline` | Rolling and robust statistics: mean, median, standard deviation, MAD, EWMA, percentiles, rate of change. |
| `wse-detection` | Change, anomaly and early-signal detection. Produces candidates. |
| `wse-correlation` | Cross-source convergence detection. |
| `wse-signals` | Event formation, signal generation, lifecycle, quality dimensions. |
| `wse-presentation` | The human-language layer: turning a signal's machine facts into a readable narrative (headline, what changed, magnitude, unknowns) and its lifecycle status. No detection, no I/O. |
| `wse-engine` | Wires the above into one cycle: `Engine::run_collector`, `ingest_observations`. |
| `wse-scheduler` | Collection scheduling and the live/replay clocks. |
| `wse-api` | The REST API and the static web UI. |
| `wse-cli` | The `wse` binary. |

## One cycle

`Engine::run_collector` is the whole system in miniature:

1. The collector fetches and returns a `CollectionResult`.
2. On failure, source health is updated and **no observation is fabricated**.
3. Raw payloads are retained in the `RawStore` keyed by content hash.
4. Observations already seen are dropped as duplicates.
5. New observations are pushed into their series tracker.
6. The baseline *before* the new point is computed and stored.
7. Anomaly and early-signal detectors turn the tracker into candidates.
8. Candidates are grouped into events.
9. Convergence is evaluated across the candidate set.
10. Signals are formed from events, candidates and convergence groups.

If a step produces nothing, the cycle ends there and reports zero. It does not
invent activity.

## Storage

Storage is behind traits so the engine is never locked to one database. The MVP
ships an in-memory implementation that is genuinely sufficient: it keeps each
series sorted by observation time so range queries and "latest" lookups are
correct even when data arrives out of order.

`RawStore` exists because the drill-down ends at `RAW DATA`. Observations carry a
`RawReference` (locator, content hash, content type, size); the payload itself is
retained under that hash. Without it, "show me the evidence" would be a promise
to re-fetch a URL that may since have changed.

## Failure isolation

A collector failure and a source reporting zero are different facts and are
stored as such:

- `collector_failure_total` increments, `SourceHealth` records the error, and
  `source_failed` is set on the outcome.
- No observation, anomaly, event or signal is created from the failure.

A detector that produces nothing likewise stops at that point. One broken source
never stops the pipeline; the other collectors run in the same cycle regardless.

## Observability

The engine counts what it does, and `/metrics` renders it in Prometheus text
format:

```text
wse_sources_registered
wse_collector_success_total
wse_collector_failure_total
wse_collector_latency_ms
wse_observations_total
wse_observations_duplicate_total
wse_anomalies_total
wse_events_total
wse_signals_total
wse_signal_types_total{type="..."}
```

## Modes

- **LIVE** — collectors run against their real endpoints.
- **REPLAY** — a synthetic or historical stream is fed through the same pipeline
  as though it were arriving now. Replay exists so the detection engine can be
  backtested and so false positives, false negatives and detection latency can
  actually be measured.
