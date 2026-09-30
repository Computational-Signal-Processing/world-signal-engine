/* World Signal Engine — web UI.
 *
 * No build step, no framework. The routes mirror the brief's drill-down:
 *
 *   #/world              active signals
 *   #/lenses             the configured lenses and what they show
 *   #/signal/:id         signal detail + why + quality
 *   #/event/:id          the event's development
 *   #/observation/:id    one observation, its source and its raw data
 *   #/source/:id         source metadata and health
 *   #/timeline/:series   NORMAL ──╮ ╰──● NOW
 *   #/map                signal/event level geography only
 *
 * All values are inserted as text nodes, never as HTML, so a hostile source
 * payload cannot inject markup.
 */

const view = document.getElementById("view");
const statusEl = document.getElementById("status");
const metricsEl = document.getElementById("metrics");
const lensSelect = document.getElementById("lens-select");
const keyInput = document.getElementById("api-key");
const keyPicker = document.getElementById("key-picker");

/* Reveal the key field on demand.
 *
 * A loopback demo runs with no key, and showing an empty credential box on
 * every visit would imply the API needs one. It appears when a key is already
 * stored, or when a request comes back 401 — i.e. exactly when it is needed. */
function revealKeyPicker() {
  if (keyPicker) keyPicker.classList.add("visible");
}

/* The API key, if the deployment requires one.
 *
 * Kept in localStorage rather than in the URL: a key in the query string ends
 * up in browser history, in server logs, and in any Referer the page sends.
 * It is sent as a header on every request below. */
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

/** The active lens, kept in the URL so a view is linkable and survives reload. */
function activeLens() {
  return new URLSearchParams(window.location.search).get("lens") || "";
}

/** A hash route carrying the active lens, so drill-down does not lose the view. */
function withLens(hash, lens = activeLens()) {
  return lens ? `${hash}${hash.includes("?") ? "&" : "?"}lens=${encodeURIComponent(lens)}` : hash;
}

/** Keep the picker in step with the URL, which is the source of truth. */
function syncLensPicker() {
  if (lensSelect) lensSelect.value = activeLens();
}

/** Small DOM helper. */
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

async function api(path) {
  const headers = { accept: "application/json" };
  const key = apiKey();
  if (key) headers["Authorization"] = `Bearer ${key}`;
  const response = await fetch(path, { headers });
  if (!response.ok) {
    if (response.status === 401) {
      revealKeyPicker();
      throw new Error("This deployment requires an API key. Enter it in the KEY field above.");
    }
    let detail = response.statusText;
    try {
      const body = await response.json();
      if (body && body.error) detail = body.error;
    } catch (_) { /* non-JSON error body */ }
    throw new Error(detail);
  }
  return response.json();
}

function fmtTime(iso) {
  if (!iso) return "—";
  const d = new Date(iso);
  return d.toISOString().replace("T", " ").slice(0, 16) + "Z";
}

