//! Turning a signal into language a person reads.
//!
//! This is the step the product was missing. Detection produces a series key, a
//! sigma and a duration; a person needs "Software attention is rising sharply —
//! a very large departure from the recent norm, 4 sources". The narrative is
//! built **only** from what the signal's own record supports: its evidence,
//! its types, its direction. Nothing here invents a cause, a place or a
//! consequence the data does not carry, and everything the data does not carry
//! is stated in [`SignalNarrative::unknowns`].

use wse_model::{
    CandidateDirection, DataOrigin, Evidence, Signal, SignalNarrative, SignalStatus, SignalType,
};

use crate::vocabulary::{category_label, entity_vocab, metric_vocab};

/// Build the human-readable narrative for a signal.
///
/// Reads only the signal itself, so it is deterministic, works identically for
/// a freshly formed signal and a merged one, and needs no access to the store.
pub fn narrative_for(signal: &Signal) -> SignalNarrative {
    let best = strongest_evidence(signal);
    let metric = best
        .map(|e| e.metric.clone())
        .unwrap_or_else(|| metric_of(&signal.series_key));
    let vocab = metric_vocab(&metric);

    let subject = subject_for(signal, best, vocab.as_ref());
    let unit = vocab
        .as_ref()
        .map(|v| v.unit_name)
        .filter(|u| !u.is_empty())
        .unwrap_or("units");

    let direction_text = direction_text(signal.direction, is_early_only(signal));
    let (magnitude_text, band) = magnitude_text(best, vocab.as_ref());
    let what_changed = what_changed_text(best, &subject, unit);
    let headline = headline_for(&subject, signal.direction, band, is_early_only(signal));
    let where_text = where_text(signal);
    let why_signal = why_signal_text(signal, band);
    let evidence_sources = signal.distinct_sources();
    let unknowns = unknowns_for(signal, best, vocab.is_some(), evidence_sources);

    SignalNarrative {
        headline,
        subject,
        where_text,
        what_changed,
        direction_text,
        magnitude_text,
        why_signal,
        evidence_sources,
        unknowns,
    }
}

/// The evidence point that best represents the signal: the largest deviation.
fn strongest_evidence(signal: &Signal) -> Option<&Evidence> {
    signal.evidence.iter().max_by(|a, b| {
        let ka = a.deviation_sigma.map(f64::abs).unwrap_or(0.0);
        let kb = b.deviation_sigma.map(f64::abs).unwrap_or(0.0);
        ka.partial_cmp(&kb).unwrap_or(std::cmp::Ordering::Equal)
    })
}

/// The metric name from a `source::entity::metric::unit` series key.
fn metric_of(series_key: &str) -> String {
    let mut parts = series_key.split("::");
    let _source = parts.next();
    let _entity = parts.next();
    parts.next().unwrap_or(series_key).to_string()
}

/// What the change is about, in a person's words.
///
/// Prefers the entity (which can name a place) and falls back to the metric,
/// then to the raw metric name. It never returns an empty string.
fn subject_for(
    signal: &Signal,
    best: Option<&Evidence>,
    vocab: Option<&crate::vocabulary::MetricVocab>,
) -> String {
    if let Some(entity) = signal.entities.first() {
        if let Some(ev) = entity_vocab(entity) {
            return match ev.place {
                Some(place) if !place.is_empty() => {
                    let base = vocab.map(|v| v.short).unwrap_or("activity");
                    format!("{base} in {}", title_case(&place))
                }
                _ => title_case(&ev.short),
            };
        }
    }
    if let Some(v) = vocab {
        return title_case(v.short);
    }
    // No vocabulary: name the metric as it is, rather than pretending.
    let metric = best
        .map(|e| e.metric.clone())
        .unwrap_or_else(|| metric_of(&signal.series_key));
    title_case(&metric.replace('_', " "))
}

fn direction_text(direction: CandidateDirection, early: bool) -> String {
    match (direction, early) {
        (CandidateDirection::Up, true) => "rising steadily".to_string(),
        (CandidateDirection::Down, true) => "falling steadily".to_string(),
        (CandidateDirection::Flat, true) => "drifting sideways".to_string(),
        (CandidateDirection::Up, false) => "rising".to_string(),
        (CandidateDirection::Down, false) => "falling".to_string(),
        (CandidateDirection::Flat, false) => "moving sideways".to_string(),
    }
}

