//! Multi-species community model.
//!
//! Every species keeps its own namespace and its own compartments. A single
//! shared medium (compartment `s`) sits between the cells and carries every
//! metabolite that leaves or enters a cell. The result is an ordinary
//! [`Model`], so the FBA layer does not need to know a community is involved.
//!
//! ```text
//! sp@rxn            internal reaction of one species (per gDW of that species)
//! sp@EX_x_e         former exchange, now a transfer  x_e(sp) ⇌ a_sp · x(medium)
//! EX_x_s            community boundary: supply to / washout from the medium
//! COMMUNITY_GROWTH  only for the equal-growth objective
//! ```
//!
//! The abundance a_sp (gDW of that species per gDW of community) lives on the
//! medium side of each transfer, which converts a species-specific flux into a
//! community-basis flux: the fixed-abundance form of SteadyCom.

use std::sync::{Arc, LazyLock};

use indexmap::IndexMap;
use regex::Regex;
use serde_json::{Map, Value, json};

use crate::label_sources::label_sources_to_value;
use crate::model::{Model, ModelError, ReactionKind, strip_compartment};

pub const SHARED_COMPARTMENT: &str = "s";
pub const LINK_COMPARTMENT: &str = "link";
pub const COMMUNITY_GROWTH: &str = "COMMUNITY_GROWTH";
const SEP: char = '@';

/// The three linear objectives of ordinary community FBA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Objective {
    /// One µ shared by every species (fixed-abundance SteadyCom form).
    #[default]
    EqualGrowth,
    /// Σ a_i · µ_i: each species grows at its own rate.
    SumBiomass,
    /// One medium exchange: the yield of a product.
    Product,
}

impl Objective {
    pub fn as_str(self) -> &'static str {
        match self {
            Objective::EqualGrowth => "equal-growth",
            Objective::SumBiomass => "sum-biomass",
            Objective::Product => "product",
        }
    }
    /// The objective a JS name (`equal-growth`, `sum-biomass`, `product`) stands for.
    pub fn parse(s: &str) -> Option<Objective> {
        match s {
            "equal-growth" => Some(Objective::EqualGrowth),
            "sum-biomass" => Some(Objective::SumBiomass),
            "product" => Some(Objective::Product),
            _ => None,
        }
    }
}

/// `<species>@<id>`: the merged-model id of something that belongs to one species.
pub fn namespaced(species_id: &str, id: &str) -> String {
    format!("{species_id}{SEP}{id}")
}
/// `M@<base>`: a metabolite of the shared medium.
pub fn shared_metabolite(base: &str) -> String {
    format!("M{SEP}{base}")
}
/// `EX_<base>_s`: the community boundary exchange of a medium metabolite.
pub fn community_exchange(base: &str) -> String {
    format!("EX_{base}_s")
}

/// Split a namespaced id back into `(species, id)`.
pub fn split_id(id: &str) -> (Option<&str>, &str) {
    match id.find(SEP) {
        Some(i) => (Some(&id[..i]), &id[i + 1..]),
        None => (None, id),
    }
}

/// Base metabolite id of a merged metabolite, regardless of namespace.
pub fn base_of(merged_id: &str, compartment: &str) -> String {
    let rest = merged_id.strip_prefix("M@").unwrap_or(merged_id);
    let (_, id) = split_id(rest);
    strip_compartment(id, compartment)
}

/// What a reaction of the merged model is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReactionRole {
    Metabolic,
    Transport,
    Exchange,
    Biomass,
    Sink,
    /// A species' former exchange, now joining it to the shared medium.
    Transfer,
    /// `COMMUNITY_GROWTH`.
    Community,
}

impl ReactionRole {
    pub fn as_str(self) -> &'static str {
        match self {
            ReactionRole::Metabolic => "metabolic",
            ReactionRole::Transport => "transport",
            ReactionRole::Exchange => "exchange",
            ReactionRole::Biomass => "biomass",
            ReactionRole::Sink => "sink",
            ReactionRole::Transfer => "transfer",
            ReactionRole::Community => "community",
        }
    }
}

