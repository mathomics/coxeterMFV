//! Run SteadyCom on a published community.
//!
//! Ports `src/communities/steadycom-runtime.js` (and `exchangeBase` of
//! `src/communities/steadycom-ecoli.js`, which it imports).
//!
//! `steadycom` holds the method, and this holds what the application needs to hand
//! it. A published community ships everything needed:
//!
//! ```text
//!   model-<member>.json   the member model with its own bounds already applied
//!   media.csv             metabolite, exchange reaction, lower and upper bound
//!                         for the whole community
//!   bounds.csv            complete reaction ownership and bounds for a joint
//!                         model with an explicit shared [u] compartment
//! ```
//!
//! so the spec is assembled from those rather than rebuilt from the paper.

use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

use coxeter_core::community::{
    COMMUNITY_GROWTH, LINK_COMPARTMENT, MetaboliteOwner, Objective, ReactionOwner, ReactionRole, Transfer,
};
use coxeter_core::js;
use coxeter_core::model::{Model, strip_compartment};
use indexmap::{IndexMap, IndexSet};
use regex::Regex;
use serde_json::{Map, Value, json};

use crate::gut9_figure4::{JointView, fiber_constraints};
use crate::steadycom::{
    BiomassTerm, Constraint, ConstraintTerm, ExchangeBounds, Fiber, JointCommunity, JointMetadata, JointReaction, Member,
    SteadyComError, SteadyComSpec,
};

static EXTERNAL_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)extracellular|external").unwrap());
static TRAILING_BRACKET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[[^\]]+\]$").unwrap());
static COMMUNITY_BOUNDARY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(UT|EX)_(.+)\(u\)$").unwrap());
static BRACKET_COMPARTMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[([a-z])(?:_[^\]]+)?\]$").unwrap());
static SUFFIX_COMPARTMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"_([a-z][a-z0-9]?)$").unwrap());

fn invalid<T>(message: impl Into<String>) -> Result<T, SteadyComError> {
    Err(SteadyComError::Invalid(message.into()))
}

/// A member model as the runtime receives it: `{ id, model }`.
#[derive(Debug, Clone)]
pub struct MemberModel {
    pub id: String,
    pub model: Arc<Model>,
}

impl MemberModel {
    /// A member named `id` with its model.
    pub fn new(id: &str, model: Model) -> MemberModel {
        MemberModel { id: id.into(), model: Arc::new(model) }
    }
}

/// `EX_glc_e` → `glc`: the metabolite an exchange reaction trades.
pub fn exchange_base(rid: &str) -> String {
    let s = rid.strip_prefix("EX_").unwrap_or(rid);
    s.strip_suffix("_e").unwrap_or(s).to_string()
}

/// Single-metabolite extracellular boundaries, keyed by the metabolite traded.
pub fn exchange_index(model: &Model) -> Result<IndexMap<String, String>, SteadyComError> {
    let mut out: IndexMap<String, String> = IndexMap::new();
    for (rid, reaction) in &model.reactions {
        if reaction.metabolites.len() != 1 {
            continue;
        }
        let (mid, coefficient) = reaction.metabolites.iter().next().map(|(k, v)| (k.as_str(), *v)).unwrap();
        let metabolite = model.metabolites.get(mid);
        let external = metabolite.is_some_and(|m| {
            m.compartment == "e"
                || EXTERNAL_NAME.is_match(model.compartment_names.get(&m.compartment).map_or("", String::as_str))
        });
        if !external && !rid.starts_with("EX_") {
            continue;
        }
        if coefficient != -1.0 {
            return invalid(format!("{rid}: SteadyCom exchange requires one consumed metabolite with coefficient -1."));
        }
        let base = match metabolite {
            Some(m) => TRAILING_BRACKET.replace(&strip_compartment(mid, &m.compartment), "").into_owned(),
            None => exchange_base(rid),
        };
        if out.get(&base).is_some_and(|r| r != rid) {
            return invalid(format!("{base}: more than one exchange reaction needs an explicit community formulation."));
        }
        out.insert(base, rid.clone());
    }
    js::js_key_order(&mut out);
    Ok(out)
}

/// Read `media.csv`: `metabolite,exchange_reaction,lower_bound,upper_bound`.
///
/// A negative lower bound is uptake, and SteadyCom wants it as a positive
/// budget, so the sign is flipped here. Rows that only allow secretion carry
/// no budget and are left out. With members, each row's exchange reaction is
/// resolved to the metabolite those members actually trade through it.
pub fn parse_media_csv(text: &str, members: &[MemberModel]) -> Result<IndexMap<String, f64>, SteadyComError> {
    let mut uptake: IndexMap<String, f64> = IndexMap::new();
    let trimmed = text.trim();
    let mut lines = trimmed.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l));
    let header: Vec<String> = lines.next().unwrap_or("").split(',').map(|s| s.trim().to_string()).collect();
    let col = |name: &str| header.iter().position(|h| h == name);
    let (i_met, i_lower, i_exchange) = (col("metabolite"), col("lower_bound"), col("exchange_reaction"));
    let mut bases_by_reaction: IndexMap<String, String> = IndexMap::new();
    for member in members {
        for (base, reaction) in exchange_index(&member.model)? {
            if bases_by_reaction.get(&reaction).is_some_and(|b| *b != base) {
                return invalid(format!("{reaction}: member exports disagree about the exchanged metabolite."));
            }
            bases_by_reaction.insert(reaction, base);
        }
    }
    let (Some(i_met), Some(i_lower)) = (i_met, i_lower) else {
        return invalid("media.csv has no \"metabolite\" and \"lower_bound\" columns.");
    };
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let cells: Vec<&str> = line.split(',').collect();
        let lower = cells.get(i_lower).map_or(f64::NAN, |c| js::string_to_number(c));
        if !lower.is_finite() || lower >= 0.0 {
            continue;
        }
        let reaction = i_exchange.and_then(|i| cells.get(i)).map(|c| c.trim().to_string());
        let met_cell = cells.get(i_met).map(|c| c.trim().to_string());
        let base = if members.is_empty() {
            met_cell.clone()
        } else {
            reaction.as_ref().and_then(|r| bases_by_reaction.get(r).cloned())
        };
        let Some(base) = base.filter(|b| !b.is_empty()) else {
            let what = reaction.or(met_cell).unwrap_or_else(|| "undefined".into());
            return invalid(format!("{what}: the medium uptake has no matching member exchange."));
        };
        uptake.insert(base, -lower);
    }
    js::js_key_order(&mut uptake);
    Ok(uptake)
}

