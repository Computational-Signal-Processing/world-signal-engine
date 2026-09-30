//! Engine metrics.
//!
//! The system observes itself. These counters are exposed by `GET /metrics`
//! and are the basis for the "is a source broken?" question, which must never
//! be confused with "is the world quiet?".

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Metrics {
    pub sources_registered: u64,
    pub collector_success_total: u64,
    pub collector_failure_total: u64,
    pub collector_latency_ms: Option<u64>,
    pub observations_total: u64,
    pub observations_duplicate_total: u64,
    /// Observations that were new in the most recent ingest cycle.
    pub observations_new_last_cycle: usize,
    pub anomalies_total: u64,
    pub events_total: u64,
    pub signals_total: u64,
    /// Counts per signal type, keyed by the type name.
    pub signal_types_total: BTreeMap<String, u64>,
}

impl Metrics {
    /// A flat, text exposition suitable for `/metrics`.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "wse_sources_registered {}\n",
            self.sources_registered
        ));
        out.push_str(&format!(
            "wse_collector_success_total {}\n",
            self.collector_success_total
        ));
        out.push_str(&format!(
            "wse_collector_failure_total {}\n",
            self.collector_failure_total
        ));
        if let Some(latency) = self.collector_latency_ms {
            out.push_str(&format!("wse_collector_latency_ms {latency}\n"));
        }
        out.push_str(&format!(
            "wse_observations_total {}\n",
            self.observations_total
        ));
        out.push_str(&format!(
            "wse_observations_duplicate_total {}\n",
            self.observations_duplicate_total
        ));
        out.push_str(&format!("wse_anomalies_total {}\n", self.anomalies_total));
        out.push_str(&format!("wse_events_total {}\n", self.events_total));
        out.push_str(&format!("wse_signals_total {}\n", self.signals_total));
        for (ty, count) in &self.signal_types_total {
            out.push_str(&format!(
                "wse_signal_types_total{{type=\"{ty}\"}} {count}\n"
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_contains_the_core_counters() {
        let mut m = Metrics {
            observations_total: 5,
            signals_total: 2,
            ..Default::default()
        };
        m.signal_types_total.insert("ANOMALY".into(), 2);
        let text = m.render();
        assert!(text.contains("wse_observations_total 5"));
        assert!(text.contains("wse_signals_total 2"));
        assert!(text.contains("wse_signal_types_total{type=\"ANOMALY\"} 2"));
    }

    #[test]
    fn latency_is_optional() {
        let m = Metrics::default();
        assert!(!m.render().contains("latency"));
    }
}
