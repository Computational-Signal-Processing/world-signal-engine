//! Live runtime state: what the engine is doing right now.
//!
//! The [`Engine`](crate::Engine) is the pipeline — a pure, synchronous
//! observation-to-signal machine. The runtime is the operational shell around
//! it: is collection on, which sources are enabled, what is scheduled next,
//! what just happened. Everything the System/Control screen shows and
//! everything a live UI needs to react to is read from here, so the API, the
//! scheduler and the UI share one source of truth instead of each keeping a
//! private copy that drifts.
//!
//! It is deliberately *not* part of the engine's detector state: a control flag
//! or a recent-activity ring is not something replay should reproduce, and
//! mixing the two would make the deterministic pipeline non-deterministic.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::sync::broadcast;

/// How many activity entries to keep for the live stream. Bounded on purpose:
/// a long-running process must not grow a log it never trims.
pub const ACTIVITY_LIMIT: usize = 500;

/// Broadcast buffer depth. A UI that falls further behind than this simply
/// re-syncs on reconnect rather than replaying a stale backlog.
const ACTIVITY_BROADCAST_CAPACITY: usize = 256;

/// What kind of thing happened. Drives the icon and the filter in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ActivityKind {
    Started,
    Observation,
    Anomaly,
    Event,
    Signal,
    SourceRecovered,
    SourceFailed,
    SourceRateLimited,
    Control,
}

/// One line in the live activity stream.
///
/// Every field is derived from a real engine outcome; nothing here is
/// synthesized for display. A UI that shows this is showing what happened.
#[derive(Debug, Clone, Serialize)]
pub struct Activity {
    pub at: DateTime<Utc>,
    pub kind: ActivityKind,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
}

impl Activity {
    pub fn new(kind: ActivityKind, message: impl Into<String>) -> Self {
        Self {
            at: Utc::now(),
            kind,
            message: message.into(),
            source_id: None,
            signal_id: None,
            event_id: None,
        }
    }

    pub fn for_source(mut self, source_id: impl Into<String>) -> Self {
        self.source_id = Some(source_id.into());
        self
    }

    pub fn for_signal(mut self, signal_id: impl Into<String>) -> Self {
        self.signal_id = Some(signal_id.into());
        self
    }

    pub fn for_event(mut self, event_id: impl Into<String>) -> Self {
        self.event_id = Some(event_id.into());
        self
    }
}

/// Where a source sits in the schedule, as the UI needs it.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ScheduleView {
    pub cadence_seconds: Option<u64>,
    pub last_run: Option<DateTime<Utc>>,
    pub last_success: Option<DateTime<Utc>>,
    pub next_run: Option<DateTime<Utc>>,
    pub consecutive_failures: u32,
}

/// One row of the Control screen's source table.
#[derive(Debug, Clone, Serialize)]
pub struct SourceControl {
    pub source_id: String,
    pub name: String,
    pub category: String,
    pub enabled: bool,
    pub running: bool,
    #[serde(flatten)]
    pub schedule: ScheduleView,
}

/// The full operational state, as one serializable object.
///
/// Built by the API from the store plus the runtime, so a single GET gives the
/// Control screen everything it shows — no field on that screen is hard-coded.
#[derive(Debug, Clone, Serialize)]
pub struct ControlSnapshot {
    pub status: &'static str,
    pub version: &'static str,
    pub started_at: DateTime<Utc>,
    pub uptime_seconds: u64,
    /// Whether a collection loop is running at all.
    pub collector_active: bool,
    pub collection_enabled: bool,
    /// `collector_active && collection_enabled` — what the "WORLD WATCH" badge
    /// reads. Never true for a static dashboard.
    pub monitoring: bool,
    pub sources: Vec<SourceControl>,
    pub signals_active: usize,
    pub signals_total: usize,
    pub observations_total: usize,
    pub events_total: usize,
    pub disk: DiskSummary,
    pub latency: LatencySummary,
}

