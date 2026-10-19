//! The Zuñiga et al. (2020) published case (the `zuniga*` methods of
//! `store.js`).
//!
//! The case downloads the authors' complete models, verifies their hashes and
//! solves three scenarios with `loadZunigaStudy`, `createZunigaScenario` and
//! `solveZunigaScenario` (`coxeter_analysis::zuniga`). Author models are
//! fetched privately; the drawing never becomes the LP.
//!
//! In JS `store.community` is then the case's own community object (the
//! authors' matrix with ownership read off the identifiers). Here the store
//! keeps that [`ZunigaCommunity`] in [`ZunigaStudyState::prepared`] and puts
//! a [`CommunityModel`] holding the same merged model and ownership in
//! `store.community`, which is what the map, the labels and the search read;
//! what the case computes differently (cross-feeding, growth, the explorer
//! and the omics mapping) is asked of the case's community
//! ([`Store::community_view`]).
//!
//! Downloading the sources is a background job ([`JobKind::Zuniga`]): the
//! store answers at once and [`Store::poll_jobs`] installs the study, unless
//! another scenario was opened meanwhile (`_zunigaGeneration`), in which case
//! the late download is dropped with an `AbortError`.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use coxeter_analysis::community_view::{CommunityView, TransferRef};
use coxeter_analysis::zuniga::{ZunigaModels, ZunigaRun, ZunigaScenario, ZunigaSourceModels};
use coxeter_analysis::{
    DirectoryCache, SourceCache, ZunigaCommunity, ZunigaLoadOptions, ZunigaPrepared, ZunigaStudy, create_zuniga_scenario,
    load_zuniga_study, solve_zuniga_scenario,
};
use coxeter_core::community::{CommunityModel, CrossFeeding, Objective, ReactionOwner, SpeciesEntry, Transfer};
use coxeter_core::js;
use coxeter_core::model::Model;
use coxeter_map::editable_map::EditableMap;
use coxeter_map::multi_selection::empty_selection;
use coxeter_io::reproducibility::{Tolerances, compare_replayed_solution, verify_reproducibility_bundle};
use indexmap::IndexMap;
use serde_json::{Map, Value, json};

use crate::assets::Fetcher;
use crate::error::{Result, StoreError};
use crate::jobs::{JobContext, JobKind, JobMessage, JobPayload};
use crate::store::Store;
use crate::types::{SteadyComConfig, Solution, Species, Status, StatusState, LogKind};
use crate::util::num_json;

pub use coxeter_analysis::ZUNIGA_SCENARIOS;
pub use coxeter_analysis::zuniga::ZUNIGA_ID;

/// The scenario with this id.
pub fn zuniga_scenario(id: &str) -> Option<&'static ZunigaScenario> {
    ZUNIGA_SCENARIOS.iter().find(|s| s.id == id)
}

/// What a solved scenario records about itself (`result.zuniga`): the run,
/// with the growth and cross-feeding the store adds to it.
#[derive(Debug, Clone)]
pub struct ZunigaRecord {
    pub run: ZunigaRun,
    pub growth: IndexMap<String, f64>,
    pub cross_feeding: CrossFeeding,
}

impl ZunigaRecord {
    pub fn to_json(&self) -> Value {
        let mut out = match self.run.to_json() {
            Value::Object(m) => m,
            _ => Map::new(),
        };
        out.insert("growth".into(), Value::Object(self.growth.iter().map(|(k, v)| (k.clone(), num_json(*v))).collect()));
        out.insert("crossFeeding".into(), cross_feeding_json(&self.cross_feeding));
        Value::Object(out)
    }
}

/// `{links, perMetabolite}` as the JS object prints.
pub fn cross_feeding_json(cf: &CrossFeeding) -> Value {
    let share = |s: &coxeter_core::community::FluxShare| json!({"speciesId": s.species_id, "flux": num_json(s.flux)});
    json!({
        "links": cf.links.iter().map(|l| json!({"base": l.base, "from": l.from, "to": l.to, "flux": num_json(l.flux)})).collect::<Vec<_>>(),
        "perMetabolite": cf.per_metabolite.iter().map(|m| json!({
            "base": m.base,
            "producers": m.producers.iter().map(share).collect::<Vec<_>>(),
            "consumers": m.consumers.iter().map(share).collect::<Vec<_>>(),
            "total": num_json(m.total),
        })).collect::<Vec<_>>(),
    })
}

