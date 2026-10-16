//! Port of `src/core/model-edit.js`: user-facing biological model editing.
//!
//! These helpers mutate a source [`Model`] (one species), never the merged
//! community model directly. The community and FBA are rebuilt afterwards by
//! whoever owns them (the Store in the JS app).
//!
//! The specs are the objects the JS editor form builds, given as JSON values
//! so that what a form typed (`"-10"`, `""`, a number) is read exactly as the
//! JS reads it: `{id, name, compartment, formula, charge}` for a metabolite and
//! `{id, name, metabolites, lowerBound, upperBound, geneReactionRule | gpr,
//! subsystem}` for a reaction. [`MetaboliteSpec`] and [`ReactionSpec`] build
//! them from typed fields.

use std::sync::LazyLock;

use indexmap::{IndexMap, IndexSet};
use regex::Regex;
use serde::Serialize;
use serde_json::{Map, Value, json};

use coxeter_core::js;
use coxeter_core::label_sources::{EntityKind, entity_label_sources};
use coxeter_core::model::{Metabolite, Model, Reaction};

use crate::error::{Error, ErrorKind, Result};
use crate::jscompat::{self, DOT, NS, S, js_trim, object_entries, string_or_empty};

const RESERVED_GPR: [&str; 5] = ["and", "or", "not", "true", "false"];

fn err(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::Edit, message)
}

static SPLIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\n,;]+").expect("pattern"));
const NUMBER: &str = r"([+-]?(?:[0-9]+(?:\.[0-9]*)?|\.[0-9]+)(?:[eE][+-]?[0-9]+)?)";
static COLON: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"^({DOT}+?){s}*:{s}*{NUMBER}$", s = *S)).expect("pattern"));
static SPACED: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"^({NS}+){S}+{NUMBER}$", NS = *NS, S = *S)).expect("pattern"));

/// Read a stoichiometry typed by the user: `a_c:-1, b_c:1`, one `id coef` per
/// line, or a JSON object `{"a_c": -1}`. Repeated ids add up, and terms that
/// cancel out are dropped.
pub fn parse_stoichiometry(text: &str) -> Result<IndexMap<String, f64>> {
    let raw = js_trim(text);
    if raw.is_empty() {
        return Err(err("The stoichiometry cannot be empty."));
    }

    let entries: Vec<(String, Value)> = if raw.starts_with('{') {
        let doc: Value = serde_json::from_str(raw).map_err(|e| err(format!("Invalid stoichiometry JSON: {e}")))?;
        if !doc.is_object() {
            return Err(err("Stoichiometry JSON must be an object of { metabolite: coefficient }."));
        }
        object_entries(&doc)
    } else {
        let mut entries = Vec::new();
        for part in SPLIT.split(raw).map(js_trim).filter(|p| !p.is_empty()) {
            if let Some(c) = COLON.captures(part) {
                entries.push((js_trim(&c[1]).to_string(), Value::String(c[2].to_string())));
            } else if let Some(c) = SPACED.captures(part) {
                entries.push((c[1].to_string(), Value::String(c[2].to_string())));
            } else {
                return Err(err(format!("Could not read “{part}”. Use metabolite:coefficient.")));
            }
        }
        entries
    };

    if entries.is_empty() {
        return Err(err("The stoichiometry has no terms."));
    }

    let mut out: IndexMap<String, f64> = IndexMap::new();
    for (id_raw, coefficient_raw) in entries {
        let id = js_trim(&id_raw).to_string();
        let coefficient = js::to_number(&coefficient_raw);
        if id.is_empty() {
            return Err(err("A metabolite in the stoichiometry has no ID."));
        }
        if !coefficient.is_finite() {
            return Err(err(format!("Non-numeric coefficient for {id}.")));
        }
        if coefficient.abs() < 1e-15 {
            return Err(err(format!("The coefficient of {id} cannot be zero.")));
        }
        *out.entry(id).or_insert(0.0) += coefficient;
    }
    js::js_key_order(&mut out);
    out.retain(|_, c| c.abs() >= 1e-15);
    if out.is_empty() {
        return Err(err("The net stoichiometry came out empty."));
    }
    Ok(out)
}

