# 0020 — GitHub: one series per repository

**Status:** accepted

## Context

`github_rust_activity` measures the star count of each repository in a fixed
16-repo universe, hourly. All repositories share the entity `ecosystem_rust`,
the metric `repo_stars` and the unit `stars`; they were distinguished only by
`identity` (the repo name).

The engine keys its series trackers and rolling windows on the **series key**,
not on `identity`. With no other discriminator, every repository collapsed into
one series and therefore one rolling baseline — a pool of unrelated projects'
star counts. The consequences were recorded in `docs/reality-audit.md` finding 6:

- a repository's **first** appearance was scored against the *other*
  repositories' star counts, producing a meaningless cold-start deviation
  (`+1527σ` from a median-0/MAD-0 history);
- a fast-growing project and a dormant one shared a baseline that described
  neither.

`docs/decisions/0015-cisa-derived-growth.md` already identified the underlying
limitation — "the engine keys its series trackers and rolling windows on
`series_key` alone; `identity` is carried as candidate metadata, never as a
key" — and deferred it as F9.

## Decision

**Give each repository its own series via the `repo` dimension.**

`parse_repo` now calls `.with_dimension("repo", full_name)`, the same mechanism
`nws_alerts`, `nasa_eonet`, `gdelt_news_volume` and `usgs_earthquakes` use for
their sub-series. The series key becomes
`github_rust_activity::ecosystem_rust::repo_stars::stars|repo=owner/name`, so
each repository has its own rolling window and baseline.

This is the semantic audit's stated constraint — "rolling statistics per
repository (`repo_stars` with `identity` = repo)" — achieved through the
existing dimension contract rather than a new one. `identity` remains the record
discriminator (stable id and de-duplication across polls); `dimension` is what
separates the series.

Because each series now needs its own `min_samples` points before it is judged,
the per-repo cold start is a guard rather than a false signal: the first
baseline is that repository's own history.

## Consequences

- A signal on `repo_stars` is now about **one repository**, named by the
  dimension and the identity, not about a pool.
- The universe stays fixed; membership is still the code constant.
- No detector change: the anomaly engine already keys on the series key, which
  now includes the repository.
- `identity` and `dimension` both carry the repo name; that is intentional —
  they answer different questions (which record vs. which series) and the
  engine reads each for its own purpose.

## Verification

`crates/sources/src/github.rs` unit tests: two different repositories get
distinct series keys containing their own name, and the same repository across
polls keeps one series. The F9 spec in
`crates/sources/tests/semantic_regression.rs` drives the shipped `parse_repo`
and fails against the pooled-series code (both repositories then share one key).
