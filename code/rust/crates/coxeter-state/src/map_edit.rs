//! Editing the drawing: history, selection, labels, cells and compartments,
//! layouts, reactions, metabolites and texts (the map sections of
//! `store.js`).
//!
//! These are the methods the map canvas drives, by their JavaScript names in
//! snake_case. A gesture captures the drawing with
//! [`Store::begin_map_edit`], moves things with the methods that neither
//! record history nor emit (`move_node`, `move_reaction`,
//! `set_reaction_angle`, `move_map_text`, `edit_geometry_rect`, …) and ends
//! with [`Store::commit_map_edit`], which records one undo step. Everything
//! else is a complete edit: one undo step and an event.
//!
//! None of this touches the model, the LP or the solution: the map is
//! geometry and membership; the numbers live elsewhere.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use coxeter_core::community::CommunityModel;
use coxeter_core::compartments::cell_compartment_name;
use coxeter_core::js;
use coxeter_core::label_sources::{EntityKind, LabelEntity, LabelObject, LabelSources, label_source_name, object_label, object_label_sources};
use coxeter_core::mapdata::{Canvas, MapText, Node, Point, Rect};
use coxeter_core::text_style::{clamp_text_size, normalize_text_font};
use coxeter_map::compartment_reactions::reactions_in_compartment;
use coxeter_map::core_metabolism_overview::{PathwayGroup, build_core_metabolism_overview, core_overview_support, OverviewSupport};
use coxeter_map::editable_map::{
    AddedShell, CellSpec, ColorChange, CompartmentSpec, DuplicateOptions, EditableMap, MergeResult, RemovedReaction, TextOptions,
};
use coxeter_core::geometry::compartment_shape_preset;
use coxeter_map::editable_map::RemovedCompartment;
use coxeter_map::geometry_edit::{GeometryEdit, ShapeEdit, ShapePatch};
use coxeter_map::layout_settings::{RawLayoutSettings, layout_engine_options, layout_preset, normalize_layout_settings};
use coxeter_map::multi_selection::{ItemKind, Moved, SelectionRect, empty_selection, selection_size};
use coxeter_map::place_reaction::{PlaceOptions, Placed};
use coxeter_map::reaction_edit::Handle;
use coxeter_render::reaction_colors::{PathwayRibbonMode, category_color};
use coxeter_render::state::GeometrySelection;
use indexmap::{IndexMap, IndexSet};
use serde_json::{Map, Value};

use crate::jobs::{JobContext, JobKind, JobMessage, JobPayload};
use crate::store::{AUTO_DRAW_METABOLITE_LIMIT, AUTO_DRAW_REACTION_LIMIT, Store};
use crate::types::*;
use crate::util::normalize_search;

/// A full-map layout running in the background.
pub(crate) struct LayoutJob {
    pub problem_serial: u64,
    pub cancel: Arc<AtomicBool>,
}

/// What `showCoreMetabolismOverview` drew.
#[derive(Debug, Clone, PartialEq)]
pub struct CoreOverviewResult {
    pub placed: usize,
    pub pathway_groups: Vec<PathwayGroup>,
}

/// `{ fill, stroke }` of `setMapGeometryColors`: each may be left alone,
/// reset to the theme, or set to a `#rrggbb` colour (anything else is left
/// alone).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GeometryColors {
    pub fill: ColorChange,
    pub stroke: ColorChange,
}

/// The label sources a map object carries itself (`object.labelSources`).
fn own_label_sources(extra: &Map<String, Value>) -> Option<LabelSources> {
    let Value::Object(raw) = extra.get("labelSources")? else { return None };
    let mut out = LabelSources::new();
    for (k, v) in raw {
        let values: Vec<String> = match v {
            Value::Array(items) => items.iter().map(js::to_string).collect(),
            Value::Null => continue,
            other => vec![js::to_string(other)],
        };
        out.insert(k.clone(), values);
    }
    Some(out)
}

impl Store {
    // History

    /// `beginMapEdit(label)`: capture the current drawing before a
    /// multi-step edit such as dragging.
    pub fn begin_map_edit(&self, label: &str) -> Option<MapTransaction> {
        self.map.as_ref().map(|m| MapTransaction { label: label.to_string(), before: m.to_json() })
    }

    /// `commitMapEdit(transaction)`: record a previously captured edit if it
    /// actually changed the map.
    pub fn commit_map_edit(&mut self, transaction: Option<MapTransaction>) -> bool {
        let (Some(tx), Some(map)) = (transaction, self.map.as_ref()) else { return false };
        let changed = self.map_history.record(&tx.before, &map.to_json(), &tx.label);
        if changed {
            self.emit("history");
        }
        changed
    }

    /// `canUndoMap`.
    pub fn can_undo_map(&self) -> bool {
        self.map_history.can_undo()
    }

    /// `canRedoMap`.
    pub fn can_redo_map(&self) -> bool {
        self.map_history.can_redo()
    }

    /// Record `before` → the current map as one undo step.
    fn record_map(&mut self, before: &Value, label: &str) -> bool {
        let Some(map) = self.map.as_ref() else { return false };
        let after = map.to_json();
        self.map_history.record(before, &after, label)
    }

    /// `undoMap()`: the label of the edit taken back.
    pub fn undo_map(&mut self) -> Option<String> {
        let current = self.map.as_ref()?.to_json();
        let entry = self.map_history.undo(&current)?;
        self.map = EditableMap::from_json(&entry.doc).ok();
        self.selected_reaction = None;
        self.selected_entity = None;
        self.variability = None;
        self.emit("map");
        Some(entry.label)
    }

    /// `redoMap()`.
    pub fn redo_map(&mut self) -> Option<String> {
        let current = self.map.as_ref()?.to_json();
        let entry = self.map_history.redo(&current)?;
        self.map = EditableMap::from_json(&entry.doc).ok();
        self.selected_reaction = None;
        self.selected_entity = None;
        self.variability = None;
        self.emit("map");
        Some(entry.label)
    }

    // Multi-selection

    /// `multiSelectionSize`.
    pub fn multi_selection_size(&self) -> usize {
        selection_size(&self.multi_selection)
    }

    fn multi_selection_bucket(&mut self, kind: &str) -> &mut IndexSet<String> {
        match kind {
            "reaction" => &mut self.multi_selection.reactions,
            "text" => &mut self.multi_selection.texts,
            _ => &mut self.multi_selection.nodes,
        }
    }

    /// `isMultiSelected(kind, id)`.
    pub fn is_multi_selected(&self, kind: &str, id: &str) -> bool {
        match kind {
            "reaction" => self.multi_selection.reactions.contains(id),
            "text" => self.multi_selection.texts.contains(id),
            _ => self.multi_selection.nodes.contains(id),
        }
    }

    /// `clearMultiSelection({ emit })`.
    pub fn clear_multi_selection(&mut self, emit: bool) -> bool {
        if self.multi_selection_size() == 0 {
            return false;
        }
        self.multi_selection = empty_selection();
        if emit {
            self.emit("multi-selection");
        }
        true
    }

    /// `toggleMultiSelection(kind, id)`: `kind` is `node`, `reaction` or `text`.
    pub fn toggle_multi_selection(&mut self, kind: &str, id: &str) {
        if id.is_empty() || !["node", "reaction", "text"].contains(&kind) {
            return;
        }
        let bucket = self.multi_selection_bucket(kind);
        if !bucket.shift_remove(id) {
            bucket.insert(id.to_string());
        }
        self.emit("multi-selection");
    }

    /// `selectInRect(rect, { replace })`: what a selection rectangle catches.
    /// Secondary nodes hidden by the Cofactors toggle are not caught.
    pub fn select_in_rect(&mut self, rect: SelectionRect, replace: bool) -> &coxeter_map::Selection {
        let Some(map) = self.map.as_ref() else { return &self.multi_selection };
        let found = map.selectable_in_rect(rect, self.view.show_currency);
        if replace {
            self.multi_selection = empty_selection();
        }
        self.multi_selection.nodes.extend(found.nodes);
        self.multi_selection.reactions.extend(found.reactions);
        self.multi_selection.texts.extend(found.texts);
        self.multi_selection.compartments.extend(found.compartments);
        self.emit("multi-selection");
        &self.multi_selection
    }

    /// `moveMultiSelection(dx, dy)`: part of a drag; no history, no event.
    /// The selection moves as one rigid body and carries the compartments in
    /// it (`drag_selection`), so what it holds stays together.
    pub fn move_multi_selection(&mut self, dx: f64, dy: f64) -> Option<Moved> {
        let selection = self.multi_selection.clone();
        self.map.as_mut()?.drag_selection(&selection, dx, dy)
    }

    /// `moveCompartmentLabel(cellId, compartmentId, x, y)`: part of a drag.
    pub fn move_compartment_label(&mut self, cell_id: Option<&str>, compartment_id: &str, x: f64, y: f64) -> Option<Point> {
        debug_assert!(crate::operation_boundary::is_map_operation("moveCompartmentLabel"));
        self.map.as_mut()?.set_compartment_label_position(cell_id, compartment_id, x, y)
    }

