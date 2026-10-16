//! Port of the Store tests of `test/metabolite-label.test.mjs`.
//!
//! Not ported here, as they test `EditableMap`, `src/map/metabolite-label.js`
//! or the renderer alone (no Store): "legacy label coordinates remain exactly
//! above every supported metabolite role", "labels orbit at the role radius
//! and align outward in each quadrant", "angle reads normalize complete turns
//! and reject nonnumeric or nonfinite state", "setting an eligible label angle
//! changes only its angle, including hidden nodes", "invalid angle input and
//! nonmetabolite ids are no-ops", "duplicate instances of one biological
//! metabolite keep independent label angles", "moving a label leaves
//! geometry, reaction membership, biological models and LP untouched",
//! "angles persist independently through detached map JSON save and load",
//! "legacy and invalid saved angles load with the original top placement" and
//! "SVG renders orbit positions and outward alignment without rotating label
//! text".
//!
//! Adapted: "Store delegates angle updates without emitting, solving,
//! rebuilding or recording history" replaces the store's `emit`, `solve`,
//! `rebuild`, `relayoutMap`, `beginMapEdit`, `commitMapEdit`,
//! `mapHistory.record` and `map.setMetaboliteLabelAngle` with spies, which
//! Rust cannot do. What the spies would catch is asserted on its effects
//! instead: no event is pending (`emit`), the undo history is empty
//! (`record`, `commitMapEdit`), the solution, the flux problem and the
//! community are the very objects the test installed (`solve`, `rebuild`;
//! `Arc::ptr_eq`, which an in-place edit would also break, the test holding a
//! second reference), the status is untouched (`solve`), and the drawing
//! changed in nothing but the angle (`relayoutMap`). The arguments handed to
//! `map.setMetaboliteLabelAngle` (`delegated`) are not observable; the
//! delegation shows in the node's angle. `setMetaboliteLabelAngle` returns
//! whether the node took the angle (the JS returns the node or null).

mod common;

use std::sync::Arc;

use coxeter_core::community::{CommunityModel, CommunitySpec, MediumLimit, SpeciesSpec};
use coxeter_core::metabolite_label::{DEFAULT_METABOLITE_LABEL_ANGLE, metabolite_label_angle};
use coxeter_core::model::Model;
use coxeter_map::EditableMap;
use coxeter_map::layout::LayoutOptions;
use coxeter_solver::fba::FluxProblem;
use coxeter_solver::lp::LpStatus;
use coxeter_state::operation_boundary::{OperationDomain, is_map_operation, operation_domain};
use coxeter_state::Solution;
use serde_json::{Value, json};

use common::{model, store};

/// `visualMap()`: one node of each role in a row, a hidden one and a piece of
/// furniture, on a 700 × 300 canvas.
fn visual_map() -> EditableMap {
    let nodes = [
        json!({ "id": "primary", "kind": "metabolite", "primary": true, "mergedId": "eco@glc_c", "base": "PrimaryLabel" }),
        json!({ "id": "secondary", "kind": "metabolite", "primary": false, "mergedId": "eco@p_c", "base": "SecondaryLabel" }),
        json!({ "id": "medium-primary", "kind": "medium", "primary": true, "mergedId": "M@glc_s", "base": "MediumLabel" }),
        json!({ "id": "medium-secondary", "kind": "medium", "primary": false, "mergedId": "M@p_s", "base": "SmallMediumLabel" }),
        json!({ "id": "currency", "kind": "currency", "mergedId": "eco@atp_c", "base": "CurrencyLabel" }),
        json!({ "id": "hidden", "kind": "metabolite", "mergedId": "eco@hidden_c", "base": "HiddenLabel", "hidden": true }),
        json!({ "id": "furniture", "kind": "marker", "base": "Furniture", "x": 75, "y": 75 }),
    ];
    let nodes: Vec<Value> = nodes
        .into_iter()
        .enumerate()
        .map(|(i, node)| {
            // `{ x: 100 + i * 50, y: 120, compartment: 'c', ...node }`
            let mut out = json!({ "x": 100 + i * 50, "y": 120, "compartment": "c" });
            for (k, v) in node.as_object().unwrap() {
                out[k] = v.clone();
            }
            out
        })
        .collect();
    EditableMap::from_json(&json!({
        "canvas": { "width": 700, "height": 300 },
        "environment": { "id": "s", "hidden": true, "rect": { "x": 0, "y": 0, "w": 700, "h": 300 } },
        "nodes": nodes,
        "cells": [],
        "reactions": [],
        "segments": [],
    }))
    .expect("a valid map")
}

