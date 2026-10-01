/* World Signal Engine — web UI.
 *
 * No build step, no framework. The routes mirror the brief's drill-down:
 *
 *   #/observatory        the control room: the whole world on one screen (default)
 *   #/world              active signals (the feed)
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
 * Presentation modifiers (not routes): `?broadcast=1` hides the chrome for a
 * stream; `?window=7d` widens the observatory's activity chart.
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

/** A coarse "how long ago", for the freshness line. */
function fmtAgo(seconds) {
  if (seconds === null || seconds === undefined) return "—";
  const s = Math.max(0, Math.round(seconds));
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86400)}d`;
}

function fmtBytes(n) {
  if (n === null || n === undefined) return "—";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let v = n, i = 0;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  return `${v.toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
}

/**
 * Render a measured value so it does not lie. A solar X-ray flux of 2.5e-7
 * shown as `0.00` reads as "zero", which is a false statement about the world;
 * small and very large magnitudes keep significant digits.
 */
function fmtValue(v) {
  if (v === null || v === undefined) return "—";
  const a = Math.abs(v);
  if (a === 0) return "0";
  if (a < 0.01 || a >= 10_000) return v.toExponential(3).replace("e+", "e");
  return String(Number(v.toFixed(2)));
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

/* --------------------------------------------------------------- language --
 *
 * The engine now describes its signals in human language (see
 * crates/presentation). What remains the client's job is naming the *kinds* of
 * change — the five signal types, the lifecycle statuses, the direction words —
 * and doing it in the reader's language. Those words are not detection output;
 * they are labels, and labels are translated here.
 *
 * The default is English. Turkish is selected by the browser, or by ?lang=tr,
 * or by the toggle in the header. Detection itself is untouched by this: only
 * presentation changes.
 */

const I18N = {
  en: {
    lang: "en",
    nav: { observatory: "Observatory", world: "World", sources: "Sources", lenses: "Lenses", map: "Map", system: "System" },
    field: { lens: "Lens", key: "Key" },
    conn: { connecting: "connecting", live: "live", reconnecting: "reconnecting", error: "auth required", offline: "offline" },
    type: {
      NOW: "NOW", ANOMALY: "ANOMALY", EARLY_SIGNAL: "EARLY SIGNAL",
      CONVERGENCE: "CONVERGENCE", IMPACT: "IMPACT",
    },
    typeHint: {
      NOW: "a meaningful change happening right now",
      ANOMALY: "a clear departure from normal behaviour",
      EARLY_SIGNAL: "small but persistent and still growing",
      CONVERGENCE: "independent sources pointing at the same change",
      IMPACT: "a change that touches a scope you follow",
    },
    status: {
      NEW: "new", DEVELOPING: "developing", CONFIRMED: "confirmed",
      STABLE: "stable", FADING: "fading", RESOLVED: "resolved",
    },
    direction: { Up: "rising", Down: "falling", Flat: "sideways", up: "rising", down: "falling", flat: "sideways" },
    sev: { CRITICAL: "critical", HIGH: "high", MEDIUM: "medium", LOW: "low" },
    origin: { LIVE: "live data", SYNTHETIC: "synthetic data" },
    obs: {
      title: "World Signal Observatory",
      metricSignals: "active signals",
      metricSignalsHint: (total) => `${total} formed since the engine started`,
      metricObserved: "observations",
      metricObservedHint: "measurements stored, all sources",
      metricSources: "sources healthy",
      metricSourcesHint: "a failing collector is not a quiet world",
      metricActivity: (w) => `arrivals · ${w}`,
      metricActivityHint: (mean) => `latest hour · mean ${mean}`,
      metricElevated: "elevated hours",
      metricElevatedHint: "at or above twice the mean",
      metricFreshness: "data freshness",
      metricFreshnessHint: "age of the newest observation",
      noData: "no data",
      globe: "Global hotspots",
      globeMeta: (n) => `${n} located`,
      globeEmpty: "No active signal carries a location. Location appears only when a source reports it — the engine does not invent one.",
      activity: "Observation activity",
      activityMeta: (w, mean) => `${w} · mean ${mean.toFixed(1)}/bucket`,
      activityEmpty: "No observations in this window yet.",
      feed: "Signal feed",
      feedMeta: (n) => `${n} active`,
      catSignals: (n) => `${n} live`,
      catNoData: "no data yet",
      tickerTag: "Breaking",
      tickerIdle: "No active signal. The engine is observing; a departure from normal will appear here.",
      alertKicker: (sev) => `${sev} · attention required`,
      alertSources: "independent sources",
      alertConfidence: "confidence",
      alertClose: "Dismiss",
      broadcastHint: "Presentation mode: hide the chrome",
    },
    world: {
      title: "World",
      subtitle: "Every active signal the engine is currently surfacing. Each one says what changed, how far it departed from normal, and for how long.",
      subtitleLens: (name) => `Active signals visible through the ${name} lens. A lens changes what is shown, never what is detected.`,
      emptyTitle: "Nothing is changing beyond normal",
      empty: "No signals right now. The engine is observing; when something departs from normal, it will appear here.",
      emptyLens: (name) => `No signals through ${name} right now.`,
      now: "NOW",
      nowHint: "the freshest signals, best first",
      statuses: "by status",
      openSystem: "Open system status",
      sourcesHealthy: (up, total) => `${up}/${total} sources healthy`,
      observed: "observed",
      watching: "watching the world",
      paused: "monitoring paused",
      notWatching: "not watching",
      watchingHint: (age) => `new signals appear here on their own · last data ${age} ago`,
      pausedHint: "collection is paused — the feed will not change until it resumes",
      notWatchingHint: "no collection loop is running, so this is a static snapshot, not a live world",
      noDataYet: "no data collected yet",
      stale: (age) => `no fresh data for ${age}`,
      newBanner: (n) => `${n} new signal${n === 1 ? "" : "s"} since you started watching`,
      newBannerAction: "show",
      delivery: (ms) => `delivered in ${ms} ms`,
      streamDown: "live updates are not connected",
    },
    signal: {
      why: "Why this signal exists",
      where: "Where",
      whatChanged: "What changed",
      magnitude: "How large",
      whatWeDontKnow: "What we do not know",
      evidence: (n) => `Evidence (${n}) — each traces to an observation`,
      evidenceEmpty: "No evidence is attached to this signal yet.",
      timeline: "Timeline",
      details: "Signal",
      quality: "Quality (seven separate dimensions, not one score)",
      firstSeen: "first seen", lastUpdate: "last update", duration: "duration",
      direction: "direction", series: "series", entities: "entities",
      categories: "categories", lenses: "lenses", status: "status", origin: "data",
      evidenceSources: (n) => `${n} source${n === 1 ? "" : "s"}`,
      investigate: "Investigate",
      openTimeline: "NORMAL ──╮ ╰──● NOW",
    },
    facts: { deviation: "deviation", persistence: "persistence", sources: "sources", evidence: "evidence" },
    lenses: {
      title: "Lenses", subtitle: "A lens changes what is visible, never what is detected. The dataset underneath is one.",
      empty: "No lenses configured.", signals: "signals", id: "id", noDescription: "No description configured.",
    },
    map: {
      title: "Map", subtitle: "Signals and events only. Raw observations are never piled onto the map.",
      empty: "No active signal currently carries a location. Location appears when a source reports it — the engine will not invent one.",
      count: (n) => `${n} located signal${n === 1 ? "" : "s"}`,
    },
    system: {
      title: "System",
      collectionOn: "monitoring on",
      collectionPaused: "monitoring paused",
      noLoop: "no collection loop",
      latency: "Latency (measured, not estimated)",
      lagObservation: "source lag",
      lagCollector: "collector fetch",
      lagDetection: "detection",
      lagNewest: "newest signal age",
      lagHint: "Source lag is data freshness, not a defect: a daily feed arrives all at once.",
    },
  },
  tr: {
    lang: "tr",
    nav: { observatory: "Gözlemevi", world: "Dünya", sources: "Kaynaklar", lenses: "Lensler", map: "Harita", system: "Sistem" },
    field: { lens: "Lens", key: "Anahtar" },
    conn: { connecting: "bağlanıyor", live: "canlı", reconnecting: "yeniden bağlanıyor", error: "anahtar gerekli", offline: "çevrimdışı" },
    type: {
      NOW: "ŞİMDİ", ANOMALY: "ANOMALİ", EARLY_SIGNAL: "ERKEN SİNYAL",
      CONVERGENCE: "YAKINSAMA", IMPACT: "ETKİ",
    },
    typeHint: {
      NOW: "şu anda gerçekleşen anlamlı bir değişim",
      ANOMALY: "normal davranıştan belirgin bir sapma",
      EARLY_SIGNAL: "küçük ama sürekli ve hâlâ büyüyen",
      CONVERGENCE: "bağımsız kaynaklar aynı değişime işaret ediyor",
      IMPACT: "takip ettiğiniz bir alana dokunan bir değişim",
    },
    status: {
      NEW: "yeni", DEVELOPING: "gelişiyor", CONFIRMED: "doğrulandı",
      STABLE: "durağan", FADING: "sönümleniyor", RESOLVED: "sona erdi",
    },
    direction: { Up: "yükseliyor", Down: "düşüyor", Flat: "yatay", up: "yükseliyor", down: "düşüyor", flat: "yatay" },
    sev: { CRITICAL: "kritik", HIGH: "yüksek", MEDIUM: "orta", LOW: "düşük" },
    origin: { LIVE: "canlı veri", SYNTHETIC: "sentetik veri" },
    obs: {
      title: "Dünya Sinyal Gözlemevi",
      metricSignals: "aktif sinyal",
      metricSignalsHint: (total) => `motor başladığından beri ${total} sinyal oluştu`,
      metricObserved: "gözlem",
      metricObservedHint: "saklanan ölçümler, tüm kaynaklar",
      metricSources: "sağlıklı kaynak",
      metricSourcesHint: "bozuk bir toplayıcı, sakin bir dünya demek değildir",
      metricActivity: (w) => `gelen veri · ${w}`,
      metricActivityHint: (mean) => `son saat · ortalama ${mean}`,
      metricElevated: "yüksek saat",
      metricElevatedHint: "ortalamanın en az iki katı",
      metricFreshness: "veri tazeliği",
      metricFreshnessHint: "en yeni gözlemin yaşı",
      noData: "veri yok",
      globe: "Küresel odaklar",
      globeMeta: (n) => `${n} konumlu`,
      globeEmpty: "Konum taşıyan aktif sinyal yok. Konum yalnızca bir kaynak bildirdiğinde görünür — motor konum uydurmaz.",
      activity: "Gözlem etkinliği",
      activityMeta: (w, mean) => `${w} · ortalama ${mean.toFixed(1)}/kova`,
      activityEmpty: "Bu pencerede henüz gözlem yok.",
      feed: "Sinyal akışı",
      feedMeta: (n) => `${n} aktif`,
      catSignals: (n) => `${n} canlı`,
      catNoData: "henüz veri yok",
      tickerTag: "Son dakika",
      tickerIdle: "Aktif sinyal yok. Motor gözlemliyor; normalden bir sapma burada görünecek.",
      alertKicker: (sev) => `${sev} · dikkat gerekli`,
      alertSources: "bağımsız kaynak",
      alertConfidence: "güven",
      alertClose: "Kapat",
      broadcastHint: "Sunum modu: arayüzü gizle",
    },
    world: {
      title: "Dünya",
      subtitle: "Motorun şu anda öne çıkardığı tüm aktif sinyaller. Her biri neyin değiştiğini, normalden ne kadar saptığını ve ne kadar süredir sürdüğünü söyler.",
      subtitleLens: (name) => `${name} lensinden görünen aktif sinyaller. Lens yalnızca görüneni değiştirir, tespit edileni asla.`,
      emptyTitle: "Normalin dışında bir değişim yok",
      empty: "Şu anda sinyal yok. Motor gözlemliyor; bir şey normalden saparsa burada görünecek.",
      emptyLens: (name) => `${name} lensinden şu anda sinyal yok.`,
      now: "ŞİMDİ",
      nowHint: "en taze sinyaller, en iyisi başta",
      statuses: "duruma göre",
      openSystem: "Sistem durumunu aç",
      sourcesHealthy: (up, total) => `${total} kaynağın ${up} tanesi sağlıklı`,
      observed: "gözlem",
      watching: "dünya izleniyor",
      paused: "izleme duraklatıldı",
      notWatching: "izlenmiyor",
      watchingHint: (age) => `yeni sinyaller burada kendiliğinden görünür · son veri ${age} önce`,
      pausedHint: "toplama duraklatıldı — yeniden başlatılana kadar akış değişmez",
      notWatchingHint: "çalışan bir toplama döngüsü yok; bu canlı bir dünya değil, sabit bir görüntü",
      noDataYet: "henüz veri toplanmadı",
      stale: (age) => `${age} boyunca taze veri yok`,
      newBanner: (n) => `izlemeye başladığınızdan beri ${n} yeni sinyal`,
      newBannerAction: "göster",
      delivery: (ms) => `${ms} ms'de ulaştı`,
      streamDown: "canlı güncellemeler bağlı değil",
    },
    signal: {
      why: "Bu sinyal neden var",
      where: "Nerede",
      whatChanged: "Ne değişti",
      magnitude: "Ne kadar büyük",
      whatWeDontKnow: "Bilmediklerimiz",
      evidence: (n) => `Kanıt (${n}) — her biri bir gözleme kadar izlenebilir`,
      evidenceEmpty: "Bu sinyale henüz kanıt bağlı değil.",
      timeline: "Zaman çizelgesi",
      details: "Sinyal",
      quality: "Kalite (tek bir puan değil, yedi ayrı boyut)",
      firstSeen: "ilk görülme", lastUpdate: "son güncelleme", duration: "süre",
      direction: "yön", series: "seri", entities: "varlıklar",
      categories: "kategoriler", lenses: "lensler", status: "durum", origin: "veri",
      evidenceSources: (n) => `${n} kaynak`,
      investigate: "İncele",
      openTimeline: "NORMAL ──╮ ╰──● ŞİMDİ",
    },
    facts: { deviation: "sapma", persistence: "süreklilik", sources: "kaynak", evidence: "kanıt" },
    lenses: {
      title: "Lensler", subtitle: "Lens yalnızca görüneni değiştirir, tespit edileni asla. Altta yatan veri kümesi birdir.",
      empty: "Yapılandırılmış lens yok.", signals: "sinyal", id: "kimlik", noDescription: "Açıklama yapılandırılmamış.",
    },
    map: {
      title: "Harita", subtitle: "Yalnızca sinyaller ve olaylar. Ham gözlemler haritaya asla yığılmaz.",
      empty: "Şu anda konum taşıyan aktif sinyal yok. Konum, bir kaynak bildirdiğinde görünür — motor konum uydurmaz.",
      count: (n) => `${n} konumlu sinyal`,
    },
    system: {
      title: "Sistem",
      collectionOn: "izleme açık",
      collectionPaused: "izleme duraklatıldı",
      noLoop: "toplama döngüsü yok",
      latency: "Gecikme (tahmin değil, ölçüm)",
      lagObservation: "kaynak gecikmesi",
      lagCollector: "toplayıcı çekme",
      lagDetection: "tespit",
      lagNewest: "en yeni sinyal yaşı",
      lagHint: "Kaynak gecikmesi veri tazeliğidir, kusur değil: günlük besleme bir kerede gelir.",
    },
  },
};

function detectLang() {
  const q = new URLSearchParams(window.location.search).get("lang");
  if (q && I18N[q]) return q;
  try {
    const stored = localStorage.getItem("wse-lang");
    if (stored && I18N[stored]) return stored;
  } catch (_) { /* private mode */ }
  const nav = (navigator.language || "en").toLowerCase();
  return nav.startsWith("tr") ? "tr" : "en";
}

let LANG = "en";
function t() { return I18N[LANG] || I18N.en; }
function setLang(lang) {
  if (!I18N[lang]) return;
  LANG = lang;
  try { localStorage.setItem("wse-lang", lang); } catch (_) { /* ignore */ }
  document.documentElement.lang = lang;
  applyChrome();
}

/** The localized label for a signal type. */
function typeLabel(type) { return (t().type[type] || type).replace(/_/g, " "); }
/** The localized one-line meaning of a signal type. */
function typeHint(type) { return t().typeHint[type] || ""; }
function statusLabel(status) { return t().status[status] || String(status || "").toLowerCase(); }
function directionLabel(d) { return t().direction[d] || t().direction[String(d || "").toLowerCase()] || d; }
function originLabel(o) { return t().origin[o] || o; }

/** Type is carried by icon + label + shape, never by colour alone. */
function typeBadge(type) {
  return el("span", { class: "type-badge", "data-type": type, title: typeHint(type) }, [
    el("span", { class: "glyph", "aria-hidden": "true", text: TYPE_GLYPH[type] || "•" }),
    typeLabel(type),
  ]);
}

/** A small pill for a lifecycle status, in the reader's language. */
function statusPill(status) {
  if (!status) return null;
  return el("span", { class: "status-pill", "data-status": status }, [statusLabel(status)]);
}

/** A one-line summary of a signal's human narrative, for the feed card. */
function narrativeLine(signal) {
  const n = signal.narrative;
  if (!n) return signal.summary || "";
  return n.what_changed || n.magnitude_text || signal.summary || "";
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
    facts.push([t().facts.deviation, `${sigma >= 0 ? "+" : ""}${sigma.toFixed(1)}σ`]);
  }
  facts.push([t().facts.persistence, fmtDuration(signal.duration_seconds)]);
  const sources = new Set(signal.evidence.map((e) => e.source_id));
  facts.push([t().facts.sources, String(sources.size)]);
  facts.push([t().facts.evidence, String(signal.evidence.length)]);
  return el("div", { class: "sig-facts" }, facts.map(([k, v]) =>
    el("span", { class: "fact" }, [el("b", { text: k }), el("span", { text: v })])
  ));
}

