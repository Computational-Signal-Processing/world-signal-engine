# AGENTS.md

Repository-specific knowledge for agents working on the World Signal Engine.

## What this project is

An autonomous world-observation and change-detection engine. It observes public
data sources, stores observations historically, learns what normal looks like,
detects meaningful change, correlates independent changes, and surfaces the few
signals a human should investigate.

It is **not** a news app, chatbot, dashboard or LLM summarizer. There is no LLM
in the detection path and the engine must run with no LLM present.

Read [docs/philosophy.md](docs/philosophy.md) before making design decisions.

## Layout

```text
crates/model        vocabulary only; no I/O, no detection
crates/collector    Collector contract + synthetic world
crates/sources      real collectors (usgs, afad, nasa, eonet, noaa_kp, nws,
                    gdelt, cisa_kev, ecb, crossref, arxiv, hackernews, github)
crates/normalize    payload → observation
crates/storage      store traits + in-memory and SQLite impls (incl. RawStore)
crates/baseline     rolling/robust statistics
crates/detection    change, anomaly, early signal → AnomalyCandidate
crates/correlation  convergence across sources
crates/signals      events, signals, lifecycle, quality
crates/presentation human language: signal facts → readable narrative + status
crates/engine       the cycle: Engine::run_collector / ingest_observations
crates/scheduler    cadence + live/replay clocks
crates/api          REST API + static UI
crates/cli          the `wse` binary
web/                static UI (no build step)
tests/fixtures/     real payloads for parser tests
docs/decisions/     architecture decision records
```

## Commands

```bash
cargo test --workspace                                   # 347 tests, offline
cargo fmt --all && cargo clippy --workspace --all-targets
cargo run -p wse-cli -- demo                             # synthetic acceptance world
cargo run -p wse-cli -- sources                          # print the catalog
cargo run -p wse-cli -- collect --verbose                # hit the real APIs once
cargo run -p wse-cli -- serve --port 8080 --collect      # API+UI backed by live data
cargo run -p wse-cli -- serve --port 8080 --collect \    # persistent + authenticated
  --data-dir /var/lib/wse --retention-days 90
cargo check -p wse-cli --no-default-features             # the no-SQLite build
```

The test suite is offline by design — synthetic world plus checked-in fixtures.
No network or credentials are needed to run it.

## Hard rules

1. **No LLM in detection.** Detection is mathematics. An LLM may explain a
   signal later; it may never decide one exists.
2. **Absence is not an event.** A failed collector means *no data*, never *zero
   activity*. `SourceHealth` is stored separately from observations for this
   reason. Never let an error path return an empty result that looks like
   success.
3. **Detectors produce candidates, not signals.** `AnomalyCandidate` is a
   measurement that looks unusual; nothing has decided it matters yet.
4. **Explainability is constructed, not generated.** A signal's `reasons[]` come
   from its own baseline/deviation/method/duration. Never write "the AI found
   this important".
5. **No single importance score.** Signal quality stays seven dimensions.
6. **A signal's identity is `event_id` + `direction`.** Not the series. An event
   is one ongoing change and its id is stable; its candidates can come from
   several series and the "dominant" one can flip between cycles. Keying on the
   series made one change mint a new signal id, so the merge missed and the
   engine stored a duplicate while the first froze. `series_key` is display.
7. **Sources are independent.** One broken collector must not affect another.
8. **Storage stays behind traits.** Do not reach for a concrete database from
   engine code.

## Conventions

- `parse` in a source module is **pure** (bytes in, observations out) and must
  reject malformed input explicitly. Returning empty to mean "bad response" is
  the exact confusion the engine exists to prevent.
- The raw payload's hash and its body must be derived from one shared function,
  or the drill-down's last step dead-ends. See
  `docs/decisions/0003-retain-raw-payloads.md`.
- Comments explain *why* — non-obvious invariants, workarounds, trade-offs.
  Do not restate the code or narrate the diff.
- Edit existing files; do not create `foo_v2.rs` variants.
- Adding a source: module + fixture + test + two registrations in
  `crates/sources/src/lib.rs`. No core changes.
- **A source declares its `measurement` semantics.** `stable_series` and
  `fixed_universe` are detected on; `unstable_population` (a top-N ranking, a
  search result) is stored for evidence but never detected on, because its
  aggregate is membership churn, not a world change. The engine reads this in
  `Engine::ingest_observations`; do not "fix" a churning source by detecting on it.
