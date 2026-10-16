//! Port of `test/flux-color-stops.test.mjs` (the Store parts).
//!
//! Not ported here: "custom flux stops are sorted and owned copies, with no
//! arbitrary point limit", "invalid flux stops are rejected instead of
//! becoming a misleading palette", "negative, zero and positive fluxes use
//! their exact signed color anchors", "RGB interpolation uses real flux
//! distances rather than log or normalized magnitude", "custom flux colors
//! clamp outside anchors and one anchor gives a constant color" and
//! "automatic colors keep their existing behavior and custom stops never
//! change flux widths": `normalizeFluxColorStops` and `FluxScale` only, no
//! Store; ported in `coxeter-render/tests/js_tests.rs`.
//!
//! Object identity: `assert.equal(store.community, before.community)` (and
//! `problem`, `solution`) is `Arc::ptr_eq`; the map is owned by the store, so
//! "the same map" is the same map JSON.

mod common;

use std::sync::Arc;

use coxeter_render::theme::FluxColorStop;
use coxeter_state::Store;
use serde_json::{Value, json};

use common::{model, store, view_patch};

fn stops() -> Vec<FluxColorStop> {
    vec![
        FluxColorStop { value: -20.0, color: "#ff0000".into() },
        FluxColorStop { value: 0.0, color: "#ffffff".into() },
        FluxColorStop { value: 5.0, color: "#0000ff".into() },
    ]
}

fn stops_json() -> Value {
    json!([
        {"value": -20, "color": "#ff0000"},
        {"value": 0, "color": "#ffffff"},
        {"value": 5, "color": "#0000ff"}
    ])
}

/// `makeStore()`.
fn make_store() -> Store {
    let mut store = store();
    store.set_model(
        "palette-demo",
        model(json!({
            "id": "palette-demo",
            "compartments": {"c": "Cytosol", "e": "Extracellular"},
            "metabolites": [{"id": "a_e", "compartment": "e"}, {"id": "a_c", "compartment": "c"}],
            "reactions": [
                {"id": "EX_a_e", "metabolites": {"a_e": -1}, "lower_bound": -10, "upper_bound": 1000},
                {"id": "At", "metabolites": {"a_e": -1, "a_c": 1}, "lower_bound": 0, "upper_bound": 1000},
                {"id": "BIOMASS", "metabolites": {"a_c": -1}, "lower_bound": 0, "upper_bound": 1000, "objective_coefficient": 1}
            ]
        })),
    );
    store.scenarios = vec![json!({
        "id": "palette-demo", "name": "Palette demo", "objective": "equal-growth",
        "species": [{"id": "demo", "model": "palette-demo", "abundance": 1}],
        "medium": {"a": 10}
    })];
    store.use_scenario("palette-demo").unwrap();
    assert!(store.solution.as_ref().unwrap().is_optimal());
    store
}

fn color(store: &Store, v: f64) -> String {
    store.scale.as_ref().expect("a flux scale").color(v)
}

#[test]
fn changing_palette_is_presentation_only_and_invalid_edits_preserve_the_previous_palette() {
    let mut store = make_store();
    assert_eq!(store.view.flux_color_stops, None);
    let community = store.community.clone().unwrap();
    let problem = store.problem.clone().unwrap();
    let solution = store.solution.clone().unwrap();
    let map = store.map.as_ref().unwrap().to_json();
    let bounds = problem.snapshot_bounds();
    let fluxes = solution.fluxes.clone();

    store.set_flux_color_stops(Some(stops())).unwrap();
    assert_eq!(store.view.flux_color_stops, Some(stops()));
    assert_eq!(color(&store, 0.0), "#ffffff");
    assert!(Arc::ptr_eq(store.community.as_ref().unwrap(), &community));
    assert!(Arc::ptr_eq(store.problem.as_ref().unwrap(), &problem));
    assert!(Arc::ptr_eq(store.solution.as_ref().unwrap(), &solution));
    assert_eq!(store.map.as_ref().unwrap().to_json(), map);
    assert_eq!(store.problem.as_ref().unwrap().snapshot_bounds(), bounds);
    assert_eq!(store.solution.as_ref().unwrap().fluxes, fluxes);
    assert!(store.set_flux_color_stops(Some(vec![FluxColorStop { value: 1.0, color: "#fff".into() }])).is_err());
    assert_eq!(store.view.flux_color_stops, Some(stops()));
    store.solve();
    assert_eq!(color(&store, 5.0), "#0000ff", "solving must keep the chosen presentation palette");
    store.set_view(view_patch(&json!({"dark": true}))).unwrap();
    assert_eq!(color(&store, 5.0), "#0000ff", "changing theme must keep the chosen palette");
    store.set_flux_color_stops(None).unwrap();
    assert_eq!(store.view.flux_color_stops, None);
    assert_eq!(color(&store, 0.0), "#475569");
}

#[test]
fn scenario_export_and_import_preserve_custom_stops_older_scenarios_restore_automatic_colors() {
    let mut source = make_store();
    source.set_flux_color_stops(Some(stops())).unwrap();
    let mut doc: Value = serde_json::from_str(&source.state_json()).unwrap();
    assert_eq!(doc["fluxColorStops"], stops_json());
    let mut restored = make_store();
    restored.load_state(&doc.to_string()).unwrap();
    assert_eq!(restored.view.flux_color_stops, Some(stops()));
    assert_eq!(color(&restored, -20.0), "#ff0000");
    assert_eq!(color(&restored, 5.0), "#0000ff");
    doc.as_object_mut().unwrap().shift_remove("fluxColorStops");
    restored.load_state(&doc.to_string()).unwrap();
    assert_eq!(restored.view.flux_color_stops, None, "old scenarios must not inherit an unrelated custom palette");
    assert_eq!(color(&restored, 0.0), "#cbd5e1");
}

#[test]
fn map_export_and_import_preserve_custom_stops_older_wrapped_and_raw_maps_restore_automatic_colors() {
    let mut source = make_store();
    source.set_flux_color_stops(Some(stops())).unwrap();
    let mut doc: Value = serde_json::from_str(&source.map_json().unwrap()).unwrap();
    assert_eq!(doc["fluxColorStops"], stops_json());
    let mut restored = make_store();
    let solution = restored.solution.clone().unwrap();
    restored.load_map_json(&doc.to_string()).unwrap();
    assert_eq!(restored.view.flux_color_stops, Some(stops()));
    assert_eq!(color(&restored, 0.0), "#ffffff");
    assert!(Arc::ptr_eq(restored.solution.as_ref().unwrap(), &solution), "loading map presentation must not run FBA");
    doc.as_object_mut().unwrap().shift_remove("fluxColorStops");
    restored.load_map_json(&doc.to_string()).unwrap();
    assert_eq!(restored.view.flux_color_stops, None);
    assert_eq!(color(&restored, 0.0), "#cbd5e1");
    restored.set_flux_color_stops(Some(stops())).unwrap();
    restored.load_map_json(&doc["map"].to_string()).unwrap();
    assert_eq!(restored.view.flux_color_stops, None, "legacy raw EditableMap JSON defaults to automatic colors");
}
