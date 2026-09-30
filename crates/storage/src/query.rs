//! Query types shared by all storage backends.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// An inclusive-start, exclusive-end time range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeRange {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
}

impl TimeRange {
    pub fn new(from: DateTime<Utc>, to: DateTime<Utc>) -> Self {
        Self { from, to }
    }

    /// The last `seconds` up to `now`.
    pub fn last(now: DateTime<Utc>, seconds: i64) -> Self {
        Self {
            from: now - chrono::Duration::seconds(seconds),
            to: now,
        }
    }

    pub fn contains(&self, at: DateTime<Utc>) -> bool {
        at >= self.from && at < self.to
    }
}

/// A simple pagination envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: usize,
    pub limit: usize,
    pub offset: usize,
}

impl<T> Page<T> {
    pub fn new(items: Vec<T>, total: usize, limit: usize, offset: usize) -> Self {
        Self {
            items,
            total,
            limit,
            offset,
        }
    }

    pub fn map<U>(self, f: impl FnMut(T) -> U) -> Page<U> {
        Page {
            items: self.items.into_iter().map(f).collect(),
            total: self.total,
            limit: self.limit,
            offset: self.offset,
        }
    }
}

/// Filters for observation queries.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ObservationQuery {
    pub source_id: Option<String>,
    pub entity_id: Option<String>,
    pub metric: Option<String>,
    pub series_key: Option<String>,
    pub range: Option<TimeRange>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    /// Return newest first rather than oldest first.
    pub newest_first: bool,
}

impl ObservationQuery {
    pub fn for_series(series_key: impl Into<String>) -> Self {
        Self {
            series_key: Some(series_key.into()),
            ..Self::default()
        }
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn newest_first(mut self) -> Self {
        self.newest_first = true;
        self
    }

    pub fn in_range(mut self, range: TimeRange) -> Self {
        self.range = Some(range);
        self
    }
}

/// Filters for signal queries.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SignalQuery {
    pub category: Option<String>,
    pub entity_id: Option<String>,
    pub signal_type: Option<wse_model::SignalType>,
    pub lens_id: Option<String>,
    pub active_only: bool,
    pub range: Option<TimeRange>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

impl SignalQuery {
    pub fn active() -> Self {
        Self {
            active_only: true,
            ..Self::default()
        }
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn with_type(mut self, ty: wse_model::SignalType) -> Self {
        self.signal_type = Some(ty);
        self
    }

    pub fn with_category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_range_contains_is_half_open() {
        let from = DateTime::from_timestamp(100, 0).unwrap();
        let to = DateTime::from_timestamp(200, 0).unwrap();
        let r = TimeRange::new(from, to);
        assert!(r.contains(from));
        assert!(r.contains(DateTime::from_timestamp(150, 0).unwrap()));
        assert!(!r.contains(to));
    }

    #[test]
    fn page_maps_items_preserving_metadata() {
        let p = Page::new(vec![1, 2, 3], 10, 3, 0);
        let mapped = p.map(|i| i * 2);
        assert_eq!(mapped.items, vec![2, 4, 6]);
        assert_eq!(mapped.total, 10);
        assert_eq!(mapped.limit, 3);
    }
}
