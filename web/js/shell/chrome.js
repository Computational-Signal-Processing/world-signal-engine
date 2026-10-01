/* The chrome: the only persistent UI, and it is metadata rather than navigation.
 *
 * It answers four questions a viewer of a live picture needs answered — is this
 * live, what system is this, which scene is on air, what time is it — and
 * nothing else. There are no links here on purpose: navigating a broadcast by
 * clicking is what made the previous UI read as a website. Scenes change
 * through the director or the control room. */

import { el } from "../dom.js";
import { clock } from "../fmt.js";

export class Chrome {
  /**
   * @param {HTMLElement} host
   * @param {object} options
   * @param {(connection: object) => string} options.connectionLabel
   */
  constructor(host, { connectionLabel, brand = "WORLD SIGNAL ENGINE" } = {}) {
    this.host = host;
    this.connectionLabel = connectionLabel;

    this.mark = el("span", { class: "chrome-mark", "aria-hidden": "true" });
    this.name = el("span", { class: "chrome-name", text: brand });
    this.brandBox = el("div", { class: "chrome-brand" }, [this.mark, this.name]);

    this.sceneName = el("span", { class: "chrome-scene-name", text: "—" });
    this.sceneBox = el("div", { class: "chrome-scene" }, [this.sceneName]);

    this.dot = el("span", { class: "live-dot", "aria-hidden": "true" });
    this.liveText = el("span", { class: "live-text", text: "connecting" });
    this.badge = el("div", {
      class: "live-badge",
      dataset: { state: "connecting" },
      role: "status",
      "aria-live": "polite",
    }, [this.dot, this.liveText]);

    this.clockText = el("span", { class: "num", text: "--:--:--" });
    this.zone = el("span", { class: "zone", text: "UTC" });
    this.clockBox = el("div", { class: "chrome-clock" }, [this.clockText, this.zone]);

    this.right = el("div", { class: "chrome-right" }, [this.badge, this.clockBox]);

    this.root = el("header", { class: "chrome", role: "banner" }, [this.brandBox, this.sceneBox, this.right]);
    host.append(this.root);

    this.timer = null;
  }

  start() {
    if (this.timer) return;
    this.tick();
    // Once a second is enough for a wall clock, and it keeps a screen that runs
    // for weeks from doing needless work.
    this.timer = setInterval(() => this.tick(), 1000);
  }

  stop() {
    if (this.timer) clearInterval(this.timer);
    this.timer = null;
  }

  tick() {
    const now = new Date();
    this.clockText.textContent = `${String(now.getUTCHours()).padStart(2, "0")}:${String(now.getUTCMinutes()).padStart(2, "0")}:${String(now.getUTCSeconds()).padStart(2, "0")}`;
  }

  /** Name the scene on air. */
  setScene(label) {
    this.sceneName.textContent = label || "—";
  }

  /**
   * Reflect connection state.
   *
   * The word changes as well as the colour. A wall display is often seen from
   * too far to read a hue, and a recorded picture must never be mistakable for
   * a live one.
   */
  setConnection(connection) {
    const state = connection?.state ?? "connecting";
    this.badge.dataset.state = state;
    this.liveText.textContent = this.connectionLabel(connection);
  }

  /** Show the time of the last frame received, for a stalled stream. */
  setStaleSince(iso) {
    if (!iso) return;
    this.clockBox.title = `last event ${clock(iso)}`;
  }
}
