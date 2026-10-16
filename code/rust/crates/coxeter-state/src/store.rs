//! Application state (port of `Store` in `src/ui/store.js`): the struct, its
//! construction, loading, events, the status line and the log.
//!
//! Two kinds of change, with very different costs:
//!
//! ```text
//!   structural  species set, abundances, objective  ->  rebuild (+ relayout)
//!   numeric     medium limits, reaction bounds      ->  re-solve only
//! ```
//!
//! Keeping them apart is what makes dragging a bound slider feel live: only
//! the LP is re-run, the map geometry is untouched.
//!
//! Three layers meet here and are kept apart:
//!
//! ```text
//!   model       CommunityModel — everything the biology makes available
//!   map         EditableMap    — what is currently drawn
//!   simulation  FluxProblem + solution — the LP and its answer
//! ```
//!
//! The simulation reads the model, never the map, so what is on screen cannot
//! change a flux. The map reads the model when it is built and then owns its
//! own geometry.
//!
//! Medium values are stored as positive magnitudes (mmol/gDW/h available to
//! the community) and converted to the negative lower bound an exchange
//! reaction wants at the point of use.
//!
//! The rest of the store lives beside this file, one concern per module, with
//! the JavaScript method names in snake_case: `scenarios.rs` (loading and
//! rebuilding), `simulation.rs` (solving, FVA, bounds), `steadycom.rs`,
//! `map_edit.rs`, `pathways.rs`, `documents.rs` (files in and out), `work.rs`
//! (saved/unsaved), `omics.rs`, `cross_feeding.rs`, `zuniga.rs`,
//! `model_editor.rs`, `render_state.rs` and `jobs.rs` (background work).

use std::sync::Arc;

use coxeter_core::community::CommunityModel;
use coxeter_core::community::CrossFeeding;
use coxeter_core::model::Model;
use coxeter_map::editable_map::EditableMap;
use coxeter_map::history::MapHistory;
use coxeter_map::layout_settings::{DEFAULT_LAYOUT_PRESET, LayoutSettings, layout_preset};
use coxeter_map::multi_selection::Selection;
use coxeter_render::state::GeometrySelection;
use coxeter_render::theme::FluxScale;
use coxeter_solver::fba::{BoundsSnapshot, FluxProblem};
use coxeter_solver::steadycom::SteadyComSpec;
use coxeter_solver::steadycom_runtime::CommunityExchangeIds;
use indexmap::{IndexMap, IndexSet};
use serde_json::Value;

use crate::assets::{Assets, DirAssets, Fetcher, NoFetcher};
use crate::cross_feeding::CrossFeedingExplorer;
use crate::error::{Result, StoreError};
use crate::jobs::Jobs;
use crate::omics::{CommunityOmics, OmicsDataset, OmicsSettings};
use crate::operation_boundary::{OperationDomain, assert_operation_domain, operation_domain};
use crate::steadycom::{CachedSpec, HighsSteadyComRunner, SteadyComRunner};
use crate::types::*;
use crate::zuniga::{SourceZunigaLoader, ZunigaLoader, ZunigaStudyState};

/// How many lines the log keeps.
pub const LOG_LIMIT: usize = 200;

/// Above this many reactions an upload is not solved on arrival.
pub const AUTO_SOLVE_REACTION_LIMIT: usize = 1500;

/// When an uploaded model is drawn in full without being asked.
///
/// The automatic layout resolves overlaps inside each compartment, and that
/// cost climbs steeply with how crowded one cell gets. Measured in Chromium on
/// one cell: 95 reactions / 72 metabolites take 0.3 s, 300 / 453 take 1.4 s,
/// 500 / 667 take 3.2 s, and iAF1260 (2383 / 1668) takes over a minute and is
/// unreadable at the end of it. Metabolite count dominates, so both are
/// capped. Below these limits the draw costs about what the bundled scenarios
/// cost, so it happens on upload; above them the core-metabolism overview is
/// drawn instead and the full map stays one explicit click away.
pub const AUTO_DRAW_REACTION_LIMIT: usize = 350;
/// The metabolite half of that limit (see [`AUTO_DRAW_REACTION_LIMIT`]).
pub const AUTO_DRAW_METABOLITE_LIMIT: usize = 500;

/// The community objective strings (`OBJECTIVES` of `community.js`).
pub mod objectives {
    pub const EQUAL_GROWTH: &str = "equal-growth";
    pub const SUM_BIOMASS: &str = "sum-biomass";
    pub const PRODUCT: &str = "product";
}