    /// `removeMultiSelection()`: remove the current multi-selection from the
    /// drawing only, as one undoable edit.
    pub fn remove_multi_selection(&mut self) -> Option<RemovedSelection> {
        if self.multi_selection_size() == 0 {
            return None;
        }
        let map = self.map.as_mut()?;
        let before = map.to_json();
        let mut removed = RemovedSelection::default();
        // Whole compartments first, with what is drawn in them: what is left
        // of the selection after that is what lies outside them.
        for (cell_id, compartment_id) in self.multi_selection.compartments.clone() {
            if cell_id.is_empty() {
                continue;
            }
            let Some(result) = map.remove_compartment(&cell_id, &compartment_id) else { continue };
            removed.compartments.extend(result.compartment_ids);
            removed.reactions.extend(result.reactions);
            removed.texts.extend(result.texts);
        }
        // Reactions next: their reaction-local visual nodes may disappear
        // with them. Then remove only still-existing, explicitly selected
        // node instances.
        for id in self.multi_selection.reactions.clone() {
            if map.remove_reaction(&id).is_some() {
                removed.reactions.push(id);
            }
        }
        for id in self.multi_selection.nodes.clone() {
            if !map.has_node(&id) || !map.can_remove_node(&id) {
                continue;
            }
            if map.remove_node(&id).is_some() {
                removed.nodes.push(id);
            }
        }
        // Annotations go with the rest, so a deleted pathway does not leave
        // its metabolite names behind, labelling reactions that are gone.
        for id in self.multi_selection.texts.clone() {
            if map.remove_text(&id).is_some() {
                removed.texts.push(id);
            }
        }
        if removed.reactions.is_empty() && removed.nodes.is_empty() && removed.texts.is_empty() && removed.compartments.is_empty() {
            return None;
        }
        self.record_map(&before, "Remove selection");
        self.multi_selection = empty_selection();
        if !removed.compartments.is_empty()
            && let Some(g) = &self.selected_geometry
            && !self.map.as_ref().is_some_and(|m| m.geometry_target(g.cell_id.as_deref(), &g.compartment_id).is_some())
        {
            self.selected_geometry = None;
        }
        if self.selected_reaction.as_ref().is_some_and(|r| removed.reactions.contains(r)) {
            self.selected_reaction = None;
            self.variability = None;
        }
        if let Some(entity) = &self.selected_entity {
            let gone = match entity.kind.as_str() {
                "reaction" => removed.reactions.contains(&entity.id),
                "metabolite" => !self.map.as_ref().is_some_and(|m| m.has_node(&entity.id)),
                "text" => removed.texts.contains(&entity.id),
                "compartment" => self.selected_geometry.is_none(),
                "cell" => !self.map.as_ref().is_some_and(|m| m.cell(&entity.id).is_some()),
                _ => false,
            };
            if gone {
                self.selected_entity = None;
            }
        }
        self.emit("multi-selection");
        self.emit("map");
        Some(removed)
    }

    /// `canArrangeSelection(action)`: two objects, or one reaction
    /// (Escher-style line).
    pub fn can_arrange_selection(&self, action: &str) -> bool {
        let Some(map) = self.map.as_ref() else { return false };
        if self.multi_selection_size() >= 2 {
            return true;
        }
        (action == "align-horizontal" || action == "align-vertical") && map.selection_has_line_reactions(&self.multi_selection)
    }

    /// `arrangeMultiSelection(action)`: `align-horizontal`, `align-vertical`,
    /// `align-left/right/top/bottom`, `distribute-horizontal/vertical`.
    pub fn arrange_multi_selection(&mut self, action: &str) -> Option<ArrangeResult> {
        if !self.can_arrange_selection(action) {
            return None;
        }
        let label = match action {
            "align-horizontal" => "Align horizontally",
            "align-vertical" => "Align vertically",
            "align-left" => "Align left",
            "align-right" => "Align right",
            "align-top" => "Align top",
            "align-bottom" => "Align bottom",
            "distribute-horizontal" => "Distribute horizontally",
            "distribute-vertical" => "Distribute vertically",
            _ => return None,
        };
        let selection = self.multi_selection.clone();
        let map = self.map.as_mut()?;
        let before = map.to_json();
        let mode = &action[action.find('-').map_or(0, |i| i + 1)..];
        // With reactions selected, Horizontal and Vertical work as in Escher:
        // each reaction comes with its metabolites and they all end on one
        // line. Transporters stay on their membranes, so a selection with no
        // other reaction is aligned as plain objects.
        let as_line = (mode == "horizontal" || mode == "vertical")
            && action.starts_with("align-")
            && map.selection_has_line_reactions(&selection);
        let mut result = ArrangeResult::default();
        if as_line {
            if let Some(line) = map.align_selection_in_line(&selection, mode) {
                result.moved.extend(line.nodes.into_iter().map(|id| ("node".to_string(), id)));
                result.moved.extend(line.reactions.into_iter().map(|id| ("reaction".to_string(), id)));
                result.moved.extend(line.texts.into_iter().map(|id| ("text".to_string(), id)));
                result.held = line.held;
            }
        } else {
            let arranged =
                if action.starts_with("align-") { map.align_selection(&selection, mode) } else { map.distribute_selection(&selection, mode) };
            result.moved = arranged.moved.into_iter().map(|(kind, id)| (kind.as_str().to_string(), id)).collect();
        }
        if !result.held.is_empty() {
            let n = result.held.len();
            self.set_status_message(
                StatusState::Warn,
                format!("{n} metabolite{} could not reach the line: a metabolite stays inside its compartment.", if n == 1 { "" } else { "s" }),
            );
        }
        result.changed = self.record_map(&before, label);
        if result.changed {
            self.emit("map");
        } else if !result.held.is_empty() {
            self.emit("status");
        }
        Some(result)
    }

    // Labels

    /// `mapLabelSource(kind)`: what a kind of object is labelled with,
    /// map-wide.
    pub fn map_label_source(&self, kind: &str) -> String {
        match &self.map {
            Some(map) => map.label_source(kind),
            None => if kind == "metabolite" { "base".into() } else { "id".into() },
        }
    }

    /// `mapLabelText(kind, id, source)`: the label an object is drawn with.
    pub fn map_label_text(&self, kind: &str, id: &str, source: Option<&str>) -> String {
        let Some(map) = self.map.as_ref() else { return String::new() };
        let merged = self.community.as_ref().map(|c| &c.merged);
        if kind == "metabolite" {
            let Some(node) = map.node(id) else { return String::new() };
            self.node_label(node, merged, source, &self.map_label_source(kind))
        } else {
            let Some(rxn) = map.reaction(id) else { return String::new() };
            let own = own_label_sources(&rxn.extra);
            let object = LabelObject {
                id: Some(&rxn.id),
                local_id: rxn.local_id.as_deref(),
                cell_id: rxn.cell_id.as_deref(),
                name: Some(&rxn.name),
                base: None,
                label_sources: own.as_ref(),
            };
            let entity = merged.and_then(|m| m.reactions.get(&rxn.id)).map(|r| LabelEntity {
                id: Some(&r.id),
                name: Some(&r.name),
                base: None,
                label_sources: Some(&r.label_sources),
            });
            let chosen = source
                .map(str::to_string)
                .or_else(|| rxn.extra.get("labelSource").filter(|v| !v.is_null()).map(js::to_string))
                .unwrap_or_else(|| self.map_label_source(kind));
            object_label(&object, entity.as_ref(), EntityKind::Reaction, &chosen)
        }
    }

    fn node_label(&self, node: &Node, merged: Option<&coxeter_core::model::Model>, source: Option<&str>, default: &str) -> String {
        let own = own_label_sources(&node.extra);
        let object = LabelObject {
            id: Some(&node.id),
            local_id: node.local_id.as_deref(),
            cell_id: node.cell_id.as_deref(),
            name: node.name.as_deref(),
            base: node.base.as_deref(),
            label_sources: own.as_ref(),
        };
        let entity = merged.and_then(|m| node.merged_id.as_ref().and_then(|id| m.metabolites.get(id))).map(|m| LabelEntity {
            id: Some(&m.id),
            name: Some(&m.name),
            base: Some(&m.base),
            label_sources: Some(&m.label_sources),
        });
        let chosen = source
            .map(str::to_string)
            .or_else(|| node.extra.get("labelSource").filter(|v| !v.is_null()).map(js::to_string))
            .unwrap_or_else(|| default.to_string());
        object_label(&object, entity.as_ref(), EntityKind::Metabolite, &chosen)
    }

    /// `entityLabel(kind, mergedId)`: a metabolite or reaction of the model
    /// named as the map names it, drawn or not: by the label source the map
    /// is set to (Map options ▸ Labels). A menu that offers things to add
    /// reads the way the map does, so a yeast drawn with names is not offered
    /// as `s_0034`. Also the model's own id and its name, for menus that show
    /// one beside the other.
    pub fn entity_label(&self, kind: &str, merged_id: &str) -> EntityLabel {
        let metabolite = kind == "metabolite";
        let Some(community) = self.community.as_ref() else {
            return EntityLabel { label: merged_id.to_string(), id: merged_id.to_string(), name: String::new() };
        };
        let merged = &community.merged;
        let (owner_id, owner_species) = if metabolite {
            community.metabolite_owner.get(merged_id).map(|o| (Some(o.original_id.clone()), o.species_id.clone())).unwrap_or((None, None))
        } else {
            community.reaction_owner.get(merged_id).map(|o| (Some(o.original_id.clone()), o.species_id.clone())).unwrap_or((None, None))
        };
        let met = metabolite.then(|| merged.metabolites.get(merged_id)).flatten();
        let rxn = (!metabolite).then(|| merged.reactions.get(merged_id)).flatten();
        let entity_id = met.map(|m| m.id.as_str()).or(rxn.map(|r| r.id.as_str()));
        let entity_name = met.map(|m| m.name.as_str()).or(rxn.map(|r| r.name.as_str()));
        let local_id = owner_id.or(entity_id.map(str::to_string)).unwrap_or_else(|| merged_id.to_string());
        let base = metabolite.then(|| coxeter_map::place_reaction::local_metabolite_base(community, merged_id, met));
        // The fields a drawn node or reaction carries, so both are labelled alike.
        let object = LabelObject {
            id: Some(merged_id),
            local_id: Some(&local_id),
            cell_id: owner_species.as_deref(),
            name: entity_name,
            base: base.as_deref(),
            label_sources: None,
        };
        let entity = match (met, rxn) {
            (Some(m), _) => Some(LabelEntity { id: Some(&m.id), name: Some(&m.name), base: Some(&m.base), label_sources: Some(&m.label_sources) }),
            (_, Some(r)) => Some(LabelEntity { id: Some(&r.id), name: Some(&r.name), base: None, label_sources: Some(&r.label_sources) }),
            _ => None,
        };
        let entity_kind = if metabolite { EntityKind::Metabolite } else { EntityKind::Reaction };
        let sources = object_label_sources(&object, entity.as_ref(), entity_kind);
        EntityLabel {
            label: object_label(&object, entity.as_ref(), entity_kind, &self.map_label_source(kind)),
            id: sources.get("id").cloned().unwrap_or_default(),
            name: entity_name.unwrap_or("").to_string(),
        }
    }

