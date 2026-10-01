/* The studio orchestrator.
 *
 * It owns the screen: which scene is on air, the blocks that scene mounted, and
 * the transitions between scenes. It is the only thing that talks to the scene
 * manager, the director, the chrome and the ticker, which keeps the wiring in
 * one readable place instead of spread across the modules that do the work.
 *
 * The director decides *what* the broadcast should do; this decides *how*. That
 * separation is deliberate — a policy change must never require touching the
 * composition code, and a layout change must never require touching policy. */

import { SceneManager } from "./scene.js";
import { transition } from "./transition.js";
import { Director } from "./director.js";
import { grade } from "./priority.js";
import { loadScene, loadPolicy } from "../scenes/load.js";
import { t, sceneLabel } from "../i18n.js";

export class Studio {
  /**
   * @param {object} options
   * @param {import("../data/store.js").Store} options.store
   * @param {import("../shell/chrome.js").Chrome} options.chrome
   * @param {import("../shell/ticker.js").Ticker} options.ticker
   * @param {HTMLElement} options.stage
   * @param {HTMLElement} options.watermark
   * @param {object} [options.hooks]  host callbacks a block may invoke, e.g. `open`
   */
  constructor({ store, chrome, ticker, stage, watermark, hooks = null }) {
    this.store = store;
    this.chrome = chrome;
    this.ticker = ticker;
    this.stage = stage;
    this.watermark = watermark;

    // The hooks travel down to every block's context. They are the only way a
    // block can ask for a different scene, which keeps navigation out of blocks.
    this.scenes = new SceneManager(stage, store, { onError: (e) => this.sceneError(e), hooks });
    this.sceneCache = new Map();
    this.currentScene = null;
    this.transitionKind = "fade";
    this.unsubscribes = [];
    this.headlines = [];
    this.screenOwner = "scene";     // scene | takeover
    this.errors = [];

    this.director = new Director(store, {
      cut: (id, opts) => this.cutTo(id, opts),
      takeover: (signal, graded) => this.startTakeover(signal, graded),
      headline: (item) => this.addHeadline(item),
      refresh: () => this.scenes.update(),
    });
  }

  async start() {
    const policy = await loadPolicy();
    if (policy) this.director.setPolicy(policy);
    this.wire();
    // A block that measures its box learns about a format change here, without
    // a window listener and without re-rendering regions whose box is unchanged.
    this.scenes.watchResize(({ regionId }) => {
      this.scenes.resizeRegion(regionId);
      // A format change is exactly when a viewer has stepped away and come
      // back, so it is also when the broadcast should be showing the live
      // signal again rather than a stale pick from before.
      this.store.clearSelection();
    });
    return this.cutTo("overview", { reason: "boot", transition: "cut" });
  }

  /** Connect the studio to the store's events. */
  wire() {
    const bus = this.store.bus;

    this.unsubscribes.push(bus.on("connection", (connection) => {
      this.chrome.setConnection(connection);
      this.reflectWatermark();
    }));

    // Data arriving is the common case: push it into the regions rather than
    // recomposing. Recomposing on every poll would restart every animation.
    this.unsubscribes.push(bus.on("data", () => {
      this.scenes.update();
      this.reflectWatermark();
      this.tickerFromSignals();
    }));

    this.unsubscribes.push(bus.on("data:signals", () => {
      this.director.observe(this.store.signals);
    }));

    // A signal arriving on the live stream is the moment the broadcast must
    // react, so it is handled on the event rather than on the next poll.
    this.unsubscribes.push(bus.on("sse:signal", (signal) => {
      if (signal) this.director.observe([signal]);
    }));

    this.unsubscribes.push(bus.on("sse:source_failed", (event) => {
      this.addHeadline({ glyph: "▲", text: `${event?.source_id ?? "a source"}: collector failure — this is not zero activity` });
    }));

    this.unsubscribes.push(bus.on("sse:source_recovered", (event) => {
      this.addHeadline({ glyph: "●", text: `${event?.source_id ?? "a source"}: collector recovered` });
    }));
  }

