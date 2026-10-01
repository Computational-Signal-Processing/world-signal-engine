//! # wse-config
//!
//! Loads configuration from the filesystem.
//!
//! Lenses and the impact scope are configuration, not code: adding one is a new
//! YAML file under `config/lenses/` or `config/impact/`, with no Rust changes.
//! This crate is what makes that true.
//!
//! ## Failure policy
//!
//! A lens is a *view* and the impact scope is a *declaration*; losing either
//! must never take down collection or detection (brief §36), so the loaders are
//! deliberately tolerant:
//!
//! - A missing directory is not an error. No lenses (or no impact scope)
//!   configured is a valid state — the world dataset is complete without them.
//! - A malformed file is skipped, recorded in the catalog's `problems`, and the
//!   remaining files still load. One bad file does not lose the others.
//! - Only being unable to read the directory at all is a [`ConfigError`].
//!
//! Problems are reported rather than swallowed. Tolerating a bad file is not the
//! same as pretending it was fine — an operator needs to know a lens silently
//! stopped applying, or a typo would hide signals with no trace.

use std::collections::BTreeSet;
use std::path::Path;

use wse_model::lens::Lens;
use wse_model::LensId;

mod impact;

pub use impact::{load_impact, ImpactCatalog, ImpactScope};

/// Something went wrong badly enough that no configuration could be read.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("could not read config directory {path}: {source}")]
    Directory {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// The lenses that loaded, plus a note of the files that did not.
#[derive(Debug, Default, Clone)]
pub struct LensCatalog {
    pub lenses: Vec<Lens>,
    /// Human-readable descriptions of files that failed to load.
    pub problems: Vec<String>,
}

impl LensCatalog {
    /// Find the lens with this id.
    pub fn get(&self, id: &str) -> Option<&Lens> {
        self.lenses.iter().find(|l| l.id.as_str() == id)
    }
}

/// Load every `*.yaml` / `*.yml` file in `dir` as a lens.
///
/// Results are sorted by lens id so iteration order does not depend on
/// filesystem read order. That matters: lens matches are written onto signals,
/// and a replayed run must produce the same signal bytes as the original.
pub fn load_lenses(dir: impl AsRef<Path>) -> Result<LensCatalog, ConfigError> {
    let dir = dir.as_ref();
    let mut catalog = LensCatalog::default();

    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        // Not configured is a valid state, not a failure.
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(catalog),
        Err(source) => {
            return Err(ConfigError::Directory {
                path: dir.display().to_string(),
                source,
            })
        }
    };

    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                catalog.problems.push(format!("{}: {err}", dir.display()));
                continue;
            }
        };
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        match path.extension().and_then(|e| e.to_str()) {
            Some("yaml") | Some("yml") => paths.push(path),
            _ => {}
        }
    }
    paths.sort();

    let mut seen: BTreeSet<String> = BTreeSet::new();
    for path in paths {
        match load_lens_file(&path) {
            Ok(lens) => {
                if !seen.insert(lens.id.as_str().to_string()) {
                    catalog.problems.push(format!(
                        "{}: duplicate lens id {} (later definition ignored)",
                        path.display(),
                        lens.id
                    ));
                    continue;
                }
                catalog.lenses.push(lens);
            }
            Err(problem) => catalog.problems.push(problem),
        }
    }

    catalog
        .lenses
        .sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
    Ok(catalog)
}

fn load_lens_file(path: &Path) -> Result<Lens, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|err| format!("{}: could not read: {err}", path.display()))?;
    let lens: Lens = serde_yaml::from_str(&text)
        .map_err(|err| format!("{}: not a valid lens: {err}", path.display()))?;
    if lens.id.as_str().is_empty() {
        return Err(format!("{}: lens id must not be empty", path.display()));
    }
    Ok(lens)
}