    /// `mapLabelOptions(kind, id)`: the sources an object (or, without an id,
    /// any object of that kind) can be labelled with.
    pub fn map_label_options(&self, kind: &str, id: Option<&str>) -> Vec<LabelOption> {
        let mut keys: IndexSet<String> =
            if kind == "metabolite" { ["base", "id", "name"] } else { ["id", "name", ""] }.iter().filter(|k| !k.is_empty()).map(|k| k.to_string()).collect();
        let merged = self.community.as_ref().map(|c| &c.merged);
        match id {
            Some(id) => {
                if let Some(map) = self.map.as_ref() {
                    let sources = if kind == "metabolite" {
                        map.node(id).map(|node| {
                            let own = own_label_sources(&node.extra);
                            let object = LabelObject {
                                id: Some(&node.id),
                                local_id: node.local_id.as_deref(),
                                cell_id: node.cell_id.as_deref(),
                                name: node.name.as_deref(),
                                base: node.base.as_deref(),
                                label_sources: own.as_ref(),
                            };
                            let entity = merged.and_then(|m| node.merged_id.as_ref().and_then(|i| m.metabolites.get(i))).map(|m| LabelEntity {
                                id: Some(&m.id),
                                name: Some(&m.name),
                                base: Some(&m.base),
                                label_sources: Some(&m.label_sources),
                            });
                            object_label_sources(&object, entity.as_ref(), EntityKind::Metabolite)
                        })
                    } else {
                        map.reaction(id).map(|rxn| {
                            let own = own_label_sources(&rxn.extra);
                            let object = LabelObject {
                                id: Some(&rxn.id),
                                local_id: rxn.local_id.as_deref(),
                                cell_id: rxn.cell_id.as_deref(),
                                name: Some(&rxn.name),
                                base: None,
                                label_sources: own.as_ref(),
                            };
                            let entity = merged.and_then(|m| m.reactions.get(&rxn.id)).map(|r| LabelEntity {
                                id: Some(&r.id),
                                name: Some(&r.name),
                                base: None,
                                label_sources: Some(&r.label_sources),
                            });
                            object_label_sources(&object, entity.as_ref(), EntityKind::Reaction)
                        })
                    };
                    if let Some(sources) = sources {
                        keys.extend(sources.keys().cloned());
                    }
                }
            }
            None => {
                if let Some(m) = merged {
                    if kind == "metabolite" {
                        for met in m.metabolites.values() {
                            keys.extend(met.label_sources.keys().cloned());
                        }
                    } else {
                        for r in m.reactions.values() {
                            keys.extend(r.label_sources.keys().cloned());
                        }
                    }
                }
            }
        }
        keys.into_iter().map(|k| LabelOption { label: label_source_name(&k), value: k }).collect()
    }

    /// `setMapLabelSource(kind, source, id)`: map-wide (no id) or for one
    /// object; `inherit` clears an object's own choice.
    pub fn set_map_label_source(&mut self, kind: &str, source: &str, id: Option<&str>) -> bool {
        if self.map.is_none() || !["metabolite", "reaction"].contains(&kind) {
            return false;
        }
        let options = self.map_label_options(kind, id);
        if source != "inherit" && !options.iter().any(|o| o.value == source) {
            return false;
        }
        let history = self.begin_map_edit("Change map labels");
        let map = self.map.as_mut().expect("checked");
        match id {
            Some(id) => {
                let value = if source == "inherit" { None } else { Some(source) };
                if !map.set_object_label_source(kind, id, value) {
                    return false;
                }
            }
            None => {
                if source == "inherit" {
                    return false;
                }
                map.set_label_source(kind, source);
            }
        }
        let changed = self.commit_map_edit(history);
        self.emit("map-labels");
        changed
    }

    /// `mapLabels(kind)`: the label of every drawn node or reaction.
    pub fn map_labels(&self, kind: &str) -> IndexMap<String, String> {
        let Some(map) = self.map.as_ref() else { return IndexMap::new() };
        let ids: Vec<String> = if kind == "metabolite" { map.node_ids() } else { map.reaction_ids() };
        ids.into_iter().map(|id| {
            let text = self.map_label_text(kind, &id, None);
            (id, text)
        }).collect()
    }

    // The map

    /// `setMap(map)`: replace what is drawn, without touching the model or
    /// the solution.
    ///
    /// This is the seam a map editor plugs into: the community, the LP and
    /// the current fluxes are all untouched, so swapping in a different
    /// subset of the same reactions redraws the screen and changes no number.
    pub fn set_map(&mut self, map: Option<EditableMap>) {
        self.map = map;
        self.multi_selection = empty_selection();
        if let Some(rid) = &self.selected_reaction {
            if !self.map.as_ref().is_some_and(|m| m.has_reaction(rid)) {
                self.selected_reaction = None;
                self.selected_entity = None;
                self.variability = None;
            }
        }
        self.emit("map");
    }

    /// `addMapReaction(reactionId, opts)`: add one reaction to the drawing.
    /// Graphical only: no rebuild, solve or relayout of existing geometry.
    pub fn add_map_reaction(&mut self, reaction_id: &str, opts: &PlaceOptions) -> Option<Placed> {
        let community = self.community.clone()?;
        let map = self.map.as_mut()?;
        let before = map.to_json();
        let result = map.add_reaction(&community, reaction_id, opts)?;
        self.record_map(&before, "Add reaction");
        self.emit("map");
        Some(result)
    }

    /// `removeMapReaction(reactionId)`: from the drawing, never from the
    /// model or LP.
    pub fn remove_map_reaction(&mut self, reaction_id: &str) -> Option<RemovedReaction> {
        let map = self.map.as_mut()?;
        let before = map.to_json();
        let result = map.remove_reaction(reaction_id)?;
        self.record_map(&before, "Remove reaction");
        if self.selected_reaction.as_deref() == Some(reaction_id) {
            self.selected_reaction = None;
            self.selected_entity = None;
            self.variability = None;
        }
        self.multi_selection.reactions.shift_remove(reaction_id);
        for id in &result.nodes {
            self.multi_selection.nodes.shift_remove(id);
        }
        self.emit("map");
        Some(result)
    }

    /// `showEmptyMap()`: replace the drawing with a truly blank canvas.
    pub fn show_empty_map(&mut self) -> bool {
        if self.community.is_none() {
            return false;
        }
        self.map_mode = "empty".into();
        let before = self.map.as_ref().map(EditableMap::to_json);
        let width = self.map.as_ref().map_or(1200.0, EditableMap::width);
        let height = self.map.as_ref().map_or(800.0, EditableMap::height);
        let next = EditableMap::blank(Some(width), Some(height));
        if let Some(before) = before {
            self.map_history.record(&before, &next.to_json(), "Empty map");
        }
        self.selected_reaction = None;
        self.selected_geometry = None;
        self.selected_entity = None;
        self.variability = None;
        self.set_map(Some(next));
        true
    }

    /// `showFullMap()`: restore the complete automatic map for the current
    /// biological model. On a genome-scale model this takes minutes; see
    /// [`Store::show_full_map_in_background`].
    pub fn show_full_map(&mut self) -> bool {
        let Some(community) = self.community.clone() else { return false };
        let next = EditableMap::from_community(&community, &layout_engine_options(&(&self.layout_settings).into()));
        self.install_full_map(next);
        true
    }

    fn install_full_map(&mut self, next: EditableMap) {
        if let Some(before) = self.map.as_ref().map(EditableMap::to_json) {
            self.map_history.record(&before, &next.to_json(), "Full map");
        }
        self.map_mode = "full".into();
        self.remove_core_overview_pathways();
        self.selected_reaction = None;
        self.selected_entity = None;
        self.variability = None;
        self.set_map(Some(next));
    }

    /// `showFullMap()` on a background thread: the layout runs there and the
    /// map is installed by `poll_jobs` (one undo step, as `showFullMap`).
    /// Returns false when there is no community to draw.
    pub fn show_full_map_in_background(&mut self) -> bool {
        let Some(community) = self.community.clone() else { return false };
        self.cancel_layout_job();
        let opts = layout_engine_options(&(&self.layout_settings).into());
        let cancel = Arc::new(AtomicBool::new(false));
        let job = LayoutJob { problem_serial: self.problem_serial, cancel: cancel.clone() };
        let total = community.merged.reactions.len();
        self.set_status_message(StatusState::Busy, format!("Laying out all {total} reactions…"));
        self.emit("status");
        self.jobs.spawn(JobKind::Layout, "layout", total, JobContext::Layout(job), cancel, move |id, tx, _cancel| {
            let t = Instant::now();
            let map = EditableMap::from_community(&community, &opts);
            let _ = tx.send(JobMessage::Done { id, payload: JobPayload::Layout { map: Box::new(map), elapsed: t.elapsed() } });
        });
        true
    }

    /// Drop the result of a background layout (the thread finishes its work
    /// and the map it made is discarded).
    pub fn cancel_layout_job(&mut self) {
        for entry in self.jobs.running.values() {
            if entry.kind == JobKind::Layout {
                entry.cancel.store(true, Ordering::SeqCst);
            }
        }
    }

    pub(crate) fn apply_layout_job(&mut self, job: LayoutJob, map: EditableMap, elapsed: Duration) -> crate::error::Result<()> {
        if job.cancel.load(Ordering::SeqCst) || job.problem_serial != self.problem_serial {
            return Err(crate::error::StoreError::aborted("The layout was cancelled."));
        }
        self.install_full_map(map);
        self.status_mut().layout_ms = js::round(elapsed.as_secs_f64() * 1000.0);
        let n = self.map.as_ref().map_or(0, |m| m.reactions().len());
        self.set_status_message(StatusState::Ok, format!("Full map drawn: {n} reactions."));
        Ok(())
    }

    /// `coreMetabolismOverviewSupport()`.
    pub fn core_metabolism_overview_support(&self) -> Option<OverviewSupport> {
        self.community.as_ref().map(|c| core_overview_support(c))
    }

