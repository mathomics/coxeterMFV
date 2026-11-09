//! CPLEX, opened at run time from the user's own installation.
//!
//! Nothing of CPLEX is linked into Coxeter or shipped with it. The Callable
//! Library (`cplex2210.dll`, `libcplex2210.so`, `libcplex2210.dylib`) is
//! opened where CPLEX Optimization Studio installed it, each function is
//! looked up by name, and the user's licence decides what it solves. CPLEX
//! 12.6 and later have every function used here, with the prototypes of
//! `cplex.h`.
//!
//! Each [`CplexInstance`] solves in an environment of its own, taken from a
//! small pool (CPLEX keeps parameters per environment, so every solve sets all
//! the ones it relies on). The free Community Edition refuses problems above
//! 1000 variables or constraints; [`Cplex::open`] finds out whether the
//! installation is limited, and a refused problem says why.
//!
//! A solve is stopped through CPLEX's informational callbacks, which return
//! nonzero once the caller's flag is set.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::backends::{SolverError, SolverKind};
use crate::dylib::{c_api, open_library};
use crate::lp::{Basis, LpInstance, LpProblem, LpSolution, LpStatus, ModelStatus, SolveOptions, SparseColumns, row_activity};

// cpxconst.h.
const CPX_INFBOUND: f64 = 1.0e20;
const CPX_MAX: c_int = -1;
const CPX_MIN: c_int = 1;
/// `CPX_BASIC` of `CPXgetbase`.
pub(crate) const CPX_BASIC: i32 = 1;
const CPX_NO_SOLN: c_int = 0;
const CPXMESSAGEBUFSIZE: usize = 1024;
const CPXERR_RESTRICTED_VERSION: c_int = 1016;

const CPXPARAM_ADVANCE: c_int = 1001;
const CPXPARAM_PRESOLVE: c_int = 1030;
const CPXPARAM_SCREEN_OUTPUT: c_int = 1035;
const CPXPARAM_TIME_LIMIT: c_int = 1039;
const CPXPARAM_THREADS: c_int = 1067;
const CPXPARAM_ABS_MIP_GAP: c_int = 2008;
const CPXPARAM_MIP_GAP: c_int = 2009;
const CPXPARAM_INTEGRALITY: c_int = 2010;
const CPXPARAM_NODE_LIMIT: c_int = 2017;

const CPX_STAT_OPTIMAL: c_int = 1;
const CPX_STAT_UNBOUNDED: c_int = 2;
const CPX_STAT_INFEASIBLE: c_int = 3;
const CPX_STAT_INFORUNBD: c_int = 4;
const CPX_STAT_OPTIMAL_INFEAS: c_int = 5;
const CPX_STAT_NUM_BEST: c_int = 6;
const CPX_STAT_ABORT_IT_LIM: c_int = 10;
const CPX_STAT_ABORT_TIME_LIM: c_int = 11;
const CPX_STAT_ABORT_OBJ_LIM: c_int = 12;
const CPX_STAT_ABORT_USER: c_int = 13;
const CPX_STAT_ABORT_PRIM_OBJ_LIM: c_int = 21;
const CPX_STAT_ABORT_DUAL_OBJ_LIM: c_int = 22;
const CPX_STAT_ABORT_DETTIME_LIM: c_int = 25;
const CPXMIP_OPTIMAL: c_int = 101;
const CPXMIP_OPTIMAL_TOL: c_int = 102;
const CPXMIP_INFEASIBLE: c_int = 103;
const CPXMIP_SOL_LIM: c_int = 104;
const CPXMIP_NODE_LIM_FEAS: c_int = 105;
const CPXMIP_NODE_LIM_INFEAS: c_int = 106;
const CPXMIP_TIME_LIM_FEAS: c_int = 107;
const CPXMIP_TIME_LIM_INFEAS: c_int = 108;
const CPXMIP_FAIL_FEAS: c_int = 109;
const CPXMIP_FAIL_INFEAS: c_int = 110;
const CPXMIP_MEM_LIM_FEAS: c_int = 111;
const CPXMIP_MEM_LIM_INFEAS: c_int = 112;
const CPXMIP_ABORT_FEAS: c_int = 113;
const CPXMIP_ABORT_INFEAS: c_int = 114;
const CPXMIP_OPTIMAL_INFEAS: c_int = 115;
const CPXMIP_FAIL_FEAS_NO_TREE: c_int = 116;
const CPXMIP_FAIL_INFEAS_NO_TREE: c_int = 117;
const CPXMIP_UNBOUNDED: c_int = 118;
const CPXMIP_INFORUNBD: c_int = 119;
const CPXMIP_DETTIME_LIM_FEAS: c_int = 131;
const CPXMIP_DETTIME_LIM_INFEAS: c_int = 132;

