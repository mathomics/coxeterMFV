//! Port of `src/analysis/cross-feeding-explorer.js`.
//!
//! Cross-Feeding Explorer V1: builds an explicit, filterable event index from
//! the solved community fluxes, and ranks the exchanged metabolites the map
//! shows.

use std::cmp::Ordering;
use std::sync::LazyLock;

use coxeter_core::community::{CrossFeeding, CrossFeedingLink};
use coxeter_core::js;
use indexmap::{IndexMap, IndexSet};
use regex::Regex;
use serde_json::{Value, json};

use crate::community_view::CommunityView;
use crate::jsutil::{descending, js_trim, json_number};

/// One producer → consumer pairing between two different species.
#[derive(Debug, Clone, PartialEq)]
pub struct CrossFeedingEvent {
    pub base: String,
    pub from: String,
    pub to: String,
    pub flux: f64,
    /// `cf:<from>:<base>:<to>:<n>`, `n` being its place before sorting.
    pub id: String,
    /// The producer's transfer reaction for this metabolite, when it has one.
    pub producer_transfer: Option<String>,
    pub consumer_transfer: Option<String>,
    /// Always `"active-flux"`: the event exists because flux moves.
    pub evidence: &'static str,
}

impl CrossFeedingEvent {
    pub fn to_json(&self) -> Value {
        json!({
            "base": self.base, "from": self.from, "to": self.to, "flux": json_number(self.flux),
            "id": self.id, "producerTransfer": self.producer_transfer, "consumerTransfer": self.consumer_transfer,
            "evidence": self.evidence,
        })
    }
}

/// Everything one species passes to another, summed.
#[derive(Debug, Clone, PartialEq)]
pub struct CrossFeedingPair {
    pub from: String,
    pub to: String,
    pub total_flux: f64,
    pub events: usize,
    pub metabolites: Vec<String>,
}

impl CrossFeedingPair {
    pub fn to_json(&self) -> Value {
        json!({
            "from": self.from, "to": self.to, "totalFlux": json_number(self.total_flux),
            "events": self.events, "metabolites": self.metabolites,
        })
    }
}

/// The explorer's index: events largest first, the metabolites and species
/// they involve (sorted), and the species pairs, largest first.
#[derive(Debug, Clone, PartialEq)]
pub struct CrossFeedingIndex {
    pub events: Vec<CrossFeedingEvent>,
    pub metabolites: Vec<String>,
    pub species: Vec<String>,
    pub pairs: Vec<CrossFeedingPair>,
    pub threshold: f64,
}

impl CrossFeedingIndex {
    pub fn to_json(&self) -> Value {
        json!({
            "events": self.events.iter().map(CrossFeedingEvent::to_json).collect::<Vec<_>>(),
            "metabolites": self.metabolites,
            "species": self.species,
            "pairs": self.pairs.iter().map(CrossFeedingPair::to_json).collect::<Vec<_>>(),
            "threshold": self.threshold,
        })
    }
}

/// Options of [`build_cross_feeding_index`].
#[derive(Debug, Clone)]
pub struct CrossFeedingOptions {
    /// Fluxes at or below this are no exchange at all (1e-6).
    pub threshold: f64,
    /// Cross-feeding already computed elsewhere (SteadyCom, a joint model).
    /// When absent, the community is asked for it.
    pub raw: Option<CrossFeeding>,
}

impl Default for CrossFeedingOptions {
    fn default() -> Self {
        CrossFeedingOptions { threshold: 1e-6, raw: None }
    }
}

