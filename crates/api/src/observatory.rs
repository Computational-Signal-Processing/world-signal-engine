//! The observatory: one screen that shows the whole watched world at once.
//!
//! This is a *view*, not a new system. It reads the same signals the feed does
//! and the same observations the timeline does, and composes them into the
//! shapes a single-screen control room needs: per-category rollups, a severity
//! ordered feed, and a breaking ticker.
//!
//! Two rules from `docs/decisions/0026-observatory-ui.md` are enforced here,
//! because this is the loudest surface in the product and the easiest place to
//! fake a number:
//!
//! 1. **Every figure is a real measurement.** A category card carries the
//!    current value of a real series, the baseline it was compared against, the
//!    change, and the sparkline of that series. Nothing is a synthetic "risk
//!    index".
//! 2. **No data is not zero.** A category whose sources have never produced a
//!    series reports `has_data: false` and a reason, not a quiet `0`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use wse_model::{BaselineSnapshot, Signal, SignalStatus, SignalType};
use wse_storage::{ObservationQuery, SignalQuery};

/// How a category or a signal is flagged on the board.
///
/// This is a *display* severity derived from the signal's own quality and
/// deviation, never a stored verdict. The derivation is returned alongside the
/// label (`severity_reason`) so the board can say *why* a row is CRITICAL
/// instead of asserting it with a colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Severity {
    Low,
    Medium,
    Critical,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Low => "LOW",
            Severity::Medium => "MEDIUM",
            Severity::Critical => "CRITICAL",
        }
    }
}

/// One point of a sparkline: enough to draw the shape, nothing more.
#[derive(Debug, Clone, Serialize)]
pub struct SparkPoint {
    pub at: DateTime<Utc>,
    pub value: f64,
}

/// A per-category rollup, backed by one real series.
#[derive(Debug, Clone, Serialize)]
pub struct CategoryCard {
    /// The catalog category, e.g. `space`, `geophysics`.
    pub category: String,
    /// A human label, from the presentation vocabulary.
    pub label: String,
    /// The series this card measures. Empty when no series backs the category.
    pub series_key: String,
    /// Whether a real series backs this category at all.
    ///
    /// `false` means "no connected source has produced a measurable series in
    /// this category" — a fact to show, not a zero to draw.
    pub has_data: bool,
    /// Why the card is empty, when it is. Populated only when `has_data` is
    /// false, so the board never shows a blank card without a reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub empty_reason: Option<String>,
    /// Current value of the series (the newest observation).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    pub unit: String,
    /// The baseline the value was compared against, when one exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline: Option<BaselineSnapshot>,
    /// Percent change from the baseline median. `None` when there is no
    /// baseline or the median is zero (a percent of zero is undefined, not
    /// infinite).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change_pct: Option<f64>,
    /// Robust z-score of the current value against the baseline.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deviation_sigma: Option<f64>,
    /// The sparkline: recent points of this series, oldest first.
    pub sparkline: Vec<SparkPoint>,
    /// How many signals in this category are currently active.
    pub active_signals: usize,
    /// The strongest signal type present, for the card's badge. `None` when no
    /// signal is active.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_type: Option<String>,
    pub severity: Severity,
    /// The plain statement behind `severity`, e.g. "3.4σ, 1 source, 12 min".
    pub severity_reason: String,
}

/// One row of the live feed.
#[derive(Debug, Clone, Serialize)]
pub struct FeedItem {
    pub signal_id: String,
    pub title: String,
    pub summary: String,
    pub types: Vec<String>,
    pub status: String,
    pub severity: Severity,
    pub severity_reason: String,
    pub direction: String,
    /// The largest absolute deviation across the signal's evidence, in robust
    /// sigma. `None` when no evidence states one — the UI then says so rather
    /// than printing a zero.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deviation_sigma: Option<f64>,
    /// Detector confidence, `0..=1`, as the signal stores it.
    pub confidence: f64,
    /// The series this signal is about. Display only — not the signal's identity.
    pub series_key: String,
    pub first_seen: DateTime<Utc>,
    pub last_updated: DateTime<Utc>,
    pub age_seconds: i64,
    pub duration_seconds: i64,
    /// Where, when the data supports a place. `None` otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<wse_model::Location>,
    /// The sources behind it, for the "which sensors" line.
    pub sources: Vec<String>,
    /// Categories it touches.
    pub categories: Vec<String>,
    /// Whether the data is live or synthetic. Never hidden.
    pub data_origin: String,
}

