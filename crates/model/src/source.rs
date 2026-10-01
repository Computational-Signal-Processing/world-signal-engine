//! Sources: the catalog of things we observe.
//!
//! A [`Source`] is metadata. The code that actually talks to a source is a
//! collector; a source entry plus a collector implementation is all that is
//! needed to add a new feed (see `SOURCE_CATALOG.md`).

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::SourceId;

/// How trustworthy a source is, as provenance metadata.
///
/// This is deliberately **not** a signal score. It says where a measurement
/// came from, so a signal corroborated by several independent institutional
/// measurements can be told apart from one produced by a single community
/// feed. It never multiplies into a signal's magnitude.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceTier {
    /// Official, primary or scientific institutional data (USGS, NASA, NWS,
    /// FRED, ECB, EIA, CISA, arXiv).
    Tier1,
    /// An established independent data provider (GDELT, Crossref, npm).
    Tier2,
    /// A community or platform signal (Hacker News, GitHub).
    Tier3,
    /// Exploratory or weak; kept for experiments, not for world claims.
    Tier4,
}

impl SourceTier {
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceTier::Tier1 => "tier_1",
            SourceTier::Tier2 => "tier_2",
            SourceTier::Tier3 => "tier_3",
            SourceTier::Tier4 => "tier_4",
        }
    }

    /// A short human label for the registry.
    pub fn label(&self) -> &'static str {
        match self {
            SourceTier::Tier1 => "Tier 1 — institutional",
            SourceTier::Tier2 => "Tier 2 — independent provider",
            SourceTier::Tier3 => "Tier 3 — community signal",
            SourceTier::Tier4 => "Tier 4 — exploratory",
        }
    }

    /// Whether this is institutional, primary data. Used by the UI to
    /// distinguish institutional corroboration from community noise.
    pub fn is_institutional(&self) -> bool {
        matches!(self, SourceTier::Tier1)
    }
}

/// Whether the quantity a source emits is a stable time series.
///
/// A series is only meaningful if the *population* being measured is fixed (or
/// changes in a way we can account for). A changing search-result set is not a
/// stable series: the top-50 repositories by "recently updated" churn
/// constantly, so a change in the aggregate reflects membership churn, not a
/// change in the world. This field makes that judgement explicit and
/// machine-readable, so a semantically invalid metric cannot masquerade as a
/// world measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementSemantics {
    /// The source reports an authoritative measurement of a fixed subject: a
    /// count of open events, a gauge reading, an index level. Comparable across
    /// collections.
    StableSeries,
    /// The source reports a fixed universe that is re-measured each poll: a
    /// declared set of repositories, a fixed set of packages. Membership is
    /// stable, so the aggregate is comparable.
    FixedUniverse,
    /// The source reports a set whose *membership changes* between collections
    /// (a top-N ranking, a search result). The aggregate is **not** a stable
    /// series and must not be treated as one.
    UnstablePopulation,
}

impl MeasurementSemantics {
    pub fn as_str(&self) -> &'static str {
        match self {
            MeasurementSemantics::StableSeries => "stable_series",
            MeasurementSemantics::FixedUniverse => "fixed_universe",
            MeasurementSemantics::UnstablePopulation => "unstable_population",
        }
    }

    /// Whether a change in this metric can be read as a real-world change.
    ///
    /// Only `false` for `UnstablePopulation`; that is the case the engine must
    /// never promote into a signal.
    pub fn is_comparable(&self) -> bool {
        !matches!(self, MeasurementSemantics::UnstablePopulation)
    }
}

/// Full source-catalog entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub id: SourceId,
    pub name: String,
    pub provider: String,
    pub category: String,
    pub subcategory: Option<String>,
    pub endpoint: String,
    pub protocol: Protocol,
    pub format: DataFormat,
    pub cadence: Cadence,
    pub timezone: Option<String>,
    pub license: Option<String>,
    pub authentication: AuthKind,
    pub cost: Cost,
    pub historical_available: bool,
    pub realtime_available: bool,
    pub geospatial: bool,
    /// Entity labels this source is expected to produce.
    pub entities: Vec<String>,
    /// Lower is more important; used to order collection and display.
    pub priority: u32,
    pub enabled: bool,
    /// Name of the collector implementation that knows how to read this source.
    pub collector_type: String,
    /// Free-form extras (poll interval hints, query parameters, ...).
    pub parameters: BTreeMap<String, String>,
    /// Provenance tier: institutional, independent, community or exploratory.
    ///
    /// Provenance metadata, not a score. Defaults to the weakest tier so a new
    /// source must declare its standing rather than inherit a flattering one.
    #[serde(default = "default_tier")]
    pub tier: SourceTier,
    /// Whether the emitted quantity is a stable, comparable time series.
    #[serde(default = "default_semantics")]
    pub measurement: MeasurementSemantics,
    /// The lenses this source is intended to feed.
    ///
    /// A source is a sensor; a lens is a view. One sensor can feed several
    /// lenses (USGS feeds EARTH, WORLD and TURKEY when geographically
    /// relevant). The mapping is declared here so the Lens screen can show
    /// which sensors actually back each lens — and say `NO CONNECTED SOURCES`
    /// rather than showing a lens as healthy when nothing feeds it.
    #[serde(default)]
    pub feeds_lenses: Vec<String>,
}

