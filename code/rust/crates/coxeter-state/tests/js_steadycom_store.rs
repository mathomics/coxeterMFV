//! Port of `test/steadycom-store.test.mjs`.
//!
//! The JS test initialises the shipped GLPK backend first; the Rust store
//! solves on HiGHS, which is linked in.
//!
//! Platform differences, as designed into the Rust store:
//! - `await store.steadyComRange()` / `solveSteadyComAt(mu)` start a
//!   background job; the test waits for it (`wait_for_jobs`) and reads the
//!   outcome of the SteadyCom job, which is what the JS promise resolved or
//!   rejected with. The range is `store.steady_com_state`.
//! - `store.runSteadyComTask = …` is `store.steady_com_runner`: a runner that
//!   records the request types before solving on HiGHS, one that must not be
//!   called, and `deferredTask` — a runner that hands the request and its
//!   cancel flag (`controls.signal`) to the test and blocks until the test
//!   resolves it.
//! - The JS `fetch` stubs are `Assets`: one that records what is read (and
//!   refuses anything outside `models/`) over the repository, and an
//!   in-memory one holding only the joint fixture's `bounds.csv`.
//! - Object identity (`assert.equal(store.solution, solution)`) is
//!   `Arc::ptr_eq`; `assert.equal(await store.steadyComRange(), range)` (the
//!   cached state object) is the request answering `Cached` with the state's
//!   serial unchanged.
//!
//! Not ported, in "invalid and out-of-range μ never replace the last valid
//! fixed solution": the string values `''` and `'2'` — `solve_steady_com_at`
//! takes an `f64`, so the type refuses them before the store sees them.
//! Not ported, in "medium and member-bound changes invalidate the previous
//! interval…": `assert.equal(store._steadyComSpec, null)` — the cached spec
//! is private to the crate; the state, which is public, is checked.
//! Adapted, in "GUT9 loads full biology…": the JS replaces `store.solve` with
//! a function that throws; Rust cannot replace a method, so the test checks
//! that no solve ran (no solution, no solve in the log). Two of its
//! assertions are stale in the JS itself, which fails this test today
//! (`node --test test/steadycom-store.test.mjs`: "drawing must stay an
//! excerpt, got 1171"): the community's manifest names a curated map
//! (`models/published/steadycom-gut-nine/map.json`, 1171 reactions, nine
//! cells), which `drawPublishedPreview` draws instead of the preview excerpt,
//! so the drawing is not under 400 reactions and `preview-Bt.json` is never
//! read. The Rust store does what the JS store does (checked with the JS
//! store under the same fetch stub: 1171 reactions, 9 cells, no solution, 88
//! medium entries, reads `manifest.json`, `bounds.csv` and `map.json`), and
//! the test asserts that: the drawing is the curated map (still an excerpt of
//! the 11010-reaction model), and `map.json` is read in place of
//! `preview-Bt.json`.

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use coxeter_core::model::Model;
use coxeter_core::testkit::repo_root;
use coxeter_solver::lp::LpStatus;
use coxeter_solver::steadycom::{AtResult, SteadyComResult, SteadyComSpec, SteadyComStatus};
use coxeter_state::steadycom::HighsSteadyComRunner;
use coxeter_state::{
    Assets, DirAssets, JobKind, LogKind, MemoryAssets, Result, SteadyComRequest, SteadyComRunner, SteadyComTask, SteadyComTaskResult,
    Store, StoreError, TaskControls, UserNumber,
};
use indexmap::IndexMap;
use serde_json::{Value, json};

use common::{WAIT, store};

// Helpers