/// Build the event index from the solved fluxes. Links to or from the medium
/// and a species feeding itself are not events.
pub fn build_cross_feeding_index(
    community: &impl CommunityView,
    fluxes: &IndexMap<String, f64>,
    options: &CrossFeedingOptions,
) -> CrossFeedingIndex {
    let computed;
    let raw: &CrossFeeding = match &options.raw {
        Some(raw) => raw,
        None => {
            computed = community.cross_feeding(fluxes, options.threshold);
            &computed
        }
    };
    let between_species = |x: &&CrossFeedingLink| match (&x.from, &x.to) {
        (Some(from), Some(to)) => !from.is_empty() && !to.is_empty() && from != to,
        _ => false,
    };
    let mut events: Vec<CrossFeedingEvent> = raw
        .links
        .iter()
        .filter(between_species)
        .enumerate()
        .map(|(i, x)| {
            let (from, to) = (x.from.clone().unwrap_or_default(), x.to.clone().unwrap_or_default());
            let transfers = community.transfers(&x.base);
            let transfer_of = |species: &str| transfers.iter().find(|t| t.species_id == species).map(|t| t.reaction_id.to_string());
            CrossFeedingEvent {
                id: format!("cf:{from}:{}:{to}:{i}", x.base),
                producer_transfer: transfer_of(&from),
                consumer_transfer: transfer_of(&to),
                base: x.base.clone(),
                from,
                to,
                flux: x.flux,
                evidence: "active-flux",
            }
        })
        .collect();
    events.sort_by(|a, b| descending(&a.flux, &b.flux));

    let mut metabolites: Vec<String> = events.iter().map(|x| x.base.clone()).collect::<IndexSet<_>>().into_iter().collect();
    metabolites.sort_by(|a, b| js::code_unit_compare(a, b));
    let mut species: Vec<String> =
        events.iter().flat_map(|x| [x.from.clone(), x.to.clone()]).collect::<IndexSet<_>>().into_iter().collect();
    species.sort_by(|a, b| js::code_unit_compare(a, b));

    let mut pairs: IndexMap<String, CrossFeedingPair> = IndexMap::new();
    for e in &events {
        let pair = pairs.entry(format!("{}→{}", e.from, e.to)).or_insert_with(|| CrossFeedingPair {
            from: e.from.clone(),
            to: e.to.clone(),
            total_flux: 0.0,
            events: 0,
            metabolites: Vec::new(),
        });
        pair.total_flux += e.flux;
        pair.events += 1;
        pair.metabolites.push(e.base.clone());
    }
    let mut pairs: Vec<CrossFeedingPair> = pairs.into_values().collect();
    pairs.sort_by(|a, b| descending(&a.total_flux, &b.total_flux));

    CrossFeedingIndex { events, metabolites, species, pairs, threshold: options.threshold }
}

/// What the explorer's controls narrow the events to. Empty text means "any".
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CrossFeedingFilter {
    /// Free text looked for in the metabolite and both species, ignoring case.
    pub query: String,
    pub metabolite: String,
    pub producer: String,
    pub consumer: String,
    /// Events below this flux are left out. NaN is no minimum, as the JS's
    /// `Number(minFlux || 0)` treats a missing or NaN value.
    pub min_flux: f64,
}

/// The events of an index that pass a filter, in the index's order.
pub fn filter_cross_feeding_index<'a>(index: Option<&'a CrossFeedingIndex>, filter: &CrossFeedingFilter) -> Vec<&'a CrossFeedingEvent> {
    let q = js_trim(&filter.query).to_lowercase();
    // `Number(minFlux || 0)`: no number is no minimum.
    let min_flux = if filter.min_flux.is_nan() { 0.0 } else { filter.min_flux };
    let Some(index) = index else { return Vec::new() };
    index
        .events
        .iter()
        .filter(|e| {
            (filter.metabolite.is_empty() || e.base == filter.metabolite)
                && (filter.producer.is_empty() || e.from == filter.producer)
                && (filter.consumer.is_empty() || e.to == filter.consumer)
                && e.flux >= min_flux
                && (q.is_empty() || [&e.base, &e.from, &e.to].iter().any(|x| x.to_lowercase().contains(&q)))
        })
        .collect()
}