/// The application's state and every action it can take. See the module
/// documentation; the fields are the JavaScript store's, in snake_case.
pub struct Store {
    /// Where bundled files come from (`fetch(`${base}/models/…`)`).
    pub assets: Arc<dyn Assets>,
    /// Where downloads come from (Yeast9, the Zuñiga sources).
    pub fetcher: Arc<dyn Fetcher>,
    /// `models/index.json` → `models`.
    pub catalog: Vec<Value>,
    pub scenarios: Vec<Value>,
    /// Benchmark communities from the literature. Metadata only: the papers
    /// these come from use genome-scale models, which are reproduced
    /// headlessly rather than drawn as a map.
    pub published_communities: Vec<Value>,
    pub published_community_id: Option<String>,
    /// Manifests of the generated assets, one read each. A community with no
    /// manifest has nothing downloadable and no map (`None`).
    pub published_manifests: IndexMap<String, Option<Value>>,

    /// What SteadyCom has been asked so far for the open community.
    pub steady_com_state: Option<SteadyComState>,
    pub(crate) steady_com_spec: Option<CachedSpec>,
    pub(crate) steady_com_revision: u64,
    pub(crate) steady_com_generation: u64,
    /// The cancel flag of the SteadyCom task in flight (`_steadyComController`).
    pub(crate) steady_com_controller: Option<Arc<std::sync::atomic::AtomicBool>>,
    pub steady_com_config: SteadyComConfig,
    /// `runSteadyComTask`, replaceable as the JS tests replace it.
    pub steady_com_runner: Arc<dyn SteadyComRunner>,
    pub(crate) next_state_serial: u64,

    /// The Zuñiga published case.
    pub zuniga: ZunigaStudyState,
    /// `loadZunigaStudy`, replaceable as the JS tests replace it.
    pub zuniga_loader: Arc<dyn ZunigaLoader>,

    /// File stem → model.
    pub models: ModelCatalog,
    pub uploaded_model_keys: IndexSet<String>,

    /// The open scenario, as the catalogue (or a loader) gives it.
    pub scenario: Option<Value>,
    pub species: Vec<Species>,
    pub objective: String,
    pub product_target: Option<String>,
    /// false = FBA, true = pFBA.
    pub parsimonious: bool,
    /// Percentage of the primary optimum fixed in pFBA.
    pub pfba_biomass_percent: f64,
    /// Exact loopless FBA (MILP).
    pub loopless: bool,
    pub fva_enabled: bool,
    pub fva_ranges: IndexMap<String, FvaRange>,
    pub fva_progress: FvaProgress,
    pub(crate) fva_generation: u64,
    /// Base metabolite → uptake limit (positive).
    pub medium: IndexMap<String, f64>,

    pub community: Option<Arc<CommunityModel>>,
    /// `community.communityExchangeIds` of a joint projection.
    pub community_exchange_ids: Option<IndexMap<String, CommunityExchangeIds>>,
    pub problem: Option<Arc<FluxProblem>>,
    /// Changes whenever `problem` is replaced (the JS compares the object).
    pub(crate) problem_serial: u64,
    /// Changes whenever the problem's bounds are set again.
    pub(crate) bounds_revision: u64,
    /// Generation of background solves (a newer solve supersedes them).
    pub(crate) solve_generation: u64,
    pub default_bounds: Option<BoundsSnapshot>,
    /// What is drawn.
    pub map: Option<EditableMap>,
    pub map_history: MapHistory,
    /// Model identity → biological edits made to it.
    pub(crate) model_edits: EditCounts,
    /// Model identity → edits written to a file.
    pub(crate) model_edits_saved: EditCounts,
    /// `{ context, scenario, shared }`: see `unsaved_work()`.
    pub saved_work: Option<SavedWork>,
    pub solution: Option<Arc<Solution>>,
    pub scale: Option<FluxScale>,
    pub cross_feeding: CrossFeeding,
    pub cross_feeding_explorer: CrossFeedingExplorer,
    pub growth: IndexMap<String, f64>,
    /// Merged reaction id → `{ lower, upper }`.
    pub overrides: IndexMap<String, Bounds>,
    pub manual_pathways: IndexMap<String, ManualPathway>,
    /// Model pathway name → `{ color?, visible? }`.
    pub pathway_settings: IndexMap<String, PathwaySetting>,
    pub omics_dataset: Option<OmicsDataset>,
    pub(crate) omics_dataset_revision: u64,
    pub omics_settings: OmicsSettings,

