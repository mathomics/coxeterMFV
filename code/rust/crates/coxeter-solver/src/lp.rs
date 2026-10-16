//! A small, safe LP/MILP interface over HiGHS.
//!
//! Ports the interface `src/solver/simplex.js` and `src/solver/glpk.js`
//! implement in the JavaScript app (the problem shape, the four status codes
//! and their meaning) onto HiGHS. The bundled dense simplex of the JS app is
//! not ported as a solver: HiGHS is linked statically and always available,
//! so there is no fallback to choose.
//!
//! This is the only module that touches FFI. Everything else in the crate
//! builds an [`LpProblem`] and reads an [`LpSolution`], or keeps a [`Highs`]
//! instance alive to re-solve it after changing the objective or the bounds
//! (HiGHS then starts from the previous basis).
//!
//! Status semantics, as glpk.js maps GLPK's:
//!
//! * [`LpStatus::Optimal`]: an optimum was proven.
//! * [`LpStatus::Infeasible`]: the LP was proven to have no solution.
//! * [`LpStatus::Unbounded`]: the objective was proven unbounded.
//! * [`LpStatus::IterationLimit`]: anything else, meaning a time, iteration or node
//!   limit, an interrupt, or a solver that gave up. A feasible point does not
//!   prove that an objective reached its maximum, and an infeasible current
//!   basis is not proof that the LP has no solution, so neither is ever read as
//!   a verdict. Callers (SteadyCom in particular) must not infer impossibility
//!   from this status.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::fmt;
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use highs_sys as ffi;

/// `LP_OPTIMAL` of simplex.js.
pub const LP_OPTIMAL: &str = "optimal";
/// `LP_INFEASIBLE` of simplex.js.
pub const LP_INFEASIBLE: &str = "infeasible";
/// `LP_UNBOUNDED` of simplex.js.
pub const LP_UNBOUNDED: &str = "unbounded";
/// `LP_ITERATION_LIMIT` of simplex.js: the solve did not finish with a verdict.
pub const LP_ITERATION_LIMIT: &str = "iteration_limit";

/// The outcome of an LP or MILP, with the meaning glpk.js gives it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LpStatus {
    Optimal,
    Infeasible,
    Unbounded,
    /// Unfinished: a limit, an interrupt or an undecided solver.
    IterationLimit,
}

impl LpStatus {
    /// The JS status string (`'optimal'`, `'infeasible'`, `'unbounded'`,
    /// `'iteration_limit'`).
    pub fn as_str(self) -> &'static str {
        match self {
            LpStatus::Optimal => LP_OPTIMAL,
            LpStatus::Infeasible => LP_INFEASIBLE,
            LpStatus::Unbounded => LP_UNBOUNDED,
            LpStatus::IterationLimit => LP_ITERATION_LIMIT,
        }
    }

    /// The status a JS status string names (see [`LpStatus::as_str`]).
    pub fn parse(s: &str) -> Option<LpStatus> {
        match s {
            LP_OPTIMAL => Some(LpStatus::Optimal),
            LP_INFEASIBLE => Some(LpStatus::Infeasible),
            LP_UNBOUNDED => Some(LpStatus::Unbounded),
            LP_ITERATION_LIMIT => Some(LpStatus::IterationLimit),
            _ => None,
        }
    }

    pub fn is_optimal(self) -> bool {
        self == LpStatus::Optimal
    }
}

impl fmt::Display for LpStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// HiGHS' own model status, kept for diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModelStatus(pub i32);

impl ModelStatus {
    /// A readable name of the status, for messages.
    pub fn name(self) -> &'static str {
        match self.0 {
            ffi::MODEL_STATUS_NOTSET => "not set",
            ffi::MODEL_STATUS_LOAD_ERROR => "load error",
            ffi::MODEL_STATUS_MODEL_ERROR => "model error",
            ffi::MODEL_STATUS_PRESOLVE_ERROR => "presolve error",
            ffi::MODEL_STATUS_SOLVE_ERROR => "solve error",
            ffi::MODEL_STATUS_POSTSOLVE_ERROR => "postsolve error",
            ffi::MODEL_STATUS_MODEL_EMPTY => "empty",
            ffi::MODEL_STATUS_OPTIMAL => "optimal",
            ffi::MODEL_STATUS_INFEASIBLE => "infeasible",
            ffi::MODEL_STATUS_UNBOUNDED_OR_INFEASIBLE => "unbounded or infeasible",
            ffi::MODEL_STATUS_UNBOUNDED => "unbounded",
            ffi::MODEL_STATUS_OBJECTIVE_BOUND => "objective bound",
            ffi::MODEL_STATUS_OBJECTIVE_TARGET => "objective target",
            ffi::MODEL_STATUS_REACHED_TIME_LIMIT => "time limit",
            ffi::MODEL_STATUS_REACHED_ITERATION_LIMIT => "iteration limit",
            ffi::MODEL_STATUS_UNKNOWN => "unknown",
            ffi::MODEL_STATUS_REACHED_SOLUTION_LIMIT => "solution limit",
            ffi::MODEL_STATUS_REACHED_INTERRUPT => "interrupted",
            ffi::MODEL_STATUS_REACHED_MEMORY_LIMIT => "memory limit",
            _ => "unrecognised",
        }
    }

    /// The status the rest of the crate reads. See the module notes.
    pub fn lp_status(self) -> LpStatus {
        match self.0 {
            ffi::MODEL_STATUS_OPTIMAL | ffi::MODEL_STATUS_MODEL_EMPTY => LpStatus::Optimal,
            ffi::MODEL_STATUS_INFEASIBLE => LpStatus::Infeasible,
            ffi::MODEL_STATUS_UNBOUNDED => LpStatus::Unbounded,
            _ => LpStatus::IterationLimit,
        }
    }

    /// Whether the solve stopped at its time limit.
    pub fn is_time_limit(self) -> bool {
        self.0 == ffi::MODEL_STATUS_REACHED_TIME_LIMIT
    }

    /// Whether the solve stopped because it was interrupted.
    pub fn is_interrupt(self) -> bool {
        self.0 == ffi::MODEL_STATUS_REACHED_INTERRUPT
    }

    /// Presolve's "unbounded or infeasible": not a verdict on either.
    pub fn is_unbounded_or_infeasible(self) -> bool {
        self.0 == ffi::MODEL_STATUS_UNBOUNDED_OR_INFEASIBLE
    }
}