/// The SteadyCom problem for a community of separate member models.
pub fn steady_com_spec(
    members: &[MemberModel],
    community_uptake: &IndexMap<String, f64>,
    total_biomass: f64,
) -> Result<SteadyComSpec, SteadyComError> {
    if members.is_empty() {
        return invalid("A SteadyCom problem needs at least one member.");
    }
    let mut out = Vec::new();
    for MemberModel { id, model } in members {
        let Some(biomass) = model.biomass_reaction_id() else {
            return invalid(format!("{id} has no biomass reaction, so its growth cannot be coupled."));
        };
        out.push(Member {
            id: id.clone(),
            name: None,
            biomass_reaction: biomass,
            atp_maintenance: None,
            model: Some(model.clone()),
            exchange_for: exchange_index(model)?,
        });
    }
    Ok(SteadyComSpec {
        members: out,
        total_biomass,
        biomass_cap: None,
        community_uptake: community_uptake.clone(),
        community_exchange_bounds: IndexMap::new(),
        joint: None,
    })
}

/// Options of [`joint_steady_com_spec`].
#[derive(Debug, Clone, Copy)]
pub struct JointSpecOptions {
    pub total_biomass: f64,
    pub fiber: Option<Fiber>,
}

impl Default for JointSpecOptions {
    fn default() -> Self {
        JointSpecOptions { total_biomass: 1.0, fiber: None }
    }
}

struct BoundsRow {
    species: String,
    lower: f64,
    upper: f64,
    objective: f64,
}