static GPR_TOKEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"[^{}()]+", jscompat::WS_CHARS)).expect("pattern"));

/// The gene ids a rule mentions, in order, without the `and`/`or` words.
pub fn gene_ids_from_gpr(rule: &str) -> Vec<String> {
    let text = js_trim(rule);
    if text.is_empty() {
        return Vec::new();
    }
    let ids: IndexSet<String> = GPR_TOKEN
        .find_iter(text)
        .map(|m| js_trim(m.as_str()).to_string())
        .filter(|t| !t.is_empty() && !RESERVED_GPR.contains(&t.to_lowercase().as_str()))
        .collect();
    ids.into_iter().collect()
}

fn valid_id(value: Option<&Value>, kind: &str) -> Result<String> {
    let id = js_trim(&string_or_empty(value)).to_string();
    if id.is_empty() {
        return Err(err(format!("The {kind} ID is required.")));
    }
    if id.chars().any(jscompat::is_js_ws) {
        return Err(err(format!("The {kind} ID cannot contain spaces.")));
    }
    Ok(id)
}

fn field<'a>(spec: &'a Value, key: &str) -> Option<&'a Value> {
    spec.as_object().and_then(|o| o.get(key))
}

/// Add a metabolite to `model`. `spec` is `{id, name, compartment, formula, charge}`.
pub fn add_metabolite_to_model<'m>(model: &'m mut Model, spec: &Value) -> Result<&'m Metabolite> {
    // The JS id kind reads `metabolito` (sic): the message is kept as the app shows it.
    let id = valid_id(field(spec, "id"), "metabolito")?;
    if model.metabolites.contains_key(&id) {
        return Err(err(format!("Metabolite {id} already exists.")));
    }

    let compartment = js_trim(&string_or_empty(field(spec, "compartment"))).to_string();
    if compartment.is_empty() || !model.compartment_ids().contains(&compartment) {
        let shown = if compartment.is_empty() { "empty" } else { &compartment };
        return Err(err(format!("Invalid compartment: {shown}.")));
    }

    let name = js_trim(&string_or_empty(field(spec, "name"))).to_string();
    if name.is_empty() {
        return Err(err("The metabolite name is required."));
    }

    let charge = match field(spec, "charge") {
        None | Some(Value::Null) => 0.0,
        Some(Value::String(s)) if s.is_empty() => 0.0,
        Some(v) => js::to_number(v),
    };
    if !charge.is_finite() {
        return Err(err("The charge must be numeric."));
    }

    let raw = json!({
        "id": id,
        "name": name,
        "compartment": compartment,
        "formula": js_trim(&string_or_empty(field(spec, "formula"))),
        "charge": charge,
    });
    let raw = raw.as_object().expect("object literal");
    let met = Metabolite::from_raw(raw, entity_label_sources(raw, EntityKind::Metabolite, &[], 0));
    model.metabolites.insert(id.clone(), met);
    model.finalize().map_err(|e| err(e.to_string()))?;
    Ok(&model.metabolites[&id])
}

