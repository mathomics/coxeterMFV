//! The Store, as the map widget sees it: `coxeter_canvas::MapHost` over
//! `coxeter_state::Store`, by delegation to the store methods of the same
//! name (`src/ui/store.js`). In JavaScript the gestures of `app.js` call the
//! global `store` directly; here they go through this adapter.
//!
//! The two traits disagree only in what they return: the canvas wants the
//! few values a gesture reads back (a position, an id, whether anything
//! changed), the Store returns the objects the JS methods return.

use coxeter_canvas::host::{
    CompartmentPlacement, MapEdit, RenderOptions, ScrollInput, SelectKind, SelectedEntity, TextStyle, ViewPatch,
};
use coxeter_canvas::MapHost;
use coxeter_core::mapdata::{Point, Rect};
use coxeter_map::EditableMap;
use coxeter_map::multi_selection::{Moved, Selection, SelectionRect};
use coxeter_map::reaction_edit::{ControlPoint, Handle};
use coxeter_render::{GeometrySelection, RenderState};
use coxeter_state::Store;

/// `&mut Store` as a [`MapHost`].
pub struct StoreHost<'a> {
    pub store: &'a mut Store,
    /// `ui.tool === 'edit'`, which `mapRenderState()` reads from the interface.
    pub reaction_edit: bool,
}

impl<'a> StoreHost<'a> {
    pub fn new(store: &'a mut Store) -> StoreHost<'a> {
        StoreHost { store, reaction_edit: false }
    }
}

fn kind(k: SelectKind) -> &'static str {
    k.as_str()
}

fn store_style(style: &TextStyle) -> coxeter_state::TextStyle {
    coxeter_state::TextStyle { font_size: style.font_size, font_family: style.font_family.clone() }
}

