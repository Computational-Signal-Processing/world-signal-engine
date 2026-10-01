/* Formatters.
 *
 * Pure functions, no state. They exist so that "not measured" is expressed once
 * and consistently: every one of them returns the void dash for a missing
 * value rather than 0, because a zero is a measurement and an absent reading is
 * not. */

export const VOID = "—";

const isNil = (v) => v === null || v === undefined || v === "" || Number.isNaN(v);

/** Clock time from an ISO instant, e.g. `14:32:07Z`. */
export function clock(iso) {
  if (isNil(iso)) return VOID;
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return VOID;
  return `${String(d.getUTCHours()).padStart(2, "0")}:${String(d.getUTCMinutes()).padStart(2, "0")}:${String(d.getUTCSeconds()).padStart(2, "0")}Z`;
}

/** Date and time, e.g. `2026-10-01 14:32Z`. */
export function stamp(iso) {
  if (isNil(iso)) return VOID;
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return VOID;
  const date = d.toISOString().slice(0, 10);
  return `${date} ${clock(iso)}`;
}

/** Compact elapsed span from a second count, e.g. `10h 43m`, `3d 4h`, `42s`. */
export function duration(seconds) {
  if (isNil(seconds)) return VOID;
  const s = Math.max(0, Math.floor(Number(seconds)));
  if (!Number.isFinite(s)) return VOID;
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 24) return m % 60 ? `${h}h ${m % 60}m` : `${h}h`;
  const d = Math.floor(h / 24);
  return h % 24 ? `${d}d ${h % 24}h` : `${d}d`;
}

/** Relative age from a millisecond lag, e.g. `42s ago`. */
export function ago(ms) {
  if (isNil(ms)) return VOID;
  return `${duration(Number(ms) / 1000)} ago`;
}

/** A signed standard deviation, e.g. `+16.3σ`, `-0.2σ`. */
export function sigma(value, digits = 1) {
  if (isNil(value) || !Number.isFinite(Number(value))) return VOID;
  const n = Number(value);
  const sign = n > 0 ? "+" : n < 0 ? "−" : "";
  return `${sign}${Math.abs(n).toFixed(digits)}σ`;
}

/** A signed percentage, e.g. `+84%`, `-6.2%`. */
export function percent(value, digits = 0) {
  if (isNil(value) || !Number.isFinite(Number(value))) return VOID;
  const n = Number(value);
  const sign = n > 0 ? "+" : n < 0 ? "−" : "";
  return `${sign}${Math.abs(n).toFixed(digits)}%`;
}

/**
 * A measurement with its unit, switching to exponential notation only when the
 * magnitude makes plain decimal unreadable (flux readings, for instance).
 */
export function measure(value, unit, digits = 3) {
  if (isNil(value) || !Number.isFinite(Number(value))) return VOID;
  const n = Number(value);
  const abs = Math.abs(n);
  let body;
  if (abs !== 0 && (abs < 0.001 || abs >= 100000)) {
    const exp = n.toExponential(digits - 1);
    body = exp.replace("e", "e");
  } else {
    body = n.toFixed(abs >= 100 ? 0 : digits);
  }
  return unit ? `${body} ${unit}` : body;
}

/** A count, or the void dash when the engine did not report one. */
export function count(value) {
  if (isNil(value) || !Number.isFinite(Number(value))) return VOID;
  return Number(value).toLocaleString("en-US");
}

/** A confidence/quality fraction as a percentage, or void. */
export function ratio(value, digits = 0) {
  if (isNil(value) || !Number.isFinite(Number(value))) return VOID;
  return `${(Number(value) * 100).toFixed(digits)}%`;
}

/** Milliseconds as a short latency, e.g. `242 ms`, `1.2 s`. */
export function latency(ms) {
  if (isNil(ms) || !Number.isFinite(Number(ms))) return VOID;
  const n = Number(ms);
  return n < 1000 ? `${Math.round(n)} ms` : `${(n / 1000).toFixed(1)} s`;
}

/** Bytes as a short human size. */
export function bytes(n) {
  if (isNil(n) || !Number.isFinite(Number(n))) return VOID;
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = Number(n);
  let i = 0;
  while (value >= 1024 && i < units.length - 1) { value /= 1024; i += 1; }
  return `${value.toFixed(value >= 10 || i === 0 ? 0 : 1)} ${units[i]}`;
}

/**
 * The human part of a series key.
 *
 * Keys are `source::entity::metric::unit|dimensions`. Showing the whole key in
 * a broadcast is noise; showing the metric and unit is what a viewer reads.
 */
export function seriesLabel(key) {
  if (isNil(key)) return VOID;
  const parts = String(key).split("::");
  if (parts.length < 3) return String(key);
  const metric = parts[2];
  const unit = (parts[3] || "").split("|")[0];
  return unit ? `${metric} (${unit})` : metric;
}

/** The source segment of a series key. */
export function seriesSource(key) {
  if (isNil(key)) return VOID;
  return String(key).split("::")[0] || VOID;
}

/** Title-case a snake_case identifier for display. */
export function humanize(id) {
  if (isNil(id)) return VOID;
  return String(id).replace(/_/g, " ").replace(/\b\w/g, (c) => c.toUpperCase());
}
