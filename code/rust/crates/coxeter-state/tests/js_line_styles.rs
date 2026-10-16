//! The Store half of `test/line-styles.test.mjs` (7b5aaa0): colour edits are
//! undoable and travel in the Map JSON, the line switches are kept in the
//! Scenario JSON, hops on a real map, and the hover text and metadata of an
//! exported SVG. The rendering half is in `coxeter-render`.

mod common;

use coxeter_map::editable_map::ColorChange;
use coxeter_render::render_map_svg;
use coxeter_state::{Store, ViewPatch};
use serde_json::{Value, json};

use common::{init_store, model, store};

fn mini_model() -> Value {
    json!({
        "id": "mini",
        "compartments": { "e": "extracellular", "c": "cytoplasm" },
        "metabolites": [
            { "id": "glc_e", "name": "glucose", "compartment": "e" },
            { "id": "glc_c", "name": "glucose", "compartment": "c", "formula": "C6H12O6" },
            { "id": "g6p_c", "name": "glucose 6-phosphate", "compartment": "c" },
            { "id": "dead_c", "name": "dead end", "compartment": "c" }
        ],
        "reactions": [
            { "id": "EX_glc_e", "metabolites": { "glc_e": -1 }, "lower_bound": -10, "upper_bound": 1000 },
            { "id": "GLCt", "name": "glucose transport", "metabolites": { "glc_e": -1, "glc_c": 1 }, "lower_bound": 0, "upper_bound": 1000 },
            { "id": "HEX", "name": "hexokinase", "metabolites": { "glc_c": -1, "g6p_c": 1 }, "lower_bound": 0, "upper_bound": 1000, "subsystem": "Glycolysis" },
            { "id": "DEAD", "name": "dead end", "metabolites": { "g6p_c": -1, "dead_c": 1 }, "lower_bound": 0, "upper_bound": 1000, "subsystem": "Glycolysis" },
            { "id": "BIO", "name": "growth", "metabolites": { "g6p_c": -1 }, "lower_bound": 0, "upper_bound": 1000, "objective_coefficient": 1 }
        ]
    })
}

fn mini_store() -> Store {
    let mut store = store();
    store.set_model("mini", model(mini_model()));
    store.scenarios = vec![json!({
        "id": "mini", "name": "Mini", "objective": "equal-growth",
        "species": [{ "id": "sp", "model": "mini", "abundance": 1 }], "medium": { "glc": 10 }
    })];
    store.use_scenario("mini").unwrap();
    assert!(store.solution.as_ref().is_some_and(|s| s.is_optimal()));
    store
}

fn g6p_node(store: &Store) -> String {
    store.map.as_ref().unwrap().nodes().values().find(|n| n.merged_id.as_deref() == Some("sp@g6p_c")).unwrap().id.clone()
}