/// How large a deviation reads, in words, plus the band for reuse.
///
/// Returns `(sentence, band)` where band is `"exceptional"`, `"very large"`,
/// `"large"`, `"moderate"` or `"small"`.
fn magnitude_text(
    best: Option<&Evidence>,
    vocab: Option<&crate::vocabulary::MetricVocab>,
) -> (String, &'static str) {
    let sigma = best
        .and_then(|e| e.deviation_sigma)
        .map(f64::abs)
        .unwrap_or(0.0);
    let band = sigma_band(sigma);

    let percent = best
        .and_then(|e| e.baseline.as_ref().map(|b| (e.value, b.mean)))
        .and_then(|(current, mean)| percent_change(current, mean));

    let mut text = format!("A {band} departure from the recent norm");
    if let Some(pct) = percent {
        text.push_str(&format!(" (about {pct:+.0}% versus the baseline average)"));
    }
    if let Some(v) = vocab {
        text.push_str(&format!(". {}", v.movement_meaning));
        if !v.movement_meaning.ends_with('.') {
            text.push('.');
        }
    } else {
        text.push('.');
    }
    (text, band)
}

fn sigma_band(sigma: f64) -> &'static str {
    if sigma >= 10.0 {
        "exceptional"
    } else if sigma >= 5.0 {
        "very large"
    } else if sigma >= 3.5 {
        "large"
    } else if sigma >= 2.0 {
        "moderate"
    } else {
        "small"
    }
}

/// Percent change versus the baseline average, when the baseline supports it.
fn percent_change(current: f64, mean: f64) -> Option<f64> {
    if mean.abs() < 1e-9 {
        return None;
    }
    Some((current - mean) / mean.abs() * 100.0)
}

fn what_changed_text(best: Option<&Evidence>, subject: &str, unit: &str) -> String {
    let Some(best) = best else {
        return format!("{subject} changed, but the detail is not available in this record.");
    };
    match &best.baseline {
        Some(b) => format!(
            "{subject} went from about {from} to {to} {unit}.",
            from = trim_number(b.mean),
            to = trim_number(best.value),
            unit = unit
        ),
        None => format!(
            "{subject} is now {to} {unit}.",
            to = trim_number(best.value),
            unit = unit
        ),
    }
}

fn headline_for(subject: &str, direction: CandidateDirection, band: &str, early: bool) -> String {
    let adverb = match (band, direction) {
        ("exceptional", _) | ("very large", _) => "sharply",
        ("large", _) => "sharply",
        ("moderate", _) => "noticeably",
        _ => "slightly",
    };
    let verb = match direction {
        CandidateDirection::Up => "rising",
        CandidateDirection::Down => "falling",
        CandidateDirection::Flat => "moving sideways",
    };
    if early {
        format!("{subject} is {verb} steadily")
    } else if matches!(direction, CandidateDirection::Flat) {
        format!("{subject} is {verb}")
    } else {
        format!("{subject} is {verb} {adverb}")
    }
}

/// Where the change is, but only when the data actually says so.
fn where_text(signal: &Signal) -> Option<String> {
    if let Some(entity) = signal.entities.first() {
        if let Some(ev) = entity_vocab(entity) {
            if let Some(place) = ev.place {
                if !place.is_empty() {
                    return Some(title_case(&place));
                }
            }
        }
    }
    if signal.location.is_some() {
        // A coordinate is recorded, but naming a place from a point would be
        // inventing geography the data does not contain.
        return Some("a recorded location (see the event for coordinates)".to_string());
    }
    signal
        .categories
        .first()
        .and_then(|c| category_label(c))
        .map(|c| format!("worldwide ({c} coverage)"))
}

fn why_signal_text(signal: &Signal, band: &str) -> String {
    let mut parts = Vec::new();
    if signal.has_type(SignalType::Anomaly) {
        parts.push(format!(
            "the movement is a {band} departure from the recent baseline"
        ));
    }
    if signal.has_type(SignalType::EarlySignal) {
        parts.push(format!(
            "the change is small but has persisted for {} and is still growing",
            human_duration(signal.duration_seconds)
        ));
    }
    if signal.has_type(SignalType::Convergence) {
        parts.push(format!(
            "{} independent sources point at the same change",
            signal.distinct_sources()
        ));
    }
    if signal.has_type(SignalType::Now) {
        parts.push("it was observed in the current window".to_string());
    }
    if signal.has_type(SignalType::Impact) {
        parts.push("it touches a configured impact scope".to_string());
    }
    if parts.is_empty() {
        return "The engine recorded a change outside the recent norm.".to_string();
    }
    let sentence = parts.join("; ");
    let mut chars = sentence.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => sentence,
    }
}

