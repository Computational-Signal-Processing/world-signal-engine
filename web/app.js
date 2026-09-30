/* World Signal Engine — web UI.
 *
 * No build step, no framework. The routes mirror the brief's drill-down:
 *
 *   #/world              active signals (the product's front page)
 *   #/signal/:id         signal detail + why + quality + evidence
 *   #/event/:id          the event's development
 *   #/observation/:id    one observation, its source and its raw data
 *   #/source/:id         source metadata and health
 *   #/sources            every source and its health
 *   #/lenses             the configured lenses and what they show
 *   #/timeline/:series   NORMAL ──╮ ╰──● NOW
 *   #/map                signal/event level geography only
 *   #/system             live control: collection, sources, activity stream
 *
 * Everything is inserted as text nodes, never as HTML, so a hostile source
 * payload cannot inject markup. The page runs against the same origin as the
 * API; the only configuration is the optional API key.
 */

const view = document.getElementById("view");
const metricsEl = document.getElementById("metrics");
const lensSelect = document.getElementById("lens-select");
const lensField = document.getElementById("lens-field");
const keyInput = document.getElementById("api-key");
const keyField = document.getElementById("key-field");
const connEl = document.getElementById("conn");
const connText = document.getElementById("conn-text");
const toastEl = document.getElementById("toast");

/* --------------------------------------------------------------- secrets */

/* The API key, if the deployment requires one.
 *
 * Kept in localStorage rather than the URL: a key in the query string ends up
 * in browser history, in server logs, and in any Referer the page sends. It is
 * sent as a header on every request below. */
const KEY_STORAGE = "wse-api-key";

function apiKey() {
  try { return localStorage.getItem(KEY_STORAGE) || ""; } catch (_) { return ""; }
}

function setApiKey(value) {
  try {
    if (value) localStorage.setItem(KEY_STORAGE, value);
    else localStorage.removeItem(KEY_STORAGE);
  } catch (_) { /* private mode: the key simply does not persist */ }
}

/* Reveal the key field on demand.
 *
 * A loopback demo runs with no key; an empty credential box on every visit
 * would imply the API needs one. It appears when a key is already stored, or
 * when a request comes back 401 — i.e. exactly when it is needed. */
function revealKeyField() {
  if (keyField) keyField.hidden = false;
}

/* ------------------------------------------------------------------ util */

function el(tag, props = {}, children = []) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(props)) {
    if (key === "class") node.className = value;
    else if (key === "text") node.textContent = value;
    else if (key.startsWith("on")) node.addEventListener(key.slice(2), value);
    else if (value !== undefined && value !== null) node.setAttribute(key, value);
  }
  for (const child of [].concat(children)) {
    if (child === null || child === undefined) continue;
    node.append(typeof child === "string" ? document.createTextNode(child) : child);
  }
  return node;
}

function svgEl(tag, props = {}) {
  const node = document.createElementNS("http://www.w3.org/2000/svg", tag);
  for (const [key, value] of Object.entries(props)) {
    if (value !== undefined && value !== null) node.setAttribute(key, value);
  }
  return node;
}

function fmtTime(iso) {
  if (!iso) return "—";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "—";
  return d.toISOString().replace("T", " ").slice(0, 16) + "Z";
}

function fmtClock(iso) {
  if (!iso) return "—";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "—";
  return d.toISOString().slice(11, 19);
}

