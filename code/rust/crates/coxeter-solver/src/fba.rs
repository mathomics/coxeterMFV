//! Flux balance analysis.
//!
//! Ports `src/solver/fba.js`. Everything works against a plain [`Model`]: the
//! community merger in `coxeter_core::community` produces a normal COBRA model
//! with namespaced ids, so single-organism and multi-organism problems take
//! exactly the same path.
//!
//! The JS module sits on two LP backends (its bundled simplex and GLPK). This
//! port runs on HiGHS, or on the Gurobi or CPLEX the user has, behind
//! [`crate::lp`] ([`FluxProblem::set_solver`]). The formulations (FBA, the
//! split-variable pFBA, the thermodynamic loopless MILP and FVA with the
//! objective held in a range) are the JS ones, row for row.
//!
//! Flux variability runs in parallel: one solver instance per worker, each
//! loaded once with the FVA problem and re-solved with a different objective
//! per reaction, starting every solve from the basis of the primary optimum.
//! Because every solve starts from that same state, the ranges are identical
//! whatever the number of threads and the order the reactions are taken in.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use coxeter_core::community::{COMMUNITY_GROWTH, LINK_COMPARTMENT};
use coxeter_core::model::{Model, ReactionKind};
use coxeter_core::js;
use indexmap::IndexMap;

use crate::backends::{Solver, SolverKind};
use crate::lp::{Basis, LpInstance, LpProblem, LpSolution, LpStatus, SolveOptions, SparseColumns};
use crate::milp::{MilpOptions, solve_binary_milp};

/// Replaces ±Infinity in bounds: a large finite number the simplex is
/// comfortable with.
pub const BIG: f64 = 1e6;

/// `finite(v, fallback)`: NaN takes the fallback, ±Infinity becomes ±[`BIG`].
fn finite(v: f64, fallback: f64) -> f64 {
    if v.is_nan() {
        fallback
    } else if v == f64::INFINITY {
        BIG
    } else if v == f64::NEG_INFINITY {
        -BIG
    } else {
        v
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum FbaError {
    #[error("Unknown reaction {0}")]
    UnknownReaction(String),
}

/// What the dual values of a result mean, as the JS records it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DualAnalysis {
    pub available: bool,
    pub source: Option<String>,
    pub interpretation: Option<String>,
    pub reason: Option<String>,
}

/// Extra fields of a loopless solve.
#[derive(Debug, Clone, PartialEq)]
pub struct LooplessInfo {
    pub nodes: u64,
    pub binary_variables: usize,
    pub loopless_internal_reactions: usize,
    pub proven_optimal: bool,
}

/// Extra fields of a CycleFreeFlux solve: the total internal flux, Σ |v_j|
/// over the internal reactions, after the cycles were removed and before.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CycleFreeInfo {
    pub internal_flux: f64,
    pub internal_flux_before: f64,
}

/// The result of [`FluxProblem::optimize`], [`FluxProblem::parsimonious`],
/// [`FluxProblem::cycle_free`] or [`FluxProblem::loopless`]: the JS result object.
#[derive(Debug, Clone)]
pub struct FluxResult {
    pub status: LpStatus,
    pub objective_value: f64,
    /// Reaction id → flux; empty unless optimal (or a loopless incumbent).
    pub fluxes: IndexMap<String, f64>,
    /// Metabolite id → dual of its mass balance.
    pub shadow_prices: IndexMap<String, f64>,
    /// Reaction id → `c_j − a_jᵀ y`; zero for basic reactions.
    pub reduced_costs: IndexMap<String, f64>,
    /// `RC_i / max_j |RC_j|` (V3.6.1), zero when every RC is zero.
    pub scaled_reduced_costs: IndexMap<String, f64>,
    /// `max_j |RC_j|`, or `None` where duals are not reported (loopless).
    pub reduced_cost_scale: Option<f64>,
    pub dual_analysis: DualAnalysis,
    pub iterations: u64,
    pub message: String,
    /// pFBA: the minimised total absolute flux.
    pub total_flux: Option<f64>,
    /// Loopless FBA only.
    pub loopless: Option<LooplessInfo>,
    /// CycleFreeFlux only.
    pub cycle_free: Option<CycleFreeInfo>,
    /// The solver that found it.
    pub solver: SolverKind,
}

/// The minimum and maximum of one reaction's flux.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FvaRange {
    /// `NaN` when that LP did not solve.
    pub min: f64,
    pub max: f64,
}

/// One finished reaction of a flux variability run.
#[derive(Debug, Clone, Copy)]
pub struct FvaProgress<'a> {
    pub done: usize,
    pub total: usize,
    pub reaction_id: &'a str,
    pub range: FvaRange,
}

/// Options of [`FluxProblem::variability`].
#[derive(Clone)]
pub struct FvaOptions<'a> {
    /// Hold the objective at this fraction of its optimum (1.0).
    pub fraction: f64,
    /// Worker threads; `None` uses every core, `Some(1)` runs serially on the
    /// calling thread.
    pub threads: Option<usize>,
    /// Called after every reaction (from a worker thread, one call at a time,
    /// with `done` counting up).
    pub on_progress: Option<&'a (dyn Fn(&FvaProgress<'_>) + Sync)>,
    /// Set to stop: reactions not yet finished are left out of the result,
    /// and an LP in progress is interrupted.
    pub cancel: Option<Arc<AtomicBool>>,
}

impl Default for FvaOptions<'_> {
    fn default() -> Self {
        FvaOptions { fraction: 1.0, threads: None, on_progress: None, cancel: None }
    }
}

