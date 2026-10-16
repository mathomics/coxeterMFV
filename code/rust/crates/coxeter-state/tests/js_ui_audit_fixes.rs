//! Port of `test/ui-audit-fixes.test.mjs` (the Store parts).
//!
//! Not ported here:
//! - "PNG scales fit each browser canvas ceiling, largest first":
//!   `rasterScales` only, no Store; ported in
//!   `coxeter-render/src/raster_size.rs`.
//! - "member, community-data and dated exports keep their context in the
//!   name", and the `exportFileName`/`fileStem` assertions of "exported files
//!   are named after the model with a short suffix": `src/ui/file-names.js`
//!   only, no Store; ported in `coxeter-io/src/file_names.rs`. The Store half
//!   of the second test (`exportBaseName`, `fileNameFor`) is here.
//! - "the model badge is read from the loaded model", "the model badge reads
//!   the compartments the community already counted" (`modelKindLabel`), "a
//!   shared-medium metabolite belongs to the shared medium, not to a cell
//!   called M", "colour pickers show the colour drawn and offer a reset only
//!   once customised", "a cofactor helper node explains its role instead of
//!   offering a dead Primary button" (`renderInspector`) and "label options,
//!   Add Metabolite help and blocked layout sliders say what they do"
//!   (`renderViewControls`, `renderTaskPanel`, `renderLayoutSettings`): HTML
//!   of `src/ui/panels.js` only; the GUI is another crate.
//!
//! HTML assertions dropped from tests ported here: the `renderInspector`
//! assertions of "switching a species off keeps a hand-built map, its undo
//! history and every bound edit" and "a zero-abundance joint member is held
//! out like a switched-off one", and the `renderLogEntries` assertions of
//! "the log keeps every message, counts repeats and folds back-to-back
//! solves".
//!
//! Adapted:
//! - SVG assertions draw what the store hands the renderer
//!   (`render_map_svg(map.graph(), &store.map_render_state(..))`) where the
//!   JS calls `renderMap(store.layout, state)` with a hand-built state: the
//!   label mode and the Cofactors toggles go through `set_view`, selections
//!   through the store's fields. Where the JS state holds something the store
//!   does not (`solvedFluxes: new Map()`, the picked cross-feeding event,
//!   `forExport`), that field of the render state is set by hand, as the JS
//!   does.
//! - The JS counts solves by wrapping `store.solve`; here they are the
//!   `solution` events the store emits (`take_events`).
//! - JS stubs (`store.community = { activeSpecies: [...] }`, `store.map = {
//!   reactions: { size } }`, `big.reactions = { size: 5000 }`,
//!   `store._jointProjectionSpec = {}`) become real objects of those sizes:
//!   a community over a generated model, maps with that many reactions, a
//!   community whose members run on a larger model, an empty SteadyCom spec.
//! - `assert.equal(store.map, map)` (the last species cannot be switched off)
//!   compares the map's JSON: the store owns its map.
//! - "every bundled scenario and published community opens as Saved": the JS
//!   stubs `fetch` with the repository's files (here `DirAssets` over the
//!   repository) and `store.solve` with a no-op, because the genome-scale LPs
//!   would only add minutes; the solve cannot be stubbed here, so the opens
//!   solve for real (HiGHS: a few seconds for all of them).
//! - "map history versions follow edits, undo and redo back to the saved map"
//!   tests `MapHistory` alone; it is not ported in `coxeter-map`, so it is
//!   here, on the history the store keeps.
//! - "an exported map does not keep the cross-feeding event picked in the
//!   explorer" FAILS IN JS (`node --test`): its pattern
//!   `class="emc-crossfeed(?:-route)?"` stops at the closing quote, so it
//!   misses the picked line (`class="emc-crossfeed-route is-picked"`) and
//!   reads the opacity of another line. The pattern here allows the extra
//!   classes, as the port in `coxeter-render/tests/js_tests.rs` does; the
//!   test file in JS should be fixed the same way.

mod common;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use coxeter_core::community::{CommunityModel, CommunitySpec, CrossFeeding, CrossFeedingLink, SpeciesSpec};
use coxeter_core::model::Model;
use coxeter_core::testkit::repo_root;
use coxeter_io::file_names::export_suffix;
use coxeter_map::editable_map::{ColorChange, EditableMap};
use coxeter_map::history::{DEFAULT_LABEL, MapHistory};
use coxeter_map::multi_selection::SelectionRect;
use coxeter_map::reconcile_map::prune_map_to_model;
use coxeter_render::mapview::render_map_svg;
use coxeter_render::state::{CrossFeedingEvent, CrossFeedingExplorerView, GeometrySelection, RenderState};
use coxeter_solver::steadycom::SteadyComSpec;
use coxeter_state::map_edit::GeometryColors;
use coxeter_state::status_line::{NOTICE_MS, status_line};
use coxeter_state::{LogKind, RenderOptions, Status, StatusState, Store, UnsavedWork, UserNumber};
use indexmap::IndexMap;
use regex::Regex;
use serde_json::{Value, json};

use common::{WAIT, model, store, view_patch};

// Helpers