/// Reassemble a published joint community from complete member exports and
/// its complete bounds table. Shared `[u]` metabolites remain shared. They are
/// never stripped or duplicated into separate member mass balances.
pub fn joint_steady_com_spec(
    members: &[MemberModel],
    bounds_csv: &str,
    options: JointSpecOptions,
) -> Result<SteadyComSpec, SteadyComError> {
    if members.is_empty() {
        return invalid("A joint SteadyCom problem needs member models.");
    }
    if !options.total_biomass.is_finite() || options.total_biomass <= 0.0 {
        return invalid("Total biomass must be positive and finite.");
    }
    if let Some(f) = options.fiber {
        if ![f.bacteroides, f.faecalibacterium].iter().all(|v| v.is_finite() && *v > 0.0) {
            return invalid("Both fiber uptake limits must be positive and finite.");
        }
    }
    let bounds = joint_bounds(bounds_csv)?;
    let mut reactions: IndexMap<String, JointReaction> = IndexMap::new();
    let mut metabolites: IndexMap<String, Map<String, Value>> = IndexMap::new();
    let mut reaction_owners: IndexMap<String, Option<String>> = IndexMap::new();
    let mut bases: IndexMap<String, String> = IndexMap::new();
    let mut species_exchange: IndexMap<String, String> = IndexMap::new();
    let mut community_uptake: IndexMap<String, String> = IndexMap::new();
    let mut community_export: IndexMap<String, String> = IndexMap::new();
    let mut member_ids: HashSet<&str> = HashSet::new();
    let mut descriptors = Vec::new();

    for MemberModel { id, model } in members {
        if id.is_empty() || id == "comm" || !member_ids.insert(id) {
            return invalid(format!("Invalid or duplicate member {id}."));
        }
        let biomass = model.biomass_reaction_id().filter(|b| model.reactions.contains_key(b));
        let Some(biomass) = biomass else {
            return invalid(format!("{id} has no valid biomass reaction."));
        };
        descriptors.push(Member {
            id: id.clone(),
            name: Some(model.name.clone()),
            biomass_reaction: biomass,
            atp_maintenance: None,
            model: None,
            exchange_for: IndexMap::new(),
        });
        for metabolite in model.metabolites.values() {
            let shared = metabolite.id.ends_with("[u]");
            let mut doc = match metabolite.to_json() {
                Value::Object(m) => m,
                _ => Map::new(),
            };
            doc.insert("species".into(), json!(if shared { "comm" } else { id.as_str() }));
            if let Some(previous) = metabolites.get(&metabolite.id) {
                if serde_json::to_string(previous).ok() != serde_json::to_string(&doc).ok() {
                    return invalid(format!("{}: member exports disagree about the shared metabolite.", metabolite.id));
                }
            }
            metabolites.insert(metabolite.id.clone(), doc);
            if shared {
                bases.insert(metabolite.id.clone(), metabolite.id[..metabolite.id.len() - 3].to_string());
            }
        }
        for reaction in model.reactions.values() {
            let row = bounds.get(&reaction.id).filter(|r| r.species == *id);
            let Some(row) = row else {
                return invalid(format!("{}: no matching bounds for member {id}.", reaction.id));
            };
            if reactions.contains_key(&reaction.id) {
                return invalid(format!("Duplicate joint reaction {}.", reaction.id));
            }
            reactions.insert(
                reaction.id.clone(),
                JointReaction {
                    id: reaction.id.clone(),
                    name: reaction.name.clone(),
                    subsystem: reaction.subsystem.clone(),
                    metabolites: reaction.metabolites.clone(),
                    species: id.clone(),
                    lower_bound: row.lower,
                    upper_bound: row.upper,
                    objective: row.objective,
                },
            );
            reaction_owners.insert(reaction.id.clone(), Some(id.clone()));
            let shared: Vec<(&String, f64)> =
                reaction.metabolites.iter().filter(|(m, _)| m.ends_with("[u]")).map(|(m, c)| (m, *c)).collect();
            if shared.is_empty() {
                continue;
            }
            let local: Vec<(&String, f64)> =
                reaction.metabolites.iter().filter(|(m, _)| !m.ends_with("[u]")).map(|(m, c)| (m, *c)).collect();
            if shared.len() != 1 || shared[0].1 != 1.0 || local.len() != 1 || local[0].1 != -1.0 {
                return invalid(format!("{}: unsupported species-to-community exchange structure.", reaction.id));
            }
            let key = format!("{}\u{0}{id}", shared[0].0);
            if species_exchange.contains_key(&key) {
                return invalid(format!("{}: duplicate community exchange for {id}.", shared[0].0));
            }
            species_exchange.insert(key, reaction.id.clone());
        }
    }

    for (rid, row) in &bounds {
        if row.species != "comm" {
            if !reactions.contains_key(rid) {
                return invalid(format!("{rid}: bounds table names a missing member reaction."));
            }
            continue;
        }
        let Some(m) = COMMUNITY_BOUNDARY.captures(rid) else {
            return invalid(format!("{rid}: unsupported community boundary identifier."));
        };
        let met = format!("{}[u]", &m[2]);
        bases.insert(met.clone(), m[2].to_string());
        if !metabolites.contains_key(&met) {
            let doc = json!({"id": met, "name": met, "compartment": "u", "species": "comm"});
            metabolites.insert(met.clone(), doc.as_object().cloned().unwrap());
        }
        let uptake = &m[1] == "UT";
        let index = if uptake { &mut community_uptake } else { &mut community_export };
        if index.contains_key(&met) {
            return invalid(format!("{met}: duplicate community {} boundary.", if uptake { "uptake" } else { "export" }));
        }
        index.insert(met.clone(), rid.clone());
        reactions.insert(
            rid.clone(),
            JointReaction {
                id: rid.clone(),
                name: rid.clone(),
                subsystem: String::new(),
                metabolites: [(met.clone(), if uptake { 1.0 } else { -1.0 })].into_iter().collect(),
                species: "comm".into(),
                lower_bound: row.lower,
                upper_bound: row.upper,
                objective: row.objective,
            },
        );
        reaction_owners.insert(rid.clone(), None);
    }
    for met in bases.keys() {
        if !community_uptake.contains_key(met) || !community_export.contains_key(met) {
            return invalid(format!("{met}: the bounds table lacks a community uptake or export boundary."));
        }
    }
    let constraints = match options.fiber {
        Some(fiber) => {
            let view = JointView { reactions: &reactions, species_exchange: &species_exchange, species: Vec::new() };
            fiber_constraints(&view, fiber)?
        }
        None => Vec::new(),
    };
    Ok(SteadyComSpec {
        members: descriptors,
        total_biomass: options.total_biomass,
        biomass_cap: None,
        community_uptake: IndexMap::new(),
        community_exchange_bounds: IndexMap::new(),
        joint: Some(JointCommunity {
            reactions,
            metabolites,
            community_tag: "comm".into(),
            constraints,
            metadata: Some(JointMetadata {
                reaction_owners,
                bases,
                species_exchange,
                community_uptake,
                community_export,
                fiber: options.fiber,
            }),
        }),
    })
}

fn joint_bounds(text: &str) -> Result<IndexMap<String, BoundsRow>, SteadyComError> {
    let trimmed = text.trim();
    let mut lines = trimmed.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l));
    let header = csv_row(lines.next().unwrap_or(""))?;
    let required = ["reaction", "species", "lower_bound", "upper_bound", "objective_coefficient"];
    let columns: Vec<Option<usize>> = required.iter().map(|name| header.iter().position(|h| h == name)).collect();
    if columns.iter().any(Option::is_none) {
        return invalid("Joint bounds.csv is missing required columns.");
    }
    let mut out = IndexMap::new();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let fields = csv_row(line)?;
        let get = |k: usize| columns[k].and_then(|i| fields.get(i)).map(String::as_str);
        let (id, species) = (get(0).unwrap_or(""), get(1).unwrap_or(""));
        let texts = [get(2), get(3), get(4)];
        let numbers: Vec<f64> = texts.iter().map(|t| t.map_or(f64::NAN, js::string_to_number)).collect();
        if id.is_empty()
            || species.is_empty()
            || texts.iter().any(|t| t.is_none_or(str::is_empty))
            || !numbers.iter().all(|n| n.is_finite())
            || numbers[0] > numbers[1]
        {
            let id = if id.is_empty() { "Unknown reaction" } else { id };
            return invalid(format!("{id}: invalid joint bounds row."));
        }
        if out.contains_key(id) {
            return invalid(format!("Duplicate joint bounds row {id}."));
        }
        out.insert(id.to_string(), BoundsRow { species: species.to_string(), lower: numbers[0], upper: numbers[1], objective: numbers[2] });
    }
    Ok(out)
}

