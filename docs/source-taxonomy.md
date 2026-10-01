# Source Taxonomy

The catalog is the engine's sensor network. This document fixes the **domains**
that network is meant to cover, so that adding a source is a deliberate act
against a plan rather than a series of ad-hoc attachments.

It is the answer to one question: *which parts of the world is the machine
supposed to be watching, and how well are we watching each?*

## Principles

1. **A domain earns its place by having a real, lawful, machine-readable sensor.**
   "Politics" is a domain of the world; it has no free, stable, comparable API,
   so it is covered indirectly through `global_events` (news volume) rather than
   faked with a scraped feed.

2. **A source is a sensor, not a topic.** `github_repo_universe` is one sensor of
   the software ecosystem; `npm_downloads` and `pypi_downloads` are others. A
   domain is covered when it has *several independent* sensors, so convergence
   (brief §15) has something to work with.

3. **Coverage is measured, not claimed.** `wse sources` and the Lens screen show
   which domains actually have connected sources. A domain with no source is
   listed here as **uncovered** rather than silently absent.

4. **The same world, many lenses.** Categories below are the canonical strings
   sources emit. Lenses (`config/lenses/`) filter over them; a lens never changes
   the data. See [lenses.md](lenses.md).

## The domains

| domain | category | what it measures | sources today | coverage |
| --- | --- | --- | --- | --- |
| Seismology | `geophysics` | earthquakes, their magnitude and place | `usgs_earthquakes`, `afad_earthquakes` | **strong** (global + Turkey) |
| Volcanology & geohazards | `geophysics` | volcanic unrest, tsunamis | *(via `nasa_eonet`, `gdacs_disasters`)* | partial |
| Weather | `weather` | temperature, precipitation, active alerts | `nws_alerts`, `open_meteo_weather` | **strong** |
| Air & environment | `environment` | air quality, particulates | `open_meteo_air_quality` | partial |
| Earth observation | `earth` | wildfires, storms, floods, volcanoes as observed events | `nasa_eonet` | **strong** |
| Space weather | `space` | geomagnetic disturbance, solar flares | `noaa_kp_index`, `noaa_goes_xray` | **strong** (two independent sensors) |
| Near-Earth objects | `space` | close approaches | `nasa_neo` | **strong** |
| Disasters | `disasters` | official multi-hazard alerts with severity | `gdacs_disasters` | **strong** |
| Humanitarian | `humanitarian` | displacement, aid, population flows | `unhcr_displacement` | partial |
| Health | `health` | outbreaks and epidemic events | `who_outbreaks` | partial |
| Global events | `global_events` | news volume / attention | `gdelt_news_volume` | partial (one sensor) |
| Cyber | `cyber` | exploited vulnerabilities | `cisa_kev` | **strong** |
| Software ecosystem | `technology` | developer attention, releases, package adoption | `hackernews_frontpage`, `github_repo_universe`, `npm_downloads`, `pypi_downloads` | **strong** (four independent sensors) |
| Finance | `finance` | reference rates, credit | `ecb_exchange_rates` | partial |
| Markets | `markets` | crypto and commodity prices | `coingecko_market` | partial |
| Energy | `energy` | generation, grid load | *(none — see below)* | **uncovered** |
| Food & agriculture | `agriculture` | production, food prices | *(none — see below)* | **uncovered** |
| Science | `science` | publication and preprint output | `crossref_works`, `arxiv_submissions` | **strong** |
| Transport | `transport` | aviation, maritime, freight | *(none — see below)* | **uncovered** |

## The uncovered domains

Three domains are named here as targets with no lawful free sensor connected
yet. They are listed so the gap is visible, not hidden.

- **Energy.** Prices and grid load are the classic leading indicators. Free,
  keyless, real-time series are scarce: most market feeds are licensed, and
  grid operators publish per-country, not globally. `energy-charts` (Fraunhofer
  ISE) is a credible candidate for European generation and is under evaluation.
  The ENERGY lens ships unfed and says so.

- **Food & agriculture.** Yield and food-price series exist (FAOSTAT, World
  Bank commodity "pink sheet") but are annual or monthly and heavily licensed;
  a monthly series is too slow to baseline at the engine's cadence and would be
  a source that never changes. Left uncovered deliberately rather than connected
  as a source that cannot move.

- **Transport.** Aviation (`adsb.lol`, OpenSky) and maritime (AIS) feeds exist
  but are either unstable-population snapshots (a live position map is not a
  time series) or licence-restricted. A *count* of aircraft in a fixed airspace
  is a comparable series and is under evaluation; position dumps are not.

## Why coverage is measured per domain, not per source

The reality audit (finding 7) recorded that "13 sources across 9 categories"
still left several categories with a single source, so convergence had no
independent material. The fix is not more sources for their own sake: it is
**two or more independent sensors in the domains that matter most**. Space
weather now has two (Kp and GOES X-ray); software has four; the disaster stack
has three (EONET, GDACS, USGS). That is the shape this taxonomy optimizes for.
