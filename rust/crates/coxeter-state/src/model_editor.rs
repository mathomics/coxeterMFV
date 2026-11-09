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
        let mut state = StatusState::Ok;
        let mut message = format!("Reaction {} created in {}{added}.", reaction.id, sp.id);
        if add_to_map && map_result.is_none() {
            state = StatusState::Warn;
            message = format!(
                "Reaction {} created in {}, but not drawn. Add its compartments or free some map space.",
                reaction.id, sp.id,
            );
        }
        self.set_status_message(state, message);
        self.emit("model-edit");
        Ok(CreatedReaction { reaction, merged_id, map_result })
    }
}

#[cfg(test)]
mod creation_validation_tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;

    fn make_store() -> Store {
        let mut store = Store::with_dir(coxeter_core::testkit::repo_root());
        store.init().expect("bundled models");
        store.use_scenario("solo-ecoli").expect("scenario");
        store
    }

    fn reaction_spec() -> Value {
        json!({
            "id": "QA_PGI", "name": "QA parallel glucose isomerase",
            "metabolites": "g6p_c:-1, f6p_c:1", "lowerBound": -10, "upperBound": 10,
            "geneReactionRule": "qa_a and (qa_b or qa_c)",
        })
    }

    #[test]
    fn rejected_creation_preserves_the_installed_calculation_and_drawing() {
        let mut store = make_store();
        let biology = store.model_editor_model().expect("model").to_json();
        let drawing = store.map.as_ref().expect("map").to_json();
        let solution = store.solution.clone().expect("solution");
        let bounds = store.problem.as_ref().expect("problem").snapshot_bounds();
        let work = store.unsaved_work();
        let version = store.map_history.version();
        for fields in [
            json!({"id": "PGI"}), json!({"metabolites": "missing_c:-1, f6p_c:1"}),
            json!({"metabolites": "g6p_c:-1, g6p_c:1"}), json!({"metabolites": "g6p_c:0, f6p_c:1"}),
            json!({"lowerBound": 20, "upperBound": 10}), json!({"lowerBound": ""}),
            json!({"geneReactionRule": "qa_a and (qa_b"}),
        ] {
            let mut spec = reaction_spec();
            for (key, value) in fields.as_object().expect("fields") {
                spec[key] = value.clone();
            }
            assert!(store.create_model_reaction(&spec, true).is_err());
            assert_eq!(store.model_editor_model().expect("model").to_json(), biology);
            assert_eq!(store.map.as_ref().expect("map").to_json(), drawing);
            assert!(Arc::ptr_eq(store.solution.as_ref().expect("solution"), &solution));
            assert_eq!(store.problem.as_ref().expect("problem").snapshot_bounds(), bounds);
            assert_eq!(store.unsaved_work(), work);
            assert_eq!(store.map_history.version(), version);
        }
    }

    #[test]
    fn valid_creation_reaches_the_lp_while_map_undo_leaves_the_biological_edit_intact() {
        let mut store = make_store();
        let created = store.create_model_reaction(&reaction_spec(), true).expect("new reaction");
        assert_eq!(created.merged_id, "eco@QA_PGI");
        assert!(created.map_result.is_some());
        assert!(store.problem.as_ref().expect("problem").reaction_index(&created.merged_id).is_some());
        assert!(store.map.as_ref().expect("map").reactions().contains_key(&created.merged_id));
        let biology = store.model_editor_model().expect("model").to_json();
        store.undo_map().expect("undo drawing insertion");
        assert!(!store.map.as_ref().expect("map").reactions().contains_key(&created.merged_id));
        assert!(store.problem.as_ref().expect("problem").reaction_index(&created.merged_id).is_some());
        assert_eq!(store.model_editor_model().expect("model").to_json(), biology);
        store.redo_map().expect("redo drawing insertion");
        assert!(store.map.as_ref().expect("map").reactions().contains_key(&created.merged_id));
        store.toggle_gene_knockout("eco@qa_b").expect("knock out one isozyme");
        assert!(!store.gene_closed_reactions().contains_key(&created.merged_id));
        store.toggle_gene_knockout("eco@qa_c").expect("knock out second isozyme");
        assert!(store.gene_closed_reactions().contains_key(&created.merged_id));
        let bundle = store.reproducibility_bundle().expect("bundle");
        let replay = store.import_reproducibility_bundle(&bundle).expect("round trip");
        assert_eq!(replay["replay"]["pass"], true);
        assert!(store.gene_closed_reactions().contains_key(&created.merged_id));
        assert_eq!(store.model_editor_model().expect("model").to_json()["reactions"], biology["reactions"]);
    }

    #[test]
    fn creation_stays_in_its_source_model_when_another_scenario_is_opened_and_returned() {
        for kind in ["metabolite", "reaction"] {
            let mut store = make_store();
            if kind == "reaction" {
                store.create_model_reaction(&reaction_spec(), true).expect("new reaction");
            } else {
                store.create_model_metabolite(&json!({
                    "id": "qa_metabolite_c", "name": "QA metabolite", "compartment": "c",
                })).expect("new metabolite");
            }
            store.use_scenario("solo-levadura").expect("another scenario");
            assert!(store.problem.as_ref().expect("yeast problem").reaction_index("eco@QA_PGI").is_none());
            assert!(store.model_editor_model().expect("yeast model").reactions.get("QA_PGI").is_none());
            store.use_scenario("solo-ecoli").expect("return to source");
            let model = store.model_editor_model().expect("edited model");
            assert_eq!(model.reactions.contains_key("QA_PGI"), kind == "reaction");
            assert_eq!(model.metabolites.contains_key("qa_metabolite_c"), kind == "metabolite");
            assert!(store.unsaved_work().model);
            assert!(store.solution.as_ref().expect("solution").is_optimal());
        }
    }

    #[test]
    fn successive_valid_creations_retain_both_biological_and_drawing_edits() {
        let mut store = make_store();
        store.create_model_reaction(&reaction_spec(), true).expect("first reaction");
        let mut second = reaction_spec();
        second["id"] = json!("QA_SECOND");
        second["lowerBound"] = json!(-5);
        second["upperBound"] = json!(5);
        store.create_model_reaction(&second, true).expect("second reaction");
        for id in ["QA_PGI", "QA_SECOND"] {
            assert!(store.model_editor_model().expect("model").reactions.contains_key(id));
            assert!(store.problem.as_ref().expect("problem").reaction_index(&format!("eco@{id}")).is_some());
            assert!(store.map.as_ref().expect("map").reactions().contains_key(&format!("eco@{id}")));
        }
        assert_eq!(store.selected_reaction.as_deref(), Some("eco@QA_SECOND"));
        assert!(store.solution.as_ref().expect("solution").is_optimal());
        assert!(store.unsaved_work().model);
    }

    #[test]
    fn a_rejected_coefficient_edit_preserves_an_accepted_creation_in_the_same_context() {
        let mut store = make_store();
        store.create_model_reaction(&reaction_spec(), true).expect("accepted reaction");
        let biology = store.model_editor_model().expect("model").to_json();
        let drawing = store.map.as_ref().expect("map").to_json();
        let solution = store.solution.clone().expect("solution");
        let version = store.map_history.version();
        let mut rejected = reaction_spec();
        rejected["id"] = json!("QA_REJECTED");
        rejected["metabolites"] = json!({ "g6p_c": true, "f6p_c": 1 });
        assert!(store.create_model_reaction(&rejected, true).is_err());
        assert_eq!(store.model_editor_model().expect("model").to_json(), biology);
        assert_eq!(store.map.as_ref().expect("map").to_json(), drawing);
        assert!(Arc::ptr_eq(store.solution.as_ref().expect("retained result"), &solution));
        assert_eq!(store.map_history.version(), version);
        assert!(store.problem.as_ref().expect("problem").reaction_index("eco@QA_PGI").is_some());
        assert!(store.problem.as_ref().expect("problem").reaction_index("eco@QA_REJECTED").is_none());
    }
}