/// What the Community Edition allows: 1000 variables and 1000 constraints.
const COMMUNITY_SIZE: usize = 1000;

/// Idle environments the pool keeps.
const MAX_IDLE: usize = 1;

/// Held while an environment is opened or closed.
static LIFECYCLE: Mutex<()> = Mutex::new(());

type Callback = unsafe extern "system" fn(*const c_void, *mut c_void, c_int, *mut c_void) -> c_int;

c_api! {
    pub(crate) struct Api {
        CPXopenCPLEX: fn(*mut c_int) -> *mut c_void;
        CPXcloseCPLEX: fn(*mut *mut c_void) -> c_int;
        CPXversion: fn(*const c_void) -> *const c_char;
        CPXgeterrorstring: fn(*const c_void, c_int, *mut c_char) -> *const c_char;
        CPXsetintparam: fn(*mut c_void, c_int, c_int) -> c_int;
        CPXsetdblparam: fn(*mut c_void, c_int, f64) -> c_int;
        CPXsetlongparam: fn(*mut c_void, c_int, i64) -> c_int;
        CPXsetlpcallbackfunc: fn(*mut c_void, Option<Callback>, *mut c_void) -> c_int;
        CPXsetinfocallbackfunc: fn(*mut c_void, Option<Callback>, *mut c_void) -> c_int;
        CPXcreateprob: fn(*const c_void, *mut c_int, *const c_char) -> *mut c_void;
        CPXfreeprob: fn(*const c_void, *mut *mut c_void) -> c_int;
        CPXcopylp: fn(
            *const c_void, *mut c_void, c_int, c_int, c_int, *const f64, *const f64, *const c_char,
            *const c_int, *const c_int, *const c_int, *const f64, *const f64, *const f64, *const f64,
        ) -> c_int;
        CPXcopyctype: fn(*const c_void, *mut c_void, *const c_char) -> c_int;
        CPXlpopt: fn(*const c_void, *mut c_void) -> c_int;
        CPXmipopt: fn(*const c_void, *mut c_void) -> c_int;
        CPXgetstat: fn(*const c_void, *const c_void) -> c_int;
        CPXgetx: fn(*const c_void, *const c_void, *mut f64, c_int, c_int) -> c_int;
        CPXgetpi: fn(*const c_void, *const c_void, *mut f64, c_int, c_int) -> c_int;
        CPXgetdj: fn(*const c_void, *const c_void, *mut f64, c_int, c_int) -> c_int;
        CPXgetbase: fn(*const c_void, *const c_void, *mut c_int, *mut c_int) -> c_int;
        CPXcopybase: fn(*const c_void, *mut c_void, *const c_int, *const c_int) -> c_int;
        CPXchgobj: fn(*const c_void, *mut c_void, c_int, *const c_int, *const f64) -> c_int;
        CPXchgobjsen: fn(*const c_void, *mut c_void, c_int) -> c_int;
        CPXgetitcnt: fn(*const c_void, *const c_void) -> c_int;
        CPXgetmipitcnt: fn(*const c_void, *const c_void) -> c_int;
        CPXgetnodecnt: fn(*const c_void, *const c_void) -> c_int;
        CPXsolninfo: fn(*const c_void, *const c_void, *mut c_int, *mut c_int, *mut c_int, *mut c_int) -> c_int;
    }
}

/// An environment, handed from the pool to one instance at a time.
struct EnvHandle(*mut c_void);

// SAFETY: a CPLEX environment has no thread affinity. The pool gives each one
// to a single instance, which uses it from one thread at a time.
unsafe impl Send for EnvHandle {}

/// CPLEX's library, opened and checked.
pub(crate) struct Cplex {
    api: Api,
    path: PathBuf,
    version: String,
    limit: Option<String>,
    idle: Mutex<Vec<EnvHandle>>,
}