function fmtDuration(seconds) {
  const s = Math.max(0, Math.round(seconds || 0));
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m`;
  return `${Math.floor(s / 86400)}d ${Math.floor((s % 86400) / 3600)}h`;
}

/** Source-side lag in the units a reader thinks in. */
function fmtLag(ms) {
  if (ms === null || ms === undefined) return "—";
  const abs = Math.abs(ms);
  if (abs < 1000) return `${Math.round(ms)} ms`;
  if (abs < 60_000) return `${(ms / 1000).toFixed(1)} s`;
  if (abs < 3_600_000) return `${(ms / 60_000).toFixed(1)} min`;
  return `${(ms / 3_600_000).toFixed(1)} h`;
}

/**
 * A health badge. "rate limited" and "down" are deliberately different: a
 * throttled source is healthy, we are simply asking too often.
 */
function healthBadge(status) {
  return el("span", { class: "health", "data-health": status || "unknown" }, [
    el("span", { class: "dot", "aria-hidden": "true" }),
    String(status || "unknown").replace("_", " "),
  ]);
}

function typeBadge(type) {
  return el("span", { class: "type", "data-type": type }, [
    el("span", { class: "glyph", "aria-hidden": "true" }),
    type.replace("_", " "),
  ]);
}

/** The "primary" type drives the card's shape accent; it is not a ranking. */
function primaryType(types) {
  for (const candidate of ["ANOMALY", "CONVERGENCE", "EARLY_SIGNAL", "NOW"]) {
    if (types.includes(candidate)) return candidate;
  }
  return types[0] || "NOW";
}

function trail(parts) {
  const node = el("div", { class: "trail" });
  parts.forEach((part, i) => {
    if (i > 0) node.append(el("span", { class: "sep", text: "→" }));
    if (part.href) node.append(el("a", { href: part.href, text: part.label }));
    else node.append(el("span", { text: part.label }));
  });
  return node;
}

function setStatus(text, isError = false) {
  statusEl.textContent = text;
  statusEl.classList.toggle("error", isError);
}

function setNav(active) {
  document.querySelectorAll("[data-nav]").forEach((a) => {
    a.classList.toggle("active", a.dataset.nav === active);
  });
}

function render(active, title, nodes) {
  setNav(active);
  syncLensPicker();
  view.replaceChildren(el("h1", { text: title }), ...[].concat(nodes));
}

function errorView(err) {
  setStatus("error", true);
  view.replaceChildren(
    el("div", { class: "empty" }, [
      el("p", { text: `Could not reach the engine: ${err.message}` }),
      el("button", { class: "link", text: "Retry", onclick: () => route() }),
    ])
  );
}

/* --------------------------------------------------------------- LENSES -- */

let lensCache = null;

/** The configured lenses, fetched once per page load. */
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
    el("option", { value: "", text: "all lenses" }),
    ...lenses.map((l) => el("option", { value: l.id, text: l.name }))
  );
  lensSelect.value = current;
}

/**
 * The lenses a signal is visible through.
 *
 * Shown because "why am I not seeing this?" is usually answered by which lens
 * it fell under, and a signal matched by nothing is itself informative.
 */
function lensBadges(lensIds, lenses) {
  const ids = lensIds || [];
  if (ids.length === 0) return null;
  const name = (id) => lenses.find((l) => l.id === id)?.name || id;
  return el(
    "div",
    { class: "lens-badges" },
    ids.map((id) => el("span", { class: "lens-badge", text: name(id) }))
  );
}

/** The lens index: what each lens covers, and how much it currently shows. */
async function lensesView() {
  const lenses = await loadLenses();
  setStatus(`${lenses.length} lens(es)`);

  if (lenses.length === 0) {
    render(
      "lenses",
      "Lenses",
      el("div", { class: "empty", text: "No lenses configured." })
    );
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
    el("a", { class: "card", href: `#/world?lens=${encodeURIComponent(lens.id)}` }, [
      el("div", { class: "title", text: lens.name }),
      el("div", { class: "summary", text: describe(lens) }),
      el("div", { class: "meta" }, [
        el("span", { text: `${lens.matching_signals} signal(s)` }),
        el("span", { text: lens.id }),
      ]),
      lens.matching_signals === 0
        ? el("div", { class: "lens-empty", text: "no signals through this lens yet" })
        : null,
    ])
  );

  render("lenses", "Lenses", [
    el("p", {
      class: "summary",
      text: "A lens changes what is visible, never what is detected. The dataset underneath is one.",
    }),
    el("div", { class: "grid" }, cards),
  ]);
}

/* ---------------------------------------------------------------- WORLD -- */