/// What the engine does not know about this signal. Never empty.
///
/// The brief asks the system to be honest about the limits of its own evidence;
/// this is where that is stated rather than left for the reader to guess.
fn unknowns_for(
    signal: &Signal,
    best: Option<&Evidence>,
    has_vocab: bool,
    sources: usize,
) -> Vec<String> {
    let mut out = Vec::new();
    if sources < 2 {
        out.push("No second, independent source currently corroborates this change.".to_string());
    }
    if signal.location.is_none() {
        out.push("The data carries no location, so this cannot be placed on a map.".to_string());
    }
    if let Some(b) = best.and_then(|e| e.baseline.as_ref()) {
        if b.sample_size < 20 {
            out.push(format!(
                "The baseline rests on only {} sample(s); the deviation may still reflect a regime change rather than an anomaly.",
                b.sample_size
            ));
        }
    } else {
        out.push(
            "No baseline is attached to this evidence, so the deviation is uncalibrated."
                .to_string(),
        );
    }
    if signal.evidence.len() <= 1 {
        out.push("A single observation supports this signal so far.".to_string());
    }
    if !has_vocab {
        out.push(
            "This metric has no richer description configured; it is shown by its own name."
                .to_string(),
        );
    }
    if signal.data_origin == DataOrigin::Synthetic {
        out.push("This signal comes from synthetic data, not a live feed.".to_string());
    }
    out.push(
        "Coverage is limited to the connected sources; a quiet feed is not a quiet world."
            .to_string(),
    );
    out
}

/// Where a signal is in its life, from how long it has been observed and how
/// many independent sources stand behind it.
pub fn status_for(
    first_seen: chrono::DateTime<chrono::Utc>,
    last_updated: chrono::DateTime<chrono::Utc>,
    now: chrono::DateTime<chrono::Utc>,
    distinct_sources: usize,
    duration_seconds: i64,
) -> SignalStatus {
    let age = (now - first_seen).num_seconds().max(0);
    let since_update = (now - last_updated).num_seconds().max(0);
    // A signal that has not been touched for several times its own lifetime is
    // over. Scaling by the signal's duration is what keeps a daily source's
    // signal alive for a day while retiring a minute-cadence one within hours.
    let stale_after = 3 * duration_seconds.max(1800);
    if since_update > stale_after && since_update > 3600 {
        return SignalStatus::Resolved;
    }
    if age < 60 {
        return SignalStatus::New;
    }
    if distinct_sources >= 2 || duration_seconds >= 3600 {
        if since_update > 1800 {
            return SignalStatus::Fading;
        }
        return SignalStatus::Confirmed;
    }
    if since_update > 900 {
        return SignalStatus::Stable;
    }
    SignalStatus::Developing
}

fn is_early_only(signal: &Signal) -> bool {
    signal.has_type(SignalType::EarlySignal) && !signal.has_type(SignalType::Anomaly)
}

fn human_duration(seconds: i64) -> String {
    let s = seconds.max(0);
    if s < 60 {
        format!("{s} seconds")
    } else if s < 3600 {
        format!("{} minutes", s / 60)
    } else if s < 86_400 {
        format!("{} hours", s / 3600)
    } else {
        format!("{} days", s / 86_400)
    }
}

