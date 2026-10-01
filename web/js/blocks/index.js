/* Block registration.
 *
 * One import site for the studio. Each block registers itself here, so the
 * studio core never imports a visual and a new block is added by writing a file
 * and adding one line — which is the whole point of the registry. */

import { registerBlock } from "./registry.js";

import { globeBlock } from "./globe.js";
import { worldMapBlock } from "./world-map.js";
import { signalFeedBlock } from "./signal-feed.js";
import { signalCardBlock } from "./signal-card.js";
import { metricGroupBlock } from "./metric-group.js";
import { categoryStripBlock } from "./category-strip.js";
import { activityChartBlock } from "./activity-chart.js";
import { timelineBlock } from "./timeline.js";
import { evidenceChainBlock } from "./evidence-chain.js";
import { sourceHealthBlock } from "./source-health.js";
import { sourceCardBlock } from "./source-card.js";
import { systemPipelineBlock } from "./system-pipeline.js";
import { telemetryBlock } from "./telemetry.js";
import { activityStreamBlock } from "./activity-stream.js";
import { rawViewerBlock } from "./raw-viewer.js";

const CATALOG = {
  globe: globeBlock,
  "world-map": worldMapBlock,
  "signal-feed": signalFeedBlock,
  "signal-card": signalCardBlock,
  "metric-group": metricGroupBlock,
  "category-strip": categoryStripBlock,
  "activity-chart": activityChartBlock,
  timeline: timelineBlock,
  "evidence-chain": evidenceChainBlock,
  "source-health": sourceHealthBlock,
  "source-card": sourceCardBlock,
  "system-pipeline": systemPipelineBlock,
  telemetry: telemetryBlock,
  "activity-stream": activityStreamBlock,
  "raw-viewer": rawViewerBlock,
};

/** Register every built-in block. Called once, at boot. */
export function registerBuiltinBlocks() {
  for (const [name, spec] of Object.entries(CATALOG)) registerBlock(name, spec);
  return Object.keys(CATALOG);
}