fn csv_row(line: &str) -> Result<Vec<String>, SteadyComError> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '"' {
            if quoted && chars.get(i + 1) == Some(&'"') {
                field.push('"');
                i += 1;
            } else {
                quoted = !quoted;
            }
        } else if ch == ',' && !quoted {
            fields.push(field.trim().to_string());
            field.clear();
        } else {
            field.push(ch);
        }
        i += 1;
    }
    if quoted {
        return invalid("Unclosed quote in joint bounds.csv.");
    }
    fields.push(field.trim().to_string());
    Ok(fields)
}

// Snapshots

fn constraint_to_json(c: &Constraint) -> Value {
    let mut out = Map::new();
    if let Some(n) = &c.name {
        out.insert("name".into(), json!(n));
    }
    out.insert(
        "terms".into(),
        Value::Array(
            c.terms
                .iter()
                .map(|t| {
                    let mut m = Map::new();
                    m.insert("reaction".into(), json!(t.reaction));
                    m.insert("coef".into(), json!(t.coef));
                    if t.uptake_only {
                        m.insert("part".into(), json!("uptake"));
                    }
                    Value::Object(m)
                })
                .collect(),
        ),
    );
    out.insert(
        "biomass".into(),
        Value::Array(c.biomass.iter().map(|b| json!({"species": b.species, "coef": b.coef})).collect()),
    );
    if let Some(lo) = c.lo {
        out.insert("lo".into(), json!(lo));
    }
    if let Some(hi) = c.hi {
        out.insert("hi".into(), json!(hi));
    }
    Value::Object(out)
}

fn constraint_from_json(v: &Value) -> Constraint {
    Constraint {
        name: v.get("name").and_then(Value::as_str).map(str::to_string),
        terms: v
            .get("terms")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|t| ConstraintTerm {
                        reaction: t["reaction"].as_str().unwrap_or_default().to_string(),
                        coef: js::to_number(&t["coef"]),
                        uptake_only: t.get("part").and_then(Value::as_str) == Some("uptake"),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        biomass: v
            .get("biomass")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|b| BiomassTerm { species: b["species"].as_str().unwrap_or_default().to_string(), coef: js::to_number(&b["coef"]) })
                    .collect()
            })
            .unwrap_or_default(),
        lo: v.get("lo").and_then(Value::as_f64),
        hi: v.get("hi").and_then(Value::as_f64),
    }
}

fn pairs<V: Clone + Into<Value>>(m: &IndexMap<String, V>) -> Value {
    Value::Array(m.iter().map(|(k, v)| json!([k, v.clone().into()])).collect())
}

fn read_pairs(v: &Value) -> Vec<(String, Value)> {
    v.as_array()
        .map(|a| a.iter().filter_map(|p| Some((p.get(0)?.as_str()?.to_string(), p.get(1)?.clone()))).collect())
        .unwrap_or_default()
}

/// A self-contained JSON snapshot of a spec (`serializeSteadyComSpec`): member
/// models become their documents and maps become `[key, value]` lists, so the
/// spec can be saved with a scenario or handed to another process.
pub fn serialize_steady_com_spec(spec: &SteadyComSpec) -> Value {
    let members: Vec<Value> = spec
        .members
        .iter()
        .map(|m| {
            let mut out = Map::new();
            out.insert("id".into(), json!(m.id));
            if let Some(n) = &m.name {
                out.insert("name".into(), json!(n));
            }
            out.insert("biomassReaction".into(), json!(m.biomass_reaction));
            if let Some(a) = &m.atp_maintenance {
                out.insert("atpMaintenance".into(), json!(a));
            }
            if !m.exchange_for.is_empty() {
                out.insert("exchangeFor".into(), json!(m.exchange_for));
            }
            if let Some(model) = &m.model {
                out.insert("model".into(), model.to_json());
            }
            Value::Object(out)
        })
        .collect();
    let mut out = Map::new();
    out.insert("members".into(), Value::Array(members));
    out.insert("totalBiomass".into(), json!(spec.total_biomass));
    if let Some(cap) = spec.biomass_cap {
        out.insert("biomassCap".into(), json!(cap));
    }
    if !spec.community_uptake.is_empty() || spec.joint.is_none() {
        out.insert("communityUptake".into(), json!(spec.community_uptake));
    }
    if !spec.community_exchange_bounds.is_empty() {
        let bounds: Map<String, Value> = spec
            .community_exchange_bounds
            .iter()
            .map(|(k, b)| (k.clone(), json!({"lower": b.lower, "upper": b.upper})))
            .collect();
        out.insert("communityExchangeBounds".into(), Value::Object(bounds));
    }
    match &spec.joint {
        None => {
            out.insert("metadata".into(), json!({"kind": "separate"}));
        }
        Some(j) => {
            let reactions: Vec<Value> = j
                .reactions
                .values()
                .map(|r| {
                    json!([r.id, {
                        "id": r.id, "name": r.name, "subsystem": r.subsystem, "metabolites": r.metabolites,
                        "species": r.species, "lowerBound": r.lower_bound, "upperBound": r.upper_bound, "objective": r.objective,
                    }])
                })
                .collect();
            out.insert("reactions".into(), Value::Array(reactions));
            out.insert(
                "metabolites".into(),
                Value::Array(j.metabolites.iter().map(|(k, v)| json!([k, Value::Object(v.clone())])).collect()),
            );
            out.insert("communityTag".into(), json!(j.community_tag));
            out.insert("constraints".into(), Value::Array(j.constraints.iter().map(constraint_to_json).collect()));
            if let Some(m) = &j.metadata {
                let owners: Vec<Value> = m.reaction_owners.iter().map(|(k, v)| json!([k, v])).collect();
                out.insert(
                    "metadata".into(),
                    json!({
                        "kind": "joint",
                        "reactionOwners": owners,
                        "bases": pairs(&m.bases),
                        "speciesExchange": pairs(&m.species_exchange),
                        "communityUptake": pairs(&m.community_uptake),
                        "communityExport": pairs(&m.community_export),
                        "fiber": m.fiber.map(|f| json!({"bacteroides": f.bacteroides, "faecalibacterium": f.faecalibacterium})),
                    }),
                );
            } else {
                out.insert("metadata".into(), json!({"kind": "joint"}));
            }
        }
    }
    Value::Object(out)
}

