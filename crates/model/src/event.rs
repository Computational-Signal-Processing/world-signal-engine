//! Events: several anomalies that describe one thing happening in the world.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::anomaly::CandidateDirection;
use crate::geo::Location;
use crate::ids::{fnv1a_hex, AnomalyId, EntityId, EventId, ObservationId};

/// An event groups anomaly candidates that share entity, time window,
/// direction and category. It is the unit that the signal engine reasons over.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub id: EventId,
    pub title: String,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    /// The grouping key this event was formed under (`entity:…` or `series:…`).
    ///
    /// Retained rather than re-derived from `entities`/`anomalies`, because
    /// those vectors are ordered by arrival and can be empty in a partial
    /// record. Re-deriving the key made grouping depend on which field happened
    /// to be populated, which silently split one ongoing event into several.
    pub group_key: String,
    pub entities: Vec<EntityId>,
    pub observations: Vec<ObservationId>,
    pub anomalies: Vec<AnomalyId>,
    pub categories: Vec<String>,
    pub location: Option<Location>,
    pub direction: CandidateDirection,
    pub state: EventState,
    /// Number of distinct sources contributing to this event.
    pub source_count: usize,
}

impl Event {
    /// A fresh event under `group_key`.
    ///
    /// The id is derived from the group key and the time the event started, so
    /// re-forming the same event reproduces the same id. A random id here would
    /// defeat [`crate::Signal::stable_id`], which is built on top of it: every
    /// replay would mint new signals for a change that never changed.
    pub fn new_for(
        group_key: impl Into<String>,
        title: impl Into<String>,
        at: DateTime<Utc>,
    ) -> Self {
        let group_key = group_key.into();
        let id = EventId::new(format!(
            "evt_{}",
            fnv1a_hex(&format!("{}|{}", group_key, at.to_rfc3339()))
        ));
        Self {
            id,
            title: title.into(),
            first_seen: at,
            last_seen: at,
            group_key,
            entities: Vec::new(),
            observations: Vec::new(),
            anomalies: Vec::new(),
            categories: Vec::new(),
            location: None,
            direction: CandidateDirection::Flat,
            state: EventState::Detected,
            source_count: 0,
        }
    }

    /// A fresh event with a random id.
    ///
    /// Only for records that are not expected to recur, such as unit tests that
    /// need distinct events. The pipeline uses [`Event::new_for`].
    pub fn new(title: impl Into<String>, at: DateTime<Utc>) -> Self {
        Self {
            id: EventId::generate(),
            title: title.into(),
            first_seen: at,
            last_seen: at,
            group_key: String::new(),
            entities: Vec::new(),
            observations: Vec::new(),
            anomalies: Vec::new(),
            categories: Vec::new(),
            location: None,
            direction: CandidateDirection::Flat,
            state: EventState::Detected,
            source_count: 0,
        }
    }

    /// Duration in seconds between first and last observation of the event.
    pub fn duration_seconds(&self) -> i64 {
        (self.last_seen - self.first_seen).num_seconds().max(0)
    }

    pub fn observation_count(&self) -> usize {
        self.observations.len()
    }

    /// Advance the event's clock and state.
    ///
    /// Lifecycle is intentionally simple and deterministic: a new event is
    /// `Detected`; while it keeps accumulating it is `Active`; once it stops
    /// changing it becomes `Stabilized`, and once nothing has arrived for
    /// `resolve_after` it becomes `Resolved`.
    pub fn observe(&mut self, at: DateTime<Utc>) {
        if at > self.last_seen {
            self.last_seen = at;
        }
    }

    pub fn stabilize(&mut self) {
        if self.state != EventState::Resolved {
            self.state = EventState::Stabilized;
        }
    }

    pub fn resolve(&mut self) {
        self.state = EventState::Resolved;
    }

    /// Recompute state from how recently the event was updated.
    pub fn refresh_state(&mut self, now: DateTime<Utc>, resolve_after_seconds: i64) {
        if self.state == EventState::Resolved {
            return;
        }
        let idle = (now - self.last_seen).num_seconds();
        if idle >= resolve_after_seconds {
            self.state = EventState::Resolved;
        } else if idle > 0 {
            self.state = EventState::Stabilized;
        } else if self.observation_count() > 1 {
            self.state = EventState::Changing;
        } else {
            self.state = EventState::Active;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EventState {
    Detected,
    Active,
    Changing,
    Stabilized,
    Resolved,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
    }

    #[test]
    fn duration_tracks_first_and_last_seen() {
        let mut e = Event::new("test", ts(0));
        e.observe(ts(300));
        assert_eq!(e.duration_seconds(), 300);
    }

    #[test]
    fn observe_never_moves_last_seen_backwards() {
        let mut e = Event::new("test", ts(100));
        e.observe(ts(50));
        assert_eq!(e.last_seen, ts(100));
    }

    #[test]
    fn lifecycle_resolves_after_idle_window() {
        let mut e = Event::new("test", ts(0));
        e.observations.push(ObservationId::new("obs_1"));
        // A fresh event with no idle time is Active.
        e.refresh_state(ts(0), 60);
        assert_eq!(e.state, EventState::Active);
        // Idle, but inside the resolve window: Stabilized, not Resolved.
        e.refresh_state(ts(10), 60);
        assert_eq!(e.state, EventState::Stabilized);
        // Past the resolve window: Resolved.
        e.refresh_state(ts(120), 60);
        assert_eq!(e.state, EventState::Resolved);
    }

    #[test]
    fn resolved_is_terminal() {
        let mut e = Event::new("test", ts(0));
        e.resolve();
        e.refresh_state(ts(0), 60);
        assert_eq!(e.state, EventState::Resolved);
    }
}
