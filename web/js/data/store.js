/* The studio's single source of truth.
 *
 * Data reaches a block through exactly one path:
 *
 *     backend → adapters → store → data bus → blocks
 *
 * The store owns the polling loops, the live-stream ingestion, the connection
 * state and the current lens, and it publishes changes on the bus. Blocks read
 * a snapshot from here and subscribe; they never poll, never fetch, and never
 * learn an endpoint name.
 *
 * Every domain carries its own state — `loading`, `ready`, `stale`, `error`,
 * `unavailable` — so a reader can tell "we have no data yet" from "the engine
 * stopped answering" from "the engine answered with nothing". A failed read is
 * never published as empty data, which is the same distinction the engine makes
 * between a quiet world and a dead collector. */

import { EventBus } from "../studio/event-bus.js";
import { ApiClient, ENDPOINTS } from "./api.js";
import * as adapt from "./adapters.js";

/** Poll cadences, in milliseconds. Matched to how fast each resource moves. */
const CADENCE = {
  world: 20_000,
  control: 30_000,
  signals: 30_000,
  activity: 20_000,
  metrics: 30_000,
  observatory: 60_000,
  sources: 120_000,
  lenses: 600_000,
  health: 30_000,
};

/** How a domain is read: which endpoint, whether it is text, how it adapts. */
const DOMAINS = {
  world:       { path: ENDPOINTS.world,       adapt: adapt.world },
  control:     { path: ENDPOINTS.control,     adapt: adapt.control },
  signals:     { path: ENDPOINTS.signals,     adapt: adapt.signals },
  activity:    { path: ENDPOINTS.activity,    adapt: adapt.activity },
  observatory: { path: ENDPOINTS.observatory, adapt: adapt.observatory },
  sources:     { path: ENDPOINTS.sources,     adapt: adapt.sources },
  lenses:      { path: ENDPOINTS.lenses,      adapt: adapt.lenses },
  health:      { path: ENDPOINTS.health,      adapt: adapt.health },
  metrics:     { path: ENDPOINTS.metrics,     text: true, adapt: adapt.metrics },
};

export class Store {
  constructor({ apiKey = () => "" } = {}) {
    this.bus = new EventBus();
    this.api = new ApiClient({ key: apiKey });

    this.connection = { state: "connecting", at: null, error: null };
    this.lens = "";
    this.lenses = [];

    // Domain values. `null` means "nothing read yet"; the matching entry in
    // `state` says whether that is because we are still loading or because the
    // read failed.
    this.world = null;
    this.control = null;
    this.signals = [];
    this.activity = [];
    this.metrics = null;
    this.observatory = null;
    this.sources = [];
    this.health = null;

    this.state = {};
    for (const name of Object.keys(DOMAINS)) this.state[name] = "loading";

    this.sourceHealth = new Map();   // source_id -> health payload
    this.selection = { signalId: null, eventId: null, observationId: null, sourceId: null };
    this.detail = new Map();         // cache of fetched detail resources

    this.timers = [];
    this.started = false;
    this.lastError = null;
  }

  /* ------------------------------------------------------------ lifecycle */

  start() {
    if (this.started) return;
    this.started = true;
    for (const [name, ms] of Object.entries(CADENCE)) {
      this.timers.push(setInterval(() => this.refresh(name), ms));
    }
    // The first pass runs immediately and is not awaited, so the shell can paint
    // before the network settles. `sources` is read on the first paint because
    // the globe asks the catalog which sources carry coordinates.
    for (const name of Object.keys(CADENCE)) this.refresh(name);
  }

  stop() {
    for (const id of this.timers) clearInterval(id);
    this.timers = [];
    this.started = false;
  }

  /* -------------------------------------------------------------- reading */

  /** The state of one domain: loading | ready | stale | error | unavailable. */
  dataState(name) { return this.state[name] ?? "loading"; }

  /** True when a domain has a value a block may draw. */
  hasData(name) {
    const value = this[name];
    if (value === null || value === undefined) return false;
    if (Array.isArray(value)) return value.length > 0;
    return true;
  }

  /**
   * Read one domain and publish it.
   *
   * A failure is published as a domain state, not as empty data. When a value
   * was already held it is kept and the domain is marked `stale` — the reader is
   * told the picture is old, rather than being shown a fresh-looking zero.
   */
  async refresh(name) {
    const domain = DOMAINS[name];
    if (!domain) return;

    try {
      const raw = domain.text
        ? await this.api.getText(domain.path)
        : await this.api.get(domain.path, { query: name === "signals" ? this.signalQuery() : null });
      const value = domain.adapt(raw);
      if (value === null) {
        // The engine answered, but not with something this adapter understands.
        // That is a failure to read, not an empty world.
        const err = new Error(`${name}: unexpected response shape`);
        err.kind = "shape";
        throw err;
      }
      this[name] = value;
      this.setState(name, "ready");
      this.setConnection("live", null);
      this.publish(name);
    } catch (err) {
      this.lastError = err;
      // `unavailable` when the shape was wrong and we hold nothing to fall back
      // on; `stale` when we are still showing a value that is now old.
      const next = this.hasData(name) ? "stale" : (err?.kind === "shape" ? "unavailable" : "error");
      this.setState(name, next);
      this.setConnection(connectionStateFor(err), err);
      this.bus.emit(`fail:${name}`, err);
      this.publish(name);
    }
  }

  /** Publish a domain change on the bus. */
  publish(name) {
    this.bus.emit(`data:${name}`, this.snapshot());
    this.bus.emit("data", { name, snapshot: this.snapshot() });
  }

