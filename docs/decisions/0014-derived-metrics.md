# 0014 — Derived metrics: a cumulative level is not a world change

**Status:** accepted

## Context

Some sources report a quantity that is not itself the thing whose change
matters. arXiv reports `preprint_total` — the cumulative number of preprints in
a category. It only ever grows. Running a level z-score on it is close to
meaningless: the series drifts upward and almost never deviates, while the
quantity a reader actually cares about — *how many new preprints appeared* — is
its increment, which the code never computed. The module doc comment claimed a
"submission velocity" that did not exist in the stored data.

The reconnaissance (commit `ff692c2`) established that the value entering
detection is exactly `Observation.value` as the collector set it, and that the
collector's `parse` is pure and stateless, so it cannot see the previous
observation. The transformation therefore cannot live in the collector.

## Decision

Introduce a **declared, per-source derivation**, evaluated by the engine at
ingest, producing a normal observation on the derived metric's own series.

- The declaration is data on the `Source` (`derivations: Vec<Derivation>`), the
  same pattern as `measurement` and `feeds_lenses`. The engine reads it
  generically; nothing branches on a source id or a metric name.
- The only kind implemented is `Delta`: `to = from(t) - from(previous)`.
- The pure arithmetic lives in `wse_baseline::derive::evaluate`, isolated from
  the engine orchestration. The engine supplies the predecessor.
- The derived observation is built by `Observation::derived_from`. It is an
  ordinary observation on its own series, so storage, baseline, detection, the
  API and drill-down all work unchanged.
- arXiv declares `preprint_new = Delta(preprint_total)`. The raw `preprint_total`
  is stored and queryable but becomes **evidence-only**: it is excluded from
  detection, so the engine detects on the velocity, not the level.

## Consequences

- **Missing predecessor is not zero.** A first observation produces no derived
  value. Emitting `0` would claim "nothing happened" when the truth is "nothing
  to compare against". A genuine `0` (unchanged total) is emitted and is
  distinguishable from the absence because it is present at all.
- **A counter reset is not a large negative change.** When `current < previous`
  the sequence restarted; no value is emitted and the next observation becomes
  the new predecessor. This is what stops a reset from manufacturing a huge
  negative anomaly.
- **The interval is explicit.** `observed_at` is the interval end; the interval
  itself (`interval_start`, `interval_end`) is carried on the derived
  observation's provenance, never re-inferred from polling time.
- **Provenance is on the observation.** `DerivationProvenance { kind, inputs,
  interval_start, interval_end, formula }` links the derived point to both raw
  inputs, so the drill-down can show `100 → 107 → +7` and reach the raw data.
  No parallel provenance store was created.
- **Identity is deterministic.** The derived id is keyed on its own series, the
  interval end and the current observation's id, so reprocessing the same inputs
  yields the same id and re-ingest de-duplicates. Nothing uses the wall clock.
- **The raw level is suppressed generically.** The set of `(source, metric)`
  pairs named as a derivation input is computed once per ingest; those series
  are stored but not fed to a tracker. A raw series named by a derivation can
  therefore never be detected on, without any metric-specific branch.

## Limits

Only `Delta` exists. `WindowCount` (overlapping-window counts, e.g. KEV,
Crossref) and gap-aware handling (ECB business days) are deliberately out of
scope; they need the overlap/gap semantics discussed in the reconnaissance
(`docs/source-semantic-audit.md`) and will be separate decisions.
