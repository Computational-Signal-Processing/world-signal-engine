//! The synthetic world: a deterministic stand-in for real feeds.
//!
//! The brief is explicit that detection must not be declared "working" until it
//! passes a synthetic test. This module generates exactly the scenarios that
//! matter:
//!
//! ```text
//! 100 normal observations          -> nothing
//! 101..120 gradual upward drift    -> EARLY_SIGNAL
//! 201..205 large spike             -> ANOMALY
//! multiple aligned streams         -> CONVERGENCE
//! ```
//!
//! Every stream is reproducible from its seed.

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use wse_model::{EntityId, Observation, RawReference, SourceId};

use crate::rng::Rng;
use crate::{CollectionMode, CollectionResult, Collector, CollectorError, RawPayload, Schedule};

/// A deterministic generator for one synthetic series.
#[derive(Debug, Clone)]
pub struct SyntheticStream {
    pub series_name: String,
    pub source_id: SourceId,
    pub entity_id: Option<EntityId>,
    pub metric: String,
    pub unit: String,
    /// Value the series oscillates around before any event.
    pub baseline: f64,
    /// Standard deviation of the normal noise.
    pub noise: f64,
    /// Seconds between consecutive observations.
    pub step_seconds: i64,
    pub seed: u64,
    pub location: Option<(f64, f64)>,
}

impl SyntheticStream {
    pub fn new(series_name: impl Into<String>, baseline: f64, noise: f64) -> Self {
        let series_name = series_name.into();
        Self {
            source_id: SourceId::new(format!("synthetic_{series_name}")),
            entity_id: Some(EntityId::new(format!("ent_{series_name}"))),
            metric: series_name.clone(),
            unit: "unit".to_string(),
            baseline,
            noise,
            step_seconds: 3600,
            seed: 1,
            location: None,
            series_name,
        }
    }

    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    pub fn with_step_seconds(mut self, seconds: i64) -> Self {
        self.step_seconds = seconds;
        self
    }

    pub fn with_entity(mut self, entity_id: EntityId) -> Self {
        self.entity_id = Some(entity_id);
        self
    }

    pub fn with_location(mut self, lat: f64, lon: f64) -> Self {
        self.location = Some((lat, lon));
        self
    }

    /// Generate the observation at index `i`, relative to `origin`.
    ///
    /// The shape of the series is driven entirely by `i`, which is what makes
    /// replay and synthetic tests deterministic.
    pub fn observation_at(&self, i: usize, origin: DateTime<Utc>) -> Observation {
        let mut rng = Rng::new(self.seed.wrapping_add(i as u64).wrapping_mul(2_654_435_761));
        let noise = rng.normal() * self.noise;
        let value = self.baseline + noise + self.event_offset(i);
        let at = origin + Duration::seconds(i as i64 * self.step_seconds);

        let raw = raw_reference(&self.series_name, i);
        let mut obs = Observation::new(
            self.source_id.clone(),
            self.entity_id.clone(),
            self.metric.clone(),
            value,
            self.unit.clone(),
            at,
            raw,
        )
        .with_received_at(at);
        if let Some((lat, lon)) = self.location {
            obs = obs.with_location(lat, lon);
        }
        obs
    }

    /// The deterministic, non-random part of the series: the "story".
    ///
    /// Override this to script a scenario. The default is a flat baseline.
    pub fn event_offset(&self, _i: usize) -> f64 {
        0.0
    }
}

/// The canonical raw body for a synthetic locator.
///
/// Kept as a pure function of the locator so the observation's reference hash
/// and the payload the collector stores can never disagree: both are computed
/// from this one string. If they could drift, the drill-down's final step
/// would dead-end on a hash that no payload matches.
fn raw_body_for(locator: &str) -> String {
    format!("{{\"locator\":\"{locator}\"}}")
}

/// Build the raw reference for a synthetic point at index `i`.
fn raw_reference(series_name: &str, i: usize) -> RawReference {
    let locator = format!("synthetic://{series_name}/{i}");
    let hash = wse_model::fnv1a_hex(&raw_body_for(&locator));
    RawReference::new(locator, hash)
}