/// `member(id, makes, needs)`: an obligate cross-feeder.
fn member(id: &str, makes: &str, needs: &str) -> Model {
    let metabolites: Vec<Value> = ["glc", makes, needs]
        .iter()
        .flat_map(|base| ["e", "c"].map(|compartment| json!({"id": format!("{base}_{compartment}"), "compartment": compartment})))
        .collect();
    let mut biomass = serde_json::Map::new();
    biomass.insert("glc_c".into(), json!(-1));
    biomass.insert(format!("{needs}_c"), json!(-1));
    let mut make = serde_json::Map::new();
    make.insert("glc_c".into(), json!(-1));
    make.insert(format!("{makes}_c"), json!(1));
    Model::from_value(&json!({
        "id": id, "compartments": {"e": "Extracellular", "c": "Cytosol"},
        "metabolites": metabolites,
        "reactions": [
            {"id": "EX_glc_e", "metabolites": {"glc_e": -1}, "lower_bound": -10, "upper_bound": 1000},
            {"id": "GLCt", "metabolites": {"glc_e": -1, "glc_c": 1}, "lower_bound": 0, "upper_bound": 100},
            {"id": "BIOMASS", "metabolites": biomass, "lower_bound": 0, "upper_bound": 100},
            {"id": format!("MK_{makes}"), "metabolites": make, "lower_bound": 0, "upper_bound": 100},
            {"id": format!("T_{makes}"), "metabolites": {format!("{makes}_c"): -1, format!("{makes}_e"): 1}, "lower_bound": 0, "upper_bound": 100},
            {"id": format!("EX_{makes}_e"), "metabolites": {format!("{makes}_e"): -1}, "lower_bound": 0, "upper_bound": 1000},
            {"id": format!("T_{needs}"), "metabolites": {format!("{needs}_e"): -1, format!("{needs}_c"): 1}, "lower_bound": 0, "upper_bound": 100},
            {"id": format!("EX_{needs}_e"), "metabolites": {format!("{needs}_e"): -1}, "lower_bound": -100, "upper_bound": 1000}
        ],
        "objective": {"BIOMASS": 1}
    }))
    .expect("a valid member model")
}

/// `pairStore()`.
fn pair_store() -> Store {
    let mut store = store();
    store.set_model("member-A", member("A", "x", "y"));
    store.set_model("member-B", member("B", "y", "x"));
    let species = json!([{"id": "A", "model": "member-A", "abundance": 0.5}, {"id": "B", "model": "member-B", "abundance": 0.5}]);
    store.scenarios = vec![
        json!({"id": "mutualists", "name": "Mutualists", "objective": "equal-growth", "medium": {"glc": 10}, "species": species}),
        json!({"id": "lean-mutualists", "name": "Lean mutualists", "objective": "equal-growth", "medium": {"glc": 2}, "species": species}),
    ];
    store.use_scenario("mutualists").unwrap();
    store
}

/// The outcome of the SteadyCom request the store has in flight, once it
/// (and any fixed-µ solve it leads to) has finished: what the JS promise
/// resolved or rejected with.
fn settle(store: &mut Store) -> Result<()> {
    store.wait_steady_com(WAIT).expect("a SteadyCom request finished")
}

/// `await store.steadyComRange()`.
fn steady_com_range(store: &mut Store) -> Result<()> {
    match store.steady_com_range(false)? {
        SteadyComRequest::Cached => Ok(()),
        SteadyComRequest::Started => settle(store),
    }
}

/// `await store.solveSteadyComAt(mu)`.
fn solve_steady_com_at(store: &mut Store, mu: f64) -> Result<()> {
    match store.solve_steady_com_at(mu)? {
        SteadyComRequest::Cached => Ok(()),
        SteadyComRequest::Started => settle(store),
    }
}

/// `await store.steadyComProblem()`.
fn steady_com_problem(store: &mut Store) -> Arc<SteadyComSpec> {
    store.steady_com_problem(false).expect("a SteadyCom problem")
}

fn bounds(store: &Store, rid: &str) -> (f64, f64) {
    store.problem.as_ref().unwrap().get_bounds(rid).unwrap_or_else(|| panic!("{rid} is in the problem"))
}

fn flux(store: &Store, rid: &str) -> f64 {
    *store.solution.as_ref().unwrap().fluxes.get(rid).unwrap_or_else(|| panic!("{rid} has a flux"))
}

fn same_solution(store: &Store, solution: &Option<Arc<coxeter_state::Solution>>) -> bool {
    match (&store.solution, solution) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    }
}

/// `assertFixedSolution(store, mu)`.
fn assert_fixed_solution(store: &Store, mu: f64) {
    let solution = store.solution.as_ref().expect("a solution");
    assert_eq!(solution.status, LpStatus::Optimal);
    let sc = solution.steady_com.as_ref().expect("a fixed-µ SteadyCom solution");
    assert_eq!(sc.mu, mu);
    assert_eq!(solution.objective_value, mu);
    assert_eq!(store.steady_com_state.as_ref().unwrap().key, store.steady_com_key());
    assert!((sc.abundances.values().sum::<f64>() - 1.0).abs() < 1e-7);
    for id in ["A", "B"] {
        assert!(sc.abundances[id] > 0.0, "{id} must be present in this obligate mutualism");
        assert!((flux(store, &format!("{id}@BIOMASS")) - mu).abs() < 1e-7, "{id} specific biomass flux equals requested μ");
    }
}

/// `runSteadyComTask` wrapped to record each request's type.
struct Recording {
    requests: Arc<Mutex<Vec<String>>>,
}