impl Cplex {
    /// Open the library at `path` and check that an environment opens (a
    /// licence, for the commercial and academic editions).
    pub(crate) fn open(path: &Path) -> Result<Cplex, SolverError> {
        let library = open_library(path).map_err(SolverError::Load)?;
        let api = Api::load(library).map_err(|e| SolverError::Load(format!("{}: {e}", path.display())))?;
        let mut cplex = Cplex { api, path: path.to_path_buf(), version: String::new(), limit: None, idle: Mutex::new(Vec::new()) };
        let env = cplex.new_env().map_err(|message| SolverError::Unusable { version: None, message })?;
        // SAFETY: an open environment; CPLEX owns the string, copied at once.
        cplex.version = unsafe {
            let text = (cplex.api.CPXversion)(env.0);
            if text.is_null() { String::new() } else { CStr::from_ptr(text).to_string_lossy().trim().to_string() }
        };
        cplex.limit = cplex.size_limit(&env);
        cplex.checkin(env);
        Ok(cplex)
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

    /// An open environment: quiet, and single-threaded, since parallelism
    /// here is across problems, never inside one.
    fn new_env(&self) -> Result<EnvHandle, String> {
        let _lifecycle = LIFECYCLE.lock().unwrap_or_else(|e| e.into_inner());
        let mut status = 0;
        // SAFETY: an out-pointer to a local.
        let env = unsafe { (self.api.CPXopenCPLEX)(&mut status) };
        if env.is_null() {
            return Err(self.error_message(ptr::null(), status));
        }
        // SAFETY: an open environment.
        unsafe {
            (self.api.CPXsetintparam)(env, CPXPARAM_SCREEN_OUTPUT, 0);
            (self.api.CPXsetintparam)(env, CPXPARAM_THREADS, 1);
        }
        Ok(EnvHandle(env))
    }

    /// CPLEX's text for an error code (`CPLEX Error  1016: Community
    /// Edition. Problem size limits exceeded. …`).
    fn error_message(&self, env: *const c_void, code: c_int) -> String {
        let mut buffer = [0 as c_char; CPXMESSAGEBUFSIZE];
        // SAFETY: a buffer of CPXMESSAGEBUFSIZE bytes, as CPLEX requires; the
        // environment may be null.
        let text = unsafe {
            let found = (self.api.CPXgeterrorstring)(env, code, buffer.as_mut_ptr());
            if found.is_null() { String::new() } else { CStr::from_ptr(buffer.as_ptr()).to_string_lossy().trim().to_string() }
        };
        match (code, text.is_empty()) {
            (CPXERR_RESTRICTED_VERSION, _) => format!(
                "The problem is larger than CPLEX Community Edition allows ({COMMUNITY_SIZE} variables and {COMMUNITY_SIZE} constraints). {text}"
            )
            .trim_end()
            .to_string(),
            (_, true) => format!("CPLEX error {code}."),
            (_, false) => text,
        }
    }

    /// Whether this is the Community Edition: it refuses a problem one
    /// variable larger than it allows.
    fn size_limit(&self, env: &EnvHandle) -> Option<String> {
        let api = &self.api;
        let n = COMMUNITY_SIZE + 1;
        let zeros = vec![0.0; n];
        let upper = vec![1.0; n];
        let counts: Vec<c_int> = vec![0; n];
        let (no_index, no_value, sense, rhs) = ([0 as c_int], [0.0f64], [b'E' as c_char], [0.0f64]);
        let mut status = 0;
        // SAFETY: an open environment, and arrays of `n` columns with no
        // entries; the single-element arrays stand for the empty ones.
        unsafe {
            let mut lp = (api.CPXcreateprob)(env.0, &mut status, c"coxeter-size-probe".as_ptr());
            if lp.is_null() {
                return None;
            }
            let mut error = (api.CPXcopylp)(
                env.0,
                lp,
                n as c_int,
                0,
                CPX_MIN,
                zeros.as_ptr(),
                rhs.as_ptr(),
                sense.as_ptr(),
                counts.as_ptr(),
                counts.as_ptr(),
                no_index.as_ptr(),
                no_value.as_ptr(),
                zeros.as_ptr(),
                upper.as_ptr(),
                ptr::null(),
            );
            if error == 0 {
                error = (api.CPXlpopt)(env.0, lp);
            }
            (api.CPXfreeprob)(env.0, &mut lp);
            (error == CPXERR_RESTRICTED_VERSION)
                .then(|| format!("Community Edition: at most {COMMUNITY_SIZE} variables and {COMMUNITY_SIZE} constraints"))
        }
    }

    fn checkout(&self) -> Result<EnvHandle, String> {
        let idle = self.idle.lock().unwrap_or_else(|e| e.into_inner()).pop();
        match idle {
            Some(env) => Ok(env),
            None => self.new_env(),
        }
    }

    fn checkin(&self, env: EnvHandle) {
        // SAFETY: an open environment; its callbacks pointed at an instance
        // that is going away.
        unsafe {
            (self.api.CPXsetlpcallbackfunc)(env.0, None, ptr::null_mut());
            (self.api.CPXsetinfocallbackfunc)(env.0, None, ptr::null_mut());
        }
        let mut idle = self.idle.lock().unwrap_or_else(|e| e.into_inner());
        if idle.len() < MAX_IDLE {
            idle.push(env);
            return;
        }
        drop(idle);
        self.close_env(env);
    }

    fn close_env(&self, env: EnvHandle) {
        let _lifecycle = LIFECYCLE.lock().unwrap_or_else(|e| e.into_inner());
        let mut env = env.0;
        // SAFETY: an environment of this library no instance holds any more.
        unsafe { (self.api.CPXcloseCPLEX)(&mut env) };
    }
}

impl Drop for Cplex {
    fn drop(&mut self) {
        let idle = std::mem::take(self.idle.get_mut().unwrap_or_else(|e| e.into_inner()));
        for env in idle {
            self.close_env(env);
        }
    }
}

/// Called by CPLEX as it works: nonzero stops the solve.
unsafe extern "system" fn interrupt_callback(_env: *const c_void, _cbdata: *mut c_void, _wherefrom: c_int, handle: *mut c_void) -> c_int {
    if handle.is_null() {
        return 0;
    }
    // SAFETY: the handle is the AtomicBool inside the Arc the instance keeps
    // alive for as long as the callback is registered.
    let flag = unsafe { &*(handle as *const AtomicBool) };
    c_int::from(flag.load(Ordering::Relaxed))
}

/// One CPLEX problem, solved again after changing its objective, its sense or
/// its basis (see [`LpInstance`]).
pub(crate) struct CplexInstance {
    cplex: Arc<Cplex>,
    env: Option<EnvHandle>,
    lp: *mut c_void,
    cols: usize,
    rows: usize,
    cost: Vec<f64>,
    a: SparseColumns,
    mip: bool,
    settings: SolveOptions,
    interrupt: Option<Arc<AtomicBool>>,
    /// The next solve starts from scratch, not from the last basis.
    cold: bool,
}

// SAFETY: the problem and its environment belong to this instance alone and
// are used from one thread at a time (the instance is not Sync).
unsafe impl Send for CplexInstance {}

impl CplexInstance {
    pub(crate) fn new(cplex: Arc<Cplex>) -> Result<CplexInstance, String> {
        let env = cplex.checkout()?;
        let instance = CplexInstance {
            cplex,
            env: Some(env),
            lp: ptr::null_mut(),
            cols: 0,
            rows: 0,
            cost: Vec::new(),
            a: SparseColumns::default(),
            mip: false,
            settings: SolveOptions::default(),
            interrupt: None,
            cold: false,
        };
        instance.push_settings();
        Ok(instance)
    }

