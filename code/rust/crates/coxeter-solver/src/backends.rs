//! Which LP/MILP solver runs the analyses: HiGHS, linked in and always there,
//! or Gurobi or CPLEX from the user's own installation.
//!
//! Gurobi and CPLEX are never linked into Coxeter or shipped with it. Their C
//! library is looked for where its installer puts it, opened at run time
//! ([`crate::gurobi`], [`crate::cplex`]), and the user's licence decides
//! whether it solves. Every backend keeps the contract of [`crate::lp`]: the
//! same problem shape, the same four statuses, duals in the same convention
//! and the same limits, so an analysis does not know which one it runs on.
//!
//! A [`Solver`] is a cheap handle (it clones an `Arc`). [`probe`] finds,
//! opens and checks one solver; [`find_library`] only looks for its file.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::cplex::{Cplex, CplexInstance};
use crate::gurobi::{Gurobi, GurobiInstance};
use crate::lp::{Highs, LpBackend, LpInstance, LpProblem, LpSolution, LpStatus, SolveOptions, highs_version, solve_lp, solve_on};

/// The solvers the app can run on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum SolverKind {
    /// HiGHS, built into the program.
    #[default]
    Highs,
    Gurobi,
    Cplex,
}

impl SolverKind {
    pub const ALL: [SolverKind; 3] = [SolverKind::Highs, SolverKind::Gurobi, SolverKind::Cplex];

    /// The id kept in settings and solutions: `highs`, `gurobi`, `cplex`.
    pub fn id(self) -> &'static str {
        match self {
            SolverKind::Highs => "highs",
            SolverKind::Gurobi => "gurobi",
            SolverKind::Cplex => "cplex",
        }
    }

    /// The name people know it by.
    pub fn name(self) -> &'static str {
        match self {
            SolverKind::Highs => "HiGHS",
            SolverKind::Gurobi => "Gurobi",
            SolverKind::Cplex => "CPLEX",
        }
    }

    /// The solver an id or a name stands for, in any case.
    pub fn parse(s: &str) -> Option<SolverKind> {
        let s = s.trim();
        SolverKind::ALL.into_iter().find(|k| s.eq_ignore_ascii_case(k.id()) || s.eq_ignore_ascii_case(k.name()))
    }

    /// The basis status that means "basic" in this solver's own codes.
    pub(crate) fn basic_status(self) -> i32 {
        match self {
            SolverKind::Highs => highs_sys::kHighsBasisStatusBasic,
            SolverKind::Gurobi => crate::gurobi::GRB_BASIC,
            SolverKind::Cplex => crate::cplex::CPX_BASIC,
        }
    }
}

impl fmt::Display for SolverKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Why a solver cannot be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SolverError {
    /// No library where its installer puts it, nor where the user pointed.
    NotFound(String),
    /// A file that does not open as the solver's library.
    Load(String),
    /// The library opened, but it cannot solve: no valid licence, most often.
    Unusable { version: Option<String>, message: String },
}

impl SolverError {
    /// The solver's version, when its library opened.
    pub fn version(&self) -> Option<&str> {
        match self {
            SolverError::Unusable { version, .. } => version.as_deref(),
            _ => None,
        }
    }
}

impl fmt::Display for SolverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SolverError::NotFound(m) | SolverError::Load(m) => f.write_str(m),
            SolverError::Unusable { message, .. } => f.write_str(message),
        }
    }
}

impl std::error::Error for SolverError {}

/// A solver ready to run: HiGHS, or an opened and licensed Gurobi or CPLEX.
///
/// Cloning shares the opened library. Every solve takes an instance of its
/// own ([`Solver::instance`]), so one handle serves any number of threads.
#[derive(Clone, Default)]
pub struct Solver {
    inner: Inner,
}

#[derive(Clone, Default)]
enum Inner {
    #[default]
    Highs,
    Gurobi(Arc<Gurobi>),
    Cplex(Arc<Cplex>),
}

impl Solver {
    /// HiGHS, built in.
    pub fn highs() -> Solver {
        Solver::default()
    }