impl SteadyComRunner for Recording {
    fn run(&self, spec: &SteadyComSpec, task: &SteadyComTask, controls: &TaskControls<'_>) -> Result<SteadyComTaskResult> {
        self.requests.lock().unwrap().push(task.kind().to_string());
        HighsSteadyComRunner.run(spec, task, controls)
    }
}

fn record_requests(store: &mut Store) -> Arc<Mutex<Vec<String>>> {
    let requests = Arc::new(Mutex::new(Vec::new()));
    store.steady_com_runner = Arc::new(Recording { requests: requests.clone() });
    requests
}

/// `() => { throw new Error('Cached range must not dispatch another task') }`.
struct Forbidden;

impl SteadyComRunner for Forbidden {
    fn run(&self, _: &SteadyComSpec, _: &SteadyComTask, _: &TaskControls<'_>) -> Result<SteadyComTaskResult> {
        Err(StoreError::msg("Cached range must not dispatch another task"))
    }
}

/// One call of the deferred task: `{ spec, request, controls }`.
struct Call {
    task: SteadyComTask,
    /// `controls.signal.aborted`.
    cancel: Arc<AtomicBool>,
}

struct Deferred {
    calls: Mutex<Sender<Call>>,
    results: Mutex<Receiver<Result<SteadyComTaskResult>>>,
}

impl SteadyComRunner for Deferred {
    fn run(&self, _: &SteadyComSpec, task: &SteadyComTask, controls: &TaskControls<'_>) -> Result<SteadyComTaskResult> {
        let _ = self.calls.lock().unwrap().send(Call { task: task.clone(), cancel: controls.cancel.clone() });
        match self.results.lock().unwrap().recv() {
            Ok(result) => result,
            Err(_) => Err(StoreError::aborted("The test ended before resolving the task.")),
        }
    }
}

/// `deferredTask(store)`: `called` gives the request, `resolve` answers it.
struct DeferredTask {
    called: Receiver<Call>,
    resolve: Sender<Result<SteadyComTaskResult>>,
}

impl DeferredTask {
    fn called(&self) -> Call {
        self.called.recv_timeout(WAIT).expect("the store called runSteadyComTask")
    }
    fn resolve(&self, result: SteadyComTaskResult) {
        self.resolve.send(Ok(result)).expect("the task is waiting");
    }
}

fn deferred_task(store: &mut Store) -> DeferredTask {
    let (call_tx, call_rx) = channel();
    let (result_tx, result_rx) = channel();
    store.steady_com_runner = Arc::new(Deferred { calls: Mutex::new(call_tx), results: Mutex::new(result_rx) });
    DeferredTask { called: call_rx, resolve: result_tx }
}

/// `fakeRange()`.
fn fake_range() -> SteadyComTaskResult {
    SteadyComTaskResult::Range(SteadyComResult {
        status: SteadyComStatus::Optimal,
        growth_rate: 99.0,
        range_lower: Some(0.0),
        range_upper: Some(99.0),
        upper_bound: Some(100.0),
        resolution: Some(0.001),
        zero_feasible: Some(true),
        abundances: IndexMap::new(),
        fluxes: IndexMap::new(),
        iterations: 1,
        message: String::new(),
    })
}

// Tests

#[test]
fn community_export_bounds_constrain_the_separate_member_steadycom_interval() {
    let mut store = store();
    store.set_model(
        "waste-maker",
        Model::from_value(&json!({
            "id": "A", "compartments": {"e": "Extracellular"},
            "metabolites": [{"id": "a_e", "compartment": "e"}, {"id": "w_e", "compartment": "e"}],
            "reactions": [
                {"id": "EX_a_e", "metabolites": {"a_e": -1}, "lower_bound": -10, "upper_bound": 100},
                {"id": "BIOMASS", "metabolites": {"a_e": -1, "w_e": 1}, "lower_bound": 0, "upper_bound": 100},
                {"id": "EX_w_e", "metabolites": {"w_e": -1}, "lower_bound": 0, "upper_bound": 100}
            ],
            "objective": {"BIOMASS": 1}
        }))
        .unwrap(),
    );
    store.scenarios =
        vec![json!({"id": "waste", "name": "Waste maker", "species": [{"id": "A", "model": "waste-maker", "abundance": 1}], "medium": {"a": 10}})];
    store.use_scenario("waste").unwrap();
    steady_com_range(&mut store).unwrap();
    let initial = store.steady_com_state.as_ref().unwrap().max.unwrap();
    assert!((initial - 10.0).abs() < 1e-4);
    store.set_reaction_bounds("EX_w_s", 0.0, 2.0).unwrap();
    steady_com_range(&mut store).unwrap();
    let limited = store.steady_com_state.as_ref().unwrap().max.unwrap();
    assert!((limited - 2.0).abs() < 1e-4, "export cap must limit growth to 2, got {limited}");
    let problem = steady_com_problem(&mut store);
    let w = &problem.community_exchange_bounds["w"];
    assert_eq!((w.lower, w.upper), (Some(0.0), Some(2.0)));
    solve_steady_com_at(&mut store, 1.0).unwrap();
    assert!((flux(&store, "A@EX_w_e") - 1.0).abs() < 1e-7);
}