/* ---------------------------------------------------------- OBSERVATORY -- */

/* The control room: one screen, the whole world.
 *
 * This is a different surface from the feed, not a reskin of it. The feed is
 * for reading; this is for watching — it must answer "is anything changing
 * anywhere, right now?" from across a room, and it must survive being captured
 * into a stream (see `?broadcast=1`).
 *
 * It reads one endpoint (`/observatory`) that is itself a composed read over
 * the same stores as the feed, so the board cannot disagree with the feed. Every
 * number on it is either measured or explicitly absent; nothing is invented to
 * fill a slot.
 */

/** `?broadcast=1` hides chrome. Returns whether it is on. */
function broadcastMode() {
  return new URLSearchParams(window.location.search).get("broadcast") === "1";
}

/** `?window=7d` widens the activity chart. */
function activityWindow() {
  return new URLSearchParams(window.location.search).get("window") === "7d" ? "7d" : "24h";
}

/* The clock and the relative ages must tick without refetching. One interval
 * re-renders only the text nodes that age, so the board is live even between
 * server pushes. */
let obsTicker = null;
/* The observatory's SSE subscription, so a signal forming on the server redraws
 * the board without a reload. */
let obsLive = null;

function stopObservatory() {
  if (obsTicker) { clearInterval(obsTicker); obsTicker = null; }
  if (obsLive) {
    if (obsLive.handler) removeSseHandler(obsLive.handler);
    if (obsLive.timer) clearTimeout(obsLive.timer);
    obsLive = null;
  }
  closeAlert();
  document.body.classList.remove("obs-active");
}

