//! Port of the Store tests of `test/add-menu-labels.test.mjs`: the menus that
//! offer metabolites and reactions to add name them the way the map does. A
//! yeast-GEM model calls ATP `s_0434`; a map labelled by name or by BiGG id
//! offered its next reaction as `r_0534: s_0563 + s_0434 → …`.
//!
//! The panel and menu halves ("the Add Reaction panel lists reactions and
//! their equations…", "by default the panel still lists the model id…", "the
//! membrane menu offers transport reactions…") are in
//! `coxeter-app/tests/menus.rs`.

mod common;

use coxeter_state::Store;
use serde_json::{Value, json};

use common::{model, store};

/// A yeast-GEM in miniature: opaque ids, readable names, BiGG annotations.
pub fn yeast_like() -> Value {
    json!({
        "id": "ymini",
        "compartments": { "e": "extracellular", "c": "cytoplasm" },
        "metabolites": [
            { "id": "s_0563", "name": "D-glucose", "compartment": "e", "annotation": { "bigg.metabolite": "glc__D" } },
            { "id": "s_0565", "name": "D-glucose", "compartment": "c", "annotation": { "bigg.metabolite": "glc__D" } },
            { "id": "s_0434", "name": "ATP", "compartment": "c", "annotation": { "bigg.metabolite": "atp" } },
            { "id": "s_0394", "name": "ADP", "compartment": "c", "annotation": { "bigg.metabolite": "adp" } },
            { "id": "s_0568", "name": "D-glucose 6-phosphate", "compartment": "c", "annotation": { "bigg.metabolite": "g6p" } }
        ],
        "reactions": [
            { "id": "r_1714", "name": "D-glucose exchange", "metabolites": { "s_0563": -1 }, "lower_bound": -10, "upper_bound": 1000 },
            { "id": "r_1166", "name": "glucose transport", "metabolites": { "s_0563": -1, "s_0565": 1 }, "lower_bound": 0, "upper_bound": 1000, "annotation": { "bigg.reaction": "GLCt1" } },
            { "id": "r_0534", "name": "hexokinase", "metabolites": { "s_0565": -1, "s_0434": -1, "s_0568": 1, "s_0394": 1 }, "lower_bound": 0, "upper_bound": 1000, "annotation": { "bigg.reaction": "HEX1" } },
            { "id": "r_atp", "name": "ATP source", "metabolites": { "s_0394": -1, "s_0434": 1 }, "lower_bound": 0, "upper_bound": 1000 },
            { "id": "r_bio", "name": "growth", "metabolites": { "s_0568": -1 }, "lower_bound": 0, "upper_bound": 1000, "objective_coefficient": 1 }
        ]
    })
}

fn yeast_store() -> Store {
    let mut store = store();
    store.set_model("ymini", model(yeast_like()));
    store.scenarios = vec![json!({
        "id": "ymini", "name": "Yeast in miniature", "objective": "equal-growth",
        "species": [{ "id": "sce", "model": "ymini", "abundance": 1 }], "medium": { "s_0563": 10 }
    })];
    store.use_scenario("ymini").unwrap();
    store
}

/// The merged id of a local reaction or metabolite of `sce`.
fn merged(store: &Store, kind: &str, local: &str) -> String {
    let c = store.community.as_ref().unwrap();
    if kind == "metabolite" {
        c.metabolite_owner.iter().find(|(_, o)| o.species_id.as_deref() == Some("sce") && o.original_id == local).unwrap().0.clone()
    } else {
        c.reaction_owner.iter().find(|(_, o)| o.species_id.as_deref() == Some("sce") && o.original_id == local).unwrap().0.clone()
    }
}

#[test]
fn a_metabolite_or_reaction_not_yet_drawn_is_named_as_the_map_would_name_it() {
    let mut store = yeast_store();
    for source in ["name", "bigg", "id"] {
        store.set_map_label_source("metabolite", source, None);
        store.set_map_label_source("reaction", source, None);
        let map = store.map.as_ref().unwrap();
        for node in map.nodes().values() {
            let Some(merged_id) = node.merged_id.as_deref() else { continue };
            if node.extra.contains_key("labelSource") {
                continue;
            }
            assert_eq!(store.entity_label("metabolite", merged_id).label, store.map_label_text("metabolite", &node.id, None), "{source}: {}", node.id);
        }
        for id in map.reactions().keys() {
            assert_eq!(store.entity_label("reaction", id).label, store.map_label_text("reaction", id, None), "{source}: {id}");
        }
    }
}

#[test]
fn add_metabolite_lists_the_compartments_metabolites_in_the_maps_labels_searchable_by_id_too() {
    let mut store = yeast_store();
    store.set_map_label_source("metabolite", "name", None);
    let listed = store.metabolites_available_in_compartment("sce", "c");
    let atp = listed.iter().find(|m| m.model_id == "s_0434").unwrap();
    assert_eq!(atp.label, "ATP");
    assert_eq!(atp.name, "s_0434", "beside a name, the model id");

    store.set_map_label_source("metabolite", "bigg", None);
    let listed = store.metabolites_available_in_compartment("sce", "c");
    let g6p = listed.iter().find(|m| m.model_id == "s_0568").unwrap();
    assert_eq!((g6p.label.as_str(), g6p.name.as_str()), ("g6p", "D-glucose 6-phosphate"));
}

#[test]
fn the_reaction_library_finds_a_reaction_by_the_label_the_map_shows_and_lists_it_by_that_label() {
    let mut store = yeast_store();
    store.set_map_label_source("reaction", "bigg", None);
    let hits = store.search_reactions("HEX1", 30);
    let hit = hits.first().expect("found by its BiGG id");
    assert_eq!(hit.reaction_id, merged(&store, "reaction", "r_0534"));
    assert_eq!(hit.label, "HEX1");
    assert_eq!(hit.detail, "hexokinase");
    assert_eq!(store.search_reactions("r_0534", 30)[0].label, "HEX1", "the model id still finds it");
}