/// Options of [`FluxProblem::parsimonious`].
#[derive(Debug, Clone, Copy)]
pub struct PfbaOptions {
    /// Fraction of the primary optimum to fix before minimising total flux
    /// (`pfbaBiomassPercent / 100` in the app).
    pub fraction: f64,
    pub maximize: bool,
}

impl Default for PfbaOptions {
    fn default() -> Self {
        PfbaOptions { fraction: 1.0, maximize: true }
    }
}

/// Options of [`FluxProblem::cycle_free`].
#[derive(Debug, Clone)]
pub struct CycleFreeOptions {
    pub maximize: bool,
    /// An FBA optimum to free of its cycles in place of the one the solver
    /// returns (reaction id → flux).
    pub fluxes: Option<IndexMap<String, f64>>,
}

impl Default for CycleFreeOptions {
    fn default() -> Self {
        CycleFreeOptions { maximize: true, fluxes: None }
    }
}

/// Options of [`FluxProblem::loopless`]. The defaults are the JS ones.
#[derive(Debug, Clone)]
pub struct LooplessOptions {
    pub maximize: bool,
    pub epsilon: f64,
    pub thermodynamic_big_m: f64,
    pub node_limit: u64,
    pub time_limit_ms: f64,
    pub interrupt: Option<Arc<AtomicBool>>,
}

impl Default for LooplessOptions {
    fn default() -> Self {
        LooplessOptions {
            maximize: true,
            epsilon: 1.0,
            thermodynamic_big_m: 1000.0,
            node_limit: 20_000,
            time_limit_ms: 8_000.0,
            interrupt: None,
        }
    }
}

/// Snapshot of all bounds, for undo.
#[derive(Debug, Clone, PartialEq)]
pub struct BoundsSnapshot {
    pub lb: Vec<f64>,
    pub ub: Vec<f64>,
}

/// `normalizePfbaPercent` of the app's store: the pFBA biomass target in
/// percent, clamped to 0–100. Anything that is not a finite number is 100.
pub fn normalize_pfba_percent(value: f64) -> f64 {
    if !value.is_finite() {
        return 100.0;
    }
    js::min(100.0, js::max(0.0, value))
}

/// The pFBA `fraction` for a biomass-target percentage (V2.2).
pub fn pfba_fraction(percent: f64) -> f64 {
    normalize_pfba_percent(percent) / 100.0
}

/// A reusable LP view of a model. Bounds and objective can be tweaked between
/// solves without rebuilding the stoichiometric matrix.
#[derive(Debug, Clone)]
pub struct FluxProblem {
    pub met_ids: Vec<String>,
    pub rxn_ids: Vec<String>,
    met_index: HashMap<String, usize>,
    rxn_index: HashMap<String, usize>,
    a: SparseColumns,
    lb: Vec<f64>,
    ub: Vec<f64>,
    c: Vec<f64>,
    b: Vec<f64>,
    objective: IndexMap<String, f64>,
    /// `isLooplessInternalReaction` of each reaction, as the model said.
    internal: Vec<bool>,
    /// The solver every analysis of this problem runs on (HiGHS by default).
    solver: Solver,
}

/// The objective `fba.js` falls back on: the model's objective reactions, else
/// its biomass reaction, else nothing.
pub fn default_objective(model: &Model) -> IndexMap<String, f64> {
    let objective = model.objective_reactions();
    if !objective.is_empty() {
        return objective.into_iter().collect();
    }
    match model.biomass_reaction_id() {
        Some(id) => [(id, 1.0)].into_iter().collect(),
        None => IndexMap::new(),
    }
}

impl FluxProblem {
    /// Build the LP of `model` with its default objective.
    pub fn new(model: &Model) -> FluxProblem {
        FluxProblem::with_objective(model, None)
    }

    /// Build the LP with `objective` (reaction id → coefficient), or the model's
    /// default objective when `None`. An empty map is an empty objective.
    pub fn with_objective(model: &Model, objective: Option<&IndexMap<String, f64>>) -> FluxProblem {
        let met_ids: Vec<String> = model.metabolites.keys().cloned().collect();
        let rxn_ids: Vec<String> = model.reactions.keys().cloned().collect();
        let met_index: HashMap<String, usize> = met_ids.iter().enumerate().map(|(i, id)| (id.clone(), i)).collect();
        let rxn_index: HashMap<String, usize> = rxn_ids.iter().enumerate().map(|(i, id)| (id.clone(), i)).collect();
        let (m, n) = (met_ids.len(), rxn_ids.len());
        let mut a = SparseColumns::new(m, n);
        let mut lb = vec![0.0; n];
        let mut ub = vec![0.0; n];
        let mut internal = vec![false; n];
        for (j, rxn) in model.reactions.values().enumerate() {
            for (mid, coef) in &rxn.metabolites {
                // `if (!coef) continue`: zero and NaN carry nothing.
                if *coef == 0.0 || coef.is_nan() {
                    continue;
                }
                if let Some(&i) = met_index.get(mid) {
                    a.add(i, j, *coef);
                }
            }
            lb[j] = finite(rxn.lower_bound, -BIG);
            ub[j] = finite(rxn.upper_bound, BIG);
            internal[j] = rxn.id != COMMUNITY_GROWTH
                && !rxn.compartments.iter().any(|c| c == LINK_COMPARTMENT)
                && matches!(rxn.kind, ReactionKind::Metabolic | ReactionKind::Transport);
        }
        let mut problem = FluxProblem {
            met_ids,
            rxn_ids,
            met_index,
            rxn_index,
            a,
            lb,
            ub,
            c: vec![0.0; n],
            b: vec![0.0; m],
            objective: IndexMap::new(),
            internal,
            solver: Solver::highs(),
        };
        match objective {
            Some(o) => problem.set_objective(o),
            None => problem.set_objective(&default_objective(model)),
        };
        problem
    }

