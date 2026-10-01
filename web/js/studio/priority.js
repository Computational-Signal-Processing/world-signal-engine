/* Broadcast presentation priority.
 *
 * This is *not* the engine's signal type. The engine classifies a change as
 * NOW, ANOMALY, EARLY_SIGNAL, CONVERGENCE or IMPACT — that is a statement about
 * what kind of change it is. Priority is a statement about how loudly the
 * broadcast should say it, and it is ours, not the engine's.
 *
 * Keeping the two apart matters because the thresholds below are editorial. A
 * 6σ anomaly is "HIGH" here because a wall display should cut to it, not
 * because the engine believes anything about importance. If the thresholds are
 * ever wrong, the fix belongs in policy, not in the detection engine. */

export const PRIORITY = {
  INFO: 0,
  LOW: 1,
  MEDIUM: 2,
  HIGH: 3,
  CRITICAL: 4,
};

export const PRIORITY_ORDER = ["INFO", "LOW", "MEDIUM", "HIGH", "CRITICAL"];

/** Default thresholds. Overridable from the control room. */
export const DEFAULT_POLICY = {
  /**
   * The engine's own strength score at which a change becomes worth cutting to.
   *
   * The engine reports `quality.strength` as a bounded 0–1 figure, and it is the
   * engine that decides what "strong" means for a given metric. Grading on that
   * rather than on a raw σ count keeps the editorial thresholds out of the
   * statistics: a 0.3σ move on a very quiet series and a 30σ move on a noisy one
   * are both judged by the engine's own normalisation.
   */
  highStrength: 0.55,
  criticalStrength: 0.8,
  /** Engine types the project treats as corroborated, which raises the floor. */
  corroborating: ["CONVERGENCE", "IMPACT"],
  /** A signal the engine has confirmed outranks one it is still developing. */
  confirmedBonus: 0.08,
};

/**
 * Grade a signal for the broadcast.
 *
 * @param {object} signal   a signal as `/signals` returns it
 * @param {object} policy   thresholds
 * @returns {{ level: string, score: number, why: string }}
 */
export function grade(signal, policy = DEFAULT_POLICY) {
  if (!signal) return { level: "INFO", score: PRIORITY.INFO, why: "no signal" };
  // A caller's policy is a set of overrides, not a replacement: the takeover
  // policy in config carries no grading thresholds at all, and merging keeps a
  // partial policy from silently disabling corroboration.
  const p = { ...DEFAULT_POLICY, ...(policy ?? {}) };

  const types = Array.isArray(signal.types) ? signal.types : [];
  const quality = signal.quality ?? {};
  const strength = numberOrNull(quality.strength);
  const corroborated = types.some((type) => p.corroborating.includes(type));
  const reasons = [];

  let level;
  if (strength == null) {
    // No strength reported is not the same as a small one. Grade on type alone
    // and say so, rather than inventing a magnitude.
    level = types.includes("NOW") ? "MEDIUM" : "LOW";
    reasons.push("no strength reported");
  } else {
    if (strength >= p.criticalStrength) level = "CRITICAL";
    else if (strength >= p.highStrength) level = "HIGH";
    else if (strength >= 0.3) level = "MEDIUM";
    else level = "LOW";
    reasons.push(`strength ${strength.toFixed(2)}`);
  }

  if (signal.status === "CONFIRMED" && PRIORITY[level] < PRIORITY.CRITICAL) {
    reasons.push("confirmed by the engine");
  }

  if (corroborated && PRIORITY[level] < PRIORITY.HIGH) {
    level = "HIGH";
    reasons.push("corroborated by independent sources");
  }
  // A resolved signal is history; it should not interrupt.
  if (signal.status === "RESOLVED" && level === "CRITICAL") level = "HIGH";

  return { level, score: PRIORITY[level], why: reasons.join("; ") };
}

/**
 * The largest deviation a signal carries, in σ.
 *
 * This is a *reading*, not a grade: it is what the engine measured, taken from
 * the per-evidence deviations it reports. It is used for display. The engine's
 * own summary line is preferred when present, because that is the figure the
 * engine itself considers the headline.
 */
export function largestDeviation(signal) {
  const candidates = [];
  const fromSummary = /largest deviation\s+(-?\d+(?:\.\d+)?)\s*σ/i.exec(signal.summary || "");
  if (fromSummary) candidates.push(Math.abs(Number(fromSummary[1])));
  for (const row of signal.evidence ?? []) {
    if (typeof row.deviation_sigma === "number") candidates.push(Math.abs(row.deviation_sigma));
    else {
      const match = /\(([+-]?\d+(?:\.\d+)?)σ/.exec(row.statement || "");
      if (match) candidates.push(Math.abs(Number(match[1])));
    }
  }
  const usable = candidates.filter((n) => Number.isFinite(n));
  return usable.length ? Math.max(...usable) : null;
}

/** A 0–1 quality dimension, or null when the engine did not report one. */
export function quality(signal, dimension) {
  return numberOrNull(signal?.quality?.[dimension]);
}

function numberOrNull(value) {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

export function atLeast(level, floor) {
  return PRIORITY[level] >= PRIORITY[floor];
}