  /**
   * Fold one live-stream event into the store.
   *
   * This is the second data path: the initial snapshot builds the state, and a
   * stream event updates the domain it concerns. A block is never told to reload
   * the page, and it never sees the stream directly.
   *
   * The event kinds are the engine's own (`ActivityKind`, SCREAMING_SNAKE_CASE):
   * `OBSERVATION`, `SIGNAL`, `SOURCE_FAILED`, `SOURCE_RECOVERED`, and the rest.
   */
  ingestActivity(event) {
    if (!event || typeof event !== "object") return;
    // Always surface the raw line, so the activity stream block can append it
    // without waiting for the next poll.
    this.bus.emit("sse:activity", event);

    switch (event.kind) {
      case "OBSERVATION":
        this.refresh("activity");
        break;
      case "SIGNAL":
        this.refresh("signals");
        this.refresh("world");
        break;
      case "EVENT":
      case "ANOMALY":
        this.refresh("signals");
        break;
      case "SOURCE_FAILED":
      case "SOURCE_RATE_LIMITED":
        this.bus.emit("sse:source_failed", event);
        break;
      case "SOURCE_RECOVERED":
        this.bus.emit("sse:source_recovered", event);
        break;
      default:
        break;
    }
  }

  /* --------------------------------------------------------------- details */

  /** Fetch a signal's full detail and remember it. */
  async loadSignal(id) {
    return this.loadDetail("signal", id, () =>
      this.api.get(`${ENDPOINTS.signals}/${encodeURIComponent(id)}`, { ttl: 0 }).then(adapt.signal));
  }

  /** Fetch source health and remember it per source. */
  async loadSource(id) {
    const data = await this.loadDetail("source", id, () =>
      this.api.get(`${ENDPOINTS.sources}/${encodeURIComponent(id)}`, { ttl: 0 }).then(adapt.source));
    if (data) this.sourceHealth.set(id, data.health ?? null);
    return data;
  }

  /** Fetch a series' observations plus its baseline. */
  async loadTimeline(seriesKey) {
    return this.loadDetail("timeline", seriesKey, () =>
      this.api.get(ENDPOINTS.timeline, { ttl: 0, query: { series: seriesKey, limit: 200 } }).then(adapt.timeline));
  }

  /** Fetch a single observation's full record. */
  async loadObservation(id) {
    return this.loadDetail("observation", id, () =>
      this.api.get(`${ENDPOINTS.observations}/${encodeURIComponent(id)}`, { ttl: 0 }).then(adapt.observation));
  }

  /** Fetch the raw payload behind an observation. */
  async loadRaw(observationId) {
    return this.loadDetail("raw", observationId, () =>
      this.api.get(`${ENDPOINTS.observations}/${encodeURIComponent(observationId)}/raw`, { ttl: 0 }).then(adapt.raw));
  }

  /** Fetch an event, with the observation chain the drill-down walks. */
  async loadEvent(id) {
    return this.loadDetail("event", id, () =>
      this.api.get(`${ENDPOINTS.events}/${encodeURIComponent(id)}`, { ttl: 0 }).then(adapt.event));
  }

  async loadDetail(kind, id, fetch) {
    if (!id) return null;
    const key = `${kind}:${id}`;
    if (this.detail.has(key)) return this.detail.get(key);
    const data = await fetch();
    this.detail.set(key, data);
    this.bus.emit(`detail:${kind}`, data);
    return data;
  }

  /** Forget fetched details, e.g. when the selection changes. */
  forgetDetails() { this.detail.clear(); }

  /* --------------------------------------------------------------- filters */

  signalQuery() {
    const query = { limit: 40 };
    if (this.lens) query.lens = this.lens;
    return query;
  }

  setLens(lens) {
    if (this.lens === lens) return;
    this.lens = lens || "";
    this.refresh("signals");
    this.bus.emit("lens", this.lens);
  }

  /* ------------------------------------------------------------- selection */

  select(patch) {
    Object.assign(this.selection, patch);
    this.bus.emit("selection", { ...this.selection });
  }

  /**
   * Drop the reader's pick.
   *
   * A format change is a different reader at a different distance — a pick made
   * on a desk screen should not still be holding the drill-down open on a phone.
   * The live signal is the right thing to show again.
   */
  clearSelection() {
    const { signalId, eventId, observationId, sourceId } = this.selection;
    if (!signalId && !eventId && !observationId && !sourceId) return;
    this.selection = { signalId: null, eventId: null, observationId: null, sourceId: null };
    this.bus.emit("selection", { ...this.selection });
  }

  /* ------------------------------------------------------------ connection */

  setState(name, state) {
    if (this.state[name] === state) return;
    this.state[name] = state;
    this.bus.emit(`state:${name}`, state);
  }

  setConnection(state, error) {
    const changed = this.connection.state !== state;
    this.connection = { state, at: Date.now(), error: error ?? null };
    if (changed) this.bus.emit("connection", this.connection);
  }

  /* -------------------------------------------------------------- snapshot */

  /** A frozen view of everything a block might need, cheap enough to pass around. */
  snapshot() {
    return {
      connection: this.connection,
      dataState: this.state,
      lens: this.lens,
      lenses: this.lenses,
      world: this.world,
      control: this.control,
      signals: this.signals,
      activity: this.activity,
      metrics: this.metrics,
      observatory: this.observatory,
      sources: this.sources,
      health: this.health,
      selection: this.selection,
    };
  }

  /** Force a full refresh, e.g. after the key changes or a reconnect. */
  refreshAll() {
    this.api.invalidate();
    for (const name of Object.keys(DOMAINS)) this.refresh(name);
  }
}

function connectionStateFor(err) {
  if (err?.kind === "auth") return "error";
  if (err?.kind === "network") return "offline";
  return "reconnecting";
}
