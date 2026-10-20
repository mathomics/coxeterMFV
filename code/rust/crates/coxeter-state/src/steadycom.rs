//! SteadyCom (the SteadyCom section of `store.js`).
//!
//! SteadyCom asks a different question from the one the map answers by
//! default. The map runs joint FBA with one growth rate shared by every
//! member, picked by the LP. SteadyCom instead treats the growth rate as the
//! thing being searched for: fix µ and the problem becomes an ordinary LP in
//! the fluxes and the abundances, and the community can hold together at any
//! µ up to the largest one that still supports the required biomass. So there
//! is a whole range of admissible growth rates, each with its own flux
//! distribution and its own population composition, and only the top of that
//! range is a single number.
//!
//! This exposes both: the range, and the solve at a growth rate the user
//! picks inside it. Both run as background jobs (the JS runs them in a
//! Worker); [`Store::steady_com_range`] and [`Store::solve_steady_com_at`]
//! start them and [`Store::poll_jobs`] applies their results, guarded by
//! `_steadyComGeneration` and the constraint key so a stale result is
//! dropped.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use coxeter_core::community::{CrossFeeding, CrossFeedingLink, FluxShare, MetaboliteExchange, community_exchange, split_id};
use coxeter_core::js;
use coxeter_solver::backends::Solver;
use coxeter_solver::lp::{LpBackend, LpStatus};
use coxeter_solver::steadycom::{
    AtOptions, AtResult, BiomassTerm, Constraint, ExchangeBounds, Fiber, SteadyComOptions, SteadyComProgress, SteadyComResult,
    SteadyComSpec, SteadyComStatus, max_total_biomass_at, parsimonious_at, steady_com,
};
use coxeter_solver::steadycom_runtime::{
    JointSpecOptions, MemberModel, community_balances, joint_steady_com_spec, steady_com_spec, to_merged_fluxes,
};
use indexmap::IndexMap;
use serde_json::{Value, json};

use crate::error::{Result, StoreError};
use crate::jobs::{JobContext, JobKind, JobMessage, JobPayload};
use crate::store::Store;
use crate::types::*;

/// Which published communities are distributed as SteadyCom problems.
pub const STEADYCOM_COMMUNITIES: [&str; 2] = ["steadycom-ecoli-auxotrophs", "steadycom-gut-nine"];

/// `runSteadyComTask`'s request.
#[derive(Debug, Clone, PartialEq)]
pub enum SteadyComTask {
    /// The admissible µ range (`steadyCom`).
    Range { upper_guess: f64, tolerance: f64, biomass_cap: Option<f64>, time_limit_seconds: f64 },
    /// One feasible composition at fixed µ (`maxTotalBiomassAt`).
    Fixed { mu: f64, time_limit_seconds: f64 },
    /// The least total member flux at fixed µ (`parsimoniousAt`).
    Parsimonious { mu: f64, time_limit_seconds: f64 },
}

impl SteadyComTask {
    /// `request.type`.
    pub fn kind(&self) -> &'static str {
        match self {
            SteadyComTask::Range { .. } => "range",
            SteadyComTask::Fixed { .. } => "fixed",
            SteadyComTask::Parsimonious { .. } => "parsimonious",
        }
    }
}

/// What a task returns.
#[derive(Debug, Clone, PartialEq)]
pub enum SteadyComTaskResult {
    Range(SteadyComResult),
    At(AtResult),
}

/// `{ signal, timeoutMs, onProgress }`, and the solver to run on.
pub struct TaskControls<'a> {
    /// Set when the request is cancelled (`signal.aborted`).
    pub cancel: Arc<AtomicBool>,
    pub timeout: Duration,
    pub on_progress: &'a (dyn Fn(SteadyComProgress) + Send + Sync),
    /// The store's solver.
    pub solver: Solver,
}

