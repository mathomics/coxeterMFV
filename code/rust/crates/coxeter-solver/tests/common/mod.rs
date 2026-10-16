//! Helpers shared by the parity tests.
#![allow(dead_code)]

use std::sync::Arc;

use coxeter_core::community::MediumLimit;
use coxeter_core::testkit::repo_root;
use coxeter_core::{CommunityModel, CommunitySpec, Model, Objective, SpeciesSpec};
use serde_json::Value;

pub fn load_model(id: &str) -> Model {
    let path = repo_root().join("models").join(format!("{id}.json"));
    Model::from_json_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
}

pub fn read_json(path: &std::path::Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// The community a fixture describes, merged as fixtures-solver.mjs merged it.
pub fn community_from_spec(spec: &Value) -> CommunityModel {
    let mut medium = indexmap::IndexMap::new();
    for (base, limit) in spec["medium"].as_object().unwrap() {
        medium.insert(base.clone(), MediumLimit { lower: limit["lower"].as_f64(), upper: limit["upper"].as_f64() });
    }
    let species = spec["species"]
        .as_array()
        .unwrap()
        .iter()
        .map(|sp| SpeciesSpec {
            id: Some(sp["id"].as_str().unwrap().into()),
            model: Arc::new(load_model(sp["model"].as_str().unwrap())),
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

/// A JSON number, with `null` read as NaN (JSON.stringify writes NaN as null).
pub fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or(f64::NAN)
}

/// Equal to `tol` relative to their size (absolute below 1), NaN equal to NaN.
pub fn close(a: f64, b: f64, tol: f64) -> bool {
    if a.is_nan() || b.is_nan() {
        return a.is_nan() && b.is_nan();
    }
    (a - b).abs() <= tol * a.abs().max(b.abs()).max(1.0)
}
