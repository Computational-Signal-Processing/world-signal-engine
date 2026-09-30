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
crates/sources      real collectors (usgs, nasa, gdelt, hackernews, github)
crates/normalize    payload → observation
crates/storage      store traits + in-memory impl (incl. RawStore)
crates/baseline     rolling/robust statistics
crates/detection    change, anomaly, early signal → AnomalyCandidate
crates/correlation  convergence across sources
crates/signals      events, signals, lifecycle, quality
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
cargo test --workspace                                   # 227 tests, offline
cargo fmt --all && cargo clippy --workspace --all-targets
cargo run -p wse-cli -- demo                             # synthetic acceptance world
cargo run -p wse-cli -- sources                          # print the catalog
cargo run -p wse-cli -- collect --verbose                # hit the real APIs once
cargo run -p wse-cli -- serve --port 8080 --collect      # API+UI backed by live data
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
6. **A signal's identity is `series_key` + `direction`.** That is what makes it
   persist across cycles instead of re-emitting every minute.
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

## Gotchas

- **GDELT rate-limits aggressively** and returns a plain-text notice with HTTP
  429 instead of JSON. The parser rejects non-JSON explicitly, so this shows up
  as a `degraded` source with `error_count`, not as zero news volume. Do not
  "fix" this by making the parser lenient.
- **GitHub search responses are wrapped** in an envelope; the repos are under
  `items`. The fixture reflects this.
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