/// A ticker line: one breaking item, short enough to read as it scrolls.
#[derive(Debug, Clone, Serialize)]
pub struct TickerItem {
    pub signal_id: String,
    pub severity: Severity,
    pub text: String,
    pub at: DateTime<Utc>,
}

/// One bucket of the activity chart: how many observations arrived in a window.
///
/// This is a real count of stored observations, not a synthetic "activity
/// index". It is what lets the board show the world's own cadence against its
/// recent norm.
#[derive(Debug, Clone, Serialize)]
pub struct ActivityBucket {
    /// Start of the bucket.
    pub at: DateTime<Utc>,
    pub count: usize,
}

/// The activity series the chart plots, plus the norm it is compared against.
#[derive(Debug, Clone, Serialize)]
pub struct ActivitySeries {
    /// `24H` or `7D`.
    pub window: String,
    pub buckets: Vec<ActivityBucket>,
    /// Mean observations per bucket over the window — the dashed baseline.
    pub baseline: f64,
    /// The newest bucket's count.
    pub current: usize,
    /// Buckets at or above twice the baseline. Stated as a count, not a score.
    pub elevated_buckets: usize,
}

/// The activity chart's window: how far back it looks and how finely it buckets.
#[derive(Debug, Clone)]
pub struct ActivityWindow {
    pub label: String,
    pub seconds: i64,
    pub bucket_seconds: i64,
}

impl ActivityWindow {
    /// Parse the `window` query parameter. Unknown values fall back to `24H`
    /// rather than erroring: a bad display parameter should not blank a screen
    /// that a person may be watching.
    pub fn parse(raw: Option<&str>) -> Self {
        match raw.map(str::to_ascii_lowercase).as_deref() {
            Some("7d") | Some("7") => ActivityWindow {
                label: "7D".to_string(),
                seconds: 7 * 24 * 3600,
                bucket_seconds: 6 * 3600,
            },
            _ => ActivityWindow {
                label: "24H".to_string(),
                seconds: 24 * 3600,
                bucket_seconds: 3600,
            },
        }
    }
}

/// The whole board, in one response.
#[derive(Debug, Clone, Serialize)]
pub struct ObservatorySnapshot {
    pub generated_at: DateTime<Utc>,
    /// Which categories the board shows, in priority order.
    pub categories: Vec<CategoryCard>,
    pub activity: ActivitySeries,
    pub feed: Vec<FeedItem>,
    pub ticker: Vec<TickerItem>,
    pub active_signals: usize,
    pub signals_total: usize,
    pub observations_total: usize,
    pub sources_total: usize,
    pub sources_healthy: usize,
    /// Whether collection is running at all. `false` means the board is a
    /// frozen snapshot, and it must say so rather than looking live.
    pub monitoring: bool,
    pub collection_enabled: bool,
    /// Age of the newest stored data, in seconds. `None` when nothing has ever
    /// been collected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_age_seconds: Option<i64>,
    /// The alert modal: the single most severe active signal, when one is at
    /// CRITICAL. `None` when nothing crosses the line — the modal is not
    /// decorative and does not appear for a routine signal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alert: Option<FeedItem>,
}

