//! Files in and out: map documents, scenario state, models, flux tables and
//! reproducibility bundles (the "map I/O" and "exports" sections of
//! `store.js`).

use std::collections::HashSet;
use std::sync::Arc;

use coxeter_analysis::create_zuniga_scenario;

use coxeter_core::js;
use coxeter_core::community::{CommunityModel, CommunitySpec, MediumLimit, Objective, SpeciesSpec};
use coxeter_core::model::Model;
use coxeter_io::model_import::parse_model_json;
use coxeter_io::reproducibility::{
    OmicsDataset as ReproDataset, OmicsState as ReproOmics, ReproStore, SolutionState, Tolerances, compare_archived_solution, create_reproducibility_bundle,
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
use coxeter_render::theme::{FluxColorStop, normalize_absolute_flux_colors_value, normalize_flux_color_stops_value};
use coxeter_solver::steadycom::Fiber;
use indexmap::IndexMap;
use serde_json::{Map, Value, json};

use crate::error::{Result, StoreError};
use crate::pathways::flux_color_stops_json;
use crate::store::{Store, objectives};
use crate::types::*;
use crate::util::{clamp_number, normalize_pfba_percent, num_json, to_exponential};
use crate::work::{layout_settings_json, raw_layout_settings};
use crate::omics::{CommunityOmics, CommunityOmicsPatch, community_gimme_inputs, dataset_from_archive};
use crate::zuniga::{ZUNIGA_ID, zuniga_community_model};

/// How long an open waits for the Zuñiga sources (a first download is a few
/// tens of megabytes; later opens read the cache).
pub const ZUNIGA_LOAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1800);

struct PreparedBundle {
    species: Vec<Species>,
    models: IndexMap<String, Arc<Model>>,
    map: Option<EditableMap>,
}

/// Read every embedded model and drawing before replacing the open work.
fn prepare_bundle_models(bundle: &Value) -> Result<PreparedBundle> {
    let Some(raw_species) = bundle.get("community").and_then(|c| c.get("species")).and_then(Value::as_array).filter(|s| !s.is_empty()) else {
        return Err(StoreError::msg("The bundle contains no reproducible species."));
    };
    let mut species_docs = Vec::new();
    for raw in raw_species {
        let mut doc = raw.as_object().cloned().unwrap_or_default();
        let id = raw.get("id").and_then(Value::as_str).unwrap_or("");
        doc.insert("file".into(), json!(format!("repro-{id}")));
        species_docs.push(Value::Object(doc));
    }
    let medium = bundle.get("constraints").and_then(|c| c.get("medium")).cloned().unwrap_or(Value::Null);
    let bounds = bundle.get("constraints").and_then(|c| c.get("reactionOverrides")).filter(|v| !v.is_null());
    let mut overrides = Vec::new();
    if let Some(bounds) = bounds {
        let Some(bounds) = bounds.as_object() else {
            return Err(StoreError::msg("The bundle reaction bounds must be an object."));
        };
        for (id, value) in bounds {
            let mut doc = value.as_object().cloned().unwrap_or_default();
            doc.insert("id".into(), json!(id));
            overrides.push(Value::Object(doc));
        }
    }
    validate_scenario_state(&json!({"species": species_docs, "medium": medium, "overrides": overrides}))?;
    let species: Vec<_> = species_docs.iter().map(Species::from_json).collect();
    if !species.iter().any(|sp| sp.enabled) {
        return Err(StoreError::msg("The bundle must enable at least one species."));
    }
    let mut models = IndexMap::new();
    let mut entries = Vec::new();
    for sp in &species {
        let Some(raw) = bundle.get("models").and_then(|m| m.get(&sp.id)).filter(|m| !m.is_null()) else {
            return Err(StoreError::msg(format!("The bundle model for \"{}\" is missing.", sp.id)));
        };
        let model = Arc::new(parse_model_json(&raw.to_string(), &format!("Bundle model \"{}\"", sp.id))?);
        models.insert(sp.file.clone(), model.clone());
        entries.push(SpeciesSpec {
            id: Some(sp.id.clone()), model, abundance: Some(sp.abundance), enabled: Some(sp.enabled), color: None,
        });
    }
    let mut medium_spec = IndexMap::new();
    for (id, limit) in medium.as_object().expect("the medium was validated") {
        let limit = limit.as_f64().expect("the medium limit was validated");
        medium_spec.insert(id.clone(), MediumLimit { lower: Some(-limit), upper: Some(1000.0) });
    }
    let community = CommunityModel::new(CommunitySpec {
        name: None, species: entries, medium: medium_spec,
        objective: Objective::parse(bundle["community"]["objective"].as_str().unwrap_or(objectives::EQUAL_GROWTH)),
        product_target: bundle["community"]["productTarget"].as_str().map(str::to_string),
    })?;
    if bundle["analysis"]["method"] == "GIMME" {
        let mut omics = CommunityOmics::default();
        omics.dataset = Some(Arc::new(dataset_from_archive(&bundle["omics"])));
        omics.assign(CommunityOmicsPatch::from_json(&bundle["omics"]["settings"]));
        let gimme = &bundle["analysis"]["gimme"];
        if !gimme.is_null() && !gimme.is_object() {
            return Err(StoreError::msg("The bundle GIMME settings must be an object."));
        }
        if let Some(condition) = gimme.get("condition").filter(|v| !v.is_null()) {
            let Some(condition) = condition.as_str() else {
                return Err(StoreError::msg("Choose a GIMME condition present in the expression data."));
            };
            omics.condition_a = Some(condition.to_string());
        }
        if let Some(threshold) = gimme.get("threshold").filter(|v| !v.is_null()) {
            let Some(threshold) = threshold.as_f64() else {
                return Err(StoreError::msg("The GIMME threshold must be a finite number or blank for automatic."));
            };
            if !threshold.is_finite() {
                return Err(StoreError::msg("The GIMME threshold must be a finite number or blank for automatic."));
            }
            if bundle["omics"]["settings"].get("gimmeThreshold").is_none() {
                omics.gimme_threshold = Some(threshold);
            }
        }
        if let Some(fraction) = gimme.get("objectiveFraction").filter(|v| !v.is_null()) {
            omics.gimme_objective_fraction = fraction.as_f64().unwrap_or(f64::NAN);
        }
        let omics_species: Vec<_> = species.iter().map(|sp| coxeter_analysis::OmicsSpecies::new(&sp.id, &sp.file)).collect();
        community_gimme_inputs(&community, &omics_species, &models, &omics)?;
    }
    let map = match bundle.get("map").filter(|m| !m.is_null()) {
        Some(map) => {
            validate_map_document(map, &community, &species)?;
            Some(EditableMap::from_json(map)?)
        }
        None => None,
    };
    Ok(PreparedBundle { species, models, map })
}

/// Validate scientific settings that must not fall back to a different analysis.
fn validate_bundle_analysis(bundle: &Value) -> Result<()> {
    let analysis = bundle.get("analysis").filter(|a| !a.is_null());
    if analysis.is_some_and(|a| !a.is_object()) {
        return Err(StoreError::msg("The bundle analysis must be an object."));
    }
    let method = analysis.and_then(|a| a.get("method")).filter(|m| !m.is_null()).map(js::to_string).unwrap_or_else(|| "FBA".into());
    let ordinary = ["FBA", "pFBA", "pFVA", "loopless FBA", "CycleFreeFlux", "GIMME"].contains(&method.as_str());
    let study = method == "Stationary source-model FBA" && bundle["study"]["id"].as_str() == Some(ZUNIGA_ID);
    if !ordinary && !study {
        return Err(StoreError::msg(format!("The bundle analysis method \"{method}\" is not supported.")));
    }
    for key in ["parsimonious", "loopless", "cycleFree", "fvaEnabled", "pfvaEnabled", "pfbaExactObjective"] {
        if let Some(value) = analysis.and_then(|a| a.get(key)) {
            if !value.is_boolean() {
                return Err(StoreError::msg(format!("The bundle analysis setting \"{key}\" must be boolean.")));
            }
        }
    }
    if let Some(percent) = analysis.and_then(|a| a.get("pfbaBiomassPercent")) {
        let percent = percent.as_f64().unwrap_or(f64::NAN);
        if !percent.is_finite() || !(0.0..=100.0).contains(&percent) {
            return Err(StoreError::msg("The bundle pFBA biomass percentage must be between 0 and 100."));
        }
    }
    if method == "GIMME" && bundle.get("omics").is_none_or(Value::is_null) {
        return Err(StoreError::msg("The bundle declares GIMME but lacks the omics dataset needed to reproduce it."));
    }
    if let Some(omics) = bundle.get("omics").filter(|o| !o.is_null()) {
        let values = omics.get("values").and_then(Value::as_object);
        if !omics.is_object() || !omics.get("conditions").is_some_and(Value::is_array) || values.is_none() {
            return Err(StoreError::msg("The bundle omics must contain conditions and a values object."));
        }
        for row in values.expect("the omics values object was validated").values() {
            let Some(values) = row.get("values").and_then(Value::as_object) else {
                return Err(StoreError::msg("The bundle omics rows must contain a values object."));
            };
            for value in values.values() {
                if !value.as_f64().is_some_and(f64::is_finite) {
                    return Err(StoreError::msg("The bundle omics values must be finite numbers."));
                }
            }
        }
    }
    for key in ["absolute", "relative"] {
        if let Some(value) = bundle.get("replay").and_then(|r| r.get("tolerances")).and_then(|t| t.get(key)) {
            let value = value.as_f64().unwrap_or(f64::NAN);
            if !value.is_finite() || value < 0.0 {
                return Err(StoreError::msg("The bundle replay tolerances must be finite nonnegative numbers."));
            }
        }
    }
    Ok(())
}