/// The Zuñiga fields of the store.
#[derive(Debug, Clone)]
pub struct ZunigaStudyState {
    /// `zunigaStudy`: the manifest, source hashes, source models and the
    /// two member models.
    pub study: Option<Arc<ZunigaStudy>>,
    /// `zunigaPrepared`: the open scenario's community and problem.
    pub prepared: Option<Arc<ZunigaPrepared>>,
    pub scenario_id: Option<String>,
    /// `absolute` or `difference`.
    pub comparison: String,
    pub solutions: IndexMap<String, Arc<Solution>>,
    /// `_zunigaGeneration`.
    pub generation: u64,
    /// `showCrossFeeding` before the case hid it (`_zunigaPreviousView`).
    pub previous_view: Option<bool>,
}

impl Default for ZunigaStudyState {
    fn default() -> Self {
        ZunigaStudyState {
            study: None,
            prepared: None,
            scenario_id: None,
            comparison: "absolute".into(),
            solutions: IndexMap::new(),
            generation: 0,
            previous_view: None,
        }
    }
}

// The community

/// The community the analysis functions read: the ordinary one, or the
/// Zuñiga case's own.
#[derive(Debug, Clone, Copy)]
pub enum CommunityRef<'a> {
    Model(&'a CommunityModel),
    Zuniga(&'a ZunigaCommunity),
}

impl CommunityView for CommunityRef<'_> {
    fn merged_reaction_ids(&self) -> Vec<&str> {
        match self {
            CommunityRef::Model(c) => c.merged_reaction_ids(),
            CommunityRef::Zuniga(c) => c.merged_reaction_ids(),
        }
    }

    fn reaction_owner(&self, reaction_id: &str) -> Option<&ReactionOwner> {
        match self {
            CommunityRef::Model(c) => CommunityView::reaction_owner(*c, reaction_id),
            CommunityRef::Zuniga(c) => CommunityView::reaction_owner(*c, reaction_id),
        }
    }

    fn transfers(&self, base: &str) -> Vec<TransferRef<'_>> {
        match self {
            CommunityRef::Model(c) => c.transfers(base),
            CommunityRef::Zuniga(c) => c.transfers(base),
        }
    }

    fn cross_feeding(&self, fluxes: &IndexMap<String, f64>, threshold: f64) -> CrossFeeding {
        match self {
            CommunityRef::Model(c) => c.cross_feeding(fluxes, threshold),
            CommunityRef::Zuniga(c) => c.cross_feeding(fluxes, threshold),
        }
    }
}

impl CommunityRef<'_> {
    /// `community.growthRates(fluxes)`.
    pub fn growth_rates(&self, fluxes: &IndexMap<String, f64>) -> IndexMap<String, f64> {
        match self {
            CommunityRef::Model(c) => c.growth_rates(fluxes),
            CommunityRef::Zuniga(c) => c.growth_rates(fluxes),
        }
    }
}

/// The case's community as a [`CommunityModel`]: the same merged model,
/// members, ownership and transfers, for everything that only reads them.
pub fn zuniga_community_model(c: &ZunigaCommunity) -> CommunityModel {
    CommunityModel {
        name: c.name.clone(),
        // `source-biomass-objective` has no counterpart among the community
        // objectives; nothing rebuilds this community, so it is never read.
        objective_type: Objective::EqualGrowth,
        product_target: None,
        medium: IndexMap::new(),
        species: c
            .species
            .iter()
            .map(|m| SpeciesEntry { id: m.id.clone(), model: m.model.clone(), abundance: m.abundance, enabled: m.enabled, color: m.color.clone() })
            .collect(),
        merged: (*c.merged).clone(),
        reaction_owner: c.reaction_owner.clone(),
        metabolite_owner: c.metabolite_owner.clone(),
        transfers_by_base: c
            .transfers_by_base
            .iter()
            .map(|(base, list)| {
                let list = list
                    .iter()
                    .map(|t| Transfer { species_id: t.species_id.clone(), reaction_id: t.reaction_id.clone(), local_id: t.local_id.clone().unwrap_or_default() })
                    .collect();
                (base.clone(), list)
            })
            .collect(),
        species_compartments: c.species_compartments.clone(),
        biomass_ids: c.biomass_ids.clone(),
        abundances: c.abundances.clone(),
    }
}

