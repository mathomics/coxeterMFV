//! Binary MILP on HiGHS.
//!
//! Ports `src/solver/milp.js`. The JS module is a small depth-first
//! branch-and-bound over its own simplex. Here HiGHS' MIP solver does the
//! search, and this module keeps the JS contract: the problem is an LP in the
//! `A x = b` shape plus a list of binary columns, the safeguards are a node
//! limit and a time limit, and the result says whether optimality was proven.
//!
//! As in the JS: an exhausted search with an incumbent returns that incumbent
//! with status `iteration_limit` and `proven_optimal = false`, so a caller can
//! show it without labelling an unproven point an exact optimum.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::lp::{Highs, LpProblem, LpStatus, SolveOptions, solve_lp};

/// `solveBinaryMILP` options. The defaults are the JS ones.
#[derive(Debug, Clone)]
pub struct MilpOptions {
    /// Most branch-and-bound nodes to explore (20 000).
    pub node_limit: u64,
    /// Wall-clock limit in milliseconds (8 000).
    pub time_limit_ms: f64,
    /// How far from 0 or 1 a binary may be and still count as integral (1e-7).
    pub integrality_tolerance: f64,
    /// A node whose bound cannot beat the incumbent by more than this is pruned
    /// (1e-8): HiGHS' absolute gap. The relative gap is zero, as in the JS.
    pub objective_tolerance: f64,
    /// Set from any thread to stop the search.
    pub interrupt: Option<Arc<AtomicBool>>,
}

impl Default for MilpOptions {
    fn default() -> Self {
        MilpOptions {
            node_limit: 20_000,
            time_limit_ms: 8_000.0,
            integrality_tolerance: 1e-7,
            objective_tolerance: 1e-8,
            interrupt: None,
        }
    }
}

/// What `solveBinaryMILP` returns.
#[derive(Debug, Clone)]
pub struct MilpResult {
    pub status: LpStatus,
    /// `NaN` when there is no point.
    pub objective: f64,
    /// The optimum, or the incumbent when a limit stopped the search.
    pub x: Option<Vec<f64>>,
    pub iterations: u64,
    pub nodes: u64,
    pub proven_optimal: bool,
    pub incumbent_objective: Option<f64>,
    pub message: String,
}

/// Solve `problem` with the columns in `binary` restricted to 0 or 1.
pub fn solve_binary_milp(problem: &LpProblem, binary: &[usize], options: &MilpOptions) -> MilpResult {
    let mut mip = problem.clone();
    mip.integer = vec![false; mip.cols()];
    for &j in binary {
        if j >= mip.cols() {
            continue;
        }
        mip.integer[j] = true;
        // Branching in the JS fixes a binary at 0 or at 1 within its bounds.
        mip.col_lower[j] = mip.col_lower[j].max(0.0);
        mip.col_upper[j] = mip.col_upper[j].min(1.0);
    }
    if mip.col_lower.iter().zip(&mip.col_upper).any(|(lo, hi)| lo > hi) {
        return MilpResult {
            status: LpStatus::Infeasible,
            objective: f64::NAN,
            x: None,
            iterations: 0,
            nodes: 0,
            proven_optimal: true,
            incumbent_objective: None,
            message: "Loopless MILP has no feasible solution.".into(),
        };
    }
    let solve_options = SolveOptions {
        time_limit: Some(options.time_limit_ms / 1000.0),
        node_limit: Some(options.node_limit),
        integrality_tolerance: Some(options.integrality_tolerance),
        mip_abs_gap: Some(options.objective_tolerance),
        mip_rel_gap: Some(0.0),
        interrupt: options.interrupt.clone(),
        ..Default::default()
    };
    let mut highs = Highs::new();
    highs.apply(&solve_options);
    if let Err(message) = highs.pass_problem(&mip) {
        return MilpResult {
            status: LpStatus::IterationLimit,
            objective: f64::NAN,
            x: None,
            iterations: 0,
            nodes: 0,
            proven_optimal: false,
            incumbent_objective: None,
            message,
        };
    }
    let res = highs.run();
    let nodes = res.nodes;

    match res.status {
        LpStatus::Optimal => MilpResult {
            status: LpStatus::Optimal,
            objective: res.objective,
            incumbent_objective: Some(res.objective),
            x: res.x,
            iterations: res.iterations,
            nodes,
            proven_optimal: true,
            message: String::new(),
        },
        LpStatus::Infeasible => infeasible(nodes),
        _ => {
            // HiGHS may answer "unbounded" or "unbounded or infeasible" for the
            // MILP as a whole. The JS answers for the root relaxation: an
            // unbounded relaxation is reported as such, an infeasible one means
            // the MILP has no solution.
            let ambiguous = res.status == LpStatus::Unbounded || res.model_status.is_unbounded_or_infeasible();
            if ambiguous {
                let mut relaxation = mip.clone();
                relaxation.integer.clear();
                let root = solve_lp(&relaxation, &SolveOptions { time_limit: solve_options.time_limit, ..Default::default() });
                match root.status {
                    LpStatus::Unbounded => {
                        return MilpResult {
                            status: LpStatus::Unbounded,
                            objective: f64::NAN,
                            x: None,
                            iterations: root.iterations,
                            nodes,
                            proven_optimal: false,
                            incumbent_objective: None,
                            message: "The LP relaxation of the MILP is unbounded.".into(),
                        };
                    }
                    LpStatus::Infeasible => return infeasible(nodes),
                    _ => {}
                }
            }
            match res.x {
                Some(x) => MilpResult {
                    status: LpStatus::IterationLimit,
                    objective: res.objective,
                    incumbent_objective: Some(res.objective),
                    x: Some(x),
                    iterations: res.iterations,
                    nodes,
                    proven_optimal: false,
                    message: format!(
                        "A loopless solution was found, but optimality was not proven before the limit ({nodes} nodes)."
                    ),
                },
                None => MilpResult {
                    status: LpStatus::IterationLimit,
                    objective: f64::NAN,
                    x: None,
                    iterations: res.iterations,
                    nodes,
                    proven_optimal: false,
                    incumbent_objective: None,
                    message: format!("Loopless MILP hit the search limit ({nodes} nodes)."),
                },
            }
        }
    }
}

