//! The JS unit tests of the solver, ported: test/backend.test.mjs,
//! test/simplex.test.mjs (its semantics, on HiGHS), test/loopless.test.mjs,
//! test/fva-mode.test.mjs and the solver parts of test/simulation-mode.test.mjs
//! and test/dual-analysis.test.mjs (those two are also unit tests in src/fba.rs).
//!
//! The Store/UI halves of those files (checkbox wiring, persistence, rendering)
//! belong to the GUI and are not ported here.

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use coxeter_core::Model;
use coxeter_core::testkit::read_fixture;
use coxeter_solver::fba::{FluxProblem, FvaOptions, LooplessOptions};
use coxeter_solver::lp::{LpProblem, LpStatus, SolveOptions, SparseColumns, solve_lp};
use coxeter_solver::milp::{MilpOptions, solve_binary_milp};
use common::{close, load_model, num};
use serde_json::json;

const MODELS: [&str; 5] = ["cyano_core_mc", "ecoli_core_mc", "lactobacillus_core_mc", "methanogen_core_mc", "yeast_core_mc"];

// backend.test.mjs

#[test]
fn every_bundled_model_solves_to_a_feasible_optimum() {
    for name in MODELS {
        let model = load_model(name);
        let res = FluxProblem::new(&model).optimize();
        assert_eq!(res.status, LpStatus::Optimal, "{name}");
        let mut residual = indexmap::IndexMap::<String, f64>::new();
        for (rid, v) in &res.fluxes {
            if *v == 0.0 {
                continue;
            }
            let rxn = &model.reactions[rid];
            assert!(*v >= rxn.lower_bound - 1e-6 && *v <= rxn.upper_bound + 1e-6, "{name} {rid} = {v} outside its bounds");
            for (mid, coef) in &rxn.metabolites {
                *residual.entry(mid.clone()).or_default() += coef * v;
            }
        }
        for (mid, value) in residual {
            assert!(value.abs() < 1e-6, "{name}: {mid} unbalanced by {value}");
        }
    }
    assert!(FluxProblem::new(&load_model("ecoli_core_mc")).optimize().objective_value > 0.8);
}

#[test]
fn an_essential_knockout_stops_growth_and_does_not_leak() {
    let mut problem = FluxProblem::new(&load_model("ecoli_core_mc"));
    let res = problem.knockout("ENO").unwrap();
    assert!(res.status != LpStatus::Optimal || res.objective_value < 1e-6, "expected no growth, got {}", res.objective_value);
    assert!(problem.optimize().objective_value > 0.5, "knockout leaked");
    assert_eq!(problem.backend(), "highs");
}

// simplex.test.mjs

fn bound(v: &serde_json::Value) -> f64 {
    match v.as_str() {
        Some("inf") => f64::INFINITY,
        Some("-inf") => f64::NEG_INFINITY,
        _ => v.as_f64().unwrap(),
    }
}

/// Every LP of test/simplex.test.mjs (and its 2-D random sweep), as the JS
/// simplex answered it.
#[test]
fn the_simplex_test_problems_get_the_javascript_answers() {
    let fx = read_fixture("solver/lp-cases.json").expect("run rust/tools/fixtures-solver.mjs");
    for case in fx["lp"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let rows = case["rows"].as_u64().unwrap() as usize;
        let cols = case["cols"].as_u64().unwrap() as usize;
        let mut a = SparseColumns::new(rows, cols);
        for e in case["entries"].as_array().unwrap() {
            a.set(e[0].as_u64().unwrap() as usize, e[1].as_u64().unwrap() as usize, e[2].as_f64().unwrap());
        }
        let vec = |k: &str| case[k].as_array().unwrap().iter().map(bound).collect::<Vec<f64>>();
        let p = LpProblem::equality(a, &vec("b"), vec("c"), vec("lb"), vec("ub"), case["maximize"].as_bool().unwrap());
        let r = solve_lp(&p, &SolveOptions::default());
        assert_eq!(r.status.as_str(), case["status"], "{name}: {}", r.message);
        if r.status != LpStatus::Optimal {
            assert!(r.x.is_none() && r.objective.is_nan(), "{name}: a failed solve carries no point");
            continue;
        }
        assert!(close(r.objective, num(&case["objective"]), 1e-7), "{name}: {} vs {}", r.objective, case["objective"]);
        // One equality row, so the point is unique unless the objective is
        // parallel to the constraint; the named cases are not.
        if !name.starts_with("random") {
            for (j, v) in case["x"].as_array().unwrap().iter().enumerate() {
                assert!(close(r.x.as_ref().unwrap()[j], num(v), 1e-7), "{name}: x[{j}]");
            }
        }
        if name.starts_with("duals") {
            for (i, v) in case["duals"].as_array().unwrap().iter().enumerate() {
                assert!(close(r.duals.as_ref().unwrap()[i], num(v), 1e-9), "{name}: dual {i}");
            }
            for (j, v) in case["reducedCosts"].as_array().unwrap().iter().enumerate() {
                assert!(close(r.reduced_costs.as_ref().unwrap()[j], num(v), 1e-9), "{name}: reduced cost {j}");
            }
        }
    }
}

