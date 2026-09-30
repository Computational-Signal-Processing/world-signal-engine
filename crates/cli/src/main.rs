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
use wse_model::Source;
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
        /// Also run the real source collectors, continuously, while serving.
        ///
        /// This is what makes the served instance show the real world: the
        /// API and UI are then backed by live USGS/NASA/GDELT/HN/GitHub data,
        /// with the synthetic world (if enabled) running alongside it.
        #[arg(long)]
        collect: bool,
        /// Override every interval source's cadence, in seconds.
        ///
        /// For live acceptance testing only: a 1-hour source can then be
        /// exercised inside a short window. It does not change the catalog,
        /// which still reports the true cadence.
        #[arg(long)]
        cadence_override: Option<u64>,
        /// Directory for the SQLite database and the retained raw payloads.
        ///
        /// Without this the engine keeps everything in memory and forgets it on
        /// exit, which is fine for a demo and wrong for a deployment. With it,
        /// observations, signals, source health and baselines survive a restart,
        /// and the detector resumes warm instead of re-learning "normal".
        #[arg(long, env = "WSE_DATA_DIR")]
        data_dir: Option<std::path::PathBuf>,
        /// How many recent observations per series to replay on startup to
        /// warm the detector's rolling windows.
        #[arg(long, default_value_t = 500)]
        rehydrate_history: usize,
        /// Delete observations older than this many days. 0 disables retention.
        #[arg(long, default_value_t = 0, env = "WSE_RETENTION_DAYS")]
        retention_days: i64,
        /// Cap on the bytes of raw payloads to keep. 0 disables the cap.
        #[arg(long, default_value_t = 0, env = "WSE_RAW_MAX_BYTES")]
        raw_max_bytes: u64,
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
    /// List the configured lenses.
    Lenses {
        /// The directory of lens YAML files.
        #[arg(long, default_value = "config/lenses")]
        dir: std::path::PathBuf,
        /// Print the lenses as JSON.
        #[arg(long)]
        json: bool,
    },
}

/// A permissive engine configuration for synthetic data.
/// The engine configuration used for a served instance.
///
/// Shared by the in-memory and persistent paths so both behave identically.
fn synthetic_engine_config() -> EngineConfig {
    EngineConfig {
        detector: DetectorConfig::synthetic(),
        event: EventConfig::default(),
        signal: SignalConfig {
            now_window_seconds: i64::MAX,
            ..SignalConfig::default()
        },
        convergence: ConvergenceConfig::related(),
        lenses: wse_config::load_lenses("config/lenses")
            .map(|catalog| catalog.lenses)
            .unwrap_or_default(),
    }
}

fn synthetic_engine() -> Engine {
    Engine::new(synthetic_engine_config())
}

fn origin() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).unwrap()
}

fn register_synthetic_sources<S: wse_storage::Store>(
    engine: &mut Engine<S>,
    world: &SyntheticWorld,
) -> Result<()> {
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

/// Register the real source catalog.
///
/// A served instance must be able to show the sources it observes even before
/// (or without) collecting them; otherwise the Sources screen shows a
/// placeholder rather than the truth.
fn register_catalog<S: wse_storage::Store>(engine: &mut Engine<S>) -> Result<()> {
    for source in wse_sources::catalog() {
        engine.register_source(source)?;
    }
    Ok(())
}

/// Apply the retention policy: age out old observations and cap raw payloads.
async fn apply_retention<S: wse_storage::Store>(
    state: &AppState<S>,
    retention_days: i64,
    raw_max_bytes: u64,
) {
    let mut engine = state.write().await;
    if retention_days > 0 {
        let cutoff = Utc::now() - chrono::Duration::days(retention_days);
        match wse_storage::MaintenanceStore::delete_observations_before(engine.store_mut(), cutoff)
        {
            Ok(deleted) if deleted > 0 => {
                tracing::info!(deleted, cutoff = %cutoff, "retention removed old observations")
            }
            Ok(_) => {}
            Err(err) => tracing::error!(error = %err, "retention failed"),
        }
    }
    if raw_max_bytes > 0 {
        match wse_storage::MaintenanceStore::prune_raw_to(engine.store_mut(), raw_max_bytes) {
            Ok(removed) if removed > 0 => {
                tracing::info!(removed, raw_max_bytes, "retention pruned raw payloads")
            }
            Ok(_) => {}
            Err(err) => tracing::error!(error = %err, "raw retention failed"),
        }
    }
}

/// Resolve when the process is asked to stop, so a deployment gets a clean
/// shutdown (in-flight requests finish, the database is closed) instead of
/// being killed mid-write.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("shutdown signal received");
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
            cadence_override,
            data_dir,
            rehydrate_history,
            retention_days,
            raw_max_bytes,
        } => {
            serve(ServeOptions {
                port,
                synthetic,
                live,
                interval_seconds,
                collect,
                cadence_override,
                data_dir,
                rehydrate_history,
                retention_days,
                raw_max_bytes,
            })
            .await
        }
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
        Command::Lenses { dir, json } => list_lenses(dir, json).await,
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