async function observatoryView() {
  const epoch = viewEpoch;
  const data = await api(`/observatory?window=${activityWindow()}`);
  if (epoch !== viewEpoch) return;
  // A refresh redraws the whole board, so the previous aging interval must go;
  // otherwise every redraw would leave another timer behind.
  if (obsTicker) { clearInterval(obsTicker); obsTicker = null; }

  const root = el("div", { class: "obs" });
  root.append(
    obsHeader(data),
    obsMetrics(data),
    obsBody(data),
    obsCategories(data),
    obsTickerBar(data),
    obsWatermark(data),
  );
  // The observatory owns the whole viewport; the shared chrome is not part of it.
  view.replaceChildren(root);
  document.body.classList.add("obs-active");

  paintAlert(data.alert);

  // Age every relative timestamp in place, once a second. A control room whose
  // "12m" freezes at page load is lying about how old its data is.
  const startedAt = Date.now();
  obsTicker = setInterval(() => {
    root.querySelectorAll("[data-ts]").forEach((node) => {
      node.textContent = fmtAgo(Math.max(0, (Date.now() - Number(node.dataset.ts)) / 1000));
    });
    const clock = root.querySelector("#obs-clock");
    if (clock) clock.textContent = new Date().toISOString().slice(11, 19);
    const stamp = root.querySelector("#obs-watermark-time");
    if (stamp) stamp.textContent = new Date().toISOString().slice(0, 19).replace("T", " ") + "Z";
    if (Date.now() - startedAt > 30_000) { stopObservatory(); }
  }, 1000);
  startObservatoryStream();
}

/**
 * Subscribe to the engine's activity stream and redraw the board when the world
 * changes.
 *
 * The stream is the same one the World screen uses; the observatory only cares
 * about frames that can alter the board (a new signal, a resolved one, a source
 * failing). Bursts are coalesced into one refetch, so a collector firing ten
 * observations does not trigger ten redraws.
 */
function startObservatoryStream() {
  if (obsLive) return;
  startActivityStream();
  const state = { handler: null, timer: null };
  obsLive = state;
  state.handler = (activity) => {
    if (!["SIGNAL", "ANOMALY", "EVENT", "SOURCE_FAILED", "SOURCE_RECOVERED"].includes(activity.kind)) return;
    if (state.timer) return;
    state.timer = setTimeout(async () => {
      if (!obsLive) return;
      obsLive.timer = null;
      await observatoryView();
    }, 1500);
  };
  addSseHandler(state.handler);
}

function obsHeader(data) {
  const state = data.monitoring
    ? (data.collection_enabled ? t().world.watching : t().world.paused)
    : t().world.notWatching;
  return el("header", { class: "obs-head" }, [
    el("div", { class: "obs-brand" }, [
      el("span", { class: "obs-title", text: t().obs.title }),
      el("span", { class: "obs-sub", text: state }),
    ]),
    el("nav", { class: "obs-nav" }, [
      el("a", { href: "#/world", text: t().nav.world }),
      el("a", { href: "#/map", text: t().nav.map }),
      el("a", { href: "#/sources", text: t().nav.sources }),
      el("a", { href: "#/system", text: t().nav.system }),
    ]),
    el("div", { class: "obs-clock" }, [
      el("span", { class: "obs-clock-time", id: "obs-clock", text: new Date().toISOString().slice(11, 19) }),
      el("span", { class: "obs-clock-zone", text: "UTC" }),
      el("button", {
        class: "btn", type: "button", id: "obs-broadcast",
        title: t().obs.broadcastHint, "aria-label": t().obs.broadcastHint,
        text: "⛶",
        onclick: toggleBroadcast,
      }),
    ]),
  ]);
}

/** Toggle presentation mode without a reload: it only hides chrome. */
function toggleBroadcast() {
  const on = document.body.classList.toggle("broadcast");
  const url = new URL(window.location.href);
  if (on) url.searchParams.set("broadcast", "1");
  else url.searchParams.delete("broadcast");
  window.history.replaceState(null, "", url);
}

/** The top strip: the six numbers that say what the world is doing. */
function obsMetrics(data) {
  const age = data.data_age_seconds;
  const healthy = data.sources_total > 0 && data.sources_healthy === data.sources_total;
  const activity = data.activity || {};
  const windowLabel = activity.window || "24H";
  return el("div", { class: "obs-metrics" }, [
    obsMetric(t().obs.metricSignals, String(data.active_signals), "bad",
      t().obs.metricSignalsHint(data.signals_total)),
    obsMetric(t().obs.metricObserved, String(data.observations_total), null,
      t().obs.metricObservedHint),
    obsMetric(t().obs.metricSources, `${data.sources_healthy}/${data.sources_total}`,
      healthy ? "ok" : "warn", t().obs.metricSourcesHint),
    obsMetric(t().obs.metricActivity(windowLabel), String(activity.current ?? 0), null,
      t().obs.metricActivityHint(Math.round(activity.baseline ?? 0))),
    obsMetric(t().obs.metricElevated, String(activity.elevated_buckets ?? 0),
      (activity.elevated_buckets ?? 0) > 0 ? "warn" : null, t().obs.metricElevatedHint),
    obsMetric(t().obs.metricFreshness,
      age === null || age === undefined ? t().obs.noData : fmtAgo(age),
      age === null || age === undefined ? "muted" : null, t().obs.metricFreshnessHint),
  ]);
}

