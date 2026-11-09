//! COBRA-compatible metabolic model.
//!
//! Reads plain COBRApy JSON (`cobra.io.save_json_model`) and a few optional
//! extension keys that COBRApy ignores on load:
//!
//! ```text
//! compartment_tree      : { "<child>": "<parent>|null" }  explicit organelle nesting
//! cell                  : { name, organism, color, gram, shape }
//! metabolites[].primary : boolean  force a metabolite to be a map hub
//! reactions[].layout    : { rank, lane }  coarse placement hints for the map
//! ```
//!
//! Nothing here is required: a stock BiGG model loads and renders with
//! inferred nesting and an automatic layout.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use indexmap::{IndexMap, IndexSet};
use regex::Regex;
use serde_json::{Map, Value, json};

use crate::compartments::{self, CompartmentTree, Names};
use crate::js;
use crate::label_sources::{self, EntityKind, LabelSources, label_sources_to_value};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ModelError {
    #[error("Reaction {reaction} references unknown metabolite {metabolite}")]
    UnknownMetabolite { reaction: String, metabolite: String },
    #[error("{0}")]
    Invalid(String),
}

static ELEMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([A-Z][a-z]?)([0-9]*\.?[0-9]*)").unwrap());
static BRACKETS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[()\[\]]").unwrap());
static COMPARTMENT_SUFFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(.*)_([a-z][a-z0-9]?)$").unwrap());
static BIOMASS_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"BIOMASS|GROWTH").unwrap());

/// Parse `"C6H12O6"` into `{C: 6, H: 12, O: 6}`, or `None` for unusable input
/// (R groups, charges, brackets or anything else the pattern skips).
pub fn parse_formula(formula: &str) -> Option<BTreeMap<String, f64>> {
    let f = formula.trim();
    if f.is_empty() || f == "null" || BRACKETS.is_match(f) {
        return None;
    }
    let mut out: BTreeMap<String, f64> = BTreeMap::new();
    let mut consumed = 0;
    for m in ELEMENT.captures_iter(f) {
        let whole = m.get(0).unwrap().as_str();
        consumed += whole.len();
        let digits = &m[2];
        let count = if digits.is_empty() { 1.0 } else { js::string_to_number(digits) };
        if !count.is_finite() {
            return None;
        }
        *out.entry(m[1].to_string()).or_insert(0.0) += count;
    }
    if consumed != f.len() {
        return None;
    }
    Some(out)
}

/// Strip the trailing `_<compartment>` from a BiGG metabolite id.
pub fn strip_compartment(metabolite_id: &str, compartment_id: &str) -> String {
    if !compartment_id.is_empty() {
        if let Some(stem) = metabolite_id.strip_suffix(&format!("_{compartment_id}")) {
            return stem.to_string();
        }
    }
    match COMPARTMENT_SUFFIX.captures(metabolite_id) {
        Some(c) => c[1].to_string(),
        None => metabolite_id.to_string(),
    }
}

/// Guess a compartment id from a BiGG metabolite id suffix.
pub fn infer_compartment(metabolite_id: &str) -> String {
    match COMPARTMENT_SUFFIX.captures(metabolite_id) {
        Some(c) => c[2].to_string(),
        None => "c".into(),
    }
}

/// Currency metabolites that are duplicated per reaction instead of shared.
pub const DEFAULT_CURRENCY: &[&str] = &[
    "h", "h2o", "atp", "adp", "amp", "pi", "ppi", "nad", "nadh", "nadp", "nadph", "co2", "o2", "coa", "nh4",
    "q8", "q8h2", "q6", "q6h2", "fad", "fadh2", "so4", "hco3", "na1", "k", "mg2", "cl", "accoa", "fdxo",
    "fdxr", "pqh2", "pq", "h2", "photon",
];

/// Whether a base metabolite id is one of the [`DEFAULT_CURRENCY`] metabolites.
pub fn is_default_currency(base: &str) -> bool {
    DEFAULT_CURRENCY.contains(&base)
}

