/* The engine's own account of a signal.
 *
 * The engine does not return a sentence — it returns the parts of one: what the
 * subject was, where it happened, what changed, which way it moved, how big the
 * move was, why the engine raised it, and what it does not know. That structure
 * is preserved rather than flattened into a paragraph, because the "unknowns"
 * list is the most useful thing on the screen: it is the engine saying which
 * parts of the picture are missing.
 *
 * The narrative is optional. When the engine has not written one the block
 * renders nothing, and the caller simply shows no narrative section. */

import { el } from "../dom.js";
import { t } from "../i18n.js";

/** True when the engine attached a usable narrative. */
export function hasNarrative(signal) {
  const n = signal?.narrative;
  if (!n || typeof n !== "object") return false;
  return Boolean(n.headline || n.what_changed || n.why_signal || (n.unknowns ?? []).length);
}

/**
 * Build the narrative section.
 *
 * @returns {HTMLElement|null} null when there is nothing to show
 */
export function narrativeBlock(signal) {
  const n = signal?.narrative;
  if (!hasNarrative(signal)) return null;

  const statements = [
    n.what_changed ? row("changed", n.what_changed) : null,
    n.magnitude_text ? row("magnitude", n.magnitude_text) : null,
    n.why_signal ? row("raised", n.why_signal) : null,
    n.where_text ? row(t().field.location, n.where_text) : null,
  ].filter(Boolean);

  const unknowns = Array.isArray(n.unknowns) ? n.unknowns.filter(Boolean) : [];

  return el("section", { class: "card-narrative" }, [
    el("div", { class: "label", text: "narrative" }),
    n.subject ? el("div", { class: "narrative-subject dim", text: n.subject }) : null,
    el("div", { class: "narrative-rows" }, statements),
    unknowns.length
      ? el("div", { class: "narrative-unknowns" }, [
        el("div", { class: "label", text: "unknowns" }),
        el("ul", {}, unknowns.map((u) => el("li", { text: u }))),
      ])
      : null,
  ]);
}

function row(label, value) {
  return el("div", { class: "narrative-row" }, [
    el("span", { class: "narrative-label faint", text: label }),
    el("span", { class: "narrative-value", text: value }),
  ]);
}