#[test]
fn store_computes_and_caches_a_real_permitted_interval_without_rewriting_member_bounds() {
    let mut store = pair_store();
    let documents: Vec<(String, Value)> = store.models.iter().map(|(id, model)| (id.clone(), model.to_json())).collect();
    let bounds = store.problem.as_ref().unwrap().snapshot_bounds();
    steady_com_range(&mut store).unwrap();
    let range = store.steady_com_state.clone().unwrap();
    assert_eq!(range.source, "computed");
    assert_eq!(range.min, Some(0.0));
    let max = range.max.unwrap();
    assert!((max - 5.0).abs() < 2e-5, "two glucose costs per biomass give μ≤5, received {max}");
    assert!(range.iterations.unwrap() > 1);
    assert_eq!(range.phase, None);
    assert_eq!(store.problem.as_ref().unwrap().snapshot_bounds(), bounds);
    assert_eq!(store.models.iter().map(|(id, model)| (id.clone(), model.to_json())).collect::<Vec<_>>(), documents);
    store.steady_com_runner = Arc::new(Forbidden);
    assert_eq!(store.steady_com_range(false).unwrap(), SteadyComRequest::Cached);
    assert!(!store.job_running(JobKind::SteadyCom));
    assert_eq!(store.steady_com_state.as_ref().unwrap().serial, range.serial, "the cached range is the same state");
}

#[test]
fn the_first_fixed_mu_solve_computes_its_range_before_returning_usable_state_and_population_specific_fluxes() {
    let mut store = pair_store();
    let requests = record_requests(&mut store);
    let before = store.problem.as_ref().unwrap().snapshot_bounds();
    solve_steady_com_at(&mut store, 2.0).unwrap();
    assert_eq!(*requests.lock().unwrap(), ["range", "fixed"]);
    let state = store.steady_com_state.as_ref().unwrap();
    assert!(state.max.is_some_and(|max| max.is_finite() && max > 2.0));
    assert_eq!(state.phase, None);
    assert_fixed_solution(&store, 2.0);
    assert_eq!(store.problem.as_ref().unwrap().snapshot_bounds(), before);
    assert!(!store.parsimonious);
    assert!(!store.loopless);
}

#[test]
fn invalid_and_out_of_range_mu_never_replace_the_last_valid_fixed_solution() {
    let mut store = pair_store();
    solve_steady_com_at(&mut store, 2.0).unwrap();
    let solution = store.solution.clone();
    let max = store.steady_com_state.as_ref().unwrap().max.unwrap();
    for mu in [f64::NAN, f64::INFINITY, -1.0, max + 1.0] {
        let error = solve_steady_com_at(&mut store, mu).unwrap_err().to_string();
        assert!(error.contains("finite growth rate") || error.contains("computed range"), "{mu}: {error}");
        assert!(same_solution(&store, &solution));
    }
    assert_fixed_solution(&store, 2.0);
}

#[test]
fn medium_and_member_bound_changes_invalidate_the_previous_interval_and_are_included_in_the_new_solve() {
    let mut store = pair_store();
    steady_com_range(&mut store).unwrap();
    let initial = store.steady_com_state.clone().unwrap();
    store.set_medium_limit("glc", 4.0).unwrap();
    assert!(store.steady_com_state.is_none());
    steady_com_range(&mut store).unwrap();
    let lean = store.steady_com_state.clone().unwrap();
    assert_ne!(lean.key, initial.key);
    assert!((lean.max.unwrap() - 2.0).abs() < 2e-5);
    store.set_reaction_bounds("A@MK_x", 0.0, 0.1).unwrap();
    assert!(store.steady_com_state.is_none());
    steady_com_range(&mut store).unwrap();
    let constrained = store.steady_com_state.clone().unwrap();
    assert!(
        constrained.max.unwrap() < lean.max.unwrap() - 0.1,
        "limiting the cross-fed product must reduce the sustainable growth rate"
    );
    let spec = steady_com_problem(&mut store);
    let a = spec.members.iter().find(|m| m.id == "A").unwrap();
    assert_eq!(a.model.as_ref().unwrap().reactions["MK_x"].upper_bound, 0.1);
    assert_eq!(
        store.models.get("member-A").unwrap().reactions["MK_x"].upper_bound,
        100.0,
        "simulation overrides must not edit the catalog model"
    );
}

