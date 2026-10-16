//! Omics overlays, community omics (metatranscriptomics) and GIMME: the
//! "omics overlay" and "community omics" sections of `store.js`.
//!
//! Two imports live side by side, each with its own settings, as in JS:
//!
//! - the **omics overlay** (`omicsDataset`, `omicsSettings`): a wide gene or
//!   protein table read by `importOmicsCSV`, saved with the scenario, drawn
//!   by `omicsOverlay()`;
//! - **community omics** (`store.omics`): a metatranscriptomics table per
//!   member (`importMetatranscriptomics`), its mapping report, the reaction
//!   scores and colours, and GIMME on top of them.
//!
//! What turns a table into reaction scores and colours is `coxeter-analysis`
//! (`overlay.js`, `community-omics.js`, `gimme.js`).

use std::sync::Arc;

use coxeter_analysis::{
    CompareMetric, CompareOptions, GimmeOptions, GimmeResult, MappingReport, OmicsCsvOptions, OmicsRow, OmicsScale,
    OmicsSpecies, community_omics_mapping_report, compare_reaction_scores, diverging_omics_colors, omics_key, parse_metatranscriptomics,
    parse_omics_csv, reaction_omics_scores, sequential_omics_colors, serialize_omics_dataset,
};
use coxeter_core::js;
use coxeter_core::model::Model;
use indexmap::IndexMap;
use serde_json::{Value, json};

use crate::error::{Result, StoreError};
use crate::jobs::{JobContext, JobKind, JobMessage, JobPayload};
use crate::store::Store;
use crate::types::{Solution, StatusState};
use crate::util::num_json;

pub use coxeter_analysis::OmicsDataset;

/// The conditions (columns) of a dataset.
pub fn dataset_conditions(dataset: Option<&OmicsDataset>) -> Vec<String> {
    dataset.map(|d| d.conditions.clone()).unwrap_or_default()
}

/// `deserializeOmicsDataset(raw)`: a saved dataset, or `None` when it is not
/// one or holds no rows.
pub fn deserialize_omics_dataset(raw: Option<&Value>) -> Option<OmicsDataset> {
    raw.and_then(coxeter_analysis::deserialize_omics_dataset)
}

/// `store.omicsSettings`.
#[derive(Debug, Clone, PartialEq)]
pub struct OmicsSettings {
    pub enabled: bool,
    /// `gene` or `protein`.
    pub data_type: String,
    /// `single` or `compare`.
    pub mode: String,
    pub condition_a: Option<String>,
    pub condition_b: Option<String>,
    /// `log2fc` or `difference`.
    pub metric: String,
    pub pseudocount: f64,
}

impl Default for OmicsSettings {
    fn default() -> Self {
        OmicsSettings {
            enabled: false,
            data_type: "gene".into(),
            mode: "single".into(),
            condition_a: None,
            condition_b: None,
            metric: "log2fc".into(),
            pseudocount: 1.0,
        }
    }
}

impl OmicsSettings {
    pub fn to_json(&self) -> Value {
        json!({
            "enabled": self.enabled,
            "dataType": self.data_type,
            "mode": self.mode,
            "conditionA": self.condition_a,
            "conditionB": self.condition_b,
            "metric": self.metric,
            "pseudocount": num_json(self.pseudocount),
        })
    }
}

/// A `setOmicsSettings(patch)`: fields left `None` are left as they are.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OmicsSettingsPatch {
    pub enabled: Option<bool>,
    pub data_type: Option<String>,
    pub mode: Option<String>,
    pub condition_a: Option<Option<String>>,
    pub condition_b: Option<Option<String>>,
    pub metric: Option<String>,
    pub pseudocount: Option<f64>,
}

/// `store.omics`: community omics (metatranscriptomics per member) and GIMME.
#[derive(Debug, Clone)]
pub struct CommunityOmics {
    pub dataset: Option<Arc<OmicsDataset>>,
    pub enabled: bool,
    /// `single` or `compare`.
    pub mode: String,
    pub condition_a: Option<String>,
    pub condition_b: Option<String>,
    /// `log2fc` or `difference`.
    pub metric: String,
    pub pseudocount: f64,
    pub scores: IndexMap<String, f64>,
    pub colors: IndexMap<String, String>,
    pub scale: Option<OmicsScale>,
    pub report: Option<MappingReport>,
    pub gimme_enabled: bool,
    pub gimme_threshold: Option<f64>,
    pub gimme_objective_fraction: f64,
    pub gimme_result: Option<Arc<GimmeResult>>,
}

