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
 * @param {string} kind             cut | fade | slide | zoom | orbit
 * @param {() => void} swap         compose the next scene
 * @param {HTMLElement} stage       the stage element
 * @returns {Promise<void>} resolves when the transition has finished
 */
export async function transition(kind, swap, stage) {
  const ms = DURATION[kind] ?? DURATION.fade;
  if (REDUCED.matches || kind === "cut" || ms === 0) {
    swap();
    return;
  }

  const outgoing = stage.firstElementChild;
  if (outgoing) {
    outgoing.dataset.transitionOut = kind;
    outgoing.style.setProperty("--transition-ms", `${ms}ms`);
  }

  swap();

  const incoming = stage.firstElementChild;
  if (incoming && incoming !== outgoing) {
    incoming.dataset.transitionIn = kind;
    incoming.style.setProperty("--transition-ms", `${ms}ms`);
  }

  await wait(ms);
  if (outgoing && outgoing.isConnected) outgoing.remove();
  if (incoming) {
    delete incoming.dataset.transitionIn;
    incoming.style.removeProperty("--transition-ms");
  }
}

function wait(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/** The transition names the control room offers. */
export const TRANSITIONS = Object.keys(DURATION);