/// Restore a [`serialize_steady_com_spec`] snapshot, giving every separate
/// member its own [`Model`].
pub fn restore_steady_com_spec(snapshot: &Value) -> Result<SteadyComSpec, SteadyComError> {
    let bad = || SteadyComError::Invalid("Not a SteadyCom spec snapshot.".into());
    let members = snapshot.get("members").and_then(Value::as_array).ok_or_else(bad)?;
    let mut out_members = Vec::new();
    for m in members {
        let model = match m.get("model") {
            Some(doc) => Some(Arc::new(Model::from_value(doc).map_err(|e| SteadyComError::Invalid(e.to_string()))?)),
            None => None,
        };
        out_members.push(Member {
            id: m.get("id").and_then(Value::as_str).ok_or_else(bad)?.to_string(),
            name: m.get("name").and_then(Value::as_str).map(str::to_string),
            biomass_reaction: m.get("biomassReaction").and_then(Value::as_str).unwrap_or_default().to_string(),
            atp_maintenance: m.get("atpMaintenance").and_then(Value::as_str).map(str::to_string),
            model,
            exchange_for: m
                .get("exchangeFor")
                .and_then(Value::as_object)
                .map(|o| o.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string())).collect())
                .unwrap_or_default(),
        });
    }
    let community_uptake = snapshot
        .get("communityUptake")
        .and_then(Value::as_object)
        .map(|o| o.iter().map(|(k, v)| (k.clone(), js::to_number(v))).collect())
        .unwrap_or_default();
    let community_exchange_bounds = snapshot
        .get("communityExchangeBounds")
        .and_then(Value::as_object)
        .map(|o| {
            o.iter()
                .map(|(k, v)| (k.clone(), ExchangeBounds { lower: v.get("lower").and_then(Value::as_f64), upper: v.get("upper").and_then(Value::as_f64) }))
                .collect()
        })
        .unwrap_or_default();
    let joint = match snapshot.get("reactions") {
        None => None,
        Some(list) => {
            let mut reactions = IndexMap::new();
            for (id, r) in read_pairs(list) {
                reactions.insert(
                    id.clone(),
                    JointReaction {
                        id,
                        name: r.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
                        subsystem: r.get("subsystem").and_then(Value::as_str).unwrap_or_default().to_string(),
                        metabolites: r
                            .get("metabolites")
                            .and_then(Value::as_object)
                            .map(|o| o.iter().map(|(k, v)| (k.clone(), js::to_number(v))).collect())
                            .unwrap_or_default(),
                        species: r.get("species").and_then(Value::as_str).unwrap_or_default().to_string(),
                        lower_bound: r.get("lowerBound").map_or(f64::NAN, js::to_number),
                        upper_bound: r.get("upperBound").map_or(f64::NAN, js::to_number),
                        objective: r.get("objective").map_or(0.0, js::to_number),
                    },
                );
            }
            let metabolites = read_pairs(snapshot.get("metabolites").unwrap_or(&Value::Null))
                .into_iter()
                .map(|(k, v)| (k, v.as_object().cloned().unwrap_or_default()))
                .collect();
            let metadata = snapshot.get("metadata").filter(|m| m.get("kind").and_then(Value::as_str) == Some("joint"));
            let metadata = metadata.filter(|m| m.get("bases").is_some()).map(|m| {
                let strings = |key: &str| -> IndexMap<String, String> {
                    read_pairs(m.get(key).unwrap_or(&Value::Null))
                        .into_iter()
                        .map(|(k, v)| (k, v.as_str().unwrap_or_default().to_string()))
                        .collect()
                };
                JointMetadata {
                    reaction_owners: read_pairs(m.get("reactionOwners").unwrap_or(&Value::Null))
                        .into_iter()
                        .map(|(k, v)| (k, v.as_str().map(str::to_string)))
                        .collect(),
                    bases: strings("bases"),
                    species_exchange: strings("speciesExchange"),
                    community_uptake: strings("communityUptake"),
                    community_export: strings("communityExport"),
                    fiber: m.get("fiber").and_then(Value::as_object).map(|f| Fiber {
                        bacteroides: f.get("bacteroides").map_or(f64::NAN, js::to_number),
                        faecalibacterium: f.get("faecalibacterium").map_or(f64::NAN, js::to_number),
                    }),
                }
            });
            Some(JointCommunity {
                reactions,
                metabolites,
                community_tag: snapshot.get("communityTag").and_then(Value::as_str).unwrap_or("comm").to_string(),
                constraints: snapshot
                    .get("constraints")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().map(constraint_from_json).collect())
                    .unwrap_or_default(),
                metadata,
            })
        }
    };
    Ok(SteadyComSpec {
        members: out_members,
        total_biomass: snapshot.get("totalBiomass").map_or(1.0, js::to_number),
        biomass_cap: snapshot.get("biomassCap").and_then(Value::as_f64),
        community_uptake,
        community_exchange_bounds,
        joint,
    })
}

// Reading results