/// One scripted phase of a synthetic series.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Phase {
    /// Inclusive start index.
    pub from: usize,
    /// Exclusive end index.
    pub to: usize,
    /// Value added at the start of the phase.
    pub offset_from: f64,
    /// Value added at the end of the phase (linearly interpolated between).
    pub offset_to: f64,
}

impl Phase {
    pub fn drift(from: usize, to: usize, from_offset: f64, to_offset: f64) -> Self {
        Self {
            from,
            to,
            offset_from: from_offset,
            offset_to: to_offset,
        }
    }

    /// A flat offset across the whole phase.
    pub fn level(from: usize, to: usize, offset: f64) -> Self {
        Self {
            from,
            to,
            offset_from: offset,
            offset_to: offset,
        }
    }

    /// A single-point spike.
    pub fn spike(at: usize, magnitude: f64) -> Self {
        Self {
            from: at,
            to: at + 1,
            offset_from: magnitude,
            offset_to: magnitude,
        }
    }

    pub fn offset_at(&self, i: usize) -> f64 {
        if i < self.from || i >= self.to {
            return 0.0;
        }
        let span = (self.to - self.from).max(1) as f64;
        let progress = (i - self.from) as f64 / span;
        self.offset_from + (self.offset_to - self.offset_from) * progress
    }
}

/// A synthetic stream whose shape is described by a list of [`Phase`]s.
#[derive(Debug, Clone)]
pub struct ScriptedStream {
    pub stream: SyntheticStream,
    pub phases: Vec<Phase>,
}

impl ScriptedStream {
    pub fn new(stream: SyntheticStream, phases: Vec<Phase>) -> Self {
        Self { stream, phases }
    }

    pub fn observation_at(&self, i: usize, origin: DateTime<Utc>) -> Observation {
        let mut rng = Rng::new(
            self.stream
                .seed
                .wrapping_add(i as u64)
                .wrapping_mul(2_654_435_761),
        );
        let noise = rng.normal() * self.stream.noise;
        let offset: f64 = self.phases.iter().map(|p| p.offset_at(i)).sum();
        let value = self.stream.baseline + noise + offset;
        let at = origin + Duration::seconds(i as i64 * self.stream.step_seconds);
        let raw = raw_reference(&self.stream.series_name, i);
        let mut obs = Observation::new(
            self.stream.source_id.clone(),
            self.stream.entity_id.clone(),
            self.stream.metric.clone(),
            value,
            self.stream.unit.clone(),
            at,
            raw,
        )
        .with_received_at(at);
        if let Some((lat, lon)) = self.stream.location {
            obs = obs.with_location(lat, lon);
        }
        obs
    }
}

/// A world made of one or more scripted streams.
#[derive(Debug, Clone)]
pub struct SyntheticWorld {
    pub streams: Vec<ScriptedStream>,
    pub origin: DateTime<Utc>,
    /// Index the world will produce next; supports sequential replay.
    pub cursor: usize,
}

impl SyntheticWorld {
    pub fn new(origin: DateTime<Utc>) -> Self {
        Self {
            streams: Vec::new(),
            origin,
            cursor: 0,
        }
    }

    pub fn with_stream(mut self, stream: ScriptedStream) -> Self {
        self.streams.push(stream);
        self
    }

    /// The canonical acceptance-test world from the brief.
    ///
    /// * indices `0..100`: pure normal noise (nothing to report)
    /// * indices `100..120`: gradual upward drift (`EARLY_SIGNAL`)
    /// * indices `120..200`: back to normal
    /// * indices `200..205`: a large spike (`ANOMALY`)
    pub fn acceptance(origin: DateTime<Utc>) -> Self {
        let stream = SyntheticStream::new("sensor", 100.0, 1.0)
            .with_seed(7)
            .with_step_seconds(3600)
            .with_location(41.0, 29.0);
        let phases = vec![
            Phase::drift(100, 120, 0.0, 4.0),
            Phase::level(120, 200, 4.0),
            Phase::spike(200, 60.0),
        ];
        Self::new(origin).with_stream(ScriptedStream::new(stream, phases))
    }