#[test]
fn equality_constraints_hold_on_a_flow_network() {
    // A small flow network: conservation at each node, maximise throughput.
    let (m, n) = (4, 7);
    let mut a = SparseColumns::new(m, n);
    let edges = [(0, 1), (0, 2), (1, 2), (1, 3), (2, 3), (3, 0), (2, 0)];
    for (j, (from, to)) in edges.iter().enumerate() {
        a.set(*from, j, -1.0);
        a.set(*to, j, 1.0);
    }
    let mut c = vec![0.0; n];
    c[3] = 1.0;
    let r = solve_lp(&LpProblem::equality(a, &vec![0.0; m], c, vec![0.0; n], vec![5.0; n], true), &SolveOptions::default());
    assert_eq!(r.status, LpStatus::Optimal);
    let x = r.x.unwrap();
    let mut residual = [0.0; 4];
    for (j, (from, to)) in edges.iter().enumerate() {
        residual[*from] -= x[j];
        residual[*to] += x[j];
    }
    for (i, v) in residual.iter().enumerate() {
        assert!(v.abs() < 1e-8, "node {i} residual {v}");
    }
}

#[test]
fn the_milp_test_problems_get_the_javascript_answers() {
    let fx = read_fixture("solver/lp-cases.json").unwrap();
    for case in fx["milp"].as_array().unwrap() {
        let cols = case["cols"].as_u64().unwrap() as usize;
        let mut a = SparseColumns::new(1, cols);
        for e in case["entries"].as_array().unwrap() {
            a.add(e[0].as_u64().unwrap() as usize, e[1].as_u64().unwrap() as usize, e[2].as_f64().unwrap());
        }
        let vec = |k: &str| case[k].as_array().unwrap().iter().map(bound).collect::<Vec<f64>>();
        let p = LpProblem::equality(a, &vec("b"), vec("c"), vec![0.0; cols], vec("ub"), true);
        let binaries: Vec<usize> = case["binaries"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as usize).collect();
        let r = solve_binary_milp(&p, &binaries, &MilpOptions { node_limit: 100, time_limit_ms: 1000.0, ..Default::default() });
        assert_eq!(r.status.as_str(), case["status"], "{}", case["name"]);
        assert_eq!(r.proven_optimal, case["provenOptimal"].as_bool().unwrap());
        if r.status == LpStatus::Optimal {
            assert!(close(r.objective, num(&case["objective"]), 1e-9));
        }
    }
}

// loopless.test.mjs

fn forced_cycle_model() -> Model {
    Model::from_value(&json!({
        "id": "forced_cycle", "compartments": {"c": "cytosol"},
        "metabolites": [{"id": "a_c", "compartment": "c"}, {"id": "b_c", "compartment": "c"}],
        "reactions": [
            {"id": "R1", "metabolites": {"a_c": -1, "b_c": 1}, "lower_bound": 1, "upper_bound": 1000, "objective_coefficient": 1},
            {"id": "R2", "metabolites": {"b_c": -1, "a_c": 1}, "lower_bound": 1, "upper_bound": 1000}
        ]
    }))
    .unwrap()
}

fn feasible_pathway(reversible_r1: bool) -> Model {
    Model::from_value(&json!({
        "id": "pathway", "compartments": {"c": "cytosol"},
        "metabolites": [{"id": "a_c", "compartment": "c"}, {"id": "b_c", "compartment": "c"}],
        "reactions": [
            {"id": "EX_a", "metabolites": {"a_c": -1}, "lower_bound": -10, "upper_bound": 1000},
            {"id": "R1", "metabolites": {"a_c": -1, "b_c": 1}, "lower_bound": if reversible_r1 { -1000 } else { 0 }, "upper_bound": 1000},
            {"id": "DM_b", "metabolites": {"b_c": -1}, "lower_bound": 0, "upper_bound": 1000, "objective_coefficient": 1}
        ]
    }))
    .unwrap()
}

fn quick() -> LooplessOptions {
    LooplessOptions { node_limit: 1000, time_limit_ms: 2000.0, ..Default::default() }
}

