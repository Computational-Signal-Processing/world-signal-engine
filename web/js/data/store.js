/* The studio's single source of truth.
 *
 * It owns the polling loops, the connection state and the current lens, and it
 * publishes changes on the bus. Blocks read snapshots from here and subscribe;
 * they never poll and never fetch. Because there is exactly one poller per
 * resource, adding a region costs no extra requests. */

import { EventBus } from "../studio/event-bus.js";
import { ApiClient, ENDPOINTS } from "./api.js";

/** Poll cadences, in milliseconds. Matched to how fast each resource moves. */
const CADENCE = {
  world: 20_000,
  control: 30_000,
  signals: 30_000,
  activity: 20_000,
  metrics: 30_000,
  observatory: 60_000,
  sources: 120_000,
};

export class Store {
  constructor({ apiKey = () => "" } = {}) {
    this.bus = new EventBus();
    this.api = new ApiClient({ key: apiKey });

    this.connection = { state: "connecting", at: null, error: null };
    this.lens = "";
    this.lenses = [];

    this.world = null;
    this.control = null;
    this.signals = [];
    this.activity = [];
    this.metrics = null;
    this.observatory = null;
    this.sources = [];
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
    // before the network settles.
    this.refresh("world");
    this.refresh("control");
    this.refresh("signals");
    this.refresh("activity");
    this.refresh("metrics");
    this.refresh("observatory");
    // The catalog is static, but it is read on the first paint — the globe asks
    // it which sources carry coordinates — so waiting for its two-minute poll
    // would leave those blocks blind on a fresh screen.
    this.refresh("sources");
  }

  stop() {
    for (const id of this.timers) clearInterval(id);
    this.timers = [];
    this.started = false;
  }

  /* -------------------------------------------------------------- reading */

  /**
   * Refresh one resource and publish it.
   *
   * A failure is published too, as a connection state rather than as empty
   * data — the distinction the whole project rests on. A source that stops
   * answering must never look like a world that went quiet.
   */
  async refresh(name) {
    try {
      switch (name) {
        case "world":       this.world = await this.api.get(ENDPOINTS.world); break;
        case "control":     this.control = await this.api.get(ENDPOINTS.control); break;
        case "signals":     this.signals = normalizeSignals(await this.api.get(ENDPOINTS.signals, { query: this.signalQuery() })); break;
        case "activity":    this.activity = (await this.api.get(ENDPOINTS.activity, { query: { limit: 60 } }))?.items ?? []; break;
        case "metrics":     this.metrics = parseMetrics(await this.api.getText(ENDPOINTS.metrics)); break;
        case "observatory": this.observatory = await this.api.get(ENDPOINTS.observatory); break;
        case "sources":     this.sources = listOf(await this.api.get(ENDPOINTS.sources)); break;
        default: return;
      }
      this.setConnection("live", null);
      this.bus.emit(`data:${name}`, this.snapshot());
      this.bus.emit("data", { name, snapshot: this.snapshot() });
    } catch (err) {
      this.lastError = err;
      this.setConnection(connectionStateFor(err), err);
      this.bus.emit(`fail:${name}`, err);
    }
  }

  /** Fetch a signal's full detail and remember it. */
  async loadSignal(id) {
    if (!id) return null;
    const key = `signal:${id}`;
    if (this.detail.has(key)) return this.detail.get(key);
    const data = await this.api.get(`${ENDPOINTS.signals}/${encodeURIComponent(id)}`, { ttl: 0 });
    this.detail.set(key, data);
    this.bus.emit("detail:signal", data);
    return data;
  }

  /** Fetch source health and remember it per source. */
  async loadSource(id) {
    if (!id) return null;
    const key = `source:${id}`;
    if (this.detail.has(key)) return this.detail.get(key);
    const data = await this.api.get(`${ENDPOINTS.sources}/${encodeURIComponent(id)}`, { ttl: 0 });
    this.detail.set(key, data);
    this.sourceHealth.set(id, data?.health ?? null);
    this.bus.emit("detail:source", data);
    return data;
  }

