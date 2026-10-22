//! Port of `test/fva-mode.test.mjs` (the Store parts).
//!
//! Not ported here:
//! - "progressive FVA matches synchronous FVA for the same reactions" and
//!   "progressive FVA can be cancelled between reactions": `FluxProblem` only,
//!   no Store; ported in `coxeter-solver/tests/js_unit_fba.rs`.
//!
//! Adapted:
//! - "Store FVA mode computes and exposes global ranges": the JS stubs
//!   `store.problem` with a fake `variabilityAsync`. The Rust store runs the
//!   real `FluxProblem::variability` on a background job, so the stub is a
//!   real two-reaction problem whose flux ranges are exactly the ranges the
//!   stub returned (R1 ∈ [-1, 3], R2 ∈ [0, 5]: no mass balance ties them and
//!   the objective is zero). The `setTimeout(15)` wait is `wait_for_jobs`.
//! - "FVA mode renders min and max beside every drawn reaction" and "FVA mode
//!   marks reactions whose range is still pending": the JS calls `renderMap`
//!   with a hand-built state. Here the same map is put in a store and drawn
//!   with what the store hands the renderer (`map_render_state`), so the
//!   assertions also cover the store passing `fvaEnabled` and `fvaRanges` on.

mod common;

use std::sync::Arc;

use coxeter_core::model::Model;
use coxeter_map::editable_map::EditableMap;
use coxeter_render::mapview::render_map_svg;
use coxeter_solver::fba::FluxProblem;
use coxeter_state::{FvaRange, RenderOptions, Solution};
use coxeter_solver::lp::LpStatus;
use indexmap::IndexMap;
use serde_json::json;

use common::{WAIT, store};

/// `{ status: 'optimal', objectiveValue, fluxes }`.
fn solution(objective_value: f64, fluxes: &[(&str, f64)]) -> Solution {
    Solution {
        status: LpStatus::Optimal,
        objective_value,
        fluxes: fluxes.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        message: String::new(),
        solver: "highs".into(),
        result: None,
        steady_com: None,
        extra: None,
        zuniga: None,
    }
}

/// Two reactions with nothing tying them together: their flux ranges are
/// their bounds.
fn two_free_reactions() -> Model {
    Model::from_value(&json!({
        "id": "fva_stub",
        "compartments": {"c": "cytosol"},
        "metabolites": [],
        "reactions": [
            {"id": "R1", "metabolites": {}, "lower_bound": -1, "upper_bound": 3},
            {"id": "R2", "metabolites": {}, "lower_bound": 0, "upper_bound": 5}
        ]
    }))
    .expect("a valid model")
}

#[test]
fn store_fva_mode_computes_and_exposes_global_ranges() {
    let mut store = store();
    let problem = FluxProblem::new(&two_free_reactions());
    assert_eq!(problem.rxn_ids, ["R1", "R2"]);
    store.problem = Some(Arc::new(problem));
    store.solution = Some(Arc::new(solution(1.0, &[("R1", 1.0), ("R2", 2.0)])));

    store.set_fva(true).unwrap();
    store.wait_for_jobs(WAIT);

    assert!(store.fva_enabled);
    assert_eq!(store.fva_ranges.get("R1"), Some(&FvaRange { min: -1.0, max: 3.0 }));
    assert_eq!(store.fva_ranges.get("R2"), Some(&FvaRange { min: 0.0, max: 5.0 }));
    assert!(!store.fva_progress.running);
    assert_eq!(store.fva_progress.done, 2);

    store.select(Some("R1"));
    assert_eq!(store.variability, Some(FvaRange { min: -1.0, max: 3.0 }));

    store.set_fva(false).unwrap();
    assert!(!store.fva_enabled);
    assert_eq!(store.fva_ranges.len(), 0);
    assert_eq!(store.variability, None);
}

#[test]
fn scenario_json_persists_fva_checkbox_state() {
    let mut store = store();
    store.fva_enabled = true;
    let doc: serde_json::Value = serde_json::from_str(&store.state_json()).unwrap();
    assert_eq!(doc["fvaEnabled"], json!(true));
}

/// The one-reaction map both render tests draw.
fn one_reaction_map() -> EditableMap {
    EditableMap::from_json(&json!({
        "canvas": {"width": 400, "height": 240},
        "environment": {"id": "u", "label": "Medium", "rect": {"x": 0, "y": 0, "w": 400, "h": 240}},
        "cells": [],
        "medium": {"nodes": [], "exchanges": []},
        "nodes": [],
        "reactions": [{"id": "R1", "localId": "R1", "x": 200, "y": 120, "onMembrane": false, "compartments": []}],
        "segments": []
    }))
    .expect("a valid map")
}

#[test]
fn fva_mode_renders_min_and_max_beside_every_drawn_reaction() {
    let mut store = store();
    store.map = Some(one_reaction_map());
    store.apply_solution(solution(2.0, &[("R1", 2.0)]), false);
    store.view.labels = "none".into();
    store.view.show_cross_feeding = false;
    store.fva_enabled = true;
    store.fva_ranges = IndexMap::from([("R1".to_string(), FvaRange { min: -1.25, max: 4.5 })]);

    let state = store.map_render_state(RenderOptions::default());
    let svg = render_map_svg(store.map.as_ref().unwrap().graph(), &state);

    assert!(svg.contains("R1  [-1.25, 4.5]"));
    assert!(svg.contains("emc-label-fva"));
}

#[test]
fn fva_mode_marks_reactions_whose_range_is_still_pending() {
    let mut store = store();
    store.map = Some(one_reaction_map());
    store.apply_solution(solution(2.0, &[("R1", 2.0)]), false);
    store.view.show_cross_feeding = false;
    store.fva_enabled = true;
    store.fva_ranges = IndexMap::new();

    let state = store.map_render_state(RenderOptions::default());
    let svg = render_map_svg(store.map.as_ref().unwrap().graph(), &state);

    assert!(svg.contains("R1  [computing…]"));
}
