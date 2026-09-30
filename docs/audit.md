# Reality audit — Phases 0–12

This is a check of what the repository *does*, against what the roadmap *says*.
Every claim below was produced by running a command in this repository, not by
reading a document. Where a claim could not be reproduced, it is listed as a
gap rather than repeated as fact.

Audit run: 2026-09-30, on `main` (shallow clone at `08e4b73`), Rust stable.

## Method

- `cargo test --workspace` — the offline suite (synthetic world + fixtures).
- `cargo clippy --workspace --all-targets`, `cargo fmt --all --check`.
- `cargo check -p wse-cli --no-default-features` — the build without SQLite.
- The `wse` binary, run: `demo`, `collect`, `replay-stream`, `backtest`,
  `lenses`, `serve`.
- `curl` against a served instance for the drill-down chain.

## Results

### Phase 0 — Repository bootstrap

| Claim | Evidence | Verdict |
| --- | --- | --- |
| Cargo workspace | 14 member crates in `Cargo.toml` | ✅ |
| Edition 2021, MSRV 1.88 | `edition`/`rust-version` in `[workspace.package]` | ✅ |
| AGPL-3.0-or-later | `LICENSE` is AGPLv3; `license.workspace = true` | ✅ |
| Formatting | `cargo fmt --all --check` exits 0 | ✅ |
| Linting | `cargo clippy --workspace --all-targets` exits 0, no warnings | ✅ |
| CI | `.github/workflows/ci.yml`: fmt, clippy, test, no-sqlite check, release build, MSRV job on 1.88 | ✅ |

### Phase 1 — Core domain model

`Source`, `Observation`, `Entity`, `Event`, `AnomalyCandidate`, `Signal`, `Lens`,
`SignalQuality`, `RawReference` all exist in `wse-model` with serde derives.
`cargo test -p wse-model` passes. ✅

### Phase 2 — Synthetic world

`wse demo` prints real signals from the synthetic world, with baseline, method
and deviation in each `why` line:

```text
[NOW + ANOMALY] Anomaly: sensor rising
  why: deviation +3.6σ from baseline (median 99.64, MAD 0.49) via RobustZScore
```

No network is used. ✅

### Phase 3 — Storage

Store traits (`ObservationStore`, `EventStore`, `SignalStore`, `SourceStore`,
`BaselineStore`, `RawStore`, `MaintenanceStore`) plus two implementations:
`InMemoryStore` and `SqliteStore`. `cargo test -p wse-storage --features sqlite`
passes 27 tests, covering migration idempotency, duplicate observations, reopen
persistence, range/facet filters, evidence, baselines and retention. ✅

### Phase 4 — Baseline + detection

`wse-baseline` implements rolling mean/median, standard deviation, MAD, z-score,
robust z-score, EWMA, percentile and rate of change; `wse-detection` produces
`AnomalyCandidate`s with method and deviation recorded. The `demo` output shows
both `ZScore` and `RobustZScore` firing on real data. ✅

### Phase 5 — Event engine

Events form from candidates grouped by entity/series, window and direction, with
lifecycle states. The drill-down check below reads `state: ACTIVE` from a live
event. ✅

### Phase 6 — Signal engine

Five signal types observed in live output: `NOW`, `ANOMALY`, `EARLY_SIGNAL`
(from `demo` and `serve`), and `CONVERGENCE` (Phase 11). `IMPACT` exists as a
type; no collector currently emits an impact-scored signal. Seven quality
dimensions are present on `SignalQuality`. ⚠️ IMPACT unexercised end to end.

### Phase 7 — First real collectors

`wse collect` against the live APIs:

```text
usgs_earthquakes       ok      6 observation(s) [healthy]
nasa_neo               ok      33 observation(s) [healthy]
gdelt_news_volume      FAILED  (source health: ratelimited) — no observations recorded
hackernews_frontpage   ok      29 observation(s) [healthy]
github_rust_activity   ok      50 observation(s), 6 anomaly candidate(s), 2 signal(s) [healthy]
```

Five collectors, real network, and GDELT's HTTP 429 correctly classified as
`rate_limited` rather than zero volume. ✅

### Phase 8 — API

`GET /health`, `/metrics`, `/signals`, `/signals/:id`, `/events/:id`,
`/observations/:id`, `/observations/:id/raw`, `/sources`, `/sources/:id`,
`/entities/:id`, `/timeline`, `/lenses`, `/lenses/:id` all respond, as do the
operational routes added after this audit's first pass: `GET /control`,
`GET /activity`, `GET /events` (SSE), `POST /control/collection`,
`POST /sources/:id/enabled`, `POST /sources/:id/run`. Verified by `curl` against
a served instance and by `crates/api/tests/http.rs` (26 tests). ✅

### Phase 9 — Web UI

`GET /` serves `web/index.html` (200), and `app.js`, `styles.css`,
`manifest.webmanifest` and `icon.svg` are served with the right content types.
The UI drives world → signal → event → observation → source, with a lens picker,
an API key field (revealed only when a key is needed), a map, a timeline, and a
System screen that streams live activity over SSE and exposes the control plane.
The static files need no build step. ✅