impl Default for CommunityOmics {
    fn default() -> Self {
        CommunityOmics {
            dataset: None,
            enabled: false,
            mode: "single".into(),
            condition_a: None,
            condition_b: None,
            metric: "log2fc".into(),
            pseudocount: 1.0,
            scores: IndexMap::new(),
            colors: IndexMap::new(),
            scale: None,
            report: None,
            gimme_enabled: false,
            gimme_threshold: None,
            gimme_objective_fraction: 0.9,
            gimme_result: None,
        }
    }
}

/// `Object.assign(this.omics, options)`: fields left `None` are left alone.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CommunityOmicsPatch {
    pub enabled: Option<bool>,
    pub mode: Option<String>,
    pub condition_a: Option<Option<String>>,
    pub condition_b: Option<Option<String>>,
    pub metric: Option<String>,
    pub pseudocount: Option<f64>,
    pub gimme_enabled: Option<bool>,
    pub gimme_threshold: Option<Option<f64>>,
    pub gimme_objective_fraction: Option<f64>,
}

impl CommunityOmicsPatch {
    /// The patch a saved `{enabled, mode, conditionA, …}` object describes
    /// (the `settings` of an archived bundle).
    pub fn from_json(v: &Value) -> CommunityOmicsPatch {
        let text = |k: &str| v.get(k).map(|x| if x.is_null() { None } else { Some(js::to_string(x)) });
        let number = |k: &str| v.get(k).filter(|x| !x.is_null()).map(js::to_number);
        CommunityOmicsPatch {
            enabled: v.get("enabled").map(js::truthy),
            mode: text("mode").flatten(),
            condition_a: text("conditionA"),
            condition_b: text("conditionB"),
            metric: text("metric").flatten(),
            pseudocount: number("pseudocount"),
            gimme_enabled: v.get("gimmeEnabled").map(js::truthy),
            gimme_threshold: v.get("gimmeThreshold").map(|x| if x.is_null() { None } else { Some(js::to_number(x)) }),
            gimme_objective_fraction: number("gimmeObjectiveFraction"),
        }
    }
}

impl CommunityOmics {
    /// `Object.assign(this.omics, patch)`.
    pub(crate) fn assign(&mut self, patch: CommunityOmicsPatch) {
        if let Some(v) = patch.enabled {
            self.enabled = v;
        }
        if let Some(v) = patch.mode {
            self.mode = v;
        }
        if let Some(v) = patch.condition_a {
            self.condition_a = v;
        }
        if let Some(v) = patch.condition_b {
            self.condition_b = v;
        }
        if let Some(v) = patch.metric {
            self.metric = v;
        }
        if let Some(v) = patch.pseudocount {
            self.pseudocount = v;
        }
        if let Some(v) = patch.gimme_enabled {
            self.gimme_enabled = v;
        }
        if let Some(v) = patch.gimme_threshold {
            self.gimme_threshold = v;
        }
        if let Some(v) = patch.gimme_objective_fraction {
            self.gimme_objective_fraction = v;
        }
    }
}

/// What `omicsOverlay()` hands the renderer and the legend.
#[derive(Debug, Clone, PartialEq)]
pub struct OmicsOverlay {
    pub enabled: bool,
    pub colors: IndexMap<String, String>,
    pub values: IndexMap<String, f64>,
    pub scores_a: IndexMap<String, f64>,
    pub scores_b: IndexMap<String, f64>,
    pub matched_reactions: usize,
    pub total_drawn: usize,
    pub scale: Option<OmicsScale>,
    pub condition_a: Option<String>,
    pub condition_b: Option<String>,
    pub mode: String,
    pub metric: String,
    pub fallback_color: String,
}

