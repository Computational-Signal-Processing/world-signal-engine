# Development

## Requirements

- Rust 1.82 or newer (the workspace MSRV; some code uses `Option::is_none_or`).
- No external services are needed to build or test. The full test suite runs
  against the synthetic world and checked-in fixtures, with no network.

## Build, test, lint

```bash
cargo build --workspace
cargo test  --workspace          # 227 tests, no network required
cargo fmt   --all
cargo clippy --workspace --all-targets
```

CI should run exactly these four, plus a release build.

## The CLI

```bash
# Run the synthetic acceptance world and print the signals.
cargo run -p wse-cli -- demo --steps 205

# Feed the synthetic world through the pipeline as if it were live.
cargo run -p wse-cli -- replay --steps 205 --verbose

# List the catalog.
cargo run -p wse-cli -- sources

# Run the real collectors once.
cargo run -p wse-cli -- collect --verbose
cargo run -p wse-cli -- collect --source usgs_earthquakes --json

# Serve the API and UI. --collect backs it with live data at startup.
cargo run -p wse-cli -- serve --port 8080 --collect
```

`serve --synthetic` drives the deterministic synthetic world in the background;
`serve --live` keeps producing synthetic observations forever. The synthetic
detector config is permissive on purpose — it makes the pipeline visible in
seconds rather than making real-world claims.

## Testing philosophy

Tests drive real code paths. There are no mocks standing in for the engine, the
storage or the detectors. Where a test needs a source, it uses the deterministic
synthetic world or a checked-in fixture, both of which are real inputs to real
parsing code.

Three kinds of test:

- **Unit tests** next to the code: statistics, parsers, model invariants.
- **Acceptance tests** (`crates/engine/tests/acceptance.rs`): the whole pipeline
  on scripted synthetic streams — drift produces an `EARLY_SIGNAL`, a spike
  produces an `ANOMALY`, aligned streams produce `CONVERGENCE`, and a repeated
  collection produces no duplicate observations.
- **HTTP tests** (`crates/api/tests/http.rs`): the real router on a real socket,
  including the full drill-down.

The synthetic world is the acceptance test the project does not ship without:

```text
100 observations, normal distribution
101–120        gradual upward drift   → EARLY_SIGNAL
201–205        large spike            → ANOMALY
aligned streams                        → CONVERGENCE
```

If that test does not pass, the detection engine is not considered working.

## Adding a collector

The split between fetching and parsing is what makes a collector testable.

1. **Create the module** `crates/sources/src/mysource.rs`:

   ```rust
   pub const SOURCE_ID: &str = "mysource_feed";
   pub const COLLECTOR_TYPE: &str = "mysource_feed";

   /// The catalog entry: metadata only.
   pub fn source() -> Source { /* ... */ }

   /// Pure: bytes in, observations out. No I/O.
   pub fn parse(body: &[u8], received_at: DateTime<Utc>)
       -> Result<Vec<Observation>, SourceError> { /* ... */ }

   /// The collector: fetch, then call `parse`.
   pub struct MySourceCollector { /* transport, config */ }
   ```

   `parse` must reject malformed input explicitly. Never return an empty
   observation list to mean "the response was wrong" — that is indistinguishable
   from "the world reported nothing", which is exactly the confusion the engine
   exists to avoid.

2. **Add a fixture** at `tests/fixtures/mysource.json`, taken from a real
   response.

3. **Unit-test `parse`** against the fixture, including at least one malformed
   input case.

4. **Register it**:
   - add the module to `crates/sources/src/lib.rs`,
   - add `mysource::source()` to `catalog()`,
   - add the collector to `live_collectors()`.

That is the whole change. No core code is touched.

## Adding a detector

A detector reads a series tracker and returns `AnomalyCandidate`s. It does not
know about sources, storage, events or signals.

```rust
pub fn detect_mine(tracker: &SeriesTracker) -> Vec<AnomalyCandidate>
```

Returning an empty vector is a valid, common outcome. A detector that fails or
finds nothing must not stop the rest of the cycle.

## Adding a lens

A lens is configuration, not code. See [docs/lenses.md](docs/lenses.md).

## Repository layout

```text
crates/          the workspace (see ARCHITECTURE.md for what each crate owns)
config/          sources/, detectors/, lenses/ configuration
docs/            philosophy, detection, correlation, lenses, decisions
tests/fixtures/  real payloads used by parser tests
web/             the static UI served by wse-api
scripts/         developer helpers
```

## House style

- Comments explain *why*, not *what*. If a line needs a comment to say what it
  does, rename things instead.
- A comment earns its place by recording a non-obvious invariant, a workaround,
  or a deliberate trade-off — for example why a raw payload's hash and body are
  computed from one shared function.
- Prefer a new type over a new boolean parameter.
- Errors are explicit. "Not found" and "failed" are different responses.
