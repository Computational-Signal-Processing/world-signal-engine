/* One source, in full.
 *
 * The drill-down's fourth hop. It states what the source is, who publishes it,
 * how it is reached, how often it runs and what the engine has recorded about
 * its health. The health figures come from the engine's own tallies; where a
 * figure is absent the row says "not measured" rather than showing zero, and a
 * source that has never run is described as never having run. */

import { el, replace } from "../dom.js";
import { t } from "../i18n.js";
import { count, duration, latency, stamp, VOID } from "../fmt.js";

export const sourceCardBlock = {
  mount(host, ctx) {
    this.render(host, ctx);
  },
  update(host, ctx) { this.render(host, ctx); },
  async render(host, ctx) {
    const data = ctx.data();
    const id = data.selection.sourceId || pickSource(data);
    if (!id) {
      replace(host, [el("div", { class: "region-empty" }, [
        el("div", { class: "region-empty-title", text: t().field.sources }),
        el("div", { class: "region-empty-body", text: t().why.notMeasured }),
      ])]);
      return;
    }

    const summary = (data.sources ?? []).find((s) => s.id === id) ?? null;
    let health = null;
    try {
      health = await ctx.load.source(id);
    } catch (_) { /* the catalog entry alone is still worth showing */ }
    if (ctx.stale()) return;

    replace(host, nodes(id, summary, health));
  },
  unmount(host) { host.replaceChildren(); },
};

function pickSource(data) {
  const sources = data.control?.sources ?? [];
  if (!sources.length) return null;
  // Prefer a source that is misbehaving: it is the one worth looking at.
  const failing = sources.find((s) => s.consecutive_failures > 0 || !s.last_success);
  return (failing ?? sources[0]).source_id;
}

function nodes(id, summary, health) {
  const head = el("div", { class: "region-head" }, [
    el("span", { class: "label", text: t().field.sources }),
    el("span", { class: "region-head-meta", text: id }),
  ]);

  if (!summary) {
    return [head, el("div", { class: "region-empty" }, [
      el("div", { class: "region-empty-title", text: t().state.unavailable }),
      el("div", { class: "region-empty-body", text: `${id} — ${t().why.noData}` }),
    ])];
  }

  const title = el("h2", { class: "card-title title", text: summary.name });
  const provider = el("div", { class: "dim", text: summary.provider || VOID });

  const facts = el("dl", { class: "card-facts" }, [
    pair(t().field.category, summary.category || VOID),
    pair(t().field.provider, summary.provider || VOID),
    pair(t().field.cadence, cadenceText(summary)),
    pair(t().field.enabled, summary.enabled ? t().state.on : t().state.off),
    pair(t().field.latency, health?.health?.latency_ms != null ? latency(health.health.latency_ms) : null),
    pair(t().field.lastSuccess, health?.health?.last_success ? stamp(health.health.last_success) : null),
    pair(t().field.records, health?.health?.records_received != null ? count(health.health.records_received) : null),
    pair(t().field.duplicates, health?.health?.records_duplicate != null ? count(health.health.records_duplicate) : null),
    pair(t().field.errors, health?.health?.error_count != null ? count(health.health.error_count) : null),
  ]);

  const endpoint = summary.endpoint
    ? el("div", { class: "source-endpoint mono faint", text: summary.endpoint })
    : null;

  const license = summary.license
    ? el("div", { class: "source-license faint", text: `${summary.license}` })
    : null;

  return [head, title, provider, facts, endpoint, license].filter(Boolean);
}

function pair(label, value) {
  return el("div", { class: "card-fact" }, [
    el("dt", { class: "label", text: label }),
    el("dd", { class: value == null || value === VOID ? "void" : "num", text: value ?? VOID }),
  ]);
}

/**
 * How often a source is read, as text.
 *
 * The engine reports cadence two ways: `event` for a push feed and a structured
 * `{interval:{seconds}}` / `{daily:{hour_utc}}` for a polled one, alongside a
 * ready `cadence_label`. The label is preferred, and the structure is read
 * directly when it is absent, so the card never shows a raw object.
 */
function cadenceText(source) {
  if (source.cadence_label) return source.cadence_label;
  const cadence = source.cadence;
  if (!cadence) return VOID;
  if (typeof cadence === "string") return cadence;
  if (typeof cadence === "number") return `${cadence}s`;
  if (cadence.interval?.seconds) return duration(cadence.interval.seconds);
  if (cadence.daily) return `daily@${String(cadence.daily.hour_utc ?? 0).padStart(2, "0")}Z`;
  return VOID;
}
