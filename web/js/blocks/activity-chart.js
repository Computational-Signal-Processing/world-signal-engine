/* Activity over the last window.
 *
 * Observations arriving per hour, with the window's own mean drawn as a rule.
 * The comparison is the point: a spike means nothing until it is read against
 * the world's recent norm, and both come from the same buckets so the
 * comparison is honest.
 *
 * The window is whatever the engine says it is — it is stated on screen rather
 * than assumed, because a "24H" chart that was actually a different span would
 * mislead every reading taken from it. */

import { el, replace } from "../dom.js";
import { t } from "../i18n.js";
import { count } from "../fmt.js";
import { barsNode } from "./_chart.js";

export const activityChartBlock = {
  mount(host, ctx) {
    host.append(el("div", { class: "region-head" }, [
      el("span", { class: "label", text: t().field.observations }),
      el("span", { class: "region-head-meta", dataset: { role: "window" } }),
    ]), el("div", { class: "chart-box" }));
    this.render(host, ctx);
  },
  update(host, ctx) { this.render(host, ctx); },
  render(host, ctx) {
    const activity = ctx.data().observatory?.activity;
    const box = host.querySelector(".chart-box");
    const windowLabel = host.querySelector('[data-role="window"]');

    if (windowLabel) windowLabel.textContent = activity?.window ?? "";

    const built = activity?.buckets ? barsNode(activity.buckets, { width: 600, height: 160 }) : null;
    if (!built) {
      replace(box, [el("div", { class: "region-empty" }, [
        el("div", { class: "region-empty-title", text: t().state.notMeasured }),
        el("div", { class: "region-empty-body", text: t().why.notMeasured }),
      ])]);
      return;
    }

    const caption = el("div", { class: "chart-caption" }, [
      el("span", { class: "faint", text: `${built.count} ${t().action.measurements}` }),
      el("span", { class: "chart-mean faint" }, [
        el("span", { class: "mean-rule", "aria-hidden": "true" }),
        el("span", { text: `mean ${count(Math.round(built.mean * 10) / 10)}` }),
      ]),
      el("span", { class: "faint", text: `peak ${count(built.max)}` }),
    ]);

    replace(box, [built.node, caption]);
  },
  unmount(host) { host.replaceChildren(); },
};