/// `biologyFixture()`: a toy model, its community, the LP and the full map.
struct Biology {
    model: Model,
    community: Arc<CommunityModel>,
    problem: Arc<FluxProblem>,
    map: EditableMap,
}

fn biology_fixture() -> Biology {
    let model = model(json!({
        "id": "label-toy",
        "compartments": { "e": "Extracellular", "c": "Cytosol" },
        "metabolites": [
            { "id": "glc_e", "name": "Glucose", "compartment": "e" },
            { "id": "glc_c", "name": "Glucose", "compartment": "c" },
            { "id": "p_c", "name": "Product", "compartment": "c" },
        ],
        "reactions": [
            { "id": "EX_glc", "metabolites": { "glc_e": -1 }, "lower_bound": -10, "upper_bound": 1000 },
            { "id": "T_glc", "metabolites": { "glc_e": -1, "glc_c": 1 }, "lower_bound": 0, "upper_bound": 1000 },
            { "id": "MAKE_p", "metabolites": { "glc_c": -1, "p_c": 1 }, "lower_bound": 0, "upper_bound": 1000 },
            { "id": "BIOMASS", "metabolites": { "p_c": -1 }, "lower_bound": 0, "upper_bound": 1000, "objective_coefficient": 1 },
        ],
    }));
    let community = CommunityModel::new(CommunitySpec {
        species: vec![SpeciesSpec::new("eco", Arc::new(model.clone())).abundance(1.0)],
        medium: [("glc".to_string(), MediumLimit { lower: Some(-10.0), upper: None })].into_iter().collect(),
        ..Default::default()
    })
    .unwrap();
    let problem = FluxProblem::new(&community.merged);
    let map = EditableMap::from_community(&community, &LayoutOptions::default());
    Biology { model, community: Arc::new(community), problem: Arc::new(problem), map }
}

/// `biologySnapshot(fixture)`: the model, the merged model and its owners, and
/// the LP's ids and bounds.
fn biology_snapshot(model: &Model, community: &CommunityModel, problem: &FluxProblem) -> Value {
    let bounds = problem.snapshot_bounds();
    json!({
        "model": model.to_json(),
        "merged": community.merged.to_json(),
        "reactionOwner": community.reaction_owner.keys().collect::<Vec<_>>(),
        "metaboliteOwner": community.metabolite_owner.keys().collect::<Vec<_>>(),
        "metIds": problem.met_ids,
        "rxnIds": problem.rxn_ids,
        "lb": bounds.lb,
        "ub": bounds.ub,
        "objective": problem.objective_vector(),
    })
}

/// `withoutAngles(doc)`.
fn without_angles(mut doc: Value) -> Value {
    for node in doc["nodes"].as_array_mut().unwrap() {
        node.as_object_mut().unwrap().remove("labelAngle");
    }
    doc
}

