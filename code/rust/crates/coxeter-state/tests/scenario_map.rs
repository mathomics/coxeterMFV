//! Port of `test/scenario-map.test.mjs`: a scenario that names a curated map
//! (`map`, a path under `models/`) opens with it; without one, or when the
//! file is not there, the automatic layout draws it. A published community
//! whose manifest names a map opens with that map.

mod common;

use coxeter_solver::lp::LpStatus;
use serde_json::json;

use common::{memory_store, model};

#[test]
fn a_scenario_opens_with_its_curated_map_and_with_the_automatic_layout_without_one() {
    let (mut store, assets) = memory_store();
    store.set_model(
        "demo",
        model(json!({
            "id": "demo",
            "compartments": { "c": "Cytosol", "e": "Extracellular" },
            "metabolites": [{ "id": "a_e", "compartment": "e" }, { "id": "a_c", "compartment": "c" }],
            "reactions": [
                { "id": "EX_a_e", "metabolites": { "a_e": -1 }, "lower_bound": -10, "upper_bound": 1000 },
                { "id": "At", "metabolites": { "a_e": -1, "a_c": 1 }, "lower_bound": 0, "upper_bound": 1000 },
                { "id": "BIOMASS", "metabolites": { "a_c": -1 }, "lower_bound": 0, "upper_bound": 1000, "objective_coefficient": 1 }
            ]
        })),
    );
    store.scenarios = vec![json!({
        "id": "demo", "name": "Demo", "objective": "equal-growth",
        "species": [{ "id": "d", "model": "demo", "abundance": 1 }],
        "medium": { "a": 10 }
    })];

    store.use_scenario("demo").unwrap();
    let mut automatic = store.map.as_ref().unwrap().reaction_ids();
    automatic.sort();
    assert!(automatic.len() > 1);
    assert_eq!(store.scenario_map_file, None);

    let kept = automatic[0].clone();
    store.map = Some(store.map.as_ref().unwrap().subset_by_reactions(&[kept.clone()]));
    assets.insert_json("models/maps/demo.json", &store.map_document().unwrap());
    store.scenarios[0]["map"] = json!("maps/demo.json");
    store.use_scenario("demo").unwrap();
    assert_eq!(store.map.as_ref().unwrap().reaction_ids(), vec![kept]);
    assert_eq!(store.scenario_map_file.as_deref(), Some("maps/demo.json"));
    assert_eq!(store.map_load_note, "", "the curated map fits its scenario as it is");
    assert!(!store.map_history.can_undo(), "and opens unedited");
    assert!(store.keeps_drawing_on_species_toggle());
    assert_eq!(store.solution.as_ref().unwrap().status, LpStatus::Optimal);

    store.scenarios[0]["map"] = json!("maps/missing.json");
    store.use_scenario("demo").unwrap();
    let mut drawn = store.map.as_ref().unwrap().reaction_ids();
    drawn.sort();
    assert_eq!(drawn, automatic);
    assert_eq!(store.scenario_map_file, None);
}

#[test]
fn a_published_community_whose_manifest_names_a_map_opens_with_it_and_keeps_it_when_a_member_is_switched_off() {
    let id = "stolyar-2007-syntrophy";
    let raw = std::fs::read_to_string(coxeter_core::testkit::repo_root().join(format!("models/published/{id}/map.json"))).unwrap();
    let drawn: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let mut store = common::init_store();
    store.use_published_community(id).unwrap();
    let map = store.map.as_ref().expect("a map");
    assert_eq!(map.reactions().len(), drawn["map"]["reactions"].as_array().unwrap().len());
    assert_eq!(map.texts().len(), drawn["map"]["texts"].as_array().unwrap().len(), "the curated map, not the automatic layout");
    assert_eq!(store.map_load_note, "", "it fits the member models as they are");
    assert_eq!(store.solution.as_ref().unwrap().status, LpStatus::Optimal);

    store.set_species_enabled("Mm", false).unwrap();
    assert_eq!(store.map.as_ref().unwrap().reactions().len(), drawn["map"]["reactions"].as_array().unwrap().len(), "switching a member off keeps the map");
}