    /// Open `kind`'s library from `library`, or from where its installer put
    /// it when `None`, and check that it can solve. HiGHS needs nothing.
    pub fn open(kind: SolverKind, library: Option<&Path>) -> Result<Solver, SolverError> {
        let inner = match kind {
            SolverKind::Highs => Inner::Highs,
            SolverKind::Gurobi => Inner::Gurobi(Arc::new(Gurobi::open(&resolve_library(kind, library)?)?)),
            SolverKind::Cplex => Inner::Cplex(Arc::new(Cplex::open(&resolve_library(kind, library)?)?)),
        };
        Ok(Solver { inner })
    }

    pub fn kind(&self) -> SolverKind {
        match &self.inner {
            Inner::Highs => SolverKind::Highs,
            Inner::Gurobi(_) => SolverKind::Gurobi,
            Inner::Cplex(_) => SolverKind::Cplex,
        }
    }

    /// The version the library reports (`1.11.0`, `12.0.1`, `22.1.0.0`).
    pub fn version(&self) -> String {
        match &self.inner {
            Inner::Highs => highs_version().trim_start_matches('v').to_string(),
            Inner::Gurobi(g) => g.version().to_string(),
            Inner::Cplex(c) => c.version().to_string(),
        }
    }

    /// Name and version, as reports and the methods text cite it.
    pub fn label(&self) -> String {
        format!("{} {}", self.kind().name(), self.version())
    }

    /// The library it was opened from; `None` for HiGHS.
    pub fn library(&self) -> Option<&Path> {
        match &self.inner {
            Inner::Highs => None,
            Inner::Gurobi(g) => Some(g.path()),
            Inner::Cplex(c) => Some(c.path()),
        }
    }

    /// The problem size a restricted licence allows (CPLEX Community, the
    /// Gurobi licence that comes with `pip install gurobipy`), when limited.
    pub fn limit(&self) -> Option<&str> {
        match &self.inner {
            Inner::Highs => None,
            Inner::Gurobi(g) => g.limit(),
            Inner::Cplex(c) => c.limit(),
        }
    }

    /// Whether both handles run the same solver from the same library.
    pub fn same_as(&self, other: &Solver) -> bool {
        self.kind() == other.kind() && self.library() == other.library()
    }

    /// A new instance, for repeated solves of one problem.
    pub fn instance(&self) -> Result<Box<dyn LpInstance>, String> {
        Ok(match &self.inner {
            Inner::Highs => Box::new(Highs::new()),
            Inner::Gurobi(g) => Box::new(GurobiInstance::new(g.clone())?),
            Inner::Cplex(c) => Box::new(CplexInstance::new(c.clone())?),
        })
    }

    /// Solve one problem on a fresh instance (what [`solve_lp`] does on HiGHS).
    pub fn solve(&self, problem: &LpProblem, options: &SolveOptions) -> LpSolution {
        if let Inner::Highs = self.inner {
            return solve_lp(problem, options);
        }
        match self.instance() {
            Ok(mut instance) => solve_on(instance.as_mut(), problem, options),
            Err(message) => LpSolution::failed(LpStatus::IterationLimit, message),
        }
    }
}

impl fmt::Debug for Solver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.inner {
            Inner::Highs => f.write_str("Solver(HiGHS)"),
            Inner::Gurobi(g) => write!(f, "Solver(Gurobi {} at {})", g.version(), g.path().display()),
            Inner::Cplex(c) => write!(f, "Solver(CPLEX {} at {})", c.version(), c.path().display()),
        }
    }
}

impl LpBackend for Solver {
    fn solve(&self, problem: &LpProblem, options: &SolveOptions) -> LpSolution {
        Solver::solve(self, problem, options)
    }
}

// Detection

/// What is known about one solver on this computer.
#[derive(Debug, Clone)]
pub struct SolverInfo {
    pub kind: SolverKind,
    /// The library found or given (`None` for HiGHS, or when none was found).
    pub library: Option<PathBuf>,
    /// The version the library reports, when it opened.
    pub version: Option<String>,
    /// The solver, when it can solve.
    pub solver: Option<Solver>,
    /// Why it cannot, when it cannot.
    pub error: Option<String>,
}

impl SolverInfo {
    /// HiGHS, which is always there.
    pub fn highs() -> SolverInfo {
        let solver = Solver::highs();
        SolverInfo { kind: SolverKind::Highs, library: None, version: Some(solver.version()), solver: Some(solver), error: None }
    }

