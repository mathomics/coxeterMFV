//! The plain data the store holds: the JavaScript objects of `store.js`, typed.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use coxeter_core::js;
use coxeter_core::model::Model;
use coxeter_core::text_style::{DEFAULT_TEXT_FONT, TEXT_SIZE};
use coxeter_render::reaction_colors::{PathwayRibbonMode, ReactionColorMode};
use coxeter_render::theme::FluxColorStop;
use coxeter_solver::fba::FluxResult;
use coxeter_solver::lp::LpStatus;
use coxeter_solver::steadycom::{SteadyComProgress, SteadyComSpec};
use indexmap::{IndexMap, IndexSet};
use serde_json::{Map, Value, json};

// Numbers

/// What a numeric control holds: a number, the text typed into it, or
/// nothing. JavaScript callers pass either; [`finite_input`] reads it.
#[derive(Debug, Clone, PartialEq)]
pub enum UserNumber {
    Number(f64),
    Text(String),
    Empty,
}

impl From<f64> for UserNumber {
    fn from(v: f64) -> Self {
        UserNumber::Number(v)
    }
}
impl From<i32> for UserNumber {
    fn from(v: i32) -> Self {
        UserNumber::Number(v as f64)
    }
}
impl From<&str> for UserNumber {
    fn from(v: &str) -> Self {
        UserNumber::Text(v.to_string())
    }
}
impl From<String> for UserNumber {
    fn from(v: String) -> Self {
        UserNumber::Text(v)
    }
}
impl<T: Into<UserNumber>> From<Option<T>> for UserNumber {
    fn from(v: Option<T>) -> Self {
        v.map_or(UserNumber::Empty, Into::into)
    }
}

/// `finiteInput(value)`: the number a control holds, or `None` when it holds
/// none.
///
/// `Number('')` and `Number('  ')` are 0, which would turn a cleared bound
/// field into a bound of 0. Empty, blank and non-finite input is "no
/// value" instead.
pub fn finite_input(value: &UserNumber) -> Option<f64> {
    let n = match value {
        UserNumber::Empty => return None,
        UserNumber::Text(t) if t.trim().is_empty() => return None,
        UserNumber::Text(t) => js::string_to_number(t),
        UserNumber::Number(n) => *n,
    };
    n.is_finite().then_some(n)
}

/// `Number(value)` of a control, NaN when it is not a number.
pub fn user_number(value: &UserNumber) -> f64 {
    match value {
        UserNumber::Empty => 0.0,
        UserNumber::Text(t) => js::string_to_number(t),
        UserNumber::Number(n) => *n,
    }
}

// Status

/// `status.state`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum StatusState {
    #[default]
    Idle,
    Busy,
    Ok,
    Warn,
    Error,
}

impl StatusState {
    pub fn as_str(self) -> &'static str {
        match self {
            StatusState::Idle => "idle",
            StatusState::Busy => "busy",
            StatusState::Ok => "ok",
            StatusState::Warn => "warn",
            StatusState::Error => "error",
        }
    }
}

/// `store.status`: one message at a time, plus the last LP and layout times.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Status {
    pub state: StatusState,
    pub message: String,
    pub solve_ms: f64,
    pub layout_ms: f64,
}

/// `kind` of a log entry: back-to-back solves fold into one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogKind {
    #[default]
    Message,
    Solve,
}

/// One line of the Logs tab.
#[derive(Debug, Clone, PartialEq)]
pub struct LogEntry {
    /// `Date.now()` of the last occurrence.
    pub time: f64,
    pub level: StatusState,
    pub message: String,
    pub kind: LogKind,
    pub count: u32,
}

/// `Date.now()`.
pub fn now_ms() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_millis() as f64)
}

// Species

/// One member of the community: `{ id, file, abundance, enabled }`.
///
/// `file` is the key of its model in the store's model catalog. Fields a
/// saved scenario carries besides these are kept in `extra`, so a scenario
/// file round-trips.
#[derive(Debug, Clone, PartialEq)]
pub struct Species {
    pub id: String,
    pub file: String,
    pub abundance: f64,
    pub enabled: bool,
    pub extra: Map<String, Value>,
}

impl Species {
    pub fn new(id: &str, file: &str, abundance: f64, enabled: bool) -> Species {
        Species { id: id.into(), file: file.into(), abundance, enabled, extra: Map::new() }
    }

