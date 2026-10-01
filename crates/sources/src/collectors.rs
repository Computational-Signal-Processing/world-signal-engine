//! HTTP collectors: fetch a payload, hand it to the source's `parse`.
//!
//! Every collector here is generic over a [`Transport`]. The live transport is
//! [`LiveTransport`]; tests substitute a fixture-backed one, so the whole
//! collector — URL building, status handling, normalization, result assembly —
//! is exercised without touching the network.
//!
//! A collector that cannot reach its source returns `Err`, which the engine
//! records as source health. A collector that reaches its source and finds no
//! records returns `Ok` with zero observations. The two are never conflated.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use wse_collector::{
    CollectionMode, CollectionResult, Collector, CollectorError, RawPayload, Schedule,
};
use wse_model::SourceId;
use wse_scheduler::{Clock, LiveClock};

use crate::{
    afad, arxiv, cisa_kev, crossref, ecb, eonet, gdelt, github, hackernews, nasa, noaa_kp, nws,
    usgs,
};

/// A single HTTP GET, plus the headers the source needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub url: String,
    pub headers: Vec<(String, String)>,
}

impl Request {
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            headers: Vec::new(),
        }
    }

    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
}

/// A transport error. The message is safe to log: collectors must never put
/// credentials into it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("transport error: {message}")]
pub struct TransportError {
    pub message: String,
    /// The HTTP status, when the source answered with one.
    pub status: Option<u16>,
}

impl TransportError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            status: None,
        }
    }

    pub fn with_status(status: u16, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            status: Some(status),
        }
    }

    /// Whether the source is throttling us rather than broken.
    pub fn is_rate_limited(&self) -> bool {
        self.status == Some(429)
    }
}

impl From<String> for TransportError {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

impl From<&str> for TransportError {
    fn from(message: &str) -> Self {
        Self::new(message)
    }
}

/// How a collector reaches its source.
pub trait Transport: Send + Sync {
    /// Perform the request and return the response body.
    fn fetch(&self, request: &Request) -> Result<Vec<u8>, TransportError>;
}

/// Real HTTP over TLS.
#[derive(Debug, Clone, Copy, Default)]
pub struct LiveTransport;

impl Transport for LiveTransport {
    fn fetch(&self, request: &Request) -> Result<Vec<u8>, TransportError> {
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("world-signal-engine/0.1 (+https://github.com/)")
            .build();

        let mut call = agent.get(&request.url);
        for (name, value) in &request.headers {
            call = call.set(name, value);
        }

        match call.call() {
            Ok(response) => {
                let mut body = Vec::new();
                response
                    .into_reader()
                    .read_to_end(&mut body)
                    .map_err(|e| TransportError::new(format!("reading response body: {e}")))?;
                Ok(body)
            }
            Err(ureq::Error::Status(code, response)) => {
                // Keep the body: sources often explain the problem in JSON.
                let body = response
                    .into_string()
                    .unwrap_or_default()
                    .chars()
                    .take(200)
                    .collect::<String>();
                Err(TransportError::with_status(
                    code,
                    format!("HTTP {code}: {body}"),
                ))
            }
            Err(ureq::Error::Transport(t)) => Err(TransportError::new(t.to_string())),
        }
    }
}

use std::io::Read as _;

/// Everything a collector needs that is not source-specific.
#[derive(Clone)]
pub struct CollectorContext {
    pub transport: Arc<dyn Transport>,
    pub clock: Arc<dyn Clock>,
}

impl Default for CollectorContext {
    fn default() -> Self {
        Self::live()
    }
}

impl CollectorContext {
    pub fn live() -> Self {
        Self {
            transport: Arc::new(LiveTransport),
            clock: Arc::new(LiveClock),
        }
    }

    pub fn new(transport: Arc<dyn Transport>, clock: Arc<dyn Clock>) -> Self {
        Self { transport, clock }
    }

    fn now(&self) -> DateTime<Utc> {
        self.clock.now()
    }
}

impl std::fmt::Debug for CollectorContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CollectorContext")
            .field("mode", &self.clock.mode())
            .finish()
    }
}

/// Assemble a [`CollectionResult`] from a payload and its parsed observations.
fn build_result(
    source_id: SourceId,
    request: &Request,
    body: &[u8],
    observations: Vec<wse_model::Observation>,
    content_type: &str,
    started_at: DateTime<Utc>,
) -> CollectionResult {
    let received = observations.len() as u64;
    let mut result = CollectionResult::new(source_id);
    result.observations = observations;
    result.raw_payloads = vec![RawPayload::new(
        request.url.clone(),
        body.to_vec(),
        content_type,
    )];
    result.records_received = received;
    result.records_changed = received;
    result.started_at = Some(started_at);
    result.finished_at = Some(Utc::now());
    result
}

