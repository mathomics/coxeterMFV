//! Solving and everything numeric: `applySolution`, `solve`, `runAnalysis`,
//! the pFBA / loopless / FVA switches, flux variability, species, abundances,
//! the objective, the medium and reaction bounds (the "mutations" section of
//! `store.js`).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use coxeter_analysis::community_view::CommunityView;
use coxeter_core::community::{CrossFeeding, community_exchange, split_id};
use coxeter_core::js;
use coxeter_solver::fba::{FluxProblem, FluxResult, FvaOptions, FvaProgress as SolverFvaProgress, LooplessOptions, PfbaOptions};
use indexmap::IndexMap;

use crate::error::{Result, StoreError};
use crate::jobs::{JobContext, JobKind, JobMessage, JobPayload, sleep_unless_cancelled};
use crate::steadycom::steady_com_cross_feeding;
use crate::store::{AUTO_DRAW_METABOLITE_LIMIT, AUTO_DRAW_REACTION_LIMIT, Store};
use crate::types::*;
use crate::util::{csv, describe_status, format_duration, normalize_pfba_percent};

/// How a solve is posed: plain FBA, pFBA at a biomass percentage, or the
/// loopless MILP.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SolvePlan {
    pub loopless: bool,
    pub parsimonious: bool,
    pub pfba_percent: f64,
}

impl SolvePlan {
    /// The method name the log uses.
    pub fn method(&self) -> &'static str {
        if self.loopless {
            "Loopless FBA"
        } else if self.parsimonious {
            "pFBA"
        } else {
            "FBA"
        }
    }
}

/// Solve one flux problem as `solve()` would.
pub fn compute_solution(problem: &FluxProblem, plan: SolvePlan, interrupt: Option<Arc<AtomicBool>>) -> FluxResult {
    if plan.loopless {
        problem.loopless(&LooplessOptions { interrupt, ..LooplessOptions::default() })
    } else if plan.parsimonious {
        problem.parsimonious(PfbaOptions { fraction: plan.pfba_percent / 100.0, maximize: true })
    } else {
        problem.optimize()
    }
}

/// A solve running in the background for `runAnalysis`.
pub(crate) struct SolveJob {
    pub generation: u64,
    pub problem_serial: u64,
    pub bounds_revision: u64,
    pub plan: SolvePlan,
    pub run_serial: Option<u64>,
    /// FVA follows the solve (the FVA analysis).
    pub then_fva: bool,
}

impl Store {
    /// `applySolution(res, { allowFVA })`: install a solver result and
    /// refresh every state derived from it.
    pub fn apply_solution(&mut self, res: Solution, allow_fva: bool) -> Arc<Solution> {
        self.apply_solution_arc(Arc::new(res), allow_fva)
    }

    /// [`Store::apply_solution`] of a result held elsewhere too (the Zuñiga
    /// case keeps one per scenario): `store.solution` is that same object.
    pub fn apply_solution_arc(&mut self, res: Arc<Solution>, allow_fva: bool) -> Arc<Solution> {
        self.solution = Some(res.clone());
        self.refresh_flux_scale();
        if res.is_optimal() {
            self.cross_feeding = if let Some(sc) = &res.steady_com {
                steady_com_cross_feeding(&sc.spec, &sc.original_fluxes, 1e-6)
            } else if let Some(joint) = self.joint_projection_spec.clone() {
                let abundances = self.community.as_ref().map(|c| c.abundances.clone()).unwrap_or_default();
                let fluxes: IndexMap<String, f64> = res
                    .fluxes
                    .iter()
                    .map(|(id, value)| {
                        let (species, original) = split_id(id);
                        let scale = match species {
                            Some(s) => abundances.get(s).copied().unwrap_or(0.0),
                            None => 1.0,
                        };
                        (original.to_string(), value * scale)
                    })
                    .collect();
                steady_com_cross_feeding(&joint, &fluxes, 1e-6)
            } else {
                // The Zuñiga case's own community nets each member's
                // transfers before pairing them.
                self.community_view().map(|c| c.cross_feeding(&res.fluxes, 1e-5)).unwrap_or_default()
            };
            self.growth = self.community_view().map(|c| c.growth_rates(&res.fluxes)).unwrap_or_default();
            self.refresh_cross_feeding_explorer();
            if allow_fva && self.fva_enabled {
                self.schedule_fva(40.0);
            } else if !allow_fva && self.fva_enabled {
                self.fva_enabled = false;
                self.cancel_fva(true, false);
            }
        } else {
            self.cross_feeding = CrossFeeding::default();
            self.growth = IndexMap::new();
            self.cross_feeding_explorer.index = None;
            self.cross_feeding_explorer.selected_id = None;
            self.cancel_fva(true, false);
        }
        res
    }

