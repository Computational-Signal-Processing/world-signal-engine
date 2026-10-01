/* The category strip.
 *
 * One card per category the engine reports, showing the latest value, how far
 * it sits from its own baseline, and a sparkline of its recent history.
 *
 * The strip is where the "nothing is compressed" rule lives. When the cards do
 * not fit, the strip does not shrink them or hide the extras behind an "and 9
 * more" label — it rotates, showing every card in turn and stating which page
 * it is on. A viewer who glances at the wall therefore sees all of the world,
 * just not all at once. */

import { el, replace } from "../dom.js";
import { t } from "../i18n.js";
import { measure, percent, sigma, seriesLabel } from "../fmt.js";
import { sparklineNode } from "./_chart.js";

const DEFAULT_INTERVAL = 9_000;

export const categoryStripBlock = {
  mount(host, ctx) {
    const head = el("div", { class: "region-head" }, [
      el("span", { class: "label", text: t().field.category }),
      el("span", { class: "region-head-meta num", dataset: { role: "page" } }),
    ]);
    const track = el("div", { class: "strip-track" });
    host.append(head, track);

    this.render(host, ctx);
    this.startRotation(host, ctx);
  },
  update(host, ctx) {
    this.render(host, ctx);
    this.startRotation(host, ctx);
  },
  /**
   * Re-paginate when the region's box changes.
   *
   * `perPage` is read from the region's own size, so a format change alters how
   * many cards fit. Without this the strip would keep the page size it measured
   * at mount and either clip cards or leave the row half empty.
   */
  resize(host, ctx) {
    this.render(host, ctx);
    this.startRotation(host, ctx);
  },
  render(host, ctx) {
    const data = ctx.data();
    const categories = data.observatory?.categories ?? [];
    const track = host.querySelector(".strip-track");
    const page = host.querySelector('[data-role="page"]');

    if (!categories.length) {
      replace(track, [el("div", { class: "region-empty" }, [
        el("div", { class: "region-empty-title", text: t().state.noData }),
        el("div", { class: "region-empty-body", text: t().why.noData }),
      ])]);
      if (page) page.textContent = "";
      host.dataset.pages = "0";
      return;
    }

    const perPage = this.perPage(host);
    const pages = Math.max(1, Math.ceil(categories.length / perPage));
    host.dataset.pages = String(pages);
    host.dataset.perPage = String(perPage);

    const current = Math.min(Number(host.dataset.page || 0), pages - 1);
    host.dataset.page = String(current);

    const slice = categories.slice(current * perPage, current * perPage + perPage);
    replace(track, slice.map(card));
    if (page) {
      page.textContent = pages > 1 ? `${current + 1} / ${pages}` : `${categories.length}`;
    }
  },
  /**
   * How many cards fit.
   *
   * Measured from the region's own box rather than from a viewport breakpoint,
   * because a region's size depends on the scene composition, not on the
   * window. `getBoundingClientRect` on a hidden or zero-height region returns
   * nothing useful, so it falls back to one card rather than to a division by
   * zero.
   */
  perPage(host) {
    const width = host.clientWidth;
    const height = host.clientHeight - (host.querySelector(".region-head")?.offsetHeight ?? 0);
    if (!width || !height) return 1;
    const cardW = 210;
    const cardH = 132;
    const cols = Math.max(1, Math.floor(width / cardW));
    const rows = Math.max(1, Math.floor(height / cardH));
    return cols * rows;
  },
  startRotation(host, ctx) {
    // One timer, restarted on each update so the interval always reflects the
    // current page count. The registry disposes it when the region unmounts.
    if (this.timer) { clearInterval(this.timer); this.timer = null; }
    const pages = Number(host.dataset.pages || 1);
    if (pages <= 1) return;
    const interval = ctx.region?.rotate?.every ? ctx.region.rotate.every * 1000 : DEFAULT_INTERVAL;
    this.timer = ctx.cleanup.interval(() => {
      const total = Number(host.dataset.pages || 1);
      if (total <= 1) return;
      const next = (Number(host.dataset.page || 0) + 1) % total;
      host.dataset.page = String(next);
      this.render(host, ctx);
    }, interval);
  },
  unmount(host) {
    if (this.timer) { clearInterval(this.timer); this.timer = null; }
    host.replaceChildren();
  },
};

function card(category) {
  const hasData = category.has_data && category.value != null;
  const deviation = deviationSigma(category);
  const change = category.change_pct;

  const value = hasData
    ? el("div", { class: "cat-value num", text: measure(category.value, null, 2) })
    : el("div", { class: "cat-value void", text: "—" });

  const unit = el("div", { class: "cat-unit faint", text: category.unit || "" });

  const delta = el("div", {
    class: "cat-delta num",
    dataset: { dir: change > 0 ? "up" : change < 0 ? "down" : "flat" },
    text: change == null ? "—" : percent(change, 1),
  });

  const dev = el("div", {
    class: "cat-sigma num",
    text: deviation == null ? t().state.notMeasured : sigma(deviation),
  });

  const sparkValues = (category.sparkline ?? []).map((p) => Number(p.value));
  const spark = sparklineNode(sparkValues, { width: 160, height: 26 });
  const sparkBox = el("div", { class: "cat-spark" }, spark ? [spark] : [
    el("span", { class: "faint", text: t().state.noData }),
  ]);

  return el("article", {
    class: "cat-card",
    dataset: { has: hasData ? "1" : "0" },
    title: category.series_key ? seriesLabel(category.series_key) : category.category,
  }, [
    el("div", { class: "cat-head" }, [
      el("span", { class: "cat-name", text: category.label || category.category }),
      el("span", { class: "cat-key faint", text: category.category }),
    ]),
    el("div", { class: "cat-body" }, [value, unit]),
    sparkBox,
    el("div", { class: "cat-foot" }, [delta, dev]),
  ]);
}

/**
 * Deviation in standard deviations, computed only from what the engine reports.
 *
 * The engine gives a value, a mean and a standard deviation. If the deviation
 * is zero or missing the ratio is undefined, and the card says "not measured"
 * rather than showing an infinity or a made-up magnitude.
 */
function deviationSigma(category) {
  const baseline = category.baseline;
  if (!baseline || category.value == null) return null;
  const sd = Number(baseline.std_dev);
  if (!Number.isFinite(sd) || sd === 0) return null;
  const mean = Number(baseline.mean);
  if (!Number.isFinite(mean)) return null;
  return (Number(category.value) - mean) / sd;
}
