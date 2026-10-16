//! Shared by the parity tests: the bundled models and scenarios, built as
//! `rust/tools/fixtures-map.mjs` builds them, and the comparison rule.
#![allow(dead_code)]

use std::sync::Arc;

use coxeter_core::community::{CommunityModel, CommunitySpec, MediumLimit, Objective, SpeciesSpec};
use coxeter_core::model::Model;
use coxeter_core::testkit::{read_fixture, repo_root};
use coxeter_map::layout::{Contents, LayoutOptions};
use indexmap::IndexMap;
use serde_json::Value;

pub fn load_model(name: &str) -> Model {
    let text = std::fs::read_to_string(repo_root().join(format!("models/{name}.json"))).unwrap();
    Model::from_json_str(&text).unwrap()
}

/// The community of a bundled scenario.
pub fn scenario_community(id: &str) -> CommunityModel {
    let text = std::fs::read_to_string(repo_root().join("models/scenarios.json")).unwrap();
    let raw: Value = serde_json::from_str(&text).unwrap();
    let s = raw["scenarios"].as_array().unwrap().iter().find(|s| s["id"] == id).unwrap_or_else(|| panic!("no scenario {id}"));
    let n = s["species"].as_array().unwrap().len() as f64;
    let mut medium = IndexMap::new();
    if let Some(m) = s["medium"].as_object() {
        for (base, limit) in m {
            let l = coxeter_core::js::to_number(limit);
            let l = if l.is_nan() { 0.0 } else { l };
            medium.insert(base.clone(), MediumLimit { lower: Some(-l.abs()), upper: Some(1000.0) });
        }
    }
    let species = s["species"]
        .as_array()
        .unwrap()
        .iter()
        .map(|sp| SpeciesSpec {
            id: Some(sp["id"].as_str().unwrap().into()),
            model: Arc::new(load_model(sp["model"].as_str().unwrap())),
            abundance: Some(sp["abundance"].as_f64().unwrap_or(1.0 / n)),
            enabled: Some(sp["enabled"].as_bool() != Some(false)),
            color: None,
        })
        .collect();
    CommunityModel::new(CommunitySpec {
        name: s["name"].as_str().map(String::from),
        species,
        medium,
        objective: Some(Objective::parse(s["objective"].as_str().unwrap_or("equal-growth")).unwrap_or_default()),
        product_target: s["productTarget"].as_str().filter(|t| !t.is_empty()).map(String::from),
    })
    .unwrap()
}

/// Layout options as the fixture script wrote them.
pub fn layout_options(v: &Value) -> LayoutOptions {
    LayoutOptions {
        seed: v["seed"].as_f64(),
        contents: if v["contents"] == "none" { Contents::None } else { Contents::All },
        width: v["width"].as_f64(),
        height: v["height"].as_f64(),
        aspect: v["aspect"].as_f64(),
        cell_gap: v["cellGap"].as_f64(),
        min_shell: v["minShell"].as_f64(),
        node_spacing: v["nodeSpacing"].as_f64(),
        cell_area_multiplier: v["cellAreaMultiplier"].as_f64(),
        reactions: None,
    }
}

/// A fixture by its path under `rust/fixtures/` (`map/…` or `large/map/…`);
/// None when it was not generated (large ones are not committed).
pub fn fixture(rel: &str) -> Option<Value> {
    read_fixture(rel)
}

/// A map's JSON as both sides are compared: `null` and absent are the same
/// to every reader of a map, and the core types leave out `false` for two
/// segment flags JavaScript only ever writes as `true`.
pub fn canonical(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.iter()
                .filter(|(k, v)| !v.is_null() && !((k.as_str() == "backboneAnchor" || k.as_str() == "transportLane") && **v == Value::Bool(false)))
                .map(|(k, v)| (k.clone(), canonical(v)))
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(canonical).collect()),
        other => other.clone(),
    }
}

/// Is this entry one the comparison leaves out (see [`canonical`])?
fn left_out(key: &str, v: &Value) -> bool {
    v.is_null() || ((key == "backboneAnchor" || key == "transportLane") && *v == Value::Bool(false))
}

/// The tolerance the parity tests recorded before d7b2c0e compare with.
/// That commit rewrote `closestPerimeterPoint` for elliptical corners
/// (`compartmentCorners`): every boundary projection, circular corners
/// included, goes through the ellipse form, which gives the same points
/// to the last bit or two. Not regenerated (lean testing); a relative 1e-9
/// still catches any real disagreement.
pub const PROJECTION_TOL: f64 = 1e-9;

