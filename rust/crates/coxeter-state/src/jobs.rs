//! Threads instead of Workers and `await frame()`.
//!
//! Long work runs on a background thread: flux variability (the JS
//! `scheduleFVA` timer and `variabilityAsync`), the SteadyCom range and
//! fixed-µ solves (the JS Worker of `runSteadyComTask`), the analyses the
//! Simulate panel starts (`runAnalysis`, including the loopless MILP), the
//! full-map layout, GIMME and the downloads (Yeast9, the Zuñiga sources). Each job reports progress and can be cancelled.
//!
//! Nothing a job computes touches the store from its thread. Its result comes
//! back as a message and is applied on the caller's thread by
//! [`Store::poll_jobs`], which the application calls once per frame and tests
//! call in a loop (or through [`Store::wait_for_jobs`]). A result is applied
//! only if it is still current, judged by the same generation counters the JS
//! uses (`_fvaGeneration`, `_steadyComGeneration`); a stale one is dropped.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};

use coxeter_map::editable_map::EditableMap;
use coxeter_solver::fba::{FluxResult, FvaRange};
use coxeter_solver::steadycom::SteadyComProgress;
use indexmap::IndexMap;

use crate::error::{Result, StoreError};
use crate::steadycom::{SteadyComJob, SteadyComTaskResult};
use crate::store::Store;

/// What a background job is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JobKind {
    /// Flux variability of every reaction.
    Fva,
    /// A SteadyCom range search or fixed-µ solve.
    SteadyCom,
    /// An analysis started from the Simulate panel (FBA, pFBA, loopless).
    Solve,
    /// The automatic layout of the full map.
    Layout,
    /// Downloading and verifying the Zuñiga case's source models.
    Zuniga,
    /// GIMME on the community LP.
    Gimme,
    /// Downloading the official Yeast9 SBML.
    Download,
}

/// A running job, for a progress display.
#[derive(Debug, Clone, PartialEq)]
pub struct JobInfo {
    pub id: u64,
    pub kind: JobKind,
    pub label: String,
    pub elapsed: Duration,
    /// Work finished and work in total, when the job can count it.
    pub done: usize,
    pub total: usize,
}

/// A job that finished (or was dropped as stale) during a poll.
#[derive(Debug, Clone, PartialEq)]
pub struct JobOutcome {
    pub id: u64,
    pub kind: JobKind,
    /// What the JS promise of the request resolved or rejected with.
    pub result: Result<()>,
}

pub(crate) enum JobMessage {
    FvaProgress { id: u64, reaction_id: String, range: FvaRange, done: usize, total: usize },
    SteadyComProgress { id: u64, progress: SteadyComProgress },
    /// A line for the status bar (the Zuñiga download).
    Progress { id: u64, message: String },
    Done { id: u64, payload: JobPayload },
}

pub(crate) enum JobPayload {
    Fva { ranges: IndexMap<String, FvaRange>, elapsed: Duration },
    SteadyCom(Box<Result<SteadyComTaskResult>>),
    Solve { result: Box<FluxResult>, elapsed: Duration },
    Layout { map: Box<EditableMap>, elapsed: Duration },
    Zuniga(Box<Result<coxeter_analysis::ZunigaStudy>>),
    Gimme(Box<Result<coxeter_analysis::GimmeResult>>),
    Download(Box<Result<crate::assets::FetchResponse>>),
}

/// What the store needs to know to apply a job's result.
pub(crate) enum JobContext {
    Fva { generation: u64, problem_serial: u64 },
    SteadyCom(Box<SteadyComJob>),
    Solve(crate::simulation::SolveJob),
    Layout(crate::map_edit::LayoutJob),
    Zuniga(Box<crate::zuniga::ZunigaJob>),
    Gimme(crate::omics::GimmeJob),
    /// The scenario the download opens.
    Download { scenario: Box<serde_json::Value>, generation: u64, work_generation: u64 },
}

pub(crate) struct JobEntry {
    pub kind: JobKind,
    pub label: String,
    pub cancel: Arc<AtomicBool>,
    pub started: Instant,
    pub done: usize,
    pub total: usize,
    pub ctx: JobContext,
}

pub(crate) struct Jobs {
    tx: Sender<JobMessage>,
    rx: Receiver<JobMessage>,
    next_id: u64,
    pub running: IndexMap<u64, JobEntry>,
}

impl Jobs {
    pub fn new() -> Jobs {
        let (tx, rx) = channel();
        Jobs { tx, rx, next_id: 0, running: IndexMap::new() }
    }

    /// Start `work` on a thread. It is handed its id, a sender for progress
    /// and the result, and its cancel flag.
    pub fn spawn<F>(&mut self, kind: JobKind, label: &str, total: usize, ctx: JobContext, cancel: Arc<AtomicBool>, work: F) -> u64
    where
        F: FnOnce(u64, Sender<JobMessage>, Arc<AtomicBool>) + Send + 'static,
    {
        self.next_id += 1;
        let id = self.next_id;
        let tx = self.tx.clone();
        let flag = cancel.clone();
        self.running.insert(
            id,
            JobEntry { kind, label: label.to_string(), cancel, started: Instant::now(), done: 0, total, ctx },
        );
        std::thread::Builder::new()
            .name(format!("coxeter-{label}"))
            .spawn(move || work(id, tx, flag))
            .expect("the system can start a thread");
        id
    }

    pub fn cancel_all(&mut self) {
        for entry in self.running.values() {
            entry.cancel.store(true, Ordering::SeqCst);
        }
    }

    pub fn is_running(&self, kind: JobKind) -> bool {
        self.running.values().any(|e| e.kind == kind)
    }
}