/// What a GIMME run needs, gathered on the store's thread.
struct GimmeRun {
    problem: Arc<coxeter_solver::fba::FluxProblem>,
    expression: IndexMap<String, f64>,
    options: GimmeOptions,
    condition: Option<String>,
}

/// A GIMME run in the background, and what it was started against.
pub(crate) struct GimmeJob {
    pub solve_generation: u64,
    pub problem_serial: u64,
    pub bounds_revision: u64,
    pub condition: Option<String>,
}

/// What `importMetatranscriptomics` returns: the parsed table and its
/// mapping report.
#[derive(Debug, Clone)]
pub struct MetatranscriptomicsImport {
    pub dataset: Arc<OmicsDataset>,
    /// `long` or `wide`.
    pub format: &'static str,
    pub report: MappingReport,
}

/// An archived community dataset (`{source, dataType, conditions, values}`
/// of a reproducibility bundle) as a dataset again.
pub fn dataset_from_archive(archived: &Value) -> OmicsDataset {
    let text = |k: &str, fallback: &str| match archived.get(k) {
        None | Some(Value::Null) => fallback.to_string(),
        Some(v) => js::to_string(v),
    };
    let mut values = IndexMap::new();
    if let Some(Value::Object(rows)) = archived.get("values") {
        for (key, row) in rows {
            let id = row.get("id").filter(|v| !v.is_null()).map_or_else(String::new, js::to_string);
            let species = row.get("species").filter(|v| !v.is_null()).map_or_else(String::new, js::to_string);
            let mut row_values: IndexMap<String, f64> = IndexMap::new();
            if let Some(Value::Object(saved)) = row.get("values") {
                for (condition, value) in saved {
                    if let Some(n) = value.as_f64() {
                        row_values.insert(condition.clone(), n);
                    }
                }
            }
            js::js_key_order(&mut row_values);
            let key = if key.is_empty() { omics_key(&species, &id) } else { key.clone() };
            values.insert(key, OmicsRow { id, species, values: row_values });
        }
    }
    OmicsDataset {
        source: text("source", "reproducibility-bundle"),
        data_type: text("dataType", "omics"),
        id_column: "id".into(),
        species_column: None,
        conditions: archived.get("conditions").and_then(Value::as_array).map(|c| c.iter().map(js::to_string).collect()).unwrap_or_default(),
        row_count: values.len(),
        values,
    }
}

impl Store {
    /// The species and models the omics functions read (`this.species`,
    /// `this.models`).
    pub(crate) fn omics_inputs(&self) -> (Vec<OmicsSpecies>, IndexMap<String, Arc<Model>>) {
        let species: Vec<OmicsSpecies> = self.species.iter().map(|sp| OmicsSpecies::new(&sp.id, &sp.file)).collect();
        let models = self.species.iter().filter_map(|sp| self.models.arc(&sp.file).map(|m| (sp.file.clone(), m))).collect();
        (species, models)
    }

    /// `reactionOmicsScores(this.community, this.species, this.models,
    /// dataset, condition)`; empty without a community.
    pub(crate) fn reaction_scores(&self, dataset: &OmicsDataset, condition: Option<&str>) -> IndexMap<String, f64> {
        let Some(community) = self.community_view() else { return IndexMap::new() };
        let (species, models) = self.omics_inputs();
        reaction_omics_scores(&community, &species, &models, Some(dataset), condition.unwrap_or(""))
    }

    // Omics overlay

    /// `importOmicsCSV(text, { filename, dataType })`.
    pub fn import_omics_csv(&mut self, text: &str, filename: &str, data_type: &str) -> Result<OmicsDataset> {
        let dataset = parse_omics_csv(text, &OmicsCsvOptions { source: filename.to_string(), data_type: data_type.to_string() })?;
        self.install_omics_dataset(dataset.clone(), data_type);
        Ok(dataset)
    }

    /// The tail of `importOmicsCSV` once a dataset is parsed.
    pub fn install_omics_dataset(&mut self, dataset: OmicsDataset, data_type: &str) {
        let conditions = dataset.conditions.clone();
        self.omics_dataset = Some(dataset);
        self.omics_dataset_revision += 1;
        self.omics_settings = OmicsSettings {
            enabled: true,
            data_type: data_type.to_string(),
            mode: if conditions.len() >= 2 { "compare" } else { "single" }.into(),
            condition_a: conditions.first().cloned(),
            condition_b: conditions.get(1).or(conditions.first()).cloned(),
            ..self.omics_settings.clone()
        };
        self.emit("omics");
    }

