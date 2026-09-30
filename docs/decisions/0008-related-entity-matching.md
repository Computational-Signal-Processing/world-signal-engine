# 0008 — Related entity and geographic matching, behind a merge mode

**Status:** accepted

## Context

The convergence engine grouped candidates by identical entity id, falling back
to the series key. On synthetic data that looked fine: the synthetic world gives
every stream a distinct `ent_<series>` id, so nothing was ever expected to match.

On real data it is too strict to work. The collectors emit prefixed slugs:

```text
USGS          region_san_francisco
another feed  region_san_francisco_bay_area
```

Two providers describing the same place under different names is the normal
case, and it is precisely the case convergence exists to catch. Exact matching
sees two unrelated entities and reports nothing.

There was a second gap: the module documented geography as a grouping dimension
and `AnomalyCandidate` carries `latitude`/`longitude`, but no code ever read
them. `docs/correlation.md` claimed a rule the implementation did not have.

## Decision

Add `ConvergenceConfig::merge_mode`, defaulting to `MergeMode::Exact` — the
original behaviour, so nothing changes underneath an existing deployment.

Under `MergeMode::Related`:

- Two entity ids are related when one's canonical segment set is a **subset** of
  the other's, sharing at least `min_shared_segments` (default 2) segments.
  Subset, not overlap: overlap would call `region_south_fiji` and
  `region_south_tonga` the same place because they share "south". Requiring two
  shared segments stops the bare prefix from matching everything.
- Candidates within `radius_km` (default 250) of each other may converge on
  geography, when entities are not enough or absent.

Related entities are merged with union-find, so a chain of names
(`region_hormuz` ⊂ `region_strait_hormuz` ⊂ `region_strait_of_hormuz`) forms one
group rather than a set of pairs. Geographic clustering uses the same structure
over connected components.

`ConvergenceGroup` gained `entity_ids` and `match_kinds` so a signal can report
*how* its sources agree, not merely that they do.

## Consequences

- The brief's convergence case now works on real collector output, which the
  exact rule could not reach.
- The stricter mode remains the default. Widening matching is a deliberate act
  (`ConvergenceConfig::related()`), not something a deployment inherits.
- Matching is still deterministic and explainable: no learned similarity, no
  embeddings. A group can always name the rule that produced it.
- `min_shared_segments` is a knob with no single right value. Two is a defensible
  default for prefixed slugs; a catalog with longer, more descriptive names may
  want three.
- `detect_convergence` now sorts by `group_key` as a final tiebreaker, and uses
  `BTreeMap` rather than `HashMap`. Equal-strength groups previously came out in
  hash order, which varies per run — invisible until replay compared two runs.
