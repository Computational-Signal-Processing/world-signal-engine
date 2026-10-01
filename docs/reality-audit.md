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
| 6 | Cold-start degenerate baseline: GitHub emitted **`+1527σ`** from a median-0/MAD-0 history (classical z on a near-zero σ), because all repositories pooled into one series. | FIXED (CAP-2D) |
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

## H. Follow-up status (2026-10-01, after productization)

| # | Finding | Status | Evidence |
| --- | --- | --- | --- |
| 1 | Daily cadence runs 24h from launch | FIXED | `schedule_for` maps `Daily { hour_utc }` to the next wall-clock hour; verified next run at the declared hour. |
| 2 | `serve` without `--collect` serves an empty dashboard | FIXED | Monitoring is the default for `serve`; `/health` reports `monitoring`, `collector_active`, `collection_enabled`. |
| 3 | World screen is not live | FIXED | `worldView` opens the stream, re-fetches on a `SIGNAL` frame, and shows a new-signal banner. Verified in the browser: "7 new signals since you started watching". |
| 4 | Monitoring requires `--collect` | FIXED | Same as finding 2. |
| 5 | No latency telemetry | FIXED | System screen shows source lag, collector fetch, detection, and newest-signal age, each measured from stored timestamps. |
| 6 | Cold-start degenerate baseline | FIXED (CAP-2D) | GitHub's pooled baseline is gone: each repository is its own series (`repo` dimension), so a first appearance is judged against that repository's own history. See `docs/decisions/0020-github-per-repo-series.md`. |
| 7 | Coverage is 4 narrow domains | IMPROVED | 13 sources across 9 categories: geophysics ×2, space ×2, weather, earth, global events, cyber, finance, science ×2, technology ×2. See *Source network* below. |
| 8 | No acceptance test for the live SSE → world path | PARTIAL | The activity wire contract (SCREAMING_SNAKE_CASE kinds) is now locked by an API test; the full UI refresh path is verified by hand, not yet automated. |
| 9 | `IMPACT` has no producer | OPEN (documented) | |
| 10 | No labelled ground truth | OPEN (documented) | |

### New sources added

| Source | Domain | Cadence | Auth | What it measures |
| --- | --- | --- | --- | --- |
| `nws_alerts` | weather | 600s | none | active weather alerts per severity, national + per US state |
| `nasa_eonet` | earth | 1800s | none | open natural events (wildfires, storms, volcanoes) per category, geospatial |

Both are snapshots: their counts fall as well as rise, which exercises the
baseline engine in both directions. Both are verified live (72 and 13 records
per poll respectively). `nasa_eonet` gives the map real coordinates.

## I. Source network (2026-10-01, evidence-based expansion)

Finding 7 is the one this pass exists to close. Coverage was widened with
sources chosen for *measurement character* and *independence*, not count. Each
new source was picked so that two things hold: the measurement is honest (a
stable, comparable quantity, not membership churn), and it is independent of the
sources already connected, so cross-source convergence has real material.

| Source | Domain | Tier | Measurement | Verified live |
| --- | --- | --- | --- | --- |
| `afad_earthquakes` | geophysics (Turkey) | 1 | stable series, per province | 16 obs |
| `noaa_kp_index` | space weather | 1 | stable series | 56 obs |
| `cisa_kev` | cyber | 1 | stable series ×2 | 2 obs |
| `ecb_exchange_rates` | finance | 1 | stable series | 7 obs |
| `crossref_works` | science | 2 | stable series, fixed topic list | 3 obs |
| `arxiv_submissions` | science | 2 | stable series, fixed category list | 4 obs |

All six were verified against the live APIs with `wse collect` (counts above are
from one run). `gdelt_news_volume` and `crossref_works` are rate-limited by their
providers and record that honestly as a source failure rather than as zero
activity — the distinction the brief's rule 29 requires.

### What changed structurally

Two fields were added to the catalog and are now load-bearing:

- **`measurement`** (`stable_series` / `fixed_universe` / `unstable_population`).
  The engine reads it before detecting: an `unstable_population` source is
  stored for evidence but never detected on, because its aggregate is membership
  churn, not a world change. This is enforced in `Engine::ingest` and locked by
  `an_unstable_population_is_stored_but_never_detected_on`.
- **`feeds_lenses`**. Every source declares which lenses it backs, and a test
  (`crates/cli/tests/lens_coverage.rs`) fails if a source feeds a lens that does
  not exist, or if a lens has no connected source and is not declared
  intentionally unfed. Coverage is a checked property, not a claim.

### Lens coverage after this pass

| Lens | Backed by |
| --- | --- |
| EARTH | USGS, AFAD, NWS, EONET |
| SPACE | NASA NEO, NOAA Kp |
| GLOBAL EVENTS | GDELT |
| CYBER | CISA KEV |
| FINANCE | ECB |
| SCIENCE | Crossref, arXiv |
| AI | Crossref, arXiv |
| SOFTWARE | Hacker News, GitHub |
| TURKEY | USGS, AFAD (geographic) |

`AGRICULTURE` and `ENERGY` remain intentionally unfed and are declared as such
in the coverage test; they are placeholders, not accidents.