#[test]
fn ordinary_fba_permits_a_forced_internal_cycle() {
    let r = FluxProblem::new(&forced_cycle_model()).optimize();
    assert_eq!(r.status, LpStatus::Optimal);
    assert!(r.objective_value > 0.0);
}

#[test]
fn loopless_fba_rejects_a_forced_thermodynamically_impossible_cycle() {
    let r = FluxProblem::new(&forced_cycle_model()).loopless(&quick());
    assert_eq!(r.status, LpStatus::Infeasible);
    assert!(!r.dual_analysis.available);
    assert_eq!(r.reduced_cost_scale, None);
}

#[test]
fn loopless_fba_preserves_a_feasible_acyclic_pathway_optimum() {
    let p = FluxProblem::new(&feasible_pathway(false));
    let ordinary = p.optimize();
    let loopless = p.loopless(&quick());
    assert_eq!(ordinary.status, LpStatus::Optimal);
    assert_eq!(loopless.status, LpStatus::Optimal);
    assert!((loopless.objective_value - ordinary.objective_value).abs() < 1e-7);
    assert!(loopless.loopless.unwrap().proven_optimal);
}

#[test]
fn reversible_internal_reactions_create_binary_direction_variables() {
    let r = FluxProblem::new(&feasible_pathway(true)).loopless(&quick());
    assert_eq!(r.status, LpStatus::Optimal);
    assert_eq!(r.loopless.unwrap().binary_variables, 1);
}

// fva-mode.test.mjs

#[test]
fn progressive_fva_matches_plain_fva_and_reports_progress() {
    let problem = FluxProblem::new(&load_model("ecoli_core_mc"));
    let ids = ["PGI", "TALA", "EX_o2_e"];
    let expected = problem.variability(&ids, &FvaOptions::default());
    let progress = Mutex::new(Vec::new());
    let on_progress = |p: &coxeter_solver::fba::FvaProgress<'_>| progress.lock().unwrap().push((p.done, p.total));
    let actual = problem.variability(&ids, &FvaOptions { on_progress: Some(&on_progress), ..Default::default() });
    assert_eq!(actual.len(), ids.len());
    let progress = progress.into_inner().unwrap();
    assert_eq!(progress.last(), Some(&(ids.len(), ids.len())));
    assert_eq!(progress.iter().map(|p| p.0).collect::<Vec<_>>(), [1, 2, 3], "done counts up");
    for rid in ids {
        assert!((actual[rid].min - expected[rid].min).abs() < 1e-7, "{rid} min differs");
        assert!((actual[rid].max - expected[rid].max).abs() < 1e-7, "{rid} max differs");
    }
}

#[test]
fn fva_can_be_cancelled_between_reactions() {
    let problem = FluxProblem::new(&load_model("ecoli_core_mc"));
    let ids = ["PGI", "TALA", "PFK", "PYK"];
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let on_progress = move |p: &coxeter_solver::fba::FvaProgress<'_>| {
        if p.done == 1 {
            flag.store(true, Ordering::Relaxed);
        }
    };
    let result = problem.variability(
        &ids,
        &FvaOptions { threads: Some(1), on_progress: Some(&on_progress), cancel: Some(cancel.clone()), ..Default::default() },
    );
    assert_eq!(result.len(), 1);
    assert!(result.contains_key("PGI"));

    // In parallel the workers stop at their next reaction: fewer than all.
    cancel.store(false, Ordering::Relaxed);
    let all: Vec<String> = problem.rxn_ids.clone();
    let stopper = cancel.clone();
    let stop_early = move |p: &coxeter_solver::fba::FvaProgress<'_>| {
        if p.done == 3 {
            stopper.store(true, Ordering::Relaxed);
        }
    };
    let partial = problem.variability(
        &all,
        &FvaOptions { threads: Some(4), on_progress: Some(&stop_early), cancel: Some(cancel), ..Default::default() },
    );
    assert!(partial.len() >= 3 && partial.len() < all.len(), "{} of {}", partial.len(), all.len());
    let full = problem.variability_all(&FvaOptions::default());
    for (rid, r) in &partial {
        assert_eq!(r, &full[rid], "a cancelled run's finished ranges are the real ones");
    }
}

#[test]
fn fva_of_an_infeasible_problem_is_empty() {
    let mut problem = FluxProblem::new(&forced_cycle_model());
    problem.set_bounds("R1", Some(1.0), Some(1.0)).unwrap();
    problem.set_bounds("R2", Some(2.0), Some(2.0)).unwrap();
    assert_eq!(problem.optimize().status, LpStatus::Infeasible);
    assert!(problem.variability_all(&FvaOptions::default()).is_empty());
}