#[test]
fn a_cancelled_calculation_cannot_apply_a_late_successful_range_result() {
    let mut store = pair_store();
    let deferred = deferred_task(&mut store);
    assert_eq!(store.steady_com_range(false).unwrap(), SteadyComRequest::Started);
    let call = deferred.called();
    let ordinary_solution = store.solution.clone();
    store.cancel_steady_com(false);
    assert!(call.cancel.load(Ordering::SeqCst));
    deferred.resolve(fake_range());
    let pending = settle(&mut store);
    assert!(pending.as_ref().is_err_and(StoreError::is_abort), "{pending:?}");
    assert!(same_solution(&store, &ordinary_solution));
    assert_ne!(store.steady_com_state.as_ref().and_then(|s| s.max), Some(99.0));
}

#[test]
fn changing_medium_while_a_range_task_runs_makes_its_late_result_stale() {
    let mut store = pair_store();
    let deferred = deferred_task(&mut store);
    assert_eq!(store.steady_com_range(false).unwrap(), SteadyComRequest::Started);
    let call = deferred.called();
    store.set_medium_limit("glc", 4.0).unwrap();
    let current_solution = store.solution.clone();
    deferred.resolve(fake_range());
    let pending = settle(&mut store);
    assert!(pending.as_ref().is_err_and(StoreError::is_abort), "{pending:?}");
    assert!(call.cancel.load(Ordering::SeqCst));
    assert!(store.steady_com_state.is_none());
    assert!(same_solution(&store, &current_solution));
    assert_eq!(store.medium["glc"], 4.0);
}

#[test]
fn switching_scenarios_during_a_fixed_task_cannot_install_its_old_composition_or_fluxes() {
    let mut store = pair_store();
    steady_com_range(&mut store).unwrap();
    let deferred = deferred_task(&mut store);
    assert_eq!(store.solve_steady_com_at(2.0).unwrap(), SteadyComRequest::Started);
    let call = deferred.called();
    assert_eq!(call.task.kind(), "fixed");
    store.use_scenario("lean-mutualists").unwrap();
    let current_solution = store.solution.clone();
    deferred.resolve(SteadyComTaskResult::At(AtResult {
        feasible: true,
        total: 1.0,
        status: LpStatus::Optimal,
        message: String::new(),
        objective: None,
        abundances: IndexMap::from([("A".to_string(), 0.9), ("B".to_string(), 0.1)]),
        fluxes: IndexMap::from([("A|BIOMASS".to_string(), 1.8), ("B|BIOMASS".to_string(), 0.2)]),
        lp_evaluations: 1,
        total_flux: None,
    }));
    let pending = settle(&mut store);
    assert!(pending.as_ref().is_err_and(StoreError::is_abort), "{pending:?}");
    assert!(call.cancel.load(Ordering::SeqCst));
    assert_eq!(store.scenario_id().as_deref(), Some("lean-mutualists"));
    assert!(same_solution(&store, &current_solution));
    assert!(store.solution.as_ref().unwrap().steady_com.is_none());
    assert_eq!(store.species.iter().map(|sp| sp.abundance).collect::<Vec<_>>(), [0.5, 0.5]);
}

#[test]
fn scenario_persistence_recomputes_and_restores_fixed_mu_in_a_new_context_legacy_scenarios_return_to_ordinary_fba() {
    let mut source = pair_store();
    solve_steady_com_at(&mut source, 1.5).unwrap();
    let mut document: Value = serde_json::from_str(&source.state_json()).unwrap();
    assert_eq!(document["steadyCom"], json!({"mu": 1.5}));
    let mut restored = pair_store();
    let requests = record_requests(&mut restored);
    restored.load_state(&document.to_string()).unwrap();
    settle(&mut restored).unwrap();
    assert_eq!(*requests.lock().unwrap(), ["range", "fixed"]);
    assert_fixed_solution(&restored, 1.5);
    assert_ne!(restored.steady_com_state.as_ref().unwrap().key, source.steady_com_state.as_ref().unwrap().key);
    let doc = document.as_object_mut().unwrap();
    doc.shift_remove("steadyCom");
    doc.shift_remove("steadyComConfig");
    restored.load_state(&document.to_string()).unwrap();
    restored.wait_for_jobs(WAIT);
    assert!(restored.steady_com_state.is_none());
    assert!(restored.solution.as_ref().unwrap().steady_com.is_none());
    assert_eq!(restored.solution.as_ref().unwrap().status, LpStatus::Optimal);
}