/// Real latency telemetry, so "real-time" can be checked rather than asserted.
///
/// Every figure is computed from stored timestamps, never estimated. UI
/// delivery latency is deliberately absent: only the browser can measure when a
/// frame actually arrived, so the client computes it from the event's own
/// timestamp instead of the server guessing.
#[derive(Debug, Clone, Default, Serialize)]
pub struct LatencySummary {
    /// Source-side lag: `received_at - observed_at`, averaged over recent
    /// observations. Large for a batch feed (a day's earthquakes arrive at
    /// once), small for a live one. This is data freshness, not a defect.
    pub observation_lag_ms: Option<i64>,
    /// Collector fetch time, from the engine's metrics.
    pub collector_ms: Option<u64>,
    /// Detection latency: `signal.first_seen - earliest evidence.observed_at`,
    /// averaged over recent signals. How long after the data existed the signal
    /// existed.
    pub detection_ms: Option<i64>,
    /// Age of the newest signal: `now - first_seen`. If the stream or the
    /// pipeline stalled, this grows; a small value means the engine is keeping
    /// up with the world.
    pub newest_signal_age_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiskSummary {
    pub database_bytes: u64,
    pub raw_bytes: u64,
    pub raw_files: u64,
    pub total_bytes: u64,
}

/// Operational state shared between the scheduler, the API and the UI.
///
/// Cheap to clone (it is one `Arc`), so every task and every request handle
/// points at the same state.
#[derive(Clone)]
pub struct RuntimeState {
    inner: Arc<Inner>,
}

struct Inner {
    started: Instant,
    started_at: DateTime<Utc>,
    /// Whether the continuous collection loop should run. When off, the
    /// scheduler keeps ticking but skips every source, so the API and UI stay
    /// up — collection is paused, the engine is not.
    collection_enabled: AtomicBool,
    /// Whether a collection loop was actually started. This is *not* the same
    /// as `collection_enabled`: a served engine with no loop has collection
    /// "enabled" and still observes nothing. The UI must be able to tell the
    /// two apart, or it would show a static dashboard as if it were watching
    /// the world.
    collector_active: AtomicBool,
    disabled_sources: Mutex<HashSet<String>>,
    /// Sources an admin asked to run now, awaiting the scheduler's next pass.
    run_now: Mutex<VecDeque<String>>,
    /// Sources whose collector is executing right now. Used to guarantee one
    /// in-flight run per source: a "run now" must never race the scheduled run.
    running: Mutex<HashSet<String>>,
    schedules: Mutex<HashMap<String, ScheduleView>>,
    activity: Mutex<VecDeque<Activity>>,
    events: broadcast::Sender<Activity>,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeState {
    pub fn new() -> Self {
        let (events, _) = broadcast::channel(ACTIVITY_BROADCAST_CAPACITY);
        Self {
            inner: Arc::new(Inner {
                started: Instant::now(),
                started_at: Utc::now(),
                collection_enabled: AtomicBool::new(true),
                collector_active: AtomicBool::new(false),
                disabled_sources: Mutex::new(HashSet::new()),
                run_now: Mutex::new(VecDeque::new()),
                running: Mutex::new(HashSet::new()),
                schedules: Mutex::new(HashMap::new()),
                activity: Mutex::new(VecDeque::new()),
                events,
            }),
        }
    }

    /// Subscribe to the activity stream. The receiver is what the SSE endpoint
    /// forwards to the browser.
    pub fn subscribe(&self) -> broadcast::Receiver<Activity> {
        self.inner.events.subscribe()
    }

    pub fn started_at(&self) -> DateTime<Utc> {
        self.inner.started_at
    }

    pub fn uptime_seconds(&self) -> u64 {
        self.inner.started.elapsed().as_secs()
    }

    // -- collection control -------------------------------------------------

    pub fn collection_enabled(&self) -> bool {
        self.inner.collection_enabled.load(Ordering::Relaxed)
    }

    /// Whether a collection loop is actually running.
    ///
    /// Set once by the process that starts the loop. A served engine that never
    /// started one reports `false` here even though `collection_enabled()` is
    /// true — the distinction the UI needs to avoid showing a static dashboard
    /// as a live world watch.
    pub fn collector_active(&self) -> bool {
        self.inner.collector_active.load(Ordering::Relaxed)
    }

    pub fn set_collector_active(&self, active: bool) {
        self.inner.collector_active.store(active, Ordering::Relaxed);
    }

    /// The one honest answer to "is the world being watched right now?".
    ///
    /// Monitoring is active only when a loop is running *and* it is not paused.
    pub fn monitoring(&self) -> bool {
        self.collector_active() && self.collection_enabled()
    }

    /// Turn continuous collection on or off, recording the change as activity
    /// so the stream reflects who did what.
    pub fn set_collection_enabled(&self, enabled: bool) {
        self.inner
            .collection_enabled
            .store(enabled, Ordering::Relaxed);
        self.record(Activity::new(
            ActivityKind::Control,
            if enabled {
                "continuous collection enabled"
            } else {
                "continuous collection paused"
            },
        ));
    }

    // -- per-source control -------------------------------------------------

    pub fn is_source_enabled(&self, source_id: &str) -> bool {
        !self
            .inner
            .disabled_sources
            .lock()
            .expect("runtime lock poisoned")
            .contains(source_id)
    }

    /// Enable or disable a source. Disabled sources are skipped by the
    /// scheduler; the catalog entry itself is untouched, so this is reversible
    /// and cannot corrupt the source's metadata.
    pub fn set_source_enabled(&self, source_id: &str, enabled: bool) {
        {
            let mut disabled = self
                .inner
                .disabled_sources
                .lock()
                .expect("runtime lock poisoned");
            if enabled {
                disabled.remove(source_id);
            } else {
                disabled.insert(source_id.to_string());
            }
        }
        self.record(
            Activity::new(
                ActivityKind::Control,
                format!(
                    "source {source_id} {}",
                    if enabled { "enabled" } else { "disabled" }
                ),
            )
            .for_source(source_id),
        );
    }

    /// Ask for a source to run at the next scheduler pass.
    ///
    /// Coalesced: asking twice before the pass runs still queues one run. The
    /// scheduler then checks [`is_running`](Self::is_running) before starting,
    /// so a manual run can never overlap a scheduled one on the same source.
    pub fn request_run_now(&self, source_id: &str) {
        {
            let mut queue = self.inner.run_now.lock().expect("runtime lock poisoned");
            if !queue.iter().any(|id| id == source_id) {
                queue.push_back(source_id.to_string());
            }
        }
        self.record(
            Activity::new(
                ActivityKind::Control,
                format!("manual run requested for {source_id}"),
            )
            .for_source(source_id),
        );
    }

    /// Take everything queued by `request_run_now`, emptying the queue.
    pub fn take_run_now(&self) -> Vec<String> {
        let mut queue = self.inner.run_now.lock().expect("runtime lock poisoned");
        queue.drain(..).collect()
    }

    pub fn is_running(&self, source_id: &str) -> bool {
        self.inner
            .running
            .lock()
            .expect("runtime lock poisoned")
            .contains(source_id)
    }

    /// Mark a source as in flight. Returns `false` if it already was, so the
    /// caller can skip rather than start a second concurrent run.
    pub fn try_begin_run(&self, source_id: &str) -> bool {
        self.inner
            .running
            .lock()
            .expect("runtime lock poisoned")
            .insert(source_id.to_string())
    }

    pub fn end_run(&self, source_id: &str) {
        self.inner
            .running
            .lock()
            .expect("runtime lock poisoned")
            .remove(source_id);
    }

    // -- schedule bookkeeping ----------------------------------------------

    pub fn set_schedule(&self, source_id: &str, view: ScheduleView) {
        self.inner
            .schedules
            .lock()
            .expect("runtime lock poisoned")
            .insert(source_id.to_string(), view);
    }

    pub fn schedule(&self, source_id: &str) -> Option<ScheduleView> {
        self.inner
            .schedules
            .lock()
            .expect("runtime lock poisoned")
            .get(source_id)
            .cloned()
    }

    pub fn schedules(&self) -> HashMap<String, ScheduleView> {
        self.inner
            .schedules
            .lock()
            .expect("runtime lock poisoned")
            .clone()
    }

    // -- activity -----------------------------------------------------------

    /// Record an activity line and broadcast it to live subscribers.
    ///
    /// Recording never blocks on a slow subscriber: the broadcast channel drops
    /// the oldest item for a lagging receiver, and the UI re-syncs on reconnect.
    pub fn record(&self, activity: Activity) {
        {
            let mut log = self.inner.activity.lock().expect("runtime lock poisoned");
            if log.len() == ACTIVITY_LIMIT {
                log.pop_front();
            }
            log.push_back(activity.clone());
        }
        // A send error just means nobody is listening yet.
        let _ = self.inner.events.send(activity);
    }

    /// The most recent activity, newest first.
    pub fn recent_activity(&self, limit: usize) -> Vec<Activity> {
        let log = self.inner.activity.lock().expect("runtime lock poisoned");
        log.iter().rev().take(limit).cloned().collect()
    }

    pub fn activity_count(&self) -> usize {
        self.inner
            .activity
            .lock()
            .expect("runtime lock poisoned")
            .len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_is_newest_first_and_bounded() {
        let runtime = RuntimeState::new();
        for i in 0..(ACTIVITY_LIMIT + 25) {
            runtime.record(Activity::new(ActivityKind::Observation, format!("n{i}")));
        }
        let recent = runtime.recent_activity(10);
        assert_eq!(recent.len(), 10);
        assert_eq!(recent[0].message, format!("n{}", ACTIVITY_LIMIT + 24));
        assert_eq!(runtime.activity_count(), ACTIVITY_LIMIT);
    }

    #[test]
    fn run_now_is_coalesced() {
        let runtime = RuntimeState::new();
        runtime.request_run_now("src_a");
        runtime.request_run_now("src_a");
        runtime.request_run_now("src_b");
        let taken = runtime.take_run_now();
        assert_eq!(taken, vec!["src_a".to_string(), "src_b".to_string()]);
        // Taking empties the queue, so a second pass has nothing to do.
        assert!(runtime.take_run_now().is_empty());
    }

    #[test]
    fn a_source_cannot_run_twice_at_once() {
        let runtime = RuntimeState::new();
        assert!(runtime.try_begin_run("src_a"));
        assert!(
            !runtime.try_begin_run("src_a"),
            "second start must be refused"
        );
        runtime.end_run("src_a");
        assert!(runtime.try_begin_run("src_a"));
    }

    #[test]
    fn disabling_is_reversible_and_defaults_to_enabled() {
        let runtime = RuntimeState::new();
        assert!(runtime.is_source_enabled("src_a"));
        runtime.set_source_enabled("src_a", false);
        assert!(!runtime.is_source_enabled("src_a"));
        runtime.set_source_enabled("src_a", true);
        assert!(runtime.is_source_enabled("src_a"));
    }

    #[tokio::test]
    async fn activity_is_broadcast_to_subscribers() {
        let runtime = RuntimeState::new();
        let mut rx = runtime.subscribe();
        runtime.record(Activity::new(ActivityKind::Signal, "signal detected").for_signal("sig_1"));
        let received = rx
            .recv()
            .await
            .expect("subscriber should receive the event");
        assert_eq!(received.kind, ActivityKind::Signal);
        assert_eq!(received.signal_id.as_deref(), Some("sig_1"));
    }
}