function fmtDuration(seconds) {
  const s = Math.max(0, Math.round(seconds || 0));
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m`;
  return `${Math.floor(s / 86400)}d ${Math.floor((s % 86400) / 3600)}h`;
}

function fmtLag(ms) {
  if (ms === null || ms === undefined) return "—";
  const abs = Math.abs(ms);
  if (abs < 1000) return `${Math.round(ms)} ms`;
  if (abs < 60_000) return `${(ms / 1000).toFixed(1)} s`;
  if (abs < 3_600_000) return `${(ms / 60_000).toFixed(1)} min`;
  return `${(ms / 3_600_000).toFixed(1)} h`;
}

function fmtBytes(n) {
  if (n === null || n === undefined) return "—";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let v = n, i = 0;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  return `${v.toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
}

/** Turn a series key like `src_usgs::-::magnitude::richter` into a label. */
function seriesLabel(key) {
  if (!key) return "—";
  const parts = String(key).split("::");
  return parts.length >= 3 ? `${parts[1]} · ${parts[2]}` : key;
}

function relative(iso) {
  if (!iso) return "—";
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return "—";
  const secs = Math.max(0, Math.round((Date.now() - then) / 1000));
  return `${fmtDuration(secs)} ago`;
}

function toast(message, tone = "info") {
  if (!toastEl) return;
  toastEl.textContent = message;
  toastEl.dataset.tone = tone;
  toastEl.hidden = false;
  clearTimeout(toast._t);
  toast._t = setTimeout(() => { toastEl.hidden = true; }, 3600);
}

/* ------------------------------------------------------------------- API */

async function api(path, options = {}) {
  const headers = { accept: "application/json" };
  const key = apiKey();
  if (key) headers["Authorization"] = `Bearer ${key}`;
  if (options.body) headers["Content-Type"] = "application/json";
  const response = await fetch(path, { ...options, headers });
  if (!response.ok) {
    if (response.status === 401) {
      revealKeyField();
      throw new Error("This deployment requires an API key. Enter it in the KEY field above.");
    }
    let detail = response.statusText;
    try {
      const body = await response.json();
      if (body && body.error) detail = body.error;
    } catch (_) { /* non-JSON error body */ }
    throw new Error(detail || `HTTP ${response.status}`);
  }
  const type = response.headers.get("content-type") || "";
  return type.includes("json") ? response.json() : response.text();
}

function post(path, body) {
  return api(path, { method: "POST", body: JSON.stringify(body ?? {}) });
}

/* --------------------------------------------------------------- lenses -- */

let lensCache = null;

async function loadLenses() {
  if (lensCache) return lensCache;
  try {
    lensCache = await api("/lenses");
  } catch (_) {
    lensCache = [];
  }
  populateLensPicker(lensCache);
  return lensCache;
}

function populateLensPicker(lenses) {
  if (!lensSelect) return;
  const current = activeLens();
  lensSelect.replaceChildren(
    el("option", { value: "", text: "all" }),
    ...lenses.map((l) => el("option", { value: l.id, text: l.name }))
  );
  lensSelect.value = current;
}

function activeLens() {
  return new URLSearchParams(window.location.search).get("lens") || "";
}

/** A hash route carrying the active lens, so drill-down does not lose the view. */
function withLens(hash, lens = activeLens()) {
  return lens ? `${hash}${hash.includes("?") ? "&" : "?"}lens=${encodeURIComponent(lens)}` : hash;
}

function lensName(id, lenses) {
  return (lenses || lensCache || []).find((l) => l.id === id)?.name || id;
}

/* --------------------------------------------------------------- signals -- */

const TYPE_GLYPH = {
  NOW: "⚡",
  ANOMALY: "◇",
  EARLY_SIGNAL: "◎",
  CONVERGENCE: "🔗",
  IMPACT: "◆",
};

/** Type is carried by icon + label + shape, never by colour alone. */
function typeBadge(type) {
  return el("span", { class: "type-badge", "data-type": type }, [
    el("span", { class: "glyph", "aria-hidden": "true", text: TYPE_GLYPH[type] || "•" }),
    type.replace("_", " "),
  ]);
}

/** The "primary" type drives the card's shape accent; it is not a ranking. */
function primaryType(types) {
  for (const candidate of ["ANOMALY", "CONVERGENCE", "EARLY_SIGNAL", "IMPACT", "NOW"]) {
    if ((types || []).includes(candidate)) return candidate;
  }
  return (types && types[0]) || "NOW";
}

function healthBadge(status) {
  return el("span", { class: "health", "data-status": status || "Unknown" }, [
    el("span", { class: "dot", "aria-hidden": "true" }),
    String(status || "Unknown").replace("_", " "),
  ]);
}

/**
 * A sparkline of the signal's series around its baseline.
 *
 * The point is the brief's `NORMAL ──╮ ╰──● NOW`: the reader must see where
 * normal sat and where the change happened, not just a number.
 */
function sparkline(observations, baseline) {
  const width = 800, height = 160, padding = 14;
  if (!observations || observations.length < 2) return null;
  const values = observations.map((o) => o.value);
  const bounds = values.slice();
  if (baseline) bounds.push(baseline.mean, baseline.p05, baseline.p95);
  let min = Math.min(...bounds), max = Math.max(...bounds);
  if (min === max) { min -= 1; max += 1; }
  const span = max - min;

  const x = (i) => padding + (i / Math.max(1, observations.length - 1)) * (width - padding * 2);
  const y = (v) => height - padding - ((v - min) / span) * (height - padding * 2);

  const svg = svgEl("svg", { viewBox: `0 0 ${width} ${height}`, class: "spark" });
  if (baseline) {
    for (const value of [baseline.p95, baseline.mean, baseline.p05]) {
      svg.append(svgEl("line", {
        x1: padding, x2: width - padding, y1: y(value), y2: y(value), class: "base",
      }));
    }
  }
  const points = observations.map((o, i) => `${x(i)},${y(o.value)}`).join(" ");
  svg.append(svgEl("polygon", {
    class: "area",
    points: `${padding},${height - padding} ${points} ${width - padding},${height - padding}`,
  }));
  svg.append(svgEl("polyline", { class: "line", points }));
  const last = observations.length - 1;
  svg.append(svgEl("circle", {
    cx: x(last), cy: y(observations[last].value), r: 4, fill: "currentColor",
  }));
  return svg;
}

function qualityBars(quality) {
  const rows = [
    ["novelty", quality.novelty],
    ["strength", quality.strength],
    ["persist", quality.persistence],
    ["confid", quality.confidence],
    ["breadth", quality.breadth],
    ["converg", quality.convergence],
    ["relevance", quality.relevance],
  ];
  return el("div", { class: "quality" }, rows.map(([label, value]) =>
    el("div", { class: "qbar" }, [
      el("span", { class: "q-label", text: label }),
      el("div", { class: "q-track" }, [
        el("div", { class: "q-fill", style: `width:${Math.round((value || 0) * 100)}%` }),
      ]),
    ])
  ));
}

/**
 * The facts a signal states about itself. Deliberately not an "importance"
 * number: the reader is told the deviation, the persistence and the number of
 * independent sources, and judges for themselves.
 */
function signalFacts(signal) {
  const dev = signal.evidence.find((e) => e.deviation_sigma !== null && e.deviation_sigma !== undefined);
  const facts = [];
  if (dev) {
    const sigma = dev.deviation_sigma;
    facts.push(["deviation", `${sigma >= 0 ? "+" : ""}${sigma.toFixed(1)}σ`]);
  }
  facts.push(["persistence", fmtDuration(signal.duration_seconds)]);
  const sources = new Set(signal.evidence.map((e) => e.source_id));
  facts.push(["sources", String(sources.size)]);
  facts.push(["evidence", String(signal.evidence.length)]);
  return el("div", { class: "sig-facts" }, facts.map(([k, v]) =>
    el("span", { class: "fact" }, [el("b", { text: k }), el("span", { text: v })])
  ));
}

/* ----------------------------------------------------------------- pages */

function render(active, title, subtitle, nodes) {
  document.querySelectorAll("[data-nav]").forEach((a) => {
    a.classList.toggle("active", a.dataset.nav === active);
  });
  if (lensSelect) lensSelect.value = activeLens();
  const head = el("div", { class: "page-head" }, [
    el("h1", { class: "page-title", text: title }),
    subtitle ? el("p", { class: "page-sub", text: subtitle }) : null,
  ]);
  view.replaceChildren(head, ...[].concat(nodes).filter(Boolean));
}

function crumbs(parts) {
  const node = el("div", { class: "crumb" });
  parts.forEach((part, i) => {
    if (i > 0) node.append(el("span", { class: "sep", text: "→" }));
    if (part.href) node.append(el("a", { href: part.href, text: part.label }));
    else node.append(el("span", { text: part.label }));
  });
  return node;
}

function errorView(err) {
  render("world", "Unavailable", null,
    el("div", { class: "empty" }, [
      el("h3", { text: "Could not reach the engine" }),
      el("p", { text: err.message }),
      el("div", { class: "link-row", style: "justify-content:center" }, [
        el("button", { class: "btn primary", text: "Retry", onclick: () => route() }),
      ]),
    ])
  );
}

/* ---------------------------------------------------------------- WORLD -- */

async function worldView() {
  const lens = activeLens();
  const query = lens ? `&lens=${encodeURIComponent(lens)}` : "";
  const page = await api(`/signals?limit=100${query}`);
  const signals = page.items || [];
  const lenses = await loadLenses();

  const subtitle = lens
    ? `Active signals visible through the ${lensName(lens, lenses)} lens. A lens changes what is shown, never what is detected.`
    : "Every active signal the engine is currently surfacing. Each one explains what changed, how far it deviated, and for how long.";

  if (signals.length === 0) {
    render("world", "World", subtitle,
      el("div", { class: "empty" }, [
        el("h3", { text: "Nothing is changing beyond normal" }),
        el("p", { text: lens
          ? `No signals through ${lensName(lens, lenses)} right now.`
          : "No signals right now. The engine is observing; when something departs from normal, it will appear here." }),
        el("div", { class: "link-row", style: "justify-content:center" }, [
          el("a", { class: "btn", href: "#/system", text: "Open system status" }),
        ]),
      ])
    );
    return;
  }

  const cards = signals.map((signal) =>
    el("article", {
      class: "sig",
      "data-primary": primaryType(signal.types),
      onclick: () => { window.location.hash = withLens(`#/signal/${signal.id}`); },
    }, [
      el("div", { class: "sig-head" }, [
        el("div", { class: "sig-types" }, (signal.types || []).map(typeBadge)),
        el("span", { class: "fact" }, [
          el("b", { text: "last update" }),
          el("span", { text: fmtClock(signal.last_updated) }),
        ]),
      ]),
      el("h2", { class: "sig-title", text: signal.title }),
      el("p", { class: "sig-summary", text: signal.summary }),
      signalFacts(signal),
      signal.series_key
        ? el("a", {
            class: "btn",
            href: withLens(`#/timeline/${encodeURIComponent(signal.series_key)}`),
            text: "Timeline",
            onclick: (e) => e.stopPropagation(),
          })
        : null,
    ])
  );

  render("world", "World", subtitle, el("div", { class: "feed" }, cards));
}

/* --------------------------------------------------------------- SIGNAL -- */

async function signalView(id) {
  const signal = await api(`/signals/${encodeURIComponent(id)}`);

  const evidence = el("div", { class: "evidence" },
    (signal.evidence || []).map((item) =>
      el("div", { class: "ev" }, [
        el("div", { class: "ev-top" }, [
          el("span", { class: "ev-metric", text: `${item.metric} = ${item.value} ${item.unit}` }),
          item.deviation_sigma !== null && item.deviation_sigma !== undefined
            ? el("span", { class: "ev-sigma", text: `${item.deviation_sigma >= 0 ? "+" : ""}${item.deviation_sigma.toFixed(2)}σ` })
            : null,
        ]),
        el("div", { class: "ev-statement", text: item.statement }),
        el("div", { class: "ev-links" }, [
          el("a", { href: `#/observation/${item.observation_id}`, text: "observation" }),
          el("a", { href: `#/source/${item.source_id}`, text: "source" }),
          el("a", { href: `/observations/${encodeURIComponent(item.observation_id)}/raw`, target: "_blank", rel: "noreferrer", text: "raw data" }),
          el("span", { text: fmtTime(item.observed_at) }),
        ]),
      ])
    )
  );

  render("world", signal.title, signal.summary, [
    crumbs([
      { label: "WORLD", href: withLens("#/world") },
      { label: `SIGNAL ${signal.id.slice(0, 10)}…` },
      { label: `EVENT ${signal.event_id.slice(0, 10)}…`, href: `#/event/${signal.event_id}` },
    ]),
    el("div", { class: "sig-types", style: "margin-bottom:14px" }, (signal.types || []).map(typeBadge)),
    el("div", { class: "panel" }, [
      el("h2", { class: "panel-title", text: "Why this signal exists" }),
      el("ul", { class: "reasons" }, (signal.reasons || []).map((r) => el("li", { text: r }))),
    ]),
    el("div", { class: "panel" }, [
      el("h2", { class: "panel-title", text: `Evidence (${(signal.evidence || []).length}) — each traces to an observation` }),
      evidence,
    ]),
    signal.series_key
      ? el("div", { class: "panel" }, [
          el("h2", { class: "panel-title", text: "Timeline" }),
          el("div", { class: "link-row" }, [
            el("a", { class: "btn primary", href: withLens(`#/timeline/${encodeURIComponent(signal.series_key)}`), text: "NORMAL ──╮ ╰──● NOW" }),
          ]),
        ])
      : null,
    el("div", { class: "panel" }, [
      el("h2", { class: "panel-title", text: "Signal" }),
      el("dl", { class: "kv" }, [
        el("dt", { text: "first seen" }), el("dd", { text: fmtTime(signal.first_seen) }),
        el("dt", { text: "last update" }), el("dd", { text: fmtTime(signal.last_updated) }),
        el("dt", { text: "duration" }), el("dd", { text: fmtDuration(signal.duration_seconds) }),
        el("dt", { text: "direction" }), el("dd", { text: signal.direction }),
        el("dt", { text: "series" }), el("dd", { class: "mono", text: signal.series_key || "—" }),
        el("dt", { text: "entities" }), el("dd", { text: (signal.entities || []).join(", ") || "—" }),
        el("dt", { text: "categories" }), el("dd", { text: (signal.categories || []).join(", ") || "—" }),
        el("dt", { text: "lenses" }), el("dd", { text: (signal.lens_matches || []).join(", ") || "—" }),
      ]),
    ]),
    el("div", { class: "panel" }, [
      el("h2", { class: "panel-title", text: "Quality (seven separate dimensions, not one score)" }),
      qualityBars(signal.quality),
    ]),
  ]);
}

/* ---------------------------------------------------------------- EVENT -- */

async function eventView(id) {
  const event = await api(`/events/${encodeURIComponent(id)}`);
  const links = el("div", { class: "evidence" },
    (event.observations || []).map((obsId) =>
      el("div", { class: "ev" }, [
        el("a", { class: "ev-metric", href: `#/observation/${obsId}`, text: obsId }),
        el("span", { class: "ev-statement", text: "observation · source · raw data" }),
      ])
    )
  );

  render("world", event.title, `An event groups the observations that represent one thing happening.`, [
    crumbs([
      { label: "WORLD", href: withLens("#/world") },
      { label: `EVENT ${event.id.slice(0, 10)}…` },
    ]),
    el("div", { class: "panel" }, [
      el("h2", { class: "panel-title", text: "Event" }),
      el("dl", { class: "kv" }, [
        el("dt", { text: "state" }), el("dd", { text: event.state }),
        el("dt", { text: "direction" }), el("dd", { text: event.direction }),
        el("dt", { text: "first seen" }), el("dd", { text: fmtTime(event.first_seen) }),
        el("dt", { text: "last seen" }), el("dd", { text: fmtTime(event.last_seen) }),
        el("dt", { text: "sources" }), el("dd", { text: String(event.source_count) }),
        el("dt", { text: "entities" }), el("dd", { text: (event.entities || []).join(", ") || "—" }),
        el("dt", { text: "categories" }), el("dd", { text: (event.categories || []).join(", ") || "—" }),
        el("dt", { text: "group key" }), el("dd", { class: "mono", text: event.group_key || "—" }),
      ]),
    ]),
    el("div", { class: "panel" }, [
      el("h2", { class: "panel-title", text: `Development (${(event.observations || []).length} observations)` }),
      links,
    ]),
  ]);
}

/* ---------------------------------------------------------- OBSERVATION -- */

async function observationView(id) {
  const observation = await api(`/observations/${encodeURIComponent(id)}`);
  const source = await api(`/sources/${encodeURIComponent(observation.source_id)}`).catch(() => null);
  const raw = observation.raw || {};
  const rawHref = `/observations/${encodeURIComponent(id)}/raw`;

  render("world", `Observation ${observation.id.slice(0, 14)}…`, "One measurement, its source, and the exact bytes the source returned.", [
    crumbs([
      { label: "WORLD", href: withLens("#/world") },
      { label: "OBSERVATION" },
      { label: "SOURCE", href: `#/source/${observation.source_id}` },
      { label: "RAW DATA" },
    ]),
    el("div", { class: "panel" }, [
      el("h2", { class: "panel-title", text: "Measurement" }),
      el("dl", { class: "kv" }, [
        el("dt", { text: "metric" }), el("dd", { text: observation.metric }),
        el("dt", { text: "value" }), el("dd", { text: `${observation.value} ${observation.unit}` }),
        el("dt", { text: "observed at" }), el("dd", { text: fmtTime(observation.observed_at) }),
        el("dt", { text: "received at" }), el("dd", { text: fmtTime(observation.received_at) }),
        el("dt", { text: "lag" }), el("dd", { text: fmtLag(observation.lag_ms) }),
        el("dt", { text: "quality" }), el("dd", { text: observation.quality?.score?.toFixed?.(2) ?? "—" }),
        el("dt", { text: "flags" }), el("dd", { text: (observation.quality?.flags || []).join(", ") || "none" }),
        el("dt", { text: "entity" }), el("dd", { text: observation.entity_id || "—" }),
        el("dt", { text: "series" }), el("dd", { class: "mono", text: observation.series_key || "—" }),
      ]),
    ]),
    el("div", { class: "panel" }, [
      el("h2", { class: "panel-title", text: "Raw data — exactly as the source returned it" }),
      el("dl", { class: "kv" }, [
        el("dt", { text: "locator" }), el("dd", { class: "mono", text: raw.locator || "—" }),
        el("dt", { text: "hash" }), el("dd", { class: "mono", text: raw.hash || "—" }),
        el("dt", { text: "content type" }), el("dd", { text: raw.content_type || "—" }),
        el("dt", { text: "bytes" }), el("dd", { text: raw.bytes ?? "—" }),
      ]),
      el("div", { class: "link-row" }, [
        el("a", { class: "btn primary", href: rawHref, target: "_blank", rel: "noreferrer", text: "Open raw payload" }),
      ]),
    ]),
    source
      ? el("div", { class: "panel" }, [
          el("h2", { class: "panel-title", text: "Source" }),
          el("dl", { class: "kv" }, [
            el("dt", { text: "name" }), el("dd", { text: source.name }),
            el("dt", { text: "provider" }), el("dd", { text: source.provider }),
            el("dt", { text: "category" }), el("dd", { text: source.category }),
            el("dt", { text: "cadence" }), el("dd", { text: source.cadence_label || "—" }),
            el("dt", { text: "license" }), el("dd", { text: source.license || "—" }),
            el("dt", { text: "health" }), el("dd", {}, [source.health ? healthBadge(source.health.status) : el("span", { text: "never run" })]),
          ]),
          el("div", { class: "link-row" }, [
            el("a", { class: "btn", href: `#/source/${source.id}`, text: "Source detail" }),
          ]),
        ])
      : null,
  ]);
}

/* --------------------------------------------------------------- SOURCE -- */

function healthRows(health) {
  if (!health) return [el("dt", { text: "health" }), el("dd", { text: "never run" })];
  return [
    el("dt", { text: "status" }), el("dd", {}, [healthBadge(health.status)]),
    el("dt", { text: "last success" }), el("dd", { text: fmtTime(health.last_success) }),
    el("dt", { text: "last failure" }), el("dd", { text: fmtTime(health.last_failure) }),
    el("dt", { text: "last error" }), el("dd", { text: health.last_error || "—" }),
    el("dt", { text: "latency" }), el("dd", { text: `${health.last_latency_ms ?? "—"} ms` }),
    el("dt", { text: "records received" }), el("dd", { text: String(health.records_received ?? 0) }),
    el("dt", { text: "records changed" }), el("dd", { text: String(health.records_changed ?? 0) }),
    el("dt", { text: "records duplicate" }), el("dd", { text: String(health.records_duplicate ?? 0) }),
    el("dt", { text: "errors" }), el("dd", { text: String(health.error_count ?? 0) }),
    el("dt", { text: "rate limited" }), el("dd", { text: String(health.rate_limit_count ?? 0) }),
  ];
}

async function sourceView(id) {
  const source = await api(`/sources/${encodeURIComponent(id)}`);

  render("sources", source.name, `${source.category} · ${source.provider}`, [
    crumbs([
      { label: "WORLD", href: withLens("#/world") },
      { label: "SOURCES", href: "#/sources" },
      { label: source.id },
    ]),
    el("div", { class: "panel" }, [
      el("h2", { class: "panel-title", text: "Health" }),
      el("p", { class: "page-sub", text: "A collector failure is recorded here. It never means \"world activity = 0\"." }),
      el("dl", { class: "kv" }, healthRows(source.health)),
    ]),
    el("div", { class: "panel" }, [
      el("h2", { class: "panel-title", text: "Catalog" }),
      el("dl", { class: "kv" }, [
        el("dt", { text: "id" }), el("dd", { class: "mono", text: source.id }),
        el("dt", { text: "provider" }), el("dd", { text: source.provider }),
        el("dt", { text: "category" }), el("dd", { text: source.category }),
        el("dt", { text: "subcategory" }), el("dd", { text: source.subcategory || "—" }),
        el("dt", { text: "endpoint" }), el("dd", { class: "mono", text: source.endpoint || "—" }),
        el("dt", { text: "protocol" }), el("dd", { text: source.protocol }),
        el("dt", { text: "format" }), el("dd", { text: source.format }),
        el("dt", { text: "cadence" }), el("dd", { text: source.cadence_label || "—" }),
        el("dt", { text: "license" }), el("dd", { text: source.license || "—" }),
        el("dt", { text: "authentication" }), el("dd", { text: source.authentication || "—" }),
        el("dt", { text: "enabled" }), el("dd", { text: source.enabled ? "yes" : "no" }),
      ]),
    ]),
  ]);
}

async function sourcesIndex() {
  const sources = await api("/sources");
  const rows = sources.map((source) =>
    el("tr", {}, [
      el("td", {}, [el("a", { class: "row-link", href: `#/source/${source.id}`, text: source.name })]),
      el("td", { text: source.category }),
      el("td", { class: "mono", text: source.cadence_label || "—" }),
      el("td", {}, [source.health ? healthBadge(source.health.status) : el("span", { class: "health", "data-status": "Unknown" }, [el("span", { class: "dot" }), "never run"])]),
      el("td", { class: "mono", text: source.health?.last_success ? relative(source.health.last_success) : "—" }),
      el("td", { text: source.enabled ? "yes" : "no" }),
    ])
  );
  render("sources", "Sources", "Every feed the engine watches, and whether each one is currently healthy. A broken collector is visible here, never mistaken for a quiet world.", [
    el("table", { class: "table" }, [
      el("thead", {}, [el("tr", {}, [
        el("th", { text: "Source" }), el("th", { text: "Category" }), el("th", { text: "Cadence" }),
        el("th", { text: "Health" }), el("th", { text: "Last success" }), el("th", { text: "Enabled" }),
      ])]),
      el("tbody", {}, rows),
    ]),
  ]);
}

/* --------------------------------------------------------------- LENSES -- */

async function lensesView() {
  const lenses = await loadLenses();
  if (lenses.length === 0) {
    render("lenses", "Lenses", null, el("div", { class: "empty", text: "No lenses configured." }));
    return;
  }
  const describe = (lens) => {
    const parts = [];
    if (lens.categories?.length) parts.push(`categories: ${lens.categories.join(", ")}`);
    if (lens.entities?.length) parts.push(`entities: ${lens.entities.join(", ")}`);
    if (lens.keywords?.length) parts.push(`keywords: ${lens.keywords.join(", ")}`);
    if (lens.bbox) parts.push("a bounding box");
    return parts.length ? parts.join(" · ") : "everything — no filter";
  };
  const cards = lenses.map((lens) =>
    el("a", { class: "sig", href: `#/world?lens=${encodeURIComponent(lens.id)}`, "data-primary": "NOW" }, [
      el("h2", { class: "sig-title", text: lens.name }),
      el("p", { class: "sig-summary", text: describe(lens) }),
      el("div", { class: "sig-facts" }, [
        el("span", { class: "fact" }, [el("b", { text: "signals" }), el("span", { text: String(lens.matching_signals ?? 0) })]),
        el("span", { class: "fact" }, [el("b", { text: "id" }), el("span", { text: lens.id })]),
      ]),
    ])
  );
  render("lenses", "Lenses", "A lens changes what is visible, never what is detected. The dataset underneath is one.", el("div", { class: "feed" }, cards));
}

/* ------------------------------------------------------------- TIMELINE -- */

function timelineChart(observations, baseline) {
  const width = 900, height = 220, pad = 28;
  if (observations.length === 0) return el("div", { class: "empty", text: "No points." });
  const values = observations.map((o) => o.value);
  const bounds = values.slice();
  if (baseline) bounds.push(baseline.mean, baseline.p05, baseline.p95);
  let min = Math.min(...bounds), max = Math.max(...bounds);
  if (min === max) { min -= 1; max += 1; }
  const span = max - min;
  const x = (i) => pad + (i / Math.max(1, observations.length - 1)) * (width - pad * 2);
  const y = (v) => height - pad - ((v - min) / span) * (height - pad * 2);

  const svg = svgEl("svg", { viewBox: `0 0 ${width} ${height}`, class: "timeline-svg" });
  svg.append(svgEl("line", { x1: pad, x2: width - pad, y1: height - pad, y2: height - pad, class: "axis" }));
  svg.append(svgEl("line", { x1: pad, x2: pad, y1: pad, y2: height - pad, class: "axis" }));
  if (baseline) {
    for (const [value, label] of [[baseline.p95, "p95"], [baseline.mean, "mean"], [baseline.p05, "p05"]]) {
      svg.append(svgEl("line", { x1: pad, x2: width - pad, y1: y(value), y2: y(value), class: "grid" }));
      const text = svgEl("text", { x: width - pad + 2, y: y(value) + 3 });
      text.textContent = label;
      svg.append(text);
    }
  }
  svg.append(svgEl("polyline", { class: "series", points: observations.map((o, i) => `${x(i)},${y(o.value)}`).join(" ") }));
  const last = observations.length - 1;
  svg.append(svgEl("circle", { cx: x(last), cy: y(observations[last].value), r: 4, class: "point" }));
  return svg;
}

async function timelineView(seriesKey) {
  const data = await api(`/timeline?series=${encodeURIComponent(seriesKey)}&limit=400`);
  const observations = (data.observations || []).slice().reverse();
  render("world", `Timeline · ${seriesLabel(seriesKey)}`, "Normal behaviour, where it sat, and where the change happened.", [
    crumbs([
      { label: "WORLD", href: withLens("#/world") },
      { label: "TIMELINE" },
      { label: seriesKey },
    ]),
    el("div", { class: "panel" }, [
      el("h2", { class: "panel-title", text: "NORMAL ──╮ ╰──● NOW" }),
      timelineChart(observations, data.baseline),
      data.baseline
        ? el("dl", { class: "kv", style: "margin-top:14px" }, [
            el("dt", { text: "baseline n" }), el("dd", { text: String(data.baseline.sample_size) }),
            el("dt", { text: "mean" }), el("dd", { text: data.baseline.mean.toFixed(3) }),
            el("dt", { text: "median" }), el("dd", { text: data.baseline.median.toFixed(3) }),
            el("dt", { text: "mad" }), el("dd", { text: data.baseline.mad.toFixed(3) }),
            el("dt", { text: "p05 / p95" }), el("dd", { text: `${data.baseline.p05.toFixed(2)} / ${data.baseline.p95.toFixed(2)}` }),
          ])
        : el("p", { class: "page-sub", text: "No baseline yet — the series has not accumulated enough history." }),
    ]),
  ]);
}

/* ------------------------------------------------------------------ MAP -- */

function project(lat, lon, width, height) {
  return [((lon + 180) / 360) * width, ((90 - lat) / 180) * height];
}

async function mapView() {
  const page = await api("/signals?limit=200");
  const located = (page.items || []).filter((s) => s.location);
  const width = 720, height = 360;
  const svg = svgEl("svg", { viewBox: `0 0 ${width} ${height}`, class: "map-wrap", style: "display:block;width:100%" });
  svg.append(svgEl("rect", { x: 0, y: 0, width, height, fill: "var(--bg-sunken)" }));
  for (let lon = -180; lon <= 180; lon += 30) {
    const [px] = project(0, lon, width, height);
    svg.append(svgEl("line", { x1: px, x2: px, y1: 0, y2: height, stroke: "var(--line)", "stroke-width": 1 }));
  }
  for (let lat = -90; lat <= 90; lat += 30) {
    const [, py] = project(lat, 0, width, height);
    svg.append(svgEl("line", { x1: 0, x2: width, y1: py, y2: py, stroke: "var(--line)", "stroke-width": 1 }));
  }
  const color = { ANOMALY: "var(--anomaly)", EARLY_SIGNAL: "var(--early)", CONVERGENCE: "var(--convergence)", IMPACT: "var(--impact)", NOW: "var(--now)" };
  for (const signal of located) {
    const [px, py] = project(signal.location.latitude, signal.location.longitude, width, height);
    const marker = svgEl("circle", {
      cx: px, cy: py, r: 5, class: "map-dot",
      fill: color[primaryType(signal.types)] || "var(--now)",
      stroke: "var(--bg)", "stroke-width": 1.5,
    });
    marker.addEventListener("click", () => { window.location.hash = `#/signal/${signal.id}`; });
    const title = svgEl("title");
    title.textContent = `${signal.title} — ${signal.summary}`;
    marker.append(title);
    svg.append(marker);
  }
  render("map", "Map", "Signal and event level geography only. Raw observations are not scattered on the map.", [
    el("div", { class: "panel" }, [
      el("div", { class: "map-wrap" }, [svg]),
      el("div", { class: "legend" }, [
        el("span", { class: "item", text: `${located.length} located signal(s)` }),
      ]),
    ]),
  ]);
}

/* --------------------------------------------------------------- SYSTEM -- */

const ACTIVITY_ICON = {
  Started: "▶", Observation: "·", Anomaly: "◇", Event: "□", Signal: "⚡",
  SourceRecovered: "✓", SourceFailed: "✕", SourceRateLimited: "⏳", Control: "⚙",
};

let activityLog = [];
let stream = null;

function activityRow(entry) {
  return el("div", { class: "act", "data-kind": entry.kind }, [
    el("span", { class: "act-time", text: fmtClock(entry.at) }),
    el("span", { class: "act-ico", "aria-hidden": "true", text: ACTIVITY_ICON[entry.kind] || "·" }),
    el("span", { class: "act-msg", text: entry.message }),
  ]);
}

async function systemView() {
  const control = await api("/control");
  const recent = await api("/activity?limit=80");
  activityLog = recent.items || [];

  const stat = (label, value, note) =>
    el("div", { class: "stat" }, [
      el("div", { class: "stat-label", text: label }),
      el("div", { class: "stat-value", text: String(value) }),
      note ? el("div", { class: "stat-note", text: note }) : null,
    ]);

  const sourceRows = (control.sources || []).map((source) =>
    el("tr", {}, [
      el("td", {}, [el("a", { class: "row-link", href: `#/source/${source.source_id}`, text: source.name })]),
      el("td", { text: source.category }),
      el("td", { class: "mono", text: source.cadence_seconds ? fmtDuration(source.cadence_seconds) : "event" }),
      el("td", { class: "mono", text: source.last_run ? fmtClock(source.last_run) : "—" }),
      el("td", { class: "mono", text: source.next_run ? fmtClock(source.next_run) : "—" }),
      el("td", {}, [source.running ? el("span", { class: "pill on", text: "running" }) : el("span", { class: "pill", text: "idle" })]),
      el("td", {}, [el("span", { class: `pill ${source.enabled ? "on" : "off"}`, text: source.enabled ? "on" : "off" })]),
      el("td", {}, [
        el("button", {
          class: "btn", text: source.enabled ? "Disable" : "Enable",
          onclick: async (e) => {
            e.stopPropagation();
            try {
              await post(`/sources/${encodeURIComponent(source.source_id)}/enabled`, { enabled: !source.enabled });
              toast(`${source.source_id} ${source.enabled ? "disabled" : "enabled"}`);
              systemView();
            } catch (err) { toast(err.message, "error"); }
          },
        }),
        " ",
        el("button", {
          class: "btn", text: "Run now",
          onclick: async (e) => {
            e.stopPropagation();
            try {
              await post(`/sources/${encodeURIComponent(source.source_id)}/run`, {});
              toast(`run queued for ${source.source_id}`);
            } catch (err) { toast(err.message, "error"); }
          },
        }),
      ]),
    ])
  );

  const collectionToggle = el("button", {
    class: `btn ${control.collection_enabled ? "danger" : "primary"}`,
    text: control.collection_enabled ? "Pause collection" : "Resume collection",
    onclick: async () => {
      try {
        await post("/control/collection", { enabled: !control.collection_enabled });
        toast(control.collection_enabled ? "collection paused" : "collection resumed");
        systemView();
      } catch (err) { toast(err.message, "error"); }
    },
  });

  const feed = el("div", { class: "activity", id: "activity-feed" }, activityLog.map(activityRow));

  render("system", "System", "The engine's real operational state: what is running, what it is doing, and the live stream of changes.", [
    el("div", { class: "bar" }, [
      el("span", { class: "pill " + (control.collection_enabled ? "on" : "off"), text: control.collection_enabled ? "collection on" : "collection paused" }),
      collectionToggle,
      el("span", { class: "pill", text: `up ${fmtDuration(control.uptime_seconds)}` }),
      el("span", { class: "pill", text: `v${control.version}` }),
    ]),
    el("div", { class: "stat-grid" }, [
      stat("active signals", control.signals_active, `${control.signals_total} total`),
      stat("events", control.events_total),
      stat("observations", control.observations_total),
      stat("sources", (control.sources || []).length, `${(control.sources || []).filter((s) => s.enabled).length} enabled`),
      stat("disk", fmtBytes(control.disk.total_bytes), `${fmtBytes(control.disk.raw_bytes)} raw · ${control.disk.raw_files} files`),
    ]),
    el("div", { class: "panel" }, [
      el("h2", { class: "panel-title", text: "Live activity" }),
      feed,
    ]),
    el("div", { class: "panel" }, [
      el("h2", { class: "panel-title", text: "Sources" }),
      el("table", { class: "table" }, [
        el("thead", {}, [el("tr", {}, [
          el("th", { text: "Source" }), el("th", { text: "Category" }), el("th", { text: "Cadence" }),
          el("th", { text: "Last run" }), el("th", { text: "Next" }), el("th", { text: "State" }),
          el("th", { text: "Enabled" }), el("th", { text: "Control" }),
        ])]),
        el("tbody", {}, sourceRows),
      ]),
    ]),
  ]);

  // The activity feed is live only while the System screen is open.
  startActivityStream();
}

function appendActivity(entry) {
  activityLog.push(entry);
  if (activityLog.length > 200) activityLog.shift();
  const feed = document.getElementById("activity-feed");
  if (!feed) return;
  feed.prepend(activityRow(entry));
  while (feed.childElementCount > 200) feed.lastElementChild.remove();
}

/* A live stream over fetch + ReadableStream rather than EventSource.
 *
 * EventSource cannot set an Authorization header, and the key must not go in
 * the URL. Reading the SSE frames by hand keeps the key in a header while
 * still getting server push. A dropped connection is retried with a backoff;
 * the recent-activity endpoint re-syncs anything missed. */
let streamController = null;

function startActivityStream() {
  stopActivityStream();
  const controller = new AbortController();
  streamController = controller;
  runStream(controller);
}

function stopActivityStream() {
  if (streamController) { streamController.abort(); streamController = null; }
}

async function runStream(controller) {
  let backoff = 1000;
  while (!controller.signal.aborted) {
    try {
      const headers = { accept: "text/event-stream" };
      const key = apiKey();
      if (key) headers["Authorization"] = `Bearer ${key}`;
      const response = await fetch("/events", { headers, signal: controller.signal });
      if (response.status === 401) { revealKeyField(); setConn("error", "auth required"); return; }
      if (!response.ok || !response.body) throw new Error(`HTTP ${response.status}`);
      setConn("live", "live");
      backoff = 1000;
      await readStream(response.body, controller.signal);
      setConn("reconnecting", "reconnecting");
    } catch (err) {
      if (controller.signal.aborted) return;
      setConn("reconnecting", "reconnecting");
    }
    await new Promise((r) => setTimeout(r, backoff));
    backoff = Math.min(backoff * 2, 15000);
    // Re-sync in case events were missed while disconnected.
    try {
      const recent = await api("/activity?limit=80");
      if (recent.items) {
        activityLog = recent.items;
        const feed = document.getElementById("activity-feed");
        if (feed) feed.replaceChildren(...activityLog.map(activityRow));
      }
    } catch (_) { /* offline; the next attempt will retry */ }
  }
}

async function readStream(body, signal) {
  const reader = body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  while (!signal.aborted) {
    const { done, value } = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, { stream: true });
    const frames = buffer.split("\n\n");
    buffer = frames.pop();
    for (const frame of frames) {
      const entry = parseSseFrame(frame);
      if (entry) appendActivity(entry);
    }
  }
}