async function worldView() {
  const lens = activeLens();
  const query = lens ? `&lens=${encodeURIComponent(lens)}` : "";
  const page = await api(`/signals?limit=100${query}`);
  const signals = page.items || [];
  const lenses = await loadLenses();
  const lensName = (id) => lenses.find((l) => l.id === id)?.name || id;

  setStatus(
    lens
      ? `${signals.length} signal(s) through ${lensName(lens)}`
      : `${signals.length} active signal(s)`
  );

  if (signals.length === 0) {
    render(
      "world",
      "World",
      el("div", { class: "empty" }, [
        el("p", {
          text: lens
            ? `No signals through ${lensName(lens)} right now.`
            : "No signals right now.",
        }),
        // An empty lens is often just a lens nothing feeds yet, which is worth
        // saying rather than leaving the reader to guess.
        lens ? el("a", { href: "#/lenses", text: "See what each lens covers →" }) : null,
      ])
    );
    return;
  }

  const cards = signals.map((signal) =>
    el("a", { class: "card", href: withLens(`#/signal/${signal.id}`), "data-primary": primaryType(signal.types) }, [
      el("div", { class: "types" }, signal.types.map(typeBadge)),
      el("div", { class: "title", text: signal.title }),
      el("div", { class: "summary", text: signal.summary }),
      lensBadges(signal.lens_matches, lenses),
      el("div", { class: "meta" }, [
        el("span", { text: `first seen ${fmtTime(signal.first_seen)}` }),
        el("span", { text: `duration ${fmtDuration(signal.duration_seconds)}` }),
        el("span", { text: `${signal.evidence.length} evidence` }),
      ]),
    ])
  );
  render("world", "World", el("div", { class: "grid" }, cards));
}

/* --------------------------------------------------------------- SIGNAL -- */

function qualityPanel(quality) {
  const rows = [
    ["novelty", quality.novelty],
    ["strength", quality.strength],
    ["persistence", quality.persistence],
    ["confidence", quality.confidence],
    ["breadth", quality.breadth],
    ["convergence", quality.convergence],
    ["relevance", quality.relevance],
  ];
  return el(
    "div",
    { class: "panel" },
    [
      el("h2", { text: "Quality" }),
      el(
        "div",
        { class: "quality" },
        rows.map(([label, value]) =>
          el("div", { class: "row" }, [
            el("span", { text: label }),
            el("div", { class: "bar" }, [
              el("span", { style: `width:${Math.round((value || 0) * 100)}%` }),
            ]),
            el("span", { text: (value || 0).toFixed(2) }),
          ])
        )
      ),
    ]
  );
}

async function signalView(id) {
  const signal = await api(`/signals/${encodeURIComponent(id)}`);
  setStatus(`signal ${id.slice(0, 12)}…`);

  const evidence = el(
    "ul",
    { class: "evidence" },
    signal.evidence.map((item) =>
      el("li", {}, [
        el("div", { text: item.statement }),
        el("code", {
          text: `${item.observation_id} · ${item.source_id} · ${fmtTime(item.observed_at)}`,
        }),
        el("div", {}, [
          el("a", {
            class: "trail",
            href: `#/observation/${item.observation_id}`,
            text: "→ observation · source · raw data",
          }),
        ]),
      ])
    )
  );

  render("world", signal.title, [
    trail([
      { label: "WORLD", href: withLens("#/world") },
      { label: `SIGNAL ${signal.id.slice(0, 10)}…` },
      { label: `EVENT ${signal.event_id.slice(0, 10)}…`, href: `#/event/${signal.event_id}` },
    ]),
    el("div", { class: "types" }, signal.types.map(typeBadge)),
    el("div", { class: "detail" }, [
      el("div", {}, [
        el("div", { class: "panel" }, [
          el("h2", { text: "Why this signal exists" }),
          el("ul", { class: "reasons" }, signal.reasons.map((r) => el("li", { text: r }))),
        ]),
        el("div", { class: "panel", style: "margin-top:1rem" }, [
          el("h2", { text: `Evidence (${signal.evidence.length})` }),
          evidence,
        ]),
        signal.series_key
          ? el("div", { class: "panel", style: "margin-top:1rem" }, [
              el("h2", { text: "Timeline" }),
              el("a", {
                class: "trail",
                href: `#/timeline/${encodeURIComponent(signal.series_key)}`,
                text: "→ NORMAL ──╮ ╰──● NOW",
              }),
            ])
          : null,
      ]),
      el("div", {}, [
        el("div", { class: "panel" }, [
          el("h2", { text: "Signal" }),
          el("dl", { class: "kv" }, [
            el("dt", { text: "first seen" }), el("dd", { text: fmtTime(signal.first_seen) }),
            el("dt", { text: "last update" }), el("dd", { text: fmtTime(signal.last_updated) }),
            el("dt", { text: "duration" }), el("dd", { text: fmtDuration(signal.duration_seconds) }),
            el("dt", { text: "direction" }), el("dd", { text: signal.direction }),
            el("dt", { text: "entities" }),
            el("dd", { text: signal.entities.join(", ") || "—" }),
            el("dt", { text: "categories" }),
            el("dd", { text: signal.categories.join(", ") || "—" }),
            el("dt", { text: "lenses" }),
            el("dd", { text: (signal.lens_matches || []).join(", ") || "—" }),
          ]),
        ]),
        el("div", { style: "margin-top:1rem" }, [qualityPanel(signal.quality)]),
      ]),
    ]),
  ]);
}