    /// `clearOmics({ emit })`.
    pub fn clear_omics(&mut self, emit: bool) {
        self.omics_dataset = None;
        self.omics_settings = OmicsSettings { data_type: self.omics_settings.data_type.clone(), ..OmicsSettings::default() };
        if emit {
            self.emit("omics");
        }
    }

    /// `setOmicsSettings(patch)`.
    pub fn set_omics_settings(&mut self, patch: OmicsSettingsPatch) -> OmicsSettings {
        let cur = self.omics_settings.clone();
        let mut next = OmicsSettings {
            enabled: patch.enabled.unwrap_or(cur.enabled),
            data_type: patch.data_type.unwrap_or(cur.data_type),
            mode: patch.mode.unwrap_or(cur.mode),
            condition_a: patch.condition_a.unwrap_or(cur.condition_a),
            condition_b: patch.condition_b.unwrap_or(cur.condition_b),
            metric: patch.metric.unwrap_or(cur.metric),
            pseudocount: patch.pseudocount.unwrap_or(cur.pseudocount),
        };
        if !["gene", "protein"].contains(&next.data_type.as_str()) {
            next.data_type = "gene".into();
        }
        if !["single", "compare"].contains(&next.mode.as_str()) {
            next.mode = "single".into();
        }
        if !["log2fc", "difference"].contains(&next.metric.as_str()) {
            next.metric = "log2fc".into();
        }
        // `Math.max(0, Number(p) || 0)`: NaN is 0.
        next.pseudocount = js::max(0.0, if next.pseudocount.is_nan() { 0.0 } else { next.pseudocount });
        let conditions = dataset_conditions(self.omics_dataset.as_ref());
        if !next.condition_a.as_ref().is_some_and(|c| conditions.contains(c)) {
            next.condition_a = conditions.first().cloned();
        }
        if !next.condition_b.as_ref().is_some_and(|c| conditions.contains(c)) {
            next.condition_b = conditions.get(1).or(conditions.first()).cloned();
        }
        next.enabled = next.enabled && self.omics_dataset.is_some();
        self.omics_settings = next.clone();
        self.emit("omics");
        next
    }

    /// `omicsState()`: what the Scenario file keeps.
    pub fn omics_state(&self) -> Value {
        json!({ "dataset": serialize_omics_dataset(self.omics_dataset.as_ref()), "settings": self.omics_settings.to_json() })
    }

    /// `restoreOmicsState(raw)`.
    pub fn restore_omics_state(&mut self, raw: Option<&Value>) {
        self.omics_dataset = deserialize_omics_dataset(raw.and_then(|r| r.get("dataset")));
        self.omics_dataset_revision += 1;
        let conditions = dataset_conditions(self.omics_dataset.as_ref());
        let settings = raw.and_then(|r| r.get("settings"));
        let text = |k: &str| settings.and_then(|s| s.get(k)).and_then(Value::as_str).map(str::to_string);
        let pick = |k: &str, allowed: &[&str], fallback: &str| {
            text(k).filter(|v| allowed.contains(&v.as_str())).unwrap_or_else(|| fallback.to_string())
        };
        let condition = |k: &str, fallback: Option<String>| text(k).filter(|c| conditions.contains(c)).or(fallback);
        let pseudocount = settings.and_then(|s| s.get("pseudocount")).map_or(f64::NAN, js::to_number);
        self.omics_settings = OmicsSettings {
            enabled: settings.and_then(|s| s.get("enabled")).is_some_and(js::truthy) && self.omics_dataset.is_some(),
            data_type: pick("dataType", &["gene", "protein"], "gene"),
            mode: pick("mode", &["single", "compare"], "single"),
            condition_a: condition("conditionA", conditions.first().cloned()),
            condition_b: condition("conditionB", conditions.get(1).or(conditions.first()).cloned()),
            metric: pick("metric", &["log2fc", "difference"], "log2fc"),
            pseudocount: js::max(0.0, if pseudocount.is_nan() { 0.0 } else { pseudocount }),
        };
    }

