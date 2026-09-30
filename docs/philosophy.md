# Philosophy

## The one sentence

> The machine measures the world continuously; a human investigates only what
> changed.

This is not "an AI that understands the world". It is an instrument. An
instrument's job is to measure reliably, and to point at what moved.

## What this project is not

It is not a news app, a chatbot, a dashboard, or a summarizer. Those all start
from the assumption that a human is already watching, and that the problem is
presentation. The problem here is the opposite: the world produces far more
measurements than anyone can watch, and almost none of them matter. The job is to
find the few that do, and to be able to prove why.

## No LLM in the core

The detection path is mathematics: rolling statistics, deviations, persistence,
correlation. It is deterministic, cheap, explainable and testable. A large
language model is none of those things, and using one to decide that something
matters would make the system's output unfalsifiable.

An LLM may later sit *on top* of this engine: explaining a signal, synthesizing
context, comparing sources, answering "why did this signal appear?". It reads the
evidence package the engine already produced. It never decides that a signal
exists. The engine must run correctly with no LLM present in the process.

## The distinctions the design protects

Three pairs of things that look similar and are not.

### Observation vs. Change vs. Anomaly

An observation is a measurement. A change is an observation differing from its
previous state. An anomaly is a statistically significant departure from normal
behaviour.

A value can change without being anomalous — a small step in a noisy series. A
value can be anomalous without having changed — a series that has been in an
unusual regime all along. Comparing absolute values alone (`105` vs `100`) is not
detection.

### Event vs. Signal

An event is what the data says happened. A signal is what a person should look
at. The gap between them is a decision, and it is made by an explicit component
with explicit rules, not by a threshold buried in the anomaly detector.

This is why detectors produce `AnomalyCandidate`s and never signals. A candidate
is a measurement that looks unusual. Nothing has decided it matters yet.

### No data vs. zero

A failed collector means we do not know what happened. It does not mean nothing
happened.

This is the most dangerous confusion in the whole system, because the failure
mode is silent: a broken collector that reports zero looks exactly like a quiet
world, and the system would report calm while blind. The engine therefore keeps
source health entirely separate from observations, and renders `NO DATA`
differently from `DATA = ZERO`.

## Explainability as a construction property

The system must never say "AI found this important". It must be able to say:

> This measurement is 4.5 standard deviations above its baseline of 95.00
> (median 95.00, MAD 103.04), measured by robust z-score, and has been above it
> for 18 minutes, supported by 4 independent observations from 2 sources.

That sentence is assembled from the data the signal already carries — baseline,
deviation, method, duration, evidence. It is not generated after the fact. If the
engine cannot produce that sentence, it does not have a signal; it has a number.

## Not one importance score

Collapsing quality into a single "importance" figure destroys exactly the
information a person needs to judge it. The engine keeps seven dimensions
(`novelty`, `strength`, `persistence`, `confidence`, `breadth`, `convergence`,
`relevance`) and shows them separately, so the reader can disagree with the
system's emphasis.

## Early signals matter more than loud ones

The most valuable thing this system can do is notice something *before* it is
obvious. A slow, persistent, directional drift is invisible point-by-point and
obvious in hindsight:

```text
+0.3σ, +0.5σ, +0.8σ, +1.2σ, +1.6σ
```

No single observation is a large anomaly. The sequence is a signal. A detector
that only fires on large single deviations misses precisely the changes that are
worth knowing about early.

## Convergence is the strongest evidence

One measurement moving is weak. Independent sources, with different collection
methods, different failure modes and no shared bias, moving together is strong.
The engine looks for that explicitly, because it is the closest thing to
independent confirmation that automatically collected data can provide.

## Lenses change visibility, not truth

Different people care about different things. That is a viewing concern, not a
storage concern. The underlying dataset is shared and complete; a lens filters
what is shown. A signal is never *about* a lens, and adding a lens never changes
what the engine detects.

## Absence of an event is not evidence of calm

If the engine has not seen a signal, the honest reading is "the engine has not
seen a signal" — not "the world is calm". Coverage is uneven, sources fail, and
some changes are not observable with the sources currently connected. The UI
should never imply more certainty than the pipeline provides.

## Why a vertical slice at a time

A system like this is easy to make impressive-looking and hard to make correct.
Connecting a thousand sources proves nothing if none of them is measured
properly. So the order is fixed: one source through the entire pipeline, proven
end to end, then the next. Success is a working chain from observation to
explainable signal, not a count of integrations.
