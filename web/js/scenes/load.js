/* Scene loading.
 *
 * A scene is fetched, not imported, so the same build serves a different
 * composition when a JSON file changes — no rebuild, no code edit. This is the
 * mechanism behind "add or remove a region whenever we want".
 *
 * If a scene file is missing or malformed the studio falls back to the built-in
 * overview rather than showing nothing: a broadcast that cannot load its layout
 * still has to say something about the world. */

const SCENES = ["overview", "map", "signal", "sources", "system", "evidence"];
const BASE = "/config/studio";

const cache = new Map();

/** Fetch one scene definition by id. */
export async function loadScene(id) {
  if (cache.has(id)) return cache.get(id);
  const response = await fetch(`${BASE}/scenes/${id}.json`, { cache: "no-cache" });
  if (!response.ok) throw new Error(`scene "${id}" not found (${response.status})`);
  const scene = await response.json();
  cache.set(id, scene);
  return scene;
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