/// What a reaction does, as the model's structure says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReactionKind {
    Metabolic,
    Transport,
    Exchange,
    Biomass,
    Sink,
}

impl ReactionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ReactionKind::Metabolic => "metabolic",
            ReactionKind::Transport => "transport",
            ReactionKind::Exchange => "exchange",
            ReactionKind::Biomass => "biomass",
            ReactionKind::Sink => "sink",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Reaction {
    pub id: String,
    pub name: String,
    /// Metabolite id → coefficient, in the model's order.
    pub metabolites: IndexMap<String, f64>,
    pub lower_bound: f64,
    pub upper_bound: f64,
    pub objective_coefficient: f64,
    pub subsystem: String,
    pub gene_reaction_rule: String,
    pub layout: Option<Value>,
    pub notes: Value,
    pub annotation: Value,
    pub label_sources: LabelSources,
    pub pathway: String,
    /// Sorted compartment ids of its metabolites, set by `Model::finalize`.
    pub compartments: Vec<String>,
    pub kind: ReactionKind,
}

impl Reaction {
    /// Whether its bounds let it run both ways (lower below zero, upper above).
    pub fn reversible(&self) -> bool {
        self.lower_bound < 0.0 && self.upper_bound > 0.0
    }

    /// Metabolites consumed at positive flux.
    pub fn substrates(&self) -> impl Iterator<Item = (&String, f64)> {
        self.metabolites.iter().filter(|(_, c)| **c < 0.0).map(|(k, c)| (k, *c))
    }

    /// Metabolites produced at positive flux.
    pub fn products(&self) -> impl Iterator<Item = (&String, f64)> {
        self.metabolites.iter().filter(|(_, c)| **c > 0.0).map(|(k, c)| (k, *c))
    }

    /// Human readable equation, e.g. `"atp_c + f6p_c --> adp_c + fdp_c + h_c"`.
    pub fn equation(&self) -> String {
        fn fmt<'a>(list: impl Iterator<Item = (&'a String, f64)>) -> String {
            let parts: Vec<String> = list
                .map(|(id, c)| {
                    if c.abs() == 1.0 {
                        id.clone()
                    } else {
                        format!("{} {id}", js::number_to_string(js::round_to(c.abs(), 6)))
                    }
                })
                .collect();
            if parts.is_empty() { "∅".into() } else { parts.join(" + ") }
        }
        let arrow = if self.reversible() {
            "<=>"
        } else if self.upper_bound > 0.0 {
            "-->"
        } else {
            "<--"
        };
        format!("{} {arrow} {}", fmt(self.substrates()), fmt(self.products()))
    }

    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        out.insert("id".into(), json!(self.id));
        out.insert("name".into(), json!(self.name));
        out.insert("metabolites".into(), json!(self.metabolites));
        out.insert("lower_bound".into(), json!(self.lower_bound));
        out.insert("upper_bound".into(), json!(self.upper_bound));
        out.insert("gene_reaction_rule".into(), json!(self.gene_reaction_rule));
        if !self.subsystem.is_empty() {
            out.insert("subsystem".into(), json!(self.subsystem));
        }
        if !self.pathway.is_empty() {
            out.insert("pathway".into(), json!(self.pathway));
        }
        if self.objective_coefficient != 0.0 && !self.objective_coefficient.is_nan() {
            out.insert("objective_coefficient".into(), json!(self.objective_coefficient));
        }
        if let Some(layout) = &self.layout {
            if js::truthy(layout) {
                out.insert("layout".into(), layout.clone());
            }
        }
        if non_empty_object(&self.notes) {
            out.insert("notes".into(), self.notes.clone());
        }
        if non_empty_object(&self.annotation) {
            out.insert("annotation".into(), self.annotation.clone());
        }
        if !self.label_sources.is_empty() {
            out.insert("labelSources".into(), label_sources_to_value(&self.label_sources));
        }
        Value::Object(out)
    }
}