    /// The solve the current switches ask for.
    pub fn solve_plan(&self, parsimonious: Option<bool>) -> SolvePlan {
        SolvePlan {
            loopless: self.loopless,
            parsimonious: parsimonious.unwrap_or(self.parsimonious),
            pfba_percent: self.pfba_biomass_percent,
        }
    }

    /// `solve()`: solve and refresh everything derived from the solution.
    pub fn solve(&mut self) -> Option<Arc<Solution>> {
        self.solve_with(None, false)
    }

    /// `solve({ parsimonious, silent })`.
    pub fn solve_with(&mut self, parsimonious: Option<bool>, silent: bool) -> Option<Arc<Solution>> {
        self.problem.as_ref()?;
        if self.is_zuniga_study() {
            return self.solve_zuniga(silent);
        }
        self.cancel_steady_com(true);
        if self.solution.as_ref().is_some_and(|s| s.steady_com.is_some()) {
            if let Err(error) = self.refresh_composition_problem() {
                self.set_status_message(StatusState::Error, error.to_string());
                return None;
            }
        }
        // A background solve still running is superseded by this one.
        self.jobs_solve_generation_bump();
        let plan = self.solve_plan(parsimonious);
        let t = Instant::now();
        let res = compute_solution(self.problem.as_ref()?, plan, None);
        let ms = js::round(t.elapsed().as_secs_f64() * 1000.0);
        Some(self.install_solve_result(res, plan, ms, silent))
    }

    fn install_solve_result(&mut self, res: FluxResult, plan: SolvePlan, ms: f64, silent: bool) -> Arc<Solution> {
        let optimal = res.status.is_optimal();
        let message = if optimal {
            String::new()
        } else if !res.message.is_empty() {
            res.message.clone()
        } else {
            describe_status(res.status.as_str())
        };
        let solution = self.apply_solution(Solution::from_result(res), true);
        let layout_ms = self.status().layout_ms;
        self.set_status(Status {
            state: if optimal { StatusState::Ok } else { StatusState::Error },
            message,
            solve_ms: ms,
            layout_ms,
        });
        if optimal {
            let objective = if solution.objective_value.is_finite() { js::to_fixed(solution.objective_value, 5) } else { "—".into() };
            self.add_log(
                StatusState::Ok,
                &format!("{} solved: optimal, objective {objective}, {} ms", plan.method(), js::number_to_string(ms)),
                LogKind::Solve,
            );
        }
        if !silent {
            self.emit("solution");
        }
        solution
    }

    // Run analysis

