/* A minimal synchronous event bus.
 *
 * The director, the store and the SSE reader all need to announce things
 * without knowing who is listening. Handlers run synchronously so that an
 * update arriving from the stream is applied before the next one is read; a
 * broadcast screen should never show two states at once because a listener was
 * scheduled for later. */

export class EventBus {
  constructor() {
    this.handlers = new Map();
  }

  /** Subscribe. Returns an unsubscribe function. */
  on(type, fn) {
    if (typeof fn !== "function") return () => {};
    let set = this.handlers.get(type);
    if (!set) { set = new Set(); this.handlers.set(type, set); }
    set.add(fn);
    return () => this.off(type, fn);
  }

  /** Subscribe for a single delivery. */
  once(type, fn) {
    const off = this.on(type, (payload) => { off(); fn(payload); });
    return off;
  }

  off(type, fn) {
    const set = this.handlers.get(type);
    if (!set) return;
    set.delete(fn);
    if (!set.size) this.handlers.delete(type);
  }

  /**
   * Publish. A throwing handler is contained: one broken block must not stop
   * the others from receiving the same event, and must not break the stream.
   */
  emit(type, payload) {
    const set = this.handlers.get(type);
    if (set) {
      for (const fn of [...set]) {
        try { fn(payload); } catch (err) { reportHandlerError(type, err); }
      }
    }
    const wildcard = this.handlers.get("*");
    if (wildcard) {
      for (const fn of [...wildcard]) {
        try { fn({ type, payload }); } catch (err) { reportHandlerError(type, err); }
      }
    }
  }

  /** How many listeners a type has. Used by the leak check in the control room. */
  size(type) {
    return type ? (this.handlers.get(type)?.size ?? 0) : [...this.handlers.values()].reduce((n, s) => n + s.size, 0);
  }

  clear() { this.handlers.clear(); }
}

function reportHandlerError(type, err) {
  // Surfaced on the console rather than swallowed: a silently failing listener
  // is the kind of bug that only shows up as a region that stopped updating.
  console.error(`[bus] handler for "${type}" threw`, err);
}
