/* A series over time, against its own baseline.
 *
 * The band is the engine's own p05–p95 range and the rule is its mean. Drawing
 * them is what turns a line into a statement: the viewer can see how far the
 * latest reading has travelled from normal, which is the question the whole
 * project exists to answer.
 *
 * When the engine reports no baseline for a series, no band is drawn and the
 * block says so. A band invented from the visible points would look identical
 * to a real one and would be a lie about what was measured. */

import { el, replace } from "../dom.js";
import { t } from "../i18n.js";
import { count, measure, seriesLabel, VOID } from "../fmt.js";
import { bandNode } from "./_chart.js";
import { grade } from "../studio/priority.js";

export const timelineBlock = {
  mount(host, ctx) {
    host.append(el("div", { class: "region-head" }, [
      el("span", { class: "label", text: t().field.baseline }),
      el("span", { class: "region-head-meta", dataset: { role: "series" } }),
    ]), el("div", { class: "timeline-box" }));
    this.render(host, ctx);
  },
  update(host, ctx) { this.render(host, ctx); },
  async render(host, ctx) {
    const data = ctx.data();
    const key = data.selection.seriesKey || pickSeries(data);
    const box = host.querySelector(".timeline-box");
    const label = host.querySelector('[data-role="series"]');

    if (!key) {
      if (label) label.textContent = "";
      replace(box, [el("div", { class: "region-empty" }, [
        el("div", { class: "region-empty-title", text: t().state.noData }),
        el("div", { class: "region-empty-body", text: t().why.noData }),
      ])]);
      return;
    }
    if (label) label.textContent = seriesLabel(key);

    let timeline;
    try {
      timeline = await ctx.load.timeline(key);
    } catch (err) {
      replace(box, [el("div", { class: "region-empty" }, [
        el("div", { class: "region-empty-title", text: t().state.unavailable }),
        el("div", { class: "region-empty-body", text: err.message }),
      ])]);
      return;
    }
    if (ctx.stale()) return;

    const observations = timeline?.observations ?? [];
    const baseline = timeline?.baseline ?? null;
    const chart = bandNode(observations, baseline, { width: 600, height: 190 });

    if (!chart) {
      replace(box, [el("div", { class: "region-empty" }, [
        el("div", { class: "region-empty-title", text: t().state.noData }),
        el("div", {
          class: "region-empty-body",
          text: `${count(observations.length)} ${t().field.observations} — two are needed to draw a line`,
        }),
      ])]);
      return;
    }

    const stats = el("div", { class: "timeline-stats" }, [
      stat(t().field.current, lastValue(observations)),
      stat("mean", baseline ? measure(baseline.mean, null, 2) : null),
      stat("p05", baseline ? measure(baseline.p05, null, 2) : null),
      stat("p95", baseline ? measure(baseline.p95, null, 2) : null),
      stat("n", baseline?.sample_size != null ? count(baseline.sample_size) : count(observations.length)),
    ]);

    const bandNote = baseline
      ? el("div", { class: "timeline-note faint", text: `${t().field.baseline}: p05–p95` })
      : el("div", { class: "timeline-note faint", text: `${t().field.baseline}: ${t().state.notMeasured}` });

    replace(box, [chart, stats, bandNote]);
  },
  unmount(host) { host.replaceChildren(); },
};

function pickSeries(data) {
  const signals = data.world?.now ?? data.signals ?? [];
  if (!signals.length) return data.observatory?.categories?.[0]?.series_key ?? null;
  return [...signals].sort((a, b) => grade(b).score - grade(a).score)[0].series_key ?? null;
}

function lastValue(observations) {
  const last = observations[observations.length - 1];
  if (!last) return VOID;
  return measure(last.value, last.unit, 3);
}

function stat(label, value) {
  return el("div", { class: "timeline-stat" }, [
    el("span", { class: "label", text: label }),
    el("span", { class: "num", text: value ?? VOID }),
  ]);
}
