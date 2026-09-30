# Correlation

## Why this is the distinguishing part

One measurement moving is weak evidence. It could be noise, a source artefact,
or a coincidence. Independent sources moving together is the closest thing to
confirmation that automatically collected data can provide, because independent
sources have different collection methods, different failure modes and no shared
bias.

The engine looks for that explicitly.

## The problem with naive matching

The brief's example:

```text
oil_price ↑
shipping_delay ↑
port_activity ↓
news_mentions ↑
```

Three move up and one moves down. A naive "same direction" rule would miss the
story entirely — and the one that moves *down* is often the most informative.

## The rule

Grouping is by **entity, place and time window first**, then direction is
reported rather than required:

```text
entity       (exact or related, per merge_mode)
geography    (per merge_mode)
time window
category
direction    (reported, not required)
```

`ConvergenceConfig`:

```text
window_seconds       6 hours
min_sources          2
min_series           2
merge_mode           Exact | Related
radius_km            250
min_shared_segments  2
```

A `ConvergenceGroup` requires at least two distinct sources and at least two
distinct series. Both conditions matter:

- **Two sources** rules out one feed echoing itself.
- **Two series** rules out one metric being counted twice.

### Grouping key and merge mode

`merge_mode` decides how strict the notion of "the same thing" is.

**`Exact` (the default).** Candidates are grouped by identical `entity_id` when
present, and by `series_key` otherwise — so an entity-less candidate never
converges with anything. This is the original rule, kept as the default so the
behaviour of an existing deployment does not change underneath it.

**`Related`.** Entity ids may be *related* rather than identical, and candidates
that share no entity may converge on geography.

### Related entity names

Exact matching was too strict to be useful on real data. The collectors emit
prefixed slugs, so the same place arrives from two providers under different
names:

```text
USGS    region_san_francisco
other   region_san_francisco_bay_area
```

These are the same place, and exact matching could not see it — which is exactly
the case convergence exists to catch. Under `Related`, two entity ids are related
when one's canonical segment set is a *subset* of the other's, sharing at least
`min_shared_segments` segments:

```text
{region, san, francisco}   ⊆ {region, san, francisco, bay, area}   related
{region, south, fiji}      ⊄ {region, south, tonga}                distinct
{region, fiji}             ⊄ {region, tonga}                       distinct
```

Subset rather than plain overlap: overlap would call `region_south_fiji` and
`region_south_tonga` the same place on the strength of the shared word "south".
Requiring two shared segments stops the bare prefix (`region`) from matching
everything. Identical ids are related too, since a set is a subset of itself.

Related entities are merged with union-find, so a chain of names forms one group
rather than a set of pairs.

### Geographic matching

Candidates carry coordinates. When entities are not enough — or absent — two
candidates within `radius_km` of each other can converge on location. Clustering
is by connected components, so a chain of nearby points forms one place rather
than an arbitrary split. Geography is only consulted under `Related`; the
default never groups by place.

### Sliding window

Within each group, a sliding window is applied over time. A window closes once
the next candidate is more than `window_seconds` past the window's first
candidate. This lets a long-running convergence be reported as several
consecutive groups rather than one unbounded blob.

### Strength

`strength` in `0..=1` expresses how strong the agreement is, from the number of
independent sources and series involved. It is one input to the signal engine's
`convergence` quality dimension; it is not itself a verdict.

### Deterministic ordering

Groups are sorted by strength, then first-seen, then `group_key`. The key
tiebreaker is load-bearing: equal-strength groups previously came out in hash
order, which varies per run, and that made a replayed backtest non-reproducible.
Bucket maps are `BTreeMap` for the same reason.

## What a group carries

```text
entity_id           the first entity in the group, when there is one
entity_ids[]        every distinct entity, more than one when matched as related
group_key           the grouping key
match_kinds[]       exact_entity | related_entity | geography | series
directions[]        observed directions, most common first
dominant_direction  the majority direction, if any
source_ids[]        the independent sources involved
series_keys[]       the independent metrics involved
candidate_ids[]     → the drill-down entry points
first_seen, last_seen
strength
```

`match_kinds` is reported rather than assumed, so a signal can say *how* its
sources agree — "three sources at one place" reads differently from "three
sources naming the same region".

## Where it sits in the pipeline

Convergence is computed *across* candidates, before signals are formed, because
it is a property of the candidate set rather than of any one candidate. The
signal engine owns the detection and its configuration, so the rule that matches
sources and the code that forms signals cannot drift apart:

```text
candidates → events
candidates → convergence groups   (inside SignalEngine::form_signals)
events + candidates + groups → signals
```

A signal that has a convergence group becomes `CONVERGENCE` in addition to
whatever else it is. Types combine; a signal can be
`ANOMALY + CONVERGENCE + IMPACT` at once.

## What is deliberately not used yet

Deterministic correlation is enough to prove the mechanism, and it is
explainable: the engine can say exactly which sources and series agreed, over
what window, and by which rule. Graph methods and learned correlations can be
layered on later, but only once the deterministic version is measured — a
learned correlation that cannot be explained is not an improvement on one that
can.