/// Sleep up to `delay`, waking early when cancelled; true if cancelled.
pub(crate) fn sleep_unless_cancelled(delay: Duration, cancel: &AtomicBool) -> bool {
    let until = Instant::now() + delay;
    while Instant::now() < until {
        if cancel.load(Ordering::SeqCst) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(2).min(until.saturating_duration_since(Instant::now())));
    }
    cancel.load(Ordering::SeqCst)
}

impl Store {
    /// Apply what background jobs have sent since the last call: progress,
    /// and finished results that are still current. Returns the jobs that
    /// finished. Call it once per frame; it never blocks.
    pub fn poll_jobs(&mut self) -> Vec<JobOutcome> {
        let mut messages = Vec::new();
        while let Ok(m) = self.jobs.rx.try_recv() {
            messages.push(m);
        }
        self.apply_messages(messages)
    }

    /// Block until every job has finished (or `timeout` has passed), applying
    /// their results as they arrive. A finished job may start another (a
    /// SteadyCom range then the fixed-µ solve that asked for it); those are
    /// waited for too. Returns every job that finished.
    pub fn wait_for_jobs(&mut self, timeout: Duration) -> Vec<JobOutcome> {
        let until = Instant::now() + timeout;
        let mut out = self.poll_jobs();
        while self.has_pending_job_results() {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            match self.jobs.rx.recv_timeout(left) {
                Ok(first) => {
                    let mut messages = vec![first];
                    while let Ok(m) = self.jobs.rx.try_recv() {
                        messages.push(m);
                    }
                    out.extend(self.apply_messages(messages));
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        out
    }

    /// Block until no job of `kind` is running (or `timeout` has passed),
    /// applying every message as it arrives. Returns every job that
    /// finished meanwhile.
    pub fn wait_for_kind(&mut self, kind: JobKind, timeout: Duration) -> Vec<JobOutcome> {
        let until = Instant::now() + timeout;
        let mut out = self.poll_jobs();
        while self.jobs.is_running(kind) {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            match self.jobs.rx.recv_timeout(left) {
                Ok(first) => {
                    let mut messages = vec![first];
                    while let Ok(m) = self.jobs.rx.try_recv() {
                        messages.push(m);
                    }
                    out.extend(self.apply_messages(messages));
                }
                Err(_) => break,
            }
        }
        out
    }

    /// The jobs still running.
    pub fn running_jobs(&self) -> Vec<JobInfo> {
        self.jobs
            .running
            .iter()
            .filter(|(_, entry)| self.job_is_active(entry))
            .map(|(id, e)| JobInfo {
                id: *id,
                kind: e.kind,
                label: e.label.clone(),
                elapsed: e.started.elapsed(),
                done: e.done,
                total: e.total,
            })
            .collect()
    }

    /// Whether any current background job is still running.
    pub fn has_running_jobs(&self) -> bool {
        self.jobs.running.values().any(|entry| self.job_is_active(entry))
    }

    /// Whether workers still have messages to deliver, including retired requests.
    pub fn has_pending_job_results(&self) -> bool {
        !self.jobs.running.is_empty()
    }

    fn job_is_active(&self, entry: &JobEntry) -> bool {
        if entry.cancel.load(Ordering::SeqCst) {
            return false;
        }
        match &entry.ctx {
            JobContext::Fva { generation, problem_serial } => {
                self.fva_enabled && *generation == self.fva_generation && *problem_serial == self.problem_serial
            }
            JobContext::SteadyCom(job) => {
                job.generation == self.steady_com_generation && job.key == self.steady_com_key()
            }
            JobContext::Solve(job) => {
                job.generation == self.solve_generation && job.problem_serial == self.problem_serial
                    && job.bounds_revision == self.bounds_revision
            }
            JobContext::Layout(job) => {
                job.problem_serial == self.problem_serial && job.map_generation == self.map_history.generation()
                    && job.map_version == self.map_history.version()
            }
            JobContext::Zuniga(job) => job.generation == self.zuniga.generation,
            JobContext::Gimme(job) => {
                job.solve_generation == self.solve_generation && job.problem_serial == self.problem_serial
                    && job.bounds_revision == self.bounds_revision
            }
            JobContext::Download { generation, work_generation, .. } => {
                *generation == self.yeast9_generation && *work_generation == self.work_generation
            }
        }
    }

    /// Whether an analysis or model load still has a result to install.
    pub fn analysis_or_context_running(&self) -> bool {
        for entry in self.jobs.running.values() {
            if entry.kind != JobKind::Layout && self.job_is_active(entry) {
                return true;
            }
        }
        false
    }

    /// Whether a job of this kind is running.
    pub fn job_running(&self, kind: JobKind) -> bool {
        self.jobs.running.values().any(|entry| entry.kind == kind && self.job_is_active(entry))
    }

    /// Cancel one job by id, through the cancel of what it does
    /// (`cancelFVA`, `cancelSteadyCom`, …).
    pub fn cancel_job(&mut self, id: u64) {
        let Some(entry) = self.jobs.running.get(&id) else { return };
        if !self.job_is_active(entry) {
            return;
        }
        let kind = entry.kind;
        match kind {
            JobKind::Fva => self.cancel_fva(false, true),
            JobKind::SteadyCom => self.cancel_steady_com(false),
            JobKind::Solve => self.cancel_background_solve(),
            JobKind::Layout => self.cancel_layout_job(),
            // What arrives is not installed.
            JobKind::Download => {
                let active = self.jobs.running.get(&id).is_some_and(|entry| self.job_is_active(entry));
                if let Some(entry) = self.jobs.running.get(&id) {
                    entry.cancel.store(true, Ordering::SeqCst);
                }
                if active {
                    self.set_status_message(crate::types::StatusState::Ok, "Model download cancelled.");
                    self.emit("status");
                }
            }
            // GIMME's solver finishes independently, but its result is retired now.
            JobKind::Gimme => {
                self.jobs_solve_generation_bump();
                self.set_status_message(crate::types::StatusState::Ok, "GIMME cancelled.");
                self.emit("status");
            }
            // Leaving the case is what cancels its download.
            JobKind::Zuniga => {
                if let Some(entry) = self.jobs.running.get(&id) {
                    entry.cancel.store(true, Ordering::SeqCst);
                }
                self.reset_zuniga_study();
            }
        }
    }

    fn apply_messages(&mut self, messages: Vec<JobMessage>) -> Vec<JobOutcome> {
        let mut out = Vec::new();
        let mut fva_progress = false;
        for message in messages {
            match message {
                JobMessage::FvaProgress { id, reaction_id, range, done, total } => {
                    let ctx = self.jobs.running.get_mut(&id).and_then(|entry| {
                        entry.done = done;
                        entry.total = total;
                        match entry.ctx {
                            JobContext::Fva { generation, problem_serial } => Some((generation, problem_serial, entry.started)),
                            _ => None,
                        }
                    });
                    if let Some((generation, problem_serial, started)) = ctx {
                        fva_progress |= self.apply_fva_progress(generation, problem_serial, reaction_id, range, done, total, started);
                    }
                }
                JobMessage::SteadyComProgress { id, progress } => {
                    let ctx = self.jobs.running.get_mut(&id).and_then(|entry| {
                        entry.done = progress.iterations as usize;
                        match &entry.ctx {
                            JobContext::SteadyCom(job) => Some((job.generation, job.key.clone(), job.state_serial)),
                            _ => None,
                        }
                    });
                    if let Some((generation, key, serial)) = ctx {
                        self.apply_steady_com_progress(generation, &key, serial, progress);
                    }
                }
                JobMessage::Progress { id, message } => {
                    let generation = self.jobs.running.get(&id).and_then(|entry| match &entry.ctx {
                        JobContext::Zuniga(job) => Some(job.generation),
                        _ => None,
                    });
                    if let Some(generation) = generation {
                        self.apply_zuniga_progress(generation, &message);
                    }
                }
                JobMessage::Done { id, payload } => {
                    let Some(entry) = self.jobs.running.shift_remove(&id) else { continue };
                    let kind = entry.kind;
                    let active = self.job_is_active(&entry);
                    let result = match (entry.ctx, payload) {
                        (JobContext::Fva { generation, problem_serial }, JobPayload::Fva { ranges, elapsed }) => {
                            self.apply_fva_done(generation, problem_serial, ranges, elapsed)
                        }
                        (JobContext::SteadyCom(job), JobPayload::SteadyCom(result)) => self.apply_steady_com_done(*job, *result),
                        (JobContext::Solve(job), JobPayload::Solve { result, elapsed }) => {
                            self.apply_solve_job(job, *result, elapsed)
                        }
                        (JobContext::Layout(job), JobPayload::Layout { map, elapsed }) => {
                            self.apply_layout_job(job, *map, elapsed)
                        }
                        (JobContext::Zuniga(job), JobPayload::Zuniga(result)) => self.apply_zuniga_job(*job, *result),
                        (JobContext::Gimme(job), JobPayload::Gimme(result)) => self.apply_gimme_job(job, *result),
                        (JobContext::Download { scenario, .. }, JobPayload::Download(result)) => {
                            if !active {
                                Err(StoreError::aborted("The model download was cancelled or superseded by another context."))
                            } else {
                                self.apply_yeast9_download(*scenario, *result)
                            }
                        }
                        _ => Err(StoreError::msg("A background job returned a result of another kind.")),
                    };
                    out.push(JobOutcome { id, kind, result });
                }
            }
        }
        if fva_progress {
            self.emit("fva-progress");
        }
        out
    }
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use coxeter_core::model::Model;
    use serde_json::{Value, json};
    use crate::omics::CommunityOmicsPatch;
    use crate::types::{RunPhase, StatusState};

    fn make_store() -> Store {
        let mut store = Store::with_dir(coxeter_core::testkit::repo_root());
        store.init().expect("bundled models");
        let mut raw: Value = serde_json::from_str(include_str!("../../../../models/ecoli_core_mc.json")).expect("bundled core");
        for reaction in raw["reactions"].as_array_mut().expect("reactions") {
            if reaction["id"] == "GAPD" {
                reaction["gene_reaction_rule"] = json!("test_gap");
            }
            if reaction["id"] == "PGI" {
                reaction["gene_reaction_rule"] = json!("test_pgi");
            }
        }
        store.models.insert("ecoli_core_mc", Model::from_value(&raw).expect("annotated model"));
        store.use_scenario("solo-ecoli").expect("scenario");
        store.import_metatranscriptomics(
            "MAG,gene,condition,TPM\neco,test_gap,Control,1\neco,test_pgi,Control,8\n", "job-genes.csv",
        ).expect("expression");
        let _ = store.set_community_omics(CommunityOmicsPatch {
            gimme_threshold: Some(Some(5.0)), gimme_objective_fraction: Some(0.8), ..Default::default()
        });
        assert_eq!(store.omics.gimme_threshold, Some(5.0));
        assert_eq!(store.omics.gimme_objective_fraction, 0.8);
        store
    }

    fn latest_job(store: &Store, kind: JobKind) -> u64 {
        store.running_jobs().into_iter().rev().find(|job| job.kind == kind).expect("requested job").id
    }

    fn take_done(store: &Store, id: u64) -> Vec<JobMessage> {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut messages = Vec::new();
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let message = store.jobs.rx.recv_timeout(remaining).expect("worker completion");
            let done = match &message {
                JobMessage::Done { id: completed, .. } => *completed == id,
                _ => false,
            };
            messages.push(message);
            if done {
                return messages;
            }
        }
    }

    fn prepare_steady_range(store: &mut Store) {
        store.steady_com_range(false).expect("permitted range request");
        store.wait_steady_com(Duration::from_secs(20)).expect("permitted range result").expect("permitted range");
    }

    #[test]
    fn steady_cancellation_releases_range_and_fixed_jobs_without_replacing_the_installed_result() {
        for method in ["range", "fixed", "parsimonious"] {
            let mut store = make_store();
            if method != "range" {
                prepare_steady_range(&mut store);
                store.set_steady_com_parsimonious(method == "parsimonious");
            }
            let solution = store.solution.clone().expect("ordinary result");
            let drawing = store.map.as_ref().expect("drawing").to_json();
            if method == "range" {
                store.steady_com_range(true).expect("range request");
            } else {
                store.solve_steady_com_at(0.1).expect("fixed request");
            }
            let id = latest_job(&store, JobKind::SteadyCom);
            let messages = take_done(&store, id);
            store.cancel_job(id);
            assert!(!store.steady_com_running(), "{method}");
            assert!(!store.job_running(JobKind::SteadyCom));
            assert_eq!(store.status().state, StatusState::Ok);
            assert_eq!(store.reproducibility_bundle().expect("cancelled export")["analysis"]["method"], "FBA");
            let outcomes = store.apply_messages(messages);
            assert!(outcomes.iter().all(|outcome| outcome.result.as_ref().is_err_and(|error| error.is_abort())));
            assert!(Arc::ptr_eq(store.solution.as_ref().expect("retained result"), &solution));
            assert_eq!(store.map.as_ref().expect("retained drawing").to_json(), drawing);
        }
    }

    #[test]
    fn steady_cancelled_replacement_keeps_the_previous_fixed_growth_and_composition() {
        for parsimonious in [false, true] {
            let mut store = make_store();
            store.solve_steady_com_at(0.1).expect("first fixed growth");
            store.wait_steady_com(Duration::from_secs(20)).expect("first completion").expect("first result");
            let solution = store.solution.clone().expect("first result");
            let composition: Vec<f64> = store.species.iter().map(|species| species.abundance).collect();
            store.set_steady_com_parsimonious(parsimonious);
            store.solve_steady_com_at(0.2).expect("new fixed growth");
            let id = latest_job(&store, JobKind::SteadyCom);
            let messages = take_done(&store, id);
            store.cancel_job(id);
            assert_eq!(store.state_value()["steadyCom"]["mu"], 0.1);
            store.apply_messages(messages);
            assert!(Arc::ptr_eq(store.solution.as_ref().expect("retained result"), &solution));
            assert_eq!(store.species.iter().map(|species| species.abundance).collect::<Vec<_>>(), composition);
            assert!(!store.solution.as_ref().expect("retained result").steady_com.as_ref().expect("fixed growth").parsimonious);
        }
    }

    #[test]
    fn steady_new_fixed_growth_retires_an_older_analysis_and_its_late_report() {
        for cached in [false, true] {
            for method in ["fba", "pfba", "loopless", "cyclefree", "fva", "pfva"] {
                let mut store = make_store();
                if cached {
                    prepare_steady_range(&mut store);
                }
                store.run_analysis(method).expect("older analysis");
                let older = latest_job(&store, JobKind::Solve);
                let old_messages = take_done(&store, older);
                store.solve_steady_com_at(0.1).expect("newer fixed growth");
                assert!(!store.job_running(JobKind::Solve), "{method}, cached range: {cached}");
                while store.job_running(JobKind::SteadyCom) {
                    let newer = latest_job(&store, JobKind::SteadyCom);
                    let messages = take_done(&store, newer);
                    let outcomes = store.apply_messages(messages);
                    assert!(outcomes.iter().all(|outcome| outcome.result.is_ok()));
                }
                let solution = store.solution.clone().expect("fixed result");
                let status = store.status().clone();
                store.apply_messages(old_messages);
                assert!(Arc::ptr_eq(store.solution.as_ref().expect("retained fixed result"), &solution), "{method}");
                assert_eq!(solution.steady_com.as_ref().expect("fixed growth").mu, 0.1);
                assert!(store.last_run.is_none());
                assert_eq!(store.status(), &status);
            }
        }
    }

    #[test]
    fn steady_new_fixed_growth_retires_partial_fva_and_its_running_report() {
        let mut store = make_store();
        prepare_steady_range(&mut store);
        let older = start_ranges(&mut store, "pfva");
        let mut old_messages = take_done(&store, older);
        let first = old_messages.iter().position(|message| matches!(message, JobMessage::FvaProgress { .. })).expect("progress");
        store.apply_messages(vec![old_messages.remove(first)]);
        store.solve_steady_com_at(0.1).expect("new fixed growth");
        assert!(!store.job_running(JobKind::Fva));
        let newer = latest_job(&store, JobKind::SteadyCom);
        let messages = take_done(&store, newer);
        store.apply_messages(messages);
        let solution = store.solution.clone().expect("fixed result");
        store.apply_messages(old_messages);
        assert!(Arc::ptr_eq(store.solution.as_ref().expect("retained fixed result"), &solution));
        assert!(store.last_run.is_none());
        assert!(!store.fva_enabled);
    }

    #[test]
    fn steady_cancelled_replacement_preserves_previously_completed_fva_and_pfva_ranges() {
        for method in ["fva", "pfva"] {
            let mut store = make_store();
            prepare_steady_range(&mut store);
            store.run_analysis_blocking(method, Duration::from_secs(20)).expect("complete ranges");
            let solution = store.solution.clone().expect("primary result");
            let ranges = store.fva_ranges.clone();
            store.solve_steady_com_at(0.1).expect("new fixed growth");
            let id = latest_job(&store, JobKind::SteadyCom);
            let messages = take_done(&store, id);
            store.cancel_job(id);
            let bundle = store.reproducibility_bundle().expect("retained variability export");
            assert_eq!(bundle["analysis"]["method"], if method == "pfva" { "pFVA" } else { "FBA" });
            assert_eq!(bundle["analysis"]["fvaEnabled"], true);
            assert_eq!(bundle["solution"]["fva"].as_object().expect("archived ranges").len(), ranges.len());
            store.apply_messages(messages);
            assert!(Arc::ptr_eq(store.solution.as_ref().expect("retained result"), &solution));
            assert_eq!(store.fva_ranges, ranges);
        }
    }

    #[test]
    fn steady_new_fixed_growth_retires_a_gimme_worker_and_ignores_its_late_result() {
        let mut store = make_store();
        store.run_community_gimme(CommunityOmicsPatch::default()).expect("installed GIMME");
        store.start_community_gimme(CommunityOmicsPatch::default()).expect("older GIMME");
        let older = latest_job(&store, JobKind::Gimme);
        let old_messages = take_done(&store, older);
        store.solve_steady_com_at(0.1).expect("new fixed growth");
        assert!(!store.job_running(JobKind::Gimme));
        store.cancel_job(older);
        while store.job_running(JobKind::SteadyCom) {
            let id = latest_job(&store, JobKind::SteadyCom);
            let messages = take_done(&store, id);
            let outcomes = store.apply_messages(messages);
            assert!(outcomes.iter().all(|outcome| outcome.result.is_ok()));
        }
        let solution = store.solution.clone().expect("fixed result");
        let status = store.status().clone();
        store.apply_messages(old_messages);
        assert!(Arc::ptr_eq(store.solution.as_ref().expect("retained fixed result"), &solution));
        assert_eq!(solution.steady_com.as_ref().expect("fixed growth").mu, 0.1);
        assert_eq!(store.status(), &status);
        assert!(!store.omics.gimme_enabled);
    }

    #[test]
    fn steady_cancellation_of_an_older_id_cannot_cancel_the_newer_fixed_growth() {
        let mut store = make_store();
        prepare_steady_range(&mut store);
        store.solve_steady_com_at(0.1).expect("older request");
        let older = latest_job(&store, JobKind::SteadyCom);
        let old_messages = take_done(&store, older);
        store.solve_steady_com_at(0.2).expect("newer request");
        let newer = latest_job(&store, JobKind::SteadyCom);
        let messages = take_done(&store, newer);
        store.cancel_job(older);
        let outcomes = store.apply_messages(messages);
        store.apply_messages(old_messages);
        assert!(outcomes.iter().all(|outcome| outcome.result.is_ok()));
        assert_eq!(store.solution.as_ref().expect("new result").steady_com.as_ref().expect("fixed growth").mu, 0.2);
    }

    #[test]
    fn steady_cancelled_range_does_not_continue_into_its_waiting_fixed_growth_request() {
        let mut store = make_store();
        let solution = store.solution.clone().expect("ordinary result");
        store.solve_steady_com_at(0.1).expect("range then fixed request");
        let id = latest_job(&store, JobKind::SteadyCom);
        let messages = take_done(&store, id);
        store.cancel_job(id);
        store.apply_messages(messages);
        assert!(!store.has_running_jobs());
        assert!(Arc::ptr_eq(store.solution.as_ref().expect("retained ordinary result"), &solution));
    }

    #[test]
    fn steady_late_worker_failures_are_aborts_after_cancellation_but_visible_for_current_requests() {
        for cancelled in [false, true] {
            let mut store = make_store();
            store.steady_com_range(false).expect("range request");
            let id = latest_job(&store, JobKind::SteadyCom);
            let _ = take_done(&store, id);
            if cancelled {
                store.cancel_job(id);
            }
            let outcomes = store.apply_messages(vec![JobMessage::Done {
                id,
                payload: JobPayload::SteadyCom(Box::new(Err(StoreError::msg("The earlier worker failed.")))),
            }]);
            let error = outcomes[0].result.as_ref().expect_err("worker failure");
            assert_eq!(error.is_abort(), cancelled);
            assert_eq!(store.status().state, if cancelled { StatusState::Ok } else { StatusState::Error });
        }
    }

    fn start_ranges(store: &mut Store, method: &str) -> u64 {
        store.run_analysis(method).expect("variability analysis");
        let solve = latest_job(store, JobKind::Solve);
        let messages = take_done(store, solve);
        let outcomes = store.apply_messages(messages);
        assert!(outcomes.iter().all(|outcome| outcome.result.is_ok()));
        latest_job(store, JobKind::Fva)
    }

    #[test]
    fn fva_cancellation_before_delivery_finishes_the_report_and_permits_primary_result_export() {
        for method in ["fva", "pfva"] {
            let mut store = make_store();
            let id = start_ranges(&mut store, method);
            let messages = take_done(&store, id);
            let solution = store.solution.clone().expect("primary result");
            store.cancel_job(id);
            assert_eq!(store.last_run.as_ref().expect("report").phase, RunPhase::Cancelled, "{method}");
            assert_eq!(store.status().state, StatusState::Ok);
            let bundle = store.reproducibility_bundle().expect("cancelled variability export");
            assert_eq!(bundle["analysis"]["method"], if method == "pfva" { "pFBA" } else { "FBA" });
            assert_eq!(bundle["analysis"]["fvaEnabled"], false);
            assert!(bundle["solution"]["fva"].is_null());
            store.apply_messages(messages);
            assert!(Arc::ptr_eq(store.solution.as_ref().expect("retained primary"), &solution));
            assert_eq!(store.last_run.as_ref().expect("retained report").phase, RunPhase::Cancelled);
        }
    }

    #[test]
    fn fva_cancellation_preserves_partial_visible_ranges_without_archiving_them_as_complete() {
        for method in ["fva", "pfva"] {
            let mut store = make_store();
            let id = start_ranges(&mut store, method);
            let mut messages = take_done(&store, id);
            let first = messages.iter().position(|message| matches!(message, JobMessage::FvaProgress { .. })).expect("progress");
            let progress = messages.remove(first);
            store.apply_messages(vec![progress]);
            assert_eq!(store.fva_ranges.len(), 1);
            let solution = store.solution.clone().expect("primary result");
            store.cancel_job(id);
            assert_eq!(store.last_run.as_ref().expect("report").phase, RunPhase::Cancelled, "{method}");
            let bundle = store.reproducibility_bundle().expect("partial variability export");
            assert_eq!(bundle["analysis"]["method"], if method == "pfva" { "pFBA" } else { "FBA" });
            assert_eq!(bundle["analysis"]["fvaEnabled"], false);
            assert!(bundle["solution"]["fva"].is_null());
            store.apply_messages(messages);
            assert_eq!(store.fva_ranges.len(), 1);
            assert!(Arc::ptr_eq(store.solution.as_ref().expect("retained primary"), &solution));
            let mut target = make_store();
            target.import_reproducibility_bundle(&bundle).expect("primary result replay");
            assert!(!target.fva_enabled);
            assert!(target.fva_ranges.is_empty());
            assert_eq!(target.reproducibility_bundle().expect("replayed provenance")["analysis"]["method"], bundle["analysis"]["method"]);
        }
    }

    #[test]
    fn fva_incomplete_results_fail_the_report_and_export_only_the_completed_primary_solution() {
        for method in ["fva", "pfva"] {
            let mut store = make_store();
            let id = start_ranges(&mut store, method);
            let messages = take_done(&store, id);
            let mut partial = Vec::new();
            for message in messages {
                if let JobMessage::Done { id, payload: JobPayload::Fva { ranges, elapsed } } = message {
                    let ranges = ranges.into_iter().take(20).collect();
                    partial.push(JobMessage::Done { id, payload: JobPayload::Fva { ranges, elapsed } });
                }
            }
            store.apply_messages(partial);
            assert_eq!(store.fva_ranges.len(), 20);
            assert_eq!(store.last_run.as_ref().expect("failed report").phase, RunPhase::Failed);
            let bundle = store.reproducibility_bundle().expect("incomplete variability export");
            assert_eq!(bundle["analysis"]["method"], if method == "pfva" { "pFBA" } else { "FBA" });
            assert_eq!(bundle["analysis"]["fvaEnabled"], false);
            assert!(bundle["solution"]["fva"].is_null());
        }
    }

    #[test]
    fn fva_primary_only_pfva_archives_replay_the_objective_interval_rather_than_an_exact_target() {
        for (coefficient, sense) in [(1.0, crate::types::ObjectiveSense::Min), (-1.0, crate::types::ObjectiveSense::Max)] {
            let mut store = make_store();
            for reaction in store.objective_coefficients().keys() {
                store.set_objective_coefficient(reaction, 0.0).expect("clear objective");
            }
            store.set_objective_coefficient("eco@ATPM", coefficient).expect("ATPM objective");
            store.set_objective_sense(sense).expect("objective sense");
            store.set_pfba_biomass_percent(90.0).expect("retention target");
            let id = start_ranges(&mut store, "pfva");
            let messages = take_done(&store, id);
            for message in messages {
                if let JobMessage::Done { id, payload: JobPayload::Fva { ranges, elapsed } } = message {
                    let ranges = ranges.into_iter().take(20).collect();
                    store.apply_messages(vec![JobMessage::Done { id, payload: JobPayload::Fva { ranges, elapsed } }]);
                }
            }
            let objective = store.solution.as_ref().expect("primary result").objective_value;
            assert!((objective - coefficient * 8.39).abs() < 1e-6);
            let bundle = store.reproducibility_bundle().expect("primary export");
            let mut target = make_store();
            target.import_reproducibility_bundle(&bundle).expect("primary replay");
            let replayed_objective = target.solution.as_ref().expect("replayed result").objective_value;
            assert!((replayed_objective - objective).abs() < 1e-6, "{sense:?}: {replayed_objective}");
            assert_eq!(bundle["analysis"]["method"], "pFBA");
            assert_eq!(bundle["analysis"]["pfbaExactObjective"], false);
            let (replayed, report) = target.methods_provenance_report().expect("primary report");
            assert_eq!(replayed["analysis"]["pfbaExactObjective"], false);
            assert!(report.contains("pFBA minimized total absolute flux within"));
            assert!(!report.contains("pFBA fixed the primary objective"));
        }
    }

    #[test]
    fn fva_cancellation_of_an_older_id_cannot_cancel_newer_ranges() {
        let mut store = make_store();
        let older = start_ranges(&mut store, "fva");
        let old_messages = take_done(&store, older);
        let newer = start_ranges(&mut store, "pfva");
        let messages = take_done(&store, newer);
        store.cancel_job(older);
        let outcomes = store.apply_messages(messages);
        store.apply_messages(old_messages);
        assert!(outcomes.iter().all(|outcome| outcome.result.is_ok()));
        assert_eq!(store.last_run.as_ref().expect("new report").phase, RunPhase::Done);
        assert_eq!(store.reproducibility_bundle().expect("new provenance")["analysis"]["method"], "pFVA");
        assert_eq!(store.fva_ranges.len(), store.problem.as_ref().expect("problem").rxn_ids.len());
    }

    #[test]
    fn a_current_gimme_worker_still_installs_its_result_and_releases_export_guards() {
        let mut store = make_store();
        store.start_community_gimme(CommunityOmicsPatch::default()).expect("GIMME");
        let id = latest_job(&store, JobKind::Gimme);
        let messages = take_done(&store, id);
        assert!(store.reproducibility_bundle().is_err());
        let outcomes = store.apply_messages(messages);
        assert!(outcomes.iter().all(|outcome| outcome.result.is_ok()));
        assert!(store.omics.gimme_enabled);
        assert_eq!(store.reproducibility_bundle().expect("completed bundle")["analysis"]["method"], "GIMME");
    }

    #[test]
    fn cancelling_gimme_preserves_the_installed_solution_and_immediately_releases_its_controls() {
        for previous_gimme in [false, true] {
            let mut store = make_store();
            if previous_gimme {
                store.run_community_gimme(CommunityOmicsPatch::default()).expect("previous GIMME");
            }
            let solution = store.solution.clone().expect("installed result");
            store.start_community_gimme(CommunityOmicsPatch::default()).expect("pending GIMME");
            let id = latest_job(&store, JobKind::Gimme);
            let messages = take_done(&store, id);
            store.cancel_job(id);
            let active = store.job_running(JobKind::Gimme);
            let status = store.status().clone();
            let export = store.reproducibility_bundle();
            let outcomes = store.apply_messages(messages);
            assert!(!active, "cancelled GIMME remained active");
            assert_eq!(status.state, StatusState::Ok);
            assert!(status.message.to_lowercase().contains("cancelled"));
            assert!(export.is_ok(), "cancelled GIMME blocked a valid installed result");
            assert!(outcomes.iter().all(|outcome| outcome.result.as_ref().is_err_and(|error| error.is_abort())));
            assert!(Arc::ptr_eq(store.solution.as_ref().expect("retained result"), &solution));
            assert_eq!(store.omics.gimme_enabled, previous_gimme);
        }
    }

    #[test]
    fn return_fba_retires_a_pending_gimme_job_without_waiting_for_its_delivery() {
        let mut store = make_store();
        store.run_community_gimme(CommunityOmicsPatch::default()).expect("previous GIMME");
        store.start_community_gimme(CommunityOmicsPatch::default()).expect("pending GIMME");
        let id = latest_job(&store, JobKind::Gimme);
        let messages = take_done(&store, id);
        store.disable_gimme();
        let solution = store.solution.clone().expect("FBA result");
        let export = store.reproducibility_bundle();
        let active = store.job_running(JobKind::Gimme);
        store.apply_messages(messages);
        assert!(!active, "retired GIMME remained active after Return FBA");
        assert_eq!(export.expect("FBA export")["analysis"]["method"], "FBA");
        assert!(Arc::ptr_eq(store.solution.as_ref().expect("retained FBA"), &solution));
        assert!(!store.omics.gimme_enabled);
    }

    #[test]
    fn opening_another_scenario_retires_gimme_before_its_old_worker_message_arrives() {
        let mut store = make_store();
        store.start_community_gimme(CommunityOmicsPatch::default()).expect("pending GIMME");
        let id = latest_job(&store, JobKind::Gimme);
        let messages = take_done(&store, id);
        store.use_scenario("solo-ecoli").expect("new context");
        let solution = store.solution.clone().expect("new result");
        let export = store.reproducibility_bundle();
        let active = store.job_running(JobKind::Gimme);
        store.apply_messages(messages);
        assert!(!active, "old GIMME still owned the new context");
        assert!(export.is_ok());
        assert!(Arc::ptr_eq(store.solution.as_ref().expect("retained result"), &solution));
    }

    #[test]
    fn cancelling_an_older_gimme_id_cannot_cancel_the_newer_gimme_request() {
        let mut store = make_store();
        store.start_community_gimme(CommunityOmicsPatch::default()).expect("older GIMME");
        let older = latest_job(&store, JobKind::Gimme);
        let old_messages = take_done(&store, older);
        store.start_community_gimme(CommunityOmicsPatch::default()).expect("newer GIMME");
        let newer = latest_job(&store, JobKind::Gimme);
        store.cancel_job(older);
        let messages = take_done(&store, newer);
        let outcomes = store.apply_messages(messages);
        store.apply_messages(old_messages);
        assert!(outcomes.iter().all(|outcome| outcome.result.is_ok()));
        assert!(store.omics.gimme_enabled);
        assert_eq!(store.reproducibility_bundle().expect("newer GIMME export")["analysis"]["method"], "GIMME");
    }

    #[test]
    fn cancelling_an_analysis_immediately_finishes_its_report_without_replacing_the_installed_result() {
        for method in ["fba", "pfba", "loopless"] {
            let mut store = make_store();
            let solution = store.solution.clone().expect("installed result");
            store.run_analysis(method).expect("pending analysis");
            let id = latest_job(&store, JobKind::Solve);
            let messages = take_done(&store, id);
            store.cancel_job(id);
            let phase = store.last_run.as_ref().expect("cancelled report").phase;
            let export = store.reproducibility_bundle();
            store.apply_messages(messages);
            assert_eq!(phase, RunPhase::Cancelled, "{method}");
            assert!(export.is_ok(), "cancelled {method} blocked an installed result");
            assert!(Arc::ptr_eq(store.solution.as_ref().expect("retained result"), &solution));
            assert_eq!(store.last_run.as_ref().expect("retained report").phase, RunPhase::Cancelled);
        }
    }

    #[test]
    fn cancelling_an_older_analysis_id_cannot_cancel_the_newer_analysis() {
        let mut store = make_store();
        store.run_analysis("fba").expect("older analysis");
        let older = latest_job(&store, JobKind::Solve);
        let old_messages = take_done(&store, older);
        store.run_analysis("loopless").expect("newer analysis");
        let newer = latest_job(&store, JobKind::Solve);
        store.cancel_job(older);
        let messages = take_done(&store, newer);
        let outcomes = store.apply_messages(messages);
        store.apply_messages(old_messages);
        assert!(outcomes.iter().all(|outcome| outcome.result.is_ok()));
        assert_eq!(store.last_run.as_ref().expect("newer report").phase, RunPhase::Done);
    }

    #[test]
    fn a_current_layout_worker_installs_one_undo_step_without_changing_the_solution() {
        let mut store = make_store();
        let solution = store.solution.clone().expect("installed result");
        assert!(store.show_full_map_in_background());
        let id = latest_job(&store, JobKind::Layout);
        let messages = take_done(&store, id);
        let outcomes = store.apply_messages(messages);
        assert!(outcomes.iter().all(|outcome| outcome.result.is_ok()));
        assert_eq!(store.map_mode, "full");
        assert!(store.map_history.can_undo());
        assert!(Arc::ptr_eq(store.solution.as_ref().expect("unchanged result"), &solution));
    }

    #[test]
    fn cancelling_a_layout_releases_its_busy_notice_without_changing_the_drawing() {
        let mut store = make_store();
        let map = store.map.as_ref().expect("drawing").to_json();
        assert!(store.show_full_map_in_background());
        let id = latest_job(&store, JobKind::Layout);
        let messages = take_done(&store, id);
        store.cancel_job(id);
        let status = store.status().clone();
        let export = store.reproducibility_bundle();
        store.apply_messages(messages);
        assert_eq!(status.state, StatusState::Ok);
        assert!(status.message.to_lowercase().contains("cancelled"));
        assert!(export.is_ok());
        assert_eq!(store.map.as_ref().expect("retained drawing").to_json(), map);
    }

    #[test]
    fn newer_drawing_actions_retire_a_pending_layout_and_preserve_their_undo_history() {
        for action in ["empty", "overview", "annotation"] {
            let mut store = make_store();
            let solution = store.solution.clone().expect("installed result");
            assert!(store.show_full_map_in_background());
            let id = latest_job(&store, JobKind::Layout);
            let messages = take_done(&store, id);
            match action {
                "empty" => {
                    assert!(store.show_empty_map());
                }
                "overview" => {
                    store.show_core_metabolism_overview(true).expect("overview");
                }
                _ => {
                    store.add_map_text("Keep this annotation", 200.0, 200.0, None).expect("annotation");
                }
            }
            let map = store.map.as_ref().expect("newer drawing").to_json();
            let version = store.map_history.version();
            let active = store.job_running(JobKind::Layout);
            let outcomes = store.apply_messages(messages);
            assert!(outcomes.iter().all(|outcome| outcome.result.as_ref().is_err_and(|error| error.is_abort())), "{action}");
            assert!(!active, "a retired layout remained active after {action}");
            assert_eq!(store.map_history.version(), version, "{action}");
            assert_eq!(store.map.as_ref().expect("retained drawing").to_json(), map, "{action}");
            assert!(Arc::ptr_eq(store.solution.as_ref().expect("unchanged result"), &solution));
        }
    }

    #[test]
    fn cancelling_an_older_layout_id_cannot_cancel_the_newer_layout() {
        let mut store = make_store();
        assert!(store.show_full_map_in_background());
        let older = latest_job(&store, JobKind::Layout);
        let old_messages = take_done(&store, older);
        assert!(store.show_full_map_in_background());
        let newer = latest_job(&store, JobKind::Layout);
        store.cancel_job(older);
        let messages = take_done(&store, newer);
        let outcomes = store.apply_messages(messages);
        store.apply_messages(old_messages);
        assert!(outcomes.iter().all(|outcome| outcome.result.is_ok()));
        assert_eq!(store.map_mode, "full");
    }

    #[test]
    fn beginning_a_drag_retires_layout_before_the_drag_is_committed() {
        let mut store = make_store();
        assert!(store.show_full_map_in_background());
        let id = latest_job(&store, JobKind::Layout);
        let messages = take_done(&store, id);
        let transaction = store.begin_map_edit("Move metabolite");
        store.move_node("met:eco@g6p_c", 250.0, 250.0).expect("moved metabolite");
        let map = store.map.as_ref().expect("dragged drawing").to_json();
        let outcomes = store.apply_messages(messages);
        assert!(outcomes.iter().all(|outcome| outcome.result.as_ref().is_err_and(|error| error.is_abort())));
        assert_eq!(store.map.as_ref().expect("retained drawing").to_json(), map);
        assert!(store.commit_map_edit(transaction));
    }

    #[test]
    fn undo_and_redo_retire_pending_layout_without_losing_the_restored_drawing() {
        for action in ["undo", "redo"] {
            let mut store = make_store();
            store.add_map_text("History annotation", 200.0, 200.0, None).expect("annotation");
            if action == "redo" {
                store.undo_map().expect("undo setup");
            }
            assert!(store.show_full_map_in_background());
            let id = latest_job(&store, JobKind::Layout);
            let messages = take_done(&store, id);
            if action == "undo" {
                store.undo_map().expect("undo drawing");
            } else {
                store.redo_map().expect("redo drawing");
            }
            let map = store.map.as_ref().expect("restored drawing").to_json();
            let version = store.map_history.version();
            let outcomes = store.apply_messages(messages);
            assert!(outcomes.iter().all(|outcome| outcome.result.as_ref().is_err_and(|error| error.is_abort())));
            assert_eq!(store.map.as_ref().expect("retained drawing").to_json(), map);
            assert_eq!(store.map_history.version(), version);
            assert_eq!(store.status().state, StatusState::Ok);
        }
    }

    #[test]
    fn display_only_changes_allow_a_current_layout_to_finish_and_keep_the_new_preference() {
        let mut store = make_store();
        let width = !store.view.width_by_flux;
        assert!(store.show_full_map_in_background());
        let id = latest_job(&store, JobKind::Layout);
        let messages = take_done(&store, id);
        let _ = store.set_view(crate::types::ViewPatch { width_by_flux: Some(width), ..Default::default() });
        let outcomes = store.apply_messages(messages);
        assert!(outcomes.iter().all(|outcome| outcome.result.is_ok()));
        assert_eq!(store.map_mode, "full");
        assert_eq!(store.view.width_by_flux, width);
    }
}
