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
use crate::gene_rules::{evaluate_gene_rule, gene_rule_genes};
use crate::jscompat::{self, DOT, NS, S, js_trim, object_entries, string_or_empty};

fn err(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::Edit, message)
}

fn input_number(value: Option<&Value>) -> f64 {
    match value {
        Some(value @ Value::Number(_)) => js::to_number(value),
        Some(value @ Value::String(text)) if !js_trim(text).is_empty() => js::to_number(value),
        _ => f64::NAN,
    }
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
        let coefficient = input_number(Some(&coefficient_raw));
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
    for (id, coefficient) in &out {
        if !coefficient.is_finite() {
            return Err(err(format!("The net coefficient of {id} must be finite.")));
        }
    }
    out.retain(|_, c| c.abs() >= 1e-15);
    if out.is_empty() {
        return Err(err("The net stoichiometry came out empty."));
    }
    Ok(out)
}

/// The gene ids a rule mentions, in order, without the `and`/`or` words.
pub fn gene_ids_from_gpr(rule: &str) -> Vec<String> {
    gene_rule_genes(rule)
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
    let id = valid_id(field(spec, "id"), "metabolite")?;
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
        Some(Value::String(s)) if js_trim(s).is_empty() => 0.0,
        Some(v) => input_number(Some(v)),
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
                let coefficient = input_number(Some(&coefficient_raw));
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

    let lower = input_number(field(spec, "lowerBound"));
    let upper = input_number(field(spec, "upperBound"));
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
    if let Err(error) = evaluate_gene_rule(&gpr, &|_| false) {
        return Err(err(format!("Invalid gene rule: {}", error.message)));
    }
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

#[cfg(test)]
mod creation_validation_tests {
    use super::*;
    use crate::gene_rules::gene_rule_genes;

    fn model() -> Model {
        Model::from_json_str(include_str!("../../../../models/ecoli_core_mc.json")).expect("bundled model")
    }

    fn reaction_spec() -> Value {
        json!({
            "id": "QA_PGI",
            "name": "QA parallel glucose isomerase",
            "metabolites": "g6p_c:-1, f6p_c:1",
            "lowerBound": -10,
            "upperBound": 10,
            "geneReactionRule": "qa_a and (qa_b or qa_c)",
        })
    }

    #[test]
    fn cleared_or_nonnumeric_required_bounds_cannot_silently_create_a_zero_bound() {
        let mut model = model();
        let before = model.to_json();
        for side in ["lowerBound", "upperBound"] {
            for value in [json!(""), json!("  "), Value::Null, json!(true), json!([]), json!([1]), json!({})] {
                let mut spec = reaction_spec();
                spec[side] = value;
                let error = add_reaction_to_model(&mut model, &spec).expect_err("invalid bound");
                assert!(error.message.contains("bounds must be numbers"));
                assert_eq!(model.to_json(), before);
            }
            let mut spec = reaction_spec();
            spec.as_object_mut().expect("spec").remove(side);
            assert!(add_reaction_to_model(&mut model, &spec).is_err());
            assert_eq!(model.to_json(), before);
        }
    }

    #[test]
    fn malformed_new_gene_rules_are_rejected_before_reactions_or_genes_are_inserted() {
        let mut model = model();
        let before = model.to_json();
        for rule in ["qa_a and (qa_b", "qa_a or", "qa_a qa_b", "and qa_a", "qa_a and ()", "not qa_a"] {
            let mut spec = reaction_spec();
            spec["geneReactionRule"] = json!(rule);
            let error = add_reaction_to_model(&mut model, &spec).expect_err("invalid gene rule");
            assert!(error.message.to_lowercase().contains("gene rule"));
            assert_eq!(model.to_json(), before);
        }
    }

    #[test]
    fn empty_and_spaced_metabolite_identifiers_report_english_validation_errors() {
        let mut model = model();
        let before = model.to_json();
        for id in ["", "qa metabolite"] {
            let spec = json!({ "id": id, "name": "QA metabolite", "compartment": "c" });
            let error = add_metabolite_to_model(&mut model, &spec).expect_err("invalid ID");
            assert!(error.message.contains("metabolite ID"));
            assert_eq!(model.to_json(), before);
        }
    }

    #[test]
    fn gene_metadata_agrees_with_knockout_parsing_for_every_accepted_gene_identifier() {
        let mut model = model();
        let mut spec = reaction_spec();
        let rule = "not or (true and false)";
        spec["geneReactionRule"] = json!(rule);
        add_reaction_to_model(&mut model, &spec).expect("valid gene IDs");
        let genes = gene_rule_genes(rule);
        assert_eq!(gene_ids_from_gpr(rule), genes);
        for id in genes {
            assert!(model.genes.iter().any(|gene| gene["id"] == id));
        }
    }

    #[test]
    fn stoichiometry_preserves_identifiers_that_match_inherited_object_properties() {
        let mut model = model();
        for id in ["__proto__", "constructor", "toString"] {
            add_metabolite_to_model(&mut model, &json!({
                "id": id, "name": format!("QA {id}"), "compartment": "c",
            })).expect("new metabolite");
            let text = format!("{id}:-1, f6p_c:1");
            let parsed = parse_stoichiometry(&text).expect("stoichiometry");
            assert_eq!(parsed[id], -1.0);
            let reaction_id = format!("QA_{id}");
            add_reaction_to_model(&mut model, &json!({
                "id": reaction_id, "metabolites": text, "lowerBound": 0, "upperBound": 10,
            })).expect("new reaction");
            let round_trip = Model::from_value(&model.to_json()).expect("model round trip");
            assert_eq!(round_trip.reactions[&reaction_id].metabolites[id], -1.0);
            let json = json!({ id: -1, "f6p_c": 1 }).to_string();
            assert_eq!(parse_stoichiometry(&json).expect("JSON stoichiometry"), parsed);
            assert_eq!(parse_stoichiometry(&format!("{id}:-1, {id}:1, f6p_c:1")).expect("cancelled term"),
                IndexMap::from([("f6p_c".into(), 1.0)]));
        }
    }

    #[test]
    fn repeated_finite_coefficients_cannot_overflow_to_an_infinite_stoichiometry() {
        let error = parse_stoichiometry("g6p_c:1e308, g6p_c:1e308, f6p_c:-1").expect_err("overflow");
        assert!(error.message.contains("g6p_c") && error.message.contains("finite"));
        assert_eq!(parse_stoichiometry("g6p_c:1e308, g6p_c:-1e308, f6p_c:1").expect("finite cancellation"),
            IndexMap::from([("f6p_c".into(), 1.0)]));
    }

    #[test]
    fn json_stoichiometry_rejects_booleans_and_arrays_instead_of_converting_them_to_coefficients() {
        for value in [json!(true), json!(false), json!([]), json!([1]), json!(["-2"]), json!([[1]]), json!({}), Value::Null, json!("")] {
            let text = json!({ "g6p_c": value, "f6p_c": 1 }).to_string();
            let error = parse_stoichiometry(&text).expect_err("invalid coefficient");
            assert!(error.message.contains("coefficient") && error.message.contains("g6p_c"));
        }
    }

    #[test]
    fn object_stoichiometry_rejects_nonscalar_coefficients_before_changing_a_model() {
        let mut model = model();
        let before = model.to_json();
        for value in [json!(true), json!(false), json!([]), json!([1]), json!(["-2"]), json!([[1]]), json!({}), Value::Null, json!("")] {
            let spec = json!({
                "id": "QA_NUMBERS", "metabolites": { "g6p_c": value, "f6p_c": 1 },
                "lowerBound": -10, "upperBound": 10,
            });
            let error = add_reaction_to_model(&mut model, &spec).expect_err("invalid coefficient");
            assert!(error.message.contains("coefficient") && error.message.contains("g6p_c"));
            assert_eq!(model.to_json(), before);
        }
    }

    #[test]
    fn optional_charge_rejects_booleans_and_arrays_before_changing_a_model() {
        let mut model = model();
        let before = model.to_json();
        for value in [json!(true), json!(false), json!([]), json!([1]), json!(["-2"]), json!([[1]]), json!({})] {
            let spec = json!({ "id": "qa_numbers_c", "name": "QA numbers", "compartment": "c", "charge": value });
            let error = add_metabolite_to_model(&mut model, &spec).expect_err("invalid charge");
            assert!(error.message.contains("charge must be numeric"));
            assert_eq!(model.to_json(), before);
        }
    }

    #[test]
    fn numeric_strings_and_numbers_retain_their_signs_fractions_and_exponents() {
        let mut model = model();
        let parsed = parse_stoichiometry(r#"{"g6p_c":" -2e-1 ","f6p_c":0.2}"#).expect("numeric coefficients");
        let reaction = add_reaction_to_model(&mut model, &json!({
            "id": "QA_NUMBERS", "metabolites": { "g6p_c": "-0.2", "f6p_c": 0.2 },
            "lowerBound": "-1e1", "upperBound": " 2.5 ",
        })).expect("numeric reaction");
        assert_eq!(reaction.metabolites, parsed);
        assert_eq!(reaction.lower_bound, -10.0);
        assert_eq!(reaction.upper_bound, 2.5);
        let metabolite = add_metabolite_to_model(&mut model, &json!({
            "id": "qa_numbers_c", "name": "QA numbers", "compartment": "c", "charge": "-2",
        })).expect("numeric charge");
        assert_eq!(metabolite.charge, -2.0);
    }

    #[test]
    fn omitted_and_blank_optional_charges_remain_zero() {
        let mut model = model();
        for (index, value) in [Value::Null, json!(""), json!("  "), json!(0), json!("0")].into_iter().enumerate() {
            let metabolite = add_metabolite_to_model(&mut model, &json!({
                "id": format!("qa_blank_{index}_c"), "name": "QA blank charge", "compartment": "c", "charge": value,
            })).expect("blank charge");
            assert_eq!(metabolite.charge, 0.0);
        }
        let metabolite = add_metabolite_to_model(&mut model, &json!({
            "id": "qa_omitted_c", "name": "QA blank charge", "compartment": "c",
        })).expect("omitted charge");
        assert_eq!(metabolite.charge, 0.0);
    }
}
