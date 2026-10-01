# Signal Model

## From candidate to signal

A signal is never produced directly by a detector. The chain is:

```text
detector → AnomalyCandidate → event → signal
```

An `AnomalyCandidate` is a measurement that looks unusual. It is not a signal,
because nothing has yet decided it matters. The event engine groups candidates;
the signal engine decides what a person should see.

## Signal types

Signal types are **not** an importance ranking, and a single signal can carry
several at once. `ANOMALY + CONVERGENCE + IMPACT` on one signal is normal and
expected.

| Type | Meaning |
| --- | --- |
| `NOW` | A meaningful change happening right now. |
| `ANOMALY` | A clear departure from normal behaviour. |
| `EARLY_SIGNAL` | Small, but persistent, directional and accelerating. |
| `CONVERGENCE` | Independent sources pointing at the same change. |
| `IMPACT` | A change in a declared impact scope (a systemic category or entity). |

Types are conveyed in the UI by icon, label, shape, typography, timeline and
state. Colour alone never carries the type.

## Detection methods

Each candidate records the method that produced it, so a signal can always say
*how* it was found:

```text
Change            absolute/relative change from the previous point
ZScore            classical z-score against a rolling baseline
RobustZScore      median/MAD-based z-score
Velocity          rate-of-change anomaly
PersistenceDrift  sustained directional drift (early-signal precursor)
RegimeShift       direction change
```

## Early signals

An early signal is the case the rest of the system would miss. Consider:

```text
Day 1  +0.3σ
Day 2  +0.5σ
Day 3  +0.8σ
Day 4  +1.2σ
Day 5  +1.6σ
```

No single point is a large anomaly. But the sequence is persistent, directional
and accelerating, so it is raised as an `EARLY_SIGNAL` candidate. This is the
combination the early-signal detector looks for:

```text
small deviation + persistence + direction + acceleration
```

## Signal quality

Signal quality is **not** one "importance score". It is seven independent
dimensions, so the UI can say:

> "A 41% deviation from normal, sustained for 18 minutes, backed by 4 independent
> observations."

instead of:

> "This is important."

| Dimension | Meaning |
| --- | --- |
| `novelty` | How new this change is relative to recent history. |
| `strength` | Magnitude of the deviation. |
| `persistence` | How long it has been sustained. |
| `confidence` | Detector confidence. |
| `breadth` | How many distinct entities/categories it touches. |
| `convergence` | How many independent sources agree. |
| `relevance` | Relevance to the active lens set. |

## Evidence

Every signal carries the specific observations behind it:

```text
source_id
observation_id      → the drill-down entry point
metric, unit
statement           e.g. "story_score 723.00 points (+4.5σ vs baseline, ZScore)"
observed_at
value
deviation_sigma
```

An evidence entry is a direct link into `OBSERVATION → SOURCE → RAW DATA`.

## Explainability

Every signal carries `reasons[]`, written by the engine at the moment it decided
to emit the signal. For example:

```text
deviation +4.5σ from baseline (median 95.00, MAD 103.04) via ZScore
change observed within the current window
persistent directional drift for 2580 min, +3.5σ and still growing
```

These are generated from the measurement itself. The engine never says "an AI
found this interesting", because no AI is involved in deciding.

## Time

Every signal carries:

```text
first_seen
last_updated
duration_seconds
observation_count (via its evidence)
```

So the UI can render when normality ended:

```text
NORMAL
───────╮
       ╰──────● NOW
```

## Persistence and identity

A signal's identity is its `event_id` together with its `direction` (see
`docs/decisions/0012`). An event is one ongoing change, and its id is stable
across cycles, so a signal that keeps being confirmed is *updated* —
`last_updated` moves, evidence accumulates, duration grows — rather than being
re-emitted every cycle. A signal that stops being confirmed stops being updated,
and its state resolves.

`series_key` is *not* part of the identity. An event's candidates can come from
several series as sources converge, and the dominant one can change between
cycles; keying on it would fork one change into a new signal whenever that
happened. The series names what a reader is looking at, not what the signal is.

## Lifecycle

The event a signal is derived from moves through:

```text
DETECTED → ACTIVE → CHANGING → STABILIZED → RESOLVED
```

## Rendering

Signals are presented as observations with evidence, not as verdicts:

```text
⚡ NOW                  ◇ ANOMALY              ◎ EARLY SIGNAL
ENERGY / OIL            Satellite anomaly       New persistent drift
                                                
First seen   14:32      Deviation   +4.1σ       7 observations
Last update  14:37      Persistence 18 min      3 days
Duration     5m                                 accelerating

normal ────╮            [INVESTIGATE]           [ZOOM]
        ╰──● NOW
[NEDEN?] [ZOOM]
```