/// A number with trailing zeros trimmed, for reading inside a sentence.
fn trim_number(value: f64) -> String {
    if !value.is_finite() {
        return "—".to_string();
    }
    let s = format!("{value:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    s.to_string()
}

fn title_case(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use wse_model::{BaselineSnapshot, EntityId, EventId, ObservationId, SourceId};

    fn snapshot(mean: f64, sample_size: usize) -> BaselineSnapshot {
        BaselineSnapshot {
            sample_size,
            mean,
            median: mean,
            std_dev: 1.0,
            mad: 1.0,
            p05: mean - 1.0,
            p95: mean + 1.0,
            ewma: mean,
            trend_per_second: 0.0,
            volatility: 1.0,
        }
    }

    fn signal_with(types: &[SignalType], direction: CandidateDirection) -> Signal {
        let now = Utc::now();
        let mut s = Signal::new(EventId::new("evt_1"), now);
        for ty in types {
            s.add_type(*ty);
        }
        s.direction = direction;
        s.series_key = "hackernews_frontpage::software_ecosystem::story_score::points".into();
        s.entities.push(EntityId::new("software_ecosystem"));
        s.categories.push("technology".into());
        s.evidence.push(Evidence {
            source_id: SourceId::new("hackernews_frontpage"),
            observation_id: ObservationId::new("obs_1"),
            metric: "story_score".into(),
            unit: "points".into(),
            statement: "story_score 623 points".into(),
            observed_at: now,
            value: 623.0,
            deviation_sigma: Some(5.9),
            baseline: Some(snapshot(150.0, 40)),
            identity: Some("123".into()),
            record_label: Some("A new kind of database".into()),
        });
        s
    }

    #[test]
    fn headline_is_human_language_not_a_metric_name() {
        let s = signal_with(
            &[SignalType::Anomaly, SignalType::Now],
            CandidateDirection::Up,
        );
        let n = narrative_for(&s);
        assert!(
            n.headline.contains("Software attention"),
            "headline should name the subject, got {}",
            n.headline
        );
        assert!(
            !n.headline.contains("story_score"),
            "headline must not leak the raw metric"
        );
    }

    #[test]
    fn magnitude_is_described_in_words_with_a_percentage() {
        let s = signal_with(&[SignalType::Anomaly], CandidateDirection::Up);
        let n = narrative_for(&s);
        assert!(
            n.magnitude_text.contains("very large"),
            "{}",
            n.magnitude_text
        );
        // (623 - 150) / 150 = +315%
        assert!(n.magnitude_text.contains("+315%"), "{}", n.magnitude_text);
    }

    #[test]
    fn unknowns_are_always_populated() {
        let s = signal_with(&[SignalType::Anomaly], CandidateDirection::Up);
        let n = narrative_for(&s);
        assert!(!n.unknowns.is_empty());
        assert!(n
            .unknowns
            .iter()
            .any(|u| u.contains("second, independent source")));
    }

    #[test]
    fn a_region_signal_names_the_place() {
        let now = Utc::now();
        let mut s = Signal::new(EventId::new("evt_1"), now);
        s.add_type(SignalType::Anomaly);
        s.direction = CandidateDirection::Up;
        s.series_key = "usgs_earthquakes::region_hormuz::earthquake_magnitude::magnitude".into();
        s.entities.push(EntityId::new("region_hormuz"));
        s.categories.push("geophysics".into());
        s.evidence.push(Evidence {
            source_id: SourceId::new("usgs_earthquakes"),
            observation_id: ObservationId::new("obs_1"),
            metric: "earthquake_magnitude".into(),
            unit: "magnitude".into(),
            statement: "s".into(),
            observed_at: now,
            value: 5.4,
            deviation_sigma: Some(4.2),
            baseline: Some(snapshot(2.5, 60)),
            identity: None,
            record_label: None,
        });
        let n = narrative_for(&s);
        assert!(n.subject.contains("Hormuz"), "{}", n.subject);
        assert_eq!(n.where_text.as_deref(), Some("Hormuz"));
        assert!(n.headline.contains("Hormuz"), "{}", n.headline);
    }

    #[test]
    fn an_unmapped_metric_is_named_not_hidden() {
        let now = Utc::now();
        let mut s = Signal::new(EventId::new("evt_1"), now);
        s.add_type(SignalType::Anomaly);
        s.series_key = "src_x::ent_y::mystery_metric::units".into();
        s.evidence.push(Evidence {
            source_id: SourceId::new("src_x"),
            observation_id: ObservationId::new("obs_1"),
            metric: "mystery_metric".into(),
            unit: "units".into(),
            statement: "s".into(),
            observed_at: now,
            value: 1.0,
            deviation_sigma: Some(4.0),
            baseline: None,
            identity: None,
            record_label: None,
        });
        let n = narrative_for(&s);
        assert!(n.subject.contains("Mystery metric"), "{}", n.subject);
        assert!(n
            .unknowns
            .iter()
            .any(|u| u.contains("no richer description")));
    }

    #[test]
    fn status_reflects_age_sources_and_recency() {
        let now = Utc::now();
        assert_eq!(
            status_for(now, now, now, 1, 0),
            SignalStatus::New,
            "just formed"
        );
        let two_min_ago = now - chrono::Duration::seconds(120);
        assert_eq!(
            status_for(two_min_ago, now, now, 1, 120),
            SignalStatus::Developing
        );
        assert_eq!(
            status_for(two_min_ago, now, now, 3, 120),
            SignalStatus::Confirmed,
            "multiple sources confirm"
        );
        let stale = now - chrono::Duration::seconds(7200);
        assert_eq!(
            status_for(stale, stale, now, 1, 600),
            SignalStatus::Resolved
        );
    }
}