    /// The object `JSON.stringify` writes for it.
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        out.insert("id".into(), json!(self.id));
        out.insert("file".into(), json!(self.file));
        out.insert("abundance".into(), crate::util::num_json(self.abundance));
        out.insert("enabled".into(), json!(self.enabled));
        for (k, v) in &self.extra {
            out.entry(k.clone()).or_insert_with(|| v.clone());
        }
        Value::Object(out)
    }

    /// A species as a saved scenario or bundle gives it. `file` falls back on
    /// `model` (the scenario catalogue's spelling); a missing `enabled` is on.
    pub fn from_json(raw: &Value) -> Species {
        let empty = Map::new();
        let o = raw.as_object().unwrap_or(&empty);
        let text = |k: &str| o.get(k).filter(|v| !v.is_null()).map(js::to_string);
        let mut extra = o.clone();
        for k in ["id", "file", "abundance", "enabled"] {
            extra.shift_remove(k);
        }
        Species {
            id: text("id").unwrap_or_default(),
            file: text("file").or_else(|| text("model")).unwrap_or_default(),
            abundance: o.get("abundance").filter(|v| !v.is_null()).map_or(f64::NAN, js::to_number),
            enabled: o.get("enabled") != Some(&Value::Bool(false)),
            extra,
        }
    }
}

/// `{ lower, upper }` of a reaction override.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub lower: f64,
    pub upper: f64,
}

impl Bounds {
    pub fn to_json(&self) -> Value {
        json!({ "lower": crate::util::num_json(self.lower), "upper": crate::util::num_json(self.upper) })
    }
}

// View

/// The size and font new texts are set in.
#[derive(Debug, Clone, PartialEq)]
pub struct TextStyle {
    pub font_size: f64,
    pub font_family: String,
}

impl Default for TextStyle {
    fn default() -> Self {
        TextStyle { font_size: TEXT_SIZE.default, font_family: DEFAULT_TEXT_FONT.to_string() }
    }
}

/// `store.view`: presentation only. Nothing here changes a flux.
#[derive(Debug, Clone, PartialEq)]
pub struct View {
    /// `auto`, `all`, `active` or `none`.
    pub labels: String,
    /// How the next new text is set: the last size and font chosen.
    pub text_style: TextStyle,
    /// `auto`, `mouse` or `touchpad`.
    pub scroll_input: String,
    pub flux_color_stops: Option<Vec<FluxColorStop>>,
    pub reaction_color_mode: ReactionColorMode,
    /// How reaction lines are drawn, each its own switch: width by flux,
    /// grey dashes for what carries none, a hop where one crosses another.
    pub width_by_flux: bool,
    pub dash_no_flux: bool,
    pub line_hops: bool,
    /// What lines are coloured by when not by flux: `pathway` or `subsystem`.
    pub line_color_group: String,
    pub pathway_ribbon_mode: PathwayRibbonMode,
    pub pathway_ribbon_width: f64,
    pub pathway_ribbon_opacity: f64,
    pub show_currency: bool,
    pub currency_labels: bool,
    pub show_cross_feeding: bool,
    /// Exchanged metabolites drawn as cross-feeding lines: the
    /// `cross_feeding_top` most important (0 = all), unless a list was picked
    /// by hand (`cross_feeding_shown`).
    pub cross_feeding_top: f64,
    pub cross_feeding_shown: Option<Vec<String>>,
    pub dark: bool,
    pub geometry_edit: bool,
    pub multi_select: bool,
    pub text_tool: bool,
}

impl Default for View {
    fn default() -> Self {
        View {
            labels: "auto".into(),
            text_style: TextStyle::default(),
            scroll_input: "auto".into(),
            flux_color_stops: None,
            reaction_color_mode: ReactionColorMode::Flux,
            width_by_flux: false,
            dash_no_flux: true,
            line_hops: true,
            line_color_group: "pathway".into(),
            pathway_ribbon_mode: PathwayRibbonMode::Off,
            pathway_ribbon_width: 56.0,
            pathway_ribbon_opacity: 0.18,
            show_currency: true,
            currency_labels: true,
            show_cross_feeding: false,
            cross_feeding_top: 5.0,
            cross_feeding_shown: None,
            dark: false,
            geometry_edit: false,
            multi_select: false,
            text_tool: false,
        }
    }
}

