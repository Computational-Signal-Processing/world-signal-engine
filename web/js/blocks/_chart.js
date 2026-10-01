/* Small chart primitives shared by the blocks that draw one.
 *
 * Both are SVG path builders, not chart libraries. They exist so a sparkline
 * looks the same everywhere it appears and so the "no data" case is handled in
 * one place: a series with fewer than two points draws nothing rather than a
 * flat line that would read as a measurement. */

import { svg } from "../dom.js";

/**
 * A sparkline path over a list of numbers.
 *
 * @returns {{ line: SVGPathElement, area: SVGPathElement, viewBox: string }|null}
 */
export function sparkline(values, { width = 100, height = 28, pad = 2 } = {}) {
  const usable = (values ?? []).filter((v) => Number.isFinite(v));
  if (usable.length < 2) return null;

  const min = Math.min(...usable);
  const max = Math.max(...usable);
  const span = max - min || 1;
  const step = (width - pad * 2) / (usable.length - 1);

  const points = usable.map((v, i) => {
    const x = pad + i * step;
    const y = height - pad - ((v - min) / span) * (height - pad * 2);
    return [x, y];
  });

  const line = points.map(([x, y], i) => `${i === 0 ? "M" : "L"}${x.toFixed(2)},${y.toFixed(2)}`).join(" ");
  const area = `${line} L${points[points.length - 1][0].toFixed(2)},${height} L${points[0][0].toFixed(2)},${height} Z`;

  return {
    line,
    area,
    viewBox: `0 0 ${width} ${height}`,
    min,
    max,
  };
}

/** Build a sparkline element, or null when there is nothing to draw. */
export function sparklineNode(values, { width = 100, height = 28, class: cls = "spark" } = {}) {
  const shape = sparkline(values, { width, height });
  if (!shape) return null;
  const node = svg("svg", {
    viewBox: shape.viewBox,
    class: cls,
    preserveAspectRatio: "none",
    "aria-hidden": "true",
  });
  node.append(svg("path", { d: shape.area, class: "spark-area" }));
  node.append(svg("path", { d: shape.line, class: "spark-line" }));
  return node;
}

/**
 * An hourly bar chart with the window mean drawn as a rule.
 *
 * The mean is the point of the chart: a spike means nothing until it is read
 * against the world's own recent norm. Both come from the same buckets, so the
 * comparison is honest.
 */
export function barsNode(buckets, { width = 600, height = 180, pad = 4 } = {}) {
  const counts = (buckets ?? []).map((b) => Number(b.count));
  if (!counts.length) return null;
  const max = Math.max(...counts, 1);
  const mean = counts.reduce((a, b) => a + b, 0) / counts.length;

  const barWidth = (width - pad * 2) / counts.length;
  const node = svg("svg", {
    viewBox: `0 0 ${width} ${height}`,
    class: "bars",
    preserveAspectRatio: "none",
    role: "img",
  });

  counts.forEach((count, i) => {
    const h = Math.max(1, (count / max) * (height - pad * 2));
    node.append(svg("rect", {
      x: pad + i * barWidth + barWidth * 0.15,
      y: height - pad - h,
      width: barWidth * 0.7,
      height: h,
      class: count >= mean * 2 ? "bar bar-high" : "bar",
    }));
  });

  const meanY = height - pad - (mean / max) * (height - pad * 2);
  node.append(svg("line", { x1: pad, x2: width - pad, y1: meanY, y2: meanY, class: "bars-mean" }));

  return { node, max, mean, count: counts.length };
}

/** A baseline band and a series line, for the timeline block. */
export function bandNode(observations, baseline, { width = 600, height = 200, pad = 8 } = {}) {
  const values = (observations ?? []).map((o) => Number(o.value)).filter((v) => Number.isFinite(v));
  if (values.length < 2) return null;

  const min = Math.min(...values);
  const max = Math.max(...values);
  const span = max - min || 1;
  const step = (width - pad * 2) / (values.length - 1);
  const toY = (v) => height - pad - ((v - min) / span) * (height - pad * 2);
  const points = values.map((v, i) => [pad + i * step, toY(v)]);
  const line = points.map(([x, y], i) => `${i === 0 ? "M" : "L"}${x.toFixed(2)},${y.toFixed(2)}`).join(" ");

  const node = svg("svg", {
    viewBox: `0 0 ${width} ${height}`,
    class: "band",
    preserveAspectRatio: "none",
    role: "img",
  });

  // The band is drawn only when the engine reported the bounds. Without them
  // there is no baseline to compare against, and inventing one would be a lie
  // about the measurement.
  if (baseline && Number.isFinite(baseline.p05) && Number.isFinite(baseline.p95)) {
    const top = toY(Math.max(baseline.p95, min));
    const bottom = toY(Math.min(baseline.p05, max));
    node.append(svg("rect", {
      x: pad,
      y: Math.min(top, bottom),
      width: width - pad * 2,
      height: Math.max(1, Math.abs(bottom - top)),
      class: "band-range",
    }));
  }
  if (baseline && Number.isFinite(baseline.mean)) {
    const y = toY(baseline.mean);
    node.append(svg("line", { x1: pad, x2: width - pad, y1: y, y2: y, class: "band-mean" }));
  }

  node.append(svg("path", { d: line, class: "band-line" }));
  const last = points[points.length - 1];
  node.append(svg("circle", { cx: last[0], cy: last[1], r: 4, class: "band-now" }));
  return node;
}
