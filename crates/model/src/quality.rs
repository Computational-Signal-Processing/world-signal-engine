//! Data quality and presence semantics.
//!
//! See `docs/philosophy.md` and the "critical rule" in the project brief:
//! **the absence of data is not an event.** [`DataPresence`] exists so that the
//! pipeline can distinguish "the world was quiet" from "the collector broke".

use serde::{Deserialize, Serialize};

/// Quality assessment attached to an observation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quality {
    /// Normalized quality score in `0.0..=1.0`, where `1.0` is pristine.
    pub score: f64,
    pub flags: Vec<QualityFlag>,
}

impl Default for Quality {
    fn default() -> Self {
        Self {
            score: 1.0,
            flags: Vec::new(),
        }
    }
}

impl Quality {
    /// A clean observation with no known defects.
    pub fn pristine() -> Self {
        Self::default()
    }

    pub fn scored(score: f64) -> Self {
        Self {
            score: score.clamp(0.0, 1.0),
            flags: Vec::new(),
        }
    }

    /// Record a defect, de-duplicating repeated flags.
    pub fn with_flag(mut self, flag: QualityFlag) -> Self {
        if !self.flags.contains(&flag) {
            self.flags.push(flag);
        }
        self
    }

    pub fn is_degraded(&self) -> bool {
        !self.flags.is_empty() || self.score < 1.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum QualityFlag {
    /// Source timestamp disagrees with our clock by more than tolerance.
    ClockDrift,
    /// Data arrived later than the source's expected cadence.
    Delayed,
    /// Required fields were missing and had to be defaulted.
    MissingFields,
    /// Value fell outside the physically plausible range for the metric.
    OutOfRange,
    /// Value was derived/estimated rather than measured.
    Estimated,
    /// The collector recognised this record as a duplicate.
    Duplicate,
    /// Only part of the expected payload was retrievable.
    Partial,
    /// The collector could not assess quality.
    Unknown,
}

/// Whether a series actually produced data in a window.
///
/// This is the machine-checkable form of the critical rule. A series that
/// reports `NoData` or `SourceDown` must never be interpreted as a value of
/// zero, and must never feed anomaly detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DataPresence {
    /// At least one observation was received in the window.
    Present,
    /// The collector ran successfully but the source reported nothing.
    NoData,
    /// The collector itself failed; we have no evidence either way.
    SourceDown,
}

impl DataPresence {
    /// Whether the window carries an actual measurement.
    pub fn has_measurement(self) -> bool {
        matches!(self, DataPresence::Present)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_quality_is_pristine() {
        let q = Quality::default();
        assert_eq!(q.score, 1.0);
        assert!(!q.is_degraded());
    }

    #[test]
    fn flags_are_deduplicated_and_mark_degradation() {
        let q = Quality::pristine()
            .with_flag(QualityFlag::Delayed)
            .with_flag(QualityFlag::Delayed);
        assert_eq!(q.flags.len(), 1);
        assert!(q.is_degraded());
    }

    #[test]
    fn scores_are_clamped() {
        assert_eq!(Quality::scored(5.0).score, 1.0);
        assert_eq!(Quality::scored(-1.0).score, 0.0);
    }

    #[test]
    fn no_data_is_not_a_measurement() {
        assert!(DataPresence::Present.has_measurement());
        assert!(!DataPresence::NoData.has_measurement());
        assert!(!DataPresence::SourceDown.has_measurement());
    }
}
