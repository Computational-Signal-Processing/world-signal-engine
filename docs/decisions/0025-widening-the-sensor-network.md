# 0025 — Widening the sensor network across domains

- **Status:** accepted
- **Date:** 2026-10-01
- **Related:** [0024](0024-github-universe-spans-ecosystems.md),
  [0020](0020-github-per-repo-series.md), `docs/source-taxonomy.md`

## Context

The catalog had 13 sources across 9 categories, but several categories had a
single source. A single sensor cannot converge: convergence (brief §15) is the
claim that *independent* observations point at the same change, and one sensor
can never make that claim. The reality audit recorded this as a coverage gap.

Widening the network is not "connect everything". It is: give the domains that
matter two or more *independent* sensors, from lawful, keyless, machine-readable
feeds, without lowering quality.

## Decision

Eight sources were added, chosen to close the largest coverage gaps with the
best available free feeds:

| source | domain | why this one |
| --- | --- | --- |
| `open_meteo_weather` | weather | global model grid, keyless; a physical sensor independent of the NWS alert feed |
| `open_meteo_air_quality` | environment | particulates from CAMS; the first real environment sensor |
| `noaa_goes_xray` | space | solar X-ray flux — the second, *independent* space-weather sensor (Kp measures disturbance at Earth; X-ray measures the solar driver) |
| `gdacs_disasters` | disasters | official UN/EC multi-hazard alerts with severity; independent of EONET (which observes events rather than assessing impact) |
| `who_outbreaks` | health | the authoritative outbreak feed; a new domain |
| `coingecko_market` | markets | the one 24/7, globally priced market with a free keyless API |
| `npm_downloads` | software | download volume — usage, independent of stars (attention) and HN (discussion) |
| `pypi_downloads` | software | the Python counterpart to npm |

## Consequences

- Space weather now has **two** independent sensors; software has **four**
  (HN, GitHub, npm, PyPI); the disaster stack has **three** (USGS, EONET,
  GDACS). Convergence has real material in these domains.
- Three new lenses: `lens_health`, `lens_humanitarian`, and a newly fed
  `lens_agriculture` (via Open-Meteo weather/air, category `weather`). Only
  `lens_energy` remains intentionally unfed.
- Fixed-universe sources (npm, PyPI) follow the discipline of 0020/0024: the
  package set is fixed in code so each package is its own stable series.
- **Quality bar held.** Every added source is keyless (or optional-key), free,
  and machine-readable. Sources that would have required scraping, a paid key,
  or an unstable population (a live flight/marine position map) were left out —
  see `docs/source-taxonomy.md` for the domains deliberately left uncovered.

## Alternatives considered

- **Add sources by category count rather than by independence.** Rejected: two
  feeds that resell the same upstream are one sensor, not two.
- **Scrape energy/food/transport feeds.** Rejected for now: no stable, lawful,
  keyless series; a source that cannot move at the engine's cadence is worse
  than a named gap.