    /// Replace the objective (reaction id → coefficient). Unknown ids are kept
    /// in [`FluxProblem::objective`] but contribute nothing. NaN counts as 0.
    pub fn set_objective(&mut self, coefficients: &IndexMap<String, f64>) -> &mut Self {
        self.c.fill(0.0);
        self.objective = coefficients.clone();
        for (rid, coef) in coefficients {
            if let Some(&j) = self.rxn_index.get(rid) {
                self.c[j] = if coef.is_nan() { 0.0 } else { *coef };
            }
        }
        self
    }

    /// The objective as last set.
    pub fn objective(&self) -> &IndexMap<String, f64> {
        &self.objective
    }

    /// Objective coefficient of every reaction, in `rxn_ids` order.
    pub fn objective_vector(&self) -> &[f64] {
        &self.c
    }

    /// Change a reaction's bounds. `None` leaves that side as it is. NaN keeps
    /// the old value, ±Infinity becomes ±[`BIG`], and a lower bound above the
    /// upper one is pulled down to it.
    pub fn set_bounds(&mut self, reaction_id: &str, lower: Option<f64>, upper: Option<f64>) -> Result<&mut Self, FbaError> {
        let j = *self.rxn_index.get(reaction_id).ok_or_else(|| FbaError::UnknownReaction(reaction_id.into()))?;
        if let Some(l) = lower {
            self.lb[j] = finite(l, self.lb[j]);
        }
        if let Some(u) = upper {
            self.ub[j] = finite(u, self.ub[j]);
        }
        if self.lb[j] > self.ub[j] {
            self.lb[j] = self.ub[j];
        }
        Ok(self)
    }

    /// `(lower, upper)`, or `None` for an unknown reaction.
    pub fn get_bounds(&self, reaction_id: &str) -> Option<(f64, f64)> {
        self.rxn_index.get(reaction_id).map(|&j| (self.lb[j], self.ub[j]))
    }

    /// Every reaction's bounds, to put back later with
    /// [`FluxProblem::restore_bounds`].
    pub fn snapshot_bounds(&self) -> BoundsSnapshot {
        BoundsSnapshot { lb: self.lb.clone(), ub: self.ub.clone() }
    }

    /// Put back the bounds a [`FluxProblem::snapshot_bounds`] took.
    pub fn restore_bounds(&mut self, snap: &BoundsSnapshot) -> &mut Self {
        self.lb.copy_from_slice(&snap.lb);
        self.ub.copy_from_slice(&snap.ub);
        self
    }

    /// The LP column of a reaction, by reaction id.
    pub fn reaction_index(&self, id: &str) -> Option<usize> {
        self.rxn_index.get(id).copied()
    }

    /// The LP row of a metabolite's mass balance, by metabolite id.
    pub fn metabolite_index(&self, id: &str) -> Option<usize> {
        self.met_index.get(id).copied()
    }

    /// The stoichiometric matrix (metabolites × reactions).
    pub fn stoichiometry(&self) -> &SparseColumns {
        &self.a
    }