#[cfg(test)]
mod default_placement_tests {
    use super::*;
    use coxeter_core::mapdata::Point;
    use coxeter_map::editable_map::EditableMap;
    use coxeter_map::layout::LayoutOptions;
    use serde_json::json;
    use std::sync::Arc;

    fn make_store() -> Store {
        let mut store = Store::with_dir(coxeter_core::testkit::repo_root());
        store.init().expect("bundled models");
        store.use_scenario("solo-ecoli").expect("scenario");
        store
    }

    fn empty_structure(store: &mut Store) {
        let community = store.community.as_ref().expect("community");
        store.map = Some(EditableMap::empty_from_community(community, &LayoutOptions::default()));
    }

    fn reaction_spec() -> Value {
        json!({ "id": "QA_PGI", "metabolites": "g6p_c:-1, f6p_c:1", "lowerBound": -10, "upperBound": 10 })
    }

    #[test]
    fn creation_on_an_empty_drawing_reports_that_requested_map_insertion_did_not_happen() {
        let mut store = make_store();
        store.show_empty_map();
        let drawing = store.map.as_ref().expect("map").to_json();
        let version = store.map_history.version();
        let result = store.create_model_reaction(&reaction_spec(), true).expect("reaction");
        assert!(result.map_result.is_none());
        assert_eq!(store.status().state, StatusState::Warn);
        assert!(store.status().message.contains("created"));
        assert!(store.status().message.contains("not drawn"));
        assert_eq!(store.map.as_ref().expect("map").to_json(), drawing);
        assert_eq!(store.map_history.version(), version);
        assert!(store.model_editor_model().expect("model").reactions.contains_key("QA_PGI"));
        assert!(store.problem.as_ref().expect("problem").reaction_index("eco@QA_PGI").is_some());
        assert!(store.solution.as_ref().expect("solution").is_optimal());
    }

