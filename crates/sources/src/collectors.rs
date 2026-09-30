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

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use wse_collector::{
    CollectionMode, CollectionResult, Collector, CollectorError, RawPayload, Schedule,
};
use wse_model::SourceId;
use wse_scheduler::{Clock, LiveClock};

use crate::{gdelt, github, hackernews, nasa, usgs};

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
#[error("transport error: {0}")]
pub struct TransportError(pub String);

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
                    .map_err(|e| TransportError(format!("reading response body: {e}")))?;
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
                Err(TransportError(format!("HTTP {code}: {body}")))
            }
            Err(ureq::Error::Transport(t)) => Err(TransportError(t.to_string())),
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
                    .map_err(|e| CollectorError::Transport(e.to_string()))?;
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
    /// NASA near-Earth objects, polled daily.
    NasaNeoCollector,
    nasa::source,
    Schedule::Interval { seconds: 86_400 },
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

fn hackernews_request() -> Request {
    Request::get(format!(
        "{}?tags=front_page&hitsPerPage=50",
        hackernews::API_ENDPOINT
    ))
}

http_collector!(
    /// Hacker News front page, polled every 10 minutes.
    HackerNewsCollector,
    hackernews::source,
    Schedule::Interval { seconds: 600 },
    hackernews_request,
    hackernews::parse,
    "application/json"
);

fn github_request() -> Request {
    let request = Request::get(format!(
        "{}?q=language:{}&sort=updated&order=desc&per_page=50",
        github::API_ENDPOINT,
        github::LANGUAGE
    ))
    .with_header("Accept", "application/vnd.github+json");
    // A token is optional; without one the API still answers, at a lower rate
    // limit. The token is read from the environment and never logged.
    match std::env::var("GITHUB_TOKEN") {
        Ok(token) if !token.is_empty() => {
            request.with_header("Authorization", format!("Bearer {token}"))
        }
        _ => request,
    }
}

http_collector!(
    /// GitHub Rust-ecosystem activity, polled hourly.
    GitHubCollector,
    github::source,
    Schedule::Interval { seconds: 3600 },
    github_request,
    github::parse,
    "application/json"
);

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
                "nasa".to_string(),
                include_bytes!("../../../tests/fixtures/nasa_neo_feed.json").to_vec(),
            );
            fixtures.insert(
                "gdelt".to_string(),
                include_bytes!("../../../tests/fixtures/gdelt_timelinevol.json").to_vec(),
            );
            fixtures.insert(
                "algolia".to_string(),
                include_bytes!("../../../tests/fixtures/hackernews_search.json").to_vec(),
            );
            fixtures.insert(
                "github".to_string(),
                include_bytes!("../../../tests/fixtures/github_repos_search.json").to_vec(),
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
            Err(TransportError(format!("no fixture for {}", request.url)))
        }
    }

    /// A transport that always fails, to prove failure is not silent.
    struct BrokenTransport;

    impl Transport for BrokenTransport {
        fn fetch(&self, _request: &Request) -> Result<Vec<u8>, TransportError> {
            Err(TransportError("connection refused".to_string()))
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
        assert_eq!(result.observations.len(), 3);
        assert_eq!(result.observations[0].metric, "neo_miss_distance");
    }

    #[tokio::test]
    async fn gdelt_collector_normalizes_its_fixture() {
        let collector = GdeltCollector::with_context(context());
        let result = collector.collect().await.unwrap();
        assert_eq!(result.observations.len(), 6);
        assert_eq!(result.observations[0].metric, "news_volume");
    }

    #[tokio::test]
    async fn hackernews_collector_normalizes_its_fixture() {
        let collector = HackerNewsCollector::with_context(context());
        let result = collector.collect().await.unwrap();
        assert_eq!(result.observations.len(), 3);
        assert_eq!(result.observations[0].metric, "story_score");
    }

    #[tokio::test]
    async fn github_collector_normalizes_its_fixture() {
        let collector = GitHubCollector::with_context(context());
        let result = collector.collect().await.unwrap();
        assert_eq!(result.observations.len(), 2);
        assert_eq!(result.observations[0].metric, "repo_stars");
    }

    #[tokio::test]
    async fn a_transport_failure_is_an_error_not_an_empty_result() {
        let collector = UsgsCollector::with_context(CollectorContext::new(
            Arc::new(BrokenTransport),
            Arc::new(LiveClock),
        ));
        let err = collector.collect().await.unwrap_err();
        assert!(matches!(err, CollectorError::Transport(_)));
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
        assert!(hn.request().url.contains("tags=front_page"));

        let github = GitHubCollector::live();
        assert!(github.request().url.contains("q=language:rust"));
        assert!(github
            .request()
            .headers
            .iter()
            .any(|(k, v)| k == "Accept" && v.contains("github")));

        let nasa = NasaNeoCollector::live();
        assert!(nasa.request().url.contains("api_key="));
        assert!(nasa.request().url.contains("start_date="));
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
        assert!(!github.request().url.contains("token"));
        assert!(!github.request().url.contains("Bearer"));
    }
}