/// `runSteadyComTask(spec, request, controls)`. Runs on a job thread. The
/// store's runner can be replaced, as the JS tests replace the function.
pub trait SteadyComRunner: Send + Sync {
    fn run(&self, spec: &SteadyComSpec, task: &SteadyComTask, controls: &TaskControls<'_>) -> Result<SteadyComTaskResult>;
}

/// The runner on the store's solver (`coxeter_solver::steadycom`), with the
/// task timeout.
#[derive(Debug, Clone, Copy, Default)]
pub struct SolverSteadyComRunner;

impl SteadyComRunner for SolverSteadyComRunner {
    fn run(&self, spec: &SteadyComSpec, task: &SteadyComTask, controls: &TaskControls<'_>) -> Result<SteadyComTaskResult> {
        if controls.cancel.load(Ordering::SeqCst) {
            return Err(StoreError::aborted("SteadyCom was cancelled."));
        }
        // The timeout is a watchdog that cancels the solve, as the JS timer
        // rejects the task.
        let timed_out = Arc::new(AtomicBool::new(false));
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        let watchdog = {
            let (cancel, timed_out, timeout) = (controls.cancel.clone(), timed_out.clone(), controls.timeout);
            std::thread::spawn(move || {
                if let Err(std::sync::mpsc::RecvTimeoutError::Timeout) = done_rx.recv_timeout(timeout) {
                    timed_out.store(true, Ordering::SeqCst);
                    cancel.store(true, Ordering::SeqCst);
                }
            })
        };
        let backend: &dyn LpBackend = &controls.solver;
        let at = |time: f64| AtOptions {
            biomass_cap: None,
            time_limit_seconds: Some(time),
            cancel: Some(controls.cancel.clone()),
            backend: Some(backend),
        };
        let result = match task {
            SteadyComTask::Range { upper_guess, tolerance, biomass_cap, time_limit_seconds } => {
                let progress = controls.on_progress;
                steady_com(
                    spec,
                    SteadyComOptions {
                        tolerance: *tolerance,
                        upper_guess: *upper_guess,
                        biomass_cap: *biomass_cap,
                        time_limit_seconds: *time_limit_seconds,
                        on_progress: Some(Box::new(move |p: &SteadyComProgress| progress(*p))),
                        cancel: Some(controls.cancel.clone()),
                        backend: Some(backend),
                        ..SteadyComOptions::default()
                    },
                )
                .map(SteadyComTaskResult::Range)
            }
            SteadyComTask::Fixed { mu, time_limit_seconds } => {
                max_total_biomass_at(spec, *mu, &at(*time_limit_seconds)).map(SteadyComTaskResult::At)
            }
            SteadyComTask::Parsimonious { mu, time_limit_seconds } => {
                parsimonious_at(spec, *mu, &at(*time_limit_seconds)).map(SteadyComTaskResult::At)
            }
        };
        drop(done_tx);
        let _ = watchdog.join();
        let cancelled = controls.cancel.load(Ordering::SeqCst);
        match result {
            Err(_) | Ok(_) if timed_out.load(Ordering::SeqCst) => Err(StoreError::aborted(format!(
                "SteadyCom exceeded its {} ms task timeout.",
                controls.timeout.as_millis()
            ))),
            Ok(_) if cancelled => Err(StoreError::aborted("SteadyCom was cancelled.")),
            other => other.map_err(StoreError::from),
        }
    }
}

/// `_steadyComSpec`: the spec built for a constraint key.
#[derive(Debug, Clone)]
pub struct CachedSpec {
    pub key: String,
    pub community_id: Option<String>,
    pub spec: Arc<SteadyComSpec>,
}

/// What a SteadyCom request did right away.
#[derive(Debug, Clone, PartialEq)]
pub enum SteadyComRequest {
    /// The range for these constraints was known already: nothing to run.
    Cached,
    /// A background job was started; its result arrives through `poll_jobs`.
    Started,
}

/// A SteadyCom job in flight.
pub(crate) struct SteadyComJob {
    pub generation: u64,
    pub key: String,
    pub state_serial: u64,
    pub task: SteadyComTask,
    pub spec: Arc<SteadyComSpec>,
    pub started: Instant,
    pub controller: Arc<AtomicBool>,
    /// A fixed-µ solve waiting for this range (`solveSteadyComAt` without a range).
    pub then_fixed: Option<f64>,
}

/// `staleSteadyComError()`.
fn stale_steady_com_error() -> StoreError {
    StoreError::aborted("The community constraints changed; this calculation was discarded.")
}

/// `steadyComCapIsSafe(spec)`: whether capping the total biomass cannot cut
/// off a feasible point (every community bound admits zero).
pub fn steady_com_cap_is_safe(spec: &SteadyComSpec) -> bool {
    for b in spec.community_exchange_bounds.values() {
        if b.lower.is_some_and(|l| l > 0.0) || b.upper.is_some_and(|u| u < 0.0) {
            return false;
        }
    }
    if let Some(joint) = &spec.joint {
        for reaction in joint.reactions.values() {
            if reaction.species != spec.community_tag() {
                continue;
            }
            if reaction.lower_bound > 0.0 || reaction.upper_bound < 0.0 {
                return false;
            }
        }
        for c in &joint.constraints {
            if c.lo.unwrap_or(f64::NEG_INFINITY) > 0.0 || c.hi.unwrap_or(f64::INFINITY) < 0.0 {
                return false;
            }
        }
    }
    true
}

/// `steadyComCrossFeeding(spec, fluxes, threshold)`: who feeds whom in a
/// SteadyCom (or joint) solution, in the shape `CommunityModel.crossFeeding`
/// gives.
pub fn steady_com_cross_feeding(spec: &SteadyComSpec, fluxes: &IndexMap<String, f64>, threshold: f64) -> CrossFeeding {
    let mut links = Vec::new();
    let mut per_metabolite = Vec::new();
    let (species_exchanges, uptake, export) = if spec.joint.is_some() {
        let b = community_balances(spec, fluxes);
        (b.species_exchanges, b.community_uptake, b.community_export)
    } else {
        let mut species_exchanges: IndexMap<String, IndexMap<String, f64>> = IndexMap::new();
        for member in &spec.members {
            for (base, rid) in &member.exchange_for {
                let value = fluxes.get(&format!("{}|{rid}", member.id)).copied().unwrap_or(0.0);
                species_exchanges.entry(base.clone()).or_default().insert(member.id.clone(), value);
            }
        }
        let mut uptake = IndexMap::new();
        let mut export = IndexMap::new();
        for (base, by_species) in &species_exchanges {
            let net = by_species.values().fold(0.0, |s, v| s + v);
            uptake.insert(base.clone(), js::max(0.0, -net));
            export.insert(base.clone(), js::max(0.0, net));
        }
        (species_exchanges, uptake, export)
    };
    let bases = spec.joint.as_ref().and_then(|j| j.metadata.as_ref()).map(|m| &m.bases);
    for (mid, by_species) in &species_exchanges {
        let base = bases.and_then(|b| b.get(mid)).cloned().unwrap_or_else(|| mid.clone());
        let mut producers = Vec::new();
        let mut consumers = Vec::new();
        for (species_id, value) in by_species {
            if *value > threshold {
                producers.push(FluxShare { species_id: Some(species_id.clone()), flux: *value });
            }
            if *value < -threshold {
                consumers.push(FluxShare { species_id: Some(species_id.clone()), flux: -*value });
            }
        }
        let up = uptake.get(mid).copied().unwrap_or(0.0);
        let out = export.get(mid).copied().unwrap_or(0.0);
        if up > threshold {
            producers.push(FluxShare { species_id: None, flux: up });
        }
        if out > threshold {
            consumers.push(FluxShare { species_id: None, flux: out });
        }
        let output = producers.iter().fold(0.0, |s, p| s + p.flux);
        let input = consumers.iter().fold(0.0, |s, p| s + p.flux);
        let total = js::min(output, input);
        if output > 0.0 && input > 0.0 {
            for p in &producers {
                for q in &consumers {
                    let flux = (p.flux / output) * (q.flux / input) * total;
                    if flux > threshold {
                        links.push(CrossFeedingLink { base: base.clone(), from: p.species_id.clone(), to: q.species_id.clone(), flux });
                    }
                }
            }
        }
        if !producers.is_empty() || !consumers.is_empty() {
            per_metabolite.push(MetaboliteExchange { base, producers, consumers, total });
        }
    }
    links.sort_by(|a, b| b.flux.partial_cmp(&a.flux).unwrap_or(std::cmp::Ordering::Equal));
    CrossFeeding { links, per_metabolite }
}

fn fiber_json(fiber: Option<Fiber>) -> Value {
    fiber.map_or(Value::Null, |f| json!({"bacteroides": f.bacteroides, "faecalibacterium": f.faecalibacterium}))
}

impl Store {
    /// `steadyComSupport(communityId)`: whether a community can be posed as a
    /// SteadyCom problem here.
    ///
    /// Takes an id because the card in the Scenario workspace shows whichever
    /// community is *selected* in the list, which is not always the one that
    /// is open. Asked about the selected one, the answer is about that one.
    pub fn steady_com_support(&self, community_id: Option<&str>) -> SteadyComSupport {
        let active = self.scenario_published().or_else(|| self.scenario_id());
        let community_id = community_id.map(str::to_string).or_else(|| active.clone());
        let refuse = |reason: &str| SteadyComSupport { supported: false, community_id: None, kind: None, reason: reason.into() };
        let Some(id) = community_id.filter(|id| Some(id) == active.as_ref()) else {
            return refuse("Open this community first to load its models.");
        };
        if self.scenario_published().is_some() && !STEADYCOM_COMMUNITIES.contains(&id.as_str()) {
            return refuse("This published example does not supply a validated SteadyCom formulation and its full constraints.");
        }
        if self.community.is_none() || !self.species.iter().any(|sp| sp.enabled) {
            return refuse("No active member models are available.");
        }
        if self.assets.is_embedded() && self.scenario_published().is_some() {
            let manifest = self.published_manifests.get(&id).cloned().flatten();
            let members = manifest.as_ref().and_then(|m| m.get("members")).and_then(Value::as_array).cloned().unwrap_or_default();
            let all = members.iter().all(|m| {
                let model = m.get("model").map(js::to_string).unwrap_or_else(|| "undefined".into());
                self.assets.contains(&format!("models/published/{id}/{model}"))
            });
            if !all {
                return refuse("This older standalone contains only map excerpts. Open the updated standalone or serve the application to calculate growth rates.");
            }
        }
        let kind = if id == "steadycom-gut-nine" { "joint" } else { "separate" };
        SteadyComSupport { supported: true, community_id: Some(id), kind: Some(kind), reason: String::new() }
    }

