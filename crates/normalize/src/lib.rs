//! # wse-normalize
//!
//! Normalization helpers shared by collectors.
//!
//! Different sources describe the same physical quantity in different ways.
//! This crate provides the small, reusable pieces — unit canonicalization,
//! plausibility ranges, and quality assessment from timestamps — so each
//! collector only has to describe *its* payload shape.
//!
//! Normalization never invents data. A missing field becomes an explicit
//! quality flag, not a default value silently passed off as a measurement.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use wse_model::{Quality, QualityFlag};

/// Canonical form of a unit string.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Unit {
    pub symbol: String,
    pub quantity: String,
    pub scale: f64,
}

/// Resolve a source-provided unit string to a canonical unit and scale.
///
/// Unknown units are passed through unchanged with scale `1.0`; silently
/// guessing would corrupt downstream comparisons.
pub fn canonical_unit(raw: &str) -> Unit {
    let key = raw.trim().to_lowercase();
    let (symbol, quantity, scale): (&str, &str, f64) = match key.as_str() {
        "c" | "celsius" | "°c" | "degc" => ("celsius", "temperature", 1.0),
        "k" | "kelvin" => ("kelvin", "temperature", 1.0),
        "f" | "fahrenheit" | "°f" => ("fahrenheit", "temperature", 1.0),
        "m" | "meter" | "metre" | "meters" => ("meter", "length", 1.0),
        "km" | "kilometer" | "kilometre" => ("kilometer", "length", 1.0),
        "mm" | "millimeter" => ("millimeter", "length", 1.0),
        "kg" | "kilogram" => ("kilogram", "mass", 1.0),
        "g" | "gram" => ("gram", "mass", 1.0),
        "s" | "sec" | "second" | "seconds" => ("second", "time", 1.0),
        "min" | "minute" | "minutes" => ("minute", "time", 1.0),
        "h" | "hr" | "hour" | "hours" => ("hour", "time", 1.0),
        "pa" | "pascal" => ("pascal", "pressure", 1.0),
        "hpa" | "hectopascal" => ("hectopascal", "pressure", 1.0),
        "bar" => ("bar", "pressure", 1.0),
        "j" | "joule" => ("joule", "energy", 1.0),
        "kwh" => ("kilowatt_hour", "energy", 1.0),
        "w" | "watt" => ("watt", "power", 1.0),
        "usd" | "$" => ("usd", "currency", 1.0),
        "eur" | "€" => ("eur", "currency", 1.0),
        "try" | "₺" => ("try", "currency", 1.0),
        "bbl" | "barrel" | "barrels" => ("barrel", "volume", 1.0),
        "teu" => ("teu", "container_volume", 1.0),
        "n" | "count" | "counts" | "mentions" => ("count", "count", 1.0),
        "m/s" => ("meter_per_second", "speed", 1.0),
        "km/h" => ("kilometer_per_hour", "speed", 1.0),
        "mm/h" => ("millimeter_per_hour", "rate", 1.0),
        "usd/bbl" => ("usd_per_barrel", "price", 1.0),
        "usd/t" => ("usd_per_tonne", "price", 1.0),
        "mag" | "magnitude" => ("magnitude", "magnitude", 1.0),
        _ => (raw.trim(), "unknown", 1.0),
    };
    Unit {
        symbol: symbol.to_string(),
        quantity: quantity.to_string(),
        scale,
    }
}

/// A plausibility range for a metric.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PlausibleRange {
    pub min: f64,
    pub max: f64,
}

impl PlausibleRange {
    pub fn new(min: f64, max: f64) -> Self {
        Self { min, max }
    }

    pub fn contains(&self, value: f64) -> bool {
        value.is_finite() && value >= self.min && value <= self.max
    }
}

/// Assessment inputs for one normalized record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QualityInputs {
    pub observed_at: DateTime<Utc>,
    pub received_at: DateTime<Utc>,
    /// How late the source is *expected* to be, e.g. an hourly feed may lag by
    /// several minutes without that being a defect.
    pub expected_lag: Duration,
    /// Beyond this disagreement we assume our clock and the source's clock
    /// differ rather than that the data is merely late.
    pub clock_drift_tolerance: Duration,
    pub missing_fields: bool,
    pub plausibility: Option<PlausibleRange>,
    pub value: f64,
    pub partial: bool,
}

impl QualityInputs {
    pub fn new(observed_at: DateTime<Utc>, received_at: DateTime<Utc>, value: f64) -> Self {
        Self {
            observed_at,
            received_at,
            expected_lag: Duration::minutes(5),
            clock_drift_tolerance: Duration::hours(1),
            missing_fields: false,
            plausibility: None,
            value,
            partial: false,
        }
    }

    pub fn with_expected_lag(mut self, lag: Duration) -> Self {
        self.expected_lag = lag;
        self
    }

    pub fn with_plausibility(mut self, range: PlausibleRange) -> Self {
        self.plausibility = Some(range);
        self
    }