    fn api(&self) -> &Api {
        &self.cplex.api
    }

    fn env(&self) -> *mut c_void {
        self.env.as_ref().map_or(ptr::null_mut(), |e| e.0)
    }

    fn free_problem(&mut self) {
        if !self.lp.is_null() {
            // SAFETY: the problem this instance created, freed once.
            unsafe { (self.api().CPXfreeprob)(self.env(), &mut self.lp) };
            self.lp = ptr::null_mut();
        }
    }

    /// Write the settings of the solve into the environment. Every one is set,
    /// so nothing is left from an earlier solve in this environment.
    fn push_settings(&self) {
        let env = self.env();
        if env.is_null() {
            return;
        }
        let s = &self.settings;
        let api = self.api();
        // SAFETY: an open environment; CPLEX's defaults where nothing is asked.
        unsafe {
            (api.CPXsetdblparam)(env, CPXPARAM_TIME_LIMIT, s.time_limit.filter(|t| t.is_finite() && *t > 0.0).unwrap_or(1e75));
            (api.CPXsetintparam)(env, CPXPARAM_PRESOLVE, c_int::from(s.presolve));
            (api.CPXsetlongparam)(env, CPXPARAM_NODE_LIMIT, s.node_limit.map_or(9_223_372_036_800_000_000, |n| n.min(i64::MAX as u64) as i64));
            (api.CPXsetdblparam)(env, CPXPARAM_INTEGRALITY, s.integrality_tolerance.unwrap_or(1e-5).clamp(0.0, 0.5));
            (api.CPXsetdblparam)(env, CPXPARAM_ABS_MIP_GAP, s.mip_abs_gap.unwrap_or(1e-6).max(0.0));
            (api.CPXsetdblparam)(env, CPXPARAM_MIP_GAP, s.mip_rel_gap.unwrap_or(1e-4).clamp(0.0, 1.0));
        }
    }