    /// `omicsOverlay()`: the colours an omics dataset gives the drawn
    /// reactions, with what a legend needs.
    pub fn omics_overlay(&self) -> OmicsOverlay {
        let settings = &self.omics_settings;
        let fallback_color = if self.view.dark { "#475569" } else { "#cbd5e1" };
        let drawn: std::collections::HashSet<String> =
            self.map.as_ref().map(|m| m.reaction_ids().into_iter().collect()).unwrap_or_default();
        let empty = OmicsOverlay {
            enabled: false,
            colors: IndexMap::new(),
            values: IndexMap::new(),
            scores_a: IndexMap::new(),
            scores_b: IndexMap::new(),
            matched_reactions: 0,
            total_drawn: drawn.len(),
            scale: None,
            condition_a: settings.condition_a.clone(),
            condition_b: settings.condition_b.clone(),
            mode: settings.mode.clone(),
            metric: settings.metric.clone(),
            fallback_color: fallback_color.into(),
        };
        let (Some(dataset), true, Some(condition_a)) =
            (self.omics_dataset.as_ref(), settings.enabled, settings.condition_a.as_deref().filter(|c| !c.is_empty()))
        else {
            return empty;
        };
        if self.community.is_none() {
            return empty;
        }
        let scores_a = self.reaction_scores(dataset, Some(condition_a));
        let mut scores_b = IndexMap::new();
        let (values, scale) = match settings.condition_b.as_deref().filter(|b| settings.mode == "compare" && !b.is_empty()) {
            Some(condition_b) => {
                scores_b = self.reaction_scores(dataset, Some(condition_b));
                let values = compare_reaction_scores(
                    &scores_a,
                    &scores_b,
                    &CompareOptions { metric: CompareMetric::parse(&settings.metric), pseudocount: settings.pseudocount },
                );
                let scale = diverging_omics_colors(&values, self.view.dark);
                (values, scale)
            }
            None => {
                let scale = sequential_omics_colors(&scores_a, self.view.dark);
                (scores_a.clone(), scale)
            }
        };
        let matched_reactions = values.keys().filter(|rid| drawn.contains(*rid)).count();
        OmicsOverlay {
            enabled: true,
            colors: scale.colors.clone(),
            values,
            scores_a,
            scores_b,
            matched_reactions,
            total_drawn: drawn.len(),
            scale: Some(scale),
            ..empty
        }
    }

    // Community omics and metatranscriptomics

    /// `importMetatranscriptomics(text, filename)`.
    pub fn import_metatranscriptomics(&mut self, text: &str, filename: &str) -> Result<MetatranscriptomicsImport> {
        let parsed = parse_metatranscriptomics(text, Some(filename))?;
        let Some(community) = self.community_view() else {
            return Err(StoreError::msg("Open a community before importing metatranscriptomics."));
        };
        let (species, models) = self.omics_inputs();
        let report = community_omics_mapping_report(&community, &species, &models, &parsed.dataset);
        let dataset = Arc::new(parsed.dataset);
        self.omics.dataset = Some(dataset.clone());
        self.omics.enabled = true;
        self.omics.condition_a = dataset.conditions.first().cloned();
        self.omics.condition_b = dataset.conditions.get(1).or(dataset.conditions.first()).cloned();
        self.omics.report = Some(report.clone());
        self.recompute_community_omics();
        let coverage = js::round(report.reaction_coverage * 100.0);
        self.set_status_message(
            StatusState::Ok,
            format!(
                "Metatranscriptomics: {} genes/MAG, {}% of GPR reactions mapped.",
                dataset.row_count,
                js::number_to_string(coverage)
            ),
        );
        self.emit("omics");
        let report = self.omics.report.clone().unwrap_or(report);
        Ok(MetatranscriptomicsImport { dataset, format: parsed.format.as_str(), report })
    }

