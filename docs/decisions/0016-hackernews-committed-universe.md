# 0016 — Hacker News: a fixed universe that is actually fixed

**Status:** accepted

## Context

The Hacker News catalog entry declares `measurement: fixed_universe`, and the
module doc explained the intent: resolve the current top-N story ids *once*, then
re-measure the *same* ids on every later collection, so a story's score is a
comparable measurement of attention on a known story.

The collector did not do this. `HackerNewsCollector::collect` read the top-story
list and took the current head-`UNIVERSE_SIZE` ids on **every** poll. The helper
that would have made the universe stable, `hackernews::next_universe`, was
implemented and unit-tested but never called — dead code. So the population
churned every ten minutes, and the engine detected on a series the catalog
claimed was stable. The semantic audit called this a **catalog/implementation
contradiction** (risk #2, remediation F2).

## Decision

**Fix the collector, not the catalog.** Hacker News is a genuinely useful
SOFTWARE signal — a story's score rising is real attention — and the declared
intent is correct. Reclassifying it to `unstable_population` would have thrown
that away to hide a bug.

The universe is now committed and carried across collections:

- `HackerNewsCollector` holds `universe: Arc<Mutex<Vec<i64>>>`, shared across
  clones. The serve loop builds the collectors once and reuses them each cycle,
  so this state persists between polls.
- **The first resolution commits.** An empty universe commits the first
  `UNIVERSE_SIZE` candidate ids. From then on the universe is that set.
- **A story leaving the front page is kept.** Evicting it would make the series
  move with membership rather than with attention — the exact bug being fixed.
- **Only a gone story frees a slot.** A tracked id is dropped only when it fails
  with **404**; its slot is refilled from the current top list. Any other fetch
  failure (timeout, 5xx) holds the slot, so a transient blip is never read as
  "the story disappeared". This is the same absence-vs-zero distinction the rest
  of the engine holds to.
- The universe never grows past the size it committed to; new candidates cannot
  enlarge it.

## Consequences

- `next_universe(current, resolved, candidates, size)` now has committed
  semantics rather than "top up to `size`". Its signature gained `resolved` so
  the gone-vs-transient distinction is explicit at the call site.
- Detection now runs on a stable population per story. The summed score across
  the universe still mixes stories of different ages and remains non-meaningful;
  the per-story series is the one to baseline.
- The catalog keeps `fixed_universe`, and for the first time it is true.

## Limits

The universe is in-memory. A process restart resolves a fresh universe, which is
correct — there is no durable commitment yet, and a restart is a cold start, not
a world change. Persisting the universe across restarts (so a long-running
deployment keeps the exact same ids through a redeploy) is deliberately out of
scope.

## Verification

`crates/sources/tests/semantic_regression.rs` — the F2 spec, previously
`#[ignore]`d, is now active: a disjoint top list on the second poll must not
change the measured ids. `crates/sources/src/collectors.rs` adds an end-to-end
test through a churning transport: poll 1 commits `1..=3`, a disjoint poll 2
still measures `1..=3`, and after story `2` returns 404 the universe becomes
`1, 3, 4`. Unit tests cover the first-commit, no-growth, departure-keeps, and
gone-frees-a-slot rules. The spec fails again if the collector samples the
current top-N, which is how the fix was shown to be load-bearing.
