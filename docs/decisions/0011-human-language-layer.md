# 0011 — A human-language layer, and where it lives

**Status:** accepted

## Context

The detection engine is honest and precise: it reports series keys, sigma values,
robust z-scores, source ids and candidate kinds. None of that is what a person
reads. Before this change the UI showed signal titles like
`Anomaly: synthetic_sensor::ent_sensor::sensor::unit`, and a reader had to
translate the machine's own vocabulary in their head to understand what had
changed.

The brief is explicit on both counts: the engine must explain itself, and it must
say *which measurement deviated from which baseline by how much* — never "an AI
found this important". So the explanation had to become a real part of the
output, not a rendering trick in the browser.

## Decision

**Add a `wse-presentation` crate that owns the translation.** It takes a
`Signal` and writes a `Narrative` onto it: a headline, the subject, what changed,
where, how large, why the signal exists, and — always — what is *not* known. It
also assigns the lifecycle status. It depends on `wse-model` only: no I/O, no
detection, no clock beyond what it is handed. Like the rest of the pipeline it is
deterministic and unit-testable.

**The narrative is produced by the engine, not the client.** `wse-signals` fills
it when a signal is formed, and `wse-engine` re-derives it after a merge so a
signal that has absorbed new evidence reports its full, updated span. The UI
renders the narrative; it does not invent it. This keeps the explanation
authoritative — the same text is what the API returns, what the CLI prints, and
what a future LLM would read as an evidence package.

**Unknowns are mandatory.** Every narrative carries a list of what the engine
cannot claim: a single source, no second corroboration, a metric with no richer
description configured, synthetic rather than live data, and the standing caveat
that a quiet feed is not a quiet world. A signal that only asserts is not
trustworthy; a signal that states its limits is.

**Language is not detection.** The signal *types* (`NOW`, `ANOMALY`,
`EARLY_SIGNAL`, `CONVERGENCE`, `IMPACT`), the lifecycle *statuses*, and the
direction words are labels, not measurements. They are translated in the UI
(English and Turkish, chosen by browser, `?lang=`, or the header toggle), while
the engine emits stable English identifiers. Detection is identical in every
language; only the words around it change.

**Lenses carry a human description.** Each `config/lenses/*.yaml` gained a
`description` field, so the Lenses screen explains what a lens shows in a
sentence instead of echoing its raw filter fields. A lens that shows nothing yet
says so, rather than looking broken.

**The WORLD screen composes its answer once, on the server.** `GET /world`
returns the active-signal count, the per-type and per-status breakdown, source
health, and the freshest signals as a "NOW" strip. The client previously fetched
every signal and counted in the browser; composing it server-side means the
numbers on the header and the cards in the feed come from one query and cannot
drift.

## Consequences

- A signal now reads in a sentence a person can act on, and still drills down to
  the observation and the raw bytes.
- The explanation is part of the API contract, so it is versioned and testable
  rather than being a property of one client.
- Turkish support is presentation-only. Adding a language is one object in
  `web/app.js`; no Rust changes and no detection changes.
- The narrative is bounded by the evidence the engine captured. It cannot
  explain more than the pipeline measured — which is the point.
