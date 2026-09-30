//! Geospatial helpers.

use serde::{Deserialize, Serialize};

/// A coarse geographic anchor for an event or signal.
///
/// Observations carry raw `latitude`/`longitude`; events and signals carry a
/// single representative [`Location`] so the map never becomes a pile of raw
/// points.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Location {
    pub latitude: f64,
    pub longitude: f64,
    pub name: Option<String>,
    /// Uncertainty radius in kilometres, when known.
    pub radius_km: Option<f64>,
}

impl Location {
    pub fn new(latitude: f64, longitude: f64) -> Self {
        Self {
            latitude,
            longitude,
            name: None,
            radius_km: None,
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn with_radius_km(mut self, radius_km: f64) -> Self {
        self.radius_km = Some(radius_km);
        self
    }
}

/// Great-circle distance between two points in kilometres (haversine).
pub fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const EARTH_RADIUS_KM: f64 = 6371.0088;
    let (phi1, phi2) = (lat1.to_radians(), lat2.to_radians());
    let dphi = (lat2 - lat1).to_radians();
    let dlambda = (lon2 - lon1).to_radians();
    let a = (dphi / 2.0).sin().powi(2) + phi1.cos() * phi2.cos() * (dlambda / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_KM * a.sqrt().asin()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_distance_for_identical_points() {
        assert_eq!(haversine_km(41.0, 29.0, 41.0, 29.0), 0.0);
    }

    #[test]
    fn istanbul_ankara_is_roughly_350km() {
        // Istanbul (41.0082, 28.9784) -> Ankara (39.9334, 32.8597)
        let d = haversine_km(41.0082, 28.9784, 39.9334, 32.8597);
        assert!((330.0..380.0).contains(&d), "unexpected distance {d}");
    }
}