/// Normalize saved drawing settings before any bundle state is installed.
fn normalize_bundle_presentation(raw: Option<&Value>) -> Result<Option<Value>> {
    let Some(raw) = raw.filter(|v| !v.is_null()) else { return Ok(None) };
    let Some(mut state) = raw.as_object().cloned() else {
        return Err(StoreError::msg("The bundle presentation must be an object."));
    };
    let colors = normalize_flux_color_stops_value(raw.get("fluxColorStops").unwrap_or(&Value::Null))?;
    state.insert("fluxColorStops".into(), flux_color_stops_json(colors.as_deref()));
    let (absolute, log) = normalize_absolute_flux_colors_value(raw.get("absoluteFluxColors").unwrap_or(&Value::Null))?;
    state.insert("absoluteFluxColors".into(), json!({"stops": flux_color_stops_json(absolute.as_deref()), "log": log}));
    if let Some(layout) = raw.get("layoutSettings").filter(|v| !v.is_null()) {
        let layout = coxeter_map::layout_settings::normalize_layout_settings(&raw_layout_settings(layout));
        state.insert("layoutSettings".into(), layout_settings_json(&layout));
    }
    if let Some(mode) = raw.get("mapMode") {
        if !mode.as_str().is_some_and(|m| ["full", "empty", "core-overview", "published-case"].contains(&m)) {
            return Err(StoreError::msg("The bundle presentation declares an invalid drawing mode."));
        }
    }
    for key in ["widthByFlux", "dashNoFlux", "lineHops", "showCurrency", "showCrossFeeding"] {
        if let Some(value) = raw.get(key) {
            if !value.is_boolean() {
                return Err(StoreError::msg(format!("The bundle presentation setting \"{key}\" must be boolean.")));
            }
        }
    }
    for key in ["crossFeedingWidth", "pathwayRibbonWidth", "pathwayRibbonOpacity"] {
        if let Some(value) = raw.get(key) {
            if !value.as_f64().is_some_and(f64::is_finite) {
                return Err(StoreError::msg(format!("The bundle presentation setting \"{key}\" must be a finite number.")));
            }
        }
    }
    if let Some(manager) = raw.get("pathwayManager").filter(|v| !v.is_null()) {
        if !manager.get("manual").is_some_and(Value::is_array) || !manager.get("settings").is_some_and(Value::is_array) {
            return Err(StoreError::msg("The bundle pathway presentation must contain manual and settings lists."));
        }
        for pathway in manager["manual"].as_array().expect("the manual pathway list was validated") {
            let Some(ids) = pathway.get("reactionIds").and_then(Value::as_array) else {
                return Err(StoreError::msg("The bundle pathway presentation must use reaction ID lists."));
            };
            for id in ids {
                if !id.is_string() {
                    return Err(StoreError::msg("The bundle pathway reaction IDs must be strings."));
                }
            }
        }
    }
    if let Some(dataset) = raw.get("mapOmics").and_then(|o| o.get("dataset")).filter(|v| !v.is_null()) {
        if !dataset.get("conditions").is_some_and(Value::is_array) || !dataset.get("rows").is_some_and(Value::is_array) {
            return Err(StoreError::msg("The bundle map omics must contain conditions and rows."));
        }
    }
    Ok(Some(Value::Object(state)))
}

