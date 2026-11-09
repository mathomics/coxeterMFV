//! Saved and unsaved work (the "saved / unsaved" section of `store.js`).

use coxeter_core::js;
use coxeter_map::layout_settings::{LayoutSettings, RawLayoutSettings};
use serde_json::{Map, Value, json};

use crate::store::Store;
use crate::error::Result;
use crate::types::{ModelSaveSnapshot, SavedWork, UnsavedWork, WorkSnapshot};
use crate::util::num_json;

/// `layoutSettings` as the JS object prints.
pub fn layout_settings_json(s: &LayoutSettings) -> Value {
    json!({
        "preset": s.preset,
        "cellSpacing": num_json(s.cell_spacing),
        "minimumCompartmentSize": num_json(s.minimum_compartment_size),
        "nodeSpacing": num_json(s.node_spacing),
        "cellAreaMultiplier": num_json(s.cell_area_multiplier),
    })
}

/// A layout-settings object as a file gives it: an absent field is
/// `undefined` (the preset's value), anything else goes through `Number()`.
pub fn raw_layout_settings(v: &Value) -> RawLayoutSettings {
    let num = |k: &str| v.get(k).map(js::to_number);
    RawLayoutSettings {
        preset: v.get("preset").filter(|p| !p.is_null()).map(js::to_string),
        cell_spacing: num("cellSpacing"),
        minimum_compartment_size: num("minimumCompartmentSize"),
        node_spacing: num("nodeSpacing"),
        cell_area_multiplier: num("cellAreaMultiplier"),
    }
}

impl Store {
    /// `workContextKey()`: what was opened. Switching it starts over with
    /// nothing to save.
    pub fn work_context_key(&self) -> String {
        let species: Vec<Value> = self.species.iter().map(|sp| json!([sp.id, sp.file])).collect();
        let zuniga = if self.is_zuniga_study() { json!(self.zuniga.scenario_id) } else { Value::Null };
        js::stringify(&json!([self.scenario_id(), self.published_community_id, zuniga, species]), None)
    }

    /// `scenarioSignature()`: what only the Scenario file holds and the user
    /// can change.
    pub fn scenario_signature(&self) -> String {
        let medium: Map<String, Value> = self.medium.iter().map(|(k, v)| (k.clone(), num_json(*v))).collect();
        let species: Vec<Value> =
            self.species.iter().map(|sp| json!([sp.id, sp.enabled, num_json(sp.abundance)])).collect();
        let overrides: Vec<Value> = self.overrides.iter().map(|(id, b)| json!([id, b.to_json()])).collect();
        let omics = json!({
            "settings": self.omics_settings.to_json(),
            "dataset": if self.omics_dataset.is_some() { json!(self.omics_dataset_revision) } else { Value::Null },
        });
        js::stringify(
            &json!({
                "medium": medium,
                "species": species,
                "overrides": overrides,
                "geneKnockouts": self.sorted_gene_knockouts(),
                "objective": self.objective,
                "productTarget": self.product_target,
                "customObjective": self.custom_objective.as_ref().map(crate::types::CustomObjective::to_json),
                "parsimonious": self.parsimonious,
                "pfbaBiomassPercent": num_json(self.pfba_biomass_percent),
                "loopless": self.loopless,
                "cycleFree": self.cycle_free,
                "fvaEnabled": self.fva_enabled,
                "pfvaEnabled": self.pfva_enabled,
                "steadyComConfig": self.steady_com_config.to_json(),
                "steadyCom": self.solution.as_ref().and_then(|solution| solution.steady_com.as_ref()).map(|fixed| json!({"mu": num_json(fixed.mu)})),
                "layoutSettings": layout_settings_json(&self.layout_settings),
                "reactionColorMode": self.view.reaction_color_mode.as_str(),
                "lines": [self.view.width_by_flux, self.view.dash_no_flux, self.view.line_hops, num_json(self.view.cross_feeding_width)],
                "ribbons": [self.view.pathway_ribbon_mode.as_str(), num_json(self.view.pathway_ribbon_width), num_json(self.view.pathway_ribbon_opacity)],
                "omics": omics,
            }),
            None,
        )
    }

    /// `sharedSignature()`: what Map JSON holds besides the drawing
    /// (pathways, flux colours and the Zuñiga case). The Scenario file carries
    /// them too, so either export saves them.
    pub fn shared_signature(&self) -> String {
        js::stringify(
            &json!({
                "pathwayManager": self.pathway_manager_state(),
                "fluxColorStops": crate::pathways::flux_color_stops_json(self.view.flux_color_stops.as_deref()),
                "absoluteFluxColors": self.absolute_flux_colors_json(),
                "study": self.zuniga_state(),
            }),
            None,
        )
    }

