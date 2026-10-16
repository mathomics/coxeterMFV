//! Port of `test/pathway-manager.test.mjs`.
//!
//! Not ported here: "map document v3 persists pathway manager data while
//! older maps remain parseable": `createMapDocument` / `parseMapDocument`
//! only, no Store; `coxeter-map/tests/parity_documents.rs` compares both with
//! the JS, pathway manager and legacy maps included.
//!
//! Adapted: the JS `makeStore()` stubs `store.community.merged.reactions`
//! with three annotated reactions and `store.map` with a drawn set. Here the
//! store opens a one-species scenario whose model has those three reactions
//! (R1, R2 in Glycolysis, R3 in the TCA cycle, all in subsystem "Central
//! carbon"), and draws it in full. The merged model names them `demo@R1`,
//! `demo@R2`, `demo@R3`, and it also holds the exchange, transport and
//! biomass reactions a community needs, which carry no pathway: the
//! assertions on R1–R3 are the JS ones.

mod common;

use coxeter_render::reaction_colors::{PathwayRibbonMode, ReactionColorMode};
use coxeter_state::Store;
use serde_json::json;

use common::{model, store};

const R1: &str = "demo@R1";
const R2: &str = "demo@R2";
const R3: &str = "demo@R3";

/// `makeStore()`.
fn make_store() -> Store {
    let mut store = store();
    store.set_model(
        "pathway-demo",
        model(json!({
            "id": "pathway-demo",
            "compartments": {"c": "Cytosol", "e": "Extracellular"},
            "metabolites": [
                {"id": "a_e", "compartment": "e"}, {"id": "a_c", "compartment": "c"},
                {"id": "b_c", "compartment": "c"}, {"id": "c_c", "compartment": "c"},
                {"id": "d_c", "compartment": "c"}
            ],
            "reactions": [
                {"id": "EX_a_e", "metabolites": {"a_e": -1}, "lower_bound": -10, "upper_bound": 1000},
                {"id": "At", "metabolites": {"a_e": -1, "a_c": 1}, "lower_bound": 0, "upper_bound": 1000},
                {"id": "R1", "metabolites": {"a_c": -1, "b_c": 1}, "lower_bound": 0, "upper_bound": 1000,
                 "pathway": "Glycolysis", "subsystem": "Central carbon"},
                {"id": "R2", "metabolites": {"b_c": -1, "c_c": 1}, "lower_bound": 0, "upper_bound": 1000,
                 "pathway": "Glycolysis", "subsystem": "Central carbon"},
                {"id": "R3", "metabolites": {"c_c": -1, "d_c": 1}, "lower_bound": 0, "upper_bound": 1000,
                 "pathway": "TCA cycle", "subsystem": "Central carbon"},
                {"id": "BIOMASS", "metabolites": {"d_c": -1}, "lower_bound": 0, "upper_bound": 1000, "objective_coefficient": 1}
            ]
        })),
    );
    store.scenarios = vec![json!({
        "id": "pathway-demo", "name": "Pathway demo", "objective": "equal-growth",
        "species": [{"id": "demo", "model": "pathway-demo", "abundance": 1}],
        "medium": {"a": 10}
    })];
    store.use_scenario("pathway-demo").unwrap();
    let map = store.map.as_ref().expect("the scenario is drawn");
    for rid in [R1, R2, R3] {
        assert!(map.has_reaction(rid), "{rid} is drawn");
    }
    store
}

fn ids(list: &[&str]) -> Option<Vec<String>> {
    Some(list.iter().map(|s| s.to_string()).collect())
}

fn sorted<'a>(it: impl IntoIterator<Item = &'a String>) -> Vec<String> {
    let mut v: Vec<String> = it.into_iter().cloned().collect();
    v.sort();
    v
}

