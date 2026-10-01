/* One signal, in full.
 *
 * This is the screen a viewer lands on when they ask "why is this on the wall".
 * It states the deviation, how long it has run, how much evidence stands behind
 * it, and — when the engine says so — the sentence it wrote describing the
 * change. Nothing here is inferred: a field the engine did not report shows the
 * void dash with the reason beside it, never a plausible-looking number. */

import { el, replace } from "../dom.js";
import { t, typeLabel, typeHint, statusLabel, directionLabel } from "../i18n.js";
import { duration, ratio, sigma, stamp, VOID } from "../fmt.js";
import { grade, largestDeviation, quality } from "../studio/priority.js";
import { narrativeBlock } from "./narrative.js";

export const signalCardBlock = {
  mount(host, ctx) {
    this.render(host, ctx);
    this.watch(host, ctx);
  },
  update(host, ctx) {
    this.render(host, ctx);
    this.watch(host, ctx);
  },
  /** The selection is a store concern, so it is subscribed to rather than polled. */
  watch(host, ctx) {
    if (this.off) this.off();
    this.off = ctx.on("selection", () => this.render(host, ctx));
  },
  async render(host, ctx) {
    const data = ctx.data();
    const id = data.selection.signalId || pickDefault(data);
    if (!id) {
      replace(host, [empty()]);
      return;
    }
    let signal;
    try {
      signal = await ctx.load.signal(id);
    } catch (err) {
      replace(host, [failed(id, err)]);
      return;
    }
    if (ctx.stale() || !signal) return;
    replace(host, nodes(signal));
  },
  unmount(host) {
    if (this.off) { this.off(); this.off = null; }
    host.replaceChildren();
  },
};

/** With no explicit selection, the loudest signal is the one worth showing. */
function pickDefault(data) {
  const signals = data.world?.now ?? data.signals ?? [];
  if (!signals.length) return null;
  return [...signals].sort((a, b) => grade(b).score - grade(a).score)[0].id;
}

function empty() {
  return el("div", { class: "region-empty" }, [
    el("div", { class: "region-empty-title", text: t().state.quiet }),
    el("div", { class: "region-empty-body", text: t().reason.noSignal }),
  ]);
}

function failed(id, err) {
  return el("div", { class: "region-empty" }, [
    el("div", { class: "region-empty-title", text: `${id} — ${t().state.unavailable}` }),
    el("div", { class: "region-empty-body", text: err.message }),
  ]);
}

function nodes(signal) {
  const graded = grade(signal);
  const deviation = largestDeviation(signal);

  const header = el("header", { class: "card-head", dataset: { level: graded.level } }, [
    el("div", { class: "card-badges" },
      (signal.types ?? []).map((type) => el("span", {
        class: "type-badge",
        dataset: { type },
        title: typeHint(type),
        text: typeLabel(type),
      }))),
    el("span", { class: "card-status", text: statusLabel(signal.status) }),
  ]);

  const title = el("h2", { class: "card-title headline", text: signal.title });
  const summary = signal.summary
    ? el("p", { class: "card-summary dim", text: signal.summary })
    : null;

  const hero = el("div", { class: "card-hero" }, [
    el("div", { class: "card-hero-value display num", text: sigma(deviation) }),
    el("div", { class: "card-hero-label label", text: t().field.deviation }),
  ]);

  const facts = el("dl", { class: "card-facts" }, [
    pair(t().field.persistence, duration(signal.duration_seconds)),
    pair(t().field.evidence, String((signal.evidence ?? []).length)),
    pair(t().field.sourceCount, String(uniqueSources(signal).length)),
    pair(t().field.confidence, ratio(signal.confidence)),
    pair(t().field.direction, signal.direction ? directionLabel(signal.direction) : VOID),
    pair(t().field.firstSeen, stamp(signal.first_seen)),
    pair(t().field.lastUpdate, stamp(signal.last_updated)),
    pair(t().field.metric, signal.series_key || VOID),
  ]);

  const reasons = (signal.reasons ?? []).length
    ? el("section", { class: "card-why" }, [
      el("div", { class: "label", text: "why" }),
      el("ul", {}, signal.reasons.map((r) => el("li", { text: r }))),
    ])
    : null;

  const narrative = narrativeBlock(signal);

  const scores = scoreRows(signal);

  const origin = signal.data_origin
    ? el("div", { class: "card-origin faint", text: signal.data_origin })
    : null;

  return [header, title, summary, hero, facts, scores, reasons, narrative, origin].filter(Boolean);
}

/**
 * The engine's quality dimensions, each named and each void when unreported.
 *
 * These are shown as separate readings on purpose. A single "importance" number
 * would hide which of novelty, strength, persistence, confidence, breadth,
 * convergence and relevance is doing the work — and the project's whole claim is
 * that a signal can be explained rather than asserted.
 */
function scoreRows(signal) {
  const dimensions = ["novelty", "strength", "persistence", "confidence", "breadth", "convergence", "relevance"];
  const reported = dimensions
    .map((key) => ({ key, value: quality(signal, key) }))
    .filter((row) => row.value != null);
  if (!reported.length) return null;

  return el("section", { class: "card-scores" }, [
    el("div", { class: "label", text: t().field.quality }),
    el("div", { class: "score-rows" }, reported.map((row) => el("div", { class: "score-row" }, [
      el("span", { class: "score-name faint", text: row.key }),
      el("span", { class: "score-track" }, [
        el("span", { class: "score-fill", style: { width: `${Math.round(row.value * 100)}%` } }),
      ]),
      el("span", { class: "score-value num", text: row.value.toFixed(2) }),
    ]))),
  ]);
}

function pair(label, value) {
  return el("div", { class: "card-fact" }, [
    el("dt", { class: "label", text: label }),
    el("dd", { class: "num", text: value }),
  ]);
}

function uniqueSources(signal) {
  const ids = new Set();
  for (const row of signal.evidence ?? []) {
    if (row.source_id) ids.add(row.source_id);
  }
  return [...ids];
}
