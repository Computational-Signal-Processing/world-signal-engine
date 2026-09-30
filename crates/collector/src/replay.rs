//! Replaying a captured stream.
//!
//! The brief requires two run modes, `LIVE` and `REPLAY`, and is explicit about
//! why: without replaying *real* historical data there is no way to measure
//! false positives, false negatives or detection latency. The synthetic world
//! proves the pipeline is wired correctly; a captured stream is what lets the
//! detector be scored against something that actually happened.
//!
//! A stream is newline-delimited JSON: one `header` record, then one
//! `observation` record per line, ordered by `observed_at`. The format is
//! deliberately plain — `jq`, `grep` and `wc -l` all work on it, and an
//! observation can be read back without the engine.
//!
//! Replay is grouped by `received_at`: every observation that arrived together
//! is fed to the pipeline in the same cycle. That is what makes the replayed
//! stream behave like the live one — detection sees the same batches, in the
//! same order, that it saw when the data was first collected.

use std::collections::VecDeque;
use std::io::{BufRead, Write};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use wse_model::{Observation, Source, SourceId};

use crate::{CollectionMode, CollectionResult, Collector, CollectorError, Schedule};

/// The `format` string every stream file carries.
pub const STREAM_FORMAT: &str = "wse.stream";
/// Bumped when the on-disk shape changes in a way a reader must notice.
pub const STREAM_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum ReplayError {
    #[error("stream I/O failure: {0}")]
    Io(#[from] std::io::Error),
    #[error("stream is not valid JSON on line {line}: {source}")]
    Json {
        line: usize,
        source: serde_json::Error,
    },
    #[error("stream has no header record")]
    MissingHeader,
    #[error("unexpected stream format {found:?}, expected {expected:?}")]
    WrongFormat { found: String, expected: String },
    #[error("unsupported stream version {found}, this build reads {supported}")]
    UnsupportedVersion { found: u32, supported: u32 },
}

/// The first line of a stream: enough to identify it and rebuild the catalog.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamHeader {
    pub format: String,
    pub version: u32,
    pub exported_at: DateTime<Utc>,
    pub observation_count: u64,
    /// The catalog entries the observations came from, so a replay can register
    /// sources (and therefore categories) without contacting anything.
    #[serde(default)]
    pub sources: Vec<Source>,
}

impl StreamHeader {
    pub fn new(observation_count: u64, sources: Vec<Source>) -> Self {
        Self {
            format: STREAM_FORMAT.to_string(),
            version: STREAM_FORMAT_VERSION,
            exported_at: Utc::now(),
            observation_count,
            sources,
        }
    }
}

/// One line of a stream file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StreamRecord {
    Header(Box<StreamHeader>),
    Observation(Box<Observation>),
}

/// A parsed stream: its header and its observations, oldest first.
#[derive(Debug, Clone, Default)]
pub struct Stream {
    pub header: Option<StreamHeader>,
    pub observations: Vec<Observation>,
}

impl Stream {
    /// Distinct sources referenced by the observations.
    pub fn source_ids(&self) -> Vec<SourceId> {
        let mut ids: Vec<SourceId> = self
            .observations
            .iter()
            .map(|o| o.source_id.clone())
            .collect();
        ids.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        ids.dedup();
        ids
    }

    /// The time span the stream covers, per the sources' own clocks.
    pub fn observed_span(&self) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
        let first = self.observations.iter().map(|o| o.observed_at).min()?;
        let last = self.observations.iter().map(|o| o.observed_at).max()?;
        Some((first, last))
    }

    /// Group into cycles by `received_at`, preserving arrival order.
    ///
    /// Observations that arrived together are replayed together. Ties are
    /// broken by `observed_at` and then by id so the grouping is deterministic
    /// even if the file was written in a different order.
    pub fn cycles(&self) -> Vec<Vec<Observation>> {
        let mut sorted: Vec<Observation> = self.observations.clone();
        sorted.sort_by(|a, b| {
            a.received_at
                .cmp(&b.received_at)
                .then_with(|| a.observed_at.cmp(&b.observed_at))
                .then_with(|| a.id.as_str().cmp(b.id.as_str()))
        });

        let mut cycles: Vec<Vec<Observation>> = Vec::new();
        for observation in sorted {
            match cycles.last_mut() {
                Some(batch) if batch[0].received_at == observation.received_at => {
                    batch.push(observation)
                }
                _ => cycles.push(vec![observation]),
            }
        }
        cycles
    }
}

