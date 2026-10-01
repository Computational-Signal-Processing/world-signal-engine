# 0015 — CISA catalog growth: extending the derived-metric pattern, and why GitHub is not

**Status:** accepted

## Context

[0014](0014-derived-metrics.md) introduced a declared, per-source derivation and
applied it to arXiv: `preprint_new = Delta(preprint_total)`. The
`docs/source-semantic-audit.md` remediation list carried two sibling items that
looked like the same shape:

- **F5 / risk #5 — CISA.** `kev_catalog_total` is a cumulative level that only
  grows; a level z-score on it fires on the fact that the catalogue exists, not
  on anything that changed. (`kev_added`, the trailing 7-day sum, is a separate
  series and is untouched here.)
- **F9 / risk #9 — GitHub.** `repo_stars` starts from an empty baseline and
  records ~1527σ on cold start; a level z-score fires on "we just started
  looking", not on world change.

Both were assumed to be "difference the total". Reconnaissance (this change)
found they are not the same problem.

## Decision

**CISA: declare the derivation.** `kev_catalog_total` is a single record per
series per poll, and it is monotonic. A `Delta` is exactly correct:
`kev_catalog_growth = kev_catalog_total(t) - kev_catalog_total(t-1)`. The raw
total becomes **evidence-only** — stored and drill-downable, never
detection-tracked. This reuses the 0014 machinery with no new code: a constant
pair and one `Derivation::delta(...)` declaration.

**GitHub: do not.** The derivation pattern does not fit `repo_stars`, and
applying it would be a bug:

- `github_rust_activity` emits **one observation per repository** (16 of them),
  all sharing a single `series_key` (same source, entity `ecosystem_rust`,
  metric `repo_stars`, unit `stars`) and distinguished only by `identity` (the
  repo name).
- The engine keys its series trackers and rolling windows on `series_key`
  alone; `identity` is carried as candidate metadata, never as a key. So a
  `Delta` predecessor lookup — which matches on `series_key` and
  `observed_at` — would, within one poll cycle, subtract an arbitrary *other
  repository's* previous count. That is not a growth measurement; it is noise.
- GitHub's actual defect is the **cold-start level deviation**, which is a
  baseline/readiness concern, not a "wrong series" concern. It stays open as F9.

Making GitHub derivable would require identity-aware predecessor lookup
(tracker and window keyed by `identity`), which is a detection-core change well
beyond this scope. It is deliberately deferred.

## Consequences

- The CISA detection series is now `kev_catalog_growth`. "Nothing added since
  the last poll" is a genuine `0`; the first poll emits nothing (no predecessor).
  A catalogue shrink (never expected) is a `Reset`, not a negative anomaly.
- `kev_added` and `kev_catalog_total` remain stored; only the total is
  suppressed from detection.
- The 0014 limit is unchanged and now explicit: `WindowCount` (F5's *other*
  half — the overlapping 7-day window) and gap-aware handling (F8) remain out of
  scope. This change does not touch the trailing-window semantics of
  `kev_added`.
- GitHub/F9 remains an open item in the semantic audit; no GitHub source code
  changed here.

## Verification

`crates/cli/tests/cisa_derived_metric.rs` drives the **real shipped catalogue**
through the real engine: the raw total is stored but has no baseline, the
derived `kev_catalog_growth` series does, its value is the interval difference,
and its provenance links both raw inputs. A second test asserts a single poll
emits no growth. The evidence-only gate is load-bearing: disabling it makes the
first test fail (`the raw catalog size must not be detection-tracked`).