    /// `steadyComKey()`: everything a computed range depends on.
    pub fn steady_com_key(&self) -> String {
        let mut medium: Vec<(&String, &f64)> = self.medium.iter().collect();
        medium.sort_by(|a, b| js::locale_compare(a.0, b.0));
        let effective = self.effective_overrides();
        let mut overrides: Vec<(&String, &Bounds)> = effective.iter().collect();
        overrides.sort_by(|a, b| js::locale_compare(a.0, b.0));
        js::stringify(
            &json!({
                "community": self.scenario_published().or_else(|| self.scenario_id()),
                "revision": self.steady_com_revision,
                "members": self.species.iter().map(|sp| json!([sp.id, sp.file, sp.enabled])).collect::<Vec<_>>(),
                "medium": medium.iter().map(|(k, v)| json!([k, crate::util::num_json(**v)])).collect::<Vec<_>>(),
                "overrides": overrides.iter().map(|(k, b)| json!([k, b.to_json()])).collect::<Vec<_>>(),
                "fiber": fiber_json(self.steady_com_config.fiber),
            }),
            None,
        )
    }

    /// Whether a SteadyCom task is in flight.
    pub fn steady_com_running(&self) -> bool {
        self.steady_com_controller.is_some()
    }

    /// `cancelSteadyCom({ silent })`.
    pub fn cancel_steady_com(&mut self, silent: bool) {
        let running = self.steady_com_controller.is_some();
        self.steady_com_generation += 1;
        if let Some(c) = self.steady_com_controller.take() {
            c.store(true, Ordering::SeqCst);
        }
        if let Some(state) = self.steady_com_state.as_mut() {
            state.phase = None;
        }
        if running && !silent {
            self.set_status_message(StatusState::Ok, "SteadyCom calculation cancelled.");
            self.emit("steadycom");
        }
    }

