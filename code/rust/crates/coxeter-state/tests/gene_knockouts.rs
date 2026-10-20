//! Gene knockouts through the reactions' gene rules (port of
//! `test/gene-knockouts.test.mjs`; the rule evaluation itself is tested in
//! `coxeter_io::gene_rules`).

mod common;

use coxeter_core::model::Model;
use coxeter_state::reactions_explorer::RowStatus;
use coxeter_state::{Bounds, Store};
use serde_json::json;

/// One cell: a takes two routes to b (R1, and R2 or R3), b feeds biomass.
fn make_store() -> Store {
    let mut store = common::store();
    let model = Model::from_value(&json!({
        "id": "gpr-demo",
        "metabolites": [{ "id": "a_e", "compartment": "e" }, { "id": "a_c", "compartment": "c" }, { "id": "b_c", "compartment": "c" }],
        "reactions": [
            { "id": "EX_a_e", "metabolites": { "a_e": -1 }, "lower_bound": -10, "upper_bound": 1000 },
            { "id": "At", "metabolites": { "a_e": -1, "a_c": 1 }, "lower_bound": 0, "upper_bound": 1000, "gene_reaction_rule": "g2 or g3" },
            { "id": "R1", "metabolites": { "a_c": -1, "b_c": 1 }, "lower_bound": 0, "upper_bound": 1000, "gene_reaction_rule": "g1" },
            { "id": "R2", "metabolites": { "a_c": -1, "b_c": 1 }, "lower_bound": 0, "upper_bound": 1000, "gene_reaction_rule": "(g1 and g4) or (g5 and g6)" },
            { "id": "R3", "metabolites": { "a_c": -1, "b_c": 1 }, "lower_bound": 0, "upper_bound": 1000, "gene_reaction_rule": "g1 and g7" },
            { "id": "BIOMASS", "metabolites": { "b_c": -1 }, "lower_bound": 0, "upper_bound": 1000, "objective_coefficient": 1 },
        ],
        "genes": ["g1", "g2", "g3", "g4", "g5", "g6", "g7"],
    }))
    .unwrap();
    store.set_model("gpr-demo", model);
    store.scenarios = vec![json!({
        "id": "gpr-demo", "name": "GPR demo", "objective": "equal-growth",
        "species": [{ "id": "demo", "model": "gpr-demo", "abundance": 1 }], "medium": { "a": 10 },
    })];
    store.use_scenario("gpr-demo").unwrap();
    assert!(store.solution.as_ref().unwrap().is_optimal());
    store
}

fn objective_value(store: &Store) -> f64 {
    store.solution.as_ref().unwrap().objective_value
}

fn closed(store: &Store) -> Vec<String> {
    store.gene_closed_reactions().keys().cloned().collect()
}

#[test]
fn a_gene_knockout_closes_exactly_the_reactions_its_rules_lose_and_undo_reset_and_the_scenario_file_follow_it() {
    let mut store = make_store();
    assert_eq!(store.gene_search("g1", 8)[0].closes, ["demo@R1", "demo@R3"], "the preview names what it would close");

    store.toggle_gene_knockout("demo@g1").unwrap();
    assert_eq!(closed(&store), ["demo@R1", "demo@R3"]);
    assert_eq!(store.current_reaction_bounds("demo@R1"), Bounds { lower: 0.0, upper: 0.0 });
    assert!(!store.reaction_knocked_out("demo@R2"), "g5 and g6 still make R2");
    assert!(store.solution.as_ref().unwrap().is_optimal());
    assert!(store.solution.as_ref().unwrap().fluxes.get("demo@R1").copied().unwrap_or(0.0).abs() < 1e-9);
    let row = store.reaction_table_rows().into_iter().find(|r| r.id() == "demo@R3").unwrap();
    assert_eq!((row.status, row.gene_knockouts), (RowStatus::KnockedOut, vec!["demo@g1".to_string()]));
    let changes = store.gene_knockout_changes();
    assert_eq!((changes[0].key.as_str(), changes[0].title.as_str(), changes[0].reactions.clone()), ("demo@g1", "g1", vec!["demo@R1".to_string(), "demo@R3".to_string()]));
    let greyed: Vec<String> = store.constraint_changes().into_iter().filter(|c| c.knocked_out).map(|c| c.id).collect();
    assert_eq!(greyed, ["demo@R1", "demo@R3"], "the map greys them out");

    let growth = objective_value(&store);
    store.set_gene_knockouts(&["demo@g2".to_string(), "demo@g3".to_string()], true).unwrap();
    assert!(objective_value(&store).abs() < 1e-9, "both transporter isozymes gone: no growth");
    store.undo_constraints().unwrap();
    assert!((objective_value(&store) - growth).abs() < 1e-9);
    assert_eq!(store.gene_knockouts.iter().collect::<Vec<_>>(), ["demo@g1"]);

    let file = store.state_json();
    let doc: serde_json::Value = serde_json::from_str(&file).unwrap();
    assert_eq!(doc["geneKnockouts"], json!(["demo@g1"]));
    let mut other = make_store();
    other.load_state(&file).unwrap();
    assert_eq!(closed(&other), ["demo@R1", "demo@R3"]);
    let bundle = store.reproducibility_bundle().unwrap();
    assert_eq!(bundle["constraints"]["geneKnockouts"], json!(["demo@g1"]));
    let replayed = make_store().import_reproducibility_bundle(&bundle).unwrap();
    assert_eq!(replayed["replay"]["pass"], true, "the bundle replays with the gene knockout: {}", replayed["replay"]);

    store.undo_constraints().unwrap();
    assert!(!store.reaction_knocked_out("demo@R1"), "undo reopens what the gene closed");
    store.redo_constraints().unwrap();
    store.reset_all_reactions().unwrap();
    assert!(store.gene_knockouts.is_empty());
    assert!(store.constraint_changes().is_empty());
}