    /// `runAnalysis(method)`: run one of the analyses of the Simulate panel,
    /// as its buttons do, and say when it has finished.
    ///
    /// The run is announced first and its end is kept in `last_run` for the
    /// results bar, and goes to the status line and the log. The solve runs
    /// in the background (a loopless MILP or a pFBA on a genome-scale
    /// community takes seconds), so the run is finished by
    /// [`Store::poll_jobs`]; FVA has finished when its ranges are in, not
    /// when it is started.
    pub fn run_analysis(&mut self, method: &str) -> Result<()> {
        let Some(m) = AnalysisMethod::parse(method) else {
            return Err(StoreError::msg(format!("Unknown analysis \"{method}\".")));
        };
        if self.is_zuniga_study() && m != AnalysisMethod::Fba {
            return Ok(());
        }
        if m == AnalysisMethod::Fva && self.solution.as_ref().is_some_and(|s| s.steady_com.is_some()) {
            return Ok(());
        }
        self.next_run_serial += 1;
        let serial = self.next_run_serial;
        self.last_run = Some(LastRun {
            method: m,
            label: m.label(),
            phase: RunPhase::Running,
            started_at: now_ms(),
            t0: Instant::now(),
            outcome: None,
            ms: None,
            took: None,
            finished_at: None,
            objective: None,
            serial,
        });
        self.set_status_message(StatusState::Busy, format!("Running {}…", m.label()));
        self.emit("status");
        if self.is_zuniga_study() {
            self.solve();
            self.finish_run_with_solution();
            return Ok(());
        }

        match m {
            AnalysisMethod::Fba => {
                self.parsimonious = false;
                self.loopless = false;
                self.set_fva(false)?;
                self.start_solve_job(false);
            }
            AnalysisMethod::Pfba => {
                self.loopless = false;
                self.set_fva(false)?;
                self.assert_published_study_constraints()?;
                if self.parsimonious && self.solution.is_some() {
                    self.finish_run_with_solution();
                } else {
                    self.parsimonious = true;
                    self.start_solve_job(false);
                }
            }
            AnalysisMethod::Loopless => {
                self.assert_published_study_constraints()?;
                if self.loopless && self.solution.is_some() {
                    self.finish_run_with_solution();
                } else {
                    self.loopless = true;
                    self.parsimonious = false;
                    if self.fva_enabled {
                        self.set_fva(false)?;
                    }
                    self.start_solve_job(false);
                }
            }
            AnalysisMethod::Fva => {
                self.loopless = false;
                self.parsimonious = false;
                self.start_solve_job(true);
            }
        }
        Ok(())
    }

    /// `runAnalysis` to the end: start it and wait for it (tests and the
    /// command line).
    pub fn run_analysis_blocking(&mut self, method: &str, timeout: Duration) -> Result<()> {
        self.run_analysis(method)?;
        self.wait_for_jobs(timeout);
        Ok(())
    }

    fn finish_run_with_solution(&mut self) {
        let outcome = if self.problem.is_none() {
            "no model loaded".to_string()
        } else {
            self.solution.as_ref().map_or_else(|| "not run".to_string(), |s| s.status.as_str().to_string())
        };
        self.finish_run(&outcome);
    }

