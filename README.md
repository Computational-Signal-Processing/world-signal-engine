# World Signal Engine

> The machine measures the world continuously; a human investigates only what changed.

The World Signal Engine observes accessible real-time and periodic data sources,
stores those observations in historical context, learns what normal looks like,
detects meaningful change, relates independent changes to one another, and
surfaces the small number of signals a person should actually look at.

It is **not** a news app, a chatbot, a dashboard, or an LLM summarizer. The core
is statistics and stream processing. There is no LLM in the detection path, and
the engine runs correctly with no LLM anywhere in the process.

The question the system exists to answer:

> **"What is changing in the world right now that I should know about?"**

and then, for any answer it gives:

> **"Show me the evidence."**

## The drill-down

Every signal can be followed all the way back to the bytes the source returned:

```text
SIGNAL → EVENT → OBSERVATION → SOURCE → RAW DATA
```

Nothing is summarized away. A signal says *which measurement deviated, from which
baseline, by how much, for how long, and backed by how many independent
observations* — it never says "the AI thought this was important."

## Status

The pipeline runs end to end today, from real collectors through to the API and
web UI. Current state:

| Stage | State |
| --- | --- |
| Source catalog | 13 sources across 9 categories, each with a provenance tier and measurement semantics |
| Collectors | USGS, AFAD, NASA NEO, NASA EONET, NOAA Kp, NWS alerts, GDELT, CISA KEV, ECB rates, Crossref, arXiv, Hacker News, GitHub |
| Normalization | Observation model with dimensions and attributes |
| Storage | In-memory or SQLite `Observation`/`Event`/`Signal`/`Source`/`Baseline`/`Raw` stores |
| Baseline | Rolling and robust statistics |
| Detection | Change, anomaly, early signal |
| Correlation | Cross-source convergence |
| Signals | `NOW`, `ANOMALY`, `EARLY_SIGNAL`, `CONVERGENCE`, `IMPACT` |
| API | REST, with a raw-data endpoint |
| Operations | `/control`, pause/resume, per-source enable/run, live activity over SSE |
| Web UI | World, signal, event, observation, source, lenses, timeline, map, system |

480 tests pass across the workspace.

## Quick start

```bash
# See the whole pipeline run on the deterministic synthetic world.
cargo run -p wse-cli -- demo

# List the source catalog.
cargo run -p wse-cli -- sources

# Collect from the real sources once and print what came back.
cargo run -p wse-cli -- collect --verbose

# Serve the API and web UI, backed by live data collected at startup.
cargo run -p wse-cli -- serve --port 8080 --collect

# The same, but persistent and protected, as a deployment would run it.
export WSE_API_KEYS="$(openssl rand -hex 32)"
cargo run -p wse-cli -- serve --port 8080 --collect \
  --data-dir /var/lib/world-signal-engine --retention-days 90
```

Then open <http://localhost:8080>.

With `--data-dir` the engine keeps observations, signals, source health and
baselines in SQLite, so a restart resumes rather than re-learning. With
`WSE_API_KEYS` set, every route but `/health` needs the key. See
[docs/deployment.md](docs/deployment.md) for the full operational story.

The API is small on purpose:

```text
GET  /health
GET  /metrics
GET  /world                   the one-screen "what is changing now" summary
GET  /control                 the engine's live operational state
POST /control/collection      pause or resume continuous collection
GET  /activity                the recent activity stream, newest first
GET  /events                  live activity stream (Server-Sent Events)
GET  /signals                 ?category=&type=&entity=&active=&lens=&status=
GET  /signals/:id
GET  /events/:id
GET  /observations/:id
GET  /observations/:id/raw    the raw bytes the source returned
GET  /sources
GET  /sources/:id
POST /sources/:id/enabled     enable or disable one source
POST /sources/:id/run         run one source at the next scheduler pass
GET  /entities/:id
GET  /lenses
GET  /lenses/:id
GET  /timeline                ?series=
```

The UI at `/` is the product: a live World feed of explainable signals, a
System screen showing what is running and streaming activity live, and a
drill-down from any signal to `EVENT → OBSERVATION → SOURCE → RAW DATA`.

## Documentation

| Document | Contents |
| --- | --- |
| [ARCHITECTURE.md](ARCHITECTURE.md) | The pipeline, the crates, and how a cycle runs |
| [DATA_MODEL.md](DATA_MODEL.md) | Observation, entity, event, source, raw reference |
| [SIGNAL_MODEL.md](SIGNAL_MODEL.md) | Anomaly candidates, signal types, signal quality |
| [SOURCE_CATALOG.md](SOURCE_CATALOG.md) | Every source, its cadence, license and collector |
| [DEVELOPMENT.md](DEVELOPMENT.md) | Build, test, lint, and how to add a collector |
| [ROADMAP.md](ROADMAP.md) | The phases, and what is deliberately not built yet |
| [CONTRIBUTING.md](CONTRIBUTING.md) | How to contribute |
| [docs/philosophy.md](docs/philosophy.md) | Why the engine is built this way |
| [docs/detection.md](docs/detection.md) | Baselines, change, anomaly, early signal |
| [docs/correlation.md](docs/correlation.md) | Convergence across independent sources |
| [docs/lenses.md](docs/lenses.md) | Lenses and relevance |
| [docs/deployment.md](docs/deployment.md) | Persistence, retention, security, systemd |
| [docs/audit.md](docs/audit.md) | Reality audit of Phases 0–12 against real commands |

## Design principles

- **No LLM in the core.** Detection is mathematics. An LLM may later explain a
  signal; it may never decide that one exists.
- **Absence is not an event.** A failed collector means *no data*, never
  *zero activity*. The two are stored differently and rendered differently.
- **Change is not deviation.** An observation can change without being
  anomalous, and be anomalous without having changed.
- **Explainable by construction.** Every signal carries the measurements,
  baselines and thresholds that produced it.
- **Sources are independent.** One broken source never stops the pipeline.
- **Storage is abstracted.** The application is not locked to one database.

## License

AGPL-3.0-or-later. See [LICENSE](LICENSE).
