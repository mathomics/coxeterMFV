//! Gurobi, opened at run time from the user's own installation.
//!
//! Nothing of Gurobi is linked into Coxeter or shipped with it. The C library
//! (`gurobi120.dll`, `libgurobi120.so`, `libgurobi120.dylib`) is opened where
//! the Gurobi installer put it, each function is looked up by name, and the
//! user's licence decides whether it solves. Gurobi 9.0 and later have every
//! function used here, with the prototypes of `gurobi_c.h`.
//!
//! Gurobi wants one environment per thread, created and freed one at a time.
//! Each [`GurobiInstance`] takes an environment of its own from a small pool,
//! and creating or freeing one holds [`LIFECYCLE`]. The pool keeps a single
//! idle environment for the next solve: the extra ones flux variability uses
//! are freed when it ends, so a floating licence is not held longer than the
//! work needs it.
//!
//! The problem is passed as it is, with one difference: Gurobi takes a row as
//! a sense and a right-hand side, so a row bounded on both sides becomes
//! `a·x − s = 0` with a slack column `s ∈ [lower, upper]` appended. The slacks
//! are this module's own: callers see the columns and rows they passed, and a
//! row's dual is the sensitivity to whichever of its bounds is active, as with
//! HiGHS.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::backends::{SolverError, SolverKind};
use crate::dylib::{c_api, open_library};
use crate::lp::{Basis, LpInstance, LpProblem, LpSolution, LpStatus, ModelStatus, SolveOptions, SparseColumns, row_activity};

// gurobi_c.h, unchanged since Gurobi 9.0.
const GRB_INFINITY: f64 = 1e100;
const GRB_MAXIMIZE: c_int = -1;
const GRB_MINIMIZE: c_int = 1;
/// `GRB_BASIC` of `VBasis` and `CBasis`.
pub(crate) const GRB_BASIC: i32 = 0;
const GRB_OPTIMAL: c_int = 2;
const GRB_INFEASIBLE: c_int = 3;
const GRB_INF_OR_UNBD: c_int = 4;
const GRB_UNBOUNDED: c_int = 5;
const GRB_CUTOFF: c_int = 6;
const GRB_ITERATION_LIMIT: c_int = 7;
const GRB_NODE_LIMIT: c_int = 8;
const GRB_TIME_LIMIT: c_int = 9;
const GRB_SOLUTION_LIMIT: c_int = 10;
const GRB_INTERRUPTED: c_int = 11;
const GRB_NUMERIC: c_int = 12;
const GRB_SUBOPTIMAL: c_int = 13;
const GRB_USER_OBJ_LIMIT: c_int = 15;
const GRB_WORK_LIMIT: c_int = 16;
const GRB_MEM_LIMIT: c_int = 17;
const GRB_ERROR_NO_LICENSE: c_int = 10009;
const GRB_ERROR_SIZE_LIMIT_EXCEEDED: c_int = 10010;

/// What a size-limited licence allows (the one `pip install gurobipy` comes
/// with): 2000 variables and 2000 constraints. A model one variable larger
/// tells such a licence apart.
const RESTRICTED_SIZE: usize = 2000;

/// Idle environments the pool keeps.
const MAX_IDLE: usize = 1;

/// Held while an environment is created or freed: Gurobi does neither safely
/// on two threads at once.
static LIFECYCLE: Mutex<()> = Mutex::new(());

type Callback = unsafe extern "system" fn(*mut c_void, *mut c_void, c_int, *mut c_void) -> c_int;

