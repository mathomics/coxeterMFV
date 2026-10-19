//! Files in and out: map documents, scenario state, models, flux tables and
//! reproducibility bundles (the "map I/O" and "exports" sections of
//! `store.js`).

use std::collections::HashSet;
use std::sync::Arc;

use coxeter_analysis::create_zuniga_scenario;

use coxeter_core::js;
use coxeter_core::model::Model;
use coxeter_io::reproducibility::{
    OmicsDataset as ReproDataset, OmicsState as ReproOmics, ReproStore, SolutionState, Tolerances, compare_replayed_solution, create_reproducibility_bundle,
    methods_provenance_report, verify_reproducibility_bundle,
};
use coxeter_io::sbml_write::{SbmlWriteOptions, to_sbml};
use coxeter_map::editable_map::EditableMap;
use coxeter_map::map_document::{
    ContextSpecies, MapContext, ParsedMapDocument, create_map_document, map_species_ids, parse_map_document, parse_map_document_str,
    rename_map_species, validate_map_context,
};
use coxeter_map::reconcile_map::{describe_reconciliation, prune_map_to_model, reconcile_compartments};
use coxeter_render::reaction_colors::{normalize_pathway_ribbon_mode, normalize_reaction_color_mode};
use coxeter_render::theme::{FluxColorStop, normalize_flux_color_stops_value};
use coxeter_solver::steadycom::Fiber;
use indexmap::IndexMap;
use serde_json::{Map, Value, json};

use crate::error::{Result, StoreError};
use crate::pathways::flux_color_stops_json;
use crate::store::{Store, objectives};
use crate::types::*;
use crate::util::{clamp_number, normalize_pfba_percent, num_json, to_exponential};
use crate::work::{layout_settings_json, raw_layout_settings};
use crate::omics::{CommunityOmicsPatch, dataset_from_archive};
use crate::zuniga::{ZUNIGA_ID, zuniga_community_model};

/// How long an open waits for the Zuñiga sources (a first download is a few
/// tens of megabytes; later opens read the cache).
pub const ZUNIGA_LOAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1800);

/// `validateMapDocument(doc, community, species)`: whether a map belongs to
/// the active community.
pub fn validate_map_document(doc: &Value, community: &coxeter_core::CommunityModel, species: &[Species]) -> Result<()> {
    if !doc.is_object() {
        return Err(StoreError::msg("The file does not contain a valid map."));
    }
    let (Some(reactions), Some(nodes), Some(_)) =
        (doc.get("reactions").and_then(Value::as_array), doc.get("nodes").and_then(Value::as_array), doc.get("segments").and_then(Value::as_array))
    else {
        return Err(StoreError::msg("The file does not have the expected EditableMap structure."));
    };
    // Every cell the map draws has to be a species of the community. It is
    // checked against the community and not against what happens to be
    // drawn: a map is allowed to draw fewer cells than there are species, and
    // the one on screen may be a blank canvas with none at all. A
    // switched-off species counts: a hand-built map keeps its cell, dimmed.
    let known: HashSet<&str> = species.iter().map(|sp| sp.id.as_str()).collect();
    let cells = doc.get("cells").and_then(Value::as_array).cloned().unwrap_or_default();
    if cells.iter().any(|c| !known.contains(c.get("id").and_then(Value::as_str).unwrap_or("\u{0}undefined"))) {
        return Err(StoreError::msg("The map was created for a different set of cells/species."));
    }
    // A model that is still being built loses and renames things between
    // one export and the next, and a map drawn over the earlier one is still
    // a map of this model: what is gone is taken off it when it is applied.
    // Only a map that shares nothing with the active model is a map of
    // something else.
    let ids: Vec<String> =
        reactions.iter().filter_map(|r| r.get("id")).filter(|v| js::truthy(v)).map(js::to_string).collect();
    let has = |id: &String| community.merged.reactions.contains_key(id);
    if let Some(missing) = ids.iter().find(|id| !has(id)) {
        if !ids.iter().any(has) {
            return Err(StoreError::msg(if ids.len() == 1 {
                format!("Reaction {missing} does not exist in the active model.")
            } else {
                format!("None of the {} reactions on this map exist in the active model (the first is {missing}).", ids.len())
            }));
        }
    }
    if ids.is_empty() {
        let merged: Vec<String> =
            nodes.iter().filter_map(|n| n.get("mergedId")).filter(|v| js::truthy(v)).map(js::to_string).collect();
        let has = |id: &String| community.merged.metabolites.contains_key(id);
        if let Some(lost) = merged.iter().find(|id| !has(id)) {
            if !merged.iter().any(has) {
                return Err(StoreError::msg(format!("Metabolite {lost} does not exist in the active model.")));
            }
        }
    }
    Ok(())
}

/// `hasMapStructure(doc)`: whether a parsed map has the drawing a map is
/// made of, whatever it was drawn over.
fn has_map_structure(doc: &Value) -> bool {
    doc.is_object() && ["reactions", "nodes", "segments"].iter().all(|k| doc.get(*k).is_some_and(Value::is_array))
}

/// `mapModelLabel(parsed)`: the model a map was drawn over, as the user
/// knows it: an imported model by the name of its file, anything else by its
/// species.
fn map_model_label(parsed: &ParsedMapDocument) -> String {
    let uploads: Vec<String> = parsed
        .metadata
        .as_ref()
        .and_then(|m| m.get("species"))
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|sp| sp.get("model").filter(|v| !v.is_null()).map(js::to_string))
                .filter_map(|m| m.strip_prefix("upload:").filter(|s| !s.is_empty()).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if !uploads.is_empty() {
        return uploads.iter().map(|s| format!("“{s}”")).collect::<Vec<_>>().join(", ");
    }
    let species = map_species_ids(parsed);
    if !species.is_empty() {
        return format!("species {}", species.iter().map(|s| format!("“{s}”")).collect::<Vec<_>>().join(", "));
    }
    "its model".into()
}

