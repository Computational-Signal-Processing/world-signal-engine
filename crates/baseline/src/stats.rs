//! Pure statistical primitives. No I/O, no state, fully unit-tested.

/// Arithmetic mean. Returns `0.0` for an empty slice.
pub fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / values.len() as f64
}

/// Sample standard deviation (Bessel-corrected). Returns `0.0` for fewer than
/// two values.
pub fn std_dev(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let m = mean(values);
    let ss: f64 = values.iter().map(|v| (v - m).powi(2)).sum();
    (ss / (values.len() - 1) as f64).sqrt()
}

/// Median. Returns `0.0` for an empty slice.
pub fn median(values: &[f64]) -> f64 {
    percentile(values, 0.5)
}

/// Linear-interpolated percentile using the `(n-1) * p` index convention.
///
/// `p` is clamped to `0.0..=1.0`.
pub fn percentile(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    if sorted.len() == 1 {
        return sorted[0];
    }
    let p = p.clamp(0.0, 1.0);
    let rank = p * (sorted.len() - 1) as f64;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    if lower == upper {
        sorted[lower]
    } else {
        let frac = rank - lower as f64;
        sorted[lower] * (1.0 - frac) + sorted[upper] * frac
    }
}

/// Median absolute deviation, scaled by `1.4826` so it estimates the standard
/// deviation for normally distributed data.
pub fn median_absolute_deviation(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let med = median(values);
    let deviations: Vec<f64> = values.iter().map(|v| (v - med).abs()).collect();
    1.4826 * median(&deviations)
}

/// Exponentially weighted moving average over a chronological slice.
///
/// `alpha` is clamped to `0.0..=1.0`. Returns `0.0` for an empty slice.
pub fn ewma(values: &[f64], alpha: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let alpha = alpha.clamp(0.0, 1.0);
    let mut acc = values[0];
    for v in &values[1..] {
        acc = alpha * v + (1.0 - alpha) * acc;
    }
    acc
}

/// Successive differences of a series, i.e. its discrete velocity.
pub fn first_differences(values: &[f64]) -> Vec<f64> {
    values.windows(2).map(|w| w[1] - w[0]).collect()
}

/// Standard deviation of successive differences.
///
/// Used as a simple, explainable volatility measure. Returns `0.0` when there
/// are fewer than two differences.
pub fn volatility(values: &[f64]) -> f64 {
    std_dev(&first_differences(values))
}

/// Least-squares slope of `values` against `timestamps`, expressed in
/// value-units per second. Returns `0.0` when the slope is undefined.
pub fn trend_per_second(timestamps: &[i64], values: &[f64]) -> f64 {
    let n = timestamps.len().min(values.len());
    if n < 2 {
        return 0.0;
    }
    let ts: Vec<f64> = timestamps[..n].iter().map(|t| *t as f64).collect();
    let vs: Vec<f64> = values[..n].to_vec();
    let mean_t = mean(&ts);
    let mean_v = mean(&vs);
    let mut num = 0.0;
    let mut den = 0.0;
    for (t, v) in ts.iter().zip(vs.iter()) {
        num += (t - mean_t) * (v - mean_v);
        den += (t - mean_t).powi(2);
    }
    if den.abs() < f64::EPSILON {
        0.0
    } else {
        num / den
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn mean_of_empty_is_zero() {
        assert_eq!(mean(&[]), 0.0);
    }

    #[test]
    fn mean_is_arithmetic() {
        assert!(close(mean(&[1.0, 2.0, 3.0, 4.0]), 2.5));
    }

    #[test]
    fn sample_std_dev_is_bessel_corrected() {
        // values 2,4,4,4,5,5,7,9 -> mean 5, sample std dev ~2.13809
        let v = [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
        assert!((std_dev(&v) - 2.138089935).abs() < 1e-6);
        assert_eq!(std_dev(&[1.0]), 0.0);
    }

    #[test]
    fn median_handles_odd_and_even_lengths() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), 2.5);
        assert_eq!(median(&[]), 0.0);
    }

    #[test]
    fn percentiles_are_interpolated() {
        let v = [0.0, 10.0, 20.0, 30.0, 40.0];
        assert_eq!(percentile(&v, 0.0), 0.0);
        assert_eq!(percentile(&v, 1.0), 40.0);
        assert_eq!(percentile(&v, 0.5), 20.0);
        assert_eq!(percentile(&v, 0.25), 10.0);
    }

    #[test]
    fn mad_of_constant_series_is_zero() {
        assert_eq!(median_absolute_deviation(&[7.0, 7.0, 7.0]), 0.0);
    }

    #[test]
    fn mad_scales_deviations() {
        // median 3, deviations [2,1,0,1,2] -> median 1 -> 1.4826
        let v = [1.0, 2.0, 3.0, 4.0, 5.0];
        assert!((median_absolute_deviation(&v) - 1.4826).abs() < 1e-9);
    }

    #[test]
    fn ewma_moves_towards_recent_values() {
        // alpha=0.5: 0 -> 5 -> 7.5
        let v = [0.0, 10.0, 10.0];
        assert!(close(ewma(&v, 0.5), 7.5));
        assert_eq!(ewma(&[], 0.5), 0.0);
    }

    #[test]
    fn differences_are_velocities() {
        assert_eq!(first_differences(&[1.0, 3.0, 6.0]), vec![2.0, 3.0]);
    }

    #[test]
    fn trend_detects_slope() {
        let ts = [0, 1, 2, 3, 4];
        let vs = [0.0, 2.0, 4.0, 6.0, 8.0];
        assert!(close(trend_per_second(&ts, &vs), 2.0));
    }

    #[test]
    fn trend_of_flat_series_is_zero() {
        let ts = [0, 1, 2];
        let vs = [5.0, 5.0, 5.0];
        assert_eq!(trend_per_second(&ts, &vs), 0.0);
    }

    #[test]
    fn trend_needs_two_points() {
        assert_eq!(trend_per_second(&[0], &[1.0]), 0.0);
    }

    #[test]
    fn volatility_of_constant_series_is_zero() {
        assert_eq!(volatility(&[3.0, 3.0, 3.0]), 0.0);
    }
}