function obsMetric(label, value, tone, hint) {
  return el("div", { class: "obs-metric" }, [
    el("span", { class: "obs-metric-label", text: label }),
    el("span", { class: "obs-metric-value", "data-tone": tone || "", text: value }),
    el("span", { class: "obs-metric-hint", text: hint }),
  ]);
}

function obsBody(data) {
  const body = el("div", { class: "obs-body" });
  body.append(obsGlobe(data), obsActivityPanel(data), obsFeedPanel(data));
  return body;
}

/* --- globe ------------------------------------------------------------- */

/**
 * An orthographic view of the Earth with the located signals on it.
 *
 * Only signals and events are plotted; raw observations are never piled onto a
 * map (the brief is explicit about this). When nothing carries a location, the
 * panel says so rather than showing an empty sphere that implies coverage.
 *
 * The projection is orthographic because that is what reads as a planet from a
 * distance; it is cheap to compute and needs no tiles or network.
 */
function obsGlobe(data) {
  const width = 520, height = 380, radius = 150;
  const cx = width / 2, cy = height / 2;
  const located = (data.feed || []).filter((s) => s.location);
  const body = el("div", { class: "obs-panel-body" });

  if (located.length === 0) {
    body.append(el("div", { class: "obs-empty", text: t().obs.globeEmpty }));
    return obsPanel(t().obs.globe, t().obs.globeMeta(0), body);
  }

  const svg = svgEl("svg", { viewBox: `0 0 ${width} ${height}`, class: "obs-globe", preserveAspectRatio: "xMidYMid meet" });
  svg.append(svgEl("circle", { cx, cy, r: radius, class: "limb" }));
  // Graticule at 30° steps. Longitude lines collapse toward the limb, which is
  // the visual cue that this is a sphere and not a flat map.
  for (let lat = -60; lat <= 60; lat += 30) {
    const y = cy - (lat / 90) * radius;
    const half = Math.sqrt(Math.max(0, radius * radius - (y - cy) * (y - cy)));
    svg.append(svgEl("line", { x1: cx - half, x2: cx + half, y1: y, y2: y, class: "grid" }));
  }
  for (let lon = -180; lon < 180; lon += 30) {
    const points = [];
    for (let lat = -90; lat <= 90; lat += 5) {
      const [px, py] = ortho(lat, lon, cx, cy, radius);
      if (px !== null) points.push(`${px},${py}`);
    }
    if (points.length > 1) svg.append(svgEl("polyline", { points: points.join(" "), class: "grid" }));
  }

  const color = {
    CRITICAL: "var(--bad)", HIGH: "var(--anomaly)", MEDIUM: "var(--warn)", LOW: "var(--now)",
  };
  for (const signal of located) {
    const [px, py] = ortho(signal.location.latitude, signal.location.longitude, cx, cy, radius);
    if (px === null) continue; // on the far side of the globe
    const tint = color[signal.severity] || "var(--now)";
    const marker = svgEl("circle", { cx: px, cy: py, r: 5, class: "dot", fill: tint });
    const title = svgEl("title");
    title.textContent = `${signal.title} — ${signal.severity_reason}`;
    marker.append(title);
    marker.addEventListener("click", () => { window.location.hash = `#/signal/${signal.signal_id}`; });
    svg.append(marker);
    // A ring marks the highest severities so the shape, not only the colour,
    // says "this one is worse".
    if (signal.severity === "CRITICAL" || signal.severity === "HIGH") {
      svg.append(svgEl("circle", { cx: px, cy: py, r: 11, class: "ring", stroke: tint }));
    }
  }

  body.append(svg);
  return obsPanel(t().obs.globe, t().obs.globeMeta(located.length), body);
}

/** Orthographic projection; `null` when the point faces away from the viewer. */
function ortho(lat, lon, cx, cy, radius) {
  const phi = (lat * Math.PI) / 180;
  const lambda = (lon * Math.PI) / 180;
  // Centred on 0°E, 0°N so the default view is the whole world at once.
  const cosC = Math.cos(phi) * Math.cos(lambda);
  if (cosC < 0) return [null, null];
  return [cx + radius * Math.cos(phi) * Math.sin(lambda), cy - radius * Math.sin(phi)];
}

/* --- activity chart ---------------------------------------------------- */

/**
 * Observations arriving per bucket over the window, with the mean drawn as a
 * dashed rule.
 *
 * The point of the chart is the comparison, not the curve: a spike means
 * nothing until it is read against the world's own recent norm. The dashed line
 * is that norm, computed from the same buckets.
 */
function obsActivityPanel(data) {
  const activity = data.activity || {};
  const buckets = activity.buckets || [];
  const width = 520, height = 190, pad = 22;
  const body = el("div", { class: "obs-panel-body" });

  if (buckets.length < 2) {
    body.append(el("div", { class: "obs-empty", text: t().obs.activityEmpty }));
    return obsPanel(t().obs.activity, activity.window || "24H", body);
  }

  const counts = buckets.map((b) => b.count);
  const max = Math.max(1, ...counts, Math.ceil(activity.baseline || 0));
  const x = (i) => pad + (i / (buckets.length - 1)) * (width - pad * 2);
  const y = (v) => height - pad - (v / max) * (height - pad * 2);

  const svg = svgEl("svg", { viewBox: `0 0 ${width} ${height}`, class: "obs-chart", preserveAspectRatio: "none" });
  // Elevated buckets (>= 2x the mean) are drawn as bars under the curve, so a
  // burst is visible as a shape even before the numbers are read.
  const threshold = 2 * (activity.baseline || 0);
  if (threshold > 0) {
    counts.forEach((count, i) => {
      if (count < threshold) return;
      const barWidth = (width - pad * 2) / buckets.length;
      svg.append(svgEl("rect", {
        x: x(i) - barWidth / 2, y: y(count), width: Math.max(1, barWidth * 0.8),
        height: height - pad - y(count), class: "bar",
      }));
    });
  }
  const points = buckets.map((b, i) => `${x(i)},${y(b.count)}`).join(" ");
  svg.append(svgEl("polygon", {
    class: "area",
    points: `${x(0)},${height - pad} ${points} ${x(buckets.length - 1)},${height - pad}`,
  }));
  svg.append(svgEl("polyline", { class: "line", points }));
  if (activity.baseline > 0) {
    svg.append(svgEl("line", { x1: pad, x2: width - pad, y1: y(activity.baseline), y2: y(activity.baseline), class: "mean" }));
    const label = svgEl("text", { x: pad + 2, y: y(activity.baseline) - 3, class: "mean-label" });
    label.textContent = `mean ${activity.baseline.toFixed(1)}`;
    svg.append(label);
  }

  body.append(svg);
  return obsPanel(t().obs.activity, t().obs.activityMeta(activity.window || "24H", activity.baseline || 0), body);
}

/* --- feed -------------------------------------------------------------- */

function obsFeedPanel(data) {
  const rows = (data.feed || []).map(obsFeedRow);
  const body = el("div", { class: "obs-panel-body" });
  body.append(rows.length
    ? el("ul", { class: "obs-feed-list" }, rows)
    : el("div", { class: "obs-empty", text: t().world.empty }));
  return obsPanel(t().obs.feed, t().obs.feedMeta(rows.length), body);
}

function obsFeedRow(item) {
  const glyph = TYPE_GLYPH[primaryType(item.types)] || "•";
  const ts = new Date(item.last_updated).getTime();
  return el("li", { class: "obs-feed-row", onclick: () => openAlert(item) }, [
    el("span", { class: "obs-feed-glyph", "aria-hidden": "true", text: glyph }),
    el("span", { class: "obs-feed-main" }, [
      el("span", { class: "obs-feed-title", text: item.title }),
      el("span", { class: "obs-feed-sub", text: item.severity_reason }),
    ]),
    el("span", { class: "obs-feed-right" }, [
      el("span", { class: "obs-feed-age", "data-ts": String(ts), text: fmtAgo(item.age_seconds) }),
      el("span", { class: "obs-feed-sev", "data-sev": item.severity, text: t().sev[item.severity] || item.severity }),
    ]),
  ]);
}

