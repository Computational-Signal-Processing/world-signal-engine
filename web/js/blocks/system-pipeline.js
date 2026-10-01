/* The pipeline, as the engine's own counters describe it.
 *
 * The engine does not expose a per-stage metric, so this is honest about what
 * it is: the stages are drawn from counters that exist (observations,
 * anomalies, events, signals) and from the collector's own success and failure
 * tallies, and each stage states whether the engine reports anything for it. A
 * stage with no counter says "not measured" rather than showing a zero, because
 * the whole point of the project is that absence is not zero.
 *
 * It is a picture of the engine, not a claim about internals we cannot see. */

import { el, replace } from "../dom.js";
import { t } from "../i18n.js";
import { count, latency } from "../fmt.js";

/** Each stage names the counters that back it. Empty means "not measured". */
const STAGES = [
  { key: "source", counters: ["wse_sources_registered"] },
  { key: "collect", counters: ["wse_collector_success_total", "wse_collector_failure_total"] },
  { key: "observe", counters: ["wse_observations_total", "wse_observations_duplicate_total"] },
  { key: "detect", counters: ["wse_anomalies_total"] },
  { key: "signal", counters: ["wse_signals_total"] },
];

export const systemPipelineBlock = {
  mount(host, ctx) {
    host.append(el("div", { class: "pipeline" }));
    this.render(host, ctx);
  },
  update(host, ctx) { this.render(host, ctx); },
  render(host, ctx) {
    const data = ctx.data();
    const values = data.metrics?.values ?? {};
    const control = data.control;
    const pipeline = host.querySelector(".pipeline");

    replace(pipeline, STAGES.map((stage, index) => stageNode(stage, values, control, index)));
  },
  unmount(host) { host.replaceChildren(); },
};

function stageNode(stage, values, control, index) {
  const readings = stage.counters
    .map((name) => ({ name, value: values[name] }))
    .filter((reading) => reading.value != null);

  const measured = readings.length > 0;
  const failed = values.wse_collector_failure_total > 0 && stage.key === "collect";
  const status = !measured ? "unmeasured" : failed ? "warn" : "active";

  return el("div", { class: "pipe-stage", dataset: { state: status, key: stage.key } }, [
    el("div", { class: "pipe-glyph", "aria-hidden": "true", text: measured ? "●" : "○" }),
    el("div", { class: "pipe-name label", text: t().stage[stage.key] || stage.key }),
    el("div", { class: "pipe-values" }, measured
      ? readings.map((reading) => el("span", { class: "pipe-value num", text: count(reading.value) }))
      : [el("span", { class: "pipe-value void", text: t().state.notMeasured })]),
    index < STAGES.length - 1
      ? el("div", { class: "pipe-link", "aria-hidden": "true" })
      : null,
  ]);
}

/** Latency readings, each labelled and each void when unmeasured. */
export function telemetryRows(data) {
  const latencyData = data.control?.latency ?? data.world?.latency;
  const rows = [
    { label: t().action.observationLag, value: latencyData?.observation_lag_ms, ms: true },
    { label: t().action.collector, value: latencyData?.collector_ms, ms: true },
    { label: t().action.detection, value: latencyData?.detection_ms, ms: true },
    { label: t().action.newestSignal, value: latencyData?.newest_signal_age_ms, ms: true },
    { label: t().field.signals, value: data.world?.active_signals },
    { label: t().field.events, value: data.world?.events_total },
    { label: t().field.observations, value: data.world?.observations_total },
    { label: t().action.uptime, value: data.control?.uptime_seconds, seconds: true },
  ];
  return rows;
}