macro_rules! http_collector {
    (
        $(#[$meta:meta])*
        $name:ident, $source:path, $schedule:expr, $request:ident, $parse:path, $content_type:literal
    ) => {
        $(#[$meta])*
        #[derive(Clone)]
        pub struct $name {
            context: CollectorContext,
            request: Request,
        }

        impl $name {
            /// Build the collector against a live transport and the wall clock.
            pub fn live() -> Self {
                Self::with_context(CollectorContext::live())
            }

            /// Build with an explicit transport and clock (tests, replay).
            pub fn with_context(context: CollectorContext) -> Self {
                let request = $request();
                Self { context, request }
            }

            /// Override the request, e.g. to widen a query.
            pub fn with_request(mut self, request: Request) -> Self {
                self.request = request;
                self
            }

            pub fn request(&self) -> &Request {
                &self.request
            }
        }

        #[async_trait]
        impl Collector for $name {
            fn source_id(&self) -> SourceId {
                $source().id
            }

            fn schedule(&self) -> Schedule {
                $schedule
            }

            fn mode(&self) -> CollectionMode {
                self.context.clock.mode()
            }

            async fn collect(&self) -> Result<CollectionResult, CollectorError> {
                let started_at = self.context.now();
                let body = self
                    .context
                    .transport
                    .fetch(&self.request)
                    .map_err(|e| {
                        // A 429 is the source throttling us, not a broken
                        // source; surface it as its own error kind.
                        if e.is_rate_limited() {
                            CollectorError::RateLimited(e.message)
                        } else {
                            CollectorError::Transport(e.message)
                        }
                    })?;
                let observations = $parse(&body, started_at)?;
                Ok(build_result(
                    $source().id,
                    &self.request,
                    &body,
                    observations,
                    $content_type,
                    started_at,
                ))
            }
        }
    };
}

fn usgs_request() -> Request {
    Request::get(usgs::ALL_HOUR_ENDPOINT).with_header("Accept", "application/geo+json")
}

http_collector!(
    /// USGS earthquakes, polled every 60 seconds.
    UsgsCollector,
    usgs::source,
    Schedule::Event { poll_seconds: 60 },
    usgs_request,
    usgs::parse,
    "application/geo+json"
);

fn nasa_request() -> Request {
    // NASA requires a key; DEMO_KEY is the documented public placeholder and
    // is rate-limited, which is exactly what a default should be.
    let key = std::env::var("NASA_API_KEY").unwrap_or_else(|_| "DEMO_KEY".to_string());
    let today = Utc::now().date_naive();
    let start = today - chrono::Duration::days(6);
    Request::get(format!(
        "{}?start_date={}&end_date={}&api_key={}",
        nasa::FEED_ENDPOINT,
        start.format("%Y-%m-%d"),
        today.format("%Y-%m-%d"),
        key
    ))
}

http_collector!(
    /// NASA near-Earth objects, collected daily at the hour the feed publishes.
    NasaNeoCollector,
    nasa::source,
    Schedule::Daily { hour_utc: 6 },
    nasa_request,
    nasa::parse,
    "application/json"
);

fn gdelt_request() -> Request {
    let query = gdelt::DEFAULT_QUERY.replace(' ', "+");
    // GDELT rate-limits to roughly one request every five seconds and answers
    // 429 (with a body) when that is exceeded. The scheduler's 15-minute
    // cadence stays well inside the limit.
    Request::get(format!(
        "{}?query={}&mode=timelinevol&format=json&timespan=1d",
        gdelt::API_ENDPOINT,
        query
    ))
}

http_collector!(
    /// GDELT news volume for the tracked topic, polled every 15 minutes.
    GdeltCollector,
    gdelt::source,
    Schedule::Interval { seconds: 900 },
    gdelt_request,
    gdelt::parse,
    "application/json"
);

/// GitHub is collected differently from the single-request sources: it walks
/// the fixed repository universe, one request per repository, and merges the
/// per-repository observations into one result.
#[derive(Clone)]
pub struct GitHubCollector {
    context: CollectorContext,
}

impl GitHubCollector {
    pub fn live() -> Self {
        Self::with_context(CollectorContext::live())
    }

    pub fn with_context(context: CollectorContext) -> Self {
        Self { context }
    }

    /// The request for one repository in the universe.
    pub fn repo_request(&self, repo: &str) -> Request {
        let request = Request::get(github::REPO_ENDPOINT.replace("{repo}", repo))
            .with_header("Accept", "application/vnd.github+json");
        match std::env::var("GITHUB_TOKEN") {
            Ok(token) if !token.is_empty() => {
                request.with_header("Authorization", format!("Bearer {token}"))
            }
            _ => request,
        }
    }
}

#[async_trait]
impl Collector for GitHubCollector {
    fn source_id(&self) -> SourceId {
        github::source().id
    }

    fn schedule(&self) -> Schedule {
        Schedule::Interval { seconds: 3600 }
    }

    fn mode(&self) -> CollectionMode {
        self.context.clock.mode()
    }

    async fn collect(&self) -> Result<CollectionResult, CollectorError> {
        let started_at = self.context.now();
        let mut result = CollectionResult::new(self.source_id());
        let mut errors = 0usize;

        for repo in github::UNIVERSE {
            let request = self.repo_request(repo);
            match self.context.transport.fetch(&request) {
                Ok(body) => {
                    result.records_received += 1;
                    match github::parse_repo(&body, started_at)? {
                        Some(observation) => {
                            result.records_changed += 1;
                            result.observations.push(observation);
                        }
                        // Archived or unmeasurable: counted, not an observation.
                        None => result.records_duplicate += 1,
                    }
                    result.raw_payloads.push(RawPayload::new(
                        request.url.clone(),
                        body,
                        "application/json",
                    ));
                }
                Err(err) => {
                    errors += 1;
                    result.errors.push(format!("{repo}: {}", err.message));
                }
            }
        }

        // Every request failed: the source is down, not empty. Reporting an
        // empty success would read as "no activity" — the exact confusion the
        // brief forbids.
        if errors > 0 && result.observations.is_empty() {
            return Err(CollectorError::Transport(format!(
                "all {} repository requests failed: {}",
                github::UNIVERSE.len(),
                result.errors.first().cloned().unwrap_or_default()
            )));
        }

        result.started_at = Some(started_at);
        result.finished_at = Some(Utc::now());
        Ok(result)
    }
}

/// Hacker News is collected in two steps: resolve the fixed universe from the
/// top-story list, then fetch each tracked item. The item ids are durable, so a
/// story's score stays comparable across collections.
///
/// The universe is **carried across collections** in `universe`. It is resolved
/// once and then held: a story leaving the front page does not replace it,
/// because that would make the series move with membership rather than with
/// attention. A slot is freed only when a tracked item no longer resolves.
#[derive(Clone)]
pub struct HackerNewsCollector {
    context: CollectorContext,
    /// The committed universe, shared by clones. Empty until the first
    /// successful resolution.
    universe: Arc<Mutex<Vec<i64>>>,
}

impl HackerNewsCollector {
    pub fn live() -> Self {
        Self::with_context(CollectorContext::live())
    }

    pub fn with_context(context: CollectorContext) -> Self {
        Self {
            context,
            universe: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn top_request(&self) -> Request {
        Request::get(hackernews::TOP_STORIES_ENDPOINT)
    }

    pub fn item_request(&self, id: i64) -> Request {
        Request::get(hackernews::ITEM_ENDPOINT.replace("{id}", &id.to_string()))
    }

    /// The ids currently committed to the universe, for tests and inspection.
    pub fn tracked(&self) -> Vec<i64> {
        self.universe.lock().map(|u| u.clone()).unwrap_or_default()
    }
}

#[async_trait]
impl Collector for HackerNewsCollector {
    fn source_id(&self) -> SourceId {
        hackernews::source().id
    }

    fn schedule(&self) -> Schedule {
        Schedule::Interval { seconds: 600 }
    }

    fn mode(&self) -> CollectionMode {
        self.context.clock.mode()
    }

    async fn collect(&self) -> Result<CollectionResult, CollectorError> {
        let started_at = self.context.now();
        let mut result = CollectionResult::new(self.source_id());

        // The top-story list is read every collection, but only as the source of
        // *candidates* for the fixed universe — never as the universe itself.
        let top_request = self.top_request();
        let top_body = self
            .context
            .transport
            .fetch(&top_request)
            .map_err(|e| CollectorError::Transport(e.message))?;
        let candidates = hackernews::parse_ids(&top_body)?;
        result.raw_payloads.push(RawPayload::new(
            top_request.url.clone(),
            top_body,
            "application/json",
        ));

        // Measure the ids we already committed to, plus any candidate needed to
        // fill a free slot. Reusing the same ids every collection is what makes
        // the series comparable; a story leaving the top list does not evict it.
        let current = self.tracked();
        let targets =
            hackernews::next_universe(&current, &current, &candidates, hackernews::UNIVERSE_SIZE);

        let mut resolved: Vec<i64> = Vec::new();
        for id in &targets {
            let request = self.item_request(*id);
            match self.context.transport.fetch(&request) {
                Ok(body) => {
                    // A story that still resolves keeps its slot even if it has
                    // gone quiet (parses to `None`, e.g. dead/no score).
                    resolved.push(*id);
                    result.records_received += 1;
                    match hackernews::parse_item(&body, started_at)? {
                        Some(observation) => {
                            result.records_changed += 1;
                            result.observations.push(observation);
                        }
                        None => result.records_duplicate += 1,
                    }
                    result.raw_payloads.push(RawPayload::new(
                        request.url.clone(),
                        body,
                        "application/json",
                    ));
                }
                Err(err) => {
                    // A story that is genuinely gone (404) frees its slot. Any
                    // other failure is transient: hold the slot so a network
                    // blip is never read as "the story disappeared".
                    if err.status != Some(404) {
                        resolved.push(*id);
                    }
                    result.errors.push(format!("item {id}: {}", err.message));
                }
            }
        }

        // Commit the universe: ids that resolved are kept, gone ids are dropped
        // and their slots refilled from this cycle's candidates.
        let next =
            hackernews::next_universe(&current, &resolved, &candidates, hackernews::UNIVERSE_SIZE);
        if let Ok(mut universe) = self.universe.lock() {
            *universe = next;
        }

        result.started_at = Some(started_at);
        result.finished_at = Some(Utc::now());
        Ok(result)
    }
}

fn nws_request() -> Request {
    // The NWS asks callers to identify themselves; a plain User-Agent is the
    // documented courtesy and needs no key.
    Request::get(nws::ALERTS_ENDPOINT)
        .with_header("Accept", "application/geo+json")
        .with_header("User-Agent", "world-signal-engine/0.1")
}

http_collector!(
    /// NWS active weather alerts, polled every 10 minutes.
    NwsAlertsCollector,
    nws::source,
    Schedule::Interval { seconds: 600 },
    nws_request,
    nws::parse,
    "application/geo+json"
);

fn eonet_request() -> Request {
    Request::get(eonet::EVENTS_ENDPOINT).with_header("Accept", "application/json")
}

http_collector!(
    /// NASA EONET natural events, polled every 30 minutes.
    EonetCollector,
    eonet::source,
    Schedule::Interval { seconds: 1800 },
    eonet_request,
    eonet::parse,
    "application/json"
);

fn cisa_kev_request() -> Request {
    Request::get(cisa_kev::API_ENDPOINT).with_header("Accept", "application/json")
}

http_collector!(
    /// CISA Known Exploited Vulnerabilities, collected daily.
    CisaKevCollector,
    cisa_kev::source,
    Schedule::Interval { seconds: 86_400 },
    cisa_kev_request,
    cisa_kev::parse,
    "application/json"
);

fn ecb_request() -> Request {
    Request::get(ecb::API_ENDPOINT).with_header("Accept", "application/json")
}

http_collector!(
    /// ECB euro reference rates, collected daily.
    EcbRatesCollector,
    ecb::source,
    Schedule::Interval { seconds: 86_400 },
    ecb_request,
    ecb::parse,
    "application/json"
);

fn noaa_kp_request() -> Request {
    Request::get(noaa_kp::API_ENDPOINT).with_header("Accept", "application/json")
}

http_collector!(
    /// NOAA planetary K-index, polled hourly.
    NoaaKpCollector,
    noaa_kp::source,
    Schedule::Interval { seconds: 3600 },
    noaa_kp_request,
    noaa_kp::parse,
    "application/json"
);

/// Crossref is collected per topic: one request per fixed topic, merged into
/// one result. The topic list is fixed, so each topic is a stable series.
#[derive(Clone)]
pub struct CrossrefCollector {
    context: CollectorContext,
}

impl CrossrefCollector {
    pub fn live() -> Self {
        Self::with_context(CollectorContext::live())
    }

    pub fn with_context(context: CollectorContext) -> Self {
        Self { context }
    }

    pub fn topic_request(&self, query: &str, now: DateTime<Utc>) -> Request {
        let (from, until) = crossref::window(now);
        let mailto = std::env::var("CROSSREF_MAILTO").ok();
        Request::get(crossref::works_url(query, &from, &until, mailto.as_deref()))
            .with_header("Accept", "application/json")
    }
}

#[async_trait]
impl Collector for CrossrefCollector {
    fn source_id(&self) -> SourceId {
        crossref::source().id
    }

    fn schedule(&self) -> Schedule {
        Schedule::Interval { seconds: 86_400 }
    }

    fn mode(&self) -> CollectionMode {
        self.context.clock.mode()
    }

    async fn collect(&self) -> Result<CollectionResult, CollectorError> {
        let started_at = self.context.now();
        let mut result = CollectionResult::new(self.source_id());
        let mut errors = 0usize;

        for (slug, label, query) in crossref::TOPICS {
            let request = self.topic_request(query, started_at);
            match self.context.transport.fetch(&request) {
                Ok(body) => {
                    result.records_received += 1;
                    let count = crossref::parse_count(&body)?;
                    result.observations.push(crossref::observation_for(
                        slug, label, count, started_at, &body,
                    ));
                    result.records_changed += 1;
                    result.raw_payloads.push(RawPayload::new(
                        request.url.clone(),
                        body,
                        "application/json",
                    ));
                }
                Err(err) => {
                    errors += 1;
                    result.errors.push(format!("{slug}: {}", err.message));
                }
            }
        }

        if errors > 0 && result.observations.is_empty() {
            return Err(CollectorError::Transport(format!(
                "all crossref topic requests failed: {}",
                result.errors.first().cloned().unwrap_or_default()
            )));
        }

        result.started_at = Some(started_at);
        result.finished_at = Some(Utc::now());
        Ok(result)
    }
}

/// arXiv is collected per category, one request each.
#[derive(Clone)]
pub struct ArxivCollector {
    context: CollectorContext,
}

impl ArxivCollector {
    pub fn live() -> Self {
        Self::with_context(CollectorContext::live())
    }

    pub fn with_context(context: CollectorContext) -> Self {
        Self { context }
    }

    pub fn category_request(&self, category: &str) -> Request {
        Request::get(arxiv::category_url(category)).with_header("Accept", "application/atom+xml")
    }
}

#[async_trait]
impl Collector for ArxivCollector {
    fn source_id(&self) -> SourceId {
        arxiv::source().id
    }

    fn schedule(&self) -> Schedule {
        Schedule::Interval { seconds: 86_400 }
    }

    fn mode(&self) -> CollectionMode {
        self.context.clock.mode()
    }

    async fn collect(&self) -> Result<CollectionResult, CollectorError> {
        let started_at = self.context.now();
        let mut result = CollectionResult::new(self.source_id());
        let mut errors = 0usize;

        for (slug, label, category) in arxiv::CATEGORIES {
            let request = self.category_request(category);
            match self.context.transport.fetch(&request) {
                Ok(body) => {
                    result.records_received += 1;
                    let total = arxiv::parse_total(&body)?;
                    result.observations.push(arxiv::observation_for(
                        slug, label, total, started_at, &body,
                    ));
                    result.records_changed += 1;
                    result.raw_payloads.push(RawPayload::new(
                        request.url.clone(),
                        body,
                        "application/atom+xml",
                    ));
                }
                Err(err) => {
                    errors += 1;
                    result.errors.push(format!("{slug}: {}", err.message));
                }
            }
        }

        if errors > 0 && result.observations.is_empty() {
            return Err(CollectorError::Transport(format!(
                "all arxiv category requests failed: {}",
                result.errors.first().cloned().unwrap_or_default()
            )));
        }

        result.started_at = Some(started_at);
        result.finished_at = Some(Utc::now());
        Ok(result)
    }
}

/// AFAD is collected over a rolling window computed at collection time, so a
/// long-running process does not freeze the window it started with.
#[derive(Clone)]
pub struct AfadCollector {
    context: CollectorContext,
}

impl AfadCollector {
    pub fn live() -> Self {
        Self::with_context(CollectorContext::live())
    }

    pub fn with_context(context: CollectorContext) -> Self {
        Self { context }
    }

    pub fn window_request(&self, now: DateTime<Utc>) -> Request {
        let end = now.date_naive();
        let start = end - chrono::Duration::days(1);
        Request::get(afad::filter_url(
            &start.format("%Y-%m-%d").to_string(),
            &end.format("%Y-%m-%d").to_string(),
        ))
        .with_header("Accept", "application/json")
    }
}

#[async_trait]
impl Collector for AfadCollector {
    fn source_id(&self) -> SourceId {
        afad::source().id
    }

    fn schedule(&self) -> Schedule {
        Schedule::Interval { seconds: 3600 }
    }

    fn mode(&self) -> CollectionMode {
        self.context.clock.mode()
    }

    async fn collect(&self) -> Result<CollectionResult, CollectorError> {
        let started_at = self.context.now();
        let request = self.window_request(started_at);
        let body = self
            .context
            .transport
            .fetch(&request)
            .map_err(|e| CollectorError::Transport(e.message))?;
        let observations = afad::parse(&body, started_at)?;
        Ok(build_result(
            afad::source().id,
            &request,
            &body,
            observations,
            "application/json",
            started_at,
        ))
    }
}

/// Build one live collector per catalog entry.
///
/// This is the bridge from the catalog to the pipeline: adding a source means
/// adding an arm here (or, better, a registration call) and nothing else.
pub fn live_collectors() -> Vec<Box<dyn Collector>> {
    vec![
        Box::new(UsgsCollector::live()),
        Box::new(NasaNeoCollector::live()),
        Box::new(GdeltCollector::live()),
        Box::new(HackerNewsCollector::live()),
        Box::new(GitHubCollector::live()),
        Box::new(NwsAlertsCollector::live()),
        Box::new(EonetCollector::live()),
        Box::new(NoaaKpCollector::live()),
        Box::new(CisaKevCollector::live()),
        Box::new(EcbRatesCollector::live()),
        Box::new(CrossrefCollector::live()),
        Box::new(ArxivCollector::live()),
        Box::new(AfadCollector::live()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A transport backed by checked-in fixtures, keyed by URL substring.
    struct FixtureTransport {
        fixtures: HashMap<String, Vec<u8>>,
    }

    impl FixtureTransport {
        fn new() -> Self {
            let mut fixtures = HashMap::new();
            fixtures.insert(
                "usgs".to_string(),
                include_bytes!("../../../tests/fixtures/usgs_all_hour.geojson").to_vec(),
            );
            fixtures.insert(
                "neo/rest".to_string(),
                include_bytes!("../../../tests/fixtures/nasa_neo_feed.json").to_vec(),
            );
            fixtures.insert(
                "gdelt".to_string(),
                include_bytes!("../../../tests/fixtures/gdelt_timelinevol.json").to_vec(),
            );
            fixtures.insert(
                "topstories".to_string(),
                include_bytes!("../../../tests/fixtures/hackernews_topstories.json").to_vec(),
            );
            fixtures.insert(
                "item/41000001".to_string(),
                include_bytes!("../../../tests/fixtures/hackernews_item.json").to_vec(),
            );
            fixtures.insert(
                "item/41000002".to_string(),
                include_bytes!("../../../tests/fixtures/hackernews_item2.json").to_vec(),
            );
            fixtures.insert(
                "item/41000003".to_string(),
                include_bytes!("../../../tests/fixtures/hackernews_item3.json").to_vec(),
            );
            fixtures.insert(
                "repos/".to_string(),
                include_bytes!("../../../tests/fixtures/github_repo.json").to_vec(),
            );
            fixtures.insert(
                "known_exploited".to_string(),
                include_bytes!("../../../tests/fixtures/cisa_kev.json").to_vec(),
            );
            fixtures.insert(
                "EXR/".to_string(),
                include_bytes!("../../../tests/fixtures/ecb_exr.json").to_vec(),
            );
            fixtures.insert(
                "planetary-k-index".to_string(),
                include_bytes!("../../../tests/fixtures/noaa_kp_index.json").to_vec(),
            );
            fixtures.insert(
                "api.crossref.org".to_string(),
                include_bytes!("../../../tests/fixtures/crossref_works.json").to_vec(),
            );
            fixtures.insert(
                "export.arxiv.org".to_string(),
                include_bytes!("../../../tests/fixtures/arxiv_query.xml").to_vec(),
            );
            fixtures.insert(
                "deprem.afad".to_string(),
                include_bytes!("../../../tests/fixtures/afad_events.json").to_vec(),
            );
            Self { fixtures }
        }
    }

    impl Transport for FixtureTransport {
        fn fetch(&self, request: &Request) -> Result<Vec<u8>, TransportError> {
            for (needle, body) in &self.fixtures {
                if request.url.contains(needle) {
                    return Ok(body.clone());
                }
            }
            Err(TransportError::new(format!(
                "no fixture for {}",
                request.url
            )))
        }
    }

    /// A transport that always fails, to prove failure is not silent.
    struct BrokenTransport;

    impl Transport for BrokenTransport {
        fn fetch(&self, _request: &Request) -> Result<Vec<u8>, TransportError> {
            Err(TransportError::new("connection refused"))
        }
    }

    /// A transport that always answers 429, to prove throttling is classified.
    struct RateLimitedTransport;

    impl Transport for RateLimitedTransport {
        fn fetch(&self, _request: &Request) -> Result<Vec<u8>, TransportError> {
            Err(TransportError::with_status(429, "HTTP 429: rate limited"))
        }
    }

    fn context() -> CollectorContext {
        CollectorContext::new(Arc::new(FixtureTransport::new()), Arc::new(LiveClock))
    }

    #[tokio::test]
    async fn usgs_collector_normalizes_its_fixture() {
        let collector = UsgsCollector::with_context(context());
        let result = collector.collect().await.unwrap();
        assert_eq!(result.records_received, 4);
        assert_eq!(result.records_changed, 4);
        assert_eq!(result.observations.len(), 4);
        assert!(!result.is_failure());
        assert_eq!(
            result.raw_payloads.len(),
            1,
            "raw payload must be preserved"
        );
        assert!(result.raw_payloads[0].reference.bytes.unwrap() > 0);
    }

    #[tokio::test]
    async fn nasa_collector_normalizes_its_fixture() {
        let collector = NasaNeoCollector::with_context(context());
        let result = collector.collect().await.unwrap();
        // The fixture spans two UTC days.
        assert_eq!(result.observations.len(), 2);
        assert_eq!(result.observations[0].metric, "neo_close_approaches");
    }

    #[tokio::test]
    async fn gdelt_collector_normalizes_its_fixture() {
        let collector = GdeltCollector::with_context(context());
        let result = collector.collect().await.unwrap();
        assert_eq!(result.observations.len(), 6);
        assert_eq!(result.observations[0].metric, "news_volume");
    }

    #[tokio::test]
    async fn hackernews_collector_resolves_its_fixed_universe() {
        let collector = HackerNewsCollector::with_context(context());
        let result = collector.collect().await.unwrap();
        // One top-story list request plus one request per tracked story.
        assert_eq!(result.observations.len(), 3);
        assert_eq!(result.observations[0].metric, "story_score");
        assert_eq!(result.raw_payloads.len(), 4);
    }

    /// A transport whose top-story list changes and whose one chosen item can
    /// stop resolving, to drive the committed-universe refill path.
    struct ChurningUniverse {
        polls: Mutex<usize>,
        gone: Mutex<std::collections::HashSet<i64>>,
    }

    impl Transport for ChurningUniverse {
        fn fetch(&self, request: &Request) -> Result<Vec<u8>, TransportError> {
            if request.url.contains("topstories") {
                let mut n = self.polls.lock().unwrap();
                *n += 1;
                // Poll 1 offers 1..=3, poll 2 offers a disjoint 4..=6.
                let ids: Vec<i64> = if *n == 1 {
                    vec![1, 2, 3]
                } else {
                    vec![4, 5, 6]
                };
                return Ok(serde_json::to_vec(&ids).unwrap());
            }
            let id: i64 = request
                .url
                .rsplit('/')
                .next()
                .and_then(|s| s.split('.').next())
                .and_then(|s| s.parse().ok())
                .expect("item url");
            if self.gone.lock().unwrap().contains(&id) {
                return Err(TransportError::with_status(404, "not found"));
            }
            let body = serde_json::json!({
                "id": id, "type": "story", "title": format!("story {id}"),
                "score": 10.0, "time": 1_700_000_000
            });
            Ok(serde_json::to_vec(&body).unwrap())
        }
    }

    #[tokio::test]
    async fn hackernews_universe_is_committed_and_only_refilled_when_a_story_is_gone() {
        let transport = Arc::new(ChurningUniverse {
            polls: Mutex::new(0),
            gone: Mutex::new(std::collections::HashSet::new()),
        });
        let collector = HackerNewsCollector::with_context(CollectorContext::new(
            transport.clone(),
            Arc::new(LiveClock),
        ));

        // Poll 1 commits ids 1..=3.
        collector.collect().await.unwrap();
        assert_eq!(collector.tracked(), vec![1, 2, 3]);

        // Poll 2 offers a disjoint top list. The committed universe is kept:
        // a story leaving the front page is not a world change.
        let second = collector.collect().await.unwrap();
        assert_eq!(collector.tracked(), vec![1, 2, 3]);
        let ids: Vec<String> = second
            .observations
            .iter()
            .filter_map(|o| o.identity.clone())
            .collect();
        assert_eq!(ids, vec!["1", "2", "3"]);

        // Now story 2 genuinely disappears (404). Its slot is freed and refilled
        // from the current top list, but the surviving ids are untouched.
        transport.gone.lock().unwrap().insert(2);
        collector.collect().await.unwrap();
        assert_eq!(collector.tracked(), vec![1, 3, 4]);
    }

    #[tokio::test]
    async fn github_collector_walks_its_universe() {
        let collector = GitHubCollector::with_context(context());
        let result = collector.collect().await.unwrap();
        // One request and one observation per repository in the fixed
        // universe. De-duplication of unchanged star counts happens downstream
        // in the pipeline, not here.
        assert_eq!(result.observations.len(), github::UNIVERSE.len());
        assert!(result.observations.iter().all(|o| o.metric == "repo_stars"));
    }

    #[tokio::test]
    async fn cisa_kev_collector_emits_both_series() {
        let collector = CisaKevCollector::with_context(context());
        let result = collector.collect().await.unwrap();
        assert_eq!(result.observations.len(), 2);
        assert!(result.observations.iter().any(|o| o.metric == "kev_added"));
        assert!(result
            .observations
            .iter()
            .any(|o| o.metric == "kev_catalog_total"));
    }

    #[tokio::test]
    async fn ecb_collector_normalizes_its_fixture() {
        let collector = EcbRatesCollector::with_context(context());
        let result = collector.collect().await.unwrap();
        assert_eq!(result.observations.len(), 3);
        assert_eq!(result.observations[0].metric, "exchange_rate");
    }

    #[tokio::test]
    async fn noaa_kp_collector_normalizes_its_fixture() {
        let collector = NoaaKpCollector::with_context(context());
        let result = collector.collect().await.unwrap();
        assert_eq!(result.observations.len(), 3);
        assert_eq!(result.observations[0].metric, "kp_index");
    }

    #[tokio::test]
    async fn crossref_collector_covers_every_topic() {
        let collector = CrossrefCollector::with_context(context());
        let result = collector.collect().await.unwrap();
        assert_eq!(result.observations.len(), crossref::TOPICS.len());
        assert!(result
            .observations
            .iter()
            .all(|o| o.metric == "works_registered"));
    }

    #[tokio::test]
    async fn arxiv_collector_covers_every_category() {
        let collector = ArxivCollector::with_context(context());
        let result = collector.collect().await.unwrap();
        assert_eq!(result.observations.len(), arxiv::CATEGORIES.len());
        assert!(result
            .observations
            .iter()
            .all(|o| o.metric == "preprint_total"));
    }

    #[tokio::test]
    async fn afad_collector_normalizes_its_fixture() {
        let collector = AfadCollector::with_context(context());
        let result = collector.collect().await.unwrap();
        assert_eq!(result.observations.len(), 3);
        assert_eq!(result.observations[0].metric, "earthquake_magnitude");
    }

    #[tokio::test]
    async fn a_transport_failure_is_an_error_not_an_empty_result() {
        let collector = UsgsCollector::with_context(CollectorContext::new(
            Arc::new(BrokenTransport),
            Arc::new(LiveClock),
        ));
        let err = collector.collect().await.unwrap_err();
        assert!(matches!(err, CollectorError::Transport(_)));
        assert_eq!(err.kind(), wse_model::FailureKind::Transport);
    }

    #[tokio::test]
    async fn a_throttled_source_is_reported_as_rate_limited() {
        let collector = GdeltCollector::with_context(CollectorContext::new(
            Arc::new(RateLimitedTransport),
            Arc::new(LiveClock),
        ));
        let err = collector.collect().await.unwrap_err();
        assert!(matches!(err, CollectorError::RateLimited(_)), "got {err:?}");
        assert_eq!(err.kind(), wse_model::FailureKind::RateLimited);
    }

    #[test]
    fn each_collector_reports_the_catalog_source_id() {
        let ctx = context();
        assert_eq!(
            UsgsCollector::with_context(ctx.clone())
                .source_id()
                .as_str(),
            usgs::SOURCE_ID
        );
        assert_eq!(
            NasaNeoCollector::with_context(ctx.clone())
                .source_id()
                .as_str(),
            nasa::SOURCE_ID
        );
        assert_eq!(
            GdeltCollector::with_context(ctx.clone())
                .source_id()
                .as_str(),
            gdelt::SOURCE_ID
        );
        assert_eq!(
            HackerNewsCollector::with_context(ctx.clone())
                .source_id()
                .as_str(),
            hackernews::SOURCE_ID
        );
        assert_eq!(
            GitHubCollector::with_context(ctx).source_id().as_str(),
            github::SOURCE_ID
        );
    }

    #[test]
    fn requests_carry_the_parameters_the_apis_require() {
        let gdelt = GdeltCollector::live();
        assert!(gdelt.request().url.contains("mode=timelinevol"));
        assert!(gdelt.request().url.contains("format=json"));

        let hn = HackerNewsCollector::live();
        assert!(hn.top_request().url.contains("topstories"));

        let github = GitHubCollector::live();
        assert!(github
            .repo_request("rust-lang/rust")
            .url
            .contains("repos/rust-lang/rust"));
        assert!(github
            .repo_request("rust-lang/rust")
            .headers
            .iter()
            .any(|(k, v)| k == "Accept" && v.contains("github")));

        let nasa = NasaNeoCollector::live();
        assert!(nasa.request().url.contains("api_key="));
        assert!(nasa.request().url.contains("start_date="));

        let now = Utc::now();
        let crossref = CrossrefCollector::live();
        assert!(crossref
            .topic_request("machine learning", now)
            .url
            .contains("query.bibliographic=machine+learning"));

        let arxiv = ArxivCollector::live();
        assert!(arxiv.category_request("cs.AI").url.contains("cat:cs.AI"));
    }

    #[test]
    fn live_collectors_covers_the_whole_catalog() {
        let collectors = live_collectors();
        assert_eq!(collectors.len(), crate::catalog().len());
        let mut ids: Vec<String> = collectors
            .iter()
            .map(|c| c.source_id().as_str().to_string())
            .collect();
        ids.sort();
        let mut expected: Vec<String> = crate::catalog()
            .iter()
            .map(|s| s.id.as_str().to_string())
            .collect();
        expected.sort();
        assert_eq!(ids, expected);
    }

    #[test]
    fn collectors_never_put_a_token_in_the_url() {
        // A token in a URL leaks into logs and raw references.
        let github = GitHubCollector::live();
        let request = github.repo_request("rust-lang/rust");
        assert!(!request.url.contains("token"));
        assert!(!request.url.contains("Bearer"));
    }
}
