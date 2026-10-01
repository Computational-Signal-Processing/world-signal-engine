# Data Model

The model separates the pipeline stages so they cannot be confused with one
another:

```text
Observation → Change → AnomalyCandidate → Event → Signal
```

## The distinctions that matter

These are deliberately different types, not different values of one type.

| Term | Meaning |
| --- | --- |
| **Observation** | A measurement from a source. |
| **Change** | An observation differing from its previous state. |
| **Anomaly** | A statistically significant deviation from normal behaviour. |
| **Event** | One or more observations representing an occurrence. |
| **Signal** | An event or change made worth a human's investigation. |
| **Context** | Related data that makes a signal intelligible. |
| **Impact** | Potential effect through a particular domain or lens. |

A value can change without being anomalous, and be anomalous without having
changed. Collapsing these into one "importance" number is the mistake this model
exists to avoid.

## Observation

The atomic unit. Everything downstream is derived from these.

```text
observation_id      deterministic: series key + observed_at + raw hash
source_id
observed_at         when the world produced the value, per the source
received_at         when we received it
entity_id           optional; what the measurement is about
metric
value
unit
latitude, longitude  optional
quality             score + flags
raw                 RawReference
dimensions          extra grouping, e.g. {"station": "Kadikoy"}
attributes          non-numeric source fields, e.g. {"author": "..."}
```

### Identity and de-duplication

An observation's id is derived from its series key, its timestamp and the
**record's stable key**:

```text
source::entity::metric::unit | observed_at | record_key
```

The record key is the strongest stable identity the source offers for the
record — an upstream event id, an accession id, a permalink, or a time-series
point's own timestamp. It must not include the measured value: a record keeps
its identity while its measurement changes. This makes ingestion idempotent
without re-minting ids on every poll: a multi-record feed or a sliding window
whose *other* records changed still yields the same id for a record that did
not, so the second copy is dropped as a duplicate. A source that returns
unchanged data therefore produces no new observations.

A source with exactly one record per series per timestamp may omit the record
key; its payload hash is then used as the key, so re-fetching an identical
payload de-duplicates and a changed payload is a new observation.

`Observation::with_record_key` sets the record key. `Observation::with_identity`
sets a per-record *discriminator* for sources that emit several records per
series per timestamp (GitHub search, Hacker News); it is folded into the id and
kept on the observation for drill-down, but never enters the series key.

### The series key

```text
source_id::entity_id::metric::unit
```

Everything about baselines, change detection and signal persistence is keyed on
this. Two sources measuring the same thing are two series, which is what makes
convergence between them meaningful.

### Time

`observed_at` and `received_at` are both kept. When they diverge the source is
lagging, and that is visible rather than silently flattened into "now". Clock
drift and delayed data are therefore separable from real change.

## RawReference

```text
locator         URL, path or object key
hash            fingerprint of the payload
content_type
bytes
```

Every observation points back at the untouched source payload. The payload itself
is retained in the `RawStore` under `hash`, which is what makes the final step of
the drill-down — `SOURCE → RAW DATA` — real rather than a promise to re-fetch a
URL that may have changed since.

## Entity

Entities are the nouns the world is described with, canonicalized so that the
same thing from different sources is the same entity:

```text
entity_id
kind
label
```

Canonicalization is a plain function (`wse_model::canonicalize`), not a model.
It is deterministic and testable.

## Source

The catalog entry, not the collector. A source knows what it is; a collector knows
how to read it.

```text
id, name, provider
category, subcategory
endpoint, protocol, format, cadence, timezone
license, authentication, cost
historical_available, realtime_available, geospatial
entities            entity labels this source is expected to produce
priority            lower is more important
enabled
collector_type      which collector implementation reads this
parameters          free-form extras
tier                provenance: institutional, independent, community, exploratory
measurement         whether the emitted quantity is a comparable time series
feeds_lenses        lens ids this source is intended to feed
```

`tier` is provenance, not importance. `measurement` is the honesty field: a
source whose population churns between collections is marked
`unstable_population` and the engine stores its observations for evidence but
never detects on them. `feeds_lenses` declares coverage, so a lens with no
connected source can be reported rather than silently shown empty.

## SourceHealth

Kept separately from observations, because source failure is not world activity.

```text
source_id
last_success, last_failure, last_latency_ms
records_received, records_changed, records_duplicate
error_count, consecutive_failures
status
```

A source whose collector fails is `degraded` or `failing` with a recorded error.
It is never `zero`. The distinction between `NO DATA` and `DATA = ZERO` is
enforced here, in the model, and again in the UI.

## AnomalyCandidate

Produced by the detectors. Deliberately *not* a signal: a candidate is a
measurement that looks unusual, before anything has decided it matters.

```text
id, source_id, entity_id, series_key
metric, unit, observation_id, observed_at
baseline            BaselineSnapshot
current
deviation           signed, in baseline units
score               robust z-score where computable
method              ZScore, RobustZScore, ...
kind                Anomaly, EarlySignal, ...
direction           Up, Down, Flat
duration_seconds    how long the deviation has persisted
confidence          0.0..=1.0
latitude, longitude
```

## BaselineSnapshot

```text
count
mean, median
std_dev, mad
ewma
min, max
p25, p75
first_seen, last_seen
```

Stored alongside each observation so a signal can always show the baseline it was
compared against, even after the series has moved on.

## Event

Candidates grouped by entity, time window and direction.

```text
event_id
title
first_seen, last_seen
entities
observations        the observation ids behind it
anomalies           the candidate ids behind it
categories
location
direction
state
source_count        distinct sources contributing
```

### Lifecycle

```text
DETECTED → ACTIVE → CHANGING → STABILIZED → RESOLVED
```

## Signal

An event made worth looking at.

```text
signal_id, event_id
types[]             NOW, ANOMALY, EARLY_SIGNAL, CONVERGENCE, IMPACT
title, summary
first_seen, last_updated, duration_seconds
confidence
evidence[]          the specific observations, with statements
entities, categories, location
lens_matches[]
quality             the quality dimensions, not one score
direction
reasons[]           why the engine emitted this
series_key          identity, so a signal persists across cycles
```

`series_key` plus `direction` is the signal's identity. That is what makes a
signal *persistent* — updated as new evidence arrives — rather than a fresh
signal every collection cycle.

## Quality

Data quality is attached to observations, and is separate from signal quality.

```text
score
flags[]             e.g. late, clock_drift, missing_field
```

## Lens

A lens changes visibility only. The underlying dataset is shared; a lens never
moves or copies data.

```text
lens_id
name
categories, entities, regions, keywords
```

See [docs/lenses.md](docs/lenses.md).
