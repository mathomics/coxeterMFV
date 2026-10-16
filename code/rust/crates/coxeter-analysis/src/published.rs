//! Port of `src/communities/published.js`: the Published Communities registry.
//!
//! Benchmark communities from the community-FBA literature. For each one this
//! records the paper, the organisms, the public model files the authors used,
//! the exchanges the paper reports, and (kept deliberately separate) exactly
//! which parts Coxeter reproduces and which parts are Coxeter's own adaptation.
//!
//! Two rules govern what may appear here, both of them stricter than they look:
//!
//! * Nothing is invented. A constraint, a bound, a medium or an exchange is
//!   listed only when it was read off an artefact that can be fetched again:
//!   the authors' own tutorial, their code, their supplementary file. Where
//!   that artefact was not reachable the field is null and
//!   `reproduction.needs` says which file is missing and where it lives. A
//!   plausible-looking number is worse than a gap, because a gap is honest
//!   about itself.
//! * Citations carry their evidence. `doi_verified_from` names the retrieved
//!   file the DOI was checked against, or is `None` when it rests on nothing
//!   but recall.
//!
//! Status of each community is in `reproduction.status`:
//!
//! * `Reproduced`: Coxeter computes the numbers the paper reports, from the
//!   authors' own model and constraints. Verified by a test.
//! * `Blocked`: the organisms and the interaction are on record, but the
//!   authors' constraint specification could not be retrieved, so building it
//!   would mean guessing. Not attempted.
//!
//! Why so many are blocked: the models are the easy half. Several of these
//! papers publish their communities only as prose plus a supplementary file on
//! a journal host, and a community is its constraints: which strain lacks
//! which gene, what the medium supplies, what each member may take up.
//!
//! The catalogue is data, so it is kept as data: `published.json` beside this
//! file is `JSON.stringify(PUBLISHED_COMMUNITIES)` of the JS module, read into
//! the typed structures below. The parity test compares every field with the
//! JS, and `node rust/tools/fixtures-analysis.mjs --catalog` rewrites the copy
//! after the JS registry changes.

use std::sync::LazyLock;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Frameworks these papers introduced or used, for grouping in the UI
/// (`FRAMEWORKS` of the JS).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frameworks {
    pub joint_fba: &'static str,
    pub cfba: &'static str,
    pub optcom: &'static str,
    pub steadycom: &'static str,
    pub comets: &'static str,
}

impl Frameworks {
    /// Every framework, in the JS object's order.
    pub fn all(&self) -> [&'static str; 5] {
        [self.joint_fba, self.cfba, self.optcom, self.steadycom, self.comets]
    }
}

pub const FRAMEWORKS: Frameworks = Frameworks {
    joint_fba: "Joint-FBA",
    cfba: "cFBA",
    optcom: "OptCom",
    steadycom: "SteadyCom",
    comets: "COMETS",
};

/// The paper a community comes from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Paper {
    pub title: String,
    pub authors: String,
    pub year: u32,
    pub journal: String,
    pub citation: String,
    pub doi: String,
    /// The retrieved file the DOI was checked against, or `None` when it rests on
    /// recall alone. Always written out, so an unchecked DOI never passes for
    /// a checked one.
    pub doi_verified_from: Option<String>,
    /// One of [`FRAMEWORKS`].
    pub framework: String,
}

/// A reconstruction a community uses, and where its file comes from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelSource {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organism: Option<String>,
    pub reference: String,
    pub file: String,
    pub source: String,
    pub note: String,
}

/// A species' model: a reference to one of the community's `models`, or the
/// model described in place.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SpeciesModel {
    Ref {
        #[serde(rename = "ref")]
        reference: String,
    },
    Inline(ModelSource),
}

/// One organism of a published community.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PublishedSpecies {
    pub id: String,
    pub name: String,
    pub organism: String,
    pub role: String,
    /// `None` when no model is on record.
    pub model: Option<SpeciesModel>,
    /// Always `None` so far: abundances are computed, not prescribed.
    pub abundance: Option<f64>,
}

/// Who receives an exchanged metabolite: one species or several.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Recipients {
    One(String),
    Many(Vec<String>),
}

/// A metabolite one member passes on, as the paper reports it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PublishedExchange {
    pub metabolite: String,
    pub from: String,
    /// `None`: released to the medium.
    pub to: Option<Recipients>,
    /// Whether the exchange was checked against the authors' own artefact.
    pub verified: bool,
}