/// `communityBalances`: absolute community rates from a joint solution.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CommunityBalances {
    /// Shared metabolite → member → signed exchange (positive = secretion).
    pub species_exchanges: IndexMap<String, IndexMap<String, f64>>,
    /// Shared metabolite → actual UT flux (not its budget).
    pub community_uptake: IndexMap<String, f64>,
    /// Shared metabolite → actual EX flux.
    pub community_export: IndexMap<String, f64>,
}

/// Absolute community rates from the joint solution. Positive species exchange
/// means secretion. Uptake and export are the actual UT/EX columns, not budgets.
/// Keys are shared metabolite identifiers, with display bases in `metadata.bases`.
pub fn community_balances(spec: &SteadyComSpec, fluxes: &IndexMap<String, f64>) -> CommunityBalances {
    let mut out = CommunityBalances::default();
    let Some(metadata) = spec.joint.as_ref().and_then(|j| j.metadata.as_ref()) else {
        return out;
    };
    let value = |rid: &str| fluxes.get(rid).copied().unwrap_or(0.0);
    for (key, rid) in &metadata.species_exchange {
        let (met, species) = key.split_once('\u{0}').unwrap_or((key.as_str(), ""));
        let per = out.species_exchanges.entry(met.to_string()).or_default();
        *per.entry(species.to_string()).or_insert(0.0) += value(rid);
    }
    for (met, rid) in &metadata.community_uptake {
        out.community_uptake.insert(met.clone(), value(rid));
    }
    for (met, rid) in &metadata.community_export {
        out.community_export.insert(met.clone(), value(rid));
    }
    out
}

/// A member of a projection, with the abundance it was fixed at.
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveSpecies {
    pub id: String,
    pub biomass_reaction: String,
    pub abundance: f64,
}

/// The ids of one community boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct CommunityExchangeIds {
    pub uptake: String,
    pub export: String,
}

/// What [`joint_community_projection`] returns: an ordinary merged model plus
/// the ownership tables the map and the inspector read.
#[derive(Debug, Clone)]
pub struct JointProjection {
    pub model: Model,
    pub biomass_ids: IndexMap<String, String>,
    pub reaction_owner: IndexMap<String, ReactionOwner>,
    pub metabolite_owner: IndexMap<String, MetaboliteOwner>,
    pub transfers_by_base: IndexMap<String, Vec<Transfer>>,
    pub community_exchange_ids: IndexMap<String, CommunityExchangeIds>,
    pub active_species: Vec<ActiveSpecies>,
    pub abundances: IndexMap<String, f64>,
    pub species_compartments: IndexMap<String, Vec<String>>,
}

fn projection_compartment(id: &str, doc: &Map<String, Value>) -> String {
    if let Some(c) = doc.get("compartment").filter(|c| js::truthy(c)) {
        return js::to_string(c);
    }
    if let Some(c) = BRACKET_COMPARTMENT.captures(id) {
        return c[1].to_string();
    }
    if let Some(c) = SUFFIX_COMPARTMENT.captures(id) {
        return c[1].to_string();
    }
    "c".into()
}

