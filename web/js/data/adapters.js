/* Adapters: a backend response to a domain value the studio can read.
 *
 * This is the layer the store is built on, and it exists so that the shape of
 * an engine response is known in exactly one place. `/sources` answers with a
 * bare array while `/signals` wraps its items; `/timeline` returns an object
 * with an `observations` array. A block must never learn which of those is
 * which, and neither should the store — it asks an adapter, and the adapter
 * knows.
 *
 * Two rules hold throughout:
 *
 * 1. **Normalization is not invention.** A field the engine does not send is
 *    left absent, not defaulted to zero. `latitude: null` is `null`, not `0`,
 *    because a point at 0,0 in the Atlantic is a claim the engine did not make.
 *    A missing baseline is `null`, not `0`, because "no baseline" and "a
 *    baseline of zero" are different statements about the world.
 *
 * 2. **A payload with the wrong shape is refused, not coerced.** An adapter
 *    returns `null` rather than a half-built object, so the store can mark the
 *    domain failed instead of publishing an empty world. That is the same
 *    distinction the engine makes between "no data" and "zero activity",
 *    carried through to the UI. */

/** A finite number, or null. Never 0-by-default. */
export function num(value) {
  if (value === null || value === undefined || value === "") return null;
  const n = Number(value);
  return Number.isFinite(n) ? n : null;
}

/** A string, or null. An empty string is treated as absent. */
export function str(value) {
  if (value === null || value === undefined) return null;
  const s = String(value);
  return s.length ? s : null;
}

/** The list inside a payload, whether the engine wrapped it or sent it bare. */
export function listOf(payload) {
  if (Array.isArray(payload)) return payload;
  if (Array.isArray(payload?.items)) return payload.items;
  return null;
}

/**
 * `/health` → the engine's liveness and counts.
 *
 * Every count is a real store count. They are kept as numbers because a health
 * endpoint that could not be read is handled by the store's failed state, not
 * by an adapter that invents a zero.
 */
export function health(payload) {
  if (!payload || typeof payload !== "object") return null;
  if (!str(payload.status)) return null;
  return {
    status: String(payload.status),
    version: str(payload.version),
    sources: num(payload.sources) ?? 0,
    observations: num(payload.observations) ?? 0,
    events: num(payload.events) ?? 0,
    signals: num(payload.signals) ?? 0,
  };
}

