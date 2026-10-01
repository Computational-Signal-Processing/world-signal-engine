//! GDELT global news volume — JSON time series, free, no auth.
//!
//! GDELT is the global-events source. Rather than ingesting articles, this
//! collector uses the `timelinevol` mode, which returns the share of global
//! news coverage matching a query over time. That is already a series, so it
//! slots straight into the baseline engine.
//!
//! Docs: <https://blog.gdeltproject.org/gdelt-doc-2-0-api-debuts/>

use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use wse_collector::CollectorError;
use wse_model::{EntityId, Observation, RawReference, Source, SourceId};

pub const SOURCE_ID: &str = "gdelt_news_volume";
pub const COLLECTOR_TYPE: &str = "gdelt_timeline";

/// The tracked topic. Kept in one place so a deployment can widen or narrow it
/// without touching the collector.
pub const DEFAULT_QUERY: &str = "oil supply";

/// GDELT DOC 2.0 API.
pub const API_ENDPOINT: &str = "https://api.gdeltproject.org/api/v2/doc/doc";

/// The catalog entry.
pub fn source() -> Source {
    let mut parameters = std::collections::BTreeMap::new();
    parameters.insert("mode".to_string(), "timelinevol".to_string());
    parameters.insert("format".to_string(), "json".to_string());
    parameters.insert("query".to_string(), DEFAULT_QUERY.to_string());

    Source {
        id: SourceId::new(SOURCE_ID),
        name: "GDELT News Volume".to_string(),
        provider: "The GDELT Project".to_string(),
        category: "global_events".to_string(),
        subcategory: Some("news_volume".to_string()),
        endpoint: API_ENDPOINT.to_string(),
        protocol: wse_model::Protocol::Https,
        format: wse_model::DataFormat::Json,
        cadence: wse_model::Cadence::Interval { seconds: 900 },
        timezone: Some("UTC".to_string()),
        license: Some("Free for research and commercial use (GDELT terms)".to_string()),
        authentication: wse_model::AuthKind::None,
        cost: wse_model::Cost::Free,
        historical_available: true,
        realtime_available: true,
        geospatial: false,
        entities: vec!["oil".to_string()],
        priority: 30,
        enabled: true,
        collector_type: COLLECTOR_TYPE.to_string(),
        parameters,
        tier: wse_model::SourceTier::Tier2,
        measurement: wse_model::MeasurementSemantics::StableSeries,
        feeds_lenses: vec!["lens_global_events".to_string()],
        derivations: Vec::new(),
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct TimelineResponse {
    /// Older responses echo the query here.
    #[serde(default)]
    pub query: String,
    /// The live API echoes it here instead, as an object.
    #[serde(default)]
    pub query_details: Option<QueryDetails>,
    #[serde(default)]
    pub timeline: Vec<Series>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct QueryDetails {
    #[serde(default)]
    pub title: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Series {
    #[serde(default)]
    pub series: String,
    #[serde(default)]
    pub data: Vec<Point>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Point {
    /// Compact UTC timestamp. The live API returns `YYYYMMDDTHHMMSSZ`; older
    /// documentation and some responses use `YYYYMMDDHHMMSS`.
    pub date: String,
    pub value: f64,
}

/// Parse a GDELT `timelinevol` response into observations.
///
/// The entity is the query topic: the series is "how much of the world's news
/// is about this", so a sustained rise is an early signal in its own right.
pub fn parse(body: &[u8], received_at: DateTime<Utc>) -> Result<Vec<Observation>, CollectorError> {
    // GDELT answers rate limiting and bad queries with a plain-text notice and
    // an HTTP 200. If we let that reach serde it either errors obscurely or,
    // worse, could look like an empty series. Say plainly what happened.
    if !looks_like_json(body) {
        let excerpt: String = String::from_utf8_lossy(body).chars().take(160).collect();
        return Err(CollectorError::Parse(format!(
            "gdelt returned a non-JSON body (rate limited or bad query?): {excerpt}"
        )));
    }

    let response: TimelineResponse = serde_json::from_slice(body)
        .map_err(|e| CollectorError::Parse(format!("gdelt timeline: {e}")))?;

    let query = resolve_query(&response);
    let entity = EntityId::new(format!("topic_{}", wse_model::canonicalize(&query)));
    let source_id = SourceId::new(SOURCE_ID);
    let hash = wse_model::fnv1a_hex(&String::from_utf8_lossy(body));
    let mut observations = Vec::new();

    for series in &response.timeline {
        for point in &series.data {
            let Some(observed_at) = parse_gdelt_date(&point.date) else {
                continue;
            };
            let raw = RawReference {
                locator: format!("gdelt:{}:{}", series.series, point.date),
                hash: hash.clone(),
                content_type: Some("application/json".to_string()),
                bytes: Some(body.len() as u64),
            };
            let observation = Observation::new(
                source_id.clone(),
                Some(entity.clone()),
                "news_volume",
                point.value,
                "percent",
                observed_at,
                raw,
            )
            .with_received_at(received_at)
            // A GDELT bucket is keyed by its series and its time bucket; the
            // 1-day window is re-fetched each poll, so retained buckets must
            // keep one id.
            .with_record_key(format!("{}|{}", series.series, point.date))
            .with_dimension("query", query.clone())
            .with_dimension("series", series.series.clone());
            observations.push(observation);
        }
    }

    observations.sort_by_key(|o| o.observed_at);
    Ok(observations)
}

/// The tracked topic from a response, however the API chose to echo it.
fn resolve_query(response: &TimelineResponse) -> String {
    if !response.query.is_empty() {
        return response.query.clone();
    }
    if let Some(details) = &response.query_details {
        if !details.title.is_empty() {
            return details.title.clone();
        }
    }
    DEFAULT_QUERY.to_string()
}

/// GDELT timestamps are compact UTC strings, not RFC 3339.
///
/// Two shapes appear in the wild: `YYYYMMDDHHMMSS` (the documented form) and
/// `YYYYMMDDTHHMMSSZ` (what the live API returns). Both must decode, or every
/// live point would be skipped and the source would look empty.
fn parse_gdelt_date(raw: &str) -> Option<DateTime<Utc>> {
    let digits: String = raw.chars().filter(char::is_ascii_digit).collect();
    if digits.len() != 14 {
        return None;
    }
    let naive = NaiveDateTime::parse_from_str(&digits, "%Y%m%d%H%M%S").ok()?;
    Some(Utc.from_utc_datetime(&naive))
}

/// Whether a body plausibly starts a JSON document.
fn looks_like_json(body: &[u8]) -> bool {
    let first = body.iter().copied().find(|b| !b.is_ascii_whitespace());
    matches!(first, Some(b'{') | Some(b'['))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        include_bytes!("../../../tests/fixtures/gdelt_timelinevol.json").to_vec()
    }

    fn received() -> DateTime<Utc> {
        Utc.timestamp_millis_opt(1_700_020_000_000)
            .single()
            .unwrap()
    }

    #[test]
    fn parses_every_timeline_point() {
        let observations = parse(&fixture(), received()).unwrap();
        assert_eq!(observations.len(), 6);
        assert_eq!(observations[0].metric, "news_volume");
        assert_eq!(observations[0].unit, "percent");
        assert_eq!(observations[0].value, 0.412);
    }

    #[test]
    fn gdelt_dates_are_decoded_as_utc() {
        let observations = parse(&fixture(), received()).unwrap();
        let first = &observations[0];
        assert_eq!(first.observed_at.to_rfc3339(), "2023-11-15T00:00:00+00:00");
        assert_eq!(
            first.dimensions.get("query").map(String::as_str),
            Some("oil supply")
        );
    }

    #[test]
    fn a_rising_series_is_visible_in_order() {
        let observations = parse(&fixture(), received()).unwrap();
        let values: Vec<f64> = observations.iter().map(|o| o.value).collect();
        assert!(values.windows(2).all(|w| w[1] > w[0]), "{values:?}");
    }

    #[test]
    fn every_point_shares_one_series_key() {
        let observations = parse(&fixture(), received()).unwrap();
        let keys: Vec<String> = observations.iter().map(|o| o.series_key()).collect();
        assert!(keys.iter().all(|k| k == &keys[0]));
    }

    #[test]
    fn malformed_payload_is_a_parse_error() {
        assert!(matches!(
            parse(b"{", received()),
            Err(CollectorError::Parse(_))
        ));
    }

    #[test]
    fn a_plain_text_rate_limit_notice_is_a_parse_error() {
        // GDELT returns this with HTTP 200; treating it as "no data" would
        // silently hide a throttled source.
        let notice =
            b"Please limit requests to one every 5 seconds or contact kalev.leetaru5@gmail.com";
        let err = parse(notice, received()).unwrap_err();
        match err {
            CollectorError::Parse(message) => assert!(message.contains("non-JSON")),
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn short_or_bogus_dates_are_skipped() {
        assert!(parse_gdelt_date("2023").is_none());
        assert!(parse_gdelt_date("not-a-date-at-all").is_none());
    }

    #[test]
    fn the_live_api_date_format_decodes() {
        // The live API returns `YYYYMMDDTHHMMSSZ`; the documented form has no
        // separators. Both must decode or live GDELT would look empty.
        let compact = parse_gdelt_date("20231115000000").unwrap();
        let live = parse_gdelt_date("20231115T000000Z").unwrap();
        assert_eq!(compact, live);
        assert_eq!(live.to_rfc3339(), "2023-11-15T00:00:00+00:00");
    }

    #[test]
    fn a_captured_live_response_parses() {
        // A real payload captured from the live API, kept so the live date
        // format and `query_details` shape cannot silently regress.
        let body = include_bytes!("../../../tests/fixtures/gdelt_timelinevol_real.json");
        let observations = parse(body, received()).unwrap();
        assert!(!observations.is_empty(), "live payload produced no points");
        let first = &observations[0];
        assert_eq!(first.observed_at.to_rfc3339(), "2026-09-29T14:45:00+00:00");
        assert_eq!(
            first.dimensions.get("query").map(String::as_str),
            Some("climate"),
            "the topic must come from query_details when query is absent"
        );
        // A live series is one series, regardless of how the topic was echoed.
        let keys: Vec<String> = observations.iter().map(|o| o.series_key()).collect();
        assert!(keys.iter().all(|k| k == &keys[0]));
    }

    #[test]
    fn catalog_entry_polls_on_an_interval() {
        let source = source();
        assert_eq!(
            source.cadence,
            wse_model::Cadence::Interval { seconds: 900 }
        );
        assert_eq!(
            source.parameters.get("mode").map(String::as_str),
            Some("timelinevol")
        );
    }
}
