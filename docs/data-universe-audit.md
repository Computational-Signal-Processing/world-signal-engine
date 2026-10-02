# Data Universe Audit — is the engine domain-independent?

**Phase B5.** This document answers one question: *can the World Signal Engine
carry domains it has never seen — FX, crypto, equities, commodities, energy,
weather, satellites, research, cyber, news — without changing its core?*

It is an **audit**, not a feature. It was produced by reading the code and
running it, not by planning. Every claim below names a real type, route, trait
or test. Where the answer is "no", the gap is named rather than hidden.

Audit run: 2026-09-30, `main` at `177dc5d`, Rust 1.99.0.
`cargo test --workspace` → **544 passed, 0 failed**.

---

## A. Current source inventory

21 sources, 21 collectors, enforced 1:1 by the `wse-cli` unit test
`every_catalog_source_has_a_collector` (in `crates/cli/src/main.rs`).
`wse sources` prints them; the real catalog is `wse_sources::catalog()`.

| id | category | subcategory | metric(s) | unit | cadence | tier | geospatial |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `usgs_earthquakes` | geophysics | seismology | `earthquake_magnitude` | magnitude | event | 1 | yes |
| `afad_earthquakes` | geophysics | seismology | `earthquake_magnitude` | magnitude | 3600s | 1 | yes |
| `nasa_neo` | space | near_earth_objects | `neo_close_approaches` | approaches | daily@6Z | 1 | no |
| `noaa_kp_index` | space | geomagnetic_activity | `kp_index` | kp | 3600s | 1 | no |
| `noaa_goes_xray` | space | solar_activity | `xray_flux` | W/m² | 900s | 1 | no |
| `nws_alerts` | weather | public_safety | `active_weather_alerts` | alerts | 600s | 1 | yes |
| `open_meteo_weather` | weather | surface_observation | `temperature_2m`, `precipitation` | °C, mm | 1800s | 2 | yes |
| `open_meteo_air_quality` | environment | air_quality | `pm2_5`, `pm10` | µg/m³ | 3600s | 2 | yes |
| `nasa_eonet` | earth | natural_events | `open_natural_events` | events | 1800s | 1 | yes |
| `gdacs_disasters` | disasters | multi_hazard | `active_alerts` | alerts | 1800s | 1 | yes |
| `gdelt_news_volume` | global_events | news_volume | `news_volume` | percent | 900s | 2 | no |
| `cisa_kev` | cyber | exploited_vulnerabilities | `kev_added` (derived from `kev_catalog_total`) | vulnerabilities | 86400s | 1 | no |
| `hackernews_frontpage` | technology | developer_attention | `story_score` | points | 600s | 3 | no |
| `github_repo_universe` | technology | software_ecosystem | `repo_stars` | stars | 3600s | 3 | no |
| `npm_downloads` | technology | package_downloads | `weekly_downloads` | downloads | 86400s | 2 | no |
| `pypi_downloads` | technology | package_downloads | `weekly_downloads` | downloads | 86400s | 2 | no |
| `coingecko_market` | markets | crypto_spot | `spot_price` | usd | 900s | 2 | no |
| `ecb_exchange_rates` | finance | exchange_rate | `exchange_rate` | rate | 86400s | 1 | no |
| `crossref_works` | science | publication_velocity | `works_registered` | works | 86400s | 2 | no |
| `arxiv_submissions` | science | preprint_velocity | `preprint_new` (derived from `preprint_total`) | preprints | 86400s | 2 | no |
| `who_outbreaks` | health | outbreak_news | `outbreak_news` | items | 86400s | 1 | no |

Real endpoints (verified in source, not invented): `earthquake.usgs.gov`,
`deprem.afad.gov.tr`, `api.nasa.gov/neo`, `services.swpc.noaa.gov`,
`api.weather.gov`, `api.open-meteo.com`, `air-quality-api.open-meteo.com`,
`eonet.gsfc.nasa.gov`, `gdacs.org`, `api.gdeltproject.org`, `cisa.gov`,
`hacker-news.firebaseio.com`, `api.github.com`, `api.npmjs.org`,
`pypistats.org`, `api.coingecko.com`, `data-api.ecb.europa.eu`,
`api.crossref.org`, `export.arxiv.org`, `who.int`.

## B. Current domain inventory

11 distinct `category` strings are in use:

