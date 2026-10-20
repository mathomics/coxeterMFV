//! Opening things and rebuilding the community: scenarios, uploads, Yeast9,
//! published communities, `rebuild` and what the LP is built from.

use std::sync::Arc;

use coxeter_core::community::{CommunityModel, CommunitySpec, MediumLimit, Objective, SpeciesSpec, community_exchange};
use coxeter_core::js;
use coxeter_core::model::Model;
use coxeter_io::model_import::{ImportFile, ImportSummary, PreparedImport, UploadFile, prepare_imported_models, prepare_model_uploads};
use coxeter_io::sbml_write::{SbmlWriteOptions, to_sbml};
use coxeter_map::Contents;
use coxeter_map::editable_map::EditableMap;
use coxeter_map::layout::LayoutOptions;
use coxeter_map::layout_settings::layout_engine_options;
use coxeter_map::map_document::parse_map_document;
use coxeter_map::multi_selection::empty_selection;
use coxeter_map::yeast9::{
    YEAST9_CANVAS, YEAST9_MODEL_KEY, YEAST9_PREVIEW_COMPARTMENTS, YEAST9_SCENARIO_ID, YEAST9_SOURCE, Yeast9Preview,
    populate_yeast9_preview,
};
use coxeter_solver::fba::FluxProblem;
use coxeter_solver::steadycom_runtime::{
    JointSpecOptions, MemberModel, joint_community_projection, joint_steady_com_spec, parse_media_csv,
};
use indexmap::IndexMap;
use serde_json::{Map, Value, json};

use crate::error::{Result, StoreError};
use crate::store::{AUTO_DRAW_METABOLITE_LIMIT, AUTO_DRAW_REACTION_LIMIT, AUTO_SOLVE_REACTION_LIMIT, Store, objectives};
use crate::types::*;
use crate::util::normalize_pfba_percent;
use crate::zuniga::ZUNIGA_ID;

/// One uploaded model, as the import reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedModel {
    pub filename: String,
    pub model_key: String,
    pub species_id: String,
    pub abundance: f64,
    pub summary: ImportSummary,
}

/// What an import read: the models and the medium inferred from them.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportReport {
    pub entries: Vec<ImportedModel>,
    pub medium: IndexMap<String, f64>,
    /// Yeast9 only: what the curated preview drew.
    pub preview: Option<Yeast9Preview>,
}

/// `usesCuratedCoreMap(communityId)`: which community keeps its full member
/// models and draws a curated map beside them, rather than drawing the
/// members themselves.
///
/// Only the four-E. coli benchmark does: it has a hand-made
/// central-metabolism overview, so the member models can stay genome-scale
/// without anything trying to draw them.
pub fn uses_curated_core_map(community_id: &str) -> bool {
    community_id == "steadycom-ecoli-auxotrophs"
}

fn field(v: &Value, k: &str) -> Option<Value> {
    v.get(k).filter(|x| !x.is_null()).cloned()
}

fn text_field(v: &Value, k: &str) -> Option<String> {
    field(v, k).map(|x| js::to_string(&x))
}

/// `Boolean(a ?? b ?? false)` over JSON fields.
fn flag(v: &Value, keys: &[&str]) -> bool {
    keys.iter().find_map(|k| field(v, k)).is_some_and(|x| js::truthy(&x))
}

/// `{ ...scenario.medium }` as numbers.
fn medium_of(v: Option<&Value>) -> IndexMap<String, f64> {
    match v {
        Some(Value::Object(m)) => m.iter().map(|(k, x)| (k.clone(), js::to_number(x))).collect(),
        _ => IndexMap::new(),
    }
}

impl Store {
    // Scenarios

    /// `useScenario(id)`: load a named scenario and rebuild everything.
    pub fn use_scenario(&mut self, id: &str) -> Result<()> {
        self.reset_zuniga_study();
        self.invalidate_steady_com();
        self.steady_com_config = SteadyComConfig::default();
        self.published_community_id = None;
        let scenario = self
            .scenarios
            .iter()
            .find(|s| s.get("id").and_then(Value::as_str) == Some(id))
            .or(self.scenarios.first())
            .cloned()
            .ok_or_else(|| StoreError::msg(format!("Unknown scenario \"{id}\".")))?;
        if scenario.get("loader").and_then(Value::as_str) == Some("yeast9") {
            self.use_yeast9_scenario(scenario)?;
            return Ok(());
        }
        self.objective = text_field(&scenario, "objective").filter(|o| !o.is_empty()).unwrap_or_else(|| objectives::EQUAL_GROWTH.into());
        self.product_target = text_field(&scenario, "productTarget").filter(|o| !o.is_empty());
        self.custom_objective = None;
        self.parsimonious = flag(&scenario, &["parsimonious", "pfba"]);
        let percent = match field(&scenario, "pfbaBiomassPercent") {
            Some(p) => js::to_number(&p),
            None => match scenario.get("pfbaFraction") {
                Some(f) => js::to_number(f) * 100.0,
                None => 100.0,
            },
        };
        self.pfba_biomass_percent = normalize_pfba_percent(percent);
        self.loopless = flag(&scenario, &["loopless", "looplessFba"]);
        if self.loopless {
            self.parsimonious = false;
        }
        self.fva_enabled = !self.loopless && flag(&scenario, &["fvaEnabled", "fva"]);
        self.fva_ranges = IndexMap::new();
        self.fva_progress = FvaProgress::default();
        self.medium = medium_of(scenario.get("medium"));
        self.overrides.clear();
        self.gene_knockouts.clear();
        self.manual_pathways.clear();
        self.pathway_settings.clear();
        self.clear_omics(false);
        self.selected_reaction = None;
        self.selected_entity = None;
        self.variability = None;
        let list = scenario.get("species").and_then(Value::as_array).cloned().unwrap_or_default();
        let n = list.len() as f64;
        self.species = list
            .iter()
            .map(|s| {
                let abundance = field(s, "abundance").map_or(1.0 / n, |a| js::to_number(&a));
                Species::new(
                    &text_field(s, "id").unwrap_or_default(),
                    &text_field(s, "model").unwrap_or_default(),
                    abundance,
                    s.get("enabled") != Some(&Value::Bool(false)),
                )
            })
            .collect();
        self.scenario = Some(scenario);
        self.model_editor_species_id = self.species.first().map(|sp| sp.id.clone());
        self.model_editor_mode = None;
        self.rebuild(RebuildOptions::default())?;
        self.settle_opened_work();
        self.draw_waiting_map();
        Ok(())
    }

