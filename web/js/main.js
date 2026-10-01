/* Boot.
 *
 * Wires the four pieces — store, chrome, ticker, studio — and hands the screen
 * over. The order matters: blocks are registered before any scene is composed,
 * and the studio is started before the store begins polling, so the first
 * update lands on a composed scene rather than an empty stage.
 *
 * There is no router here. Which scene is on air is studio state, and the URL
 * only mirrors it so a deep link can be shared; changing the hash must never
 * reload the page. */

import { registerBuiltinBlocks } from "./blocks/index.js";
import { Store } from "./data/store.js";
import { ActivityStream } from "./data/sse.js";
import { Studio } from "./studio/studio.js";
import { BreakingLayer } from "./blocks/breaking.js";
import { Chrome } from "./shell/chrome.js";
import { Ticker } from "./shell/ticker.js";
import { detectLang, setLang, t } from "./i18n.js";

const SCENE_KEYS = ["overview", "map", "signal", "sources", "system", "evidence"];
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

async function boot() {
  setLang(detectLang());

  const stage = document.getElementById("stage");
  const chromeHost = document.getElementById("chrome");
  const tickerHost = document.getElementById("ticker");
  const watermark = document.getElementById("watermark");
  const boot = document.getElementById("boot");

  registerBuiltinBlocks();

  const store = new Store({ apiKey });
  const chrome = new Chrome(chromeHost, {
    connectionLabel: (connection) => t().conn[connection?.state] || connection?.state || "",
  });
  const ticker = new Ticker(tickerHost, { label: t().ticker });

  const studio = new Studio({
    store, chrome, ticker, stage, watermark,
    hooks: {
      open: (sceneId, id) => {
        if (id) store.select(sceneFor(sceneId, id));
        studio.cutTo(sceneId, { reason: "open" });
      },
    },
  });

  const breaking = new BreakingLayer(document.getElementById("overlay"), {
    onOpen: (signal) => {
      store.select({ signalId: signal.id });
      breaking.dismiss({ silent: true });
      studio.endTakeover(signal.id);
      studio.cutTo("signal", { reason: "breaking" });
    },
    onClose: (signal) => studio.endTakeover(signal?.id),
  });

  store.bus.on("takeover:start", ({ signal, grade }) => {
    breaking.show(signal, { seconds: studio.director.policy.dismiss_after_seconds });
  });

  // Keyboard: the studio is fully operable without a pointer. `1`-`6` select a
  // scene, `Esc` dismisses a takeover. The control room's own keys arrive with
  // that layer.
  window.addEventListener("keydown", (event) => {
    if (event.target instanceof HTMLElement && event.target.closest("input, textarea")) return;
    if (event.key === "Escape" && breaking.active) { breaking.dismiss(); return; }
    const index = Number(event.key);
    if (Number.isInteger(index) && index >= 1 && index <= SCENE_KEYS.length) {
      studio.cutTo(SCENE_KEYS[index - 1], { reason: "key" });
    }
  });

  // The URL mirrors the scene so a link can be shared, but the hash is read,
  // never written by the router, and a change never reloads.
  window.addEventListener("hashchange", () => {
    const id = sceneFromHash();
    if (id && id !== studio.currentId) studio.cutTo(id, { reason: "link" });
  });

  const stream = new ActivityStream({
    apiKey,
    onResync: () => store.refreshAll(),
  });
  stream.bus.on("activity", (event) => {
    // The stream is the studio's fastest signal. It refreshes the resource the
    // event concerns rather than patching a list, so there is one code path for
    // a value whether it arrived by poll or by push.
    if (event?.kind === "OBSERVATION") store.refresh("activity");
    if (event?.kind === "SIGNAL") store.refresh("signals");
    store.bus.emit("sse:activity", event);
    if (event?.kind === "COLLECTOR_FAILED") store.bus.emit("sse:source_failed", event);
    if (event?.kind === "COLLECTOR_RECOVERED") store.bus.emit("sse:source_recovered", event);
    if (event?.signal) store.bus.emit("sse:signal", event.signal);
  });
  stream.bus.on("state", (state) => store.setConnection(state, null));

  const initial = sceneFromHash() || "overview";
  const started = await studio.start();
  if (started && initial !== "overview") await studio.cutTo(initial, { reason: "link" });
  boot.hidden = true;

  chrome.start();
  store.start();
  stream.start();

  // Exposed for the control room and for the browser console during
  // development. It is not a public API and nothing else reads it.
  window.wse = { studio, store, chrome, ticker, breaking, stream, started };

  // The simulation layer is development scaffolding, so it is fetched only when
  // it is asked for. A static import would ship it in the boot path of every
  // production screen.
  if (new URLSearchParams(window.location.search).get("sim") === "1") {
    const { runSimulation } = await import("./sim/simulation.js");
    runSimulation({ studio, store, breaking });
  }

  return started;
}

function sceneFor(sceneId, id) {
  if (sceneId === "signal") return { signalId: id };
  if (sceneId === "evidence") return { signalId: id };
  return {};
}

function sceneFromHash() {
  const hash = window.location.hash || "";
  const match = /^#\/studio\/([a-z]+)/.exec(hash);
  if (match && SCENE_KEYS.includes(match[1])) return match[1];
  // The pre-studio links are mapped so a saved URL still lands somewhere.
  const legacy = /^#\/(\w+)/.exec(hash);
  if (legacy) {
    const map = { world: "overview", observatory: "overview", map: "map", signal: "signal", system: "system", sources: "sources" };
    if (map[legacy[1]]) return map[legacy[1]];
  }
  return null;
}

boot().catch((err) => {
  console.error("[studio] boot failed", err);
  const boot = document.getElementById("boot");
  if (boot) {
    boot.hidden = false;
    const steps = boot.querySelector(".boot-steps");
    if (steps) {
      steps.replaceChildren();
      const line = document.createElement("div");
      line.className = "boot-step";
      line.dataset.state = "fail";
      line.textContent = `boot failed: ${err.message}`;
      steps.append(line);
    }
  }
});
