/* Scene transitions.
 *
 * A transition is a broadcast cut, not decoration. It exists to make the change
 * of scene legible — the viewer should see that the picture changed on purpose,
 * rather than wonder whether the screen glitched. So the vocabulary is small
 * and every option resolves quickly.
 *
 * Nothing here flashes. Repeated high-contrast flashing is fatiguing on a wall
 * display and is a photosensitivity hazard; the alert treatment in the breaking
 * layer follows the same rule.
 *
 * Motion is skipped entirely under `prefers-reduced-motion`, where a transition
 * becomes an instant cut. */

const REDUCED = typeof matchMedia === "function"
  ? matchMedia("(prefers-reduced-motion: reduce)")
  : { matches: false };

/** Duration in milliseconds for a named transition. */
const DURATION = {
  cut: 180,
  fade: 420,
  slide: 520,
  zoom: 560,
  orbit: 720,
};

/**
 * Run a transition between two compositions.
 *
 * The incoming scene is composed while the outgoing one is still on screen, so
 * the viewer never sees an empty stage. `swap` performs the actual DOM change;
 * this function only animates around it.
 *
 * Only `.scene` roots are ever touched. The stage also holds the persistent
 * chrome (`#boot`, `#watermark`), and treating "the first child" as the
 * outgoing scene made each cut delete one of those instead — after three cuts
 * the stage was empty. The scene is the element this function is about, so it
 * is the element this function names.
 *
 * @param {string} kind             cut | fade | slide | zoom | orbit
 * @param {() => void} swap         compose the next scene
 * @param {HTMLElement} stage       the stage element
 * @returns {Promise<void>} resolves when the transition has finished
 */
export async function transition(kind, swap, stage) {
  const ms = DURATION[kind] ?? DURATION.fade;
  const outgoing = lastScene(stage);

  swap();

  const incoming = lastScene(stage);

  // Recomposing one scene reuses its root, so there is nothing to cut between
  // and the change is instant.
  if (!outgoing || outgoing === incoming) return;

  if (REDUCED.matches || kind === "cut" || ms === 0) return;

  outgoing.dataset.transitionOut = kind;
  outgoing.style.setProperty("--transition-ms", `${ms}ms`);
  incoming.dataset.transitionIn = kind;
  incoming.style.setProperty("--transition-ms", `${ms}ms`);

  await wait(ms);

  delete incoming.dataset.transitionIn;
  incoming.style.removeProperty("--transition-ms");
}

/** The scene root most recently appended to the stage. */
function lastScene(stage) {
  const roots = stage.querySelectorAll(":scope > .scene");
  return roots.length ? roots[roots.length - 1] : null;
}

function wait(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/** The transition names the control room offers. */
export const TRANSITIONS = Object.keys(DURATION);
