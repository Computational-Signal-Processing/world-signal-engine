# Lenses

## What a lens is

A lens is a saved view over the shared world dataset. Different people care about
different things; that is a viewing concern, not a storage concern.

The underlying dataset stays complete and common. A lens changes what is
*visible*, never what is stored or detected.

## Combining lenses

Multiple lenses can be active at once:

```text
WORLD + TECHNOLOGY + ENERGY + TURKEY
```

`WORLD` is the empty lens — it matches everything — so combining it with others
widens rather than narrows. A user building a view adds the domains they care
about and a region if they want one.

## What a lens carries

```text
lens_id
name
categories   empty means "all categories"
entities     canonical entity names; empty means "all entities"
keywords     matched against signal titles and summaries
bbox         (min_lat, min_lon, max_lat, max_lon)
weights      relative weights over the quality dimensions, for ranking
```

## Matching

`Lens::matches` takes a signal's facets — its categories, entities, text and
location — and returns whether the lens shows it. Each filter is independently
optional, and an empty filter imposes no constraint:

| Filter | Rule |
| --- | --- |
| `categories` | Case-insensitive match against any of the signal's categories. |
| `entities` | Canonicalized comparison, so `Oil`, `oil` and `crude oil` agree. |
| `keywords` | Case-insensitive substring match against the title/summary. |
| `bbox` | Latitude/longitude inside the box. |

### Location is a soft constraint

A bounding box only excludes a signal that *has* a location outside it. A signal
with no location is never excluded by a box, because the engine does not know
where it happened — and hiding it would be silently dropping data rather than
filtering it.

## Weights

`weights` maps quality dimensions (`novelty`, `strength`, `persistence`,
`confidence`, `breadth`, `convergence`, `relevance`) to relative weights. They
affect *ranking* within a lens; they never affect detection, and they never
collapse the dimensions into a single score.

An energy lens might weight `strength` and `persistence` heavily; a personal
lens might weight `novelty` and `relevance`.

## Suggested lens set

```text
WORLD         the empty lens; everything
EARTH         geophysics, weather, natural events
SPACE         near-Earth objects, geomagnetic activity
GLOBAL EVENTS news volume
CYBER         exploited vulnerabilities
FINANCE       reference rates
SCIENCE       publications and preprints
AI            AI/ML research velocity
SOFTWARE      developer attention and releases
AGRICULTURE   (no connected source yet)
ENERGY        (no connected source yet)
TURKEY        bbox over Turkey
PERSONAL      user-defined entities and keywords
```

A lens whose category no connected source emits shows nothing today. That is
visible rather than silent: `wse lenses` prints the match count next to each
lens, so a dead lens reads as `0` rather than looking healthy.

## What lenses must not do

- A lens must not change what the engine detects. Detection runs on the full
  dataset; lenses filter the result.
- A lens must not duplicate data. There is one world dataset.
- A signal must never be *about* a lens. `lens_matches` on a signal records which
  lenses currently show it; it is not part of the signal's identity.

## Configuration

Lenses are configuration, not code. A new lens is a new entry under
`config/lenses/`; no Rust changes are required.

```yaml
id: lens_energy
name: ENERGY
categories: [energy, markets]
keywords: [oil, gas, shipping, port]
weights:
  strength: 2.0
  persistence: 1.5
```

The loader (`wse-config`) is deliberately tolerant, because a lens is only a
view and must never be able to take down collection or detection:

- A missing directory is not an error. No lenses configured is a valid state.
- A malformed file is skipped and reported in `LensCatalog::problems`; the other
  lenses still load. One bad lens does not lose the other ten.
- Duplicate ids are reported, and the first file (by sorted path) wins.
- The catalog is sorted by lens id, so `lens_matches` is written in a stable
  order. A replayed run has to produce the same signal bytes as the original.

## How it is wired

Matching runs once, at signal formation, in `SignalEngine::assign_lenses`. The
result is recorded on the signal:

```text
SignalEngine::form_signals
        │
        ├── assign_lenses ──▶ signal.lens_matches = [lens_global, lens_turkey, ...]
        └── merge_signals ───▶ lens_matches are unioned, never replaced
```

`Signal::lens_matches` is therefore a *record* of which lenses showed the signal
when it was formed, not something recomputed per request. `GET /signals?lens=`
reads it directly.

Unions, not replacement: a signal that accumulated categories over its life can
only have gained lenses, and dropping one on merge would make a `?lens=` query
lose a signal it had already returned.

## Seeing which lenses exist

```text
GET /lenses           every lens, with its current match count
GET /lenses/:id       one lens, plus the signals it shows
wse lenses            the same, as a table, against the synthetic world
```

The match count matters: a lens whose categories no collector emits yet (ENERGY,
FINANCE) is present and honestly reports zero. An empty view should be
distinguishable from a broken one, not silently identical to it.
