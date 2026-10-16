//! Port of `test/waiting-map.test.mjs`: a map opened before the model it was
//! drawn over. A model imported from a file lives only as long as the
//! session, so a map drawn over it and opened again after a restart finds
//! nothing to sit on. It is kept instead of refused, and drawn once that
//! model is loaded.

mod common;

use coxeter_core::js;
use coxeter_core::testkit::repo_root;
use coxeter_io::model_import::ImportFile;
use coxeter_state::{StatusState, Store};
use serde_json::{Value, json};

use common::{model, store};

fn yeast_text() -> String {
    std::fs::read_to_string(repo_root().join("models/yeast_core_mc.json")).unwrap()
}

fn other_model() -> Value {
    json!({
        "id": "other",
        "compartments": { "c": "Cytosol", "e": "Extracellular" },
        "metabolites": [{ "id": "a_e", "compartment": "e" }, { "id": "a_c", "compartment": "c" }],
        "reactions": [
            { "id": "EX_a_e", "metabolites": { "a_e": -1 }, "lower_bound": -10, "upper_bound": 1000 },
            { "id": "At", "metabolites": { "a_e": -1, "a_c": 1 }, "lower_bound": 0, "upper_bound": 1000 },
            { "id": "BIOMASS", "metabolites": { "a_c": -1 }, "lower_bound": 0, "upper_bound": 1000, "objective_coefficient": 1 }
        ]
    })
}

/// A session as it opens: a teaching scenario that has nothing to do with the map.
fn fresh_store() -> Store {
    let mut store = store();
    store.set_model("other", model(other_model()));
    store.scenarios = vec![json!({
        "id": "demo", "name": "Demo", "objective": "equal-growth",
        "species": [{ "id": "demo", "model": "other", "abundance": 1 }],
        "medium": { "a": 10 }
    })];
    store.use_scenario("demo").unwrap();
    store
}

fn import(store: &mut Store, name: &str, text: String) {
    store.import_model_files(&[ImportFile { name: name.into(), text }]).expect("the upload is accepted");
}

/// A map drawn by hand over an imported yeast, with one metabolite moved:
/// (its text, the node moved, where it went, how many reactions).
fn saved_yeast_map() -> (String, String, f64, usize) {
    let mut store = fresh_store();
    import(&mut store, "my_yeast.json", yeast_text());
    let (id, x, y) = {
        let node = store.map.as_ref().unwrap().nodes().values().find(|n| n.merged_id.is_some()).unwrap();
        (node.id.clone(), node.x, node.y)
    };
    let tx = store.begin_map_edit("Move");
    store.move_node(&id, x + 123.0, y + 45.0);
    store.commit_map_edit(tx);
    let map = store.map.as_ref().unwrap();
    (store.map_json().unwrap(), id.clone(), map.node(&id).unwrap().x, map.reactions().len())
}

#[test]
fn a_map_whose_imported_model_is_not_loaded_waits_and_is_drawn_when_that_model_is_imported() {
    let (text, node, x, reactions) = saved_yeast_map();
    let mut store = fresh_store();
    let before = store.map.as_ref().unwrap().to_json();

    store.load_map_document_named(&text, Some("yeast_map.json")).unwrap();
    let waiting = store.waiting_map.clone().expect("it waits");
    assert_eq!(waiting.name.as_deref(), Some("yeast_map.json"));
    assert_eq!(waiting.needs, "“my_yeast”", "named after the file the model came from");
    assert_eq!(store.map.as_ref().unwrap().to_json(), before, "the drawing on screen is untouched while the map waits");

    import(&mut store, "my_yeast.json", yeast_text());
    assert!(store.waiting_map.is_none());
    let map = store.map.as_ref().unwrap();
    assert_eq!(map.reactions().len(), reactions);
    assert_eq!(map.node(&node).unwrap().x, x, "the hand-placed metabolite is where it was saved");
    let message = &store.status().message;
    assert!(
        regex::Regex::new(r"Imported 1 model\(s\).*Drew “yeast_map\.json”, which was waiting for this model\.").unwrap().is_match(message),
        "{message}"
    );
}

#[test]
fn the_model_under_a_re_exported_file_name_still_takes_its_waiting_map() {
    let (text, node, x, _) = saved_yeast_map();
    let mut store = fresh_store();
    store.load_map_document_named(&text, Some("yeast_map.json")).unwrap();
    import(&mut store, "my_yeast_v2.json", yeast_text());
    assert!(store.waiting_map.is_none());
    assert_eq!(store.map.as_ref().unwrap().node(&node).unwrap().x, x);
}

#[test]
fn a_model_the_map_does_not_fit_leaves_it_waiting_and_so_does_switching_scenario() {
    let (text, _, _, _) = saved_yeast_map();
    let mut store = fresh_store();
    store.load_map_document_named(&text, Some("yeast_map.json")).unwrap();

    store.use_scenario("demo").unwrap();
    assert!(store.waiting_map.is_some(), "browsing scenarios is not loading the map's model");

    import(&mut store, "other.json", js::stringify(&other_model(), None));
    assert!(store.waiting_map.is_some());
    assert_eq!(store.status().state, StatusState::Warn);
    assert!(store.status().message.contains("“yeast_map.json” is still waiting: it does not fit this model"), "{}", store.status().message);

    import(&mut store, "my_yeast.json", yeast_text());
    assert!(store.waiting_map.is_none());
}

#[test]
fn a_waiting_map_can_be_forgotten_and_is_replaced_by_the_next_map_opened() {
    let (text, node, x, _) = saved_yeast_map();
    let mut store = fresh_store();
    store.load_map_document_named(&text, Some("first.json")).unwrap();
    store.load_map_document_named(&text, Some("second.json")).unwrap();
    assert_eq!(store.waiting_map.as_ref().unwrap().name.as_deref(), Some("second.json"));

    store.discard_waiting_map();
    assert!(store.waiting_map.is_none());
    import(&mut store, "my_yeast.json", yeast_text());
    assert_ne!(store.map.as_ref().unwrap().node(&node).map(|n| n.x), Some(x), "a forgotten map is not drawn");
}

#[test]
fn a_file_that_is_not_a_map_is_still_refused_not_kept() {
    let mut store = fresh_store();
    let broken = json!({ "format": "EscherMCMCMap", "version": 3, "metadata": null, "map": { "reactions": "x" } });
    let error = store.load_map_document_named(&broken.to_string(), Some("broken.json")).expect_err("refused");
    assert!(error.to_string().contains("EditableMap structure"), "{error}");
    assert!(store.waiting_map.is_none());
}