/* ---------------------------------------------------------------- EVENT -- */

async function eventView(id) {
  const event = await api(`/events/${encodeURIComponent(id)}`);
  setStatus(`event ${id.slice(0, 12)}…`);

  const observationLinks = el(
    "ul",
    { class: "evidence" },
    event.observations.map((obsId) =>
      el("li", {}, [
        el("a", { href: `#/observation/${obsId}`, text: `observation ${obsId}` }),
      ])
    )
  );

  render("world", event.title, [
    trail([
      { label: "WORLD", href: withLens("#/world") },
      { label: `EVENT ${event.id.slice(0, 10)}…` },
    ]),
    el("div", { class: "detail" }, [
      el("div", {}, [
        el("div", { class: "panel" }, [
          el("h2", { text: `Development (${event.observations.length} observations)` }),
          observationLinks,
        ]),
      ]),
      el("div", {}, [
        el("div", { class: "panel" }, [
          el("h2", { text: "Event" }),
          el("dl", { class: "kv" }, [
            el("dt", { text: "state" }), el("dd", { text: event.state }),
            el("dt", { text: "direction" }), el("dd", { text: event.direction }),
            el("dt", { text: "first seen" }), el("dd", { text: fmtTime(event.first_seen) }),
            el("dt", { text: "last seen" }), el("dd", { text: fmtTime(event.last_seen) }),
            el("dt", { text: "duration" }), el("dd", { text: fmtDuration(event.duration_seconds) }),
            el("dt", { text: "sources" }), el("dd", { text: String(event.source_count) }),
            el("dt", { text: "entities" }),
            el("dd", { text: event.entities.join(", ") || "—" }),
          ]),
        ]),
      ]),
    ]),
  ]);
}

/* ---------------------------------------------------------- OBSERVATION -- */

