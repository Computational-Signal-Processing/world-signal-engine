/* Telemetry: what the engine measures about itself.
 *
 * Every row here is a reading the engine publishes. A row with no reading says
 * "not measured" — the studio never fills a gap with a zero or an estimate,
 * because a fabricated latency is worse than an absent one: it would be acted
 * on.
 *
 * `observation_lag_ms` is worth reading carefully and the block says so: a
 * daily feed arriving all at once is a large lag and a healthy source, not a
 * fault. The label is qualified on screen so the number is not misread. */

import { el, replace } from "../dom.js";
import { t } from "../i18n.js";
import { duration, latency, count } from "../fmt.js";

export const telemetryBlock = {
  mount(host, ctx) {
    host.append(el("div", { class: "region-head" }, [
      el("span", { class: "label", text: t().action.connection }),
      el("span", { class: "region-head-meta", dataset: { role: "state" } }),
    ]), el("div", { class: "telemetry-rows" }));
    this.render(host, ctx);
  },
  update(host, ctx) { this.render(host, ctx); },
  render(host, ctx) {
    const data = ctx.data();
    const rows = host.querySelector(".telemetry-rows");
    const state = host.querySelector('[data-role="state"]');
    if (state) state.textContent = t().conn[data.connection.state] || data.connection.state;

    replace(rows, build(data).map(row));
  },
  unmount(host) { host.replaceChildren(); },
};

function build(data) {
  const lat = data.control?.latency ?? data.world?.latency ?? {};
  const metrics = data.metrics?.values ?? {};
  return [
    { label: t().action.observationLag, value: ms(lat.observation_lag_ms), note: t().field.observedAt },
    { label: t().action.collector, value: ms(lat.collector_ms) },
    { label: t().action.detection, value: ms(lat.detection_ms) },
    { label: t().action.newestSignal, value: ms(lat.newest_signal_age_ms) },
    { label: t().action.uptime, value: data.control?.uptime_seconds != null ? duration(data.control.uptime_seconds) : null },
    { label: t().field.observations, value: num(metrics.wse_observations_total) },
    { label: t().field.duplicates, value: num(metrics.wse_observations_duplicate_total) },
    { label: t().field.anomalies, value: num(metrics.wse_anomalies_total) },
    { label: t().field.events, value: num(metrics.wse_events_total) },
    { label: t().field.rateLimited, value: num(metrics.wse_collector_rate_limited_total) },
  ];
}

function ms(value) {
  return value == null ? null : latency(value);
}

function num(value) {
  return value == null ? null : count(value);
}

function row(entry) {
  return el("div", { class: "telemetry-row", dataset: { empty: entry.value == null ? "1" : "0" } }, [
    el("span", { class: "telemetry-label faint", text: entry.label }),
    el("span", { class: entry.value == null ? "telemetry-value void" : "telemetry-value num", text: entry.value ?? t().state.notMeasured }),
    entry.note ? el("span", { class: "telemetry-note faint", text: entry.note }) : null,
  ]);
}