    /// `invalidateSteadyCom()`: the constraints changed; what was computed
    /// for the old ones is gone, and so is a fixed-µ solution.
    pub fn invalidate_steady_com(&mut self) {
        self.cancel_steady_com(true);
        self.steady_com_revision += 1;
        self.steady_com_state = None;
        self.steady_com_spec = None;
        if self.solution.as_ref().is_some_and(|s| s.steady_com.is_some()) {
            self.solution = None;
            self.refresh_flux_scale();
            self.cross_feeding = CrossFeeding::default();
            self.growth = IndexMap::new();
            self.cross_feeding_explorer.index = None;
        }
    }

    /// `setSteadyComFiber(value)`: the Figure 4 fiber limit of the
    /// nine-species example (`None` for the baseline).
    pub fn set_steady_com_fiber(&mut self, value: Option<f64>) -> Result<()> {
        if self.scenario_published().as_deref() != Some("steadycom-gut-nine") {
            return Err(StoreError::msg("Fiber conditions apply to the nine-species example."));
        }
        if let Some(rate) = value {
            if !rate.is_finite() || !(1.0..=30.0).contains(&rate) {
                return Err(StoreError::msg("Choose a B. thetaiotaomicron fiber limit between 1 and 30 C-mmol gDW⁻¹ h⁻¹."));
            }
        }
        self.invalidate_steady_com();
        self.steady_com_config.fiber = value.map(|rate| Fiber { bacteroides: rate, faecalibacterium: 5.0 });
        self.set_status_message(StatusState::Ok, "Condition changed. Calculate the permitted growth-rate range again.");
        self.emit("steadycom");
        Ok(())
    }

    /// `setSteadyComParsimonious(on)`: whether a fixed-µ solve returns the
    /// cheapest flux distribution (least total member flux) or the first
    /// feasible one. It does not change the µ range, so the range stays
    /// valid; only the next solve reads it.
    pub fn set_steady_com_parsimonious(&mut self, on: bool) {
        self.steady_com_config.parsimonious = on;
        let differs = self
            .solution
            .as_ref()
            .and_then(|s| s.steady_com.as_ref())
            .is_some_and(|sc| sc.parsimonious != self.steady_com_config.parsimonious);
        if differs {
            self.set_status_message(StatusState::Ok, "Solve at this µ again to apply the change.");
        }
        self.emit("steadycom");
    }