    /// `importModelFiles(files)`: replace the current community with one
    /// built from user-supplied COBRA JSON or SBML files. Multiple files
    /// become multiple species/cells.
    pub fn import_model_files(&mut self, files: &[ImportFile]) -> Result<ImportReport> {
        let prepared = prepare_imported_models(files)?;
        self.install_imported_models(prepared)
    }

    /// `importModelFiles(files)` for files read as bytes: COBRA JSON, SBML,
    /// COBRA Toolbox MATLAB (.mat) or Excel (.xlsx).
    pub fn import_model_uploads(&mut self, files: &[UploadFile]) -> Result<ImportReport> {
        let prepared = prepare_model_uploads(files)?;
        self.install_imported_models(prepared)
    }

    fn install_imported_models(&mut self, prepared: PreparedImport) -> Result<ImportReport> {
        self.reset_zuniga_study();
        for key in std::mem::take(&mut self.uploaded_model_keys) {
            self.models.remove(&key);
        }
        let mut entries = Vec::new();
        let count = prepared.entries.len();
        let first_name = prepared.entries.first().map(|e| e.model.name.clone()).unwrap_or_default();
        let first_file = prepared.entries.first().map(|e| e.filename.clone()).unwrap_or_default();
        for entry in prepared.entries {
            entries.push(ImportedModel {
                filename: entry.filename.clone(),
                model_key: entry.model_key.clone(),
                species_id: entry.species_id.clone(),
                abundance: entry.abundance,
                summary: entry.summary.clone(),
            });
            self.models.insert(entry.model_key.clone(), entry.model);
            self.uploaded_model_keys.insert(entry.model_key);
        }
        let medium_json: Map<String, Value> = prepared.medium.iter().map(|(k, v)| (k.clone(), crate::util::num_json(*v))).collect();
        let species_json: Vec<Value> = entries
            .iter()
            .map(|e| json!({"id": e.species_id, "model": e.model_key, "abundance": e.abundance, "enabled": true}))
            .collect();
        let scenario = json!({
            "id": "uploaded-models",
            "name": if count == 1 { first_name } else { "Imported community".to_string() },
            "description": if count == 1 { format!("Model loaded from {first_file}") } else { format!("{count} models loaded from files") },
            "objective": objectives::EQUAL_GROWTH,
            "parsimonious": false,
            "pfbaBiomassPercent": 100,
            "loopless": false,
            "fvaEnabled": false,
            "medium": medium_json,
            "species": species_json,
            "uploaded": true,
        });
        self.scenarios.retain(|s| !matches!(s.get("id").and_then(Value::as_str), Some("uploaded-json" | "uploaded-models")));
        self.scenarios.push(scenario.clone());
        self.scenario = Some(scenario);
        self.objective = objectives::EQUAL_GROWTH.into();
        self.product_target = None;
        self.custom_objective = None;
        self.parsimonious = false;
        self.pfba_biomass_percent = 100.0;
        self.loopless = false;
        self.fva_enabled = false;
        self.fva_ranges = IndexMap::new();
        self.fva_progress = FvaProgress::default();
        self.medium = prepared.medium.clone();
        self.overrides.clear();
        self.gene_knockouts.clear();
        self.manual_pathways.clear();
        self.pathway_settings.clear();
        self.clear_omics(false);
        self.selected_reaction = None;
        self.selected_geometry = None;
        self.selected_entity = None;
        self.multi_selection = empty_selection();
        self.variability = None;
        self.species = entries.iter().map(|e| Species::new(&e.species_id, &e.model_key, e.abundance, true)).collect();
        self.model_editor_species_id = self.species.first().map(|sp| sp.id.clone());
        self.model_editor_mode = None;
        self.cell_placement_species_id = self.species.iter().find(|sp| sp.enabled).map(|sp| sp.id.clone());
        self.compartment_placement_key = None;

        let total_reactions: usize = entries.iter().map(|e| e.summary.reactions).sum();
        let total_metabolites: usize = entries.iter().map(|e| e.summary.metabolites).sum();
        let auto_solve = total_reactions <= AUTO_SOLVE_REACTION_LIMIT;
        // Crowding is per cell, so it is the largest member that decides, not the sum.
        let drawable = entries
            .iter()
            .all(|e| e.summary.reactions <= AUTO_DRAW_REACTION_LIMIT && e.summary.metabolites <= AUTO_DRAW_METABOLITE_LIMIT);
        self.rebuild(RebuildOptions {
            relayout: true,
            contents: if drawable { Contents::All } else { Contents::None },
            solve: auto_solve,
        })?;

        // Bare cells and compartments with nothing inside them read as a
        // failed import. Draw what can be drawn quickly, and where that is
        // the whole model say so, so nobody goes looking for the rest.
        let drawn = if drawable {
            self.map_mode = "full".into();
            format!(" Drawn in full: {} reactions.", self.map.as_ref().map_or(0, |m| m.reactions().len()))
        } else if self.show_core_metabolism_overview(false).is_some() {
            " Drawing central carbon metabolism only — this model is too large to lay out in full automatically; use Map options ▸ Full map for everything.".to_string()
        } else {
            self.map_mode = "empty".into();
            " The drawing was left empty: this model is too large to lay out automatically. Build it up from Map structure, or use Map options ▸ Full map.".to_string()
        };
        let deferred = if auto_solve { "" } else { " Analysis was deferred for this genome-scale model; use Run Analysis when needed." };
        self.set_status_message(
            StatusState::Ok,
            format!("Imported {count} model(s): {total_reactions} reactions, {total_metabolites} metabolites.{deferred}{drawn}"),
        );
        self.settle_opened_work();
        self.draw_waiting_map();
        self.emit("rebuild");
        Ok(ImportReport { entries, medium: prepared.medium, preview: None })
    }

