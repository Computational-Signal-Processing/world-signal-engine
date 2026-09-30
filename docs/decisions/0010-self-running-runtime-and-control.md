# 0010 — A self-running engine, and how it is observed and controlled

**Status:** accepted

## Context

Through productionization the engine was deployable but not self-driving. `serve`
ran collection once at startup and then sat still: it answered queries about a
world it had stopped watching. Anything that kept it current was external — a
cron job, an operator, a second process.

Two problems fell out of that. First, a served instance aged: without an operator
re-running it, the feed drifted behind the world it claimed to describe. Second,
the engine had no way to say what it was doing *right now*. Logs were the only
window, and a log line is not a product surface — a person watching the UI could
not tell whether collection was running, stalled, or failing on one source.

## Decision

**Run the scheduler continuously inside `serve`.** With `--collect`, the process
keeps observing on its own cadence instead of collecting once. The loop honors
runtime controls on every pass, so pausing or disabling takes effect without a
restart.

**Put the runtime state in the engine, not the CLI.** `RuntimeState` holds
uptime, a collection toggle, a per-source enable/disable set, coalesced run-now
requests, and a bounded activity ring buffer. It lives behind the same lock as
the engine, so an HTTP reader sees one consistent picture and a scheduler pass
sees the same controls a request just changed. Keeping it in the CLI would have
meant the API could not expose it without reaching into a process it does not
own.

**Emit activity as the pipeline runs, and treat it as telemetry.** Each pass
records observation batches, anomaly candidates, events, signals, and source
health changes into a bounded ring buffer (last N, oldest dropped) and broadcasts
them to live subscribers. It is deliberately *not* persisted: activity is a view
of what the process is doing, not a fact about the world. The facts — the
observations, events, signals and source health — are already in the store and
survive a restart. Persisting the activity log would add unbounded write load to
record something the durable data already implies.

**Push it over Server-Sent Events, read with `fetch`.** `GET /events` streams the
activity as SSE frames. The UI does *not* use `EventSource`, which cannot set an
`Authorization` header — using it would force the API key into the query string,
where it lands in browser history, server logs and `Referer`. The UI reads the
stream off a `fetch` body by hand instead, so the key stays a header.

**Expose the controls behind the same key as the data.** `GET /control`,
`POST /control/collection`, `POST /sources/:id/enabled` and
`POST /sources/:id/run` sit behind the same authentication as every other route.
Pausing collection stops new observations; it never stops the API, the UI, or
queries over what was already collected.

**Present facts, not a verdict.** The UI states the deviation, the persistence
and the number of independent sources behind a signal, and carries signal type
in icon, label and shape rather than colour alone. It does not print a single
importance score, because that would be the "the AI thought this mattered" the
brief forbids.

## Consequences

- A served instance stays current without an operator, which is what makes it a
  product rather than a demo.
- "What is it doing right now?" is answerable from the product: the System
  screen shows real state and streams activity live.
- An operator can pause a misbehaving source and keep investigating — collection
  and observation are separable.
- A rate-limited source is reported as rate-limited, never as zero world
  activity. This is the brief's critical rule, and it now holds in the running
  product, not only in tests.
- The activity buffer is bounded, so a long-running process cannot grow without
  limit on telemetry.
- The UI needs no build step and no framework; it is static files the API serves.

## Alternatives considered

- **Collect on a timer outside the process (cron, systemd timer).** Rejected: it
  puts the schedule in a second place that cannot be paused or inspected through
  the API, and it restarts the process to change cadence.
- **`EventSource` for the live stream.** Rejected: it cannot send an
  `Authorization` header, so the key would have to go in the URL. The whole point
  of the header is to keep it out of the URL.
- **A WebSocket.** Rejected: the stream is one-way and SSE is enough. A
  WebSocket adds a second protocol and framing to maintain for no capability the
  UI needs.
- **Persisting the activity log.** Rejected: it is a view of the process, not
  data about the world; the durable facts already imply it, and storing it would
  grow without bound.
- **Auto-disabling a source after repeated failures.** Rejected: a rate limit or
  a transient 5xx is not a fault the engine should act on destructively. It
  records the failure and surfaces the health; disabling is a decision for a
  person, and the control exists for them to make it.
- **A separate control service.** Rejected: brief section 39 puts microservices
  out of scope, and the control state belongs beside the engine state it
  governs.