/// Conventional FBA at a given composition. Member fluxes are specific rates.
/// Their shared-medium coefficients carry the supplied biomass amounts. This
/// projection does not introduce SteadyCom's variable abundances or fiber rows.
pub fn joint_community_projection(
    spec: &SteadyComSpec,
    abundances: &IndexMap<String, f64>,
    objective: Objective,
    product_target: Option<&str>,
) -> Result<JointProjection, SteadyComError> {
    let (Some(joint), Some(metadata)) = (spec.joint.as_ref(), spec.joint.as_ref().and_then(|j| j.metadata.as_ref())) else {
        return invalid("A joint community projection needs a joint SteadyCom spec.");
    };
    let mut amounts: IndexMap<String, f64> = IndexMap::new();
    for member in &spec.members {
        let amount = abundances.get(&member.id).copied();
        match amount {
            Some(a) if a.is_finite() && a >= 0.0 => {
                amounts.insert(member.id.clone(), a);
            }
            other => {
                let shown = other.map_or("undefined".to_string(), js::number_to_string);
                return invalid(format!("{}: fixed abundance must be finite and nonnegative (got {shown}).", member.id));
            }
        }
    }
    if !amounts.values().any(|a| *a > 0.0) {
        return invalid("A fixed community needs at least one positive abundance.");
    }

    let mut compartments: IndexMap<String, String> = IndexMap::new();
    compartments.insert("s".into(), "Shared medium".into());
    compartments.insert(LINK_COMPARTMENT.into(), "Growth coupling (virtual)".into());
    let mut metabolites: Vec<Value> = Vec::new();
    let mut metabolite_owner: IndexMap<String, MetaboliteOwner> = IndexMap::new();
    let mut metabolite_ids: IndexMap<String, String> = IndexMap::new();
    let mut species_compartments: IndexMap<String, IndexSet<String>> =
        spec.members.iter().map(|m| (m.id.clone(), IndexSet::new())).collect();
    for (id, doc) in &joint.metabolites {
        let shared = metadata.bases.contains_key(id);
        let compartment = if shared { "s".to_string() } else { projection_compartment(id, doc) };
        let owner = if shared { None } else { doc.get("species").and_then(Value::as_str).map(str::to_string) };
        if !shared && !owner.as_ref().is_some_and(|o| amounts.contains_key(o)) {
            return invalid(format!("{id}: no member owns this metabolite."));
        }
        let base = metadata.bases.get(id).cloned();
        let merged_id = match (&base, &owner) {
            (Some(b), _) if shared => format!("M@{b}_s"),
            (_, Some(o)) => format!("{o}@{id}"),
            _ => unreachable!("checked above"),
        };
        metabolite_ids.insert(id.clone(), merged_id.clone());
        if !compartments.contains_key(&compartment) {
            let name = match compartment.as_str() {
                "c" => "Cytosol",
                "e" => "Extracellular",
                "p" => "Periplasm",
                other => other,
            };
            compartments.insert(compartment.clone(), name.to_string());
        }
        let mut entry = doc.clone();
        entry.insert("id".into(), json!(merged_id));
        entry.insert("compartment".into(), json!(compartment));
        let name = doc.get("name").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| json!(id));
        entry.insert("name".into(), name);
        let formula = doc.get("formula").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| json!(""));
        entry.insert("formula".into(), formula);
        let charge = doc.get("charge").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| json!(0));
        entry.insert("charge".into(), charge);
        metabolites.push(Value::Object(entry));
        metabolite_owner.insert(
            merged_id,
            MetaboliteOwner {
                species_id: owner.clone(),
                original_id: if shared { base.clone().unwrap_or_default() } else { id.clone() },
                compartment: compartment.clone(),
            },
        );
        if let (false, Some(o)) = (shared, &owner) {
            species_compartments.get_mut(o).unwrap().insert(compartment);
        }
    }

    let mut reactions: Vec<Map<String, Value>> = Vec::new();
    let mut reaction_index: IndexMap<String, usize> = IndexMap::new();
    let mut reaction_owner: IndexMap<String, ReactionOwner> = IndexMap::new();
    let biomass_ids: IndexMap<String, String> =
        spec.members.iter().map(|m| (m.id.clone(), format!("{}@{}", m.id, m.biomass_reaction))).collect();
    let exchange_ids: HashSet<&String> = metadata.species_exchange.values().collect();
    for (id, reaction) in &joint.reactions {
        let community = reaction.species == joint.community_tag;
        let owner = if community { None } else { Some(reaction.species.clone()) };
        let amount = if community { Some(1.0) } else { amounts.get(&reaction.species).copied() };
        let Some(amount) = amount else {
            return invalid(format!("{id}: no member owns this reaction."));
        };
        let merged_id = match &owner {
            Some(o) => format!("{o}@{id}"),
            None => id.clone(),
        };
        let mut stoichiometry = Map::new();
        for (mid, coefficient) in &reaction.metabolites {
            let Some(target) = metabolite_ids.get(mid) else {
                return invalid(format!("{id}: unknown metabolite {mid}."));
            };
            let scale = if !community && metadata.bases.contains_key(mid) { amount } else { 1.0 };
            let projected = coefficient * scale;
            if projected != 0.0 {
                stoichiometry.insert(target.clone(), json!(projected));
            }
        }
        let mut row = Map::new();
        row.insert("id".into(), json!(merged_id));
        row.insert("name".into(), json!(reaction.name));
        row.insert("metabolites".into(), Value::Object(stoichiometry));
        row.insert("lower_bound".into(), json!(if amount == 0.0 { 0.0 } else { reaction.lower_bound }));
        row.insert("upper_bound".into(), json!(if amount == 0.0 { 0.0 } else { reaction.upper_bound }));
        row.insert("subsystem".into(), json!(reaction.subsystem));
        reaction_index.insert(merged_id.clone(), reactions.len());
        reactions.push(row);
        let role = if community {
            ReactionRole::Community
        } else if exchange_ids.contains(id) {
            ReactionRole::Transfer
        } else if owner.as_ref().and_then(|o| biomass_ids.get(o)) == Some(&merged_id) {
            ReactionRole::Biomass
        } else {
            ReactionRole::Metabolic
        };
        reaction_owner.insert(merged_id, ReactionOwner { species_id: owner, original_id: id.clone(), role });
    }

    let active_species: Vec<ActiveSpecies> = spec
        .members
        .iter()
        .filter(|m| amounts[&m.id] > 0.0)
        .map(|m| ActiveSpecies { id: m.id.clone(), biomass_reaction: m.biomass_reaction.clone(), abundance: amounts[&m.id] })
        .collect();
    let mut objective_coefficients = Map::new();
    match objective {
        Objective::EqualGrowth => {
            let mut growth_mets = Map::new();
            let mut upper = f64::INFINITY;
            for member in &active_species {
                let link = format!("GROWTHLINK@{}", member.id);
                let Some(&bi) = reaction_index.get(&biomass_ids[&member.id]) else {
                    return invalid(format!("{}: the biomass reaction is missing from the joint model.", member.id));
                };
                metabolites.push(json!({
                    "id": link, "name": format!("Growth link ({})", member.id),
                    "compartment": LINK_COMPARTMENT, "formula": "", "charge": 0,
                }));
                metabolite_owner.insert(
                    link.clone(),
                    MetaboliteOwner { species_id: Some(member.id.clone()), original_id: link.clone(), compartment: LINK_COMPARTMENT.into() },
                );
                if let Some(Value::Object(m)) = reactions[bi].get_mut("metabolites") {
                    m.insert(link.clone(), json!(1));
                }
                growth_mets.insert(link, json!(-1));
                upper = js::min(upper, reactions[bi]["upper_bound"].as_f64().unwrap_or(f64::NAN));
            }
            let mut growth = Map::new();
            growth.insert("id".into(), json!(COMMUNITY_GROWTH));
            growth.insert("name".into(), json!("Community growth rate"));
            growth.insert("metabolites".into(), Value::Object(growth_mets));
            growth.insert("lower_bound".into(), json!(0));
            growth.insert("upper_bound".into(), json!(js::max(0.0, upper)));
            growth.insert("subsystem".into(), json!("Community"));
            reactions.push(growth);
            reaction_owner.insert(
                COMMUNITY_GROWTH.into(),
                ReactionOwner { species_id: None, original_id: COMMUNITY_GROWTH.into(), role: ReactionRole::Community },
            );
            objective_coefficients.insert(COMMUNITY_GROWTH.into(), json!(1));
        }
        Objective::SumBiomass => {
            for member in &active_species {
                objective_coefficients.insert(biomass_ids[&member.id].clone(), json!(amounts[&member.id]));
            }
        }
        Objective::Product => {
            let shared = product_target.and_then(|t| {
                if metadata.bases.contains_key(t) {
                    Some(t.to_string())
                } else {
                    metadata.bases.iter().find(|(_, b)| b.as_str() == t).map(|(m, _)| m.clone())
                }
            });
            let export_id = shared.as_ref().and_then(|s| metadata.community_export.get(s));
            let uptake_id = shared.as_ref().and_then(|s| metadata.community_uptake.get(s));
            let (Some(export_id), Some(uptake_id)) = (export_id, uptake_id) else {
                return invalid(format!("No community boundary is available for product {}.", product_target.unwrap_or("(none)")));
            };
            objective_coefficients.insert(export_id.clone(), json!(1));
            objective_coefficients.insert(uptake_id.clone(), json!(-1));
        }
    }

    let mut transfers_by_base: IndexMap<String, Vec<Transfer>> = IndexMap::new();
    for (key, id) in &metadata.species_exchange {
        let (met, species_id) = key.split_once('\u{0}').unwrap_or((key.as_str(), ""));
        let base = metadata.bases.get(met).cloned().unwrap_or_else(|| "undefined".into());
        let local = joint.reactions.get(id).and_then(|r| r.metabolites.keys().find(|m| m.as_str() != met));
        transfers_by_base.entry(base).or_default().push(Transfer {
            species_id: species_id.to_string(),
            reaction_id: format!("{species_id}@{id}"),
            local_id: local.and_then(|l| metabolite_ids.get(l)).cloned().unwrap_or_default(),
        });
    }
    let community_exchange_ids = metadata
        .bases
        .iter()
        .map(|(met, base)| {
            (
                base.clone(),
                CommunityExchangeIds {
                    uptake: metadata.community_uptake.get(met).cloned().unwrap_or_default(),
                    export: metadata.community_export.get(met).cloned().unwrap_or_default(),
                },
            )
        })
        .collect();
    let model = Model::from_value(&json!({
        "id": "joint-community",
        "name": "Joint community at fixed composition",
        "compartments": compartments,
        "metabolites": metabolites,
        "reactions": reactions,
        "objective": objective_coefficients,
    }))
    .map_err(|e| SteadyComError::Invalid(e.to_string()))?;
    Ok(JointProjection {
        model,
        biomass_ids,
        reaction_owner,
        metabolite_owner,
        transfers_by_base,
        community_exchange_ids,
        active_species,
        abundances: amounts,
        species_compartments: species_compartments.into_iter().map(|(k, v)| (k, v.into_iter().collect())).collect(),
    })
}

