/* The metric column: the engine's headline counts, plus the type breakdown.
 *
 * Every number here is a counter the engine actually exposes. Where it exposes
 * nothing the row says "not measured" rather than showing a zero, because a
 * zero is a reading and the absence of a reading is not. */

import { el, replace } from "../dom.js";
import { t, typeLabel } from "../i18n.js";
import { count, clock } from "../fmt.js";

export const metricGroupBlock = {
  mount(host, ctx) {
    host.append(
      el("div", { class: "region-head" }, [
        el("span", { class: "label", text: t().action.system }),
        el("span", { class: "region-head-meta", dataset: { role: "stamp" } }),
      ]),
      el("div", { class: "metric-grid" }),
      el("div", { class: "region-subhead label", text: t().field.signals }),
      el("div", { class: "type-box" }),
    );
    this.render(host, ctx);
  },
  update(host, ctx) { this.render(host, ctx); },
  render(host, ctx) {
    const data = ctx.data();
    const built = buildMetrics(data);
    replace(host.querySelector(".metric-grid"), built.cards.map(metricCard));
    replace(host.querySelector(".type-box"), [typeBreakdown(built.types)]);
    const stamp = host.querySelector('[data-role="stamp"]');
    if (stamp) stamp.textContent = data.world?.generated_at ? clock(data.world.generated_at) : "";
  },
  unmount(host) { host.replaceChildren(); },
};

function buildMetrics(data) {
  const world = data.world;

  const cards = [
    {
      key: "signals",
      label: t().field.signals,
      value: world ? count(world.active_signals) : null,
      note: world ? `${count(world.signals_total)} ${t().action.elapsed}` : t().state.unavailable,
      level: null,
    },
    {
      key: "observations",
      label: t().field.observations,
      value: world ? count(world.observations_total) : null,
      note: t().field.value,
      level: null,
    },
    {
      key: "events",
      label: t().field.events,
      value: world ? count(world.events_total) : null,
      note: t().field.value,
      level: null,
    },
    {
      key: "sources",
      label: t().action.sourcesHealthy,
      value: world ? `${count(world.sources_healthy)}/${count(world.sources_total)}` : null,
      note: t().reason.collectorFailed,
      level: world && world.sources_healthy < world.sources_total ? "warn" : "ok",
    },
  ];

  return { cards, types: world?.by_type ?? [] };
}

function metricCard(card) {
  const value = card.value == null
    ? el("div", { class: "metric-value void", text: "—" })
    : el("div", { class: "metric-value num", text: card.value });

  return el("div", { class: "metric-card", dataset: { level: card.level || "none", key: card.key } }, [
    el("div", { class: "metric-label label", text: card.label }),
    value,
    card.note ? el("div", { class: "metric-note faint", text: card.note }) : null,
  ]);
}

/** The type breakdown, drawn as a bar per type so the zero rows are visible. */
export function typeBreakdown(byType) {
  if (!byType?.length) {
    return el("div", { class: "region-empty" }, [
      el("div", { class: "region-empty-title", text: t().state.notMeasured }),
      el("div", { class: "region-empty-body", text: t().why.notMeasured }),
    ]);
  }
  const max = Math.max(...byType.map((row) => row.count), 1);
  return el("div", { class: "type-rows" }, byType.map((row) => el("div", {
    class: "type-row",
    dataset: { type: row.type, empty: row.count === 0 ? "1" : "0" },
  }, [
    el("span", { class: "type-name", text: typeLabel(row.type) }),
    el("span", { class: "type-track" }, [
      el("span", {
        class: "type-fill",
        style: { width: `${Math.round((row.count / max) * 100)}%` },
      }),
    ]),
    el("span", { class: "type-count num", text: count(row.count) }),
  ])));
}
