/* The live activity stream.
 *
 * What the engine is doing right now, line by line: an observation stored, a
 * collector that failed, a source that recovered. It is the one place where a
 * collector failure is visible as itself rather than as an absence of data,
 * which is why the failure rows are stated in words and not merely tinted.
 *
 * Rows are capped. A screen left on for weeks must not accumulate DOM until it
 * dies; the oldest fall off the top. */

import { el, replace } from "../dom.js";
import { t } from "../i18n.js";
import { clock, stamp } from "../fmt.js";

const MAX_ROWS = 80;

const KIND = {
  OBSERVATION: { glyph: "·", label: "observation" },
  SIGNAL: { glyph: "◇", label: "signal" },
  EVENT: { glyph: "◆", label: "event" },
  COLLECTOR_FAILED: { glyph: "▲", label: "collector failed" },
  COLLECTOR_RECOVERED: { glyph: "●", label: "collector recovered" },
  SOURCE_FAILED: { glyph: "▲", label: "source failed" },
  SOURCE_RECOVERED: { glyph: "●", label: "source recovered" },
  COLLECTION: { glyph: "◐", label: "collection" },
};

export const activityStreamBlock = {
  mount(host, ctx) {
    host.append(el("div", { class: "region-head" }, [
      el("span", { class: "label", text: t().action.stream }),
      el("span", { class: "region-head-meta num", dataset: { role: "count" } }),
    ]), el("div", { class: "activity-rows", role: "log", "aria-live": "off" }));
    this.render(host, ctx);
  },
  update(host, ctx) { this.render(host, ctx); },
  render(host, ctx) {
    const items = (ctx.data().activity ?? []).slice(0, MAX_ROWS);
    const rows = host.querySelector(".activity-rows");
    const count = host.querySelector('[data-role="count"]');
    if (count) count.textContent = items.length ? `${items.length}` : "";

    if (!items.length) {
      replace(rows, [el("div", { class: "region-empty" }, [
        el("div", { class: "region-empty-title", text: t().state.noData }),
        el("div", { class: "region-empty-body", text: t().reason.quietFeed }),
      ])]);
      return;
    }
    replace(rows, items.map(row));
  },
  unmount(host) { host.replaceChildren(); },
};

function row(item) {
  const kind = KIND[item.kind] || { glyph: "·", label: String(item.kind || "").toLowerCase() };
  const failed = item.kind?.includes("FAILED");
  return el("div", { class: "activity-row", dataset: { kind: item.kind, alert: failed ? "1" : "0" } }, [
    el("span", { class: "activity-glyph", "aria-hidden": "true", text: kind.glyph }),
    el("span", { class: "activity-time num faint", text: clock(item.at) }),
    el("span", { class: "activity-kind label", text: kind.label }),
    el("span", { class: "activity-message", text: item.message || "" }),
  ]);
}
