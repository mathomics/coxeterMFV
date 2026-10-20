//! The store on the solver the user chose: every analysis runs on it, the
//! solution and the reproducibility bundle name it, a change of solver
//! solves again, and a bundle replayed on another solver says so.
//!
//! Not a port of a JS test: the JS app has no solver to choose. These tests
//! run on each Gurobi or CPLEX installed with a licence, and say so and pass
//! where there is none. The small E. coli scenario fits the licences that
//! only allow small problems.

mod common;

use coxeter_solver::backends::{Solver, SolverKind, probe};
use coxeter_state::{RunPhase, Store};

use common::{WAIT, store};

/// The installed solvers that open with a licence.
fn installed() -> Vec<Solver> {
    let mut solvers = Vec::new();
    for kind in [SolverKind::Gurobi, SolverKind::Cplex] {
        let info = probe(kind, None);
        match info.solver {
            Some(solver) => solvers.push(solver),
            None => eprintln!("{kind} not tested: {}", info.error.unwrap_or_default()),
        }
    }
    solvers
}

/// The E. coli scenario, opened on `solver` from the start.
fn store_on(solver: &Solver) -> Store {
    let mut store = store();
    store.solver = solver.clone();
    store.init().expect("the bundled catalogue loads");
    store.use_scenario("solo-ecoli").unwrap();
    store
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6 * a.abs().max(b.abs()).max(1.0)
}

#[test]
fn every_analysis_runs_on_the_chosen_solver() {
    for solver in installed() {
        let label = solver.label();
        let mut store = store_on(&solver);
        assert_eq!(store.problem.as_ref().unwrap().backend(), solver.kind().id(), "{label}: the problem");
        for method in ["fba", "pfba", "fva"] {
            store.run_analysis_blocking(method, WAIT).unwrap();
            let run = store.last_run.as_ref().unwrap();
            assert_eq!(run.phase, RunPhase::Done, "{label} {method}: {:?} ({})", run.outcome, store.status().message);
            let solution = store.solution.as_ref().unwrap();
            assert_eq!(solution.solver, solver.kind().id(), "{label} {method}: the solution's solver");
        }
        assert!(!store.fva_ranges.is_empty(), "{label}: FVA ranges");
        assert_eq!(store.solution_solver_label().as_deref(), Some(label.as_str()));
        // E. coli core has no loopless flux distribution: the JS app proves
        // the MILP infeasible too (fixtures/solver/fba-scenario-solo-ecoli-*).
        store.run_analysis_blocking("loopless", WAIT).unwrap();
        let run = store.last_run.as_ref().unwrap();
        assert_eq!(run.outcome.as_deref(), Some("infeasible"), "{label} loopless ({})", store.status().message);
        assert_eq!(store.solution.as_ref().unwrap().solver, solver.kind().id(), "{label} loopless: the solution's solver");
    }
}

#[test]
fn a_change_of_solver_solves_again_with_the_same_optimum() {
    let solvers = installed();
    // Gurobi to CPLEX where both are there, else HiGHS to the one there is.
    let (from, to) = match solvers.as_slice() {
        [a, b, ..] => (a.clone(), b.clone()),
        [a] => (Solver::highs(), a.clone()),
        [] => return,
    };
    let mut store = store_on(&from);
    let before = store.solution.as_ref().unwrap().objective_value;
    assert_eq!(store.solution.as_ref().unwrap().solver, from.kind().id());
    store.set_solver(to.clone()).unwrap();
    let after = store.solution.as_ref().unwrap();
    assert_eq!(after.solver, to.kind().id(), "the new solution comes from {}", to.label());
    assert!(close(after.objective_value, before), "{} found {}, {} found {before}", to.label(), after.objective_value, from.label());
    assert_eq!(store.problem.as_ref().unwrap().backend(), to.kind().id());
    // The same solver again changes nothing.
    let solution = store.solution.clone();
    store.set_solver(to).unwrap();
    assert!(std::sync::Arc::ptr_eq(solution.as_ref().unwrap(), store.solution.as_ref().unwrap()));
}

#[test]
fn the_bundle_names_its_solver_and_a_replay_on_another_says_so() {
    let solvers = installed();
    let (from, to) = match solvers.as_slice() {
        [a, b, ..] => (a.clone(), b.clone()),
        [a] => (a.clone(), Solver::highs()),
        [] => return,
    };
    let store = store_on(&from);
    let bundle = store.reproducibility_bundle().unwrap();
    assert_eq!(bundle["analysis"]["solver"], from.label().as_str());

    let mut other = store_on(&to);
    other.import_reproducibility_bundle(&bundle).unwrap();
    let replay = other.reproducibility_replay.as_ref().unwrap();
    assert_eq!(replay["solver"]["archived"], from.label().as_str());
    assert_eq!(replay["solver"]["replayed"], to.label().as_str());
    assert_eq!(replay["solver"]["same"], false);
    assert!(other.status().message.contains(&format!("solved with {}", from.label())), "{}", other.status().message);

    let mut same = store_on(&from);
    same.import_reproducibility_bundle(&bundle).unwrap();
    assert_eq!(same.reproducibility_replay.as_ref().unwrap()["solver"]["same"], true);
}
