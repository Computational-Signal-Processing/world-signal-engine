# Source Catalog

The catalog is the engine's foundational asset. A source is described by metadata,
not by code: adding a source means adding an entry here and a collector module,
and nothing in the core changes.

The catalog lives in `wse_sources::catalog()`. `wse sources` prints it.

## Metadata

Every source declares:

```text
source_id
name
provider
category
subcategory
endpoint
protocol
format
cadence
timezone
license
authentication
cost
historical_available
realtime_available
geospatial
entities
priority
enabled
collector_type
parameters
tier                 provenance tier (1 = institutional, 4 = personal)
measurement          measurement semantics (see below)
feeds_lenses         the lens ids this source is expected to feed
```

`license` is mandatory: the catalog tests fail if any entry omits it, so usage
stays lawful by construction. `priority` orders collection and display; lower is
more important.

### Provenance tier

`tier` records how much the number can be trusted, and how it should be read:

| tier | meaning | examples |
| --- | --- | --- |
| `Tier1` | institutional, authoritative, free, stable | USGS, NOAA, NASA, ECB, CISA |
| `Tier2` | well-run open dataset with a stated methodology | GDELT, Crossref |
| `Tier3` | community/derived signal; useful but noisier | Hacker News, GitHub |
| `Tier4` | personal or experimental; the default | (none shipped) |

Tier is provenance, not importance. A tier-3 source can produce a strong signal;
tier just tells the reader how much the raw number is worth on its own.

### Measurement semantics

The most important field for detection honesty. It says whether two observations
of the same series are *comparable*, which is the precondition for any baseline:

| measurement | meaning | example |
| --- | --- | --- |
| `stable_series` | an authoritative measurement of a fixed subject, repeated | ECB USD/EUR rate |
| `fixed_universe` | a declared set of members, each re-measured | GitHub repo stars |
| `unstable_population` | the membership itself changes between collections | a top-N ranking or search result |