/// Column-major sparse matrix builder: `SparseColumns` of simplex.js.
///
/// Each column is a list of `(row, value)`. `add` accumulates into an existing
/// entry, `set` replaces it, and a zero value is never stored.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SparseColumns {
    pub rows: usize,
    data: Vec<Vec<(usize, f64)>>,
}

impl SparseColumns {
    /// An empty matrix of `rows` rows and `cols` columns.
    pub fn new(rows: usize, cols: usize) -> SparseColumns {
        SparseColumns { rows, data: vec![Vec::new(); cols] }
    }

    pub fn cols(&self) -> usize {
        self.data.len()
    }

    /// Set entry `(row, col)`, replacing what is there. A zero is ignored, so
    /// it never clears an entry.
    pub fn set(&mut self, row: usize, col: usize, value: f64) {
        if value == 0.0 {
            return;
        }
        let c = &mut self.data[col];
        if let Some(entry) = c.iter_mut().find(|(r, _)| *r == row) {
            entry.1 = value;
            return;
        }
        c.push((row, value));
    }

    /// Add `value` to entry `(row, col)`. A zero is ignored.
    pub fn add(&mut self, row: usize, col: usize, value: f64) {
        if value == 0.0 {
            return;
        }
        let c = &mut self.data[col];
        if let Some(entry) = c.iter_mut().find(|(r, _)| *r == row) {
            entry.1 += value;
            return;
        }
        c.push((row, value));
    }

    /// Append to a column without looking for an existing entry for that row.
    /// Only for builders that know the row is new to the column.
    pub fn push(&mut self, row: usize, col: usize, value: f64) {
        if value != 0.0 {
            self.data[col].push((row, value));
        }
    }

    /// The stored `(row, value)` entries of column `j`.
    pub fn column(&self, j: usize) -> &[(usize, f64)] {
        &self.data[j]
    }

    /// Append a column; returns its index.
    pub fn push_column(&mut self, entries: Vec<(usize, f64)>) -> usize {
        self.data.push(entries);
        self.data.len() - 1
    }

    /// Add an empty row; returns its index.
    pub fn push_row(&mut self) -> usize {
        self.rows += 1;
        self.rows - 1
    }

    pub fn nnz(&self) -> usize {
        self.data.iter().map(Vec::len).sum()
    }

    /// `Aᵀ y` for column `j`.
    pub fn dot(&self, j: usize, y: &[f64]) -> f64 {
        self.data[j].iter().fold(0.0, |s, (r, v)| s + v * y[*r])
    }
}

/// `max` or `min  cᵀx  s.t.  row_lower ≤ A x ≤ row_upper,  col_lower ≤ x ≤ col_upper`,
/// with some columns optionally integer.
///
/// Infinite bounds are `f64::INFINITY` / `f64::NEG_INFINITY`.
#[derive(Debug, Clone, PartialEq)]
pub struct LpProblem {
    pub a: SparseColumns,
    pub row_lower: Vec<f64>,
    pub row_upper: Vec<f64>,
    pub cost: Vec<f64>,
    pub col_lower: Vec<f64>,
    pub col_upper: Vec<f64>,
    pub maximize: bool,
    /// Integer columns; empty means a pure LP.
    pub integer: Vec<bool>,
}

impl LpProblem {
    /// The shape `solveLP` takes: `A x = b`, `lb ≤ x ≤ ub`.
    pub fn equality(a: SparseColumns, b: &[f64], cost: Vec<f64>, lb: Vec<f64>, ub: Vec<f64>, maximize: bool) -> LpProblem {
        LpProblem {
            row_lower: b.to_vec(),
            row_upper: b.to_vec(),
            a,
            cost,
            col_lower: lb,
            col_upper: ub,
            maximize,
            integer: Vec::new(),
        }
    }

    pub fn rows(&self) -> usize {
        self.a.rows
    }

    pub fn cols(&self) -> usize {
        self.a.cols()
    }

