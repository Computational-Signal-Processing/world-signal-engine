//! CoinGecko — cryptocurrency spot prices for a fixed coin set.
//!
//! Crypto is the one 24/7, globally-priced market with a free, keyless public
//! API. It is a real market measurement (a spot price), and it is independent of
//! the ECB reference rates: a crypto move and an FX move are different facts, so
//! the MARKETS and FINANCE lenses have independent material.
//!
//! ## Why a fixed coin set, not a ranking
//!
//! The measurement is per coin. The coin list is fixed in code (below), so each
//! coin is its own stable series. A "top coins" query would churn membership and
//! make the aggregate move with the ranking rather than with the market — the
//! same trap `github_repo_universe` documents.
//!
//! ## Why spot price and not market cap
//!
//! Spot price is the raw, directly comparable measurement; market cap mixes
//! price with circulating supply and is derived. The raw price is what a
//! baseline can honestly form against.
//!
//! API docs: <https://docs.coingecko.com/>
//!
//! Authentication: none for the public endpoint (rate-limited). License:
//! CoinGecko data is free to use with attribution.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use wse_collector::CollectorError;
use wse_model::{EntityId, Observation, RawReference, Source, SourceId};

pub const SOURCE_ID: &str = "coingecko_market";
pub const COLLECTOR_TYPE: &str = "coingecko_market";

/// The public spot-price endpoint.
pub const API_ENDPOINT: &str = "https://api.coingecko.com/api/v3/simple/price";

/// A coin in the fixed measurement set: `(id, display name)`.
pub const COINS: &[(&str, &str)] = &[("bitcoin", "Bitcoin"), ("ethereum", "Ethereum")];

/// Build the request URL for the fixed coin set.
pub fn price_url() -> String {
    let ids: Vec<&str> = COINS.iter().map(|c| c.0).collect();
    format!("{API_ENDPOINT}?ids={}&vs_currencies=usd", ids.join(","))
}

/// The catalog entry.
pub fn source() -> Source {
    let mut parameters = std::collections::BTreeMap::new();
    parameters.insert("coins".to_string(), COINS.len().to_string());
    parameters.insert("quote".to_string(), "usd".to_string());
    Source {
        id: SourceId::new(SOURCE_ID),
        name: "CoinGecko Crypto Prices".to_string(),
        provider: "CoinGecko".to_string(),
        category: "markets".to_string(),
        subcategory: Some("crypto_spot".to_string()),
        endpoint: API_ENDPOINT.to_string(),
        protocol: wse_model::Protocol::Https,
        format: wse_model::DataFormat::Json,
        cadence: wse_model::Cadence::Interval { seconds: 900 },
        timezone: Some("UTC".to_string()),
        license: Some("CoinGecko data; free to use with attribution".to_string()),
        authentication: wse_model::AuthKind::None,
        cost: wse_model::Cost::Free,
        historical_available: false,
        realtime_available: true,
        geospatial: false,
        entities: vec!["crypto".to_string()],
        priority: 43,
        enabled: true,
        collector_type: COLLECTOR_TYPE.to_string(),
        parameters,
        tier: wse_model::SourceTier::Tier2,
        measurement: wse_model::MeasurementSemantics::StableSeries,
        feeds_lenses: vec!["lens_finance".to_string()],
        derivations: Vec::new(),
    }
}

/// The response is `{ "<coin>": { "usd": <price> } }`.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Quote {
    #[serde(default)]
    pub usd: f64,
}

/// Parse a spot-price response into one observation per coin.
///
/// `observed_at` is the collection time: the endpoint reports "now", with no
/// source-side timestamp, so the honest observation time is when we looked.
pub fn parse(body: &[u8], observed_at: DateTime<Utc>) -> Result<Vec<Observation>, CollectorError> {
    let quotes: std::collections::BTreeMap<String, Quote> = serde_json::from_slice(body)
        .map_err(|e| CollectorError::Parse(format!("coingecko: {e}")))?;

    let source_id = SourceId::new(SOURCE_ID);
    let entity = EntityId::new("crypto");
    let hash = wse_model::fnv1a_hex(&String::from_utf8_lossy(body));
    let bytes = body.len() as u64;
    let mut observations = Vec::new();

    for (id, name) in COINS {
        let Some(quote) = quotes.get(*id) else {
            // A coin missing from the response is not zero: it is unknown.
            continue;
        };
        let raw = RawReference {
            locator: format!("{API_ENDPOINT}?ids={id}&vs_currencies=usd"),
            hash: hash.clone(),
            content_type: Some("application/json".to_string()),
            bytes: Some(bytes),
        };
        observations.push(
            Observation::new(
                source_id.clone(),
                Some(entity.clone()),
                "spot_price",
                quote.usd,
                "usd",
                observed_at,
                raw,
            )
            .with_received_at(observed_at)
            // The coin is the record identity and its own series.
            .with_record_key(*id)
            .with_dimension("coin", *id)
            .with_attribute("coin", *name),
        );
    }

    if observations.is_empty() {
        return Err(CollectorError::Parse(
            "coingecko: no requested coins in the response".to_string(),
        ));
    }
    Ok(observations)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn received() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-01T11:05:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn fixture() -> Vec<u8> {
        include_bytes!("../../../tests/fixtures/coingecko_price.json").to_vec()
    }

    #[test]
    fn parses_one_observation_per_coin() {
        let observations = parse(&fixture(), received()).unwrap();
        assert_eq!(observations.len(), COINS.len());
        assert!(observations.iter().all(|o| o.metric == "spot_price"));
        assert!(observations.iter().all(|o| o.unit == "usd"));
    }

    #[test]
    fn each_coin_is_its_own_series() {
        let observations = parse(&fixture(), received()).unwrap();
        let keys: std::collections::HashSet<String> =
            observations.iter().map(|o| o.series_key()).collect();
        assert_eq!(keys.len(), observations.len());
        assert!(keys.iter().any(|k| k.contains("coin=bitcoin")));
    }

    #[test]
    fn a_missing_coin_is_skipped_not_zeroed() {
        let body = br#"{"bitcoin":{"usd":83810}}"#;
        let observations = parse(body, received()).unwrap();
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].value, 83810.0);
    }

    #[test]
    fn a_response_with_no_requested_coins_is_a_parse_error() {
        let body = br#"{"dogecoin":{"usd":1}}"#;
        assert!(matches!(
            parse(body, received()),
            Err(CollectorError::Parse(_))
        ));
    }

    #[test]
    fn malformed_payload_is_a_parse_error() {
        assert!(matches!(
            parse(b"nope", received()),
            Err(CollectorError::Parse(_))
        ));
    }

    #[test]
    fn catalog_is_markets() {
        let source = source();
        assert_eq!(source.category, "markets");
        assert!(source.feeds_lenses.contains(&"lens_finance".to_string()));
    }
}