/// Translate SteadyCom's flux keys into the merged model's.
///
/// SteadyCom names a member's reaction `Ec1|PGI`, while the community model the map
/// is drawn from names the same reaction `Ec1@PGI`. Everything the map, the
/// inspector and the flux scale read goes through the second, so a result is
/// only usable once it has been renamed. A joint model's keys are bare reaction
/// ids, and `owners` (reaction → member) gives them their prefix.
pub fn to_merged_fluxes(
    fluxes: &IndexMap<String, f64>,
    separator: &str,
    owners: Option<&IndexMap<String, Option<String>>>,
) -> IndexMap<String, f64> {
    fluxes
        .iter()
        .map(|(key, value)| {
            let owner = owners.and_then(|o| o.get(key)).and_then(|o| o.as_deref()).filter(|o| !o.is_empty());
            let id = match (owner, key.find('|')) {
                (Some(o), _) => format!("{o}{separator}{key}"),
                (None, None) => key.clone(),
                (None, Some(at)) => format!("{}{separator}{}", &key[..at], &key[at + 1..]),
            };
            (id, *value)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exchange_bases_and_merged_names() {
        assert_eq!(exchange_base("EX_glc_e"), "glc");
        assert_eq!(exchange_base("EX_glc__D_e"), "glc__D");
        let renamed = to_merged_fluxes(
            &[("Ec1|PGI".to_string(), 1.5), ("Ec2|EX_glc_e".to_string(), -8.0), ("EX_glc_comm".to_string(), 2.0)].into_iter().collect(),
            "@",
            None,
        );
        assert_eq!(renamed["Ec1@PGI"], 1.5);
        assert_eq!(renamed["Ec2@EX_glc_e"], -8.0);
        assert_eq!(renamed["EX_glc_comm"], 2.0);
    }

    #[test]
    fn media_csv_becomes_a_budget_with_uptake_positive() {
        let uptake = parse_media_csv(
            "metabolite,exchange_reaction,lower_bound,upper_bound\nglc,EX_glc_e,-8,1000\no2,EX_o2_e,-18.5,1000\nac,EX_ac_e,0,1000\n",
            &[],
        )
        .unwrap();
        assert_eq!(uptake, [("glc".to_string(), 8.0), ("o2".to_string(), 18.5)].into_iter().collect::<IndexMap<_, _>>());
        let err = parse_media_csv("a,b\n1,2", &[]).unwrap_err().to_string();
        assert!(err.contains("metabolite") && err.contains("lower_bound"), "{err}");
    }

    #[test]
    fn csv_rows_handle_quotes() {
        assert_eq!(csv_row(r#"a, "b,c" ,"d""e""#).unwrap(), ["a", "b,c", "d\"e"]);
        assert!(csv_row("\"open").is_err());
    }
}
