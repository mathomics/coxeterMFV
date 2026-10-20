//! The Add Reaction list ordered by flux and Clear solution (port of
//! `test/solution-controls.test.mjs`), and CycleFreeFlux as a method of the
//! Store (the Store part of `test/cycle-free-flux.test.mjs`).

mod common;

use std::time::Duration;

use coxeter_core::model::Model;
use coxeter_state::Store;
use serde_json::json;

/// One cell: a is taken up and turned into b by R1 (held at 6) and R3 (the
/// remaining 2, as 2 a -> 2 b), with R2 and R4 idle; b feeds biomass.
fn make_store() -> Store {
    let mut store = common::store();
    let model = Model::from_value(&json!({
        "id": "routes",
        "metabolites": [{ "id": "a_e", "compartment": "e" }, { "id": "a_c", "compartment": "c" }, { "id": "b_c", "compartment": "c" }],
        "reactions": [
            { "id": "EX_a_e", "metabolites": { "a_e": -1 }, "lower_bound": -10, "upper_bound": 1000 },
            { "id": "At", "metabolites": { "a_e": -1, "a_c": 1 }, "lower_bound": 0, "upper_bound": 1000 },
            { "id": "R2", "metabolites": { "a_c": -1, "b_c": 1 }, "lower_bound": 0, "upper_bound": 0 },
            { "id": "R1", "metabolites": { "a_c": -1, "b_c": 1 }, "lower_bound": 6, "upper_bound": 6 },
            { "id": "R3", "metabolites": { "a_c": -2, "b_c": 2 }, "lower_bound": 0, "upper_bound": 1000 },
            { "id": "R4", "metabolites": { "a_c": -1, "b_c": 1 }, "lower_bound": 0, "upper_bound": 0 },
            { "id": "BIOMASS", "metabolites": { "b_c": -1 }, "lower_bound": 0, "upper_bound": 1000, "objective_coefficient": 1 },
        ],
    }))
    .unwrap();
    store.set_model("routes", model);
    store.scenarios = vec![json!({
        "id": "routes", "name": "Routes", "objective": "equal-growth",
        "species": [{ "id": "demo", "model": "routes", "abundance": 1 }], "medium": { "a": 10 },
    })];
    store.use_scenario("routes").unwrap();
    assert!(store.solution.as_ref().unwrap().is_optimal());
    store
}

#[test]
fn add_reaction_lists_flux_carrying_reactions_first_by_absolute_flux() {
    let mut store = make_store();
    for id in ["R1", "R2", "R3", "R4"] {
        store.map.as_mut().unwrap().remove_reaction(&format!("demo@{id}"));
    }
    let node = store.map.as_ref().unwrap().nodes_for_metabolite("demo@a_c")[0].id.clone();
    assert_eq!(store.reactions_available_from_metabolite(&node), ["demo@R1", "demo@R3", "demo@R2", "demo@R4"]);
    store.clear_solution();
    assert_eq!(
        store.reactions_available_from_metabolite(&node),
        ["demo@R2", "demo@R1", "demo@R3", "demo@R4"],
        "without a solution the list keeps the model order"
    );
}

#[test]
fn clear_solution_empties_the_solution_and_holds_it_through_bound_edits_until_an_analysis_runs() {
    let mut store = make_store();
    assert!(store.clear_solution());
    assert!(store.solution.is_none());
    assert!(store.growth.is_empty());
    assert!(store.cross_feeding.links.is_empty());
    assert!(store.fva_ranges.is_empty());

    store.set_reaction_bounds("demo@R1", 4.0, 4.0).unwrap();
    store.solve();
    store.set_medium_limit("a", 8.0).unwrap();
    assert!(store.solution.is_none(), "edits made while cleared do not solve");
    assert!(!store.clear_solution(), "nothing left to clear");

    store.run_analysis_blocking("fba", Duration::from_secs(30)).unwrap();
    let solution = store.solution.clone().unwrap();
    assert!(solution.is_optimal());
    assert!((solution.objective_value - 8.0).abs() < 1e-6, "the run sees the edits");
    store.set_medium_limit("a", 7.0).unwrap();
    assert!((store.solution.as_ref().unwrap().objective_value - 7.0).abs() < 1e-6, "and edits re-solve again");
}

#[test]
fn cycle_free_flux_is_run_like_the_other_methods_and_saved_with_the_scenario() {
    let mut store = make_store();
    store.parsimonious = true;
    store.run_analysis_blocking("cyclefree", Duration::from_secs(30)).unwrap();
    assert_eq!(store.last_run.as_ref().unwrap().label, "CycleFreeFlux");
    assert!(store.solution.as_ref().unwrap().result.as_ref().unwrap().cycle_free.is_some(), "solved by CycleFreeFlux");
    assert!((store.solution.as_ref().unwrap().objective_value - 10.0).abs() < 1e-6);
    assert_eq!((store.cycle_free, store.parsimonious), (true, false));
    let doc: serde_json::Value = serde_json::from_str(&store.state_json()).unwrap();
    assert_eq!(doc["cycleFree"], json!(true));
    store.set_loopless(true).unwrap();
    assert!(!store.cycle_free, "Loopless takes its place");
}