#[test]
fn ordinary_fva_refuses_a_fixed_mu_result_without_changing_the_active_solution() {
    let mut store = pair_store();
    solve_steady_com_at(&mut store, 2.0).unwrap();
    let solution = store.solution.clone();
    store.set_fva(true).unwrap();
    assert!(!store.fva_enabled);
    assert!(same_solution(&store, &solution));
    let message = &store.status().message;
    let fva_then_fixed = message.find("FVA").is_some_and(|i| message[i..].contains("fixed"));
    let fixed_then_steadycom = message.find("fixed").is_some_and(|i| message[i..].contains("SteadyCom"));
    assert!(fva_then_fixed || fixed_then_steadycom, "{message}");
}

/// The JS `fetch` stub of the GUT9 test: the repository's files, restricted
/// to `models/`, with every path read recorded.
struct RecordingAssets {
    dir: DirAssets,
    requested: Mutex<Vec<String>>,
}

impl Assets for RecordingAssets {
    fn read(&self, path: &str) -> Result<Vec<u8>> {
        assert!(path.starts_with("models/"), "fixture fetches are restricted to repository model assets: {path}");
        self.requested.lock().unwrap().push(path.to_string());
        self.dir.read(path)
    }
    fn contains(&self, path: &str) -> bool {
        self.dir.contains(path)
    }
}

#[test]
fn gut9_loads_full_biology_and_its_diet_while_drawing_only_an_excerpt_and_deferring_expensive_analysis() {
    let assets = Arc::new(RecordingAssets { dir: DirAssets::new(repo_root()), requested: Mutex::new(Vec::new()) });
    let mut store = Store::new(assets.clone());
    let scenarios: Value = serde_json::from_str(&std::fs::read_to_string(repo_root().join("models/scenarios.json")).unwrap()).unwrap();
    store.published_communities = scenarios["publishedCommunities"].as_array().unwrap().clone();
    // `store.solve = () => { throw … }`: selecting GUT9 must not run an
    // expensive LP. Checked below: no solution and no solve in the log.
    store.use_published_community("steadycom-gut-nine").unwrap();
    assert!(store.log.iter().all(|e| e.kind != LogKind::Solve), "Selecting GUT9 must not run an expensive LP");
    let support = store.steady_com_support(None);
    assert!(support.supported);
    assert_eq!(support.kind, Some("joint"));
    assert_eq!(store.species.len(), 9);
    assert!(store.species.iter().all(|sp| sp.file.contains("/model-")));
    let bt = store.species.iter().find(|sp| sp.id == "Bt").unwrap().file.clone();
    assert_eq!(store.models.get(&bt).unwrap().reactions.len(), 1443);
    let community = store.community.as_ref().unwrap();
    assert_eq!(community.merged.reactions.len(), 11010);
    assert_eq!(community.merged.metabolites.len(), 8169);
    assert_eq!(store.problem.as_ref().unwrap().rxn_ids.len(), 11010);
    let map = store.map.as_ref().unwrap();
    let drawn = map.reactions().len();
    // JS: `drawn > 0 && drawn < 400` ("drawing must stay an excerpt"). The
    // community ships a curated map (`manifest.map`), drawn instead of
    // the preview excerpt; see the header.
    let curated: Value =
        serde_json::from_str(&std::fs::read_to_string(repo_root().join("models/published/steadycom-gut-nine/map.json")).unwrap()).unwrap();
    assert!(drawn > 0 && drawn < store.problem.as_ref().unwrap().rxn_ids.len(), "drawing must stay an excerpt, got {drawn}");
    assert_eq!(drawn, curated["map"]["reactions"].as_array().unwrap().len(), "the curated map is drawn");
    assert_eq!(map.cells().len(), 9);
    assert!(store.solution.is_none());
    assert_eq!(store.medium.len(), 88);
    let requested = assets.requested.lock().unwrap().clone();
    // JS: `preview-Bt.json` instead of `map.json` (see the header).
    for path in [
        "models/published/steadycom-gut-nine/bounds.csv",
        "models/published/steadycom-gut-nine/model-Bt.json",
        "models/published/steadycom-gut-nine/map.json",
    ] {
        assert!(requested.iter().any(|p| p == path), "{path} was read");
    }
    let spec = steady_com_problem(&mut store);
    let joint = spec.joint.as_ref().unwrap();
    assert_eq!(joint.reactions.len(), 11009);
    assert_eq!(joint.metadata.as_ref().unwrap().species_exchange.len(), 1355);
}