    #[test]
    fn creation_without_map_insertion_on_an_empty_drawing_remains_an_ordinary_success() {
        let mut store = make_store();
        store.show_empty_map();
        let drawing = store.map.as_ref().expect("map").to_json();
        let result = store.create_model_reaction(&reaction_spec(), false).expect("reaction");
        assert!(result.map_result.is_none());
        assert_eq!(store.status().state, StatusState::Ok);
        assert_eq!(store.map.as_ref().expect("map").to_json(), drawing);
        assert!(store.problem.as_ref().expect("problem").reaction_index("eco@QA_PGI").is_some());
        assert!(store.solution.as_ref().expect("solution").is_optimal());
    }

    #[test]
    fn default_internal_additions_use_separate_space_while_preserving_all_previous_geometry() {
        let mut store = make_store();
        empty_structure(&mut store);
        let solution = store.solution.clone().expect("solution");
        for id in ["eco@PGI", "eco@PFK", "eco@GAPD"] {
            let before = store.map.clone().expect("map");
            let added = store.add_map_reaction(id, &PlaceOptions::default()).expect("placement");
            let map = store.map.as_ref().expect("map");
            let current = map.reaction(id).expect("reaction");
            for prior in before.reactions().values() {
                assert!((current.x - prior.x).hypot(current.y - prior.y) >= 20.0);
            }
            for node_id in &added.nodes {
                let node = map.node(node_id).expect("new node");
                for prior in before.nodes().values() {
                    assert!((node.x - prior.x).hypot(node.y - prior.y) >= node.r + prior.r);
                }
            }
            assert_eq!(map.cells(), before.cells());
            assert_eq!(&map.segments()[..before.segments().len()], before.segments());
            for node in before.nodes().values() {
                assert_eq!(map.node(&node.id), Some(node));
            }
            for reaction in before.reactions().values() {
                assert_eq!(map.reaction(&reaction.id), Some(reaction));
            }
        }
        assert!(Arc::ptr_eq(store.solution.as_ref().expect("solution"), &solution));
    }

    #[test]
    fn default_transport_additions_remain_on_their_membrane_without_stacking_their_markers() {
        let mut store = make_store();
        empty_structure(&mut store);
        store.add_map_reaction("eco@GLCptspp", &PlaceOptions::default()).expect("first");
        store.add_map_reaction("eco@H2Otpp", &PlaceOptions::default()).expect("second");
        let map = store.map.as_ref().expect("map");
        let first = map.reaction("eco@GLCptspp").expect("first");
        let second = map.reaction("eco@H2Otpp").expect("second");
        assert!(first.on_membrane && second.on_membrane);
        assert_eq!(first.boundary_id, second.boundary_id);
        assert!((first.x - second.x).hypot(first.y - second.y) >= 20.0);
    }

