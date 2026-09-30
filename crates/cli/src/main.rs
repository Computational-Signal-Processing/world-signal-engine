//! `wse` — the World Signal Engine command-line driver.
//!
//! Three subcommands, matching the three things the MVP must be able to do:
//!
//! * `demo`  — run the synthetic acceptance world and print the signals.
//! * `serve` — run the API and web UI, optionally driving the synthetic world.
//! * `replay`— feed a synthetic world through the pipeline as if it were live.

use anyhow::Result;
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand};
use wse_api::AppState;
use wse_collector::synthetic::{SyntheticCollector, SyntheticWorld};
use wse_detection::DetectorConfig;
use wse_engine::{ConvergenceConfig, Engine, EngineConfig};
use wse_model::{Source, SourceId};
use wse_signals::event::EventConfig;
use wse_signals::SignalConfig;
use wse_storage::{ObservationStore, SignalStore, SourceStore};

#[derive(Parser, Debug)]
#[command(
    name = "wse",
    about = "World Signal Engine: observe, detect change, surface signals."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run the synthetic acceptance world and print the signals it produces.
    Demo {
        /// Number of observations to generate per stream.
        #[arg(long, default_value_t = 205)]
        steps: usize,
        /// Emit the full signal objects as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Feed a synthetic world through the pipeline, as if it were live.
    Replay {
        #[arg(long, default_value_t = 205)]
        steps: usize,
        /// Print a line per produced signal.
        #[arg(long)]
        verbose: bool,
    },
    /// Serve the REST API and the web UI.
    Serve {
        #[arg(long, default_value_t = 8080)]
        port: u16,
        /// Drive the synthetic world in the background while serving.
        #[arg(long)]
        synthetic: bool,
        /// Keep producing synthetic observations forever (live demo).
        ///
        /// Requires enough cycles for the detector to warm up; until then the
        /// feed is legitimately empty.
        #[arg(long)]
        live: bool,
        /// Seconds between synthetic collection cycles.
        #[arg(long, default_value_t = 1)]
        interval_seconds: u64,
        /// Also run the real source collectors once at startup.
        ///
        /// This is what makes the served instance show the real world: the
        /// API and UI are then backed by live USGS/NASA/GDELT/HN/GitHub data,
        /// with the synthetic world (if enabled) running alongside it.
        #[arg(long)]
        collect: bool,
    },
    /// Run the real source collectors once against the live APIs.
    Collect {
        /// Restrict to one source id from the catalog (repeatable).
        #[arg(long = "source")]
        sources: Vec<String>,
        /// Show the observations that were produced.
        #[arg(long)]
        verbose: bool,
        /// Print the collected observations as JSON.
        #[arg(long)]
        json: bool,
        /// Write the run to a stream file, so it can be replayed later.
        #[arg(long)]
        out: Option<std::path::PathBuf>,
    },
    /// Replay a captured stream through the pipeline, as if it were live.
    ///
    /// With `--file` the stream is replayed offline and deterministically; with
    /// `--steps` the synthetic acceptance world is used instead.
    ReplayStream {
        /// The stream file to replay, as written by `collect --out`.
        #[arg(long)]
        file: std::path::PathBuf,
        /// Print a line per produced signal.
        #[arg(long)]
        verbose: bool,
        /// Print the resulting signals as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Score the detector against a captured stream.
    Backtest {
        /// The stream file to replay, as written by `collect --out`.
        #[arg(long)]
        file: std::path::PathBuf,
        /// Ground-truth labels, as a JSON array of labeled events.
        ///
        /// Without this the report still shows latency and persistence, but
        /// precision and recall are withheld rather than guessed.
        #[arg(long)]
        labels: Option<std::path::PathBuf>,
        /// Print the full report as JSON.
        #[arg(long)]
        json: bool,
    },
    /// List the source catalog.
    Sources,
}

/// A permissive engine configuration for synthetic data.
fn synthetic_engine() -> Engine {
    Engine::new(EngineConfig {
        detector: DetectorConfig::synthetic(),
        event: EventConfig::default(),
        signal: SignalConfig {
            now_window_seconds: i64::MAX,
            ..SignalConfig::default()
        },
        convergence: ConvergenceConfig::related(),
    })
}

fn origin() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).unwrap()
}

