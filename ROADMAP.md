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
Done. `IMPACT` is produced from the shipped `config/impact/systemic.yaml` scope
(a systemic category or entity), and its reason names the matched term; see
`docs/decisions/0022-*`.

### Phase 7 — First real collectors
USGS, NASA NEO, GDELT, Hacker News, GitHub. Each with a checked-in fixture and
its own tests.

### Phase 8 — API
REST, including `/observations/:id/raw` so the drill-down actually ends at the
raw bytes.

### Phase 9 — Web UI
World → signal → event → observation → source → raw data.

## Done (continued)

### Phase 10 — Replay / backtesting
Done. The engine takes an injectable clock; `replay-stream` replays a captured
stream offline and deterministically; `backtest` scores the detector against it
and reports detection latency, signal persistence, and — when labels are
supplied — false positives, false negatives, precision and recall. `collect
--out` captures a live run to a stream file.

Widening the labels for the real sources is under way: the USGS catalog is now
labelled with its real M7.0+ quakes and scored end to end (`crates/engine/tests/
real_backtest.rs`, `docs/decisions/0023-*`). The measurement is honest and
unflattering — recall 0.50, precision 0.20 — which is exactly what a detector
needs before it can be improved. Labelling the remaining real sources is the
continuing work, not a blocker.

### Phase 11 — Correlation
Done. Deterministic convergence across independent sources: entity, geography,
time window, category and direction. Entity ids now match as *related* rather
than only identical, so `region_san_francisco` and
`region_san_francisco_bay_area` from two providers converge; candidates sharing
no entity can converge on coordinates. Both are behind `MergeMode::Related`;
the default (`Exact`) is unchanged. See `docs/decisions/0008-*`.

### Phase 12 — Lenses
Done. Lenses are configuration, not code: `wse-config` loads `config/lenses/*.yaml`
and the engine matches every formed signal against them, recording the result in
`Signal::lens_matches`. Eleven default lenses ship (`WORLD`, `EARTH`, `SPACE`,
`SOFTWARE`, `ENERGY`, `FINANCE`, `AGRICULTURE`, `SCIENCE`, `GLOBAL EVENTS`,
`TURKEY`, `PERSONAL`). `GET /lenses` and `GET /lenses/:id` list them with their
match counts; `GET /signals?lens=` filters on the recorded matches; the UI adds a
lens picker, lens badges on each card, and a `#/lenses` index.

A lens whose categories no collector emits yet (ENERGY, FINANCE) is present and
honestly reports zero rather than being hidden. Lenses change visibility only —
detection always runs on the full dataset.

## Productionization

Not a phase. After Phase 12 the engine was correct but not deployable: state was
in memory, data grew without bound, and the API was open. That work is done and
documented separately.

- SQLite persistence behind the existing store traits; `--data-dir` selects it,
  and both backends share one serving path.
- Bounded rehydration on startup, so the detector resumes warm.
- Retention: observations age out, signals and events do not.
- API authentication, safe CORS defaults, request limits, graceful shutdown.
- [docs/deployment.md](docs/deployment.md) — the operational story.
- [docs/audit.md](docs/audit.md) — a reality audit of Phases 0–12 against real
  commands, including the gaps it found.
- `docs/decisions/0009-productionization.md` — why it is shaped this way.

## A running product

Not a phase either. After productionization the engine was deployable but still
needed an operator to drive each collection cycle. That work is done:

- `wse serve --collect` runs a continuous scheduler loop that honors runtime
  controls, so a served instance keeps observing on its own.
- `RuntimeState` exposes uptime, a collection toggle, per-source enable/disable,
  coalesced run-now requests, and a bounded activity ring buffer.
- The pipeline emits activity as it runs — observation batches, anomaly
  candidates, events, signals, source health — served over `GET /activity` and
  pushed live over `GET /events` (Server-Sent Events).
- A control plane (`GET /control`, `POST /control/collection`,
  `POST /sources/:id/enabled`, `POST /sources/:id/run`), behind the same key as
  every other route.
- The UI presents signals with icon, label and shape rather than colour alone,
  and states the facts (deviation, persistence, independent sources) instead of
  a single importance score. A System screen streams activity live and offers
  pause/enable/run controls.

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