// The loader

/// `loadZunigaStudy(manifest, { onProgress })`, replaceable as the JS tests
/// replace `store.loadZunigaStudy`. It runs on a background thread.
pub trait ZunigaLoader: Send + Sync {
    fn load(&self, manifest: &Value, fetcher: &dyn Fetcher, on_progress: &mut dyn FnMut(&str)) -> Result<ZunigaStudy>;
}

/// The real loader: download every pinned source through the store's
/// fetcher (or take it from the cache), verify it, convert the models.
#[derive(Debug, Clone, Default)]
pub struct SourceZunigaLoader {
    /// Where verified downloads are kept between runs (the JS keeps them in a
    /// private IndexedDB store); `None` downloads every time.
    pub cache_dir: Option<PathBuf>,
}

impl ZunigaLoader for SourceZunigaLoader {
    fn load(&self, manifest: &Value, fetcher: &dyn Fetcher, on_progress: &mut dyn FnMut(&str)) -> Result<ZunigaStudy> {
        let adapter = |url: &str| -> coxeter_analysis::Result<coxeter_analysis::FetchResponse> {
            fetcher
                .fetch(url)
                .map(|r| coxeter_analysis::FetchResponse { status: r.status, body: r.body })
                .map_err(|e| coxeter_analysis::Error::new(coxeter_analysis::ErrorKind::Download, e.to_string()))
        };
        let cache = self.cache_dir.as_ref().map(DirectoryCache::new);
        let options = ZunigaLoadOptions {
            fetcher: &adapter,
            cache: cache.as_ref().map(|c| c as &dyn SourceCache),
            on_progress: Some(on_progress),
        };
        Ok(load_zuniga_study(manifest, options)?)
    }
}

/// A Zuñiga load running in the background.
pub(crate) struct ZunigaJob {
    pub generation: u64,
    pub manifest: Value,
    pub scenario_id: String,
}

impl Store {
    /// `isZunigaStudy`.
    pub fn is_zuniga_study(&self) -> bool {
        self.scenario_published().as_deref() == Some(ZUNIGA_ID) && self.zuniga.study.is_some()
    }