    /// `runCommunityGIMME(options)`: GIMME on the community LP with the
    /// expression of condition A, solved here and now (as in JS).
    /// [`Store::start_community_gimme`] runs it in the background.
    pub fn run_community_gimme(&mut self, options: CommunityOmicsPatch) -> Result<Arc<GimmeResult>> {
        let run = self.prepare_community_gimme(options)?;
        let result = coxeter_analysis::run_gimme(&run.problem, &run.expression, &run.options)?;
        Ok(self.install_community_gimme(result, run.condition))
    }

    /// [`Store::run_community_gimme`] as a background job (GIMME on a
    /// genome-scale community takes a while). The guards and the expression
    /// scores are done at once; [`Store::poll_jobs`] installs the result,
    /// unless the problem changed or another solve started meanwhile.
    pub fn start_community_gimme(&mut self, options: CommunityOmicsPatch) -> Result<()> {
        let run = self.prepare_community_gimme(options)?;
        self.set_status_message(StatusState::Busy, "Running GIMME…");
        self.emit("status");
        let job = GimmeJob {
            solve_generation: self.solve_generation,
            problem_serial: self.problem_serial,
            bounds_revision: self.bounds_revision,
            condition: run.condition.clone(),
        };
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.jobs.spawn(JobKind::Gimme, "gimme", 0, JobContext::Gimme(job), cancel, move |id, tx, _cancel| {
            let result = coxeter_analysis::run_gimme(&run.problem, &run.expression, &run.options).map_err(StoreError::from);
            let _ = tx.send(JobMessage::Done { id, payload: JobPayload::Gimme(Box::new(result)) });
        });
        Ok(())
    }

    /// The end of a background GIMME run.
    pub(crate) fn apply_gimme_job(&mut self, job: GimmeJob, result: Result<GimmeResult>) -> Result<()> {
        let current = job.solve_generation == self.solve_generation
            && job.problem_serial == self.problem_serial
            && job.bounds_revision == self.bounds_revision;
        if !current {
            return Err(StoreError::aborted("GIMME was superseded by a newer calculation."));
        }
        match result {
            Ok(result) => {
                self.install_community_gimme(result, job.condition);
                Ok(())
            }
            Err(error) => {
                self.set_status_message(StatusState::Error, error.to_string());
                self.emit("status");
                Err(error)
            }
        }
    }

    /// The first half of `runCommunityGIMME`: the guards, the settings and
    /// the expression of condition A.
    fn prepare_community_gimme(&mut self, options: CommunityOmicsPatch) -> Result<GimmeRun> {
        self.assert_published_study_constraints()?;
        self.omics.assign(options);
        let Some(dataset) = self.omics.dataset.clone() else {
            return Err(StoreError::msg("Load metatranscriptomics data first."));
        };
        self.cancel_steady_com(true);
        if self.solution.as_ref().is_some_and(|s| s.steady_com.is_some()) {
            self.refresh_composition_problem()?;
        }
        let condition = self.omics.condition_a.clone();
        let expression = self.reaction_scores(&dataset, condition.as_deref());
        let Some(problem) = self.problem.clone() else {
            return Err(StoreError::msg("There is no model to run GIMME on."));
        };
        let options = GimmeOptions { threshold: self.omics.gimme_threshold, objective_fraction: self.omics.gimme_objective_fraction };
        Ok(GimmeRun { problem, expression, options, condition })
    }

    /// The second half of `runCommunityGIMME`: install the result as the
    /// solution and say what it found.
    fn install_community_gimme(&mut self, result: GimmeResult, condition: Option<String>) -> Arc<GimmeResult> {
        let result = Arc::new(result);
        let optimal = result.solution.status.is_optimal();
        self.omics.gimme_result = Some(result.clone());
        self.omics.gimme_enabled = optimal;
        let mut solution = Solution::from_result(result.solution.clone());
        solution.extra = Some(json!({
            "gimme": true,
            "threshold": result.threshold.map_or(Value::Null, num_json),
            "objectiveFraction": result.objective_fraction.map_or(Value::Null, num_json),
            "inconsistencyScore": result.inconsistency_score.map_or(Value::Null, num_json),
            "reason": result.reason,
        }));
        self.apply_solution(solution, false);
        let condition = condition.unwrap_or_else(|| "null".to_string());
        if optimal {
            let percent = js::to_fixed(100.0 * result.objective_fraction.unwrap_or(f64::NAN), 0);
            let threshold = to_precision(result.threshold.unwrap_or(f64::NAN), 4);
            let inconsistency = to_precision(result.inconsistency_score.unwrap_or(f64::NAN), 4);
            self.set_status_message(
                StatusState::Ok,
                format!(
                    "GIMME {condition}: objective {percent}%, threshold {threshold}, inconsistency {inconsistency}. Duals: contextual GIMME LP."
                ),
            );
        } else {
            // `result.message || result.status`
            let why = if result.solution.message.is_empty() {
                result.solution.status.as_str().to_string()
            } else {
                result.solution.message.clone()
            };
            self.set_status_message(StatusState::Error, format!("GIMME found no solution: {why}"));
        }
        self.emit("solution");
        result
    }

