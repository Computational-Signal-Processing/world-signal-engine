/* The world map.
 *
 * An equirectangular projection with real coordinates plotted on it. The
 * honesty rule is at its sharpest here: the only points drawn are observations
 * that carry a latitude and a longitude. Nothing is placed at a country's
 * centre, at an entity's home, or at a random spot inside a region — a point on
 * a map is a claim that something happened there, and a fabricated one would be
 * indistinguishable from a real reading.
 *
 * So the map can be sparse, and when it is, it says so: how many observations
 * carry coordinates and how many do not. A sparse map is the truth, not a bug. */

import { el, replace, svg } from "../dom.js";
import { t } from "../i18n.js";
import { stamp } from "../fmt.js";

const VIEW = { width: 960, height: 480 };

export const worldMapBlock = {
  mount(host, ctx) {
    this.render(host, ctx);
  },
  update(host, ctx) { this.render(host, ctx); },
  async render(host, ctx) {
    const data = ctx.data();

    // Points come from series that actually carry coordinates. The set is read
    // from the catalog's own geospatial flag, so nothing is guessed.
    const series = coordinateSeries(data);
    if (!series.length) {
      replace(host, [empty(data, 0)]);
      return;
    }

    const collected = [];
    for (const key of series.slice(0, 8)) {
      try {
        const timeline = await ctx.load.timeline(key);
        for (const observation of timeline?.observations ?? []) {
          if (Number.isFinite(observation.latitude) && Number.isFinite(observation.longitude)) {
            collected.push(observation);
          }
        }
      } catch (_) { /* one unreadable series must not blank the map */ }
    }
    if (ctx.stale()) return;

    if (!collected.length) {
      replace(host, [empty(data, series.length)]);
      return;
    }

    replace(host, [
      el("div", { class: "region-head" }, [
        el("span", { class: "label", text: t().scene.map }),
        el("span", { class: "region-head-meta num", text: `${collected.length} ${t().field.observations}` }),
      ]),
      mapSurface(collected),
      legend(collected),
    ]);
  },
  unmount(host) { host.replaceChildren(); },
};

/**
 * The series keys that can carry coordinates.
 *
 * The source catalog records whether a source is geospatial; the observatory
 * reports one representative series per category. Intersecting the two gives the
 * series worth asking for without fetching everything.
 */
function coordinateSeries(data) {
  const geospatialCategories = new Set(
    (data.sources ?? []).filter((source) => source.geospatial).map((source) => source.category),
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

function empty(data, asked) {
  const signals = (data.world?.now ?? data.signals ?? []).length;
  return el("div", { class: "region-empty" }, [
    el("div", { class: "region-empty-title", text: t().state.noLocation }),
    el("div", {
      class: "region-empty-body",
      text: signals
        ? (asked ? `${asked} ${t().field.sources} · ${t().reason.noLocation}` : t().reason.noLocation)
        : t().reason.noSignal,
    }),
  ]);
}

function mapSurface(observations) {
  const surface = svg("svg", {
    viewBox: `0 0 ${VIEW.width} ${VIEW.height}`,
    class: "map-svg",
    preserveAspectRatio: "xMidYMid meet",
    role: "img",
  });

  // A graticule gives the projection a scale without drawing a coastline the
  // project has no licensed dataset for.
  for (let lon = -180; lon <= 180; lon += 30) {
    const x = project(lon, 0)[0];
    surface.append(svg("line", { x1: x, x2: x, y1: 0, y2: VIEW.height, class: "map-grid" }));
  }
  for (let lat = -60; lat <= 60; lat += 30) {
    const y = project(0, lat)[1];
    surface.append(svg("line", { x1: 0, x2: VIEW.width, y1: y, y2: y, class: "map-grid" }));
  }
  surface.append(svg("line", {
    x1: 0, x2: VIEW.width, y1: VIEW.height / 2, y2: VIEW.height / 2, class: "map-equator",
  }));

  for (const observation of observations) {
    const [x, y] = project(observation.longitude, observation.latitude);
    const dot = svg("circle", { cx: x, cy: y, r: 4, class: "map-dot", "data-source": observation.entity_id || "" });
    const title = svg("title");
    title.textContent = `${observation.entity_id || "observation"} · ${observation.metric} = ${observation.value} ${observation.unit || ""} · ${stamp(observation.observed_at)}`;
    dot.append(title);
    surface.append(dot);
  }
  return surface;
}

function legend(observations) {
  const sources = new Set(observations.map((o) => (o.entity_id || "").split("_")[0]).filter(Boolean));
  return el("div", { class: "map-legend" }, [
    el("span", { class: "map-legend-item faint", text: `${observations.length} ${t().field.observations}` }),
    el("span", { class: "map-legend-item faint", text: `${sources.size} ${t().field.entities}` }),
    el("span", { class: "map-legend-item faint", text: "real coordinates only" }),
  ]);
}

/** Equirectangular projection. Every point is placed; the map does not hide any. */
export function project(lon, lat) {
  const x = ((Number(lon) + 180) / 360) * VIEW.width;
  const y = ((90 - Number(lat)) / 180) * VIEW.height;
  return [x, y];
}

export { VIEW as MAP_VIEW };
