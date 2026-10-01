# Reality audit — world-watch productization

Second, independent audit of the repository. Unlike
[docs/audit.md](audit.md), which checks phases against the roadmap, this one
asks a single product question and refuses to answer it from documentation:

> Start the application. Within seconds, does a person see what is changing in
> the world, how fresh it is, why it was surfaced, and where the evidence came
> from — without refreshing the page?

Every verdict below is one of:

`VERIFIED BY EXECUTION` · `VERIFIED BY CODE INSPECTION` ·
`TESTED ONLY WITH SYNTHETIC DATA` · `DOCUMENTED BUT NOT VERIFIED` ·
`PARTIALLY IMPLEMENTED` · `BROKEN` · `MISSING`

Audit run: 2026-10-01, `main` @ `15de3d1`, Rust 1.88.0, real network.

## What was actually run

```text
cargo run -p wse-cli -- sources
cargo run -p wse-cli -- lenses
cargo run -p wse-cli -- collect --verbose          # real APIs
wse serve --collect --data-dir … --cadence-override 30
wse serve                       (no --collect)
curl /health /control /metrics /world /signals /sources
curl -N /events                 (SSE capture)
kill + restart against the same --data-dir
```

## A. Verified working (executed)

| Claim | Evidence | Verdict |
| --- | --- | --- |
| Real collectors hit real APIs | `collect` returned 123 observations from USGS/NASA/HN/GitHub | VERIFIED BY EXECUTION |
| Rate limiting is not "zero activity" | GDELT `HTTP 429` recorded as `ratelimited`, 0 observations, `collector_rate_limited_total 1` | VERIFIED BY EXECUTION |
| Continuous scheduler loop | `--collect` ran USGS every 60s, HN/GitHub/GDELT/NASA on their cadences | VERIFIED BY EXECUTION |
| SSE live stream | captured `observation`, `anomaly`, `event`, `signal` frames over `/events` | VERIFIED BY EXECUTION |
| SQLite persistence + rehydration | restart logged `rehydrated detector state from storage series=6`; observations/signals survived | VERIFIED BY EXECUTION |
| Signal identity persists across cycles | same `sig_…` ids over 3+ cycles; evidence grew 2→3 on one signal | VERIFIED BY EXECUTION |
| Source health is separate from data | `/sources/:id` carries health; `/metrics` counts failures separately | VERIFIED BY EXECUTION |
| Drill-down reaches raw data | `signal → event → observation → source → /raw` resolves | VERIFIED BY EXECUTION |
| API auth | every data route 401 without key; `/health` stays open | VERIFIED BY CODE INSPECTION + tests |

## B. Incomplete, broken, or missing

| # | Finding | Verdict |
| --- | --- | --- |
| 1 | `Cadence::Daily` is discarded: NASA declares `daily 06:00Z` but `schedule_for` maps it to `Interval { 86_400 }`, so it runs every 24h **from process launch** (observed next run 00:42, not 06:00Z). The catalog does not describe reality. | BROKEN |
| 2 | `wse serve` without `--collect` serves a **static empty dashboard**: `collection_enabled: true` but zero collectors run, `observations: 0`, and the UI shows no "monitoring off" state. A user believes monitoring is active. | BROKEN |
| 3 | The **World screen is not live**. `startActivityStream()` is called only by `systemView()`; `worldView()` fetches once and never re-fetches. A new signal does not appear until manual navigation/refresh. | BROKEN |
| 4 | No **startup default**: monitoring requires the operator to know `--collect`. | PARTIALLY IMPLEMENTED |
| 5 | No **latency telemetry** for detection/signal formation/UI delivery; only observation `lag_ms` and collector latency exist. | MISSING |
| 6 | Cold-start degenerate baseline: GitHub emitted **`+1527σ`** from a median-0/MAD-0 history (classical z on a near-zero σ). Honest-ish (flagged cold-start) but not a usable magnitude. | PARTIALLY IMPLEMENTED |
| 7 | Coverage is **4 narrow domains** (geophysics, space, technology ×2, and one GDELT topic "oil supply"). Calling this "the world" is not justified. | PARTIALLY IMPLEMENTED |
| 8 | No acceptance test for the **live SSE → world-update** path; the SSE test only asserts a control event. | MISSING |
| 9 | `IMPACT` type has no producer. | MISSING (documented) |
| 10 | No labelled ground truth for real sources; backtest precision/recall is UNLABELLED. | DOCUMENTED (honest) |

## C. Source coverage matrix (as audited)

| Source | Domain | Declared cadence | Observed cadence | Auth | Real data this run | Health |
| --- | --- | --- | --- | --- | --- | --- |
| `usgs_earthquakes` | geophysics | event (poll 60s) | 60s | none | 9 obs | healthy |
| `nasa_neo` | space | daily 06:00Z | **86400s from launch** | apikey (DEMO_KEY) | 35 obs | healthy |
| `gdelt_news_volume` | global_events | 900s | 900s | none | **0 — rate limited** | rate_limited |
| `hackernews_frontpage` | technology | 600s | 600s | none | 29 obs | healthy |
| `github_rust_activity` | technology | 3600s | 3600s | token (optional) | 50 obs | healthy |

The five sources cover four categories, two of which (`technology` ×2) share
one underlying phenomenon. Cross-source convergence is therefore
*architecturally* possible but has almost no real material to work with.

## D. The real-time path, as it exists

```text
SOURCE → COLLECTOR            wse-sources (http_collector!)            WORKS
       → OBSERVATION          wse-normalize / wse-model                WORKS
       → DETECTION            wse-detection (anomaly, early)           WORKS
       → EVENT                wse-signals::event                      WORKS
       → SIGNAL               wse-signals + wse-presentation          WORKS
       → EVENT STREAM         RuntimeState → /events (SSE)             WORKS
       → UI                   web/app.js  …                          ONLY ON SYSTEM SCREEN
```

The chain is real end to end. The last hop — the stream reaching the **World**
screen — is the broken link (finding 3).

## E. Startup behavior (as audited)

- `wse serve` → empty world, no monitoring, no warning. **Misleading.**
- `wse serve --collect` → monitoring starts, world populates within one cadence
  (USGS/HN/GitHub immediately; GDELT within 900s; NASA once).
- `wse serve --synthetic --live` → a deterministic synthetic world, for demos.

## F. What this means for the product

The engine genuinely answers *"what changed abnormally since the last cycle?"*
and *"show me the evidence"* — **if an operator knows to pass `--collect` and
manually reloads the World screen.** Neither of those should be required of the
person the product is for. Findings 1–5 are the gap between "a working engine"
and "a world watch you can open".

## G. Priorities this audit sets

1. Make the World screen live over the existing event stream (finding 3).
2. Make monitoring the default and show its state honestly (findings 2, 4).
3. Make `Cadence::Daily` mean what the catalog says (finding 1).
4. Widen coverage with sources that create real convergence (finding 7).
5. Expose real latency telemetry (finding 5).
6. Be honest about cold-start baselines (finding 6).
7. Prove the live path with an automated acceptance test (finding 8).

Findings 9 and 10 are left as documented limitations rather than faked.