    /// `steadyComProblem({ recompute })`: the SteadyCom spec for the current
    /// constraints, built from copies of the member models (the catalogue is
    /// never edited), cached by constraint key.
    pub fn steady_com_problem(&mut self, recompute: bool) -> Result<Arc<SteadyComSpec>> {
        let support = self.steady_com_support(None);
        if !support.supported {
            return Err(StoreError::msg(support.reason));
        }
        let key = self.steady_com_key();
        if !recompute {
            if let Some(cached) = self.steady_com_spec.as_ref().filter(|c| c.key == key) {
                return Ok(cached.spec.clone());
            }
        }
        let joint = support.kind == Some("joint");
        let community_id = support.community_id.clone().unwrap_or_default();
        let mut members: Vec<(String, coxeter_core::model::Model)> = Vec::new();
        for sp in self.species.clone() {
            if !sp.enabled && !joint {
                continue;
            }
            let model = self.load_model(&sp.file)?;
            members.push((sp.id.clone(), (*model).clone()));
        }
        let spec = if joint {
            let members: Vec<MemberModel> = members.into_iter().map(|(id, m)| MemberModel::new(&id, m)).collect();
            let bounds = self.assets.read_text(&format!("models/published/{community_id}/bounds.csv"))?;
            let mut spec = joint_steady_com_spec(
                &members,
                &bounds,
                JointSpecOptions { fiber: self.steady_com_config.fiber, ..JointSpecOptions::default() },
            )?;
            let disabled: Vec<String> = self.species.iter().filter(|sp| !sp.enabled).map(|sp| sp.id.clone()).collect();
            let medium = self.medium.clone();
            let overrides = self.effective_overrides();
            if let Some(joint) = spec.joint.as_mut() {
                for id in disabled {
                    joint.constraints.push(Constraint {
                        name: Some(format!("disabled|{id}")),
                        terms: Vec::new(),
                        biomass: vec![BiomassTerm { species: id, coef: 1.0 }],
                        lo: Some(0.0),
                        hi: Some(0.0),
                    });
                }
                let uptake: Vec<(String, String)> = joint
                    .metadata
                    .as_ref()
                    .map(|m| m.community_uptake.iter().map(|(mid, rid)| (m.bases.get(mid).cloned().unwrap_or_default(), rid.clone())).collect())
                    .unwrap_or_default();
                for (base, rid) in uptake {
                    if let (Some(limit), Some(reaction)) = (medium.get(&base), joint.reactions.get_mut(&rid)) {
                        reaction.upper_bound = *limit;
                    }
                }
                let tag = joint.community_tag.clone();
                for (id, b) in &overrides {
                    let (species, rid) = split_id(id);
                    if let Some(reaction) = joint.reactions.get_mut(rid) {
                        let ok = match species {
                            None => reaction.species == tag,
                            Some(s) => reaction.species == s,
                        };
                        if ok {
                            reaction.lower_bound = b.lower;
                            reaction.upper_bound = b.upper;
                        }
                    }
                }
            }
            spec
        } else {
            // Member overrides go on the member's own copy of its model.
            let overrides = self.effective_overrides();
            for (id, b) in &overrides {
                let (Some(species), rid) = split_id(id) else { continue };
                if let Some((_, model)) = members.iter_mut().find(|(mid, _)| mid == species) {
                    if let Some(reaction) = model.reactions.get_mut(rid) {
                        reaction.lower_bound = b.lower;
                        reaction.upper_bound = b.upper;
                    }
                }
            }
            let members: Vec<MemberModel> = members.into_iter().map(|(id, m)| MemberModel::new(&id, m)).collect();
            let mut spec = steady_com_spec(&members, &self.medium, 1.0)?;
            let bases: indexmap::IndexSet<String> =
                spec.members.iter().flat_map(|m| m.exchange_for.keys().cloned()).collect();
            for (id, b) in &overrides {
                if split_id(id).0.is_some() {
                    continue;
                }
                for base in &bases {
                    if community_exchange(base) != *id {
                        continue;
                    }
                    spec.community_exchange_bounds.insert(base.clone(), ExchangeBounds { lower: Some(b.lower), upper: Some(b.upper) });
                    spec.community_uptake.insert(base.clone(), js::max(0.0, -b.lower));
                }
            }
            spec
        };
        let spec = Arc::new(spec);
        self.steady_com_spec = Some(CachedSpec { key, community_id: support.community_id, spec: spec.clone() });
        Ok(spec)
    }

    /// `steadyComRange({ recompute })`: the permitted µ range for the current
    /// constraints. Returns [`SteadyComRequest::Cached`] when it is known;
    /// otherwise starts the search in the background and returns
    /// [`SteadyComRequest::Started`].
    pub fn steady_com_range(&mut self, recompute: bool) -> Result<SteadyComRequest> {
        self.start_steady_com_range(recompute, None)
    }

    fn cached_range(&self) -> Option<&SteadyComState> {
        let key = self.steady_com_key();
        self.steady_com_state.as_ref().filter(|s| s.key == key && s.max.is_some_and(f64::is_finite))
    }