    pub selected_reaction: Option<String>,
    pub selected_geometry: Option<GeometrySelection>,
    pub selected_entity: Option<SelectedEntity>,
    pub multi_selection: Selection,
    pub reaction_search_query: String,
    pub model_editor_species_id: Option<String>,
    pub model_editor_mode: Option<String>,
    pub cell_placement_species_id: Option<String>,
    pub compartment_placement_key: Option<String>,
    /// The shape a newly drawn compartment gets: `auto` is the one its kind
    /// has, else a name of `geometry::COMPARTMENT_SHAPES`.
    pub compartment_shape_preset: String,
    pub variability: Option<FvaRange>,
    pub layout_settings: LayoutSettings,
    /// `full`, `empty`, `core-overview` or `published-case`.
    pub map_mode: String,
    pub view: View,
    /// Community omics (metatranscriptomics) and GIMME.
    pub omics: CommunityOmics,
    /// What the Logs tab lists: see `add_log()`.
    pub log: Vec<LogEntry>,
    status: Status,
    pub reproducibility_replay: Option<Value>,
    /// What the last map load had to do to make the file fit, for the status
    /// line.
    pub map_load_note: String,
    /// A map opened before its model was loaded: see `load_map_document`.
    pub waiting_map: Option<WaitingMap>,
    /// The analysis the Simulate panel started last.
    pub last_run: Option<LastRun>,
    pub(crate) next_run_serial: u64,
    /// The joint SteadyCom spec of the nine-species example, when open.
    pub joint_projection_spec: Option<Arc<SteadyComSpec>>,
    pub solver_ready: bool,

    events: Vec<String>,
    pub(crate) jobs: Jobs,
}

impl Store {
    /// `new Store(base)`: nothing loaded yet; call [`Store::init`].
    pub fn new(assets: Arc<dyn Assets>) -> Store {
        Store {
            assets,
            fetcher: Arc::new(NoFetcher),
            catalog: Vec::new(),
            scenarios: Vec::new(),
            published_communities: Vec::new(),
            published_community_id: None,
            published_manifests: IndexMap::new(),
            steady_com_state: None,
            steady_com_spec: None,
            steady_com_revision: 0,
            steady_com_generation: 0,
            steady_com_controller: None,
            steady_com_config: SteadyComConfig::default(),
            steady_com_runner: Arc::new(HighsSteadyComRunner),
            next_state_serial: 0,
            zuniga: ZunigaStudyState::default(),
            zuniga_loader: Arc::new(SourceZunigaLoader::default()),
            models: ModelCatalog::default(),
            uploaded_model_keys: IndexSet::new(),
            scenario: None,
            species: Vec::new(),
            objective: objectives::EQUAL_GROWTH.into(),
            product_target: None,
            parsimonious: false,
            pfba_biomass_percent: 100.0,
            loopless: false,
            fva_enabled: false,
            fva_ranges: IndexMap::new(),
            fva_progress: FvaProgress::default(),
            fva_generation: 0,
            medium: IndexMap::new(),
            community: None,
            community_exchange_ids: None,
            problem: None,
            problem_serial: 0,
            bounds_revision: 0,
            solve_generation: 0,
            default_bounds: None,
            map: None,
            map_history: MapHistory::new(100),
            model_edits: EditCounts::new(),
            model_edits_saved: EditCounts::new(),
            saved_work: None,
            solution: None,
            scale: None,
            cross_feeding: CrossFeeding::default(),
            cross_feeding_explorer: CrossFeedingExplorer::default(),
            growth: IndexMap::new(),
            overrides: IndexMap::new(),
            manual_pathways: IndexMap::new(),
            pathway_settings: IndexMap::new(),
            omics_dataset: None,
            omics_dataset_revision: 0,
            omics_settings: OmicsSettings::default(),
            selected_reaction: None,
            selected_geometry: None,
            selected_entity: None,
            multi_selection: Selection::default(),
            reaction_search_query: String::new(),
            model_editor_species_id: None,
            model_editor_mode: None,
            cell_placement_species_id: None,
            compartment_placement_key: None,
            compartment_shape_preset: "auto".into(),
            variability: None,
            layout_settings: layout_preset(Some(DEFAULT_LAYOUT_PRESET)),
            map_mode: "full".into(),
            view: View::default(),
            omics: CommunityOmics::default(),
            log: Vec::new(),
            status: Status::default(),
            reproducibility_replay: None,
            map_load_note: String::new(),
            waiting_map: None,
            last_run: None,
            next_run_serial: 0,
            joint_projection_spec: None,
            solver_ready: true,
            events: Vec::new(),
            jobs: Jobs::new(),
        }
    }