/// A `setView(patch)`: every field left `None` is left as it is.
/// `flux_color_stops: Some(None)` asks for the automatic palette.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ViewPatch {
    pub labels: Option<String>,
    pub scroll_input: Option<String>,
    pub flux_color_stops: Option<Option<Vec<FluxColorStop>>>,
    pub reaction_color_mode: Option<String>,
    pub width_by_flux: Option<bool>,
    pub dash_no_flux: Option<bool>,
    pub line_hops: Option<bool>,
    pub line_color_group: Option<String>,
    pub pathway_ribbon_mode: Option<String>,
    pub pathway_ribbon_width: Option<f64>,
    pub pathway_ribbon_opacity: Option<f64>,
    pub show_currency: Option<bool>,
    pub currency_labels: Option<bool>,
    pub show_cross_feeding: Option<bool>,
    pub cross_feeding_top: Option<f64>,
    pub cross_feeding_shown: Option<Option<Vec<String>>>,
    pub dark: Option<bool>,
    pub geometry_edit: Option<bool>,
    pub multi_select: Option<bool>,
    pub text_tool: Option<bool>,
}

// Selection

/// `store.selectedEntity`: `{ type, id, ...details }`.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectedEntity {
    /// `reaction`, `metabolite`, `text`, `cell` or `compartment`.
    pub kind: String,
    pub id: String,
    /// `cellId` of a compartment.
    pub cell_id: Option<String>,
    /// Any other detail the caller attached.
    pub details: Map<String, Value>,
}

impl SelectedEntity {
    pub fn new(kind: &str, id: &str) -> SelectedEntity {
        SelectedEntity { kind: kind.into(), id: id.into(), cell_id: None, details: Map::new() }
    }
}

// Results

/// A flux variability range.
pub type FvaRange = coxeter_solver::fba::FvaRange;

/// `store.fvaProgress`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FvaProgress {
    pub running: bool,
    pub done: usize,
    pub total: usize,
    pub ms: f64,
}

/// `solution.steadyCom`: what a fixed-µ SteadyCom solve found.
#[derive(Debug, Clone)]
pub struct SteadyComSolution {
    pub mu: f64,
    pub key: String,
    pub abundances: IndexMap<String, f64>,
    pub total_biomass: f64,
    pub spec: Arc<SteadyComSpec>,
    /// The fluxes as SteadyCom names them (`member|reaction` or joint ids).
    pub original_fluxes: IndexMap<String, f64>,
    pub parsimonious: bool,
    pub total_flux: Option<f64>,
}

/// `store.solution`: the last numerical result the map shows.
#[derive(Debug, Clone)]
pub struct Solution {
    pub status: LpStatus,
    pub objective_value: f64,
    pub fluxes: IndexMap<String, f64>,
    pub message: String,
    /// `solution.solver`.
    pub solver: String,
    /// The solver's complete result (duals, reduced costs, loopless details)
    /// when the solution came from the flux problem.
    pub result: Option<FluxResult>,
    pub steady_com: Option<SteadyComSolution>,
    /// GIMME's own fields (`gimme`, `threshold`, `inconsistencyScore`…),
    /// when the solution is GIMME's.
    pub extra: Option<Value>,
    /// `solution.zuniga`: the record of a Zuñiga case scenario.
    pub zuniga: Option<Arc<crate::zuniga::ZunigaRecord>>,
}

impl Solution {
    pub fn from_result(result: FluxResult) -> Solution {
        Solution {
            status: result.status,
            objective_value: result.objective_value,
            fluxes: result.fluxes.clone(),
            message: result.message.clone(),
            solver: "highs".into(),
            result: Some(result),
            steady_com: None,
            extra: None,
            zuniga: None,
        }
    }

    pub fn is_optimal(&self) -> bool {
        self.status == LpStatus::Optimal
    }

