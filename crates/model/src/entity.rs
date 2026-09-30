//! Entities: the things the world is measured about.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ids::EntityId;

/// A canonical "thing" that observations can be attached to.
///
/// Entities are what make cross-source correlation possible: an oil price, a
/// shipping delay and a news mention can only converge if they share an entity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    pub id: EntityId,
    pub kind: EntityKind,
    pub name: String,
    /// Lowercased, accent-stripped key used to match entities across sources.
    pub canonical: String,
    pub aliases: Vec<String>,
    pub attributes: BTreeMap<String, String>,
}

impl Entity {
    pub fn new(kind: EntityKind, name: impl Into<String>) -> Self {
        let name = name.into();
        let canonical = canonicalize(&name);
        Self {
            id: EntityId::generate(),
            kind,
            name,
            canonical,
            aliases: Vec::new(),
            attributes: BTreeMap::new(),
        }
    }

    pub fn with_id(mut self, id: EntityId) -> Self {
        self.id = id;
        self
    }

    pub fn with_attribute(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(key.into(), value.into());
        self
    }

    pub fn add_alias(mut self, alias: impl Into<String>) -> Self {
        self.aliases.push(alias.into());
        self
    }

    /// Whether this entity matches a raw label from a source.
    pub fn matches_label(&self, label: &str) -> bool {
        let candidate = canonicalize(label);
        self.canonical == candidate || self.aliases.iter().any(|a| canonicalize(a) == candidate)
    }
}

/// Normalize an entity label so that `"İstanbul "`, `"Istanbul"` and
/// `"istanbul"` collapse onto the same correlation key.
///
/// Only ASCII-folds the Turkish characters we actually encounter in feeds;
/// this is intentionally conservative rather than a full Unicode fold.
pub fn canonicalize(name: &str) -> String {
    name.trim()
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'ç' => 'c',
            'ğ' => 'g',
            'ı' => 'i',
            'ö' => 'o',
            'ş' => 's',
            'ü' => 'u',
            'â' | 'à' | 'á' => 'a',
            'é' | 'è' => 'e',
            other => other,
        })
        .filter(|c| c.is_alphanumeric() || *c == ' ')
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Country,
    Region,
    City,
    Place,
    Organization,
    Company,
    Commodity,
    Currency,
    Asset,
    Person,
    Technology,
    Software,
    Satellite,
    CelestialBody,
    Route,
    Port,
    Vessel,
    Aircraft,
    Species,
    Metric,
    Topic,
    Other,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalize_folds_case_accents_and_whitespace() {
        assert_eq!(canonicalize("  İstanbul "), "istanbul");
        assert_eq!(canonicalize("Istanbul"), "istanbul");
        assert_eq!(canonicalize("Şanlıurfa"), "sanliurfa");
        assert_eq!(canonicalize("New   York"), "new york");
    }

    #[test]
    fn entity_matches_its_aliases() {
        let e = Entity::new(EntityKind::City, "Istanbul").add_alias("Constantinople");
        assert!(e.matches_label("İSTANBUL"));
        assert!(e.matches_label("constantinople"));
        assert!(!e.matches_label("Ankara"));
    }

    #[test]
    fn entity_round_trips_through_serde() {
        let e =
            Entity::new(EntityKind::Commodity, "Brent Crude").with_attribute("unit", "USD/barrel");
        let json = serde_json::to_string(&e).unwrap();
        let back: Entity = serde_json::from_str(&json).unwrap();
        assert_eq!(back, e);
    }
}