impl From<ReactionKind> for ReactionRole {
    fn from(k: ReactionKind) -> Self {
        match k {
            ReactionKind::Metabolic => ReactionRole::Metabolic,
            ReactionKind::Transport => ReactionRole::Transport,
            ReactionKind::Exchange => ReactionRole::Exchange,
            ReactionKind::Biomass => ReactionRole::Biomass,
            ReactionKind::Sink => ReactionRole::Sink,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReactionOwner {
    pub species_id: Option<String>,
    pub original_id: String,
    pub role: ReactionRole,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MetaboliteOwner {
    pub species_id: Option<String>,
    pub original_id: String,
    pub compartment: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Transfer {
    pub species_id: String,
    pub reaction_id: String,
    /// The species' own (namespaced) metabolite the transfer moves.
    pub local_id: String,
}

/// Supply limits of one medium component, by base metabolite id.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MediumLimit {
    pub lower: Option<f64>,
    pub upper: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct SpeciesEntry {
    /// Short id used as namespace, e.g. `"eco"`.
    pub id: String,
    pub model: Arc<Model>,
    /// gDW species / gDW community, before normalisation.
    pub abundance: f64,
    pub enabled: bool,
    pub color: String,
}

/// How a species is given to [`CommunityModel::new`]. Unset fields take the
/// same defaults as the JS constructor.
#[derive(Debug, Clone)]
pub struct SpeciesSpec {
    pub id: Option<String>,
    pub model: Arc<Model>,
    pub abundance: Option<f64>,
    pub enabled: Option<bool>,
    pub color: Option<String>,
}

impl SpeciesSpec {
    /// A species named `id` with its model. Every other field is left unset.
    pub fn new(id: &str, model: Arc<Model>) -> SpeciesSpec {
        SpeciesSpec { id: Some(id.into()), model, abundance: None, enabled: None, color: None }
    }
    /// Set the abundance (gDW species / gDW community, before normalisation).
    pub fn abundance(mut self, a: f64) -> Self {
        self.abundance = Some(a);
        self
    }
}

#[derive(Debug, Clone, Default)]
pub struct CommunitySpec {
    pub name: Option<String>,
    pub species: Vec<SpeciesSpec>,
    pub medium: IndexMap<String, MediumLimit>,
    pub objective: Option<Objective>,
    pub product_target: Option<String>,
}

/// One cross-feeding pairing: `from` feeds `to` (None = the medium itself).
#[derive(Debug, Clone, PartialEq)]
pub struct CrossFeedingLink {
    pub base: String,
    pub from: Option<String>,
    pub to: Option<String>,
    pub flux: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FluxShare {
    pub species_id: Option<String>,
    pub flux: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MetaboliteExchange {
    pub base: String,
    pub producers: Vec<FluxShare>,
    pub consumers: Vec<FluxShare>,
    pub total: f64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CrossFeeding {
    pub links: Vec<CrossFeedingLink>,
    pub per_metabolite: Vec<MetaboliteExchange>,
}

static TRAILING_BRACKET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*\[[^\]]*\]\s*$").unwrap());

#[derive(Debug, Clone)]
pub struct CommunityModel {
    pub name: String,
    pub objective_type: Objective,
    pub product_target: Option<String>,
    pub medium: IndexMap<String, MediumLimit>,
    pub species: Vec<SpeciesEntry>,

    // Derived by `build()`.
    pub merged: Model,
    pub reaction_owner: IndexMap<String, ReactionOwner>,
    pub metabolite_owner: IndexMap<String, MetaboliteOwner>,
    pub transfers_by_base: IndexMap<String, Vec<Transfer>>,
    pub species_compartments: IndexMap<String, Vec<String>>,
    pub biomass_ids: IndexMap<String, String>,
    pub abundances: IndexMap<String, f64>,
}

struct MediumInfo {
    base: String,
    name: String,
    formula: String,
    charge: f64,
    label_sources: Value,
}

impl CommunityModel {
    /// Build a community from its spec and merge it ([`CommunityModel::build`]).
    /// A species without an id is named `sp<n>`, one without an abundance gets
    /// an equal share, and one without a colour takes its model's cell colour.
    pub fn new(spec: CommunitySpec) -> Result<CommunityModel, ModelError> {
        let n = spec.species.len();
        let species = spec
            .species
            .into_iter()
            .enumerate()
            .map(|(i, s)| SpeciesEntry {
                id: s.id.filter(|x| !x.is_empty()).unwrap_or_else(|| format!("sp{}", i + 1)),
                abundance: s.abundance.filter(|a| a.is_finite()).unwrap_or(1.0 / n as f64),
                enabled: s.enabled != Some(false),
                color: s
                    .color
                    .filter(|c| !c.is_empty())
                    .or_else(|| Some(s.model.cell.color.clone()).filter(|c| !c.is_empty()))
                    .unwrap_or_else(|| "#334155".into()),
                model: s.model,
            })
            .collect();
        let mut community = CommunityModel {
            name: spec.name.filter(|x| !x.is_empty()).unwrap_or_else(|| "Community".into()),
            objective_type: spec.objective.unwrap_or_default(),
            product_target: spec.product_target.filter(|x| !x.is_empty()),
            medium: spec.medium,
            species,
            merged: Model::from_value(&json!({}))?,
            reaction_owner: IndexMap::new(),
            metabolite_owner: IndexMap::new(),
            transfers_by_base: IndexMap::new(),
            species_compartments: IndexMap::new(),
            biomass_ids: IndexMap::new(),
            abundances: IndexMap::new(),
        };
        community.build()?;
        Ok(community)
    }

    /// The enabled species, in order.
    pub fn active_species(&self) -> impl Iterator<Item = &SpeciesEntry> {
        self.species.iter().filter(|s| s.enabled)
    }

    /// The species with this id, enabled or not.
    pub fn species(&self, id: &str) -> Option<&SpeciesEntry> {
        self.species.iter().find(|s| s.id == id)
    }

    /// Normalised abundances across the enabled species (they sum to 1).
    pub fn normalised_abundances(&self) -> IndexMap<String, f64> {
        let active: Vec<&SpeciesEntry> = self.active_species().collect();
        let total: f64 = active.iter().fold(0.0, |sum, s| sum + s.abundance.max(0.0));
        active
            .iter()
            .map(|s| {
                let a = if total > 0.0 { s.abundance.max(0.0) / total } else { 1.0 / active.len() as f64 };
                (s.id.clone(), a)
            })
            .collect()
    }

    /// Rebuild the merged model. Call after changing species, abundances or
    /// objective.
    pub fn build(&mut self) -> Result<&Model, ModelError> {
        let abundances = self.normalised_abundances();
        let mut metabolites: Vec<Value> = Vec::new();
        let mut reactions: Vec<Map<String, Value>> = Vec::new();
        let mut compartments = Map::new();
        let mut medium_pool: IndexMap<String, MediumInfo> = IndexMap::new();

        self.reaction_owner.clear();
        self.metabolite_owner.clear();
        self.transfers_by_base.clear();
        self.species_compartments.clear();

        let active: Vec<SpeciesEntry> = self.active_species().cloned().collect();
        for sp in &active {
            let a = abundances[&sp.id];
            let model = &sp.model;
            self.species_compartments.insert(sp.id.clone(), model.compartment_ids());
            for (id, name) in &model.compartment_names {
                compartments.insert(namespaced(&sp.id, id), json!(name));
            }

            for met in model.metabolites.values() {
                let merged_id = namespaced(&sp.id, &met.id);
                let mut raw = Map::new();
                raw.insert("id".into(), json!(merged_id));
                // The species' own name for it. Derived from the merged id it
                // would come out as `eco@g6p`, and the map would print that.
                raw.insert("base".into(), json!(met.base));
                raw.insert("name".into(), json!(met.name));
                raw.insert("labelSources".into(), label_sources_to_value(&met.label_sources));
                raw.insert("annotation".into(), met.annotation.clone());
                raw.insert("notes".into(), met.notes.clone());
                raw.insert("compartment".into(), json!(met.compartment));
                raw.insert("formula".into(), json!(met.formula));
                raw.insert("charge".into(), json!(met.charge));
                if let Some(p) = met.explicit_primary {
                    raw.insert("primary".into(), json!(p));
                }
                metabolites.push(Value::Object(raw));
                self.metabolite_owner.insert(
                    merged_id,
                    MetaboliteOwner {
                        species_id: Some(sp.id.clone()),
                        original_id: met.id.clone(),
                        compartment: met.compartment.clone(),
                    },
                );
            }

            for rxn in model.reactions.values() {
                let merged_id = namespaced(&sp.id, &rxn.id);
                let mut stoich: IndexMap<String, f64> = IndexMap::new();
                for (mid, coef) in &rxn.metabolites {
                    stoich.insert(namespaced(&sp.id, mid), *coef);
                }
                let mut role = ReactionRole::from(rxn.kind);
                if rxn.kind == ReactionKind::Exchange && rxn.metabolites.len() == 1 {
                    // Re-point the exchange at the shared medium: it stops
                    // being a boundary and becomes a real transfer between
                    // this cell and the environment.
                    let (local_id, coef) = rxn.metabolites.iter().next().map(|(k, v)| (k.clone(), *v)).unwrap();
                    let met = &model.metabolites[&local_id];
                    let base = met.base.clone();
                    let shared = shared_metabolite(&base);
                    let prior = stoich.get(&shared).copied().unwrap_or(0.0);
                    stoich.insert(shared, prior - coef * a);
                    role = ReactionRole::Transfer;
                    medium_pool.entry(base.clone()).or_insert_with(|| MediumInfo {
                        base: base.clone(),
                        name: TRAILING_BRACKET.replace(&met.name, "").into_owned(),
                        formula: met.formula.clone(),
                        charge: met.charge,
                        label_sources: label_sources_to_value(&met.label_sources),
                    });
                    self.transfers_by_base.entry(base).or_default().push(Transfer {
                        species_id: sp.id.clone(),
                        reaction_id: merged_id.clone(),
                        local_id: namespaced(&sp.id, &local_id),
                    });
                }
                let mut raw = Map::new();
                raw.insert("id".into(), json!(merged_id));
                raw.insert("name".into(), json!(rxn.name));
                raw.insert("labelSources".into(), label_sources_to_value(&rxn.label_sources));
                raw.insert("metabolites".into(), json!(stoich));
                raw.insert("lower_bound".into(), json!(rxn.lower_bound));
                raw.insert("upper_bound".into(), json!(rxn.upper_bound));
                raw.insert("subsystem".into(), json!(rxn.subsystem));
                raw.insert("pathway".into(), json!(rxn.pathway));
                raw.insert("gene_reaction_rule".into(), json!(rxn.gene_reaction_rule));
                raw.insert("layout".into(), rxn.layout.clone().unwrap_or(Value::Null));
                raw.insert("notes".into(), rxn.notes.clone());
                raw.insert("annotation".into(), rxn.annotation.clone());
                reactions.push(raw);
                self.reaction_owner.insert(
                    merged_id,
                    ReactionOwner { species_id: Some(sp.id.clone()), original_id: rxn.id.clone(), role },
                );
            }
        }

        // Shared medium metabolites and community boundary exchanges
        compartments.insert(SHARED_COMPARTMENT.into(), json!("Medio compartido"));
        for info in medium_pool.values() {
            let id = shared_metabolite(&info.base);
            metabolites.push(json!({
                "id": id,
                "base": info.base,
                "name": info.name,
                "labelSources": info.label_sources,
                "compartment": SHARED_COMPARTMENT,
                "formula": info.formula,
                "charge": info.charge,
            }));
            self.metabolite_owner.insert(
                id.clone(),
                MetaboliteOwner { species_id: None, original_id: info.base.clone(), compartment: SHARED_COMPARTMENT.into() },
            );
            let ex_id = community_exchange(&info.base);
            let limits = self.medium.get(&info.base).copied().unwrap_or_default();
            let mut raw = Map::new();
            raw.insert("id".into(), json!(ex_id));
            raw.insert("name".into(), json!(format!("{} exchange (medium)", info.name)));
            raw.insert("metabolites".into(), json!({ id: -1 }));
            raw.insert("lower_bound".into(), json!(limits.lower.filter(|v| v.is_finite()).unwrap_or(0.0)));
            raw.insert("upper_bound".into(), json!(limits.upper.filter(|v| v.is_finite()).unwrap_or(1000.0)));
            raw.insert("subsystem".into(), json!("Community exchange"));
            reactions.push(raw);
            self.reaction_owner.insert(
                ex_id.clone(),
                ReactionOwner { species_id: None, original_id: ex_id, role: ReactionRole::Exchange },
            );
        }

        // Objective wiring
        let mut objective = Map::new();
        self.biomass_ids.clear();
        for sp in &active {
            if let Some(bio) = sp.model.biomass_reaction_id() {
                self.biomass_ids.insert(sp.id.clone(), namespaced(&sp.id, &bio));
            }
        }

        if self.objective_type == Objective::EqualGrowth && !self.biomass_ids.is_empty() {
            let mut link_mets = Map::new();
            for (sp_id, rxn_id) in &self.biomass_ids {
                let link_met = format!("GROWTHLINK{SEP}{sp_id}");
                metabolites.push(json!({
                    "id": link_met,
                    "name": format!("Growth link ({sp_id})"),
                    "compartment": LINK_COMPARTMENT,
                    "formula": "",
                    "charge": 0,
                }));
                self.metabolite_owner.insert(
                    link_met.clone(),
                    MetaboliteOwner { species_id: Some(sp_id.clone()), original_id: link_met.clone(), compartment: LINK_COMPARTMENT.into() },
                );
                if let Some(target) = reactions.iter_mut().find(|r| r.get("id").and_then(Value::as_str) == Some(rxn_id)) {
                    if let Some(Value::Object(m)) = target.get_mut("metabolites") {
                        m.insert(link_met.clone(), json!(1));
                    }
                }
                link_mets.insert(link_met, json!(-1));
            }
            reactions.push(
                json!({
                    "id": COMMUNITY_GROWTH,
                    "name": "Community growth rate",
                    "metabolites": link_mets,
                    "lower_bound": 0,
                    "upper_bound": 1000,
                    "subsystem": "Community",
                })
                .as_object()
                .cloned()
                .unwrap(),
            );
            self.reaction_owner.insert(
                COMMUNITY_GROWTH.into(),
                ReactionOwner { species_id: None, original_id: COMMUNITY_GROWTH.into(), role: ReactionRole::Community },
            );
            objective.insert(COMMUNITY_GROWTH.into(), json!(1));
        } else if self.objective_type == Objective::Product && self.product_target.is_some() {
            let ex_id = community_exchange(self.product_target.as_deref().unwrap());
            if reactions.iter().any(|r| r.get("id").and_then(Value::as_str) == Some(&ex_id)) {
                objective.insert(ex_id, json!(1));
            }
        } else {
            for (sp_id, rxn_id) in &self.biomass_ids {
                objective.insert(rxn_id.clone(), json!(abundances.get(sp_id).copied().unwrap_or(1.0)));
            }
        }

        compartments.insert(LINK_COMPARTMENT.into(), json!("Acoplamiento (virtual)"));

        self.merged = Model::from_value(&json!({
            "id": "community",
            "name": self.name,
            "compartments": compartments,
            "metabolites": metabolites,
            "reactions": reactions,
            "objective": objective,
        }))?;
        self.abundances = abundances;
        Ok(&self.merged)
    }

    /// Reactions that move a metabolite between a cell and the shared medium.
    pub fn transfer_reactions(&self) -> Vec<(String, Transfer)> {
        self.transfers_by_base
            .iter()
            .flat_map(|(base, list)| list.iter().map(move |t| (base.clone(), t.clone())))
            .collect()
    }

    /// Growth rate of each species from a solved flux map.
    pub fn growth_rates(&self, fluxes: &IndexMap<String, f64>) -> IndexMap<String, f64> {
        self.biomass_ids
            .iter()
            .map(|(sp, rxn)| (sp.clone(), fluxes.get(rxn).copied().unwrap_or(0.0)))
            .collect()
    }

    /// Who feeds whom. For every medium metabolite, split the transfer fluxes
    /// into secretion (positive, community basis) and uptake (negative), then
    /// pair them up proportionally so the map can draw cross-feeding arrows.
    pub fn cross_feeding(&self, fluxes: &IndexMap<String, f64>, threshold: f64) -> CrossFeeding {
        let mut links = Vec::new();
        let mut per_metabolite = Vec::new();
        let sum = |list: &[FluxShare]| list.iter().fold(0.0, |s, x| s + x.flux);

        for (base, transfers) in &self.transfers_by_base {
            let mut producers = Vec::new();
            let mut consumers = Vec::new();
            for t in transfers {
                let v = fluxes.get(&t.reaction_id).copied().unwrap_or(0.0);
                let a = self.abundances.get(&t.species_id).copied().unwrap_or(1.0);
                let community_flux = v * a; // mmol per gDW community per h
                if community_flux > threshold {
                    producers.push(FluxShare { species_id: Some(t.species_id.clone()), flux: community_flux });
                } else if community_flux < -threshold {
                    consumers.push(FluxShare { species_id: Some(t.species_id.clone()), flux: -community_flux });
                }
            }
            let env_flux = fluxes.get(&community_exchange(base)).copied().unwrap_or(0.0);
            // Negative community exchange = supplied by the medium.
            if env_flux < -threshold {
                producers.push(FluxShare { species_id: None, flux: -env_flux });
            } else if env_flux > threshold {
                consumers.push(FluxShare { species_id: None, flux: env_flux });
            }

            if producers.is_empty() || consumers.is_empty() {
                if !producers.is_empty() || !consumers.is_empty() {
                    let p = sum(&producers);
                    // `sum(producers) || sum(consumers)`
                    let total = if p != 0.0 && !p.is_nan() { p } else { sum(&consumers) };
                    per_metabolite.push(MetaboliteExchange { base: base.clone(), producers, consumers, total });
                }
                continue;
            }
            let total_out = sum(&producers);
            let total_in = sum(&consumers);
            let total = total_out.min(total_in);
            for p in &producers {
                for q in &consumers {
                    let share = (p.flux / total_out) * (q.flux / total_in) * total;
                    if share > threshold {
                        links.push(CrossFeedingLink {
                            base: base.clone(),
                            from: p.species_id.clone(),
                            to: q.species_id.clone(),
                            flux: share,
                        });
                    }
                }
            }
            per_metabolite.push(MetaboliteExchange { base: base.clone(), producers, consumers, total });
        }
        // Stable sort, largest first, as Array#sort with (a, b) => b.flux - a.flux.
        links.sort_by(|a, b| b.flux.partial_cmp(&a.flux).unwrap_or(std::cmp::Ordering::Equal));
        CrossFeeding { links, per_metabolite }
    }

    /// Set the supply limit of a medium component (positive = available).
    pub fn set_medium_supply(&mut self, base: &str, rate: f64) -> &mut Self {
        let entry = self.medium.entry(base.into()).or_default();
        entry.lower = Some(-rate.abs());
        self
    }

    pub fn to_json(&self) -> Value {
        let medium: Map<String, Value> = self
            .medium
            .iter()
            .map(|(k, l)| {
                let mut m = Map::new();
                if let Some(v) = l.lower {
                    m.insert("lower".into(), json!(v));
                }
                if let Some(v) = l.upper {
                    m.insert("upper".into(), json!(v));
                }
                (k.clone(), Value::Object(m))
            })
            .collect();
        json!({
            "name": self.name,
            "objective": self.objective_type.as_str(),
            "productTarget": self.product_target,
            "medium": medium,
            "species": self.species.iter().map(|s| json!({
                "id": s.id, "abundance": s.abundance, "enabled": s.enabled,
                "model": s.model.id, "color": s.color,
            })).collect::<Vec<_>>(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toy(id: &str, product: &str) -> Arc<Model> {
        Arc::new(
            Model::from_value(&json!({
                "id": id,
                "metabolites": [
                    {"id": "glc_e", "compartment": "e", "name": "Glucose [e]"},
                    {"id": "glc_c", "compartment": "c"},
                    {"id": format!("{product}_c"), "compartment": "c"},
                    {"id": format!("{product}_e"), "compartment": "e"}
                ],
                "reactions": [
                    {"id": "EX_glc_e", "metabolites": {"glc_e": -1}, "lower_bound": -10},
                    {"id": format!("EX_{product}_e"), "metabolites": {format!("{product}_e"): -1}, "lower_bound": 0},
                    {"id": "GLCt", "metabolites": {"glc_e": -1, "glc_c": 1}},
                    {"id": "OUT", "metabolites": {format!("{product}_c"): -1, format!("{product}_e"): 1}},
                    {"id": "BIOMASS", "metabolites": {"glc_c": -1, format!("{product}_c"): 1}, "objective_coefficient": 1}
                ]
            }))
            .unwrap(),
        )
    }

    #[test]
    fn exchanges_become_transfers_to_the_medium() {
        let c = CommunityModel::new(CommunitySpec {
            species: vec![SpeciesSpec::new("a", toy("A", "ac")).abundance(3.0), SpeciesSpec::new("b", toy("B", "lac")).abundance(1.0)],
            ..Default::default()
        })
        .unwrap();
        assert_eq!(c.abundances["a"], 0.75);
        let t = &c.merged.reactions["a@EX_glc_e"];
        assert_eq!(t.metabolites["M@glc"], 0.75);
        assert_eq!(c.reaction_owner["a@EX_glc_e"].role, ReactionRole::Transfer);
        assert!(c.merged.reactions.contains_key("EX_glc_s"));
        assert_eq!(c.merged.metabolites["M@glc"].name, "Glucose");
        assert_eq!(c.merged.reactions[COMMUNITY_GROWTH].metabolites.len(), 2);
        assert_eq!(c.merged.objective_reactions(), [(COMMUNITY_GROWTH.to_string(), 1.0)]);
        assert_eq!(c.merged.metabolites["a@glc_c"].base, "glc");
        assert_eq!(base_of("M@glc", "s"), "glc");
        assert_eq!(base_of("a@glc_c", "c"), "glc");
    }

    #[test]
    fn cross_feeding_pairs_producers_with_consumers() {
        let c = CommunityModel::new(CommunitySpec {
            species: vec![SpeciesSpec::new("a", toy("A", "ac")), SpeciesSpec::new("b", toy("B", "ac"))],
            objective: Some(Objective::SumBiomass),
            ..Default::default()
        })
        .unwrap();
        let fluxes: IndexMap<String, f64> =
            [("a@EX_ac_e".to_string(), 2.0), ("b@EX_ac_e".to_string(), -2.0)].into_iter().collect();
        let cf = c.cross_feeding(&fluxes, 1e-6);
        assert_eq!(cf.links.len(), 1);
        assert_eq!(cf.links[0].from.as_deref(), Some("a"));
        assert_eq!(cf.links[0].to.as_deref(), Some("b"));
        assert_eq!(cf.links[0].flux, 1.0);
    }
}