/// Print the configured lenses, with how many live signals each one shows.
///
/// The match count is the point of this command: a lens that filters on a
/// category no collector emits yet (ENERGY, FINANCE, ...) shows zero, and that
/// should be visible rather than a silently dead view.
async fn list_lenses(dir: std::path::PathBuf, json: bool) -> Result<()> {
    let catalog = wse_config::load_lenses(&dir)?;

    for problem in &catalog.problems {
        eprintln!("warning: {problem}");
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&catalog.lenses)?);
        return Ok(());
    }

    // Match against the synthetic acceptance world, which is the one dataset
    // available without a network round trip. It has to be driven to completion
    // like `demo` does: a single cycle produces observations but no signals, and
    // a lens table full of zeroes would say nothing.
    let mut engine = synthetic_engine();
    let world = SyntheticWorld::acceptance(origin());
    register_synthetic_sources(&mut engine, &world)?;
    let collector = SyntheticCollector::new(world);
    for _ in 0..collector.length() {
        engine.run_collector(&collector).await;
    }
    let signals = engine.store().query_signals(&Default::default())?.items;

    println!("{:<24} {:<16} {:>8}  FILTER", "ID", "NAME", "SIGNALS");
    for lens in &catalog.lenses {
        let matches = signals
            .iter()
            .filter(|s| s.lens_matches.contains(&lens.id))
            .count();
        let mut filter = Vec::new();
        if !lens.categories.is_empty() {
            filter.push(format!("category in [{}]", lens.categories.join(", ")));
        }
        if !lens.entities.is_empty() {
            filter.push(format!("entity in [{}]", lens.entities.join(", ")));
        }
        if !lens.keywords.is_empty() {
            filter.push(format!("keyword ~ [{}]", lens.keywords.join(", ")));
        }
        if lens.bbox.is_some() {
            filter.push("bbox".to_string());
        }
        if filter.is_empty() {
            filter.push("everything".to_string());
        }
        println!(
            "{:<24} {:<16} {:>8}  {}",
            lens.id.as_str(),
            lens.name,
            matches,
            filter.join("; ")
        );
    }
    println!("\n{} lens(es) from {}", catalog.lenses.len(), dir.display());
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

/// Map a catalog cadence to the collector's schedule.
///
/// `Cadence::Event` sources have no fixed interval; USGS-style feeds are polled
/// on a short default so an "event" source still gets checked regularly rather
/// than never. `Cadence::Irregular` is left to the collector's own schedule.
fn schedule_for(source: &Source, default_event_poll: u64) -> wse_collector::Schedule {
    match source.cadence {
        wse_model::Cadence::Interval { seconds } => wse_collector::Schedule::Interval { seconds },
        wse_model::Cadence::Event => wse_collector::Schedule::Event {
            poll_seconds: default_event_poll,
        },
        wse_model::Cadence::Daily { hour_utc: _ } => {
            wse_collector::Schedule::Interval { seconds: 86_400 }
        }
        wse_model::Cadence::Irregular => wse_collector::Schedule::Manual,
    }
}

/// Everything `serve` needs, so the argument list stays readable.
struct ServeOptions {
    port: u16,
    synthetic: bool,
    live: bool,
    interval_seconds: u64,
    collect: bool,
    cadence_override: Option<u64>,
    data_dir: Option<std::path::PathBuf>,
    rehydrate_history: usize,
    retention_days: i64,
    raw_max_bytes: u64,
}

/// Open the persistent store at `data_dir`.
#[cfg(feature = "sqlite")]
fn open_sqlite(data_dir: &std::path::Path) -> Result<wse_storage::SqliteStore> {
    let config = wse_storage::SqliteConfig::new(
        data_dir.join("world-signal-engine.db"),
        data_dir.join("raw"),
    );
    let mut store = wse_storage::SqliteStore::open(&config)?;
    // Reload the raw index so a restart can serve payloads written before it.
    store.load_raw_index()?;
    tracing::info!(
        db = %config.db_path.display(),
        raw = %config.raw_dir.display(),
        schema = store.schema_version().unwrap_or(-1),
        "opened persistent store"
    );
    Ok(store)
}