/* --- categories -------------------------------------------------------- */

function obsCategories(data) {
  const cards = (data.categories || []).map((card) => {
    const dir = (card.change_pct ?? 0) >= 0 ? "up" : "down";
    const children = [
      el("div", { class: "obs-cat-top" }, [
        el("span", { class: "obs-cat-name", text: card.label }),
        card.active_signals > 0
          ? el("span", { class: "obs-cat-badge", text: t().obs.catSignals(card.active_signals) })
          : null,
      ]),
    ];

    if (card.has_data) {
      children.push(el("div", { class: "obs-cat-value" }, [
        fmtValue(card.value),
        card.unit ? el("span", { class: "obs-cat-unit", text: card.unit }) : null,
      ]));
      children.push(el("div", { class: "obs-cat-delta", "data-dir": dir }, [
        card.change_pct !== null && card.change_pct !== undefined
          ? `${card.change_pct >= 0 ? "+" : ""}${card.change_pct.toFixed(1)}%`
          : "—",
        card.deviation_sigma !== null && card.deviation_sigma !== undefined
          ? ` · ${card.deviation_sigma >= 0 ? "+" : ""}${card.deviation_sigma.toFixed(1)}σ`
          : "",
      ]));
      const spark = catSparkline(card.sparkline, card.baseline);
      if (spark) children.push(spark);
    } else {
      children.push(el("div", { class: "obs-cat-value", "data-empty": "true", text: t().obs.catNoData }));
      children.push(el("div", { class: "obs-cat-reason", text: card.empty_reason || "" }));
    }
    children.push(el("div", { class: "obs-cat-reason", text: card.severity_reason }));

    // A card with a series opens that series' timeline — the actual evidence
    // for the number. An empty card has nothing to show, so it goes to the
    // sources list instead of pretending there is a chart behind it.
    const target = card.has_data && card.series_key
      ? `#/timeline/${encodeURIComponent(card.series_key)}`
      : "#/sources";
    return el("article", {
      class: "obs-cat",
      "data-sev": card.severity,
      "data-empty": String(!card.has_data),
      title: card.severity_reason,
      onclick: () => { window.location.hash = withLens(target); },
    }, children);
  });
  return el("div", { class: "obs-cats" }, cards);
}

/** A category's recent points against its baseline, small enough to fit a card. */
function catSparkline(points, baseline) {
  if (!points || points.length < 2) return null;
  const width = 188, height = 26, pad = 2;
  const values = points.map((p) => p.value);
  const bounds = values.slice();
  if (baseline) bounds.push(baseline.p05, baseline.p95);
  let min = Math.min(...bounds), max = Math.max(...bounds);
  if (min === max) { min -= 1; max += 1; }
  const span = max - min;
  const x = (i) => pad + (i / (points.length - 1)) * (width - pad * 2);
  const y = (v) => height - pad - ((v - min) / span) * (height - pad * 2);
  const svg = svgEl("svg", { viewBox: `0 0 ${width} ${height}`, class: "obs-cat-spark", preserveAspectRatio: "none" });
  if (baseline) {
    svg.append(svgEl("line", { x1: pad, x2: width - pad, y1: y(baseline.mean), y2: y(baseline.mean), class: "base" }));
  }
  svg.append(svgEl("polyline", { class: "line", points: points.map((p, i) => `${x(i)},${y(p.value)}`).join(" ") }));
  return svg;
}

/* --- ticker ------------------------------------------------------------ */

function obsTickerBar(data) {
  const items = data.ticker || [];
  const track = el("div", { class: "obs-ticker-track" });
  if (items.length === 0) {
    track.append(el("span", { class: "obs-ticker-item", text: t().obs.tickerIdle }));
  } else {
    // Duplicated once so the CSS translate(-50%) loop is seamless.
    const line = () => items.map((item) =>
      el("span", { class: "obs-ticker-item" }, [
        el("b", { text: `${fmtClock(item.at)} · ` }),
        item.text,
      ])
    );
    track.append(el("span", { class: "obs-ticker-run" }, [...line(), ...line()]));
  }
  return el("div", { class: "obs-ticker" }, [
    el("span", { class: "obs-ticker-tag", "data-idle": String(items.length === 0), text: t().obs.tickerTag }),
    track,
  ]);
}

/**
 * The broadcast watermark.
 *
 * A recorded stream of this board must never be mistakable for a live one. The
 * watermark states the data origin and the freshness, and it is shown in
 * broadcast mode (where the chrome that would otherwise carry that context is
 * hidden). It is deliberately plain text, not a logo.
 */
function obsWatermark(data) {
  const age = data.data_age_seconds;
  const state = data.monitoring
    ? (data.collection_enabled ? t().world.watching : t().world.paused)
    : t().world.notWatching;
  const freshness = age === null || age === undefined ? t().obs.noData : fmtAgo(age);
  return el("div", { class: "obs-watermark" }, [
    el("span", { text: "WORLD SIGNAL ENGINE" }),
    el("span", { text: `${state} · ${t().obs.metricFreshness} ${freshness}` }),
    el("span", { id: "obs-watermark-time", text: new Date().toISOString().slice(0, 19).replace("T", " ") + "Z" }),
  ]);
}

function obsPanel(title, meta, body) {
  return el("section", { class: "obs-panel" }, [
    el("header", { class: "obs-panel-head" }, [
      el("h2", { class: "obs-panel-title", text: title }),
      meta ? el("span", { class: "obs-panel-meta", text: meta }) : null,
    ]),
    body,
  ]);
}

/* --- alert modal ------------------------------------------------------- */

let alertOpenFor = null;

function closeAlert() {
  alertOpenFor = null;
  const existing = document.getElementById("obs-alert");
  if (existing) existing.remove();
}

/**
 * The alert modal: one signal, the evidence for it, and the way to investigate.
 *
 * It is opened by a click on a feed row, or automatically for a CRITICAL signal
 * the reader has not seen yet. It is deliberately not shown for routine
 * signals: a modal that appears constantly is one people learn to dismiss.
 */
function openAlert(item) {
  closeAlert();
  alertOpenFor = item.signal_id;
  const facts = [
    [t().facts.deviation, evidenceSigma(item)],
    [t().facts.persistence, fmtDuration(item.duration_seconds)],
    [t().obs.alertSources, String((item.sources || []).length)],
    [t().obs.alertConfidence, `${Math.round((item.confidence || 0) * 100)}%`],
  ].filter(([, v]) => v !== null && v !== undefined);

  const scrim = el("div", { class: "obs-alert-scrim", id: "obs-alert", onclick: (e) => { if (e.target === scrim) closeAlert(); } }, [
    el("div", { class: "obs-alert", role: "dialog", "aria-modal": "true", "aria-label": item.title }, [
      el("div", { class: "obs-alert-kicker" }, [
        el("span", { "aria-hidden": "true", text: TYPE_GLYPH[primaryType(item.types)] || "•" }),
        el("span", { text: t().obs.alertKicker(item.severity) }),
      ]),
      el("h2", { class: "obs-alert-title", text: item.title }),
      el("p", { class: "obs-alert-summary", text: item.summary }),
      el("div", { class: "obs-alert-reason", text: item.severity_reason }),
      el("div", { class: "obs-alert-facts" }, facts.map(([k, v]) =>
        el("span", {}, [el("b", { text: k }), el("span", { text: v })])
      )),
      el("div", { class: "obs-alert-actions" }, [
        el("button", { class: "btn", type: "button", text: t().obs.alertClose, onclick: closeAlert }),
        el("a", { class: "btn primary", href: withLens(`#/signal/${item.signal_id}`), text: t().signal.investigate }),
      ]),
    ]),
  ]);
  document.body.append(scrim);
  const closer = (e) => { if (e.key === "Escape") { closeAlert(); document.removeEventListener("keydown", closer); } };
  document.addEventListener("keydown", closer);
}

