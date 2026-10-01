//! The vocabulary that turns engine identifiers into human language.
//!
//! Detection works in `series_key` (`source::entity::metric::unit`) and sigma.
//! A person reads "Software attention is rising sharply". This module is the
//! single place that translation lives, so the API, the web client and any
//! future client describe the same change the same way, and so the mapping can
//! be tested like any other logic.
//!
//! Every entry here describes a metric or entity that a *connected* source
//! actually emits. When a new collector is added, its vocabulary is added here;
//! nothing else needs to know. A metric with no entry is not hidden — it is
//! rendered from its own name, marked as having no richer description, so an
//! unmapped series is visible rather than silently missing.

use wse_model::EntityId;

/// How a metric reads to a person.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetricVocab {
    /// What the measurement is about, in a sentence fragment.
    pub subject: &'static str,
    /// A short noun for the subject, used in headlines.
    pub short: &'static str,
    /// The unit as a person says it ("points", "magnitude", "% of global news").
    pub unit_name: &'static str,
    /// A description of what a movement means, for the signal detail page.
    pub movement_meaning: &'static str,
}

/// How an entity reads to a person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityVocab {
    /// What the entity is, in a sentence fragment.
    pub subject: &'static str,
    /// A short noun for headlines.
    pub short: String,
    /// Where it is, when the entity itself names a place.
    pub place: Option<String>,
}

/// The metric vocabulary for every metric the connected sources emit.
pub fn metric_vocab(metric: &str) -> Option<MetricVocab> {
    let entry = match metric {
        "earthquake_magnitude" => MetricVocab {
            subject: "earthquake magnitude",
            short: "seismic activity",
            unit_name: "magnitude",
            movement_meaning: "a higher magnitude than the recent norm for this region",
        },
        "neo_close_approaches" => MetricVocab {
            subject: "close approaches of tracked near-Earth objects",
            short: "near-Earth object traffic",
            unit_name: "approaches",
            movement_meaning: "more near-Earth objects passing close by than usual",
        },
        "news_volume" => MetricVocab {
            subject: "share of global news coverage",
            short: "news coverage",
            unit_name: "% of global news",
            movement_meaning: "more or less of the world's news coverage than usual",
        },
        "story_score" => MetricVocab {
            subject: "attention on a Hacker News story",
            short: "software attention",
            unit_name: "points",
            movement_meaning: "a story gaining attention faster than the recent norm",
        },
        "repo_stars" => MetricVocab {
            subject: "stars gained by repositories in this ecosystem",
            short: "developer interest",
            unit_name: "stars",
            movement_meaning: "developer interest moving faster than the recent norm",
        },
        "kp_index" => MetricVocab {
            subject: "geomagnetic disturbance (planetary K-index)",
            short: "geomagnetic activity",
            unit_name: "kp",
            movement_meaning: "a geomagnetic storm stronger than the recent norm",
        },
        "kev_added" => MetricVocab {
            subject: "vulnerabilities added to the known-exploited catalogue",
            short: "exploited-vulnerability activity",
            unit_name: "vulnerabilities",
            movement_meaning: "more newly exploited vulnerabilities than usual",
        },
        "kev_catalog_total" => MetricVocab {
            subject: "the size of the known-exploited vulnerability catalogue",
            short: "exploited-vulnerability catalogue",
            unit_name: "vulnerabilities",
            movement_meaning: "the catalogue growing faster than its recent pace",
        },
        "exchange_rate" => MetricVocab {
            subject: "the euro reference exchange rate",
            short: "exchange rate",
            unit_name: "rate",
            movement_meaning: "a move in the reference rate larger than the recent norm",
        },
        "works_registered" => MetricVocab {
            subject: "scholarly works registered on a topic",
            short: "research output",
            unit_name: "works",
            movement_meaning: "research output on this topic moving faster than the recent norm",
        },
        "preprint_total" => MetricVocab {
            subject: "preprints posted to an arXiv category",
            short: "preprint output",
            unit_name: "preprints",
            movement_meaning: "preprint output in this category moving faster than the recent norm",
        },
        _ => return None,
    };
    Some(entry)
}