    /// `Σ c_j x_j`, as simplex.js reports its objective.
    pub fn objective_of(&self, x: &[f64]) -> f64 {
        self.cost.iter().zip(x).fold(0.0, |s, (c, v)| if *c == 0.0 { s } else { s + c * v })
    }

    fn check(&self) -> Result<(), (LpStatus, String)> {
        let (m, n) = (self.rows(), self.cols());
        if self.row_lower.len() != m || self.row_upper.len() != m {
            return Err((LpStatus::IterationLimit, format!("row bounds for {} rows, the matrix has {m}", self.row_lower.len())));
        }
        if self.cost.len() != n || self.col_lower.len() != n || self.col_upper.len() != n {
            return Err((LpStatus::IterationLimit, format!("column data does not match the {n} matrix columns")));
        }
        if !self.integer.is_empty() && self.integer.len() != n {
            return Err((LpStatus::IterationLimit, "integrality does not match the matrix columns".into()));
        }
        for j in 0..n {
            let (lo, hi) = (self.col_lower[j], self.col_upper[j]);
            if lo.is_nan() || hi.is_nan() || self.cost[j].is_nan() {
                return Err((LpStatus::IterationLimit, format!("column {j} has a NaN bound or cost")));
            }
            if lo > hi {
                return Err((LpStatus::Infeasible, format!("Reversed bounds on column {j}: [{lo}, {hi}]")));
            }
            for (i, v) in self.a.column(j) {
                if !v.is_finite() || *i >= m {
                    return Err((LpStatus::IterationLimit, format!("column {j} has an invalid coefficient")));
                }
            }
        }
        for i in 0..m {
            let (lo, hi) = (self.row_lower[i], self.row_upper[i]);
            if lo.is_nan() || hi.is_nan() {
                return Err((LpStatus::IterationLimit, format!("row {i} has a NaN bound")));
            }
            if lo > hi {
                return Err((LpStatus::Infeasible, format!("Reversed bounds on row {i}: [{lo}, {hi}]")));
            }
        }
        Ok(())
    }
}

/// A HiGHS option value for [`Highs::set_option`].
#[derive(Debug, Clone, Copy)]
pub enum OptionValue<'a> {
    Bool(bool),
    Int(i32),
    Double(f64),
    Str(&'a str),
}

/// Basis statuses of every column and row, as HiGHS reports them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Basis {
    col: Vec<ffi::HighsInt>,
    row: Vec<ffi::HighsInt>,
}

impl Basis {
    pub fn is_basic_col(&self, j: usize) -> bool {
        self.col.get(j) == Some(&ffi::kHighsBasisStatusBasic)
    }
    pub fn is_basic_row(&self, i: usize) -> bool {
        self.row.get(i) == Some(&ffi::kHighsBasisStatusBasic)
    }
    pub fn cols(&self) -> usize {
        self.col.len()
    }
    pub fn rows(&self) -> usize {
        self.row.len()
    }
}

/// What a solve returns.
///
/// Duals follow simplex.js: `duals[i]` is the change in the objective per unit
/// of row `i`'s bound, and `reduced_costs[j] = c_j − a_jᵀ·duals`, both in the
/// problem's own objective sense, and basic columns report zero.
#[derive(Debug, Clone)]
pub struct LpSolution {
    pub status: LpStatus,
    /// `Σ c_j x_j` when there is a solution, `NaN` otherwise.
    pub objective: f64,
    /// Primal values: present when optimal, and for a MILP that stopped at a
    /// limit with an incumbent.
    pub x: Option<Vec<f64>>,
    pub duals: Option<Vec<f64>>,
    pub reduced_costs: Option<Vec<f64>>,
    /// `A x` for every row, when there is a solution.
    pub row_activity: Option<Vec<f64>>,
    pub basis: Option<Basis>,
    pub iterations: u64,
    /// Branch-and-bound nodes (MILP only).
    pub nodes: u64,
    pub message: String,
    pub model_status: ModelStatus,
}

impl LpSolution {
    /// A result with no point and no duals, only `status` and a message saying
    /// why.
    pub fn failed(status: LpStatus, message: impl Into<String>) -> LpSolution {
        LpSolution {
            status,
            objective: f64::NAN,
            x: None,
            duals: None,
            reduced_costs: None,
            row_activity: None,
            basis: None,
            iterations: 0,
            nodes: 0,
            message: message.into(),
            model_status: ModelStatus(ffi::MODEL_STATUS_NOTSET),
        }
    }
}

/// Settings of one solve.
#[derive(Debug, Clone)]
pub struct SolveOptions {
    /// Wall-clock limit in seconds. A degenerate LP can cycle and never come
    /// back. A limit turns that into a bounded failure the caller can report.
    pub time_limit: Option<f64>,
    /// HiGHS' presolve (on by default, as GLPK's in glpk.js).
    pub presolve: bool,
    /// MILP: most branch-and-bound nodes to explore.
    pub node_limit: Option<u64>,
    /// MILP: how far from an integer a value may be and still count as one.
    pub integrality_tolerance: Option<f64>,
    /// MILP: stop when the incumbent is within this absolute gap of the bound.
    pub mip_abs_gap: Option<f64>,
    /// MILP: and within this relative gap.
    pub mip_rel_gap: Option<f64>,
    /// Set to `true` from any thread to interrupt the solve. The solve then
    /// returns [`LpStatus::IterationLimit`] with an "interrupted" message.
    pub interrupt: Option<Arc<AtomicBool>>,
    /// Start from this basis (of an earlier solve of a problem with the same
    /// rows and columns) instead of from scratch. Ignored if it does not fit.
    pub basis: Option<Basis>,
}