c_api! {
    pub(crate) struct Api {
        GRBemptyenv: fn(*mut *mut c_void) -> c_int;
        GRBstartenv: fn(*mut c_void) -> c_int;
        GRBfreeenv: fn(*mut c_void);
        GRBgeterrormsg: fn(*mut c_void) -> *const c_char;
        GRBversion: fn(*mut c_int, *mut c_int, *mut c_int);
        GRBsetintparam: fn(*mut c_void, *const c_char, c_int) -> c_int;
        GRBsetdblparam: fn(*mut c_void, *const c_char, f64) -> c_int;
        GRBgetenv: fn(*mut c_void) -> *mut c_void;
        GRBnewmodel: fn(*mut c_void, *mut *mut c_void, *const c_char, c_int, *mut f64, *mut f64, *mut f64, *mut c_char, *mut *mut c_char) -> c_int;
        GRBloadmodel: fn(
            *mut c_void, *mut *mut c_void, *const c_char, c_int, c_int, c_int, f64, *mut f64, *mut c_char, *mut f64,
            *mut c_int, *mut c_int, *mut c_int, *mut f64, *mut f64, *mut f64, *mut c_char, *mut *mut c_char, *mut *mut c_char,
        ) -> c_int;
        GRBfreemodel: fn(*mut c_void) -> c_int;
        GRBupdatemodel: fn(*mut c_void) -> c_int;
        GRBoptimize: fn(*mut c_void) -> c_int;
        GRBreset: fn(*mut c_void, c_int) -> c_int;
        GRBterminate: fn(*mut c_void);
        GRBsetcallbackfunc: fn(*mut c_void, Option<Callback>, *mut c_void) -> c_int;
        GRBgetintattr: fn(*mut c_void, *const c_char, *mut c_int) -> c_int;
        GRBsetintattr: fn(*mut c_void, *const c_char, c_int) -> c_int;
        GRBgetdblattr: fn(*mut c_void, *const c_char, *mut f64) -> c_int;
        GRBgetdblattrarray: fn(*mut c_void, *const c_char, c_int, c_int, *mut f64) -> c_int;
        GRBsetdblattrarray: fn(*mut c_void, *const c_char, c_int, c_int, *mut f64) -> c_int;
        GRBsetdblattrelement: fn(*mut c_void, *const c_char, c_int, f64) -> c_int;
        GRBgetintattrarray: fn(*mut c_void, *const c_char, c_int, c_int, *mut c_int) -> c_int;
        GRBsetintattrarray: fn(*mut c_void, *const c_char, c_int, c_int, *mut c_int) -> c_int;
    }
}

/// An environment, handed from the pool to one instance at a time.
struct EnvHandle(*mut c_void);

// SAFETY: a Gurobi environment has no thread affinity. The pool gives each
// one to a single instance, which uses it from one thread at a time.
unsafe impl Send for EnvHandle {}

/// Gurobi's library, opened and checked.
pub(crate) struct Gurobi {
    api: Api,
    path: PathBuf,
    version: String,
    limit: Option<String>,
    idle: Mutex<Vec<EnvHandle>>,
}