    /// The `{status, objectiveValue, fluxes, reducedCosts, scaledReducedCosts,
    /// shadowPrices}` the reproducibility replay compares.
    pub fn replay_json(&self) -> Value {
        let obj = |m: Option<&IndexMap<String, f64>>| {
            Value::Object(m.map_or_else(Map::new, |m| m.iter().map(|(k, v)| (k.clone(), crate::util::num_json(*v))).collect()))
        };
        json!({
            "status": self.status.as_str(),
            "objectiveValue": crate::util::num_json(self.objective_value),
            "fluxes": obj(Some(&self.fluxes)),
            "reducedCosts": obj(self.result.as_ref().map(|r| &r.reduced_costs)),
            "scaledReducedCosts": obj(self.result.as_ref().map(|r| &r.scaled_reduced_costs)),
            "shadowPrices": obj(self.result.as_ref().map(|r| &r.shadow_prices)),
        })
    }
}

/// Which analysis `run_analysis` ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnalysisMethod {
    Fba,
    Pfba,
    Fva,
    Loopless,
}

impl AnalysisMethod {
    pub fn parse(s: &str) -> Option<AnalysisMethod> {
        match s {
            "fba" => Some(AnalysisMethod::Fba),
            "pfba" => Some(AnalysisMethod::Pfba),
            "fva" => Some(AnalysisMethod::Fva),
            "loopless" => Some(AnalysisMethod::Loopless),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            AnalysisMethod::Fba => "fba",
            AnalysisMethod::Pfba => "pfba",
            AnalysisMethod::Fva => "fva",
            AnalysisMethod::Loopless => "loopless",
        }
    }

    /// `ANALYSIS_LABELS`: the name the Simulate panel shows.
    pub fn label(self) -> &'static str {
        match self {
            AnalysisMethod::Fba => "FBA",
            AnalysisMethod::Pfba => "pFBA",
            AnalysisMethod::Fva => "FVA",
            AnalysisMethod::Loopless => "Loopless FBA",
        }
    }
}

/// `lastRun.phase`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunPhase {
    Running,
    Done,
    Cancelled,
    Failed,
}

/// `store.lastRun`: the analysis the Simulate panel started last.
#[derive(Debug, Clone, PartialEq)]
pub struct LastRun {
    pub method: AnalysisMethod,
    pub label: &'static str,
    pub phase: RunPhase,
    /// `Date.now()` when it started.
    pub started_at: f64,
    pub t0: Instant,
    pub outcome: Option<String>,
    pub ms: Option<f64>,
    pub took: Option<String>,
    pub finished_at: Option<f64>,
    pub objective: Option<f64>,
    /// Distinguishes runs started one after another.
    pub serial: u64,
}

// SteadyCom

/// `steadyComConfig`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SteadyComConfig {
    /// The Figure 4 fiber condition of the nine-species example.
    pub fiber: Option<coxeter_solver::steadycom::Fiber>,
    /// Solve fixed µ for the least total member flux.
    pub parsimonious: bool,
}

impl SteadyComConfig {
    pub fn to_json(&self) -> Value {
        json!({
            "fiber": self.fiber.map(|f| json!({"bacteroides": f.bacteroides, "faecalibacterium": f.faecalibacterium})),
            "parsimonious": self.parsimonious,
        })
    }
}

/// `steadyComState`: what SteadyCom has been asked so far for the open
/// community: the admissible µ range, where it came from, and the last
/// growth rate solved at.
#[derive(Debug, Clone)]
pub struct SteadyComState {
    /// Identity of this state object (the JS keeps a reference to it).
    pub serial: u64,
    pub community_id: Option<String>,
    pub key: String,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub source: String,
    /// `range` or `fixed` while a task runs.
    pub phase: Option<String>,
    pub progress: Option<SteadyComProgress>,
    pub mu: Option<f64>,
    pub feasible: Option<bool>,
    pub abundances: IndexMap<String, f64>,
    pub upper_bound: Option<f64>,
    pub resolution: Option<f64>,
    pub iterations: Option<u32>,
    pub seconds: Option<f64>,
    pub error: Option<String>,
    pub parsimonious: bool,
    pub total_biomass: Option<f64>,
    pub original_fluxes: Option<IndexMap<String, f64>>,
    pub total_flux: Option<f64>,
    pub solve_ms: Option<f64>,
}

/// `steadyComSupport()`.
#[derive(Debug, Clone, PartialEq)]
pub struct SteadyComSupport {
    pub supported: bool,
    pub community_id: Option<String>,
    /// `joint` or `separate`.
    pub kind: Option<&'static str>,
    pub reason: String,
}