async function observationView(id) {
  const observation = await api(`/observations/${encodeURIComponent(id)}`);
  const source = await api(`/sources/${encodeURIComponent(observation.source_id)}`).catch(() => null);
  setStatus(`observation ${id.slice(0, 12)}…`);

  const raw = observation.raw || {};
  const rawHref = `/observations/${encodeURIComponent(id)}/raw`;

  render("world", `Observation ${observation.id.slice(0, 14)}…`, [
    trail([
      { label: "WORLD", href: withLens("#/world") },
      { label: "OBSERVATION" },
      { label: "SOURCE", href: `#/source/${observation.source_id}` },
      { label: "RAW DATA" },
    ]),
    el("div", { class: "detail" }, [
      el("div", {}, [
        el("div", { class: "panel" }, [
          el("h2", { text: "Raw data reference" }),
          el("dl", { class: "kv" }, [
            el("dt", { text: "locator" }), el("dd", { text: raw.locator || "—" }),
            el("dt", { text: "hash" }), el("dd", { text: raw.hash || "—" }),
            el("dt", { text: "content type" }), el("dd", { text: raw.content_type || "—" }),
            el("dt", { text: "bytes" }), el("dd", { text: raw.bytes ?? "—" }),
          ]),
          // The drill-down ends here, so the bytes themselves have to be
          // reachable, not just their reference.
          el("p", { style: "color: var(--ink-dim); font-size: 0.8rem", text: "Retained payload, exactly as the source returned it." }),
          el("p", {}, [
            el("a", { href: rawHref, target: "_blank", rel: "noreferrer", text: "Open raw payload →" }),
          ]),
        ]),
        el("div", { class: "panel", style: "margin-top:1rem" }, [
          el("h2", { text: "Measurement" }),
          el("dl", { class: "kv" }, [
            el("dt", { text: "metric" }), el("dd", { text: observation.metric }),
            el("dt", { text: "value" }), el("dd", { text: `${observation.value} ${observation.unit}` }),
            el("dt", { text: "observed at" }), el("dd", { text: fmtTime(observation.observed_at) }),
            el("dt", { text: "received at" }), el("dd", { text: fmtTime(observation.received_at) }),
            el("dt", { text: "lag" }), el("dd", { text: fmtLag(observation.lag_ms) }),
            el("dt", { text: "quality" }), el("dd", { text: observation.quality?.score?.toFixed?.(2) ?? "—" }),
            el("dt", { text: "flags" }),
            el("dd", { text: (observation.quality?.flags || []).join(", ") || "none" }),
            el("dt", { text: "entity" }), el("dd", { text: observation.entity_id || "—" }),
            el("dt", { text: "series" }), el("dd", { text: observation.series_key || "—" }),
          ]),
        ]),
      ]),
      el("div", {}, [
        source
          ? el("div", { class: "panel" }, [
              el("h2", { text: "Source" }),
              el("dl", { class: "kv" }, [
                el("dt", { text: "name" }), el("dd", { text: source.name }),
                el("dt", { text: "provider" }), el("dd", { text: source.provider }),
                el("dt", { text: "category" }), el("dd", { text: source.category }),
                el("dt", { text: "protocol" }), el("dd", { text: source.protocol }),
                el("dt", { text: "format" }), el("dd", { text: source.format }),
                el("dt", { text: "license" }), el("dd", { text: source.license || "—" }),
              ]),
              el("a", { href: `#/source/${source.id}`, text: "→ source detail" }),
            ])
          : el("div", { class: "panel", text: "Source metadata unavailable." }),
      ]),
    ]),
  ]);
}

/* --------------------------------------------------------------- SOURCE -- */

async function sourceView(id) {
  const detail = await api(`/sources/${encodeURIComponent(id)}`);
  const source = detail;
  const health = detail.health;
  setStatus(`source ${source.id}`);

  const healthRows = health
    ? [
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
      ]
    : [el("dt", { text: "health" }), el("dd", { text: "never run" })];

  render("sources", source.name, [
    trail([
      { label: "WORLD", href: withLens("#/world") },
      { label: "SOURCE" },
    ]),
    el("div", { class: "detail" }, [
      el("div", {}, [
        el("div", { class: "panel" }, [
          el("h2", { text: "Health" }),
          el("p", {
            class: "summary",
            text: "A collector failure is recorded here. It never means 'world activity = 0'.",
          }),
          el("dl", { class: "kv" }, healthRows),
        ]),
      ]),
      el("div", {}, [
        el("div", { class: "panel" }, [
          el("h2", { text: "Catalog" }),
          el("dl", { class: "kv" }, [
            el("dt", { text: "id" }), el("dd", { text: source.id }),
            el("dt", { text: "provider" }), el("dd", { text: source.provider }),
            el("dt", { text: "category" }), el("dd", { text: source.category }),
            el("dt", { text: "subcategory" }), el("dd", { text: source.subcategory || "—" }),
            el("dt", { text: "protocol" }), el("dd", { text: source.protocol }),
            el("dt", { text: "format" }), el("dd", { text: source.format }),
            el("dt", { text: "cadence" }),
            el("dd", { text: source.cadence_label || "—" }),
            el("dt", { text: "license" }), el("dd", { text: source.license || "—" }),
            el("dt", { text: "authentication" }), el("dd", { text: source.authentication || "—" }),
            el("dt", { text: "enabled" }), el("dd", { text: source.enabled ? "yes" : "no" }),
          ]),
        ]),
      ]),
    ]),
  ]);
}