fn default_tier() -> SourceTier {
    SourceTier::Tier4
}

fn default_semantics() -> MeasurementSemantics {
    MeasurementSemantics::StableSeries
}

impl Source {
    /// Minimal constructor for tests and synthetic sources.
    pub fn new(id: SourceId, name: impl Into<String>, collector_type: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            provider: "unknown".to_string(),
            category: "uncategorized".to_string(),
            subcategory: None,
            endpoint: String::new(),
            protocol: Protocol::None,
            format: DataFormat::Json,
            cadence: Cadence::Event,
            timezone: Some("UTC".to_string()),
            license: None,
            authentication: AuthKind::None,
            cost: Cost::Free,
            historical_available: false,
            realtime_available: false,
            geospatial: false,
            entities: Vec::new(),
            priority: 100,
            enabled: true,
            collector_type: collector_type.into(),
            parameters: BTreeMap::new(),
            tier: SourceTier::Tier4,
            measurement: MeasurementSemantics::StableSeries,
            feeds_lenses: Vec::new(),
        }
    }

    /// Declare the provenance tier.
    pub fn with_tier(mut self, tier: SourceTier) -> Self {
        self.tier = tier;
        self
    }

    /// Declare whether the emitted quantity is a stable time series.
    pub fn with_measurement(mut self, measurement: MeasurementSemantics) -> Self {
        self.measurement = measurement;
        self
    }

    /// Declare the lenses this source feeds.
    pub fn feeding(mut self, lenses: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.feeds_lenses = lenses.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_category(mut self, category: impl Into<String>) -> Self {
        self.category = category.into();
        self
    }

    pub fn with_priority(mut self, priority: u32) -> Self {
        self.priority = priority;
        self
    }

    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    Http,
    Https,
    Websocket,
    Sftp,
    Ftp,
    Grpc,
    Local,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataFormat {
    Json,
    GeoJson,
    Csv,
    Xml,
    Rss,
    Atom,
    Protobuf,
    PlainText,
    Binary,
}

/// How often a source is expected to produce data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cadence {
    /// A source that emits records whenever something happens.
    Event,
    /// Polled on a fixed interval, in seconds.
    Interval { seconds: u64 },
    /// Once per day at a fixed UTC hour.
    Daily { hour_utc: u8 },
    /// Irregular; collector decides.
    Irregular,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    None,
    ApiKey,
    OAuth,
    Basic,
    Token,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cost {
    Free,
    FreeWithRegistration,
    Paid,
    Unknown,
}

/// Runtime health of a source, kept strictly separate from world data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceHealth {
    pub source_id: SourceId,
    pub last_success: Option<DateTime<Utc>>,
    pub last_failure: Option<DateTime<Utc>>,
    pub last_latency_ms: Option<u64>,
    pub records_received: u64,
    pub records_changed: u64,
    pub records_duplicate: u64,
    pub error_count: u64,
    /// How many of the failures were the source throttling us.
    pub rate_limit_count: u64,
    pub consecutive_failures: u32,
    /// Short, credential-free description of the most recent failure.
    pub last_error: Option<String>,
    pub status: HealthStatus,
}

impl SourceHealth {
    pub fn new(source_id: SourceId) -> Self {
        Self {
            source_id,
            last_success: None,
            last_failure: None,
            last_latency_ms: None,
            records_received: 0,
            records_changed: 0,
            records_duplicate: 0,
            error_count: 0,
            rate_limit_count: 0,
            consecutive_failures: 0,
            last_error: None,
            status: HealthStatus::Unknown,
        }
    }

    pub fn record_success(
        &mut self,
        at: DateTime<Utc>,
        latency_ms: u64,
        received: u64,
        changed: u64,
        duplicates: u64,
    ) {
        self.last_success = Some(at);
        self.last_latency_ms = Some(latency_ms);
        self.records_received += received;
        self.records_changed += changed;
        self.records_duplicate += duplicates;
        self.consecutive_failures = 0;
        self.last_error = None;
        self.status = HealthStatus::Healthy;
    }

