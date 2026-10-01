/* The breaking takeover.
 *
 * The screen is handed over, not overlaid. When a signal is loud enough the
 * broadcast stops being a picture of the world and becomes a statement about
 * one thing: what changed, how far from normal, how long it has run, and how
 * much evidence stands behind it.
 *
 * The sequence is deliberate and slow enough to read:
 *
 *   pre-alert    a frame pulses at the edge, so the cut is not a surprise
 *   transition   the outgoing scene dims and recedes
 *   takeover     the alert owns the screen
 *   reveal       the evidence numbers arrive one after another
 *   close        a visible countdown, then the previous scene returns
 *
 * Nothing flashes. Repeated high-contrast flashing is fatiguing on a wall and a
 * photosensitivity hazard, so the alert pulses slowly and only while it is
 * arriving. Under `prefers-reduced-motion` every step is instant.
 *
 * Every figure shown is one the engine reported. A missing figure is an em
 * dash with the reason, never a placeholder that looks like a measurement. */

import { el, replace } from "../dom.js";
import { t, typeLabel, statusLabel } from "../i18n.js";
import { duration, ratio, sigma, stamp, VOID } from "../fmt.js";
import { largestDeviation } from "../studio/priority.js";

const REVEAL_STEP = 260;

export class BreakingLayer {
  /**
   * @param {HTMLElement} host
   * @param {object} options
   * @param {() => void} options.onOpen    "open report"
   * @param {() => void} options.onClose   "close"
   */
  constructor(host, { onOpen, onClose } = {}) {
    this.host = host;
    this.onOpen = onOpen;
    this.onClose = onClose;
    this.root = null;
    this.timers = [];
    this.countdownTimer = null;
    this.active = false;
    this.signal = null;
  }

  /** Begin the takeover. Returns when the alert owns the screen. */
  async show(signal, { seconds = 30 } = {}) {
    if (this.active) this.dismiss({ silent: true });
    this.active = true;
    this.signal = signal;

    const reduced = prefersReduced();
    if (!reduced) await this.preAlert();

    this.root = this.build(signal, seconds);
    this.host.append(this.root);
    document.body.dataset.takeover = "1";

    // The reveal is staggered so the eye is led through the evidence in order
    // rather than shown a wall of numbers at once.
    const revealables = [...this.root.querySelectorAll("[data-reveal]")];
    revealables.forEach((node, index) => {
      if (reduced) { node.dataset.shown = "1"; return; }
      this.timers.push(setTimeout(() => { node.dataset.shown = "1"; }, index * REVEAL_STEP));
    });

    if (seconds > 0) this.startCountdown(seconds);
  }

  /** A slow pulse at the edge of the screen, so the cut is announced. */
  preAlert() {
    return new Promise((resolve) => {
      const frame = el("div", { class: "pre-alert", "aria-hidden": "true" });
      this.host.append(frame);
      const done = () => { frame.remove(); resolve(); };
      this.timers.push(setTimeout(done, 800));
    });
  }

  build(signal, seconds) {
    const deviation = largestDeviation(signal);

    const kicker = el("div", { class: "breaking-kicker" }, [
      el("span", { class: "breaking-dot", "aria-hidden": "true" }),
      el("span", { text: t().breaking.kicker }),
      el("span", { class: "breaking-live faint", text: t().breaking.live }),
    ]);

    this.countdownNode = el("div", { class: "breaking-countdown num", text: "" });
    const head = el("header", { class: "breaking-head" }, [kicker, this.countdownNode]);

    const types = el("div", { class: "breaking-types" },
      (signal.types ?? []).map((type) => el("span", { class: "type-badge", dataset: { type }, text: typeLabel(type) })));

    const title = el("h2", { class: "breaking-title display", text: signal.title });
    const summary = signal.summary ? el("p", { class: "breaking-summary dim", text: signal.summary }) : null;

    const hero = el("div", { class: "breaking-hero", "data-reveal": "" }, [
      el("div", { class: "breaking-sigma display num", text: sigma(deviation) }),
      el("div", { class: "breaking-sigma-label label", text: t().field.deviation }),
    ]);

    const grid = el("div", { class: "breaking-grid" }, [
      cell(t().field.evidence, String((signal.evidence ?? []).length), 0),
      cell(t().field.sourceCount, String(uniqueSources(signal)), 1),
      cell(t().field.duration, duration(signal.duration_seconds), 2),
      cell(t().field.confidence, ratio(signal.confidence), 3),
      cell(t().field.status, statusLabel(signal.status), 4),
      cell(t().field.firstSeen, stamp(signal.first_seen), 5),
    ]);

    const actions = el("div", { class: "breaking-actions" }, [
      el("button", {
        class: "breaking-button primary",
        type: "button",
        text: t().action.open,
        on: { click: () => { this.onOpen?.(signal); } },
      }),
      el("button", {
        class: "breaking-button",
        type: "button",
        text: t().action.close,
        on: { click: () => this.dismiss() },
      }),
    ]);

    return el("div", {
      class: "breaking",
      role: "alertdialog",
      "aria-modal": "true",
      "aria-label": `${t().breaking.kicker}: ${signal.title}`,
    }, [
      el("div", { class: "breaking-panel" }, [head, types, title, summary, hero, grid, actions].filter(Boolean)),
    ]);
  }

  startCountdown(seconds) {
    let remaining = seconds;
    const tick = () => {
      remaining -= 1;
      if (this.countdownNode) {
        this.countdownNode.textContent = `${t().breaking.closing} ${String(Math.max(0, remaining)).padStart(2, "0")}`;
      }
      if (remaining <= 0) this.dismiss();
    };
    this.countdownNode.textContent = `${t().breaking.closing} ${String(seconds).padStart(2, "0")}`;
    this.countdownTimer = setInterval(tick, 1000);
  }

  /** Hand the screen back. */
  dismiss({ silent = false } = {}) {
    this.clearTimers();
    const wasActive = this.active;
    this.active = false;
    const signal = this.signal;
    this.signal = null;
    this.root?.remove();
    this.root = null;
    delete document.body.dataset.takeover;
    if (wasActive && !silent) this.onClose?.(signal);
  }

  clearTimers() {
    for (const id of this.timers) clearTimeout(id);
    this.timers = [];
    if (this.countdownTimer) clearInterval(this.countdownTimer);
    this.countdownTimer = null;
  }

  dispose() {
    this.dismiss({ silent: true });
  }
}

function cell(label, value, index) {
  return el("div", { class: "breaking-cell", "data-reveal": "", style: { "--reveal-order": String(index) } }, [
    el("span", { class: "label", text: label }),
    el("span", { class: value === VOID ? "void num" : "num", text: value }),
  ]);
}

function uniqueSources(signal) {
  const ids = new Set();
  for (const row of signal.evidence ?? []) if (row.source_id) ids.add(row.source_id);
  return ids.size;
}

function prefersReduced() {
  return typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;
}
