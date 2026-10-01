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

/// F11 — `feeds_lenses` is a declaration the engine does not enforce. A source's
/// declared lens is matched against the *signal's* category/entity/keyword, not
/// against `feeds_lenses`, so a source can declare a lens its signals never
/// reach. `nasa_eonet` is exactly that case: it declares `lens_earth` but emits
/// category `earth`, while `lens_earth` filters on `[geophysics, environment,
/// weather]`. This spec asserts the two agree; see docs/source-semantic-audit.md.
#[test]
#[ignore = "F11: feeds_lenses is declared but not enforced at runtime; see docs/source-semantic-audit.md"]
fn a_declared_lens_actually_matches_the_sources_signals() {
    let catalog = wse_config::load_lenses(lenses_dir()).expect("lens config loads");
    for source in wse_sources::catalog() {
        for lens_id in &source.feeds_lenses {
            let lens = catalog
                .lenses
                .iter()
                .find(|l| l.id.as_str() == lens_id)
                .expect("declared lens exists");
            // A universal lens (no filters) matches everything.
            let universal = lens.categories.is_empty()
                && lens.entities.is_empty()
                && lens.keywords.is_empty()
                && lens.bbox.is_none();
            if universal {
                continue;
            }
            let category_ok = lens.categories.is_empty()
                || lens
                    .categories
                    .iter()
                    .any(|c| c.eq_ignore_ascii_case(&source.category));
            assert!(
                category_ok,
                "{} declares {lens_id}, but its category {:?} is not among that \
                 lens's categories {:?}, so its signals never reach the lens",
                source.id, source.category, lens.categories
            );
        }
    }
}
