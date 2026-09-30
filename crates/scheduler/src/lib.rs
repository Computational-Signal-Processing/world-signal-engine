//! # wse-scheduler
//!
//! Two responsibilities, both small:
//!
//! * **Clock** — the source of "now". [`LiveClock`] reads the wall clock;
//!   [`ReplayClock`] advances through historical time so the detection engine
//!   can be back-tested as if the data were arriving live. The brief calls
//!   replay a first-class mode, not a test afterthought.
//! * **ScheduleState** — tracks when each collector last ran so the engine can
//!   ask "which collectors are due?".

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use wse_collector::{CollectionMode, Schedule};
use wse_model::SourceId;

/// The source of "now".
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
    fn mode(&self) -> CollectionMode;
}

/// Wall-clock time.
#[derive(Debug, Clone, Copy, Default)]
pub struct LiveClock;

impl Clock for LiveClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }

    fn mode(&self) -> CollectionMode {
        CollectionMode::Live
    }
}

/// A clock that advances through history at a controlled pace.
///
/// `step` is how far the clock moves per [`ReplayClock::advance`] call, which
/// the driver makes once per collection cycle. This makes replay deterministic
/// and independent of real elapsed time.
#[derive(Debug, Clone)]
pub struct ReplayClock {
    current: DateTime<Utc>,
    step: Duration,
}

impl ReplayClock {
    pub fn new(start: DateTime<Utc>, step: Duration) -> Self {
        Self {
            current: start,
            step,
        }
    }

    /// Move the clock forward one step and return the new time.
    pub fn advance(&mut self) -> DateTime<Utc> {
        self.current += self.step;
        self.current
    }

    pub fn set(&mut self, at: DateTime<Utc>) {
        self.current = at;
    }
}

impl Clock for ReplayClock {
    fn now(&self) -> DateTime<Utc> {
        self.current
    }

    fn mode(&self) -> CollectionMode {
        CollectionMode::Replay
    }
}

/// A [`ReplayClock`] that can be shared with the engine while the replay driver
/// still advances it.
///
/// The engine holds an `Arc<dyn Clock>`, so a replay run needs a clock that is
/// readable through the shared interface and writable by whoever drives the
/// stream. Without this, signal formation would fall back to the wall clock and
/// a replayed run would not reproduce.
#[derive(Debug, Clone)]
pub struct SharedReplayClock {
    inner: std::sync::Arc<std::sync::Mutex<ReplayClock>>,
}

impl SharedReplayClock {
    pub fn new(start: DateTime<Utc>, step: Duration) -> Self {
        Self {
            inner: std::sync::Arc::new(std::sync::Mutex::new(ReplayClock::new(start, step))),
        }
    }

    /// Move to an explicit time, e.g. the timestamp of the observation about to
    /// be ingested.
    pub fn set(&self, at: DateTime<Utc>) {
        self.inner.lock().expect("replay clock lock").set(at);
    }

    /// Advance by the configured step and return the new time.
    pub fn advance(&self) -> DateTime<Utc> {
        self.inner.lock().expect("replay clock lock").advance()
    }

    /// A handle the engine can own.
    pub fn handle(&self) -> std::sync::Arc<dyn Clock> {
        std::sync::Arc::new(self.clone())
    }
}

impl Clock for SharedReplayClock {
    fn now(&self) -> DateTime<Utc> {
        self.inner.lock().expect("replay clock lock").now()
    }

    fn mode(&self) -> CollectionMode {
        CollectionMode::Replay
    }
}

/// Per-collector scheduling state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScheduleState {
    pub source_id: SourceId,
    pub schedule: Schedule,
    pub last_run: Option<DateTime<Utc>>,
    pub last_success: Option<DateTime<Utc>>,
    pub consecutive_failures: u32,
}

impl ScheduleState {
    pub fn new(source_id: SourceId, schedule: Schedule) -> Self {
        Self {
            source_id,
            schedule,
            last_run: None,
            last_success: None,
            consecutive_failures: 0,
        }
    }

