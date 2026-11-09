//! The solvers of this computer, as the Simulation settings page lists them,
//! and the one chosen, remembered between sessions.
//!
//! At start only their libraries are looked for. Opening one and checking its
//! licence can take a while (a licence server may answer slowly), so that runs
//! on a thread of its own: the first time the page is shown, when asked
//! again, and at start when the remembered solver is not HiGHS, which the
//! analyses then switch to once it is ready.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use coxeter_solver::backends::{SolverInfo, SolverKind, find_library, probe};
use indexmap::IndexMap;

/// What the page shows and the settings keep.
pub struct Solvers {
    /// HiGHS, Gurobi and CPLEX, as last found or checked.
    pub infos: Vec<SolverInfo>,
    /// Libraries the user pointed at, by solver.
    pub libraries: IndexMap<SolverKind, PathBuf>,
    /// The solver to switch to once it has been checked.
    pub wanted: Option<SolverKind>,
    /// Whether the libraries have been opened and checked, not only found.
    pub checked: bool,
    /// Why the last check did not finish, when it did not.
    pub failure: Option<String>,
    pending: Option<Receiver<Vec<SolverInfo>>>,
}

impl Solvers {
    /// The solvers found on this computer, none of them opened yet.
    pub fn found(libraries: IndexMap<SolverKind, PathBuf>) -> Solvers {
        let infos = SolverKind::ALL
            .iter()
            .map(|&kind| match kind {
                SolverKind::Highs => SolverInfo::highs(),
                _ => SolverInfo {
                    kind,
                    library: libraries.get(&kind).cloned().or_else(|| find_library(kind)),
                    version: None,
                    solver: None,
                    error: None,
                },
            })
            .collect();
        Solvers { infos, libraries, wanted: None, checked: false, failure: None, pending: None }
    }

    /// Whether a check is running.
    pub fn checking(&self) -> bool {
        self.pending.is_some()
    }

    /// Open and check every solver on a thread of its own.
    pub fn check(&mut self) {
        let libraries = self.libraries.clone();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let infos: Vec<SolverInfo> = SolverKind::ALL.iter().map(|&kind| probe(kind, libraries.get(&kind).map(|p| p.as_path()))).collect();
            let _ = tx.send(infos);
        });
        self.failure = None;
        self.pending = Some(rx);
    }

    /// Open and check every solver now, on this thread (the command line).
    pub fn check_now(&mut self) {
        self.infos = SolverKind::ALL.iter().map(|&kind| probe(kind, self.libraries.get(&kind).map(|p| p.as_path()))).collect();
        self.checked = true;
        self.pending = None;
    }

    /// Take the result of a check that has finished: `true` once, when it has.
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.pending else { return false };
        match rx.try_recv() {
            Ok(infos) => {
                self.infos = infos;
                self.checked = true;
                self.pending = None;
                true
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => {
                self.pending = None;
                self.checked = true;
                self.failure = Some("The check of the solvers stopped before it finished.".into());
                true
            }
        }
    }

    pub fn info(&self, kind: SolverKind) -> Option<&SolverInfo> {
        self.infos.iter().find(|i| i.kind == kind)
    }

    /// One line on a solver for the page: what it is and whether it can be
    /// used (`Gurobi 12.0.1`, `CPLEX 22.1.0.0 · size-limited`, `Gurobi · not found`).
    pub fn option_label(&self, kind: SolverKind) -> String {
        let Some(info) = self.info(kind) else { return kind.name().to_string() };
        if kind == SolverKind::Highs {
            return format!("{} · built in", info.label());
        }
        if info.usable() {
            return match info.limit() {
                Some(_) => format!("{} · size-limited", info.label()),
                None => info.label(),
            };
        }
        if info.library.is_none() {
            return format!("{} · not found", kind.name());
        }
        if !self.checked {
            return format!("{} · {}", kind.name(), if self.checking() { "checking…" } else { "not checked yet" });
        }
        format!("{} · unavailable", info.label())
    }

    /// The sentence under the select on a solver other than HiGHS: where it
    /// is, its limit, or why it cannot be used.
    pub fn detail(&self, kind: SolverKind) -> String {
        let Some(info) = self.info(kind) else { return String::new() };
        let library = info.library.as_ref().map(|p| p.display().to_string());
        match (&info.solver, &info.error, library) {
            (Some(_), _, library) => match info.limit() {
                Some(limit) => format!("{}: {limit}.", info.label()),
                None => format!("{}: {}.", info.label(), library.unwrap_or_default()),
            },
            (None, Some(error), _) if info.library.is_some() => format!("{}: {error}", info.label()),
            (None, _, None) => format!("{} was not found. Install it, or locate its library.", kind.name()),
            (None, _, Some(library)) => format!("{}: found at {library}.", kind.name()),
        }
    }
}