#[test]
fn a_reaction_without_flux_is_drawn_grey_and_dashed_against_a_solution() {
    let mut store = mini_store();
    let svg = render_map_svg(store.map.as_ref().unwrap().graph(), &store.map_render_state(Default::default()));
    assert!(regex::Regex::new(r#"<path class="emc-seg[^"]* is-off[^"]*"[^>]*data-rxn="sp@DEAD""#).unwrap().is_match(&svg), "dashed by default");
    store.set_view(ViewPatch { dash_no_flux: Some(false), ..Default::default() }).unwrap();
    let svg = render_map_svg(store.map.as_ref().unwrap().graph(), &store.map_render_state(Default::default()));
    assert!(!svg.contains("is-off"), "switched off");
}

#[test]
fn colour_edits_are_undoable_and_travel_in_the_map_json() {
    let mut store = mini_store();
    let node = g6p_node(&store);
    store.set_map_node_style(&ColorChange::Set("#aabbcc".into()), &ColorChange::Keep);
    store.set_map_node_colors(std::slice::from_ref(&node), &ColorChange::Set("#ff0000".into()), &ColorChange::Keep);
    store.set_map_reaction_color(&["sp@HEX".to_string()], &ColorChange::Set("#123456".into()));
    let saved = store.map_json().unwrap();

    let map = |s: &Store| s.map.as_ref().unwrap().graph().clone();
    store.undo_map();
    assert_eq!(map(&store).reactions["sp@HEX"].own_color(), None);
    store.undo_map();
    assert_eq!(map(&store).nodes[&node].own_fill(), None);
    store.undo_map();
    assert_eq!(map(&store).node_style(), Default::default());

    store.load_map_json(&saved).unwrap();
    assert_eq!(map(&store).node_style().fill.as_deref(), Some("#aabbcc"));
    assert_eq!(map(&store).nodes[&node].own_fill(), Some("#ff0000"));
    assert_eq!(map(&store).reactions["sp@HEX"].own_color(), Some("#123456"));

    store.set_map_node_colors(std::slice::from_ref(&node), &ColorChange::Reset, &ColorChange::Reset);
    store.set_map_reaction_color(&["sp@HEX".to_string()], &ColorChange::Reset);
    assert_eq!(map(&store).nodes[&node].own_fill(), None, "reset gives it back to the map");
    assert_eq!(map(&store).reactions["sp@HEX"].own_color(), None);
}

#[test]
fn the_line_switches_are_kept_in_the_scenario_json() {
    let mut store = mini_store();
    let v = &store.view;
    assert_eq!((v.width_by_flux, v.dash_no_flux, v.line_hops), (false, true, true));
    store.set_view(ViewPatch { width_by_flux: Some(true), dash_no_flux: Some(false), line_hops: Some(false), ..Default::default() }).unwrap();
    let doc: Value = serde_json::from_str(&store.state_json()).unwrap();
    assert_eq!((doc["widthByFlux"].clone(), doc["dashNoFlux"].clone(), doc["lineHops"].clone()), (json!(true), json!(false), json!(false)));

    store.set_view(ViewPatch { width_by_flux: Some(false), dash_no_flux: Some(true), line_hops: Some(true), ..Default::default() }).unwrap();
    store.load_state(&doc.to_string()).unwrap();
    let v = &store.view;
    assert_eq!((v.width_by_flux, v.dash_no_flux, v.line_hops), (true, false, false));
}

#[test]
fn on_a_real_map_hops_are_drawn_without_adding_or_losing_a_line() {
    let mut store = init_store();
    store.use_scenario("solo-levadura").unwrap();
    let count = |svg: &str| svg.matches("<path class=\"emc-seg").count();
    let mut state = store.map_render_state(Default::default());
    state.line_hops = false;
    let plain = render_map_svg(store.map.as_ref().unwrap().graph(), &state);
    state.line_hops = true;
    let hopped = render_map_svg(store.map.as_ref().unwrap().graph(), &state);
    assert_eq!(count(&hopped), count(&plain));
    let arcs = regex::Regex::new(r" A [\d.]+ [\d.]+ 0 0 [01] ").unwrap();
    assert!(arcs.is_match(&hopped), "the yeast core map has crossings");
    assert!(!arcs.is_match(&plain));
}

#[test]
fn an_exported_map_says_what_each_reaction_and_metabolite_is_and_what_drew_it() {
    let store = mini_store();
    let tips = store.export_tooltips();
    let reaction = &tips.reactions["sp@HEX"];
    assert!(reaction.starts_with("HEX\nhexokinase\nSubsystem: Glycolysis\nBounds: [0, 1000]\nFlux: 10"), "{reaction}");
    let node = store.map.as_ref().unwrap().nodes().values().find(|n| n.merged_id.as_deref() == Some("sp@glc_c")).unwrap().id.clone();
    assert_eq!(tips.nodes[&node], "glc\nglucose\nID: glc_c\nFormula: C6H12O6\nCompartment: cytoplasm");

    let mut state = store.map_render_state(Default::default());
    state.for_export = true;
    state.tooltips = Some(std::sync::Arc::new(tips));
    let svg = render_map_svg(store.map.as_ref().unwrap().graph(), &state);
    assert!(svg.contains("<title>HEX\nhexokinase"));

    let about = store.export_metadata();
    assert_eq!(about["generator"], "Coxeter Multicell Flux Visualizer");
    assert_eq!(about["scenario"]["id"], "mini");
    assert_eq!(about["species"].as_array().unwrap().iter().map(|s| s["model"].clone()).collect::<Vec<_>>(), [json!("mini")]);
    assert_eq!(about["analysis"]["status"], "optimal");
    assert_eq!(about["map"]["labels"], json!({ "metabolite": "base", "reaction": "id" }));
    assert_eq!(about["map"]["lineHops"], true);
}
