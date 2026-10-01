/* Scene manager: composes a scene out of regions and blocks.
 *
 * This is the layer the brief calls the studio orchestrator. It owns which
 * scene is on air, mounts the blocks a scene asks for, and keeps them updated
 * as the store changes. It has no opinion about what an event *means* — that is
 * the director's job, and the two must not blur: the director decides to go to
 * the map, the scene manager knows how to put the map on screen.
 *
 * Recomposition is diffed rather than torn down. Moving one region or swapping
 * one block must not remount the globe, because a remount would restart every
 * animation and re-fetch every detail. */

import { createBlock, createMissingBlock, hasBlock } from "../blocks/registry.js";
import { applyRegionFrame, applySceneFrame, validateScene } from "./layout.js";

export class SceneManager {
  /**
   * @param {HTMLElement} stage  the scene root's parent
   * @param {import("../data/store.js").Store} store
   * @param {object} options
   */
  constructor(stage, store, { onError = null, theme = null, hooks = null } = {}) {
    this.stage = stage;
    this.store = store;
    this.onError = onError;
    this.theme = theme;
    this.hooks = hooks;

    this.root = null;
    this.scene = null;
    this.regions = new Map();   // region id -> { definition, element, instance }
    this.unsubscribes = [];
    this.epoch = 0;
    this.history = [];          // scene ids, for returning after a takeover

    // A block that draws to a measured box has to be told when that box
    // changes. Each region element is observed, so a region that arrives later
    // is covered too.
    this.observer = null;
    this.onResize = null;
    this.sizes = null;          // region element -> last reported "WxH"
  }

