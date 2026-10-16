//! Port of the Store tests of `test/map-document.test.mjs` ("opening a map
//! from another page").
//!
//! Not ported here, as they test `src/map/map-document.js` alone (no Store):
//! "V0.8 creates a versioned EscherMCMCMap document", "V0.8 parses its own
//! document format", "legacy raw EditableMap JSON remains loadable", "future
//! unsupported document versions are rejected", "map context catches a
//! different biological model", "map context accepts the same active species
//! and model" and "a published community map opens whichever build saved
//! it". `createMapDocument`, `parseMapDocument` and `validateMapContext` are
//! compared with the JavaScript on these very cases (and more) in
//! `coxeter-map/tests/parity_documents.rs`.

mod common;

use coxeter_state::Store;
use serde_json::{Value, json};

use common::{bundled_model, store};

/// `twoScenarioStore()`: the two bundled core models and a scenario over
/// each, nothing opened yet.
fn two_scenario_store() -> Store {
    let mut store = store();
    for name in ["ecoli_core_mc", "yeast_core_mc"] {
        store.set_model(name, bundled_model(name));
    }
    store.scenarios = vec![
        json!({
            "id": "just-ecoli", "name": "Just E. coli", "medium": { "glc__D": 10, "o2": 20 },
            "species": [{ "id": "eco", "model": "ecoli_core_mc" }],
        }),
        json!({
            "id": "just-yeast", "name": "Just yeast", "medium": { "glc__D": 10, "o2": 20 },
            "species": [{ "id": "sce", "model": "yeast_core_mc" }],
        }),
    ];
    store
}

fn drawn_reactions(store: &Store) -> usize {
    store.map.as_ref().expect("a map is drawn").reactions().len()
}

/// "a map opens in the scenario it was saved in, not only the active one"
#[test]
fn a_map_opens_in_the_scenario_it_was_saved_in_not_only_the_active_one() {
    // Save a map, close the page, open it again: the app is back on its
    // default scenario, and the file belongs to another one. Refusing it
    // there would make saving a map useless across sessions.
    let mut store = two_scenario_store();
    store.use_scenario("just-yeast").unwrap();
    let drawn = drawn_reactions(&store);
    let saved = store.map_json().unwrap();

    store.use_scenario("just-ecoli").unwrap();
    store.load_map_document(&saved).unwrap();

    assert_eq!(store.scenario_id().as_deref(), Some("just-yeast"), "the saved scenario is restored");
    assert_eq!(drawn_reactions(&store), drawn, "the drawing arrives intact");
    assert_eq!(store.species[0].id, "sce", "and the model underneath it is the right one");
}

/// "a map that fits the open scenario loads without moving anywhere"
#[test]
fn a_map_that_fits_the_open_scenario_loads_without_moving_anywhere() {
    let mut store = two_scenario_store();
    store.use_scenario("just-ecoli").unwrap();
    let saved = store.map_json().unwrap();
    // `store.subscribe(...)` from here on: what was emitted before is dropped.
    store.take_events();

    store.load_map_document(&saved).unwrap();
    let rebuilds = store.take_events();

    assert_eq!(store.scenario_id().as_deref(), Some("just-ecoli"));
    assert!(!rebuilds.iter().any(|r| r == "rebuild"), "no scenario switch happens when none is needed");
}

/// "a map whose scenario this build does not have waits, and says which"
/// (80e0611: refused before).
#[test]
fn a_map_whose_scenario_this_build_does_not_have_waits_and_says_which() {
    let mut store = two_scenario_store();
    store.use_scenario("just-yeast").unwrap();
    let mut saved: Value = serde_json::from_str(&store.map_json().unwrap()).unwrap();
    saved["metadata"]["scenario"] = json!("a-scenario-that-went-away");

    store.use_scenario("just-ecoli").unwrap();
    let open = store.map.as_ref().unwrap().to_json();
    store.load_map_document(&serde_json::to_string(&saved).unwrap()).expect("kept, not refused");
    let waiting = store.waiting_map.as_ref().expect("the map waits");
    assert!(waiting.reason.contains("a-scenario-that-went-away"), "the reason names the scenario: {}", waiting.reason);
    assert_eq!(store.scenario_id().as_deref(), Some("just-ecoli"), "and nothing is disturbed on the way");
    assert_eq!(store.map.as_ref().unwrap().to_json(), open);
}