    /// `showCoreMetabolismOverview({ recordHistory })`: the glycolysis
    /// overview of the community's central carbon metabolism.
    pub fn show_core_metabolism_overview(&mut self, record_history: bool) -> Option<CoreOverviewResult> {
        let community = self.community.clone()?;
        let support = core_overview_support(&community);
        if !support.supported {
            self.set_status_message(
                StatusState::Warn,
                "This community does not contain enough annotated central-carbon reactions for the core metabolism overview.",
            );
            self.emit("status");
            return None;
        }
        let built = build_core_metabolism_overview(&community, &(&self.layout_settings).into());
        match self.map.as_ref().map(EditableMap::to_json) {
            Some(before) if record_history => {
                self.map_history.record(&before, &built.map.to_json(), "Core metabolism overview");
            }
            _ => self.map_history.clear(),
        }
        self.map = Some(built.map);
        self.map_mode = "core-overview".into();
        self.install_core_overview_pathways(&built.pathway_groups);
        // Keep the glycolysis view visually clean: no pathway ribbons, no
        // cross-feeding arcs and no helper/cofactor nodes at overview scale.
        self.view.pathway_ribbon_mode = PathwayRibbonMode::Off;
        self.view.show_cross_feeding = false;
        self.view.show_currency = true;
        self.selected_reaction = None;
        self.selected_geometry = None;
        self.selected_entity = None;
        self.multi_selection = empty_selection();
        self.variability = None;
        self.set_status_message(
            StatusState::Ok,
            format!("Glycolysis overview: {} reactions arranged vertically from G6P to pyruvate.", built.placed),
        );
        self.emit("map");
        Some(CoreOverviewResult { placed: built.placed, pathway_groups: built.pathway_groups })
    }

    /// `installCoreOverviewPathways(groups)`.
    pub fn install_core_overview_pathways(&mut self, groups: &[PathwayGroup]) {
        self.remove_core_overview_pathways();
        let mut aggregated: IndexMap<String, (String, String, Vec<String>)> = IndexMap::new();
        for group in groups {
            let entry = aggregated
                .entry(group.group_id.clone())
                .or_insert_with(|| (format!("core-{}", group.group_id), group.name.clone(), Vec::new()));
            entry.2.extend(group.reaction_ids.iter().cloned());
        }
        for (_, (id, name, reaction_ids)) in aggregated {
            let color = category_color(&name, self.view.dark).to_string();
            self.manual_pathways.insert(
                id.clone(),
                ManualPathway { id, name, color, visible: true, reaction_ids: reaction_ids.into_iter().collect() },
            );
        }
    }

    /// `removeCoreOverviewPathways()`.
    pub fn remove_core_overview_pathways(&mut self) {
        self.manual_pathways.retain(|id, _| !id.starts_with("core-"));
    }

    /// `emptyMap()`: a truly blank drawing surface for the active community.
    pub fn empty_map(&self) -> Option<EditableMap> {
        self.community.as_ref()?;
        Some(EditableMap::blank(
            Some(self.map.as_ref().map_or(1200.0, EditableMap::width)),
            Some(self.map.as_ref().map_or(800.0, EditableMap::height)),
        ))
    }

    // Cells and compartments

    /// `availableMapCells()`: species/cells not yet drawn on the map.
    pub fn available_map_cells(&self) -> Vec<AvailableCell> {
        let Some(map) = self.map.as_ref() else { return Vec::new() };
        let drawn: HashSet<&str> = map.cells().iter().map(|c| c.id.as_str()).collect();
        let mut out = Vec::new();
        for sp in self.species.iter().filter(|sp| sp.enabled && !drawn.contains(sp.id.as_str())) {
            let Some(model) = self.models.get(&sp.file) else { continue };
            let tree = model.compartment_tree();
            let outer = match tree.by_id.get("e").filter(|e| e.parent.is_none()) {
                Some(e) => Some(e.id.clone()),
                None => tree.roots.first().cloned(),
            };
            let Some(outer) = outer else { continue };
            out.push(AvailableCell {
                species_id: sp.id.clone(),
                species_name: model.cell.name.clone(),
                organism: model.cell.organism.clone(),
                outer_compartment_name: cell_compartment_name(&outer, &model.compartment_names, "en"),
                outer_compartment_id: outer,
            });
        }
        out.sort_by(|a, b| js::locale_compare(&a.species_name, &b.species_name));
        out
    }

    /// `setCellPlacementSpecies(speciesId)`.
    pub fn set_cell_placement_species(&mut self, species_id: &str) -> Option<String> {
        let available = self.available_map_cells();
        let next = available
            .iter()
            .find(|c| c.species_id == species_id)
            .or(available.first())
            .map(|c| c.species_id.clone());
        self.cell_placement_species_id = next.clone();
        self.emit("cell-placement");
        next
    }

    /// `addMapCell(speciesId, rect)`: draw one species' cell boundary.
    pub fn add_map_cell(&mut self, species_id: &str, rect: Option<Rect>) -> Option<AddedShell> {
        self.map.as_ref()?;
        let available = self.available_map_cells();
        let spec = available
            .iter()
            .find(|c| c.species_id == species_id)
            .or_else(|| available.iter().find(|c| Some(&c.species_id) == self.cell_placement_species_id.as_ref()))
            .or(available.first())?
            .clone();
        let sp = self.species_by_id(&spec.species_id)?.clone();
        let model = self.models.get(&sp.file)?.clone();
        let map = self.map.as_mut()?;
        let before = map.to_json();
        let result = map.add_cell(&CellSpec {
            species_id: sp.id.clone(),
            outer_compartment_id: Some(spec.outer_compartment_id.clone()),
            rect,
            outer_label: Some(spec.outer_compartment_name.clone()),
            outer_membrane: None,
            cell_name: Some(model.cell.name.clone()),
            organism: Some(model.cell.organism.clone()),
            note: Some(model.cell.note.clone()),
            color: Some(model.cell.color.clone()),
            abundance: Some(sp.abundance),
        })?;
        self.record_map(&before, "Create cell");
        self.cell_placement_species_id = self.available_map_cells().first().map(|c| c.species_id.clone());
        self.selected_reaction = None;
        self.selected_geometry = Some(GeometrySelection {
            cell_id: Some(result.cell.id.clone()),
            compartment_id: result.cell.outer_compartment.clone().unwrap_or_default(),
        });
        self.selected_entity = Some(SelectedEntity::new("cell", &result.cell.id));
        self.emit("map");
        Some(result)
    }

    /// `selectCellGeometry(cellId)`.
    pub fn select_cell_geometry(&mut self, cell_id: &str) -> Option<GeometrySelection> {
        let cell = self.map.as_ref()?.cell(cell_id)?;
        let outer = cell
            .compartments
            .iter()
            .find(|c| Some(&c.id) == cell.outer_compartment.as_ref())
            .or_else(|| cell.compartments.iter().find(|c| c.parent_id.is_none()))?;
        let selection = GeometrySelection { cell_id: Some(cell.id.clone()), compartment_id: outer.id.clone() };
        self.selected_reaction = None;
        self.selected_geometry = Some(selection.clone());
        self.selected_entity = Some(SelectedEntity::new("cell", cell_id));
        self.variability = None;
        self.emit("geometry-selection");
        Some(selection)
    }

    /// `availableMapCompartments()`: compartments that may be added next
    /// without violating the source model's containment tree. A child
    /// becomes available after its parent is drawn.
    pub fn available_map_compartments(&self) -> Vec<AvailableCompartment> {
        let Some(map) = self.map.as_ref() else { return Vec::new() };
        let mut out = Vec::new();
        for sp in self.species.iter().filter(|sp| sp.enabled) {
            let Some(model) = self.models.get(&sp.file) else { continue };
            let Some(cell) = map.cell(&sp.id) else { continue };
            let drawn: HashSet<&str> = cell.compartments.iter().map(|c| c.id.as_str()).collect();
            let tree = model.compartment_tree();
            for node in tree.by_id.values() {
                if drawn.contains(node.id.as_str()) {
                    continue;
                }
                // Root/extracellular geometry is created by the Cell tool.
                // Manual compartment creation starts below that cell boundary.
                let Some(parent) = &node.parent else { continue };
                if !drawn.contains(parent.as_str()) {
                    continue;
                }
                out.push(AvailableCompartment {
                    species_id: sp.id.clone(),
                    species_name: model.cell.name.clone(),
                    compartment_id: node.id.clone(),
                    compartment_name: cell_compartment_name(&node.id, &model.compartment_names, "en"),
                    parent_id: Some(parent.clone()),
                    depth: node.depth,
                });
            }
        }
        out.sort_by(|a, b| {
            js::locale_compare(&a.species_name, &b.species_name)
                .then(a.depth.cmp(&b.depth))
                .then_with(|| js::locale_compare(&a.compartment_id, &b.compartment_id))
        });
        out
    }

    /// `mapCompartmentChoices()`: every compartment of every active species,
    /// drawn or not, and whether it can be drawn now (see
    /// [`CompartmentChoice`]).
    pub fn map_compartment_choices(&self) -> Vec<CompartmentChoice> {
        let Some(map) = self.map.as_ref() else { return Vec::new() };
        let mut out = Vec::new();
        for sp in self.species.iter().filter(|sp| sp.enabled) {
            let Some(model) = self.models.get(&sp.file) else { continue };
            let cell = map.cell(&sp.id);
            let drawn: HashSet<&str> = cell.map(|c| c.compartments.iter().map(|k| k.id.as_str()).collect()).unwrap_or_default();
            let tree = model.compartment_tree();
            let name = |id: &str| cell_compartment_name(id, &model.compartment_names, "en");
            for node in tree.by_id.values() {
                let (status, reason) = if drawn.contains(node.id.as_str()) {
                    ("drawn", "already on the map".to_string())
                } else if let Some(parent) = &node.parent {
                    if cell.is_none() {
                        ("waiting", "draw the cell first".to_string())
                    } else if drawn.contains(parent.as_str()) {
                        ("ready", format!("goes inside {}", name(parent)))
                    } else {
                        ("waiting", format!("goes inside {}: draw that first", name(parent)))
                    }
                } else {
                    ("cell", "draw the cell with Add Cell".to_string())
                };
                out.push(CompartmentChoice {
                    species_id: sp.id.clone(),
                    species_name: model.cell.name.clone(),
                    compartment_id: node.id.clone(),
                    compartment_name: name(&node.id),
                    parent_id: node.parent.clone(),
                    parent_name: node.parent.as_deref().map(name),
                    depth: node.depth,
                    status,
                    reason,
                });
            }
        }
        out.sort_by(|a, b| {
            js::locale_compare(&a.species_name, &b.species_name)
                .then(a.depth.cmp(&b.depth))
                .then_with(|| js::locale_compare(&a.compartment_id, &b.compartment_id))
        });
        out
    }