  /**
   * Watch the composition for size changes.
   *
   * Each region element is observed, not the scene root: `ResizeObserver` on a
   * parent does not report a child's change, and the region box is what a block
   * measures. `callback` is invoked with `{ regionId, element }` for a region
   * whose box genuinely changed — the first observation is the size the block
   * was already mounted with, so it is recorded and not announced.
   */
  watchResize(callback) {
    this.onResize = callback;
    if (this.observer || typeof ResizeObserver !== "function") return;
    this.sizes = new Map();
    this.observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        const box = entry.contentRect;
        const key = `${Math.round(box.width)}x${Math.round(box.height)}`;
        const first = !this.sizes.has(entry.target);
        const previous = this.sizes.get(entry.target);
        this.sizes.set(entry.target, key);
        if (first || previous === key) continue;
        const region = [...this.regions.values()].find((r) => r.element === entry.target);
        if (region) this.onResize?.({ regionId: region.definition.id, element: entry.target });
      }
    });
    for (const [, region] of this.regions) this.observer.observe(region.element);
  }

  /** Stop watching for size changes. */
  unwatchResize() {
    this.observer?.disconnect();
    this.observer = null;
    this.onResize = null;
    this.sizes = null;
  }

  /** The id of the scene currently on air. */
  get currentId() { return this.scene?.id ?? null; }

  /**
   * Put a scene on air.
   *
   * @param {object} scene  a validated scene definition
   * @returns {boolean} whether the scene was accepted
   */
  compose(scene) {
    const check = validateScene(scene);
    if (!check.ok) {
      // A scene that does not validate is refused, and the previous scene stays
      // up. Showing a half-composed screen is worse than showing a stale one.
      this.onError?.({ scene: scene?.id, errors: check.errors });
      return false;
    }

    if (!this.root) {
      this.root = document.createElement("div");
      this.root.className = "scene";
      this.stage.append(this.root);
    }
    applySceneFrame(this.root, scene);

    const previous = this.regions;
    const next = new Map();
    this.epoch += 1;
    const epoch = this.epoch;

    for (const definition of scene.regions) {
      const existing = previous.get(definition.id);
      if (existing && existing.definition.block === definition.block) {
        // Same region, same block: keep the mounted instance and only move it.
        applyRegionFrame(existing.element, definition);
        existing.definition = definition;
        next.set(definition.id, existing);
        previous.delete(definition.id);
      } else {
        if (existing) {
          this.disposeRegion(existing);
          previous.delete(definition.id);
        }
        next.set(definition.id, this.createRegion(definition, epoch));
      }
    }

    // Whatever is left in `previous` was in the old scene but not the new one.
    for (const [, region] of previous) this.disposeRegion(region);

    this.regions = next;
    this.scene = scene;
    if (this.history[this.history.length - 1] !== scene.id) this.history.push(scene.id);
    if (this.history.length > 12) this.history.shift();

    this.store.bus.emit("scene:composed", { id: scene.id, regions: next.size });
    return true;
  }

  createRegion(definition, epoch) {
    const element = document.createElement("div");
    element.className = "region";
    applyRegionFrame(element, definition);

    // Subscriptions are held per region, so tearing a region down releases
    // exactly what it opened. A screen left on for weeks recomposes often, and
    // a shared list would keep every dead listener alive.
    const bucket = [];
    const instance = hasBlock(definition.block)
      ? createBlock(definition.block, element, this.contextFor(definition, epoch, bucket))
      : createMissingBlock(definition.block, element);

    instance.mount();
    this.root.append(element);
    // A region that arrives after the watcher was installed still gets watched.
    if (this.observer) this.observer.observe(element);
    return { definition, element, instance, bucket };
  }

  /**
   * The context a block is given.
   *
   * Blocks get data and a way to subscribe — never a fetch, never the store
   * itself. Keeping the surface this small is what stops blocks coupling to
   * each other's internals.
   */
  contextFor(definition, epoch, bucket) {
    const store = this.store;
    const hooks = this.hooks;
    const region = this.regions.get(definition.id);
    const listeners = bucket ?? region?.bucket ?? [];
    return {
      region: definition,
      theme: this.theme,
      /**
       * Read the world. `data()` is the whole snapshot; `data("signals")` is one
       * domain. A block names a domain — `signals`, `sources`, `world` — never
       * an endpoint, which is what keeps the network out of the blocks.
       */
      data: (name) => {
        const snapshot = store.snapshot();
        return name ? snapshot[name] : snapshot;
      },
      /** A domain's state: loading | ready | stale | error | unavailable. */
      state: (name) => store.dataState(name),
      /** Whether a domain holds a value worth drawing. */
      hasData: (name) => store.hasData(name),
      /** Subscribe to a bus topic, released when the region is torn down. */
      on: (topic, fn) => {
        const off = store.bus.on(topic, fn);
        listeners.push(off);
        return off;
      },
      /** Read a resource through the store, e.g. a signal's evidence. */
      load: {
        signal: (id) => store.loadSignal(id),
        source: (id) => store.loadSource(id),
        event: (id) => store.loadEvent(id),
        timeline: (key) => store.loadTimeline(key),
        observation: (id) => store.loadObservation(id),
        raw: (id) => store.loadRaw(id),
      },
      /** Ask the studio to change scene, e.g. from a feed row. */
      open: (sceneId, id) => hooks?.open?.(sceneId, id),
      /** Update the current selection, e.g. from a clicked evidence row. */
      select: (patch) => store.select(patch),
      bus: store.bus,
      /** True once the region has been replaced, so a late async reply is dropped. */
      stale: () => epoch !== this.epoch,
      epoch,
    };
  }

  /** Tear a region down: block first, then the listeners it registered. */
  disposeRegion(region) {
    region.instance.unmount();
    this.observer?.unobserve(region.element);
    this.sizes?.delete(region.element);
    region.element.remove();
    for (const off of region.bucket ?? []) off();
    if (region.bucket) region.bucket.length = 0;
  }

  /** Push new data into every region. Called on each store change. */
  update() {
    const epoch = this.epoch;
    for (const [, region] of this.regions) {
      region.instance.update(this.contextFor(region.definition, epoch));
    }
  }

  /** Tell one region its box changed. */
  resizeRegion(id) {
    const region = this.regions.get(id);
    if (!region) return;
    region.instance.resize(this.contextFor(region.definition, this.epoch));
  }

  /** Tell every region its box changed. */
  resize() {
    const epoch = this.epoch;
    for (const [, region] of this.regions) {
      region.instance.resize(this.contextFor(region.definition, epoch));
    }
  }

  /** Replace the scene's regions in place, keeping block identity where possible. */
  setRegions(regions) {
    if (!this.scene) return false;
    return this.compose({ ...this.scene, regions });
  }

  /** Tear the scene down completely. */
  clear() {
    for (const [, region] of this.regions) this.disposeRegion(region);
    this.regions.clear();
    this.unwatchResize();
    this.root?.remove();
    this.root = null;
    this.scene = null;
    for (const off of this.unsubscribes) off();
    this.unsubscribes = [];
  }

  /** The scene that was on air before the current one. */
  previousSceneId() {
    return this.history.length > 1 ? this.history[this.history.length - 2] : null;
  }
}
