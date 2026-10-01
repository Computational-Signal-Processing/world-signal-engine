/* The evidence chain.
 *
 * This is the drill-down the project promises: from a signal, back through its
 * event, its observations and their sources, to the raw payload the engine
 * received. Every hop is a real resource the engine serves, and each is
 * clickable so a reader can follow the chain themselves rather than take the
 * summary on trust.
 *
 * The chain is drawn as a chain — one link per hop — and a hop the engine does
 * not have is marked as missing rather than hidden. A gap shown as a gap is a
 * fact about the data; a gap smoothed over is a false claim.
 *
 * Below the chain are the evidence rows the engine attached to the signal,
 * newest first. Clicking a row selects that observation, which is what feeds the
 * raw viewer beside it. */

import { el, replace } from "../dom.js";
import { t, statusLabel, typeLabel } from "../i18n.js";
import { clock, sigma, stamp, VOID } from "../fmt.js";

/** The hops of the chain, in drill-down order. */
const STEPS = ["signal", "event", "observation", "source", "raw"];

export const evidenceChainBlock = {
  mount(host, ctx) {
    this.render(host, ctx);
    this.subscribe(host, ctx);
  },
  update(host, ctx) {
    this.render(host, ctx);
    this.subscribe(host, ctx);
  },
  subscribe(host, ctx) {
    if (this.off) this.off();
    this.off = ctx.on("selection", () => this.render(host, ctx));
  },
  async render(host, ctx) {
    const data = ctx.data();
    const id = data.selection.signalId || defaultSignal(data);
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

    replace(host, [
      el("div", { class: "region-head" }, [
        el("span", { class: "label", text: t().stage.signal }),
        el("span", { class: "region-head-meta", text: signal.id }),
      ]),
      chain(signal, ctx),
      evidenceList(signal, ctx),
    ]);
  },
  unmount(host) {
    if (this.off) { this.off(); this.off = null; }
    host.replaceChildren();
  },
};

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

function defaultSignal(data) {
  const signals = data.world?.now ?? data.signals ?? [];
  return signals[0]?.id ?? null;
}

/**
 * The five hops.
 *
 * Each hop names what it holds, or says the engine has nothing for it. The raw
 * hop is the observation's own record endpoint — it exists whenever the
 * observation does, which is the honest statement of what "raw" means here.
 */
function chain(signal, ctx) {
  const first = firstObservation(signal);
  const observationId = first?.observation_id || null;
  const sourceId = first?.source_id || null;

  const rows = [
    {
      step: "signal",
      value: signal.id,
      note: `${(signal.types ?? []).map(typeLabel).join(" · ")} — ${statusLabel(signal.status)}`,
      open: null,
    },
    {
      step: "event",
      value: signal.event_id || null,
      note: signal.first_seen ? `${t().field.firstSeen} ${clock(signal.first_seen)}` : null,
      open: null,   // no per-event view is mounted yet; the id is still the record
    },
    {
      step: "observation",
      value: observationId,
      note: first?.observed_at ? stamp(first.observed_at) : null,
      open: observationId ? () => ctx.select({ observationId }) : null,
    },
    {
      step: "source",
      value: sourceId,
      note: null,
      open: sourceId ? () => ctx.select({ sourceId }) : null,
    },
    {
      step: "raw",
      value: observationId,
      note: observationId ? t().field.raw : null,
      open: observationId ? () => ctx.select({ observationId }) : null,
    },
  ];

  return el("ol", { class: "chain" }, rows.map(link));
}

function link(row) {
  const missing = !row.value;
  const node = el("li", {
    class: "chain-link",
    dataset: { step: row.step, missing: missing ? "1" : "0", clickable: row.open ? "1" : "0" },
    tabindex: row.open ? "0" : null,
    role: row.open ? "button" : null,
    on: row.open ? {
      click: row.open,
      keydown: (event) => {
        if (event.key === "Enter" || event.key === " ") { event.preventDefault(); row.open(); }
      },
    } : {},
  }, [
    el("span", { class: "chain-glyph", "aria-hidden": "true", text: missing ? "○" : "●" }),
    el("span", { class: "chain-step label", text: t().stage[row.step] || row.step }),
    el("span", { class: missing ? "chain-value void" : "chain-value num", text: row.value || t().state.unavailable }),
    row.note ? el("span", { class: "chain-note faint", text: row.note }) : null,
  ]);
  return node;
}

/** The evidence rows the engine attached, newest first. */
function evidenceList(signal, ctx) {
  const rows = [...(signal.evidence ?? [])];
  if (!rows.length) {
    return el("div", { class: "region-empty" }, [
      el("div", { class: "region-empty-title", text: t().field.evidence }),
      el("div", { class: "region-empty-body", text: t().why.noData }),
    ]);
  }
  rows.sort((a, b) => String(b.observed_at).localeCompare(String(a.observed_at)));

  return el("div", { class: "evidence-list" }, rows.slice(0, 40).map((row) => el("article", {
    class: "evidence-row",
    // The observation id is on the element, not only in the click handler: the
    // row is the hand-off point to the raw viewer and has to be identifiable
    // from outside it.
    dataset: {
      observation: row.observation_id || "",
      selected: row.observation_id && row.observation_id === ctx.data().selection.observationId ? "1" : "0",
    },
    tabindex: row.observation_id ? "0" : null,
    on: row.observation_id ? {
      click: () => ctx.select({ observationId: row.observation_id }),
      keydown: (event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          ctx.select({ observationId: row.observation_id });
        }
      },
    } : {},
  }, [
    el("div", { class: "evidence-when num faint", text: stamp(row.observed_at) }),
    el("div", { class: "evidence-statement", text: row.statement || VOID }),
    el("div", { class: "evidence-meta faint" }, [
      el("span", { class: "num", text: row.deviation_sigma != null ? sigma(row.deviation_sigma) : VOID }),
      el("span", { text: row.source_id || VOID }),
    ]),
  ])));
}

function firstObservation(signal) {
  return (signal.evidence ?? [])[0] ?? null;
}

export { STEPS };