    fn start_solve_job(&mut self, then_fva: bool) {
        let Some(problem) = self.problem.clone() else {
            self.finish_run("no model loaded");
            return;
        };
        self.cancel_steady_com(true);
        if self.solution.as_ref().is_some_and(|s| s.steady_com.is_some()) {
            if let Err(error) = self.refresh_composition_problem() {
                self.set_status_message(StatusState::Error, error.to_string());
                self.finish_run("error");
                return;
            }
        }
        let problem = self.problem.clone().unwrap_or(problem);
        let generation = self.jobs_solve_generation_bump();
        let plan = self.solve_plan(None);
        let job = SolveJob {
            generation,
            problem_serial: self.problem_serial,
            bounds_revision: self.bounds_revision,
            plan,
            run_serial: self.last_run.as_ref().map(|r| r.serial),
            then_fva,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        self.jobs.spawn(JobKind::Solve, plan.method(), 0, JobContext::Solve(job), cancel, move |id, tx, cancel| {
            let t = Instant::now();
            let result = compute_solution(&problem, plan, Some(cancel));
            let _ = tx.send(JobMessage::Done { id, payload: JobPayload::Solve { result: Box::new(result), elapsed: t.elapsed() } });
        });
    }

    /// The generation of background solves: a new solve supersedes them.
    fn jobs_solve_generation_bump(&mut self) -> u64 {
        for entry in self.jobs.running.values() {
            if entry.kind == JobKind::Solve {
                entry.cancel.store(true, Ordering::SeqCst);
            }
        }
        self.solve_generation += 1;
        self.solve_generation
    }

    /// Stop a background solve; the run it belongs to ends as cancelled.
    pub fn cancel_background_solve(&mut self) {
        self.jobs_solve_generation_bump();
    }

    pub(crate) fn apply_solve_job(&mut self, job: SolveJob, result: FluxResult, elapsed: Duration) -> Result<()> {
        let current = job.generation == self.solve_generation
            && job.problem_serial == self.problem_serial
            && job.bounds_revision == self.bounds_revision;
        let this_run = self.last_run.as_ref().is_some_and(|r| Some(r.serial) == job.run_serial && r.phase == RunPhase::Running);
        if !current {
            if this_run {
                self.finish_run("cancelled");
            }
            return Err(StoreError::aborted("The analysis was superseded by a newer calculation."));
        }
        let ms = js::round(elapsed.as_secs_f64() * 1000.0);
        self.install_solve_result(result, job.plan, ms, false);
        if job.then_fva {
            self.set_fva(true)?;
            // Its ranges come in later; the FVA job finishes the run then.
            if self.solution.as_ref().is_some_and(|s| s.is_optimal()) && self.fva_progress.running {
                self.set_status_message(StatusState::Busy, "Running FVA…");
                self.emit("status");
                return Ok(());
            }
        }
        if this_run {
            self.finish_run_with_solution();
        }
        Ok(())
    }

    /// `finishRun(outcome)`: close the run `runAnalysis` started, with the
    /// outcome it ended in.
    pub fn finish_run(&mut self, outcome: &str) {
        let Some(run) = self.last_run.as_ref().filter(|r| r.phase == RunPhase::Running) else { return };
        let ms = js::round(run.t0.elapsed().as_secs_f64() * 1000.0);
        let (label, method) = (run.label, run.method);
        let ok = outcome == "optimal";
        let objective = self.solution.as_ref().map(|s| s.objective_value);
        let took = format_duration(ms);
        if let Some(run) = self.last_run.as_mut() {
            run.phase = if ok {
                RunPhase::Done
            } else if outcome == "cancelled" {
                RunPhase::Cancelled
            } else {
                RunPhase::Failed
            };
            run.outcome = Some(outcome.to_string());
            run.ms = Some(ms);
            run.took = Some(took.clone());
            run.finished_at = Some(now_ms());
            run.objective = objective;
        }
        if ok {
            let detail = if method == AnalysisMethod::Fva {
                format!("{} flux ranges", self.fva_ranges.len())
            } else {
                let o = objective.filter(|o| o.is_finite());
                format!("objective {}", o.map_or_else(|| "—".to_string(), |o| js::to_fixed(o, 5)))
            };
            self.set_status_message(StatusState::Ok, format!("{label} completed · {detail} · {took}"));
        } else if outcome == "cancelled" {
            self.set_status_message(StatusState::Ok, format!("{label} cancelled after {took}"));
        } else {
            let why = if self.problem.is_none() {
                "no model is loaded yet.".to_string()
            } else if self.status().state == StatusState::Error && !self.status().message.is_empty() {
                self.status().message.clone()
            } else {
                describe_status(outcome)
            };
            self.set_status_message(StatusState::Error, format!("{label} did not finish: {why}"));
        }
        self.emit("status");
    }

    // Switches

    /// `setParsimonious(enabled)`.
    pub fn set_parsimonious(&mut self, enabled: bool) -> Result<Option<Arc<Solution>>> {
        self.assert_published_study_constraints()?;
        if enabled && self.loopless {
            self.loopless = false;
        }
        if self.parsimonious == enabled && self.solution.is_some() {
            return Ok(self.solution.clone());
        }
        self.parsimonious = enabled;
        Ok(self.solve())
    }

    /// `setPFBABiomassPercent(value)`.
    pub fn set_pfba_biomass_percent(&mut self, value: impl Into<UserNumber>) -> Result<Option<Arc<Solution>>> {
        self.assert_published_study_constraints()?;
        let value = value.into();
        let blank = match &value {
            UserNumber::Empty => true,
            UserNumber::Text(t) => t.trim().is_empty(),
            UserNumber::Number(_) => false,
        };
        if blank {
            return Ok(self.solution.clone());
        }
        let next = normalize_pfba_percent(user_number(&value));
        if (next - self.pfba_biomass_percent).abs() < 1e-12 {
            return Ok(self.solution.clone());
        }
        self.pfba_biomass_percent = next;
        if self.parsimonious && !self.loopless {
            return Ok(self.solve());
        }
        self.emit("pfba-fraction");
        Ok(self.solution.clone())
    }

    /// `setLoopless(enabled)`.
    pub fn set_loopless(&mut self, enabled: bool) -> Result<Option<Arc<Solution>>> {
        self.assert_published_study_constraints()?;
        if self.loopless == enabled && self.solution.is_some() {
            return Ok(self.solution.clone());
        }
        self.loopless = enabled;
        if enabled {
            self.parsimonious = false;
            if self.fva_enabled {
                self.set_fva(false)?;
            }
        }
        Ok(self.solve())
    }

    /// `setFVA(enabled)`: flux variability of every reaction, in the
    /// background, after each solve.
    pub fn set_fva(&mut self, enabled: bool) -> Result<()> {
        self.assert_published_study_constraints()?;
        if enabled && self.solution.as_ref().is_some_and(|s| s.steady_com.is_some()) {
            self.set_status_message(
                StatusState::Warn,
                "Ordinary FVA does not apply to the fixed-μ SteadyCom LP. Run an ordinary analysis first.",
            );
            self.emit("status");
            return Ok(());
        }
        if enabled && self.loopless {
            self.loopless = false;
            self.solve();
        }
        if self.fva_enabled == enabled {
            return Ok(());
        }
        self.fva_enabled = enabled;
        if !enabled {
            self.cancel_fva(true, false);
            self.variability = None;
            self.emit("fva");
            if self.last_run.as_ref().is_some_and(|r| r.method == AnalysisMethod::Fva) {
                self.finish_run("cancelled");
            }
            return Ok(());
        }
        self.schedule_fva(0.0);
        self.emit("fva");
        Ok(())
    }

    /// `cancelFVA({ clear, emit })`.
    pub fn cancel_fva(&mut self, clear: bool, emit: bool) {
        self.fva_generation += 1;
        for entry in self.jobs.running.values() {
            if entry.kind == JobKind::Fva {
                entry.cancel.store(true, Ordering::SeqCst);
            }
        }
        self.fva_progress = FvaProgress::default();
        if clear {
            self.fva_ranges = IndexMap::new();
            self.variability = None;
        }
        if emit {
            self.emit("fva");
        }
    }

    /// `scheduleFVA(delay)`: start flux variability after `delay_ms` (a slider
    /// drag reschedules it on every solve, so only the last one runs).
    pub fn schedule_fva(&mut self, delay_ms: f64) {
        let optimal = self.solution.as_ref().is_some_and(|s| s.is_optimal());
        let Some(problem) = self.problem.clone().filter(|_| self.fva_enabled && optimal) else { return };
        self.fva_generation += 1;
        let generation = self.fva_generation;
        for entry in self.jobs.running.values() {
            if entry.kind == JobKind::Fva {
                entry.cancel.store(true, Ordering::SeqCst);
            }
        }
        let total = problem.rxn_ids.len();
        self.fva_ranges = IndexMap::new();
        self.variability = None;
        self.fva_progress = FvaProgress { running: true, done: 0, total, ms: 0.0 };
        let delay = Duration::from_secs_f64(js::max(0.0, if delay_ms.is_nan() { 0.0 } else { delay_ms }) / 1000.0);
        let ctx = JobContext::Fva { generation, problem_serial: self.problem_serial };
        let cancel = Arc::new(AtomicBool::new(false));
        self.jobs.spawn(JobKind::Fva, "fva", total, ctx, cancel, move |id, tx, cancel| {
            let t = Instant::now();
            if sleep_unless_cancelled(delay, &cancel) {
                let _ = tx.send(JobMessage::Done { id, payload: JobPayload::Fva { ranges: IndexMap::new(), elapsed: t.elapsed() } });
                return;
            }
            let progress_tx = std::sync::Mutex::new(tx.clone());
            let on_progress = move |p: &SolverFvaProgress<'_>| {
                if let Ok(tx) = progress_tx.lock() {
                    let _ = tx.send(JobMessage::FvaProgress {
                        id,
                        reaction_id: p.reaction_id.to_string(),
                        range: p.range,
                        done: p.done,
                        total: p.total,
                    });
                }
            };
            let ids = problem.rxn_ids.clone();
            let ranges = problem.variability(
                &ids,
                &FvaOptions { fraction: 1.0, threads: None, on_progress: Some(&on_progress), cancel: Some(cancel) },
            );
            let _ = tx.send(JobMessage::Done { id, payload: JobPayload::Fva { ranges, elapsed: t.elapsed() } });
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_fva_progress(
        &mut self,
        generation: u64,
        problem_serial: u64,
        reaction_id: String,
        range: FvaRange,
        done: usize,
        total: usize,
        started: Instant,
    ) -> bool {
        if generation != self.fva_generation || problem_serial != self.problem_serial {
            return false;
        }
        self.fva_ranges.insert(reaction_id, range);
        self.variability = self.selected_reaction.as_ref().and_then(|r| self.fva_ranges.get(r).copied());
        self.fva_progress =
            FvaProgress { running: done < total, done, total, ms: js::round(started.elapsed().as_secs_f64() * 1000.0) };
        true
    }

    pub(crate) fn apply_fva_done(
        &mut self,
        generation: u64,
        problem_serial: u64,
        ranges: IndexMap<String, FvaRange>,
        elapsed: Duration,
    ) -> Result<()> {
        if !self.fva_enabled || generation != self.fva_generation || problem_serial != self.problem_serial {
            return Err(StoreError::aborted("The flux variability run was superseded."));
        }
        let total = self.problem.as_ref().map_or(ranges.len(), |p| p.rxn_ids.len());
        self.fva_ranges = ranges;
        self.variability = self.selected_reaction.as_ref().and_then(|r| self.fva_ranges.get(r).copied());
        self.fva_progress = FvaProgress {
            running: false,
            done: self.fva_ranges.len(),
            total,
            ms: js::round(elapsed.as_secs_f64() * 1000.0),
        };
        self.emit("fva");
        if self.last_run.as_ref().is_some_and(|r| r.method == AnalysisMethod::Fva) {
            self.finish_run("optimal");
        }
        Ok(())
    }

    // The community

    /// `setSpeciesEnabled(id, enabled)`.
    pub fn set_species_enabled(&mut self, id: &str, enabled: bool) -> Result<()> {
        self.assert_published_study_constraints()?;
        let Some(index) = self.species.iter().position(|s| s.id == id) else { return Ok(()) };
        if self.species[index].enabled == enabled {
            return Ok(());
        }
        // With no species left there is no community, and the map went with it.
        if !enabled && !self.species.iter().enumerate().any(|(i, other)| i != index && other.enabled) {
            self.set_status_message(StatusState::Warn, "At least one species has to stay enabled.");
            self.emit("species");
            return Ok(());
        }
        let keep_drawing = self.keeps_drawing_on_species_toggle();
        self.species[index].enabled = enabled;
        // Bounds edited on any species are kept. Those of a species that has
        // left are skipped while it is out (applyOverrides sets only what the
        // problem has) and apply again when it is back. Clearing them all
        // would throw away the edits on every species that stayed.
        self.selected_reaction = None;
        self.selected_entity = None;
        self.rebuild(RebuildOptions { relayout: !keep_drawing, ..RebuildOptions::default() })?;
        if keep_drawing && enabled && self.map.as_ref().is_some_and(|m| !m.cells().iter().any(|c| c.id == id)) {
            self.set_status_message(
                StatusState::Warn,
                format!("{id} is simulated again but is not drawn on this hand-built map. Draw its cell with the Cell tool, or use Draw full model."),
            );
            self.emit("status");
        }
        Ok(())
    }

    /// `keepsDrawingOnSpeciesToggle()`: whether switching a species on or off
    /// keeps the map as drawn.
    ///
    /// A drawing made by hand is kept, with its undo history: a species that
    /// leaves stays on it, dimmed and not simulated, and comes back as it
    /// was. Only a map nobody has touched is laid out again, and a
    /// genome-scale model never is, which would take minutes.
    pub fn keeps_drawing_on_species_toggle(&self) -> bool {
        // Edits waiting in Redo are hand-made too: laying the map out again
        // would clear the history and lose them.
        let hand_made = self.map_history.can_undo() || self.map_history.can_redo();
        self.map.is_some() && (hand_made || self.any_species_too_large_to_auto_draw())
    }

    /// `anySpeciesTooLargeToAutoDraw()`: like `tooLargeToAutoDraw`, but over
    /// every species of the community, switched-off ones included.
    pub fn any_species_too_large_to_auto_draw(&self) -> bool {
        self.species.iter().any(|sp| {
            self.models.get(&sp.file).is_some_and(|m| {
                m.reactions.len() > AUTO_DRAW_REACTION_LIMIT || m.metabolites.len() > AUTO_DRAW_METABOLITE_LIMIT
            })
        })
    }

    /// `disabledDrawnSpeciesIds()`: species switched off but still drawn on
    /// the map, dimmed.
    pub fn disabled_drawn_species_ids(&self) -> Vec<String> {
        let drawn: std::collections::HashSet<&str> =
            self.map.as_ref().map(|m| m.cells().iter().map(|c| c.id.as_str()).collect()).unwrap_or_default();
        self.species.iter().filter(|sp| !sp.enabled && drawn.contains(sp.id.as_str())).map(|sp| sp.id.clone()).collect()
    }

    /// `setAbundance(id, value)`: abundance lives in the stoichiometry of the
    /// transfer reactions, so this is structural; the geometry, however,
    /// does not depend on it.
    pub fn set_abundance(&mut self, id: &str, value: impl Into<UserNumber>) -> Result<()> {
        self.assert_published_study_constraints()?;
        let abundance = finite_input(&value.into());
        let (Some(index), Some(abundance)) = (self.species.iter().position(|s| s.id == id), abundance) else {
            return Ok(());
        };
        self.species[index].abundance = js::max(0.0, abundance);
        self.rebuild(RebuildOptions { relayout: false, ..RebuildOptions::default() })
    }

    /// `setObjective(type, target)`.
    pub fn set_objective(&mut self, objective: &str, target: Option<&str>) -> Result<()> {
        self.assert_published_study_constraints()?;
        self.objective = objective.to_string();
        self.product_target = target.map(str::to_string);
        self.rebuild(RebuildOptions { relayout: false, ..RebuildOptions::default() })
    }

    // Medium and bounds

    /// `setMediumLimit(base, value)`: `false` when the field holds no number
    /// (a cleared or half-typed field is not a limit of 0).
    pub fn set_medium_limit(&mut self, base: &str, value: impl Into<UserNumber>) -> Result<bool> {
        self.assert_published_study_constraints()?;
        let Some(limit) = finite_input(&value.into()) else { return Ok(false) };
        self.invalidate_steady_com();
        if self.scenario_published().as_deref() == Some("steadycom-gut-nine") {
            self.overrides.shift_remove(&format!("UT_{base}(u)"));
        }
        self.medium.insert(base.to_string(), limit.abs());
        // Per-reaction overrides are applied after the medium, so an override
        // left on this exchange would swallow the slider and make it look
        // broken. The slider is the more direct expression of intent, so it
        // wins.
        self.overrides.shift_remove(&community_exchange(base));
        self.apply_overrides();
        self.solve();
        Ok(true)
    }

    /// `setReactionBounds(rid, lower, upper)`: change a reaction's bounds
    /// without solving; the caller decides when to solve.
    ///
    /// A side given as nothing, an empty string or anything that is not a
    /// finite number is left as it is: a field the user has just cleared is
    /// not a bound of 0.
    pub fn set_reaction_bounds(&mut self, rid: &str, lower: impl Into<UserNumber>, upper: impl Into<UserNumber>) -> Result<Bounds> {
        self.assert_published_study_constraints()?;
        let current = self.current_reaction_bounds(rid);
        let lower = finite_input(&lower.into());
        let upper = finite_input(&upper.into());
        if lower.is_none() && upper.is_none() {
            return Ok(current);
        }
        self.invalidate_steady_com();
        let mut next = Bounds { lower: lower.unwrap_or(current.lower), upper: upper.unwrap_or(current.upper) };
        if next.lower > next.upper {
            if lower.is_none() {
                next.upper = next.lower;
            } else {
                next.lower = next.upper;
            }
        }
        self.overrides.insert(rid.to_string(), next);
        self.apply_overrides();
        Ok(next)
    }

    /// `defaultReactionBounds(rid)`: the bounds the model gives.
    pub fn default_reaction_bounds(&self, rid: &str) -> Bounds {
        let index = self.problem.as_ref().and_then(|p| p.reaction_index(rid));
        match (index, &self.default_bounds) {
            (Some(j), Some(d)) => Bounds { lower: d.lb[j], upper: d.ub[j] },
            _ => Bounds { lower: 0.0, upper: 0.0 },
        }
    }

    /// `currentReactionBounds(rid)`.
    pub fn current_reaction_bounds(&self, rid: &str) -> Bounds {
        self.overrides.get(rid).copied().unwrap_or_else(|| self.default_reaction_bounds(rid))
    }

    /// `resetReaction(rid)`.
    pub fn reset_reaction(&mut self, rid: &str) -> Result<()> {
        self.assert_published_study_constraints()?;
        self.invalidate_steady_com();
        self.overrides.shift_remove(rid);
        self.apply_overrides();
        self.solve();
        Ok(())
    }

    /// `resetAllReactions()`.
    pub fn reset_all_reactions(&mut self) -> Result<()> {
        self.assert_published_study_constraints()?;
        self.invalidate_steady_com();
        self.overrides.clear();
        self.apply_overrides();
        self.solve();
        Ok(())
    }

    /// `knockout(rid)`.
    pub fn knockout(&mut self, rid: &str) -> Result<()> {
        self.set_reaction_bounds(rid, 0.0, 0.0)?;
        self.solve();
        Ok(())
    }

    // Export

    /// `fluxCSV()`.
    pub fn flux_csv(&self) -> String {
        let mut rows = vec!["reaction,species,local_id,name,subsystem,flux,lower_bound,upper_bound".to_string()];
        let Some(solution) = self.solution.as_ref().filter(|s| s.is_optimal()) else { return rows.join("\n") };
        let Some(community) = self.community.as_ref() else { return rows.join("\n") };
        for (rid, v) in &solution.fluxes {
            let owner = community.reaction_owner.get(rid);
            let rxn = community.merged.reactions.get(rid);
            let raw_id = owner.map_or(rid.as_str(), |o| o.original_id.as_str());
            let species = owner.and_then(|o| o.species_id.clone());
            let specific = solution.steady_com.as_ref().and_then(|sc| {
                if let Some(r) = sc.spec.joint.as_ref().and_then(|j| j.reactions.get(raw_id)) {
                    return Some((r.lower_bound, r.upper_bound));
                }
                sc.spec
                    .members
                    .iter()
                    .find(|m| Some(&m.id) == species.as_ref())
                    .and_then(|m| m.model.as_ref())
                    .and_then(|m| m.reactions.get(raw_id))
                    .map(|r| (r.lower_bound, r.upper_bound))
            });
            let bounds = specific.or_else(|| self.problem.as_ref().and_then(|p| p.get_bounds(rid)));
            let (lower, upper) = bounds.map_or((String::new(), String::new()), |(l, u)| (js::number_to_string(l), js::number_to_string(u)));
            rows.push(
                [
                    csv(rid),
                    csv(species.as_deref().unwrap_or("")),
                    csv(owner.map_or("", |o| o.original_id.as_str())),
                    csv(rxn.map_or("", |r| r.name.as_str())),
                    csv(rxn.map_or("", |r| r.subsystem.as_str())),
                    js::to_fixed(*v, 6),
                    lower,
                    upper,
                ]
                .join(","),
            );
        }
        rows.join("\n")
    }
}
