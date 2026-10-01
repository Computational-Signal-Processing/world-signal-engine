/* The only place in the studio that touches the network.
 *
 * Blocks never fetch. They ask the store for a resource and subscribe to
 * changes. That keeps every request in one file — so caching, auth, error
 * shape and retry are decided once — and it means a block can be tested with a
 * plain object instead of a stubbed fetch. */

import { EventBus } from "../studio/event-bus.js";

/** The API paths the studio reads. Every one of these exists on the engine. */
export const ENDPOINTS = {
  health: "/health",
  metrics: "/metrics",
  world: "/world",
  observatory: "/observatory",
  control: "/control",
  activity: "/activity",
  signals: "/signals",
  events: "/events",
  observations: "/observations",
  sources: "/sources",
  lenses: "/lenses",
  timeline: "/timeline",
};

const DEFAULT_TTL = 15_000;

/**
 * A tiny request cache with in-flight de-duplication.
 *
 * Two regions asking for the same resource in the same tick must produce one
 * request, not two — the studio re-composes scenes often and a duplicate fetch
 * per region per transition would be the easiest way to overload the engine.
 */
export class ApiClient {
  constructor({ base = "", key = () => "" } = {}) {
    this.base = base;
    this.key = key;
    this.cache = new Map();      // path -> { at, data, ttl }
    this.inflight = new Map();   // path -> Promise
    this.bus = new EventBus();
    this.failures = 0;
  }

  /** Current API key, read lazily so the control room can set it at runtime. */
  headers() {
    const k = this.key();
    return k ? { Authorization: `Bearer ${k}` } : {};
  }

  /**
   * GET a JSON resource.
   *
   * `ttl: 0` forces a fresh read. A failed request is never cached, so a source
   * that recovers is visible immediately rather than after a stale timeout.
   */
  async get(path, { ttl = DEFAULT_TTL, query = null } = {}) {
    const url = this.url(path, query);
    const cached = this.cache.get(url);
    if (cached && ttl > 0 && Date.now() - cached.at < cached.ttl) return cached.data;
    if (this.inflight.has(url)) return this.inflight.get(url);

    const promise = this.fetchJson(url, ttl)
      .finally(() => this.inflight.delete(url));
    this.inflight.set(url, promise);
    return promise;
  }

  /** GET a text resource (the metrics exposition format is not JSON). */
  async getText(path, { ttl = DEFAULT_TTL, query = null } = {}) {
    const url = this.url(path, query);
    const cached = this.cache.get(url);
    if (cached && ttl > 0 && Date.now() - cached.at < cached.ttl) return cached.data;
    if (this.inflight.has(url)) return this.inflight.get(url);

    const promise = this.fetchText(url, ttl)
      .finally(() => this.inflight.delete(url));
    this.inflight.set(url, promise);
    return promise;
  }

  url(path, query) {
    if (!query) return this.base + path;
    const params = new URLSearchParams();
    for (const [k, v] of Object.entries(query)) {
      if (v !== null && v !== undefined && v !== "") params.set(k, String(v));
    }
    const qs = params.toString();
    return this.base + path + (qs ? `?${qs}` : "");
  }

  async fetchJson(url, ttl) {
    const response = await this.raw(url);
    if (!response.ok) throw await this.error(response);
    const data = await response.json();
    this.cache.set(url, { at: Date.now(), data, ttl });
    this.failures = 0;
    return data;
  }

  async fetchText(url, ttl) {
    const response = await this.raw(url);
    if (!response.ok) throw await this.error(response);
    const data = await response.text();
    this.cache.set(url, { at: Date.now(), data, ttl });
    this.failures = 0;
    return data;
  }

  async raw(url) {
    try {
      return await fetch(url, { headers: this.headers(), cache: "no-store" });
    } catch (cause) {
      this.failures += 1;
      const err = new Error(`network unreachable: ${url}`);
      err.kind = "network";
      err.cause = cause;
      throw err;
    }
  }

  async error(response) {
    this.failures += 1;
    let detail = "";
    try {
      const body = await response.json();
      detail = body?.error || body?.message || "";
    } catch (_) { /* not every error is JSON */ }
    const err = new Error(detail || `request failed: ${response.status} ${url_tail(response)}`);
    err.kind = response.status === 401 || response.status === 403 ? "auth" : "http";
    err.status = response.status;
    return err;
  }

  /** Drop every cached entry, e.g. after the API key changes. */
  invalidate() { this.cache.clear(); }
}

function url_tail(response) {
  try { return new URL(response.url).pathname; } catch (_) { return ""; }
}