- **A source declares `feeds_lenses`, and the declaration is enforced.** At
  runtime the engine reads the declarations from the stored catalog and routes a
  signal to the lenses its sources feed, by provenance, unioned with the lens
  filters (`SignalEngine::assign_lens_matches`). So a declared lens is one the
  source's signals actually reach. `crates/cli/tests/lens_coverage.rs` checks
  both the declaration (a source may not feed a lens that does not exist) and its
  reachability (`a_declared_lens_is_reachable_at_runtime`); it fails if a lens
  has no connected source and is not listed in `INTENTIONALLY_UNFED`. Keep that
  list honest: it is the difference between a deliberate placeholder and a dead
  lens. Routing is derived from the catalog in `Engine::register_source`, so do
  not special-case a source or category in core code.
- **arXiv rejects `max_results=0`** with HTTP 500 while still reporting the true
  `opensearch:totalResults`; the smallest accepted value is `1`. Fetch one and
  discard it.
- **A "spike" can be entirely artificial if the population churns.** Before
  adding a source, ask whether the same query would return a different member
  set next poll; if so, it is `unstable_population`.

## Gotchas

- **GDELT rate-limits aggressively** and returns a plain-text notice with HTTP
  429 instead of JSON. The transport classifies this as
  `CollectorError::RateLimited`, so it shows up as a `rate_limited` source with
  `rate_limit_count` and the last error, not as zero news volume and not as
  `down` — a throttled source is healthy, we are simply asking too often. The
  parser rejects non-JSON explicitly. Do not "fix" this by making the parser
  lenient, and do not collapse `rate_limited` into `down`.
- **GDELT's live API does not match its own docs.** The doc endpoint
  (`api.gdeltproject.org`) uses `timelinevol` with `datetime`/`series`; the live
  API (`api.gdeltproject.org/api/v2/doc/doc`) returns `query_details` and
  `YYYYMMDDTHHMMSSZ` timestamps. Both shapes are decoded (`resolve_query`,
  `parse_datetime`), and `tests/fixtures/gdelt_timelinevol_real.json` is a real
  captured payload so this cannot silently regress to an empty source.
- **GitHub search responses are wrapped** in an envelope; the repos are under
  `items`. The fixture reflects this.
- **Observation ids are the record key, not the payload hash.** A multi-record
  feed or a sliding window changes its whole-body hash whenever *any* record
  changes, so seeding ids with that hash re-mints the ids of the records that
  did not change and re-ingests them as duplicates. Ids are now
  `series_key | observed_at | record_key`, where the record key is the source's
  stable identity for the record (USGS feature id, AFAD `eventID`, ECB period,
  NOAA `time_tag`, GDELT `series|date`). The payload hash is only the fallback
  for a source with one record per series per timestamp. Never put the measured
  value in the record key: a revised measurement for the same record must keep
  one identity. `Observation::with_record_key` sets it.
- **A dimensioned source must pass its full `series_key()` as the record key.**
  The id is seeded with the *base* series key (`source::entity::metric::unit`),
  which does **not** include dimensions. A source that emits several series per
  entity that differ only by a dimension — NWS emits four severities per entity —
  collapses them onto one id and silently drops all but one. Passing
  `observation.series_key()` (base key plus dimensions) fixes it: the dimensions
  stay out of the series key, so each count is its own series *and* its own id.
  Alternatively, encode the dimension in the entity, as EONET/AFAD/GDELT do.
- **Several records per series per timestamp need `Observation.identity`.**
  GitHub search returns many repositories, Hacker News many stories — all with
  the same `observed_at` and payload hash. Without a discriminator they share an
  id and de-duplication silently drops all but one. `identity` is part of the
  id and **never** part of `series_key`, so the records stay independently
  observable while still forming one series for baseline and detection.
- **MSRV is 1.88**, not 1.75 or 1.82. It is set by the dependency graph, not by
  our code: `icu_* 2.3.0` (pulled in via `url`, which both `ureq` and `reqwest`
  depend on) requires 1.88, and several transitive crates are edition 2024, which
  needs Cargo ≥ 1.85. Do not lower `rust-version` without re-running the MSRV CI
  job; `cargo check` on the older toolchain is what proves it.
