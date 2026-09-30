# Roadmap

The order below is the order work happens in. Each phase ends with a working
vertical slice and tests that prove it, not with a claim that it is done.

## Done

### Phase 0 — Repository bootstrap
Workspace, crates, MSRV, license, formatting, linting, tests.

### Phase 1 — Core domain model
`Source`, `Observation`, `Entity`, `Event`, `AnomalyCandidate`, `Signal`, `Lens`,
`Quality`, `RawReference`, with serde and unit tests.

### Phase 2 — Synthetic world
`SyntheticCollector` / `SyntheticWorld`, driving the whole pipeline with no real
sources. This is the acceptance harness for everything after it.

### Phase 3 — Storage
`ObservationStore`, `EventStore`, `SignalStore`, `SourceStore`, `BaselineStore`
as traits, plus the in-memory implementation and historical range queries.

### Phase 4 — Baseline + detection
Rolling mean/median, standard deviation, MAD, z-score, robust z-score, EWMA,
percentiles, rate of change, persistence.

### Phase 5 — Event engine
Candidates grouped by entity, window and direction, with lifecycle.

### Phase 6 — Signal engine
Five signal types, evidence references, explainability, seven quality dimensions.

### Phase 7 — First real collectors
USGS, NASA NEO, GDELT, Hacker News, GitHub. Each with a checked-in fixture and
its own tests.

### Phase 8 — API
REST, including `/observations/:id/raw` so the drill-down actually ends at the
raw bytes.

### Phase 9 — Web UI
World → signal → event → observation → source → raw data.

## In progress

### Phase 10 — Replay / backtesting
`replay` runs the synthetic world through the pipeline as though it were live.
Remaining: running a *historical* stream — data captured from a real source —
through the same path, so false positives, false negatives, detection latency and
signal persistence can be measured against known outcomes.

### Phase 11 — Correlation
Deterministic convergence detection across independent sources is in place
(entity, geography, time window, category, direction). Remaining: widening the
entity/geography matching beyond the current exact-match rules.

### Phase 12 — Lenses
Lens configuration and matching exist. Remaining: shipping the full default lens
set and letting a user combine lenses (`WORLD + TECHNOLOGY + ENERGY + TURKEY`).

## Deliberately not built yet

The following are out of scope until the `observation → signal` chain is proven
on real streams, and adding them early would hide weaknesses in it:

- LLM agents, RAG, vector databases
- Microservices, Kubernetes, distributed clusters
- Complex ML, predictive models, future prediction
- Autonomous agents
- Native mobile applications
- Fancy maps or a graph database

An LLM may later explain a signal, synthesize context, compare sources or answer
natural-language questions. It may never decide that a signal exists. Detection
must not depend on it, and the engine must run correctly with no LLM present.

## Success criteria

The project is not successful because it connected a thousand sources. It is
successful when:

1. It follows a real data stream, learns what normal looks like, detects change,
   turns that into an event, and shows it as an explainable signal a human can
   act on.
2. A user can go from a signal all the way back to the raw data.
3. Adding a new source to the same architecture is easy.
4. The detection engine can be tested against historical replay.