impl Default for SolveOptions {
    fn default() -> Self {
        SolveOptions {
            time_limit: None,
            presolve: true,
            node_limit: None,
            integrality_tolerance: None,
            mip_abs_gap: None,
            mip_rel_gap: None,
            interrupt: None,
            basis: None,
        }
    }
}

/// Something that solves an [`LpProblem`]. HiGHS ([`HighsBackend`]) is the one
/// the app uses, and tests substitute their own to exercise status handling.
pub trait LpBackend: Sync {
    fn solve(&self, problem: &LpProblem, options: &SolveOptions) -> LpSolution;
}

/// The HiGHS backend: a fresh instance per solve.
#[derive(Debug, Clone, Copy, Default)]
pub struct HighsBackend;

impl LpBackend for HighsBackend {
    fn solve(&self, problem: &LpProblem, options: &SolveOptions) -> LpSolution {
        solve_lp(problem, options)
    }
}

/// Solve one problem on a fresh HiGHS instance.
pub fn solve_lp(problem: &LpProblem, options: &SolveOptions) -> LpSolution {
    if let Err((status, message)) = problem.check() {
        return LpSolution::failed(status, message);
    }
    let mut highs = Highs::new();
    highs.apply(options);
    if let Err(message) = highs.pass_problem(problem) {
        return LpSolution::failed(LpStatus::IterationLimit, message);
    }
    if let Some(basis) = &options.basis {
        // A basis that does not fit is simply not used.
        let _ = highs.set_basis(basis);
    }
    highs.run()
}

/// A HiGHS instance holding one problem, for repeated solves.
///
/// After [`Highs::pass_problem`], the objective, the sense and the bounds can be
/// changed and the problem solved again: HiGHS starts from the basis it ended
/// on, which is what makes flux variability cheap (one problem, many
/// objectives). [`Highs::set_basis`] restarts from a basis kept earlier.
///
/// An instance can move between threads but is not shared by them.
pub struct Highs {
    ptr: NonNull<c_void>,
    interrupt: Option<Arc<AtomicBool>>,
    cols: usize,
    rows: usize,
    cost: Vec<f64>,
    maximize: bool,
    mip: bool,
    presolve: bool,
}

// SAFETY: a Highs object is a plain C++ object with no thread affinity; HiGHS'
// task scheduler is thread-local and set up again on whichever thread runs it.
// The instance is never used from two threads at once (no Sync).
unsafe impl Send for Highs {}

impl Default for Highs {
    fn default() -> Self {
        Highs::new()
    }
}

impl Drop for Highs {
    fn drop(&mut self) {
        // SAFETY: ptr came from Highs_create and is destroyed once.
        unsafe { ffi::Highs_destroy(self.ptr.as_ptr()) }
    }
}

/// Called by HiGHS between simplex/IPM iterations and MIP nodes.
unsafe extern "C" fn interrupt_callback(
    _callback_type: c_int,
    _message: *const c_char,
    _data_out: *const ffi::HighsCallbackDataOut,
    data_in: *mut ffi::HighsCallbackDataIn,
    user_data: *mut c_void,
) {
    if data_in.is_null() || user_data.is_null() {
        return;
    }
    // SAFETY: user_data is the AtomicBool inside the Arc the Highs instance
    // keeps alive for as long as the callback is registered.
    let flag = unsafe { &*(user_data as *const AtomicBool) };
    if flag.load(Ordering::Relaxed) {
        // SAFETY: HiGHS passes a valid data_in for interrupt callbacks.
        unsafe { (*data_in).user_interrupt = 1 };
    }
}

impl Highs {
    /// A quiet instance that runs single-threaded: parallelism in this crate is
    /// across problems, never inside one.
    pub fn new() -> Highs {
        // SAFETY: plain constructor.
        let raw = unsafe { ffi::Highs_create() };
        let ptr = NonNull::new(raw).expect("Highs_create returned null");
        let mut h = Highs { ptr, interrupt: None, cols: 0, rows: 0, cost: Vec::new(), maximize: true, mip: false, presolve: true };
        h.set_bool("output_flag", false);
        h.set_int("threads", 1);
        h
    }

    fn set_bool(&mut self, name: &str, value: bool) {
        let name = CString::new(name).unwrap();
        // SAFETY: valid instance and NUL-terminated option name.
        unsafe { ffi::Highs_setBoolOptionValue(self.ptr.as_ptr(), name.as_ptr(), value as ffi::HighsInt) };
    }

    fn set_int(&mut self, name: &str, value: i64) {
        let name = CString::new(name).unwrap();
        let value = value.clamp(i32::MIN as i64, i32::MAX as i64) as ffi::HighsInt;
        // SAFETY: as above.
        unsafe { ffi::Highs_setIntOptionValue(self.ptr.as_ptr(), name.as_ptr(), value) };
    }

