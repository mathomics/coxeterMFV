//! Port of `test/simulation-mode.test.mjs` (the Store parts).
//!
//! Not ported here: "real pFBA solver fixes the requested fraction of
//! optimum": `FluxProblem` only, no Store; ported as a unit test in
//! `coxeter-solver/src/fba.rs`.
//!
//! Adapted: the JS `mockStore()` replaces `store.problem` with an object that
//! counts `optimize` and `parsimonious` calls, records the pFBA `fraction`,
//! and returns an optimum equal to that fraction; it also stubs
//! `store.community`. The Rust store holds a real `FluxProblem`, so the mock
//! is a two-reaction problem whose FBA optimum is 1 (an uptake of at most 1
//! feeding the objective `R`): pFBA at a fraction then reaches exactly that
//! fraction of it, as the stub did. The solves are counted through the
//! `solution` events an action emits (one per solve), and told apart by the
//! result: a pFBA result carries `total_flux`, a plain FBA does not. The pFBA
//! fraction is read back as the objective it reached. No community is
//! needed: with none, cross-feeding and growth rates are empty, as the stub
//! returned.
//!
//! "an analysis that fails is reported as not finished" replaces the stub's
//! `optimize` by an infeasible problem (the objective reaction is forced
//! above what the uptake allows): HiGHS's message replaces the stub's "No
//! feasible point.", and the status line must carry it after "FBA did not
//! finish: ".
//!
//! "running an analysis from the panel says when it has finished" records
//! `lastRun.phase` at each `subscribe` call in JS; here the phase is read
//! where the events are taken: after `run_analysis` returns (the run was
//! announced) and after its background solve has been applied.

mod common;

use std::sync::Arc;

use coxeter_core::model::Model;
use coxeter_solver::fba::FluxProblem;
use coxeter_state::util::describe_status;
use coxeter_state::{RunPhase, StatusState, Store};
use serde_json::json;

use common::{WAIT, store};

/// The FBA optimum of [`mock_store`]'s problem.
const OPTIMUM: f64 = 1.0;

fn mock_model() -> Model {
    Model::from_value(&json!({
        "id": "mock",
        "compartments": {"c": "cytosol"},
        "metabolites": [{"id": "a_c", "compartment": "c"}],
        "reactions": [
            {"id": "EX_a", "metabolites": {"a_c": -1}, "lower_bound": -OPTIMUM, "upper_bound": 0},
            {"id": "R", "metabolites": {"a_c": -1}, "lower_bound": 0, "upper_bound": 1000, "objective_coefficient": 1}
        ]
    }))
    .expect("a valid model")
}

/// `mockStore()`.
fn mock_store() -> Store {
    let mut store = store();
    store.problem = Some(Arc::new(FluxProblem::new(&mock_model())));
    store
}

/// `calls()` and `pfbaFractions` of the JS mock.
#[derive(Debug, Default)]
struct Calls {
    fba: u32,
    pfba: u32,
    pfba_fractions: Vec<f64>,
}

impl Calls {
    /// `{ fbaCalls, pfbaCalls }`.
    fn counts(&self) -> (u32, u32) {
        (self.fba, self.pfba)
    }

    /// Run one store action and count the solves it ran.
    fn track<T>(&mut self, store: &mut Store, action: impl FnOnce(&mut Store) -> T) -> T {
        store.take_events();
        let out = action(store);
        store.wait_for_jobs(WAIT);
        let solves = store.take_events().iter().filter(|e| *e == "solution").count() as u32;
        if solves > 0 {
            let solution = store.solution.as_ref().expect("a solve leaves a solution");
            let result = solution.result.as_ref().expect("a solver result");
            if result.total_flux.is_some() {
                self.pfba += solves;
                self.pfba_fractions.push(solution.objective_value / OPTIMUM);
            } else {
                self.fba += solves;
            }
        }
        out
    }
}

fn close(a: Option<&f64>, b: f64) -> bool {
    a.is_some_and(|a| (a - b).abs() < 1e-9)
}

#[test]
fn simulation_defaults_to_standard_fba() {
    let mut store = mock_store();
    let mut calls = Calls::default();
    assert!(!store.parsimonious);
    calls.track(&mut store, |s| s.solve());
    assert_eq!(calls.counts(), (1, 0));
}

#[test]
fn checking_pfba_switches_solves_to_parsimonious_fba() {
    let mut store = mock_store();
    let mut calls = Calls::default();
    calls.track(&mut store, |s| s.set_parsimonious(true).unwrap());
    assert!(store.parsimonious);
    assert_eq!(calls.counts(), (0, 1));
    calls.track(&mut store, |s| s.solve());
    assert_eq!(calls.counts(), (0, 2));
}

#[test]
fn unchecking_pfba_returns_to_standard_fba() {
    let mut store = mock_store();
    let mut calls = Calls::default();
    calls.track(&mut store, |s| s.set_parsimonious(true).unwrap());
    calls.track(&mut store, |s| s.set_parsimonious(false).unwrap());
    assert!(!store.parsimonious);
    assert_eq!(calls.counts(), (1, 1));
}

