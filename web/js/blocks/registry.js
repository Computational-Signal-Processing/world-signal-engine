/* Block registry.
 *
 * The studio core never imports a block. It looks one up by name here, which is
 * what makes "add a region with a different block" a configuration change
 * rather than a code change: dropping a new file in `blocks/` and calling
 * `registerBlock` is the whole cost of a new visual.
 *
 * Every block gets the same lifecycle. The registry wraps the block's own
 * functions so that a block cannot forget to tear down what it started — the
 * wrapper owns the cleanup bag and disposes it on unmount, and it guards
 * against a block being mounted twice into the same region. */

import { cleanupBag } from "../dom.js";

const blocks = new Map();

/**
 * Register a block.
 *
 * @param {string} name  identifier used by scene JSON
 * @param {object} spec
 * @param {(host: HTMLElement, ctx: object) => void} spec.mount
 * @param {(host: HTMLElement, ctx: object) => void} [spec.update]
 * @param {(host: HTMLElement, ctx: object) => void} [spec.resize]
 * @param {(host: HTMLElement, ctx: object) => void} [spec.unmount]
 */
export function registerBlock(name, spec) {
  if (typeof name !== "string" || !name) throw new Error("registerBlock: a name is required");
  if (typeof spec?.mount !== "function") throw new Error(`registerBlock("${name}"): mount() is required`);
  blocks.set(name, spec);
}

export function hasBlock(name) { return blocks.has(name); }
export function blockNames() { return [...blocks.keys()].sort(); }

/** A mounted block instance, owned by one region. */
class BlockInstance {
  constructor(name, spec, host, ctx) {
    this.name = name;
    this.spec = spec;
    this.host = host;
    this.ctx = ctx;
    this.bag = cleanupBag();
    // Per-instance state, prototyped on the spec. A block writes to `this`
    // (a timer, a subscription, a cached response) and two regions of the same
    // block must not share those — but its methods are the same everywhere, so
    // they stay on the prototype and only the data lands on the instance.
    this.state = Object.create(spec);
    this.mounted = false;
  }

  mount() {
    if (this.mounted) return;
    this.mounted = true;
    // The bag is handed to the block so any interval, observer or listener it
    // opens is registered for teardown. A screen left on for weeks cannot leak
    // one timer per update.
    this.ctx.cleanup = this.bag;
    try {
      this.spec.mount.call(this.state, this.host, this.ctx);
    } catch (err) {
      this.fail(err);
    }
  }

  update(ctx) {
    if (!this.mounted) return;
    this.ctx = ctx;
    this.ctx.cleanup = this.bag;
    try {
      this.spec.update?.call(this.state, this.host, this.ctx);
    } catch (err) {
      this.fail(err);
    }
  }

  resize(ctx) {
    if (!this.mounted) return;
    this.ctx = ctx;
    this.ctx.cleanup = this.bag;
    try {
      this.spec.resize?.call(this.state, this.host, this.ctx);
    } catch (err) {
      this.fail(err);
    }
  }

  unmount() {
    if (!this.mounted) return;
    this.mounted = false;
    try {
      this.spec.unmount?.call(this.state, this.host, this.ctx);
    } catch (err) {
      this.fail(err);
    }
    this.bag.dispose();
    this.host.replaceChildren();
  }

  /**
   * A block that throws is reported in place rather than taking the screen
   * down. One broken visual must not black out the broadcast.
   */
  fail(err) {
    console.error(`[block:${this.name}]`, err);
    this.host.dataset.error = "1";
    this.host.replaceChildren(errorCard(this.name, err));
  }
}

function errorCard(name, err) {
  const box = document.createElement("div");
  box.className = "region-error";
  const title = document.createElement("div");
  title.className = "region-error-title";
  title.textContent = `${name} failed`;
  const body = document.createElement("div");
  body.className = "region-error-body";
  body.textContent = err?.message || String(err);
  box.append(title, body);
  return box;
}

/** Create an instance for a region. The caller owns mounting and unmounting. */
export function createBlock(name, host, ctx) {
  const spec = blocks.get(name);
  if (!spec) throw new Error(`unknown block "${name}"`);
  return new BlockInstance(name, spec, host, ctx);
}

/** A stub used when a scene names a block that is not registered yet. */
export function createMissingBlock(name, host) {
  return {
    name,
    mount() {
      host.dataset.missing = "1";
      const box = document.createElement("div");
      box.className = "region-missing";
      const title = document.createElement("div");
      title.className = "region-missing-title";
      title.textContent = name;
      const note = document.createElement("div");
      note.className = "region-missing-note";
      note.textContent = "block not registered";
      box.append(title, note);
      host.replaceChildren(box);
    },
    update() {},
    resize() {},
    unmount() { host.replaceChildren(); },
  };
}