/// The entity vocabulary for the entity ids the connected sources emit.
///
/// Matching is deliberately structural: entities are slugs
/// (`region_hormuz`, `topic_oil_supply`, `ecosystem_rust`), so the prefix
/// selects the family and the remainder is turned back into words.
pub fn entity_vocab(entity: &EntityId) -> Option<EntityVocab> {
    let id = entity.as_str();
    let (prefix, rest) = id.split_once('_')?;
    match prefix {
        "region" => Some(EntityVocab {
            subject: "a seismic region",
            short: "regional seismicity".to_string(),
            place: Some(words(rest)),
        }),
        "topic" => Some(EntityVocab {
            subject: "news coverage of a topic",
            short: format!("news coverage of {}", words(rest)),
            place: None,
        }),
        "ecosystem" => Some(EntityVocab {
            subject: "a software ecosystem",
            short: format!("{} ecosystem", words(rest)),
            place: None,
        }),
        "neo_class" => Some(EntityVocab {
            subject: "tracked near-Earth objects",
            short: "near-Earth objects".to_string(),
            place: None,
        }),
        "province" => Some(EntityVocab {
            subject: "a province",
            short: "provincial seismicity".to_string(),
            place: Some(words(rest)),
        }),
        "research" => Some(EntityVocab {
            subject: "a research topic",
            short: format!("research on {}", words(rest)),
            place: None,
        }),
        "arxiv" => Some(EntityVocab {
            subject: "an arXiv category",
            short: format!("{} preprints", words(rest)),
            place: None,
        }),
        "geomagnetic" => Some(EntityVocab {
            subject: "global geomagnetic activity",
            short: "geomagnetic activity".to_string(),
            place: None,
        }),
        "cyber" => Some(EntityVocab {
            subject: "the exploited-vulnerability catalogue",
            short: "exploited vulnerabilities".to_string(),
            place: None,
        }),
        "fx" => Some(EntityVocab {
            subject: "an exchange rate",
            short: format!("{} rate", words(rest)),
            place: None,
        }),
        _ => None,
    }
}

/// Turn an entity slug remainder back into words: `oil_supply` -> `oil supply`.
fn words(slug: &str) -> String {
    slug.replace('_', " ").trim().to_string()
}

/// Human names for the categories the catalog assigns.
pub fn category_label(category: &str) -> Option<&'static str> {
    let label = match category {
        "geophysics" => "Earth",
        "space" => "Space",
        "global_events" => "Global events",
        "technology" => "Technology",
        "weather" => "Weather",
        "environment" => "Environment",
        "energy" => "Energy",
        "markets" => "Markets",
        "finance" => "Finance",
        "science" => "Science",
        "agriculture" => "Agriculture",
        "transport" => "Transport",
        "cyber" => "Cyber",
        "research" => "Research",
        _ => return None,
    };
    Some(label)
}

/// A human name for a source id, for the evidence list.
pub fn source_label(source_id: &str) -> Option<&'static str> {
    let label = match source_id {
        "usgs_earthquakes" => "USGS earthquakes",
        "afad_earthquakes" => "AFAD Turkey earthquakes",
        "nasa_neo" => "NASA near-Earth objects",
        "noaa_kp_index" => "NOAA geomagnetic index",
        "nasa_eonet" => "NASA natural events",
        "nws_alerts" => "NWS weather alerts",
        "gdelt_news_volume" => "GDELT news volume",
        "hackernews_frontpage" => "Hacker News",
        "github_rust_activity" => "GitHub activity",
        "cisa_kev" => "CISA exploited vulnerabilities",
        "ecb_exchange_rates" => "ECB reference rates",
        "crossref_works" => "Crossref scholarly works",
        "arxiv_submissions" => "arXiv preprints",
        _ => return None,
    };
    Some(label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_connected_metric_has_vocabulary() {
        // These are the metrics the shipped collectors emit. If a collector
        // starts emitting a new metric, this test is the reminder that the
        // vocabulary must follow.
        for metric in [
            "earthquake_magnitude",
            "neo_close_approaches",
            "news_volume",
            "story_score",
            "repo_stars",
            "kp_index",
            "kev_added",
            "kev_catalog_total",
            "exchange_rate",
            "works_registered",
            "preprint_total",
        ] {
            assert!(metric_vocab(metric).is_some(), "no vocab for {metric}");
        }
    }

    #[test]
    fn entity_prefixes_are_understood() {
        let region = entity_vocab(&EntityId::new("region_hormuz")).unwrap();
        assert_eq!(region.place.as_deref(), Some("hormuz"));

        let topic = entity_vocab(&EntityId::new("topic_oil_supply")).unwrap();
        assert!(topic.short.contains("oil supply"));

        let eco = entity_vocab(&EntityId::new("ecosystem_rust")).unwrap();
        assert!(eco.short.contains("rust"));
    }

    #[test]
    fn unknown_metrics_and_entities_return_none() {
        assert!(metric_vocab("something_new").is_none());
        assert!(entity_vocab(&EntityId::new("mystery_thing")).is_none());
        assert!(category_label("uncategorized").is_none());
    }
}