    /// `settleOpenedWork()`: an open has finished. What is on screen now is
    /// what was opened, so it is all "saved". Every way of opening something
    /// calls this as its last step, so defaults a loader sets late (the
    /// core-overview pathways, the colour mode of a published case) are part
    /// of what was opened, not unsaved edits.
    pub fn settle_opened_work(&mut self) {
        self.saved_work = None;
        self.settle_opened_constraints();
        self.track_work_context();
    }

    /// `trackWorkContext()`: the fallback for anything that changes the
    /// context without being one of those opens. The starting point is taken
    /// when the store announces its first change in the new context (see
    /// `emit`), not when the header is painted.
    pub fn track_work_context(&mut self) {
        let context = self.work_context_key();
        if self.saved_work.as_ref().is_some_and(|s| s.context == context) {
            return;
        }
        self.work_generation += 1;
        self.saved_work = Some(SavedWork { context, scenario: self.scenario_signature(), shared: self.shared_signature() });
    }

    /// `unsavedWork()`: which parts of the work differ from what was last
    /// saved or opened.
    ///
    /// The map part follows the undo history, so undoing back to the saved
    /// map is saved again, plus the pathways and colours the map file holds.
    /// The scenario part compares the scenario settings with those of the
    /// last Scenario export or import, or with what was opened. The model part
    /// asks whether a model in use has edits not yet written out; that belongs
    /// to the model itself, so opening another scenario that uses the same
    /// edited model does not hide it. Nothing here changes any state.
    pub fn unsaved_work(&self) -> UnsavedWork {
        let saved = self.saved_work.as_ref();
        let known = saved.is_some_and(|s| s.context == self.work_context_key());
        UnsavedWork {
            map: self.map.is_some()
                && (self.map_history.dirty() || (known && saved.is_some_and(|s| self.shared_signature() != s.shared))),
            scenario: known && saved.is_some_and(|s| self.scenario_signature() != s.scenario),
            model: self.species.iter().any(|sp| self.model_has_unsaved_edits(self.models.serial(&sp.file))),
        }
    }

    /// `workSnapshot()`: capture the versions a file will contain.
    pub fn work_snapshot(&mut self) -> WorkSnapshot {
        self.track_work_context();
        WorkSnapshot {
            context: self.work_context_key(),
            generation: self.work_generation,
            scenario: self.scenario_signature(),
            shared: self.shared_signature(),
            map_generation: self.map_history.generation(),
            map_version: self.map_history.version(),
        }
    }

    /// `markMapSaved()`: the current drawing was written or opened.
    pub fn mark_map_saved(&mut self) {
        let snapshot = self.work_snapshot();
        self.mark_map_snapshot_saved(&snapshot);
    }

    /// Record the drawing and shared settings actually written to a Map file.
    pub fn mark_map_snapshot_saved(&mut self, snapshot: &WorkSnapshot) {
        if snapshot.generation != self.work_generation || snapshot.context != self.work_context_key()
            || snapshot.map_generation != self.map_history.generation() {
            return;
        }
        self.map_history.mark_version_saved(snapshot.map_version, snapshot.map_generation);
        if let Some(saved) = self.saved_work.as_mut() {
            saved.shared = snapshot.shared.clone();
        }
    }

    /// `markScenarioSaved()`: the scenario settings, and what they share with
    /// the map file, are now in a file.
    pub fn mark_scenario_saved(&mut self) {
        let snapshot = self.work_snapshot();
        self.mark_scenario_snapshot_saved(&snapshot);
    }

    /// Record the Scenario settings actually written before later edits arrived.
    pub fn mark_scenario_snapshot_saved(&mut self, snapshot: &WorkSnapshot) {
        if snapshot.generation != self.work_generation || snapshot.context != self.work_context_key() {
            return;
        }
        if let Some(saved) = self.saved_work.as_mut() {
            saved.scenario = snapshot.scenario.clone();
            saved.shared = snapshot.shared.clone();
        }
    }

    /// Record calculated composition without saving edits made while the opening solve was pending.
    pub(crate) fn settle_opened_steady_com(&mut self, context: &str) -> Result<()> {
        let Some(mu) = self.solution.as_ref().and_then(|solution| solution.steady_com.as_ref()).map(|fixed| fixed.mu) else {
            return Ok(());
        };
        if self.work_context_key() != context {
            return Ok(());
        }
        let Some(saved) = self.saved_work.as_mut().filter(|saved| saved.context == context) else {
            return Ok(());
        };
        let mut signature: Value = serde_json::from_str(&saved.scenario)?;
        if let Some(entries) = signature.get_mut("species").and_then(Value::as_array_mut) {
            for entry in entries {
                let id = entry.get(0).and_then(Value::as_str).unwrap_or("");
                if let Some(species) = self.species.iter().find(|species| species.id == id) {
                    if let Some(abundance) = entry.get_mut(2) {
                        *abundance = num_json(species.abundance);
                    }
                }
            }
        }
        signature["steadyCom"] = json!({"mu": num_json(mu)});
        saved.scenario = js::stringify(&signature, None);
        Ok(())
    }