/// `.replace(/\s+/g, ' ')`.
fn collapse_spaces(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn context_species(species: &[Species]) -> Vec<ContextSpecies> {
    species
        .iter()
        .map(|sp| ContextSpecies { id: sp.id.clone(), file: Some(sp.file.clone()), model: None, enabled: Some(sp.enabled) })
        .collect()
}

impl Store {
    // File names

    /// `exportBaseName()`: what exported files are named after. A
    /// single-species model goes by its own id, a community by its scenario
    /// or published-community id, several uploaded models by their ids
    /// together.
    pub fn export_base_name(&self) -> String {
        let model_id = |sp: &Species| {
            self.models.get(&sp.file).map(|m| m.id.clone()).filter(|id| !id.is_empty()).unwrap_or_else(|| sp.id.clone())
        };
        if self.species.len() == 1 {
            return model_id(&self.species[0]);
        }
        let scenario = self.scenario_id().unwrap_or_default();
        let scenario = scenario.strip_prefix("published:").unwrap_or(&scenario).to_string();
        if !scenario.is_empty() && scenario != "uploaded-models" {
            return scenario;
        }
        if !self.species.is_empty() {
            let ids: Vec<String> = self.species.iter().take(3).map(model_id).collect();
            let more = if self.species.len() > 3 { format!("+{}", self.species.len() - 3) } else { String::new() };
            return format!("{}{more}", ids.join("+"));
        }
        self.community.as_ref().map(|c| c.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| "coxeter".into())
    }

    /// `fileNameFor(suffix, extension)`: `<exportBaseName>_<suffix>.<ext>`.
    pub fn file_name_for(&self, suffix: &str, extension: &str) -> String {
        coxeter_io::file_names::export_file_name(&self.export_base_name(), suffix, extension)
    }

    // Map files

    fn map_context(&self) -> MapContext {
        MapContext {
            species: context_species(&self.species),
            scenario: self.scenario_id(),
            pathway_manager: Some(self.pathway_manager_state()),
            flux_color_stops: Some(flux_color_stops_json(self.view.flux_color_stops.as_deref())),
            study: Some(self.zuniga_state()),
        }
    }

    /// `exportTooltips()`: hover text for an exported SVG: what a reaction or
    /// a metabolite is, named as the map names it, with its model id, and
    /// what the solution puts through a reaction.
    pub fn export_tooltips(&self) -> coxeter_render::Tooltips {
        let mut out = coxeter_render::Tooltips::default();
        let Some(map) = self.map.as_ref() else { return out };
        let solved = self.solution.as_ref().is_some_and(|s| s.is_optimal());
        let fluxes = self.display_fluxes();
        let named = |kind: &str, id: &str| {
            let n = self.entity_label(kind, id);
            let mut lines = vec![n.label.clone()];
            if !n.name.is_empty() && n.name != n.label {
                lines.push(n.name.clone());
            }
            if !n.id.is_empty() && n.id != n.label {
                lines.push(format!("ID: {}", n.id));
            }
            lines
        };
        // Bounds as written in the model: 1000, not 1.00e+3.
        let bound = |v: f64| {
            if v.is_finite() { js::number_to_string(js::string_to_number(&crate::omics::to_precision(v, 6))) } else { js::number_to_string(v) }
        };
        for rid in map.reactions().keys() {
            let mut lines = named("reaction", rid);
            if let Some(reaction) = self.community.as_ref().and_then(|c| c.merged.reactions.get(rid)) {
                if !reaction.subsystem.is_empty() {
                    lines.push(format!("Subsystem: {}", reaction.subsystem));
                }
            }
            let bounds = self.current_reaction_bounds(rid);
            lines.push(format!("Bounds: [{}, {}]", bound(bounds.lower), bound(bounds.upper)));
            lines.push(if solved {
                format!("Flux: {}", coxeter_render::format_flux(Some(fluxes.get(rid).copied().unwrap_or(0.0)), 4))
            } else {
                "Flux: not solved".to_string()
            });
            out.reactions.insert(rid.clone(), lines.join("\n"));
        }
        for node in map.nodes().values() {
            let Some(merged_id) = node.merged_id.as_deref().filter(|m| !m.is_empty()) else {
                out.nodes.insert(node.id.clone(), node.base.clone().unwrap_or_else(|| node.id.clone()));
                continue;
            };
            let mut lines = named("metabolite", merged_id);
            if let Some(m) = self.community.as_ref().and_then(|c| c.merged.metabolites.get(merged_id)) {
                if !m.formula.is_empty() {
                    lines.push(format!("Formula: {}", m.formula));
                }
            }
            if let Some(compartment) = node.compartment.as_deref().filter(|c| !c.is_empty()) {
                let names = self
                    .species
                    .iter()
                    .find(|sp| Some(&sp.id) == node.cell_id.as_ref())
                    .and_then(|sp| self.models.get(&sp.file))
                    .map(|m| m.compartment_names.clone())
                    .unwrap_or_default();
                lines.push(format!("Compartment: {}", coxeter_core::compartments::cell_compartment_name(compartment, &names, "en")));
            }
            out.nodes.insert(node.id.clone(), lines.join("\n"));
        }
        out
    }

    /// `exportMetadata()`: what an exported SVG says about itself in its
    /// `<metadata>`: the program that drew it, the model and analysis behind
    /// it, and how the map was drawn.
    pub fn export_metadata(&self) -> Value {
        let method = match &self.last_run {
            Some(run) => run.label.to_string(),
            None if self.loopless => "Loopless FBA".into(),
            None if self.parsimonious => "pFBA".into(),
            None => "FBA".into(),
        };
        let objective_value = self.solution.as_ref().map(|s| s.objective_value).filter(|v| v.is_finite());
        json!({
            "generator": "Coxeter Multicell Flux Visualizer",
            "software": coxeter_io::reproducibility::software_identity().to_json(),
            "exportedAt": coxeter_map::map_document::iso_timestamp_now(),
            "scenario": { "id": self.scenario_id(), "name": self.scenario_str("name") },
            "species": self.species.iter().filter(|sp| sp.enabled).map(|sp| json!({
                "id": sp.id,
                "model": self.models.get(&sp.file).map(|m| m.id.clone()).filter(|id| !id.is_empty()).unwrap_or_else(|| sp.file.clone()),
                "abundance": num_json(sp.abundance),
            })).collect::<Vec<_>>(),
            "analysis": {
                "method": method,
                "status": self.solution.as_ref().map(|s| s.status.as_str()),
                "objective": self.objective,
                "objectiveValue": objective_value.map(num_json),
            },
            "map": {
                "reactions": self.map.as_ref().map_or(0, |m| m.reactions().len()),
                "metabolites": self.map.as_ref().map_or(0, |m| m.nodes().len()),
                "labels": { "metabolite": self.map_label_source("metabolite"), "reaction": self.map_label_source("reaction") },
                "lineColor": self.view.reaction_color_mode.as_str(),
                "lineWidthByFlux": self.view.width_by_flux,
                "dashedWithoutFlux": self.view.dash_no_flux,
                "lineHops": self.view.line_hops,
            },
            "note": "Exported map. Hover a reaction or metabolite for its identifiers and flux.",
        })
    }

    /// `mapDocument()`: the map with its context, as Save map writes it.
    pub fn map_document(&self) -> Result<Value> {
        let map = self.map.as_ref().ok_or_else(|| StoreError::msg("There is no active map to save."))?;
        Ok(create_map_document(map, &self.map_context())?)
    }

    /// `mapJSON()`.
    pub fn map_json(&self) -> Result<String> {
        Ok(js::stringify(&self.map_document()?, Some(2)))
    }

    /// `loadMapJSON(text)`: open a map onto the active context.
    pub fn load_map_json(&mut self, text: &str) -> Result<()> {
        self.map_load_note = String::new();
        let parsed = parse_map_document_str(text)?;
        self.apply_map_document(parsed)
    }

    /// `loadMapDocument(text)`: [`Store::load_map_document_named`] without a
    /// file name.
    pub fn load_map_document(&mut self, text: &str) -> Result<()> {
        self.load_map_document_named(text, None)
    }

    /// `loadMapDocument(text, { name })`: open a saved map, switching to the
    /// context it was drawn over if need be.
    ///
    /// A map only means something over the model it was drawn on, so the
    /// file records which scenario or published community that was. Try the
    /// active context first, so nothing moves when the file already fits, and
    /// only then go to the one the file names.
    ///
    /// A map whose model is not here at all (one drawn over an imported
    /// model, opened after a restart) is kept rather than refused, and drawn
    /// as soon as that model is loaded (see [`Store::draw_waiting_map`]): Ok,
    /// with `waiting_map` set. A file that is not a map is still an error.
    pub fn load_map_document_named(&mut self, text: &str, name: Option<&str>) -> Result<()> {
        let parsed = parse_map_document_str(text)?;
        self.map_load_note = String::new();
        // The map opened now replaces one that was still waiting for its model.
        self.waiting_map = None;
        let study = parsed.study.as_ref().and_then(|s| s.get("id")).and_then(Value::as_str);
        if study == Some(ZUNIGA_ID) && !self.is_zuniga_study() {
            let scenario = parsed.study.as_ref().and_then(|s| s.get("scenarioId")).and_then(Value::as_str).map(str::to_string);
            self.use_zuniga_study_blocking(scenario.as_deref(), ZUNIGA_LOAD_TIMEOUT)?;
        }
        match self.open_map_document(parsed.clone()) {
            Ok(()) => Ok(()),
            Err(error) => {
                // A broken file, or the Zuñiga case with its own context, is
                // not waiting for anything.
                if parsed.study.as_ref().is_some_and(js::truthy) || !has_map_structure(&parsed.map) {
                    return Err(error);
                }
                let reason = error.to_string();
                let log = format!("Map {} kept until its model is loaded: {reason}", name.unwrap_or(""));
                self.waiting_map = Some(WaitingMap { needs: map_model_label(&parsed), parsed, name: name.map(str::to_string), reason });
                self.add_log(StatusState::Warn, &collapse_spaces(&log), LogKind::Message);
                self.emit("waiting-map");
                Ok(())
            }
        }
    }

    /// `drawWaitingMap()`: draw the map that was opened before its model was
    /// loaded, now that a model has been. Called after each context is
    /// opened; only an import of models, or the very scenario the map names,
    /// counts as loading its model. A teaching scenario that happens to
    /// share reaction ids does not. Whether it was drawn.
    pub fn draw_waiting_map(&mut self) -> bool {
        let Some(waiting) = self.waiting_map.clone() else { return false };
        if self.map.is_none() || self.community.is_none() {
            return false;
        }
        let here = self.scenario_id();
        let uploaded = self.scenario.as_ref().and_then(|s| s.get("uploaded")).is_some_and(js::truthy);
        let imported = uploaded || here.as_deref() == Some("reproducibility-bundle");
        let wanted = waiting.parsed.metadata.as_ref().and_then(|m| m.get("scenario")).filter(|v| !v.is_null()).map(js::to_string);
        if !imported && here != wanted {
            return false;
        }
        // An import has just said what it loaded; say what was drawn after it.
        let opened = if imported && !self.status().message.is_empty() { format!("{} ", self.status().message) } else { String::new() };
        let file = waiting.name.as_ref().map_or_else(|| "the map".to_string(), |n| format!("“{n}”"));
        self.map_load_note = String::new();
        let drawn = match self.apply_map_document(waiting.parsed.clone()) {
            Ok(()) => Ok(()),
            Err(error) => match self.adopt_map_species(&waiting.parsed) {
                Ok(true) => Ok(()),
                Ok(false) => Err(error),
                Err(other) => Err(other),
            },
        };
        if let Err(error) = drawn {
            self.set_status_message(StatusState::Warn, format!("{opened}{file} is still waiting: it does not fit this model ({error})"));
            self.emit("status");
            return false;
        }
        self.waiting_map = None;
        let state = if self.map_load_note.is_empty() { self.status().state } else { StatusState::Warn };
        let message = format!("{opened}Drew {file}, which was waiting for this model.{}", self.map_load_note);
        self.set_status_message(state, message);
        let log = format!("Drew the waiting map {} over {}", waiting.name.as_deref().unwrap_or(""), here.unwrap_or_default());
        self.add_log(StatusState::Ok, &collapse_spaces(&log), LogKind::Message);
        self.emit("status");
        true
    }

    /// `discardWaitingMap()`: forget a map that is waiting for its model.
    pub fn discard_waiting_map(&mut self) {
        if self.waiting_map.take().is_some() {
            self.emit("waiting-map");
        }
    }

    /// `openMapDocument(parsed)`: the body of `loadMapDocument`: fit the map
    /// to the active context or the one it names.
    fn open_map_document(&mut self, parsed: ParsedMapDocument) -> Result<()> {
        match self.apply_map_document(parsed.clone()) {
            Ok(()) => Ok(()),
            Err(error) => {
                let wanted = parsed.metadata.as_ref().and_then(|m| m.get("scenario")).filter(|v| !v.is_null()).map(js::to_string);
                if let Some(wanted) = wanted.filter(|w| Some(w) != self.scenario_id().as_ref()) {
                    self.restore_map_context(&wanted)?;
                    return self.apply_map_document(parsed);
                }
                // The right scenario is already open and the map still does
                // not fit it: the one thing left that is not a real mismatch
                // is the same model under another id.
                if self.adopt_map_species(&parsed)? {
                    return Ok(());
                }
                Err(error)
            }
        }
    }

    /// `adoptMapSpecies(parsed)`: open a map that was drawn over the same
    /// model under another id. With one species in the file and one in the
    /// session there is no doubt which is meant. `false` when this is not
    /// that case.
    pub fn adopt_map_species(&mut self, parsed: &ParsedMapDocument) -> Result<bool> {
        let saved = map_species_ids(parsed);
        let active: Vec<Species> = self.species.iter().filter(|sp| sp.enabled).cloned().collect();
        if saved.len() != 1 || active.len() != 1 || saved[0] == active[0].id {
            return Ok(false);
        }
        let mut renamed = rename_map_species(parsed, &saved[0], &active[0].id, Some(&active[0].file));
        // The cell is drawn with the name of the model it belongs to now.
        if let Some(name) = self.models.get(&active[0].file).map(|m| m.cell.name.clone()).filter(|n| !n.is_empty()) {
            if let Some(cells) = renamed.map.get_mut("cells").and_then(Value::as_array_mut) {
                for cell in cells {
                    if let Some(o) = cell.as_object_mut() {
                        o.insert("name".into(), json!(name));
                    }
                }
            }
        }
        if let Err(error) = self.apply_map_document(renamed) {
            return Err(StoreError::msg(format!(
                "{error} (The map was saved for \"{}\" and was read against \"{}\", the model now open.)",
                saved[0], active[0].id
            )));
        }
        self.map_load_note = format!(" It was saved for \"{}\" and has been read against \"{}\".{}", saved[0], active[0].id, self.map_load_note);
        Ok(true)
    }

    /// `restoreMapContext(wanted)`: switch to the scenario or published
    /// community a saved map names.
    pub fn restore_map_context(&mut self, wanted: &str) -> Result<()> {
        if let Some(id) = wanted.strip_prefix("published:").filter(|id| !id.is_empty()) {
            let Some(community) = self.published_communities.iter().find(|c| c.get("id").and_then(Value::as_str) == Some(id)).cloned()
            else {
                return Err(StoreError::msg(format!("This map belongs to the published community \"{id}\", which is not available here.")));
            };
            if id == ZUNIGA_ID {
                self.use_zuniga_study_blocking(None, ZUNIGA_LOAD_TIMEOUT)?;
            } else {
                self.use_published_community(id)?;
            }
            if self.map.is_none() {
                let name = community.get("name").map(js::to_string).unwrap_or_default();
                return Err(StoreError::msg(format!("{name} bundles no models in this build, so there is nothing for the map to sit on.")));
            }
            return Ok(());
        }
        if !self.scenarios.iter().any(|s| s.get("id").and_then(Value::as_str) == Some(wanted)) {
            return Err(StoreError::msg(format!(
                "This map belongs to the scenario \"{wanted}\", which is not loaded."
            )));
        }
        self.use_scenario(wanted)
    }

    /// `applyMapDocument(parsed)`: install a parsed map over the active
    /// community, bringing a map from an earlier export up to date.
    pub fn apply_map_document(&mut self, parsed: ParsedMapDocument) -> Result<()> {
        let stops = flux_color_stops_json(
            parsed
                .flux_color_stops
                .as_ref()
                .map(|s| s.iter().map(|x| FluxColorStop { value: x.value, color: x.color.clone() }).collect::<Vec<_>>())
                .as_deref(),
        );
        let flux_color_stops = normalize_flux_color_stops_value(&stops)?;
        let (Some(community), Some(_)) = (self.community.clone(), self.map.as_ref()) else {
            return Err(StoreError::msg("There is no active model to receive the map."));
        };
        let study = parsed.study.clone().filter(|s| s.get("id").and_then(Value::as_str) == Some(ZUNIGA_ID));
        if let Some(study) = &study {
            if !self.is_zuniga_study() {
                return Err(StoreError::msg(
                    "Open the Zuñiga case before importing this map, or use the File menu to restore its context.",
                ));
            }
            self.validate_zuniga_state(study)?;
        }
        validate_map_context(&parsed, &MapContext { species: context_species(&self.species), ..MapContext::default() })?;
        // The case's maps are drawn over its joint (coculture) community,
        // whichever scenario is open.
        let map_community = match self.zuniga.study.clone().filter(|_| self.is_zuniga_study()) {
            Some(study) => Arc::new(zuniga_community_model(&create_zuniga_scenario(&study, "mutant-coculture")?.community)),
            None => community.clone(),
        };
        validate_map_document(&parsed.map, &map_community, &self.species)?;
        if let Some(study) = &study {
            let id = study.get("scenarioId").and_then(Value::as_str).unwrap_or("mutant-coculture").to_string();
            self.use_zuniga_scenario(&id, false)?;
            self.zuniga.comparison = study.get("comparison").and_then(Value::as_str).unwrap_or("absolute").to_string();
        }
        let before = self.map.as_ref().expect("checked").to_json();
        let mut next = EditableMap::from_json(&parsed.map)?;
        // A map from an earlier export of the model, or from an earlier build
        // of this program, is brought up to date rather than refused. A map
        // that needs nothing comes through untouched.
        if !self.is_zuniga_study() {
            // What a switched-off species has drawn is kept, not taken as lost.
            let keep: HashSet<String> = self.species.iter().filter(|sp| !sp.enabled).map(|sp| sp.id.clone()).collect();
            let contents = prune_map_to_model(&mut next, &community, &keep);
            let shell = reconcile_compartments(&mut next, &community);
            self.map_load_note = format!("{}{}", self.map_load_note, describe_reconciliation(Some(&contents), Some(&shell)));
        }
        self.map_history.record(&before, &next.to_json(), "Load map JSON");
        self.map = Some(next);
        self.view.flux_color_stops = flux_color_stops;
        self.refresh_flux_scale();
        self.restore_pathway_manager_state(parsed.pathway_manager.as_ref());
        // What was just opened is what is in the file, pathways and colours
        // included. Undo still goes back to the previous drawing, which then
        // counts as unsaved.
        self.mark_map_saved();
        self.selected_reaction = None;
        self.selected_entity = None;
        self.variability = None;
        self.emit("map");
        Ok(())
    }

    // Scenario state

    /// `stateJSON()`: the Scenario file.
    pub fn state_json(&self) -> String {
        js::stringify(&self.state_value(), Some(2))
    }

    /// The Scenario file as a value.
    pub fn state_value(&self) -> Value {
        let medium: Map<String, Value> = self.medium.iter().map(|(k, v)| (k.clone(), num_json(*v))).collect();
        let overrides: Vec<Value> = self
            .overrides
            .iter()
            .map(|(id, b)| json!({"id": id, "lower": num_json(b.lower), "upper": num_json(b.upper)}))
            .collect();
        json!({
            "scenario": self.scenario_id(),
            "study": self.zuniga_state(),
            "steadyComConfig": self.steady_com_config.to_json(),
            "steadyCom": self.solution.as_ref().and_then(|s| s.steady_com.as_ref()).map(|sc| json!({"mu": num_json(sc.mu)})),
            "objective": self.objective,
            "productTarget": self.product_target,
            "customObjective": self.custom_objective.as_ref().map(CustomObjective::to_json),
            "parsimonious": self.parsimonious,
            "pfbaBiomassPercent": num_json(self.pfba_biomass_percent),
            "loopless": self.loopless,
            "fvaEnabled": self.fva_enabled,
            "layoutSettings": layout_settings_json(&self.layout_settings),
            "reactionColorMode": self.view.reaction_color_mode.as_str(),
            "widthByFlux": self.view.width_by_flux,
            "dashNoFlux": self.view.dash_no_flux,
            "lineHops": self.view.line_hops,
            "fluxColorStops": flux_color_stops_json(self.view.flux_color_stops.as_deref()),
            "pathwayRibbonMode": self.view.pathway_ribbon_mode.as_str(),
            "pathwayRibbonWidth": num_json(self.view.pathway_ribbon_width),
            "pathwayRibbonOpacity": num_json(self.view.pathway_ribbon_opacity),
            "pathwayManager": self.pathway_manager_state(),
            "omics": self.omics_state(),
            "medium": medium,
            "species": self.species.iter().map(Species::to_json).collect::<Vec<_>>(),
            "overrides": overrides,
        })
    }

    /// `loadState(text)`: open a Scenario file. A fixed-µ SteadyCom result is
    /// recomputed in the background (see `poll_jobs`).
    pub fn load_state(&mut self, text: &str) -> Result<()> {
        let doc: Value = serde_json::from_str(text)?;
        let flux_color_stops = normalize_flux_color_stops_value(doc.get("fluxColorStops").unwrap_or(&Value::Null))?;
        let study_id = doc.get("study").and_then(|s| s.get("id")).and_then(Value::as_str);
        if study_id == Some(ZUNIGA_ID) || doc.get("scenario").and_then(Value::as_str) == Some(&format!("published:{ZUNIGA_ID}")) {
            let study = doc.get("study").filter(|s| js::truthy(s));
            let scenario = study
                .and_then(|s| s.get("scenarioId"))
                .filter(|v| !v.is_null())
                .map_or_else(|| "mutant-coculture".to_string(), js::to_string);
            if !self.is_zuniga_study() {
                self.use_zuniga_study_blocking(Some(&scenario), ZUNIGA_LOAD_TIMEOUT)?;
            }
            if let Some(study) = study {
                self.validate_zuniga_state(study)?;
            }
            self.use_zuniga_scenario(&scenario, false)?;
            self.view.flux_color_stops = flux_color_stops;
            let layout = match doc.get("layoutSettings").filter(|v| !v.is_null()) {
                Some(v) => raw_layout_settings(v),
                None => (&self.layout_settings).into(),
            };
            self.layout_settings = coxeter_map::layout_settings::normalize_layout_settings(&layout);
            let mode = doc.get("reactionColorMode").filter(|v| !v.is_null()).map_or_else(|| "flux".to_string(), js::to_string);
            self.view.reaction_color_mode = normalize_reaction_color_mode(&mode);
            let comparison = study
                .and_then(|s| s.get("comparison"))
                .filter(|v| !v.is_null())
                .map_or_else(|| "absolute".to_string(), js::to_string);
            self.set_zuniga_comparison(&comparison)?;
            self.restore_pathway_manager_state(doc.get("pathwayManager").filter(|v| !v.is_null()));
            self.mark_scenario_saved();
            self.emit("solution");
            return Ok(());
        }
        let steady_mu = doc.get("steadyCom").filter(|v| js::truthy(v)).map(|s| s.get("mu").and_then(Value::as_f64).unwrap_or(f64::NAN));
        if steady_mu.is_some_and(|mu| !mu.is_finite() || mu < 0.0) {
            return Err(StoreError::msg("The saved SteadyCom growth rate is invalid."));
        }
        let fiber = doc.get("steadyComConfig").and_then(|c| c.get("fiber")).filter(|f| !f.is_null());
        let fiber = match fiber {
            None => None,
            Some(f) => {
                let bt = f.get("bacteroides").and_then(Value::as_f64).unwrap_or(f64::NAN);
                let fp = f.get("faecalibacterium").and_then(Value::as_f64);
                if !bt.is_finite() || !(1.0..=30.0).contains(&bt) || fp != Some(5.0) {
                    return Err(StoreError::msg("The saved SteadyCom fiber condition is invalid."));
                }
                Some(Fiber { bacteroides: bt, faecalibacterium: 5.0 })
            }
        };
        self.invalidate_steady_com();
        let scenario_id = doc.get("scenario").and_then(Value::as_str).map(str::to_string);
        if let Some(id) = scenario_id.as_deref().and_then(|s| s.strip_prefix("published:")) {
            self.use_published_community(id)?;
        }
        let parsimonious = doc.get("steadyComConfig").and_then(|c| c.get("parsimonious")).is_some_and(js::truthy);
        self.steady_com_config = SteadyComConfig { fiber, parsimonious };
        if let Some(s) = self.scenarios.iter().find(|s| s.get("id").and_then(Value::as_str) == scenario_id.as_deref()).cloned() {
            self.scenario = Some(s);
        }
        self.reset_opened_constraints();
        let get = |k: &str| doc.get(k).filter(|v| !v.is_null());
        self.objective = get("objective").map_or_else(|| objectives::EQUAL_GROWTH.to_string(), js::to_string);
        self.product_target = get("productTarget").map(js::to_string);
        self.custom_objective = None;
        self.parsimonious = get("parsimonious").or_else(|| get("pfba")).is_some_and(js::truthy);
        let percent = match get("pfbaBiomassPercent") {
            Some(p) => js::to_number(p),
            None => doc.get("pfbaFraction").map_or(100.0, |f| js::to_number(f) * 100.0),
        };
        self.pfba_biomass_percent = normalize_pfba_percent(percent);
        self.loopless = get("loopless").or_else(|| get("looplessFba")).is_some_and(js::truthy);
        if self.loopless {
            self.parsimonious = false;
        }
        self.fva_enabled = !self.loopless && get("fvaEnabled").or_else(|| get("fva")).is_some_and(js::truthy);
        self.fva_ranges = IndexMap::new();
        self.fva_progress = FvaProgress::default();
        let layout = match get("layoutSettings") {
            Some(v) => raw_layout_settings(v),
            None => (&self.layout_settings).into(),
        };
        self.layout_settings = coxeter_map::layout_settings::normalize_layout_settings(&layout);
        if let Some(mode) = get("reactionColorMode") {
            self.view.reaction_color_mode = normalize_reaction_color_mode(&js::to_string(mode));
        }
        // Older scenario files say nothing about these: they keep what is set.
        if let Some(Value::Bool(b)) = get("widthByFlux") {
            self.view.width_by_flux = *b;
        }
        if let Some(Value::Bool(b)) = get("dashNoFlux") {
            self.view.dash_no_flux = *b;
        }
        if let Some(Value::Bool(b)) = get("lineHops") {
            self.view.line_hops = *b;
        }
        self.view.flux_color_stops = flux_color_stops;
        if let Some(mode) = get("pathwayRibbonMode") {
            self.view.pathway_ribbon_mode = normalize_pathway_ribbon_mode(&js::to_string(mode));
        }
        if let Some(w) = get("pathwayRibbonWidth") {
            self.view.pathway_ribbon_width = clamp_number(js::to_number(w), 20.0, 120.0, 56.0);
        }
        if let Some(o) = get("pathwayRibbonOpacity") {
            self.view.pathway_ribbon_opacity = clamp_number(js::to_number(o), 0.04, 0.45, 0.18);
        }
        self.medium = match get("medium") {
            Some(Value::Object(m)) => m.iter().map(|(k, v)| (k.clone(), js::to_number(v))).collect(),
            _ => IndexMap::new(),
        };
        if let Some(Value::Array(list)) = get("species") {
            self.species = list.iter().map(Species::from_json).collect();
        }
        let gut_nine = self.scenario_published().as_deref() == Some("steadycom-gut-nine");
        self.rebuild(RebuildOptions {
            relayout: true,
            contents: if gut_nine { coxeter_map::Contents::None } else { coxeter_map::Contents::All },
            solve: steady_mu.is_none(),
        })?;
        self.restore_pathway_manager_state(get("pathwayManager"));
        self.restore_omics_state(get("omics"));
        self.overrides = get("overrides")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .map(|o| {
                        let id = o.get("id").map(js::to_string).unwrap_or_default();
                        let n = |k: &str| o.get(k).map_or(f64::NAN, js::to_number);
                        (id, Bounds { lower: n("lower"), upper: n("upper") })
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.custom_objective = self.custom_objective_from(doc.get("customObjective"));
        self.apply_overrides();
        // Before solving, so the repaint that follows already reads "Saved".
        self.mark_scenario_saved();
        match steady_mu {
            Some(mu) => {
                self.solve_steady_com_at(mu)?;
            }
            None => {
                self.solve();
            }
        }
        Ok(())
    }

    // Models

    fn export_species(&self, species_id: Option<&str>) -> Result<(Species, &Model)> {
        let sp = species_id
            .and_then(|id| self.species_by_id(id))
            .or_else(|| self.model_editor_species())
            .cloned()
            .ok_or_else(|| StoreError::msg("Select a species to export."))?;
        let model = self.models.get(&sp.file).ok_or_else(|| StoreError::msg(format!("The model for {} was not found.", sp.id)))?;
        Ok((sp, model))
    }

    /// `modelJSON(speciesId)`: the selected species' model as COBRA JSON.
    pub fn model_json(&self, species_id: Option<&str>) -> Result<String> {
        let (_, model) = self.export_species(species_id.or(self.model_editor_species_id.as_deref()))?;
        Ok(js::stringify(&model.to_json(), Some(2)))
    }

    /// `modelSBML(speciesId)`: the selected species as SBML L3V1 + fbc-v2,
    /// the format cobrapy and the COBRA Toolbox read. Built from the model in
    /// memory, so it covers whatever the user has edited or imported.
    pub fn model_sbml(&self, species_id: Option<&str>) -> Result<String> {
        let (_, model) = self.export_species(species_id.or(self.model_editor_species_id.as_deref()))?;
        Ok(to_sbml(model, &SbmlWriteOptions::default())?)
    }

    // Reproducibility

    fn repro_store(&self) -> ReproStore<'_> {
        let solution = self.solution.as_ref().map(|s| {
            let r = s.result.as_ref();
            let entries = |m: Option<&IndexMap<String, f64>>| m.map(|m| m.iter().map(|(k, v)| (k.clone(), *v)).collect()).unwrap_or_default();
            let dual = r.map(|r| {
                let mut o = Map::new();
                o.insert("available".into(), json!(r.dual_analysis.available));
                if let Some(x) = &r.dual_analysis.source {
                    o.insert("source".into(), json!(x));
                }
                if let Some(x) = &r.dual_analysis.interpretation {
                    o.insert("interpretation".into(), json!(x));
                }
                if let Some(x) = &r.dual_analysis.reason {
                    o.insert("reason".into(), json!(x));
                }
                Value::Object(o)
            });
            SolutionState {
                status: json!(s.status.as_str()),
                objective_value: Some(s.objective_value),
                iterations: r.map(|r| json!(r.iterations)),
                fluxes: s.fluxes.iter().map(|(k, v)| (k.clone(), *v)).collect(),
                reduced_costs: entries(r.map(|r| &r.reduced_costs)),
                scaled_reduced_costs: entries(r.map(|r| &r.scaled_reduced_costs)),
                reduced_cost_scale: r.and_then(|r| r.reduced_cost_scale),
                shadow_prices: entries(r.map(|r| &r.shadow_prices)),
                dual_analysis: dual,
                zuniga: s.zuniga.as_ref().map(|z| z.to_json()),
            }
        });
        let o = &self.omics;
        let omics = ReproOmics {
            dataset: o.dataset.as_ref().map(|d| ReproDataset {
                source: json!(d.source),
                data_type: json!(d.data_type),
                conditions: json!(d.conditions),
                values: d.values.iter().map(|(k, row)| (k.clone(), row.to_json())).collect(),
            }),
            enabled: json!(o.enabled),
            mode: json!(o.mode),
            condition_a: json!(o.condition_a),
            condition_b: json!(o.condition_b),
            metric: json!(o.metric),
            pseudocount: num_json(o.pseudocount),
            gimme_enabled: o.gimme_enabled,
            gimme_threshold: o.gimme_threshold.map_or(Value::Null, num_json),
            gimme_objective_fraction: num_json(o.gimme_objective_fraction),
            gimme_result: o.gimme_result.as_ref().map(|r| r.to_json()),
            report: o.report.as_ref().map(|r| r.to_json()),
        };
        ReproStore {
            species: self.species.iter().map(Species::to_json).collect(),
            models: self.species.iter().filter_map(|sp| self.models.get(&sp.file).map(|m| (sp.file.clone(), m))).collect(),
            scenario: self.scenario.clone(),
            objective: json!(self.objective),
            product_target: self.product_target.as_ref().map(|t| json!(t)),
            custom_objective: self.custom_objective.as_ref().map(CustomObjective::to_json),
            medium: self.medium.iter().map(|(k, v)| (k.clone(), num_json(*v))).collect(),
            overrides: self.overrides.iter().map(|(k, b)| (k.clone(), b.to_json())).collect(),
            parsimonious: self.parsimonious,
            pfba_biomass_percent: num_json(self.pfba_biomass_percent),
            loopless: self.loopless,
            fva_enabled: self.fva_enabled,
            fva_ranges: self.fva_ranges.iter().map(|(k, r)| (k.clone(), json!({"min": num_json(r.min), "max": num_json(r.max)}))).collect(),
            growth: self.growth.iter().map(|(k, v)| (k.clone(), num_json(*v))).collect(),
            map: self.map.as_ref().map(EditableMap::to_json),
            omics: Some(omics),
            solution,
            zuniga: self.zuniga_bundle_state(),
        }
    }

    /// `reproducibilityBundle()`.
    pub fn reproducibility_bundle(&self) -> Result<Value> {
        if self.solution.as_ref().is_some_and(|s| s.steady_com.is_some()) {
            return Err(StoreError::msg(
                "Fixed-μ SteadyCom replay is saved through Scenario JSON. This reproducibility bundle supports ordinary FBA analyses; export the scenario and flux CSV for this solution.",
            ));
        }
        Ok(create_reproducibility_bundle(&self.repro_store()))
    }

    /// `methodsProvenanceReport()`: the bundle and its Markdown report.
    pub fn methods_provenance_report(&self) -> Result<(Value, String)> {
        let bundle = self.reproducibility_bundle()?;
        let text = methods_provenance_report(&bundle);
        Ok((bundle, text))
    }

    /// `verifyReproducibilityBundle(bundle)`.
    pub fn verify_reproducibility_bundle(&self, bundle: &Value) -> Result<Value> {
        Ok(serde_json::to_value(verify_reproducibility_bundle(bundle)?)?)
    }

    /// `importReproducibilityBundle(bundle)`: restore an archived analysis
    /// and replay it. Returns the integrity check with the replay comparison.
    pub fn import_reproducibility_bundle(&mut self, bundle: &Value) -> Result<Value> {
        let check = verify_reproducibility_bundle(bundle)?;
        if !check.valid {
            return Err(StoreError::msg("The bundle fails SHA-256 verification; it may have been modified or be corrupt."));
        }
        if bundle.get("study").and_then(|s| s.get("id")).and_then(Value::as_str) == Some(ZUNIGA_ID) {
            return self.import_zuniga_bundle(bundle);
        }
        self.reset_zuniga_study();
        for key in std::mem::take(&mut self.uploaded_model_keys) {
            self.models.remove(&key);
        }
        let community = bundle.get("community");
        let mut species = Vec::new();
        for raw in community.and_then(|c| c.get("species")).and_then(Value::as_array).cloned().unwrap_or_default() {
            let mut sp = Species::from_json(&raw);
            let Some(doc) = bundle.get("models").and_then(|m| m.get(&sp.id)) else { continue };
            let key = format!("repro-{}", sp.id);
            self.models.insert(key.clone(), Model::from_value(doc)?);
            self.uploaded_model_keys.insert(key.clone());
            sp.file = key;
            species.push(sp);
        }
        if species.is_empty() {
            return Err(StoreError::msg("The bundle contains no reproducible models."));
        }
        let get = |v: Option<&Value>, k: &str| v.and_then(|x| x.get(k)).filter(|x| !x.is_null()).cloned();
        self.scenario = Some(json!({
            "id": "reproducibility-bundle",
            "name": get(community, "name").unwrap_or_else(|| json!("Reproduced analysis")),
            "objective": get(community, "objective"),
            "species": species.iter().map(Species::to_json).collect::<Vec<_>>(),
        }));
        self.species = species;
        self.objective = get(community, "objective").map_or_else(|| objectives::EQUAL_GROWTH.to_string(), |v| js::to_string(&v));
        self.product_target = get(community, "productTarget").map(|v| js::to_string(&v));
        self.custom_objective = None;
        let constraints = bundle.get("constraints");
        self.medium = match get(constraints, "medium") {
            Some(Value::Object(m)) => m.iter().map(|(k, v)| (k.clone(), js::to_number(v))).collect(),
            _ => IndexMap::new(),
        };
        self.overrides = match get(constraints, "reactionOverrides") {
            Some(Value::Object(m)) => m
                .iter()
                .map(|(k, b)| {
                    let n = |x: &str| b.get(x).map_or(f64::NAN, js::to_number);
                    (k.clone(), Bounds { lower: n("lower"), upper: n("upper") })
                })
                .collect(),
            _ => IndexMap::new(),
        };
        let analysis = bundle.get("analysis");
        self.parsimonious = get(analysis, "parsimonious").is_some_and(|v| js::truthy(&v));
        self.pfba_biomass_percent = get(analysis, "pfbaBiomassPercent").map_or(100.0, |v| js::to_number(&v));
        self.loopless = get(analysis, "loopless").is_some_and(|v| js::truthy(&v));
        // FVA is replayed only after the primary numerical solution has been
        // compared; its ranges are restored from the archive below.
        self.fva_enabled = false;
        self.rebuild(RebuildOptions::default())?;
        self.custom_objective = self.custom_objective_from(community.and_then(|c| c.get("customObjective")));
        self.apply_overrides();
        // Restore the exact archived drawing after the biological model exists.
        if let Some(map) = get(Some(bundle), "map") {
            self.map = Some(EditableMap::from_json(&map)?);
            self.map_history.clear();
        }
        // Rehydrate the omics dataset and its settings before replaying GIMME.
        let archived_omics = get(Some(bundle), "omics");
        match &archived_omics {
            Some(o) => {
                self.omics.dataset = Some(Arc::new(dataset_from_archive(o)));
                if let Some(settings) = o.get("settings").filter(|v| v.is_object()) {
                    self.omics.assign(CommunityOmicsPatch::from_json(settings));
                }
                self.omics.gimme_result = None;
                self.recompute_community_omics();
            }
            None => {
                let o = &mut self.omics;
                o.dataset = None;
                o.enabled = false;
                o.scores = IndexMap::new();
                o.colors = IndexMap::new();
                o.scale = None;
                o.report = None;
                o.gimme_enabled = false;
                o.gimme_result = None;
            }
        }
        let method = get(analysis, "method").map_or_else(|| "FBA".to_string(), |v| js::to_string(&v));
        if method == "GIMME" {
            if self.omics.dataset.is_none() {
                return Err(StoreError::msg("The bundle declares GIMME but lacks the omics dataset needed to reproduce it."));
            }
            self.omics.gimme_enabled = false;
            let gimme = get(analysis, "gimme");
            let at = |k: &str| gimme.as_ref().and_then(|g| g.get(k)).filter(|v| !v.is_null()).cloned();
            if let Some(t) = at("threshold") {
                self.omics.gimme_threshold = Some(js::to_number(&t));
            }
            if let Some(f) = at("objectiveFraction") {
                self.omics.gimme_objective_fraction = js::to_number(&f);
            }
            if let Some(c) = at("condition") {
                self.omics.condition_a = Some(js::to_string(&c));
            }
            self.run_community_gimme(CommunityOmicsPatch::default())?;
        } else {
            self.omics.gimme_enabled = false;
            self.solve_with(Some(method == "pFBA"), true);
        }
        let tolerances = get(bundle.get("replay"), "tolerances")
            .map(|t| Tolerances {
                absolute: t.get("absolute").and_then(Value::as_f64).unwrap_or(1e-7),
                relative: t.get("relative").and_then(Value::as_f64).unwrap_or(1e-6),
            })
            .unwrap_or_default();
        let current = self.solution.as_ref().map_or(Value::Null, |s| s.replay_json());
        let replay = compare_replayed_solution(bundle.get("solution").unwrap_or(&Value::Null), &current, tolerances);
        let fva = get(bundle.get("solution"), "fva");
        if get(analysis, "fvaEnabled").is_some_and(|v| js::truthy(&v)) && fva.as_ref().is_some_and(js::truthy) {
            self.fva_enabled = true;
            self.fva_ranges = fva
                .as_ref()
                .and_then(Value::as_object)
                .map(|m| {
                    m.iter()
                        .map(|(k, r)| {
                            let n = |x: &str| r.get(x).map_or(f64::NAN, js::to_number);
                            (k.clone(), FvaRange { min: n("min"), max: n("max") })
                        })
                        .collect()
                })
                .unwrap_or_default();
            let n = self.fva_ranges.len();
            self.fva_progress = FvaProgress { running: false, done: n, total: n, ms: 0.0 };
        } else {
            self.fva_enabled = false;
            self.fva_ranges = IndexMap::new();
            self.fva_progress = FvaProgress::default();
        }
        let mut check_json = serde_json::to_value(&check)?;
        if let Some(o) = check_json.as_object_mut() {
            o.insert("replay".into(), replay.to_json());
        }
        self.reproducibility_replay = Some(json!({
            "integrity": check_json,
            "replay": replay.to_json(),
            "archivedAt": get(bundle.get("metadata"), "createdAt"),
            "restored": {
                "map": bundle.get("map").is_some_and(js::truthy) && self.map.is_some(),
                "omicsExpected": archived_omics.is_some(),
                "omics": archived_omics.is_some() && self.omics.dataset.is_some(),
            },
        }));
        let verdict = if replay.pass { "numerical replay OK" } else { "WARNING: numerical differences" };
        self.settle_opened_work();
        let hash: String = bundle
            .get("integrity")
            .and_then(|i| i.get("payloadHash"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .chars()
            .take(12)
            .collect();
        self.set_status_message(
            if replay.pass { StatusState::Ok } else { StatusState::Error },
            format!("Bundle restored · {verdict} · max Δflux {} · SHA-256 {hash}…", to_exponential(replay.fluxes.max_abs_error, 2)),
        );
        self.draw_waiting_map();
        self.emit("rebuild");
        Ok(check_json)
    }

    /// Parse a map document from a value (`parseMapDocument(object)`).
    pub fn parse_map_document(doc: &Value) -> Result<ParsedMapDocument> {
        Ok(parse_map_document(doc)?)
    }
}