    /// Which backend solves this problem (`highs`, `gurobi`, `cplex`). The
    /// JS chooses between its simplex and GLPK.
    pub fn backend(&self) -> &'static str {
        self.solver.kind().id()
    }

    /// The solver every analysis of this problem runs on.
    pub fn solver(&self) -> &Solver {
        &self.solver
    }

    /// Run every analysis of this problem on `solver`.
    pub fn set_solver(&mut self, solver: Solver) -> &mut Self {
        self.solver = solver;
        self
    }

    /// The FBA LP: `S v = 0`, `lb ≤ v ≤ ub`, `max/min cᵀv`.
    pub fn lp(&self, maximize: bool) -> LpProblem {
        LpProblem::equality(self.a.clone(), &self.b, self.c.clone(), self.lb.clone(), self.ub.clone(), maximize)
    }

    /// Maximise the current objective.
    pub fn optimize(&self) -> FluxResult {
        self.optimize_with(true)
    }

    /// Solve the current problem in either sense.
    pub fn optimize_with(&self, maximize: bool) -> FluxResult {
        let res = self.solver.solve(&self.lp(maximize), &SolveOptions::default());
        self.wrap(res)
    }

    fn wrap(&self, res: LpSolution) -> FluxResult {
        let optimal = res.status == LpStatus::Optimal;
        let mut fluxes = IndexMap::new();
        if optimal {
            if let Some(x) = &res.x {
                for (j, rid) in self.rxn_ids.iter().enumerate() {
                    fluxes.insert(rid.clone(), x[j]);
                }
            }
        }
        let mut shadow_prices = IndexMap::new();
        if let (true, Some(y)) = (optimal, &res.duals) {
            for (i, mid) in self.met_ids.iter().enumerate() {
                shadow_prices.insert(mid.clone(), y[i]);
            }
        }
        let mut reduced_costs = IndexMap::new();
        if let (true, Some(rc)) = (optimal, &res.reduced_costs) {
            for (j, rid) in self.rxn_ids.iter().enumerate() {
                reduced_costs.insert(rid.clone(), rc[j]);
            }
        }
        let (scale, scaled) = scale_reduced_costs(&reduced_costs);
        FluxResult {
            status: res.status,
            objective_value: res.objective,
            fluxes,
            shadow_prices,
            reduced_costs,
            scaled_reduced_costs: scaled,
            reduced_cost_scale: Some(scale),
            dual_analysis: if optimal {
                DualAnalysis {
                    available: true,
                    source: Some("optimal LP basis".into()),
                    interpretation: Some("primary objective".into()),
                    reason: None,
                }
            } else {
                DualAnalysis::default()
            },
            iterations: res.iterations,
            message: res.message,
            total_flux: None,
            loopless: None,
            cycle_free: None,
            solver: self.solver.kind(),
        }
    }

    /// Exact loopless FBA using the standard thermodynamic MILP formulation
    /// (ll-FBA, Schellenberger, Lewis & Palsson 2011).
    ///
    /// Every internal reaction that can carry flux receives a binary direction
    /// variable a: a = 1 allows forward flux and requires ΔG ≤ -ε, a = 0 allows
    /// reverse flux and requires ΔG ≥ ε. Irreversible reactions get one too,
    /// because a fixed sign of ΔG would make any cycle of them infeasible even
    /// when the cycle carries no flux; with the binary, a = 0 (or a = 1 for a
    /// reverse-only reaction) switches the flux off instead. Chemical potential
    /// variables carry the ΔG. Boundary exchanges, sinks, biomass and the
    /// virtual community growth link are excluded from the thermodynamic
    /// constraints.
    ///
    /// This is substantially more expensive than LP FBA because it is a MILP.
    pub fn loopless(&self, opts: &LooplessOptions) -> FluxResult {
        let built = self.loopless_problem(opts.maximize, opts.epsilon, opts.thermodynamic_big_m);
        let result = solve_binary_milp(
            &built.problem,
            &built.binary_indices,
            &MilpOptions {
                node_limit: opts.node_limit,
                time_limit_ms: opts.time_limit_ms,
                interrupt: opts.interrupt.clone(),
                solver: self.solver.clone(),
                ..Default::default()
            },
        );
        let mut fluxes = IndexMap::new();
        let mut objective_value = f64::NAN;
        if let Some(x) = &result.x {
            for (j, rid) in self.rxn_ids.iter().enumerate() {
                let v = x[j];
                fluxes.insert(rid.clone(), if v.abs() < 1e-9 { 0.0 } else { v });
            }
            objective_value = 0.0;
            for j in 0..self.rxn_ids.len() {
                objective_value += self.c[j] * x[j];
            }
        }
        FluxResult {
            status: result.status,
            objective_value,
            fluxes,
            shadow_prices: IndexMap::new(),
            reduced_costs: IndexMap::new(),
            scaled_reduced_costs: IndexMap::new(),
            reduced_cost_scale: None,
            dual_analysis: DualAnalysis {
                available: false,
                reason: Some("Dual values are not reported for the loopless MILP solution.".into()),
                ..Default::default()
            },
            iterations: result.iterations,
            message: result.message,
            total_flux: None,
            loopless: Some(LooplessInfo {
                nodes: result.nodes,
                binary_variables: built.binary_indices.len(),
                loopless_internal_reactions: built.internal_count,
                proven_optimal: result.proven_optimal,
            }),
            cycle_free: None,
            solver: self.solver.kind(),
        }
    }

    /// `_looplessProblem`: fluxes, then one potential per metabolite, then one
    /// binary per internal reaction that can carry flux.
    ///
    /// The inequalities are ranged rows, where the JS adds a slack column to
    /// each: on the slack form of a five-species community HiGHS can search for
    /// minutes, on this one it finds the optimum at the root node.
    pub fn loopless_problem(&self, maximize: bool, epsilon: f64, big_m: f64) -> LooplessProblem {
        let n = self.rxn_ids.len();
        let m = self.met_ids.len();
        let mut internal: Vec<(usize, f64, f64)> = Vec::new();
        for j in 0..n {
            if !self.internal[j] {
                continue;
            }
            let (lower, upper) = (self.lb[j], self.ub[j]);
            if lower.abs() < 1e-12 && upper.abs() < 1e-12 {
                continue;
            }
            if upper <= 0.0 && lower >= 0.0 {
                continue;
            }
            internal.push((j, lower, upper));
        }
        // Two ΔG rows per reaction, plus a flux row for each direction it can run.
        let inequality_rows: usize = internal
            .iter()
            .map(|&(_, lower, upper)| 2 + usize::from(upper > 0.0) + usize::from(lower < 0.0))
            .sum();
        let rows = m + inequality_rows;
        let flux_offset = 0;
        let mu_offset = n;
        let binary_offset = mu_offset + m;
        let cols = binary_offset + internal.len();

        let mut a = SparseColumns::new(rows, cols);
        let mut row_lower = vec![0.0; rows];
        let mut row_upper = vec![0.0; rows];
        let mut c = vec![0.0; cols];
        let mut lb = vec![0.0; cols];
        let mut ub = vec![0.0; cols];
        for j in 0..n {
            for &(i, v) in self.a.column(j) {
                a.add(i, flux_offset + j, v);
            }
            lb[flux_offset + j] = self.lb[j];
            ub[flux_offset + j] = self.ub[j];
            c[flux_offset + j] = self.c[j];
        }
        for i in 0..m {
            lb[mu_offset + i] = -big_m;
            ub[mu_offset + i] = big_m;
        }
        let mut binary_indices = Vec::with_capacity(internal.len());
        for index in 0..internal.len() {
            let col = binary_offset + index;
            binary_indices.push(col);
            lb[col] = 0.0;
            ub[col] = 1.0;
        }

        let mut row = m;
        let add_potential = |a: &mut SparseColumns, target_row: usize, reaction: usize| {
            for &(met_row, v) in self.a.column(reaction) {
                a.add(target_row, mu_offset + met_row, v);
            }
        };
        let mut at_most = |row: &mut usize, bound: f64| {
            row_lower[*row] = f64::NEG_INFINITY;
            row_upper[*row] = bound;
            *row += 1;
        };
        for (&(j, lower, upper), &y) in internal.iter().zip(&binary_indices) {
            // v <= U y; for a reverse-only reaction U = 0 and the bound already says it.
            if upper > 0.0 {
                a.add(row, flux_offset + j, 1.0);
                a.add(row, y, -upper);
                at_most(&mut row, 0.0);
            }
            // v >= L (1-y), as -v - L y <= -L; for a forward-only reaction L = 0.
            if lower < 0.0 {
                a.add(row, flux_offset + j, -1.0);
                a.add(row, y, -lower);
                at_most(&mut row, -lower);
            }
            // ΔG <= M - (M+eps)y
            add_potential(&mut a, row, j);
            a.add(row, y, big_m + epsilon);
            at_most(&mut row, big_m);
            // ΔG >= eps - (M+eps)y, as -ΔG - (M+eps)y <= -eps
            for &(met_row, v) in self.a.column(j) {
                a.add(row, mu_offset + met_row, -v);
            }
            a.add(row, y, -(big_m + epsilon));
            at_most(&mut row, -epsilon);
        }
        LooplessProblem {
            problem: LpProblem {
                a,
                row_lower,
                row_upper,
                cost: c,
                col_lower: lb,
                col_upper: ub,
                maximize,
                integer: Vec::new(),
            },
            binary_indices,
            internal_count: internal.len(),
        }
    }

    /// Parsimonious FBA: keep the optimal objective (within `fraction`) and then
    /// minimise total flux. This often removes gratuitous cycle flux, but it does
    /// not enforce thermodynamic looplessness. Use [`FluxProblem::loopless`] for that.
    pub fn parsimonious(&self, opts: PfbaOptions) -> FluxResult {
        let first = self.optimize_with(opts.maximize);
        if first.status != LpStatus::Optimal {
            return first;
        }
        let target = first.objective_value * if opts.maximize { opts.fraction } else { 2.0 - opts.fraction };
        let n = self.rxn_ids.len();
        let m = self.met_ids.len();

        // Split every flux into forward/reverse halves so |v| becomes linear.
        let mut a = SparseColumns::new(m + 1, 2 * n);
        let mut lb = vec![0.0; 2 * n];
        let mut ub = vec![0.0; 2 * n];
        let mut c = vec![0.0; 2 * n];
        let mut b = vec![0.0; m + 1];
        for j in 0..n {
            for &(i, v) in self.a.column(j) {
                a.add(i, j, v);
                a.add(i, n + j, -v);
            }
            lb[j] = self.lb[j].max(0.0);
            ub[j] = self.ub[j].max(0.0);
            lb[n + j] = (-self.ub[j]).max(0.0);
            ub[n + j] = (-self.lb[j]).max(0.0);
            c[j] = 1.0;
            c[n + j] = 1.0;
            if self.c[j] != 0.0 {
                a.add(m, j, self.c[j]);
                a.add(m, n + j, -self.c[j]);
            }
        }
        b[m] = target;

        // The objective row is an equality at `target`. With fraction < 1 that
        // is a deliberate choice (hit exactly that share of the optimum).
        let res = self.solver.solve(&LpProblem::equality(a, &b, c, lb, ub, false), &SolveOptions::default());
        if res.status != LpStatus::Optimal {
            return first;
        }
        let x = res.x.as_deref().unwrap_or(&[]);
        let mut fluxes = IndexMap::new();
        for (j, rid) in self.rxn_ids.iter().enumerate() {
            let v = x[j] - x[n + j];
            fluxes.insert(rid.clone(), if v.abs() < 1e-9 { 0.0 } else { v });
        }
        let mut objective_value = 0.0;
        for (j, rid) in self.rxn_ids.iter().enumerate() {
            objective_value += self.c[j] * fluxes[rid];
        }
        FluxResult {
            status: LpStatus::Optimal,
            objective_value,
            fluxes,
            shadow_prices: first.shadow_prices,
            reduced_costs: first.reduced_costs,
            scaled_reduced_costs: first.scaled_reduced_costs,
            reduced_cost_scale: first.reduced_cost_scale,
            dual_analysis: DualAnalysis {
                available: true,
                source: Some("primary FBA optimum".into()),
                interpretation: Some("primary objective before parsimonious secondary minimization".into()),
                reason: None,
            },
            iterations: first.iterations + res.iterations,
            message: String::new(),
            total_flux: Some(res.objective),
            loopless: None,
            cycle_free: None,
            solver: self.solver.kind(),
        }
    }

    /// CycleFreeFlux (Desouki, Jarre, Gelius-Dietrich & Lercher 2015): the FBA
    /// optimum v* with its internal cycles removed, by one LP instead of the
    /// loopless MILP.
    ///
    /// Every boundary reaction (what loopless leaves out: exchanges, sinks,
    /// biomass, the community growth link) and every reaction in the objective
    /// is held at v*. Every internal reaction stays between 0 and v*_j, inside
    /// its own bounds, and the LP minimises the internal flux Σ sign(v*_j) v_j,
    /// which is Σ |v_j| there. A cycle carries flux without changing any
    /// exchange, so the least total flux runs none, and the objective and the
    /// exchanges are those of v*.
    ///
    /// The result is an optimum of the FBA LP too, so the duals of the FBA
    /// solved first are reported for it. `opts.fluxes` is an FBA optimum to
    /// free of its cycles in place of the one the solver returns.
    pub fn cycle_free(&self, opts: &CycleFreeOptions) -> FluxResult {
        let first = self.optimize_with(opts.maximize);
        if first.status != LpStatus::Optimal {
            return first;
        }
        let start = opts.fluxes.as_ref().unwrap_or(&first.fluxes);
        let n = self.rxn_ids.len();
        let mut lb = vec![0.0; n];
        let mut ub = vec![0.0; n];
        let mut c = vec![0.0; n];
        let mut internal = vec![false; n];
        let mut internal_flux_before = 0.0;
        for (j, rid) in self.rxn_ids.iter().enumerate() {
            let v = start.get(rid).copied().unwrap_or(0.0);
            if self.c[j] == 0.0 && self.internal[j] {
                internal[j] = true;
                internal_flux_before += v.abs();
                lb[j] = js::max(self.lb[j], js::min(0.0, v));
                ub[j] = js::min(self.ub[j], js::max(0.0, v));
                c[j] = if v > 0.0 { 1.0 } else if v < 0.0 { -1.0 } else { 0.0 };
            } else {
                lb[j] = v;
                ub[j] = v;
            }
        }
        let res = self.solver.solve(&LpProblem::equality(self.a.clone(), &self.b, c, lb, ub, false), &SolveOptions::default());
        if res.status != LpStatus::Optimal {
            let status = res.status;
            return FluxResult {
                message: format!("CycleFreeFlux could not remove the cycles ({}); the FBA solution is shown.", status.as_str()),
                ..first
            };
        }
        let x = res.x.as_deref().unwrap_or(&[]);
        let mut fluxes = IndexMap::new();
        let mut internal_flux = 0.0;
        for (j, rid) in self.rxn_ids.iter().enumerate() {
            let v = if x[j].abs() < 1e-9 { 0.0 } else { x[j] };
            fluxes.insert(rid.clone(), v);
            if internal[j] {
                internal_flux += v.abs();
            }
        }
        let mut objective_value = 0.0;
        for (j, rid) in self.rxn_ids.iter().enumerate() {
            objective_value += self.c[j] * fluxes[rid];
        }
        FluxResult {
            status: LpStatus::Optimal,
            objective_value,
            fluxes,
            shadow_prices: first.shadow_prices,
            reduced_costs: first.reduced_costs,
            scaled_reduced_costs: first.scaled_reduced_costs,
            reduced_cost_scale: first.reduced_cost_scale,
            dual_analysis: DualAnalysis {
                available: true,
                source: Some("primary FBA optimum".into()),
                interpretation: Some("primary objective before the cycles were removed".into()),
                reason: None,
            },
            iterations: first.iterations + res.iterations,
            message: String::new(),
            total_flux: None,
            loopless: None,
            cycle_free: Some(CycleFreeInfo { internal_flux, internal_flux_before }),
            solver: self.solver.kind(),
        }
    }

    /// The FVA problem: the FBA rows plus the objective row
    /// `cᵀv + s = fraction·opt`, `s ∈ [min(0, fraction·opt − opt), max(0, opt − fraction·opt)]`,
    /// written as the range `cᵀv ∈ [b − s_hi, b − s_lo]`.
    ///
    /// Note what that range is: for `opt ≥ 0` it is `[(2·fraction − 1)·opt, opt]`,
    /// wider than the `[fraction·opt, opt]` the JS comment describes. The port
    /// keeps the JS formulation, so FVA at 90 % holds the objective between
    /// 80 % and 100 % of its optimum, exactly as the app does.
    pub fn variability_problem(&self, optimum: f64, fraction: f64) -> LpProblem {
        let m = self.met_ids.len();
        let n = self.rxn_ids.len();
        let mut a = SparseColumns::new(m + 1, n);
        for j in 0..n {
            for &(i, v) in self.a.column(j) {
                a.add(i, j, v);
            }
            if self.c[j] != 0.0 {
                a.add(m, j, self.c[j]);
            }
        }
        let b = optimum * fraction;
        let slack_low = js::min(0.0, optimum * fraction - optimum);
        let slack_high = js::max(0.0, optimum - optimum * fraction);
        let mut row_lower = vec![0.0; m + 1];
        let mut row_upper = vec![0.0; m + 1];
        row_lower[m] = b - slack_high;
        row_upper[m] = b - slack_low;
        LpProblem {
            a,
            row_lower,
            row_upper,
            cost: vec![0.0; n],
            col_lower: self.lb.clone(),
            col_upper: self.ub.clone(),
            maximize: true,
            integer: Vec::new(),
        }
    }

    /// Flux variability for a subset of reactions while holding the objective
    /// at `fraction` of its optimum. Unknown ids are skipped, and the result keeps
    /// the order of `reaction_ids`. Empty when the primary problem has no
    /// optimum.
    ///
    /// Parallel unless `opts.threads == Some(1)`. The result is the same either way.
    pub fn variability<S: AsRef<str>>(&self, reaction_ids: &[S], opts: &FvaOptions<'_>) -> IndexMap<String, FvaRange> {
        let base = self.optimize();
        if base.status != LpStatus::Optimal {
            return IndexMap::new();
        }
        let ids: Vec<(String, usize)> = reaction_ids
            .iter()
            .filter_map(|r| self.rxn_index.get(r.as_ref()).map(|&j| (r.as_ref().to_string(), j)))
            .collect();
        let problem = self.variability_problem(base.objective_value, opts.fraction);

        // The basis every solve starts from: the optimum of the primary
        // objective on the FVA problem itself, so it is feasible for every
        // reaction's LP.
        let start = {
            let Ok(mut h) = self.solver.instance() else { return IndexMap::new() };
            if h.pass_problem(&problem).is_err() {
                return IndexMap::new();
            }
            h.set_costs(&self.c);
            let r = h.run();
            if r.status == LpStatus::Optimal { h.basis() } else { None }
        };

        let total = ids.len();
        let threads = opts.threads.unwrap_or_else(rayon::current_num_threads).max(1).min(total.max(1));
        let next = AtomicUsize::new(0);
        let done = Mutex::new(0usize);
        let results: Mutex<Vec<Option<FvaRange>>> = Mutex::new(vec![None; total]);
        let cancelled = || opts.cancel.as_ref().is_some_and(|f| f.load(Ordering::Relaxed));

        let work = || {
            let mut worker = match FvaWorker::new(&self.solver, &problem, start.as_ref(), opts.cancel.clone()) {
                Some(w) => w,
                None => return,
            };
            loop {
                if cancelled() {
                    return;
                }
                let k = next.fetch_add(1, Ordering::Relaxed);
                if k >= total {
                    return;
                }
                let (rid, j) = &ids[k];
                let Some(range) = worker.range(*j) else {
                    return; // interrupted: leave this reaction out
                };
                results.lock().unwrap()[k] = Some(range);
                let mut d = done.lock().unwrap();
                *d += 1;
                if let Some(cb) = opts.on_progress {
                    cb(&FvaProgress { done: *d, total, reaction_id: rid, range });
                }
            }
        };

        if threads == 1 {
            work();
        } else {
            match rayon::ThreadPoolBuilder::new().num_threads(threads).build() {
                Ok(pool) => pool.scope(|s| {
                    for _ in 0..threads {
                        s.spawn(|_| work());
                    }
                }),
                Err(_) => work(),
            }
        }

        let results = results.into_inner().unwrap();
        ids.iter()
            .zip(results)
            .filter_map(|((rid, _), r)| r.map(|range| (rid.clone(), range)))
            .collect()
    }

    /// FVA over every reaction.
    pub fn variability_all(&self, opts: &FvaOptions<'_>) -> IndexMap<String, FvaRange> {
        self.variability(&self.rxn_ids, opts)
    }

    /// Objective value with one reaction forced to zero.
    pub fn knockout(&mut self, reaction_id: &str) -> Result<FluxResult, FbaError> {
        let snap = self.snapshot_bounds();
        self.set_bounds(reaction_id, Some(0.0), Some(0.0))?;
        let res = self.optimize();
        self.restore_bounds(&snap);
        Ok(res)
    }
}