  /** Fetch a series' observations plus its baseline. */
  async loadTimeline(seriesKey) {
    if (!seriesKey) return null;
    const key = `timeline:${seriesKey}`;
    if (this.detail.has(key)) return this.detail.get(key);
    const data = await this.api.get(ENDPOINTS.timeline, { ttl: 0, query: { series: seriesKey, limit: 200 } });
    this.detail.set(key, data);
    this.bus.emit("detail:timeline", data);
    return data;
  }

  /** Fetch a single observation's full record. */
  async loadObservation(id) {
    if (!id) return null;
    const key = `observation:${id}`;
    if (this.detail.has(key)) return this.detail.get(key);
    const data = await this.api.get(`${ENDPOINTS.observations}/${encodeURIComponent(id)}`, { ttl: 0 });
    this.detail.set(key, data);
    this.bus.emit("detail:observation", data);
    return data;
  }

  /** Fetch the raw payload behind an observation. */
  async loadRaw(observationId) {
    if (!observationId) return null;
    const key = `raw:${observationId}`;
    if (this.detail.has(key)) return this.detail.get(key);
    const data = await this.api.get(`${ENDPOINTS.observations}/${encodeURIComponent(observationId)}/raw`, { ttl: 0 });
    this.detail.set(key, data);
    this.bus.emit("detail:raw", data);
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

  async loadLenses() {
    try {
      this.lenses = await this.api.get(ENDPOINTS.lenses, { ttl: 600_000 });
      this.bus.emit("lenses", this.lenses);
    } catch (_) { /* the picker simply stays empty */ }
  }

  /* ------------------------------------------------------------- selection */

  select(patch) {
    Object.assign(this.selection, patch);
    this.bus.emit("selection", { ...this.selection });
  }

  /* ------------------------------------------------------------ connection */

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
      lens: this.lens,
      lenses: this.lenses,
      world: this.world,
      control: this.control,
      signals: this.signals,
      activity: this.activity,
      metrics: this.metrics,
      observatory: this.observatory,
      sources: this.sources,
      selection: this.selection,
    };
  }

  /** Force a full refresh, e.g. after the key changes or a scene recomposes. */
  refreshAll() {
    this.api.invalidate();
    for (const name of Object.keys(CADENCE)) this.refresh(name);
  }
}

/* ------------------------------------------------------------- normalizing */

/** The signals endpoint may answer with a bare array or an envelope. */
function normalizeSignals(payload) {
  if (Array.isArray(payload)) return payload;
  return payload?.items ?? [];
}

/**
 * A list resource, whether the engine wrapped it or not.
 *
 * `/sources` answers with `{ items: [...] }` while `/signals` has its own
 * normalizer; unwrapping in one place means a block never has to know which
 * endpoint used which envelope.
 */
function listOf(payload) {
  if (Array.isArray(payload)) return payload;
  return payload?.items ?? [];
}

function connectionStateFor(err) {
  if (err?.kind === "auth") return "error";
  if (err?.kind === "network") return "offline";
  return "reconnecting";
}

/**
 * Parse the Prometheus exposition format.
 *
 * Only the counters the studio actually shows are kept, and unlabelled samples
 * are stored as plain numbers while labelled ones keep their labels — the
 * signal-type breakdown needs `wse_signal_types_total{type="ANOMALY"}` and
 * cannot be read from a flat map.
 */
export function parseMetrics(text) {
  const values = {};
  const labelled = {};
  if (typeof text !== "string") return { values, labelled, at: Date.now() };
  for (const line of text.split("\n")) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;
    const match = /^([a-zA-Z_:][a-zA-Z0-9_:]*)(\{[^}]*\})?\s+(-?[\d.eE+-]+)$/.exec(trimmed);
    if (!match) continue;
    const [, name, labelPart, raw] = match;
    const value = Number(raw);
    if (!Number.isFinite(value)) continue;
    if (labelPart) {
      const labels = {};
      for (const pair of labelPart.slice(1, -1).split(",")) {
        const [k, v] = pair.split("=");
        if (k) labels[k.trim()] = (v || "").replace(/^"|"$/g, "");
      }
      (labelled[name] ??= []).push({ labels, value });
    } else {
      values[name] = value;
    }
  }
  return { values, labelled, at: Date.now() };
}