| category | sources | notes |
| --- | --- | --- |
| `geophysics` | 2 | two independent seismic sensors (global + Turkey) |
| `space` | 3 | Kp + GOES X-ray + NEO — two independent space-weather sensors |
| `weather` | 2 | |
| `environment` | 1 | |
| `earth` | 1 | |
| `disasters` | 1 | |
| `global_events` | 1 | |
| `cyber` | 1 | |
| `technology` | 4 | |
| `markets` | 1 | |
| `finance` | 1 | |
| `health` | 1 | |
| `science` | 2 | |

**Key structural fact:** `category` lives on the **`Source`**, not on the
observation or the metric. One source emits exactly one category. Events and
signals acquire their category by looking the candidate's source up in the
catalog (`category_of` in `wse-engine::ingest_observations`). This is what makes
the whole downstream model domain-agnostic — see E.

## C. Canonical observation model

`wse_model::Observation` — the single normalized shape every source must
produce:

```text
id            deterministic (series + observed_at + record key)
source_id     which sensor
observed_at   when the world produced it (per source)
received_at   when we got it            → lag_ms()
entity_id     optional canonical subject (prefixed slug)
metric        what was measured
value         f64
unit          canonical unit string
latitude/longitude   optional
quality       Quality (source timestamp vs received, plausibility)
raw           RawReference {locator, hash, content_type, bytes}
dimensions    BTreeMap — folded into series_key (grouping)
attributes    BTreeMap — NOT in series_key (per-record detail, drill-down)
identity      per-record discriminator (several records per series/timestamp)
derivation    Option<DerivationProvenance> (derived series provenance)
```

Series identity is `source::entity::metric::unit[|dim…]`. **The model is a
generic (source, entity, metric, unit, time, value) tuple with provenance.**
Nothing in it names a domain. The richest single field is `entity_id`, which is
an opaque `EntityId` string.

## D. Source-specific assumptions

Where the code actually branches on a source or a domain. Exhaustive:

1. **`wse-presentation::vocabulary`** — `metric_vocab`, `entity_vocab`,
   `category_label`, `source_label` are hard-coded maps keyed by metric name,
   entity prefix and source id (`"earthquake_magnitude"`, `"kp_index"`,
   `"usgs_earthquakes"`, …). This is the **only** place a new domain must be
   registered to read well. It is *not* load-bearing: an unmapped metric renders
   from its own name and is explicitly marked as having no richer description
   (`narrative.rs` → `unknowns_for(..., vocab.is_some(), ...)`). So it degrades
   gracefully but reads poorly.
2. **`wse-api::handlers::OBSERVATORY_CATEGORIES`** — a preferred *display order*
   for the observatory board. It is intersected with the catalog and **unknown
   categories are appended**, so a new domain appears without a code change;
   only its board ordering is unopinionated.
3. **`config/impact/systemic.yaml`** — the declared IMPACT scope names
   categories (`finance`, `cyber`) and an entity (`Hormuz`). This is a
   *declaration*, deliberately data, not a code branch.
4. **`config/lenses/*.yaml`** — lens filters name categories/entities/keywords.
   Data, not code.
5. **`crates/cli/tests/lens_coverage.rs::INTENTIONALLY_UNFED`** — a test constant
   naming `lens_energy` as a knowingly-unfed lens. A test invariant, not runtime.

No collector, detector, baseline, storage, event, signal or correlation code
names a domain. Verified by grep: the only hits for `earthquake|solar|usgs|…`
outside tests are the four items above.

## E. Domain-agnostic parts

These carry any domain unchanged:

- **`Collector` trait** (`wse-collector`) — `source_id`, `schedule`, `mode`,
  `collect()`. No domain vocabulary.
- **`Observation` / `RawReference` / `Quality`** — generic tuple + provenance.
- **Storage** — traits `ObservationStore`, `EventStore`, `SignalStore`,
  `SourceStore`, `BaselineStore`, `RawStore`, `MaintenanceStore`. Indexed by
  series/source/entity/category/lens; no domain columns.
- **Baseline** (`wse-baseline`) — rolling mean/median/std/MAD/EWMA/percentile,
  robust z-score. Pure statistics over a `Vec<f64>`.
- **Detection** (`wse-detection`) — change, robust z, z, velocity, persistence
  drift, regime shift. Operates on a `SeriesTracker`; never sees a source.
- **Event engine** (`wse-signals::event`) — groups candidates by entity/series +
  window + direction.
- **Signal engine** (`wse-signals`) — five types, multi-dimensional quality,
  evidence. Category is carried from the catalog, not computed.
- **Correlation** (`wse-correlation`) — entity-segment subset matching +
  geographic radius. Domain-agnostic by construction.