/// [`json_diff`] of the two [`canonical`] forms, without building them:
/// where two maps first differ, numbers compared exactly when `tol` is 0.
pub fn map_diff(a: &Value, b: &Value, tol: f64) -> Option<String> {
    fn walk(a: &Value, b: &Value, tol: f64, path: &mut String) -> Option<String> {
        match (a, b) {
            (Value::Number(x), Value::Number(y)) => {
                let (x, y) = (x.as_f64().unwrap(), y.as_f64().unwrap());
                let same = if tol == 0.0 { x == y } else { (x - y).abs() <= tol * x.abs().max(y.abs()).max(1.0) };
                (!same).then(|| format!("{path}: {x} != {y}"))
            }
            (Value::Array(x), Value::Array(y)) => {
                if x.len() != y.len() {
                    return Some(format!("{path}: array length {} != {}", x.len(), y.len()));
                }
                for (i, (p, q)) in x.iter().zip(y).enumerate() {
                    let len = path.len();
                    path.push_str(&format!("[{i}]"));
                    if let Some(d) = walk(p, q, tol, path) {
                        return Some(d);
                    }
                    path.truncate(len);
                }
                None
            }
            (Value::Object(x), Value::Object(y)) => {
                for (k, p) in x {
                    if left_out(k, p) {
                        continue;
                    }
                    match y.get(k).filter(|q| !left_out(k, q)) {
                        None => return Some(format!("{path}.{k}: only on the left ({p})")),
                        Some(q) => {
                            let len = path.len();
                            path.push('.');
                            path.push_str(k);
                            if let Some(d) = walk(p, q, tol, path) {
                                return Some(d);
                            }
                            path.truncate(len);
                        }
                    }
                }
                for (k, q) in y {
                    if !left_out(k, q) && !x.get(k).is_some_and(|p| !left_out(k, p)) {
                        return Some(format!("{path}.{k}: only on the right ({q})"));
                    }
                }
                None
            }
            _ => (a != b).then(|| format!("{path}: {a} != {b}")),
        }
    }
    walk(a, b, tol, &mut String::from("$"))
}

/// Panic with the first difference between a map and the JavaScript's.
#[track_caller]
pub fn assert_map_eq(actual: &Value, expected: &Value, tol: f64, what: &str) {
    if let Some(d) = map_diff(actual, expected, tol) {
        let d: String = d.chars().take(600).collect();
        panic!("{what} differs from the JavaScript output at {d}");
    }
}

/// An input file the fixture script read: under the repository, or (for the
/// user's own files) under $HOME. None when it is not on this machine.
pub fn input_path(entry: &Value) -> Option<std::path::PathBuf> {
    let rel = entry["input"].as_str()?;
    let path = if entry["home"].as_bool() == Some(true) {
        std::path::PathBuf::from(std::env::var("HOME").ok()?).join(rel)
    } else {
        repo_root().join(rel)
    };
    path.exists().then_some(path)
}

pub mod edits;

/// Apply one `mapDelta` of the fixture script to the running expected map.
pub fn apply_delta(expected: &mut Value, delta: &Value) {
    if let Some(full) = delta.get("full") {
        *expected = full.clone();
        return;
    }
    let map = expected.as_object_mut().expect("a map document");
    for key in ["version", "canvas", "environment", "cells", "medium", "labelSettings"] {
        match delta.get(key) {
            None => {}
            Some(Value::Null) => {
                map.remove(key);
            }
            Some(v) => {
                map.insert(key.into(), v.clone());
            }
        }
    }
    for key in ["nodes", "reactions", "texts"] {
        let Some(d) = delta.get(key) else { continue };
        let mut by_id: IndexMap<String, Value> = map[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| (x["id"].as_str().unwrap().to_string(), x.clone()))
            .collect();
        for x in d["set"].as_array().unwrap() {
            by_id.insert(x["id"].as_str().unwrap().to_string(), x.clone());
        }
        let list: Vec<Value> = match d.get("order").and_then(Value::as_array) {
            Some(order) => order.iter().map(|id| by_id[id.as_str().unwrap()].clone()).collect(),
            None => by_id.into_values().collect(),
        };
        map.insert(key.into(), Value::Array(list));
    }
    if let Some(d) = delta.get("segments") {
        let segments = map["segments"].as_array_mut().unwrap();
        segments.resize(d["length"].as_u64().unwrap() as usize, Value::Null);
        for (i, seg) in d["set"].as_object().unwrap() {
            segments[i.parse::<usize>().unwrap()] = seg.clone();
        }
    }
}

/// A community of one uploaded model, as the fixture script builds it.
pub fn single_species_community(species_id: &str, model: Model) -> CommunityModel {
    CommunityModel::new(CommunitySpec {
        name: None,
        species: vec![SpeciesSpec { id: Some(species_id.into()), model: Arc::new(model), abundance: Some(1.0), enabled: Some(true), color: None }],
        medium: IndexMap::new(),
        objective: None,
        product_target: None,
    })
    .unwrap()
}

/// A JavaScript `parseMapDocument` result, read back from its JSON.
pub fn parsed_from_json(v: &Value) -> coxeter_map::map_document::ParsedMapDocument {
    use coxeter_map::map_document::{FluxColorStop, ParsedMapDocument};
    let non_null = |k: &str| v.get(k).filter(|x| !x.is_null()).cloned();
    ParsedMapDocument {
        format: v["format"].as_str().unwrap_or("").to_string(),
        version: v["version"].as_f64().unwrap_or(f64::NAN),
        metadata: non_null("metadata"),
        pathway_manager: non_null("pathwayManager"),
        flux_color_stops: v.get("fluxColorStops").and_then(Value::as_array).map(|l| {
            l.iter().map(|s| FluxColorStop { value: s["value"].as_f64().unwrap(), color: s["color"].as_str().unwrap().into() }).collect()
        }),
        study: non_null("study"),
        map: v.get("map").cloned().unwrap_or(Value::Null),
        legacy: v["legacy"].as_bool().unwrap_or(false),
    }
}