A series whose population changes underneath it (`unstable_population`, e.g. "all
repositories matching a search") cannot be baselined: a rise may just be more
members. The engine refuses to form signals from such a series. `fixed_universe`
is the fix: pin the members, then the count means something.

## Current sources

| id | tier | measurement | category | protocol | format | cadence | auth | cost | geo | priority |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `usgs_earthquakes` | 1 | stable_series | geophysics | https | geojson | event | none | free | yes | 10 |
| `afad_earthquakes` | 1 | stable_series | geophysics | https | json | 3600s | none | free | yes | 15 |
| `nasa_neo` | 1 | stable_series | space | https | json | daily 06:00Z | api key | free w/ registration | no | 20 |
| `nws_alerts` | 1 | stable_series | weather | https | geojson | 600s | none | free | yes | 20 |
| `nasa_eonet` | 1 | stable_series | earth | https | json | 1800s | none | free | yes | 25 |
| `noaa_kp_index` | 1 | stable_series | space | https | json | 3600s | none | free | no | 25 |
| `gdelt_news_volume` | 2 | stable_series | global_events | https | json | 900s | none | free | no | 30 |
| `cisa_kev` | 1 | stable_series | cyber | https | json | daily | none | free | no | 35 |
| `hackernews_frontpage` | 3 | fixed_universe | technology | https | json | 600s | none | free | no | 40 |
| `ecb_exchange_rates` | 1 | stable_series | finance | https | json | daily | none | free | no | 44 |
| `crossref_works` | 2 | stable_series | science | https | json | daily | none | free | no | 45 |
| `arxiv_submissions` | 2 | stable_series | science | https | atom | daily | none | free | no | 46 |
| `github_rust_activity` | 3 | fixed_universe | technology | https | json | 3600s | token | free w/ registration | no | 50 |

### usgs_earthquakes

- **Provider:** U.S. Geological Survey
- **Endpoint:** USGS "all hour" earthquake feed
- **License:** public domain
- **Entities:** `earthquake`
- **Geospatial:** yes
- **Notes:** event-driven rather than polled; the feed only changes when the earth
  does. Observations carry magnitude and coordinates.

### afad_earthquakes

- **Provider:** AFAD (Turkish Disaster and Emergency Management Presidency)
- **Endpoint:** AFAD earthquake event-filter API
- **License:** AFAD open data terms
- **Entities:** `province_*` (one series per Turkish province)
- **Geospatial:** yes
- **Notes:** the Turkey lens's home source, and genuinely independent of USGS:
  it reports events USGS thresholds away. Groups by province (one series per
  province, matching USGS's region grouping), and the window is computed at
  collection time so a long-running process does not freeze it. AFAD reports
  local time (UTC+3, fixed year-round since 2016) and the parser converts it.

### nasa_neo

- **Provider:** NASA / JPL
- **Endpoint:** Near-Earth Object feed
- **License:** public domain (NASA)
- **Entities:** `near_earth_object`
- **Notes:** daily cadence. Requires an API key, but works with the `DEMO_KEY`
  default so the engine is runnable out of the box.

### nws_alerts

- **Provider:** U.S. National Weather Service
- **Endpoint:** `https://api.weather.gov/alerts/active?status=actual&message_type=alert`
- **License:** public domain (US Government)
- **Entities:** `weather`, `united_states`
- **Notes:** a real-time public-safety feed. Tracks the count of active alerts
  per severity, nationally (`weather_united_states`) and per US state
  (`weather_us_tx`). It is a snapshot, so the count falls as well as rises — a
  useful test of the baseline in both directions. A severity with no alerts is
  recorded as `0`, not omitted.

### nasa_eonet

- **Provider:** NASA Earth Observatory
- **Endpoint:** `https://eonet.gsfc.nasa.gov/api/v3/events?status=open`
- **License:** public domain (NASA)
- **Entities:** `natural_events`, `earth`
- **Notes:** open natural events — wildfires, severe storms, volcanoes — counted
  per category. Geospatial: each category carries the newest event's coordinate
  so the map has a real position. Closed events are not counted.

### noaa_kp_index

- **Provider:** NOAA Space Weather Prediction Center
- **Endpoint:** `https://services.swpc.noaa.gov/products/noaa-planetary-k-index.json`
- **License:** public domain (US Government)
- **Entities:** `geomagnetic_kp`
- **Notes:** the planetary K-index — geomagnetic disturbance, in thirds of a
  unit. A fixed, authoritative series; a jump is a real geomagnetic storm. Feeds
  the SPACE lens alongside NASA NEO.

### gdelt_news_volume

- **Provider:** The GDELT Project
- **Endpoint:** GDELT timeline volume API
- **License:** free for research and commercial use (GDELT terms)
- **Entities:** `oil`
- **Parameters:** `mode=timelinevol`, `format=json`, `query=oil`
- **Notes:** the global-events source. GDELT rate-limits aggressively and returns
  a plain-text notice with HTTP 429 instead of JSON; the parser rejects non-JSON
  bodies explicitly so a rate-limited response is recorded as a source failure,
  never as "zero news volume". See *Degraded sources* below.

### cisa_kev

- **Provider:** CISA (US Cybersecurity and Infrastructure Security Agency)
- **Endpoint:** Known Exploited Vulnerabilities catalogue JSON
- **License:** public domain (US Government)
- **Entities:** `cyber_kev`
- **Notes:** emits two series: `kev_added` (catalogue additions in the recent
  window) and `kev_catalog_total` (catalogue size). A rise in `kev_added` is a
  real increase in vulnerabilities being exploited in the wild — the CYBER lens's
  home source. `kev_catalog_total` only ever grows, so it is **evidence-only**:
  the catalogue declares a `Delta` derivation to `kev_catalog_growth` (the
  vulnerabilities added since the previous poll), and that derived series is
  what detection runs on. See
  [docs/decisions/0015-cisa-derived-growth.md](docs/decisions/0015-cisa-derived-growth.md).

### hackernews_frontpage

- **Provider:** Y Combinator
- **Endpoint:** Hacker News top-stories and item API
- **License:** public API, no key required
- **Entities:** `software_ecosystem`
- **Notes:** a proxy for developer attention. `measurement: fixed_universe`: the
  collector tracks a fixed set of story ids resolved from the top-story list, so
  a story's score is comparable across collections. Story scores make a usable
  time series for the technology lens.

### ecb_exchange_rates

- **Provider:** European Central Bank
- **Endpoint:** ECB SDMX data API (EXR series)
- **License:** ECB terms; reference rates are free to reuse
- **Entities:** `fx_usd_eur`
- **Notes:** the euro reference exchange rate, an institutional daily series. A
  move here is a real move in the rate. The FINANCE lens's home source.

### crossref_works

- **Provider:** Crossref
- **Endpoint:** `https://api.crossref.org/works`
- **License:** Crossref REST API terms; metadata is open
- **Entities:** `research_*` (one series per tracked topic)
- **Parameters:** `window_days=2`, a fixed topic list
- **Notes:** counts works registered in a recent window for each of a fixed set
  of topics (artificial intelligence, machine learning, climate change, CRISPR,
  quantum computing). The topic list is fixed, so a change is a change in
  research output. Feeds the SCIENCE and AI lenses.

### arxiv_submissions

- **Provider:** arXiv (Cornell University)
- **Endpoint:** `https://export.arxiv.org/api/query`
- **License:** arXiv API terms; metadata is open
- **Entities:** `arxiv_*` (one series per category)
- **Parameters:** a fixed category list (`cs.AI`, `cs.LG`, `cs.CL`, `cs.CV`)
- **Notes:** the Atom feed's `opensearch:totalResults` is the raw cumulative
  measurement (`preprint_total`). The catalog declares
  `preprint_new = Delta(preprint_total)`; the raw level is stored but
  evidence-only, and detection runs on the derived per-interval velocity.
  Feeds the SCIENCE and AI lenses.

### github_rust_activity

- **Provider:** GitHub
- **Endpoint:** GitHub repository API (one request per repository)
- **License:** public API; GitHub Acceptable Use terms apply
- **Entities:** `ecosystem_rust`
- **Notes:** `measurement: fixed_universe`. The collector walks a fixed list of
  well-known Rust repositories and measures stars for each. Token authentication
  is optional and only raises the rate limit. If every request fails the
  collection is a source failure, never "zero activity".

## Source health

Health is tracked separately from observations and is never rendered as world
activity:

```text
last_success, last_failure, last_latency_ms
records_received, records_changed, records_duplicate
error_count, consecutive_failures
status
```

`GET /sources/:id` returns the catalog entry together with its health.

## Degraded sources

A source can be up, degraded, or failing. The engine is explicit about which:

- **`NO DATA`** — the collector failed. Health records the error. No observation
  is produced.
- **`DATA = ZERO`** — the collector succeeded and the world genuinely reported
  zero.

These are different facts, stored differently, and rendered differently. A
rate-limited GDELT is `NO DATA`.

## Adding a source

1. Add a module under `crates/sources/src/` with:
   - a `source()` catalog entry,
   - a pure `parse(bytes, received_at) -> Vec<Observation>`,
   - a `Collector` that fetches and calls `parse`.
2. Add a checked-in fixture under `tests/fixtures/` and unit-test `parse`
   against it.
3. Register the entry in `wse_sources::catalog()` and the collector in
   `live_collectors()`.

No core code changes. See [DEVELOPMENT.md](DEVELOPMENT.md) for the walkthrough.