- **Lens** (`wse_model::lens`) — optional category/entity/keyword/bbox filters.
- **API** — `/signals`, `/events`, `/observations`, `/sources`, `/entities`,
  `/timeline`, `/world`, `/observatory` all iterate the catalog/store.
- **Web data bus** (`web/js/data/store.js`, `adapters.js`) — domains are keyed by
  endpoint; adapters map JSON generically; no source name appears.
- **Catalog fields that generalize the semantics of a source without naming it:**
  `tier` (provenance), `measurement` (`stable_series` / `fixed_universe` /
  `unstable_population`), `derivations` (declared delta), `feeds_lenses`. These
  are the mechanism that lets a new source be *honest* about its data with **no
  core branch** — the engine reads the declaration, not the source name.

## F. Domain-specific parts

Only the five items in section D. Of these, only (1) `vocabulary` is a real
per-domain cost; the rest are data/test constants.

## G. New source onboarding contract

To add a domain, today, with no core change:

```text
1. crates/sources/src/<source>.rs
     - pub fn source() -> Source            (catalog entry)
     - pure `parse(bytes, received_at) -> Result<Vec<Observation>>`
     - a Collector that fetches and calls parse
2. crates/sources/src/lib.rs
     - `pub mod <source>;` + add `source()` to `catalog()`
3. crates/sources/src/collectors.rs
     - add the collector to `live_collectors()`
4. (recommended, not required)
     - vocabulary entries: metric_vocab / source_label / category_label
     - config/lenses/<domain>.yaml + declare `feeds_lenses` on the Source
     - add the category to OBSERVATORY_CATEGORIES for board ordering
```

`every_catalog_source_has_a_collector` (a `wse-cli` unit test) and
`every_source_feeds_a_lens_that_exists` (in `crates/cli/tests/lens_coverage.rs`)
make steps 2–3 self-checking. **The contract is a module plus two list entries.** That is the strongest evidence
that the engine is already source-independent.

## H. Required backend changes

For a **new scalar sensor in any domain** (FX, crypto, equity, commodity,
weather, satellite count, research, cyber): **none to the engine.** A collector,
a catalog entry and (optionally) vocabulary.

For the whole target universe, two real, non-blocking gaps:

1. **Entity identity is free-form per collector.** Each collector mints its own
   `entity_id` slug (`region_hormuz`, `crypto`, `fx_usd_eur`, `software`). There
   is no canonical entity registry. Cross-source convergence *within* a domain
   works when collectors happen to share segments; **cross-domain** convergence
   (an FX move + a news-volume move about the same currency) needs a shared
   entity vocabulary. This is the single most important architectural gap.
2. **No entity store.** `Entity` (with `kind`, `canonical`, `aliases`) exists in
   `wse-model` but there is no `EntityStore` trait or table; `/entities/{id}`
   reconstructs an entity by searching signals and observations. Fine for
   drill-down; insufficient for a curated cross-domain entity registry.

## I. Required Data Bus changes

**None.** `store.js` declares each domain as `{ path, adapt },` keyed by
endpoint, with generic adapters (`adapters.js` maps JSON shape, never a source
name). A new source's data reaches blocks through the existing `/world`,
`/observatory`, `/sources`, `/signals` domains. A brand-new *screen* would add a
domain, not change one.

## J. Required API changes

**None for new sources.** `/signals` already filters by `category`, `entity`,
`type`, `lens`, `status`, `time`; `/sources` and `/observatory` iterate the
catalog. A domain-scoped listing is expressible as `?category=…` today.

## K. Required Storage changes

**None for new sources.** The schema indexes `observations(series_key,
observed_at)`, `observations(source_id)`, `observations(entity_id)`,
`signal_categories(category)`, `signal_entities(entity)`, `signal_lenses(lens)`.
New categories and metrics are rows, not columns. Optional future work: an
`entities` table if a canonical entity registry (H) is added.

## L. Required Signal Engine changes

**None for new scalar sensors.** Signals form from candidates and events; the
category is carried from the catalog. Convergence and IMPACT already consume
entity/category declarations. The only improvement that a wide universe needs is
the entity vocabulary in H.1.

## M. What can already support FX / Crypto / Markets / Space / Weather

- **Crypto** — already connected (`coingecko_market`): `spot_price`, unit `usd`,
  900s cadence, StableSeries. A second exchange would converge by shared entity.
- **FX** — already connected (`ecb_exchange_rates`): `exchange_rate`, 86400s,
  Daily. Intraday FX needs only an `Interval` cadence collector.
