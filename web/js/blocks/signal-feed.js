/* The live signal feed.
 *
 * One row per active signal, ordered by broadcast priority so the loudest thing
 * is always at the top of the column a viewer's eye reaches first. Severity is
 * carried by a glyph, an edge rule and a word as well as a hue, because a wall
 * display is read from too far away for colour alone.
 *
 * A resolved signal stays in the list but reads as history: it keeps its row
 * rather than vanishing, so a viewer who looked away for a minute can still see
 * what just ended. */

import { el, replace } from "../dom.js";
import { t, typeLabel, statusLabel } from "../i18n.js";
import { duration, sigma, clock } from "../fmt.js";
import { grade, largestDeviation, PRIORITY } from "../studio/priority.js";

const GLYPH = { CRITICAL: "◆", HIGH: "◇", MEDIUM: "▸", LOW: "·", INFO: "·" };

export const signalFeedBlock = {
  mount(host, ctx) {
    const list = el("div", { class: "feed-list", role: "list" });
    host.append(el("div", { class: "region-head" }, [
      el("span", { class: "label", text: t().field.signals }),
      el("span", { class: "region-head-meta", dataset: { role: "count" } }),
    ]), list);
    this.render(host, ctx);
  },
  update(host, ctx) { this.render(host, ctx); },
  render(host, ctx) {
    const data = ctx.data();
    const signals = [...(data.world?.now ?? data.signals ?? [])]
      .map((signal) => ({ signal, grade: grade(signal) }))
      .sort((a, b) => b.grade.score - a.grade.score);

    const count = host.querySelector('[data-role="count"]');
    if (count) {
      const active = signals.filter((s) => s.signal.status !== "RESOLVED").length;
      count.textContent = signals.length ? `${active}/${signals.length}` : "";
    }

    const list = host.querySelector(".feed-list");
    if (!signals.length) {
      replace(list, [emptyRow(data)]);
      return;
    }
    replace(list, signals.map((entry) => row(entry, ctx)));
  },
  unmount(host) { host.replaceChildren(); },
};

function emptyRow(data) {
  const collecting = data.control?.collection_enabled;
  const detail = collecting === false ? t().reason.collectorFailed : t().reason.quietFeed;
  return el("div", { class: "region-empty" }, [
    el("div", { class: "region-empty-title", text: t().state.quiet }),
    el("div", { class: "region-empty-body", text: detail }),
  ]);
}

function row({ signal, grade: graded }, ctx) {
  const resolved = signal.status === "RESOLVED";
  const deviation = largestDeviation(signal);

  const head = el("div", { class: "feed-row-head" }, [
    el("span", { class: "feed-glyph", "aria-hidden": "true", text: GLYPH[graded.level] || "·" }),
    ...(signal.types ?? []).slice(0, 2).map((type) =>
      el("span", { class: "type-badge", dataset: { type }, text: typeLabel(type) })),
    el("span", { class: "feed-status", text: statusLabel(signal.status) }),
  ]);

  const title = el("div", { class: "feed-title", text: signal.title });

  const facts = el("div", { class: "feed-facts" }, [
    fact(t().field.deviation, sigma(deviation)),
    fact(t().field.persistence, duration(signal.duration_seconds)),
    fact(t().field.evidence, String((signal.evidence ?? []).length)),
  ]);

  const node = el("article", {
    class: "feed-row",
    dataset: { level: graded.level, resolved: resolved ? "1" : "0" },
    role: "listitem",
    tabindex: "0",
    "aria-label": `${signal.title}. ${graded.level}. ${graded.why}`,
    on: {
      click: () => ctx.open?.("signal", signal.id),
      keydown: (event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          ctx.open?.("signal", signal.id);
        }
      },
    },
  }, [head, title, facts]);

  return node;
}

function fact(label, value) {
  return el("span", { class: "feed-fact" }, [
    el("b", { text: label }),
    el("span", { class: "num", text: value }),
  ]);
}

export { PRIORITY };