    #[test]
    fn an_explicit_requested_position_is_honored_even_when_another_marker_is_already_there() {
        let mut store = make_store();
        empty_structure(&mut store);
        let map = store.map.as_ref().expect("map");
        let rect = map.cells()[0].compartments.iter().find(|c| c.id == "c").expect("cytosol").rect;
        let anchor = Point::new(rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
        let options = PlaceOptions { anchor: Some(anchor), ..PlaceOptions::default() };
        for id in ["eco@PGI", "eco@PFK"] {
            store.add_map_reaction(id, &options).expect("placement");
            let reaction = store.map.as_ref().expect("map").reaction(id).expect("reaction");
            assert_eq!(Point::new(reaction.x, reaction.y), anchor);
        }
    }

    #[test]
    fn nonfinite_requested_positions_cannot_add_broken_drawing_objects() {
        let mut store = make_store();
        empty_structure(&mut store);
        let drawing = store.map.as_ref().expect("map").to_json();
        let version = store.map_history.version();
        for anchor in [Point::new(f64::NAN, 10.0), Point::new(10.0, f64::INFINITY)] {
            let options = PlaceOptions { anchor: Some(anchor), ..PlaceOptions::default() };
            assert!(store.add_map_reaction("eco@PFK", &options).is_none());
            assert_eq!(store.map.as_ref().expect("map").to_json(), drawing);
            assert_eq!(store.map_history.version(), version);
        }
    }

    #[test]
    fn default_placement_can_add_every_reaction_that_fits_at_an_explicit_position() {
        let store = make_store();
        let community = store.community.as_ref().expect("community");
        let mut automatic = EditableMap::empty_from_community(community, &LayoutOptions::default());
        let mut explicit = EditableMap::empty_from_community(community, &LayoutOptions::default());
        let options = PlaceOptions { anchor: Some(Point::new(500.0, 500.0)), ..PlaceOptions::default() };
        let mut missing = Vec::new();
        for id in community.merged.reactions.keys() {
            if explicit.add_reaction(community, id, &options).is_none() {
                continue;
            }
            if automatic.add_reaction(community, id, &PlaceOptions::default()).is_none() {
                missing.push(id.clone());
            }
        }
        assert!(missing.is_empty(), "Missing automatic additions: {missing:?}");
    }
}

#[cfg(test)]
mod placement_region_tests {
    use super::*;
    use coxeter_core::mapdata::{NodeKind, Point, Rect};
    use coxeter_map::drag::{Placeable, node_fits_at};
    use coxeter_map::editable_map::EditableMap;
    use coxeter_map::layout::LayoutOptions;
    use serde_json::json;
    use std::sync::Arc;

    fn make_store() -> Store {
        let mut store = Store::with_dir(coxeter_core::testkit::repo_root());
        store.init().expect("bundled models");
        store.use_scenario("solo-ecoli").expect("scenario");
        let community = store.community.as_ref().expect("community");
        store.map = Some(EditableMap::empty_from_community(community, &LayoutOptions::default()));
        store.map_history.clear();
        store
    }

    fn assert_connected(map: &EditableMap) {
        for segment in map.segments() {
            let Some(id) = &segment.node_id else {
                continue;
            };
            let node = map.node(id).expect("node");
            let point = if segment.side.as_deref() == Some("substrate") { segment.from } else { segment.to };
            assert!((node.x - point.x).hypot(node.y - point.y) < 1e-8, "{id}");
        }
    }

    #[test]
    fn reaction_insertion_stays_inside_an_imported_compartment_smaller_than_its_padding() {
        let mut store = make_store();
        let mut drawing = store.map.as_ref().expect("map").to_json();
        let rect = Rect::new(300.0, 300.0, 12.0, 10.0);
        let compartment = drawing["cells"][0]["compartments"].as_array_mut().expect("compartments")
            .iter_mut().find(|c| c["id"] == "c").expect("cytosol");
        compartment["rect"] = json!(rect);
        store.load_map_json(&drawing.to_string()).expect("small imported drawing");
        let solution = store.solution.clone().expect("solution");
        store.add_map_reaction("eco@PGI", &PlaceOptions::default()).expect("placement");
        let map = store.map.as_ref().expect("map");
        let reaction = map.reaction("eco@PGI").expect("reaction");
        assert_eq!(Point::new(reaction.x, reaction.y), Point::new(306.0, 305.0));
        for node in map.nodes().values() {
            assert!(node.x >= rect.x && node.x <= rect.x + rect.w, "{}", node.id);
            assert!(node.y >= rect.y && node.y <= rect.y + rect.h, "{}", node.id);
        }
        assert_connected(map);
        assert!(Arc::ptr_eq(store.solution.as_ref().expect("solution"), &solution));
    }

    #[test]
    fn outer_membrane_insertion_keeps_metabolites_out_of_the_nested_cytosol() {
        let mut store = make_store();
        let solution = store.solution.clone().expect("solution");
        for id in ["eco@GLCtex", "eco@H2Otex"] {
            store.add_map_reaction(id, &PlaceOptions::default()).expect("placement");
        }
        let map = store.map.as_ref().expect("map");
        for node in map.nodes().values() {
            assert!(node_fits_at(map, Placeable::from(node), node.x, node.y, None), "{}", node.id);
        }
        assert_connected(map);
        assert!(Arc::ptr_eq(store.solution.as_ref().expect("solution"), &solution));
    }