impl Gurobi {
    /// Open the library at `path` and check that it can solve: an
    /// environment starts only with a valid licence.
    pub(crate) fn open(path: &Path) -> Result<Gurobi, SolverError> {
        let library = open_library(path).map_err(SolverError::Load)?;
        let api = Api::load(library).map_err(|e| SolverError::Load(format!("{}: {e}", path.display())))?;
        let (mut major, mut minor, mut technical) = (0, 0, 0);
        // SAFETY: three out-pointers to locals.
        unsafe { (api.GRBversion)(&mut major, &mut minor, &mut technical) };
        let mut gurobi = Gurobi {
            api,
            path: path.to_path_buf(),
            version: format!("{major}.{minor}.{technical}"),
            limit: None,
            idle: Mutex::new(Vec::new()),
        };
        let env = gurobi.new_env().map_err(|message| SolverError::Unusable { version: Some(gurobi.version.clone()), message })?;
        gurobi.limit = gurobi.size_limit(&env);
        gurobi.checkin(env);
        Ok(gurobi)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn version(&self) -> &str {
        &self.version
    }

    pub(crate) fn limit(&self) -> Option<&str> {
        self.limit.as_deref()
    }

    /// A started environment: quiet, and single-threaded, since parallelism
    /// here is across problems, never inside one.
    fn new_env(&self) -> Result<EnvHandle, String> {
        let _lifecycle = LIFECYCLE.lock().unwrap_or_else(|e| e.into_inner());
        let mut env = ptr::null_mut();
        // SAFETY: an out-pointer to a local.
        let error = unsafe { (self.api.GRBemptyenv)(&mut env) };
        if env.is_null() {
            return Err(format!("Gurobi could not create an environment (error {error})."));
        }
        if error != 0 {
            let message = self.error_message(env, error);
            // SAFETY: the environment GRBemptyenv returned, freed once.
            unsafe { (self.api.GRBfreeenv)(env) };
            return Err(message);
        }
        // SAFETY: a new environment and NUL-terminated parameter names.
        let error = unsafe {
            (self.api.GRBsetintparam)(env, c"OutputFlag".as_ptr(), 0);
            (self.api.GRBsetintparam)(env, c"Threads".as_ptr(), 1);
            (self.api.GRBstartenv)(env)
        };
        if error != 0 {
            let message = self.error_message(env, error);
            // SAFETY: as above.
            unsafe { (self.api.GRBfreeenv)(env) };
            return Err(message);
        }
        Ok(EnvHandle(env))
    }

    /// Gurobi's own explanation of `code`, which names the licence problem
    /// when there is one.
    fn error_message(&self, env: *mut c_void, code: c_int) -> String {
        // SAFETY: a valid environment; Gurobi returns a NUL-terminated string
        // it owns, copied at once.
        let text = unsafe {
            let text = (self.api.GRBgeterrormsg)(env);
            if text.is_null() { String::new() } else { CStr::from_ptr(text).to_string_lossy().trim().to_string() }
        };
        match (code, text.is_empty()) {
            (GRB_ERROR_NO_LICENSE, true) => "No valid Gurobi licence was found.".to_string(),
            (GRB_ERROR_NO_LICENSE, false) => format!("No valid Gurobi licence: {text}"),
            (GRB_ERROR_SIZE_LIMIT_EXCEEDED, _) => format!(
                "The problem is larger than this Gurobi licence allows ({RESTRICTED_SIZE} variables and {RESTRICTED_SIZE} constraints). {text}"
            )
            .trim_end()
            .to_string(),
            (_, true) => format!("Gurobi error {code}."),
            (_, false) => format!("Gurobi error {code}: {text}"),
        }
    }

    /// Whether the licence is size-limited: such a licence refuses a model
    /// one variable larger than it allows.
    fn size_limit(&self, env: &EnvHandle) -> Option<String> {
        let mut model = ptr::null_mut();
        let n = (RESTRICTED_SIZE + 1) as c_int;
        // SAFETY: a started environment. Null arrays give `n` continuous
        // variables in [0, ∞) with no cost and no names.
        let error = unsafe {
            (self.api.GRBnewmodel)(
                env.0,
                &mut model,
                c"coxeter-size-probe".as_ptr(),
                n,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        if error != 0 || model.is_null() {
            return None;
        }
        // SAFETY: the model just made, freed once.
        let error = unsafe {
            let error = (self.api.GRBoptimize)(model);
            (self.api.GRBfreemodel)(model);
            error
        };
        (error == GRB_ERROR_SIZE_LIMIT_EXCEEDED)
            .then(|| format!("size-limited licence: at most {RESTRICTED_SIZE} variables and {RESTRICTED_SIZE} constraints"))
    }

    fn checkout(&self) -> Result<EnvHandle, String> {
        let idle = self.idle.lock().unwrap_or_else(|e| e.into_inner()).pop();
        match idle {
            Some(env) => Ok(env),
            None => self.new_env(),
        }
    }

    fn checkin(&self, env: EnvHandle) {
        let mut idle = self.idle.lock().unwrap_or_else(|e| e.into_inner());
        if idle.len() < MAX_IDLE {
            idle.push(env);
            return;
        }
        drop(idle);
        self.free_env(env);
    }

    fn free_env(&self, env: EnvHandle) {
        let _lifecycle = LIFECYCLE.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: an environment of this library no instance holds any more.
        unsafe { (self.api.GRBfreeenv)(env.0) };
    }
}

impl Drop for Gurobi {
    fn drop(&mut self) {
        let idle = std::mem::take(self.idle.get_mut().unwrap_or_else(|e| e.into_inner()));
        for env in idle {
            self.free_env(env);
        }
    }
}

/// What the callback reads: the flag, and how to stop the solve.
struct Interrupt {
    flag: Arc<AtomicBool>,
    terminate: unsafe extern "system" fn(*mut c_void),
}

/// Called by Gurobi as it works: stops the solve once the flag is set.
unsafe extern "system" fn interrupt_callback(model: *mut c_void, _cbdata: *mut c_void, _where: c_int, usrdata: *mut c_void) -> c_int {
    if usrdata.is_null() {
        return 0;
    }
    // SAFETY: usrdata is the boxed Interrupt the instance keeps for as long
    // as the callback is registered.
    let interrupt = unsafe { &*(usrdata as *const Interrupt) };
    if interrupt.flag.load(Ordering::Relaxed) {
        // SAFETY: the model being solved, as Gurobi passed it; GRBterminate
        // may be called from a callback.
        unsafe { (interrupt.terminate)(model) };
    }
    0
}

/// One Gurobi model, solved again after changing its objective, its sense or
/// its basis (see [`LpInstance`]).
pub(crate) struct GurobiInstance {
    gurobi: Arc<Gurobi>,
    env: Option<EnvHandle>,
    model: *mut c_void,
    cols: usize,
    rows: usize,
    /// Slack columns appended for rows bounded on both sides.
    extra: usize,
    cost: Vec<f64>,
    a: SparseColumns,
    mip: bool,
    settings: SolveOptions,
    interrupt: Option<Box<Interrupt>>,
}

// SAFETY: the model and its environment belong to this instance alone and are
// used from one thread at a time (the instance is not Sync).
unsafe impl Send for GurobiInstance {}

impl GurobiInstance {
    pub(crate) fn new(gurobi: Arc<Gurobi>) -> Result<GurobiInstance, String> {
        let env = gurobi.checkout()?;
        Ok(GurobiInstance {
            gurobi,
            env: Some(env),
            model: ptr::null_mut(),
            cols: 0,
            rows: 0,
            extra: 0,
            cost: Vec::new(),
            a: SparseColumns::default(),
            mip: false,
            settings: SolveOptions::default(),
            interrupt: None,
        })
    }

    fn api(&self) -> &Api {
        &self.gurobi.api
    }

    /// The environment Gurobi reads parameters from: the model's own copy
    /// once there is a model.
    fn param_env(&self) -> *mut c_void {
        if self.model.is_null() {
            self.env.as_ref().map_or(ptr::null_mut(), |e| e.0)
        } else {
            // SAFETY: a loaded model.
            unsafe { (self.api().GRBgetenv)(self.model) }
        }
    }

    fn free_model(&mut self) {
        if !self.model.is_null() {
            // SAFETY: the model this instance loaded, freed once.
            unsafe { (self.api().GRBfreemodel)(self.model) };
            self.model = ptr::null_mut();
        }
    }

    /// Write the settings of the solve into the model's parameters. Every one
    /// is set, so nothing is left from an earlier solve.
    fn push_settings(&self) {
        if self.model.is_null() {
            return;
        }
        let env = self.param_env();
        let s = &self.settings;
        let api = self.api();
        let time = s.time_limit.filter(|t| t.is_finite() && *t > 0.0).unwrap_or(GRB_INFINITY);
        // SAFETY: the model's environment and NUL-terminated parameter names.
        unsafe {
            (api.GRBsetdblparam)(env, c"TimeLimit".as_ptr(), time);
            (api.GRBsetintparam)(env, c"Presolve".as_ptr(), if s.presolve { -1 } else { 0 });
            (api.GRBsetdblparam)(env, c"NodeLimit".as_ptr(), s.node_limit.map_or(GRB_INFINITY, |n| n as f64));
            (api.GRBsetdblparam)(env, c"IntFeasTol".as_ptr(), s.integrality_tolerance.unwrap_or(1e-5).clamp(1e-9, 1e-1));
            (api.GRBsetdblparam)(env, c"MIPGapAbs".as_ptr(), s.mip_abs_gap.unwrap_or(1e-10).max(0.0));
            (api.GRBsetdblparam)(env, c"MIPGap".as_ptr(), s.mip_rel_gap.unwrap_or(1e-4).max(0.0));
        }
    }

    fn push_callback(&self) {
        if self.model.is_null() {
            return;
        }
        let (callback, data): (Option<Callback>, *mut c_void) = match &self.interrupt {
            Some(i) => (Some(interrupt_callback), &**i as *const Interrupt as *mut c_void),
            None => (None, ptr::null_mut()),
        };
        // SAFETY: a loaded model; the data is the boxed Interrupt this
        // instance keeps until the model is freed or the callback replaced.
        unsafe { (self.api().GRBsetcallbackfunc)(self.model, callback, data) };
    }

    fn int_attr(&self, name: &CStr) -> Option<c_int> {
        let mut value = 0;
        // SAFETY: a loaded model and an out-pointer to a local.
        let error = unsafe { (self.api().GRBgetintattr)(self.model, name.as_ptr(), &mut value) };
        (error == 0).then_some(value)
    }

    fn dbl_attr(&self, name: &CStr) -> Option<f64> {
        let mut value = 0.0;
        // SAFETY: as above.
        let error = unsafe { (self.api().GRBgetdblattr)(self.model, name.as_ptr(), &mut value) };
        (error == 0).then_some(value)
    }

    /// `len` values of an array attribute, or `None` when Gurobi has none.
    fn dbl_array(&self, name: &CStr, len: usize) -> Option<Vec<f64>> {
        let mut values = vec![0.0; len];
        if len == 0 {
            return Some(values);
        }
        // SAFETY: a loaded model and an array of `len` doubles.
        let error = unsafe { (self.api().GRBgetdblattrarray)(self.model, name.as_ptr(), 0, len as c_int, values.as_mut_ptr()) };
        (error == 0).then_some(values)
    }

    fn int_array(&self, name: &CStr, len: usize) -> Option<Vec<c_int>> {
        let mut values = vec![0; len];
        if len == 0 {
            return Some(values);
        }
        // SAFETY: as above, with ints.
        let error = unsafe { (self.api().GRBgetintattrarray)(self.model, name.as_ptr(), 0, len as c_int, values.as_mut_ptr()) };
        (error == 0).then_some(values)
    }

    fn optimize(&self) -> Result<(), String> {
        // SAFETY: a loaded model.
        let error = unsafe { (self.api().GRBoptimize)(self.model) };
        if error == 0 { Ok(()) } else { Err(self.gurobi.error_message(self.param_env(), error)) }
    }

    fn collect(&self, status: c_int) -> LpSolution {
        let model_status = model_status(status);
        let lp_status = model_status.lp_status();
        let mut out = LpSolution::failed(lp_status, String::new());
        out.model_status = model_status;
        out.iterations = self.dbl_attr(c"IterCount").filter(|v| v.is_finite() && *v > 0.0).map_or(0, |v| v as u64);
        if self.mip {
            out.nodes = self.dbl_attr(c"NodeCount").filter(|v| v.is_finite() && *v > 0.0).map_or(0, |v| v as u64);
        }
        if lp_status != LpStatus::Optimal {
            out.message = format!("Gurobi status {}", status_name(status));
        }
        // A MILP stopped at a limit may still hold an incumbent.
        let has_point = lp_status == LpStatus::Optimal || (self.mip && self.int_attr(c"SolCount").unwrap_or(0) > 0);
        if !has_point {
            return out;
        }
        let Some(x) = self.dbl_array(c"X", self.cols) else { return out };
        out.objective = self.cost.iter().zip(&x).fold(0.0, |s, (c, v)| if *c == 0.0 { s } else { s + c * v });
        if !self.mip && lp_status == LpStatus::Optimal {
            if let (Some(duals), Some(mut reduced)) = (self.dbl_array(c"Pi", self.rows), self.dbl_array(c"RC", self.cols)) {
                let basis = self.basis();
                if let Some(b) = &basis {
                    for (j, d) in reduced.iter_mut().enumerate() {
                        if b.is_basic_col(j) {
                            *d = 0.0;
                        }
                    }
                }
                out.duals = Some(duals);
                out.reduced_costs = Some(reduced);
                out.basis = basis;
            }
        }
        out.row_activity = Some(row_activity(&self.a, &x));
        out.x = Some(x);
        out
    }
}

impl Drop for GurobiInstance {
    fn drop(&mut self) {
        self.free_model();
        if let Some(env) = self.env.take() {
            self.gurobi.checkin(env);
        }
    }
}

impl LpInstance for GurobiInstance {
    fn apply(&mut self, options: &SolveOptions) {
        self.settings = SolveOptions { basis: None, interrupt: None, ..options.clone() };
        self.set_interrupt(options.interrupt.clone());
        self.push_settings();
    }

    fn set_interrupt(&mut self, flag: Option<Arc<AtomicBool>>) {
        let terminate = self.api().GRBterminate;
        self.interrupt = flag.map(|flag| Box::new(Interrupt { flag, terminate }));
        self.push_callback();
    }

    fn pass_problem(&mut self, p: &LpProblem) -> Result<(), String> {
        p.check().map_err(|(_, message)| message)?;
        self.free_model();
        let env = self.env.as_ref().map(|e| e.0).ok_or("Gurobi has no environment.")?;
        let (m, n) = (p.rows(), p.cols());
        let mut used = vec![false; m];
        for j in 0..n {
            for (i, v) in p.a.column(j) {
                if *v != 0.0 {
                    used[*i] = true;
                }
            }
        }
        // Rows: a sense and a right-hand side. A row with no coefficients, or
        // no finite bound, constrains nothing (glpk.js leaves such rows out).
        let mut sense: Vec<c_char> = Vec::with_capacity(m.max(1));
        let mut rhs: Vec<f64> = Vec::with_capacity(m.max(1));
        let mut ranged: Vec<(usize, f64, f64)> = Vec::new();
        for i in 0..m {
            let (lo, hi) = (p.row_lower[i], p.row_upper[i]);
            let (s, r) = if !used[i] || (lo == f64::NEG_INFINITY && hi == f64::INFINITY) {
                (b'<', GRB_INFINITY)
            } else if lo == hi {
                (b'=', lo)
            } else if lo == f64::NEG_INFINITY {
                (b'<', hi)
            } else if hi == f64::INFINITY {
                (b'>', lo)
            } else {
                ranged.push((i, lo, hi));
                (b'=', 0.0)
            };
            sense.push(s as c_char);
            rhs.push(r);
        }
        let total = n + ranged.len();
        let mut vbeg: Vec<c_int> = Vec::with_capacity(total.max(1));
        let mut vlen: Vec<c_int> = Vec::with_capacity(total.max(1));
        let mut vind: Vec<c_int> = Vec::with_capacity(p.a.nnz().max(1));
        let mut vval: Vec<f64> = Vec::with_capacity(p.a.nnz().max(1));
        for j in 0..n {
            let mut column: Vec<(usize, f64)> = p.a.column(j).iter().filter(|(_, v)| *v != 0.0).copied().collect();
            column.sort_by_key(|(r, _)| *r);
            vbeg.push(to_int(vind.len())?);
            vlen.push(to_int(column.len())?);
            for (r, v) in column {
                vind.push(to_int(r)?);
                vval.push(v);
            }
        }
        let infinite = |v: f64| v.clamp(-GRB_INFINITY, GRB_INFINITY);
        let mut obj: Vec<f64> = Vec::with_capacity(total.max(1));
        obj.extend_from_slice(&p.cost);
        let mut lb: Vec<f64> = Vec::with_capacity(total.max(1));
        lb.extend(p.col_lower.iter().map(|v| infinite(*v)));
        let mut ub: Vec<f64> = Vec::with_capacity(total.max(1));
        ub.extend(p.col_upper.iter().map(|v| infinite(*v)));
        for &(i, lo, hi) in &ranged {
            vbeg.push(to_int(vind.len())?);
            vlen.push(1);
            vind.push(to_int(i)?);
            vval.push(-1.0);
            obj.push(0.0);
            lb.push(lo);
            ub.push(hi);
        }
        let mip = p.integer.iter().any(|b| *b);
        let mut vtype: Vec<c_char> = Vec::with_capacity(total.max(1));
        if mip {
            vtype.extend((0..total).map(|j| if p.integer.get(j).copied().unwrap_or(false) { b'I' as c_char } else { b'C' as c_char }));
        }
        let mut model = ptr::null_mut();
        // SAFETY: every array has the length Gurobi reads (`total` columns,
        // `m` rows, `vind.len()` entries) and outlives the call, which copies
        // them; the pointers are never written through.
        let error = unsafe {
            (self.api().GRBloadmodel)(
                env,
                &mut model,
                c"coxeter".as_ptr(),
                to_int(total)?,
                to_int(m)?,
                if p.maximize { GRB_MAXIMIZE } else { GRB_MINIMIZE },
                0.0,
                obj.as_mut_ptr(),
                sense.as_mut_ptr(),
                rhs.as_mut_ptr(),
                vbeg.as_mut_ptr(),
                vlen.as_mut_ptr(),
                vind.as_mut_ptr(),
                vval.as_mut_ptr(),
                lb.as_mut_ptr(),
                ub.as_mut_ptr(),
                if mip { vtype.as_mut_ptr() } else { ptr::null_mut() },
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        if error != 0 || model.is_null() {
            if !model.is_null() {
                // SAFETY: a model Gurobi returned along with the error.
                unsafe { (self.api().GRBfreemodel)(model) };
            }
            return Err(self.gurobi.error_message(env, error));
        }
        self.model = model;
        self.cols = n;
        self.rows = m;
        self.extra = ranged.len();
        self.cost = p.cost.clone();
        self.a = p.a.clone();
        self.mip = mip;
        self.push_settings();
        self.push_callback();
        Ok(())
    }

    fn set_costs(&mut self, cost: &[f64]) {
        assert_eq!(cost.len(), self.cols, "objective length");
        if self.cols == 0 || self.model.is_null() {
            return;
        }
        let mut values = cost.to_vec();
        // SAFETY: a loaded model and `cols` costs.
        unsafe { (self.api().GRBsetdblattrarray)(self.model, c"Obj".as_ptr(), 0, self.cols as c_int, values.as_mut_ptr()) };
        self.cost.copy_from_slice(cost);
    }

    fn set_cost(&mut self, j: usize, c: f64) {
        assert!(j < self.cols, "column out of range");
        // SAFETY: a loaded model and a valid column.
        unsafe { (self.api().GRBsetdblattrelement)(self.model, c"Obj".as_ptr(), j as c_int, c) };
        self.cost[j] = c;
    }

    fn set_maximize(&mut self, maximize: bool) {
        if self.model.is_null() {
            return;
        }
        // SAFETY: a loaded model.
        unsafe { (self.api().GRBsetintattr)(self.model, c"ModelSense".as_ptr(), if maximize { GRB_MAXIMIZE } else { GRB_MINIMIZE }) };
    }

    fn basis(&self) -> Option<Basis> {
        if self.model.is_null() || self.mip {
            return None;
        }
        let mut col = self.int_array(c"VBasis", self.cols + self.extra)?;
        let row = self.int_array(c"CBasis", self.rows)?;
        let extra = col.split_off(self.cols);
        Some(Basis::new(SolverKind::Gurobi, col, row, extra))
    }

    fn set_basis(&mut self, basis: &Basis) -> Result<(), String> {
        if self.model.is_null()
            || basis.solver() != SolverKind::Gurobi
            || basis.cols() != self.cols
            || basis.rows() != self.rows
            || basis.extra_statuses().len() != self.extra
        {
            return Err("basis does not match the problem".into());
        }
        let mut col: Vec<c_int> = basis.col_statuses().iter().chain(basis.extra_statuses()).copied().collect();
        let mut row: Vec<c_int> = basis.row_statuses().to_vec();
        // SAFETY: a loaded model, and arrays sized to its columns and rows.
        let error = unsafe {
            (self.api().GRBupdatemodel)(self.model);
            let mut error = 0;
            if !col.is_empty() {
                error = (self.api().GRBsetintattrarray)(self.model, c"VBasis".as_ptr(), 0, col.len() as c_int, col.as_mut_ptr());
            }
            if error == 0 && !row.is_empty() {
                error = (self.api().GRBsetintattrarray)(self.model, c"CBasis".as_ptr(), 0, row.len() as c_int, row.as_mut_ptr());
            }
            error
        };
        if error == 0 { Ok(()) } else { Err("Gurobi rejected the basis".into()) }
    }

    fn clear_solver(&mut self) {
        if !self.model.is_null() {
            // SAFETY: a loaded model. Keeps the parameters, drops the solution
            // and the basis, so the next solve starts from scratch.
            unsafe { (self.api().GRBreset)(self.model, 0) };
        }
    }

    fn run(&mut self) -> LpSolution {
        if self.model.is_null() {
            return LpSolution::failed(LpStatus::IterationLimit, "No problem was passed to Gurobi.");
        }
        if self.interrupt.as_ref().is_some_and(|i| i.flag.load(Ordering::Relaxed)) {
            let mut out = LpSolution::failed(LpStatus::IterationLimit, "Gurobi status interrupted");
            out.model_status = ModelStatus::INTERRUPTED;
            return out;
        }
        if let Err(message) = self.optimize() {
            return LpSolution::failed(LpStatus::IterationLimit, message);
        }
        let mut status = self.int_attr(c"Status").unwrap_or(0);
        // "Infeasible or unbounded" is not a verdict on either. Without dual
        // reductions Gurobi tells them apart (its own advice), as glpk.js
        // retries a GLP_UNDEF without the presolver before believing it.
        if status == GRB_INF_OR_UNBD {
            let env = self.param_env();
            // SAFETY: the model's environment and the model.
            unsafe {
                (self.api().GRBsetintparam)(env, c"DualReductions".as_ptr(), 0);
                (self.api().GRBreset)(self.model, 0);
            }
            let again = self.optimize();
            // SAFETY: as above.
            unsafe { (self.api().GRBsetintparam)(env, c"DualReductions".as_ptr(), 1) };
            if let Err(message) = again {
                return LpSolution::failed(LpStatus::IterationLimit, message);
            }
            status = self.int_attr(c"Status").unwrap_or(0);
        }
        self.collect(status)
    }
}

fn to_int(n: usize) -> Result<c_int, String> {
    c_int::try_from(n).map_err(|_| "problem too large for Gurobi".to_string())
}

/// Gurobi's status in the crate's terms (HiGHS' numbering; see [`ModelStatus`]).
fn model_status(status: c_int) -> ModelStatus {
    match status {
        GRB_OPTIMAL => ModelStatus::OPTIMAL,
        GRB_INFEASIBLE => ModelStatus::INFEASIBLE,
        GRB_INF_OR_UNBD => ModelStatus::UNBOUNDED_OR_INFEASIBLE,
        GRB_UNBOUNDED => ModelStatus::UNBOUNDED,
        GRB_CUTOFF | GRB_USER_OBJ_LIMIT => ModelStatus::OBJECTIVE_BOUND,
        GRB_ITERATION_LIMIT => ModelStatus::ITERATION_LIMIT,
        // HiGHS reports its node limit as a solution limit too.
        GRB_NODE_LIMIT | GRB_SOLUTION_LIMIT => ModelStatus::SOLUTION_LIMIT,
        GRB_TIME_LIMIT | GRB_WORK_LIMIT => ModelStatus::TIME_LIMIT,
        GRB_INTERRUPTED => ModelStatus::INTERRUPTED,
        GRB_NUMERIC => ModelStatus::SOLVE_ERROR,
        GRB_MEM_LIMIT => ModelStatus::MEMORY_LIMIT,
        _ => ModelStatus::UNKNOWN,
    }
}

fn status_name(status: c_int) -> &'static str {
    match status {
        GRB_OPTIMAL => "optimal",
        GRB_INFEASIBLE => "infeasible",
        GRB_INF_OR_UNBD => "infeasible or unbounded",
        GRB_UNBOUNDED => "unbounded",
        GRB_CUTOFF => "cutoff",
        GRB_ITERATION_LIMIT => "iteration limit",
        GRB_NODE_LIMIT => "node limit",
        GRB_TIME_LIMIT => "time limit",
        GRB_SOLUTION_LIMIT => "solution limit",
        GRB_INTERRUPTED => "interrupted",
        GRB_NUMERIC => "numerical trouble",
        GRB_SUBOPTIMAL => "suboptimal",
        GRB_USER_OBJ_LIMIT => "objective limit",
        GRB_WORK_LIMIT => "work limit",
        GRB_MEM_LIMIT => "memory limit",
        _ => "unknown",
    }
}