const PUBLISHED: &str = "steadycom-gut-nine";

/// `tinyJointStore(t)`: the published-joint Store route with a small,
/// exactly solvable model.
fn tiny_joint_store() -> Store {
    let assets = Arc::new(MemoryAssets::new());
    let mut store = Store::new(assets.clone());
    let mut rows = vec!["reaction,species,lower_bound,upper_bound,objective_coefficient".to_string()];
    let mut descriptors = Vec::new();
    for id in ["A", "B"] {
        let (e, c) = (format!("a[e_{id}]"), format!("a[c_{id}]"));
        let mut ex = serde_json::Map::new();
        ex.insert(e.clone(), json!(-1));
        ex.insert("a[u]".into(), json!(1));
        let mut t = serde_json::Map::new();
        t.insert(e.clone(), json!(-1));
        t.insert(c.clone(), json!(1));
        let mut bio = serde_json::Map::new();
        bio.insert(c.clone(), json!(-1));
        let mut objective = serde_json::Map::new();
        objective.insert(format!("BIO_{id}"), json!(1));
        let doc = json!({
            "id": id, "compartments": {"e": "Extracellular", "c": "Cytosol", "u": "Shared medium"},
            "metabolites": [
                {"id": e, "compartment": "e"}, {"id": c, "compartment": "c"},
                {"id": "a[u]", "compartment": "u"}
            ],
            "reactions": [
                {"id": format!("EX_{id}"), "metabolites": ex, "lower_bound": -20, "upper_bound": 1000},
                {"id": format!("T_{id}"), "metabolites": t, "lower_bound": 0, "upper_bound": 1000},
                {"id": format!("BIO_{id}"), "metabolites": bio, "lower_bound": 0, "upper_bound": 1000}
            ],
            "objective": objective
        });
        store.set_model(&format!("published/{PUBLISHED}/model-{id}"), Model::from_value(&doc).unwrap());
        let mut preview = doc.clone();
        preview["metabolites"].as_array_mut().unwrap().retain(|m| m["id"] != "a[u]");
        preview["compartments"].as_object_mut().unwrap().shift_remove("u");
        preview["reactions"][0]["metabolites"].as_object_mut().unwrap().shift_remove("a[u]");
        store.set_model(&format!("published/{PUBLISHED}/preview-{id}"), Model::from_value(&preview).unwrap());
        rows.push(format!("EX_{id},{id},-20,1000,0"));
        rows.push(format!("T_{id},{id},0,1000,0"));
        rows.push(format!("BIO_{id},{id},0,1000,1"));
        descriptors.push(json!({"id": id, "model": format!("model-{id}.json"), "preview": format!("preview-{id}.json")}));
    }
    rows.push("UT_a(u),comm,0,10,0".into());
    rows.push("EX_a(u),comm,0,10000,0".into());
    store.published_communities = vec![json!({"id": PUBLISHED, "name": "Small joint Store fixture"})];
    store.published_manifests.insert(PUBLISHED.into(), Some(json!({"members": descriptors, "medium": {"a": 10}})));
    // The fetch stub answers this one file only.
    assets.insert(format!("models/published/{PUBLISHED}/bounds.csv"), rows.join("\n"));
    store.use_published_community(PUBLISHED).unwrap();
    store
}

#[test]
fn disabling_a_joint_member_preserves_the_excerpt_and_closes_its_entire_fixed_composition_biology() {
    let mut store = tiny_joint_store();
    store.set_species_enabled("B", false).unwrap();
    let map = store.map.as_ref().unwrap();
    assert_eq!(map.cells().len(), 1);
    assert_eq!(map.cells()[0].id, "A");
    assert!(map.reactions().len() < 20);
    assert_eq!(store.community.as_ref().unwrap().abundances.get("B").copied().unwrap_or(0.0), 0.0);
    for id in ["B@EX_B", "B@T_B", "B@BIO_B"] {
        assert_eq!(bounds(&store, id), (0.0, 0.0), "{id}");
    }
    let spec = steady_com_problem(&mut store);
    assert_eq!(spec.joint.as_ref().unwrap().reactions.len(), 8, "the joint source must remain complete when one member is inactive");
    for mu in [0.0, 2.0] {
        solve_steady_com_at(&mut store, mu).unwrap();
        let sc = store.solution.as_ref().unwrap().steady_com.as_ref().unwrap();
        assert_eq!(sc.abundances["B"], 0.0, "disabled B must remain absent even at μ={mu}");
        for id in ["B@EX_B", "B@T_B", "B@BIO_B"] {
            assert_eq!(flux(&store, id), 0.0, "{id}");
        }
    }
}