    pub fn record_failure(&mut self, at: DateTime<Utc>) {
        self.record_failure_with(at, FailureKind::Unknown, None);
    }

    /// Record a failure, distinguishing throttling from real breakage.
    ///
    /// "Rate limited" and "down" are different facts about a source, and the
    /// reader must be able to tell them apart: a throttled source is healthy,
    /// we are simply asking too often.
    pub fn record_failure_with(
        &mut self,
        at: DateTime<Utc>,
        kind: FailureKind,
        detail: Option<String>,
    ) {
        self.last_failure = Some(at);
        self.error_count += 1;
        if kind == FailureKind::RateLimited {
            self.rate_limit_count += 1;
        }
        self.last_error = detail;
        self.status = match kind {
            FailureKind::RateLimited => HealthStatus::RateLimited,
            _ => {
                self.consecutive_failures += 1;
                if self.consecutive_failures >= 3 {
                    HealthStatus::Down
                } else {
                    HealthStatus::Degraded
                }
            }
        };
    }
}

/// Why a collection run failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    /// The source asked us to slow down (typically HTTP 429).
    RateLimited,
    /// The source could not be reached.
    Transport,
    /// The source answered with something we could not read.
    Parse,
    /// Anything else.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    Unknown,
    Healthy,
    /// Recovering, or failing once or twice.
    Degraded,
    /// Repeatedly unreachable, or answering unreadably.
    Down,
    /// The source is fine; we are polling it too often.
    RateLimited,
}

impl HealthStatus {
    /// A stable lower-case name for display and API payloads.
    pub fn as_str(&self) -> &'static str {
        match self {
            HealthStatus::Unknown => "unknown",
            HealthStatus::Healthy => "healthy",
            HealthStatus::Degraded => "degraded",
            HealthStatus::Down => "down",
            HealthStatus::RateLimited => "rate_limited",
        }
    }

    /// Whether the source is currently usable.
    pub fn is_healthy(&self) -> bool {
        matches!(self, HealthStatus::Healthy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_failures_escalate_status() {
        let mut h = SourceHealth::new(SourceId::new("src_test"));
        let now = Utc::now();
        h.record_failure(now);
        assert_eq!(h.status, HealthStatus::Degraded);
        h.record_failure(now);
        h.record_failure(now);
        assert_eq!(h.status, HealthStatus::Down);
        assert_eq!(h.consecutive_failures, 3);
    }

    #[test]
    fn success_resets_failure_streak() {
        let mut h = SourceHealth::new(SourceId::new("src_test"));
        let now = Utc::now();
        h.record_failure(now);
        h.record_success(now, 42, 10, 3, 7);
        assert_eq!(h.status, HealthStatus::Healthy);
        assert_eq!(h.consecutive_failures, 0);
        assert_eq!(h.records_changed, 3);
        assert_eq!(h.records_duplicate, 7);
    }

    #[test]
    fn rate_limiting_is_not_reported_as_down() {
        // A throttled source is healthy, we are asking too often. Even three
        // rate-limit answers in a row must not read as "down".
        let mut h = SourceHealth::new(SourceId::new("src_test"));
        let now = Utc::now();
        for _ in 0..3 {
            h.record_failure_with(now, FailureKind::RateLimited, Some("HTTP 429".into()));
        }
        assert_eq!(h.status, HealthStatus::RateLimited);
        assert_eq!(h.rate_limit_count, 3);
        assert_eq!(
            h.consecutive_failures, 0,
            "throttling is not a failure streak"
        );
        assert_eq!(h.last_error.as_deref(), Some("HTTP 429"));
    }

    #[test]
    fn a_real_failure_after_a_rate_limit_still_escalates() {
        let mut h = SourceHealth::new(SourceId::new("src_test"));
        let now = Utc::now();
        h.record_failure_with(now, FailureKind::RateLimited, None);
        h.record_failure_with(
            now,
            FailureKind::Transport,
            Some("connection refused".into()),
        );
        assert_eq!(h.status, HealthStatus::Degraded);
        assert_eq!(h.consecutive_failures, 1);
    }

    #[test]
    fn success_clears_the_last_error() {
        let mut h = SourceHealth::new(SourceId::new("src_test"));
        let now = Utc::now();
        h.record_failure_with(now, FailureKind::Parse, Some("bad json".into()));
        h.record_success(now, 1, 1, 1, 0);
        assert!(h.last_error.is_none());
    }

    #[test]
    fn source_round_trips_through_serde() {
        let s = Source::new(SourceId::new("usgs_earthquakes"), "USGS", "usgs_earthquake")
            .with_category("geophysics");
        let json = serde_json::to_string(&s).unwrap();
        let back: Source = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }
}