    pub fn usable(&self) -> bool {
        self.solver.is_some()
    }

    /// The size a restricted licence allows, when it is restricted.
    pub fn limit(&self) -> Option<&str> {
        self.solver.as_ref().and_then(Solver::limit)
    }

    /// Name and version (`Gurobi 12.0.1`), or the name alone when unknown.
    pub fn label(&self) -> String {
        match &self.version {
            Some(v) => format!("{} {v}", self.kind.name()),
            None => self.kind.name().to_string(),
        }
    }
}

/// Find (unless `library` is given), open and check one solver.
///
/// `library` may name the library itself or a folder holding it.
pub fn probe(kind: SolverKind, library: Option<&Path>) -> SolverInfo {
    if kind == SolverKind::Highs {
        return SolverInfo::highs();
    }
    let path = match resolve_library(kind, library) {
        Ok(path) => path,
        Err(error) => return SolverInfo { kind, library: None, version: None, solver: None, error: Some(error.to_string()) },
    };
    match Solver::open(kind, Some(&path)) {
        Ok(solver) => SolverInfo { kind, library: Some(path), version: Some(solver.version()), solver: Some(solver), error: None },
        Err(error) => {
            SolverInfo { kind, library: Some(path), version: error.version().map(str::to_string), solver: None, error: Some(error.to_string()) }
        }
    }
}

/// The library to open: `given` (a file, or a folder holding one), else
/// the first one [`find_library`] finds.
fn resolve_library(kind: SolverKind, given: Option<&Path>) -> Result<PathBuf, SolverError> {
    match given {
        Some(path) if path.is_dir() => newest_in(kind, path)
            .ok_or_else(|| SolverError::NotFound(format!("{} holds no {} library.", path.display(), kind.name()))),
        Some(path) if path.is_file() => Ok(path.to_path_buf()),
        Some(path) => Err(SolverError::NotFound(format!("{} does not exist.", path.display()))),
        None => find_library(kind).ok_or_else(|| SolverError::NotFound(format!("{} was not found on this computer.", kind.name()))),
    }
}

/// `kind`'s library where its installer puts it, without opening it.
pub fn find_library(kind: SolverKind) -> Option<PathBuf> {
    library_candidates(kind).into_iter().next()
}

/// Every library of `kind` found, the likeliest first: the folders the
/// solver's own environment variables name, then the search path, then the
/// default installation folders, newest version first.
pub fn library_candidates(kind: SolverKind) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    match kind {
        SolverKind::Highs => return Vec::new(),
        SolverKind::Gurobi => {
            if let Some(home) = std::env::var_os("GUROBI_HOME") {
                let home = PathBuf::from(home);
                dirs.push(home.join(if cfg!(windows) { "bin" } else { "lib" }));
                dirs.push(home.join("lib"));
            }
        }
        SolverKind::Cplex => {
            // CPLEX_STUDIO_BINARIES<ver> lists folders; CPLEX_STUDIO_DIR<ver>
            // is the installation, newest version first.
            let mut vars: Vec<(String, std::ffi::OsString)> = std::env::vars_os()
                .filter_map(|(k, v)| k.into_string().ok().map(|k| (k, v)))
                .filter(|(k, _)| k.to_ascii_uppercase().starts_with("CPLEX_STUDIO_"))
                .collect();
            vars.sort_by_key(|(name, _)| std::cmp::Reverse(studio_version(name)));
            for (name, value) in &vars {
                let upper = name.to_ascii_uppercase();
                if upper.starts_with("CPLEX_STUDIO_BINARIES") {
                    dirs.extend(std::env::split_paths(value));
                } else if upper.starts_with("CPLEX_STUDIO_DIR") {
                    dirs.extend(cplex_bin_dirs(&PathBuf::from(value)));
                }
            }
        }
    }
    let search = if cfg!(windows) {
        "PATH"
    } else if cfg!(target_os = "macos") {
        "DYLD_LIBRARY_PATH"
    } else {
        "LD_LIBRARY_PATH"
    };
    if let Some(value) = std::env::var_os(search) {
        dirs.extend(std::env::split_paths(&value));
    }
    dirs.extend(default_dirs(kind));

    let mut found: Vec<PathBuf> = Vec::new();
    for dir in dirs {
        if let Some(path) = newest_in(kind, &dir) {
            if !found.iter().any(|f| same_file(f, &path)) {
                found.push(path);
            }
        }
    }
    found
}