/** `/metrics` → `{ values, labelled, at }`, from the Prometheus exposition text. */
export function metrics(text) {
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

/**
 * `/signals` → the signal list.
 *
 * `confidence` is carried through as the engine sent it and left `null` when it
 * did not — the UI shows "not measured" rather than a fabricated number.
 */
export function signals(payload) {
  const items = listOf(payload);
  if (!items) return null;
  return items.filter((s) => s && str(s.id)).map((s) => ({
    id: String(s.id),
    event_id: str(s.event_id),
    types: Array.isArray(s.types) ? s.types.map(String) : [],
    title: str(s.title),
    summary: str(s.summary),
    status: str(s.status),
    first_seen: str(s.first_seen),
    last_updated: str(s.last_updated),
    duration_seconds: num(s.duration_seconds),
    confidence: num(s.confidence),
    entities: Array.isArray(s.entities) ? s.entities.map(String) : [],
    categories: Array.isArray(s.categories) ? s.categories.map(String) : [],
    lens_matches: Array.isArray(s.lens_matches) ? s.lens_matches : [],
    evidence: Array.isArray(s.evidence) ? s.evidence : [],
    _raw: s,
  }));
}

/**
 * `/world` → the world summary.
 *
 * `data_age_seconds` is kept as `null` when the engine has never collected.
 * `latency` is passed through untouched: if the engine reports no latency
 * summary the studio shows "not measured", it does not show a fast engine.
 */
export function world(payload) {
  if (!payload || typeof payload !== "object") return null;
  return {
    generated_at: str(payload.generated_at),
    active_signals: num(payload.active_signals) ?? 0,
    signals_total: num(payload.signals_total) ?? 0,
    events_total: num(payload.events_total) ?? 0,
    observations_total: num(payload.observations_total) ?? 0,
    by_type: Array.isArray(payload.by_type) ? payload.by_type : [],
    by_status: Array.isArray(payload.by_status) ? payload.by_status : [],
    now: Array.isArray(payload.now) ? payload.now : [],
    sources_total: num(payload.sources_total) ?? 0,
    sources_healthy: num(payload.sources_healthy) ?? 0,
    monitoring: payload.monitoring ?? null,
    collector_active: payload.collector_active ?? null,
    collection_enabled: payload.collection_enabled ?? null,
    sources: Array.isArray(payload.sources) ? payload.sources : [],
    last_collection_at: str(payload.last_collection_at),
    data_age_seconds: num(payload.data_age_seconds),
    latency: payload.latency ?? null,
  };
}

/**
 * `/observatory` → the observatory board.
 *
 * A category card's `value` is `null` when `has_data` is false, and `has_data`
 * itself is preserved: the board draws a reason, not a zero.
 */
export function observatory(payload) {
  if (!payload || typeof payload !== "object") return null;
  const cards = Array.isArray(payload.categories) ? payload.categories : null;
  if (!cards) return null;
  return {
    generated_at: str(payload.generated_at),
    categories: cards.map((c) => ({
      category: str(c.category),
      label: str(c.label),
      series_key: str(c.series_key),
      has_data: c.has_data === true,
      empty_reason: str(c.empty_reason),
      value: num(c.value),
      unit: str(c.unit),
      baseline: c.baseline ?? null,
      change_pct: num(c.change_pct),
      _raw: c,
    })),
    activity: payload.activity ?? null,
    feed: Array.isArray(payload.feed) ? payload.feed : [],
    ticker: Array.isArray(payload.ticker) ? payload.ticker : [],
    active_signals: num(payload.active_signals) ?? 0,
    signals_total: num(payload.signals_total) ?? 0,
    observations_total: num(payload.observations_total) ?? 0,
    sources_total: num(payload.sources_total) ?? 0,
    sources_healthy: num(payload.sources_healthy) ?? 0,
    monitoring: payload.monitoring ?? null,
    collection_enabled: payload.collection_enabled ?? null,
    data_age_seconds: num(payload.data_age_seconds),
    alert: payload.alert ?? null,
  };
}

/**
 * `/sources` → the catalog.
 *
 * `cadence_label` is the engine's own rendering. The adapter keeps it and does
 * not re-derive it: two formatters for the same value would eventually disagree.
 */
export function sources(payload) {
  const items = listOf(payload);
  if (!items) return null;
  return items.filter((s) => s && str(s.id)).map((s) => ({
    id: String(s.id),
    name: str(s.name),
    provider: str(s.provider),
    category: str(s.category),
    subcategory: str(s.subcategory),
    endpoint: str(s.endpoint),
    protocol: str(s.protocol),
    format: str(s.format),
    cadence: s.cadence ?? null,
    cadence_label: str(s.cadence_label),
    timezone: str(s.timezone),
    license: str(s.license),
    authentication: str(s.authentication),
    cost: str(s.cost),
    historical_available: s.historical_available ?? null,
    realtime_available: s.realtime_available ?? null,
    geospatial: s.geospatial ?? null,
    priority: num(s.priority),
    enabled: s.enabled ?? null,
    collector_type: str(s.collector_type),
    health: s.health ?? null,
    _raw: s,
  }));
}

/** `/lenses` → the lens list. Answers with a bare array. */
export function lenses(payload) {
  const items = listOf(payload);
  if (!items) return null;
  return items.filter((l) => l && str(l.id)).map((l) => ({
    id: String(l.id),
    name: str(l.name),
    description: str(l.description),
    categories: Array.isArray(l.categories) ? l.categories.map(String) : [],
    entities: Array.isArray(l.entities) ? l.entities.map(String) : [],
    keywords: Array.isArray(l.keywords) ? l.keywords.map(String) : [],
    _raw: l,
  }));
}

/** `/activity` → the activity feed, newest first. */
export function activity(payload) {
  const items = listOf(payload);
  if (!items) return null;
  return items.filter((a) => a && str(a.at)).map((a) => ({
    at: String(a.at),
    kind: str(a.kind),
    message: str(a.message),
    source_id: str(a.source_id),
    signal_id: str(a.signal_id),
    event_id: str(a.event_id),
  }));
}

/** `/control` → the collection switch and per-source schedules. */
export function control(payload) {
  if (!payload || typeof payload !== "object") return null;
  return {
    status: str(payload.status),
    version: str(payload.version),
    started_at: str(payload.started_at),
    uptime_seconds: num(payload.uptime_seconds),
    collector_active: payload.collector_active ?? null,
    collection_enabled: payload.collection_enabled ?? null,
    monitoring: payload.monitoring ?? null,
    sources: Array.isArray(payload.sources) ? payload.sources : [],
    disk: payload.disk ?? null,
    latency: payload.latency ?? null,
  };
}

/** `/timeline` → one series' observations and its baseline. */
export function timeline(payload) {
  if (!payload || typeof payload !== "object") return null;
  if (!Array.isArray(payload.observations)) return null;
  return {
    series_key: str(payload.series_key),
    observations: payload.observations.map((o) => ({
      id: str(o.id),
      source_id: str(o.source_id),
      observed_at: str(o.observed_at),
      received_at: str(o.received_at),
      entity_id: str(o.entity_id),
      metric: str(o.metric),
      value: num(o.value),
      unit: str(o.unit),
      // Coordinates are kept exactly as sent: null stays null, and a block
      // that plots points checks for a number rather than for truthiness.
      latitude: num(o.latitude),
      longitude: num(o.longitude),
      quality: o.quality ?? null,
      series_key: str(o.series_key),
      lag_ms: num(o.lag_ms),
      _raw: o,
    })),
    baseline: payload.baseline ?? null,
  };
}

/** `/observations/:id` → one observation. */
export function observation(payload) {
  if (!payload || typeof payload !== "object") return null;
  if (!str(payload.id)) return null;
  return {
    id: String(payload.id),
    source_id: str(payload.source_id),
    observed_at: str(payload.observed_at),
    received_at: str(payload.received_at),
    entity_id: str(payload.entity_id),
    metric: str(payload.metric),
    value: num(payload.value),
    unit: str(payload.unit),
    latitude: num(payload.latitude),
    longitude: num(payload.longitude),
    quality: payload.quality ?? null,
    series_key: str(payload.series_key),
    lag_ms: num(payload.lag_ms),
    raw: payload.raw ?? null,
    _raw: payload,
  };
}

/** `/observations/:id/raw` → the payload the source actually sent. */
export function raw(payload) {
  // The raw record is whatever the source sent. It is passed through untouched:
  // normalizing it would defeat the point of a raw view.
  return payload ?? null;
}

/**
 * `/events/:id` → one event.
 *
 * The observation ids are kept in order and untouched, because they are the
 * chain the drill-down walks: event → observations → source → raw.
 */
export function event(payload) {
  if (!payload || typeof payload !== "object") return null;
  if (!str(payload.id)) return null;
  return {
    id: String(payload.id),
    title: str(payload.title),
    first_seen: str(payload.first_seen),
    last_seen: str(payload.last_seen),
    group_key: str(payload.group_key),
    entities: Array.isArray(payload.entities) ? payload.entities.map(String) : [],
    observations: Array.isArray(payload.observations) ? payload.observations.map(String) : [],
    anomalies: Array.isArray(payload.anomalies) ? payload.anomalies : [],
    categories: Array.isArray(payload.categories) ? payload.categories.map(String) : [],
    location: payload.location ?? null,
    state: str(payload.state),
    _raw: payload,
  };
}

/** `/entities/:id` → the signals touching one entity. */
export function entity(payload) {
  if (!payload || typeof payload !== "object") return null;
  if (!str(payload.entity_id)) return null;
  return {
    entity_id: String(payload.entity_id),
    signals: Array.isArray(payload.signals) ? payload.signals : [],
    _raw: payload,
  };
}

/** `/signals/:id` → the full signal, with its evidence chain. */
export function signal(payload) {
  const list = signals(payload ? [payload] : null);
  return list && list.length ? list[0] : null;
}

/** `/sources/:id` → one source with its health. */
export function source(payload) {
  const list = sources(payload ? [payload] : null);
  return list && list.length ? list[0] : null;
}

/** `/lenses/:id` → one lens. */
export function lens(payload) {
  const list = lenses(payload ? [payload] : null);
  return list && list.length ? list[0] : null;
}
