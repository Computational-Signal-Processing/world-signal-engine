# 0022 — Impact is a declared scope, not a score (IMPACT)

**Status:** accepted

## Context

`IMPACT` is one of the five signal types (brief §3): *a change with meaningful
potential effect for a lens or entity*. The mechanism existed —
`SignalEngine::has_impact` compares a signal's categories and entities against
`SignalConfig::impact_categories` / `impact_entities` — and was unit-tested in
isolation.

But both config fields defaulted to empty, and no shipped configuration ever
declared a scope. `docs/audit.md` and `docs/reality-audit.md` recorded this as
the top open limitation: the type was *reachable in a test*, never *producible
by a running engine*. A type that no served engine can emit is not implemented,
however well it is tested.

The design question is how the engine should decide a change is "impactful"
without violating the brief's core rule: the engine must never say "AI decided
this is important" (§2, §43). An opaque importance score would do exactly that.

## Decision

**Impact is a declared scope, loaded from configuration, and the signal's reason
names the term that matched.**

- A scope is a YAML file under `config/impact/` (`wse-config::load_impact`),
  merged like lenses: missing directory is an empty scope, a malformed file is
  reported and skipped, only an unreadable directory is an error. The failure
  policy is identical to `load_lenses` and for the same reason — a scope is a
  declaration, not load-bearing for collection or detection (§36).
- The shipped scope (`config/impact/systemic.yaml`) names the `finance` and
  `cyber` categories and the `Hormuz` entity. These are systemic domains whose
  changes transmit; the file explains each term.
- `has_impact` returns the matched term, and the signal's reason reads
  `touches configured impact scope: category finance` or `… entity Hormuz`. The
  reader can check the claim; the engine never asserts importance.
- Impact is **additive**. A finance spike is `ANOMALY` *and* `IMPACT`, not one
  instead of the other. `assign_types` sets each type from its own condition and
  the types accumulate (§3: a signal can carry several types at once).
- Adding a term is a config edit, not a code change — the extensibility rule
  (§37) that already holds for lenses and sources now holds for impact.

`crates/cli/tests/impact_scope.rs` proves it end to end against the real shipped
config and catalog: a finance spike produces an `IMPACT` signal whose reason
names the term; an in-scope entity is `IMPACT` even when its category is not
listed (the entity path); and an out-of-scope `earth` spike is detected but is
*not* `IMPACT` (the negative control).

## Consequences

- The fifth signal type now has a producer in `demo` and `serve`. The IMPACT
  limitation is removed from the audit docs.
- The scope is deliberately small. `Hormuz` has no connected source today, so
  the entity path is exercised by a probe rather than by production data; it is
  forward-looking, and connecting an energy or shipping source needs no
  impact-scope work. Widening the scope is a one-line config change, and the
  guard is the same as for lenses: a scope that lists everything is the same as
  having no scope.
- The alternative — a computed importance score — was rejected. It would have
  made the engine's judgement opaque and violated §43; a declared scope keeps
  the judgement with the operator and the reason checkable.