- **`reqwest` is a dev-dependency of `wse-api` only**, and is built with
  `default-features = false, features = ["json"]` — deliberately no TLS. The HTTP
  tests talk to `http://127.0.0.1` and nothing else. Enabling `rustls-tls` pulls
  in `quinn` → `rand 0.10` → `rand_core 0.10`, which is edition 2024 and pushes
  the whole graph's MSRV up for no benefit. All real network I/O goes through
  `ureq` in `wse-sources`.
- The synthetic detector config (`DetectorConfig::synthetic()`) is deliberately
  permissive so the pipeline is visible in seconds. It is a demo config and makes
  no claims about the real world.

## Drill-down contract

This must keep working end to end, and is covered by
`crates/api/tests/http.rs::drill_down_reaches_raw_data`:

```text
GET /signals/:id → event_id
GET /events/:id  → observations[0]
GET /observations/:id → source_id, raw.hash
GET /sources/:id
GET /observations/:id/raw → the actual bytes
```

## Replay and backtesting (Phase 10)

Two run modes: `LIVE` and `REPLAY`. Replay is how the detector is measured, not
a test helper. See `docs/decisions/0005-replay-as-a-run-mode.md`.

- The engine takes an injectable clock (`Engine::with_clock`). Anything that
  stamps a signal with "now" must use `self.clock.now()`, never `Utc::now()`, or
  replay reports wall-clock times against historical data.
- `wse-scheduler::SharedReplayClock` is a shareable, advanceable clock. The
  replay driver pins it to each arrival batch's `received_at`.
- A stream is newline-delimited JSON (`wse-collector/src/replay.rs`): a `header`
  record, then one `observation` per line. `collect --out FILE` writes one;
  `replay-stream --file FILE` and `backtest --file FILE` read it.
- Replay groups by `received_at`, not `observed_at`, so detection sees the same
  batches live collection produced. Replay is deterministic: same stream + same
  config means a byte-identical report, signal ids included.
- `backtest` withholds precision/recall when no `--labels` are given. Do not
  "fix" that by inventing a number; an unlabelled run cannot tell "wrong" from
  "not yet known to be right".
- Running past the end of a stream is an empty **success**, never a failure:
  `NO DATA` is not `DATA = ZERO` (brief section 29).

Two latent bugs the replay work surfaced, both fixed and ADR'd:

- **Event grouping** (`docs/decisions/0006-*`): `event_group_key` re-derived the
  key from `entities`/`anomalies`, so `entity:...` and `series:...` never matched
  and one ongoing change was split into a new event every cycle. The key is now
  stored on the event as `Event::group_key`.
- **Event identity** (`docs/decisions/0007-*`): `Event::new` used a random UUID,
  which silently defeated `Signal::stable_id`'s documented determinism. The
  pipeline now uses `Event::new_for`, whose id is derived from group key plus
  start time. `Event::new` remains for tests only.


## Convergence matching (Phase 11)

Convergence is computed inside `SignalEngine::form_signals`, from the engine's
own `ConvergenceConfig`. Do not pass groups in from outside — the rule that
matches sources and the code that forms signals must not be able to drift apart.

- `MergeMode::Exact` is the default and is the original behaviour: identical
  entity id, else series key. `MergeMode::Related` also matches *related* entity
  names and geography. Widening matching is opt-in
  (`ConvergenceConfig::related()`); a deployment must not inherit it.
- Related entity rule: one canonical segment set must be a **subset** of the
  other, sharing >= `min_shared_segments` (2). Subset, not overlap, so
  `region_south_fiji` and `region_south_tonga` stay apart despite "south". Real
  collector slugs look like `region_san_francisco_bay_area`, `topic_oil_price`.
- Related entities are merged with union-find so a chain of names is one group,
  not a set of pairs. Geographic clustering (haversine, `radius_km`) uses the
  same structure over connected components.
- `ConvergenceGroup` carries `entity_ids` and `match_kinds`
  (`exact_entity | related_entity | geography | series`) so a signal can report
  *how* its sources agree.

## Lenses (Phase 12)

A lens is a view. It changes what is *visible*, never what is detected or
stored. Detection always runs on the full dataset; lenses filter the result.

- Lenses are YAML under `config/lenses/`, loaded by `wse-config`. Adding one is
  a new file, not a Rust change. The loader is tolerant by design: a missing
  directory is a valid state, and a malformed file is skipped and reported in
  `LensCatalog::problems` so the other lenses still load. A view must never be
  able to take down collection or detection.