/// The installation folders each solver's installer uses by default.
fn default_dirs(kind: SolverKind) -> Vec<PathBuf> {
    let mut roots: Vec<(Vec<u32>, PathBuf)> = Vec::new();
    let mut scan = |parent: &Path, prefix: &str| {
        let Ok(entries) = std::fs::read_dir(parent) else { return };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if name.starts_with(prefix) && entry.path().is_dir() {
                roots.push((studio_version(&name), entry.path()));
            }
        }
    };
    match kind {
        SolverKind::Highs => {}
        SolverKind::Gurobi => {
            if cfg!(windows) {
                scan(Path::new("C:\\"), "gurobi");
            } else if cfg!(target_os = "macos") {
                scan(Path::new("/Library"), "gurobi");
            } else {
                scan(Path::new("/opt"), "gurobi");
            }
        }
        SolverKind::Cplex => {
            if cfg!(windows) {
                for base in ["C:\\Program Files\\IBM\\ILOG", "C:\\Program Files (x86)\\IBM\\ILOG"] {
                    scan(Path::new(base), "cplex_studio");
                }
            } else if cfg!(target_os = "macos") {
                scan(Path::new("/Applications"), "cplex_studio");
            } else {
                scan(Path::new("/opt/ibm/ILOG"), "cplex_studio");
            }
        }
    }
    // Newest installation first.
    roots.sort_by(|a, b| b.0.cmp(&a.0));
    let mut dirs = Vec::new();
    for (_, root) in roots {
        match kind {
            SolverKind::Gurobi => {
                for platform in ["win64", "linux64", "macos_universal2", "mac64", "armlinux64"] {
                    dirs.push(root.join(platform).join(if cfg!(windows) { "bin" } else { "lib" }));
                }
            }
            SolverKind::Cplex => dirs.extend(cplex_bin_dirs(&root)),
            SolverKind::Highs => {}
        }
    }
    dirs
}

/// The folders of a CPLEX Studio installation that hold the library.
fn cplex_bin_dirs(studio: &Path) -> Vec<PathBuf> {
    let bin = studio.join("cplex").join("bin");
    ["x64_win64", "x86-64_linux", "arm64_osx", "x86-64_osx", "ppc64le_linux", "s390x_linux"].iter().map(|p| bin.join(p)).collect()
}

/// The digits in a name (`CPLEX_STUDIO_DIR2211`, `gurobi1201`,
/// `CPLEX_Studio129`), read as a version to sort by, newest largest.
fn studio_version(name: &str) -> Vec<u32> {
    let digits: String = name.chars().filter(char::is_ascii_digit).collect();
    // CPLEX 12.x writes its minor version in two digits (1210 is 12.10).
    if let Some(rest) = digits.strip_prefix("12") {
        return vec![12, rest.parse().unwrap_or(0)];
    }
    digits.parse::<u32>().map(|n| vec![n]).unwrap_or_default()
}

/// The newest library of `kind` in `dir`: the prefix, a version of digits and
/// the platform's suffix, nothing else (`gurobi120.dll`, but neither
/// `gurobi120_light.dll` nor `GurobiJni120.dll`).
fn newest_in(kind: SolverKind, dir: &Path) -> Option<PathBuf> {
    let (prefix, suffix) = library_name_parts(kind)?;
    let entries = std::fs::read_dir(dir).ok()?;
    let mut best: Option<(Vec<u32>, PathBuf)> = None;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        let Some(digits) = name.strip_prefix(prefix).and_then(|n| n.strip_suffix(suffix)) else { continue };
        if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let version = library_version(kind, digits);
        if best.as_ref().is_none_or(|(v, _)| version > *v) {
            best = Some((version, entry.path()));
        }
    }
    best.map(|(_, path)| path)
}