    fn set_double(&mut self, name: &str, value: f64) {
        let name = CString::new(name).unwrap();
        // SAFETY: as above.
        unsafe { ffi::Highs_setDoubleOptionValue(self.ptr.as_ptr(), name.as_ptr(), value) };
    }

    fn set_string(&mut self, name: &str, value: &str) {
        let name = CString::new(name).unwrap();
        let value = CString::new(value).unwrap();
        // SAFETY: as above.
        unsafe { ffi::Highs_setStringOptionValue(self.ptr.as_ptr(), name.as_ptr(), value.as_ptr()) };
    }

    fn int_info(&self, name: &str) -> i64 {
        let name = CString::new(name).unwrap();
        let mut value: ffi::HighsInt = 0;
        // SAFETY: valid instance, out-pointer to a local.
        let status = unsafe { ffi::Highs_getIntInfoValue(self.ptr.as_ptr(), name.as_ptr(), &mut value) };
        if status == ffi::STATUS_ERROR { 0 } else { value as i64 }
    }

    fn int64_info(&self, name: &str) -> i64 {
        let name = CString::new(name).unwrap();
        let mut value: i64 = 0;
        // SAFETY: as above.
        let status = unsafe { ffi::Highs_getInt64InfoValue(self.ptr.as_ptr(), name.as_ptr(), &mut value) };
        if status == ffi::STATUS_ERROR { 0 } else { value }
    }

    /// Apply the settings of one solve.
    pub fn apply(&mut self, options: &SolveOptions) {
        self.set_time_limit(options.time_limit);
        self.set_presolve(options.presolve);
        if let Some(n) = options.node_limit {
            self.set_int("mip_max_nodes", n.min(i32::MAX as u64) as i64);
        }
        if let Some(t) = options.integrality_tolerance {
            self.set_double("mip_feasibility_tolerance", t);
        }
        if let Some(g) = options.mip_abs_gap {
            self.set_double("mip_abs_gap", g);
        }
        if let Some(g) = options.mip_rel_gap {
            self.set_double("mip_rel_gap", g);
        }
        self.set_interrupt(options.interrupt.clone());
    }

    /// Set any HiGHS option by name (see the HiGHS documentation). Returns
    /// `false` when HiGHS rejects the name or the value.
    pub fn set_option(&mut self, name: &str, value: OptionValue<'_>) -> bool {
        let Ok(cname) = CString::new(name) else { return false };
        let p = self.ptr.as_ptr();
        // SAFETY: valid instance and NUL-terminated strings.
        let status = unsafe {
            match value {
                OptionValue::Bool(b) => ffi::Highs_setBoolOptionValue(p, cname.as_ptr(), b as ffi::HighsInt),
                OptionValue::Int(i) => ffi::Highs_setIntOptionValue(p, cname.as_ptr(), i),
                OptionValue::Double(d) => ffi::Highs_setDoubleOptionValue(p, cname.as_ptr(), d),
                OptionValue::Str(s) => match CString::new(s) {
                    Ok(v) => ffi::Highs_setStringOptionValue(p, cname.as_ptr(), v.as_ptr()),
                    Err(_) => return false,
                },
            }
        };
        if name == "presolve" {
            if let OptionValue::Str(s) = value {
                self.presolve = s != "off";
            }
        }
        status != ffi::STATUS_ERROR
    }

    /// `None` lifts the limit.
    pub fn set_time_limit(&mut self, seconds: Option<f64>) {
        let s = seconds.filter(|s| s.is_finite() && *s > 0.0).unwrap_or(f64::INFINITY);
        self.set_double("time_limit", s);
    }

    /// Turn HiGHS' presolve on (`choose`) or off.
    pub fn set_presolve(&mut self, on: bool) {
        self.presolve = on;
        self.set_string("presolve", if on { "choose" } else { "off" });
    }

    /// Register (or clear) a flag that interrupts the solve when set.
    pub fn set_interrupt(&mut self, flag: Option<Arc<AtomicBool>>) {
        let p = self.ptr.as_ptr();
        // SAFETY: the callback only reads the AtomicBool, which `self.interrupt`
        // keeps alive while it is registered.
        unsafe {
            match &flag {
                Some(f) => {
                    ffi::Highs_setCallback(p, Some(interrupt_callback), Arc::as_ptr(f) as *mut c_void);
                    ffi::Highs_startCallback(p, ffi::kHighsCallbackSimplexInterrupt);
                    ffi::Highs_startCallback(p, ffi::kHighsCallbackIpmInterrupt);
                    ffi::Highs_startCallback(p, ffi::kHighsCallbackMipInterrupt);
                }
                None => {
                    if self.interrupt.is_some() {
                        ffi::Highs_stopCallback(p, ffi::kHighsCallbackSimplexInterrupt);
                        ffi::Highs_stopCallback(p, ffi::kHighsCallbackIpmInterrupt);
                        ffi::Highs_stopCallback(p, ffi::kHighsCallbackMipInterrupt);
                        ffi::Highs_setCallback(p, None, std::ptr::null_mut());
                    }
                }
            }
        }
        self.interrupt = flag;
    }