- The catalog is sorted by lens id, and `lens_matches` is written in that order.
  This is a determinism requirement, not tidiness: a replayed run has to produce
  the same signal bytes as the original.
- Matching happens in `SignalEngine::assign_lens_matches`, once, at signal
  formation. It unions the lens filters with the source→lens routing read from
  the catalog (see the `feeds_lenses` note above): a signal reaches a lens by
  filter *or* by the provenance of a source that declares it. `Signal::lens_matches`
  is therefore a *record* of which lenses showed the signal, not something
  recomputed per query. `?lens=` reads it directly.
- `merge_signals` **unions** lens matches rather than replacing them. A signal
  that accumulated categories over its life can only have gained lenses, and
  dropping one would make a `?lens=` query lose a signal it had already returned.
- A signal with no location is never excluded by a bbox. The engine does not
  know where it happened, and hiding it would drop data rather than filter it.
- A lens that matches nothing is a legitimate, visible state (ENERGY and FINANCE
  have no collector yet). `GET /lenses` reports the count so an empty view is
  distinguishable from a broken one.
- Lenses are not part of a signal's identity. `lens_matches` is not hashed into
  `Signal::stable_id`, so adding a lens does not rewrite existing signal ids.

## Convergence ordering (Phase 11, still load-bearing)

- `detect_convergence` sorts by strength, then first_seen, then `group_key`, and
  uses `BTreeMap` not `HashMap`. The key tiebreaker and the ordered map are
  load-bearing: equal-strength groups in hash order made replay
  non-reproducible. Do not revert either to `HashMap`/unsorted.
- `wse-engine` re-exports `ConvergenceConfig` and `MergeMode`; the CLI depends on
  `wse-engine`, not `wse-correlation`, so import them from `wse_engine`.

Synthetic worlds for these: `SyntheticWorld::related_entities` and
`::geographic_convergence`. Acceptance tests assert both sides — convergence
under `Related`, none under `Exact` — so a regression that makes matching
unconditionally permissive fails too.


## Deployment (persistence, retention, security)

See [docs/deployment.md](docs/deployment.md) for the operational story and
`docs/decisions/0009-productionization.md` for why it is shaped this way.

- **`serve` is generic over the store.** `run_served<S: Store>` drives both the
  in-memory and the SQLite backend. Do not reintroduce a per-backend copy of the
  serving logic; a change to collection or retention must reach both.
- **`--data-dir` selects SQLite; without it the engine is in memory.** With it,
  observations, events, signals, source health and baselines survive a restart.
  `--rehydrate-history N` (default 500) replays the N most recent observations
  per series so the detector resumes warm. `series=0` in the startup log means
  an empty store, not a quiet world.
- **`SqliteStore` wraps its `Connection` in a `std::sync::Mutex`.** One guard per
  function; the mutex is not reentrant. Holding a guard across a call that takes
  it again self-deadlocks — this is what `prune_raw_to` did.
- **Retention ages out observations, not events or signals.** A signal is the
  conclusion a person was investigating; its evidence may be gone and the
  drill-down says so. Do not "fix" an empty evidence list by deleting the
  signal.
- **Raw pruning must forget the payload, not just delete its file.** Deleting
  only the file leaves the size in the in-memory index, so `bytes_used` stays
  inflated and pruning runs until the store looks empty. Covered by
  `raw_retention_prunes_oldest_payloads_until_under_the_cap`.
- **The API is authenticated when `WSE_API_KEYS` is set.** Every route but
  `/health`. `/metrics` is *behind* the key by default; `WSE_PUBLIC_METRICS=1`
  opens it. Key comparison is constant-time — do not replace it with `==`.
- **CORS defaults to no origins.** The bundled UI is same-origin. Adding a
  wildcard with auth on would let any page read the API through a visitor's
  browser; that is why `SecurityConfig::default()` allows none.
- **The UI sends the key as a header, never in the query string**, and keeps it
  in `localStorage`. A key in a URL lands in history, logs and `Referer`.
- **Serving without a key logs a warning.** There is no flag that makes an open
  API safe on a public address; the warning is the honest signal.
- **`SIGTERM`/`SIGINT` shut down cleanly**, so `systemctl restart` does not kill
  a write in progress.
