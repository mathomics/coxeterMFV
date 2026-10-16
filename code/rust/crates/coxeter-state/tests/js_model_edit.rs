//! Port of the Store test of `test/model-edit.test.mjs`.
//!
//! Ported in coxeter-io (`tests/js_tests.rs`, `src/core/model-edit.js` alone,
//! no Store): "stoichiometry accepts comma/line syntax and JSON objects", "new
//! metabolite is added to the biological Model and exported", "new reaction
//! validates stoichiometry and exports bounds and GPR", "genes mentioned in a
//! new GPR are added to model genes" and "duplicate IDs, bad compartments,
//! unknown metabolites and bad bounds are rejected".

mod common;

use coxeter_io::model_edit::{add_metabolite_to_model, add_reaction_to_model};
use coxeter_state::Species;
use serde_json::{Value, json};

use common::{model, store};

/// `model()`: two cytosolic metabolites, no reaction.
fn demo_model() -> coxeter_core::model::Model {
    model(json!({
        "id": "demo",
        "name": "Demo",
        "compartments": { "c": "cytosol", "e": "extracellular" },
        "metabolites": [
            { "id": "a_c", "name": "A", "compartment": "c" },
            { "id": "b_c", "name": "B", "compartment": "c" },
        ],
        "reactions": [],
        "genes": [],
    }))
}

/// "Store model export contains user-created metabolites and reactions"
#[test]
fn store_model_export_contains_user_created_metabolites_and_reactions() {
    let mut m = demo_model();
    add_metabolite_to_model(&mut m, &json!({ "id": "c_c", "name": "C", "compartment": "c" })).unwrap();
    add_reaction_to_model(
        &mut m,
        &json!({
            "id": "R_EXPORT",
            "name": "Exported edit",
            "metabolites": "a_c:-1,c_c:1",
            "lowerBound": -5,
            "upperBound": 10,
            "gpr": "gx1 or gx2",
        }),
    )
    .unwrap();

    let mut store = store();
    store.set_model("demo-key", m);
    store.species = vec![Species::new("sp1", "demo-key", 1.0, true)];
    store.model_editor_species_id = Some("sp1".into());

    let doc: Value = serde_json::from_str(&store.model_json(None).unwrap()).unwrap();
    assert!(doc["metabolites"].as_array().unwrap().iter().any(|x| x["id"] == "c_c"));
    let reaction = doc["reactions"].as_array().unwrap().iter().find(|x| x["id"] == "R_EXPORT");
    assert!(reaction.is_some());
    let reaction = reaction.unwrap();
    assert_eq!(reaction["metabolites"], json!({ "a_c": -1, "c_c": 1 }));
    assert_eq!(reaction["lower_bound"], json!(-5));
    assert_eq!(reaction["upper_bound"], json!(10));
    assert_eq!(reaction["gene_reaction_rule"], json!("gx1 or gx2"));
}