/// The version a library's file name carries: Gurobi writes major and
/// minor (`gurobi95`, `gurobi120`), CPLEX major, minor and patch (`cplex2210`,
/// `cplex12100` for 12.10.0).
fn library_version(kind: SolverKind, digits: &str) -> Vec<u32> {
    let d: Vec<u32> = digits.chars().filter_map(|c| c.to_digit(10)).collect();
    let number = |s: &[u32]| s.iter().fold(0u32, |a, x| a.saturating_mul(10).saturating_add(*x));
    let n = d.len();
    match kind {
        SolverKind::Gurobi if n >= 2 => vec![number(&d[..n - 1]), d[n - 1]],
        SolverKind::Cplex if digits.starts_with("12") && n >= 4 => vec![12, number(&d[2..n - 1]), d[n - 1]],
        SolverKind::Cplex if n >= 3 => vec![number(&d[..2]), number(&d[2..n - 1]), d[n - 1]],
        _ => d,
    }
}

/// File-name prefix and suffix of each solver's library on this platform.
fn library_name_parts(kind: SolverKind) -> Option<(&'static str, &'static str)> {
    let (gurobi, cplex, suffix) = if cfg!(windows) {
        ("gurobi", "cplex", ".dll")
    } else if cfg!(target_os = "macos") {
        ("libgurobi", "libcplex", ".dylib")
    } else {
        ("libgurobi", "libcplex", ".so")
    };
    match kind {
        SolverKind::Highs => None,
        SolverKind::Gurobi => Some((gurobi, suffix)),
        SolverKind::Cplex => Some((cplex, suffix)),
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_parse_from_ids_and_names() {
        assert_eq!(SolverKind::parse("gurobi"), Some(SolverKind::Gurobi));
        assert_eq!(SolverKind::parse(" CPLEX "), Some(SolverKind::Cplex));
        assert_eq!(SolverKind::parse("HiGHS"), Some(SolverKind::Highs));
        assert_eq!(SolverKind::parse("glpk"), None);
        for kind in SolverKind::ALL {
            assert_eq!(SolverKind::parse(kind.id()), Some(kind));
        }
    }

    #[test]
    fn versions_in_folder_names_sort_newest_first() {
        let mut names = vec!["CPLEX_Studio129", "CPLEX_Studio221", "CPLEX_Studio1210", "CPLEX_Studio201"];
        names.sort_by_key(|n| std::cmp::Reverse(studio_version(n)));
        assert_eq!(names, ["CPLEX_Studio221", "CPLEX_Studio201", "CPLEX_Studio1210", "CPLEX_Studio129"]);
        assert!(studio_version("gurobi1301") > studio_version("gurobi952"));
    }

    #[test]
    fn library_names_carry_their_versions() {
        assert_eq!(library_version(SolverKind::Gurobi, "95"), [9, 5]);
        assert_eq!(library_version(SolverKind::Gurobi, "120"), [12, 0]);
        assert_eq!(library_version(SolverKind::Cplex, "2210"), [22, 1, 0]);
        assert_eq!(library_version(SolverKind::Cplex, "12100"), [12, 10, 0]);
        assert_eq!(library_version(SolverKind::Cplex, "1290"), [12, 9, 0]);
        assert!(library_version(SolverKind::Cplex, "2210") > library_version(SolverKind::Cplex, "12100"));
    }

    #[test]
    fn only_the_main_library_is_taken() {
        let dir = std::env::temp_dir().join(format!("coxeter-solver-names-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (prefix, suffix) = library_name_parts(SolverKind::Gurobi).unwrap();
        for name in [format!("{prefix}95{suffix}"), format!("{prefix}95_light{suffix}"), format!("{prefix}jni95{suffix}"), format!("{prefix}120{suffix}")] {
            std::fs::write(dir.join(name), b"").unwrap();
        }
        let found = newest_in(SolverKind::Gurobi, &dir).unwrap();
        assert_eq!(found.file_name().unwrap().to_string_lossy().to_ascii_lowercase(), format!("{prefix}120{suffix}"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_library_is_reported_not_found() {
        let missing = std::env::temp_dir().join("coxeter-no-such-solver").join("gurobi95.dll");
        let info = probe(SolverKind::Gurobi, Some(&missing));
        assert!(!info.usable());
        assert!(info.error.unwrap().contains("does not exist"));
    }
}