    /// A store reading its bundled files from a directory (the repository
    /// root in development).
    pub fn with_dir(root: impl Into<std::path::PathBuf>) -> Store {
        Store::new(Arc::new(DirAssets::new(root)))
    }

    /// Use this fetcher for downloads.
    pub fn with_fetcher(mut self, fetcher: Arc<dyn Fetcher>) -> Store {
        self.fetcher = fetcher;
        self
    }

    /// Keep the verified Zuñiga sources in this folder between runs.
    pub fn with_download_cache(mut self, dir: impl Into<std::path::PathBuf>) -> Store {
        self.zuniga_loader = Arc::new(SourceZunigaLoader { cache_dir: Some(dir.into()) });
        self
    }

    /// `init()`: read the catalogue, the scenarios and the published
    /// communities, and load every bundled model.
    pub fn init(&mut self) -> Result<&mut Store> {
        // HiGHS is linked in; there is no optional solver to wait for.
        self.solver_ready = true;
        let index = self.assets.read_json("models/index.json")?;
        let scenarios = self.assets.read_json("models/scenarios.json")?;
        self.catalog = index.get("models").and_then(Value::as_array).cloned().unwrap_or_default();
        self.scenarios = scenarios.get("scenarios").and_then(Value::as_array).cloned().unwrap_or_default();
        self.published_communities =
            scenarios.get("publishedCommunities").and_then(Value::as_array).cloned().unwrap_or_default();
        // Open on one that actually reproduces its paper. The list stays in the
        // registry's order, which follows the papers, but landing on a blocked
        // entry would show an empty result where there is a real one to show.
        let reproduced = self.published_communities.iter().find(|c| {
            c.get("reproduction").and_then(|r| r.get("status")).and_then(Value::as_str) == Some("reproduced")
        });
        self.published_community_id = reproduced
            .or(self.published_communities.first())
            .and_then(|c| c.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let files: Vec<String> = self
            .catalog
            .iter()
            .filter_map(|m| m.get("file").and_then(Value::as_str))
            .map(|f| f.strip_suffix(".json").unwrap_or(f).to_string())
            .collect();
        for name in files {
            self.load_model(&name)?;
        }
        Ok(self)
    }

    /// `loadModel(name)`: a model of the catalogue (`models/<name>.json`),
    /// read once.
    pub fn load_model(&mut self, name: &str) -> Result<Arc<Model>> {
        if let Some(model) = self.models.arc(name) {
            return Ok(model);
        }
        let doc = self.assets.read_json(&format!("models/{name}.json"))?;
        let model = Model::from_value(&doc)?;
        self.models.insert(name, model);
        Ok(self.models.arc(name).expect("just inserted"))
    }

    /// Put a model in the catalogue under `key` (`store.models.set`).
    pub fn set_model(&mut self, key: &str, model: Model) {
        self.models.insert(key, model);
    }

    /// The drawn map, under its historical name (`store.layout`).
    pub fn layout(&self) -> Option<&EditableMap> {
        self.map.as_ref()
    }

    // Events

    /// `emit(reason)`: announce a change. The views read the reasons with
    /// [`Store::take_events`] to know what to rebuild.
    pub fn emit(&mut self, reason: &str) {
        self.track_work_context();
        self.events.push(reason.to_string());
    }

    /// Every reason emitted since the last call, oldest first.
    pub fn take_events(&mut self) -> Vec<String> {
        std::mem::take(&mut self.events)
    }

    /// The reasons waiting to be taken.
    pub fn pending_events(&self) -> &[String] {
        &self.events
    }

    // Status

    /// `store.status`.
    pub fn status(&self) -> &Status {
        &self.status
    }

    /// `store.status = value`.
    ///
    /// The status line holds one message at a time, and the one before it is
    /// gone the moment anything else happens. Every message that is set also
    /// goes to the log, which is what the Logs tab lists, so an import note, a
    /// warning or an error can still be read after the next solve has
    /// replaced it.
    pub fn set_status(&mut self, value: Status) {
        let log = !value.message.is_empty() && matches!(value.state, StatusState::Ok | StatusState::Warn | StatusState::Error);
        let (state, message) = (value.state, value.message.clone());
        self.status = value;
        if log {
            self.add_log(state, &message, LogKind::Message);
        }
    }

    /// `this.status = { ...this.status, state, message }`.
    pub fn set_status_message(&mut self, state: StatusState, message: impl Into<String>) {
        let next = Status { state, message: message.into(), ..self.status.clone() };
        self.set_status(next);
    }

    /// Change one field of the status in place (`this.status.message = …`),
    /// which, as in JS, does not go to the log.
    pub fn status_mut(&mut self) -> &mut Status {
        &mut self.status
    }

    /// `addLog(level, message, { kind })`: one line, newest last. The same
    /// line twice in a row is one entry with a count; so are back-to-back
    /// solves (`kind: 'solve'`), which a slider drag produces by the dozen and
    /// which would push everything else out.
    pub fn add_log(&mut self, level: StatusState, message: &str, kind: LogKind) -> &LogEntry {
        let now = now_ms();
        let fold = self.log.last().is_some_and(|last| {
            last.level == level && (last.message == message || (kind == LogKind::Solve && last.kind == LogKind::Solve))
        });
        if fold {
            let last = self.log.last_mut().expect("checked");
            last.message = message.to_string();
            last.time = now;
            last.count += 1;
            return self.log.last().expect("checked");
        }
        self.log.push(LogEntry { time: now, level, message: message.to_string(), kind, count: 1 });
        if self.log.len() > LOG_LIMIT {
            let excess = self.log.len() - LOG_LIMIT;
            self.log.drain(..excess);
        }
        self.log.last().expect("just pushed")
    }

    /// Empty the Logs tab.
    pub fn clear_log(&mut self) {
        self.log.clear();
        self.emit("status");
    }

    // Edit domains

    /// Public contract used by UI code and tests to keep BUILD and MODEL
    /// edits separate.
    pub fn editing_operation_domain(&self, name: &str) -> Option<OperationDomain> {
        operation_domain(name)
    }
    /// An error unless `name` is a MAP operation.
    pub fn assert_map_operation(&self, name: &str) -> Result<bool> {
        assert_operation_domain(name, OperationDomain::Map)
    }
    /// An error unless `name` is a MODEL operation.
    pub fn assert_model_operation(&self, name: &str) -> Result<bool> {
        assert_operation_domain(name, OperationDomain::Model)
    }

    // Small reads

    /// A string field of the open scenario.
    pub fn scenario_str(&self, key: &str) -> Option<String> {
        match self.scenario.as_ref()?.get(key)? {
            Value::Null => None,
            Value::String(s) => Some(s.clone()),
            other => Some(coxeter_core::js::to_string(other)),
        }
    }

    /// `this.scenario?.id`.
    pub fn scenario_id(&self) -> Option<String> {
        self.scenario_str("id")
    }

    /// `this.scenario?.published`.
    pub fn scenario_published(&self) -> Option<String> {
        self.scenario_str("published")
    }

    /// The species with this id.
    pub fn species_by_id(&self, id: &str) -> Option<&Species> {
        self.species.iter().find(|sp| sp.id == id)
    }

    /// Edit the community in place (copy-on-write if a job holds it).
    pub(crate) fn community_mut(&mut self) -> Option<&mut CommunityModel> {
        self.community.as_mut().map(Arc::make_mut)
    }

    /// Edit the flux problem in place (copy-on-write if a job holds it).
    pub(crate) fn problem_mut(&mut self) -> Option<&mut FluxProblem> {
        self.problem.as_mut().map(Arc::make_mut)
    }

    /// Install a new flux problem built from the community's merged model.
    pub(crate) fn set_problem(&mut self, problem: Option<FluxProblem>) {
        self.problem = problem.map(Arc::new);
        self.problem_serial += 1;
        self.bounds_revision += 1;
    }

    /// A new identity for a SteadyCom state object.
    pub(crate) fn next_serial(&mut self) -> u64 {
        self.next_state_serial += 1;
        self.next_state_serial
    }

    /// The error a published case throws when its constraints would change
    /// (`assertPublishedStudyConstraints`).
    pub fn assert_published_study_constraints(&self) -> Result<()> {
        if self.is_zuniga_study() {
            return Err(StoreError::msg(
                "The Zuñiga case preserves the authors’ objective, medium, bounds and population coefficients. Choose a documented case scenario to change these conditions. Map geometry remains editable.",
            ));
        }
        Ok(())
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        // Background work has no one to report to any more.
        self.jobs.cancel_all();
    }
}
