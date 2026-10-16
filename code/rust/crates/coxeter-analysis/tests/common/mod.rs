//! Helpers shared by the parity tests.
#![allow(dead_code)]

use std::sync::Arc;

use coxeter_core::community::{CrossFeeding, CrossFeedingLink, MediumLimit, MetaboliteExchange};
use coxeter_core::testkit::{read_fixture, repo_root};
use coxeter_core::{CommunityModel, CommunitySpec, Model, Objective, SpeciesSpec};
use indexmap::IndexMap;
use serde_json::{Value, json};

/// `rust/fixtures/analysis/<name>`, which `rust/tools/fixtures-analysis.mjs` writes.
pub fn fixture(name: &str) -> Value {
    read_fixture(&format!("analysis/{name}")).unwrap_or_else(|| panic!("analysis/{name} is missing: run node rust/tools/fixtures-analysis.mjs"))
}

pub fn fixture_bytes(name: &str) -> Vec<u8> {
    std::fs::read(repo_root().join("rust/fixtures/analysis").join(name)).unwrap()
}

pub fn repo(rel: &str) -> std::path::PathBuf {
    repo_root().join(rel)
}

/// A bundled model's document (`models/<id>.json`).
pub fn model_doc(id: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(repo(&format!("models/{id}.json"))).unwrap()).unwrap()
}

/// A bundled model with the fixture's synthetic gene rules applied to its
/// document, as the JS applied them before `new Model(doc)`.
pub fn model_with_rules(id: &str, rules: Option<&Value>) -> Model {
    let mut doc = model_doc(id);
    if let Some(rules) = rules.and_then(Value::as_object) {
        for r in doc["reactions"].as_array_mut().unwrap() {
            if let Some(rule) = rules.get(r["id"].as_str().unwrap()) {
                r["gene_reaction_rule"] = rule.clone();
            }
        }
    }
    Model::from_value(&doc).unwrap()
}

/// The models of a scenario spec, by file, with the gene rules the fixture records.
pub fn spec_models(spec: &Value, gprs: Option<&Value>) -> IndexMap<String, Arc<Model>> {
    let mut models = IndexMap::new();
    for sp in spec["species"].as_array().unwrap() {
        let file = sp["model"].as_str().unwrap();
        if !models.contains_key(file) {
            models.insert(file.to_string(), Arc::new(model_with_rules(file, gprs.and_then(|g| g.get(file)))));
        }
    }
    models
}

/// The community a fixture describes, merged as fixtures-analysis.mjs merged it.
pub fn community_from_spec(spec: &Value, models: &IndexMap<String, Arc<Model>>) -> CommunityModel {
    let mut medium = IndexMap::new();
    for (base, limit) in spec["medium"].as_object().unwrap() {
        medium.insert(base.clone(), MediumLimit { lower: limit["lower"].as_f64(), upper: limit["upper"].as_f64() });
    }
    let species = spec["species"]
        .as_array()
        .unwrap()
        .iter()
        .map(|sp| SpeciesSpec {
            id: Some(sp["id"].as_str().unwrap().into()),
            model: models[sp["model"].as_str().unwrap()].clone(),
            abundance: sp["abundance"].as_f64(),
            enabled: sp["enabled"].as_bool(),
            color: None,
        })
        .collect();
    CommunityModel::new(CommunitySpec {
        name: spec["name"].as_str().map(str::to_string),
        species,
        medium,
        objective: Objective::parse(spec["objective"].as_str().unwrap()),
        product_target: spec["productTarget"].as_str().map(str::to_string),
    })
    .unwrap()
}

/// A JSON number, with `null` and the strings JSON cannot hold read back.
pub fn num(v: &Value) -> f64 {
    match v {
        Value::String(s) if s == "NaN" => f64::NAN,
        Value::String(s) if s == "Infinity" => f64::INFINITY,
        Value::String(s) if s == "-Infinity" => f64::NEG_INFINITY,
        other => other.as_f64().unwrap_or(f64::NAN),
    }
}

/// An object of numbers as an ordered map.
pub fn number_map(v: &Value) -> IndexMap<String, f64> {
    v.as_object().map(|o| o.iter().map(|(k, x)| (k.clone(), num(x))).collect()).unwrap_or_default()
}

/// Equal to `tol` relative to their size (absolute below 1), NaN equal to NaN.
pub fn close(a: f64, b: f64, tol: f64) -> bool {
    if a.is_nan() || b.is_nan() {
        return a.is_nan() && b.is_nan();
    }
    (a - b).abs() <= tol * a.abs().max(b.abs()).max(1.0)
}

pub fn link_json(l: &CrossFeedingLink) -> Value {
    json!({"base": l.base, "from": l.from, "to": l.to, "flux": l.flux})
}

fn exchange_json(m: &MetaboliteExchange) -> Value {
    let shares = |v: &Vec<coxeter_core::community::FluxShare>| {
        v.iter().map(|s| json!({"speciesId": s.species_id, "flux": s.flux})).collect::<Vec<_>>()
    };
    json!({"base": m.base, "producers": shares(&m.producers), "consumers": shares(&m.consumers), "total": m.total})
}

/// `{links, perMetabolite}` as the JS returns it.
pub fn cross_feeding_json(c: &CrossFeeding) -> Value {
    json!({
        "links": c.links.iter().map(link_json).collect::<Vec<_>>(),
        "perMetabolite": c.per_metabolite.iter().map(exchange_json).collect::<Vec<_>>(),
    })
}

/// A value from an options object, with the fixture's string encodings.
pub fn option_number(v: Option<&Value>) -> Option<f64> {
    match v {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s == "undefined" => None,
        Some(x) => Some(num(x)),
    }
}