/// The three-reaction model of `makeStore()`, under `id`.
fn audit_model(id: &str) -> Model {
    model(json!({
        "id": id,
        "compartments": {"c": "Cytosol", "e": "Extracellular"},
        "metabolites": [{"id": "a_e", "compartment": "e"}, {"id": "a_c", "compartment": "c"}],
        "reactions": [
            {"id": "EX_a_e", "metabolites": {"a_e": -1}, "lower_bound": -10, "upper_bound": 1000},
            {"id": "At", "metabolites": {"a_e": -1, "a_c": 1}, "lower_bound": 0, "upper_bound": 1000},
            {"id": "BIOMASS", "metabolites": {"a_c": -1}, "lower_bound": 0, "upper_bound": 1000, "objective_coefficient": 1}
        ]
    }))
}

/// `makeStore()`.
fn make_store() -> Store {
    let mut store = store();
    store.set_model("audit-demo", audit_model("audit-demo"));
    store.scenarios = vec![json!({
        "id": "audit-demo", "name": "Audit demo", "objective": "equal-growth",
        "species": [{"id": "demo", "model": "audit-demo", "abundance": 1}],
        "medium": {"a": 10}
    })];
    store.use_scenario("audit-demo").unwrap();
    assert!(store.solution.as_ref().unwrap().is_optimal());
    store
}

/// `makePairStore()`.
fn make_pair_store() -> Store {
    let mut store = make_store();
    store.scenarios = vec![json!({
        "id": "audit-pair", "name": "Audit pair", "objective": "equal-growth",
        "species": [
            {"id": "demo", "model": "audit-demo", "abundance": 0.5},
            {"id": "other", "model": "audit-demo", "abundance": 0.5}
        ],
        "medium": {"a": 10}
    })];
    store.use_scenario("audit-pair").unwrap();
    assert_eq!(store.map.as_ref().unwrap().cells().len(), 2);
    store
}

/// `makeTwoModelStore()`.
fn make_two_model_store() -> Store {
    let mut store = make_store();
    store.set_model("audit-other", audit_model("audit-other"));
    store.scenarios.push(json!({
        "id": "audit-two", "name": "Two models", "objective": "equal-growth",
        "species": [{"id": "demo", "model": "audit-demo", "abundance": 0.5}, {"id": "oth", "model": "audit-other", "abundance": 0.5}],
        "medium": {"a": 10}
    }));
    store.use_scenario("audit-two").unwrap();
    store
}

fn unsaved(map: bool, scenario: bool, model: bool) -> UnsavedWork {
    UnsavedWork { map, scenario, model }
}

/// `tx = beginMapEdit(label); map.setNodePrimary(node, false); commitMapEdit(tx)`.
fn hand_edit(store: &mut Store, label: &str, node: &str) {
    let tx = store.begin_map_edit(label);
    store.map.as_mut().unwrap().set_node_primary(node, false).expect("the node is drawn");
    store.commit_map_edit(tx);
}

/// The map as the store hands it to the renderer.
fn render_state(store: &Store) -> RenderState {
    store.map_render_state(RenderOptions::default())
}

fn draw(store: &Store, state: &RenderState) -> String {
    render_map_svg(store.map.as_ref().expect("a map").graph(), state)
}

fn draw_store(store: &Store) -> String {
    draw(store, &render_state(store))
}

fn set_view(store: &mut Store, patch: Value) {
    store.set_view(view_patch(&patch)).unwrap();
}

/// How many times the store solved since the last call: the `solution`
/// events it emitted.
fn solves(store: &mut Store) -> usize {
    store.take_events().iter().filter(|r| *r == "solution").count()
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).unwrap()
}

// Cleared numeric fields

#[test]
fn a_cleared_or_half_typed_bound_is_not_a_bound_of_0() {
    let mut store = make_store();
    let values = [
        ("", UserNumber::Text("".into())),
        ("   ", UserNumber::Text("   ".into())),
        ("-", UserNumber::Text("-".into())),
        ("abc", UserNumber::Text("abc".into())),
        ("NaN", UserNumber::Number(f64::NAN)),
        ("Infinity", UserNumber::Number(f64::INFINITY)),
    ];
    for (label, value) in values {
        let result = store.set_reaction_bounds("demo@At", UserNumber::Empty, value).unwrap();
        assert_eq!(result.upper, 1000.0, "upper bound changed by {label:?}");
        assert_eq!(store.current_reaction_bounds("demo@At").upper, 1000.0);
    }
    assert!(!store.overrides.contains_key("demo@At"), "an ignored value must not create an override");

    store.set_reaction_bounds("demo@At", UserNumber::Empty, "5").unwrap();
    assert_eq!(store.current_reaction_bounds("demo@At").upper, 5.0, "a real number still applies");
    store.set_reaction_bounds("demo@At", "", UserNumber::Empty).unwrap();
    assert_eq!(store.current_reaction_bounds("demo@At").lower, 0.0, "clearing the other side keeps it");
    store.set_reaction_bounds("demo@At", UserNumber::Empty, "0").unwrap();
    assert_eq!(store.current_reaction_bounds("demo@At").upper, 0.0, "an explicit 0 is still a bound of 0");
}

