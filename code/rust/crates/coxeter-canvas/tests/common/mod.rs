//! Shared by the canvas tests: the gesture references written by
//! `tools/fixtures-canvas.mjs` (the real app in headless Firefox), the host
//! they start from, and the rule maps are compared by.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Arc;

use coxeter_canvas::test_host::{CellTemplate, CompartmentTemplate, EditableMapHost};
use coxeter_canvas::host::TextStyle;
use coxeter_map::EditableMap;
use coxeter_render::state::LabelMode;
use coxeter_render::{FluxScale, RenderState};
use coxeter_canvas::targets::Target;
use serde_json::{Value, json};
use std::collections::BTreeSet;

/// `tests/fixtures/gestures.json`, if it was written.
pub fn gestures() -> Option<Value> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gestures.json");
    let text = std::fs::read_to_string(path).ok()?;
    Some(serde_json::from_str(&text).expect("gestures.json"))
}

fn map_of(v: &Value) -> HashMap<String, f64> {
    v.as_object().map(|m| m.iter().filter_map(|(k, x)| Some((k.clone(), x.as_f64()?))).collect()).unwrap_or_default()
}

fn strings_of(v: &Value) -> HashMap<String, String> {
    v.as_object().map(|m| m.iter().filter_map(|(k, x)| Some((k.clone(), x.as_str()?.to_string()))).collect()).unwrap_or_default()
}

/// What `mapRenderState()` told the renderer in the browser, besides the
/// selection (which the host keeps).
pub fn render_state(doc: &Value) -> RenderState {
    let r = &doc["render"];
    let fluxes = map_of(&r["fluxes"]);
    RenderState {
        scale: Arc::new(FluxScale::automatic(fluxes.values().copied(), false)),
        fluxes,
        solved_fluxes: Some(map_of(&r["solvedFluxes"])),
        growth: map_of(&r["growth"]),
        labels: LabelMode::parse(r["labels"].as_str().unwrap_or("auto")),
        show_currency: r["showCurrency"].as_bool().unwrap_or(true),
        currency_labels: r["currencyLabels"].as_bool().unwrap_or(false),
        metabolite_labels: strings_of(&r["metaboliteLabels"]),
        reaction_labels: strings_of(&r["reactionLabels"]),
        ..RenderState::default()
    }
}

/// The host a case starts from: its map, the browser's render state, and
/// what the models offer the Cell and Compartment tools.
pub fn host_for(doc: &Value, case: &Value) -> EditableMapHost {
    let map_json = if case["start"] == "own" { &case["map"] } else { &doc["initial"] };
    let map = EditableMap::from_json(map_json).expect("a map");
    let mut host = EditableMapHost::new(map);
    host.render = Some(render_state(doc));
    host.view.show_currency = doc["render"]["showCurrency"].as_bool().unwrap_or(true);
    if let Some(style) = doc["render"]["textStyle"].as_object() {
        host.view.text_style = TextStyle {
            font_size: style.get("fontSize").and_then(Value::as_f64).unwrap_or(22.0),
            font_family: style.get("fontFamily").and_then(Value::as_str).unwrap_or("sans").to_string(),
        };
    }
    let s = |v: &Value, k: &str| v[k].as_str().unwrap_or("").to_string();
    host.cells = doc["cells"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| CellTemplate {
            species_id: s(c, "speciesId"),
            outer_compartment_id: s(c, "outerCompartmentId"),
            outer_label: s(c, "outerCompartmentName"),
            cell_name: s(c, "cellName"),
            organism: s(c, "organism"),
            note: s(c, "note"),
            color: s(c, "color"),
            abundance: c["abundance"].as_f64(),
        })
        .collect();
    host.compartments = doc["compartments"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| CompartmentTemplate {
            species_id: s(c, "speciesId"),
            compartment_id: s(c, "compartmentId"),
            parent_id: s(c, "parentId"),
            label: s(c, "label"),
            depth: c["depth"].as_u64().unwrap_or(0) as usize,
            organism: s(c, "organism"),
            note: s(c, "note"),
            color: s(c, "color"),
            abundance: c["abundance"].as_f64(),
            cell_name: s(c, "speciesName"),
        })
        .collect();
    host
}

/// A map's JSON as both sides are compared (coxeter-map's rule): `null` and
/// absent are the same, and `backboneAnchor` / `transportLane` are only ever
/// written as `true`.
pub fn canonical(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.iter()
                .filter(|(k, x)| !x.is_null() && !((k.as_str() == "backboneAnchor" || k.as_str() == "transportLane") && **x == Value::Bool(false)))
                .map(|(k, x)| (k.clone(), canonical(x)))
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(canonical).collect()),
        other => other.clone(),
    }
}

/// Apply the edits `diff()` of the fixture script wrote: `[path]` removes,
/// `[path, value]` sets.
pub fn apply_edits(doc: &Value, edits: &Value) -> Value {
    let mut out = doc.clone();
    for edit in edits.as_array().into_iter().flatten() {
        let path = edit[0].as_array().expect("a path");
        let value = edit.get(1).cloned();
        if path.is_empty() {
            out = value.unwrap_or(Value::Null);
            continue;
        }
        let mut at = &mut out;
        for key in &path[..path.len() - 1] {
            at = match key {
                Value::String(k) => &mut at[k.as_str()],
                Value::Number(n) => &mut at[n.as_u64().unwrap() as usize],
                _ => unreachable!(),
            };
        }
        match (&path[path.len() - 1], value) {
            (Value::String(k), Some(v)) => {
                at.as_object_mut().expect("an object").insert(k.clone(), v);
            }
            (Value::String(k), None) => {
                at.as_object_mut().expect("an object").shift_remove(k);
            }
            (Value::Number(n), Some(v)) => at[n.as_u64().unwrap() as usize] = v,
            _ => panic!("bad edit {edit}"),
        }
    }
    out
}

/// What the browser reports for an element, from a Rust target.
pub fn describe(t: &Target) -> Value {
    match t.element() {
        None => json!({ "tag": "div", "cls": "stage", "data": {} }),
        Some(e) => {
            let data: serde_json::Map<String, Value> = e
                .attrs
                .iter()
                .filter(|a| a.name.starts_with("data-"))
                .map(|a| (a.name.to_string(), Value::String(a.value.as_deref().unwrap_or("").to_string())))
                .collect();
            json!({ "tag": e.name, "cls": e.get("class").unwrap_or(""), "data": data })
        }
    }
}

pub fn classes(v: &Value) -> BTreeSet<String> {
    v["cls"].as_str().unwrap_or("").split_ascii_whitespace().filter(|c| !c.starts_with("is-")).map(str::to_string).collect()
}

/// Whether two element descriptions name the same element.
pub fn same_target(rust: &Value, js: &Value) -> bool {
    if js.is_null() {
        return false;
    }
    let js_stage = js["tag"] == "div";
    let rust_stage = rust["tag"] == "div";
    if js_stage || rust_stage {
        return js_stage == rust_stage;
    }
    rust["tag"] == js["tag"] && rust["data"] == js["data"] && classes(rust) == classes(js)
}