  /**
   * Put a scene on air.
   *
   * The scene is fetched and validated before anything is torn down, so a
   * missing or broken scene file leaves the current picture intact rather than
   * emptying the screen.
   */
  async cutTo(id, { reason = "", transition: kind = null } = {}) {
    if (!id) return false;
    if (this.currentScene === id) return true;

    let scene = this.sceneCache.get(id);
    if (!scene) {
      try {
        scene = await loadScene(id);
        this.sceneCache.set(id, scene);
      } catch (err) {
        this.recordError(`scene "${id}": ${err.message}`);
        return false;
      }
    }

    const kind_ = kind || scene.transition || this.transitionKind;
    const previous = this.currentScene;

    // Compose inside the transition so the outgoing scene stays up while the
    // incoming one is built; the viewer never sees an empty stage.
    let accepted = false;
    await transition(kind_, () => { accepted = this.scenes.compose(scene); }, this.stage);

    if (!accepted) return false;
    this.currentScene = id;
    this.chrome.setScene(sceneLabel(id));
    this.store.bus.emit("scene:changed", { id, previous, reason });
    this.note(`scene ${id}${reason ? ` (${reason})` : ""}`);
    return true;
  }

  /**
   * Return to the scene that was on air before a takeover.
   *
   * A takeover must not lose the broadcast's position: coming back from a
   * breaking alert on the map should land on the map, not on the overview.
   */
  async resume() {
    const previous = this.scenes.previousSceneId();
    this.screenOwner = "scene";
    if (previous && previous !== this.currentScene) {
      await this.cutTo(previous, { reason: "resume" });
    }
  }

  /** The director asked for a takeover. The breaking layer owns the screen now. */
  startTakeover(signal, graded, decision) {
    this.screenOwner = "takeover";
    this.store.bus.emit("takeover:start", { signal, grade: graded, decision });
  }

  /** The breaking layer finished; hand the screen back. */
  endTakeover(signalId) {
    this.screenOwner = "scene";
    this.director.release(signalId);
    this.store.bus.emit("takeover:end", { signalId });
  }

  /** Compose a different set of regions without leaving the scene. */
  setRegions(regions) {
    const ok = this.scenes.setRegions(regions);
    if (ok) this.scenes.update();
    return ok;
  }

  /** Push new data into every region, e.g. after the control room edits a region. */
  refreshRegions() { this.scenes.update(); }

  /* ------------------------------------------------------------- headlines */

  /** The ticker is derived from the live signals, so it is always current. */
  tickerFromSignals() {
    const signals = this.store.signals ?? [];
    if (!signals.length) {
      this.setHeadlines([]);
      return;
    }
    const items = signals.slice(0, 8).map((signal) => {
      const graded = grade(signal, this.director.policy);
      return {
        glyph: glyphFor(graded.level),
        text: `${signal.title} — ${graded.why}`,
        signal,
      };
    });
    this.setHeadlines(items);
  }

  addHeadline(item) {
    this.headlines.unshift(item);
    if (this.headlines.length > 8) this.headlines.pop();
    this.ticker.set(this.headlines);
  }

  setHeadlines(items) {
    this.headlines = items;
    this.ticker.set(items);
  }

  /* --------------------------------------------------------------- status */

  /**
   * Show the watermark when the engine is not collecting.
   *
   * This is the shell's half of the project's central rule. A frozen picture
   * with no explanation reads as a quiet world, and a collector failure is not
   * a quiet world — so the screen says so in words.
   */
  reflectWatermark() {
    if (!this.watermark) return;
    const { state } = this.store.connection;
    const collecting = this.store.control?.collection_enabled;
    const monitoring = this.store.control?.monitoring;

    let message = null;
    let detail = "";
    if (state === "offline") {
      message = t().state.notWatching;
      detail = t().reason.collectorFailed;
    } else if (this.store.control && (collecting === false || monitoring === false)) {
      message = t().state.paused;
      detail = t().conn.reconnecting;
    }

    if (!message) {
      this.watermark.hidden = true;
      return;
    }
    this.watermark.hidden = false;
    const text = this.watermark.querySelector(".stage-watermark-text");
    if (text) {
      text.replaceChildren();
      text.append(message);
      const small = document.createElement("small");
      small.textContent = detail;
      text.append(small);
    }
  }

  sceneError({ scene, errors }) {
    this.recordError(`scene "${scene}" refused: ${errors.join("; ")}`);
  }

  recordError(message) {
    this.errors.push({ at: Date.now(), message });
    if (this.errors.length > 20) this.errors.shift();
    console.error(`[studio] ${message}`);
  }

  note(message) { this.director.note(message); }

  dispose() {
    for (const off of this.unsubscribes) off();
    this.unsubscribes = [];
    this.scenes.clear();
  }
}

/** A glyph per priority so severity survives without colour. */
function glyphFor(level) {
  return { CRITICAL: "◆", HIGH: "◇", MEDIUM: "▸", LOW: "·", INFO: "·" }[level] || "·";
}