/*
 * Which exchanged metabolites the map shows. Ecologically, the short-chain
 * fatty acids and lactate come first, in a fixed order; then the carbon sources
 * (sugars, uronic acids, oligo- and polysaccharides); then the other
 * fermentation products; then everything else. Within a group, the one that
 * moves the most between species comes first.
 */
const KEY_ACIDS: [&str; 5] = ["ac", "but", "ppa", "lacl", "lacd"];
static CARBON_SOURCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        "^(glc|fru|gal|man|xyl|arab|rib|rmn|fuc|sucr|lcts|malt|cellb|tre|acgam|acnam|glcur|kesto|inulin|strch|starch|pect|\
         amylose|glucan|xylan|arabinan|homogal|rhamnogal|dextrin|raffin|stys|melib|lactul)",
    )
    .expect("pattern")
});
const OTHER_PRODUCTS: [&str; 17] = [
    "succ", "for", "etoh", "acald", "h2", "pyr", "12ppds", "12ppdr", "ibut", "ival", "2mbut", "isobut", "isoval", "2mba", "btd",
    "actn", "diact",
];

/// The groups of [`exchanged_metabolites`], in display order.
pub const CROSS_FEEDING_GROUPS: [&str; 4] =
    ["Short-chain fatty acids and lactate", "Carbon sources", "Other fermentation products", "Other"];

/// `ac`, `lac-L` → `lacl`: lower case, letters and digits only.
fn metabolite_key(base: &str) -> String {
    base.to_lowercase().chars().filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit()).collect()
}

fn cross_feeding_group(base: &str) -> (usize, usize) {
    let k = metabolite_key(base);
    if let Some(acid) = KEY_ACIDS.iter().position(|a| *a == k) {
        return (0, acid);
    }
    if CARBON_SOURCE.is_match(&k) {
        return (1, 0);
    }
    if OTHER_PRODUCTS.contains(&k.as_str()) {
        return (2, 0);
    }
    (3, 0)
}

/// A metabolite species pass to each other, with how much and along how many
/// producer → consumer lines.
#[derive(Debug, Clone, PartialEq)]
pub struct ExchangedMetabolite {
    pub base: String,
    pub flux: f64,
    pub lines: usize,
    /// One of [`CROSS_FEEDING_GROUPS`].
    pub group: &'static str,
}

impl ExchangedMetabolite {
    pub fn to_json(&self) -> Value {
        json!({"base": self.base, "flux": json_number(self.flux), "lines": self.lines, "group": self.group})
    }
}

/// The metabolites species pass to each other (links with a producer and a
/// consumer), most important first. `skip(base)` leaves some out (protons and
/// water).
pub fn exchanged_metabolites(links: &[CrossFeedingLink], skip: impl Fn(&str) -> bool) -> Vec<ExchangedMetabolite> {
    let named = |s: &Option<String>| s.as_deref().is_some_and(|x| !x.is_empty());
    let mut by_base: IndexMap<String, (f64, usize)> = IndexMap::new();
    for link in links {
        if !named(&link.from) || !named(&link.to) || skip(&link.base) {
            continue;
        }
        let entry = by_base.entry(link.base.clone()).or_insert((0.0, 0));
        entry.0 += link.flux;
        entry.1 += 1;
    }
    let mut ranked: Vec<((usize, usize), String, f64, usize)> =
        by_base.into_iter().map(|(base, (flux, lines))| (cross_feeding_group(&base), base, flux, lines)).collect();
    ranked.sort_by(|a, b| {
        a.0.cmp(&b.0).then_with(|| match b.2.partial_cmp(&a.2) {
            Some(Ordering::Equal) | None => js::code_unit_compare(&a.1, &b.1),
            Some(order) => order,
        })
    });
    ranked
        .into_iter()
        .map(|(rank, base, flux, lines)| ExchangedMetabolite { base, flux, lines, group: CROSS_FEEDING_GROUPS[rank.0] })
        .collect()
}
