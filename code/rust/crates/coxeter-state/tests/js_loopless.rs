//! Port of `test/loopless.test.mjs` (the Store parts).
//!
//! Not ported here: "ordinary FBA permits a forced internal stoichiometric
//! cycle", "loopless FBA rejects a forced thermodynamically impossible cycle",
//! "loopless FBA preserves a feasible acyclic pathway optimum" and "reversible
//! internal reactions create MILP binary direction variables": `FluxProblem`
//! only, no Store; ported in `coxeter-solver/tests/js_unit_fba.rs`.
//!
//! Adapted: the JS `mockStore()` replaces `store.problem` with an object that
//! counts calls to `optimize`, `parsimonious` and `loopless`, and stubs
//! `store.community`. The Rust store holds a real `FluxProblem` (a small
//! pathway, see `mock_store`), and the calls are told apart by what they
//! leave behind: a loopless MILP result carries `loopless` details, a pFBA
//! result a `total_flux`, a plain FBA neither; and every solve logs one
//! "<method> solved" line (back-to-back solves fold into one entry with a
//! count). No community is needed: with none, cross-feeding and growth rates
//! are empty, as the JS stub returned.

mod common;

use std::sync::Arc;

use coxeter_core::model::Model;
use coxeter_solver::fba::FluxProblem;
use coxeter_state::{FvaRange, LogKind, Store};
use indexmap::IndexMap;
use serde_json::json;

use common::store;

/// `mockStore()`: a store whose problem is the small feasible pathway of
/// this file's solver tests (`feasiblePathwayModel()`: `EX_a` feeds `R1`,
/// which feeds the objective `DM_b`). A reaction with no metabolites at all
/// would not do: the loopless MILP rightly finds no direction for it.
fn mock_store() -> Store {
    let model = Model::from_value(&json!({
        "id": "pathway",
        "compartments": {"c": "cytosol"},
        "metabolites": [{"id": "a_c", "compartment": "c"}, {"id": "b_c", "compartment": "c"}],
        "reactions": [
            {"id": "EX_a", "metabolites": {"a_c": -1}, "lower_bound": -10, "upper_bound": 1000},
            {"id": "R1", "metabolites": {"a_c": -1, "b_c": 1}, "lower_bound": 0, "upper_bound": 1000},
            {"id": "DM_b", "metabolites": {"b_c": -1}, "lower_bound": 0, "upper_bound": 1000, "objective_coefficient": 1}
        ]
    }))
    .expect("a valid model");
    let mut store = store();
    store.problem = Some(Arc::new(FluxProblem::new(&model)));
    store
}

/// `{ normal, pfba, loopless }`: the solves the store has run, by method,
/// read from the log.
fn calls(store: &Store) -> (u32, u32, u32) {
    let mut out = (0, 0, 0);
    for entry in store.log.iter().filter(|e| e.kind == LogKind::Solve) {
        // Folded solves keep the last message only; every test here runs at
        // most one solve per method in a row.
        if entry.message.starts_with("Loopless FBA solved") {
            out.2 += entry.count;
        } else if entry.message.starts_with("pFBA solved") {
            out.1 += entry.count;
        } else if entry.message.starts_with("FBA solved") {
            out.0 += entry.count;
        }
    }
    out
}

#[test]
fn loopless_checkbox_switches_store_solves_to_loopless_milp() {
    let mut store = mock_store();
    store.set_loopless(true).unwrap();
    assert!(store.loopless);
    assert!(!store.parsimonious);
    assert_eq!(calls(&store), (0, 0, 1));
    let result = store.solution.as_ref().and_then(|s| s.result.as_ref()).expect("a solver result");
    assert!(result.loopless.is_some(), "the solution comes from the loopless MILP");
    assert_eq!(result.total_flux, None, "not from pFBA");
}

#[test]
fn loopless_is_mutually_exclusive_with_pfba_and_fva() {
    let mut store = mock_store();
    store.parsimonious = true;
    store.fva_enabled = true;
    store.fva_ranges = IndexMap::from([("R1".to_string(), FvaRange { min: 0.0, max: 1.0 })]);

    store.set_loopless(true).unwrap();
    assert!(store.loopless);
    assert!(!store.parsimonious);
    assert!(!store.fva_enabled);

    store.set_parsimonious(true).unwrap();
    assert!(!store.loopless);
    assert!(store.parsimonious);
}

#[test]
fn scenario_json_persists_loopless_mode() {
    let mut store = mock_store();
    store.loopless = true;
    let doc: serde_json::Value = serde_json::from_str(&store.state_json()).unwrap();
    assert_eq!(doc["loopless"], json!(true));
}