#[test]
fn joint_medium_edits_update_the_real_uptake_column_used_by_ordinary_fba_and_the_next_steadycom_task() {
    let mut store = tiny_joint_store();
    store.set_medium_limit("a", 4.0).unwrap();
    assert_eq!(bounds(&store, "UT_a(u)"), (0.0, 4.0));
    let solution = store.solution.as_ref().unwrap();
    assert_eq!(solution.status, LpStatus::Optimal);
    assert!(solution.steady_com.is_none());
    assert!((solution.objective_value - 4.0).abs() < 1e-7);
    assert_eq!(steady_com_problem(&mut store).joint.as_ref().unwrap().reactions["UT_a(u)"].upper_bound, 4.0);
}

#[test]
fn a_joint_fixed_mu_result_rebuilds_ordinary_fba_with_its_calculated_composition_while_keeping_the_drawing() {
    let mut store = tiny_joint_store();
    let map = store.map.as_ref().unwrap().to_json();
    solve_steady_com_at(&mut store, 2.0).unwrap();
    let abundances = store.solution.as_ref().unwrap().steady_com.as_ref().unwrap().abundances.clone();
    assert!(
        abundances.values().any(|amount| (amount - 0.5).abs() > 1e-6),
        "this tiny LP returns a different vertex composition from the starting half/half mixture: {abundances:?}"
    );
    assert_eq!(store.map.as_ref().unwrap().to_json(), map);
    for id in ["A", "B"] {
        let community = store.community.as_ref().unwrap();
        let coefficient = community.merged.reactions[&format!("{id}@EX_{id}")].metabolites.get("M@a_s").copied().unwrap_or(0.0);
        assert!((coefficient - abundances[id]).abs() < 1e-7, "{id}: ordinary shared stoichiometry must use the newly solved abundance");
        if abundances[id] == 0.0 {
            assert_eq!(bounds(&store, &format!("{id}@BIO_{id}")), (0.0, 0.0));
        }
    }
    store.solve();
    let solution = store.solution.clone().unwrap();
    assert_eq!(solution.status, LpStatus::Optimal);
    assert!(solution.steady_com.is_none());
    assert!((solution.objective_value - 10.0).abs() < 1e-7);
    let mut residuals: IndexMap<String, f64> = IndexMap::new();
    for reaction in store.community.as_ref().unwrap().merged.reactions.values() {
        for (metabolite, coefficient) in &reaction.metabolites {
            *residuals.entry(metabolite.clone()).or_insert(0.0) += coefficient * solution.fluxes.get(&reaction.id).copied().unwrap_or(0.0);
        }
    }
    for (metabolite, residual) in residuals {
        assert!(residual.abs() < 1e-7, "{metabolite}: ordinary FBA remains mass balanced");
    }
}

#[test]
fn a_bound_edited_on_a_joint_member_does_not_reopen_it_while_it_is_switched_off() {
    let mut store = tiny_joint_store();
    store.set_reaction_bounds("B@T_B", UserNumber::Empty, "5").unwrap();
    assert_eq!(bounds(&store, "B@T_B").1, 5.0);
    store.set_species_enabled("B", false).unwrap();
    assert_eq!(bounds(&store, "B@T_B"), (0.0, 0.0), "the kept edit must not reopen a closed member");
    assert_eq!(store.overrides.get("B@T_B").map(|b| b.upper), Some(5.0), "the edit itself is kept for when B returns");
    store.set_species_enabled("B", true).unwrap();
    assert_eq!(bounds(&store, "B@T_B").1, 5.0, "and applies again once B is back");
}

#[test]
fn a_bound_edited_on_a_joint_member_does_not_reopen_it_at_zero_abundance_either() {
    let mut store = tiny_joint_store();
    store.set_reaction_bounds("B@T_B", UserNumber::Empty, "5").unwrap();
    store.set_abundance("B", 0.0).unwrap();
    assert!(store.species_held_out("B"));
    assert_eq!(bounds(&store, "B@T_B"), (0.0, 0.0), "zero abundance closes B, and the edit must not reopen it");
    store.set_abundance("B", 0.5).unwrap();
    assert_eq!(bounds(&store, "B@T_B").1, 5.0, "the edit applies again once B has an abundance");
}