#[test]
fn a_cleared_medium_limit_or_abundance_changes_nothing_and_does_not_solve() {
    let mut store = make_store();
    store.take_events();
    let mut count = 0;

    // `setMediumLimit` returns null for a field that holds no number: `false`.
    assert!(!store.set_medium_limit("a", "").unwrap());
    assert!(!store.set_medium_limit("a", "-").unwrap());
    assert_eq!(store.medium["a"], 10.0);
    count += solves(&mut store);
    assert_eq!(count, 0);
    store.set_medium_limit("a", "4").unwrap();
    assert_eq!(store.medium["a"], 4.0);
    count += solves(&mut store);
    assert_eq!(count, 1);

    store.set_abundance("demo", "").unwrap();
    assert_eq!(store.species[0].abundance, 1.0);
}

// Full-model warning

/// A model of `reactions` reactions over `metabolites` cytosolic
/// metabolites: genome-scale in size, nothing else.
fn sized_model(id: &str, reactions: usize, metabolites: usize) -> Model {
    let mets: Vec<Value> = (0..metabolites).map(|i| json!({"id": format!("m{i}_c"), "compartment": "c"})).collect();
    let rxns: Vec<Value> = (0..reactions)
        .map(|i| {
            let mut stoich = serde_json::Map::new();
            stoich.insert(format!("m{}_c", i % metabolites), json!(-1));
            stoich.insert(format!("m{}_c", (i + 1) % metabolites), json!(1));
            json!({"id": format!("R{i}"), "metabolites": stoich, "lower_bound": 0, "upper_bound": 1000})
        })
        .collect();
    model(json!({"id": id, "compartments": {"c": "Cytosol"}, "metabolites": mets, "reactions": rxns}))
}

/// A map drawing `n` reactions and nothing else (`{ reactions: { size: n } }`).
fn map_with_reactions(n: usize) -> EditableMap {
    let reactions: Vec<Value> = (0..n)
        .map(|i| json!({"id": format!("demo@R{i}"), "localId": format!("R{i}"), "x": 10 + i, "y": 10, "onMembrane": false, "compartments": []}))
        .collect();
    let map = EditableMap::from_json(&json!({
        "canvas": {"width": 400, "height": 240},
        "environment": {"id": "u", "label": "Medium", "rect": {"x": 0, "y": 0, "w": 400, "h": 240}},
        "cells": [],
        "medium": {"nodes": [], "exchanges": []},
        "nodes": [],
        "reactions": reactions,
        "segments": []
    }))
    .expect("a valid map");
    assert_eq!(map.reactions().len(), n);
    map
}

#[test]
fn drawing_the_full_model_asks_first_only_when_the_model_is_too_large_to_lay_out() {
    let mut store = make_store();
    assert_eq!(store.full_map_warning(), None, "a core model draws instantly");

    let big = sized_model("big", 4106, 2749);
    let community = CommunityModel::new(CommunitySpec {
        name: Some("Big".into()),
        species: vec![SpeciesSpec::new("demo", Arc::new(big))],
        medium: IndexMap::new(),
        objective: None,
        product_target: None,
    })
    .unwrap();
    let total = community.merged.reactions.len();
    assert_eq!(total, 4106);
    store.community = Some(Arc::new(community));
    store.map = Some(map_with_reactions(93));
    let warning = store.full_map_warning().expect("a warning");
    assert!(warning.contains("all 4106 reactions"), "{warning}");
    assert!(warning.contains("replaces the current drawing of 93 reactions"), "{warning}");
    assert!(store.relayout_blocked().is_some_and(|m| m.contains("hand-built drawing")), "the preset guard is unchanged");

    store.map = Some(map_with_reactions(0));
    assert!(!store.full_map_warning().expect("a warning").contains("replaces"), "nothing to replace on an empty map");
}

// Saved / unsaved

#[test]
fn map_history_versions_follow_edits_undo_and_redo_back_to_the_saved_map() {
    let mut history = MapHistory::default();
    assert!(!history.dirty());
    history.record(&json!({"a": 1}), &json!({"a": 2}), DEFAULT_LABEL);
    assert!(history.dirty());
    history.mark_saved();
    assert!(!history.dirty());
    history.undo(&json!({"a": 2}));
    assert!(history.dirty(), "undoing past the save is unsaved");
    history.redo(&json!({"a": 1}));
    assert!(!history.dirty(), "redoing back to the saved map is saved again");
    history.record(&json!({"a": 2}), &json!({"a": 2}), DEFAULT_LABEL);
    assert!(!history.dirty(), "a no-op edit changes nothing");
    history.record(&json!({"a": 2}), &json!({"a": 3}), DEFAULT_LABEL);
    history.clear();
    assert!(!history.dirty(), "a new map has nothing to save");
}

