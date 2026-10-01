//! Lens coverage: the source network and the lens configuration must agree.
//!
//! Two failures this guards against:
//!
//! 1. A source claims to feed a lens that does not exist (a typo in
//!    `feeds_lenses`), so the lens silently loses a source.
//! 2. A lens exists but no connected source feeds it, and nobody noticed. Some
//!    lenses are deliberately unfed (see `INTENTIONALLY_UNFED`), but that must be
//!    a decision, not an accident.

use std::collections::BTreeSet;
use std::path::PathBuf;

/// Lenses that are shipped with no connected source on purpose, because adding
/// the source is the remaining work and the lens is the placeholder for it.
/// Removing a name here means the lens must be fed; adding one means accepting
/// that it shows nothing today.
const INTENTIONALLY_UNFED: &[&str] = &["lens_agriculture", "lens_energy"];

/// Universal lenses. They impose no filter, so they match every signal and are
/// not "fed" by any particular source.
const UNIVERSAL: &[&str] = &["lens_global", "lens_personal"];

fn lenses_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../config/lenses")
}

#[test]
fn every_source_feeds_a_lens_that_exists() {
    let catalog = wse_config::load_lenses(lenses_dir()).expect("lens config loads");
    assert!(
        catalog.problems.is_empty(),
        "lens files failed to load: {:?}",
        catalog.problems
    );
    let lens_ids: BTreeSet<String> = catalog
        .lenses
        .iter()
        .map(|l| l.id.as_str().to_string())
        .collect();

    for source in wse_sources::catalog() {
        assert!(
            !source.feeds_lenses.is_empty(),
            "{} declares no lens coverage",
            source.id
        );
        for lens in &source.feeds_lenses {
            assert!(
                lens_ids.contains(lens),
                "{} claims to feed {lens}, which is not a configured lens",
                source.id
            );
        }
    }
}

#[test]
fn every_lens_is_either_fed_or_declared_unfed() {
    let catalog = wse_config::load_lenses(lenses_dir()).expect("lens config loads");
    let fed: BTreeSet<String> = wse_sources::catalog()
        .into_iter()
        .flat_map(|s| s.feeds_lenses)
        .collect();

    for lens in &catalog.lenses {
        let id = lens.id.as_str();
        if UNIVERSAL.contains(&id) {
            continue;
        }
        if INTENTIONALLY_UNFED.contains(&id) {
            assert!(
                !fed.contains(id),
                "{id} is declared intentionally unfed but a source now feeds it; \
                 remove it from INTENTIONALLY_UNFED"
            );
            continue;
        }
        assert!(
            fed.contains(id),
            "{id} has no connected source and is not declared intentionally unfed"
        );
    }
}

#[test]
fn the_core_lenses_are_covered() {
    // The point of the source-network objective: these lenses must show real
    // material, not be placeholders.
    let fed: BTreeSet<String> = wse_sources::catalog()
        .into_iter()
        .flat_map(|s| s.feeds_lenses)
        .collect();
    for id in [
        "lens_earth",
        "lens_space",
        "lens_science",
        "lens_ai",
        "lens_cyber",
        "lens_finance",
        "lens_software",
        "lens_global_events",
        "lens_turkey",
    ] {
        assert!(fed.contains(id), "{id} must be fed by at least one source");
    }
}

/// Runtime enforcement of `feeds_lenses`: a declared lens is actually reached.
///
/// This is the generic runtime proof: for *every* source in the catalog, and
/// every lens it declares, a signal carrying that source's provenance reaches
/// the declared lens — even when the lens's category/entity/keyword filters do
/// not match it. Nothing here names a source or a category; it loops over the
/// catalog, so a new source is covered automatically.
///
/// It is the runtime counterpart to the declaration checks above: those prove
/// the *declaration* is well-formed, this proves the declaration is *reachable*.
/// A source listed in `feeds_lenses` but incapable of producing a lens-visible
/// signal fails here.
#[test]
fn a_declared_lens_is_reachable_at_runtime() {
    use wse_model::{
        AnomalyCandidate, BaselineSnapshot, CandidateDirection, CandidateKind, EntityId,
        ObservationId, SourceId,
    };
    use wse_signals::event::EventEngine;
    use wse_signals::{SignalConfig, SignalEngine};

    let catalog = wse_config::load_lenses(lenses_dir()).expect("lens config loads");
    let lenses = catalog.lenses;

    // The source→lens declarations, straight from the catalog.
    let source_lenses: Vec<(String, String)> = wse_sources::catalog()
        .into_iter()
        .flat_map(|s| {
            let id = s.id.as_str().to_string();
            s.feeds_lenses.into_iter().map(move |l| (id.clone(), l))
        })
        .collect();

    let engine = SignalEngine::new(SignalConfig::default())
        .with_lenses(lenses.clone())
        .with_source_lenses(source_lenses);

    let now = chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap();
    // A category and entity no lens filters on, so the only thing that can put
    // the signal into a lens is provenance routing.
    let category_of = |_: &str| Some("sentinel_category".to_string());

    for source in wse_sources::catalog() {
        for lens_id in &source.feeds_lenses {
            let lens = lenses
                .iter()
                .find(|l| l.id.as_str() == lens_id)
                .expect("declared lens exists");

            let mut candidate = AnomalyCandidate::new(
                format!("{}::sentinel_entity::sentinel_metric::unit", source.id),
                ObservationId::new(format!("obs_{}_{lens_id}", source.id)),
                now,
                BaselineSnapshot {
                    sample_size: 50,
                    mean: 0.0,
                    median: 0.0,
                    std_dev: 1.0,
                    mad: 1.0,
                    p05: 0.0,
                    p95: 0.0,
                    ewma: 0.0,
                    trend_per_second: 0.0,
                    volatility: 1.0,
                },
                4.0,
            );
            candidate.source_id = SourceId::new(source.id.as_str());
            candidate.entity_id = Some(EntityId::new("sentinel_entity"));
            candidate.kind = CandidateKind::Anomaly;
            candidate.direction = CandidateDirection::Up;
            candidate.score = 4.0;
            candidate.confidence = 0.9;

            let candidates = vec![candidate];
            let mut events = EventEngine::new(Default::default());
            let formed = events.ingest(&candidates, &category_of);
            let signals = engine.form_signals(&formed, &candidates, now);

            let signal = signals
                .first()
                .unwrap_or_else(|| panic!("{} must form a signal", source.id));
            assert!(
                signal.lens_matches.contains(&lens.id),
                "{} declares {lens_id} but its signal does not reach it: {:?}",
                source.id,
                signal.lens_matches
            );
        }
    }
}
