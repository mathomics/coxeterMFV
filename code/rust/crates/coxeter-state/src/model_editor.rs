//! The biological model editor (the "biological model editor" section of
//! `store.js`): MODEL operations, which change a model and rebuild the
//! community.

use coxeter_core::model::{Metabolite, Model, Reaction};
use coxeter_io::model_edit::{add_metabolite_to_model, add_reaction_to_model};
use coxeter_map::place_reaction::{PlaceOptions, Placed};
use serde_json::Value;

use crate::error::{Result, StoreError};
use crate::store::Store;
use crate::types::{RebuildOptions, Species, StatusState};

/// What `createModelReaction` made.
#[derive(Debug, Clone)]
pub struct CreatedReaction {
    pub reaction: Reaction,
    pub merged_id: String,
    pub map_result: Option<Placed>,
}

impl Store {
    /// `modelEditorSpecies()`: the species the editor works on.
    pub fn model_editor_species(&self) -> Option<&Species> {
        self.model_editor_species_id
            .as_ref()
            .and_then(|id| self.species_by_id(id))
            .or(self.species.first())
    }

    /// `modelEditorModel()`.
    pub fn model_editor_model(&self) -> Option<&Model> {
        self.model_editor_species().and_then(|sp| self.models.get(&sp.file))
    }

    /// `setModelEditorSpecies(id)`.
    pub fn set_model_editor_species(&mut self, id: &str) -> Option<Species> {
        if !self.species.iter().any(|sp| sp.id == id) {
            return None;
        }
        self.model_editor_species_id = Some(id.to_string());
        self.emit("model-editor");
        self.model_editor_species().cloned()
    }

    /// `setModelEditorMode(mode)`: `metabolite`, `reaction` or none.
    pub fn set_model_editor_mode(&mut self, mode: Option<&str>) -> Result<Option<String>> {
        if mode.is_some() {
            self.assert_published_study_constraints()?;
        }
        self.model_editor_mode = mode.filter(|m| ["metabolite", "reaction"].contains(m)).map(str::to_string);
        self.emit("model-editor");
        Ok(self.model_editor_mode.clone())
    }

    fn editor_target(&self) -> Result<Species> {
        let sp = self.model_editor_species().cloned();
        match sp {
            Some(sp) if self.models.contains(&sp.file) => Ok(sp),
            _ => Err(StoreError::msg("Select a species/model.")),
        }
    }

    /// `createModelMetabolite(spec)`: add a metabolite to the editor's model
    /// (`spec` as the form gives it: `{ id, name, compartment, formula, … }`).
    pub fn create_model_metabolite(&mut self, spec: &Value) -> Result<Metabolite> {
        self.assert_published_study_constraints()?;
        self.assert_model_operation("createModelMetabolite")?;
        let sp = self.editor_target()?;
        let model = self.models.get_mut(&sp.file).ok_or_else(|| StoreError::msg("Select a species/model."))?;
        let metabolite = add_metabolite_to_model(model, spec)?.clone();
        self.count_model_edit(&sp.file);
        self.rebuild(RebuildOptions { relayout: false, ..RebuildOptions::default() })?;
        self.set_status_message(StatusState::Ok, format!("Metabolite {} created in {}.", metabolite.id, sp.id));
        self.emit("model-edit");
        Ok(metabolite)
    }

    /// `createModelReaction(spec, { addToMap })`.
    pub fn create_model_reaction(&mut self, spec: &Value, add_to_map: bool) -> Result<CreatedReaction> {
        self.assert_published_study_constraints()?;
        self.assert_model_operation("createModelReaction")?;
        let sp = self.editor_target()?;
        let model = self.models.get_mut(&sp.file).ok_or_else(|| StoreError::msg("Select a species/model."))?;
        let reaction = add_reaction_to_model(model, spec)?.clone();
        self.count_model_edit(&sp.file);
        self.rebuild(RebuildOptions { relayout: false, ..RebuildOptions::default() })?;
        let merged_id = format!("{}@{}", sp.id, reaction.id);
        let mut map_result = None;
        if add_to_map && self.map.is_some() && self.community.as_ref().is_some_and(|c| c.merged.reactions.contains_key(&merged_id)) {
            map_result = self.add_map_reaction(&merged_id, &PlaceOptions::default());
            if map_result.is_some() {
                self.select(Some(&merged_id));
            }
        }
        let added = if map_result.is_some() { " and added to the map" } else { "" };
        self.set_status_message(StatusState::Ok, format!("Reaction {} created in {}{added}.", reaction.id, sp.id));
        self.emit("model-edit");
        Ok(CreatedReaction { reaction, merged_id, map_result })
    }
}
