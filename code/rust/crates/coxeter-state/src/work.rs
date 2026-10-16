//! Saved and unsaved work (the "saved / unsaved" section of `store.js`).

use coxeter_core::js;
use coxeter_map::layout_settings::{LayoutSettings, RawLayoutSettings};
use serde_json::{Map, Value, json};

use crate::store::Store;
use crate::types::{SavedWork, UnsavedWork};
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
                "objective": self.objective,
                "productTarget": self.product_target,
                "parsimonious": self.parsimonious,
                "pfbaBiomassPercent": num_json(self.pfba_biomass_percent),
                "loopless": self.loopless,
                "fvaEnabled": self.fva_enabled,
                "steadyComConfig": self.steady_com_config.to_json(),
                "layoutSettings": layout_settings_json(&self.layout_settings),
                "reactionColorMode": self.view.reaction_color_mode.as_str(),
                "lines": [self.view.width_by_flux, self.view.dash_no_flux, self.view.line_hops],
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

    /// `markMapSaved()`: the map on screen is now the one in a file, after
    /// Save map or opening one.
    pub fn mark_map_saved(&mut self) {
        self.map_history.mark_saved();
        self.track_work_context();
        let shared = self.shared_signature();
        if let Some(saved) = self.saved_work.as_mut() {
            saved.shared = shared;
        }
    }

    /// `markScenarioSaved()`: the scenario settings, and what they share with
    /// the map file, are now in a file.
    pub fn mark_scenario_saved(&mut self) {
        self.track_work_context();
        let (scenario, shared) = (self.scenario_signature(), self.shared_signature());
        if let Some(saved) = self.saved_work.as_mut() {
            saved.scenario = scenario;
            saved.shared = shared;
        }
    }

    /// `markModelSaved(model)`: one model has been written out (the model
    /// editor exports the selected species); its edits are saved, those of
    /// any other model are not. `None` is the model of the editor's species.
    pub fn mark_model_saved(&mut self, file: Option<&str>) {
        let serial = match file {
            Some(f) => self.models.serial(f),
            None => self.model_editor_species().and_then(|sp| self.models.serial(&sp.file)),
        };
        if let Some(serial) = serial {
            let edits = self.model_edits.get(&serial).copied().unwrap_or(0);
            self.model_edits_saved.insert(serial, edits);
        }
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