async function sourcesIndex() {
  const sources = await api("/sources");
  setStatus(`${sources.length} source(s)`);
  const cards = sources.map((source) =>
    el("a", { class: "card", href: `#/source/${source.id}` }, [
      el("div", { class: "title", text: source.name }),
      el("div", { class: "summary", text: `${source.category} · ${source.provider}` }),
      el("div", { class: "meta" }, [
        el("span", { text: source.protocol }),
        el("span", { text: source.cadence_label || "—" }),
        source.health ? healthBadge(source.health.status) : el("span", { text: "never run" }),
      ]),
    ])
  );
  render("sources", "Sources", el("div", { class: "grid" }, cards));
}

/* ------------------------------------------------------------- TIMELINE -- */

/** Render the series and its baseline as an inline SVG sparkline. */
function sparkline(observations, baseline) {
  const width = 800;
  const height = 160;
  const padding = 12;
  const values = observations.map((o) => o.value);
  if (baseline) values.push(baseline.mean, baseline.p05, baseline.p95);
  const min = Math.min(...values);
  const max = Math.max(...values);
  const span = max - min || 1;

  const x = (i) => padding + (i / Math.max(1, observations.length - 1)) * (width - padding * 2);
  const y = (v) => height - padding - ((v - min) / span) * (height - padding * 2);

  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", `0 0 ${width} ${height}`);
  svg.setAttribute("class", "spark");

  if (baseline) {
    for (const [value, cls] of [[baseline.p95, "baseline"], [baseline.mean, "baseline"], [baseline.p05, "baseline"]]) {
      const line = document.createElementNS("http://www.w3.org/2000/svg", "line");
      line.setAttribute("x1", padding);
      line.setAttribute("x2", width - padding);
      line.setAttribute("y1", y(value));
      line.setAttribute("y2", y(value));
      line.setAttribute("class", cls);
      svg.append(line);
    }
  }

  const path = document.createElementNS("http://www.w3.org/2000/svg", "polyline");
  path.setAttribute("class", "series");
  path.setAttribute(
    "points",
    observations.map((o, i) => `${x(i)},${y(o.value)}`).join(" ")
  );
  svg.append(path);

  // The "● NOW" marker.
  if (observations.length > 0) {
    const lastIndex = observations.length - 1;
    const dot = document.createElementNS("http://www.w3.org/2000/svg", "circle");
    dot.setAttribute("cx", x(lastIndex));
    dot.setAttribute("cy", y(observations[lastIndex].value));
    dot.setAttribute("r", 4);
    dot.setAttribute("class", "now");
    svg.append(dot);
  }
  return svg;
}

async function timelineView(seriesKey) {
  const data = await api(`/timeline?series=${encodeURIComponent(seriesKey)}&limit=400`);
  // The API returns newest-first; the chart reads oldest-first.
  const observations = (data.observations || []).slice().reverse();
  setStatus(`${observations.length} points`);

  render("world", `Timeline · ${seriesKey}`, [
    trail([
      { label: "WORLD", href: withLens("#/world") },
      { label: "TIMELINE" },
    ]),
    el("div", { class: "panel" }, [
      el("h2", { text: "Normal ──╮ ╰──● NOW" }),
      sparkline(observations, data.baseline),
      data.baseline
        ? el("dl", { class: "kv" }, [
            el("dt", { text: "baseline n" }), el("dd", { text: String(data.baseline.sample_size) }),
            el("dt", { text: "mean" }), el("dd", { text: data.baseline.mean.toFixed(3) }),
            el("dt", { text: "median" }), el("dd", { text: data.baseline.median.toFixed(3) }),
            el("dt", { text: "mad" }), el("dd", { text: data.baseline.mad.toFixed(3) }),
            el("dt", { text: "p05 / p95" }),
            el("dd", { text: `${data.baseline.p05.toFixed(2)} / ${data.baseline.p95.toFixed(2)}` }),
          ])
        : null,
    ]),
  ]);
}

/* ------------------------------------------------------------------ MAP -- */

/** Equirectangular projection, for signal/event markers only. */
function project(lat, lon, width, height) {
  return [((lon + 180) / 360) * width, ((90 - lat) / 180) * height];
}

