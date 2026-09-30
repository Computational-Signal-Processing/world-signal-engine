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
GLOBAL       the empty lens; everything
AGRICULTURE
ENERGY
FINANCE
SOFTWARE
SCIENCE
SPACE
TURKEY       bbox over Turkey
PERSONAL     user-defined entities and keywords
```

## What lenses must not do

- A lens must not change what the engine detects. Detection runs on the full
  dataset; lenses filter the result.
- A lens must not duplicate data. There is one world dataset.
- A signal must never be *about* a lens. `lens_matches` on a signal records which
  lenses currently show it; it is not part of the signal's identity.

## Configuration

Lenses are configuration, not code. A new lens is a new entry under
`config/lenses/`; no Rust changes are required.