#[test]
fn saved_reflects_map_scenario_and_model_files_not_undo_history_or_uploads() {
    let mut store = make_store();
    assert_eq!(store.unsaved_work(), unsaved(false, false, false), "a fresh scenario is saved");

    store.set_medium_limit("a", "4").unwrap();
    assert!(store.unsaved_work().scenario, "a medium change is unsaved");
    let scenario_file = store.state_json();
    store.mark_scenario_saved();
    assert!(!store.unsaved_work().scenario, "Scenario export saves it");

    store.set_reaction_bounds("demo@At", UserNumber::Empty, "7").unwrap();
    assert!(store.unsaved_work().scenario, "a bound change is unsaved");
    store.load_state(&scenario_file).unwrap();
    store.wait_for_jobs(WAIT);
    assert!(!store.unsaved_work().scenario, "opening a scenario file is saved");

    hand_edit(&mut store, "Make secondary", "met:demo@a_c");
    assert!(store.unsaved_work().map, "a map edit is unsaved");
    let map_file = store.map_json().unwrap();
    store.mark_map_saved();
    assert!(!store.unsaved_work().map, "Map JSON export saves it");
    store.undo_map();
    assert!(store.unsaved_work().map);
    store.load_map_json(&map_file).unwrap();
    assert!(!store.unsaved_work().map, "opening a map file is saved");

    store.model_editor_species_id = Some("demo".into());
    store.create_model_metabolite(&json!({"id": "b_c", "name": "B", "compartment": "c"})).unwrap();
    assert!(store.unsaved_work().model, "a model edit is unsaved");
    store.mark_model_saved(None);
    assert!(!store.unsaved_work().model, "Model JSON export saves it");

    store.uploaded_model_keys.insert("audit-demo".into());
    assert_eq!(store.unsaved_work(), unsaved(false, false, false), "an uploaded model alone is not unsaved work");
}

// Rendering

#[test]
fn labels_none_hides_metabolite_labels_as_well_as_reaction_labels() {
    let mut store = make_store();
    set_view(&mut store, json!({"labels": "auto"}));
    let auto = draw_store(&store);
    assert!(auto.contains("emc-label-met"));
    assert!(auto.contains("emc-label-medium"));
    set_view(&mut store, json!({"labels": "none"}));
    let none = draw_store(&store);
    assert!(!re("emc-label-met|emc-label-medium|emc-label-currency|emc-label-rxn").is_match(&none));
}

#[test]
fn an_exported_map_carries_no_selection_handles_pointer_targets_or_resize_grips() {
    let mut store = make_store();
    store.selected_reaction = Some("demo@At".into());
    store.selected_geometry = Some(GeometrySelection { cell_id: Some("demo".into()), compartment_id: "c".into() });
    store.toggle_multi_selection("node", "met:demo@a_c");
    store.toggle_multi_selection("node", "met:demo@a_e");
    let state = store.map_render_state(RenderOptions { reaction_edit: true, min_hit_radius: 12.0 });
    let editor = draw(&store, &state);
    let markers = ["emc-hit", "data-canvas-resize", "emc-multi-area", "emc-multi-drag", "is-selected", "is-multi-selected", "emc-geometry-selection"];
    for marker in markers {
        assert!(editor.contains(marker), "the editor view should contain {marker}");
    }
    let exported = draw(&store, &RenderState { for_export: true, ..state });
    for marker in markers.iter().chain(&["emc-layer-edit-handles"]) {
        assert!(!exported.contains(marker), "the export must not contain {marker}");
    }
    assert!(exported.contains("emc-node"), "the map itself is still drawn");
    assert!(exported.contains("emc-canvas-frame"), "and so is its white canvas");
}

// Status

#[test]
fn an_ok_message_is_shown_for_a_while_before_the_summary_returns() {
    // `{ status, community: null }`: a store with nothing loaded.
    let mut store = store();
    *store.status_mut() = Status { state: StatusState::Ok, message: "The drawing was kept.".into(), solve_ms: 3.0, layout_ms: 0.0 };
    let fresh = status_line(&store, 0.0);
    assert_eq!(fresh.text, "The drawing was kept.");
    assert_eq!(fresh.expires_in, NOTICE_MS);
    let stale = status_line(&store, NOTICE_MS + 1.0);
    assert!(re("species · 0 reactions · objective — · LP 3 ms").is_match(&stale.text), "{}", stale.text);
    assert_eq!(stale.expires_in, 0.0);

    let mut with = |state: StatusState, message: &str, age: f64| {
        *store.status_mut() = Status { state, message: message.into(), ..Status::default() };
        status_line(&store, age)
    };
    assert_eq!(with(StatusState::Ok, "", 0.0).state, "summary");
    assert_eq!(with(StatusState::Warn, "w", 1e9).text, "w");
    assert_eq!(with(StatusState::Error, "e", 1e9).text, "e");
    assert_eq!(with(StatusState::Busy, "", 1e9).text, "Working…");
}

// Map colour pickers

fn colors(fill: ColorChange, stroke: ColorChange) -> GeometryColors {
    GeometryColors { fill, stroke }
}

fn set(color: &str) -> ColorChange {
    ColorChange::Set(color.into())
}

/// The fill and stroke of a cell's compartment, as the map holds them.
fn compartment_colors(store: &Store, cell: &str, compartment: &str) -> (Option<String>, Option<String>) {
    let cell = store.map.as_ref().unwrap().cell(cell).expect("the cell is drawn");
    let comp = cell.compartments.iter().find(|c| c.id == compartment).expect("the compartment is drawn");
    (comp.fill.clone(), comp.stroke.clone())
}

fn cell_stroke(store: &Store, cell: &str) -> Option<String> {
    store.map.as_ref().unwrap().cell(cell).expect("the cell is drawn").stroke.clone()
}