    /// `importModelJSONFiles(files)`: backwards-compatible alias.
    pub fn import_model_json_files(&mut self, files: &[ImportFile]) -> Result<ImportReport> {
        self.import_model_files(files)
    }

    // Yeast9

    /// The Yeast9 scenario of the catalogue, or the built-in one.
    pub fn yeast9_scenario(&self) -> Value {
        self.scenarios.iter().find(|s| s.get("id").and_then(Value::as_str) == Some(YEAST9_SCENARIO_ID)).cloned().unwrap_or_else(|| {
            json!({
                "id": YEAST9_SCENARIO_ID,
                "name": format!("Yeast9 {} — S. cerevisiae genome-scale", YEAST9_SOURCE.version),
                "description": format!("Official yeast-GEM {}.", YEAST9_SOURCE.version),
                "loader": "yeast9",
                "objective": objectives::SUM_BIOMASS,
                "species": [{ "id": "sce", "model": YEAST9_MODEL_KEY, "abundance": 1 }],
            })
        })
    }

    /// `importYeast9()`: load the registered Yeast9 scenario. The same path
    /// is used by the Scenario list and by the button in Species & abundance.
    pub fn import_yeast9(&mut self) -> Result<ImportReport> {
        let scenario = self.yeast9_scenario();
        self.use_yeast9_scenario(scenario)
    }

    /// `useYeast9Scenario(scenario)`: download the official SBML through the
    /// fetcher, then [`Store::install_yeast9`].
    pub fn use_yeast9_scenario(&mut self, scenario: Value) -> Result<ImportReport> {
        self.reset_zuniga_study();
        self.set_status_message(StatusState::Busy, format!("Downloading official Yeast9 {}…", YEAST9_SOURCE.version));
        self.emit("status");
        let response = self.fetcher.fetch(YEAST9_SOURCE.url)?;
        if !response.ok() {
            return Err(StoreError::msg(format!(
                "Yeast9 {} download failed (HTTP {}). You can still download yeast-GEM.xml manually and use Load model(s).",
                YEAST9_SOURCE.version, response.status
            )));
        }
        let text = String::from_utf8_lossy(&response.body).into_owned();
        self.install_yeast9(scenario, &text)
    }

    /// [`Store::use_yeast9_scenario`] with the download on a background
    /// thread, so the interface keeps painting while yeast-GEM arrives;
    /// [`Store::poll_jobs`] installs it. As in JS, nothing guards the install
    /// against a scenario opened meanwhile: the download was asked for.
    pub fn start_yeast9_scenario(&mut self, scenario: Value) {
        self.reset_zuniga_study();
        self.set_status_message(StatusState::Busy, format!("Downloading official Yeast9 {}…", YEAST9_SOURCE.version));
        self.emit("status");
        let fetcher = self.fetcher.clone();
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let ctx = crate::jobs::JobContext::Download(Box::new(scenario));
        self.jobs.spawn(crate::jobs::JobKind::Download, "yeast9", 0, ctx, cancel, move |id, tx, _cancel| {
            let result = fetcher.fetch(YEAST9_SOURCE.url);
            let _ = tx.send(crate::jobs::JobMessage::Done { id, payload: crate::jobs::JobPayload::Download(Box::new(result)) });
        });
    }