/// `Object.keys(x).length` for something JS treats as an object.
fn non_empty_object(v: &Value) -> bool {
    match v {
        Value::Object(m) => !m.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::String(s) => !s.is_empty(),
        _ => false,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Metabolite {
    pub id: String,
    pub name: String,
    pub formula: String,
    pub charge: f64,
    pub compartment: String,
    /// The name the map prints: the id without its compartment suffix, unless
    /// the document says otherwise (merged community ids need it said).
    pub base: String,
    pub explicit_primary: Option<bool>,
    pub notes: Value,
    pub annotation: Value,
    pub label_sources: LabelSources,
    /// Reaction ids touching this metabolite, set by `Model::finalize`.
    pub reactions: Vec<String>,
}

impl Metabolite {
    /// Element counts of its formula (see [`parse_formula`]).
    pub fn elements(&self) -> Option<BTreeMap<String, f64>> {
        parse_formula(&self.formula)
    }

    /// Primary metabolites get one shared node on the map. Currency
    /// metabolites get one small node per reaction so the map does not
    /// collapse into a hairball.
    pub fn is_primary(&self) -> bool {
        if let Some(p) = self.explicit_primary {
            return p;
        }
        if is_default_currency(&self.base) {
            return false;
        }
        self.reactions.len() <= 6
    }

    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        out.insert("id".into(), json!(self.id));
        out.insert("name".into(), json!(self.name));
        out.insert("compartment".into(), json!(self.compartment));
        out.insert("formula".into(), json!(self.formula));
        out.insert("charge".into(), json!(self.charge));
        if let Some(p) = self.explicit_primary {
            out.insert("primary".into(), json!(p));
        }
        if non_empty_object(&self.notes) {
            out.insert("notes".into(), self.notes.clone());
        }
        if non_empty_object(&self.annotation) {
            out.insert("annotation".into(), self.annotation.clone());
        }
        if !self.label_sources.is_empty() {
            out.insert("labelSources".into(), label_sources_to_value(&self.label_sources));
        }
        Value::Object(out)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CellInfo {
    pub name: String,
    pub organism: String,
    pub color: String,
    pub shape: String,
    pub gram: Option<Value>,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    pub id: String,
    pub name: String,
    /// As the document gives it (`raw.version || '1'`): a string or a number.
    pub version: Value,
    pub compartment_names: Names,
    pub cell: CellInfo,
    pub explicit_tree: Option<IndexMap<String, Option<String>>>,
    pub metabolites: IndexMap<String, Metabolite>,
    pub reactions: IndexMap<String, Reaction>,
    pub genes: Vec<Value>,
    pub raw_objective: Option<Value>,
}

/// First value that `Number()` turns into a finite number, else 0.
fn num_or(values: &[Option<&Value>], fallback: f64) -> f64 {
    for v in values.iter().flatten() {
        if v.is_null() {
            continue;
        }
        if let Some(n) = js::finite_number(v) {
            return n;
        }
    }
    fallback
}

fn obj_or_empty(raw: &Map<String, Value>, keys: &[&str]) -> Value {
    for k in keys {
        if let Some(v) = raw.get(*k) {
            if js::truthy(v) {
                return v.clone();
            }
        }
    }
    Value::Object(Map::new())
}

fn first_text(values: &[Option<&Value>]) -> String {
    for value in values.iter().flatten() {
        match value {
            Value::Array(items) => {
                if let Some(t) = items.iter().filter_map(Value::as_str).find(|s| !s.trim().is_empty()) {
                    return t.trim().to_string();
                }
            }
            Value::String(s) if !s.trim().is_empty() => return s.trim().to_string(),
            _ => {}
        }
    }
    String::new()
}

fn text(raw: &Map<String, Value>, key: &str) -> Option<String> {
    match raw.get(key) {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(v @ Value::Number(_)) => Some(js::to_string(v)),
        Some(Value::Bool(true)) => Some("true".into()),
        _ => None,
    }
}

impl Reaction {
    /// One reaction of a COBRA JSON document, its label sources already
    /// collected. Compartments and kind are left for [`Model::finalize`].
    pub fn from_raw(raw: &Map<String, Value>, label_sources: LabelSources) -> Reaction {
        let id = text(raw, "id").unwrap_or_default();
        let name = text(raw, "name").unwrap_or_else(|| id.clone());
        let mut metabolites = IndexMap::new();
        if let Some(Value::Object(m)) = raw.get("metabolites") {
            for (k, v) in m {
                metabolites.insert(k.clone(), js::to_number(v));
            }
        }
        js::js_key_order(&mut metabolites);
        let notes = obj_or_empty(raw, &["notes"]);
        let annotation = obj_or_empty(raw, &["annotation", "annotations"]);
        // COBRA JSON has a well-established "subsystem" field but no single
        // universal pathway field: keep explicit pathway metadata when present
        // and accept the common notes/annotation variants.
        let pathway = first_text(&[
            raw.get("pathway"),
            notes.get("pathway"),
            notes.get("Pathway"),
            annotation.get("pathway"),
            annotation.get("pathways"),
        ]);
        Reaction {
            id,
            name,
            metabolites,
            lower_bound: num_or(&[raw.get("lower_bound"), raw.get("lowerBound")], -1000.0),
            upper_bound: num_or(&[raw.get("upper_bound"), raw.get("upperBound")], 1000.0),
            objective_coefficient: num_or(&[raw.get("objective_coefficient"), raw.get("objectiveCoefficient")], 0.0),
            subsystem: text(raw, "subsystem").unwrap_or_default(),
            gene_reaction_rule: text(raw, "gene_reaction_rule").or_else(|| text(raw, "geneReactionRule")).unwrap_or_default(),
            layout: raw.get("layout").filter(|v| js::truthy(v)).cloned(),
            notes,
            annotation,
            label_sources,
            pathway,
            compartments: vec![],
            kind: ReactionKind::Metabolic,
        }
    }
}

impl Metabolite {
    /// One metabolite of a COBRA JSON document, its label sources already
    /// collected.
    pub fn from_raw(raw: &Map<String, Value>, label_sources: LabelSources) -> Metabolite {
        let id = text(raw, "id").unwrap_or_default();
        let name = text(raw, "name").unwrap_or_else(|| id.clone());
        let compartment = text(raw, "compartment").unwrap_or_else(|| infer_compartment(&id));
        // A merged community writes every id as `<species>@<id>`, which the id
        // alone cannot be told apart from. Whoever builds such a model says
        // what the metabolite is called. Otherwise it is read off the id.
        let base = match raw.get("base") {
            Some(Value::String(s)) if !s.is_empty() => s.clone(),
            _ => strip_compartment(&id, &compartment),
        };
        Metabolite {
            formula: text(raw, "formula").unwrap_or_default(),
            charge: num_or(&[raw.get("charge")], 0.0),
            explicit_primary: raw.get("primary").and_then(Value::as_bool),
            notes: obj_or_empty(raw, &["notes"]),
            annotation: obj_or_empty(raw, &["annotation", "annotations"]),
            label_sources,
            reactions: vec![],
            id,
            name,
            compartment,
            base,
        }
    }
}

impl Model {
    /// Read a COBRA JSON document.
    pub fn from_value(raw: &Value) -> Result<Model, ModelError> {
        let empty = Map::new();
        let raw = raw.as_object().unwrap_or(&empty);
        let id = text(raw, "id").unwrap_or_else(|| "model".into());
        let name = text(raw, "name").unwrap_or_else(|| id.clone());
        let version = raw.get("version").filter(|v| js::truthy(v)).cloned().unwrap_or_else(|| json!("1"));
        let mut compartment_names = Names::new();
        if let Some(Value::Object(c)) = raw.get("compartments") {
            for (k, v) in c {
                // A null name is no name, and `finalize` gives it the id.
                let name = if v.is_null() { String::new() } else { js::to_string(v) };
                compartment_names.insert(k.clone(), name);
            }
        }
        js::js_key_order(&mut compartment_names);
        let cell_raw = raw.get("cell").and_then(Value::as_object).unwrap_or(&empty);
        let cell = CellInfo {
            name: text(cell_raw, "name").unwrap_or_else(|| name.clone()),
            organism: text(cell_raw, "organism").unwrap_or_default(),
            color: text(cell_raw, "color").unwrap_or_else(|| "#334155".into()),
            shape: text(cell_raw, "shape").unwrap_or_else(|| "rounded".into()),
            gram: cell_raw.get("gram").filter(|v| js::truthy(v)).cloned(),
            note: text(cell_raw, "note").unwrap_or_default(),
        };
        // `raw.compartment_tree || null`: any object, even an empty one.
        let explicit_tree = match raw.get("compartment_tree") {
            Some(Value::Object(t)) => Some(
                t.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().filter(|s| !s.is_empty()).map(str::to_string)))
                    .collect(),
            ),
            _ => None,
        };

        let mets_raw: &[Value] = raw.get("metabolites").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);
        let rxns_raw: &[Value] = raw.get("reactions").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);

        let met_vectors = label_sources::model_label_vectors(raw, EntityKind::Metabolite, mets_raw.len());
        let mut metabolites = IndexMap::new();
        for (i, m) in mets_raw.iter().enumerate() {
            let m = m.as_object().unwrap_or(&empty);
            let sources = label_sources::entity_label_sources(m, EntityKind::Metabolite, &met_vectors, i);
            let met = Metabolite::from_raw(m, sources);
            // A later duplicate replaces the earlier one in place, as Map#set does.
            metabolites.insert(met.id.clone(), met);
        }

        let rxn_vectors = label_sources::model_label_vectors(raw, EntityKind::Reaction, rxns_raw.len());
        let mut reactions = IndexMap::new();
        for (i, r) in rxns_raw.iter().enumerate() {
            let r = r.as_object().unwrap_or(&empty);
            let sources = label_sources::entity_label_sources(r, EntityKind::Reaction, &rxn_vectors, i);
            let rxn = Reaction::from_raw(r, sources);
            reactions.insert(rxn.id.clone(), rxn);
        }

        let genes = raw
            .get("genes")
            .and_then(Value::as_array)
            .map(|gs| gs.iter().map(normalize_gene).collect())
            .unwrap_or_default();
        let mut raw_objective = raw.get("objective").filter(|v| js::truthy(v)).cloned();
        // JS lists an object's integer-like keys first: with reactions named
        // "10" and "B", the objective, and so the biomass reaction, is "10".
        if let Some(Value::Object(obj)) = raw_objective.as_mut() {
            js::js_object_key_order(obj);
        }

        let mut model = Model {
            id,
            name,
            version,
            compartment_names,
            cell,
            explicit_tree,
            metabolites,
            reactions,
            genes,
            raw_objective,
        };
        model.finalize()?;
        Ok(model)
    }

    /// Parse a COBRA JSON text.
    pub fn from_json_str(text: &str) -> Result<Model, ModelError> {
        let v: Value = serde_json::from_str(text).map_err(|e| ModelError::Invalid(e.to_string()))?;
        Model::from_value(&v)
    }

    /// Recompute what is derived from the reactions: which reactions touch
    /// each metabolite, each reaction's compartments and kind, and a name for
    /// every compartment in use.
    pub fn finalize(&mut self) -> Result<(), ModelError> {
        for met in self.metabolites.values_mut() {
            met.reactions.clear();
        }
        for rxn in self.reactions.values_mut() {
            let mut comps: IndexSet<String> = IndexSet::new();
            for mid in rxn.metabolites.keys() {
                let Some(met) = self.metabolites.get_mut(mid) else {
                    return Err(ModelError::UnknownMetabolite { reaction: rxn.id.clone(), metabolite: mid.clone() });
                };
                met.reactions.push(rxn.id.clone());
                comps.insert(met.compartment.clone());
            }
            let mut sorted: Vec<String> = comps.iter().cloned().collect();
            sorted.sort_by(|a, b| js::code_unit_compare(a, b));
            rxn.kind = classify(rxn, comps.len());
            rxn.compartments = sorted;
        }
        for id in self.compartment_ids() {
            let missing = self.compartment_names.get(&id).is_none_or(|n| n.is_empty());
            if missing {
                self.compartment_names.insert(id.clone(), id);
            }
        }
        Ok(())
    }

    /// Every compartment id the model names or uses, sorted.
    pub fn compartment_ids(&self) -> Vec<String> {
        let mut ids: IndexSet<String> = self.compartment_names.keys().cloned().collect();
        for met in self.metabolites.values() {
            ids.insert(met.compartment.clone());
        }
        let mut ids: Vec<String> = ids.into_iter().collect();
        ids.sort_by(|a, b| js::code_unit_compare(a, b));
        ids
    }

    /// The containment tree of every compartment the model names or uses,
    /// nested by its own `compartment_tree` or else by the default nesting.
    pub fn compartment_tree(&self) -> CompartmentTree {
        compartments::build_compartment_tree(&self.compartment_ids(), self.explicit_tree.as_ref(), &self.compartment_names)
    }

    /// Reactions whose objective coefficient is non-zero.
    pub fn objective_reactions(&self) -> Vec<(String, f64)> {
        if let Some(Value::Object(obj)) = &self.raw_objective {
            return obj
                .iter()
                .filter(|(id, _)| self.reactions.contains_key(*id))
                .map(|(id, c)| (id.clone(), js::to_number(c)))
                .collect();
        }
        if let Some(Value::Array(items)) = &self.raw_objective {
            // An array is an object to JS: its entries are index → value.
            return items
                .iter()
                .enumerate()
                .filter(|(i, _)| self.reactions.contains_key(&i.to_string()))
                .map(|(i, c)| (i.to_string(), js::to_number(c)))
                .collect();
        }
        self.reactions
            .values()
            .filter(|r| r.objective_coefficient != 0.0 && !r.objective_coefficient.is_nan())
            .map(|r| (r.id.clone(), r.objective_coefficient))
            .collect()
    }

    /// Best guess at the growth/biomass reaction id.
    pub fn biomass_reaction_id(&self) -> Option<String> {
        if let Some((id, _)) = self.objective_reactions().into_iter().next() {
            return Some(id);
        }
        self.reactions.values().find(|r| r.kind == ReactionKind::Biomass).map(|r| r.id.clone())
    }

    /// Exchange reactions (single metabolite, boundary).
    pub fn exchange_reactions(&self) -> Vec<&Reaction> {
        self.reactions.values().filter(|r| r.kind == ReactionKind::Exchange).collect()
    }

    /// The metabolites of one compartment, in model order.
    pub fn metabolites_in(&self, compartment_id: &str) -> Vec<&Metabolite> {
        self.metabolites.values().filter(|m| m.compartment == compartment_id).collect()
    }

    /// Elemental and charge balance check. Exchanges, sinks and lumped biomass
    /// pseudo-reactions are skipped because they are unbalanced by design.
    pub fn check_mass_balance(&self, tolerance: f64) -> Vec<(String, IndexMap<String, f64>)> {
        let mut problems = Vec::new();
        for rxn in self.reactions.values() {
            if matches!(rxn.kind, ReactionKind::Exchange | ReactionKind::Biomass | ReactionKind::Sink) {
                continue;
            }
            let mut totals: IndexMap<String, f64> = IndexMap::new();
            let mut ok = true;
            for (mid, coef) in &rxn.metabolites {
                let met = &self.metabolites[mid];
                let Some(els) = parse_formula_ordered(&met.formula) else {
                    ok = false;
                    break;
                };
                for (el, n) in els {
                    *totals.entry(el).or_insert(0.0) += n * coef;
                }
                *totals.entry("charge".into()).or_insert(0.0) += met.charge * coef;
            }
            if !ok {
                continue; // Unparseable formula: cannot judge, do not cry wolf.
            }
            let imbalance: IndexMap<String, f64> = totals
                .into_iter()
                .filter(|(_, v)| v.abs() > tolerance)
                .map(|(k, v)| (k, js::round_to(v, 6)))
                .collect();
            if !imbalance.is_empty() {
                problems.push((rxn.id.clone(), imbalance));
            }
        }
        problems
    }

    /// The stoichiometric matrix in triplet form (metabolite row, reaction
    /// column, coefficient), in model order.
    pub fn stoichiometry(&self) -> Stoichiometry {
        let met_ids: Vec<String> = self.metabolites.keys().cloned().collect();
        let rxn_ids: Vec<String> = self.reactions.keys().cloned().collect();
        let mut triplets = Vec::new();
        for (j, rxn) in self.reactions.values().enumerate() {
            for (mid, coef) in &rxn.metabolites {
                if *coef == 0.0 {
                    continue;
                }
                triplets.push((self.metabolites.get_index_of(mid).expect("finalized"), j, *coef));
            }
        }
        Stoichiometry { met_ids, rxn_ids, triplets }
    }

    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        out.insert("id".into(), json!(self.id));
        out.insert("name".into(), json!(self.name));
        out.insert("version".into(), self.version.clone());
        out.insert("compartments".into(), json!(self.compartment_names));
        let mut cell = Map::new();
        cell.insert("name".into(), json!(self.cell.name));
        cell.insert("organism".into(), json!(self.cell.organism));
        cell.insert("color".into(), json!(self.cell.color));
        cell.insert("shape".into(), json!(self.cell.shape));
        cell.insert("gram".into(), self.cell.gram.clone().unwrap_or(Value::Null));
        cell.insert("note".into(), json!(self.cell.note));
        out.insert("cell".into(), Value::Object(cell));
        out.insert(
            "compartment_tree".into(),
            match &self.explicit_tree {
                Some(t) => json!(t),
                None => Value::Null,
            },
        );
        out.insert("metabolites".into(), Value::Array(self.metabolites.values().map(Metabolite::to_json).collect()));
        out.insert("reactions".into(), Value::Array(self.reactions.values().map(Reaction::to_json).collect()));
        out.insert("genes".into(), Value::Array(self.genes.clone()));
        if let Some(obj) = &self.raw_objective {
            out.insert("objective".into(), obj.clone());
        }
        Value::Object(out)
    }
}