### Phase 10 — Replay / backtesting

`collect --out` captured 118 observations; `replay-stream` replayed them and
produced the same 2 signals; `backtest` reported latency, persistence, and, with
labels, precision/recall:

```text
signals        2
mean latency   0s
labelled       1 event(s): 1 matched, 0 false positive, 0 missed
precision      0.50
recall         1.00
```

An unlabelled run withholds precision/recall instead of inventing a number. ✅

### Phase 11 — Correlation

`cargo test -p wse-correlation` passes 17 tests, including convergence under
`MergeMode::Related` and its absence under `Exact`. ✅

### Phase 12 — Lenses

`wse lenses` lists 11 lenses; `GET /lenses` returns them; `?lens=` filters on
recorded matches (`lens_global` returned 2, `lens_energy` returned 0 — an empty
view, not a broken one). ✅

## Drill-down, end to end

Against a served SQLite-backed instance:

```text
GET /signals/sig_578775f6271fe92a   → event_id evt_aa83348d4d9edb68
GET /events/evt_aa83348d4d9edb68    → state ACTIVE, observations[obs_b1f6…]
GET /observations/obs_b1f6e0f9fdcf3d60 → source_id synthetic_sensor
GET /sources/synthetic_sensor       → health: healthy
GET /observations/obs_b1f6…/raw     → {"locator":"synthetic://sensor/21"}
```

Every link resolves. ✅

## Restart

The same store was reopened in a new process:

```text
opened persistent store schema=1
rehydrated detector state from storage series=1
```

Signals and observations were still served after the restart; the detector
resumed rather than re-learning. ✅

## Productionization (post-Phase 12)

Not a phase; the work required to run this for real. Landed with this audit:

- SQLite persistence, selected by `--data-dir`, sharing one serving path with
  the in-memory backend.
- Bounded rehydration, so a restart resumes warm without replaying all history.
- Retention: observations age out; signals and events do not.
- Raw pruning that forgets payloads as well as deleting their files (a real bug,
  fixed and covered by a test).
- API authentication (`WSE_API_KEYS`), constant-time key comparison, no CORS by
  default, body/timeout limits, and a startup warning when serving without a key.
- A UI key field, sent as a header.
- `docs/deployment.md`, and CI checking the no-SQLite build.

## Making it a running product (second pass)

The engine could already answer the two required questions, but only if an
operator drove it. This pass made it self-running and operable:

- A continuous scheduler loop in `wse serve --collect`, honoring runtime
  controls, so a served instance keeps observing the world without an operator.
- `RuntimeState` in `wse-engine`: uptime, collection enabled/disabled, per-source
  enable/disable, run-now requests (coalesced, not queued twice), and a bounded
  activity ring buffer.
- An activity stream emitted as the pipeline runs — observation batches, anomaly
  candidates, events, signals, and source health changes — exposed over
  `GET /activity` and pushed live over `GET /events` (SSE).
- A control plane: `GET /control`, `POST /control/collection`,
  `POST /sources/:id/enabled`, `POST /sources/:id/run`, all behind the same key
  as every other route.
- A System screen in the UI that shows the engine's real state, streams activity
  live, and offers pause/enable/run controls — so "what is it doing right now?"
  is answerable from the product, not from logs.
- The UI rewritten around the brief's card vocabulary: icon + label + shape for
  signal type, and the facts (deviation, persistence, independent sources) in
  place of a single opaque importance score.

Re-verified after this pass: `cargo fmt --all --check`, `cargo clippy --workspace
--all-targets`, `cargo test --workspace` (328 tests), the no-SQLite build, a live
`serve --collect` run against the real sources (USGS/NASA/HN/GitHub healthy,
GDELT rate-limited and recorded as such), and the full drill-down over `curl`
plus the browser UI.

## Known limitations

1. **`IMPACT` has no producer.** The type exists and is tested as a type, but no
   collector or detector emits an impact-scored signal. It is not a defect in
   what is built; it is an unexercised path. Left as-is rather than faked.
2. **`config/sources/` and `config/detectors/` are empty.** The source catalog
   lives in Rust (`crates/sources`) and the detector config in code. The
   directories from the original layout are placeholders. Not load-bearing; the
   lens directory *is* used (`config/lenses/*.yaml`).
3. **`tests/integration`, `tests/detection`, `tests/collectors` are empty.**
   Integration coverage lives beside the crates (`crates/api/tests`,
   `crates/engine/tests`) and in each crate's unit tests. The empty directories
   are leftovers from the planned layout.
4. **No labelled history for the real sources.** Backtesting works and is
   measured, but its ground truth so far is hand-made. This is the same
   limitation the roadmap already records under Phase 10.
5. **The activity stream is per-process.** It is an in-memory ring buffer, not a
   durable log, and a restart starts it empty. That is deliberate — activity is
   operational telemetry, not data. What matters (observations, events, signals,
   source health) is in the store and survives.

None of these block the two questions the brief requires the product to answer:
*what changed abnormally since the last cycle*, and *show me the evidence*. Both
were demonstrated above, and both are now answerable from the running product
itself rather than only from a command line.
