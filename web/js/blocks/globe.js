/* Where the observations are.
 *
 * The engine reports no location on a signal — the coordinates live on the
 * observations themselves, and only for sources that are geospatial. So the
 * globe asks the sources that carry coordinates for their recent points and
 * plots those. A point on a globe is a claim that something was measured there,
 * and every point drawn is one the engine actually recorded.
 *
 * The set of series worth asking is derived from the catalog's own `geospatial`
 * flag, not from a hardcoded list, so a new geospatial source appears on the
 * globe without a code change.
 *
 * When nothing has coordinates the globe says so in words. An empty sphere
 * could be mistaken for a quiet world, which is the one reading this project
 * exists to prevent. */

import { el, replace, svg } from "../dom.js";
import { t } from "../i18n.js";
import { seriesLabel } from "../fmt.js";

const VIEW = { width: 620, height: 460, radius: 186 };

export const globeBlock = {
  mount(host, ctx) {
    this.render(host, ctx);
  },
  update(host, ctx) { this.render(host, ctx); },

  async render(host, ctx) {
    const data = ctx.data();
    const series = geospatialSeries(data);

    if (!series.length) {
      replace(host, [emptyNode(data, 0)]);
      return;
    }

    const points = await collectPoints(series, ctx);
    if (ctx.stale()) return;

    if (!points.length) {
      replace(host, [emptyNode(data, series.length)]);
      return;
    }
    replace(host, [buildFrame(), caption(points, series.length)]);
    draw(host.querySelector("svg"), points);
  },

  unmount(host) { host.replaceChildren(); },
};

/** Read each geospatial series' recent points. One failure must not blank the globe. */
async function collectPoints(series, ctx) {
  const points = [];
  for (const key of series.slice(0, 8)) {
    try {
      const timeline = await ctx.load.timeline(key);
      for (const observation of timeline?.observations ?? []) {
        if (Number.isFinite(observation.latitude) && Number.isFinite(observation.longitude)) {
          points.push(observation);
        }
      }
    } catch (_) { /* skip an unreadable series */ }
  }
  if (ctx.stale()) return [];
  return dedupe(points);
}

/**
 * The series keys that can carry coordinates.
 *
 * Taken from the catalog's `geospatial` flag crossed with the observatory's
 * representative series per category, so no request is wasted on a source that
 * has no coordinates to give. Until the catalog arrives the signals' own series
 * are used, so the globe is not blank while it loads.
 */
function geospatialSeries(data) {
  const geospatialCategories = new Set(
    (data.sources ?? []).filter((s) => s.geospatial).map((s) => s.category),
  );
  const keys = [];
  for (const category of data.observatory?.categories ?? []) {
    if (!category.has_data || !category.series_key) continue;
    if (geospatialCategories.size && !geospatialCategories.has(category.category)) continue;
    keys.push(category.series_key);
  }
  if (!keys.length) {
    for (const signal of data.world?.now ?? data.signals ?? []) {
      if (signal.series_key) keys.push(signal.series_key);
    }
  }
  return [...new Set(keys)];
}

/** The same physical point can appear in several series; keep the newest. */
function dedupe(points) {
  const seen = new Map();
  for (const point of points) {
    const key = `${point.latitude.toFixed(3)},${point.longitude.toFixed(3)}`;
    const existing = seen.get(key);
    if (!existing || String(point.observed_at) > String(existing.observed_at)) seen.set(key, point);
  }
  return [...seen.values()];
}

function emptyNode(data, asked) {
  const signals = (data.world?.now ?? data.signals ?? []).length;
  const body = asked
    ? `${asked} ${t().field.sources} · ${t().reason.noLocation}`
    : t().reason.noLocation;
  return el("div", { class: "region-empty" }, [
    el("div", { class: "region-empty-title", text: t().state.noLocation }),
    el("div", { class: "region-empty-body", text: signals ? body : t().reason.noSignal }),
  ]);
}

function buildFrame() {
  const surface = svg("svg", {
    viewBox: `0 0 ${VIEW.width} ${VIEW.height}`,
    class: "globe-svg",
    preserveAspectRatio: "xMidYMid meet",
    role: "img",
  });
  return el("div", { class: "globe-frame" }, [surface, el("div", { class: "globe-caption" })]);
}

/** Draw the sphere and its points. Redrawn wholesale: the point count is small. */
function draw(surface, points) {
  replace(surface, []);
  const { width, height, radius } = VIEW;
  const cx = width / 2;
  const cy = height / 2;

  surface.append(svg("circle", { cx, cy, r: radius, class: "globe-disc" }));
  surface.append(svg("circle", { cx, cy, r: radius, class: "globe-limb" }));

  for (let lat = -60; lat <= 60; lat += 30) {
    const y = cy - (lat / 90) * radius;
    const half = Math.sqrt(Math.max(0, radius * radius - (y - cy) * (y - cy)));
    surface.append(svg("line", { x1: cx - half, x2: cx + half, y1: y, y2: y, class: "globe-grid" }));
  }
  for (let lon = -180; lon < 180; lon += 30) {
    const path = [];
    for (let lat = -90; lat <= 90; lat += 5) {
      const [px, py] = ortho(lat, lon, cx, cy, radius);
      if (px !== null) path.push(`${px},${py}`);
    }
    if (path.length > 1) surface.append(svg("polyline", { points: path.join(" "), class: "globe-grid" }));
  }

  // Newest points drawn last, so at a shared place the current reading is on top.
  const ordered = [...points].sort((a, b) => String(a.observed_at).localeCompare(String(b.observed_at)));
  for (const point of ordered) {
    const [px, py] = ortho(point.latitude, point.longitude, cx, cy, radius);
    if (px === null) continue;   // far side of the globe
    const dot = svg("circle", { cx: px, cy: py, r: 4, class: "globe-dot", fill: "var(--cyan)" });
    const title = svg("title");
    title.textContent = `${seriesLabel(point.series_key)} — ${point.metric} ${point.value}${point.unit ? ` ${point.unit}` : ""}`;
    dot.append(title);
    surface.append(dot);
  }
}

function caption(points, seriesCount) {
  return el("div", { class: "globe-caption" }, [
    el("span", { class: "globe-count num", text: `${points.length}` }),
    el("span", { class: "faint", text: t().field.observations }),
    el("span", { class: "globe-note faint", text: `${seriesCount} ${t().field.sources}` }),
  ]);
}

/** Orthographic projection; `null` when the point faces away from the viewer. */
export function ortho(lat, lon, cx, cy, radius) {
  const phi = (lat * Math.PI) / 180;
  const lambda = (lon * Math.PI) / 180;
  const cosC = Math.cos(phi) * Math.cos(lambda);
  if (cosC < 0) return [null, null];
  return [cx + radius * Math.cos(phi) * Math.sin(lambda), cy - radius * Math.sin(phi)];
}

export { VIEW as GLOBE_VIEW };
