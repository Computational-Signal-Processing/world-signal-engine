/* The live event stream.
 *
 * Server-Sent Events rather than polling, because the studio must react to a
 * signal the moment it forms. The reader is deliberately dumb: it parses frames
 * and publishes them. Deciding what an event *means* for the broadcast is the
 * director's job, and keeping that out of here is what stops this file growing
 * an `if (kind === ...)` ladder.
 *
 * Reconnection is handled here, with backoff and a resync on reconnect: a
 * signal that formed while the stream was down must not be missed, and a
 * duplicate must not be shown twice. */

import { EventBus } from "../studio/event-bus.js";

const MIN_BACKOFF = 1_000;
const MAX_BACKOFF = 15_000;

export class ActivityStream {
  /**
   * @param {object} options
   * @param {string} options.path        stream endpoint
   * @param {() => string} options.apiKey
   * @param {() => void} options.onResync  called after a reconnect, to refill state
   */
  constructor({ path = "/events", apiKey = () => "", onResync = null } = {}) {
    this.path = path;
    this.apiKey = apiKey;
    this.onResync = onResync;
    this.bus = new EventBus();
    this.controller = null;
    this.seen = new Set();          // event ids, to drop replayed frames
    this.seenOrder = [];
    this.state = "connecting";
    this.lastEventAt = null;
  }

  start() {
    if (this.controller) return;
    this.controller = new AbortController();
    this.run(this.controller);
  }

  stop() {
    if (!this.controller) return;
    this.controller.abort();
    this.controller = null;
  }

  setState(state) {
    if (this.state === state) return;
    this.state = state;
    this.bus.emit("state", state);
  }

  async run(controller) {
    let backoff = MIN_BACKOFF;
    while (!controller.signal.aborted) {
      try {
        const headers = { accept: "text/event-stream" };
        const key = this.apiKey();
        if (key) headers.Authorization = `Bearer ${key}`;
        const response = await fetch(this.path, { headers, signal: controller.signal });

        if (response.status === 401 || response.status === 403) {
          // Auth is a configuration problem, not a transient one. Retrying
          // would hammer the engine while showing nothing new.
          this.setState("error");
          return;
        }
        if (!response.ok || !response.body) throw new Error(`HTTP ${response.status}`);

        const reconnected = this.state === "reconnecting";
        this.setState("live");
        backoff = MIN_BACKOFF;
        if (reconnected && this.onResync) {
          try { await this.onResync(); } catch (_) { /* the next pass will retry */ }
        }
        await this.read(response.body, controller.signal);
        this.setState("reconnecting");
      } catch (err) {
        if (controller.signal.aborted) return;
        this.setState(err?.name === "TypeError" ? "offline" : "reconnecting");
      }
      if (controller.signal.aborted) return;
      await sleep(backoff);
      backoff = Math.min(backoff * 2, MAX_BACKOFF);
    }
  }

  async read(body, signal) {
    const reader = body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";
    while (!signal.aborted) {
      const { done, value } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      // Frames are separated by a blank line; the tail may be a partial frame.
      const frames = buffer.split("\n\n");
      buffer = frames.pop();
      for (const frame of frames) this.dispatch(frame);
    }
  }

  dispatch(frame) {
    const parsed = parseFrame(frame);
    if (!parsed) return;
    // The engine assigns each event an id. A reconnect replays recent events, so
    // anything already seen is dropped rather than appended a second time.
    if (parsed.id) {
      if (this.seen.has(parsed.id)) return;
      this.seen.add(parsed.id);
      this.seenOrder.push(parsed.id);
      if (this.seenOrder.length > 500) this.seen.delete(this.seenOrder.shift());
    }
    this.lastEventAt = Date.now();
    this.bus.emit("activity", parsed.data);
    if (parsed.event) this.bus.emit(`kind:${parsed.event}`, parsed.data);
  }
}

/** Parse one SSE frame into `{ id, event, data }`, or null if it carries none. */
export function parseFrame(frame) {
  let id = null;
  let event = null;
  let data = "";
  for (const line of frame.split("\n")) {
    if (line.startsWith(":")) continue;            // comment / keep-alive
    const colon = line.indexOf(":");
    if (colon === -1) continue;
    const field = line.slice(0, colon);
    const value = line.slice(colon + 1).trim();
    if (field === "id") id = value;
    else if (field === "event") event = value;
    else if (field === "data") data += value;
  }
  if (!data) return null;
  try {
    return { id, event, data: JSON.parse(data) };
  } catch (_) {
    return null;
  }
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