// Pathways

/// A pathway drawn by hand: a named, coloured set of reactions.
#[derive(Debug, Clone, PartialEq)]
pub struct ManualPathway {
    pub id: String,
    pub name: String,
    pub color: String,
    pub visible: bool,
    pub reaction_ids: IndexSet<String>,
}

/// Display settings of a model pathway: `{ color?, visible? }`, in the key
/// order they were set (the manager state writes them in that order).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PathwaySetting(pub Map<String, Value>);

impl PathwaySetting {
    pub fn color(&self) -> Option<&str> {
        self.0.get("color").and_then(Value::as_str)
    }
    /// `settings.visible !== false`.
    pub fn visible(&self) -> bool {
        self.0.get("visible") != Some(&Value::Bool(false))
    }
    /// `settings.visible === false`.
    pub fn hidden(&self) -> bool {
        self.0.get("visible") == Some(&Value::Bool(false))
    }
}

/// One row of the pathway manager.
#[derive(Debug, Clone, PartialEq)]
pub struct PathwayEntry {
    pub key: String,
    pub id: String,
    pub name: String,
    /// `model` or `manual`.
    pub source: &'static str,
    pub color: String,
    pub visible: bool,
    pub reaction_ids: Vec<String>,
    pub count: usize,
}

/// A colour legend entry.
#[derive(Debug, Clone, PartialEq)]
pub struct LegendItem {
    pub label: String,
    pub count: usize,
    pub color: String,
}

/// Per-reaction colours and groups, and the legend (`reactionColoring()`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Coloring {
    pub mode: String,
    pub colors: IndexMap<String, String>,
    pub groups: IndexMap<String, String>,
    pub legend: Vec<LegendItem>,
}

// Map tools

/// A cell that can still be drawn (`availableMapCells()`).
#[derive(Debug, Clone, PartialEq)]
pub struct AvailableCell {
    pub species_id: String,
    pub species_name: String,
    pub organism: String,
    pub outer_compartment_id: String,
    pub outer_compartment_name: String,
}

/// A compartment that can be drawn next (`availableMapCompartments()`).
#[derive(Debug, Clone, PartialEq)]
pub struct AvailableCompartment {
    pub species_id: String,
    pub species_name: String,
    pub compartment_id: String,
    pub compartment_name: String,
    pub parent_id: Option<String>,
    pub depth: usize,
}

/// One row of `mapCompartmentChoices()`: a compartment of an active species,
/// drawn or not, and whether it can be drawn now.
#[derive(Debug, Clone, PartialEq)]
pub struct CompartmentChoice {
    pub species_id: String,
    pub species_name: String,
    pub compartment_id: String,
    pub compartment_name: String,
    pub parent_id: Option<String>,
    pub parent_name: Option<String>,
    pub depth: usize,
    /// `ready` (its parent is on the map), `waiting` (its parent is not,
    /// yet), `drawn`, or `cell` (a cell's outermost compartment, which the
    /// Cell tool draws).
    pub status: &'static str,
    /// The status in words.
    pub reason: String,
}

impl CompartmentChoice {
    /// `${speciesId}|${compartmentId}`: the placement key.
    pub fn key(&self) -> String {
        format!("{}|{}", self.species_id, self.compartment_id)
    }
}

impl AvailableCompartment {
    /// `${speciesId}|${compartmentId}`: the placement key.
    pub fn key(&self) -> String {
        format!("{}|{}", self.species_id, self.compartment_id)
    }
}

/// One hit of `searchReactions`.
#[derive(Debug, Clone, PartialEq)]
pub struct ReactionSearchResult {
    pub reaction_id: String,
    pub local_id: String,
    /// What the map labels it with (`entityLabel`).
    pub label: String,
    /// Beside the label, whichever of the name and the model id it is not.
    pub detail: String,
    pub name: String,
    pub subsystem: String,
    pub species_id: String,
    pub drawn: bool,
    pub score: u8,
}

/// A metabolite that can be placed in a compartment.
#[derive(Debug, Clone, PartialEq)]
pub struct AvailableMetabolite {
    pub id: String,
    /// What the map labels it with (`entityLabel`).
    pub label: String,
    /// Beside the label, whichever of the name and the model id it is not.
    pub name: String,
    /// The model's own id, which search matches too.
    pub model_id: String,
    pub count: usize,
}