/// Write a stream: a header line, then one observation per line.
pub fn write_stream<W: Write>(
    writer: &mut W,
    header: &StreamHeader,
    observations: &[Observation],
) -> Result<(), ReplayError> {
    serde_json::to_writer(
        &mut *writer,
        &StreamRecord::Header(Box::new(header.clone())),
    )
    .map_err(|source| ReplayError::Json { line: 1, source })?;
    writer.write_all(b"\n")?;
    for observation in observations {
        serde_json::to_writer(
            &mut *writer,
            &StreamRecord::Observation(Box::new(observation.clone())),
        )
        .map_err(|source| ReplayError::Json { line: 0, source })?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

/// Read a stream, validating the header and sorting observations oldest first.
pub fn read_stream<R: BufRead>(reader: R) -> Result<Stream, ReplayError> {
    let mut stream = Stream::default();
    for (index, line) in reader.lines().enumerate() {
        let line = line?;
        let line_number = index + 1;
        if line.trim().is_empty() {
            continue;
        }
        let record: StreamRecord =
            serde_json::from_str(&line).map_err(|source| ReplayError::Json {
                line: line_number,
                source,
            })?;
        match record {
            StreamRecord::Header(header) => {
                if header.format != STREAM_FORMAT {
                    return Err(ReplayError::WrongFormat {
                        found: header.format.clone(),
                        expected: STREAM_FORMAT.to_string(),
                    });
                }
                if header.version > STREAM_FORMAT_VERSION {
                    return Err(ReplayError::UnsupportedVersion {
                        found: header.version,
                        supported: STREAM_FORMAT_VERSION,
                    });
                }
                stream.header = Some(*header);
            }
            StreamRecord::Observation(observation) => stream.observations.push(*observation),
        }
    }
    if stream.header.is_none() {
        return Err(ReplayError::MissingHeader);
    }
    stream.observations.sort_by_key(|o| o.observed_at);
    Ok(stream)
}

/// A [`Collector`] that replays a captured stream, one arrival batch at a time.
///
/// It reports [`CollectionMode::Replay`], so downstream code can tell a
/// replayed run from a live one, and it never touches the network.
#[derive(Debug)]
pub struct ReplayCollector {
    source_id: SourceId,
    cycles: std::sync::Mutex<VecDeque<Vec<Observation>>>,
    total_cycles: usize,
}

impl ReplayCollector {
    pub fn from_stream(stream: &Stream) -> Result<Self, CollectorError> {
        if stream.observations.is_empty() {
            return Err(CollectorError::Configuration(
                "stream contains no observations".to_string(),
            ));
        }
        let cycles = stream.cycles();
        let source_id = stream.observations[0].source_id.clone();
        let total_cycles = cycles.len();
        Ok(Self {
            source_id,
            cycles: std::sync::Mutex::new(cycles.into()),
            total_cycles,
        })
    }

    /// Number of arrival batches not yet replayed.
    pub fn cycles_remaining(&self) -> usize {
        self.cycles.lock().expect("replay lock").len()
    }

    /// Total number of arrival batches in the stream.
    pub fn total_cycles(&self) -> usize {
        self.total_cycles
    }

    pub fn is_exhausted(&self) -> bool {
        self.cycles_remaining() == 0
    }
}

#[async_trait]
impl Collector for ReplayCollector {
    fn source_id(&self) -> SourceId {
        self.source_id.clone()
    }

    fn schedule(&self) -> Schedule {
        Schedule::Manual
    }

    fn mode(&self) -> CollectionMode {
        CollectionMode::Replay
    }

    async fn collect(&self) -> Result<CollectionResult, CollectorError> {
        let batch = self
            .cycles
            .lock()
            .map_err(|e| CollectorError::Other(format!("replay lock poisoned: {e}")))?
            .pop_front();

        let Some(observations) = batch else {
            // Exhausted is an empty successful run, not a failure: the driver
            // distinguishes the two with `is_exhausted`.
            return Ok(CollectionResult::new(self.source_id.clone()));
        };

        let received = observations.len() as u64;
        let mut result = CollectionResult::new(self.source_id.clone());
        result.raw_payloads = observations
            .iter()
            .map(|o| crate::RawPayload {
                reference: o.raw.clone(),
                // The raw bytes are not in the stream; the reference is kept so
                // the drill-down still resolves to the original locator.
                body: replay_raw_body(&o.raw.locator).into_bytes(),
            })
            .collect();
        result.observations = observations;
        result.records_received = received;
        result.records_changed = received;
        result.started_at = Some(Utc::now());
        result.finished_at = Some(Utc::now());
        Ok(result)
    }
}

/// A placeholder body for replayed observations.
///
/// The stream carries references, not payloads, so the retained bytes describe
/// the reference rather than pretending to be the original response. The
/// locator is preserved, which is what the drill-down needs.
fn replay_raw_body(locator: &str) -> String {
    format!("{{\"replayed\":true,\"locator\":{locator:?}}}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use std::io::Cursor;
    use wse_model::RawReference;

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
    }

    fn obs(source: &str, value: f64, observed: i64, received: i64) -> Observation {
        Observation::new(
            SourceId::new(source),
            None,
            "metric",
            value,
            "unit",
            at(observed),
            RawReference::new(
                format!("https://example.test/{observed}"),
                format!("h{observed}"),
            ),
        )
        .with_received_at(at(received))
    }

    #[test]
    fn a_stream_round_trips_through_json_lines() {
        let observations = vec![obs("src_a", 1.0, 0, 0), obs("src_a", 2.0, 3600, 3600)];
        let header = StreamHeader::new(observations.len() as u64, Vec::new());

        let mut buffer = Vec::new();
        write_stream(&mut buffer, &header, &observations).unwrap();

        let text = String::from_utf8(buffer.clone()).unwrap();
        assert_eq!(text.lines().count(), 3, "one header plus two observations");

        let read = read_stream(Cursor::new(buffer)).unwrap();
        assert_eq!(read.observations, observations);
        assert_eq!(read.header.unwrap().observation_count, 2);
    }

    #[test]
    fn observations_sharing_a_receipt_time_form_one_cycle() {
        let stream = Stream {
            header: Some(StreamHeader::new(3, Vec::new())),
            observations: vec![
                obs("src_a", 1.0, 0, 0),
                obs("src_b", 2.0, 0, 0),
                obs("src_a", 3.0, 3600, 3600),
            ],
        };
        let cycles = stream.cycles();
        assert_eq!(cycles.len(), 2);
        assert_eq!(cycles[0].len(), 2);
        assert_eq!(cycles[1].len(), 1);
    }

    #[test]
    fn cycle_grouping_is_independent_of_file_order() {
        let mut stream = Stream {
            header: Some(StreamHeader::new(3, Vec::new())),
            observations: vec![
                obs("src_a", 3.0, 3600, 3600),
                obs("src_a", 1.0, 0, 0),
                obs("src_b", 2.0, 0, 0),
            ],
        };
        let forward = stream.cycles();
        stream.observations.reverse();
        let backward = stream.cycles();
        assert_eq!(forward.len(), backward.len());
        assert_eq!(forward[0].len(), backward[0].len());
        assert_eq!(forward[0][0].value, backward[0][0].value);
    }

    #[tokio::test]
    async fn replay_collector_emits_batches_then_reports_exhaustion() {
        let stream = Stream {
            header: Some(StreamHeader::new(3, Vec::new())),
            observations: vec![
                obs("src_a", 1.0, 0, 0),
                obs("src_b", 2.0, 0, 0),
                obs("src_a", 3.0, 3600, 3600),
            ],
        };
        let collector = ReplayCollector::from_stream(&stream).unwrap();
        assert_eq!(collector.total_cycles(), 2);
        assert_eq!(collector.mode(), CollectionMode::Replay);

        let first = collector.collect().await.unwrap();
        assert_eq!(first.observations.len(), 2);
        assert!(!collector.is_exhausted());

        let second = collector.collect().await.unwrap();
        assert_eq!(second.observations.len(), 1);
        assert!(collector.is_exhausted());

        // Exhaustion is an empty success, not a failure: "no more data" must
        // never be confused with "the source broke".
        let third = collector.collect().await.unwrap();
        assert!(third.observations.is_empty());
        assert!(!third.is_failure());
    }

    #[test]
    fn an_empty_stream_is_rejected_rather_than_replayed_as_nothing() {
        let stream = Stream {
            header: Some(StreamHeader::new(0, Vec::new())),
            observations: Vec::new(),
        };
        assert!(matches!(
            ReplayCollector::from_stream(&stream),
            Err(CollectorError::Configuration(_))
        ));
    }

    #[test]
    fn a_wrong_format_header_is_rejected() {
        let line = r#"{"kind":"header","format":"something.else","version":1,"exported_at":"2026-01-01T00:00:00Z","observation_count":0}"#;
        let err = read_stream(Cursor::new(line.as_bytes())).unwrap_err();
        assert!(matches!(err, ReplayError::WrongFormat { .. }));
    }

    #[test]
    fn a_future_version_is_rejected_rather_than_misread() {
        let line = format!(
            r#"{{"kind":"header","format":"{STREAM_FORMAT}","version":{},"exported_at":"2026-01-01T00:00:00Z","observation_count":0}}"#,
            STREAM_FORMAT_VERSION + 1
        );
        let err = read_stream(Cursor::new(line.as_bytes())).unwrap_err();
        assert!(matches!(err, ReplayError::UnsupportedVersion { .. }));
    }

    #[test]
    fn a_stream_without_a_header_is_rejected() {
        let err = read_stream(Cursor::new(b"".as_slice())).unwrap_err();
        assert!(matches!(err, ReplayError::MissingHeader));
    }

    #[test]
    fn span_covers_the_sources_own_clock() {
        let stream = Stream {
            header: Some(StreamHeader::new(2, Vec::new())),
            observations: vec![obs("src_a", 1.0, 0, 500), obs("src_a", 2.0, 3600, 3600)],
        };
        let (from, to) = stream.observed_span().unwrap();
        assert_eq!(from, at(0));
        assert_eq!(to, at(3600));
        assert_eq!(to - from, Duration::hours(1));
    }
}