fn infeasible(nodes: u64) -> MilpResult {
    MilpResult {
        status: LpStatus::Infeasible,
        objective: f64::NAN,
        x: None,
        iterations: 0,
        nodes,
        proven_optimal: true,
        incumbent_objective: None,
        message: "Loopless MILP has no feasible solution.".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lp::SparseColumns;

    // test/milp.test.mjs
    #[test]
    fn branch_and_bound_enforces_integrality() {
        // max x + y, x + 2y = 2.5, x/y binary: no binary point satisfies it.
        let mut a = SparseColumns::new(1, 2);
        a.add(0, 0, 1.0);
        a.add(0, 1, 2.0);
        let p = LpProblem::equality(a, &[2.5], vec![1.0, 1.0], vec![0.0, 0.0], vec![1.0, 1.0], true);
        let r = solve_binary_milp(&p, &[0, 1], &MilpOptions { node_limit: 100, time_limit_ms: 1000.0, ..Default::default() });
        assert_eq!(r.status, LpStatus::Infeasible);
    }

    #[test]
    fn returns_an_optimal_integer_solution() {
        // x + y + s = 1.5, s >= 0. max x + y with binaries -> optimum 1.
        let mut a = SparseColumns::new(1, 3);
        a.add(0, 0, 1.0);
        a.add(0, 1, 1.0);
        a.add(0, 2, 1.0);
        let p = LpProblem::equality(a, &[1.5], vec![1.0, 1.0, 0.0], vec![0.0; 3], vec![1.0, 1.0, f64::INFINITY], true);
        let r = solve_binary_milp(&p, &[0, 1], &MilpOptions { node_limit: 100, time_limit_ms: 1000.0, ..Default::default() });
        assert_eq!(r.status, LpStatus::Optimal);
        assert!((r.objective - 1.0).abs() < 1e-9);
        assert!(r.proven_optimal);
        let x = r.x.unwrap();
        for v in &x[..2] {
            assert!((v.round() - v).abs() < 1e-7 && (v.round() == 0.0 || v.round() == 1.0));
        }
    }

    #[test]
    fn an_unbounded_relaxation_is_reported_as_such() {
        // max x0 + y, x0 - x1 = 0, x free above, y binary.
        let mut a = SparseColumns::new(1, 3);
        a.add(0, 0, 1.0);
        a.add(0, 1, -1.0);
        let p = LpProblem::equality(a, &[0.0], vec![1.0, 0.0, 1.0], vec![0.0; 3], vec![f64::INFINITY, f64::INFINITY, 1.0], true);
        let r = solve_binary_milp(&p, &[2], &MilpOptions::default());
        assert_eq!(r.status, LpStatus::Unbounded);
        assert_eq!(r.message, "The LP relaxation of the MILP is unbounded.");
    }
}