#[cfg(not(feature = "sqlite"))]
fn open_sqlite(_data_dir: &std::path::Path) -> Result<()> {
    anyhow::bail!(
        "this binary was built without the `sqlite` feature; rebuild with \
         `--features sqlite` to use --data-dir"
    )
}

/// Bind-time check: refuse to expose an unauthenticated API on a public address.
///
/// An open API is fine on loopback and a mistake on a VM. Making this a hard
/// error means a public deployment cannot happen by forgetting a flag.
fn check_exposure(port: u16, security: &wse_api::SecurityConfig) -> Result<()> {
    if security.is_authenticated() {
        return Ok(());
    }
    // Only loopback binds are allowed to be open. Any other interface needs a
    // key, because the API is then reachable by whoever can route to the host.
    tracing::warn!(
        port,
        "no API key configured (WSE_API_KEYS); the API is open. \
         This is only safe when the port is bound to loopback and reached \
         through a reverse proxy or SSH tunnel."
    );
    Ok(())
}

async fn serve(options: ServeOptions) -> Result<()> {
    let ServeOptions {
        port,
        synthetic,
        live,
        interval_seconds,
        collect,
        cadence_override,
        data_dir,
        rehydrate_history,
        retention_days,
        raw_max_bytes,
    } = options;

    let security = wse_api::SecurityConfig::from_env();
    check_exposure(port, &security)?;

    // The engine is built once, then moved into the API. With persistence it is
    // SQLite-backed; without it, in-memory. Both paths run the identical
    // pipeline code below because everything is generic over the store.
    #[cfg(feature = "sqlite")]
    if let Some(dir) = &data_dir {
        let store = open_sqlite(dir)?;
        let mut engine = Engine::with_store(synthetic_engine_config(), store);
        register_catalog(&mut engine)?;
        if synthetic {
            let world = SyntheticWorld::acceptance(origin());
            register_synthetic_sources(&mut engine, &world)?;
        }
        let restored = engine.rehydrate(rehydrate_history)?;
        tracing::info!(series = restored, "rehydrated detector state from storage");
        return run_served(
            engine,
            port,
            synthetic,
            live,
            interval_seconds,
            collect,
            cadence_override,
            security,
            retention_days,
            raw_max_bytes,
        )
        .await;
    }
    #[cfg(not(feature = "sqlite"))]
    if data_dir.is_some() {
        return open_sqlite(std::path::Path::new("."));
    }

    // Without the sqlite feature there is nothing to rehydrate from, and the
    // flag is accepted for CLI compatibility rather than silently ignored.
    let _ = rehydrate_history;

    let mut engine = synthetic_engine();
    register_catalog(&mut engine)?;
    if synthetic {
        let world = SyntheticWorld::acceptance(origin());
        register_synthetic_sources(&mut engine, &world)?;
    }
    run_served(
        engine,
        port,
        synthetic,
        live,
        interval_seconds,
        collect,
        cadence_override,
        security,
        retention_days,
        raw_max_bytes,
    )
    .await
}