    /// A world where three independent streams drift together, for
    /// convergence testing.
    pub fn convergence(origin: DateTime<Utc>) -> Self {
        let names = ["shipping_delay", "port_activity", "oil_price"];
        let mut world = Self::new(origin);
        for (idx, name) in names.iter().enumerate() {
            let stream = SyntheticStream::new(*name, 100.0, 1.0)
                .with_seed(100 + idx as u64)
                .with_step_seconds(3600)
                .with_entity(EntityId::new("ent_route_hormuz"))
                .with_location(26.5, 56.5);
            // All three drift upward in the same window and direction.
            let phases = vec![Phase::drift(100, 130, 0.0, 5.0)];
            world = world.with_stream(ScriptedStream::new(stream, phases));
        }
        world
    }

    /// Total number of observations the world can produce.
    pub fn length(&self) -> usize {
        self.streams
            .iter()
            .map(|s| s.phases.iter().map(|p| p.to).max().unwrap_or(0))
            .max()
            .unwrap_or(0)
    }

    /// Number of observations the world can produce up to index `count`.
    pub fn generate(&self, count: usize) -> Vec<Observation> {
        let mut out = Vec::with_capacity(count * self.streams.len());
        for i in 0..count {
            for stream in &self.streams {
                out.push(stream.observation_at(i, self.origin));
            }
        }
        out.sort_by_key(|o| o.observed_at);
        out
    }

    /// Generate one observation per stream for the current cursor, advancing
    /// it. This is how replay feeds the pipeline "as if live".
    pub fn tick(&mut self) -> Vec<Observation> {
        let i = self.cursor;
        self.cursor += 1;
        self.streams
            .iter()
            .map(|s| s.observation_at(i, self.origin))
            .collect()
    }
}

/// A [`Collector`] over a [`SyntheticWorld`].
///
/// Holds the world behind a mutex so it can be used through the shared
/// collector interface while still advancing its cursor.
#[derive(Debug)]
pub struct SyntheticCollector {
    world: std::sync::Mutex<SyntheticWorld>,
    source_id: SourceId,
    /// How many observations to emit per collection run.
    batch: usize,
    /// Restart the world when the cursor reaches the end.
    restart: bool,
}

impl SyntheticCollector {
    pub fn new(world: SyntheticWorld) -> Self {
        let source_id = world
            .streams
            .first()
            .map(|s| s.stream.source_id.clone())
            .unwrap_or_else(|| SourceId::new("synthetic"));
        Self {
            world: std::sync::Mutex::new(world),
            source_id,
            batch: 1,
            restart: false,
        }
    }

    /// Emit `batch` observations (one per stream) per run.
    pub fn with_batch(mut self, batch: usize) -> Self {
        self.batch = batch.max(1);
        self
    }

    /// Reproduce the world from the beginning on every collection run.
    ///
    /// This is the live-observation mode: the detector is warm and the same
    /// conditions recur on a fixed cadence, so drift and spikes are surfaced
    /// as they happen. Useful for demos; a real collector never rewinds.
    pub fn with_restart(mut self, restart: bool) -> Self {
        self.restart = restart;
        self
    }

    /// The index at which the underlying world's script ends.
    pub fn length(&self) -> usize {
        self.world.lock().expect("world lock").length()
    }

    /// Current cursor, for assertions in tests.
    pub fn cursor(&self) -> usize {
        self.world.lock().expect("world lock").cursor
    }
}

#[async_trait]
impl Collector for SyntheticCollector {
    fn source_id(&self) -> SourceId {
        self.source_id.clone()
    }

    fn schedule(&self) -> Schedule {
        Schedule::Manual
    }

    fn mode(&self) -> CollectionMode {
        CollectionMode::Replay
    }

