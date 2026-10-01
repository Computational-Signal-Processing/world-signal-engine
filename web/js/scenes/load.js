/* Scene loading.
 *
 * A scene is fetched, not imported, so the same build serves a different
 * composition when a JSON file changes — no rebuild, no code edit. This is the
 * mechanism behind "add or remove a region whenever we want".
 *
 * A scene may also have a composition per format. The file for the current
 * aspect ratio wins — `overview.portrait.json` on a vertical display — and the
 * base file is the fallback, so a scene only needs a variant where the default
 * composition genuinely does not hold. A vertical screen is not a shrunken
 * horizontal one: it gets a different arrangement, not the same one scaled.
 *
 * If a scene file is missing or malformed the studio falls back to the built-in
 * overview rather than showing nothing: a broadcast that cannot load its layout
 * still has to say something about the world. */

const SCENES = ["overview", "map", "signal", "sources", "system", "evidence"];
const BASE = "/config/studio";

const cache = new Map();

/**
 * The format a viewport presents, as a scene-file suffix.
 *
 * The axis is aspect ratio, matching the composition rules in the stylesheet:
 * a vertical display, a square-ish one and a wide one are different broadcasts.
 * `landscape` is the base composition and has no suffix.
 */
export function formatFor(viewport = globalThis) {
  const width = viewport?.innerWidth || 0;
  const height = viewport?.innerHeight || 0;
  if (!width || !height) return "landscape";
  const ratio = width / height;
  if (ratio <= 0.85) return "portrait";
  if (ratio < 1.2) return "square";
  return "landscape";
}

/** Fetch one scene definition by id, preferring the composition for this format. */
export async function loadScene(id, format = formatFor()) {
  const key = `${id}@${format}`;
  if (cache.has(key)) return cache.get(key);

  // Try the format-specific file first, then the base. A 404 on the variant is
  // the normal case, not a failure: most scenes only have the base file.
  for (const suffix of format && format !== "landscape" ? [`.${format}`, ""] : [""]) {
    const response = await fetch(`${BASE}/scenes/${id}${suffix}.json`, { cache: "no-cache" });
    if (response.ok) {
      const scene = await response.json();
      // A hand-written variant is used as it stands. Falling back to the base
      // file for a vertical format would show a shrunken horizontal layout,
      // which is exactly what a vertical broadcast must not be — so the base
      // composition is re-stacked instead.
      const resolved = !suffix && format === "portrait" ? portraitise(scene) : scene;
      cache.set(key, resolved);
      return resolved;
    }
    if (response.status !== 404) {
      throw new Error(`scene "${id}" not found (${response.status})`);
    }
  }
  throw new Error(`scene "${id}" not found`);
}

/**
 * Derive a vertical composition from a horizontal one.
 *
 * There is one world dataset and one set of blocks; only the arrangement
 * changes with the screen. Rather than author a portrait file for every scene
 * (and let the ones without one degrade into a letterboxed desktop), the
 * regions are stacked in priority order down a single column. Region ids,
 * blocks, overflow and rotation policy are preserved, so every scene keeps
 * working on a vertical display with the same blocks it always had.
 *
 * A scene that ships its own `*.portrait.json` never reaches this function.
 */
export function portraitise(scene) {
  const regions = scene?.regions;
  if (!Array.isArray(regions) || !regions.length) return scene;
  // A scene already taller than wide is already vertical; leave it alone.
  const grid = scene.grid || {};
  if (grid.cols && grid.rows && grid.rows > grid.cols) return scene;

  const ordered = [...regions].sort((a, b) => (b.priority ?? 0) - (a.priority ?? 0));
  const rows = ordered.length * 2;
  const stacked = ordered.map((region, i) => ({
    ...region,
    area: `${i * 2 + 1} / 1 / ${i * 2 + 3} / 2`,
  }));

  return {
    ...scene,
    grid: { cols: 1, rows },
    regions: stacked,
    // The composition was derived, not authored; say so rather than presenting
    // it as the scene's intended portrait layout.
    derived: "portrait",
  };
}

/** Fetch the takeover policy. Falls back to the built-in defaults on failure. */
export async function loadPolicy() {
  try {
    const response = await fetch(`${BASE}/policies/takeover.json`, { cache: "no-cache" });
    if (!response.ok) return null;
    return await response.json();
  } catch (_) {
    return null;
  }
}

/** Fetch every scene, for the control room's scene list. */
export async function loadSceneIndex() {
  const settled = await Promise.allSettled(SCENES.map((id) => loadScene(id)));
  return settled
    .map((result, i) => (result.status === "fulfilled" ? result.value : { id: SCENES[i], unavailable: true }))
    .filter(Boolean);
}

/** Drop the cache, e.g. after a scene file is edited during development. */
export function invalidateScenes() { cache.clear(); }

export const SCENE_IDS = SCENES;