    #[test]
    fn an_internal_periplasm_reaction_keeps_its_marker_and_metabolites_out_of_the_cytosol() {
        let mut store = make_store();
        store.create_model_reaction(&json!({
            "id": "QA_PERIPLASM", "metabolites": "glc__D_p:-1, h_p:1", "lowerBound": 0, "upperBound": 1000,
        }), false).expect("periplasmic reaction");
        store.add_map_reaction("eco@QA_PERIPLASM", &PlaceOptions::default()).expect("placement");
        let map = store.map.as_ref().expect("map");
        let reaction = map.reaction("eco@QA_PERIPLASM").expect("reaction");
        let marker = Placeable {
            kind: &NodeKind::Metabolite, r: 5.0, cell_id: Some("eco"), compartment: Some("p"),
            x: reaction.x, y: reaction.y,
        };
        assert!(node_fits_at(map, marker, marker.x, marker.y, None), "reaction marker");
        for node in map.nodes().values() {
            assert!(node_fits_at(map, Placeable::from(node), node.x, node.y, None), "{}", node.id);
        }
        assert_connected(map);
        assert!(store.solution.as_ref().expect("solution").is_optimal());
    }

    #[test]
    fn insertion_near_a_compartment_edge_contains_secondary_metabolites_and_their_branch_endpoints() {
        let mut store = make_store();
        let rect = store.map.as_ref().expect("map").cells()[0].compartments.iter()
            .find(|c| c.id == "c").expect("cytosol").rect;
        let options = PlaceOptions {
            anchor: Some(Point::new(rect.x + rect.w - 20.0, rect.y + rect.h - 20.0)),
            ..PlaceOptions::default()
        };
        store.add_map_reaction("eco@PFK", &options).expect("placement");
        let map = store.map.as_ref().expect("map");
        for node in map.nodes().values() {
            assert!(node_fits_at(map, Placeable::from(node), node.x, node.y, None), "{}", node.id);
        }
        assert_connected(map);
    }

    #[test]
    fn insertion_repairs_only_new_objects_and_preserves_the_explicitly_reused_metabolite() {
        let mut store = make_store();
        store.add_map_reaction("eco@PGI", &PlaceOptions::default()).expect("PGI");
        let drawing = store.map.clone().expect("map");
        let anchor = drawing.nodes().values().find(|n| n.merged_id.as_deref() == Some("eco@f6p_c")).expect("anchor");
        let biology = store.model_editor_model().expect("model").to_json();
        let solution = store.solution.clone().expect("solution");
        let options = PlaceOptions {
            anchor: Some(Point::new(anchor.x + 96.0, anchor.y)), anchor_node_id: Some(anchor.id.clone()),
            placement_direction: Some(Point::new(1.0, 0.0)), ..PlaceOptions::default()
        };
        let added = store.add_map_reaction("eco@PFK", &options).expect("PFK");
        assert_eq!(added.anchor_node.as_deref(), Some(anchor.id.as_str()));
        let map = store.map.as_ref().expect("map");
        for previous in drawing.nodes().values() {
            let mut expected = previous.clone();
            if previous.id == anchor.id {
                expected.shared_instance = Some(true);
            }
            assert_eq!(map.node(&previous.id), Some(&expected));
        }
        assert_eq!(&map.segments()[..drawing.segments().len()], drawing.segments());
        assert_eq!(store.model_editor_model().expect("model").to_json(), biology);
        assert!(Arc::ptr_eq(store.solution.as_ref().expect("solution"), &solution));
        assert_connected(map);
    }

    #[test]
    fn an_automatic_insertion_with_no_distinct_marker_position_preserves_the_drawing_and_redo() {
        let mut store = make_store();
        store.map.as_mut().expect("map").edit_graph(|graph| {
            graph.cells[0].compartments.iter_mut().find(|c| c.id == "c").expect("cytosol").rect =
                Rect::new(300.0, 300.0, 40.0, 40.0);
        });
        store.add_map_reaction("eco@PGI", &PlaceOptions::default()).expect("PGI");
        store.add_map_reaction("eco@GLCtex", &PlaceOptions::default()).expect("GLCtex");
        store.undo_map().expect("undo");
        let drawing = store.map.as_ref().expect("map").to_json();
        let version = store.map_history.version();
        let solution = store.solution.clone().expect("solution");
        assert!(store.add_map_reaction("eco@PFK", &PlaceOptions::default()).is_none());
        assert_eq!(store.map.as_ref().expect("map").to_json(), drawing);
        assert_eq!(store.map_history.version(), version);
        assert!(store.map_history.can_redo());
        assert!(Arc::ptr_eq(store.solution.as_ref().expect("solution"), &solution));
    }
}