    /// `markModelSaved(model)`: one model has been written out (the model
    /// editor exports the selected species); its edits are saved, those of
    /// any other model are not. `None` is the model of the editor's species.
    pub fn mark_model_saved(&mut self, file: Option<&str>) {
        if let Some(snapshot) = self.model_save_snapshot(file) {
            self.mark_model_snapshot_saved(&snapshot);
        }
    }

    /// `modelSaveSnapshot()`: capture the model identity and edits being serialized.
    pub fn model_save_snapshot(&self, file: Option<&str>) -> Option<ModelSaveSnapshot> {
        let serial = match file {
            Some(f) => self.models.serial(f),
            None => self.model_editor_species().and_then(|sp| self.models.serial(&sp.file)),
        }?;
        Some(ModelSaveSnapshot { serial, revision: self.model_edits.get(&serial).copied().unwrap_or(0) })
    }

    /// Record only the biological edits present in the written model.
    pub fn mark_model_snapshot_saved(&mut self, snapshot: &ModelSaveSnapshot) {
        self.model_edits_saved.insert(snapshot.serial, snapshot.revision);
    }

    /// `modelHasUnsavedEdits(model)`, by model identity.
    pub fn model_has_unsaved_edits(&self, serial: Option<u64>) -> bool {
        serial.is_some_and(|s| {
            self.model_edits.get(&s).copied().unwrap_or(0) != self.model_edits_saved.get(&s).copied().unwrap_or(0)
        })
    }

    /// Count one biological edit of the model under `file`.
    pub(crate) fn count_model_edit(&mut self, file: &str) {
        if let Some(serial) = self.models.serial(file) {
            *self.model_edits.entry(serial).or_insert(0) += 1;
        }
    }
}

#[cfg(test)]
mod save_timing_tests {
    use super::*;

    fn make_store() -> Store {
        let mut store = Store::with_dir(coxeter_core::testkit::repo_root());
        store.init().expect("bundled models");
        store.use_scenario("solo-ecoli").expect("scenario");
        store
    }

    #[test]
    fn scenario_save_completion_records_the_written_inputs_and_preserves_later_edits() {
        let mut store = make_store();
        store.set_medium_limit("glc__D", "4").expect("written input");
        let snapshot = store.work_snapshot();
        let document: Value = serde_json::from_str(&store.state_json()).expect("scenario");
        store.set_medium_limit("glc__D", "3").expect("later input");
        store.mark_scenario_snapshot_saved(&snapshot);
        assert!(store.unsaved_work().scenario);
        store.set_medium_limit("glc__D", document["medium"]["glc__D"].as_f64().expect("written uptake")).expect("restored input");
        assert!(!store.unsaved_work().scenario);
    }

    #[test]
    fn scenario_save_completion_preserves_shared_pathway_edits_made_after_serialization() {
        let mut store = make_store();
        let snapshot = store.work_snapshot();
        store.create_manual_pathway("Later route", Some("#123456"), Some(&["eco@PGI".into()])).expect("pathway");
        store.mark_scenario_snapshot_saved(&snapshot);
        assert!(store.unsaved_work().map);
        store.undo_map();
        assert!(!store.unsaved_work().map);
    }

    #[test]
    fn map_save_completion_records_the_written_undo_version_and_shared_pathway_settings() {
        let mut store = make_store();
        store.add_map_text("Written annotation", 200.0, 200.0, None).expect("written annotation");
        let snapshot = store.work_snapshot();
        store.add_map_text("Later annotation", 250.0, 250.0, None).expect("later annotation");
        store.create_manual_pathway("Later route", Some("#123456"), Some(&["eco@PGI".into()])).expect("pathway");
        store.mark_map_snapshot_saved(&snapshot);
        assert!(store.unsaved_work().map);
        store.undo_map();
        store.undo_map();
        assert!(!store.unsaved_work().map);
        store.redo_map();
        assert!(store.unsaved_work().map);
    }