    /// `setCompartmentShapePreset(name)`: the shape the next compartment
    /// drawn gets (`auto`, or a name of `COMPARTMENT_SHAPES`).
    pub fn set_compartment_shape_preset(&mut self, name: &str) {
        let next = if name == "auto" || compartment_shape_preset(name).is_some() { name } else { "auto" };
        if next == self.compartment_shape_preset {
            return;
        }
        self.compartment_shape_preset = next.to_string();
        self.emit("compartment-placement");
    }

    /// `removeMapCompartment(cellId, compartmentId)`: delete a compartment
    /// from the drawing, with the compartments nested in it and everything
    /// drawn in them (a cell's outermost one takes the cell). One undoable
    /// step; the model is unchanged, and the compartment can be drawn again
    /// from Add Compartment.
    pub fn remove_map_compartment(&mut self, cell_id: &str, compartment_id: &str) -> Option<RemovedCompartment> {
        let _ = self.assert_map_operation("removeMapCompartment");
        let map = self.map.as_mut()?;
        let before = map.to_json();
        let result = map.remove_compartment(cell_id, compartment_id)?;
        self.record_map(&before, if result.cell_removed { "Remove cell" } else { "Remove compartment" });

        self.selected_geometry = None;
        let map = self.map.as_ref().expect("checked");
        if let Some(entity) = &self.selected_entity {
            let gone = matches!(entity.kind.as_str(), "compartment" | "cell")
                || (entity.kind == "reaction" && result.reactions.contains(&entity.id))
                || (entity.kind == "metabolite" && !map.has_node(&entity.id));
            if gone {
                self.selected_entity = None;
            }
        }
        if self.selected_reaction.as_ref().is_some_and(|r| !map.has_reaction(r)) {
            self.selected_reaction = None;
            self.variability = None;
        }
        self.multi_selection = empty_selection();

        let plural = |n: usize, one: &str| format!("{n} {one}{}", if n == 1 { "" } else { "s" });
        let mut what = Vec::new();
        if result.compartment_ids.len() > 1 {
            what.push(plural(result.compartment_ids.len() - 1, "nested compartment"));
        }
        if !result.reactions.is_empty() {
            what.push(plural(result.reactions.len(), "reaction"));
        }
        if !result.nodes.is_empty() {
            what.push(plural(result.nodes.len(), "metabolite"));
        }
        if !result.texts.is_empty() {
            what.push(plural(result.texts.len(), "text"));
        }
        let message = format!(
            "Removed {}{} from the map{}. The model is unchanged; Ctrl+Z brings it back.",
            if result.cell_removed { "the cell " } else { "" },
            result.label,
            if what.is_empty() { String::new() } else { format!(", with {}", what.join(", ")) }
        );
        self.set_status_message(StatusState::Ok, message);
        self.emit("map");
        Some(result)
    }

    /// `setCompartmentPlacement(key)`: key is `species|compartment`.
    pub fn set_compartment_placement(&mut self, key: &str) -> Option<AvailableCompartment> {
        let available = self.available_map_compartments();
        let next = available.iter().find(|c| c.key() == key).or(available.first()).cloned();
        let next_key = next.as_ref().map(AvailableCompartment::key);
        let changed = self.compartment_placement_key != next_key;
        self.compartment_placement_key = next_key;
        if changed {
            self.emit("compartment-placement");
        }
        next
    }

    /// `currentCompartmentPlacement()`.
    pub fn current_compartment_placement(&mut self) -> Option<AvailableCompartment> {
        let available = self.available_map_compartments();
        let selected = available
            .iter()
            .find(|c| Some(c.key()) == self.compartment_placement_key)
            .or(available.first())
            .cloned();
        if let (Some(s), None) = (&selected, &self.compartment_placement_key) {
            self.compartment_placement_key = Some(s.key());
        }
        selected
    }

    /// `compartmentParentRect(speciesId, compartmentId)`: the box a new
    /// compartment has to stay inside.
    pub fn compartment_parent_rect(&self, species_id: &str, compartment_id: &str) -> Option<Rect> {
        let available = self.available_map_compartments();
        let spec = available.iter().find(|c| c.species_id == species_id && c.compartment_id == compartment_id)?;
        let parent_id = spec.parent_id.as_ref()?;
        let cell = self.map.as_ref()?.cell(species_id)?;
        cell.compartments.iter().find(|c| &c.id == parent_id).map(|c| c.rect)
    }

    /// `addMapCompartment(speciesId, compartmentId, rect)`.
    pub fn add_map_compartment(&mut self, species_id: &str, compartment_id: &str, rect: Option<Rect>) -> Option<AddedShell> {
        self.map.as_ref()?;
        let sp = self.species.iter().find(|s| s.id == species_id && s.enabled)?.clone();
        let model = self.models.get(&sp.file)?.clone();
        let tree = model.compartment_tree();
        let tree_node = tree.by_id.get(compartment_id)?;
        let parent = tree_node.parent.clone()?;
        self.map.as_ref()?.cell(species_id)?;
        let map = self.map.as_mut()?;
        let before = map.to_json();
        let result = map.add_compartment(&CompartmentSpec {
            species_id: species_id.to_string(),
            compartment_id: compartment_id.to_string(),
            parent_id: Some(parent),
            rect,
            label: Some(cell_compartment_name(compartment_id, &model.compartment_names, "en")),
            membrane: None,
            cell_name: Some(model.cell.name.clone()),
            organism: Some(model.cell.organism.clone()),
            note: Some(model.cell.note.clone()),
            color: Some(model.cell.color.clone()),
            abundance: Some(sp.abundance),
        })?;
        if let Some(shape) = compartment_shape_preset(&self.compartment_shape_preset) {
            map.edit_compartment_shape(Some(species_id), compartment_id, ShapePatch::of(shape));
        }
        self.record_map(&before, "Agregar compartimento");
        self.selected_reaction = None;
        self.selected_geometry = Some(GeometrySelection { cell_id: Some(species_id.into()), compartment_id: compartment_id.into() });
        let mut entity = SelectedEntity::new("compartment", compartment_id);
        entity.cell_id = Some(species_id.into());
        self.selected_entity = Some(entity);
        self.compartment_placement_key = self.available_map_compartments().first().map(AvailableCompartment::key);
        self.emit("map");
        Some(result)
    }

    /// `previewCanvasSize(width, height)`: what `set_map_canvas_size` would
    /// settle on, without changing anything.
    pub fn preview_canvas_size(&self, width: f64, height: f64) -> Option<Canvas> {
        let map = self.map.as_ref()?;
        let min = map.minimum_canvas_size(40.0);
        let or = |v: f64, fallback: f64| if v.is_nan() || v == 0.0 { fallback } else { v };
        Some(Canvas {
            width: js::max(min.width, or(width, map.canvas().width)),
            height: js::max(min.height, or(height, map.canvas().height)),
        })
    }

    /// `setMapCanvasSize(width, height, { shiftX, shiftY })`: part of a
    /// resize gesture; no history, no event.
    pub fn set_map_canvas_size(&mut self, width: Option<f64>, height: Option<f64>, shift_x: f64, shift_y: f64) -> Option<Canvas> {
        Some(self.map.as_mut()?.set_canvas_size(width, height, shift_x, shift_y))
    }

    // Layouts

    /// `setLayoutPreset(name)`.
    pub fn set_layout_preset(&mut self, name: &str) {
        self.layout_settings = layout_preset(Some(name));
        if self.map_mode == "core-overview" {
            self.show_core_metabolism_overview(true);
            return;
        }
        let label = format!("Layout preset: {}", self.layout_settings.preset);
        self.relayout_map(&label);
    }

    /// `setLayoutSettings(patch)`.
    pub fn set_layout_settings(&mut self, patch: RawLayoutSettings) {
        let current = RawLayoutSettings::from(&self.layout_settings);
        self.layout_settings = normalize_layout_settings(&RawLayoutSettings {
            preset: patch.preset.or(current.preset),
            cell_spacing: patch.cell_spacing.or(current.cell_spacing),
            minimum_compartment_size: patch.minimum_compartment_size.or(current.minimum_compartment_size),
            node_spacing: patch.node_spacing.or(current.node_spacing),
            cell_area_multiplier: patch.cell_area_multiplier.or(current.cell_area_multiplier),
        });
        if self.map_mode == "core-overview" {
            self.show_core_metabolism_overview(true);
            return;
        }
        self.relayout_map("Adjust automatic layout");
    }

    /// `relayoutBlocked()`: why the automatic layout must not be re-run over
    /// the current drawing, or `None` when it may.
    ///
    /// A layout preset means "lay the map out again". For a model too large
    /// to be drawn automatically the map on screen was built by hand,
    /// reaction by reaction; laying it out again first places every reaction
    /// of the model (minutes) and then throws the hand-placed positions
    /// away. Nobody pressing "Compact" in the toolbar is asking for that.
    pub fn relayout_blocked(&self) -> Option<String> {
        let (community, map) = (self.community.as_ref()?, self.map.as_ref()?);
        let drawn = map.reactions().len();
        if drawn == 0 || drawn >= community.merged.reactions.len() {
            return None;
        }
        self.too_large_to_auto_draw().then(|| {
            "Layout presets re-run the automatic layout, which would replace this hand-built drawing and take minutes on a model this size. The drawing was kept.".to_string()
        })
    }