#[test]
fn cell_and_compartment_colours_are_drawn_undoable_and_saved_with_the_map() {
    let mut store = make_store();
    let (body, inner) = {
        let cell = store.map.as_ref().unwrap().cell("demo").expect("the demo cell");
        let body = cell.outer_compartment.clone().expect("an outer compartment");
        let inner = cell.compartments.iter().find(|c| c.id != body).expect("an inner compartment").id.clone();
        (body, inner)
    };

    store.set_map_geometry_colors("demo", None, &colors(set("#12AB34"), ColorChange::Keep));
    assert_eq!(compartment_colors(&store, "demo", &body).0.as_deref(), Some("#12ab34"), "the cell fill is the fill of its body");
    store.set_map_geometry_colors("demo", None, &colors(ColorChange::Keep, set("#aa0000")));
    assert_eq!(cell_stroke(&store, "demo").as_deref(), Some("#aa0000"));
    store.set_map_geometry_colors("demo", Some(&inner), &colors(set("#0000ff"), set("#00ff00")));
    let svg = draw_store(&store);
    assert!(re(&format!(r##"data-compartment-space="{body}"[^>]*fill="#12ab34""##)).is_match(&svg));
    assert!(re(r##"class="emc-cell-boundary"[^>]*stroke="#aa0000""##).is_match(&svg));
    assert!(re(&format!(r##"data-compartment-space="{inner}"[^>]*fill="#0000ff""##)).is_match(&svg));
    assert!(svg.contains(r##"stroke="#00ff00""##));

    store.set_map_geometry_colors("demo", Some(&inner), &colors(set("red"), ColorChange::Keep));
    assert_eq!(compartment_colors(&store, "demo", &inner).0.as_deref(), Some("#0000ff"), "only #rrggbb colours are taken");

    let saved = store.map_json().unwrap();
    store.undo_map();
    assert_eq!(compartment_colors(&store, "demo", &inner).0, None, "undo takes the last colour back");
    assert_eq!(cell_stroke(&store, "demo").as_deref(), Some("#aa0000"), "and only the last one");
    store.load_map_json(&saved).unwrap();
    assert_eq!(cell_stroke(&store, "demo").as_deref(), Some("#aa0000"), "colours survive Map JSON");
    assert_eq!(compartment_colors(&store, "demo", &inner).0.as_deref(), Some("#0000ff"));

    store.set_map_geometry_colors("demo", None, &colors(ColorChange::Reset, ColorChange::Reset));
    store.set_map_geometry_colors("demo", Some(&inner), &colors(ColorChange::Reset, ColorChange::Reset));
    let svg = draw_store(&store);
    assert!(!re("#12ab34|#aa0000|#0000ff|#00ff00").is_match(&svg), "reset goes back to the theme");
}

// Cofactors and role buttons

#[test]
fn the_cofactors_toggles_hide_every_secondary_node_its_lines_and_its_label() {
    let mut store = make_store();
    store.map.as_mut().unwrap().set_node_primary("met:demo@a_c", false).unwrap();
    let mut draw_with = |view: Value| {
        set_view(&mut store, view);
        draw_store(&store)
    };

    let shown = draw_with(json!({"showCurrency": true, "currencyLabels": true}));
    assert!(re(r#"class="emc-node[^"]*"[^>]*data-node="met:demo@a_c""#).is_match(&shown));
    assert!(shown.contains(r#"data-metabolite-label="met:demo@a_c""#));

    let hidden = draw_with(json!({"showCurrency": false, "currencyLabels": true}));
    assert!(!hidden.contains(r#"data-node="met:demo@a_c""#), "node, hit target and lines go");
    assert!(!hidden.contains(r#"data-metabolite-label="met:demo@a_c""#));
    assert!(hidden.contains(r#"data-node="met:demo@a_e""#), "primary metabolites stay");

    let unnamed = draw_with(json!({"showCurrency": true, "currencyLabels": false}));
    assert!(re(r#"class="emc-node[^"]*"[^>]*data-node="met:demo@a_c""#).is_match(&unnamed));
    assert!(!unnamed.contains(r#"data-metabolite-label="met:demo@a_c""#), "Cofactor labels names secondary nodes too");
    assert!(unnamed.contains(r#"data-metabolite-label="met:demo@a_e""#));
}

// Labels and PNG size

#[test]
fn all_active_names_only_metabolites_drawn_on_a_reaction_that_carries_flux() {
    let mut store = make_store();
    let solved: HashMap<String, f64> = store.solution.as_ref().unwrap().fluxes.clone().into_iter().collect();
    let draw_with = |store: &mut Store, labels: &str, solved_fluxes: HashMap<String, f64>| {
        set_view(store, json!({"labels": labels}));
        let state = RenderState { solved_fluxes: Some(solved_fluxes), ..render_state(store) };
        draw(store, &state)
    };
    let met_labels = |svg: &str| svg.matches("data-metabolite-label=\"").count();

    let active = draw_with(&mut store, "active", solved.clone());
    assert!(active.contains(r#"data-metabolite-label="met:demo@a_c""#), "a_c is on the active transporter");
    let idle = draw_with(&mut store, "active", HashMap::new());
    assert_eq!(met_labels(&idle), 0, "nothing carries flux, so nothing is named");
    assert_eq!(
        met_labels(&draw_with(&mut store, "all", HashMap::new())),
        met_labels(&draw_with(&mut store, "all", solved)),
        "All ignores activity"
    );

    // FVA mode, as the store hands it to the renderer.
    store.fva_enabled = true;
    let fva = draw_with(&mut store, "active", HashMap::new());
    assert!(met_labels(&fva) > 0, "FVA mode keeps every name, as it does for reactions");
}

// Species toggle and the log

#[test]
fn switching_a_species_off_keeps_a_hand_built_map_its_undo_history_and_every_bound_edit() {
    let mut store = make_pair_store();
    hand_edit(&mut store, "hand edit", "met:demo@a_c");
    store.set_reaction_bounds("demo@At", UserNumber::Empty, "7").unwrap();
    store.set_reaction_bounds("other@At", UserNumber::Empty, "3").unwrap();

    assert!(store.keeps_drawing_on_species_toggle());
    store.set_species_enabled("other", false).unwrap();
    let map = store.map.as_ref().unwrap();
    assert_eq!(map.cells().len(), 2, "the switched-off cell stays drawn");
    assert!(map.has_reaction("other@At"), "and so do its reactions");
    assert_eq!(map.node("met:demo@a_c").unwrap().primary, Some(false), "the hand edit survives");
    assert!(store.can_undo_map(), "so does the undo history");
    assert_eq!(store.disabled_drawn_species_ids(), ["other"]);
    assert!(!store.community.as_ref().unwrap().merged.reactions.contains_key("other@At"), "it is not simulated");
    assert_eq!(store.current_reaction_bounds("demo@At").upper, 7.0, "edits on the species that stayed are kept");

    // `inactiveSpeciesIds` and `inactiveSpeciesLabel`, as the store hands them on.
    let svg = draw_store(&store);
    assert!(svg.contains("· switched off"));

    store.set_species_enabled("other", true).unwrap();
    assert_eq!(store.map.as_ref().unwrap().cells().len(), 2);
    assert!(store.disabled_drawn_species_ids().is_empty());
    let (_, upper) = store.problem.as_ref().unwrap().get_bounds("other@At").expect("other@At is simulated again");
    assert_eq!(upper, 3.0, "its own bound edit applies again");
}

#[test]
fn an_untouched_automatic_map_is_laid_out_again_as_before() {
    let mut store = make_pair_store();
    assert!(!store.keeps_drawing_on_species_toggle());
    store.set_species_enabled("other", false).unwrap();
    let cells = store.map.as_ref().unwrap().cells();
    assert_eq!(cells.len(), 1);
    assert_eq!(cells[0].id, "demo");
}

#[test]
fn the_last_species_cannot_be_switched_off_which_used_to_delete_the_map() {
    let mut store = make_store();
    let map = store.map.as_ref().unwrap().to_json();
    store.set_species_enabled("demo", false).unwrap();
    assert!(store.species[0].enabled);
    assert_eq!(store.map.as_ref().map(EditableMap::to_json), Some(map));
    assert_eq!(store.status().state, StatusState::Warn);
}

#[test]
fn a_map_saved_with_a_switched_off_species_opens_again_with_that_species_still_drawn() {
    let mut store = make_pair_store();
    hand_edit(&mut store, "hand edit", "met:other@a_c");
    store.set_species_enabled("other", false).unwrap();
    let file = store.map_json().unwrap();
    store.show_empty_map();
    assert_eq!(store.map.as_ref().unwrap().cells().len(), 0);

    store.load_map_json(&file).unwrap();
    let map = store.map.as_ref().unwrap();
    assert_eq!(map.cells().len(), 2);
    assert!(map.has_reaction("other@At"), "the switched-off species is not pruned as missing");
    assert_eq!(map.node("met:other@a_c").and_then(|n| n.primary), Some(false));
}

#[test]
fn the_log_keeps_every_message_counts_repeats_and_folds_back_to_back_solves() {
    let mut store = make_store();
    store.log.clear();
    store.set_status_message(StatusState::Warn, "Layout kept.");
    store.set_status_message(StatusState::Warn, "Layout kept.");
    store.set_status_message(StatusState::Busy, "Working on it");
    store.set_status_message(StatusState::Error, "It broke.");
    store.solve();
    store.solve();
    let levels: Vec<(StatusState, u32)> = store.log.iter().map(|e| (e.level, e.count)).collect();
    assert_eq!(levels, [(StatusState::Warn, 2), (StatusState::Error, 1), (StatusState::Ok, 2)]);
    let last = &store.log.last().unwrap().message;
    assert!(re(r"^FBA solved: optimal, objective [\d.]+, \d+ ms$").is_match(last), "{last}");

    for i in 0..260 {
        store.add_log(StatusState::Ok, &format!("message {i}"), LogKind::Message);
    }
    assert_eq!(store.log.len(), 200, "the log is bounded");
    assert_eq!(store.log.last().unwrap().message, "message 259");

    store.clear_log();
    assert_eq!(store.log.len(), 0);
}

// Saved state and species toggles

#[test]
fn a_rectangle_does_not_catch_secondary_nodes_the_cofactors_toggle_has_hidden() {
    let mut store = make_store();
    store.map.as_mut().unwrap().set_node_primary("met:demo@a_c", false).unwrap();
    let everything = SelectionRect::from_xywh(-1e6, -1e6, 2e6, 2e6);
    set_view(&mut store, json!({"showCurrency": true}));
    assert!(store.select_in_rect(everything, true).nodes.contains("met:demo@a_c"));
    set_view(&mut store, json!({"showCurrency": false}));
    let hidden = store.select_in_rect(everything, true).clone();
    assert!(!hidden.nodes.contains("met:demo@a_c"), "Delete must not remove what nobody could see");
    assert!(hidden.nodes.contains("met:demo@a_e"), "primary metabolites are still caught");
}

#[test]
fn saved_also_follows_pathways_flux_colours_layout_colour_mode_and_ribbons() {
    let mut store = make_store();
    assert_eq!(store.unsaved_work(), unsaved(false, false, false));

    set_view(&mut store, json!({"fluxColorStops": [{"value": 0, "color": "#000000"}, {"value": 5, "color": "#ff0000"}]}));
    assert!(store.unsaved_work().map, "flux colours are in the map file");
    store.mark_map_saved();
    assert!(!store.unsaved_work().map);

    set_view(&mut store, json!({"reactionColorMode": "subsystem"}));
    assert!(store.unsaved_work().scenario, "the colour mode is in the scenario file");
    set_view(&mut store, json!({"pathwayRibbonOpacity": 0.3}));
    store.mark_scenario_saved();
    assert_eq!(store.unsaved_work(), unsaved(false, false, false));

    set_view(&mut store, json!({"fluxColorStops": [{"value": 0, "color": "#000000"}, {"value": 9, "color": "#00ff00"}]}));
    store.mark_scenario_saved();
    assert!(!store.unsaved_work().map, "the scenario file carries the flux colours too");
}

#[test]
fn asking_whether_the_work_is_saved_changes_nothing() {
    let mut store = make_store();
    let before = store.saved_work.clone();
    store.medium.insert("a".into(), 3.0); // changed without any announcement
    assert!(store.unsaved_work().scenario);
    store.unsaved_work();
    assert_eq!(store.saved_work, before, "the baseline is not moved by a query");
}

#[test]
fn switching_a_genome_scale_species_back_on_keeps_an_untouched_map_too() {
    let mut store = make_pair_store();
    // `big.reactions = { size: 5000 }`: the one model both species run on,
    // in the catalogue and in the community, is now genome-scale.
    let big = Arc::new(sized_model("audit-demo", 5000, 10));
    store.models.insert_arc("audit-demo", big.clone());
    let mut community = (**store.community.as_ref().unwrap()).clone();
    for sp in &mut community.species {
        sp.model = big.clone();
    }
    store.community = Some(Arc::new(community));

    store.species.iter_mut().find(|sp| sp.id == "other").unwrap().enabled = false;
    assert!(store.too_large_to_auto_draw());
    assert!(store.any_species_too_large_to_auto_draw());
    assert!(store.keeps_drawing_on_species_toggle(), "no full automatic layout on the way back");
}

#[test]
fn reopening_keeps_the_shared_medium_parts_only_a_switched_off_species_used() {
    let store = make_pair_store();
    let mut map = EditableMap::from_json(&store.map.as_ref().unwrap().to_json()).unwrap();
    assert!(map.has_reaction("EX_a_s") && map.has_node("met:M@a"));
    let keep_out = |id: &str| id.starts_with("other@") || id == "EX_a_s" || id == "demo@EX_a_e" || id == "M@a";
    let mut community = (**store.community.as_ref().unwrap()).clone();
    community.merged.reactions.retain(|id, _| !keep_out(id));
    community.merged.metabolites.retain(|id, _| !keep_out(id));
    let report = prune_map_to_model(&mut map, &community, &HashSet::from(["other".to_string()]));
    assert!(map.has_reaction("other@At"), "its own reactions stay");
    assert!(map.has_node("met:M@a"), "the medium node its transfer is drawn to stays");
    assert!(map.has_reaction("EX_a_s"), "and the community exchange on that node");
    assert!(report.dropped.iter().any(|id| id == "demo@EX_a_e"), "a missing reaction of a species that is on is still gone");
}

#[test]
fn every_bundled_scenario_and_published_community_opens_as_saved() {
    let catalog: Value = serde_json::from_str(&std::fs::read_to_string(repo_root().join("models/scenarios.json")).unwrap()).unwrap();
    let scenarios = catalog["scenarios"].as_array().unwrap().clone();
    let published = catalog["publishedCommunities"].as_array().unwrap().clone();
    let open = |label: &str, how: &dyn Fn(&mut Store)| {
        let mut store = store();
        store.scenarios = scenarios.clone();
        store.published_communities = published.clone();
        how(&mut store);
        store.emit("probe");
        assert_eq!(store.unsaved_work(), unsaved(false, false, false), "{label} opens as Unsaved");
    };
    // Yeast9 and the Zuñiga case download their models; they are covered elsewhere.
    for scenario in scenarios.iter().filter(|s| s["loader"] != "yeast9") {
        let id = scenario["id"].as_str().unwrap();
        open(id, &|store| store.use_scenario(id).unwrap());
    }
    for community in published.iter().filter(|c| c["id"] != "zuniga-2020-phea-rescue") {
        let id = community["id"].as_str().unwrap();
        open(&format!("published:{id}"), &|store| store.use_published_community(id).unwrap());
    }
}

#[test]
fn a_zero_abundance_joint_member_is_held_out_like_a_switched_off_one() {
    let mut store = make_pair_store();
    // What `installJointProjection` leaves behind.
    store.joint_projection_spec = Some(Arc::new(SteadyComSpec {
        members: Vec::new(),
        total_biomass: 1.0,
        biomass_cap: None,
        community_uptake: IndexMap::new(),
        community_exchange_bounds: IndexMap::new(),
        joint: None,
    }));
    let mut community = (**store.community.as_ref().unwrap()).clone();
    community.abundances = IndexMap::from([("demo".to_string(), 1.0), ("other".to_string(), 0.0)]);
    store.community = Some(Arc::new(community));
    assert!(store.species_held_out("other"));
    assert!(!store.species_held_out("demo"));
}

#[test]
fn hand_edits_waiting_in_redo_keep_the_drawing_when_a_species_is_toggled() {
    let mut store = make_pair_store();
    hand_edit(&mut store, "hand edit", "met:demo@a_c");
    store.undo_map();
    assert!(!store.can_undo_map());
    assert!(store.can_redo_map());
    assert!(store.keeps_drawing_on_species_toggle(), "a relayout would clear Redo");
}

// Export file names

#[test]
fn exported_files_are_named_after_the_model_with_a_short_suffix() {
    let single = make_store();
    assert_eq!(single.export_base_name(), "audit-demo", "one species: the model id");
    assert_eq!(single.file_name_for(export_suffix::SCENARIO, "json"), "audit-demo_scenario.json");

    let mut pair = make_pair_store();
    assert_eq!(pair.export_base_name(), "audit-pair", "a community: its scenario id");
    assert_eq!(pair.file_name_for(export_suffix::FLUXES, "csv"), "audit-pair_fluxes.csv");
    pair.scenario = Some(json!({"id": "published:steadycom-gut-nine"}));
    assert_eq!(pair.export_base_name(), "steadycom-gut-nine", "published communities drop the prefix");
    pair.scenario = Some(json!({"id": "uploaded-models"}));
    assert_eq!(pair.export_base_name(), "audit-demo+audit-demo", "several uploads: their model ids");
}

#[test]
fn exporting_one_model_leaves_another_models_edits_unsaved() {
    let mut store = make_two_model_store();
    store.set_model_editor_species("demo");
    store.create_model_metabolite(&json!({"id": "b_c", "name": "B", "compartment": "c"})).unwrap();
    store.set_model_editor_species("oth");
    store.create_model_metabolite(&json!({"id": "b_c", "name": "B", "compartment": "c"})).unwrap();
    // `markModelSaved(model)` names the model by its key in the catalogue.
    store.mark_model_saved(Some("audit-demo"));
    assert!(store.unsaved_work().model, "the other model was not written out");
    store.mark_model_saved(Some("audit-other"));
    assert!(!store.unsaved_work().model);
}

#[test]
fn reopening_a_scenario_does_not_hide_edits_to_a_model_it_reuses() {
    let mut store = make_store();
    store.model_editor_species_id = Some("demo".into());
    store.create_model_metabolite(&json!({"id": "b_c", "name": "B", "compartment": "c"})).unwrap();
    assert!(store.unsaved_work().model);
    store.use_scenario("audit-demo").unwrap(); // the cached, edited model is used again
    assert!(store.unsaved_work().model, "the edit is still in memory and still unsaved");
}

#[test]
fn an_exported_map_does_not_keep_the_cross_feeding_event_picked_in_the_explorer() {
    let store = make_pair_store();
    let link = CrossFeedingLink { base: "a".into(), from: Some("demo".into()), to: Some("other".into()), flux: 1.0 };
    let event = CrossFeedingEvent { from: "demo".into(), to: "other".into(), base: "a".into(), flux: 1.0, ..Default::default() };
    let state = RenderState {
        show_cross_feeding: true,
        cross_feeding: Some(CrossFeeding { links: vec![link], per_metabolite: vec![] }),
        cross_feeding_explorer: Some(CrossFeedingExplorerView {
            enabled: true,
            visible_events: vec![event.clone()],
            selected_event: Some(event),
            selected_at: None,
        }),
        ..render_state(&store)
    };
    // A routed line carries its opacity on its group, an arc on the path
    // itself. The picked line also has `is-picked` in its class, which the
    // pattern allows (see the same test in `coxeter-render/tests/js_tests.rs`).
    let opacity = |svg: &str| -> f64 {
        re(r#"class="emc-crossfeed(?:-route)?(?: [^"]*)?"[^>]*opacity="([\d.]+)""#).captures(svg).expect("a cross-feeding line")[1]
            .parse()
            .unwrap()
    };
    let on_screen = opacity(&draw(&store, &state));
    let in_figure = opacity(&draw(&store, &RenderState { for_export: true, ..state }));
    assert!(on_screen > in_figure, "on screen the picked event stands out; in the figure it is one event among the others");
}