    #[test]
    fn a_completed_old_scenario_save_cannot_replace_the_baseline_of_a_reopened_context_with_the_same_id() {
        let mut store = make_store();
        store.set_medium_limit("glc__D", "4").expect("written input");
        let snapshot = store.work_snapshot();
        let mut reopened: Value = serde_json::from_str(&store.state_json()).expect("scenario");
        reopened["medium"]["glc__D"] = json!(9);
        store.load_state(&reopened.to_string()).expect("reopened scenario");
        store.set_medium_limit("glc__D", "8").expect("later input");
        store.mark_scenario_snapshot_saved(&snapshot);
        assert!(store.unsaved_work().scenario);
        store.set_medium_limit("glc__D", "9").expect("restored input");
        assert!(!store.unsaved_work().scenario);
    }

    #[test]
    fn a_completed_old_map_save_cannot_save_a_replacement_context_drawing() {
        let mut store = make_store();
        store.add_map_text("Previous context", 200.0, 200.0, None).expect("written annotation");
        let snapshot = store.work_snapshot();
        store.use_scenario("solo-ecoli").expect("replacement context");
        store.add_map_text("Replacement context", 250.0, 250.0, None).expect("later annotation");
        store.mark_map_snapshot_saved(&snapshot);
        assert!(store.unsaved_work().map);
        store.undo_map();
        assert!(!store.unsaved_work().map);
    }

    #[test]
    fn clearing_drawing_history_retires_earlier_map_save_snapshots_within_the_same_context() {
        let mut store = make_store();
        store.add_map_text("Previous history", 200.0, 200.0, None).expect("written annotation");
        let snapshot = store.work_snapshot();
        store.map_history.clear();
        store.add_map_text("Replacement history", 250.0, 250.0, None).expect("later annotation");
        store.mark_map_snapshot_saved(&snapshot);
        assert!(store.unsaved_work().map);
        store.undo_map();
        assert!(!store.unsaved_work().map);
    }

    #[test]
    fn model_save_completion_records_only_the_biological_edits_serialized_into_the_file() {
        let mut store = make_store();
        store.create_model_metabolite(&json!({"id": "written_c", "name": "Written metabolite", "compartment": "c"})).expect("written edit");
        let snapshot = store.model_save_snapshot(None).expect("model snapshot");
        store.create_model_metabolite(&json!({"id": "later_c", "name": "Later metabolite", "compartment": "c"})).expect("later edit");
        store.mark_model_snapshot_saved(&snapshot);
        assert!(store.unsaved_work().model);
        store.mark_model_saved(None);
        assert!(!store.unsaved_work().model);
    }

    #[test]
    fn a_model_save_records_the_serialized_model_even_when_the_editor_selects_another_species() {
        let mut store = make_store();
        store.use_scenario("aerobio-mixto").expect("two-species scenario");
        let first = store.species[0].id.clone();
        let second = store.species[1].id.clone();
        store.set_model_editor_species(&first).expect("first member");
        store.create_model_metabolite(&json!({"id": "written_c", "name": "Written member", "compartment": "c"})).expect("written edit");
        let snapshot = store.model_save_snapshot(None).expect("model snapshot");
        store.set_model_editor_species(&second).expect("second member");
        store.create_model_metabolite(&json!({"id": "later_c", "name": "Other member", "compartment": "c"})).expect("other edit");
        store.mark_model_snapshot_saved(&snapshot);
        assert!(!store.model_has_unsaved_edits(Some(snapshot.serial)));
        let other = store.model_save_snapshot(None).expect("other snapshot");
        assert!(store.model_has_unsaved_edits(Some(other.serial)));
    }

    #[test]
    fn out_of_order_save_completion_follows_the_last_file_written_rather_than_the_current_inputs() {
        let mut store = make_store();
        store.set_medium_limit("glc__D", "4").expect("first input");
        let earlier = store.work_snapshot();
        store.set_medium_limit("glc__D", "3").expect("second input");
        let later = store.work_snapshot();
        store.mark_scenario_snapshot_saved(&later);
        assert!(!store.unsaved_work().scenario);
        store.mark_scenario_snapshot_saved(&earlier);
        assert!(store.unsaved_work().scenario);
        store.set_medium_limit("glc__D", "4").expect("restored input");
        assert!(!store.unsaved_work().scenario);
    }

    #[test]
    fn a_rejected_scenario_preload_preserves_the_pending_save_snapshot_of_the_current_context() {
        let mut store = make_store();
        store.set_medium_limit("glc__D", "4").expect("written input");
        let snapshot = store.work_snapshot();
        let mut document: Value = serde_json::from_str(&store.state_json()).expect("scenario");
        document["species"][0]["file"] = json!("missing-save-timing-model");
        assert!(store.load_state(&document.to_string()).is_err());
        store.set_medium_limit("glc__D", "3").expect("later input");
        store.mark_scenario_snapshot_saved(&snapshot);
        assert!(store.unsaved_work().scenario);
        store.set_medium_limit("glc__D", "4").expect("restored input");
        assert!(!store.unsaved_work().scenario);
    }
}
