/* Source health: the observatory's view of its own sensors.
 *
 * Not a settings table. Each source is a tile that states whether it is
 * answering, how often it runs, when it last succeeded and how many times it
 * has failed in a row — the facts that decide whether the picture above can be
 * trusted.
 *
 * The distinction the whole project rests on is enforced here visually: a
 * source that has never run and a source that runs and reports nothing are
 * different tiles. "Never ran" is stated in words, never drawn as a zero. */

import { el, replace } from "../dom.js";
import { t } from "../i18n.js";
import { ago, duration, stamp } from "../fmt.js";

export const sourceHealthBlock = {
  mount(host, ctx) {
    host.append(el("div", { class: "region-head" }, [
      el("span", { class: "label", text: t().field.sources }),
      el("span", { class: "region-head-meta", dataset: { role: "summary" } }),
    ]), el("div", { class: "health-grid" }));
    this.render(host, ctx);
  },
  update(host, ctx) { this.render(host, ctx); },
  render(host, ctx) {
    const data = ctx.data();
    const control = data.control;
    const grid = host.querySelector(".health-grid");
    const summary = host.querySelector('[data-role="summary"]');

    const sources = control?.sources ?? [];
    if (!sources.length) {
      replace(grid, [el("div", { class: "region-empty" }, [
        el("div", { class: "region-empty-title", text: t().state.unavailable }),
        el("div", { class: "region-empty-body", text: t().why.notMeasured }),
      ])]);
      if (summary) summary.textContent = "";
      return;
    }

    const healthy = sources.filter((s) => state(s) === "ok").length;
    if (summary) summary.textContent = `${healthy}/${sources.length}`;

    replace(grid, sources.map((source) => tile(source)));
  },
  unmount(host) { host.replaceChildren(); },
};

/**
 * Classify a source.
 *
 * `never` is its own state, not a failure and not a success: a collector that
 * has not run yet has told us nothing about the world.
 */
function state(source) {
  if (!source.enabled) return "off";
  if (source.consecutive_failures > 0) return "failing";
  if (!source.last_success) return "never";
  if (source.running) return "running";
  return "ok";
}

const STATE_LABEL = {
  ok: "●",
  running: "◐",
  failing: "▲",
  never: "○",
  off: "—",
};

function tile(source) {
  const status = state(source);

  return el("article", {
    class: "health-tile",
    dataset: { state: status },
    tabindex: "0",
    "aria-label": `${source.name}. ${status}`,
  }, [
    el("div", { class: "health-head" }, [
      el("span", { class: "health-glyph", "aria-hidden": "true", text: STATE_LABEL[status] }),
      el("span", { class: "health-name", text: source.name }),
    ]),
    el("div", { class: "health-meta faint", text: source.category || "" }),
    el("div", { class: "health-facts" }, [
      fact(t().field.cadence, source.cadence_seconds ? duration(source.cadence_seconds) : null),
      fact(t().field.lastSuccess, source.last_success ? stamp(source.last_success) : null),
      fact(t().field.errors, source.consecutive_failures || null, source.consecutive_failures > 0),
    ]),
    status === "never"
      ? el("div", { class: "health-note faint", text: t().state.neverRan })
      : null,
  ]);
}

function fact(label, value, warn = false) {
  return el("div", { class: "health-fact", dataset: { warn: warn ? "1" : "0" } }, [
    el("span", { class: "faint", text: label }),
    el("span", { class: "num", text: value == null ? "—" : String(value) }),
  ]);
}