    /// Whether this collector should run at `now`.
    pub fn is_due(&self, now: DateTime<Utc>) -> bool {
        match self.schedule.poll_seconds() {
            // Manual collectors only run when explicitly triggered.
            None => false,
            Some(interval) => match self.last_run {
                None => true,
                Some(last) => (now - last).num_seconds() >= interval as i64,
            },
        }
    }

    pub fn record_run(&mut self, at: DateTime<Utc>, success: bool) {
        self.last_run = Some(at);
        if success {
            self.last_success = Some(at);
            self.consecutive_failures = 0;
        } else {
            self.consecutive_failures += 1;
        }
    }
}

/// Tracks schedules for every registered collector.
#[derive(Debug, Default)]
pub struct Scheduler {
    states: HashMap<SourceId, ScheduleState>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, source_id: SourceId, schedule: Schedule) {
        self.states
            .insert(source_id.clone(), ScheduleState::new(source_id, schedule));
    }

    pub fn state(&self, source_id: &SourceId) -> Option<&ScheduleState> {
        self.states.get(source_id)
    }

    /// Source ids whose collectors are due at `now`, ordered by insertion for
    /// determinism.
    pub fn due(&self, now: DateTime<Utc>) -> Vec<SourceId> {
        let mut ids: Vec<SourceId> = self
            .states
            .values()
            .filter(|s| s.is_due(now))
            .map(|s| s.source_id.clone())
            .collect();
        ids.sort();
        ids
    }

    pub fn record_run(&mut self, source_id: &SourceId, at: DateTime<Utc>, success: bool) {
        if let Some(state) = self.states.get_mut(source_id) {
            state.record_run(at, success);
        }
    }

    pub fn len(&self) -> usize {
        self.states.len()
    }

    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
    }

    #[test]
    fn live_clock_reports_live_mode() {
        let c = LiveClock;
        assert_eq!(c.mode(), CollectionMode::Live);
        assert!(c.now() > at(0));
    }

    #[test]
    fn replay_clock_advances_deterministically() {
        let mut c = ReplayClock::new(at(0), Duration::hours(1));
        assert_eq!(c.now(), at(0));
        assert_eq!(c.advance(), at(3600));
        assert_eq!(c.advance(), at(7200));
        assert_eq!(c.mode(), CollectionMode::Replay);
    }

    #[test]
    fn manual_collectors_are_never_due() {
        let s = ScheduleState::new(SourceId::new("s"), Schedule::Manual);
        assert!(!s.is_due(at(0)));
    }

    #[test]
    fn interval_collectors_become_due_after_their_interval() {
        let mut s = ScheduleState::new(SourceId::new("s"), Schedule::Interval { seconds: 60 });
        assert!(s.is_due(at(0)));
        s.record_run(at(0), true);
        assert!(!s.is_due(at(30)));
        assert!(s.is_due(at(60)));
    }

    #[test]
    fn failures_are_tracked_and_reset_on_success() {
        let mut s = ScheduleState::new(SourceId::new("s"), Schedule::Interval { seconds: 60 });
        s.record_run(at(0), false);
        s.record_run(at(60), false);
        assert_eq!(s.consecutive_failures, 2);
        s.record_run(at(120), true);
        assert_eq!(s.consecutive_failures, 0);
        assert_eq!(s.last_success, Some(at(120)));
    }

    #[test]
    fn scheduler_reports_due_sources_in_order() {
        let mut sched = Scheduler::new();
        sched.register(SourceId::new("src_b"), Schedule::Interval { seconds: 10 });
        sched.register(SourceId::new("src_a"), Schedule::Interval { seconds: 10 });
        sched.register(SourceId::new("src_manual"), Schedule::Manual);
        let due = sched.due(at(0));
        assert_eq!(due, vec![SourceId::new("src_a"), SourceId::new("src_b")]);
        assert_eq!(sched.len(), 3);
    }
}
