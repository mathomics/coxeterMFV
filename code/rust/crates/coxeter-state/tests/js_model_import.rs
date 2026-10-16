//! Port of the Store tests of `test/model-import.test.mjs`: what an upload
//! leaves on screen.
//!
//! Ported: "uploading a small model draws it, rather than leaving an empty
//! cell" and "uploading a model too large to lay out says so instead of going
//! quiet" (the two that build a `new Store`).
//!
//! Not ported here — they test `src/core/model-import.js` alone and are
//! ported in coxeter-io (`tests/js_tests.rs`): "a normal COBRA JSON model
//! imports", "an exchange uptake lower bound becomes inferred community
//! medium", "multiple uploaded JSON files become distinct species", "a
//! reaction referencing an unknown metabolite is rejected clearly", "duplicate
//! reaction ids are rejected before model construction", "invalid JSON names
//! the offending file", "SBML Level 3 FBC imports compartments, bounds,
//! objective and GPR", "generic model parser detects SBML by extension or XML
//! content", "paired SBML species keep notes after the genome-scale import
//! optimisation", "JSON and SBML files can be imported together as a
//! community", "invalid XML is rejected with a clear SBML error".

mod common;

use coxeter_core::js;
use coxeter_core::testkit::repo_root;
use coxeter_io::model_import::ImportFile;
use regex::Regex;
use serde_json::{Value, json};

use common::store;

/// `assert.match(text, /pattern/)`.
#[track_caller]
fn assert_matches(text: &str, pattern: &str) {
    assert!(Regex::new(pattern).unwrap().is_match(text), "{text:?} does not match /{pattern}/");
}

/// `oversizedDoc()`: a model with more metabolites than the automatic draw is
/// willing to lay out.
fn oversized_doc() -> Value {
    let metabolites: Vec<Value> = (0..600)
        .map(|i| json!({ "id": format!("m{i}_c"), "name": format!("M{i}"), "compartment": "c", "formula": "C", "charge": 0 }))
        .collect();
    let mut reactions: Vec<Value> = (0..599)
        .map(|i| {
            json!({
                "id": format!("R{i}"), "name": format!("R{i}"),
                "metabolites": { (format!("m{i}_c")): -1, (format!("m{}_c", i + 1)): 1 },
                "lower_bound": 0, "upper_bound": 1000,
            })
        })
        .collect();
    reactions.push(json!({
        "id": "BIOMASS", "name": "Biomass", "metabolites": { "m599_c": -1 },
        "lower_bound": 0, "upper_bound": 1000, "objective_coefficient": 1,
    }));
    json!({ "id": "oversized", "compartments": { "c": "Cytosol" }, "metabolites": metabolites, "reactions": reactions })
}

/// JS: 'uploading a small model draws it, rather than leaving an empty cell'
#[test]
fn uploading_a_small_model_draws_it_rather_than_leaving_an_empty_cell() {
    let mut store = store();
    let text = std::fs::read_to_string(repo_root().join("models").join("ecoli_core_mc.json")).unwrap();

    store.import_model_files(&[ImportFile { name: "ecoli_core_mc.json".into(), text }]).expect("the upload is accepted");

    // The case this covers: a shell (cells and compartments) drawn with nothing
    // inside it reads as an import that silently failed.
    let map = store.map.as_ref().expect("a map");
    assert!(map.cells().len() == 1, "the cell is drawn");
    assert!(map.reactions().len() > 50, "reactions drawn: {}", map.reactions().len());
    assert!(map.nodes().len() > 50, "metabolites drawn: {}", map.nodes().len());
    assert_eq!(store.map_mode, "full");
    assert_matches(&store.status().message, "Drawn in full");
}

/// JS: 'uploading a model too large to lay out says so instead of going quiet'
#[test]
fn uploading_a_model_too_large_to_lay_out_says_so_instead_of_going_quiet() {
    let mut store = store();

    store
        .import_model_files(&[ImportFile { name: "oversized.json".into(), text: js::stringify(&oversized_doc(), None) }])
        .expect("the upload is accepted");

    // Too crowded to lay out in the time anyone will wait, and with no annotated
    // central carbon metabolism to fall back on — so it stays empty, but the
    // status has to say that, and say what to press.
    assert_eq!(store.map.as_ref().expect("a map").reactions().len(), 0);
    assert_eq!(store.map_mode, "empty");
    assert_matches(&store.status().message, "too large to lay out");
    assert_matches(&store.status().message, "Full map");
}