/// `reducedCostScale` and `scaledReducedCosts` of `_wrap`.
fn scale_reduced_costs(reduced: &IndexMap<String, f64>) -> (f64, IndexMap<String, f64>) {
    let scale = reduced.values().fold(0.0, |m, v| js::max(m, v.abs()));
    let scaled = reduced.iter().map(|(rid, v)| (rid.clone(), if scale > 0.0 { v / scale } else { 0.0 })).collect();
    (scale, scaled)
}

/// The loopless MILP and where its parts are.
#[derive(Debug, Clone)]
pub struct LooplessProblem {
    pub problem: LpProblem,
    pub binary_indices: Vec<usize>,
    pub internal_count: usize,
}

/// One solver instance holding the FVA problem.
struct FvaWorker {
    instance: Box<dyn LpInstance>,
    start: Option<Basis>,
    current: Option<usize>,
}

impl FvaWorker {
    fn new(solver: &Solver, problem: &LpProblem, start: Option<&Basis>, cancel: Option<Arc<AtomicBool>>) -> Option<FvaWorker> {
        let mut instance = solver.instance().ok()?;
        instance.pass_problem(problem).ok()?;
        instance.set_interrupt(cancel);
        Some(FvaWorker { instance, start: start.cloned(), current: None })
    }

    /// Both ends of reaction `j`, or `None` if a solve was interrupted.
    fn range(&mut self, j: usize) -> Option<FvaRange> {
        if let Some(prev) = self.current.take() {
            self.instance.set_cost(prev, 0.0);
        }
        self.instance.set_cost(j, 1.0);
        self.current = Some(j);
        let max = self.solve(true)?;
        let min = self.solve(false)?;
        Some(FvaRange { min, max })
    }