/// Add a reaction to `model`. `spec` is `{id, name, metabolites, lowerBound,
/// upperBound, geneReactionRule | gpr, subsystem}`, `metabolites` being a
/// stoichiometry text or an object of `{metabolite: coefficient}`. Genes the
/// rule names are added to the model's gene list.
pub fn add_reaction_to_model<'m>(model: &'m mut Model, spec: &Value) -> Result<&'m Reaction> {
    let id = valid_id(field(spec, "id"), "reaction")?;
    if model.reactions.contains_key(&id) {
        return Err(err(format!("Reaction {id} already exists.")));
    }

    let entries: Vec<(String, f64)> = match field(spec, "metabolites") {
        Some(Value::String(text)) => parse_stoichiometry(text)?.into_iter().collect(),
        other => {
            let given = match other {
                Some(v) if !v.is_null() => object_entries(v),
                _ => Vec::new(),
            };
            let mut checked = Vec::new();
            for (mid, coefficient_raw) in given {
                if !model.metabolites.contains_key(&mid) {
                    return Err(err(format!("The reaction references a metabolite that does not exist: {mid}.")));
                }
                let coefficient = js::to_number(&coefficient_raw);
                if !coefficient.is_finite() || coefficient.abs() < 1e-15 {
                    return Err(err(format!("Invalid coefficient for {mid}.")));
                }
                checked.push((mid, coefficient));
            }
            checked
        }
    };
    for (mid, _) in &entries {
        if !model.metabolites.contains_key(mid) {
            return Err(err(format!("The reaction references a metabolite that does not exist: {mid}.")));
        }
    }
    if entries.is_empty() {
        return Err(err("The reaction must contain at least one metabolite."));
    }

    let number = |key: &str| field(spec, key).map_or(f64::NAN, js::to_number);
    let (lower, upper) = (number("lowerBound"), number("upperBound"));
    if !lower.is_finite() || !upper.is_finite() {
        return Err(err("Lower and upper bounds must be numbers."));
    }
    if lower > upper {
        return Err(err("The lower bound cannot be greater than the upper bound."));
    }

    let rule = match field(spec, "geneReactionRule") {
        Some(v) if !v.is_null() => Some(v),
        _ => field(spec, "gpr"),
    };
    let gpr = js_trim(&string_or_empty(rule)).to_string();
    let name = js_trim(&string_or_empty(field(spec, "name"))).to_string();
    let metabolites: Map<String, Value> = entries.into_iter().map(|(k, c)| (k, json!(c))).collect();
    let raw = json!({
        "id": id,
        "name": if name.is_empty() { id.clone() } else { name },
        "metabolites": metabolites,
        "lower_bound": lower,
        "upper_bound": upper,
        "gene_reaction_rule": gpr,
        "subsystem": js_trim(&string_or_empty(field(spec, "subsystem"))),
    });
    let raw = raw.as_object().expect("object literal");
    let reaction = Reaction::from_raw(raw, entity_label_sources(raw, EntityKind::Reaction, &[], 0));
    model.reactions.insert(id.clone(), reaction);
    ensure_genes(model, &gene_ids_from_gpr(&gpr));
    model.finalize().map_err(|e| err(e.to_string()))?;
    Ok(&model.reactions[&id])
}

fn ensure_genes(model: &mut Model, ids: &[String]) {
    let mut existing: IndexSet<String> = model
        .genes
        .iter()
        .filter_map(|gene| match gene {
            Value::String(s) => Some(Value::String(s.clone())),
            Value::Object(o) => o.get("id").cloned(),
            _ => None,
        })
        .filter(js::truthy)
        .map(|v| js::to_string(&v))
        .collect();
    for id in ids {
        if existing.contains(id) {
            continue;
        }
        model.genes.push(json!({"id": id, "name": id}));
        existing.insert(id.clone());
    }
}

/// A metabolite as the editor form gives it.
#[derive(Debug, Clone, Default, Serialize)]
pub struct MetaboliteSpec {
    pub id: String,
    pub name: String,
    pub compartment: String,
    pub formula: String,
    /// Blank or absent is 0.
    pub charge: Option<f64>,
}

impl MetaboliteSpec {
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("plain data serialises")
    }
}

/// A reaction as the editor form gives it.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ReactionSpec {
    pub id: String,
    pub name: String,
    /// A stoichiometry text (`a_c:-1, b_c:1`) or a JSON object.
    pub metabolites: Value,
    #[serde(rename = "lowerBound")]
    pub lower_bound: f64,
    #[serde(rename = "upperBound")]
    pub upper_bound: f64,
    #[serde(rename = "geneReactionRule")]
    pub gene_reaction_rule: String,
    pub subsystem: String,
}

impl ReactionSpec {
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("plain data serialises")
    }
}
