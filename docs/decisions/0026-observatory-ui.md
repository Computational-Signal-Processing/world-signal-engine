# 0026 — The observatory UI: single screen, no build step, broadcast mode

Status: accepted (2026-10-01)

## Context

The web client today is a multi-page instrument panel: a `#/world` feed with a
NOW strip, a `#/map` page, a `#/system` page, a `#/timeline` page. Each page is
one question, read one at a time. That is the right shape for *investigating* a
signal, and it stays.

The brief asks for a second, complementary shape: a single-screen control room
that a person can leave running on a wall, in a studio, or on a livestream —
where the whole state of the world is visible at once and a breaking signal
interrupts. Call it the **observatory**. It is a view, not a new system: it
reads the same signals the feed does.

The brief proposes React, Tailwind, Shadcn UI, Recharts and Lucide Icons. This
ADR records why the observatory is built with the same no-build vanilla stack as
the rest of `web/` instead, and what "broadcast mode" means.

## Decision

### 1. No build step, no npm dependency tree

`web/` is served directly by the API's `ServeDir` fallback. It has no bundler,
no `node_modules`, no lockfile, and no transitive supply chain. A contributor
can edit a file, reload, and see the change. `cargo run -p wse-cli -- serve` is
the entire toolchain.

Adding React + Tailwind + Shadcn + Recharts would introduce a second toolchain
(Node), a second lockfile, a build artifact that must be produced before the
binary serves a usable UI, and a large dependency tree whose integrity the
Rust project's supply-chain discipline (see the deny/audit posture in
`DEVELOPMENT.md`) does not cover. For a project whose stated philosophy is
minimal, single-machine, low-operations, that cost is not repaid by the
observatory's requirements: a fixed grid, a sparkline, a scatter map, a list,
and a marquee. All five are small, and all five are already implemented in
`web/app.js` for the existing pages.

The one thing a framework would genuinely help with — reactive re-render on
every live frame — is handled here by targeted DOM updates: the observatory
keeps node references and mutates their text, the way the existing activity
stream already does. The live cadence is a few updates per second at most, not a
per-frame virtual DOM diff.

So: **vanilla, in `web/`, no build step.** Revisit only if the observatory grows
past what hand-written DOM updates can carry.

### 2. Broadcast mode is a query parameter, not a separate page

`?broadcast=1` (also reachable from a header toggle) puts the observatory into
a presentation state:

- chrome the viewer does not need is hidden — nav tabs, the lens picker, the
  API-key field, the footer;
- the layout fills the viewport with no scrollbars;
- a watermark states whether the engine is watching, how fresh its data is, and
  the wall clock, so a recorded stream can never be mistaken for a live one.

It is a query parameter rather than a route because it is a *presentation* of
the same screen, not a different screen: the data, the drill-down targets and
the live stream are identical. A route would duplicate the state machine.

Broadcast mode never hides a fact. It hides *controls*. The live/stale state,
the source health and the data-origin label stay visible, because those are what
tell a viewer whether what they are watching is true.

### 3. Every number on the observatory is a real measurement

The observatory is where a fabricated "vital sign" would be most tempting and
most damaging — a pulsing "RISK: 82" score would read as authoritative and mean
nothing. So:

- The metric cards are **per-category rollups of stored observations**, not
  invented indices: the current value, its baseline, the change, and a sparkline
  of the actual series. A category with no live series says so.
- The severity label (LOW / MEDIUM / CRITICAL) is derived from the signal's own
  quality dimensions and deviation, and the derivation is stated in the card,
  not hidden behind a colour.
- "No data" and "zero" stay distinct, exactly as in the engine: a category whose
  sources are all failing shows *no data*, not *quiet*.

This is the same rule as `docs/philosophy.md`, applied to the loudest surface in
the product.

### 4. Drill-down survives the redesign

Every element on the observatory is a link into the existing pages: a metric
card opens its series timeline, a hotspot opens its signal, a feed row opens its
signal, the alert's "Investigate" opens the signal page. The observatory
adds a way to *see* the world at a glance; it does not replace the way to
*trace* a signal back to raw data.

### 5. The observatory is the landing page

An empty hash now renders the observatory rather than `#/world`. The brief's
first question is "what in the world is changing right now?", and the board
answers it in one screen; the feed answers "what exactly is this signal?" — a
follow-up. `#/world` remains one click away, and every existing route is
unchanged.

This is a change to the default landing surface, so it is recorded here rather
than left implicit in the router.

## Consequences

- `web/` grows a second layout. It is kept in one file with the rest of the
  client; if it grows past readability it moves to `web/observatory.js` and the
  page loads two scripts.
- One new read-only endpoint backs the board: `GET /observatory?window=24h|7d`.
  It is a composed read over the existing stores (sources, observations,
  baselines, signals) and adds no new storage and no new writes.
- The project keeps one toolchain. A future contributor who wants React must
  argue it against this record.
- Accessibility and colour-blindness rules from the existing CSS carry over: a
  signal type is never carried by colour alone on the observatory either, and a
  category's severity is stated in words as well as in a border colour.