    /// `tooLargeToAutoDraw()`: a species too large for the automatic layout
    /// to place in reasonable time.
    pub fn too_large_to_auto_draw(&self) -> bool {
        self.community.as_ref().is_some_and(|c: &Arc<CommunityModel>| {
            c.active_species().any(|sp| {
                sp.model.reactions.len() > AUTO_DRAW_REACTION_LIMIT || sp.model.metabolites.len() > AUTO_DRAW_METABOLITE_LIMIT
            })
        })
    }

    /// `fullMapWarning()`: what to ask before drawing the full model, or
    /// `None` when it is cheap.
    pub fn full_map_warning(&self) -> Option<String> {
        let community = self.community.as_ref()?;
        if !self.too_large_to_auto_draw() {
            return None;
        }
        let total = community.merged.reactions.len();
        let drawn = self.map.as_ref().map_or(0, |m| m.reactions().len());
        let replaces =
            if drawn > 0 { format!(" It replaces the current drawing of {drawn} reactions (Undo restores it).") } else { String::new() };
        Some(format!(
            "Drawing all {total} reactions runs the automatic layout on a genome-scale model; the page may stop responding for several minutes.{replaces}\n\nDraw the full model?"
        ))
    }

    /// `relayoutMap(label)`: re-run only the graphical automatic layout. The
    /// model and current solution are untouched. If the map is a reaction
    /// subset, keep that same subset.
    pub fn relayout_map(&mut self, label: &str) -> bool {
        let (Some(community), Some(map)) = (self.community.clone(), self.map.as_ref()) else { return false };
        let drawn = map.reaction_ids();
        let was_blank = map.cells().is_empty() && drawn.is_empty();
        if was_blank {
            self.emit("layout-settings");
            return true;
        }
        if let Some(blocked) = self.relayout_blocked() {
            self.set_status_message(StatusState::Ok, blocked);
            self.emit("layout-settings");
            return true;
        }
        let before = map.to_json();
        let opts = layout_engine_options(&(&self.layout_settings).into());
        let next = if drawn.is_empty() {
            // Only the shell is drawn, so only the shell is laid out.
            EditableMap::empty_from_community(&community, &opts)
        } else {
            let full = EditableMap::from_community(&community, &opts);
            if drawn.len() < full.reactions().len() {
                let keep: Vec<String> = drawn.into_iter().filter(|id| full.has_reaction(id)).collect();
                full.subset_by_reactions(&keep)
            } else {
                full
            }
        };
        self.map_history.record(&before, &next.to_json(), label);
        self.map = Some(next);
        self.selected_reaction = None;
        self.selected_geometry = None;
        self.selected_entity = None;
        self.multi_selection = empty_selection();
        self.variability = None;
        self.emit("layout-settings");
        true
    }

    // Drag-time edits (canvas)

    /// `moveNode(nodeId, x, y)`: move a metabolite node by hand.
    ///
    /// Purely graphical: the geometry of one node and the segments attached
    /// to it change, and nothing else does. It neither solves nor rebuilds
    /// nor relayouts, and it does not emit, since dragging repaints at its
    /// own pace. Returns where the node ended up, or `None` when it is not
    /// movable (the JS returns the node).
    pub fn move_node(&mut self, node_id: &str, x: f64, y: f64) -> Option<Point> {
        self.map.as_mut()?.move_node(node_id, x, y).map(|n| Point::new(n.x, n.y))
    }

    /// `mergeOverlappingMetabolite(nodeId)`: complete an explicit
    /// drag-and-merge gesture. The surrounding node-drag transaction records
    /// the move and the merge as one undoable edit.
    pub fn merge_overlapping_metabolite(&mut self, node_id: &str) -> Option<MergeResult> {
        let result = self.map.as_mut()?.merge_overlapping_metabolite(node_id)?;
        if let Some(entity) = self.selected_entity.as_mut().filter(|e| e.kind == "metabolite" && e.id == node_id) {
            entity.id = result.target.id.clone();
        }
        Some(result)
    }

    /// `canMoveNode(nodeId)`.
    pub fn can_move_node(&self, node_id: &str) -> bool {
        self.map.as_ref().is_some_and(|m| m.can_move_node(node_id))
    }

    /// `canMoveReaction(reactionId)`.
    pub fn can_move_reaction(&self, reaction_id: &str) -> bool {
        self.map.as_ref().is_some_and(|m| m.can_move_reaction(reaction_id))
    }

    /// `moveReaction(reactionId, x, y)`: part of a drag. Returns where the
    /// reaction's marker ended up.
    pub fn move_reaction(&mut self, reaction_id: &str, x: f64, y: f64) -> Option<Point> {
        self.map.as_mut()?.move_reaction(reaction_id, x, y).map(|r| Point::new(r.x, r.y))
    }

    /// `moveBranchControl(reactionId, nodeId, handle, x, y)`: part of a drag.
    pub fn move_branch_control(&mut self, reaction_id: &str, node_id: &str, handle: Handle, x: f64, y: f64) -> Option<Point> {
        self.map.as_mut()?.move_branch_control(reaction_id, node_id, handle, x, y)
    }

    /// `canRotateReaction(reactionId)`.
    pub fn can_rotate_reaction(&self, reaction_id: &str) -> bool {
        self.map.as_ref().is_some_and(|m| m.can_rotate_reaction(reaction_id))
    }

    /// `setReactionAngle(reactionId, angle)`: turn a reaction's backbone
    /// while it is being dragged round; the gesture records and emits once.
    pub fn set_reaction_angle(&mut self, reaction_id: &str, angle: f64) -> bool {
        self.map.as_mut().is_some_and(|m| m.set_reaction_angle(reaction_id, angle))
    }

    /// `moveMapText(id, x, y)`: part of a drag. Returns where the text ended
    /// up.
    pub fn move_map_text(&mut self, id: &str, x: f64, y: f64) -> Option<Point> {
        self.map.as_mut()?.move_text(id, x, y).map(|t| Point::new(t.x, t.y))
    }

    /// `editGeometryRect(cellId, compartmentId, rect)`: part of a move or
    /// resize of a compartment, a cell or the shared medium.
    pub fn edit_geometry_rect(&mut self, cell_id: Option<&str>, compartment_id: &str, rect: Rect) -> Option<GeometryEdit> {
        self.map.as_mut()?.edit_geometry_rect(cell_id, compartment_id, rect)
    }

    /// `setMapNodeStyle(colors)`: the colours of every metabolite circle
    /// without its own; `Reset` for the theme's. One undo step.
    pub fn set_map_node_style(&mut self, fill: &ColorChange, stroke: &ColorChange) -> Option<coxeter_core::mapdata::NodeStyle> {
        let map = self.map.as_mut()?;
        let before = map.to_json();
        let style = map.set_node_style(fill, stroke);
        if self.record_map(&before, "Metabolite colours") {
            self.emit("map");
        }
        Some(style)
    }

    /// `setMapNodeColors(nodeIds, colors)`: colour some metabolite circles
    /// (a selection, or one); `Reset` for the map's. One undo step.
    pub fn set_map_node_colors(&mut self, node_ids: &[String], fill: &ColorChange, stroke: &ColorChange) -> Vec<String> {
        let Some(map) = self.map.as_mut() else { return Vec::new() };
        let before = map.to_json();
        let changed = map.set_node_colors(node_ids, fill, stroke);
        if !changed.is_empty() && self.record_map(&before, "Metabolite colour") {
            self.emit("map");
        }
        changed
    }

    /// `setMapReactionColor(reactionIds, color)`: the colour some reactions
    /// are drawn in when lines are not coloured by flux; `Reset` for their
    /// pathway's. One undo step.
    pub fn set_map_reaction_color(&mut self, reaction_ids: &[String], color: &ColorChange) -> Vec<String> {
        let Some(map) = self.map.as_mut() else { return Vec::new() };
        let before = map.to_json();
        let changed = map.set_reaction_color(reaction_ids, color);
        if !changed.is_empty() && self.record_map(&before, "Reaction colour") {
            self.emit("map");
        }
        changed
    }

    /// `editCompartmentShape(cellId, compartmentId, shape)`: reshape while a
    /// shape handle is dragged; the drag records the history.
    pub fn edit_compartment_shape(&mut self, cell_id: Option<&str>, compartment_id: &str, shape: ShapePatch) -> Option<ShapeEdit> {
        self.map.as_mut()?.edit_compartment_shape(cell_id, compartment_id, shape)
    }

    /// `setCompartmentShape(cellId, compartmentId, shape)`: a compartment's
    /// shape in one undoable step, as the inspector's presets set it.
    pub fn set_compartment_shape(&mut self, cell_id: Option<&str>, compartment_id: &str, shape: ShapePatch) -> Option<ShapeEdit> {
        let map = self.map.as_mut()?;
        let before = map.to_json();
        let result = map.edit_compartment_shape(cell_id, compartment_id, shape)?;
        self.record_map(&before, "Compartment shape");
        self.emit("map");
        Some(result)
    }

    /// `setMetaboliteLabelAngle(nodeId, angle)`: a drag gesture owns the
    /// repaint and its single history transaction.
    pub fn set_metabolite_label_angle(&mut self, node_id: &str, angle: f64) -> bool {
        self.map.as_mut().is_some_and(|m| m.set_metabolite_label_angle(node_id, angle).is_some())
    }

    // Arranging one reaction

    /// `editMapReaction(label, edit)`: one undoable change to how a single
    /// reaction is drawn.
    pub fn edit_map_reaction(&mut self, label: &str, edit: impl FnOnce(&mut EditableMap) -> bool) -> bool {
        let Some(map) = self.map.as_mut() else { return false };
        let before = map.to_json();
        if !edit(map) {
            return false;
        }
        self.record_map(&before, label);
        self.emit("map");
        true
    }

    /// `rotateMapReaction(reactionId, angle)`: by a fixed step, as one edit.
    pub fn rotate_map_reaction(&mut self, reaction_id: &str, angle: f64) -> bool {
        self.edit_map_reaction("Rotate reaction", |m| m.rotate_reaction(reaction_id, angle))
    }