    /// `disableGIMME()`.
    pub fn disable_gimme(&mut self) -> Option<Arc<Solution>> {
        self.omics.gimme_enabled = false;
        self.omics.gimme_result = None;
        self.solve()
    }

    /// `setCommunityOmics(options)`: change the community-omics settings and
    /// recompute.
    pub fn set_community_omics(&mut self, options: CommunityOmicsPatch) {
        self.omics.assign(options);
        self.recompute_community_omics();
        self.emit("omics");
    }

    /// `recomputeCommunityOmics()`.
    pub fn recompute_community_omics(&mut self) {
        let Some(dataset) = self.omics.dataset.clone().filter(|_| self.omics.enabled) else {
            self.omics.scores = IndexMap::new();
            self.omics.colors = IndexMap::new();
            self.omics.scale = None;
            return;
        };
        let a = self.reaction_scores(&dataset, self.omics.condition_a.as_deref());
        let compare = self.omics.mode == "compare" && self.omics.condition_b.as_deref().is_some_and(|b| !b.is_empty());
        let (scores, scale) = if compare {
            let b = self.reaction_scores(&dataset, self.omics.condition_b.as_deref());
            let scores = compare_reaction_scores(
                &a,
                &b,
                &CompareOptions { metric: CompareMetric::parse(&self.omics.metric), pseudocount: self.omics.pseudocount },
            );
            let scale = diverging_omics_colors(&scores, self.view.dark);
            (scores, scale)
        } else {
            let scale = sequential_omics_colors(&a, self.view.dark);
            (a, scale)
        };
        self.omics.colors = scale.colors.clone();
        self.omics.scores = scores;
        self.omics.scale = Some(scale);
        self.omics.report = self.community_view().map(|community| {
            let (species, models) = self.omics_inputs();
            community_omics_mapping_report(&community, &species, &models, &dataset)
        });
    }
}

/// `Number(x).toPrecision(p)`.
pub fn to_precision(x: f64, p: usize) -> String {
    if !x.is_finite() {
        return js::number_to_string(x);
    }
    if x == 0.0 {
        return js::to_fixed(0.0, p.saturating_sub(1));
    }
    let e = x.abs().log10().floor() as i32;
    // Exponential notation when the exponent is below -6 or at least p.
    let formatted = format!("{:.*e}", p - 1, x);
    let (mantissa, exp) = formatted.split_once('e').unwrap_or((&formatted, "0"));
    let exp: i32 = exp.parse().unwrap_or(e);
    if exp < -6 || exp >= p as i32 {
        let sign = if exp < 0 { '-' } else { '+' };
        return format!("{mantissa}e{sign}{}", exp.abs());
    }
    let decimals = (p as i32 - 1 - exp).max(0) as usize;
    js::to_fixed(x, decimals)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_precision_is_the_javascript_one() {
        assert_eq!(to_precision(0.123456, 4), "0.1235");
        assert_eq!(to_precision(123.456, 4), "123.5");
        assert_eq!(to_precision(12345.6, 4), "1.235e+4");
        assert_eq!(to_precision(0.0000001234, 4), "1.234e-7");
        assert_eq!(to_precision(0.0, 4), "0.000");
        assert_eq!(to_precision(1.0, 4), "1.000");
        assert_eq!(to_precision(f64::NAN, 4), "NaN");
    }
}