    fn start_steady_com_range(&mut self, recompute: bool, then_fixed: Option<f64>) -> Result<SteadyComRequest> {
        let key = self.steady_com_key();
        if !recompute && self.cached_range().is_some() {
            return Ok(SteadyComRequest::Cached);
        }
        self.cancel_steady_com(true);
        let generation = self.steady_com_generation;
        let controller = Arc::new(AtomicBool::new(false));
        self.steady_com_controller = Some(controller.clone());
        let id = self.scenario_published().or_else(|| self.scenario_id());
        let serial = self.next_serial();
        self.steady_com_state = Some(SteadyComState {
            serial,
            community_id: id.clone(),
            key: key.clone(),
            min: None,
            max: None,
            source: "computed".into(),
            phase: Some("range".into()),
            progress: None,
            mu: None,
            feasible: None,
            abundances: IndexMap::new(),
            upper_bound: None,
            resolution: None,
            iterations: None,
            seconds: None,
            error: None,
            parsimonious: false,
            total_biomass: None,
            original_fluxes: None,
            total_flux: None,
            solve_ms: None,
        });
        self.set_status_message(StatusState::Busy, "Calculating permitted community growth rates…");
        self.emit("steadycom");
        let started = Instant::now();
        let spec = match self.steady_com_problem(recompute) {
            Ok(spec) => spec,
            Err(error) => {
                self.fail_steady_com(generation, serial, &controller, &error);
                return Err(error);
            }
        };
        // Near the top of the range the nine-species LPs get slow: with the
        // Figure 4 fiber limit (Bt 4) the step at 0.0575 h⁻¹ takes more than
        // 30 s, which would fail the whole search. Those steps get more time,
        // and the task as a whole too.
        let nine = id.as_deref() == Some("steadycom-gut-nine");
        let task = SteadyComTask::Range {
            upper_guess: if nine { 0.1 } else { 1.0 },
            tolerance: 1e-5,
            biomass_cap: steady_com_cap_is_safe(&spec).then(|| 2.0 * spec.total_biomass),
            time_limit_seconds: if nine { 180.0 } else { 30.0 },
        };
        let timeout = Duration::from_millis(if nine { 1_800_000 } else { 300_000 });
        self.spawn_steady_com(SteadyComJob {
            generation,
            key,
            state_serial: serial,
            task,
            spec,
            started,
            controller,
            then_fixed,
        }, timeout);
        Ok(SteadyComRequest::Started)
    }

    /// `solveSteadyComAt(mu)`: one feasible composition and flux distribution
    /// at a growth rate inside the computed range. Without a range for the
    /// current constraints, the range is computed first.
    pub fn solve_steady_com_at(&mut self, mu: f64) -> Result<SteadyComRequest> {
        if !mu.is_finite() || mu < 0.0 {
            return Err(StoreError::msg("Enter a finite growth rate of zero or more."));
        }
        if self.cached_range().is_none() {
            self.start_steady_com_range(false, Some(mu))?;
            return Ok(SteadyComRequest::Started);
        }
        self.start_steady_com_fixed(mu)
    }

    fn start_steady_com_fixed(&mut self, mu: f64) -> Result<SteadyComRequest> {
        let key = self.steady_com_key();
        let (min, max) = {
            let state = self.cached_range().ok_or_else(|| StoreError::msg("The permitted µ range is not known."))?;
            (state.min.unwrap_or(0.0), state.max.unwrap_or(f64::NAN))
        };
        if mu < min || mu > max {
            return Err(StoreError::msg(format!(
                "Choose μ inside the computed range {}–{} h⁻¹.",
                js::number_to_string(min),
                js::number_to_string(max)
            )));
        }
        self.cancel_steady_com(true);
        let generation = self.steady_com_generation;
        let controller = Arc::new(AtomicBool::new(false));
        self.steady_com_controller = Some(controller.clone());
        let parsimonious = self.steady_com_config.parsimonious;
        let serial = {
            let state = self.steady_com_state.as_mut().expect("cached range");
            state.phase = Some("fixed".into());
            state.parsimonious = parsimonious;
            state.error = None;
            state.serial
        };
        let what = if parsimonious { "the least total flux" } else { "fluxes" };
        self.set_status_message(StatusState::Busy, format!("Calculating {what} at fixed μ = {} h⁻¹…", js::number_to_string(mu)));
        self.emit("steadycom");
        let started = Instant::now();
        let spec = match self.steady_com_problem(false) {
            Ok(spec) => spec,
            Err(error) => {
                self.fail_steady_com(generation, serial, &controller, &error);
                return Err(error);
            }
        };
        // The parsimonious LP carries an extra column and two rows per
        // reversible reaction and is an optimisation rather than a
        // feasibility question, so it gets more time than the plain solve.
        let (task, timeout) = if parsimonious {
            (SteadyComTask::Parsimonious { mu, time_limit_seconds: 600.0 }, Duration::from_millis(900_000))
        } else {
            (SteadyComTask::Fixed { mu, time_limit_seconds: 60.0 }, Duration::from_millis(300_000))
        };
        self.spawn_steady_com(
            SteadyComJob { generation, key, state_serial: serial, task, spec, started, controller, then_fixed: None },
            timeout,
        );
        Ok(SteadyComRequest::Started)
    }