    /// `setMapReactionLinear(reactionId, linear)`: keep a reaction straight
    /// between two of its metabolites, or release it.
    pub fn set_map_reaction_linear(&mut self, reaction_id: &str, linear: bool) -> bool {
        let (from, to) = if linear { self.selected_line_ends(reaction_id) } else { (None, None) };
        let label = if linear { "Make reaction straight" } else { "Release straight reaction" };
        self.edit_map_reaction(label, |m| m.set_reaction_linear(reaction_id, linear, from.as_deref(), to.as_deref()))
    }

    /// `selectedLineEnds(reactionId)`: the metabolites of this reaction that
    /// are selected on the map, as the two its line should run through: one
    /// substrate, one product. A side with none selected, or more than one,
    /// is left for the reaction to decide.
    pub fn selected_line_ends(&self, reaction_id: &str) -> (Option<String>, Option<String>) {
        let picked = &self.multi_selection.nodes;
        let Some(map) = self.map.as_ref().filter(|_| !picked.is_empty()) else { return (None, None) };
        let (substrates, products) = map.reaction_sides(reaction_id);
        let one = |entries: &[coxeter_map::reaction_edit::SideEntry]| {
            let chosen: Vec<&String> = entries.iter().map(|e| &e.node_id).filter(|id| picked.contains(*id)).collect();
            (chosen.len() == 1).then(|| chosen[0].clone())
        };
        (one(&substrates), one(&products))
    }

    /// `setMapReactionLineEnd(reactionId, nodeId)`.
    pub fn set_map_reaction_line_end(&mut self, reaction_id: &str, node_id: &str) -> bool {
        self.edit_map_reaction("Choose line metabolite", |m| m.set_reaction_line_end(reaction_id, node_id))
    }

    /// `alignMapReaction(reactionId, direction)`: lay a reaction out along a
    /// compass direction (`N`, `NE`, …), substrate to product.
    pub fn align_map_reaction(&mut self, reaction_id: &str, direction: &str) -> bool {
        let (from, to) = self.selected_line_ends(reaction_id);
        self.edit_map_reaction(&format!("Align reaction {direction}"), |m| {
            m.align_reaction(reaction_id, direction, from.as_deref(), to.as_deref())
        })
    }

    /// `alignMapReactionAxis(reactionId, axis)`: Escher's Align vertical /
    /// horizontal for one reaction, keeping the way it runs now.
    pub fn align_map_reaction_axis(&mut self, reaction_id: &str, axis: &str) -> bool {
        let Some(map) = self.map.as_ref() else { return false };
        if axis != "vertical" && axis != "horizontal" {
            return false;
        }
        let (mut from, mut to) = map.reaction_line_ends(reaction_id);
        let (sel_from, sel_to) = self.selected_line_ends(reaction_id);
        if sel_from.is_some() {
            from = sel_from;
        }
        if sel_to.is_some() {
            to = sel_to;
        }
        let (Some(from), Some(to)) = (from.and_then(|f| map.node(&f)), to.and_then(|t| map.node(&t))) else { return false };
        let direction = if axis == "vertical" {
            if to.y >= from.y { "S" } else { "N" }
        } else if to.x >= from.x {
            "E"
        } else {
            "W"
        };
        self.align_map_reaction(reaction_id, direction)
    }

    /// `tidyMapReaction(reactionId)`: cofactors and curves back in their base
    /// shape.
    pub fn tidy_map_reaction(&mut self, reaction_id: &str) -> bool {
        self.edit_map_reaction("Tidy reaction", |m| m.tidy_reaction(reaction_id))
    }

    /// `swapMapReactionMetabolites(reactionId, a, b)`.
    pub fn swap_map_reaction_metabolites(&mut self, reaction_id: &str, a: &str, b: &str) -> bool {
        self.edit_map_reaction("Swap metabolites", |m| m.swap_reaction_metabolites(reaction_id, a, b))
    }

    /// `flipMapReaction(reactionId)`: swap the substrate and product sides.
    pub fn flip_map_reaction(&mut self, reaction_id: &str) -> bool {
        self.edit_map_reaction("Swap substrates and products", |m| m.flip_reaction(reaction_id))
    }

    /// `mirrorMapReaction(reactionId)`.
    pub fn mirror_map_reaction(&mut self, reaction_id: &str) -> bool {
        self.edit_map_reaction("Mirror reaction", |m| m.mirror_reaction(reaction_id))
    }

    // Texts

    /// `addMapText(text, x, y, style)`: new text is set in `style`, by
    /// default the last size and font chosen.
    pub fn add_map_text(&mut self, text: &str, x: f64, y: f64, style: Option<&TextStyle>) -> Option<MapText> {
        let style = style.cloned().unwrap_or_else(|| self.view.text_style.clone());
        let map = self.map.as_mut()?;
        let before = map.to_json();
        let annotation = map
            .add_text(text, x, y, &TextOptions { id: None, font_size: Some(style.font_size), font_family: Some(style.font_family) })?
            .clone();
        self.record_map(&before, "Add text");
        self.emit("map");
        Some(annotation)
    }

    /// `updateMapText(id, text, style)`: words and, when given, size and font
    /// in one undo step.
    pub fn update_map_text(&mut self, id: &str, text: &str, style: Option<(Option<f64>, Option<&str>)>) -> Option<MapText> {
        let map = self.map.as_mut()?;
        let before = map.to_json();
        let mut annotation = map.update_text(id, text)?.clone();
        if let Some((size, font)) = style {
            if let Some(t) = map.style_text(id, size, font) {
                annotation = t.clone();
            }
        }
        self.record_map(&before, "Edit text");
        self.emit("map");
        Some(annotation)
    }

    /// `styleMapText(id, style)`: size and/or font. The choice also becomes
    /// the style of the next new text, as in any editor.
    pub fn style_map_text(&mut self, id: &str, font_size: Option<f64>, font_family: Option<&str>) -> Option<MapText> {
        let map = self.map.as_mut()?;
        let before = map.to_json();
        let annotation = map.style_text(id, font_size, font_family)?.clone();
        self.set_text_style(annotation.font_size, annotation.font_family.as_deref());
        self.record_map(&before, "Text style");
        self.emit("map");
        Some(annotation)
    }

    /// `setTextStyle({ fontSize, fontFamily })`: the size and font new texts
    /// are set in; a side left out keeps its value.
    pub fn set_text_style(&mut self, font_size: Option<f64>, font_family: Option<&str>) -> TextStyle {
        let current = self.view.text_style.clone();
        self.view.text_style = TextStyle {
            font_size: font_size.map_or(current.font_size, clamp_text_size),
            font_family: font_family.map_or(current.font_family, |f| normalize_text_font(f).to_string()),
        };
        self.view.text_style.clone()
    }

    /// `removeMapText(id)`.
    pub fn remove_map_text(&mut self, id: &str) -> Option<MapText> {
        let map = self.map.as_mut()?;
        let before = map.to_json();
        let annotation = map.remove_text(id)?;
        self.record_map(&before, "Eliminar texto");
        if self.selected_entity.as_ref().is_some_and(|e| e.kind == "text" && e.id == id) {
            self.selected_entity = None;
        }
        self.emit("map");
        Some(annotation)
    }

    // Metabolites

    /// `duplicateMapMetabolite(nodeId, opts)`.
    pub fn duplicate_map_metabolite(&mut self, node_id: &str, opts: &DuplicateOptions) -> Option<Node> {
        let map = self.map.as_mut()?;
        let before = map.to_json();
        let node = map.duplicate_metabolite_node(node_id, opts)?.clone();
        self.record_map(&before, "Duplicate metabolite");
        self.emit("map");
        Some(node)
    }

    /// `metaboliteCopies(nodeId)`: every drawn instance of the same
    /// biological metabolite.
    pub fn metabolite_copies(&self, node_id: &str) -> Vec<String> {
        let Some(map) = self.map.as_ref() else { return Vec::new() };
        let Some(merged) = map.node(node_id).and_then(|n| n.merged_id.clone()).filter(|m| !m.is_empty()) else { return Vec::new() };
        map.nodes_for_metabolite(&merged).into_iter().map(|n| n.id.clone()).collect()
    }

    /// `setMapNodePrimary(nodeId, primary)`.
    pub fn set_map_node_primary(&mut self, node_id: &str, primary: bool) -> Option<Node> {
        let map = self.map.as_mut()?;
        let before = map.to_json();
        let node = map.set_node_primary(node_id, primary)?.clone();
        let label = if node.primary == Some(true) { "Mark metabolite as primary" } else { "Mark metabolite as secondary" };
        self.record_map(&before, label);
        self.emit("map");
        Some(node)
    }

    /// `setMapGeometryColors(cellId, compartmentId, colors)`: colour a cell
    /// (`compartment_id: None`) or one of its compartments. Map-only,
    /// undoable and saved with the map.
    pub fn set_map_geometry_colors(&mut self, cell_id: &str, compartment_id: Option<&str>, colors: &GeometryColors) -> bool {
        let Some(map) = self.map.as_mut() else { return false };
        let before = map.to_json();
        let found = match compartment_id {
            Some(c) => map.set_compartment_colors(cell_id, c, &colors.fill, &colors.stroke).is_some(),
            None => map.set_cell_colors(cell_id, &colors.fill, &colors.stroke).is_some(),
        };
        if !found {
            return false;
        }
        self.record_map(&before, if compartment_id.is_some() { "Compartment colours" } else { "Cell colours" });
        self.emit("map");
        true
    }

    /// `setMapNodeRadius(nodeId, radius)`.
    pub fn set_map_node_radius(&mut self, node_id: &str, radius: f64) -> Option<Node> {
        let map = self.map.as_mut()?;
        let before = map.to_json();
        let node = map.set_node_radius(node_id, radius)?.clone();
        self.record_map(&before, "Resize metabolite");
        self.emit("map");
        Some(node)
    }

    /// `setMapNodeHidden(nodeId, hidden)`.
    pub fn set_map_node_hidden(&mut self, node_id: &str, hidden: bool) -> Option<Node> {
        let map = self.map.as_mut()?;
        let before = map.to_json();
        let node = map.set_node_hidden(node_id, hidden)?.clone();
        self.record_map(&before, if hidden { "Hide metabolite" } else { "Show metabolite" });
        self.emit("map");
        Some(node)
    }