#[test]
fn scenario_json_persists_the_selected_mode() {
    let mut store = mock_store();
    store.parsimonious = true;
    let doc: serde_json::Value = serde_json::from_str(&store.state_json()).unwrap();
    assert_eq!(doc["parsimonious"], json!(true));
}

#[test]
fn pfba_defaults_to_100_percent_of_the_primary_optimum() {
    let mut store = mock_store();
    let mut calls = Calls::default();
    assert_eq!(store.pfba_biomass_percent, 100.0);
    calls.track(&mut store, |s| s.set_parsimonious(true).unwrap());
    assert!(close(calls.pfba_fractions.last(), 1.0));
}

#[test]
fn user_can_set_pfba_biomass_target_as_a_percentage() {
    let mut store = mock_store();
    let mut calls = Calls::default();
    calls.track(&mut store, |s| s.set_parsimonious(true).unwrap());
    calls.track(&mut store, |s| s.set_pfba_biomass_percent(95.0).unwrap());
    assert_eq!(store.pfba_biomass_percent, 95.0);
    assert!(close(calls.pfba_fractions.last(), 0.95));

    calls.track(&mut store, |s| s.set_pfba_biomass_percent(87.5).unwrap());
    assert_eq!(store.pfba_biomass_percent, 87.5);
    assert!(close(calls.pfba_fractions.last(), 0.875));
}

#[test]
fn pfba_biomass_percentage_is_clamped_to_0_100_and_ignores_temporary_blank_input() {
    let mut store = mock_store();
    store.set_pfba_biomass_percent(120.0).unwrap();
    assert_eq!(store.pfba_biomass_percent, 100.0);
    store.set_pfba_biomass_percent(-5.0).unwrap();
    assert_eq!(store.pfba_biomass_percent, 0.0);
    store.set_pfba_biomass_percent("").unwrap();
    assert_eq!(store.pfba_biomass_percent, 0.0);
}

#[test]
fn scenario_json_persists_the_pfba_biomass_target() {
    let mut store = mock_store();
    store.pfba_biomass_percent = 92.5;
    let doc: serde_json::Value = serde_json::from_str(&store.state_json()).unwrap();
    assert_eq!(doc["pfbaBiomassPercent"], json!(92.5));
}

#[test]
fn running_an_analysis_from_the_panel_says_when_it_has_finished() {
    let mut store = mock_store();
    let mut calls = Calls::default();
    let mut seen: Vec<RunPhase> = Vec::new();
    store.take_events();
    store.run_analysis("pfba").unwrap();
    assert_eq!(store.last_run.as_ref().unwrap().phase, RunPhase::Running);
    assert_eq!(store.status().state, StatusState::Busy);
    assert!(store.status().message.contains("Running pFBA"));
    if !store.take_events().is_empty() {
        seen.push(store.last_run.as_ref().unwrap().phase);
    }
    calls.track(&mut store, |_| ());
    // `track` took the events of the background solve: the run emitted.
    seen.push(store.last_run.as_ref().unwrap().phase);
    assert_eq!(calls.counts(), (0, 1));
    let run = store.last_run.as_ref().unwrap();
    assert_eq!(run.phase, RunPhase::Done);
    assert_eq!(run.outcome.as_deref(), Some("optimal"));
    assert!(run.ms.is_some_and(f64::is_finite));
    assert_eq!(store.status().state, StatusState::Ok);
    assert!(
        store.status().message.starts_with("pFBA completed · objective 1.00000 · "),
        "{}",
        store.status().message
    );
    assert!(seen.contains(&RunPhase::Running) && seen.last() == Some(&RunPhase::Done));
    assert!(store.log.iter().any(|entry| entry.message.contains("pFBA completed")));
}

#[test]
fn an_analysis_that_fails_is_reported_as_not_finished() {
    let mut store = mock_store();
    // `store.problem.optimize = () => ({ status: 'infeasible', … })`: the
    // objective reaction is forced above what the uptake can feed.
    let mut problem = FluxProblem::new(&mock_model());
    problem.set_bounds("R", Some(2.0 * OPTIMUM), None).unwrap();
    store.problem = Some(Arc::new(problem));
    store.run_analysis_blocking("fba", WAIT).unwrap();
    let run = store.last_run.as_ref().unwrap();
    assert_eq!(run.phase, RunPhase::Failed);
    assert_eq!(run.outcome.as_deref(), Some("infeasible"));
    assert_eq!(store.status().state, StatusState::Error);
    let solution = store.solution.as_ref().unwrap();
    let why = if solution.message.is_empty() { describe_status(solution.status.as_str()) } else { solution.message.clone() };
    assert_eq!(store.status().message, format!("FBA did not finish: {why}"));
}

#[test]
fn an_unknown_analysis_is_refused() {
    let mut store = mock_store();
    let error = store.run_analysis("sampling").unwrap_err();
    assert!(error.to_string().contains("Unknown analysis"), "{error}");
}
