# Architecture (docs)

The authoritative architecture document is [../ARCHITECTURE.md](../ARCHITECTURE.md)
at the repository root. This directory holds the supporting material:

- [philosophy.md](philosophy.md) — why the engine is built this way, and the
  distinctions the design protects.
- [detection.md](detection.md) — baselines, change, anomaly and early-signal
  detection.
- [correlation.md](correlation.md) — convergence across independent sources.
- [lenses.md](lenses.md) — lenses and relevance.
- [decisions/](decisions/) — architecture decision records.

## Decision records

Each record states the context, the decision, and the consequences that were
accepted along with it. They are written when a decision is made and are not
rewritten afterwards; if a decision changes, a new record supersedes the old one.

| Record | Decision |
| --- | --- |
| [0001](decisions/0001-no-llm-in-detection.md) | No LLM in the detection path |
| [0002](decisions/0002-in-memory-storage-first.md) | In-memory storage first |
| [0003](decisions/0003-retain-raw-payloads.md) | Retain raw payloads, keyed by content hash |
| [0004](decisions/0004-separate-fetch-from-parse.md) | Separate fetch from parse in collectors |