#[test]
fn manual_pathway_is_created_from_selected_reactions_and_activates_ribbons() {
    let mut store = make_store();
    store.multi_selection.reactions.insert(R1.into());
    store.multi_selection.reactions.insert(R2.into());

    let pathway = store.create_manual_pathway("Upper glycolysis", Some("#123456"), None).unwrap();

    assert_eq!(pathway.name, "Upper glycolysis");
    assert_eq!(pathway.color, "#123456");
    assert_eq!(sorted(&pathway.reaction_ids), [R1, R2]);
    assert_eq!(store.view.pathway_ribbon_mode, PathwayRibbonMode::Pathway);
    assert_eq!(store.view.reaction_color_mode, ReactionColorMode::Flux);
}

#[test]
fn manual_pathways_are_exclusive_annotations_without_altering_model_metadata() {
    let mut store = make_store();
    let first = store.create_manual_pathway("First", None, ids(&[R1, R2]).as_deref()).unwrap();
    let second = store.create_manual_pathway("Second", None, ids(&[R2, R3]).as_deref()).unwrap();

    assert_eq!(store.manual_pathways[&first.id].reaction_ids.iter().collect::<Vec<_>>(), [R1]);
    assert_eq!(sorted(&store.manual_pathways[&second.id].reaction_ids), [R2, R3]);
    assert_eq!(store.community.as_ref().unwrap().merged.reactions[R2].pathway, "Glycolysis");
}

#[test]
fn manual_pathway_overrides_model_pathway_colour_in_managed_pathway_view() {
    let mut store = make_store();
    store.create_manual_pathway("Custom route", Some("#abcdef"), ids(&[R1]).as_deref()).unwrap();

    let coloring = store.managed_pathway_coloring(true);
    assert_eq!(coloring.groups.get(R1).map(String::as_str), Some("Custom route"));
    assert_eq!(coloring.colors.get(R1).map(String::as_str), Some("#abcdef"));
    assert_eq!(coloring.groups.get(R2).map(String::as_str), Some("Glycolysis"));
}

#[test]
fn pathway_manager_can_hide_model_and_manual_pathways() {
    let mut store = make_store();
    let manual = store.create_manual_pathway("Manual TCA", Some("#ff0000"), ids(&[R3]).as_deref()).unwrap();

    store.set_pathway_visible("model:Glycolysis", false).unwrap();
    store.set_pathway_visible(&format!("manual:{}", manual.id), false).unwrap();

    let coloring = store.managed_pathway_coloring(true);
    assert!(!coloring.groups.contains_key(R1));
    assert!(!coloring.groups.contains_key(R2));
    assert!(!coloring.groups.contains_key(R3));
}

#[test]
fn pathway_manager_selects_all_drawn_reactions_in_a_pathway() {
    let mut store = make_store();
    let mut ids = store.select_pathway("model:Glycolysis");
    ids.sort();

    assert_eq!(ids, [R1, R2]);
    assert_eq!(sorted(&store.multi_selection.reactions), [R1, R2]);
    assert!(store.view.multi_select);
}

#[test]
fn manual_pathways_and_pathway_display_settings_round_trip_through_manager_state() {
    let mut store = make_store();
    let manual = store.create_manual_pathway("Custom", Some("#654321"), ids(&[R1]).as_deref()).unwrap();
    store.set_pathway_color("model:Glycolysis", "#112233").unwrap();
    store.set_pathway_visible("model:TCA cycle", false).unwrap();

    let state = store.pathway_manager_state();
    let mut restored = make_store();
    restored.restore_pathway_manager_state(Some(&state));

    assert_eq!(restored.manual_pathways[&manual.id].color, "#654321");
    assert_eq!(restored.manual_pathways[&manual.id].reaction_ids.iter().collect::<Vec<_>>(), [R1]);
    assert_eq!(restored.pathway_settings["Glycolysis"].color(), Some("#112233"));
    assert_eq!(restored.pathway_settings["TCA cycle"].0.get("visible"), Some(&json!(false)));
}