    /// The community the analysis functions read: the Zuñiga case's own when
    /// it is open, the ordinary one otherwise.
    pub fn community_view(&self) -> Option<CommunityRef<'_>> {
        if self.is_zuniga_study() {
            if let Some(prepared) = &self.zuniga.prepared {
                return Some(CommunityRef::Zuniga(&prepared.community));
            }
        }
        self.community.as_deref().map(CommunityRef::Model)
    }

    /// `resetZunigaStudy()`.
    pub fn reset_zuniga_study(&mut self) {
        self.zuniga.generation += 1;
        if let Some(show) = self.zuniga.previous_view.take() {
            self.view.show_cross_feeding = show;
        }
        let generation = self.zuniga.generation;
        self.zuniga = ZunigaStudyState { generation, ..ZunigaStudyState::default() };
        // A download still running belongs to the case just left.
        for entry in self.jobs.running.values() {
            if entry.kind == JobKind::Zuniga {
                entry.cancel.store(true, Ordering::SeqCst);
            }
        }
    }

    /// `useZunigaStudy({ study, scenarioId })`: open the case.
    ///
    /// With a `study` in hand (a test fixture, a restored bundle) the case is
    /// installed at once. Without one the sources are downloaded by
    /// [`Store::zuniga_loader`] on a background thread and
    /// [`Store::poll_jobs`] installs them; the call returns as soon as the
    /// download has started. [`Store::use_zuniga_study_blocking`] waits.
    pub fn use_zuniga_study(&mut self, study: Option<ZunigaStudy>, scenario_id: Option<&str>) -> Result<()> {
        let scenario_id = scenario_id.unwrap_or("mutant-coculture").to_string();
        self.invalidate_steady_com();
        self.reset_zuniga_study();
        let generation = self.zuniga.generation;
        self.published_community_id = Some(ZUNIGA_ID.into());
        let name = self
            .published_communities
            .iter()
            .find(|c| c.get("id").and_then(Value::as_str) == Some(ZUNIGA_ID))
            .and_then(|c| c.get("name"))
            .filter(|n| !n.is_null())
            .cloned()
            .unwrap_or_else(|| json!("Zuñiga et al. (2020) — pheA rescue"));
        self.scenario = Some(json!({ "id": format!("published:{ZUNIGA_ID}"), "name": name, "published": ZUNIGA_ID }));
        self.set_status_message(StatusState::Busy, "Loading the authors’ complete models and verifying their SHA-256 hashes…");
        self.emit("status");
        if let Some(study) = study {
            let manifest = study.manifest.clone();
            return self.finish_zuniga_load(generation, Ok(study), manifest, &scenario_id);
        }
        let Some(manifest) = self.published_manifest(ZUNIGA_ID) else {
            return self.finish_zuniga_load(
                generation,
                Err(StoreError::msg("The Zuñiga case manifest is unavailable.")),
                Value::Null,
                &scenario_id,
            );
        };
        let loader = self.zuniga_loader.clone();
        let fetcher = self.fetcher.clone();
        let job = ZunigaJob { generation, manifest: manifest.clone(), scenario_id };
        let cancel = Arc::new(AtomicBool::new(false));
        self.jobs.spawn(JobKind::Zuniga, "zuniga", 0, JobContext::Zuniga(Box::new(job)), cancel, move |id, tx, _cancel| {
            let progress_tx = tx.clone();
            let mut on_progress = |message: &str| {
                let _ = progress_tx.send(JobMessage::Progress { id, message: message.to_string() });
            };
            let result = loader.load(&manifest, fetcher.as_ref(), &mut on_progress);
            let _ = tx.send(JobMessage::Done { id, payload: JobPayload::Zuniga(Box::new(result)) });
        });
        Ok(())
    }

    /// [`Store::use_zuniga_study`] to the end: start the download and wait
    /// for it (what an `await` of the JS call does).
    pub fn use_zuniga_study_blocking(&mut self, scenario_id: Option<&str>, timeout: Duration) -> Result<()> {
        self.use_zuniga_study(None, scenario_id)?;
        let until = Instant::now() + timeout;
        loop {
            for outcome in self.wait_for_kind(JobKind::Zuniga, until.saturating_duration_since(Instant::now())) {
                if outcome.kind == JobKind::Zuniga {
                    return outcome.result;
                }
            }
            if !self.job_running(JobKind::Zuniga) {
                // Installed synchronously (no manifest) or already applied.
                return if self.is_zuniga_study() { Ok(()) } else { Err(StoreError::msg(self.status().message.clone())) };
            }
            if Instant::now() >= until {
                return Err(StoreError::aborted("The Zuñiga models did not load in time."));
            }
        }
    }

    /// A progress message of the download, shown while it is still the
    /// current one.
    pub(crate) fn apply_zuniga_progress(&mut self, generation: u64, message: &str) {
        if generation != self.zuniga.generation {
            return;
        }
        self.set_status_message(StatusState::Busy, message);
        self.emit("status");
    }

    /// The end of a background load.
    pub(crate) fn apply_zuniga_job(&mut self, job: ZunigaJob, result: Result<ZunigaStudy>) -> Result<()> {
        self.finish_zuniga_load(job.generation, result, job.manifest, &job.scenario_id)
    }

    /// The rest of `useZunigaStudy` once the study is loaded (or failed).
    fn finish_zuniga_load(&mut self, generation: u64, loaded: Result<ZunigaStudy>, manifest: Value, scenario_id: &str) -> Result<()> {
        let superseded = || StoreError::aborted("The Zuñiga load was superseded by another scenario.");
        if generation != self.zuniga.generation {
            return Err(match loaded {
                Err(e) if e.is_abort() => e,
                _ => superseded(),
            });
        }
        let attempt = loaded.and_then(|study| self.install_zuniga_study(study, manifest, scenario_id, generation));
        match attempt {
            Ok(()) => Ok(()),
            Err(error) => {
                if generation != self.zuniga.generation {
                    return Err(error);
                }
                self.reset_zuniga_study();
                self.community = None;
                self.set_problem(None);
                self.map = None;
                self.solution = None;
                self.scale = None;
                self.growth = IndexMap::new();
                self.cross_feeding = CrossFeeding::default();
                self.cross_feeding_explorer.index = None;
                self.map_history.clear();
                self.selected_reaction = None;
                self.selected_entity = None;
                self.set_status_message(StatusState::Error, error.to_string());
                self.emit("rebuild");
                Err(error)
            }
        }
    }

    fn install_zuniga_study(&mut self, study: ZunigaStudy, manifest: Value, scenario_id: &str, generation: u64) -> Result<()> {
        let manifest = if manifest.is_null() { study.manifest.clone() } else { manifest };
        let study = Arc::new(study);
        self.zuniga.study = Some(study.clone());
        self.published_manifests.insert(ZUNIGA_ID.into(), Some(manifest.clone()));
        self.species = ["Ec", "Se"]
            .iter()
            .map(|id| {
                let file = format!("published/{ZUNIGA_ID}/model-{id}");
                if let Some(model) = study.models.get(id) {
                    self.models.insert_arc(file.clone(), model.clone());
                }
                Species::new(id, &file, 1.0, true)
            })
            .collect();
        self.model_editor_species_id = Some("Ec".into());
        self.model_editor_mode = None;
        self.objective = "published-original".into();
        self.zuniga.previous_view = Some(self.view.show_cross_feeding);
        self.view.show_cross_feeding = false;
        self.view.reaction_color_mode = coxeter_render::reaction_colors::ReactionColorMode::Flux;
        self.view.pathway_ribbon_mode = coxeter_render::reaction_colors::PathwayRibbonMode::Off;
        self.product_target = None;
        self.custom_objective = None;
        self.parsimonious = false;
        self.loopless = false;
        self.fva_enabled = false;
        self.cancel_fva(true, false);
        self.medium = IndexMap::new();
        self.overrides.clear();
        self.manual_pathways.clear();
        self.pathway_settings.clear();
        self.clear_omics(false);
        self.joint_projection_spec = None;
        self.community_exchange_ids = None;
        self.steady_com_config = SteadyComConfig::default();
        self.selected_reaction = None;
        self.selected_geometry = None;
        self.selected_entity = None;
        self.multi_selection = empty_selection();
        self.variability = None;
        for scenario in ZUNIGA_SCENARIOS.iter() {
            self.set_status_message(StatusState::Busy, format!("Calculating {} on the complete model…", scenario.name));
            self.emit("status");
            if generation != self.zuniga.generation {
                return Err(StoreError::aborted("The Zuñiga load was superseded by another scenario."));
            }
            let prepared = create_zuniga_scenario(&study, scenario.id)?;
            let solution = zuniga_solution(&prepared);
            if !solution.is_optimal() {
                let why = if solution.message.is_empty() { solution.status.as_str().to_string() } else { solution.message.clone() };
                return Err(StoreError::msg(format!("{}: {why}", scenario.name)));
            }
            self.zuniga.solutions.insert(scenario.id.to_string(), Arc::new(solution));
        }
        let map_file = manifest.get("map").filter(|m| !m.is_null()).map_or_else(|| "map.json".to_string(), js::to_string);
        let drawing = self.assets.read_json(&format!("models/published/{ZUNIGA_ID}/{map_file}"))?;
        let doc = drawing.get("map").filter(|m| !m.is_null()).unwrap_or(&drawing);
        self.map = Some(EditableMap::from_json(doc)?);
        self.map_mode = "published-case".into();
        self.map_history.clear();
        self.use_zuniga_scenario(scenario_id, false)?;
        self.settle_opened_work();
        self.emit("rebuild");
        Ok(())
    }

    /// `useZunigaScenario(id, { emit })`: switch the case to one of its three
    /// scenarios. The drawing and its history stay; the exact model, its
    /// knockout and the solution change.
    pub fn use_zuniga_scenario(&mut self, id: &str, emit: bool) -> Result<Arc<Solution>> {
        if !self.is_zuniga_study() {
            return Err(StoreError::msg("Open the Zuñiga published case first."));
        }
        let Some(scenario) = zuniga_scenario(id) else {
            return Err(StoreError::msg(format!("Unknown Zuñiga case scenario: {id}")));
        };
        let study = self.zuniga.study.clone().expect("is_zuniga_study");
        let prepared = Arc::new(create_zuniga_scenario(&study, id)?);
        self.zuniga.prepared = Some(prepared.clone());
        self.zuniga.scenario_id = Some(id.to_string());
        self.community = Some(Arc::new(zuniga_community_model(&prepared.community)));
        self.community_exchange_ids = None;
        self.default_bounds = Some(prepared.problem.snapshot_bounds());
        self.set_problem(Some(prepared.problem.clone()));
        self.overrides.clear();
        let record_name = self
            .published_communities
            .iter()
            .find(|c| c.get("id").and_then(Value::as_str) == Some(ZUNIGA_ID))
            .and_then(|c| c.get("name"))
            .filter(|n| !n.is_null())
            .map_or_else(|| "Zuñiga et al. (2020)".to_string(), js::to_string);
        if let Some(Value::Object(s)) = self.scenario.as_mut() {
            s.insert("caseScenario".into(), json!(id));
            s.insert("name".into(), json!(format!("{record_name} — {}", scenario.name)));
        }
        self.selected_reaction = None;
        self.selected_entity = None;
        let solution = match self.zuniga.solutions.get(id) {
            Some(s) => {
                // The JS writes growth and cross-feeding into the stored
                // result again; they are the same numbers.
                let mut s = (**s).clone();
                if let Some(record) = s.zuniga.as_mut() {
                    let mut r = (**record).clone();
                    r.growth = prepared.community.growth_rates(&s.fluxes);
                    r.cross_feeding = prepared.community.cross_feeding(&s.fluxes, 1e-6);
                    *record = Arc::new(r);
                }
                s
            }
            None => zuniga_solution(&prepared),
        };
        let solution = Arc::new(solution);
        self.zuniga.solutions.insert(id.to_string(), solution.clone());
        self.apply_solution_arc(solution.clone(), false);
        self.set_status_message(
            if solution.is_optimal() { StatusState::Ok } else { StatusState::Error },
            format!("{} · exact source constraints · stationary FBA adaptation of the authors’ community model.", scenario.name),
        );
        if emit {
            self.emit("solution");
        }
        Ok(solution)
    }

    /// The `solve()` of the case: the open scenario again, on its exact LP.
    pub(crate) fn solve_zuniga(&mut self, silent: bool) -> Option<Arc<Solution>> {
        let prepared = self.zuniga.prepared.clone()?;
        let t = Instant::now();
        let solution = Arc::new(zuniga_solution(&prepared));
        let id = self.zuniga.scenario_id.clone().unwrap_or_default();
        self.zuniga.solutions.insert(id.clone(), solution.clone());
        self.apply_solution_arc(solution.clone(), false);
        // Like an ordinary solve: a success leaves the header to its summary
        // and goes to the log. Shown as a notice it would hide the objective
        // for six seconds after every re-solve.
        let name = zuniga_scenario(&id).map_or("undefined", |s| s.name);
        let case_name = format!("{name} · exact source constraints");
        let optimal = solution.is_optimal();
        let solve_ms = js::round(t.elapsed().as_secs_f64() * 1000.0);
        let next = Status {
            state: if optimal { StatusState::Ok } else { StatusState::Error },
            message: if optimal { String::new() } else { case_name.clone() },
            solve_ms,
            ..self.status().clone()
        };
        self.set_status(next);
        if optimal {
            self.add_log(StatusState::Ok, &format!("{case_name} solved: optimal, {} ms", js::number_to_string(solve_ms)), LogKind::Solve);
        }
        if !silent {
            self.emit("solution");
        }
        Some(solution)
    }

    /// `setZunigaComparison(mode)`: absolute fluxes, or their difference from
    /// the WT monoculture on a fixed scale.
    pub fn set_zuniga_comparison(&mut self, mode: &str) -> Result<()> {
        if !self.is_zuniga_study() {
            return Ok(());
        }
        if mode != "absolute" && mode != "difference" {
            return Err(StoreError::msg("The comparison must be absolute or difference."));
        }
        self.zuniga.comparison = mode.to_string();
        if mode == "difference" {
            self.view.reaction_color_mode = coxeter_render::reaction_colors::ReactionColorMode::Flux;
        }
        self.refresh_flux_scale();
        self.emit("view");
        Ok(())
    }

    /// `zunigaState()`: what a saved map or scenario records about the case,
    /// or `null`.
    pub fn zuniga_state(&self) -> Value {
        if !self.is_zuniga_study() {
            return Value::Null;
        }
        let study = self.zuniga.study.as_ref().expect("is_zuniga_study");
        let flux_reference = study.manifest.get("fluxReference").filter(|v| !v.is_null()).cloned().unwrap_or(json!(0.1));
        json!({
            "id": ZUNIGA_ID,
            "scenarioId": self.zuniga.scenario_id,
            "comparison": self.zuniga.comparison,
            "fluxReference": flux_reference,
            "sourceHashes": study.source_hashes,
        })
    }

    /// `validateZunigaState(state)`.
    pub fn validate_zuniga_state(&self, state: &Value) -> Result<()> {
        let scenario = state.get("scenarioId").and_then(Value::as_str);
        if !scenario.is_some_and(|id| zuniga_scenario(id).is_some()) {
            return Err(StoreError::msg("The saved Zuñiga scenario is invalid."));
        }
        let comparison = state.get("comparison").filter(|v| !v.is_null()).and_then(Value::as_str).unwrap_or("absolute");
        if comparison != "absolute" && comparison != "difference" {
            return Err(StoreError::msg("The saved Zuñiga comparison is invalid."));
        }
        let current = self.zuniga_state();
        if let Some(reference) = state.get("fluxReference") {
            let same = match (reference.as_f64(), current.get("fluxReference").and_then(Value::as_f64)) {
                (Some(a), Some(b)) => a == b,
                _ => Some(reference) == current.get("fluxReference"),
            };
            if !same {
                return Err(StoreError::msg("The saved Zuñiga map uses a different flux scale reference."));
            }
        }
        if let (Some(saved), Some(Value::Object(hashes))) = (state.get("sourceHashes").filter(|v| js::truthy(v)), current.get("sourceHashes")) {
            if hashes.iter().any(|(file, hash)| saved.get(file) != Some(hash)) {
                return Err(StoreError::msg("The saved Zuñiga document refers to different source model hashes."));
            }
        }
        Ok(())
    }

    /// The case's study as a reproducibility bundle archives it.
    pub(crate) fn zuniga_bundle_state(&self) -> Option<coxeter_io::reproducibility::ZunigaStudyState> {
        if !self.is_zuniga_study() {
            return None;
        }
        let study = self.zuniga.study.as_ref()?;
        Some(coxeter_io::reproducibility::ZunigaStudyState {
            state: self.zuniga_state(),
            manifest: study.manifest.clone(),
            source_models: json!({ "mono": study.source_models.mono, "joint": study.source_models.joint }),
            active_model: self.community.as_ref().map_or(Value::Null, |c| c.merged.to_json()),
        })
    }

    /// `importZunigaBundle(bundle, check)`: replay a private bundle against
    /// its complete source models, without the generic community merger.
    pub fn import_zuniga_bundle(&mut self, bundle: &Value) -> Result<Value> {
        let check = verify_reproducibility_bundle(bundle)?;
        let archived = bundle.get("study").cloned().unwrap_or(Value::Null);
        let source = |k: &str| archived.get("sourceModels").and_then(|s| s.get(k)).filter(|v| js::truthy(v)).cloned();
        let (Some(mono), Some(joint)) = (source("mono"), source("joint")) else {
            return Err(StoreError::msg("The private Zuñiga bundle lacks its complete source models."));
        };
        let model = |id: &str| -> Result<Arc<Model>> {
            let doc = bundle.get("models").and_then(|m| m.get(id)).cloned().unwrap_or(Value::Null);
            Ok(Arc::new(Model::from_value(&doc)?))
        };
        let source_hashes: IndexMap<String, String> = match archived.get("sourceHashes") {
            Some(Value::Object(m)) => m.iter().map(|(k, v)| (k.clone(), js::to_string(v))).collect(),
            _ => IndexMap::new(),
        };
        let study = ZunigaStudy {
            manifest: archived.get("manifest").cloned().unwrap_or(Value::Null),
            source_hashes,
            source_models: ZunigaSourceModels { mono, joint },
            models: ZunigaModels { ec: model("Ec")?, se: model("Se")? },
        };
        let context_study = study.clone();
        let scenario_id = archived.get("scenarioId").and_then(Value::as_str).map(str::to_string);
        self.use_zuniga_study(Some(study), scenario_id.as_deref())?;
        let comparison = archived.get("comparison").filter(|v| !v.is_null()).and_then(Value::as_str).unwrap_or("absolute").to_string();
        self.set_zuniga_comparison(&comparison)?;
        if let Some(map) = bundle.get("map").filter(|m| js::truthy(m)) {
            let context = create_zuniga_scenario(&context_study, "mutant-coculture")?;
            let community = zuniga_community_model(&context.community);
            crate::documents::validate_map_document(map, &community, &self.species)?;
            self.map = Some(EditableMap::from_json(map)?);
            self.map_history.clear();
        }
        let tolerances = bundle
            .get("replay")
            .and_then(|r| r.get("tolerances"))
            .filter(|t| !t.is_null())
            .map(|t| Tolerances {
                absolute: t.get("absolute").and_then(Value::as_f64).unwrap_or(1e-7),
                relative: t.get("relative").and_then(Value::as_f64).unwrap_or(1e-6),
            })
            .unwrap_or_default();
        let current = self.solution.as_ref().map_or(Value::Null, |s| s.replay_json());
        let replay = compare_replayed_solution(bundle.get("solution").unwrap_or(&Value::Null), &current, tolerances);
        let mut check_json = serde_json::to_value(&check)?;
        if let Some(o) = check_json.as_object_mut() {
            o.insert("replay".into(), replay.to_json());
        }
        self.reproducibility_replay = Some(json!({
            "integrity": check_json,
            "replay": replay.to_json(),
            "archivedAt": bundle.get("metadata").and_then(|m| m.get("createdAt")).cloned().unwrap_or(Value::Null),
            "restored": { "map": bundle.get("map").is_some_and(js::truthy), "omicsExpected": false, "omics": false },
        }));
        self.settle_opened_work();
        let hash: String =
            bundle.get("integrity").and_then(|i| i.get("payloadHash")).and_then(Value::as_str).unwrap_or("").chars().take(12).collect();
        self.set_status_message(
            if replay.pass { StatusState::Ok } else { StatusState::Warn },
            format!(
                "Private Zuñiga bundle restored · {} · SHA-256 {hash}…",
                if replay.pass { "numerical replay OK" } else { "numerical differences" }
            ),
        );
        self.emit("rebuild");
        Ok(check_json)
    }

    /// The knockouts of the open Zuñiga scenario (`zunigaPrepared.knockouts`).
    pub fn zuniga_knockouts(&self) -> Vec<String> {
        self.zuniga.prepared.as_ref().map(|p| p.knockouts.clone()).unwrap_or_default()
    }
}

/// `solveZunigaScenario(prepared)` with the growth and cross-feeding the
/// store records on it.
fn zuniga_solution(prepared: &ZunigaPrepared) -> Solution {
    let solved = solve_zuniga_scenario(prepared);
    let growth = prepared.community.growth_rates(&solved.result.fluxes);
    let cross_feeding = prepared.community.cross_feeding(&solved.result.fluxes, 1e-6);
    let mut solution = Solution::from_result(solved.result);
    solution.solver = "glpk".into();
    solution.zuniga = Some(Arc::new(ZunigaRecord { run: solved.zuniga, growth, cross_feeding }));
    solution
}
