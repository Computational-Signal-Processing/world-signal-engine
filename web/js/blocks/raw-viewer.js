/* The raw payload, as the engine received it.
 *
 * The end of the drill-down. Two things are shown: where the observation came
 * from — source, instants, the stored locator and hash — and the payload
 * itself, folded so it does not become a wall of braces.
 *
 * The payload is shown exactly as stored. It is the one thing in the studio that
 * must never be reshaped for presentation, because a reader who has come this
 * far is checking the engine's own record rather than reading a summary of it.
 *
 * With no observation selected the block says how to pick one instead of
 * rendering nothing: an empty region that explains itself looks like an
 * observation, and an empty region that does not looks like a bug. */

import { el, replace } from "../dom.js";
import { t } from "../i18n.js";
import { bytes, count, stamp, VOID } from "../fmt.js";

export const rawViewerBlock = {
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
    const observationId = data.selection.observationId || null;

    if (!observationId) {
      replace(host, [prompt()]);
      return;
    }

    let detail = null;
    let payload = null;
    let payloadError = null;
    try {
      detail = await ctx.load.observation(observationId);
    } catch (err) {
      payloadError = err;
    }
    try {
      payload = await ctx.load.raw(observationId);
    } catch (err) {
      payloadError = payloadError ?? err;
    }
    if (ctx.stale()) return;

    if (!detail && !payload) {
      replace(host, [failed(observationId, payloadError)]);
      return;
    }
    replace(host, nodes(observationId, detail, payload));
  },
  unmount(host) {
    if (this.off) { this.off(); this.off = null; }
    host.replaceChildren();
  },
};

function prompt() {
  return el("div", { class: "region-empty" }, [
    el("div", { class: "region-empty-title", text: t().field.raw }),
    el("div", { class: "region-empty-body", text: t().why.noSelection }),
  ]);
}

function failed(id, err) {
  return el("div", { class: "region-empty" }, [
    el("div", { class: "region-empty-title", text: `${id} — ${t().state.unavailable}` }),
    el("div", { class: "region-empty-body", text: err?.message || t().why.noData }),
  ]);
}

function nodes(observationId, detail, payload) {
  const text = payload == null ? null : safeStringify(payload);

  const meta = el("dl", { class: "raw-meta" }, [
    pair("observation", observationId),
    pair("source", detail?.source_id || VOID),
    pair("metric", detail?.metric || VOID),
    pair("observed", detail?.observed_at ? stamp(detail.observed_at) : VOID),
    pair("received", detail?.received_at ? stamp(detail.received_at) : VOID),
    pair("lag", detail?.lag_ms != null ? `${count(Math.round(detail.lag_ms / 1000))} s` : VOID),
    pair("locator", detail?.raw?.locator || VOID, true),
    pair("hash", detail?.raw?.hash || VOID),
    pair("size", text ? bytes(text.length) : (detail?.raw?.bytes != null ? bytes(detail.raw.bytes) : VOID)),
  ]);

  const head = el("div", { class: "region-head" }, [
    el("span", { class: "label", text: t().field.raw }),
    el("span", { class: "region-head-meta", text: observationId }),
  ]);

  if (!text) {
    return [head, meta, el("div", { class: "region-empty" }, [
      el("div", { class: "region-empty-title", text: t().state.noData }),
      el("div", { class: "region-empty-body", text: t().why.noData }),
    ])];
  }

  const fold = el("details", { class: "raw-fold", open: "" }, [
    el("summary", { class: "raw-summary label", text: `${t().field.raw} · ${bytes(text.length)}` }),
    el("pre", { class: "raw-body mono", text }),
  ]);

  return [head, meta, fold];
}

function pair(label, value, wide = false) {
  return el("div", { class: "raw-pair", dataset: { wide: wide ? "1" : "0" } }, [
    el("dt", { class: "label", text: label }),
    el("dd", { class: value === VOID ? "void" : "num", text: value }),
  ]);
}

function safeStringify(value) {
  try {
    return JSON.stringify(value, null, 2);
  } catch (_) {
    return String(value);
  }
}
