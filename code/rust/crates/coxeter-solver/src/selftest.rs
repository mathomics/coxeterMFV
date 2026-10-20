//! A short battery of problems with known answers, run on any solver.
//!
//! It checks what the analyses rely on: optimal values, the dual convention
//! in both senses, inequality and ranged rows, infeasible and unbounded
//! problems, warm starts, interruption, binary MILPs, and FBA, pFBA, FVA and
//! loopless FBA on a small model. `coxeter --check-solvers` runs it on every
//! solver it finds, so an installation can be checked on the computer that
//! uses it; the tests run it too.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use coxeter_core::model::Model;

use crate::backends::Solver;
use crate::fba::{FluxProblem, FvaOptions, LooplessOptions, PfbaOptions};
use crate::lp::{LpProblem, LpSolution, LpStatus, SolveOptions, SparseColumns};
use crate::milp::{MilpOptions, solve_binary_milp};

/// One check and how it went.
#[derive(Debug, Clone, PartialEq)]
pub struct Check {
    pub name: &'static str,
    pub passed: bool,
    /// What was found, when it is not what was expected.
    pub detail: String,
}

/// One check: `Err` says what was found instead.
type CheckFn = fn(&Solver) -> Result<(), String>;

/// Run every check on `solver`.
pub fn self_test(solver: &Solver) -> Vec<Check> {
    let checks: [(&'static str, CheckFn); 13] = [
        ("Equality LP, maximised: optimum and duals", equality_max),
        ("Equality LP, minimised: optimum and duals", equality_min),
        ("Inequality rows: optimum and duals", inequality_rows),
        ("Inequality rows, minimised: duals", inequality_rows_min),
        ("Row bounded on both sides", ranged_row),
        ("Infeasible LP", infeasible),
        ("Unbounded LP", unbounded),
        ("Warm start after changing the objective", warm_start),
        ("Interruption", interruption),
        ("Binary MILP: infeasible", milp_infeasible),
        ("Binary MILP: optimum", milp_optimum),
        ("Binary MILP: unbounded relaxation", milp_unbounded),
        ("FBA, pFBA, FVA and loopless FBA", flux_analyses),
    ];
    checks
        .into_iter()
        .map(|(name, check)| match check(solver) {
            Ok(()) => Check { name, passed: true, detail: String::new() },
            Err(detail) => Check { name, passed: false, detail },
        })
        .collect()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6 * a.abs().max(b.abs()).max(1.0)
}

fn expect(what: &str, found: f64, expected: f64) -> Result<(), String> {
    if close(found, expected) { Ok(()) } else { Err(format!("{what}: {found}, expected {expected}")) }
}

fn expect_all(what: &str, found: Option<&Vec<f64>>, expected: &[f64]) -> Result<(), String> {
    let Some(found) = found else { return Err(format!("{what}: none reported")) };
    if found.len() == expected.len() && found.iter().zip(expected).all(|(a, b)| close(*a, *b)) {
        Ok(())
    } else {
        Err(format!("{what}: {found:?}, expected {expected:?}"))
    }
}

fn optimal(r: &LpSolution) -> Result<(), String> {
    if r.status == LpStatus::Optimal { Ok(()) } else { Err(format!("status {} ({})", r.status, r.message)) }
}

/// One row, two columns.
fn two(a0: f64, a1: f64) -> SparseColumns {
    let mut a = SparseColumns::new(1, 2);
    a.set(0, 0, a0);
    a.set(0, 1, a1);
    a
}

/// `x + 2y ≤ 4`, `3x + y ≤ 6`, `x, y ≥ 0`, with objective `cost`.
fn two_rows(cost: [f64; 2], maximize: bool) -> LpProblem {
    let mut a = SparseColumns::new(2, 2);
    a.set(0, 0, 1.0);
    a.set(1, 0, 3.0);
    a.set(0, 1, 2.0);
    a.set(1, 1, 1.0);
    LpProblem {
        a,
        row_lower: vec![f64::NEG_INFINITY; 2],
        row_upper: vec![4.0, 6.0],
        cost: cost.to_vec(),
        col_lower: vec![0.0; 2],
        col_upper: vec![f64::INFINITY; 2],
        maximize,
        integer: Vec::new(),
    }
}

// max x0 s.t. x0 + x1 = 1: dual 1, reduced costs [0 (basic), -1].
fn equality_max(solver: &Solver) -> Result<(), String> {
    let p = LpProblem::equality(two(1.0, 1.0), &[1.0], vec![1.0, 0.0], vec![0.0, 0.0], vec![10.0, 10.0], true);
    let r = solver.solve(&p, &SolveOptions::default());
    optimal(&r)?;
    expect("objective", r.objective, 1.0)?;
    expect_all("dual", r.duals.as_ref(), &[1.0])?;
    expect_all("reduced costs", r.reduced_costs.as_ref(), &[0.0, -1.0])
}

// min x0 + 2 x1 s.t. x0 + x1 = 1: optimum x0 = 1, dual 1, reduced costs [0, 1].
fn equality_min(solver: &Solver) -> Result<(), String> {
    let p = LpProblem::equality(two(1.0, 1.0), &[1.0], vec![1.0, 2.0], vec![0.0, 0.0], vec![10.0, 10.0], false);
    let r = solver.solve(&p, &SolveOptions::default());
    optimal(&r)?;
    expect("objective", r.objective, 1.0)?;
    expect_all("dual", r.duals.as_ref(), &[1.0])?;
    expect_all("reduced costs", r.reduced_costs.as_ref(), &[0.0, 1.0])
}

// max x + y: the rows meet at (1.6, 1.2); a unit more of either bound is
// worth 0.4 and 0.2.
fn inequality_rows(solver: &Solver) -> Result<(), String> {
    let r = solver.solve(&two_rows([1.0, 1.0], true), &SolveOptions::default());
    optimal(&r)?;
    expect("objective", r.objective, 2.8)?;
    expect_all("fluxes", r.x.as_ref(), &[1.6, 1.2])?;
    expect_all("duals", r.duals.as_ref(), &[0.4, 0.2])?;
    expect_all("row activity", r.row_activity.as_ref(), &[4.0, 6.0])
}

// min −x − y: the same point; a looser bound lowers the minimum.
fn inequality_rows_min(solver: &Solver) -> Result<(), String> {
    let r = solver.solve(&two_rows([-1.0, -1.0], false), &SolveOptions::default());
    optimal(&r)?;
    expect("objective", r.objective, -2.8)?;
    expect_all("duals", r.duals.as_ref(), &[-0.4, -0.2])
}

// max x s.t. 1 ≤ x + y ≤ 3, y ≥ 0.5: the upper bound holds, worth 1.
fn ranged_row(solver: &Solver) -> Result<(), String> {
    let p = LpProblem {
        a: two(1.0, 1.0),
        row_lower: vec![1.0],
        row_upper: vec![3.0],
        cost: vec![1.0, 0.0],
        col_lower: vec![0.0, 0.5],
        col_upper: vec![10.0, 10.0],
        maximize: true,
        integer: Vec::new(),
    };
    let r = solver.solve(&p, &SolveOptions::default());
    optimal(&r)?;
    expect("objective", r.objective, 2.5)?;
    expect_all("dual", r.duals.as_ref(), &[1.0])?;
    expect_all("row activity", r.row_activity.as_ref(), &[3.0])
}

fn infeasible(solver: &Solver) -> Result<(), String> {
    let p = LpProblem::equality(two(1.0, 1.0), &[5.0], vec![1.0, 0.0], vec![0.0, 0.0], vec![1.0, 1.0], true);
    let r = solver.solve(&p, &SolveOptions::default());
    if r.status == LpStatus::Infeasible { Ok(()) } else { Err(format!("status {} ({})", r.status, r.message)) }
}

fn unbounded(solver: &Solver) -> Result<(), String> {
    let p = LpProblem::equality(two(1.0, -1.0), &[0.0], vec![1.0, 0.0], vec![0.0, 0.0], vec![f64::INFINITY; 2], true);
    let r = solver.solve(&p, &SolveOptions::default());
    if r.status == LpStatus::Unbounded { Ok(()) } else { Err(format!("status {} ({})", r.status, r.message)) }
}

// One problem, three objectives, then the kept basis on a cleared solver.
fn warm_start(solver: &Solver) -> Result<(), String> {
    let p = LpProblem::equality(two(1.0, 1.0), &[4.0], vec![3.0, 2.0], vec![0.0, 0.0], vec![3.0, 3.0], true);
    let mut h = solver.instance()?;
    h.pass_problem(&p)?;
    expect("first objective", h.run().objective, 11.0)?;
    h.set_costs(&[0.0, 1.0]);
    expect("second objective", h.run().objective, 3.0)?;
    h.set_maximize(false);
    expect("minimum", h.run().objective, 1.0)?;
    let basis = h.basis().ok_or("no basis after an optimal solve")?;
    h.clear_solver();
    h.set_basis(&basis)?;
    expect("from the kept basis", h.run().objective, 1.0)
}

// A flag set before the solve stops it: unfinished, not a verdict.
fn interruption(solver: &Solver) -> Result<(), String> {
    let flag = Arc::new(AtomicBool::new(true));
    let r = solver.solve(&two_rows([1.0, 1.0], true), &SolveOptions { interrupt: Some(flag), presolve: false, ..Default::default() });
    if r.status == LpStatus::IterationLimit && r.model_status.is_interrupt() {
        Ok(())
    } else {
        Err(format!("status {} ({}, {})", r.status, r.model_status.name(), r.message))
    }
}

fn milp(solver: &Solver) -> MilpOptions {
    MilpOptions { node_limit: 100, time_limit_ms: 10_000.0, solver: solver.clone(), ..Default::default() }
}

// x + 2y = 2.5 has no binary solution.
fn milp_infeasible(solver: &Solver) -> Result<(), String> {
    let p = LpProblem::equality(two(1.0, 2.0), &[2.5], vec![1.0, 1.0], vec![0.0, 0.0], vec![1.0, 1.0], true);
    let r = solve_binary_milp(&p, &[0, 1], &milp(solver));
    if r.status == LpStatus::Infeasible { Ok(()) } else { Err(format!("status {} ({})", r.status, r.message)) }
}

// max x + y s.t. x + y + s = 1.5, x, y binary: 1, proven.
fn milp_optimum(solver: &Solver) -> Result<(), String> {
    let mut a = SparseColumns::new(1, 3);
    a.add(0, 0, 1.0);
    a.add(0, 1, 1.0);
    a.add(0, 2, 1.0);
    let p = LpProblem::equality(a, &[1.5], vec![1.0, 1.0, 0.0], vec![0.0; 3], vec![1.0, 1.0, f64::INFINITY], true);
    let r = solve_binary_milp(&p, &[0, 1], &milp(solver));
    if r.status != LpStatus::Optimal || !r.proven_optimal {
        return Err(format!("status {} ({})", r.status, r.message));
    }
    expect("objective", r.objective, 1.0)
}

// max x0 + y, x0 − x1 = 0, x free above, y binary: the relaxation is unbounded.
fn milp_unbounded(solver: &Solver) -> Result<(), String> {
    let mut a = SparseColumns::new(1, 3);
    a.add(0, 0, 1.0);
    a.add(0, 1, -1.0);
    let p = LpProblem::equality(a, &[0.0], vec![1.0, 0.0, 1.0], vec![0.0; 3], vec![f64::INFINITY, f64::INFINITY, 1.0], true);
    let r = solve_binary_milp(&p, &[2], &milp(solver));
    if r.status == LpStatus::Unbounded { Ok(()) } else { Err(format!("status {} ({})", r.status, r.message)) }
}

/// A uptake of `a` (at most 10) feeding growth through `b`, with a reversible
/// cycle between them that FBA may run and loopless FBA must not.
fn cycle_model() -> Result<Model, String> {
    let doc = serde_json::json!({
        "id": "selftest", "compartments": {"c": "cytosol"},
        "metabolites": [{"id": "a_c", "compartment": "c"}, {"id": "b_c", "compartment": "c"}],
        "reactions": [
            {"id": "EX_a", "metabolites": {"a_c": -1}, "lower_bound": -10, "upper_bound": 0},
            {"id": "R1", "metabolites": {"a_c": -1, "b_c": 1}, "lower_bound": -1000, "upper_bound": 1000},
            {"id": "R2", "metabolites": {"b_c": -1, "a_c": 1}, "lower_bound": -1000, "upper_bound": 1000},
            {"id": "BIOMASS", "metabolites": {"b_c": -1}, "lower_bound": 0, "upper_bound": 1000, "objective_coefficient": 1}
        ]
    });
    Model::from_value(&doc).map_err(|e| e.to_string())
}

fn flux_analyses(solver: &Solver) -> Result<(), String> {
    let model = cycle_model()?;
    let mut p = FluxProblem::new(&model);
    p.set_solver(solver.clone());
    let fba = p.optimize();
    if fba.status != LpStatus::Optimal {
        return Err(format!("FBA status {} ({})", fba.status, fba.message));
    }
    expect("FBA objective", fba.objective_value, 10.0)?;
    let pfba = p.parsimonious(PfbaOptions { fraction: 0.9, ..Default::default() });
    if pfba.status != LpStatus::Optimal {
        return Err(format!("pFBA status {} ({})", pfba.status, pfba.message));
    }
    expect("pFBA objective at 90 %", pfba.objective_value, 9.0)?;
    // Uptake 9 through one of R1, R2: no flux around the cycle.
    expect("pFBA total flux", pfba.total_flux.unwrap_or(f64::NAN), 27.0)?;
    let fva = p.variability(&["EX_a", "BIOMASS"], &FvaOptions { threads: Some(1), ..Default::default() });
    let ex = fva.get("EX_a").ok_or("FVA left out EX_a")?;
    expect("FVA min of EX_a", ex.min, -10.0)?;
    expect("FVA max of EX_a", ex.max, -10.0)?;
    // At 90 % the objective is held between 80 % and 100 % (the JS formulation).
    let fva90 = p.variability(&["EX_a"], &FvaOptions { fraction: 0.9, threads: Some(2), ..Default::default() });
    let ex = fva90.get("EX_a").ok_or("FVA at 90 % left out EX_a")?;
    expect("FVA 90 % min of EX_a", ex.min, -10.0)?;
    expect("FVA 90 % max of EX_a", ex.max, -8.0)?;
    let loopless = p.loopless(&LooplessOptions { time_limit_ms: 10_000.0, ..Default::default() });
    let info = loopless.loopless.as_ref().ok_or("no loopless details")?;
    if loopless.status != LpStatus::Optimal || !info.proven_optimal {
        return Err(format!("loopless status {} ({})", loopless.status, loopless.message));
    }
    expect("loopless objective", loopless.objective_value, 10.0)?;
    // R1 and R2 running the same way (a → b → a, or back) is the cycle.
    let flux = |id: &str| loopless.fluxes.get(id).copied().unwrap_or(f64::NAN);
    if flux("R1") * flux("R2") > 1e-9 {
        return Err(format!("loopless left flux around the cycle: R1 {}, R2 {}", flux("R1"), flux("R2")));
    }
    expect("loopless net flux from a to b", flux("R1") - flux("R2"), 10.0)
}
