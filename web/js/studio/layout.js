/* Layout: scene definition to positioned regions.
 *
 * A scene is data. It names a grid, a safe area and a list of regions, and each
 * region names an area and a block. Nothing about where a region sits lives in
 * CSS or in JS, which is what makes "move this region" a one-line edit and
 * "add a region" possible at all.
 *
 * Validation is part of composing, not an afterthought: an area string that the
 * grid cannot satisfy, or two regions claiming the same cells, is reported and
 * the scene is refused rather than drawn broken. */

/** Parse a CSS grid area string into `{ rowStart, colStart, rowEnd, colEnd }`. */
export function parseArea(area) {
  if (typeof area !== "string") return null;
  const parts = area.split("/").map((p) => p.trim());
  if (parts.length !== 4) return null;
  const [rowStart, colStart, rowEnd, colEnd] = parts.map(Number);
  if ([rowStart, colStart, rowEnd, colEnd].some((n) => !Number.isFinite(n))) return null;
  return { rowStart, colStart, rowEnd, colEnd };
}

/** Build a lookup of every grid cell to the region that claims it. */
function claimGrid(regions, cols, rows) {
  const claims = new Map();
  const collisions = [];
  for (const region of regions) {
    const area = parseArea(region.area);
    if (!area) { collisions.push({ region: region.id, reason: "area is not four numbers" }); continue; }
    const { rowStart, colStart, rowEnd, colEnd } = area;
    if (rowStart < 1 || colStart < 1 || rowEnd > rows + 1 || colEnd > cols + 1 || rowEnd <= rowStart || colEnd <= colStart) {
      collisions.push({ region: region.id, reason: `area ${region.area} is outside a ${cols}x${rows} grid` });
      continue;
    }
    for (let r = rowStart; r < rowEnd; r += 1) {
      for (let c = colStart; c < colEnd; c += 1) {
        const key = `${r}:${c}`;
        if (claims.has(key)) {
          collisions.push({ region: region.id, reason: `overlaps ${claims.get(key)} at row ${r}, column ${c}` });
        } else {
          claims.set(key, region.id);
        }
      }
    }
  }
  return collisions;
}

/**
 * Validate a scene definition.
 *
 * @returns {{ ok: boolean, errors: string[] }}
 */
export function validateScene(scene) {
  const errors = [];
  if (!scene || typeof scene !== "object") return { ok: false, errors: ["scene is not an object"] };
  if (!scene.id) errors.push("scene has no id");
  const cols = scene.grid?.cols;
  const rows = scene.grid?.rows;
  if (!Number.isFinite(cols) || cols < 1) errors.push("grid.cols must be a positive number");
  if (!Number.isFinite(rows) || rows < 1) errors.push("grid.rows must be a positive number");
  if (!Array.isArray(scene.regions) || !scene.regions.length) errors.push("scene has no regions");

  const ids = new Set();
  for (const region of scene.regions ?? []) {
    if (!region.id) { errors.push("a region has no id"); continue; }
    if (ids.has(region.id)) errors.push(`duplicate region id "${region.id}"`);
    ids.add(region.id);
    if (!region.block) errors.push(`region "${region.id}" names no block`);
    if (!region.area) errors.push(`region "${region.id}" has no area`);
  }

  if (!errors.length) {
    for (const collision of claimGrid(scene.regions, cols, rows)) {
      errors.push(`region "${collision.region}": ${collision.reason}`);
    }
  }
  return { ok: errors.length === 0, errors };
}

/**
 * Apply a scene's grid and safe area to a scene root element.
 *
 * The values travel as custom properties so the CSS grid is generic and a scene
 * is pure data.
 */
export function applySceneFrame(root, scene) {
  root.style.setProperty("--grid-cols", String(scene.grid.cols));
  root.style.setProperty("--grid-rows", String(scene.grid.rows));
  root.style.setProperty("--safe-top", cssLength(scene.safe_area?.top));
  root.style.setProperty("--safe-right", cssLength(scene.safe_area?.right));
  root.style.setProperty("--safe-bottom", cssLength(scene.safe_area?.bottom));
  root.style.setProperty("--safe-left", cssLength(scene.safe_area?.left));
  root.dataset.scene = scene.id;
}

/** Apply a region's placement and overflow policy to its element. */
export function applyRegionFrame(element, region) {
  element.style.setProperty("--area", region.area);
  element.dataset.region = region.id;
  element.dataset.block = region.block;
  element.dataset.overflow = region.overflow || "hidden";
  if (region.rotate) element.dataset.rotate = "1";
  return element;
}

function cssLength(value) {
  if (value == null) return "0";
  return typeof value === "number" ? `${value}px` : String(value);
}

/**
 * Merge a stored user layout over a scene definition.
 *
 * The stored form is deliberately sparse — only what the reader changed — so a
 * scene can gain regions later without the saved layout freezing it.
 */
export function mergeLayout(scene, stored) {
  if (!stored || typeof stored !== "object") return scene;
  const byId = new Map((stored.regions ?? []).map((r) => [r.id, r]));
  const regions = scene.regions
    .filter((r) => !stored.removed?.includes(r.id))
    .map((r) => {
      const patch = byId.get(r.id);
      return patch ? { ...r, ...patch } : r;
    });
  for (const added of stored.added ?? []) {
    if (!regions.some((r) => r.id === added.id)) regions.push(added);
  }
  return { ...scene, regions };
}

/** A sparse diff of `scene` against `base`, suitable for storage. */
export function layoutDiff(base, scene) {
  const baseById = new Map(base.regions.map((r) => [r.id, r]));
  const regions = [];
  const added = [];
  for (const region of scene.regions) {
    const original = baseById.get(region.id);
    if (!original) { added.push(region); continue; }
    const patch = {};
    for (const key of ["area", "block", "overflow", "rotate", "priority"]) {
      if (JSON.stringify(region[key]) !== JSON.stringify(original[key])) patch[key] = region[key];
    }
    if (Object.keys(patch).length) regions.push({ id: region.id, ...patch });
  }
  const removed = base.regions.map((r) => r.id).filter((id) => !scene.regions.some((r) => r.id === id));
  return { regions, added, removed };
}