    /// Load a problem, replacing whatever the instance held.
    ///
    /// A row with no coefficients constrains nothing and is left free, as
    /// glpk.js leaves such rows out.
    pub fn pass_problem(&mut self, p: &LpProblem) -> Result<(), String> {
        if let Err((_, message)) = p.check() {
            return Err(message);
        }
        let (m, n) = (p.rows(), p.cols());
        let mut start: Vec<ffi::HighsInt> = Vec::with_capacity(n);
        let mut index: Vec<ffi::HighsInt> = Vec::with_capacity(p.a.nnz());
        let mut value: Vec<f64> = Vec::with_capacity(p.a.nnz());
        let mut used = vec![false; m];
        for j in 0..n {
            start.push(to_int(index.len())?);
            let mut col: Vec<(usize, f64)> = p.a.column(j).iter().filter(|(_, v)| *v != 0.0).copied().collect();
            col.sort_by_key(|(r, _)| *r);
            for (r, v) in col {
                used[r] = true;
                index.push(to_int(r)?);
                value.push(v);
            }
        }
        let row_lower: Vec<f64> = (0..m).map(|i| if used[i] { p.row_lower[i] } else { f64::NEG_INFINITY }).collect();
        let row_upper: Vec<f64> = (0..m).map(|i| if used[i] { p.row_upper[i] } else { f64::INFINITY }).collect();
        let sense = if p.maximize { ffi::OBJECTIVE_SENSE_MAXIMIZE } else { ffi::OBJECTIVE_SENSE_MINIMIZE };
        let mip = p.integer.iter().any(|b| *b);
        let ptr = |v: &[f64]| if v.is_empty() { std::ptr::null() } else { v.as_ptr() };
        let iptr = |v: &[ffi::HighsInt]| if v.is_empty() { std::ptr::null() } else { v.as_ptr() };
        // SAFETY: every array has the length HiGHS expects (n columns, m rows,
        // nnz entries) and outlives the call; HiGHS copies the data.
        let status = unsafe {
            if mip {
                let integrality: Vec<ffi::HighsInt> =
                    p.integer.iter().map(|b| if *b { ffi::VAR_TYPE_INTEGER } else { ffi::VAR_TYPE_CONTINUOUS }).collect();
                ffi::Highs_passMip(
                    self.ptr.as_ptr(),
                    to_int(n)?,
                    to_int(m)?,
                    to_int(value.len())?,
                    ffi::MATRIX_FORMAT_COLUMN_WISE,
                    sense,
                    0.0,
                    ptr(&p.cost),
                    ptr(&p.col_lower),
                    ptr(&p.col_upper),
                    ptr(&row_lower),
                    ptr(&row_upper),
                    iptr(&start),
                    iptr(&index),
                    ptr(&value),
                    iptr(&integrality),
                )
            } else {
                ffi::Highs_passLp(
                    self.ptr.as_ptr(),
                    to_int(n)?,
                    to_int(m)?,
                    to_int(value.len())?,
                    ffi::MATRIX_FORMAT_COLUMN_WISE,
                    sense,
                    0.0,
                    ptr(&p.cost),
                    ptr(&p.col_lower),
                    ptr(&p.col_upper),
                    ptr(&row_lower),
                    ptr(&row_upper),
                    iptr(&start),
                    iptr(&index),
                    ptr(&value),
                )
            }
        };
        if status == ffi::STATUS_ERROR {
            return Err("HiGHS rejected the problem".into());
        }
        self.cols = n;
        self.rows = m;
        self.cost = p.cost.clone();
        self.maximize = p.maximize;
        self.mip = mip;
        Ok(())
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    /// Replace the whole objective vector.
    pub fn set_costs(&mut self, cost: &[f64]) {
        assert_eq!(cost.len(), self.cols, "objective length");
        if self.cols == 0 {
            return;
        }
        // SAFETY: cost has `cols` entries.
        unsafe { ffi::Highs_changeColsCostByRange(self.ptr.as_ptr(), 0, (self.cols - 1) as ffi::HighsInt, cost.as_ptr()) };
        self.cost.copy_from_slice(cost);
    }

    /// Change one objective coefficient.
    pub fn set_cost(&mut self, j: usize, c: f64) {
        assert!(j < self.cols, "column out of range");
        // SAFETY: j is a valid column.
        unsafe { ffi::Highs_changeColCost(self.ptr.as_ptr(), j as ffi::HighsInt, c) };
        self.cost[j] = c;
    }

    /// Maximise (`true`) or minimise the objective.
    pub fn set_maximize(&mut self, maximize: bool) {
        let sense = if maximize { ffi::OBJECTIVE_SENSE_MAXIMIZE } else { ffi::OBJECTIVE_SENSE_MINIMIZE };
        // SAFETY: valid instance.
        unsafe { ffi::Highs_changeObjectiveSense(self.ptr.as_ptr(), sense) };
        self.maximize = maximize;
    }

    /// Change the bounds of column `j`. Panics when it is out of range.
    pub fn set_col_bounds(&mut self, j: usize, lower: f64, upper: f64) {
        assert!(j < self.cols, "column out of range");
        // SAFETY: j is a valid column.
        unsafe { ffi::Highs_changeColBounds(self.ptr.as_ptr(), j as ffi::HighsInt, lower, upper) };
    }

    /// Change the bounds of row `i`. Panics when it is out of range.
    pub fn set_row_bounds(&mut self, i: usize, lower: f64, upper: f64) {
        assert!(i < self.rows, "row out of range");
        // SAFETY: i is a valid row.
        unsafe { ffi::Highs_changeRowBounds(self.ptr.as_ptr(), i as ffi::HighsInt, lower, upper) };
    }

    /// Set the coefficient of row `i` in column `j`. Panics when either is out
    /// of range.
    pub fn set_coeff(&mut self, i: usize, j: usize, value: f64) {
        assert!(i < self.rows && j < self.cols, "coefficient out of range");
        // SAFETY: valid row and column.
        unsafe { ffi::Highs_changeCoeff(self.ptr.as_ptr(), i as ffi::HighsInt, j as ffi::HighsInt, value) };
    }

    /// The basis of the last solve, if HiGHS has a valid one.
    pub fn basis(&self) -> Option<Basis> {
        let mut col = vec![0 as ffi::HighsInt; self.cols];
        let mut row = vec![0 as ffi::HighsInt; self.rows];
        if self.int_info("basis_validity") != ffi::kHighsBasisValidityValid as i64 {
            return None;
        }
        // SAFETY: arrays sized to the problem.
        let status = unsafe { ffi::Highs_getBasis(self.ptr.as_ptr(), col.as_mut_ptr(), row.as_mut_ptr()) };
        (status != ffi::STATUS_ERROR).then_some(Basis { col, row })
    }

    /// Start the next solve from `basis`.
    pub fn set_basis(&mut self, basis: &Basis) -> Result<(), String> {
        if basis.col.len() != self.cols || basis.row.len() != self.rows {
            return Err("basis does not match the problem".into());
        }
        // SAFETY: arrays sized to the problem.
        let status = unsafe { ffi::Highs_setBasis(self.ptr.as_ptr(), basis.col.as_ptr(), basis.row.as_ptr()) };
        if status == ffi::STATUS_ERROR { Err("HiGHS rejected the basis".into()) } else { Ok(()) }
    }

    /// Forget the solver state (basis, factorisation, random state) but keep
    /// the problem: the next solve is exactly what a fresh instance would do,
    /// or what a fresh instance given the same basis would do.
    pub fn clear_solver(&mut self) {
        // SAFETY: valid instance.
        unsafe { ffi::Highs_clearSolver(self.ptr.as_ptr()) };
    }

    /// Solve the problem as it stands.
    pub fn run(&mut self) -> LpSolution {
        let mut status = self.run_once();
        // An "unbounded or infeasible" answer from presolve is not a verdict on
        // either: the plain simplex tells them apart, as glpk.js retries a
        // GLP_UNDEF without the presolver before believing it.
        if status.0 == ffi::MODEL_STATUS_UNBOUNDED_OR_INFEASIBLE && self.presolve {
            self.set_string("presolve", "off");
            self.clear_solver();
            status = self.run_once();
            self.set_string("presolve", "choose");
        }
        self.collect(status)
    }

    fn run_once(&mut self) -> ModelStatus {
        // SAFETY: valid instance holding a problem.
        unsafe { ffi::Highs_run(self.ptr.as_ptr()) };
        // SAFETY: as above.
        ModelStatus(unsafe { ffi::Highs_getModelStatus(self.ptr.as_ptr()) })
    }

    fn collect(&mut self, model_status: ModelStatus) -> LpSolution {
        let status = model_status.lp_status();
        let iterations = self.int_info("simplex_iteration_count").max(0) as u64;
        let nodes = if self.mip { self.int64_info("mip_node_count").max(0) as u64 } else { 0 };
        let mut out = LpSolution::failed(status, String::new());
        out.model_status = model_status;
        out.iterations = iterations;
        out.nodes = nodes;
        if status != LpStatus::Optimal {
            out.message = format!("HiGHS status {}", model_status.name());
        }
        // A MILP stopped at a limit may still hold an incumbent.
        let has_point = status == LpStatus::Optimal
            || (self.mip && self.int_info("primal_solution_status") == ffi::kHighsSolutionStatusFeasible as i64);
        if !has_point {
            return out;
        }
        let mut x = vec![0.0; self.cols];
        let mut col_dual = vec![0.0; self.cols];
        let mut row_value = vec![0.0; self.rows];
        let mut row_dual = vec![0.0; self.rows];
        // SAFETY: arrays sized to the problem.
        unsafe {
            ffi::Highs_getSolution(
                self.ptr.as_ptr(),
                x.as_mut_ptr(),
                col_dual.as_mut_ptr(),
                row_value.as_mut_ptr(),
                row_dual.as_mut_ptr(),
            )
        };
        out.objective = self.cost.iter().zip(&x).fold(0.0, |s, (c, v)| if *c == 0.0 { s } else { s + c * v });
        if !self.mip && status == LpStatus::Optimal {
            let basis = self.basis();
            if let Some(b) = &basis {
                for (j, d) in col_dual.iter_mut().enumerate() {
                    if b.is_basic_col(j) {
                        *d = 0.0;
                    }
                }
            }
            out.duals = Some(row_dual);
            out.reduced_costs = Some(col_dual);
            out.basis = basis;
        }
        out.x = Some(x);
        out.row_activity = Some(row_value);
        out
    }
}

fn to_int(n: usize) -> Result<ffi::HighsInt, String> {
    ffi::HighsInt::try_from(n).map_err(|_| "problem too large for HiGHS".to_string())
}

/// The HiGHS version string, for provenance records.
pub fn highs_version() -> String {
    // SAFETY: returns a static NUL-terminated string.
    unsafe { CStr::from_ptr(ffi::Highs_version()) }.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two(a0: f64, a1: f64) -> SparseColumns {
        let mut a = SparseColumns::new(1, 2);
        a.set(0, 0, a0);
        a.set(0, 1, a1);
        a
    }

    #[test]
    fn sparse_columns_accumulate_and_replace() {
        let mut a = SparseColumns::new(2, 1);
        a.add(0, 0, 1.0);
        a.add(0, 0, 2.0);
        a.set(1, 0, 5.0);
        a.set(1, 0, 4.0);
        a.add(1, 0, 0.0);
        assert_eq!(a.column(0), &[(0, 3.0), (1, 4.0)]);
        let j = a.push_column(vec![(1, 1.0)]);
        assert_eq!((j, a.cols(), a.nnz()), (1, 2, 3));
    }

    #[test]
    fn duals_follow_the_simplex_convention_in_both_senses() {
        // max x0 s.t. x0 + x1 = 1: dual 1, reduced costs [0 (basic), -1].
        let p = LpProblem::equality(two(1.0, 1.0), &[1.0], vec![1.0, 0.0], vec![0.0, 0.0], vec![10.0, 10.0], true);
        let r = solve_lp(&p, &SolveOptions::default());
        assert_eq!(r.status, LpStatus::Optimal);
        assert!((r.duals.as_ref().unwrap()[0] - 1.0).abs() < 1e-9, "{:?}", r.duals);
        let rc = r.reduced_costs.unwrap();
        assert!(rc[0].abs() < 1e-9 && (rc[1] + 1.0).abs() < 1e-9, "{rc:?}");
        // min x0 + 2 x1 s.t. x0 + x1 = 1: optimum x0 = 1, dual 1, rc [0, 1].
        let p = LpProblem::equality(two(1.0, 1.0), &[1.0], vec![1.0, 2.0], vec![0.0, 0.0], vec![10.0, 10.0], false);
        let r = solve_lp(&p, &SolveOptions::default());
        assert!((r.objective - 1.0).abs() < 1e-9);
        assert!((r.duals.as_ref().unwrap()[0] - 1.0).abs() < 1e-9, "{:?}", r.duals);
        let rc = r.reduced_costs.unwrap();
        assert!(rc[0].abs() < 1e-9 && (rc[1] - 1.0).abs() < 1e-9, "{rc:?}");
    }

    #[test]
    fn reversed_bounds_are_infeasible_without_solving() {
        let p = LpProblem::equality(two(1.0, 1.0), &[1.0], vec![1.0, 0.0], vec![2.0, 0.0], vec![1.0, 1.0], true);
        let r = solve_lp(&p, &SolveOptions::default());
        assert_eq!(r.status, LpStatus::Infeasible);
        assert!(r.message.contains("Reversed bounds"));
    }

    #[test]
    fn an_interrupt_is_unfinished_not_a_verdict() {
        let flag = Arc::new(AtomicBool::new(true));
        // A problem big enough to need iterations.
        let n = 60;
        let mut a = SparseColumns::new(n, n);
        for j in 0..n {
            for i in 0..n {
                a.set(i, j, ((i * 7 + j * 13) % 11) as f64 + 1.0);
            }
        }
        let mut p = LpProblem::equality(a, &vec![100.0; n], vec![1.0; n], vec![0.0; n], vec![f64::INFINITY; n], true);
        p.row_lower = vec![f64::NEG_INFINITY; n];
        let r = solve_lp(&p, &SolveOptions { interrupt: Some(flag), presolve: false, ..Default::default() });
        assert_eq!(r.status, LpStatus::IterationLimit, "{}", r.message);
        assert!(r.model_status.is_interrupt(), "{}", r.model_status.name());
    }

    #[test]
    fn warm_start_after_changing_the_objective() {
        let p = LpProblem::equality(two(1.0, 1.0), &[4.0], vec![3.0, 2.0], vec![0.0, 0.0], vec![3.0, 3.0], true);
        let mut h = Highs::new();
        h.pass_problem(&p).unwrap();
        assert!((h.run().objective - 11.0).abs() < 1e-9);
        h.set_costs(&[0.0, 1.0]);
        assert!((h.run().objective - 3.0).abs() < 1e-9);
        h.set_maximize(false);
        assert!((h.run().objective - 1.0).abs() < 1e-9);
        let basis = h.basis().unwrap();
        h.clear_solver();
        h.set_basis(&basis).unwrap();
        assert!((h.run().objective - 1.0).abs() < 1e-9);
    }
}