impl MapHost for StoreHost<'_> {
    // Reading

    fn map(&self) -> Option<&EditableMap> {
        self.store.map.as_ref()
    }
    fn selected_reaction(&self) -> Option<String> {
        self.store.selected_reaction.clone()
    }
    fn selected_entity(&self) -> Option<SelectedEntity> {
        self.store.selected_entity.as_ref().map(|e| SelectedEntity { kind: e.kind.clone(), id: e.id.clone(), cell_id: e.cell_id.clone() })
    }
    fn selected_geometry(&self) -> Option<GeometrySelection> {
        self.store.selected_geometry.clone()
    }
    fn multi_selection(&self) -> Selection {
        self.store.multi_selection.clone()
    }
    fn multi_selection_size(&self) -> usize {
        self.store.multi_selection_size()
    }
    fn is_multi_selected(&self, k: SelectKind, id: &str) -> bool {
        self.store.is_multi_selected(kind(k), id)
    }
    fn geometry_edit(&self) -> bool {
        self.store.view.geometry_edit
    }
    fn multi_select(&self) -> bool {
        self.store.view.multi_select
    }
    fn text_tool(&self) -> bool {
        self.store.view.text_tool
    }
    fn scroll_input(&self) -> ScrollInput {
        match self.store.view.scroll_input.as_str() {
            "mouse" => ScrollInput::Mouse,
            "touchpad" => ScrollInput::Touchpad,
            _ => ScrollInput::Auto,
        }
    }
    fn text_style(&self) -> TextStyle {
        let s = &self.store.view.text_style;
        TextStyle { font_size: s.font_size, font_family: s.font_family.clone() }
    }
    fn show_currency(&self) -> bool {
        self.store.view.show_currency
    }
    fn render_state(&self, opts: RenderOptions) -> RenderState {
        self.store.map_render_state(coxeter_state::RenderOptions {
            reaction_edit: opts.reaction_edit || self.reaction_edit,
            min_hit_radius: opts.min_hit_radius,
        })
    }
    fn available_reactions(&self) -> usize {
        self.store.community.as_ref().map_or(0, |c| c.merged.reactions.len())
    }
    fn core_overview_supported(&self) -> bool {
        self.store.core_metabolism_overview_support().is_some_and(|s| s.supported)
    }

    // Edit history

    fn begin_map_edit(&mut self, label: &str) -> Option<MapEdit> {
        self.store.begin_map_edit(label).map(|t| MapEdit { label: t.label, before: t.before })
    }
    fn commit_map_edit(&mut self, edit: Option<MapEdit>) -> bool {
        self.store.commit_map_edit(edit.map(|e| coxeter_state::MapTransaction { label: e.label, before: e.before }))
    }
    fn restore_map_edit(&mut self, edit: &MapEdit) {
        // `store.map = EditableMap.fromJSON(before)`: no history, no selection change.
        if let Ok(map) = EditableMap::from_json(&edit.before) {
            self.store.map = Some(map);
            self.store.emit("map");
        }
    }
    fn undo_map(&mut self) -> bool {
        self.store.undo_map().is_some()
    }
    fn redo_map(&mut self) -> bool {
        self.store.redo_map().is_some()
    }

    // Moving things

    fn can_move_node(&self, node_id: &str) -> bool {
        self.store.can_move_node(node_id)
    }
    fn move_node(&mut self, node_id: &str, x: f64, y: f64) -> Option<Point> {
        self.store.move_node(node_id, x, y)
    }
    fn merge_overlapping_metabolite(&mut self, node_id: &str) -> Option<String> {
        self.store.merge_overlapping_metabolite(node_id).map(|m| m.target.id)
    }
    fn can_move_reaction(&self, reaction_id: &str) -> bool {
        self.store.can_move_reaction(reaction_id)
    }
    fn move_reaction(&mut self, reaction_id: &str, x: f64, y: f64) -> Option<Point> {
        self.store.move_reaction(reaction_id, x, y)
    }
    fn can_rotate_reaction(&self, reaction_id: &str) -> bool {
        self.store.can_rotate_reaction(reaction_id)
    }
    fn set_reaction_angle(&mut self, reaction_id: &str, angle: f64) -> bool {
        self.store.set_reaction_angle(reaction_id, angle)
    }
    fn reaction_control_points(&mut self, reaction_id: &str) -> Vec<ControlPoint> {
        self.store.map.as_mut().map(|m| m.reaction_control_points(reaction_id)).unwrap_or_default()
    }
    fn move_branch_control(&mut self, reaction_id: &str, node_id: &str, handle: Handle, x: f64, y: f64) -> Option<Point> {
        self.store.move_branch_control(reaction_id, node_id, handle, x, y)
    }
    fn set_metabolite_label_angle(&mut self, node_id: &str, angle: f64) -> bool {
        self.store.set_metabolite_label_angle(node_id, angle)
    }
    fn restore_metabolite_label_angle(&mut self, node_id: &str, angle: Option<f64>) {
        if let Some(map) = &mut self.store.map {
            map.edit_graph(|g| {
                if let Some(n) = g.nodes.get_mut(node_id) {
                    n.label_angle = angle;
                }
            });
        }
    }
    fn set_reaction_label_offset(&mut self, reaction_id: &str, offset: Point) -> bool {
        self.store.set_reaction_label_offset(reaction_id, offset)
    }
    fn restore_reaction_label_offset(&mut self, reaction_id: &str, offset: Option<Point>) {
        if let Some(map) = &mut self.store.map {
            map.edit_graph(|g| {
                if let Some(rxn) = g.reactions.get_mut(reaction_id) {
                    coxeter_core::reaction_label::set_reaction_label_offset(rxn, offset);
                }
            });
        }
    }
    fn move_map_text(&mut self, id: &str, x: f64, y: f64) -> Option<Point> {
        self.store.move_map_text(id, x, y)
    }
    fn move_compartment_label(&mut self, cell_id: Option<&str>, compartment_id: &str, x: f64, y: f64) -> Option<Point> {
        self.store.move_compartment_label(cell_id, compartment_id, x, y)
    }
    fn edit_geometry_rect(&mut self, cell_id: Option<&str>, compartment_id: &str, rect: Rect) -> Option<Rect> {
        self.store.edit_geometry_rect(cell_id, compartment_id, rect).map(|e| e.rect)
    }
    fn edit_compartment_shape(&mut self, cell_id: Option<&str>, compartment_id: &str, handle: &str, amount: f64) -> bool {
        let patch = coxeter_map::geometry_edit::ShapePatch::handle(handle, amount);
        self.store.edit_compartment_shape(cell_id, compartment_id, patch).is_some()
    }
    fn preview_canvas_size(&self, width: f64, height: f64) -> Option<(f64, f64)> {
        self.store.preview_canvas_size(width, height).map(|c| (c.width, c.height))
    }
    fn set_map_canvas_size(&mut self, width: f64, height: f64, shift_x: f64, shift_y: f64) -> Option<(f64, f64)> {
        self.store.set_map_canvas_size(Some(width), Some(height), shift_x, shift_y).map(|c| (c.width, c.height))
    }

    // Multi-selection

    fn toggle_multi_selection(&mut self, k: SelectKind, id: &str) {
        self.store.toggle_multi_selection(kind(k), id);
    }
    fn extend_multi_selection(&mut self, k: SelectKind, id: &str) {
        self.store.extend_multi_selection(kind(k), id);
    }
    fn select_in_rect(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, replace: bool) {
        self.store.select_in_rect(SelectionRect::from_corners(x1, y1, x2, y2), replace);
    }
    fn move_multi_selection(&mut self, dx: f64, dy: f64) -> Option<Moved> {
        self.store.move_multi_selection(dx, dy)
    }
    fn clear_multi_selection(&mut self) -> bool {
        self.store.clear_multi_selection(true)
    }
    fn remove_multi_selection(&mut self) -> bool {
        self.store.remove_multi_selection().is_some()
    }

    // Selection

    fn select(&mut self, reaction_id: Option<&str>) {
        self.store.select(reaction_id);
    }
    fn select_entity(&mut self, kind: &str, id: &str, cell_id: Option<&str>) {
        let mut details = serde_json::Map::new();
        if let Some(cell) = cell_id {
            details.insert("cellId".into(), serde_json::Value::String(cell.to_string()));
        }
        self.store.select_entity(kind, id, details);
    }
    fn select_geometry(&mut self, cell_id: Option<&str>, compartment_id: &str) {
        self.store.select_geometry(cell_id, compartment_id);
    }
    fn select_cell_geometry(&mut self, cell_id: &str) {
        self.store.select_cell_geometry(cell_id);
    }
    fn clear_geometry_selection(&mut self) {
        self.store.clear_geometry_selection();
    }
    fn toggle_cross_feeding_link(&mut self, from: &str, base: &str, to: &str, at: Option<Point>) {
        self.store.toggle_cross_feeding_link(from, base, to, at);
    }
    fn clear_cross_feeding_pick(&mut self) {
        self.store.clear_cross_feeding_pick();
    }
    fn set_view(&mut self, patch: ViewPatch) {
        // Only flags: nothing here can be refused.
        let _ = self.store.set_view(coxeter_state::ViewPatch {
            text_tool: patch.text_tool,
            geometry_edit: patch.geometry_edit,
            multi_select: patch.multi_select,
            ..Default::default()
        });
    }

    // Cells and compartments

    fn cell_placement_species_id(&self) -> Option<String> {
        // `cellPlacementSpeciesId()`: the sidebar's choice (kept in the store
        // here), else the first species still without a cell.
        let available = self.store.available_map_cells();
        self.store
            .cell_placement_species_id
            .clone()
            .filter(|id| available.iter().any(|c| &c.species_id == id))
            .or_else(|| available.first().map(|c| c.species_id.clone()))
    }
    fn add_map_cell(&mut self, species_id: &str, rect: Rect) -> Option<String> {
        self.store.add_map_cell(species_id, Some(rect)).map(|s| s.cell.id)
    }
    fn current_compartment_placement(&mut self) -> Option<CompartmentPlacement> {
        self.store.current_compartment_placement().map(|c| CompartmentPlacement {
            species_id: c.species_id,
            compartment_id: c.compartment_id,
            parent_id: c.parent_id,
        })
    }
    fn compartment_parent_rect(&self, species_id: &str, compartment_id: &str) -> Option<Rect> {
        self.store.compartment_parent_rect(species_id, compartment_id)
    }
    fn add_map_compartment(&mut self, species_id: &str, compartment_id: &str, rect: Rect) -> bool {
        self.store.add_map_compartment(species_id, compartment_id, Some(rect)).is_some()
    }

    // Removing

    fn remove_map_reaction(&mut self, reaction_id: &str) -> bool {
        self.store.remove_map_reaction(reaction_id).is_some()
    }
    fn can_remove_map_node(&self, node_id: &str) -> bool {
        self.store.can_remove_map_node(node_id)
    }
    fn remove_map_node(&mut self, node_id: &str) -> bool {
        self.store.remove_map_node(node_id).is_some()
    }
    fn remove_map_compartment(&mut self, cell_id: &str, compartment_id: &str) -> bool {
        self.store.remove_map_compartment(cell_id, compartment_id).is_some()
    }

    // Text

    fn add_map_text(&mut self, text: &str, x: f64, y: f64, style: &TextStyle) -> bool {
        self.store.add_map_text(text, x, y, Some(&store_style(style))).is_some()
    }
    fn update_map_text(&mut self, id: &str, text: &str, style: Option<&TextStyle>) -> bool {
        let style = style.map(|s| (Some(s.font_size), Some(s.font_family.as_str())));
        self.store.update_map_text(id, text, style).is_some()
    }
    fn remove_map_text(&mut self, id: &str) -> bool {
        self.store.remove_map_text(id).is_some()
    }
    fn set_text_style(&mut self, style: &TextStyle) {
        self.store.set_text_style(Some(style.font_size), Some(&style.font_family));
    }
}