/** The first stated deviation in a feed item's evidence, formatted, or a dash. */
function evidenceSigma(item) {
  const sigma = item.deviation_sigma;
  if (sigma === null || sigma === undefined) return "—";
  return `${sigma >= 0 ? "+" : ""}${sigma.toFixed(1)}σ`;
}

/** Show the alert modal for a CRITICAL signal the reader has not seen. */
function paintAlert(alert) {
  if (!alert) return;
  if (alertOpenFor === alert.signal_id) return;
  openAlert(alert);
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

/* Bumped on every navigation. A view that awaits a fetch must check it before
 * rendering: otherwise an in-flight World refresh lands after the reader has
 * already left for another screen and paints over it. */
let viewEpoch = 0;

async function worldView(options = {}) {
  const remount = options.remount !== false;
  const epoch = viewEpoch;
  const lens = activeLens();
  const world = await api("/world").catch(() => null);
  const query = lens ? `&lens=${encodeURIComponent(lens)}` : "";
  const page = await api(`/signals?limit=100${query}`);
  const signals = page.items || [];
  const lenses = await loadLenses();

  // The reader navigated away while this was loading; drop the result.
  if (epoch !== viewEpoch) return;

  const subtitle = lens
    ? t().world.subtitleLens(lensName(lens, lenses))
    : t().world.subtitle;

  // Count signals that appeared since this screen started watching, before we
  // replace the known set. A change is never silent: the banner says how many.
  // The full feed is used, not just the NOW strip, so nothing new is missed.
  if (worldLive) {
    for (const signal of signals) {
      if (!worldLive.knownSignalIds.has(signal.id)) worldLive.newCount += 1;
      worldLive.knownSignalIds.add(signal.id);
    }
  }

  worldLastSummary = world;
  worldKnownSignalIds = signals.map((s) => s.id);

  if (signals.length === 0) {
    // An empty world is only "quiet" if we are actually watching. Say which.
    render("world", t().world.title, subtitle,
      el("div", { class: "empty" }, [
        el("div", { class: "world-notice", id: "world-notice" }),
        el("h3", { text: t().world.emptyTitle }),
        el("p", { text: lens ? t().world.emptyLens(lensName(lens, lenses)) : t().world.empty }),
        el("div", { class: "link-row", style: "justify-content:center" }, [
          el("a", { class: "btn", href: "#/system", text: t().world.openSystem }),
        ]),
      ])
    );
    startActivityStream();
    if (remount) startWorldStream();
    paintWorldNotice();
    return;
  }
  const nodes = [];

  // The monitoring banner: whether the world is being watched, and how fresh
  // the data is. Rendered from the summary and updated in place, so it can
  // never claim "live" while nothing is running.
  nodes.push(el("div", { class: "world-notice", id: "world-notice" }));

  // The NOW strip: what is changing, in one glance, before the long feed.
  // It only appears on the unfiltered view — a lens is already a filter.
  if (!lens && world && (world.now || []).length) {
    nodes.push(el("div", { class: "now-strip" }, [
      el("div", { class: "now-head" }, [
        el("span", { class: "now-label", text: t().world.now }),
        el("span", { class: "now-hint", text: t().world.nowHint }),
      ]),
      el("div", { class: "now-row" }, world.now.map((signal) =>
        el("button", {
          class: "now-item",
          type: "button",
          "data-primary": primaryType(signal.types),
          onclick: () => { window.location.hash = withLens(`#/signal/${signal.id}`); },
        }, [
          el("span", { class: "now-title", text: signal.narrative?.headline || signal.title }),
          el("span", { class: "now-meta" }, [
            signal.narrative?.magnitude_text
              ? el("span", { text: signal.narrative.magnitude_text.split(".")[0] })
              : null,
            statusPill(signal.status),
          ]),
        ])
      )),
    ]));
  }

  // A compact, factual header: how much is changing, of what kind, and whether
  // the sources behind it are healthy. This is the answer to "is the engine
  // seeing anything, and can I trust it?" before reading a single card.
  if (world) {
    nodes.push(el("div", { class: "world-stats", id: "world-stats" }, worldStatChips(world)));
  }

  const cards = signals.map((signal) =>
    el("article", {
      class: "sig",
      "data-primary": primaryType(signal.types),
      onclick: () => { window.location.hash = withLens(`#/signal/${signal.id}`); },
    }, [
      el("div", { class: "sig-head" }, [
        el("div", { class: "sig-types" }, (signal.types || []).map(typeBadge)),
        statusPill(signal.status),
        el("span", { class: "fact" }, [
          el("b", { text: t().signal.lastUpdate }),
          el("span", { text: fmtClock(signal.last_updated) }),
        ]),
      ]),
      el("h2", { class: "sig-title", text: signal.narrative?.headline || signal.title }),
      el("p", { class: "sig-summary", text: narrativeLine(signal) }),
      signal.narrative?.unknowns?.length
        ? el("p", { class: "sig-unknown", text: signal.narrative.unknowns[0] })
        : null,
      signalFacts(signal),
      signal.series_key
        ? el("a", {
            class: "btn",
            href: withLens(`#/timeline/${encodeURIComponent(signal.series_key)}`),
            text: t().signal.timeline,
            onclick: (e) => e.stopPropagation(),
          })
        : null,
    ])
  );

  nodes.push(el("div", { class: "feed", id: "world-feed" }, cards));
  render("world", t().world.title, subtitle, nodes);

  // The world screen is live: it opens the event stream and re-fetches when a
  // signal forms, so a change appears on its own.
  startActivityStream();
  if (remount) startWorldStream();
  paintWorldNotice();
}

/** The header chips for a world summary, reused on live refresh. */
function worldStatChips(world) {
  return [
    statChip(String(world.active_signals), t().world.title.toLowerCase(), null),
    ...(world.by_type || []).filter((x) => x.count > 0).map((x) =>
      statChip(String(x.count), typeLabel(x.type), x.type)
    ),
    el("span", {
      class: "world-health",
      text: t().world.sourcesHealthy(world.sources_healthy, world.sources_total),
    }),
  ];
}

/** Paint the monitoring banner from the last summary. */
function paintWorldNotice() {
  const notice = document.getElementById("world-notice");
  if (!notice) return;
  const world = worldLastSummary;
  const nodes = [];

  if (world && world.monitoring) {
    nodes.push(el("span", { class: "watch-dot live", "aria-hidden": "true" }));
    nodes.push(el("span", { class: "watch-state", text: t().world.watching }));
    const age = world.last_collection_at ? fmtAgo(world.data_age_seconds) : t().world.noDataYet;
    nodes.push(el("span", { class: "watch-hint", text: t().world.watchingHint(age) }));
  } else if (world && world.collector_active && !world.collection_enabled) {
    nodes.push(el("span", { class: "watch-dot paused", "aria-hidden": "true" }));
    nodes.push(el("span", { class: "watch-state", text: t().world.paused }));
    nodes.push(el("span", { class: "watch-hint", text: t().world.pausedHint }));
  } else {
    // Either no loop was ever started, or we could not reach the engine.
    nodes.push(el("span", { class: "watch-dot off", "aria-hidden": "true" }));
    nodes.push(el("span", { class: "watch-state", text: t().world.notWatching }));
    nodes.push(el("span", { class: "watch-hint", text: t().world.notWatchingHint }));
  }

  if (connEl && connEl.dataset.state !== "live") {
    nodes.push(el("span", { class: "watch-warn", text: t().world.streamDown }));
  }

  // A change is never silent: if signals arrived since this screen opened, say
  // so, and let the reader jump to the top of the feed.
  if (worldLive && worldLive.newCount > 0) {
    nodes.push(el("button", {
      class: "new-banner",
      type: "button",
      text: `${t().world.newBanner(worldLive.newCount)} · ${t().world.newBannerAction}`,
      onclick: () => {
        if (worldLive) worldLive.newCount = 0;
        window.scrollTo({ top: 0, behavior: "smooth" });
        paintWorldNotice();
      },
    }));
  }

  notice.replaceChildren(...nodes);
}

/** Re-render the banner in place, for staleness, without a network call. */
function updateWorldHeader() {
  paintWorldNotice();
}

/** A small count chip for the world header. */
function statChip(count, label, type) {
  return el("span", { class: "stat-chip", "data-type": type || "" }, [
    el("b", { text: count }),
    el("span", { text: label }),
  ]);
}

/* --------------------------------------------------------------- SIGNAL -- */

async function signalView(id) {
  const signal = await api(`/signals/${encodeURIComponent(id)}`);
  const n = signal.narrative || {};

  const evidence = el("div", { class: "evidence" },
    (signal.evidence || []).length
      ? (signal.evidence || []).map((item) =>
          el("div", { class: "ev" }, [
            el("div", { class: "ev-top" }, [
              el("span", { class: "ev-metric", text: `${item.metric} = ${fmtValue(item.value)} ${item.unit}` }),
              item.deviation_sigma !== null && item.deviation_sigma !== undefined
                ? el("span", { class: "ev-sigma", text: `${item.deviation_sigma >= 0 ? "+" : ""}${item.deviation_sigma.toFixed(2)}σ` })
                : null,
            ]),
            item.record_label
              ? el("div", { class: "ev-record", text: item.record_label })
              : null,
            el("div", { class: "ev-statement", text: item.statement }),
            el("div", { class: "ev-links" }, [
              el("a", { href: `#/observation/${item.observation_id}`, text: "observation" }),
              el("a", { href: `#/source/${item.source_id}`, text: "source" }),
              el("a", { href: `/observations/${encodeURIComponent(item.observation_id)}/raw`, target: "_blank", rel: "noreferrer", text: "raw data" }),
              el("span", { text: fmtTime(item.observed_at) }),
            ]),
          ])
        )
      : el("p", { class: "page-sub", text: t().signal.evidenceEmpty })
  );

  // The narrative panel is the whole point: the engine's answer, in order —
  // what changed, where, how large, why it was surfaced, and what is unknown.
  const narrative = el("div", { class: "narrative" }, [
    n.headline ? el("h2", { class: "narrative-headline", text: n.headline }) : null,
    el("dl", { class: "kv narrative-kv" }, [
      n.what_changed ? el("dt", { text: t().signal.whatChanged }) : null,
      n.what_changed ? el("dd", { text: n.what_changed }) : null,
      n.where_text ? el("dt", { text: t().signal.where }) : null,
      n.where_text ? el("dd", { text: n.where_text }) : null,
      n.magnitude_text ? el("dt", { text: t().signal.magnitude }) : null,
      n.magnitude_text ? el("dd", { text: n.magnitude_text }) : null,
      n.why_signal ? el("dt", { text: t().signal.why }) : null,
      n.why_signal ? el("dd", { text: n.why_signal }) : null,
    ]),
  ]);

  render("world", signal.narrative?.headline || signal.title, n.what_changed || signal.summary, [
    crumbs([
      { label: t().world.title.toUpperCase(), href: withLens("#/world") },
      { label: `SIGNAL ${signal.id.slice(0, 10)}…` },
      { label: `EVENT ${signal.event_id.slice(0, 10)}…`, href: `#/event/${signal.event_id}` },
    ]),
    el("div", { class: "sig-types", style: "margin-bottom:14px" },
      [...(signal.types || []).map(typeBadge), statusPill(signal.status)].filter(Boolean)),
    el("div", { class: "panel" }, [narrative]),
    n.unknowns?.length
      ? el("div", { class: "panel unknown-panel" }, [
          el("h2", { class: "panel-title", text: t().signal.whatWeDontKnow }),
          el("ul", { class: "reasons" }, n.unknowns.map((u) => el("li", { text: u }))),
        ])
      : null,
    el("div", { class: "panel" }, [
      el("h2", { class: "panel-title", text: t().signal.evidence((signal.evidence || []).length) }),
      evidence,
    ]),
    signal.series_key
      ? el("div", { class: "panel" }, [
          el("h2", { class: "panel-title", text: t().signal.timeline }),
          el("div", { class: "link-row" }, [
            el("a", { class: "btn primary", href: withLens(`#/timeline/${encodeURIComponent(signal.series_key)}`), text: t().signal.openTimeline }),
          ]),
        ])
      : null,
    el("div", { class: "panel" }, [
      el("h2", { class: "panel-title", text: t().signal.details }),
      el("dl", { class: "kv" }, [
        el("dt", { text: t().signal.status }), el("dd", { text: statusLabel(signal.status) }),
        el("dt", { text: t().signal.firstSeen }), el("dd", { text: fmtTime(signal.first_seen) }),
        el("dt", { text: t().signal.lastUpdate }), el("dd", { text: fmtTime(signal.last_updated) }),
        el("dt", { text: t().signal.duration }), el("dd", { text: fmtDuration(signal.duration_seconds) }),
        el("dt", { text: t().signal.direction }), el("dd", { text: directionLabel(signal.direction) }),
        el("dt", { text: t().signal.origin }), el("dd", { text: originLabel(signal.data_origin) }),
        el("dt", { text: t().signal.series }), el("dd", { class: "mono", text: signal.series_key || "—" }),
        el("dt", { text: t().signal.entities }), el("dd", { text: (signal.entities || []).join(", ") || "—" }),
        el("dt", { text: t().signal.categories }), el("dd", { text: (signal.categories || []).join(", ") || "—" }),
        el("dt", { text: t().signal.lenses }), el("dd", { text: (signal.lens_matches || []).join(", ") || "—" }),
      ]),
    ]),
    el("div", { class: "panel" }, [
      el("h2", { class: "panel-title", text: t().signal.quality }),
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
        el("dt", { text: "value" }), el("dd", { text: `${fmtValue(observation.value)} ${observation.unit}` }),
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
    render("lenses", t().lenses.title, null, el("div", { class: "empty", text: t().lenses.empty }));
    return;
  }
  const describe = (lens) => {
    // Prefer the human description from the config. The filter fields are a
    // fallback for a lens that predates descriptions, not the primary text.
    if (lens.description) return lens.description;
    const parts = [];
    if (lens.categories?.length) parts.push(`categories: ${lens.categories.join(", ")}`);
    if (lens.entities?.length) parts.push(`entities: ${lens.entities.join(", ")}`);
    if (lens.keywords?.length) parts.push(`keywords: ${lens.keywords.join(", ")}`);
    if (lens.bbox) parts.push("a bounding box");
    return parts.length ? parts.join(" · ") : t().lenses.noDescription;
  };
  const cards = lenses.map((lens) =>
    el("a", { class: "sig", href: `#/world?lens=${encodeURIComponent(lens.id)}`, "data-primary": "NOW" }, [
      el("h2", { class: "sig-title", text: lens.name }),
      el("p", { class: "sig-summary", text: describe(lens) }),
      el("div", { class: "sig-facts" }, [
        el("span", { class: "fact" }, [el("b", { text: t().lenses.signals }), el("span", { text: String(lens.matching_signals ?? 0) })]),
        el("span", { class: "fact" }, [el("b", { text: t().lenses.id }), el("span", { text: lens.id })]),
      ]),
    ])
  );
  render("lenses", t().lenses.title, t().lenses.subtitle, el("div", { class: "feed" }, cards));
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
            el("dt", { text: "mean" }), el("dd", { text: fmtValue(data.baseline.mean) }),
            el("dt", { text: "median" }), el("dd", { text: fmtValue(data.baseline.median) }),
            el("dt", { text: "mad" }), el("dd", { text: fmtValue(data.baseline.mad) }),
            el("dt", { text: "p05 / p95" }), el("dd", { text: `${fmtValue(data.baseline.p05)} / ${fmtValue(data.baseline.p95)}` }),
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
    title.textContent = `${signal.narrative?.headline || signal.title} — ${narrativeLine(signal)}`;
    marker.append(title);
    svg.append(marker);
  }
  render("map", t().map.title, t().map.subtitle, [
    el("div", { class: "panel" }, [
      located.length
        ? el("div", { class: "map-wrap" }, [svg])
        : el("div", { class: "empty", text: t().map.empty }),
      el("div", { class: "legend" }, [
        el("span", { class: "item", text: t().map.count(located.length) }),
      ]),
    ]),
  ]);
}

/* --------------------------------------------------------------- SYSTEM -- */

/* Activity kinds arrive SCREAMING_SNAKE_CASE (the engine's serde form). */
const ACTIVITY_ICON = {
  STARTED: "▶", OBSERVATION: "·", ANOMALY: "◇", EVENT: "□", SIGNAL: "⚡",
  SOURCE_RECOVERED: "✓", SOURCE_FAILED: "✕", SOURCE_RATE_LIMITED: "⏳", CONTROL: "⚙",
};

let activityLog = [];
/* Signal ids the world screen has already shown, so a newly arriving signal
 * can be counted as new without re-fetching on every frame. */
let worldKnownSignalIds = [];
/* The world screen's last fetched summary, kept so the header can be re-rendered
 * (for staleness) without a network round-trip. */
let worldLastSummary = null;
/* Subscribers to live SSE frames, beyond the activity log. The World screen
 * registers one; the System screen does not need to. */
const sseHandlers = new Set();

function addSseHandler(fn) { sseHandlers.add(fn); }
function removeSseHandler(fn) { sseHandlers.delete(fn); }
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
    disabled: !control.collector_active,
    onclick: async () => {
      try {
        await post("/control/collection", { enabled: !control.collection_enabled });
        toast(control.collection_enabled ? "collection paused" : "collection resumed");
        systemView();
      } catch (err) { toast(err.message, "error"); }
    },
  });

  // The three states are different truths and are labelled differently: a
  // loop running, a loop paused, and no loop at all. The last is the case that
  // used to look identical to a healthy but quiet world.
  const statePill = control.monitoring
    ? el("span", { class: "pill on", text: t().system.collectionOn })
    : control.collector_active
      ? el("span", { class: "pill off", text: t().system.collectionPaused })
      : el("span", { class: "pill off", text: t().system.noLoop });

  // Real latency, each figure measured from stored timestamps.
  const lat = control.latency || {};
  const latencyPanel = el("div", { class: "panel" }, [
    el("h2", { class: "panel-title", text: t().system.latency }),
    el("div", { class: "stat-grid" }, [
      stat(t().system.lagObservation, fmtLag(lat.observation_lag_ms)),
      stat(t().system.lagCollector, fmtLag(lat.collector_ms)),
      stat(t().system.lagDetection, fmtLag(lat.detection_ms)),
      stat(t().system.lagNewest, fmtLag(lat.newest_signal_age_ms)),
    ]),
    el("p", { class: "page-sub", text: t().system.lagHint }),
  ]);

  const feed = el("div", { class: "activity", id: "activity-feed" }, activityLog.map(activityRow));

  render("system", "System", "The engine's real operational state: what is running, what it is doing, and the live stream of changes.", [
    el("div", { class: "bar" }, [
      statePill,
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
    latencyPanel,
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
  // Forward the frame to whoever is listening (the World screen re-fetches on
  // signal frames). The activity feed below is a separate concern.
  for (const handler of sseHandlers) {
    try { handler(entry); } catch (_) { /* a broken listener must not stop the stream */ }
  }
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
  // Idempotent: both the World and System screens want the stream open, and
  // neither should tear it down for the other. `route()` stops it on leave.
  if (streamController) return;
  const controller = new AbortController();
  streamController = controller;
  runStream(controller);
}

function stopActivityStream() {
  if (streamController) { streamController.abort(); streamController = null; }
  // The world screen and the system screen share one stream but not one
  // consumer; stop it for whichever view is leaving.
  stopWorldStream();
}

/* The World screen's live wiring.
 *
 * The stream is already open (startActivityStream); all the World screen does
 * is listen to the frames the API forwards. It re-fetches the world when a
 * signal frame arrives, coalescing bursts into one request, and it counts what
 * is new since the reader started watching so a change is never silent.
 *
 * It also knows *why* nothing may be arriving: monitoring paused, no loop at
 * all, or data simply stale. "Quiet" and "not being watched" are different
 * states and the header says which one it is. */
let worldLive = null;

function stopWorldStream() {
  if (worldLive) {
    if (worldLive.sseHandler) removeSseHandler(worldLive.sseHandler);
    if (worldLive.refreshTimer) clearTimeout(worldLive.refreshTimer);
    if (worldLive.ageTimer) clearInterval(worldLive.ageTimer);
    worldLive = null;
  }
}

/** Mount the World screen's live wiring, once per visit. */
function startWorldStream() {
  if (worldLive) return;
  const state = {
    // Seed with the signals already on screen so only genuinely new ones count.
    knownSignalIds: new Set(worldKnownSignalIds),
    newCount: 0,
    sseHandler: null,
    refreshTimer: null,
    ageTimer: null,
  };
  worldLive = state;

  // A signal frame is the only thing that can add a signal to the world.
  state.sseHandler = (activity) => {
    if (activity.kind !== "SIGNAL") return;
    scheduleWorldRefresh();
  };
  addSseHandler(state.sseHandler);

  // Refresh the header's "last data" line every few seconds so staleness is
  // visible even when no frame arrives.
  state.ageTimer = setInterval(updateWorldHeader, 5000);
}

function scheduleWorldRefresh() {
  if (!worldLive || worldLive.refreshTimer) return;
  // Coalesce a burst of frames into a single world fetch, and re-render in
  // place: the stream stays open, the reader's scroll stays put.
  worldLive.refreshTimer = setTimeout(async () => {
    if (!worldLive) return;
    worldLive.refreshTimer = null;
    await worldView({ remount: false });
  }, 400);
}

async function runStream(controller) {
  let backoff = 1000;
  while (!controller.signal.aborted) {
    try {
      const headers = { accept: "text/event-stream" };
      const key = apiKey();
      if (key) headers["Authorization"] = `Bearer ${key}`;
      const response = await fetch("/events", { headers, signal: controller.signal });
      if (response.status === 401) { revealKeyField(); setConn("error"); return; }
      if (!response.ok || !response.body) throw new Error(`HTTP ${response.status}`);
      setConn("live");
      backoff = 1000;
      await readStream(response.body, controller.signal);
      setConn("reconnecting");
    } catch (err) {
      if (controller.signal.aborted) return;
      setConn("reconnecting");
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
      // The world screen is also resynced on reconnect: a signal that formed
      // while the stream was down must not be missed.
      if (worldLive) await worldView({ remount: false });
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
  if (connText) connText.textContent = text || t().conn[state] || state;
}

/** Apply the current language to the static chrome (nav, labels, footer). */
function applyChrome() {
  document.documentElement.lang = t().lang;
  // Broadcast mode strips the chrome so a stream shows only the world state.
  document.body.classList.toggle("broadcast", broadcastMode());
  document.querySelectorAll("[data-nav]").forEach((a) => {
    const key = a.dataset.nav;
    if (t().nav[key]) a.textContent = t().nav[key];
  });
  document.querySelectorAll("[data-i18n]").forEach((node) => {
    const path = node.dataset.i18n.split(".");
    let value = t();
    for (const part of path) value = value?.[part];
    if (typeof value === "string") node.textContent = value;
  });
  const toggle = document.getElementById("lang-toggle");
  if (toggle) toggle.textContent = LANG === "en" ? "TR" : "EN";
  const conn = document.getElementById("conn");
  if (conn) connText.textContent = t().conn[conn.dataset.state || "connecting"] || connText.textContent;
}

/* --------------------------------------------------------------- ROUTER -- */

async function route() {
  // Invalidate any view that is still awaiting a fetch, then stop the streams
  // a leaving view owns.
  viewEpoch += 1;
  stopActivityStream();
  stopObservatory();

  const hash = window.location.hash || "#/observatory";
  const clean = hash.split("?")[0];
  const parts = clean.replace(/^#\//, "").split("/");
  const [name, ...rest] = parts;
  const id = rest.join("/");
  try {
    switch (name) {
      case "observatory": return await observatoryView();
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
const langToggle = document.getElementById("lang-toggle");
if (langToggle) {
  langToggle.addEventListener("click", () => {
    setLang(LANG === "en" ? "tr" : "en");
    route();
  });
}
window.addEventListener("DOMContentLoaded", () => {
  LANG = detectLang();
  applyChrome();
  // The connection indicator must not claim "live" before anything is open.
  setConn("offline");
  route();
  refreshMetrics();
  setInterval(refreshMetrics, 5000);
});