/// Elements in the order the formula names them (for balance reports).
fn parse_formula_ordered(formula: &str) -> Option<Vec<(String, f64)>> {
    let f = formula.trim();
    if f.is_empty() || f == "null" || BRACKETS.is_match(f) {
        return None;
    }
    let mut out: IndexMap<String, f64> = IndexMap::new();
    let mut consumed = 0;
    for m in ELEMENT.captures_iter(f) {
        consumed += m.get(0).unwrap().as_str().len();
        let count = if m[2].is_empty() { 1.0 } else { js::string_to_number(&m[2]) };
        if !count.is_finite() {
            return None;
        }
        *out.entry(m[1].to_string()).or_insert(0.0) += count;
    }
    (consumed == f.len()).then(|| out.into_iter().collect())
}

/// `{ id, name, annotation, notes }` for a gene given as a string or object.
fn normalize_gene(g: &Value) -> Value {
    match g {
        Value::String(s) => json!({"id": s, "name": s, "annotation": {}, "notes": {}}),
        Value::Object(o) => {
            let mut out = o.clone();
            let id = o.get("id").map(js::to_string).unwrap_or_else(|| "undefined".into());
            out.insert("id".into(), json!(id));
            let name = match o.get("name") {
                Some(v) if js::truthy(v) => v.clone(),
                _ => o.get("id").cloned().unwrap_or(Value::Null),
            };
            out.insert("name".into(), name);
            out.insert("annotation".into(), obj_or_empty(o, &["annotation", "annotations"]));
            out.insert("notes".into(), obj_or_empty(o, &["notes"]));
            Value::Object(out)
        }
        other => other.clone(),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stoichiometry {
    pub met_ids: Vec<String>,
    pub rxn_ids: Vec<String>,
    pub triplets: Vec<(usize, usize, f64)>,
}

fn classify(rxn: &Reaction, compartment_count: usize) -> ReactionKind {
    let n = rxn.metabolites.len();
    let id = rxn.id.to_uppercase();
    if BIOMASS_ID.is_match(&id) {
        return ReactionKind::Biomass;
    }
    if n == 1 {
        return if id.starts_with("EX_") { ReactionKind::Exchange } else { ReactionKind::Sink };
    }
    if id.starts_with("EX_") {
        return ReactionKind::Exchange;
    }
    if id.starts_with("DM_") || id.starts_with("SK_") {
        return ReactionKind::Sink;
    }
    if compartment_count > 1 {
        return ReactionKind::Transport;
    }
    ReactionKind::Metabolic
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formulas() {
        let f = parse_formula("C6H12O6").unwrap();
        assert_eq!(f["C"], 6.0);
        assert_eq!(f["H"], 12.0);
        assert!(parse_formula("C6H11O9P(R)").is_none());
        assert!(parse_formula("C6H12O6-").is_none());
        assert_eq!(parse_formula("CH3.5").unwrap().get("H").copied(), Some(3.5));
    }

    #[test]
    fn compartments_from_ids() {
        assert_eq!(strip_compartment("glc__D_e", "e"), "glc__D");
        assert_eq!(strip_compartment("glc__D_e", ""), "glc__D");
        assert_eq!(strip_compartment("M_x", "c"), "M"); // as JS: the suffix rule applies
        assert_eq!(infer_compartment("atp_c"), "c");
        assert_eq!(infer_compartment("atp"), "c");
        assert_eq!(infer_compartment("h_im"), "im");
    }

    #[test]
    fn a_small_model() {
        let doc = serde_json::json!({
            "id": "toy",
            "metabolites": [
                {"id": "glc_e", "compartment": "e"},
                {"id": "glc_c", "compartment": "c"},
                {"id": "atp_c", "compartment": "c"}
            ],
            "reactions": [
                {"id": "EX_glc_e", "metabolites": {"glc_e": -1}, "lower_bound": -10},
                {"id": "GLCt", "metabolites": {"glc_e": -1, "glc_c": 1}},
                {"id": "BIOMASS", "metabolites": {"glc_c": -1, "atp_c": -2.5}, "objective_coefficient": 1}
            ]
        });
        let m = Model::from_value(&doc).unwrap();
        assert_eq!(m.reactions["EX_glc_e"].kind, ReactionKind::Exchange);
        assert_eq!(m.reactions["GLCt"].kind, ReactionKind::Transport);
        assert_eq!(m.reactions["BIOMASS"].kind, ReactionKind::Biomass);
        assert_eq!(m.biomass_reaction_id().as_deref(), Some("BIOMASS"));
        assert_eq!(m.compartment_ids(), ["c", "e"]);
        assert_eq!(m.reactions["BIOMASS"].equation(), "glc_c + 2.5 atp_c <=> ∅");
        assert_eq!(m.reactions["GLCt"].lower_bound, -1000.0);
        assert_eq!(m.metabolites["glc_c"].reactions, ["GLCt", "BIOMASS"]);
        assert!(m.metabolites["glc_c"].is_primary());
        assert!(!m.metabolites["atp_c"].is_primary());
    }

    #[test]
    fn objective_keys_follow_js_order() {
        // node: Object.keys({B: 1, "10": 2}) is ["10", "B"].
        let doc = serde_json::json!({
            "metabolites": [{"id": "x_c"}],
            "reactions": [{"id": "B", "metabolites": {"x_c": 1}}, {"id": "10", "metabolites": {"x_c": -1}}],
            "objective": {"B": 1, "10": 2}
        });
        let m = Model::from_value(&doc).unwrap();
        assert_eq!(m.biomass_reaction_id().as_deref(), Some("10"));
        assert_eq!(m.objective_reactions(), [("10".to_string(), 2.0), ("B".to_string(), 1.0)]);
    }

    #[test]
    fn unknown_metabolites_are_an_error() {
        let doc = serde_json::json!({"metabolites": [], "reactions": [{"id": "R", "metabolites": {"x": 1}}]});
        assert!(matches!(Model::from_value(&doc), Err(ModelError::UnknownMetabolite { .. })));
    }
}