    /// The end of the Yeast9 download job.
    pub(crate) fn apply_yeast9_download(&mut self, scenario: Value, response: Result<crate::assets::FetchResponse>) -> Result<()> {
        let attempt = response.and_then(|response| {
            if !response.ok() {
                return Err(StoreError::msg(format!(
                    "Yeast9 {} download failed (HTTP {}). You can still download yeast-GEM.xml manually and use Load model(s).",
                    YEAST9_SOURCE.version, response.status
                )));
            }
            let text = String::from_utf8_lossy(&response.body).into_owned();
            self.install_yeast9(scenario, &text).map(|_| ())
        });
        if let Err(error) = &attempt {
            // The JS rejects the promise and the caller shows it.
            self.set_status_message(StatusState::Error, error.to_string());
            self.emit("status");
        }
        attempt
    }

    /// The rest of `useYeast9Scenario` once the SBML text is in hand (a GUI
    /// that downloads it itself calls this directly).
    pub fn install_yeast9(&mut self, scenario: Value, text: &str) -> Result<ImportReport> {
        self.reset_zuniga_study();
        let prepared = prepare_imported_models(&[ImportFile { name: YEAST9_SOURCE.filename.into(), text: text.into() }])?;
        let Some(entry) = prepared.entries.into_iter().next() else {
            return Err(StoreError::msg("Yeast9 import produced no model."));
        };
        for key in std::mem::take(&mut self.uploaded_model_keys) {
            self.models.remove(&key);
        }
        let summary = entry.summary.clone();
        let report_entry = ImportedModel {
            filename: entry.filename.clone(),
            model_key: YEAST9_MODEL_KEY.into(),
            species_id: entry.species_id.clone(),
            abundance: entry.abundance,
            summary: summary.clone(),
        };
        self.models.insert(YEAST9_MODEL_KEY, entry.model);
        self.uploaded_model_keys.insert(YEAST9_MODEL_KEY.into());

        self.objective = text_field(&scenario, "objective").filter(|o| !o.is_empty()).unwrap_or_else(|| objectives::SUM_BIOMASS.into());
        self.product_target = text_field(&scenario, "productTarget").filter(|o| !o.is_empty());
        self.custom_objective = None;
        self.parsimonious = flag(&scenario, &["parsimonious"]);
        self.pfba_biomass_percent =
            normalize_pfba_percent(field(&scenario, "pfbaBiomassPercent").map_or(100.0, |p| js::to_number(&p)));
        self.loopless = false;
        self.fva_enabled = false;
        self.fva_ranges = IndexMap::new();
        self.fva_progress = FvaProgress::default();
        self.medium = prepared.medium.clone();
        self.overrides.clear();
        self.gene_knockouts.clear();
        self.manual_pathways.clear();
        self.pathway_settings.clear();
        self.clear_omics(false);
        self.selected_reaction = None;
        self.selected_geometry = None;
        self.selected_entity = None;
        self.multi_selection = empty_selection();
        self.variability = None;

        let species_id = scenario
            .get("species")
            .and_then(Value::as_array)
            .and_then(|s| s.first())
            .and_then(|s| text_field(s, "id"))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "sce".into());
        self.scenario = Some(scenario);
        self.species = vec![Species::new(&species_id, YEAST9_MODEL_KEY, 1.0, true)];
        self.model_editor_species_id = Some(species_id.clone());
        self.model_editor_mode = None;
        self.cell_placement_species_id = Some(species_id.clone());
        self.compartment_placement_key = None;

        // Build the biological model and its 14-compartment shell, but do not
        // solve the 4,000-reaction LP automatically. The preview below is
        // map-only.
        self.rebuild(RebuildOptions { relayout: true, contents: Contents::None, solve: false })?;
        // The generic automatic canvas is sized for all 4,105 reactions. The
        // curated scenario draws only a representative subset, so use a
        // compact publication-like canvas where the 14 compartments remain
        // legible.
        let community = self.community.clone().ok_or_else(|| StoreError::msg("Yeast9 import produced no community."))?;
        let opts = LayoutOptions {
            width: Some(YEAST9_CANVAS.0),
            height: Some(YEAST9_CANVAS.1),
            ..layout_engine_options(&(&self.layout_settings).into())
        };
        let mut map = EditableMap::empty_from_community(&community, &opts);
        self.map_history.clear();
        let model = self.models.arc(YEAST9_MODEL_KEY).expect("just inserted");
        let preview = populate_yeast9_preview(&mut map, &community, &model, &species_id);
        self.map = Some(map);
        self.map_history.clear();

