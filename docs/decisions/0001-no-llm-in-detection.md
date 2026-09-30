# 0001 — No LLM in the detection path

**Status:** accepted

## Context

The engine's job is to decide which changes in the world a human should look at.
It would be possible to ask a language model to judge significance, and it would
be easy to build.

## Decision

Detection is mathematics. No LLM is involved in deciding whether a signal exists,
and the engine must run correctly with no LLM present in the process.

An LLM may later sit on top of the engine to explain a signal, synthesize
context, compare sources or answer natural-language questions. It reads the
evidence package the engine already produced.

## Consequences

- Detection is deterministic, cheap, explainable and testable.
- Every signal can be justified from its own data: baseline, deviation, method,
  duration, evidence.
- The system cannot say "an AI found this important", because no AI is involved
  in deciding.
- Some judgement calls that a language model might make gracefully are instead
  explicit thresholds and rules. That is the intended trade: an explicit rule can
  be argued with, tuned and tested.
- Explanation quality is bounded by the evidence the engine captures, which is
  why evidence is captured thoroughly at detection time rather than
  reconstructed later.