    async fn collect(&self) -> Result<CollectionResult, CollectorError> {
        let mut world = self
            .world
            .lock()
            .map_err(|e| CollectorError::Other(format!("world lock poisoned: {e}")))?;
        let started_at = Utc::now();
        let mut observations = Vec::new();
        for _ in 0..self.batch {
            if self.restart && world.cursor >= world.length() {
                world.cursor = 0;
            }
            observations.extend(world.tick());
        }
        let raw_payloads = observations
            .iter()
            .map(|o| RawPayload {
                reference: o.raw.clone(),
                body: raw_body_for(&o.raw.locator).into_bytes(),
            })
            .collect::<Vec<_>>();
        let received = observations.len() as u64;
        let mut result = CollectionResult::new(self.source_id.clone());
        result.observations = observations;
        result.raw_payloads = raw_payloads;
        result.records_received = received;
        result.records_changed = received;
        result.started_at = Some(started_at);
        result.finished_at = Some(Utc::now());
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).unwrap()
    }

    #[test]
    fn phases_interpolate_linearly() {
        let p = Phase::drift(100, 120, 0.0, 4.0);
        assert_eq!(p.offset_at(99), 0.0);
        assert_eq!(p.offset_at(100), 0.0);
        assert!((p.offset_at(110) - 2.0).abs() < 1e-9);
        assert!((p.offset_at(119) - 3.8).abs() < 1e-9);
        assert_eq!(p.offset_at(120), 0.0);
    }

    #[test]
    fn spike_phase_covers_exactly_one_index() {
        let p = Phase::spike(200, 60.0);
        assert_eq!(p.offset_at(199), 0.0);
        assert_eq!(p.offset_at(200), 60.0);
        assert_eq!(p.offset_at(201), 0.0);
    }

    #[test]
    fn generation_is_deterministic() {
        let w = SyntheticWorld::acceptance(origin());
        let a = w.generate(10);
        let b = w.generate(10);
        assert_eq!(a.len(), 10);
        assert_eq!(a, b);
    }

    #[test]
    fn acceptance_world_has_a_drift_then_a_spike() {
        let w = SyntheticWorld::acceptance(origin());
        let obs = w.generate(205);
        // During the drift window values climb above the baseline.
        let drift_start = &obs[100];
        let drift_end = &obs[119];
        assert!(drift_end.value > drift_start.value);
        // The spike is enormous.
        assert!(obs[200].value > obs[150].value + 50.0);
    }

    #[test]
    fn tick_advances_one_step_per_stream() {
        let mut w = SyntheticWorld::convergence(origin());
        let first = w.tick();
        assert_eq!(first.len(), 3);
        let second = w.tick();
        assert_eq!(second.len(), 3);
        assert!(second[0].observed_at > first[0].observed_at);
    }

    #[tokio::test]
    async fn synthetic_collector_produces_observations() {
        let collector = SyntheticCollector::new(SyntheticWorld::acceptance(origin()));
        let result = collector.collect().await.unwrap();
        assert_eq!(result.records_received, 1);
        assert_eq!(result.records_changed, 1);
        assert_eq!(result.observations.len(), 1);
        assert!(!result.is_failure());
        assert_eq!(collector.cursor(), 1);
    }

    #[tokio::test]
    async fn synthetic_collector_batches() {
        let collector =
            SyntheticCollector::new(SyntheticWorld::convergence(origin())).with_batch(5);
        let result = collector.collect().await.unwrap();
        assert_eq!(result.observations.len(), 15); // 3 streams * 5
    }

    #[tokio::test]
    async fn restarting_collector_rewinds_instead_of_exhausting() {
        let world = SyntheticWorld::acceptance(origin());
        let length = world.length();
        assert_eq!(length, 201, "acceptance script ends at the spike");

        let collector = SyntheticCollector::new(world).with_restart(true);
        // Run well past the end of the script.
        for _ in 0..(length + 10) {
            let result = collector.collect().await.unwrap();
            assert_eq!(result.observations.len(), 1, "a rewound world still emits");
        }
        assert!(
            collector.cursor() < length,
            "cursor should have wrapped, was {}",
            collector.cursor()
        );
    }

    #[tokio::test]
    async fn non_restarting_collector_stops_at_the_end_of_the_script() {
        let collector = SyntheticCollector::new(SyntheticWorld::acceptance(origin()));
        for _ in 0..collector.length() {
            collector.collect().await.unwrap();
        }
        assert_eq!(collector.cursor(), collector.length());
    }
}