    /// `hiddenMetaboliteCount`.
    pub fn hidden_metabolite_count(&self) -> usize {
        self.map.as_ref().map_or(0, EditableMap::hidden_node_count)
    }

    /// `showAllHiddenMetabolites()`.
    pub fn show_all_hidden_metabolites(&mut self) -> Vec<String> {
        let Some(map) = self.map.as_mut() else { return Vec::new() };
        let before = map.to_json();
        let changed = map.show_all_hidden_nodes();
        if changed.is_empty() {
            return changed;
        }
        self.record_map(&before, "Show hidden metabolites");
        self.emit("map");
        changed
    }

    /// `searchReactions(query, { limit })`: search the complete biological
    /// reaction set, including reactions not drawn. Ranked by id first, then
    /// name and subsystem.
    pub fn search_reactions(&self, query: &str, limit: usize) -> Vec<ReactionSearchResult> {
        let Some(community) = self.community.as_ref() else { return Vec::new() };
        let q = normalize_search(query);
        if q.is_empty() {
            return Vec::new();
        }
        let mut results = Vec::new();
        for (reaction_id, reaction) in &community.merged.reactions {
            let owner = community.reaction_owner.get(reaction_id);
            let local_id = owner.map(|o| o.original_id.clone()).unwrap_or_else(|| reaction.id.clone());
            let species_id = owner.and_then(|o| o.species_id.clone()).unwrap_or_default();
            let full = normalize_search(reaction_id);
            let local = normalize_search(&local_id);
            let name = normalize_search(&reaction.name);
            let subsystem = normalize_search(&reaction.subsystem);
            let label_text = self.entity_label("reaction", reaction_id).label;
            let label = normalize_search(&label_text);
            let score = if local == q || full == q || label == q {
                0
            } else if local.starts_with(&q) || label.starts_with(&q) {
                1
            } else if full.starts_with(&q) {
                2
            } else if name.starts_with(&q) {
                3
            } else if local.contains(&q) || label.contains(&q) {
                4
            } else if full.contains(&q) {
                5
            } else if name.contains(&q) {
                6
            } else if subsystem.contains(&q) {
                7
            } else {
                continue;
            };
            // Beside the label, whichever of the name and the model id it is not.
            let detail = if label_text == reaction.name {
                if local_id == label_text { String::new() } else { local_id.clone() }
            } else {
                reaction.name.clone()
            };
            results.push(ReactionSearchResult {
                reaction_id: reaction_id.clone(),
                local_id,
                label: label_text,
                detail,
                name: reaction.name.clone(),
                subsystem: reaction.subsystem.clone(),
                species_id,
                drawn: self.map.as_ref().is_some_and(|m| m.has_reaction(reaction_id)),
                score,
            });
        }
        results.sort_by(|a, b| {
            a.score
                .cmp(&b.score)
                .then_with(|| js::locale_compare(&a.label, &b.label))
                .then_with(|| js::locale_compare(&a.species_id, &b.species_id))
        });
        results.truncate(limit.max(1));
        results
    }

    /// `reactionsAvailableFromMetabolite(nodeId)`: reactions of the model
    /// that touch a drawn metabolite but are not drawn yet, in the order of
    /// [`Store::order_by_flux`].
    pub fn reactions_available_from_metabolite(&self, node_id: &str) -> Vec<String> {
        let (Some(map), Some(community)) = (self.map.as_ref(), self.community.as_ref()) else { return Vec::new() };
        let Some(merged) = map.node(node_id).and_then(|n| n.merged_id.clone()).filter(|m| !m.is_empty()) else { return Vec::new() };
        let out = community
            .merged
            .reactions
            .iter()
            .filter(|(rid, r)| !map.has_reaction(rid) && r.metabolites.contains_key(&merged))
            .map(|(rid, _)| rid.clone())
            .collect();
        self.order_by_flux(out)
    }

    /// `orderByFlux(reactionIds)`: those that carry flux in the current
    /// optimal solution first, largest |flux| first. The others keep their
    /// order behind them, and without a solution the list is returned as it is.
    pub fn order_by_flux(&self, mut reaction_ids: Vec<String>) -> Vec<String> {
        let Some(solution) = self.solution.as_ref().filter(|s| s.is_optimal()) else { return reaction_ids };
        let size = |id: &String| {
            let value = solution.fluxes.get(id).copied().unwrap_or(0.0).abs();
            if value > 1e-9 { value } else { 0.0 }
        };
        // A stable sort, so equal sizes, zero among them, keep their order.
        reaction_ids.sort_by(|a, b| size(b).partial_cmp(&size(a)).unwrap_or(std::cmp::Ordering::Equal));
        reaction_ids
    }

    /// `metabolitesAvailableInCompartment(cellId, compartmentId)`.
    pub fn metabolites_available_in_compartment(&self, cell_id: &str, compartment_id: &str) -> Vec<AvailableMetabolite> {
        let (Some(map), Some(community)) = (self.map.as_ref(), self.community.as_ref()) else { return Vec::new() };
        let mut out = Vec::new();
        for id in community.merged.metabolites.keys() {
            let Some(owner) = community.metabolite_owner.get(id) else { continue };
            if owner.species_id.as_deref() != Some(cell_id) || owner.compartment != compartment_id {
                continue;
            }
            let named = self.entity_label("metabolite", id);
            // Beside the map's label, whichever of the name and the model id it is not.
            let detail = if named.label == named.name { named.id.clone() } else { named.name.clone() };
            out.push(AvailableMetabolite {
                id: id.clone(),
                label: named.label,
                name: detail,
                model_id: named.id,
                count: map.nodes_for_metabolite(id).len(),
            });
        }
        out.sort_by(|a, b| js::locale_compare(&a.label, &b.label));
        out
    }

    /// `addMapMetabolite(mergedId, { x, y })`.
    pub fn add_map_metabolite(&mut self, merged_id: &str, x: f64, y: f64) -> Option<Node> {
        let community = self.community.clone()?;
        let map = self.map.as_mut()?;
        let before = map.to_json();
        let node = map.add_metabolite(&community, merged_id, x, y)?.clone();
        self.record_map(&before, "Add metabolite");
        self.emit("map");
        Some(node)
    }

    /// `reactionsAvailableInCompartment(cellId, compartmentId)`: internal
    /// reactions of one compartment not drawn yet.
    pub fn reactions_available_in_compartment(&self, cell_id: &str, compartment_id: &str) -> Vec<String> {
        let (Some(map), Some(community)) = (self.map.as_ref(), self.community.as_ref()) else { return Vec::new() };
        reactions_in_compartment(community, cell_id, compartment_id).into_iter().filter(|r| !map.has_reaction(r)).collect()
    }

    /// `canRemoveMapNode(nodeId)`.
    pub fn can_remove_map_node(&self, node_id: &str) -> bool {
        self.map.as_ref().is_some_and(|m| m.can_remove_node(node_id))
    }

    /// `removeMapNode(nodeId)`.
    pub fn remove_map_node(&mut self, node_id: &str) -> Option<Node> {
        let map = self.map.as_mut()?;
        let before = map.to_json();
        let node = map.remove_node(node_id)?;
        self.record_map(&before, "Eliminar metabolito");
        if self.selected_entity.as_ref().is_some_and(|e| e.kind == "metabolite" && e.id == node_id) {
            self.selected_entity = None;
        }
        self.emit("map");
        Some(node)
    }

    // Selection

    /// `selectGeometry(cellId, compartmentId)`: select a compartment's
    /// geometry (the shared medium when `cell_id` is `None`), which is what
    /// puts its handles on screen.
    pub fn select_geometry(&mut self, cell_id: Option<&str>, compartment_id: &str) -> Option<GeometrySelection> {
        if !self.map.as_ref().is_some_and(|m| m.geometry_target(cell_id, compartment_id).is_some()) {
            self.selected_geometry = None;
            self.selected_entity = None;
            self.emit("geometry-selection");
            return None;
        }
        let selection = GeometrySelection { cell_id: cell_id.map(str::to_string), compartment_id: compartment_id.to_string() };
        self.selected_geometry = Some(selection.clone());
        self.selected_reaction = None;
        let mut entity = SelectedEntity::new("compartment", compartment_id);
        entity.cell_id = cell_id.map(str::to_string);
        self.selected_entity = Some(entity);
        self.variability = None;
        self.emit("geometry-selection");
        Some(selection)
    }

    /// `clearGeometrySelection()`.
    pub fn clear_geometry_selection(&mut self) {
        if self.selected_geometry.is_none() {
            return;
        }
        self.selected_geometry = None;
        if self.selected_entity.as_ref().is_some_and(|e| e.kind == "compartment") {
            self.selected_entity = None;
        }
        self.emit("geometry-selection");
    }

    /// `select(rid)`: select a reaction (`None` selects nothing).
    pub fn select(&mut self, rid: Option<&str>) {
        self.selected_reaction = rid.map(str::to_string);
        self.selected_entity = rid.map(|r| SelectedEntity::new("reaction", r));
        self.variability = match rid {
            Some(r) if self.fva_enabled => self.fva_ranges.get(r).copied(),
            _ => None,
        };
        self.emit("selection");
    }

    /// `selectEntity(type, id, details)`.
    pub fn select_entity(&mut self, kind: &str, id: &str, details: Map<String, Value>) -> Option<SelectedEntity> {
        if kind.is_empty() || id.is_empty() {
            self.select(None);
            return None;
        }
        self.selected_reaction = (kind == "reaction").then(|| id.to_string());
        self.selected_geometry = None;
        self.variability = if self.fva_enabled && kind == "reaction" { self.fva_ranges.get(id).copied() } else { None };
        let mut entity = SelectedEntity::new(kind, id);
        entity.cell_id = details.get("cellId").and_then(Value::as_str).map(str::to_string);
        entity.details = details;
        self.selected_entity = Some(entity.clone());
        self.emit("selection");
        Some(entity)
    }
}

/// `ItemKind` as the JS strings.
pub fn item_kind_str(kind: &ItemKind) -> &'static str {
    kind.as_str()
}
