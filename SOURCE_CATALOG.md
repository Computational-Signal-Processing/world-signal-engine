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
```

`license` is mandatory: the catalog tests fail if any entry omits it, so usage
stays lawful by construction. `priority` orders collection and display; lower is
more important.

## Current sources

| id | category | protocol | format | cadence | auth | cost | geo | priority |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `usgs_earthquakes` | geophysics | https | geojson | event | none | free | yes | 10 |
| `nasa_neo` | space | https | json | daily 06:00Z | api key | free w/ registration | no | 20 |
| `nws_alerts` | weather | https | geojson | 600s | none | free | yes | 20 |
| `nasa_eonet` | earth | https | json | 1800s | none | free | yes | 25 |
| `gdelt_news_volume` | global_events | https | json | 900s | none | free | no | 30 |
| `hackernews_frontpage` | technology | https | json | 600s | none | free | no | 40 |
| `github_rust_activity` | technology | https | json | 3600s | token | free w/ registration | no | 50 |

### usgs_earthquakes

- **Provider:** U.S. Geological Survey
- **Endpoint:** USGS "all hour" earthquake feed
- **License:** public domain
- **Entities:** `earthquake`
- **Geospatial:** yes
- **Notes:** event-driven rather than polled; the feed only changes when the earth
  does. Observations carry magnitude and coordinates.

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

### hackernews_frontpage

- **Provider:** Y Combinator / Algolia
- **Endpoint:** Algolia HN Search API
- **License:** public API, no key required
- **Entities:** `software`
- **Parameters:** `tags=front_page`, `hits_per_page=50`
- **Notes:** a proxy for developer attention. Story scores make a usable time
  series for the technology lens.

### github_rust_activity

- **Provider:** GitHub
- **Endpoint:** GitHub repository search API
- **License:** public API; GitHub Acceptable Use terms apply
- **Entities:** `software`
- **Parameters:** `language=Rust`, `sort=updated`, `per_page=50`
- **Notes:** uses the search response envelope (`items`). Token authentication is
  optional and only raises the rate limit.

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