fn register_synthetic_sources(engine: &mut Engine, world: &SyntheticWorld) -> Result<()> {
    for stream in &world.streams {
        engine.register_source(
            Source::new(
                stream.stream.source_id.clone(),
                format!("Synthetic {}", stream.stream.series_name),
                "synthetic",
            )
            .with_category("synthetic"),
        )?;
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    match cli.command {
        Command::Demo { steps, json } => demo(steps, json).await,
        Command::Replay { steps, verbose } => replay(steps, verbose).await,
        Command::Serve {
            port,
            synthetic,
            live,
            interval_seconds,
            collect,
        } => serve(port, synthetic, live, interval_seconds, collect).await,
        Command::Collect {
            sources,
            verbose,
            json,
            out,
        } => collect(sources, verbose, json, out).await,
        Command::ReplayStream {
            file,
            verbose,
            json,
        } => replay_stream(file, verbose, json).await,
        Command::Backtest { file, labels, json } => backtest(file, labels, json).await,
        Command::Sources => list_sources(),
    }
}

/// Print the source catalog exactly as the collectors see it.
fn list_sources() -> Result<()> {
    println!(
        "{:<22} {:<14} {:<8} {:<14} {:<10} NAME",
        "ID", "CATEGORY", "PROTO", "CADENCE", "AUTH"
    );
    for source in wse_sources::catalog() {
        let cadence = match source.cadence {
            wse_model::Cadence::Event => "event".to_string(),
            wse_model::Cadence::Interval { seconds } => format!("{seconds}s"),
            wse_model::Cadence::Daily { hour_utc } => format!("daily@{hour_utc}Z"),
            wse_model::Cadence::Irregular => "irregular".to_string(),
        };
        println!(
            "{:<22} {:<14} {:<8} {:<14} {:<10} {}",
            source.id.as_str(),
            source.category,
            format!("{:?}", source.protocol).to_lowercase(),
            cadence,
            format!("{:?}", source.authentication).to_lowercase(),
            source.name,
        );
    }
    Ok(())
}

/// Run the real collectors once, end to end, against the live APIs.
///
/// This is the "is the pipeline real?" command: it fetches, normalizes, stores,
/// baselines, detects and reports, for every source that answers.
async fn collect(
    only: Vec<String>,
    verbose: bool,
    json: bool,
    out: Option<std::path::PathBuf>,
) -> Result<()> {
    let mut engine = Engine::new(EngineConfig::default());

    let catalog: Vec<_> = if only.is_empty() {
        wse_sources::catalog()
    } else {
        only.iter()
            .filter_map(|id| match wse_sources::source_by_id(id) {
                Some(source) => Some(source),
                None => {
                    eprintln!("unknown source id: {id}");
                    None
                }
            })
            .collect()
    };

    if catalog.is_empty() {
        anyhow::bail!("no sources selected");
    }
    for source in &catalog {
        engine.register_source(source.clone())?;
    }

    let collectors: Vec<Box<dyn wse_collector::Collector>> = wse_sources::live_collectors()
        .into_iter()
        .filter(|c| catalog.iter().any(|s| s.id == c.source_id()))
        .collect();

    println!("Collecting from {} source(s)...", collectors.len());
    for collector in &collectors {
        let source_id = collector.source_id();
        let outcome = engine.run_collector(collector.as_ref()).await;
        let health = engine.source_health(&source_id);
        let status = health
            .as_ref()
            .map(|h| format!("{:?}", h.status).to_lowercase())
            .unwrap_or_else(|| "unknown".to_string());

        if outcome.source_failed {
            // Absence of data is not an event: report the failure and move on.
            println!(
                "  {:<22} FAILED  (source health: {status}) — no observations recorded",
                source_id.as_str()
            );
            continue;
        }

        println!(
            "  {:<22} ok      {} observation(s), {} anomaly candidate(s), {} signal(s) [{status}]",
            source_id.as_str(),
            outcome.observations_ingested,
            outcome.candidates,
            outcome.signals.len()
        );

        if verbose {
            for signal in &outcome.signals {
                let types: Vec<&str> = signal.types.iter().map(|t| t.as_str()).collect();
                println!("      [{}] {}", types.join(" + "), signal.title);
                for reason in &signal.reasons {
                    println!("        why: {reason}");
                }
            }
        }
        if json {
            for signal in &outcome.signals {
                println!("{}", serde_json::to_string_pretty(signal)?);
            }
        }
    }

    println!(
        "\nTotal: {} observation(s), {} anomaly candidate(s), {} event(s), {} signal(s).",
        engine.metrics().observations_total,
        engine.metrics().anomalies_total,
        engine.metrics().events_total,
        engine.metrics().signals_total
    );

    if let Some(path) = out {
        let observations = engine
            .store()
            .query_observations(&wse_storage::ObservationQuery {
                limit: Some(usize::MAX),
                ..Default::default()
            })?
            .items;
        let header = wse_collector::StreamHeader::new(observations.len() as u64, catalog.clone());
        let file = std::fs::File::create(&path)?;
        let mut writer = std::io::BufWriter::new(file);
        wse_collector::write_stream(&mut writer, &header, &observations)?;
        println!(
            "Wrote {} observation(s) to {} (replay with: wse replay-stream --file {}).",
            observations.len(),
            path.display(),
            path.display()
        );
    }
    Ok(())
}

/// Replay a captured stream, offline and deterministically.
///
/// The engine clock is pinned to each arrival batch, so the detector sees the
/// same batches in the same order that live collection produced. No network.
async fn replay_stream(file: std::path::PathBuf, verbose: bool, json: bool) -> Result<()> {
    let handle = std::fs::File::open(&file)?;
    let stream = wse_collector::read_stream(std::io::BufReader::new(handle))?;
    let (from, to) = stream
        .observed_span()
        .map(|(a, b)| (a.to_rfc3339(), b.to_rfc3339()))
        .unwrap_or_else(|| ("-".to_string(), "-".to_string()));

    println!(
        "Replaying {} observation(s) across {} cycle(s) from {}.",
        stream.observations.len(),
        stream.cycles().len(),
        file.display()
    );
    println!("Source clock span: {from} .. {to}");

    let start = stream
        .observations
        .iter()
        .map(|o| o.received_at)
        .min()
        .unwrap_or_else(chrono::Utc::now);
    let clock = wse_scheduler::SharedReplayClock::new(start, chrono::Duration::zero());
    let mut engine = Engine::with_clock(EngineConfig::default(), clock.handle());

    for source in stream.header.iter().flat_map(|h| h.sources.clone()) {
        engine.register_source(source)?;
    }

    for (index, cycle) in stream.cycles().iter().enumerate() {
        if let Some(first) = cycle.first() {
            clock.set(first.received_at);
        }
        let (_, signals, _candidates) = engine.ingest_observations(cycle.clone());
        if verbose {
            for signal in &signals {
                let types: Vec<&str> = signal.types.iter().map(|t| t.as_str()).collect();
                println!("cycle {index}: [{}] {}", types.join(" + "), signal.title);
            }
        }
    }

    let signals = engine
        .store()
        .query_signals(&wse_storage::SignalQuery::default())?;
    println!(
        "\nReplayed {} observation(s), detected {} anomaly candidate(s), formed {} signal(s).",
        engine.metrics().observations_total,
        engine.metrics().anomalies_total,
        signals.total
    );
    for signal in &signals.items {
        let types: Vec<&str> = signal.types.iter().map(|t| t.as_str()).collect();
        println!(
            "\n[{}] {}\n  {}\n  evidence: {} observation(s) from {} source(s)\n  why: {}",
            types.join(" + "),
            signal.title,
            signal.summary,
            signal.evidence.len(),
            signal.distinct_sources(),
            signal.reasons.join("; ")
        );
        if json {
            println!("{}", serde_json::to_string_pretty(signal)?);
        }
    }
    Ok(())
}

/// Score the detector against a captured stream.
async fn backtest(
    file: std::path::PathBuf,
    labels: Option<std::path::PathBuf>,
    json: bool,
) -> Result<()> {
    let handle = std::fs::File::open(&file)?;
    let stream = wse_collector::read_stream(std::io::BufReader::new(handle))?;

    let truth: Vec<wse_engine::LabeledEvent> = match &labels {
        Some(path) => {
            let text = std::fs::read_to_string(path)?;
            serde_json::from_str(&text)?
        }
        None => Vec::new(),
    };

    if let Some(path) = &labels {
        let present = wse_engine::backtest::labeled_series_present(&stream, &truth);
        if present.len() != truth.len() {
            eprintln!(
                "warning: {} of {} label(s) in {} name a series absent from the stream; \
                 they will score as false negatives",
                truth.len() - present.len(),
                truth.len(),
                path.display()
            );
        }
    }

    let report = wse_engine::run_backtest(&stream, &truth, EngineConfig::default());

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!("Backtest: {}", file.display());
    println!(
        "  replayed       {} observation(s) in {} cycle(s)",
        report.observations_replayed, report.cycles
    );
    if let Some(span) = report.stream_span_seconds {
        println!("  stream span    {span}s");
    }
    println!("  signals        {}", report.signals_total);

    match report.mean_latency_seconds {
        Some(mean) => println!("  mean latency   {mean:.0}s (first_seen - earliest evidence)"),
        None => println!("  mean latency   n/a (no evidence to measure)"),
    }
    match report.mean_persistence_seconds {
        Some(mean) => println!(
            "  persistence    mean {mean:.0}s, max {}s",
            report.max_persistence_seconds.unwrap_or(0)
        ),
        None => println!("  persistence    n/a (no signals)"),
    }

    if report.is_labelled() {
        println!(
            "  labelled       {} event(s): {} matched, {} false positive, {} missed",
            report.truth_total,
            report.matched,
            report.false_positives,
            report.false_negatives.len()
        );
        if let (Some(p), Some(r)) = (report.precision(), report.recall()) {
            println!("  precision      {p:.2}\n  recall         {r:.2}");
        }
        for missed in &report.false_negatives {
            println!(
                "  MISSED         [{}] {} / {} ({} .. {})",
                missed.label, missed.source_id, missed.metric, missed.start, missed.end
            );
        }
    } else {
        println!("  unlabelled     precision and recall withheld: no ground truth was supplied");
    }

    for detection in &report.detections {
        let types = detection.types.join(" + ");
        let latency = detection
            .latency_seconds
            .map(|l| format!("{l}s"))
            .unwrap_or_else(|| "-".to_string());
        println!(
            "\n  [{}] {}\n    first seen {}  latency {}  duration {}s  evidence {}",
            types,
            detection.title,
            detection.first_seen.to_rfc3339(),
            latency,
            detection.duration_seconds,
            detection.evidence_count
        );
    }
    Ok(())
}

async fn demo(steps: usize, json: bool) -> Result<()> {
    let world = SyntheticWorld::acceptance(origin());
    let mut engine = synthetic_engine();
    register_synthetic_sources(&mut engine, &world)?;

    let collector = SyntheticCollector::new(world);
    for _ in 0..steps {
        engine.run_collector(&collector).await;
    }

    let signals = engine
        .store()
        .query_signals(&wse_storage::SignalQuery::default())?;
    println!(
        "Generated {} observations, detected {} anomalies, formed {} signals.",
        engine.metrics().observations_total,
        engine.metrics().anomalies_total,
        signals.total
    );
    for signal in &signals.items {
        let types: Vec<&str> = signal.types.iter().map(|t| t.as_str()).collect();
        println!(
            "\n[{}] {}\n  {}\n  evidence: {} observation(s) from {} source(s)\n  why: {}",
            types.join(" + "),
            signal.title,
            signal.summary,
            signal.evidence.len(),
            signal.distinct_sources(),
            signal.reasons.join("; ")
        );
        if json {
            println!("{}", serde_json::to_string_pretty(signal)?);
        }
    }
    Ok(())
}

async fn replay(steps: usize, verbose: bool) -> Result<()> {
    let world = SyntheticWorld::acceptance(origin());
    let mut engine = synthetic_engine();
    register_synthetic_sources(&mut engine, &world)?;
    let collector = SyntheticCollector::new(world);

    for step in 0..steps {
        let outcome = engine.run_collector(&collector).await;
        if outcome.source_failed {
            eprintln!("collector failed at step {step}");
        }
        if verbose {
            for signal in &outcome.signals {
                let types: Vec<&str> = signal.types.iter().map(|t| t.as_str()).collect();
                println!("step {step}: [{}] {}", types.join(" + "), signal.title);
            }
        }
    }
    println!(
        "Replay complete: {} observations, {} signals.",
        engine.metrics().observations_total,
        engine.metrics().signals_total
    );
    Ok(())
}

async fn serve(
    port: u16,
    synthetic: bool,
    live: bool,
    interval_seconds: u64,
    collect: bool,
) -> Result<()> {
    let mut engine = synthetic_engine();
    if synthetic {
        let world = SyntheticWorld::acceptance(origin());
        register_synthetic_sources(&mut engine, &world)?;
    } else if !collect {
        // A served instance with neither a synthetic world nor real collectors
        // still needs one catalog entry so the UI has something to list.
        engine.register_source(
            Source::new(SourceId::new("synthetic_sensor"), "Synthetic", "synthetic")
                .with_category("synthetic"),
        )?;
    }

    if collect {
        // Register the real catalog and run each collector once. Failures are
        // recorded as source health and never fabricate observations.
        for source in wse_sources::catalog() {
            engine.register_source(source)?;
        }
        for collector in wse_sources::live_collectors() {
            let source_id = collector.source_id();
            let outcome = engine.run_collector(collector.as_ref()).await;
            if outcome.source_failed {
                tracing::warn!(
                    source = %source_id,
                    "collector failed at startup; source health updated, no observations"
                );
            } else {
                tracing::info!(
                    source = %source_id,
                    observations = outcome.observations_ingested,
                    signals = outcome.signals.len(),
                    "collected"
                );
            }
        }
    }

    let state = AppState::new(engine);

    if synthetic {
        let driver = state.clone();
        let world = SyntheticWorld::acceptance(origin());
        let mut collector = SyntheticCollector::new(world);
        if live {
            // Rewind so the world keeps producing after the script runs out.
            collector = collector.with_restart(true);
        }
        tokio::spawn(async move {
            loop {
                driver.write().await.run_collector(&collector).await;
                if !live && collector.cursor() >= collector.length() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_secs(interval_seconds)).await;
            }
        });
    }

    let app = wse_api::router(state);
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    tracing::info!("World Signal Engine API listening on http://0.0.0.0:{port}");
    axum::serve(listener, app).await?;
    Ok(())
}

/// Small helper so `demo` reads cleanly.
#[allow(dead_code)]
fn _assert_traits_in_scope(store: &wse_storage::InMemoryStore) {
    let _ = store.all_sources();
    let _ = store.observation_count();
}