    fn spawn_steady_com(&mut self, job: SteadyComJob, timeout: Duration) {
        let runner = self.steady_com_runner.clone();
        let solver = self.solver.clone();
        let spec = job.spec.clone();
        let task = job.task.clone();
        let label = format!("steadycom-{}", task.kind());
        let cancel = job.controller.clone();
        self.jobs.spawn(JobKind::SteadyCom, &label, 0, JobContext::SteadyCom(Box::new(job)), cancel, move |id, tx: Sender<JobMessage>, cancel| {
            let progress_tx = std::sync::Mutex::new(tx.clone());
            let on_progress = move |progress: SteadyComProgress| {
                if let Ok(tx) = progress_tx.lock() {
                    let _ = tx.send(JobMessage::SteadyComProgress { id, progress });
                }
            };
            let controls = TaskControls { cancel, timeout, on_progress: &on_progress, solver };
            let result = runner.run(&spec, &task, &controls);
            let _ = tx.send(JobMessage::Done { id, payload: JobPayload::SteadyCom(Box::new(result)) });
        });
    }

    /// The `catch` of the JS requests: if the request is still the current
    /// one, the error goes on its state and the status line.
    fn fail_steady_com(&mut self, generation: u64, serial: u64, controller: &Arc<AtomicBool>, error: &StoreError) {
        if generation == self.steady_com_generation {
            if let Some(state) = self.steady_com_state.as_mut().filter(|s| s.serial == serial) {
                state.phase = None;
                state.error = Some(error.to_string());
            }
            let state = if error.is_abort() { StatusState::Ok } else { StatusState::Error };
            self.set_status_message(state, error.to_string());
            self.emit("steadycom");
        }
        if self.steady_com_controller.as_ref().is_some_and(|c| Arc::ptr_eq(c, controller)) {
            self.steady_com_controller = None;
        }
    }

    pub(crate) fn apply_steady_com_progress(&mut self, generation: u64, key: &str, serial: u64, progress: SteadyComProgress) {
        if generation != self.steady_com_generation || self.steady_com_key() != key {
            return;
        }
        if let Some(state) = self.steady_com_state.as_mut().filter(|s| s.serial == serial) {
            state.progress = Some(progress);
        }
        self.status_mut().message = format!("Calculating μ range: {} LP evaluations…", progress.iterations);
        self.emit("steadycom");
    }

    pub(crate) fn apply_steady_com_done(&mut self, job: SteadyComJob, result: Result<SteadyComTaskResult>) -> Result<()> {
        let outcome = self.apply_steady_com_result(&job, result);
        if let Err(error) = &outcome {
            self.fail_steady_com(job.generation, job.state_serial, &job.controller, error);
        } else if self.steady_com_controller.as_ref().is_some_and(|c| Arc::ptr_eq(c, &job.controller)) {
            self.steady_com_controller = None;
        }
        // A range asked for by a fixed-µ solve goes on to the solve.
        if let (Ok(()), Some(mu)) = (&outcome, job.then_fixed) {
            return self.start_steady_com_fixed(mu).map(|_| ());
        }
        outcome
    }

    fn apply_steady_com_result(&mut self, job: &SteadyComJob, result: Result<SteadyComTaskResult>) -> Result<()> {
        // A cancelled request rejects with its AbortError even if the task
        // finished anyway.
        if job.controller.load(Ordering::SeqCst) && job.generation != self.steady_com_generation {
            return Err(match result {
                Err(e) if e.is_abort() => e,
                _ => StoreError::aborted("SteadyCom was cancelled."),
            });
        }
        let result = result?;
        if job.generation != self.steady_com_generation || self.steady_com_key() != job.key {
            return Err(stale_steady_com_error());
        }
        match (&job.task, result) {
            (SteadyComTask::Range { .. }, SteadyComTaskResult::Range(r)) => self.install_steady_com_range(job, r),
            (SteadyComTask::Fixed { mu, .. } | SteadyComTask::Parsimonious { mu, .. }, SteadyComTaskResult::At(at)) => {
                self.install_steady_com_fixed(job, *mu, at)
            }
            _ => Err(StoreError::msg("The SteadyCom task returned a result of another kind.")),
        }
    }

    fn install_steady_com_range(&mut self, job: &SteadyComJob, result: SteadyComResult) -> Result<()> {
        let upper = result.range_upper.filter(|u| u.is_finite());
        if result.status != SteadyComStatus::Optimal || result.zero_feasible != Some(true) || upper.is_none() {
            let message = if result.message.is_empty() {
                "The solver did not establish a permitted growth-rate interval.".to_string()
            } else {
                result.message.clone()
            };
            return Err(StoreError::msg(message));
        }
        let seconds = js::round(job.started.elapsed().as_secs_f64() * 1000.0 / 100.0) / 10.0;
        let (min, max) = (result.range_lower.unwrap_or(0.0), upper.unwrap_or(f64::NAN));
        if let Some(state) = self.steady_com_state.as_mut().filter(|s| s.serial == job.state_serial) {
            state.min = Some(min);
            state.max = Some(max);
            state.upper_bound = result.upper_bound;
            state.resolution = result.resolution;
            state.iterations = Some(result.iterations);
            state.seconds = Some(seconds);
            state.phase = None;
            state.progress = None;
            state.error = None;
        }
        self.set_status_message(
            StatusState::Ok,
            format!("Permitted μ: {}–{} h⁻¹ for the current constraints.", js::number_to_string(min), js::to_fixed(max, 6)),
        );
        self.emit("steadycom");
        Ok(())
    }