/// "Store delegates angle updates without emitting, solving, rebuilding or recording history"
#[test]
fn store_delegates_angle_updates_without_emitting_solving_rebuilding_or_recording_history() {
    let fixture = biology_fixture();
    let mut store = store();
    store.map = Some(fixture.map.clone());
    store.community = Some(fixture.community.clone());
    store.problem = Some(fixture.problem.clone());
    // `{ status: 'optimal', fluxes: new Map([['eco@BIOMASS', 10]]) }`.
    store.solution = Some(Arc::new(Solution {
        status: LpStatus::Optimal,
        objective_value: 0.0,
        fluxes: [("eco@BIOMASS".to_string(), 10.0)].into_iter().collect(),
        message: String::new(),
        solver: String::new(),
        result: None,
        steady_com: None,
        extra: None,
        zuniga: None,
    }));
    let solution = store.solution.clone().unwrap();
    let status = store.status().clone();
    let before_biology = biology_snapshot(&fixture.model, &fixture.community, &fixture.problem);
    let before_map = without_angles(store.map.as_ref().unwrap().to_json());
    store.take_events();
    let node = store.map.as_ref().unwrap().nodes().values().find(|n| n.kind.as_str() == "metabolite").expect("a metabolite").id.clone();

    assert!(store.set_metabolite_label_angle(&node, 0.9), "the node takes the angle");
    assert!(!store.set_metabolite_label_angle(&node, f64::NAN), "NaN is refused");
    assert_eq!(store.map.as_ref().unwrap().node(&node).unwrap().label_angle, Some(0.9));
    // `calls` stays empty.
    assert!(store.take_events().is_empty(), "nothing is emitted");
    assert_eq!(store.map_history.undo_len(), 0, "no history is recorded");
    assert_eq!(*store.status(), status, "no solve ran");
    assert_eq!(without_angles(store.map.as_ref().unwrap().to_json()), before_map, "nothing was laid out again");
    assert!(Arc::ptr_eq(store.solution.as_ref().unwrap(), &solution), "the solution is the same");
    assert!(Arc::ptr_eq(store.problem.as_ref().unwrap(), &fixture.problem), "the problem is the same");
    assert!(Arc::ptr_eq(store.community.as_ref().unwrap(), &fixture.community), "the community is the same");
    assert_eq!(
        biology_snapshot(&fixture.model, store.community.as_ref().unwrap(), store.problem.as_ref().unwrap()),
        before_biology
    );
    assert!(!store.map_history.can_undo());
    store.map = None;
    assert!(!store.set_metabolite_label_angle(&node, 0.0), "no map, no angle");
    assert!(store.take_events().is_empty());
}

/// "a sequence of label updates forms one undoable map gesture when committed"
#[test]
fn a_sequence_of_label_updates_forms_one_undoable_map_gesture_when_committed() {
    let mut store = store();
    store.map = Some(visual_map());
    let node_id = "primary";
    let transaction = store.begin_map_edit("Move metabolite label");
    for angle in [0.2, 0.8, 1.5] {
        store.set_metabolite_label_angle(node_id, angle);
    }
    assert_eq!(store.map_history.undo_len(), 0);
    assert!(store.commit_map_edit(transaction));
    assert_eq!(store.map_history.undo_len(), 1);
    assert_eq!(store.undo_map().as_deref(), Some("Move metabolite label"));
    assert_eq!(metabolite_label_angle(store.map.as_ref().unwrap().node(node_id).unwrap().label_angle), DEFAULT_METABOLITE_LABEL_ANGLE);
    assert_eq!(store.redo_map().as_deref(), Some("Move metabolite label"));
    assert_eq!(store.map.as_ref().unwrap().node(node_id).unwrap().label_angle, Some(1.5));
}

/// "label angle editing is explicitly registered as a MAP operation"
#[test]
fn label_angle_editing_is_explicitly_registered_as_a_map_operation() {
    assert_eq!(operation_domain("setMetaboliteLabelAngle"), Some(OperationDomain::Map));
    assert!(is_map_operation("setMetaboliteLabelAngle"));
    let store = store();
    assert_eq!(store.editing_operation_domain("setMetaboliteLabelAngle"), Some(OperationDomain::Map));
    assert!(store.assert_map_operation("setMetaboliteLabelAngle").unwrap());
}
