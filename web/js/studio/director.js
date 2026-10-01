/* The event director.
 *
 * It answers one question: given what just arrived, what should the broadcast
 * do? It decides between four actions — refresh the regions, feed the ticker,
 * cut to another scene, or take over the screen — and it does that by reading a
 * policy, not by a chain of `if (signal.type === ...)`.
 *
 * The rules live in config/studio/policies/takeover.json so the thresholds can
 * be tuned without touching this file, and so the control room can edit them.
 * The director never composes a scene itself; it tells the studio what it wants
 * and the studio orchestrator does it. Those responsibilities stay separate. */

import { PRIORITY, grade, atLeast } from "./priority.js";

/** Fallback policy. The shipped one is fetched from config and merged over this. */
export const DEFAULT_TAKEOVER = {
  /** Auto direction is off by default: an unattended screen that changes scene
   * on its own is surprising, and during development it makes debugging hard. */
  enabled: false,
  min_priority: "HIGH",
  cooldown_seconds: 120,
  dismiss_after_seconds: 30,
  transition: "fade",
  rules: [
    { when: "priority >= CRITICAL", action: "takeover" },
    { when: "priority >= HIGH", action: "transition", to: "signal" },
    { when: "default", action: "update" },
  ],
};

export class Director {
  /**
   * @param {import("../data/store.js").Store} store
   * @param {object} options
   * @param {(sceneId: string, opts: object) => void} options.cut    ask the studio to change scene
   * @param {(signal: object, grade: object) => void} options.takeover
   * @param {(item: object) => void} options.headline                 feed the ticker
   * @param {(kind: string, data: object) => void} options.refresh    push data into regions
   */
  constructor(store, { cut, takeover, headline, refresh } = {}) {
    this.store = store;
    this.cut = cut;
    this.takeover = takeover;
    this.headline = headline;
    this.refresh = refresh;

    this.policy = { ...DEFAULT_TAKEOVER };
    this.lastTakeover = new Map();   // signal id -> timestamp
    this.lastTransition = 0;
    this.enabled = this.policy.enabled;
    this.log = [];
    this.maxLog = 40;
  }

  /** Replace the policy, e.g. from config or the control room. */
  setPolicy(patch) {
    this.policy = { ...this.policy, ...patch };
    if (typeof patch.enabled === "boolean") this.enabled = patch.enabled;
  }

  setEnabled(on) {
    this.enabled = Boolean(on);
    this.policy.enabled = this.enabled;
    this.note(on ? "auto director on" : "auto director off");
  }

  /**
   * Handle a batch of signals from the store.
   *
   * Called whenever the signal list refreshes or an SSE event names one. Each
   * signal is graded, the strongest is acted on, and everything is recorded in
   * the director's log so the control room can show why the screen did what it
   * did. A broadcast that changes scene without a stated reason is not
   * explainable, and this project's rule is that every signal is.
   */
  observe(signals) {
    if (!Array.isArray(signals) || !signals.length) return null;
    let strongest = null;
    for (const signal of signals) {
      const graded = grade(signal, this.policy);
      if (!strongest || graded.score > strongest.grade.score) strongest = { signal, grade: graded };
    }
    if (!strongest) return null;

    this.note(`${strongest.signal.id}: ${strongest.grade.level} (${strongest.grade.why})`);
    if (!this.enabled) return strongest;

    const decision = this.decide(strongest.signal, strongest.grade);
    this.note(`→ ${decision.action}${decision.to ? ` ${decision.to}` : ""}`);
    this.act(decision, strongest.signal, strongest.grade);
    return strongest;
  }

  /**
   * Resolve the policy to an action.
   *
   * The expression grammar is deliberately tiny — `priority >= LEVEL`, an
   * optional `&& types has TYPE`, or `default` — because it has to be readable
   * and editable from the control room, and because a general expression
   * evaluator here would be a bug surface with no payoff.
   */
  decide(signal, graded) {
    const floor = this.policy.min_priority;
    for (const rule of this.policy.rules) {
      if (rule.when === "default") continue;
      if (!this.matches(rule.when, signal, graded, floor)) continue;
      return { action: rule.action, to: rule.to, transition: rule.transition || this.policy.transition };
    }
    const fallback = this.policy.rules.find((r) => r.when === "default");
    return { action: fallback?.action ?? "update", to: fallback?.to };
  }

  matches(expression, signal, graded, floor) {
    const parts = String(expression).split("&&").map((p) => p.trim());
    return parts.every((part) => {
      let m = /^priority\s*>=\s*(\w+)$/.exec(part);
      if (m) {
        if (!atLeast(graded.level, m[1])) return false;
        // Even when a rule matches, the policy floor still applies: raising a
        // single threshold in the control room must not be undone by a rule.
        return atLeast(graded.level, floor);
      }
      m = /^types\s+has\s+(\w+)$/.exec(part);
      if (m) return (signal.types ?? []).includes(m[1]);
      m = /^status\s*==\s*(\w+)$/.exec(part);
      if (m) return signal.status === m[1];
      m = /^sigma\s*>=\s*(\d+(?:\.\d+)?)$/.exec(part);
      if (m) return (graded.why || "").includes(`${m[1]}σ`);
      return false;
    });
  }

  act(decision, signal, graded) {
    switch (decision.action) {
      case "takeover": {
        if (!this.mayTakeover(signal)) return;
        this.lastTakeover.set(signal.id, Date.now());
        this.takeover?.(signal, graded, decision);
        break;
      }
      case "transition": {
        if (!this.mayTransition()) return;
        this.lastTransition = Date.now();
        this.cut?.(decision.to, { reason: `priority ${graded.level}`, signal });
        break;
      }
      case "ticker":
        this.headline?.({ glyph: "•", text: signal.title, signal });
        break;
      default:
        this.refresh?.(signal);
    }
  }

  /** Cooldown: the same signal must not re-take the screen every refresh. */
  mayTakeover(signal) {
    const last = this.lastTakeover.get(signal.id);
    if (last == null) return true;
    return Date.now() - last > this.policy.cooldown_seconds * 1000;
  }

  /** Scenes must not flip faster than a viewer can read the reason for the cut. */
  mayTransition() {
    return Date.now() - this.lastTransition > 8_000;
  }

  /** Clear a takeover's cooldown, e.g. after the operator closed it. */
  release(signalId) {
    this.lastTakeover.delete(signalId);
  }

  note(message) {
    this.log.push({ at: Date.now(), message });
    if (this.log.length > this.maxLog) this.log.shift();
  }

  /** The recent decisions, newest first — shown in the control room. */
  recent() { return [...this.log].reverse(); }
}
