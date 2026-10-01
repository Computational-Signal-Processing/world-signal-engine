//! Impact scope: which changes count as touching something systemic.
//!
//! `IMPACT` is the fifth signal type and the one with no producer in the MVP:
//! the mechanism exists (`SignalConfig::impact_categories` / `impact_entities`)
//! but nothing ever declared a scope, so the type was unreachable in a served
//! engine.
//!
//! This module is what makes the declaration real, the same way `load_lenses`
//! makes `feeds_lenses` real. A scope is configuration, not code: naming a
//! systemic domain is a YAML file under `config/impact/`, with no Rust change.
//!
//! The scope is deliberately a *declaration*, not a score. A signal becomes
//! `IMPACT` because its category or entity is listed here, and the reason names
//! the term that matched — never because a model decided it mattered.
//!
//! ## Failure policy
//!
//! Identical to the lens loader, and for the same reason: a scope is not
//! load-bearing for collection or detection, so a missing directory is a valid
//! empty scope, a malformed file is reported and skipped, and only an unreadable
//! directory is an error.

use std::collections::BTreeSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::ConfigError;

/// The declared impact scope: categories and entities that count as systemic.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ImpactScope {
    /// Signal categories that count as an impact scope.
    #[serde(default)]
    pub categories: Vec<String>,
    /// Signal entities that count as an impact scope. Matched by canonical
    /// segment, so `Hormuz` matches `region_hormuz`.
    #[serde(default)]
    pub entities: Vec<String>,
    /// Why this scope exists, for the operator reading the config.
    #[serde(default)]
    pub note: Option<String>,
}

/// The merged impact scope, plus a note of the files that did not load.
#[derive(Debug, Default, Clone)]
pub struct ImpactCatalog {
    pub scope: ImpactScope,
    /// Human-readable descriptions of files that failed to load.
    pub problems: Vec<String>,
}

/// Load every `*.yaml` / `*.yml` file in `dir` as an impact scope and merge.
///
/// Terms are sorted and deduplicated so the result does not depend on
/// filesystem read order — impact is written onto signals, and a replayed run
/// must produce the same signal bytes as the original.
pub fn load_impact(dir: impl AsRef<Path>) -> Result<ImpactCatalog, ConfigError> {
    let dir = dir.as_ref();
    let mut catalog = ImpactCatalog::default();

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

    let mut categories: BTreeSet<String> = BTreeSet::new();
    let mut entities: BTreeSet<String> = BTreeSet::new();
    for path in paths {
        match load_impact_file(&path) {
            Ok(scope) => {
                categories.extend(scope.categories);
                entities.extend(scope.entities);
            }
            Err(problem) => catalog.problems.push(problem),
        }
    }

    catalog.scope = ImpactScope {
        categories: categories.into_iter().collect(),
        entities: entities.into_iter().collect(),
        note: None,
    };
    Ok(catalog)
}

fn load_impact_file(path: &Path) -> Result<ImpactScope, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|err| format!("{}: could not read: {err}", path.display()))?;
    serde_yaml::from_str(&text)
        .map_err(|err| format!("{}: not a valid impact scope: {err}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, body: &str) {
        std::fs::write(dir.join(name), body).unwrap();
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("wse_impact_test_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_missing_directory_is_an_empty_scope() {
        let dir = std::env::temp_dir().join("wse_impact_definitely_absent_dir");
        let _ = std::fs::remove_dir_all(&dir);
        let catalog = load_impact(&dir).unwrap();
        assert!(catalog.scope.categories.is_empty());
        assert!(catalog.scope.entities.is_empty());
        assert!(catalog.problems.is_empty());
    }

    #[test]
    fn loads_and_merges_scopes_from_yaml() {
        let dir = temp_dir("merge");
        write(&dir, "a.yaml", "categories:\n  - finance\n");
        write(
            &dir,
            "b.yaml",
            "categories:\n  - cyber\nentities:\n  - Hormuz\n",
        );

        let catalog = load_impact(&dir).unwrap();
        assert!(catalog.problems.is_empty());
        assert_eq!(
            catalog.scope.categories,
            vec!["cyber".to_string(), "finance".to_string()],
            "merged and sorted, so order does not depend on read order"
        );
        assert_eq!(catalog.scope.entities, vec!["Hormuz".to_string()]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_malformed_file_is_reported_and_the_rest_still_load() {
        let dir = temp_dir("malformed");
        write(&dir, "good.yaml", "categories:\n  - finance\n");
        write(&dir, "bad.yaml", "categories: [not, closed\n");

        let catalog = load_impact(&dir).unwrap();
        assert_eq!(catalog.scope.categories, vec!["finance".to_string()]);
        assert_eq!(catalog.problems.len(), 1);
        assert!(catalog.problems[0].contains("bad.yaml"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_shipped_impact_scope_loads_cleanly() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let catalog = load_impact(root.join("config/impact")).unwrap();
        assert!(
            catalog.problems.is_empty(),
            "shipped impact scope must load: {:?}",
            catalog.problems
        );
        assert!(
            !catalog.scope.categories.is_empty(),
            "the shipped engine must declare an impact scope, or IMPACT is unreachable"
        );
    }
}
