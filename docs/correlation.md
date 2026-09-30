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

## The MVP rule

Grouping is by **entity and time window first**, then direction is reported
rather than required:

```text
entity
geography
time window
category
direction   (reported, not required)
```

`ConvergenceConfig`:

```text
window_seconds   6 hours
min_sources      2
min_series       2
```

A `ConvergenceGroup` requires at least two distinct sources and at least two
distinct series. Both conditions matter:

- **Two sources** rules out one feed echoing itself.
- **Two series** rules out one metric being counted twice.

### Grouping key

Candidates are grouped by `entity_id` when present. When there is no entity, they
are grouped by `series_key` instead — which means an entity-less candidate can
never converge with anything. That is intentional: without a shared entity there
is no evidence the measurements are about the same thing, and grouping by
numeric similarity would declare unrelated metrics "convergent" for no reason.

### Sliding window

Within each group, a sliding window is applied over time. A window closes once
the next candidate is more than `window_seconds` past the window's first
candidate. This lets a long-running convergence be reported as several
consecutive groups rather than one unbounded blob.

### Strength

`strength` in `0..=1` expresses how strong the agreement is, from the number of
independent sources and series involved. It is one input to the signal engine's
`convergence` quality dimension; it is not itself a verdict.

## What a group carries

```text
entity_id           when the candidates share one
group_key           the series key when they do not
directions[]        observed directions, most common first
dominant_direction  the majority direction, if any
source_ids[]        the independent sources involved
series_keys[]       the independent metrics involved
candidate_ids[]     → the drill-down entry points
first_seen, last_seen
strength
```

## Where it sits in the pipeline

Convergence is computed *across* candidates, before signals are formed, because
it is a property of the candidate set rather than of any one candidate:

```text
candidates → events
candidates → convergence groups
events + candidates + groups → signals
```

A signal that has a convergence group becomes `CONVERGENCE` in addition to
whatever else it is. Types combine; a signal can be
`ANOMALY + CONVERGENCE + IMPACT` at once.

## What is deliberately not used yet

Deterministic correlation is enough to prove the mechanism, and it is
explainable: the engine can say exactly which sources and series agreed, and
over what window. Graph methods and learned correlations can be layered on
later, but only once the deterministic version is measured — a learned
correlation that cannot be explained is not an improvement on one that can.