    fn install_steady_com_fixed(&mut self, job: &SteadyComJob, mu: f64, at: AtResult) -> Result<()> {
        if at.status != LpStatus::Optimal || !at.feasible {
            return Err(StoreError::msg(if at.status == LpStatus::Infeasible {
                format!("No feasible flux distribution was found at μ = {} h⁻¹.", js::number_to_string(mu))
            } else {
                format!(
                    "The solver did not finish the fixed-μ calculation ({}). Its last solution has been retained.",
                    at.status.as_str()
                )
            }));
        }
        let spec = job.spec.clone();
        let owners = spec.joint.as_ref().and_then(|j| j.metadata.as_ref()).map(|m| &m.reaction_owners);
        // An absent member's abundance is an LP column at its zero bound,
        // which can come back a hair below zero; anything further off is
        // left to fail.
        let clean = |x: Option<f64>| match x {
            None => 0.0,
            Some(x) if x < 0.0 && x > -1e-9 => 0.0,
            Some(x) => x,
        };
        let mut fluxes = IndexMap::new();
        for (id, value) in to_merged_fluxes(&at.fluxes, "@", owners) {
            let abundance = match split_id(&id).0 {
                Some(species) => clean(at.abundances.get(species).copied()),
                None => 1.0,
            };
            fluxes.insert(id, if abundance > 1e-9 { value / abundance } else { 0.0 });
        }
        for sp in self.species.iter_mut() {
            sp.abundance = clean(at.abundances.get(&sp.id).copied());
        }
        let abundances: IndexMap<String, f64> = at.abundances.iter().map(|(id, x)| (id.clone(), clean(Some(*x)))).collect();
        if let Some(community) = self.community_mut() {
            community.abundances = abundances;
        }
        self.refresh_composition_problem()?;
        self.parsimonious = false;
        self.loopless = false;
        self.cycle_free = false;
        self.omics.gimme_enabled = false;
        self.omics.gimme_result = None;
        let _ = self.set_fva(false);
        let parsimonious = matches!(job.task, SteadyComTask::Parsimonious { .. });
        let total_flux = if parsimonious { at.total_flux } else { None };
        let solve_ms = js::round(job.started.elapsed().as_secs_f64() * 1000.0);
        if let Some(state) = self.steady_com_state.as_mut().filter(|s| s.serial == job.state_serial) {
            state.phase = None;
            state.mu = Some(mu);
            state.feasible = Some(true);
            state.abundances = at.abundances.clone();
            state.total_biomass = Some(at.total);
            state.original_fluxes = Some(at.fluxes.clone());
            state.parsimonious = parsimonious;
            state.total_flux = total_flux;
            state.solve_ms = Some(solve_ms);
        }
        let solution = Solution {
            status: LpStatus::Optimal,
            objective_value: mu,
            fluxes,
            message: String::new(),
            solver: self.solver.kind().id().into(),
            result: None,
            steady_com: Some(SteadyComSolution {
                mu,
                key: job.key.clone(),
                abundances: at.abundances.clone(),
                total_biomass: at.total,
                spec,
                original_fluxes: at.fluxes.clone(),
                parsimonious,
                total_flux,
            }),
            extra: None,
            zuniga: None,
        };
        self.apply_solution(solution, false);
        let message = if parsimonious {
            format!(
                "SteadyCom at fixed μ = {} h⁻¹, parsimonious: the composition and fluxes with the least total member flux ({} mmol h⁻¹).",
                js::number_to_string(mu),
                js::to_fixed(at.total_flux.unwrap_or(f64::NAN), 3)
            )
        } else {
            format!("SteadyCom at fixed μ = {} h⁻¹; one feasible population composition and flux distribution.", js::number_to_string(mu))
        };
        let next = Status { state: StatusState::Ok, message, solve_ms, ..self.status().clone() };
        self.set_status(next);
        self.emit("steadycom");
        Ok(())
    }

    /// Wait for the SteadyCom request in flight (and a fixed solve it leads
    /// to); the last outcome. For tests and command-line use.
    pub fn wait_steady_com(&mut self, timeout: Duration) -> Option<Result<()>> {
        let outcomes = self.wait_for_jobs(timeout);
        outcomes.into_iter().rev().find(|o| o.kind == JobKind::SteadyCom).map(|o| o.result)
    }
}