    pub fn with_missing_fields(mut self, missing: bool) -> Self {
        self.missing_fields = missing;
        self
    }

    pub fn with_partial(mut self, partial: bool) -> Self {
        self.partial = partial;
        self
    }
}

/// Derive a [`Quality`] assessment from the inputs.
///
/// Each defect lowers the score; the flags record *which* defects were seen so
/// the UI can explain the number.
pub fn assess(inputs: &QualityInputs) -> Quality {
    let mut quality = Quality::pristine();
    let mut score: f64 = 1.0;

    // A source timestamp in the future, or far in the past, is a clock problem
    // rather than a data problem.
    let skew = inputs.received_at - inputs.observed_at;
    if skew < Duration::zero() || skew > inputs.clock_drift_tolerance {
        quality = quality.with_flag(QualityFlag::ClockDrift);
        score -= 0.3;
    } else if skew > inputs.expected_lag {
        quality = quality.with_flag(QualityFlag::Delayed);
        score -= 0.15;
    }

    if inputs.missing_fields {
        quality = quality.with_flag(QualityFlag::MissingFields);
        score -= 0.2;
    }

    if inputs.partial {
        quality = quality.with_flag(QualityFlag::Partial);
        score -= 0.1;
    }

    if let Some(range) = inputs.plausibility {
        if !range.contains(inputs.value) {
            quality = quality.with_flag(QualityFlag::OutOfRange);
            score -= 0.3;
        }
    }

    if !inputs.value.is_finite() {
        quality = quality.with_flag(QualityFlag::OutOfRange);
        score -= 0.5;
    }

    quality.score = score.clamp(0.0, 1.0);
    quality
}

/// Free-form dimensions extracted from a source record.
pub type Dimensions = BTreeMap<String, String>;

/// Collect non-empty dimensions, skipping blanks rather than storing empty
/// strings that would fragment series keys.
pub fn dimensions_from(pairs: &[(&str, Option<&str>)]) -> Dimensions {
    let mut out = Dimensions::new();
    for (key, value) in pairs {
        if let Some(v) = value {
            let v = v.trim();
            if !v.is_empty() {
                out.insert((*key).to_string(), v.to_string());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
    }

    #[test]
    fn known_units_resolve_to_canonical_symbols() {
        assert_eq!(canonical_unit("C").symbol, "celsius");
        assert_eq!(canonical_unit("°C").quantity, "temperature");
        assert_eq!(canonical_unit("USD").symbol, "usd");
        assert_eq!(canonical_unit("bbl").symbol, "barrel");
    }

    #[test]
    fn unknown_units_pass_through() {
        let u = canonical_unit("furlongs");
        assert_eq!(u.symbol, "furlongs");
        assert_eq!(u.quantity, "unknown");
        assert_eq!(u.scale, 1.0);
    }

    #[test]
    fn prompt_arrival_is_pristine() {
        let q = assess(&QualityInputs::new(at(0), at(10), 1.0));
        assert!(!q.is_degraded());
        assert_eq!(q.score, 1.0);
    }

    #[test]
    fn late_arrival_is_flagged_delayed() {
        let q = assess(&QualityInputs::new(at(0), at(600), 1.0));
        assert!(q.flags.contains(&QualityFlag::Delayed));
        assert!(q.score < 1.0);
    }

    #[test]
    fn future_timestamp_is_clock_drift() {
        let q = assess(&QualityInputs::new(at(1000), at(0), 1.0));
        assert!(q.flags.contains(&QualityFlag::ClockDrift));
    }

    #[test]
    fn implausible_value_is_flagged_out_of_range() {
        let q = assess(
            &QualityInputs::new(at(0), at(1), 9999.0)
                .with_plausibility(PlausibleRange::new(0.0, 100.0)),
        );
        assert!(q.flags.contains(&QualityFlag::OutOfRange));
    }

    #[test]
    fn nan_is_out_of_range() {
        let q = assess(&QualityInputs::new(at(0), at(1), f64::NAN));
        assert!(q.flags.contains(&QualityFlag::OutOfRange));
    }

    #[test]
    fn multiple_defects_accumulate() {
        let q = assess(
            &QualityInputs::new(at(0), at(10_000), 1.0)
                .with_missing_fields(true)
                .with_partial(true),
        );
        assert!(q.flags.len() >= 3);
        assert!(q.score < 0.5);
    }

    #[test]
    fn dimensions_skip_blanks() {
        let d = dimensions_from(&[
            ("station", Some("kadikoy")),
            ("empty", Some("  ")),
            ("none", None),
        ]);
        assert_eq!(d.len(), 1);
        assert_eq!(d.get("station").unwrap(), "kadikoy");
    }

    #[test]
    fn score_never_goes_negative() {
        let q = assess(
            &QualityInputs::new(at(0), at(1_000_000), 1e12)
                .with_missing_fields(true)
                .with_partial(true)
                .with_plausibility(PlausibleRange::new(0.0, 1.0)),
        );
        assert!(q.score >= 0.0);
    }
}