    fn optimize(&self) -> Result<(), String> {
        // SAFETY: an open environment and a loaded problem.
        let error = unsafe {
            if self.mip { (self.api().CPXmipopt)(self.env(), self.lp) } else { (self.api().CPXlpopt)(self.env(), self.lp) }
        };
        if error == 0 { Ok(()) } else { Err(self.cplex.error_message(self.env(), error)) }
    }

    fn status(&self) -> c_int {
        // SAFETY: as above.
        unsafe { (self.api().CPXgetstat)(self.env(), self.lp) }
    }

    /// Fill `len` values with `get(env, lp, values, 0, len - 1)`, or `None`
    /// when CPLEX has none.
    fn values(&self, get: unsafe extern "system" fn(*const c_void, *const c_void, *mut f64, c_int, c_int) -> c_int, len: usize) -> Option<Vec<f64>> {
        let mut values = vec![0.0; len];
        if len == 0 {
            return Some(values);
        }
        // SAFETY: an open environment, a loaded problem and `len` doubles.
        let error = unsafe { get(self.env(), self.lp, values.as_mut_ptr(), 0, len as c_int - 1) };
        (error == 0).then_some(values)
    }

    fn has_solution(&self) -> bool {
        let (mut method, mut kind, mut primal, mut dual) = (0, 0, 0, 0);
        // SAFETY: out-pointers to locals.
        let error = unsafe { (self.api().CPXsolninfo)(self.env(), self.lp, &mut method, &mut kind, &mut primal, &mut dual) };
        error == 0 && kind != CPX_NO_SOLN
    }