- **Markets (equities/indices/bonds/commodities/energy)** — the model supports
  them with no change: a scalar price/index/level per instrument, `identity` or
  a `dimensions` entry (`{"instrument": "BRENT"}`) to keep per-instrument
  series, `StableSeries`. Nothing in baseline/detection/storage assumes geology.
- **Space** — 3 sensors already; a satellite-count or astronomy-catalog series
  is the same shape.
- **Weather** — 2 sensors already, geospatial, two metrics each.

The generic machinery that makes these work — deterministic ids, `identity` for
many-records-per-series, `derivations` for cumulative counters,
`measurement: unstable_population` to exclude a churning top-N, `feeds_lenses`,
`tier` — is already in place and tested.

## N. What cannot (today, without new work)

1. **Cross-domain convergence.** Without a shared entity vocabulary (H.1), an
   FX series and a news series about the same currency have different
   `entity_id`s and will not converge. Convergence is entity/geography based and
   domain-agnostic in *code*, but depends on *data* (entity ids) agreeing.
2. **Sub-second / streaming push.** `Collector::collect()` is request/response;
   `Schedule` is poll-based. `Protocol::Websocket` exists as an enum value but no
   collector uses it. High-frequency tick feeds are not supported by the contract
   (SSE exists only server→browser).
3. **Trading-calendar cadence.** `Schedule` has `Event`, `Interval`, `Daily
   { hour_utc }`, `Manual`. There is no "weekdays at 16:00 exchange-local" or
   holiday-aware schedule; business-day gaps are handled downstream (decision
   0021) but the *schedule* cannot express a market calendar.
4. **Non-numeric / text sources.** A paper title, a headline, a CVE description
   reduce to a *count*. There is no text extraction, NER or entity-from-text, so
   research/news domains can be measured only as volume, not content.
5. **Unstable populations are (correctly) undetectable.** Live position maps,
   top-N search results and similar churning sets cannot be a series
   (`measurement: unstable_population` excludes them). That is the intended
   behaviour, but it means "transport live positions" is not a series by design.

## O. Minimum architecture changes

Ordered by leverage. None is required to connect more scalar sensors.

1. **Entity vocabulary / canonical entity ids** (closes H.1 and N.1). A shared
   slug convention or a small registry so `USD`, `Brent`, `Hormuz` are the same
   entity across domains. This is what turns many sources into real convergence.
2. **Vocabulary coverage** (D.1). Per-domain `metric_vocab`/`source_label`/
   `category_label` entries, so new signals read well. Designed for, additive.
3. **(Optional) `EntityStore`** — a small table for the registry above.
4. **(Optional) Schedule expressiveness** — market-calendar cadence (N.3).
5. **(Optional) `unstable_population` for live feeds** — already handled; document
   the pattern for transport.

## P. Risks

- **False convergence without entity discipline.** If collectors mint
  overlapping-but-different slugs, correlation either misses real convergence or
  (worse) merges unrelated ones. The subset rule already guards the second case;
  the first is the real risk of a wide universe.
- **Vocabulary drift.** A new metric with no vocabulary reads as a bare metric
  name. Not wrong, but the "human language" promise degrades. A test reminder
  exists (`every_connected_metric_has_vocabulary`).
- **Cadence mismatch.** A source polled faster than it updates yields duplicate
  observations (de-duplicated, correctly) but wasted polls; a source polled
  slower than its cadence misses change. Per-source `Schedule` is the control.
- **Measurement-semantics mistakes.** Marking a churning set as `stable_series`
  would manufacture anomalies. The field exists precisely to prevent this; the
  risk is a collector author choosing it wrongly.
- **Silent coverage gap.** A domain with one sensor cannot converge. The
  taxonomy doc and the lens-coverage test surface this; it must stay a decision.

---

## Verdict

The engine is **domain-independent in structure and in most of its behaviour.**
The canonical model is a generic `(source, entity, metric, unit, time, value)`
tuple; storage, baseline, detection, events, signals, correlation, lenses, the
API and the web data bus contain **no domain branches**. Adding a new scalar
sensor is a module plus two list entries, self-checked by tests.

The universe can grow to FX, crypto, equities, commodities, energy, weather,
satellites, research and cyber **without changing the core**. What a wide
universe needs is not new architecture but **shared entity identity** (so
independent domains can converge) and **vocabulary coverage** (so new signals
read well). Neither is a blocker for connecting the next source; both are the
work that makes many sources worth more than many sources.