    fn solve(&mut self, maximize: bool) -> Option<f64> {
        // Every solve starts from the same state (the primary optimum's basis
        // and a freshly seeded solver), so its answer does not depend on which
        // reactions this worker happened to solve before.
        self.instance.clear_solver();
        if let Some(b) = &self.start {
            let _ = self.instance.set_basis(b);
        }
        self.instance.set_maximize(maximize);
        let r = self.instance.run();
        if r.model_status.is_interrupt() {
            return None;
        }
        Some(if r.status == LpStatus::Optimal { r.objective } else { f64::NAN })
    }
}

/// One-shot FBA.
pub fn fba(model: &Model) -> FluxResult {
    FluxProblem::new(model).optimize()
}

/// One-shot pFBA.
pub fn pfba(model: &Model, opts: PfbaOptions) -> FluxResult {
    FluxProblem::new(model).parsimonious(opts)
}

/// One-shot CycleFreeFlux.
pub fn cycle_free_flux(model: &Model, opts: &CycleFreeOptions) -> FluxResult {
    FluxProblem::new(model).cycle_free(opts)
}

/// One-shot loopless FBA.
pub fn loopless_fba(model: &Model, opts: &LooplessOptions) -> FluxResult {
    FluxProblem::new(model).loopless(opts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn model(doc: serde_json::Value) -> Model {
        Model::from_value(&doc).unwrap()
    }

    // test/dual-analysis.test.mjs
    #[test]
    fn duals_map_to_metabolites_and_reduced_costs_to_reactions() {
        let m = model(json!({"id": "dual-test", "metabolites": [{"id": "m_c", "name": "m", "compartment": "c"}], "reactions": [
            {"id": "SOURCE", "name": "source", "metabolites": {"m_c": 1}, "lower_bound": 0, "upper_bound": 1},
            {"id": "BIOMASS", "name": "biomass", "metabolites": {"m_c": -1}, "lower_bound": 0, "upper_bound": 1000, "objective_coefficient": 1}
        ], "genes": [], "compartments": {"c": "cytosol"}}));
        let r = FluxProblem::new(&m).optimize();
        assert_eq!(r.status, LpStatus::Optimal);
        assert!(r.dual_analysis.available);
        assert!(r.shadow_prices.contains_key("m_c"));
        assert!(r.reduced_costs.contains_key("SOURCE") && r.reduced_costs.contains_key("BIOMASS"));
        assert_eq!(r.scaled_reduced_costs.len(), r.reduced_costs.len());
        assert!(r.scaled_reduced_costs.values().all(|v| *v >= -1.0 - 1e-12 && *v <= 1.0 + 1e-12));
        let max_abs = r.reduced_costs.values().fold(0.0f64, |m, v| m.max(v.abs()));
        assert_eq!(r.reduced_cost_scale, Some(max_abs));
        if max_abs > 0.0 {
            let top = r.scaled_reduced_costs.values().fold(0.0f64, |m, v| m.max(v.abs()));
            assert!((top - 1.0).abs() < 1e-12);
        }
    }

    // test/simulation-mode.test.mjs: the real-solver case.
    #[test]
    fn pfba_fixes_the_requested_fraction_of_the_optimum() {
        let m = model(json!({"id": "pfba_fraction_demo", "compartments": {"c": "cytosol"},
            "metabolites": [{"id": "a_c", "compartment": "c"}], "reactions": [
            {"id": "EX_a", "metabolites": {"a_c": -1}, "lower_bound": -10, "upper_bound": 0},
            {"id": "BIOMASS", "metabolites": {"a_c": -1}, "lower_bound": 0, "upper_bound": 1000, "objective_coefficient": 1}
        ]}));
        let p = FluxProblem::new(&m);
        let fba = p.optimize();
        let pfba90 = p.parsimonious(PfbaOptions { fraction: pfba_fraction(90.0), ..Default::default() });
        assert_eq!(fba.status, LpStatus::Optimal);
        assert!((fba.objective_value - 10.0).abs() < 1e-8);
        assert_eq!(pfba90.status, LpStatus::Optimal);
        assert!((pfba90.objective_value - 9.0).abs() < 1e-8);
    }

    #[test]
    fn pfba_percent_is_clamped_like_the_store_does() {
        assert_eq!(normalize_pfba_percent(120.0), 100.0);
        assert_eq!(normalize_pfba_percent(-5.0), 0.0);
        assert_eq!(normalize_pfba_percent(87.5), 87.5);
        assert_eq!(normalize_pfba_percent(f64::NAN), 100.0);
        assert_eq!(pfba_fraction(95.0), 0.95);
        assert_eq!(pfba_fraction(87.5), 0.875);
    }

    #[test]
    fn bounds_snapshot_and_unknown_reactions() {
        let m = model(json!({"metabolites": [{"id": "a_c"}], "reactions": [
            {"id": "EX_a", "metabolites": {"a_c": -1}, "lower_bound": -10, "upper_bound": 0},
            {"id": "BIOMASS", "metabolites": {"a_c": -1}, "lower_bound": 0, "upper_bound": 1000, "objective_coefficient": 1}
        ]}));
        let mut p = FluxProblem::new(&m);
        let snap = p.snapshot_bounds();
        p.set_bounds("EX_a", Some(f64::NEG_INFINITY), Some(f64::NAN)).unwrap();
        assert_eq!(p.get_bounds("EX_a"), Some((-BIG, 0.0)));
        p.set_bounds("EX_a", Some(5.0), None).unwrap();
        assert_eq!(p.get_bounds("EX_a"), Some((0.0, 0.0)), "lower above upper is pulled down");
        p.restore_bounds(&snap);
        assert_eq!(p.get_bounds("EX_a"), Some((-10.0, 0.0)));
        assert_eq!(p.set_bounds("nope", None, None).unwrap_err().to_string(), "Unknown reaction nope");
        assert_eq!(p.get_bounds("nope"), None);
        let ko = p.knockout("EX_a").unwrap();
        assert!(ko.objective_value.abs() < 1e-9);
        assert!((p.optimize().objective_value - 10.0).abs() < 1e-9, "knockout leaked");
    }
}
