/* Labels, in the reader's language.
 *
 * What is translated here is *naming*, not judgement: the five signal types,
 * the lifecycle statuses, the connection states, the field labels. The engine
 * already writes the sentence that explains a signal; the client's job is only
 * to name the kinds of change and the words around them.
 *
 * Both languages are kept in one table so a missing key is visible at a glance
 * rather than showing up as an empty label on a wall. */

const I18N = {
  en: {
    lang: "en",
    brand: "WORLD SIGNAL ENGINE",
    ticker: "BREAKING",
    type: {
      NOW: "NOW",
      ANOMALY: "ANOMALY",
      EARLY_SIGNAL: "EARLY SIGNAL",
      CONVERGENCE: "CONVERGENCE",
      IMPACT: "IMPACT",
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
    conn: {
      connecting: "connecting", live: "live", reconnecting: "reconnecting",
      error: "auth required", offline: "offline",
    },
    scene: {
      overview: "GLOBAL OVERVIEW",
      map: "GLOBAL MAP",
      signal: "SIGNAL INVESTIGATION",
      sources: "SOURCE OBSERVATORY",
      system: "SYSTEM OPERATIONS",
      evidence: "EVIDENCE / RAW",
    },
    field: {
      signals: "signals", observations: "observations", sources: "sources",
      anomalies: "anomalies", events: "events", deviation: "deviation",
      persistence: "persistence", evidence: "evidence", sourceCount: "sources",
      confidence: "confidence", firstSeen: "first seen", lastUpdate: "last update",
      duration: "duration", direction: "direction", quality: "quality",
      baseline: "baseline", current: "current", value: "value", unit: "unit",
      metric: "metric", category: "category", cadence: "cadence", status: "status",
      latency: "latency", lastSuccess: "last success", lastFailure: "last failure",
      records: "records", duplicates: "duplicates", errors: "errors",
      rateLimited: "rate limited", enabled: "enabled", provider: "provider",
      location: "location", entities: "entities", lenses: "lenses",
      observedAt: "observed", receivedAt: "received", raw: "raw data",
    },
    state: {
      noData: "no data",
      notMeasured: "not measured",
      unavailable: "measurement unavailable",
      noLocation: "no location data",
      empty: "nothing to show",
      quiet: "no active signals",
      watching: "world under observation",
      paused: "collection paused",
      notWatching: "engine is not collecting",
      on: "on",
      off: "off",
      neverRan: "never ran",
      stale: "data is not current",
      loadFailed: "source error",
    },
    reason: {
      noSignal: "No signal is active right now.",
      noLocation: "These observations carry no coordinates, so nothing can be placed on a map.",
      quietFeed: "Coverage is limited to the connected sources; a quiet feed is not a quiet world.",
      collectorFailed: "A collector failure is recorded here. It never means world activity is zero.",
      stale: "The last read failed, so this value may be old. It is shown as it was, not as zero.",
    },
    why: {
      noData: "the engine has not reported a value for this",
      notMeasured: "the engine does not measure this",
      noSelection: "no observation is selected; the drill-down reaches the payload through one",
    },
    action: {
      close: "CLOSE", open: "OPEN REPORT", reset: "RESET", save: "SAVE",
      controlRoom: "CONTROL ROOM", autoDirector: "AUTO DIRECTOR",
      scene: "SCENE", layout: "LAYOUT", data: "DATA", system: "SYSTEM",
      addRegion: "ADD REGION", removeRegion: "REMOVE", moveRegion: "MOVE",
      block: "BLOCK", preset: "PRESET", lens: "LENS", language: "LANGUAGE",
      key: "API KEY", connection: "CONNECTION", diagnostics: "DIAGNOSTICS",
      regions: "REGIONS", blocks: "BLOCKS", uptime: "UPTIME", pipeline: "PIPELINE",
      region: "region", measurements: "measurements", stream: "STREAM",
      elapsed: "elapsed", sourcesHealthy: "sources healthy",
      stage: "stage", collector: "collector", detection: "detection",
      newestSignal: "newest signal", observationLag: "observation lag",
      sceneCount: "scenes", regionCount: "regions",
      rotation: "rotation", of: "of",
    },
    stage: {
      source: "SOURCE", collect: "COLLECT", observe: "OBSERVATION",
      baseline: "BASELINE", detect: "DETECTION", correlate: "CORRELATION",
      signal: "SIGNAL", store: "STORAGE", stream: "STREAM",
    },
    priority: {
      INFO: "info", LOW: "low", MEDIUM: "medium", HIGH: "high", CRITICAL: "critical",
    },
    breaking: {
      kicker: "BREAKING", live: "LIVE", closing: "closing in",
      detected: "MAJOR SIGNAL DETECTED",
    },
  },

  tr: {
    lang: "tr",
    brand: "WORLD SIGNAL ENGINE",
    ticker: "SON DAKİKA",
    type: {
      NOW: "ŞİMDİ",
      ANOMALY: "ANOMALİ",
      EARLY_SIGNAL: "ERKEN SİNYAL",
      CONVERGENCE: "YAKINSAMA",
      IMPACT: "ETKİ",
    },
    typeHint: {
      NOW: "şu anda gerçekleşen anlamlı bir değişim",
      ANOMALY: "normal davranıştan belirgin bir sapma",
      EARLY_SIGNAL: "küçük ama sürekli ve hâlâ büyüyen",
      CONVERGENCE: "bağımsız kaynaklar aynı değişime işaret ediyor",
      IMPACT: "takip ettiğiniz bir alanı etkileyen değişim",
    },
    status: {
      NEW: "yeni", DEVELOPING: "gelişiyor", CONFIRMED: "doğrulandı",
      STABLE: "durağan", FADING: "sönüyor", RESOLVED: "sona erdi",
    },
    direction: { Up: "yükseliyor", Down: "düşüyor", Flat: "yatay", up: "yükseliyor", down: "düşüyor", flat: "yatay" },
    conn: {
      connecting: "bağlanıyor", live: "canlı", reconnecting: "yeniden bağlanıyor",
      error: "anahtar gerekli", offline: "çevrimdışı",
    },
    scene: {
      overview: "GENEL BAKIŞ",
      map: "KÜRESEL HARİTA",
      signal: "SİNYAL İNCELEME",
      sources: "KAYNAK GÖZLEMEVİ",
      system: "SİSTEM OPERASYON",
      evidence: "KANIT / HAM VERİ",
    },
    field: {
      signals: "sinyal", observations: "gözlem", sources: "kaynak",
      anomalies: "anomali", events: "olay", deviation: "sapma",
      persistence: "süreklilik", evidence: "kanıt", sourceCount: "kaynak",
      confidence: "güven", firstSeen: "ilk görülme", lastUpdate: "son güncelleme",
      duration: "süre", direction: "yön", quality: "kalite",
      baseline: "temel", current: "güncel", value: "değer", unit: "birim",
      metric: "metrik", category: "kategori", cadence: "kadans", status: "durum",
      latency: "gecikme", lastSuccess: "son başarı", lastFailure: "son hata",
      records: "kayıt", duplicates: "kopya", errors: "hata",
      rateLimited: "sınırlama", enabled: "etkin", provider: "sağlayıcı",
      location: "konum", entities: "varlıklar", lenses: "lensler",
      observedAt: "gözlendi", receivedAt: "alındı", raw: "ham veri",
    },
    state: {
      noData: "veri yok",
      notMeasured: "ölçülmüyor",
      unavailable: "ölçüm yok",
      noLocation: "konum verisi yok",
      empty: "gösterilecek bir şey yok",
      quiet: "aktif sinyal yok",
      watching: "dünya izleniyor",
      paused: "toplama duraklatıldı",
      notWatching: "motor toplama yapmıyor",
      on: "açık",
      off: "kapalı",
      neverRan: "hiç çalışmadı",
      stale: "veri güncel değil",
      loadFailed: "kaynak hatası",
    },
    reason: {
      noSignal: "Şu anda aktif bir sinyal yok.",
      noLocation: "Bu gözlemler koordinat taşımıyor, bu yüzden haritaya yerleştirilecek bir şey yok.",
      quietFeed: "Kapsam bağlı kaynaklarla sınırlı; sakin bir besleme sakin bir dünya demek değildir.",
      collectorFailed: "Burada bir toplayıcı hatası kayıtlı. Bu asla dünya aktivitesinin sıfır olduğu anlamına gelmez.",
      stale: "Son okuma başarısız oldu, bu değer eski olabilir. Sıfır olarak değil, olduğu gibi gösteriliyor.",
    },
    why: {
      noData: "motor bu alan için bir değer bildirmedi",
      notMeasured: "motor bunu ölçmüyor",
      noSelection: "seçili bir gözlem yok; ham veriye inen yol buradan geçer",
    },
    action: {
      close: "KAPAT", open: "RAPORU AÇ", reset: "SIFIRLA", save: "KAYDET",
      controlRoom: "KONTROL ODASI", autoDirector: "OTOMATİK REJİ",
      scene: "SAHNE", layout: "YERLEŞİM", data: "VERİ", system: "SİSTEM",
      addRegion: "BÖLGE EKLE", removeRegion: "KALDIR", moveRegion: "TAŞI",
      block: "BLOK", preset: "ÖN AYAR", lens: "LENS", language: "DİL",
      key: "API ANAHTARI", connection: "BAĞLANTI", diagnostics: "TEŞHİS",
      regions: "BÖLGELER", blocks: "BLOKLAR", uptime: "ÇALIŞMA SÜRESİ", pipeline: "AKIŞ",
      region: "bölge", measurements: "ölçüm", stream: "AKIŞ",
      elapsed: "geçen", sourcesHealthy: "kaynak sağlıklı",
      stage: "aşama", collector: "toplayıcı", detection: "tespit",
      newestSignal: "en yeni sinyal", observationLag: "gözlem gecikmesi",
      sceneCount: "sahne", regionCount: "bölge",
      rotation: "dönüş", of: "/",
    },
    stage: {
      source: "KAYNAK", collect: "TOPLAMA", observe: "GÖZLEM",
      baseline: "TEMEL", detect: "TESPİT", correlate: "İLİŞKİLENDİRME",
      signal: "SİNYAL", store: "DEPOLAMA", stream: "YAYIN",
    },
    priority: {
      INFO: "bilgi", LOW: "düşük", MEDIUM: "orta", HIGH: "yüksek", CRITICAL: "kritik",
    },
    breaking: {
      kicker: "FLAŞ", live: "CANLI", closing: "kapanış",
      detected: "BÜYÜK SİNYAL TESPİT EDİLDİ",
    },
  },
};

let lang = "en";

/** Choose the initial language: query string, then storage, then the browser. */
export function detectLang() {
  const q = new URLSearchParams(window.location.search).get("lang");
  if (q && I18N[q]) return q;
  try {
    const stored = localStorage.getItem("wse-lang");
    if (stored && I18N[stored]) return stored;
  } catch (_) { /* private mode: fall through to the browser's preference */ }
  const nav = (navigator.language || "en").toLowerCase();
  return nav.startsWith("tr") ? "tr" : "en";
}

export function setLang(next) {
  if (!I18N[next]) return;
  lang = next;
  try { localStorage.setItem("wse-lang", next); } catch (_) { /* ignore */ }
  document.documentElement.lang = next;
}

export function getLang() { return lang; }

/** The label table for the active language. */
export function t() { return I18N[lang] || I18N.en; }

export function typeLabel(type) {
  return (t().type[type] || String(type || "")).replace(/_/g, " ");
}

export function typeHint(type) { return t().typeHint[type] || ""; }

export function statusLabel(status) {
  return t().status[status] || String(status || "").toLowerCase();
}

export function directionLabel(d) {
  return t().direction[d] || t().direction[String(d || "").toLowerCase()] || d;
}

export function sceneLabel(id) { return t().scene[id] || String(id || "").toUpperCase(); }

export function priorityLabel(p) { return t().priority[p] || String(p || "").toLowerCase(); }

/** Every language the studio ships. */
export const LANGUAGES = Object.keys(I18N);