function parseSseFrame(frame) {
  const lines = frame.split("\n");
  let data = null;
  for (const line of lines) {
    if (line.startsWith("data:")) data = (data ?? "") + line.slice(5).trim();
  }
  if (!data) return null;
  try { return JSON.parse(data); } catch (_) { return null; }
}

function setConn(state, text) {
  if (connEl) connEl.dataset.state = state;
  if (connText) connText.textContent = text;
}

/* --------------------------------------------------------------- ROUTER -- */

async function route() {
  // A view that starts a stream must stop it when we navigate away.
  stopActivityStream();

  const hash = window.location.hash || "#/world";
  const clean = hash.split("?")[0];
  const parts = clean.replace(/^#\//, "").split("/");
  const [name, ...rest] = parts;
  const id = rest.join("/");
  try {
    switch (name) {
      case "signal": return await signalView(id);
      case "event": return await eventView(id);
      case "observation": return await observationView(id);
      case "source": return await sourceView(id);
      case "sources": return await sourcesIndex();
      case "lenses": return await lensesView();
      case "timeline": return await timelineView(decodeURIComponent(id));
      case "map": return await mapView();
      case "system": return await systemView();
      default: return await worldView();
    }
  } catch (err) {
    errorView(err);
  }
}

async function refreshMetrics() {
  try {
    const headers = {};
    const key = apiKey();
    if (key) headers["Authorization"] = `Bearer ${key}`;
    const response = await fetch("/metrics", { headers });
    if (!response.ok) { metricsEl.textContent = ""; return; }
    const text = await response.text();
    const wanted = ["wse_observations_total", "wse_anomalies_total", "wse_signals_total"];
    metricsEl.textContent = text
      .split("\n")
      .filter((l) => wanted.some((w) => l.startsWith(w + " ")))
      .map((l) => l.replace("wse_", "").replace("_total", ""))
      .join("   ");
  } catch (_) { /* metrics are decorative */ }
}

window.addEventListener("hashchange", route);
if (keyInput) {
  keyInput.value = apiKey();
  if (apiKey()) revealKeyField();
  keyInput.addEventListener("change", (event) => {
    setApiKey(event.target.value.trim());
    route();
  });
}
if (lensSelect) {
  lensSelect.addEventListener("change", (event) => {
    window.location.href = withLens(window.location.hash || "#/world", event.target.value);
  });
}
window.addEventListener("DOMContentLoaded", () => {
  route();
  refreshMetrics();
  setInterval(refreshMetrics, 5000);
});
