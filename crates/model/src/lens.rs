//! Lenses: filters over the shared world dataset.
//!
//! A lens never changes the data, only visibility. Multiple lenses can be
//! active at once (e.g. `WORLD + TECHNOLOGY + ENERGY + TURKEY`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ids::LensId;

/// A saved view over the world dataset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lens {
    pub id: LensId,
    pub name: String,
    /// Categories to include; empty means "all categories".
    pub categories: Vec<String>,
    /// Entity canonical names to include; empty means "all entities".
    pub entities: Vec<String>,
    /// Keyword matches applied to signal titles/summaries.
    pub keywords: Vec<String>,
    /// Bounding box as `(min_lat, min_lon, max_lat, max_lon)`.
    pub bbox: Option<(f64, f64, f64, f64)>,
    /// Relative weights applied to quality dimensions when ranking.
    pub weights: BTreeMap<String, f64>,
}

impl Lens {
    pub fn new(id: LensId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            categories: Vec::new(),
            entities: Vec::new(),
            keywords: Vec::new(),
            bbox: None,
            weights: BTreeMap::new(),
        }
    }

    /// Whether a signal (described by its category/entity/keyword/location
    /// facets) is visible through this lens.
    pub fn matches(
        &self,
        categories: &[String],
        entities: &[String],
        text: &str,
        location: Option<(f64, f64)>,
    ) -> bool {
        let category_ok = self.categories.is_empty()
            || categories.iter().any(|c| {
                self.categories
                    .iter()
                    .any(|want| want.eq_ignore_ascii_case(c))
            });

        let entity_ok = self.entities.is_empty()
            || entities.iter().any(|e| {
                let e = crate::entity::canonicalize(e);
                self.entities
                    .iter()
                    .any(|want| crate::entity::canonicalize(want) == e)
            });

        let keyword_ok = self.keywords.is_empty() || {
            let haystack = text.to_lowercase();
            self.keywords
                .iter()
                .any(|k| haystack.contains(&k.to_lowercase()))
        };

        let location_ok = match (self.bbox, location) {
            (Some((min_lat, min_lon, max_lat, max_lon)), Some((lat, lon))) => {
                lat >= min_lat && lat <= max_lat && lon >= min_lon && lon <= max_lon
            }
            // No bbox constraint, or no location to constrain: do not filter.
            _ => true,
        };

        category_ok && entity_ok && keyword_ok && location_ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_lens_matches_everything() {
        let lens = Lens::new(LensId::new("lens_global"), "GLOBAL");
        assert!(lens.matches(&["energy".into()], &["oil".into()], "anything", None));
    }

    #[test]
    fn category_filter_is_case_insensitive() {
        let mut lens = Lens::new(LensId::new("lens_energy"), "ENERGY");
        lens.categories.push("energy".into());
        assert!(lens.matches(&["Energy".into()], &[], "", None));
        assert!(!lens.matches(&["software".into()], &[], "", None));
    }

    #[test]
    fn bbox_requires_a_location_inside_it() {
        let mut lens = Lens::new(LensId::new("lens_turkey"), "TURKEY");
        lens.bbox = Some((35.8, 25.6, 42.1, 44.8));
        assert!(lens.matches(&[], &[], "", Some((41.0, 29.0))));
        assert!(!lens.matches(&[], &[], "", Some((48.8, 2.3))));
        // No location means the bbox cannot exclude it.
        assert!(lens.matches(&[], &[], "", None));
    }

    #[test]
    fn keyword_filter_matches_text() {
        let mut lens = Lens::new(LensId::new("lens_oil"), "OIL");
        lens.keywords.push("oil".into());
        assert!(lens.matches(&[], &[], "Oil price moving", None));
        assert!(!lens.matches(&[], &[], "wheat price moving", None));
    }
}