        self.set_status_message(
            StatusState::Ok,
            format!(
                "Loaded Yeast9 {}: {} reactions, {} metabolites, {} compartments · {} representative reactions drawn across {}/{} compartments. Run Analysis when you want FBA.",
                YEAST9_SOURCE.version,
                summary.reactions,
                summary.metabolites,
                summary.compartments,
                preview.placed,
                preview.covered_compartments,
                YEAST9_PREVIEW_COMPARTMENTS.len()
            ),
        );
        self.settle_opened_work();
        self.draw_waiting_map();
        self.emit("rebuild");
        Ok(ImportReport { entries: vec![report_entry], medium: prepared.medium, preview: Some(preview) })
    }

    // Published communities

    /// `publishedManifest(id)`: the generated assets of a published
    /// community, read once and remembered, `None` meaning "this community
    /// has none", the normal case for the ones whose sources could not be
    /// retrieved.
    pub fn published_manifest(&mut self, id: &str) -> Option<Value> {
        if let Some(m) = self.published_manifests.get(id) {
            return m.clone();
        }
        let manifest = self.assets.read_json(&format!("models/published/{id}/manifest.json")).ok();
        self.published_manifests.insert(id.to_string(), manifest.clone());
        manifest
    }

    /// `publishedMemberFileForRuntime(communityId, member)`: which file of a
    /// member is the runtime's biological model.
    pub fn published_member_file_for_runtime(&self, community_id: &str, member: &Value) -> Option<String> {
        let full = text_field(member, "model");
        let preview = text_field(member, "preview");
        // Without a curated map, whatever is loaded here *is* what gets
        // drawn, and selecting the nine-species community that way would take
        // 557 seconds: nine genome-scale members, ten thousand reactions, laid
        // out as a drawing nobody can read. The excerpt exists for exactly that, and
        // the card already says the growth rate on such a map is the
        // excerpt's. The full model remains what the Download and the SBML
        // conversion read.
        if !uses_curated_core_map(community_id) && community_id != "steadycom-gut-nine" && preview.is_some() {
            return preview;
        }
        // Hosted/dev builds can read the full member files from
        // models/published/. The standalone intentionally embeds only the
        // much smaller map previews.
        if self.assets.is_embedded() {
            if let Some(f) = full.as_ref().filter(|f| self.assets.contains(&format!("models/published/{community_id}/{f}"))) {
                return Some(f.clone());
            }
            if let Some(p) = preview.as_ref().filter(|p| self.assets.contains(&format!("models/published/{community_id}/{p}"))) {
                return Some(p.clone());
            }
        }
        full.or(preview)
    }

    /// `usePublishedCommunity(id)`: load a published community as the active
    /// one.
    ///
    /// The biological model and the initial map are deliberately separate.
    /// Published communities that bundle full member models use those models
    /// for editing, reaction discovery and simulation. A curated preview may
    /// still be used to decide what is drawn initially; it must never
    /// truncate Edit Model.
    pub fn use_published_community(&mut self, id: &str) -> Result<()> {
        if id == ZUNIGA_ID {
            // The sources download in the background; `poll_jobs` opens the
            // case when they are in.
            return self.use_zuniga_study(None, None);
        }
        self.reset_zuniga_study();
        self.invalidate_steady_com();
        self.steady_com_config = SteadyComConfig::default();
        let Some(community) = self.published_communities.iter().find(|c| c.get("id").and_then(Value::as_str) == Some(id)).cloned()
        else {
            return Ok(());
        };
        self.published_community_id = Some(id.to_string());
        self.steady_com_state = None;
        self.steady_com_spec = None;
        let name = text_field(&community, "name").unwrap_or_default();
        // Selecting a community is first of all a request to read about it,
        // so the selection lands before anything is read. One whose sources
        // could not be retrieved has no models to draw, and that must not stop
        // its paper, its provenance and what it would still need from being
        // shown.
        self.scenario = Some(json!({
            "id": format!("published:{id}"),
            "name": community.get("name").cloned().unwrap_or(Value::Null),
            "description": community.get("description").cloned().unwrap_or(Value::Null),
            "published": id,
        }));

        let manifest = self.published_manifest(id);
        let members = manifest.as_ref().and_then(|m| m.get("members")).and_then(Value::as_array).cloned().unwrap_or_default();
        if members.is_empty() {
            // Do not leave the previous community/map on screen under the
            // newly selected scenario name.
            self.community = None;
            self.set_problem(None);
            self.map = None;
            self.solution = None;
            self.scale = None;
            self.cross_feeding = Default::default();
            self.growth = IndexMap::new();
            self.map_history.clear();
            self.selected_reaction = None;
            self.selected_geometry = None;
            self.selected_entity = None;
            self.multi_selection = empty_selection();
            self.variability = None;
            self.set_status_message(StatusState::Warn, format!("{name}: no model files are bundled, so there is no map to draw."));
            self.settle_opened_work();
            self.emit("rebuild");
            return Ok(());
        }
        let manifest = manifest.unwrap_or(Value::Null);
        self.objective = objectives::EQUAL_GROWTH.into();
        self.product_target = None;
        self.custom_objective = None;
        self.parsimonious = false;
        self.pfba_biomass_percent = 100.0;
        self.loopless = false;
        self.fva_enabled = false;
        self.fva_ranges = IndexMap::new();
        self.fva_progress = FvaProgress::default();
        self.medium = medium_of(manifest.get("medium"));
        self.overrides.clear();
        self.gene_knockouts.clear();
        self.manual_pathways.clear();
        self.pathway_settings.clear();
        self.clear_omics(false);
        self.selected_reaction = None;
        self.selected_entity = None;
        self.variability = None;
        let n = members.len() as f64;
        self.species = members
            .iter()
            .map(|m| {
                let file = self.published_member_file_for_runtime(id, m).unwrap_or_default();
                let file = file.strip_suffix(".json").unwrap_or(&file).to_string();
                Species::new(&text_field(m, "id").unwrap_or_default(), &format!("published/{id}/{file}"), 1.0 / n, true)
            })
            .collect();
        self.model_editor_species_id = self.species.first().map(|sp| sp.id.clone());
        self.model_editor_mode = None;
        if id == "steadycom-ecoli-auxotrophs" {
            let members = self.member_models()?;
            let text = self.assets.read_text(&format!("models/published/{id}/media.csv"))?;
            self.medium = parse_media_csv(&text, &members)?;
        } else if id == "steadycom-gut-nine" {
            let members = self.member_models()?;
            let bounds = self.assets.read_text(&format!("models/published/{id}/bounds.csv"))?;
            let joint = joint_steady_com_spec(&members, &bounds, JointSpecOptions::default())?;
            let mut medium = IndexMap::new();
            if let (Some(j), Some(meta)) = (joint.joint.as_ref(), joint.joint.as_ref().and_then(|j| j.metadata.as_ref())) {
                for (mid, rid) in &meta.community_uptake {
                    let base = meta.bases.get(mid).cloned().unwrap_or_default();
                    let value = j.reactions.get(rid).map_or(f64::NAN, |r| r.upper_bound);
                    if value > 0.0 {
                        medium.insert(base, value);
                    }
                }
            }
            js::js_key_order(&mut medium);
            self.medium = medium;
        }
        // The four-E. coli benchmark has genome-scale member models plus a
        // curated central-metabolism map. Start from an empty drawing shell
        // and populate only that readable overview; the active biological
        // model remains full iAF1260.
        let curated = uses_curated_core_map(id);
        let joint_map = id == "steadycom-gut-nine";
        self.rebuild(RebuildOptions {
            relayout: true,
            contents: if curated || joint_map { Contents::None } else { Contents::All },
            solve: !joint_map,
        })?;
        if curated {
            self.show_core_metabolism_overview(false);
        }
        self.settle_opened_work();
        self.draw_waiting_map();
        Ok(())
    }

    /// The members as `{ id, model }`, loaded.
    fn member_models(&mut self) -> Result<Vec<MemberModel>> {
        let mut members = Vec::new();
        for sp in self.species.clone() {
            let model = self.load_model(&sp.file)?;
            members.push(MemberModel { id: sp.id.clone(), model });
        }
        Ok(members)
    }

    /// `drawPublishedPreview(id)`: draw a small excerpt while keeping full
    /// models available for analysis.
    pub fn draw_published_preview(&mut self, id: &str) -> Result<()> {
        let manifest = self.published_manifest(id);
        // A curated map, when the community ships one, replaces the excerpt.
        // It is read the way a loaded map file is, so it is checked against
        // the full models.
        if let Some(map_file) = manifest.as_ref().and_then(|m| text_field(m, "map")) {
            let drawing = self.assets.read_json(&format!("models/published/{id}/{map_file}"))?;
            self.map_load_note = String::new();
            let parsed = parse_map_document(&drawing)?;
            self.apply_map_document(parsed)?;
            self.map_history.clear();
            self.set_status_message(
                StatusState::Ok,
                "Full nine-species model loaded with its curated map. Calculate the permitted μ range to run SteadyCom.",
            );
            self.emit("map");
            return Ok(());
        }
        let mut entries = Vec::new();
        let members = manifest.as_ref().and_then(|m| m.get("members")).and_then(Value::as_array).cloned().unwrap_or_default();
        for member in members {
            let member_id = text_field(&member, "id").unwrap_or_default();
            let Some(sp) = self.species.iter().find(|s| s.id == member_id).cloned() else { continue };
            if !sp.enabled {
                continue;
            }
            let preview = text_field(&member, "preview").unwrap_or_default();
            let preview = preview.strip_suffix(".json").unwrap_or(&preview).to_string();
            let model = self.load_model(&format!("published/{id}/{preview}"))?;
            entries.push(SpeciesSpec { id: Some(member_id), model, abundance: Some(sp.abundance), enabled: Some(true), color: None });
        }
        let drawing = CommunityModel::new(CommunitySpec {
            name: self.scenario_str("name"),
            species: entries,
            medium: self.medium_spec(),
            objective: Objective::parse(&self.objective),
            product_target: None,
        })?;
        self.map = Some(EditableMap::from_community(&drawing, &layout_engine_options(&(&self.layout_settings).into())));
        self.map_history.clear();
        self.set_status_message(
            StatusState::Ok,
            "Full nine-species model loaded; the map shows an excerpt. Calculate the permitted μ range to run SteadyCom.",
        );
        self.emit("map");
        Ok(())
    }

    /// `publishedMemberSBML(communityId, memberId)`: a published member's
    /// full model, converted to SBML on demand. Returns `(xml, name)`.
    pub fn published_member_sbml(&mut self, community_id: &str, member_id: &str) -> Result<(String, String)> {
        let name = format!("{community_id}-{member_id}");
        if community_id == ZUNIGA_ID {
            let Some(model) = self.models.get(&format!("published/{ZUNIGA_ID}/model-{member_id}")) else {
                return Err(StoreError::msg("Open the Zuñiga case to privately load and verify this model first."));
            };
            return Ok((to_sbml(model, &SbmlWriteOptions::default())?, name));
        }
        let manifest = self.published_manifest(community_id);
        let member = manifest
            .as_ref()
            .and_then(|m| m.get("members"))
            .and_then(Value::as_array)
            .and_then(|ms| ms.iter().find(|m| m.get("id").and_then(Value::as_str) == Some(member_id)))
            .cloned();
        let Some(file) = member.as_ref().and_then(|m| text_field(m, "model")) else {
            return Err(StoreError::msg(format!("{member_id} has no model file in this build.")));
        };
        let doc = self.assets.read_json(&format!("models/published/{community_id}/{file}"))?;
        let model = Model::from_value(&doc)?;
        Ok((to_sbml(&model, &SbmlWriteOptions::default())?, name))
    }

    // Rebuild

    /// `rebuild({ relayout, contents, solve })`: rebuild the merged model and
    /// the LP; optionally redo the geometry.
    pub fn rebuild(&mut self, opts: RebuildOptions) -> Result<()> {
        if self.is_zuniga_study() {
            if opts.solve {
                self.solve();
            }
            return Ok(());
        }
        self.invalidate_steady_com();
        self.set_status_message(StatusState::Busy, "Building model…");
        self.emit("status");

        let mut entries = Vec::new();
        for sp in self.species.clone() {
            let model = self.load_model(&sp.file)?;
            entries.push(SpeciesSpec {
                id: Some(sp.id.clone()),
                model,
                abundance: Some(sp.abundance),
                enabled: Some(sp.enabled),
                color: None,
            });
        }
        if !entries.iter().any(|e| e.enabled == Some(true)) {
            self.set_status(Status { state: StatusState::Error, message: "Enable at least one species.".into(), solve_ms: 0.0, layout_ms: 0.0 });
            self.community = None;
            self.set_problem(None);
            self.map = None;
            self.solution = None;
            self.emit("rebuild");
            return Ok(());
        }
        let members: Vec<(String, Arc<Model>)> = entries.iter().map(|e| (e.id.clone().unwrap_or_default(), e.model.clone())).collect();
        let community = CommunityModel::new(CommunitySpec {
            name: Some(self.scenario_str("name").unwrap_or_else(|| "Community".into())),
            species: entries,
            medium: self.medium_spec(),
            objective: Objective::parse(&self.objective),
            product_target: self.product_target.clone(),
        })?;
        self.community = Some(Arc::new(community));
        self.community_exchange_ids = None;
        self.joint_projection_spec = None;
        let gut_nine = self.scenario_published().as_deref() == Some("steadycom-gut-nine");
        if gut_nine {
            let members: Vec<MemberModel> = members.into_iter().map(|(id, model)| MemberModel { id, model }).collect();
            let bounds = self.assets.read_text("models/published/steadycom-gut-nine/bounds.csv")?;
            let spec = joint_steady_com_spec(&members, &bounds, JointSpecOptions::default())?;
            self.joint_projection_spec = Some(Arc::new(spec));
            self.install_joint_projection()?;
        }
        let problem = FluxProblem::new(&self.community.as_ref().expect("built").merged);
        self.default_bounds = Some(problem.snapshot_bounds());
        self.set_problem(Some(problem));
        self.apply_overrides();

        if opts.relayout || self.map.is_none() {
            let t = std::time::Instant::now();
            // The automatic layout still produces the geometry; the map adopts
            // it and owns it from here on, which is what makes it editable.
            let layout = layout_engine_options(&(&self.layout_settings).into());
            let community = self.community.as_ref().expect("built");
            let map = if opts.contents == Contents::None || gut_nine {
                EditableMap::empty_from_community(community, &layout)
            } else {
                EditableMap::from_community(community, &layout)
            };
            self.map = Some(map);
            self.map_history.clear();
            self.status_mut().layout_ms = js::round(t.elapsed().as_secs_f64() * 1000.0);
        }

        if opts.solve {
            self.solve_with(None, true);
        } else {
            self.solution = None;
            self.refresh_flux_scale();
            self.cross_feeding = Default::default();
            self.cross_feeding_explorer.index = None;
            self.cross_feeding_explorer.selected_id = None;
            self.growth = IndexMap::new();
            let next = Status {
                state: StatusState::Ok,
                message: "Model loaded. Analysis is ready to run on demand.".into(),
                solve_ms: 0.0,
                ..self.status().clone()
            };
            self.set_status(next);
        }
        if opts.relayout && gut_nine {
            self.draw_published_preview("steadycom-gut-nine")?;
        }
        self.emit("rebuild");
        Ok(())
    }

    /// `mediumSpec()`: medium limits in the shape `CommunityModel` expects.
    pub fn medium_spec(&self) -> IndexMap<String, MediumLimit> {
        self.medium
            .iter()
            .map(|(base, limit)| {
                let l = if limit.is_nan() { 0.0 } else { *limit };
                (base.clone(), MediumLimit { lower: Some(-l.abs()), upper: Some(1000.0) })
            })
            .collect()
    }

    /// `applyOverrides()`: the model's bounds, then the medium, then the
    /// per-reaction overrides, and the objective in force.
    pub fn apply_overrides(&mut self) {
        let Some(defaults) = self.default_bounds.clone() else { return };
        if self.problem.is_none() {
            return;
        }
        self.bounds_revision += 1;
        let zuniga = self.is_zuniga_study();
        let uptake_ids = self.community_exchange_ids.clone();
        let medium = self.medium.clone();
        let off: std::collections::HashSet<String> =
            self.species.iter().filter(|sp| self.species_held_out(&sp.id)).map(|sp| sp.id.clone()).collect();
        let owners: IndexMap<String, Option<String>> = if off.is_empty() {
            IndexMap::new()
        } else {
            self.community.as_ref().map(|c| c.reaction_owner.iter().map(|(k, o)| (k.clone(), o.species_id.clone())).collect()).unwrap_or_default()
        };
        let overrides = self.effective_overrides();
        let problem = self.problem_mut().expect("checked");
        problem.restore_bounds(&defaults);
        if zuniga {
            return;
        }
        for (base, limit) in &medium {
            let l = if limit.is_nan() { 0.0 } else { limit.abs() };
            if let Some(uptake) = uptake_ids.as_ref().and_then(|ids| ids.get(base)).map(|i| i.uptake.clone()) {
                if let Some(index) = problem.reaction_index(&uptake) {
                    let _ = problem.set_bounds(&uptake, Some(defaults.lb[index]), Some(l));
                    continue;
                }
            }
            let ex = community_exchange(base);
            if problem.reaction_index(&ex).is_some() {
                let _ = problem.set_bounds(&ex, Some(-l), Some(1000.0));
            }
        }
        // Per-reaction overrides win over the medium, so a knockout stays a
        // knockout. A held-out species keeps its edited bounds for when it
        // returns, but they must not reopen what holding it out closed: in a
        // joint community its reactions stay in the problem, held at zero.
        for (rid, b) in &overrides {
            if problem.reaction_index(rid).is_none() {
                continue;
            }
            if !off.is_empty() && owners.get(rid).cloned().flatten().is_some_and(|s| off.contains(&s)) {
                continue;
            }
            let _ = problem.set_bounds(rid, Some(b.lower), Some(b.upper));
        }
        self.apply_objective();
    }

    /// `speciesHeldOut(id)`: whether a species is out of the simulation
    /// (switched off, or, in a joint community, given zero abundance, which
    /// closes its reactions the same way). Its bounds can be neither applied
    /// nor edited until it is back.
    pub fn species_held_out(&self, id: &str) -> bool {
        let Some(sp) = self.species_by_id(id) else { return false };
        if !sp.enabled {
            return true;
        }
        // `!(x > 1e-9)`: a NaN abundance holds the species out too.
        let amount = self.community.as_ref().and_then(|c| c.abundances.get(id)).copied().unwrap_or(0.0);
        self.joint_projection_spec.is_some() && amount.partial_cmp(&1e-9) != Some(std::cmp::Ordering::Greater)
    }

    /// `installJointProjection()`: replace the merged model by the
    /// projection of the joint SteadyCom model at the current composition.
    pub fn install_joint_projection(&mut self) -> Result<()> {
        let Some(spec) = self.joint_projection_spec.clone() else { return Ok(()) };
        let Some(community) = self.community_mut() else { return Ok(()) };
        let amounts: IndexMap<String, f64> =
            spec.members.iter().map(|m| (m.id.clone(), community.abundances.get(&m.id).copied().unwrap_or(0.0))).collect();
        let projection = joint_community_projection(&spec, &amounts, community.objective_type, community.product_target.as_deref())?;
        community.merged = projection.model;
        community.biomass_ids = projection.biomass_ids;
        community.reaction_owner = projection.reaction_owner;
        community.metabolite_owner = projection.metabolite_owner;
        community.transfers_by_base = projection.transfers_by_base;
        community.species_compartments = projection.species_compartments;
        community.abundances = projection.abundances;
        self.community_exchange_ids = Some(projection.community_exchange_ids);
        Ok(())
    }

    /// `refreshCompositionProblem()`: rebuild the LP at the composition the
    /// species now have (after a fixed-µ SteadyCom solve changed it).
    pub fn refresh_composition_problem(&mut self) -> Result<()> {
        let species = self.species.clone();
        let joint = self.joint_projection_spec.is_some();
        let Some(community) = self.community_mut() else { return Ok(()) };
        for member in community.species.iter_mut() {
            let sp = species.iter().find(|s| s.id == member.id);
            member.abundance = sp.map_or(0.0, |s| s.abundance);
            member.enabled = sp.is_none_or(|s| s.enabled) && member.abundance > 1e-9;
        }
        community.abundances = species.iter().map(|sp| (sp.id.clone(), if sp.enabled { sp.abundance } else { 0.0 })).collect();
        if joint {
            self.install_joint_projection()?;
        } else {
            community.build()?;
        }
        let problem = FluxProblem::new(&self.community.as_ref().expect("present").merged);
        self.default_bounds = Some(problem.snapshot_bounds());
        self.set_problem(Some(problem));
        self.apply_overrides();
        Ok(())
    }
}