    fn collect(&self, status: c_int) -> LpSolution {
        let model_status = model_status(status);
        let lp_status = model_status.lp_status();
        let mut out = LpSolution::failed(lp_status, String::new());
        out.model_status = model_status;
        // SAFETY: an open environment and a loaded problem.
        unsafe {
            if self.mip {
                out.iterations = (self.api().CPXgetmipitcnt)(self.env(), self.lp).max(0) as u64;
                out.nodes = (self.api().CPXgetnodecnt)(self.env(), self.lp).max(0) as u64;
            } else {
                out.iterations = (self.api().CPXgetitcnt)(self.env(), self.lp).max(0) as u64;
            }
        }
        if lp_status != LpStatus::Optimal {
            out.message = format!("CPLEX status {}", status_name(status));
        }
        // A MILP stopped at a limit may still hold an incumbent.
        let has_point = lp_status == LpStatus::Optimal || (self.mip && self.has_solution());
        if !has_point {
            return out;
        }
        let Some(x) = self.values(self.api().CPXgetx, self.cols) else { return out };
        out.objective = self.cost.iter().zip(&x).fold(0.0, |s, (c, v)| if *c == 0.0 { s } else { s + c * v });
        if !self.mip && lp_status == LpStatus::Optimal {
            if let (Some(duals), Some(mut reduced)) = (self.values(self.api().CPXgetpi, self.rows), self.values(self.api().CPXgetdj, self.cols)) {
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

impl Drop for CplexInstance {
    fn drop(&mut self) {
        self.free_problem();
        if let Some(env) = self.env.take() {
            self.cplex.checkin(env);
        }
    }
}

impl LpInstance for CplexInstance {
    fn apply(&mut self, options: &SolveOptions) {
        self.settings = SolveOptions { basis: None, interrupt: None, ..options.clone() };
        self.set_interrupt(options.interrupt.clone());
        self.push_settings();
    }

    fn set_interrupt(&mut self, flag: Option<Arc<AtomicBool>>) {
        let (callback, handle): (Option<Callback>, *mut c_void) = match &flag {
            Some(f) => (Some(interrupt_callback), Arc::as_ptr(f) as *mut c_void),
            None => (None, ptr::null_mut()),
        };
        // SAFETY: an open environment; the handle is the flag `self.interrupt`
        // keeps alive while it is registered.
        unsafe {
            (self.api().CPXsetlpcallbackfunc)(self.env(), callback, handle);
            (self.api().CPXsetinfocallbackfunc)(self.env(), callback, handle);
        }
        self.interrupt = flag;
    }

    fn pass_problem(&mut self, p: &LpProblem) -> Result<(), String> {
        p.check().map_err(|(_, message)| message)?;
        self.free_problem();
        let env = self.env();
        if env.is_null() {
            return Err("CPLEX has no environment.".into());
        }
        let (m, n) = (p.rows(), p.cols());
        let mut used = vec![false; m];
        for j in 0..n {
            for (i, v) in p.a.column(j) {
                if *v != 0.0 {
                    used[*i] = true;
                }
            }
        }
        // Rows: a sense, a right-hand side and, for a row bounded on both
        // sides, its range above it. A row with no coefficients, or no finite
        // bound, constrains nothing (glpk.js leaves such rows out).
        let mut sense: Vec<c_char> = Vec::with_capacity(m.max(1));
        let mut rhs: Vec<f64> = Vec::with_capacity(m.max(1));
        let mut range: Vec<f64> = Vec::with_capacity(m.max(1));
        for i in 0..m {
            let (lo, hi) = (p.row_lower[i], p.row_upper[i]);
            let (s, r, g) = if !used[i] || (lo == f64::NEG_INFINITY && hi == f64::INFINITY) {
                (b'L', CPX_INFBOUND, 0.0)
            } else if lo == hi {
                (b'E', lo, 0.0)
            } else if lo == f64::NEG_INFINITY {
                (b'L', hi, 0.0)
            } else if hi == f64::INFINITY {
                (b'G', lo, 0.0)
            } else {
                (b'R', lo, hi - lo)
            };
            sense.push(s as c_char);
            rhs.push(r);
            range.push(g);
        }
        let ranged = sense.contains(&(b'R' as c_char));
        let mut beg: Vec<c_int> = Vec::with_capacity(n.max(1));
        let mut count: Vec<c_int> = Vec::with_capacity(n.max(1));
        let mut index: Vec<c_int> = Vec::with_capacity(p.a.nnz().max(1));
        let mut value: Vec<f64> = Vec::with_capacity(p.a.nnz().max(1));
        for j in 0..n {
            let mut column: Vec<(usize, f64)> = p.a.column(j).iter().filter(|(_, v)| *v != 0.0).copied().collect();
            column.sort_by_key(|(r, _)| *r);
            beg.push(to_int(index.len())?);
            count.push(to_int(column.len())?);
            for (r, v) in column {
                index.push(to_int(r)?);
                value.push(v);
            }
        }
        let infinite = |v: f64| v.clamp(-CPX_INFBOUND, CPX_INFBOUND);
        let mut cost: Vec<f64> = Vec::with_capacity(n.max(1));
        cost.extend_from_slice(&p.cost);
        let mut lb: Vec<f64> = Vec::with_capacity(n.max(1));
        lb.extend(p.col_lower.iter().map(|v| infinite(*v)));
        let mut ub: Vec<f64> = Vec::with_capacity(n.max(1));
        ub.extend(p.col_upper.iter().map(|v| infinite(*v)));
        let mip = p.integer.iter().any(|b| *b);

        let mut status = 0;
        // SAFETY: an open environment and a NUL-terminated name.
        let lp = unsafe { (self.api().CPXcreateprob)(env, &mut status, c"coxeter".as_ptr()) };
        if lp.is_null() {
            return Err(self.cplex.error_message(env, status));
        }
        self.lp = lp;
        // SAFETY: every array has the length CPLEX reads (`n` columns, `m`
        // rows, `index.len()` entries) and outlives the call, which copies them.
        let mut error = unsafe {
            (self.api().CPXcopylp)(
                env,
                lp,
                to_int(n)?,
                to_int(m)?,
                if p.maximize { CPX_MAX } else { CPX_MIN },
                cost.as_ptr(),
                rhs.as_ptr(),
                sense.as_ptr(),
                beg.as_ptr(),
                count.as_ptr(),
                index.as_ptr(),
                value.as_ptr(),
                lb.as_ptr(),
                ub.as_ptr(),
                if ranged { range.as_ptr() } else { ptr::null() },
            )
        };
        if error == 0 && mip {
            let mut kinds: Vec<c_char> = Vec::with_capacity(n.max(1));
            kinds.extend((0..n).map(|j| if p.integer.get(j).copied().unwrap_or(false) { b'I' as c_char } else { b'C' as c_char }));
            // SAFETY: as above, `n` column types.
            error = unsafe { (self.api().CPXcopyctype)(env, lp, kinds.as_ptr()) };
        }
        if error != 0 {
            let message = self.cplex.error_message(env, error);
            self.free_problem();
            return Err(message);
        }
        self.cols = n;
        self.rows = m;
        self.cost = p.cost.clone();
        self.a = p.a.clone();
        self.mip = mip;
        self.cold = false;
        Ok(())
    }

    fn set_costs(&mut self, cost: &[f64]) {
        assert_eq!(cost.len(), self.cols, "objective length");
        if self.cols == 0 || self.lp.is_null() {
            return;
        }
        let indices: Vec<c_int> = (0..self.cols as c_int).collect();
        // SAFETY: a loaded problem and `cols` indices and costs.
        unsafe { (self.api().CPXchgobj)(self.env(), self.lp, self.cols as c_int, indices.as_ptr(), cost.as_ptr()) };
        self.cost.copy_from_slice(cost);
    }

    fn set_cost(&mut self, j: usize, c: f64) {
        assert!(j < self.cols, "column out of range");
        let (index, value) = ([j as c_int], [c]);
        // SAFETY: a loaded problem and one valid column.
        unsafe { (self.api().CPXchgobj)(self.env(), self.lp, 1, index.as_ptr(), value.as_ptr()) };
        self.cost[j] = c;
    }

    fn set_maximize(&mut self, maximize: bool) {
        if self.lp.is_null() {
            return;
        }
        // SAFETY: a loaded problem.
        unsafe { (self.api().CPXchgobjsen)(self.env(), self.lp, if maximize { CPX_MAX } else { CPX_MIN }) };
    }

    fn basis(&self) -> Option<Basis> {
        if self.lp.is_null() || self.mip {
            return None;
        }
        let mut col: Vec<c_int> = vec![0; self.cols.max(1)];
        let mut row: Vec<c_int> = vec![0; self.rows.max(1)];
        // SAFETY: a solved problem and arrays sized to it.
        let error = unsafe { (self.api().CPXgetbase)(self.env(), self.lp, col.as_mut_ptr(), row.as_mut_ptr()) };
        if error != 0 {
            return None;
        }
        col.truncate(self.cols);
        row.truncate(self.rows);
        Some(Basis::new(SolverKind::Cplex, col, row, Vec::new()))
    }

    fn set_basis(&mut self, basis: &Basis) -> Result<(), String> {
        if self.lp.is_null() || basis.solver() != SolverKind::Cplex || basis.cols() != self.cols || basis.rows() != self.rows {
            return Err("basis does not match the problem".into());
        }
        let mut col: Vec<c_int> = basis.col_statuses().to_vec();
        let mut row: Vec<c_int> = basis.row_statuses().to_vec();
        col.push(0);
        row.push(0);
        // SAFETY: a loaded problem and statuses for all its columns and rows
        // (the extra element stands in for an empty array).
        let error = unsafe { (self.api().CPXcopybase)(self.env(), self.lp, col.as_ptr(), row.as_ptr()) };
        if error != 0 {
            return Err(self.cplex.error_message(self.env(), error));
        }
        self.cold = false;
        Ok(())
    }

    fn clear_solver(&mut self) {
        // CPLEX has no reset: the next solve ignores the advanced start
        // instead, unless a basis is given in between.
        self.cold = true;
    }

    fn run(&mut self) -> LpSolution {
        if self.lp.is_null() {
            return LpSolution::failed(LpStatus::IterationLimit, "No problem was passed to CPLEX.");
        }
        if self.interrupt.as_ref().is_some_and(|f| f.load(Ordering::Relaxed)) {
            let mut out = LpSolution::failed(LpStatus::IterationLimit, "CPLEX status interrupted");
            out.model_status = ModelStatus::INTERRUPTED;
            return out;
        }
        let env = self.env();
        // SAFETY: an open environment.
        unsafe { (self.api().CPXsetintparam)(env, CPXPARAM_ADVANCE, if self.cold { 0 } else { 1 }) };
        self.cold = false;
        if let Err(message) = self.optimize() {
            return LpSolution::failed(LpStatus::IterationLimit, message);
        }
        let mut status = self.status();
        // "Infeasible or unbounded" comes from the presolver and is not a
        // verdict on either: the plain simplex tells them apart, as glpk.js
        // retries a GLP_UNDEF without the presolver before believing it.
        if (status == CPX_STAT_INFORUNBD || status == CPXMIP_INFORUNBD) && self.settings.presolve {
            // SAFETY: an open environment.
            unsafe {
                (self.api().CPXsetintparam)(env, CPXPARAM_PRESOLVE, 0);
                (self.api().CPXsetintparam)(env, CPXPARAM_ADVANCE, 0);
            }
            let again = self.optimize();
            // SAFETY: as above.
            unsafe { (self.api().CPXsetintparam)(env, CPXPARAM_PRESOLVE, 1) };
            if let Err(message) = again {
                return LpSolution::failed(LpStatus::IterationLimit, message);
            }
            status = self.status();
        }
        self.collect(status)
    }
}

fn to_int(n: usize) -> Result<c_int, String> {
    c_int::try_from(n).map_err(|_| "problem too large for CPLEX".to_string())
}

/// CPLEX's status in the crate's terms (HiGHS' numbering; see [`ModelStatus`]).
fn model_status(status: c_int) -> ModelStatus {
    match status {
        // Optimal within the gaps asked for: what HiGHS calls optimal too.
        CPX_STAT_OPTIMAL | CPXMIP_OPTIMAL | CPXMIP_OPTIMAL_TOL => ModelStatus::OPTIMAL,
        CPX_STAT_INFEASIBLE | CPXMIP_INFEASIBLE => ModelStatus::INFEASIBLE,
        CPX_STAT_INFORUNBD | CPXMIP_INFORUNBD => ModelStatus::UNBOUNDED_OR_INFEASIBLE,
        CPX_STAT_UNBOUNDED | CPXMIP_UNBOUNDED => ModelStatus::UNBOUNDED,
        CPX_STAT_ABORT_IT_LIM => ModelStatus::ITERATION_LIMIT,
        CPX_STAT_ABORT_TIME_LIM | CPX_STAT_ABORT_DETTIME_LIM | CPXMIP_TIME_LIM_FEAS | CPXMIP_TIME_LIM_INFEAS | CPXMIP_DETTIME_LIM_FEAS
        | CPXMIP_DETTIME_LIM_INFEAS => ModelStatus::TIME_LIMIT,
        CPX_STAT_ABORT_OBJ_LIM | CPX_STAT_ABORT_PRIM_OBJ_LIM | CPX_STAT_ABORT_DUAL_OBJ_LIM => ModelStatus::OBJECTIVE_BOUND,
        CPX_STAT_ABORT_USER | CPXMIP_ABORT_FEAS | CPXMIP_ABORT_INFEAS => ModelStatus::INTERRUPTED,
        // HiGHS reports its node limit as a solution limit too.
        CPXMIP_NODE_LIM_FEAS | CPXMIP_NODE_LIM_INFEAS | CPXMIP_SOL_LIM => ModelStatus::SOLUTION_LIMIT,
        CPXMIP_MEM_LIM_FEAS | CPXMIP_MEM_LIM_INFEAS => ModelStatus::MEMORY_LIMIT,
        CPXMIP_FAIL_FEAS | CPXMIP_FAIL_INFEAS | CPXMIP_FAIL_FEAS_NO_TREE | CPXMIP_FAIL_INFEAS_NO_TREE => ModelStatus::SOLVE_ERROR,
        _ => ModelStatus::UNKNOWN,
    }
}

fn status_name(status: c_int) -> &'static str {
    match status {
        CPX_STAT_OPTIMAL | CPXMIP_OPTIMAL => "optimal",
        CPXMIP_OPTIMAL_TOL => "optimal within tolerance",
        CPX_STAT_INFEASIBLE | CPXMIP_INFEASIBLE => "infeasible",
        CPX_STAT_INFORUNBD | CPXMIP_INFORUNBD => "infeasible or unbounded",
        CPX_STAT_UNBOUNDED | CPXMIP_UNBOUNDED => "unbounded",
        CPX_STAT_OPTIMAL_INFEAS | CPXMIP_OPTIMAL_INFEAS => "optimal with unscaled infeasibilities",
        CPX_STAT_NUM_BEST => "numerical difficulties",
        CPX_STAT_ABORT_IT_LIM => "iteration limit",
        CPX_STAT_ABORT_TIME_LIM | CPXMIP_TIME_LIM_FEAS | CPXMIP_TIME_LIM_INFEAS => "time limit",
        CPX_STAT_ABORT_DETTIME_LIM | CPXMIP_DETTIME_LIM_FEAS | CPXMIP_DETTIME_LIM_INFEAS => "deterministic time limit",
        CPX_STAT_ABORT_OBJ_LIM | CPX_STAT_ABORT_PRIM_OBJ_LIM | CPX_STAT_ABORT_DUAL_OBJ_LIM => "objective limit",
        CPX_STAT_ABORT_USER | CPXMIP_ABORT_FEAS | CPXMIP_ABORT_INFEAS => "interrupted",
        CPXMIP_NODE_LIM_FEAS | CPXMIP_NODE_LIM_INFEAS => "node limit",
        CPXMIP_SOL_LIM => "solution limit",
        CPXMIP_MEM_LIM_FEAS | CPXMIP_MEM_LIM_INFEAS => "memory limit",
        CPXMIP_FAIL_FEAS | CPXMIP_FAIL_INFEAS | CPXMIP_FAIL_FEAS_NO_TREE | CPXMIP_FAIL_INFEAS_NO_TREE => "failed",
        _ => "unknown",
    }
}