/// `entityLabel(kind, mergedId)`: a metabolite or reaction of the model named
/// as the map names it, with the model's own id and its name beside.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EntityLabel {
    pub label: String,
    pub id: String,
    pub name: String,
}

/// `store.waitingMap`: a map opened before its model was loaded.
#[derive(Debug, Clone, PartialEq)]
pub struct WaitingMap {
    pub parsed: coxeter_map::map_document::ParsedMapDocument,
    /// The file it came from.
    pub name: Option<String>,
    /// The model it needs, as the user knows it (`mapModelLabel`).
    pub needs: String,
    /// Why it did not fit what is open.
    pub reason: String,
}

/// A label source a kind of object can be shown by.
#[derive(Debug, Clone, PartialEq)]
pub struct LabelOption {
    pub value: String,
    pub label: String,
}

/// `beginMapEdit()`: the drawing as it was before a multi-step edit.
#[derive(Debug, Clone, PartialEq)]
pub struct MapTransaction {
    pub label: String,
    pub before: Value,
}

/// What `removeMultiSelection` took off the drawing.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RemovedSelection {
    pub reactions: Vec<String>,
    pub nodes: Vec<String>,
    pub texts: Vec<String>,
    /// Whole compartments, with what was drawn in them.
    pub compartments: Vec<String>,
}

/// What `arrangeMultiSelection` did.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ArrangeResult {
    /// Every object placed, as `(kind, id)`.
    pub moved: Vec<(String, String)>,
    /// Metabolites a membrane stopped short of the line.
    pub held: Vec<String>,
    pub changed: bool,
}

/// `unsavedWork()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UnsavedWork {
    pub map: bool,
    pub scenario: bool,
    pub model: bool,
}

/// `savedWork`: what was last saved or opened.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedWork {
    pub context: String,
    pub scenario: String,
    pub shared: String,
}

/// How a `rebuild` goes: redo the geometry, what to draw, and whether to solve.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RebuildOptions {
    pub relayout: bool,
    pub contents: coxeter_map::Contents,
    pub solve: bool,
}

impl Default for RebuildOptions {
    fn default() -> Self {
        RebuildOptions { relayout: true, contents: coxeter_map::Contents::All, solve: true }
    }
}

/// The models the store has loaded: file stem → model, each with an identity
/// that survives in-place edits (the JS keys its edit counters by the model
/// object in a `WeakMap`).
#[derive(Debug, Clone, Default)]
pub struct ModelCatalog {
    entries: IndexMap<String, (Arc<Model>, u64)>,
    next_serial: u64,
}

impl ModelCatalog {
    pub fn get(&self, key: &str) -> Option<&Model> {
        self.entries.get(key).map(|(m, _)| m.as_ref())
    }
    pub fn arc(&self, key: &str) -> Option<Arc<Model>> {
        self.entries.get(key).map(|(m, _)| m.clone())
    }
    /// The identity of the model under `key`.
    pub fn serial(&self, key: &str) -> Option<u64> {
        self.entries.get(key).map(|(_, s)| *s)
    }
    pub fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }
    /// `models.set(key, model)`: a new model object, with a new identity.
    pub fn insert(&mut self, key: impl Into<String>, model: Model) -> u64 {
        self.insert_arc(key, Arc::new(model))
    }
    pub fn insert_arc(&mut self, key: impl Into<String>, model: Arc<Model>) -> u64 {
        self.next_serial += 1;
        self.entries.insert(key.into(), (model, self.next_serial));
        self.next_serial
    }
    pub fn remove(&mut self, key: &str) -> Option<Arc<Model>> {
        self.entries.shift_remove(key).map(|(m, _)| m)
    }
    /// Edit a model in place; it keeps its identity.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Model> {
        self.entries.get_mut(key).map(|(m, _)| Arc::make_mut(m))
    }
    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.entries.keys()
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn iter(&self) -> impl Iterator<Item = (&String, &Model)> {
        self.entries.iter().map(|(k, (m, _))| (k, m.as_ref()))
    }
}

/// Edit counters per model identity (`modelEdits`, `modelEditsSaved`).
pub type EditCounts = HashMap<u64, u64>;
