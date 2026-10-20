//! Every solver against the same answers: the self-test battery, and FBA,
//! pFBA, FVA and loopless FBA on every fixture case against the JS app's
//! answers (rust/tools/fixtures-solver.mjs).
//!
//! Gurobi and CPLEX are tested where they are installed with a licence, and
//! the tests say so and pass where they are not. A licence that only allows
//! small problems (CPLEX Community, Gurobi's pip licence) is tested on the
//! problems it accepts.

mod common;

use coxeter_core::testkit::read_fixture;
use coxeter_solver::backends::{Solver, SolverKind, probe};
use coxeter_solver::fba::{FluxProblem, FvaOptions, FvaRange, LooplessOptions, PfbaOptions};
use coxeter_solver::lp::LpStatus;
use coxeter_solver::selftest::self_test;
use common::{close, community_from_spec, load_model, num};
use indexmap::IndexMap;
use serde_json::Value;

const TOL: f64 = 1e-6;

/// The installed solvers that open with a licence.
fn installed() -> Vec<Solver> {
    let mut solvers = Vec::new();
    for kind in [SolverKind::Gurobi, SolverKind::Cplex] {
        let info = probe(kind, None);
        match info.solver {
            Some(solver) => {
                eprintln!("{}{}", solver.label(), solver.limit().map(|l| format!(" ({l})")).unwrap_or_default());
                solvers.push(solver);
            }
            None => eprintln!("{kind} not tested: {}", info.error.unwrap_or_default()),
        }
    }
    solvers
}

fn assert_self_test(solver: &Solver) {
    let failed: Vec<String> =
        self_test(solver).into_iter().filter(|c| !c.passed).map(|c| format!("{}: {}", c.name, c.detail)).collect();
    assert!(failed.is_empty(), "{}:\n{}", solver.label(), failed.join("\n"));
}

#[test]
fn highs_passes_the_self_test() {
    assert_self_test(&Solver::highs());
}

#[test]
fn installed_solvers_pass_the_self_test() {
    for solver in installed() {
        assert_self_test(&solver);
    }
}

fn cases() -> Vec<(String, Value)> {
    let index = read_fixture("solver/index.json").expect("run rust/tools/fixtures-solver.mjs");
    index["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let name = c.as_str().unwrap().to_string();
            let fx = read_fixture(&format!("solver/fba-{name}.json")).unwrap();
            (name, fx)
        })
        .collect()
}

fn problem_for(fx: &Value, solver: &Solver) -> FluxProblem {
    let mut p = match fx["kind"].as_str().unwrap() {
        "model" => FluxProblem::new(&load_model(fx["model"].as_str().unwrap())),
        _ => FluxProblem::new(&community_from_spec(&fx["spec"]).merged),
    };
    p.set_solver(solver.clone());
    p
}

fn check_ranges(label: &str, ours: &IndexMap<String, FvaRange>, js: &Value) {
    let js = js.as_object().unwrap();
    assert_eq!(ours.len(), js.len(), "{label}: number of ranges");
    for (rid, pair) in js {
        let r = ours.get(rid).unwrap_or_else(|| panic!("{label}: no range for {rid}"));
        let (min, max) = (num(&pair[0]), num(&pair[1]));
        assert!(close(r.min, min, TOL), "{label} {rid}: min {} vs JS {min}", r.min);
        assert!(close(r.max, max, TOL), "{label} {rid}: max {} vs JS {max}", r.max);
    }
}

/// Whether a size-limited licence refused this problem: such a result is
/// skipped, never read as an answer.
fn refused(solver: &Solver, status: LpStatus, message: &str) -> bool {
    solver.limit().is_some() && status == LpStatus::IterationLimit && message.contains("larger than")
}

#[test]
fn installed_solvers_match_the_javascript_on_every_case() {
    for solver in installed() {
        let label = solver.label();
        let (mut compared, mut skipped) = (0, 0);
        for (name, fx) in cases() {
            let p = problem_for(&fx, &solver);
            let fba = p.optimize();
            if refused(&solver, fba.status, &fba.message) {
                skipped += 1;
                continue;
            }
            assert_eq!(fba.status.as_str(), fx["fba"]["status"], "{label} {name}: FBA status ({})", fba.message);
            let js_opt = num(&fx["fba"]["objectiveValue"]);
            assert!(close(fba.objective_value, js_opt, TOL), "{label} {name}: FBA {} vs JS {js_opt}", fba.objective_value);

            let min = p.optimize_with(false);
            assert_eq!(min.status.as_str(), fx["fbaMin"]["status"], "{label} {name}: minimisation status");
            if min.status == LpStatus::Optimal {
                assert!(close(min.objective_value, num(&fx["fbaMin"]["objectiveValue"]), TOL), "{label} {name}: minimum");
            }

            for (key, fraction) in [("pfba", 1.0), ("pfba90", 0.9)] {
                let r = p.parsimonious(PfbaOptions { fraction, ..Default::default() });
                let js = &fx[key];
                // pFBA falls back on the FBA result when its own LP is refused.
                let Some(total) = r.total_flux else {
                    assert!(solver.limit().is_some(), "{label} {name} {key}: no total flux");
                    continue;
                };
                assert_eq!(r.status.as_str(), js["status"], "{label} {name} {key}: status");
                assert!(close(r.objective_value, num(&js["objectiveValue"]), TOL), "{label} {name} {key}: objective");
                assert!(close(total, num(&js["totalFlux"]), TOL), "{label} {name} {key}: total flux {total} vs JS {}", js["totalFlux"]);
            }

            check_ranges(&format!("{label} {name} FVA 100%"), &p.variability_all(&FvaOptions::default()), &fx["fva"]);
            check_ranges(&format!("{label} {name} FVA 90%"), &p.variability_all(&FvaOptions { fraction: 0.9, ..Default::default() }), &fx["fva90"]);

            let js = &fx["loopless"];
            if js["provenOptimal"] == true {
                let r = p.loopless(&LooplessOptions { time_limit_ms: 60_000.0, ..Default::default() });
                if !refused(&solver, r.status, &r.message) {
                    assert_eq!(r.status.as_str(), js["status"], "{label} {name}: loopless status ({})", r.message);
                    if r.status == LpStatus::Optimal {
                        assert!(close(r.objective_value, num(&js["objectiveValue"]), TOL), "{label} {name}: loopless objective");
                    }
                }
            }
            compared += 1;
        }
        eprintln!("{label}: {compared} cases compared, {skipped} too large for its licence");
        assert!(compared > 0, "{label}: no case was small enough to compare");
    }
}

#[test]
fn a_result_names_the_solver_that_found_it() {
    for solver in installed() {
        let model = load_model("ecoli_core_mc");
        let mut p = FluxProblem::new(&model);
        p.set_solver(solver.clone());
        assert_eq!(p.optimize().solver, solver.kind());
        assert_eq!(p.backend(), solver.kind().id());
    }
}