impl PublishedExchange {
    /// The receiving species (`[].concat(x.to ?? [])`).
    pub fn recipients(&self) -> Vec<&str> {
        match &self.to {
            None => Vec::new(),
            Some(Recipients::One(s)) => vec![s.as_str()],
            Some(Recipients::Many(v)) => v.iter().map(String::as_str).collect(),
        }
    }
}

/// An axis or a series of a published figure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FigureKey {
    pub key: String,
    pub label: String,
}

/// One panel of a published figure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FigurePanel {
    pub title: String,
    pub y_label: String,
    pub series: Vec<FigureKey>,
}

/// The figure a reproduction draws.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Figure {
    pub x: FigureKey,
    pub panels: Vec<FigurePanel>,
    pub caption: String,
}

/// Whether Coxeter reproduces a community.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReproductionStatus {
    Reproduced,
    Blocked,
}

impl ReproductionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ReproductionStatus::Reproduced => "reproduced",
            ReproductionStatus::Blocked => "blocked",
        }
    }
}

/// The numbers a reproduction must land on. Each community records the ones
/// its paper gives, and the rest are absent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Expected {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub growth_rate: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub growth_rates: Option<IndexMap<String, f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub growth_rate_range: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exchange_range: Option<[f64; 2]>,
    pub tolerance: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abundances: Option<IndexMap<String, f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abundance_note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub methane: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hydrogen_transfer: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formate_transfer: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mutant_monoculture: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phenylalanine_transfer: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scenario_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// What is reproduced, what is adapted, and how to run it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reproduction {
    pub status: ReproductionStatus,
    pub reproduces: Vec<String>,
    pub adaptations: Vec<String>,
    /// `None` for a blocked community, which claims nothing.
    pub expected: Option<Expected>,
    /// The JS module that builds it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builder: Option<String>,
    /// The script that runs it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_file: Option<String>,
    /// What is missing, for a blocked community, empty once reproduced.
    pub needs: Vec<String>,
}

/// One published community.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PublishedCommunity {
    pub id: String,
    pub name: String,
    pub paper: Paper,
    pub description: String,
    /// The reconstructions the species refer to (absent when none is on record).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<ModelSource>,
    pub species: Vec<PublishedSpecies>,
    /// Medium, objective and the like, in words, or `None` until reproduced.
    pub conditions: Option<IndexMap<String, String>>,
    pub exchanges: Vec<PublishedExchange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub figure: Option<Figure>,
    pub reproduction: Reproduction,
}

impl PublishedCommunity {
    /// The model a species' `{ref}` points at, or the one it describes.
    pub fn species_model<'a>(&'a self, species: &'a PublishedSpecies) -> Option<&'a ModelSource> {
        match species.model.as_ref()? {
            SpeciesModel::Ref { reference } => self.models.iter().find(|m| &m.id == reference),
            SpeciesModel::Inline(m) => Some(m),
        }
    }

    /// The community as the JS object (`JSON.parse(JSON.stringify(c))`).
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("the catalogue serialises")
    }
}

/// `PUBLISHED_COMMUNITIES`, in the JS order.
pub static PUBLISHED_COMMUNITIES: LazyLock<Vec<PublishedCommunity>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("published.json")).expect("the bundled catalogue of published communities parses")
});

/// Every published community.
pub fn published_communities() -> &'static [PublishedCommunity] {
    &PUBLISHED_COMMUNITIES
}

/// `findCommunity(id)`.
pub fn find_community(id: &str) -> Option<&'static PublishedCommunity> {
    PUBLISHED_COMMUNITIES.iter().find(|c| c.id == id)
}

/// The communities Coxeter reproduces.
pub fn reproduced_communities() -> Vec<&'static PublishedCommunity> {
    PUBLISHED_COMMUNITIES.iter().filter(|c| c.reproduction.status == ReproductionStatus::Reproduced).collect()
}

/// The communities that are on record but not attempted.
pub fn blocked_communities() -> Vec<&'static PublishedCommunity> {
    PUBLISHED_COMMUNITIES.iter().filter(|c| c.reproduction.status == ReproductionStatus::Blocked).collect()
}

/// The whole catalogue as the JS array.
pub fn published_communities_json() -> Value {
    Value::Array(PUBLISHED_COMMUNITIES.iter().map(PublishedCommunity::to_json).collect())
}
