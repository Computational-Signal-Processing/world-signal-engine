/* The running headline strip.
 *
 * It is the broadcast's "son dakika": one line that keeps moving so a viewer
 * who looks up mid-sentence still learns what changed. It is driven by the
 * director — this file only knows how to render a list of headlines and how to
 * stop moving when the list is empty.
 *
 * The strip hides itself when there is nothing to say. An empty strip that
 * keeps sliding would be a picture of activity where there is none. */

import { el, replace } from "../dom.js";

export class Ticker {
  constructor(host, { label = "SON DAKİKA" } = {}) {
    this.host = host;
    this.badge = el("div", { class: "ticker-badge", text: label });
    this.track = el("div", { class: "ticker-track" });
    this.run = el("div", { class: "ticker-run" });
    this.track.append(this.run);
    this.root = el("div", { class: "ticker", role: "marquee", "aria-live": "off" }, [this.badge, this.track]);
    host.append(this.root);
    this.items = [];
  }

  /** Replace the headlines. An empty list hides the strip. */
  set(items) {
    this.items = items;
    if (!items.length) {
      this.root.hidden = true;
      return;
    }
    this.root.hidden = false;
    // The track is duplicated so the animation can loop seamlessly at -50%.
    const nodes = items.map((item) => this.itemNode(item));
    replace(this.run, [...nodes, ...nodes.map((n) => n.cloneNode(true))]);
    // A longer list needs longer to cross, or it would read as a blur.
    const seconds = Math.max(28, Math.min(120, items.length * 9));
    this.run.style.setProperty("--ticker-duration", `${seconds}s`);
  }

  itemNode(item) {
    const glyph = el("span", { class: "glyph", "aria-hidden": "true", text: item.glyph || "•" });
    const text = el("span", { text: item.text });
    return el("span", { class: "ticker-item" }, [glyph, text]);
  }

  /** Stop the animation, e.g. while a takeover owns the screen. */
  pause() { this.run.style.animationPlayState = "paused"; }
  resume() { this.run.style.animationPlayState = "running"; }
}