async function mapView() {
  const page = await api("/signals?limit=200");
  const located = (page.items || []).filter((s) => s.location);
  setStatus(`${located.length} located signal(s)`);

  const width = 720;
  const height = 360;
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", `0 0 ${width} ${height}`);
  svg.setAttribute("class", "map");

  for (let lon = -180; lon <= 180; lon += 30) {
    const line = document.createElementNS("http://www.w3.org/2000/svg", "line");
    const [px] = project(0, lon, width, height);
    line.setAttribute("x1", px); line.setAttribute("x2", px);
    line.setAttribute("y1", 0); line.setAttribute("y2", height);
    line.setAttribute("class", "graticule");
    svg.append(line);
  }
  for (let lat = -90; lat <= 90; lat += 30) {
    const line = document.createElementNS("http://www.w3.org/2000/svg", "line");
    const [, py] = project(lat, 0, width, height);
    line.setAttribute("x1", 0); line.setAttribute("x2", width);
    line.setAttribute("y1", py); line.setAttribute("y2", py);
    line.setAttribute("class", "graticule");
    svg.append(line);
  }

  for (const signal of located) {
    const [px, py] = project(signal.location.latitude, signal.location.longitude, width, height);
    const primary = primaryType(signal.types);
    const marker = document.createElementNS("http://www.w3.org/2000/svg", "circle");
    marker.setAttribute("cx", px);
    marker.setAttribute("cy", py);
    marker.setAttribute("r", 4);
    marker.setAttribute(
      "class",
      "marker" + (primary === "ANOMALY" ? " anomaly" : primary === "EARLY_SIGNAL" ? " early" : "")
    );
    const title = document.createElementNS("http://www.w3.org/2000/svg", "title");
    title.textContent = `${signal.title} — ${signal.summary}`;
    marker.append(title);
    svg.append(marker);
  }

  render("map", "Map", [
    el("div", { class: "panel" }, [
      el("h2", { text: "Signals with a location" }),
      el("p", {
        class: "summary",
        text: "Only signal/event level geography is shown. Raw observations are not scattered on the map.",
      }),
      svg,
    ]),
  ]);
}

/* --------------------------------------------------------------- ROUTER -- */

async function route() {
  // The lens rides in the query string, which is part of the hash. Strip it
  // before splitting, or a lens id would be read as a route segment and
  // `#/world?lens=lens_energy` would fall through to the default view.
  const hash = window.location.hash || "#/world";
  const route = hash.split("?")[0];
  const parts = route.replace(/^#\//, "").split("/");
  const [name, ...rest] = parts;
  const id = rest.join("/");
  try {
    switch (name) {
      case "signal":
        return await signalView(id);
      case "event":
        return await eventView(id);
      case "observation":
        return await observationView(id);
      case "source":
        return await sourceView(id);
      case "sources":
        return await sourcesIndex();
      case "lenses":
        return await lensesView();
      case "timeline":
        return await timelineView(decodeURIComponent(id));
      case "map":
        return await mapView();
      default:
        return await worldView();
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
    const text = await (await fetch("/metrics", { headers })).text();
    const wanted = ["wse_observations_total", "wse_anomalies_total", "wse_signals_total"];
    const lines = text
      .split("\n")
      .filter((l) => wanted.some((w) => l.startsWith(w + " ")))
      .map((l) => l.replace("wse_", "").replace("_total", ""));
    metricsEl.textContent = lines.join("  ");
  } catch (_) { /* metrics are decorative */ }
}

window.addEventListener("hashchange", route);
if (keyInput) {
  keyInput.value = apiKey();
  if (apiKey()) revealKeyPicker();
  keyInput.addEventListener("change", (event) => {
    setApiKey(event.target.value.trim());
    // Re-run the current view so the new key takes effect immediately.
    route();
  });
}
if (lensSelect) {
  lensSelect.addEventListener("change", (event) => {
    // Keep the current hash; only the query changes. `worldView` re-reads it.
    window.location.href = withLens(window.location.hash || "#/world", event.target.value);
  });
}
window.addEventListener("DOMContentLoaded", () => {
  route();
  refreshMetrics();
  setInterval(refreshMetrics, 5000);
});