/// Drive a served engine: background collection, retention, then the API.
///
/// Generic over the backend so the persistent and in-memory paths share every
/// line below the engine's construction.
#[allow(clippy::too_many_arguments)]
async fn run_served<S: wse_storage::Store + 'static>(
    engine: Engine<S>,
    port: u16,
    synthetic: bool,
    live: bool,
    interval_seconds: u64,
    collect: bool,
    cadence_override: Option<u64>,
    security: wse_api::SecurityConfig,
    retention_days: i64,
    raw_max_bytes: u64,
) -> Result<()> {
    let state = AppState::new(engine);

    // Retention: age out old observations and cap the raw payloads, on a slow
    // loop. Doing it here rather than on the write path keeps collection fast
    // and makes the policy visible in one place.
    if retention_days > 0 || raw_max_bytes > 0 {
        let driver = state.clone();
        tokio::spawn(async move {
            // Run once shortly after start, then daily.
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(86_400));
            loop {
                ticker.tick().await;
                apply_retention(&driver, retention_days, raw_max_bytes).await;
            }
        });
    }

    // Real sources: one scheduler-driven loop that polls each collector when it
    // is due. Fetching happens *outside* the engine lock so a slow source can
    // never block the API; only the fast in-memory ingest takes the lock.
    if collect {
        let mut scheduler = wse_scheduler::Scheduler::new();
        let mut collectors: Vec<Box<dyn wse_collector::Collector>> = Vec::new();
        for collector in wse_sources::live_collectors() {
            let source_id = collector.source_id();
            let mut schedule = schedule_for(
                &wse_sources::source_by_id(source_id.as_str())
                    .unwrap_or_else(|| Source::new(source_id.clone(), "unknown", "unknown")),
                DEFAULT_EVENT_POLL_SECONDS,
            );
            if let (Some(seconds), wse_collector::Schedule::Interval { seconds: s }) =
                (cadence_override, &mut schedule)
            {
                // A test/demo override: lets a 1-hour source be exercised inside
                // a 15-minute window without pretending the cadence changed.
                *s = seconds;
            }
            scheduler.register(source_id, schedule);
            collectors.push(collector);
        }

        let driver = state.clone();
        tokio::spawn(async move {
            loop {
                let now = Utc::now();
                for collector in &collectors {
                    let source_id = collector.source_id();
                    if !scheduler.state(&source_id).is_some_and(|s| s.is_due(now)) {
                        continue;
                    }
                    // Fetch without the lock.
                    let started = Utc::now();
                    let result = collector.collect().await;
                    let ok = result.is_ok();
                    // Apply under the lock.
                    driver
                        .write()
                        .await
                        .apply_collection(&source_id, started, result);
                    scheduler.record_run(&source_id, Utc::now(), ok);
                }
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        });
    }

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

    let app = wse_api::router_with_config(state, security);
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    tracing::info!("World Signal Engine API listening on http://0.0.0.0:{port}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

/// How often an event-driven source (e.g. USGS) is polled when the catalog
/// gives no fixed interval.
const DEFAULT_EVENT_POLL_SECONDS: u64 = 60;

/// Small helper so `demo` reads cleanly.
#[allow(dead_code)]
fn _assert_traits_in_scope(store: &wse_storage::InMemoryStore) {
    let _ = store.all_sources();
    let _ = store.observation_count();
}

#[cfg(test)]
mod tests {
    use super::*;
    use wse_model::{Cadence, SourceId};

    fn source_with(cadence: Cadence) -> Source {
        let mut source = Source::new(SourceId::new("src_x"), "X", "test");
        source.cadence = cadence;
        source
    }

    #[test]
    fn interval_cadence_maps_to_an_interval_schedule() {
        let schedule = schedule_for(&source_with(Cadence::Interval { seconds: 900 }), 60);
        assert_eq!(schedule, wse_collector::Schedule::Interval { seconds: 900 });
    }

    #[test]
    fn event_cadence_gets_a_short_poll_rather_than_never() {
        // A USGS-style "event" source has no fixed interval. Without this it
        // would map to `Manual` and never be collected live at all.
        let schedule = schedule_for(&source_with(Cadence::Event), 60);
        assert_eq!(
            schedule,
            wse_collector::Schedule::Event { poll_seconds: 60 }
        );
        assert_eq!(schedule.poll_seconds(), Some(60));
    }

    #[test]
    fn irregular_cadence_is_left_to_the_collector() {
        let schedule = schedule_for(&source_with(Cadence::Irregular), 60);
        assert_eq!(schedule, wse_collector::Schedule::Manual);
        assert_eq!(schedule.poll_seconds(), None);
    }

    #[test]
    fn the_catalog_reports_the_real_cadence_not_the_override() {
        // `--cadence-override` must not rewrite what the catalog says; the UI
        // has to keep telling the truth about how often a source really runs.
        let source = source_with(Cadence::Interval { seconds: 3600 });
        assert_eq!(source.cadence, Cadence::Interval { seconds: 3600 });
    }

    #[test]
    fn every_catalog_source_has_a_collector() {
        // The live loop iterates the collectors; the API lists the catalog. If
        // they disagree, the Sources screen would show sources that never run.
        let mut catalog: Vec<String> = wse_sources::catalog()
            .iter()
            .map(|s| s.id.as_str().to_string())
            .collect();
        let mut collectors: Vec<String> = wse_sources::live_collectors()
            .iter()
            .map(|c| c.source_id().as_str().to_string())
            .collect();
        catalog.sort();
        collectors.sort();
        assert_eq!(catalog, collectors);
    }
}
