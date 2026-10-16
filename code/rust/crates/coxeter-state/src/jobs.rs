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
    Download(Box<serde_json::Value>),
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
        while !self.jobs.running.is_empty() {
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

    /// Whether any background job is still running.
    pub fn has_running_jobs(&self) -> bool {
        !self.jobs.running.is_empty()
    }

    /// Whether a job of this kind is running.
    pub fn job_running(&self, kind: JobKind) -> bool {
        self.jobs.is_running(kind)
    }

    /// Cancel one job by id, through the cancel of what it does
    /// (`cancelFVA`, `cancelSteadyCom`, …).
    pub fn cancel_job(&mut self, id: u64) {
        let Some(kind) = self.jobs.running.get(&id).map(|e| e.kind) else { return };
        match kind {
            JobKind::Fva => self.cancel_fva(false, true),
            JobKind::SteadyCom => self.cancel_steady_com(false),
            JobKind::Solve => self.cancel_background_solve(),
            JobKind::Layout => self.cancel_layout_job(),
            // What arrives is not installed.
            JobKind::Download => {
                if let Some(entry) = self.jobs.running.get(&id) {
                    entry.cancel.store(true, Ordering::SeqCst);
                }
            }
            // A newer solve supersedes it; the run itself cannot be stopped.
            JobKind::Gimme => {
                self.solve_generation += 1;
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
                        (JobContext::Download(scenario), JobPayload::Download(result)) => {
                            if entry.cancel.load(Ordering::SeqCst) {
                                Err(StoreError::aborted("The download was cancelled."))
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