/// The lenses that show a signal described by these facets.
///
/// Order follows the catalog, which is sorted by id, so the result is stable.
pub fn matching_lenses<'a>(
    lenses: &'a [Lens],
    categories: &[String],
    entities: &[String],
    text: &str,
    location: Option<(f64, f64)>,
) -> Vec<&'a LensId> {
    lenses
        .iter()
        .filter(|lens| lens.matches(categories, entities, text, location))
        .map(|lens| &lens.id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, body: &str) {
        std::fs::write(dir.join(name), body).unwrap();
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("wse_config_test_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_missing_directory_is_not_an_error() {
        let dir = std::env::temp_dir().join("wse_config_definitely_absent_dir");
        let _ = std::fs::remove_dir_all(&dir);
        let catalog = load_lenses(&dir).unwrap();
        assert!(catalog.lenses.is_empty());
        assert!(catalog.problems.is_empty());
    }

    #[test]
    fn loads_lenses_from_yaml() {
        let dir = temp_dir("loads");
        write(
            &dir,
            "energy.yaml",
            "id: lens_energy\nname: ENERGY\ncategories:\n  - energy\n  - markets\n",
        );
        write(
            &dir,
            "turkey.yaml",
            "id: lens_turkey\nname: TURKEY\nbbox: [35.8, 25.6, 42.1, 44.8]\n",
        );

        let catalog = load_lenses(&dir).unwrap();
        assert!(catalog.problems.is_empty());
        assert_eq!(catalog.lenses.len(), 2);
        // Sorted by id, not by filename or readdir order.
        assert_eq!(catalog.lenses[0].id.as_str(), "lens_energy");
        assert_eq!(catalog.lenses[1].id.as_str(), "lens_turkey");
        assert_eq!(catalog.lenses[1].bbox, Some((35.8, 25.6, 42.1, 44.8)));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_malformed_file_is_reported_and_the_rest_still_load() {
        let dir = temp_dir("malformed");
        write(&dir, "good.yaml", "id: lens_good\nname: GOOD\n");
        write(&dir, "bad.yaml", "id: [this is not a lens\n");

        let catalog = load_lenses(&dir).unwrap();
        assert_eq!(catalog.lenses.len(), 1, "the good lens must still load");
        assert_eq!(catalog.lenses[0].id.as_str(), "lens_good");
        assert_eq!(catalog.problems.len(), 1);
        assert!(
            catalog.problems[0].contains("bad.yaml"),
            "the problem must name the file: {:?}",
            catalog.problems
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_lens_without_an_id_is_rejected() {
        let dir = temp_dir("no_id");
        write(&dir, "nameless.yaml", "id: ''\nname: NAMELESS\n");
        let catalog = load_lenses(&dir).unwrap();
        assert!(catalog.lenses.is_empty());
        assert_eq!(catalog.problems.len(), 1);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn duplicate_ids_are_reported_and_only_one_survives() {
        let dir = temp_dir("dupes");
        write(&dir, "a.yaml", "id: lens_same\nname: FIRST\n");
        write(&dir, "b.yaml", "id: lens_same\nname: SECOND\n");

        let catalog = load_lenses(&dir).unwrap();
        assert_eq!(catalog.lenses.len(), 1);
        assert_eq!(catalog.lenses[0].name, "FIRST", "the first file wins");
        assert_eq!(catalog.problems.len(), 1);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn non_yaml_files_are_ignored() {
        let dir = temp_dir("non_yaml");
        write(&dir, "notes.md", "not a lens");
        write(&dir, "lens.yaml", "id: lens_ok\nname: OK\n");
        let catalog = load_lenses(&dir).unwrap();
        assert_eq!(catalog.lenses.len(), 1);
        assert!(catalog.problems.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn matching_returns_lenses_in_catalog_order() {
        let dir = temp_dir("matching");
        write(
            &dir,
            "b.yaml",
            "id: lens_b\nname: B\ncategories:\n  - energy\n",
        );
        write(
            &dir,
            "a.yaml",
            "id: lens_a\nname: A\ncategories:\n  - energy\n",
        );
        let catalog = load_lenses(&dir).unwrap();
        let ids = matching_lenses(&catalog.lenses, &["energy".to_string()], &[], "", None);
        let ids: Vec<&str> = ids.iter().map(|l| l.as_str()).collect();
        assert_eq!(ids, vec!["lens_a", "lens_b"]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_shipped_lens_set_loads_cleanly() {
        // The repository ships config/lenses/*.yaml; they must all parse.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let catalog = load_lenses(root.join("config/lenses")).unwrap();
        assert!(
            catalog.problems.is_empty(),
            "shipped lenses must all load: {:?}",
            catalog.problems
        );
        assert!(
            catalog.lenses.len() >= 9,
            "expected the full default lens set, got {}",
            catalog.lenses.len()
        );
        assert!(catalog.get("lens_global").is_some());
    }
}