/// Validate the biological settings before a Scenario file replaces any open work.
fn validate_scenario_state(doc: &Value) -> Result<()> {
    let species = doc.get("species").and_then(Value::as_array);
    if !doc.is_object() || species.is_none_or(Vec::is_empty) {
        return Err(StoreError::msg("Open a Scenario file containing species and medium settings."));
    }
    let Some(medium) = doc.get("medium").and_then(Value::as_object) else {
        return Err(StoreError::msg("The scenario medium must contain numeric uptake limits."));
    };
    for (id, limit) in medium {
        let limit = limit.as_f64().unwrap_or(f64::NAN);
        if !limit.is_finite() || limit < 0.0 {
            return Err(StoreError::msg(format!("The scenario medium limit for \"{id}\" must be a finite nonnegative number.")));
        }
    }
    let mut species_ids = HashSet::new();
    for species in species.expect("the scenario species list was validated") {
        let id = species.get("id").and_then(Value::as_str).unwrap_or("");
        let file = species.get("file").and_then(Value::as_str).unwrap_or("");
        if id.trim().is_empty() || file.trim().is_empty() {
            return Err(StoreError::msg("Each scenario species needs an ID and a model file."));
        }
        if !species_ids.insert(id) {
            return Err(StoreError::msg(format!("The scenario repeats species \"{id}\".")));
        }
        if let Some(abundance) = species.get("abundance") {
            let abundance = abundance.as_f64().unwrap_or(f64::NAN);
            if !abundance.is_finite() || abundance < 0.0 {
                return Err(StoreError::msg(format!("The abundance of scenario species \"{id}\" must be a finite nonnegative number.")));
            }
        }
        if let Some(enabled) = species.get("enabled") {
            if !enabled.is_boolean() {
                return Err(StoreError::msg(format!("The enabled setting of scenario species \"{id}\" must be true or false.")));
            }
        }
    }
    if let Some(overrides) = doc.get("overrides") {
        let Some(overrides) = overrides.as_array() else {
            return Err(StoreError::msg("The scenario reaction overrides must be a list of bounds."));
        };
        let mut reaction_ids = HashSet::new();
        for bounds in overrides {
            let id = bounds.get("id").and_then(Value::as_str).unwrap_or("");
            let lower = bounds.get("lower").and_then(Value::as_f64).unwrap_or(f64::NAN);
            let upper = bounds.get("upper").and_then(Value::as_f64).unwrap_or(f64::NAN);
            if id.trim().is_empty() || !lower.is_finite() || !upper.is_finite() || lower > upper {
                return Err(StoreError::msg("Each scenario reaction override needs an ID and finite bounds with lower ≤ upper."));
            }
            if !reaction_ids.insert(id) {
                return Err(StoreError::msg(format!("The scenario repeats the reaction override for \"{id}\".")));
            }
        }
    }
    Ok(())
}

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
            absolute_flux_colors: Some(self.absolute_flux_colors_json()),
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
        let provenance = self.current_solution_provenance();
        let solved = provenance.as_ref();
        let objective_value = self.solution.as_ref().map(|s| s.objective_value).filter(|v| v.is_finite());
        json!({
            "generator": "Coxeter Multicell Flux Visualizer",
            "software": coxeter_io::reproducibility::software_identity().to_json(),
            "exportedAt": coxeter_map::map_document::iso_timestamp_now(),
            "scenario": solved.and_then(|s| s.get("scenario")).cloned().unwrap_or_else(|| json!({"id": self.scenario_id(), "name": self.scenario_str("name")})),
            "species": solved.and_then(|s| s.get("species")).cloned().unwrap_or_else(|| json!(self.species.iter().filter(|sp| sp.enabled).map(|sp| json!({
                "id": sp.id,
                "model": self.models.get(&sp.file).map(|m| m.id.clone()).filter(|id| !id.is_empty()).unwrap_or_else(|| sp.file.clone()),
                "abundance": num_json(sp.abundance),
            })).collect::<Vec<_>>())),
            "analysis": {
                "method": solved.and_then(|s| s.get("method")),
                "solver": solved.and_then(|s| s.get("solver")),
                "status": self.solution.as_ref().map(|s| s.status.as_str()),
                "objective": solved.and_then(|s| s.get("objective")).cloned().unwrap_or_else(|| json!(self.objective)),
                "productTarget": solved.and_then(|s| s.get("productTarget")).cloned().unwrap_or_else(|| json!(self.product_target)),
                "objectiveSense": solved.and_then(|s| s.get("objectiveSense")).cloned().unwrap_or_else(|| json!(if self.objective_sense() == ObjectiveSense::Min { "min" } else { "max" })),
                "customObjective": solved.and_then(|s| s.get("customObjective")).cloned().unwrap_or_else(|| self.custom_objective.as_ref().map_or(Value::Null, CustomObjective::to_json)),
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

    /// Provenance of the installed result, independent of a replacement calculation's settings.
    pub fn current_solution_provenance(&self) -> Option<Value> {
        self.solution.as_ref()?;
        let mut state = self.solution_provenance.clone()?;
        if state["method"] == "pFBA" && self.pfva_enabled && self.has_complete_fva_ranges() {
            state["method"] = json!("pFVA");
        }
        Some(state)
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
        if parsed.study.as_ref().and_then(|study| study.get("id")).and_then(Value::as_str) == Some(ZUNIGA_ID) {
            self.validate_zuniga_document(parsed.study.as_ref().expect("the study ID was checked"))?;
        }
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
        let absolute_flux_colors = normalize_absolute_flux_colors_value(&parsed.absolute_flux_colors)?;
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
        let before_presentation = self.map_presentation_state();
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
        self.map = Some(next);
        if !self.is_zuniga_study() {
            self.map_mode = "full".into();
        }
        self.view.flux_color_stops = flux_color_stops;
        (self.view.absolute_flux_color_stops, self.view.absolute_flux_log) = absolute_flux_colors;
        self.refresh_flux_scale();
        self.restore_pathway_manager_state(parsed.pathway_manager.as_ref());
        self.record_map_presentation(&before_presentation, "Load map JSON", Some(&before));
        // What was just opened is what is in the file, pathways and colours
        // included. Undo still goes back to the previous drawing, which then
        // counts as unsaved.
        self.mark_map_saved();
        self.selected_reaction = None;
        self.selected_geometry = None;
        self.selected_entity = None;
        self.multi_selection = coxeter_map::multi_selection::empty_selection();
        self.variability = None;
        self.emit("map");
        Ok(())
    }

    // Scenario state

    /// `stateJSON()`: the Scenario file.
    pub fn state_json(&self) -> String {
        js::stringify(&self.state_value(), Some(2))
    }

    /// The Scenario file as a value; `geneKnockouts` only when there are any.
    pub fn state_value(&self) -> Value {
        let medium: Map<String, Value> = self.medium.iter().map(|(k, v)| (k.clone(), num_json(*v))).collect();
        let overrides: Vec<Value> = self
            .overrides
            .iter()
            .map(|(id, b)| json!({"id": id, "lower": num_json(b.lower), "upper": num_json(b.upper)}))
            .collect();
        let mut state = json!({
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
            "cycleFree": self.cycle_free,
            "fvaEnabled": self.fva_enabled,
            "pfvaEnabled": self.pfva_enabled,
            "layoutSettings": layout_settings_json(&self.layout_settings),
            "reactionColorMode": self.view.reaction_color_mode.as_str(),
            "widthByFlux": self.view.width_by_flux,
            "dashNoFlux": self.view.dash_no_flux,
            "lineHops": self.view.line_hops,
            "crossFeedingWidth": num_json(self.view.cross_feeding_width),
            "fluxColorStops": flux_color_stops_json(self.view.flux_color_stops.as_deref()),
            "absoluteFluxColors": self.absolute_flux_colors_json(),
            "pathwayRibbonMode": self.view.pathway_ribbon_mode.as_str(),
            "pathwayRibbonWidth": num_json(self.view.pathway_ribbon_width),
            "pathwayRibbonOpacity": num_json(self.view.pathway_ribbon_opacity),
            "pathwayManager": self.pathway_manager_state(),
            "omics": self.omics_state(),
            "medium": medium,
            "species": self.species.iter().map(Species::to_json).collect::<Vec<_>>(),
            "overrides": overrides,
        });
        if let (false, Some(doc)) = (self.gene_knockouts.is_empty(), state.as_object_mut()) {
            doc.insert("geneKnockouts".into(), json!(self.sorted_gene_knockouts()));
        }
        state
    }

    /// `loadState(text)`: open a Scenario file. A fixed-µ SteadyCom result is
    /// recomputed in the background (see `poll_jobs`).
    pub fn load_state(&mut self, text: &str) -> Result<()> {
        let doc: Value = serde_json::from_str(text)?;
        validate_scenario_state(&doc)?;
        let flux_color_stops = normalize_flux_color_stops_value(doc.get("fluxColorStops").unwrap_or(&Value::Null))?;
        let absolute_flux_colors = doc.get("absoluteFluxColors").cloned().unwrap_or(Value::Null);
        normalize_absolute_flux_colors_value(&absolute_flux_colors)?;
        let study_id = doc.get("study").and_then(|s| s.get("id")).and_then(Value::as_str);
        if study_id == Some(ZUNIGA_ID) || doc.get("scenario").and_then(Value::as_str) == Some(&format!("published:{ZUNIGA_ID}")) {
            let study = doc.get("study").filter(|s| js::truthy(s));
            let scenario = study
                .and_then(|s| s.get("scenarioId"))
                .filter(|v| !v.is_null())
                .map_or_else(|| "mutant-coculture".to_string(), js::to_string);
            if let Some(study) = study {
                self.validate_zuniga_document(study)?;
            }
            if !self.is_zuniga_study() {
                self.use_zuniga_study_blocking(Some(&scenario), ZUNIGA_LOAD_TIMEOUT)?;
            }
            if let Some(study) = study {
                self.validate_zuniga_state(study)?;
            }
            self.work_generation += 1;
            self.solution_cleared = false;
            self.use_zuniga_scenario(&scenario, false)?;
            self.view.flux_color_stops = flux_color_stops;
            self.restore_absolute_flux_colors(&absolute_flux_colors)?;
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
        for species in doc["species"].as_array().expect("the scenario species list was validated") {
            let file = species["file"].as_str().expect("the scenario model file was validated");
            self.load_model(file)?;
        }
        self.work_generation += 1;
        self.reset_zuniga_study();
        self.reset_community_gimme();
        self.solution_cleared = false;
        self.invalidate_steady_com();
        let scenario_id = doc.get("scenario").and_then(Value::as_str).filter(|id| !id.trim().is_empty()).unwrap_or("imported-scenario");
        if let Some(id) = scenario_id.strip_prefix("published:") {
            self.use_published_community(id)?;
        } else {
            self.published_community_id = None;
            self.scenario = Some(self.scenarios.iter().find(|scenario| scenario.get("id").and_then(Value::as_str) == Some(scenario_id))
                .cloned().unwrap_or_else(|| json!({
                    "id": scenario_id,
                    "name": "Imported scenario",
                    "medium": doc["medium"].clone(),
                })));
        }
        let parsimonious = doc.get("steadyComConfig").and_then(|c| c.get("parsimonious")).is_some_and(js::truthy);
        self.steady_com_config = SteadyComConfig { fiber, parsimonious };
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
        self.cycle_free = !self.loopless && get("cycleFree").is_some_and(js::truthy);
        if self.loopless || self.cycle_free {
            self.parsimonious = false;
        }
        self.fva_enabled = !self.loopless && !self.cycle_free && get("fvaEnabled").or_else(|| get("fva")).is_some_and(js::truthy);
        self.pfva_enabled = self.fva_enabled && get("pfvaEnabled").is_some_and(js::truthy);
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
        if let Some(w) = get("crossFeedingWidth") {
            let value = if w.is_null() || w.as_str() == Some("") { f64::NAN } else { js::to_number(w) };
            self.view.cross_feeding_width = crate::view::cross_feeding_width_value(value, self.view.cross_feeding_width);
        }
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
        // After the rebuild: a scenario's curated map, drawn again, brings its
        // own palettes, and the ones in the file are what was saved.
        self.view.flux_color_stops = flux_color_stops;
        self.restore_absolute_flux_colors(&absolute_flux_colors)?;
        self.refresh_flux_scale();
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
        self.gene_knockouts = self.gene_knockouts_from(doc.get("geneKnockouts"));
        self.custom_objective = self.custom_objective_from(doc.get("customObjective"));
        self.apply_overrides();
        // Before solving, so the repaint that follows already reads "Saved".
        self.mark_scenario_saved();
        match steady_mu {
            Some(mu) => {
                self.restore_steady_com_at(mu)?;
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
        let provenance = self.current_solution_provenance();
        let mut pfba_biomass_percent = num_json(self.pfba_biomass_percent);
        let mut pfba_exact_objective = !self.pfva_enabled;
        if let Some(state) = &provenance {
            if let Some(percent) = state.get("pfbaBiomassPercent") {
                pfba_biomass_percent = percent.clone();
            }
            if let Some(exact) = state.get("pfbaExactObjective").and_then(Value::as_bool) {
                pfba_exact_objective = exact;
            }
        }
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
                solver: Some(s.solver.clone()),
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
            gene_knockouts: self.sorted_gene_knockouts(),
            parsimonious: self.parsimonious,
            pfba_biomass_percent,
            pfba_exact_objective: Some(pfba_exact_objective),
            loopless: self.loopless,
            cycle_free: self.cycle_free,
            fva_enabled: self.fva_enabled && (self.solution.is_none() || self.has_complete_fva_ranges()),
            pfva_enabled: self.pfva_enabled,
            fva_ranges: self.fva_ranges.iter().map(|(k, r)| (k.clone(), json!({"min": num_json(r.min), "max": num_json(r.max)}))).collect(),
            growth: self.growth.iter().map(|(k, v)| (k.clone(), num_json(*v))).collect(),
            map: self.map.as_ref().map(EditableMap::to_json),
            presentation: Some(self.reproducibility_presentation_state()),
            omics: Some(omics),
            solution,
            zuniga: self.zuniga_bundle_state(),
            analysis_method: provenance.as_ref().and_then(|p| p.get("method")).and_then(Value::as_str).map(str::to_string),
            solver: provenance.as_ref().and_then(|p| p.get("solver")).and_then(Value::as_str).map(str::to_string)
                .or_else(|| Some(self.solution_solver_label().unwrap_or_else(|| self.solver.label()))),
        }
    }

    /// `reproducibilityBundle()`.
    pub fn reproducibility_bundle(&self) -> Result<Value> {
        if self.analysis_or_context_running() || self.status().state == StatusState::Busy || self.last_run.as_ref().is_some_and(|run| run.phase == RunPhase::Running)
            || self.fva_progress.running || self.community_gimme_running() {
            return Err(StoreError::msg("Wait for the current analysis to finish before exporting a reproducibility bundle."));
        }
        if self.solution.as_ref().is_some_and(|s| s.steady_com.is_some()) {
            return Err(StoreError::msg("Reproducibility bundles cover ordinary FBA only; save a SteadyCom solution as Scenario JSON."));
        }
        for species in &self.species {
            if self.models.get(&species.file).is_none() {
                return Err(StoreError::msg(format!("The bundle model for \"{}\" is not loaded.", species.id)));
            }
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

    /// The saved drawing settings and map overlay, separate from biological inputs.
    pub fn reproducibility_presentation_state(&self) -> Value {
        let mut state = self.map_presentation_state();
        let doc = state.as_object_mut().expect("drawing presentation is an object");
        doc.insert("layoutSettings".into(), layout_settings_json(&self.layout_settings));
        doc.insert("widthByFlux".into(), json!(self.view.width_by_flux));
        doc.insert("dashNoFlux".into(), json!(self.view.dash_no_flux));
        doc.insert("lineHops".into(), json!(self.view.line_hops));
        doc.insert("crossFeedingWidth".into(), num_json(self.view.cross_feeding_width));
        doc.insert("pathwayRibbonWidth".into(), num_json(self.view.pathway_ribbon_width));
        doc.insert("pathwayRibbonOpacity".into(), num_json(self.view.pathway_ribbon_opacity));
        doc.insert("mapOmics".into(), self.omics_state());
        state
    }

    pub(crate) fn restore_reproducibility_presentation_state(&mut self, state: Option<&Value>) {
        self.manual_pathways.clear();
        self.pathway_settings.clear();
        self.clear_omics(false);
        let Some(state) = state else { return };
        self.restore_map_presentation_state(state);
        if let Some(layout) = state.get("layoutSettings") {
            self.layout_settings = coxeter_map::layout_settings::normalize_layout_settings(&raw_layout_settings(layout));
        }
        if let Some(value) = state.get("widthByFlux").and_then(Value::as_bool) {
            self.view.width_by_flux = value;
        }
        if let Some(value) = state.get("dashNoFlux").and_then(Value::as_bool) {
            self.view.dash_no_flux = value;
        }
        if let Some(value) = state.get("lineHops").and_then(Value::as_bool) {
            self.view.line_hops = value;
        }
        if let Some(value) = state.get("crossFeedingWidth").and_then(Value::as_f64) {
            self.view.cross_feeding_width = value;
        }
        if let Some(value) = state.get("pathwayRibbonWidth").and_then(Value::as_f64) {
            self.view.pathway_ribbon_width = value;
        }
        if let Some(value) = state.get("pathwayRibbonOpacity").and_then(Value::as_f64) {
            self.view.pathway_ribbon_opacity = value;
        }
        self.restore_omics_state(state.get("mapOmics"));
    }

    /// `importReproducibilityBundle(bundle)`: restore an archived analysis
    /// and replay it. Returns the integrity check with the replay comparison.
    pub fn import_reproducibility_bundle(&mut self, bundle: &Value) -> Result<Value> {
        let check = verify_reproducibility_bundle(bundle)?;
        if !check.valid {
            return Err(StoreError::msg("The bundle fails SHA-256 verification; it may have been modified or be corrupt."));
        }
        let presentation = normalize_bundle_presentation(bundle.get("presentation"))?;
        validate_bundle_analysis(bundle)?;
        if bundle.get("study").and_then(|s| s.get("id")).and_then(Value::as_str) == Some(ZUNIGA_ID) {
            let result = self.import_zuniga_bundle(bundle)?;
            self.restore_reproducibility_presentation_state(presentation.as_ref());
            self.settle_opened_work();
            self.emit("map");
            return Ok(result);
        }
        let prepared = prepare_bundle_models(bundle)?;
        self.solution_cleared = false;
        self.reset_zuniga_study();
        self.reset_community_gimme();
        for key in std::mem::take(&mut self.uploaded_model_keys) {
            self.models.remove(&key);
        }
        let community = bundle.get("community");
        for (key, model) in prepared.models {
            self.models.insert_arc(key.clone(), model);
            self.uploaded_model_keys.insert(key.clone());
        }
        let species = prepared.species;
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
        self.cycle_free = !self.loopless && get(analysis, "cycleFree").is_some_and(|v| js::truthy(&v));
        // FVA is replayed only after the primary numerical solution has been
        // compared; its ranges are restored from the archive below.
        self.fva_enabled = false;
        self.pfva_enabled = false;
        self.rebuild(RebuildOptions {
            relayout: true,
            contents: if prepared.map.is_some() { coxeter_map::Contents::None } else { coxeter_map::Contents::All },
            solve: false,
        })?;
        self.gene_knockouts = self.gene_knockouts_from(constraints.and_then(|c| c.get("geneKnockouts")));
        self.custom_objective = self.custom_objective_from(community.and_then(|c| c.get("customObjective")));
        self.apply_overrides();
        // Restore the exact archived drawing after the biological model exists.
        if let Some(map) = prepared.map {
            self.map = Some(map);
            self.map_history.clear();
        }
        self.restore_reproducibility_presentation_state(presentation.as_ref());
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
            if archived_omics.as_ref().and_then(|o| o.get("settings")).and_then(|s| s.get("gimmeThreshold")).is_none() {
                self.omics.gimme_threshold = at("threshold").map(|value| js::to_number(&value));
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
            self.pfva_enabled = method == "pFVA" || (method == "pFBA" && get(analysis, "pfbaExactObjective") == Some(json!(false)));
            self.solve_with(Some(method == "pFBA" || method == "pFVA"), true);
        }
        let tolerances = get(bundle.get("replay"), "tolerances")
            .map(|t| Tolerances {
                absolute: t.get("absolute").and_then(Value::as_f64).unwrap_or(1e-7),
                relative: t.get("relative").and_then(Value::as_f64).unwrap_or(1e-6),
            })
            .unwrap_or_default();
        let current = self.solution.as_ref().map_or(Value::Null, |s| s.replay_json());
        let replay = compare_archived_solution(bundle.get("solution").unwrap_or(&Value::Null), &current, tolerances);
        let replay_json = replay.as_ref().map_or(Value::Null, |comparison| comparison.to_json());
        // Another solver finds the same optimum, but where the optimum is not
        // unique its fluxes may differ: the replay says which solvers it compares.
        let archived_solver = get(analysis, "solver").map(|v| js::to_string(&v));
        let replay_solver = self.solution_solver_label().unwrap_or_else(|| self.solver.label());
        let other_solver = archived_solver.as_deref().filter(|archived| *archived != replay_solver);
        let fva = get(bundle.get("solution"), "fva");
        if get(analysis, "fvaEnabled").is_some_and(|v| js::truthy(&v)) && fva.as_ref().is_some_and(js::truthy) {
            self.fva_enabled = true;
            self.pfva_enabled = get(analysis, "pfvaEnabled").is_some_and(|v| js::truthy(&v));
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
        } else if bundle.get("solution").is_none_or(Value::is_null) && get(analysis, "fvaEnabled").is_some_and(|v| js::truthy(&v)) {
            self.pfva_enabled = get(analysis, "pfvaEnabled").is_some_and(|v| js::truthy(&v));
            self.set_fva(true)?;
        } else {
            self.fva_enabled = false;
            self.pfva_enabled = false;
            self.fva_ranges = IndexMap::new();
            self.fva_progress = FvaProgress::default();
        }
        let mut check_json = serde_json::to_value(&check)?;
        if let Some(o) = check_json.as_object_mut() {
            o.insert("replay".into(), replay_json.clone());
        }
        self.reproducibility_replay = Some(json!({
            "integrity": check_json,
            "replay": replay_json,
            "archivedAt": get(bundle.get("metadata"), "createdAt"),
            "restored": {
                "map": bundle.get("map").is_some_and(js::truthy) && self.map.is_some(),
                "omicsExpected": archived_omics.is_some(),
                "omics": archived_omics.is_some() && self.omics.dataset.is_some(),
            },
            "solver": {
                "archived": archived_solver,
                "replayed": replay_solver,
                "same": replay.as_ref().map(|_| other_solver.is_none()),
            },
        }));
        let mut state = StatusState::Ok;
        let mut verdict = "no archived numerical result to compare".to_string();
        let mut detail = String::new();
        if let Some(comparison) = &replay {
            if !comparison.pass {
                state = StatusState::Error;
                verdict = match other_solver {
                    Some(archived) => format!("WARNING: numerical differences (solved with {archived}, replayed with {replay_solver}; alternative optima can differ)"),
                    None => "WARNING: numerical differences".to_string(),
                };
            } else if !self.solution.as_ref().is_some_and(|s| s.is_optimal()) {
                state = StatusState::Warn;
                let status = self.solution.as_ref().map_or("not run", |s| s.status.as_str());
                verdict = format!("analysis status reproduced: {status}");
            } else {
                verdict = match other_solver {
                    Some(archived) => format!("numerical replay OK (solved with {archived}, replayed with {replay_solver})"),
                    None => "numerical replay OK".to_string(),
                };
            }
            if comparison.fluxes.count > 0 {
                detail = format!(" · max Δflux {}", to_exponential(comparison.fluxes.max_abs_error, 2));
            }
        } else if !self.solution.as_ref().is_some_and(|s| s.is_optimal()) {
            state = StatusState::Error;
            let status = self.solution.as_ref().map_or("not run", |s| s.status.as_str());
            verdict.push_str(&format!(" · new analysis: {status}"));
        }
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
            state,
            format!("Bundle restored · {verdict}{detail} · SHA-256 {hash}…"),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_store() -> Store {
        let mut store = Store::with_dir(coxeter_core::testkit::repo_root());
        store.init().expect("bundled files");
        store.use_scenario("solo-ecoli").expect("scenario");
        assert!(store.remove_map_reaction("eco@ATPM").is_some());
        store.clear_solution();
        store
    }

    fn refused_without_changes(store: &mut Store, doc: &Value, message: &str) {
        let state = store.state_value();
        let drawing = store.map.as_ref().expect("map").to_json();
        let unsaved = store.unsaved_work();
        let error = store.load_state(&doc.to_string()).expect_err("invalid scenario");
        assert!(error.to_string().to_lowercase().contains(message), "{error}");
        assert_eq!(store.state_value(), state);
        assert_eq!(store.map.as_ref().expect("map").to_json(), drawing);
        assert!(store.solution.is_none());
        assert!(store.solution_cleared);
        assert!(store.can_undo_map());
        assert_eq!(store.unsaved_work(), unsaved);
    }

    #[test]
    fn a_file_without_scenario_species_and_medium_is_refused_without_changing_the_open_work() {
        let mut store = make_store();
        for doc in [Value::Null, json!([]), json!({}), json!({"species": [], "medium": {}}), json!({"species": "eco", "medium": {}})] {
            refused_without_changes(&mut store, &doc, "scenario");
        }
    }

    #[test]
    fn invalid_species_and_medium_values_are_refused_without_changing_the_open_work() {
        let mut store = make_store();
        let valid = store.state_value();
        let member = valid["species"][0].clone();
        let mut invalid_species = vec![json!([null]), json!([member.clone(), member.clone()])];
        for (key, value) in [
            ("id", json!("")),
            ("file", json!("")),
            ("abundance", json!(-1)),
            ("abundance", json!("one")),
            ("enabled", json!("false")),
        ] {
            let mut species = member.clone();
            species[key] = value;
            invalid_species.push(json!([species]));
        }
        for species in invalid_species {
            let mut doc = valid.clone();
            doc["species"] = species;
            refused_without_changes(&mut store, &doc, "species");
        }
        for medium in [Value::Null, json!([]), json!("glucose"), json!({"glc__D": -10}), json!({"glc__D": "ten"}), json!({"glc__D": null})] {
            let mut doc = valid.clone();
            doc["medium"] = medium;
            refused_without_changes(&mut store, &doc, "medium");
        }
    }

    #[test]
    fn invalid_reaction_overrides_are_refused_without_changing_the_open_work() {
        let mut store = make_store();
        let valid = store.state_value();
        let bound = json!({"id": "eco@ATPM", "lower": 0, "upper": 10});
        let mut invalid = vec![json!({}), json!([null]), json!([bound.clone(), bound.clone()])];
        for (key, value) in [("id", json!("")), ("lower", json!("zero")), ("upper", Value::Null), ("lower", json!(11))] {
            let mut entry = bound.clone();
            entry[key] = value;
            invalid.push(json!([entry]));
        }
        for overrides in invalid {
            let mut doc = valid.clone();
            doc["overrides"] = overrides;
            refused_without_changes(&mut store, &doc, "override");
        }
    }

    #[test]
    fn an_unavailable_scenario_model_is_refused_before_replacing_the_open_work() {
        let mut store = make_store();
        let mut doc = store.state_value();
        doc["species"][0]["file"] = json!("missing-model");
        refused_without_changes(&mut store, &doc, "missing-model");
    }

    #[test]
    fn valid_scenario_numbers_keep_zero_supply_zero_abundance_and_negative_reversible_bounds() {
        let mut store = make_store();
        let mut doc = store.state_value();
        doc["medium"]["o2"] = json!(0);
        doc["species"].as_array_mut().expect("species").push(json!({
            "id": "other", "file": "ecoli_core_mc", "abundance": 0, "enabled": false,
        }));
        doc["overrides"] = json!([{"id": "eco@PGI", "lower": -100, "upper": 100}]);
        store.load_state(&doc.to_string()).expect("valid scenario");
        assert_eq!(store.medium["o2"], 0.0);
        assert_eq!(store.species[1].abundance, 0.0);
        assert!(!store.species[1].enabled);
        let bounds = store.current_reaction_bounds("eco@PGI");
        assert_eq!((bounds.lower, bounds.upper), (-100.0, 100.0));
        assert_eq!(store.solution.as_ref().expect("solution").status, coxeter_solver::lp::LpStatus::Optimal);
    }
}

#[cfg(test)]
mod bundle_tests {
    use super::*;
    use coxeter_io::model_import::ImportFile;
    use coxeter_io::reproducibility::sha256;
    use coxeter_render::theme::FluxColorStop;

    fn make_store() -> Store {
        let mut store = Store::with_dir(coxeter_core::testkit::repo_root());
        store.init().expect("bundled files");
        store.use_scenario("solo-ecoli").expect("scenario");
        store
    }

    fn drawing_state(store: &Store) -> Value {
        json!({
            "map": store.map.as_ref().expect("map").to_json(),
            "presentation": {
                "mapMode": store.map_mode, "pathwayManager": store.pathway_manager_state(),
                "fluxColorStops": flux_color_stops_json(store.view.flux_color_stops.as_deref()),
                "absoluteFluxColors": store.absolute_flux_colors_json(),
                "pathwayRibbonMode": store.view.pathway_ribbon_mode.as_str(),
                "reactionColorMode": store.view.reaction_color_mode.as_str(),
                "showCrossFeeding": store.view.show_cross_feeding, "showCurrency": store.view.show_currency,
            },
            "layout": layout_settings_json(&store.layout_settings),
            "widthByFlux": store.view.width_by_flux, "dashNoFlux": store.view.dash_no_flux,
            "lineHops": store.view.line_hops, "crossFeedingWidth": store.view.cross_feeding_width,
            "ribbonWidth": store.view.pathway_ribbon_width, "ribbonOpacity": store.view.pathway_ribbon_opacity,
            "omics": store.omics_state(),
        })
    }

    fn seal(bundle: &mut Value) {
        let models = bundle["models"].as_object().expect("models").clone();
        let mut hashes = Map::new();
        for (id, model) in models {
            hashes.insert(id, json!(sha256(&model)));
        }
        bundle["modelHashes"] = Value::Object(hashes);
        bundle.as_object_mut().expect("bundle").remove("integrity");
        let hash = sha256(bundle);
        bundle["integrity"] = json!({"algorithm": "SHA-256", "payloadHash": hash});
    }

    fn editable_store() -> Store {
        let mut store = make_store();
        let text = store.model_json(None).expect("model JSON");
        store.import_model_files(&[ImportFile { name: "current.json".into(), text }]).expect("import model");
        store.add_map_text("Keep this drawing", 300.0, 300.0, None).expect("text");
        store.clear_solution();
        store
    }

    fn refuse_without_changes(store: &mut Store, bundle: &mut Value) {
        let state = store.state_value();
        let map = store.map.as_ref().expect("map").to_json();
        let keys: Vec<_> = store.models.keys().cloned().collect();
        let uploaded = store.uploaded_model_keys.clone();
        let history = store.map_history.undo_len();
        let cleared = store.solution_cleared;
        let unsaved = store.unsaved_work();
        seal(bundle);
        assert!(store.import_reproducibility_bundle(bundle).is_err(), "the malformed bundle must be rejected");
        assert!(store.state_value() == state, "the scenario settings are preserved");
        assert!(store.map.as_ref().expect("map").to_json() == map, "the drawing is preserved");
        assert_eq!(store.models.keys().cloned().collect::<Vec<_>>(), keys);
        assert_eq!(store.uploaded_model_keys, uploaded);
        assert_eq!(store.map_history.undo_len(), history);
        assert_eq!(store.solution_cleared, cleared);
        assert_eq!(store.unsaved_work(), unsaved);
    }

    #[test]
    fn a_bundle_restores_drawing_palettes_pathways_layout_and_map_omics() {
        let mut source = make_store();
        source.set_layout_preset("compact");
        source.create_manual_pathway("Saved route", Some("#123456"), Some(&["eco@TALA".into()])).expect("pathway");
        source.set_flux_color_stops(Some(vec![
            FluxColorStop { value: 0.0, color: "#ffffff".into() },
            FluxColorStop { value: 10.0, color: "#0000ff".into() },
        ])).expect("colors");
        source.restore_absolute_flux_colors(&json!({
            "stops": [{"value": 0, "color": "#ffffff"}, {"value": 10, "color": "#00ff00"}], "log": true,
        })).expect("absolute colors");
        source.set_view(ViewPatch {
            reaction_color_mode: Some("absflux".into()), pathway_ribbon_mode: Some("pathway".into()),
            pathway_ribbon_width: Some(72.0), pathway_ribbon_opacity: Some(0.3), width_by_flux: Some(true),
            dash_no_flux: Some(false), line_hops: Some(false), cross_feeding_width: Some(1.5),
            show_cross_feeding: Some(true), show_currency: Some(false), ..Default::default()
        }).expect("display choices");
        source.import_omics_csv("species,gene,Control,Treatment\neco,b0008,1,4\n", "omics.csv", "gene").expect("omics");
        source.add_map_text("Archived drawing", 300.0, 300.0, None).expect("text");
        let expected = drawing_state(&source);
        let bundle = source.reproducibility_bundle().expect("bundle");
        let mut target = make_store();
        target.create_manual_pathway("Unrelated route", None, Some(&["eco@PGI".into()])).expect("pathway");
        let result = target.import_reproducibility_bundle(&bundle).expect("restore bundle");
        assert_eq!(result["valid"], true);
        assert_eq!(result["replay"]["pass"], true);
        assert!(drawing_state(&target) == expected, "the complete drawing presentation is restored");
        assert!(!target.can_undo_map());
        let unsaved = target.unsaved_work();
        assert!(!unsaved.map && !unsaved.scenario && !unsaved.model);
    }

    #[test]
    fn older_bundles_load_without_inheriting_unrelated_pathway_and_map_omics_annotations() {
        let source = make_store();
        let original = source.reproducibility_bundle().expect("bundle");
        for version in [1, 2] {
            let mut bundle = original.clone();
            bundle["version"] = json!(version);
            bundle.as_object_mut().expect("bundle").remove("presentation");
            seal(&mut bundle);
            let mut target = make_store();
            target.create_manual_pathway("Unrelated route", None, Some(&["eco@PGI".into()])).expect("pathway");
            target.import_omics_csv("gene,Control\nb0008,1\n", "omics.csv", "gene").expect("omics");
            let result = target.import_reproducibility_bundle(&bundle).expect("restore old bundle");
            assert_eq!(result["replay"]["pass"], true);
            assert!(target.manual_pathways.is_empty());
            assert!(target.omics_dataset.is_none());
        }
    }

    #[test]
    fn a_bundle_export_keeps_its_snapshot_when_another_community_loads() {
        let mut source = make_store();
        let bundle = source.reproducibility_bundle().expect("bundle");
        source.use_scenario("consorcio-anaerobio").expect("another community");
        assert_eq!(bundle["community"]["species"][0]["id"], "eco");
        assert_eq!(bundle["community"]["scenario"], "solo-ecoli");
        assert_eq!(bundle["models"].as_object().expect("models").keys().collect::<Vec<_>>(), vec!["eco"]);
        assert!(verify_reproducibility_bundle(&bundle).expect("integrity").valid);
    }

    #[test]
    fn missing_or_duplicate_bundle_species_are_refused_without_replacing_uploaded_models() {
        let source = make_store();
        let original = source.reproducibility_bundle().expect("bundle");
        let mut missing = original.clone();
        missing["models"].as_object_mut().expect("models").remove("eco");
        let mut duplicate = original.clone();
        let species = duplicate["community"]["species"][0].clone();
        duplicate["community"]["species"].as_array_mut().expect("species").push(species);
        let mut absent = original;
        absent["community"]["species"] = json!([]);
        for mut bundle in [missing, duplicate, absent] {
            refuse_without_changes(&mut editable_store(), &mut bundle);
        }
    }

    #[test]
    fn an_invalid_embedded_model_is_refused_before_any_current_model_is_deleted() {
        let source = make_store();
        let mut bundle = source.reproducibility_bundle().expect("bundle");
        bundle["community"]["species"].as_array_mut().expect("species").push(json!({
            "id": "broken", "file": "unused", "abundance": 1, "enabled": true,
        }));
        bundle["models"]["broken"] = json!({
            "id": "broken", "metabolites": [], "reactions": [{"id": "R", "metabolites": {"missing": -1}}],
        });
        refuse_without_changes(&mut editable_store(), &mut bundle);
    }

    #[test]
    fn invalid_bundle_medium_and_reaction_bounds_are_refused_before_replacing_the_open_work() {
        let source = make_store();
        let original = source.reproducibility_bundle().expect("bundle");
        for patch in [
            json!({"medium": {"glc__D": "ten"}}), json!({"medium": {"glc__D": -1}}),
            json!({"reactionOverrides": {"eco@ATPM": {"lower": 10, "upper": 1}}}),
            json!({"reactionOverrides": {"eco@ATPM": {"lower": 0, "upper": null}}}),
        ] {
            let mut bundle = original.clone();
            for (key, value) in patch.as_object().expect("patch") {
                bundle["constraints"][key] = value.clone();
            }
            refuse_without_changes(&mut editable_store(), &mut bundle);
        }
    }

    #[test]
    fn a_malformed_bundle_drawing_is_refused_before_replacing_the_open_work() {
        let source = make_store();
        let mut bundle = source.reproducibility_bundle().expect("bundle");
        bundle["map"]["nodes"] = json!({});
        refuse_without_changes(&mut editable_store(), &mut bundle);
    }

    #[test]
    fn invalid_analysis_methods_missing_gimme_data_and_replay_tolerances_are_refused_before_replacement() {
        let source = make_store();
        let original = source.reproducibility_bundle().expect("bundle");
        for method in ["GIMME", "unknown-analysis"] {
            let mut bundle = original.clone();
            bundle["analysis"]["method"] = json!(method);
            bundle["omics"] = Value::Null;
            refuse_without_changes(&mut editable_store(), &mut bundle);
        }
        let mut bundle = original;
        bundle["replay"]["tolerances"]["absolute"] = json!(-1);
        refuse_without_changes(&mut editable_store(), &mut bundle);
    }

    #[test]
    fn invalid_presentation_colors_are_refused_before_replacing_the_open_work() {
        let source = make_store();
        let mut bundle = source.reproducibility_bundle().expect("bundle");
        bundle["presentation"] = json!({"fluxColorStops": [{"value": -1, "color": "broken"}]});
        refuse_without_changes(&mut editable_store(), &mut bundle);
    }

    #[test]
    fn malformed_pathway_reaction_lists_are_refused_before_replacing_the_open_work() {
        let source = make_store();
        let mut bundle = source.reproducibility_bundle().expect("bundle");
        bundle["presentation"] = json!({
            "pathwayManager": {
                "manual": [{"id": "bad-route", "name": "Invalid route", "reactionIds": "eco@TALA"}], "settings": [],
            },
        });
        refuse_without_changes(&mut editable_store(), &mut bundle);
    }

    #[test]
    fn bundle_versions_must_be_supported_positive_integers() {
        let source = make_store();
        let original = source.reproducibility_bundle().expect("bundle");
        for version in [Value::Null, json!("2"), json!(0), json!(-1), json!(1.5)] {
            let mut bundle = original.clone();
            bundle["version"] = version;
            seal(&mut bundle);
            assert!(verify_reproducibility_bundle(&bundle).is_err());
        }
    }

    #[test]
    fn a_pfva_bundle_restores_its_parsimonious_solution_and_archived_flux_ranges() {
        let mut source = make_store();
        source.run_analysis_blocking("pfva", std::time::Duration::from_secs(20)).expect("pFVA");
        let ranges = source.fva_ranges.clone();
        let bundle = source.reproducibility_bundle().expect("bundle");
        let mut target = make_store();
        let result = target.import_reproducibility_bundle(&bundle).expect("restore pFVA bundle");
        assert!(target.parsimonious);
        assert!(target.pfva_enabled);
        assert!(target.fva_enabled);
        let total = source.solution.as_ref().expect("source solution").result.as_ref().expect("source flux result")
            .total_flux.expect("parsimonious total flux");
        let restored_total = target.solution.as_ref().expect("restored solution").result.as_ref().expect("restored flux result")
            .total_flux.expect("restored parsimonious total flux");
        assert!((restored_total - total).abs() < 1e-6);
        assert_eq!(target.fva_ranges, ranges);
        assert_eq!(result["replay"]["pass"], true);
    }

    #[test]
    fn malformed_community_omics_are_refused_before_replacing_the_open_work() {
        let source = make_store();
        let mut bundle = source.reproducibility_bundle().expect("bundle");
        bundle["omics"] = json!({"conditions": ["Control"], "values": [], "settings": {}});
        refuse_without_changes(&mut editable_store(), &mut bundle);
    }

    #[test]
    fn gimme_archives_with_unusable_expression_inputs_are_refused_before_replacing_the_open_work() {
        let mut original = make_store().reproducibility_bundle().expect("bundle");
        original["analysis"]["method"] = json!("GIMME");
        original["omics"] = json!({
            "conditions": ["Control"], "values": {"unknown": {"id": "unmapped", "species": "eco", "values": {"Control": 4}}},
            "settings": {"conditionA": "Control"},
        });
        for gimme in [
            json!({"condition": "Control", "objectiveFraction": 0.8, "threshold": 5}),
            json!({"condition": "missing", "objectiveFraction": 0.8, "threshold": 5}),
            json!({"condition": "Control", "objectiveFraction": -1, "threshold": 5}),
            json!({"condition": "Control", "objectiveFraction": 0.8, "threshold": "bad"}),
        ] {
            let mut bundle = original.clone();
            bundle["analysis"]["gimme"] = gimme;
            refuse_without_changes(&mut editable_store(), &mut bundle);
        }
    }
}

#[cfg(test)]
mod export_provenance_tests {
    use super::*;
    use std::time::Duration;
    use crate::omics::CommunityOmicsPatch;

    fn make_store() -> Store {
        let mut store = Store::with_dir(coxeter_core::testkit::repo_root());
        store.init().expect("bundled models");
        let mut raw: Value = serde_json::from_str(include_str!("../../../../models/ecoli_core_mc.json")).expect("model");
        for reaction in raw["reactions"].as_array_mut().expect("reactions") {
            if reaction["id"] == "GAPD" {
                reaction["gene_reaction_rule"] = json!("test_gap");
            }
        }
        store.models.insert("ecoli_core_mc", Model::from_value(&raw).expect("annotated model"));
        store.use_scenario("solo-ecoli").expect("scenario");
        store
    }

    fn gimme(store: &mut Store) {
        store.import_metatranscriptomics("MAG,gene,condition,TPM\neco,test_gap,Control,1\n", "test-genes.csv").expect("expression");
        store.run_community_gimme(CommunityOmicsPatch {
            gimme_threshold: Some(Some(5.0)), gimme_objective_fraction: Some(0.8), ..Default::default()
        }).expect("GIMME");
    }

    fn minimise_maintenance(store: &mut Store) {
        store.set_objective_coefficient("eco@BIOMASS_Ec_core", 0.0).expect("objective");
        store.set_objective_coefficient("eco@ATPM", 1.0).expect("objective");
        store.set_objective_sense(ObjectiveSense::Min).expect("minimise");
    }

    #[test]
    fn fba_pfba_cyclefreeflux_and_loopless_results_record_their_actual_backend() {
        let store = make_store();
        let problem = store.problem.as_ref().expect("problem");
        assert_eq!(problem.optimize().solver.id(), "highs");
        assert_eq!(problem.parsimonious(Default::default()).solver.id(), "highs");
        assert_eq!(problem.cycle_free(&Default::default()).solver.id(), "highs");
        assert_eq!(problem.loopless(&Default::default()).solver.id(), "highs");
    }

    #[test]
    fn bundle_and_report_solver_names_belong_to_the_result_rather_than_a_later_backend_choice() {
        let store = make_store();
        let (bundle, text) = store.methods_provenance_report().expect("report");
        assert_eq!(bundle["solution"]["solver"], store.solution.as_ref().expect("solution").solver);
        let label = store.solution_solver_label().expect("actual solver");
        assert_eq!(bundle["analysis"]["solver"], label);
        assert!(text.contains(&format!("Solver: {label}.")));
    }

    #[test]
    fn gimme_provenance_names_the_primary_and_contextual_solvers_independently() {
        let mut store = make_store();
        gimme(&mut store);
        let (bundle, text) = store.methods_provenance_report().expect("report");
        let solver = store.solution_solver_label().expect("solver");
        assert_eq!(bundle["analysis"]["gimme"]["primarySolver"], solver);
        assert_eq!(bundle["analysis"]["gimme"]["contextualSolver"], solver);
        assert!(text.contains(&format!("primary objective: {solver}")));
        assert!(text.contains(&format!("penalty minimisation: {solver}")));
    }

    #[test]
    fn figure_metadata_identifies_a_gimme_result_and_its_solver() {
        let mut store = make_store();
        gimme(&mut store);
        let metadata = store.export_metadata();
        let bundle = store.reproducibility_bundle().expect("bundle");
        assert_eq!(metadata["analysis"]["method"], "GIMME");
        assert_eq!(metadata["analysis"]["solver"], bundle["analysis"]["solver"]);
    }

    #[test]
    fn figure_metadata_keeps_the_displayed_method_while_another_analysis_is_starting() {
        let mut store = make_store();
        gimme(&mut store);
        let value = store.solution.as_ref().expect("solution").objective_value;
        store.run_analysis("pfba").expect("pFBA request");
        let metadata = store.export_metadata();
        assert_eq!(metadata["analysis"]["method"], "GIMME");
        assert_eq!(metadata["analysis"]["objectiveValue"], json!(value));
        store.wait_for_jobs(Duration::from_secs(20));
        assert_eq!(store.export_metadata()["analysis"]["method"], "pFBA");
    }

    #[test]
    fn methods_reports_and_figures_name_the_custom_objective_actually_minimised() {
        let mut store = make_store();
        minimise_maintenance(&mut store);
        let (_, text) = store.methods_provenance_report().expect("report");
        assert!(text.contains("Custom objective (min): 1 eco@ATPM."));
        assert!(!text.contains("Community objective: sum-biomass."));
        let metadata = store.export_metadata();
        assert_eq!(metadata["analysis"]["objectiveSense"], "min");
        assert_eq!(metadata["analysis"]["customObjective"], json!({"sense": "min", "coefficients": {"eco@ATPM": 1.0}}));
        let flux = store.solution.as_ref().expect("solution").fluxes["eco@ATPM"];
        assert!((metadata["analysis"]["objectiveValue"].as_f64().expect("objective") - flux).abs() < 1e-7);
    }

    #[test]
    fn methods_reports_distinguish_disabled_models_from_simulated_species() {
        let mut store = make_store();
        let mut scenario = store.scenario.as_ref().expect("scenario").clone();
        scenario["id"] = json!("held-cell");
        scenario["map"] = Value::Null;
        scenario["species"] = json!([
            {"id": "eco", "model": "ecoli_core_mc", "abundance": 0.5, "enabled": true},
            {"id": "other", "model": "ecoli_core_mc", "abundance": 0.5, "enabled": false},
        ]);
        store.scenarios.push(scenario);
        store.use_scenario("held-cell").expect("scenario");
        let (_, text) = store.methods_provenance_report().expect("report");
        let enabled = text.lines().find(|line| line.starts_with("- eco:")).expect("enabled species");
        let disabled = text.lines().find(|line| line.starts_with("- other:")).expect("disabled species");
        assert!(enabled.contains("; enabled;"));
        assert!(disabled.contains("; disabled;"));
    }

    #[test]
    fn product_objective_figure_metadata_preserves_the_solved_target() {
        let mut store = make_store();
        store.set_objective("product", Some("ac")).expect("acetate objective");
        assert_eq!(store.export_metadata()["analysis"]["productTarget"], "ac");
        store.set_objective("product", Some("succ")).expect("succinate objective");
        assert_eq!(store.export_metadata()["analysis"]["productTarget"], "succ");
    }

    #[test]
    fn cleared_figure_metadata_does_not_claim_that_a_numerical_analysis_is_present() {
        let mut store = make_store();
        gimme(&mut store);
        store.clear_solution();
        let metadata = store.export_metadata();
        let analysis = &metadata["analysis"];
        assert!(analysis["method"].is_null());
        assert!(analysis["solver"].is_null());
        assert!(analysis["status"].is_null());
        assert!(analysis["objectiveValue"].is_null());
    }

    #[test]
    fn completed_pfva_figure_metadata_agrees_with_its_bundle_analysis() {
        let mut store = make_store();
        store.run_analysis_blocking("pfva", Duration::from_secs(20)).expect("pFVA");
        let metadata = store.export_metadata();
        let bundle = store.reproducibility_bundle().expect("bundle");
        assert_eq!(metadata["analysis"]["method"], bundle["analysis"]["method"]);
        assert_eq!(metadata["analysis"]["method"], "pFVA");
        assert_eq!(metadata["analysis"]["solver"], bundle["analysis"]["solver"]);
    }
}

#[cfg(test)]
mod archive_outcome_tests {
    use super::*;
    use coxeter_io::reproducibility::compare_replayed_solution;
    use crate::omics::CommunityOmicsPatch;
    use std::time::Duration;

    fn make_store() -> Store {
        let mut store = Store::with_dir(coxeter_core::testkit::repo_root());
        store.init().expect("bundled files");
        let mut raw: Value = serde_json::from_str(include_str!("../../../../models/ecoli_core_mc.json")).expect("model");
        for reaction in raw["reactions"].as_array_mut().expect("reactions") {
            if reaction["id"] == "GAPD" {
                reaction["gene_reaction_rule"] = json!("test_gap");
            }
        }
        store.models.insert("ecoli_core_mc", Model::from_value(&raw).expect("annotated model"));
        store.use_scenario("solo-ecoli").expect("scenario");
        store
    }

    fn gimme(store: &mut Store) {
        store.import_metatranscriptomics("MAG,gene,condition,TPM\neco,test_gap,Control,1\n", "expression.csv").expect("expression");
        store.run_community_gimme(CommunityOmicsPatch {
            gimme_threshold: Some(Some(5.0)), gimme_objective_fraction: Some(0.8), ..Default::default()
        }).expect("GIMME");
    }

    fn infeasible(store: &mut Store) {
        store.set_medium_limit("glc__D", 0.0).expect("no glucose");
        store.set_medium_limit("o2", 0.0).expect("no oxygen");
        assert_eq!(store.solution.as_ref().expect("failed result").status.as_str(), "infeasible");
    }

    #[test]
    fn an_input_only_bundle_recalculates_without_claiming_a_numerical_replay() {
        let mut store = make_store();
        let drawing = store.map.as_ref().expect("drawing").to_json();
        store.clear_solution();
        let bundle = store.reproducibility_bundle().expect("bundle");
        assert!(bundle["solution"].is_null());
        let check = store.import_reproducibility_bundle(&bundle).expect("restore inputs");
        assert!(check["replay"].is_null());
        assert!(store.reproducibility_replay.as_ref().expect("report")["replay"].is_null());
        assert!(store.solution.as_ref().expect("new calculation").is_optimal());
        assert_eq!(store.status().state, StatusState::Ok);
        assert!(store.status().message.contains("no archived numerical result to compare"));
        assert_eq!(store.map.as_ref().expect("drawing").to_json(), drawing);
    }

    #[test]
    fn an_input_only_gimme_archive_excludes_retired_scores_and_calculated_solver_stages() {
        let mut store = make_store();
        gimme(&mut store);
        assert!(store.omics.gimme_result.as_ref().expect("GIMME").inconsistency_score.expect("penalty") > 0.0);
        store.clear_solution();
        let _ = store.set_community_omics(CommunityOmicsPatch { gimme_threshold: Some(Some(0.0)), ..Default::default() });
        let bundle = store.reproducibility_bundle().expect("bundle");
        assert!(bundle["solution"].is_null());
        assert_eq!(bundle["analysis"]["method"], "GIMME");
        assert_eq!(bundle["analysis"]["gimme"]["threshold"], 0.0);
        assert!(bundle["analysis"]["gimme"]["inconsistencyScore"].is_null());
        assert!(bundle["analysis"]["gimme"]["primarySolver"].is_null());
        assert!(bundle["analysis"]["gimme"]["contextualSolver"].is_null());
    }

    #[test]
    fn an_input_only_gimme_bundle_recalculates_its_configured_parameters_without_a_false_mismatch() {
        let mut store = make_store();
        gimme(&mut store);
        store.clear_solution();
        let _ = store.set_community_omics(CommunityOmicsPatch {
            gimme_threshold: Some(Some(0.0)), gimme_objective_fraction: Some(0.6), ..Default::default()
        });
        let bundle = store.reproducibility_bundle().expect("bundle");
        let check = store.import_reproducibility_bundle(&bundle).expect("restore inputs");
        assert!(check["replay"].is_null());
        assert!(store.solution.as_ref().expect("GIMME").is_optimal());
        let result = store.omics.gimme_result.as_ref().expect("GIMME details");
        assert_eq!(result.threshold, Some(0.0));
        assert_eq!(result.objective_fraction, Some(0.6));
        assert_eq!(store.status().state, StatusState::Ok);
    }

    #[test]
    fn an_input_only_pfva_bundle_restores_and_recomputes_its_selected_ranges() {
        let mut store = make_store();
        store.set_pfba_biomass_percent(90.0).expect("target");
        store.run_analysis_blocking("pfva", Duration::from_secs(20)).expect("pFVA");
        store.clear_solution();
        let bundle = store.reproducibility_bundle().expect("bundle");
        let check = store.import_reproducibility_bundle(&bundle).expect("restore inputs");
        assert!(check["replay"].is_null());
        assert!(store.pfva_enabled);
        assert!(store.fva_enabled);
        store.wait_for_jobs(Duration::from_secs(20));
        assert_eq!(store.fva_ranges.len(), store.problem.as_ref().expect("problem").rxn_ids.len());
        assert_eq!(store.export_metadata()["analysis"]["method"], "pFVA");
        store.set_fva(false).expect("stop ranges");
    }

    #[test]
    fn an_infeasible_bundle_reproduces_its_solver_status_without_a_false_objective_mismatch() {
        let mut store = make_store();
        infeasible(&mut store);
        let bundle = store.reproducibility_bundle().expect("bundle");
        assert!(bundle["solution"]["objectiveValue"].is_null());
        let check = store.import_reproducibility_bundle(&bundle).expect("replay failed result");
        assert_eq!(check["replay"]["pass"], true);
        assert_eq!(check["replay"]["checks"]["status"], true);
        assert!(store.status().message.contains("analysis status reproduced: infeasible"));
        assert!(!store.status().message.contains("numerical replay OK"));
    }

    #[test]
    fn methods_describes_input_only_archives_as_configured_work_without_completed_analysis_claims() {
        for method in ["fba", "pfba", "cyclefree", "gimme"] {
            let mut store = make_store();
            if method == "gimme" {
                gimme(&mut store);
            } else {
                store.run_analysis_blocking(method, Duration::from_secs(20)).expect("analysis");
            }
            store.clear_solution();
            let (_, report) = store.methods_provenance_report().expect("report");
            assert!(report.contains("Analysis settings were prepared"), "{method}");
            assert!(report.contains("Configured method:"), "{method}");
            assert!(!report.contains("Metabolic simulations were performed"), "{method}");
            assert!(!report.contains("removed the internal cycles"), "{method}");
            assert!(!report.contains("GIMME used condition"), "{method}");
        }
    }

    #[test]
    fn methods_distinguishes_a_failed_analysis_attempt_from_an_optimal_calculation() {
        let mut store = make_store();
        infeasible(&mut store);
        let (_, report) = store.methods_provenance_report().expect("report");
        assert!(report.contains("An analysis was attempted"));
        assert!(report.contains("No optimal solution was obtained"));
        assert!(!report.contains("Metabolic simulations were performed"));
    }

    #[test]
    fn missing_optimal_objectives_still_fail_comparison_while_unavailable_failed_objectives_agree() {
        let failed = json!({"status": "infeasible", "objectiveValue": null, "fluxes": {}});
        assert!(compare_replayed_solution(&failed, &failed, Tolerances::default()).pass);
        let invalid = json!({"status": "optimal", "objectiveValue": null, "fluxes": {}});
        assert!(!compare_replayed_solution(&invalid, &invalid, Tolerances::default()).pass);
        let different = json!({"status": "unbounded", "objectiveValue": null, "fluxes": {}});
        assert!(!compare_replayed_solution(&failed, &different, Tolerances::default()).pass);
    }
}

#[cfg(test)]
mod replay_lifecycle_tests {
    use super::*;
    use crate::omics::CommunityOmicsPatch;
    use std::time::Duration;

    fn replayed_store() -> Store {
        let mut store = Store::with_dir(coxeter_core::testkit::repo_root());
        store.init().expect("bundled models");
        store.use_scenario("solo-ecoli").expect("scenario");
        let bundle = store.reproducibility_bundle().expect("bundle");
        store.import_reproducibility_bundle(&bundle).expect("verified replay");
        assert_eq!(store.reproducibility_replay.as_ref().expect("report")["replay"]["pass"], true);
        store
    }

    #[test]
    fn a_changed_medium_retires_the_imported_numerical_verdict() {
        let mut store = replayed_store();
        let objective = store.solution.as_ref().expect("solution").objective_value;
        store.set_medium_limit("glc__D", 5.0).expect("medium");
        assert!(store.solution.as_ref().expect("changed solution").objective_value < objective);
        assert!(store.reproducibility_replay.is_none());
    }

    #[test]
    fn unsolved_bound_edits_retire_the_verdict_before_another_result_exists() {
        let mut store = replayed_store();
        let solution = store.solution.clone().expect("solution");
        store.set_reaction_bounds("eco@PGI", 0.0, 0.0).expect("bounds");
        assert!(Arc::ptr_eq(store.solution.as_ref().expect("previous solution"), &solution));
        assert!(store.reproducibility_replay.is_none());
    }

    #[test]
    fn clearing_the_solution_retires_its_imported_numerical_verdict() {
        let mut store = replayed_store();
        store.clear_solution();
        assert!(store.reproducibility_replay.is_none());
    }

    #[test]
    fn starting_another_analysis_retires_the_imported_verdict() {
        let mut store = replayed_store();
        store.run_analysis("pfba").expect("new run");
        let retired = store.reproducibility_replay.is_none();
        store.wait_for_jobs(Duration::from_secs(10));
        assert!(retired);
        assert!(store.reproducibility_replay.is_none());
    }

    #[test]
    fn replacing_the_model_retires_the_imported_verdict() {
        let mut store = replayed_store();
        store.use_scenario("solo-ecoli").expect("new context");
        assert!(store.reproducibility_replay.is_none());
    }

    #[test]
    fn changed_analysis_settings_retire_the_verdict_before_they_affect_a_solve() {
        let mut store = replayed_store();
        let solution = store.solution.clone().expect("solution");
        store.set_pfba_biomass_percent(75.0).expect("fraction");
        assert!(Arc::ptr_eq(store.solution.as_ref().expect("previous solution"), &solution));
        assert!(store.reproducibility_replay.is_none());
    }

    #[test]
    fn enabling_variability_retires_the_earlier_analysis_verdict() {
        let mut store = replayed_store();
        store.set_fva(true).expect("FVA");
        let retired = store.reproducibility_replay.is_none();
        store.set_fva(false).expect("cancel FVA");
        assert!(retired);
    }

    #[test]
    fn a_no_op_setting_or_rejected_bound_input_preserves_the_imported_verdict() {
        let mut store = replayed_store();
        let report = store.reproducibility_replay.clone();
        store.set_pfba_biomass_percent(store.pfba_biomass_percent).expect("same setting");
        store.set_reaction_bounds("eco@PGI", "", "invalid").expect("uncommitted input");
        assert_eq!(store.reproducibility_replay, report);
    }

    #[test]
    fn drawing_edits_and_rejected_file_imports_preserve_the_imported_numerical_verdict() {
        let mut store = replayed_store();
        let report = store.reproducibility_replay.clone();
        store.show_empty_map();
        assert!(store.load_state("{broken").is_err());
        assert!(store.import_reproducibility_bundle(&json!({})).is_err());
        assert_eq!(store.reproducibility_replay, report);
    }

    #[test]
    fn another_valid_import_installs_a_fresh_verdict_after_scientific_edits() {
        let mut store = replayed_store();
        let bundle = store.reproducibility_bundle().expect("bundle");
        store.set_medium_limit("glc__D", 5.0).expect("medium");
        store.import_reproducibility_bundle(&bundle).expect("new replay");
        assert_eq!(store.reproducibility_replay.as_ref().expect("new report")["replay"]["pass"], true);
    }

    #[test]
    fn omics_display_choices_preserve_the_verdict_and_changed_gimme_inputs_retire_it() {
        let mut store = replayed_store();
        let report = store.reproducibility_replay.clone();
        let solution = store.solution.clone().expect("solution");
        let _ = store.set_community_omics(CommunityOmicsPatch { enabled: Some(true), mode: Some("comparison".into()), ..Default::default() });
        assert_eq!(store.reproducibility_replay, report);
        let _ = store.set_community_omics(CommunityOmicsPatch { gimme_threshold: Some(Some(5.0)), ..Default::default() });
        assert!(Arc::ptr_eq(store.solution.as_ref().expect("previous solution"), &solution));
        assert!(store.reproducibility_replay.is_none());
    }

    #[test]
    fn replacing_expression_data_retires_the_imported_input_verdict() {
        let mut store = replayed_store();
        store.import_metatranscriptomics("MAG,gene,condition,TPM\neco,test_gap,Control,1\n", "expression.csv").expect("expression");
        assert!(store.reproducibility_replay.is_none());
    }

    #[test]
    fn a_new_gimme_request_retires_its_previous_imported_verdict_before_completion() {
        let mut store = replayed_store();
        let mut raw: Value = serde_json::from_str(include_str!("../../../../models/ecoli_core_mc.json")).expect("model");
        for reaction in raw["reactions"].as_array_mut().expect("reactions") {
            if reaction["id"] == "GAPD" {
                reaction["gene_reaction_rule"] = json!("test_gap");
            }
        }
        store.models.insert("ecoli_core_mc", Model::from_value(&raw).expect("annotated model"));
        store.use_scenario("solo-ecoli").expect("annotated scenario");
        store.import_metatranscriptomics("MAG,gene,condition,TPM\neco,test_gap,Control,1\n", "expression.csv").expect("expression");
        store.run_community_gimme(CommunityOmicsPatch::default()).expect("GIMME");
        let bundle = store.reproducibility_bundle().expect("bundle");
        store.import_reproducibility_bundle(&bundle).expect("GIMME replay");
        assert!(store.reproducibility_replay.is_some());
        store.start_community_gimme(CommunityOmicsPatch::default()).expect("new GIMME request");
        let retired = store.reproducibility_replay.is_none();
        store.wait_for_jobs(Duration::from_secs(10));
        assert!(retired);
        assert!(store.reproducibility_replay.is_none());
    }
}

#[cfg(test)]
mod steadycom_persistence_tests {
    use super::*;
    use crate::types::ViewPatch;
    use std::time::Duration;

    fn make_store() -> Store {
        let mut store = Store::with_dir(coxeter_core::testkit::repo_root());
        store.init().expect("bundled models");
        store.use_scenario("aerobio-mixto").expect("two bundled members");
        store
    }

    fn finish(store: &mut Store) {
        for outcome in store.wait_for_jobs(Duration::from_secs(10)) {
            outcome.result.expect("SteadyCom calculation");
        }
        assert!(store.solution.as_ref().is_some_and(|solution| solution.steady_com.is_some()));
    }

    #[test]
    fn a_scenario_opened_at_fixed_growth_is_saved_after_its_composition_is_recalculated() {
        let mut store = make_store();
        let mut document: Value = serde_json::from_str(&store.state_json()).expect("scenario");
        document["steadyCom"] = json!({"mu": 0.1});
        store.load_state(&document.to_string()).expect("fixed-growth scenario");
        finish(&mut store);
        assert_eq!(store.solution.as_ref().expect("solution").steady_com.as_ref().expect("SteadyCom").mu, 0.1);
        let before: Vec<f64> = document["species"].as_array().expect("species").iter().map(|species| species["abundance"].as_f64().expect("abundance")).collect();
        let after: Vec<f64> = store.species.iter().map(|species| species.abundance).collect();
        assert_ne!(after, before);
        assert!(!store.unsaved_work().scenario);
    }

    #[test]
    fn changing_fixed_growth_marks_the_scenario_unsaved_even_when_composition_stays_the_same() {
        let mut store = make_store();
        store.use_scenario("solo-ecoli").expect("single member");
        store.solve_steady_com_at(0.1).expect("fixed growth");
        finish(&mut store);
        store.mark_scenario_saved();
        let composition: Vec<f64> = store.species.iter().map(|species| species.abundance).collect();
        store.solve_steady_com_at(0.2).expect("changed growth");
        finish(&mut store);
        assert_eq!(store.species.iter().map(|species| species.abundance).collect::<Vec<_>>(), composition);
        assert!(store.unsaved_work().scenario);
    }

    #[test]
    fn clearing_fixed_growth_marks_its_removed_scenario_analysis_unsaved() {
        let mut store = make_store();
        store.solve_steady_com_at(0.1).expect("fixed growth");
        finish(&mut store);
        store.mark_scenario_saved();
        store.clear_solution();
        let document: Value = serde_json::from_str(&store.state_json()).expect("scenario");
        assert!(document["steadyCom"].is_null());
        assert!(store.unsaved_work().scenario);
    }

    #[test]
    fn edits_made_during_scenario_recalculation_stay_unsaved_and_can_be_undone_back_to_saved() {
        let mut store = make_store();
        let mut document: Value = serde_json::from_str(&store.state_json()).expect("scenario");
        document["steadyCom"] = json!({"mu": 0.1});
        store.load_state(&document.to_string()).expect("fixed-growth scenario");
        let width = store.view.width_by_flux;
        store.set_view(ViewPatch { width_by_flux: Some(!width), ..Default::default() }).expect("display edit");
        finish(&mut store);
        assert!(store.unsaved_work().scenario);
        store.set_view(ViewPatch { width_by_flux: Some(width), ..Default::default() }).expect("undo display edit");
        assert!(!store.unsaved_work().scenario);
    }

    #[test]
    fn an_automatic_opening_solve_preserves_edits_made_while_its_result_is_pending() {
        let mut store = make_store();
        store.scenario.as_mut().expect("scenario")["id"] = json!("published:steadycom-ecoli-auxotrophs");
        store.scenario.as_mut().expect("scenario")["published"] = json!("steadycom-ecoli-auxotrophs");
        store.published_community_id = Some("steadycom-ecoli-auxotrophs".into());
        store.mark_scenario_saved();
        let width = store.view.width_by_flux;
        store.solve_steady_com_on_open().expect("automatic fixed growth");
        store.set_view(ViewPatch { width_by_flux: Some(!width), ..Default::default() }).expect("display edit");
        finish(&mut store);
        assert!(store.unsaved_work().scenario);
        store.set_view(ViewPatch { width_by_flux: Some(width), ..Default::default() }).expect("undo display edit");
        assert!(!store.unsaved_work().scenario);
    }

    #[test]
    fn an_imported_scenario_outside_the_catalogue_keeps_its_own_identity_and_original_medium() {
        let mut store = make_store();
        let mut document: Value = serde_json::from_str(&store.state_json()).expect("scenario");
        document["scenario"] = json!("custom-steadycom");
        document["medium"]["glc__D"] = json!(3.0);
        store.load_state(&document.to_string()).expect("custom scenario");
        assert_eq!(store.scenario_id().as_deref(), Some("custom-steadycom"));
        assert_eq!(store.scenario_str("name").as_deref(), Some("Imported scenario"));
        assert_eq!(store.base_medium["glc__D"], 3.0);
        assert!(!store.unsaved_work().scenario);
    }

    #[test]
    fn an_imported_scenario_outside_the_catalogue_does_not_inherit_a_published_formulation() {
        let mut store = make_store();
        let mut document: Value = serde_json::from_str(&store.state_json()).expect("scenario");
        document["scenario"] = json!("custom-steadycom");
        document["steadyCom"] = json!({"mu": 0.1});
        store.scenario.as_mut().expect("scenario")["id"] = json!("published:steadycom-gut-nine");
        store.scenario.as_mut().expect("scenario")["published"] = json!("steadycom-gut-nine");
        store.published_community_id = Some("steadycom-gut-nine".into());
        store.load_state(&document.to_string()).expect("custom fixed-growth scenario");
        finish(&mut store);
        assert_eq!(store.scenario_id().as_deref(), Some("custom-steadycom"));
        assert!(store.scenario_published().is_none());
        assert!(store.published_community_id.is_none());
        assert_eq!(store.solution.as_ref().expect("solution").steady_com.as_ref().expect("SteadyCom").mu, 0.1);
    }

    #[test]
    fn fixed_growth_results_use_scenario_json_and_return_to_ordinary_fba_after_a_cleared_save() {
        let mut store = make_store();
        store.solve_steady_com_at(0.1).expect("fixed growth");
        finish(&mut store);
        assert!(store.reproducibility_bundle().expect_err("bundle excluded").to_string().contains("Scenario JSON"));
        assert!(store.methods_provenance_report().expect_err("Methods excluded").to_string().contains("Scenario JSON"));
        let document = store.state_json();
        store.load_state(&document).expect("saved fixed growth");
        finish(&mut store);
        store.clear_solution();
        store.load_state(&store.state_json()).expect("cleared scenario");
        assert!(store.solution.as_ref().expect("FBA solution").steady_com.is_none());
        assert!(store.solution.as_ref().expect("FBA solution").status.is_optimal());
    }
}