/// Derive a display severity from a signal's own numbers.
///
/// The rule is stated rather than tuned to look good: CRITICAL needs a large
/// deviation *and* independent backing; MEDIUM is a clear deviation or a
/// sustained multi-source change; everything else is LOW. A signal that is
/// `FADING` or `RESOLVED` is never CRITICAL — a receding change is not an
/// emergency.
pub fn severity_of(signal: &Signal) -> (Severity, String) {
    let sigma = signal
        .evidence
        .iter()
        .filter_map(|e| e.deviation_sigma)
        .fold(0.0f64, |acc, s| if s.abs() > acc.abs() { s } else { acc });
    let sources = signal
        .evidence
        .iter()
        .map(|e| e.source_id.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let minutes = signal.duration_seconds / 60;
    let settled = matches!(signal.status, SignalStatus::Fading | SignalStatus::Resolved);

    let magnitude = if sigma == 0.0 {
        "no sigma".to_string()
    } else {
        format!("{sigma:+.1}σ")
    };
    let reason = format!("{magnitude}, {sources} source(s), {minutes} min");

    if settled {
        return (Severity::Low, format!("{reason} (receding)"));
    }
    if sigma.abs() >= 6.0 && sources >= 2 {
        return (Severity::Critical, reason);
    }
    if sigma.abs() >= 6.0 || (sigma.abs() >= 3.5 && sources >= 2) {
        return (Severity::Medium, reason);
    }
    (Severity::Low, reason)
}

fn signal_types(signal: &Signal) -> Vec<String> {
    signal
        .types
        .iter()
        .map(|t| t.as_str().to_string())
        .collect()
}

/// The strongest type present, by the type order the UI renders.
fn top_type(signals: &[&Signal]) -> Option<String> {
    let order = [
        SignalType::Convergence,
        SignalType::Anomaly,
        SignalType::Now,
        SignalType::EarlySignal,
        SignalType::Impact,
    ];
    order
        .iter()
        .find(|ty| signals.iter().any(|s| s.types.contains(ty)))
        .map(|ty| ty.as_str().to_string())
}

fn feed_item(signal: &Signal, now: DateTime<Utc>) -> FeedItem {
    let (severity, severity_reason) = severity_of(signal);
    let mut sources: Vec<String> = signal
        .evidence
        .iter()
        .map(|e| e.source_id.as_str().to_string())
        .collect();
    sources.sort();
    sources.dedup();
    FeedItem {
        signal_id: signal.id.as_str().to_string(),
        title: if signal.narrative.headline.is_empty() {
            signal.title.clone()
        } else {
            signal.narrative.headline.clone()
        },
        summary: signal.summary.clone(),
        types: signal_types(signal),
        status: signal.status.as_str().to_string(),
        severity,
        severity_reason,
        direction: signal.direction.as_str().to_string(),
        deviation_sigma: signal
            .evidence
            .iter()
            .filter_map(|e| e.deviation_sigma)
            .max_by(|a, b| a.abs().total_cmp(&b.abs())),
        confidence: signal.confidence,
        series_key: signal.series_key.clone(),
        first_seen: signal.first_seen,
        last_updated: signal.last_updated,
        age_seconds: (now - signal.last_updated).num_seconds(),
        duration_seconds: signal.duration_seconds,
        location: signal.location.clone(),
        sources,
        categories: signal.categories.clone(),
        data_origin: format!("{:?}", signal.data_origin).to_uppercase(),
    }
}

/// Build the observatory board from the store.
///
/// `categories` is the ordered list the board shows. Each is rolled up from the
/// store's real series; a category with no series says so.
pub fn build_snapshot<S: wse_storage::Store>(
    engine: &wse_engine::Engine<S>,
    category_order: &[&str],
    window: &ActivityWindow,
    now: DateTime<Utc>,
) -> ObservatorySnapshot {
    let store = engine.store();

    let all_signals = store
        .query_signals(&SignalQuery::default())
        .map(|p| p.items)
        .unwrap_or_default();
    let active: Vec<Signal> = store
        .query_signals(&SignalQuery::active())
        .map(|p| p.items)
        .unwrap_or_default();

    let sources = store.all_sources().unwrap_or_default();
    let health: Vec<Option<wse_model::SourceHealth>> = sources
        .iter()
        .map(|s| store.get_health(&s.id).ok().flatten())
        .collect();
    let sources_healthy = health
        .iter()
        .filter(|h| h.as_ref().map(|h| h.status) == Some(wse_model::HealthStatus::Healthy))
        .count();

    let categories: Vec<CategoryCard> = category_order
        .iter()
        .map(|cat| category_card(store, cat, &sources, &active))
        .collect();

    let mut feed: Vec<FeedItem> = active.iter().map(|s| feed_item(s, now)).collect();
    // Rank like the feed does, but with severity breaking ties so the board's
    // order matches the label the reader sees.
    feed.sort_by(|a, b| {
        let rank = |s: &FeedItem| match s.severity {
            Severity::Critical => 2,
            Severity::Medium => 1,
            Severity::Low => 0,
        };
        rank(b)
            .cmp(&rank(a))
            .then_with(|| b.duration_seconds.cmp(&a.duration_seconds))
    });

    let alert = feed
        .iter()
        .find(|f| f.severity == Severity::Critical)
        .cloned();

    let ticker: Vec<TickerItem> = feed
        .iter()
        .take(12)
        .map(|f| TickerItem {
            signal_id: f.signal_id.clone(),
            severity: f.severity,
            text: format!("{} — {}", f.types.join("/"), f.title),
            at: f.last_updated,
        })
        .collect();

    let observations_total = store.observation_count().unwrap_or(0);
    let activity = activity_series(
        store,
        window.seconds,
        window.bucket_seconds,
        &window.label,
        now,
    );
    let data_age_seconds = store
        .query_observations(&ObservationQuery {
            limit: Some(1),
            ..Default::default()
        })
        .ok()
        .and_then(|page| page.items.into_iter().map(|o| o.received_at).max())
        .map(|at| (now - at).num_seconds());

    ObservatorySnapshot {
        generated_at: now,
        categories,
        activity,
        feed,
        ticker,
        active_signals: active.len(),
        signals_total: all_signals.len(),
        observations_total,
        sources_total: sources.len(),
        sources_healthy,
        monitoring: engine.runtime().monitoring(),
        collection_enabled: engine.runtime().collection_enabled(),
        data_age_seconds,
        alert,
    }
}

/// Roll one category up from the store's real series.
///
/// A category is a set of **sources** (the catalog declares each source's
/// category). The card measures the series with the most observations among
/// those sources, so it shows the category's dominant quantity even when no
/// signal is active — a board that went blank whenever the world was calm would
/// make calm look like blindness. The category's active signals are reported
/// separately.
///
/// When no source in the category has produced a series, the card carries the
/// reason instead of a fabricated zero.
fn category_card<S: wse_storage::Store>(
    store: &S,
    category: &str,
    sources: &[wse_model::Source],
    active: &[Signal],
) -> CategoryCard {
    let label = wse_presentation::vocabulary::category_label(category)
        .map(str::to_string)
        .unwrap_or_else(|| category.to_string());

    let in_category: Vec<&Signal> = active
        .iter()
        .filter(|s| s.categories.iter().any(|c| c == category))
        .collect();

    // The series with the most observations among this category's sources is
    // the card's measured quantity, so the card is stable between cycles
    // rather than flickering between series.
    let mut best: Option<(usize, String)> = None;
    for source in sources.iter().filter(|s| s.category == category) {
        let observations = match store.query_observations(&ObservationQuery {
            source_id: Some(source.id.as_str().to_string()),
            limit: Some(200),
            ..Default::default()
        }) {
            Ok(page) => page.items,
            Err(_) => continue,
        };
        let Some(last) = observations.last() else {
            continue;
        };
        let key = last.series_key();
        if best
            .as_ref()
            .is_none_or(|(count, _)| observations.len() > *count)
        {
            best = Some((observations.len(), key));
        }
    }

    let Some((_, series_key)) = best else {
        return empty_card(category, label, in_category.len());
    };

    let observations = store
        .query_observations(&ObservationQuery::for_series(series_key.clone()).with_limit(120))
        .map(|p| p.items)
        .unwrap_or_default();
    if observations.is_empty() {
        return empty_card(category, label, in_category.len());
    }

    let baseline = wse_storage::BaselineStore::get_baseline(store, &series_key)
        .ok()
        .flatten()
        .map(|(_, snapshot)| snapshot);

    let newest = observations.last().expect("non-empty checked above");
    let value = newest.value;
    let unit = newest.unit.clone();

    let change_pct = baseline
        .as_ref()
        .filter(|b| b.median.abs() > f64::EPSILON)
        .map(|b| (value - b.median) / b.median * 100.0);
    let deviation_sigma = baseline
        .as_ref()
        .map(|b| wse_model::robust_z_score(value, b.median, b.mad))
        .filter(|z| *z != 0.0);

    let sparkline: Vec<SparkPoint> = observations
        .iter()
        .rev()
        .take(40)
        .rev()
        .map(|o| SparkPoint {
            at: o.observed_at,
            value: o.value,
        })
        .collect();

    let (severity, severity_reason) = in_category
        .iter()
        .map(|s| severity_of(s))
        .max_by_key(|(sev, _)| match sev {
            Severity::Critical => 2,
            Severity::Medium => 1,
            Severity::Low => 0,
        })
        .unwrap_or((Severity::Low, "no active signal".to_string()));

    CategoryCard {
        category: category.to_string(),
        label,
        series_key,
        has_data: true,
        empty_reason: None,
        value: Some(value),
        unit,
        baseline,
        change_pct,
        deviation_sigma,
        sparkline,
        active_signals: in_category.len(),
        top_type: top_type(&in_category),
        severity,
        severity_reason,
    }
}

/// Build the activity series the chart plots.
///
/// Counts stored observations into fixed buckets over `window_seconds`. This is
/// the world's own arrival rate, measured — the chart's dashed baseline is the
/// mean of those buckets, not a constant invented to make the curve look
/// interesting. An empty store yields zero buckets and a zero baseline, and the
/// chart draws nothing rather than a flat line at a made-up level.
fn activity_series<S: wse_storage::Store>(
    store: &S,
    window_seconds: i64,
    bucket_seconds: i64,
    window_label: &str,
    now: DateTime<Utc>,
) -> ActivitySeries {
    let from = now - chrono::Duration::seconds(window_seconds);
    let observations = store
        .query_observations(&ObservationQuery {
            range: Some(wse_storage::TimeRange::new(from, now)),
            limit: Some(50_000),
            ..Default::default()
        })
        .map(|p| p.items)
        .unwrap_or_default();

    let bucket_count = (window_seconds / bucket_seconds).max(1) as usize;
    let mut counts = vec![0usize; bucket_count];
    for observation in &observations {
        let offset = (observation.received_at - from).num_seconds();
        if offset < 0 {
            continue;
        }
        let index = (offset / bucket_seconds) as usize;
        if index < bucket_count {
            counts[index] += 1;
        }
    }

    let buckets: Vec<ActivityBucket> = counts
        .iter()
        .enumerate()
        .map(|(i, count)| ActivityBucket {
            at: from + chrono::Duration::seconds(i as i64 * bucket_seconds),
            count: *count,
        })
        .collect();

    let baseline = if buckets.is_empty() {
        0.0
    } else {
        counts.iter().sum::<usize>() as f64 / buckets.len() as f64
    };
    let current = buckets.last().map(|b| b.count).unwrap_or(0);
    let elevated_buckets = counts
        .iter()
        .filter(|c| baseline > 0.0 && **c as f64 >= 2.0 * baseline)
        .count();

    ActivitySeries {
        window: window_label.to_string(),
        buckets,
        baseline,
        current,
        elevated_buckets,
    }
}

fn empty_card(category: &str, label: String, active_signals: usize) -> CategoryCard {
    CategoryCard {
        category: category.to_string(),
        label,
        series_key: String::new(),
        has_data: false,
        empty_reason: Some(
            "no connected source has produced a measurable series in this category".to_string(),
        ),
        value: None,
        unit: String::new(),
        baseline: None,
        change_pct: None,
        deviation_sigma: None,
        sparkline: Vec::new(),
        active_signals,
        top_type: None,
        severity: Severity::Low,
        severity_reason: "no data — this is not a quiet reading".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wse_model::{Evidence, ObservationId, SignalId, SourceId};

    fn signal_with(sigma: f64, sources: usize, status: SignalStatus, minutes: i64) -> Signal {
        let mut signal = Signal::new(wse_model::EventId::new("evt_x"), Utc::now());
        signal.id = SignalId::new("sig_x");
        signal.status = status;
        signal.duration_seconds = minutes * 60;
        signal.evidence = (0..sources)
            .map(|i| Evidence {
                source_id: SourceId::new(format!("src_{i}")),
                observation_id: ObservationId::new(format!("obs_{i}")),
                metric: "m".into(),
                unit: "u".into(),
                statement: "s".into(),
                observed_at: Utc::now(),
                value: 1.0,
                deviation_sigma: Some(sigma),
                baseline: None,
                identity: None,
                record_label: None,
            })
            .collect();
        signal
    }

    #[test]
    fn critical_needs_a_large_deviation_and_independent_backing() {
        let (sev, reason) = severity_of(&signal_with(7.0, 2, SignalStatus::Confirmed, 12));
        assert_eq!(sev, Severity::Critical);
        assert!(reason.contains("+7.0σ"));
        assert!(reason.contains("2 source(s)"));
    }

    #[test]
    fn a_large_deviation_from_one_source_is_not_critical() {
        // One sensor shouting is not an emergency; the board must not invent
        // corroboration it does not have.
        let (sev, _) = severity_of(&signal_with(9.0, 1, SignalStatus::Developing, 5));
        assert_eq!(sev, Severity::Medium);
    }

    #[test]
    fn a_receding_signal_is_never_critical() {
        let (sev, reason) = severity_of(&signal_with(12.0, 4, SignalStatus::Fading, 60));
        assert_eq!(sev, Severity::Low);
        assert!(reason.contains("receding"));
    }

    #[test]
    fn no_sigma_is_reported_as_such_not_as_zero() {
        let (_, reason) = severity_of(&signal_with(0.0, 1, SignalStatus::New, 1));
        assert!(reason.contains("no sigma"));
    }
}
