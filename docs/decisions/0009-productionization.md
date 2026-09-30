# 9. Persistence, retention and an authenticated API

Date: 2026-09-30

## Status

Accepted.

## Context

Through Phase 12 the engine was correct and fully in memory. `serve` built the
whole world at startup and forgot it on exit; the API was open on every route;
CORS allowed every origin. That is the right shape for a demo and the wrong
shape for a service that runs for weeks on a VM.

Three things had to change for a real deployment, and each had a design
question attached that was not obvious:

1. **State has to survive a restart.** The interesting part is not the
   observations — those are cheap to re-collect — but the *conclusions*:
   signals, source health, and the baselines the detector has learned. A
   detector that starts cold reports "normal" as everything for its first hour,
   which is exactly when a deployment is most likely to be watched.
2. **Data grows without bound.** Observations and raw payloads accumulate
   forever unless something ages them out. But a signal is a conclusion a person
   was investigating, and deleting it because its samples expired loses the
   thing the project exists to produce.
3. **An open API on a public address is a mistake.** The engine had no notion of
   a caller at all. Adding one had to not break the loopback demo, and the
   default had to be safe rather than convenient.

## Decision

### Persistence: SQLite behind the existing `Store` traits

`SqliteStore` implements the same traits as `InMemoryStore`, so no engine code
changed. `--data-dir` (or `WSE_DATA_DIR`) selects it; without it the engine runs
in memory exactly as before.

The store is selected *outside* the pipeline. `serve` now funnels both backends
through one generic `run_served<S: Store>`, so collection, retention, shutdown
and the API are one code path and cannot drift between backends. The
alternative — a `match` with a copy of the serving logic in each arm — would
have been a place for a bug to live in only one of them.

Rehydration is bounded: `--rehydrate-history N` (default 500) replays the N most
recent observations per series into the detector's windows. It does not replay
the whole history, because a year of data would delay startup for no benefit —
the detector only ever looks back a window's worth.

`SqliteStore` holds its `Connection` in a `std::sync::Mutex`. `rusqlite`'s
`Connection` is `Send` but not `Sync`, and the API holds the engine behind an
async lock. One guard per function, taken and released within the call, is the
discipline; holding two across a re-entrant call self-deadlocks, because the
mutex is not reentrant. This bit `prune_raw_to`, which held a guard while the
loop body acquired it again.

### Retention: observations age out, conclusions do not

`--retention-days` deletes observations older than a cutoff.
`--raw-max-bytes` prunes raw payloads, oldest first, until under the cap.

Events and signals are deliberately **not** deleted by retention. A signal's
evidence may point at observations that are gone; the drill-down reports that
rather than pretending they never existed.

Raw pruning drops the bytes *and* the metadata row but leaves the `RawReference`
on the observation. Losing the bytes is a documented retention policy; losing
the reference would be data loss, because the drill-down would no longer be able
to say what the payload had been.

Pruning must forget a payload in memory as well as delete its file. Deleting
only the file leaves the size in the in-memory index, so `bytes_used` keeps
counting bytes that are gone and retention keeps pruning until the store looks
empty. This was a real bug, found by the test that now covers it.

### Security: safe defaults, one hard refusal

`SecurityConfig` is read from the environment and applied as middleware.

- **Authentication.** `WSE_API_KEYS` (comma-separated) or `WSE_API_KEY`.
  Every route requires a key except `/health`. Comparison is constant-time: a
  short-circuiting `==` leaks how many leading bytes of a guess are correct.
- **`/health` stays open.** A load balancer has to reach it without credentials.
  It returns counts, not data. `/metrics` stays behind the key by default
  because it exposes operational detail useful for timing requests.
- **CORS defaults to no origins.** The bundled UI is same-origin and needs no
  headers; an unconfigured API tells no other page it may read the responses.
  `WSE_CORS_ORIGINS` opts in per origin.
- **The CLI warns when it serves without a key.** There is no flag that makes an
  open API safe on a public address, so the warning is the honest signal; a
  deployment that wants no key should bind loopback and use a proxy.

The UI gained a key field, hidden unless a key is already stored or a request
came back 401. It is kept in `localStorage` and sent as a header, never in the
query string, because a key in a URL ends up in history, logs and `Referer`.

## Consequences

- A deployment resumes warm: signals, source health and baselines survive a
  restart, and the drill-down still resolves.
- The two backends share one serving path, so a change to collection or
  retention cannot apply to only one of them.
- Retention has an explicit, testable policy, and the failure mode it had
  (counting deleted bytes) is covered by a test.
- An accidental public deployment is refused by a warning at startup rather
  than being silently permitted.
- Adding a store backend means implementing the traits; it does not mean
  touching `serve`.

## Alternatives considered

- **A separate service for persistence.** Rejected: brief section 39 puts
  microservices out of scope, and one process with SQLite is enough for the
  scale the project targets.
- **Deleting signals with their observations.** Rejected: it discards the
  project's output to save disk the project is not short of.
- **Replaying all history on startup.** Rejected: unbounded startup cost for no
  detection benefit; the detector only looks back a window.
- **A wildcard CORS policy with auth.** Rejected: authentication is a header,
  and a browser will happily send it cross-origin if CORS says the origin is
  allowed. The two have to be configured together, so the safe default is none.
