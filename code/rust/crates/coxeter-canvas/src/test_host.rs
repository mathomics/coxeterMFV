//! A [`MapHost`] over a bare [`EditableMap`] and [`MapHistory`]: the map
//! half of the Store of `src/ui/store.js`, each method with the JavaScript
//! semantics, so the widget and its gestures can be driven and tested without
//! the application (`coxeter-state` implements the same trait on the real
//! Store).
//!
//! Where the Store reads the models (which cells and compartments can be drawn
//! next), this host is told: [`EditableMapHost::cells`] and
//! [`EditableMapHost::compartments`] list what the models would offer.

use coxeter_core::mapdata::{Point, Rect};
use coxeter_core::text_style::{clamp_text_size, normalize_text_font};
use coxeter_map::editable_map::{CellSpec, CompartmentSpec, TextOptions};
use coxeter_map::history::MapHistory;
use coxeter_map::multi_selection::{Moved, Selection, SelectionRect, selection_size};
use coxeter_map::reaction_edit::{ControlPoint, Handle};
use coxeter_map::EditableMap;
use coxeter_render::{GeometrySelection, MultiSelection, RenderState};

use crate::host::{
    CompartmentPlacement, MapEdit, MapHost, RenderOptions, ScrollInput, SelectKind, SelectedEntity, TextStyle, ViewPatch,
};

/// A species the Cell tool can draw (`availableMapCells()` entries).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CellTemplate {
    pub species_id: String,
    pub outer_compartment_id: String,
    pub outer_label: String,
    /// `model.cell?.name ?? sp.id`.
    pub cell_name: String,
    pub organism: String,
    pub note: String,
    /// `model.cell?.color ?? '#64748b'`.
    pub color: String,
    pub abundance: Option<f64>,
}

/// A compartment of a species' model (`compartmentTree()` nodes).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CompartmentTemplate {
    pub species_id: String,
    pub compartment_id: String,
    pub parent_id: String,
    pub label: String,
    pub depth: usize,
    pub cell_name: String,
    pub organism: String,
    pub note: String,
    pub color: String,
    pub abundance: Option<f64>,
}

/// `store.view`, the part the canvas reads.
#[derive(Debug, Clone, PartialEq)]
pub struct HostView {
    pub geometry_edit: bool,
    pub multi_select: bool,
    pub text_tool: bool,
    pub scroll_input: ScrollInput,
    pub text_style: TextStyle,
    pub show_currency: bool,
    pub dark: bool,
}

impl Default for HostView {
    fn default() -> Self {
        HostView {
            geometry_edit: false,
            multi_select: false,
            text_tool: false,
            scroll_input: ScrollInput::Auto,
            text_style: TextStyle::default(),
            show_currency: true,
            dark: false,
        }
    }
}

/// The map half of the Store.
#[derive(Debug, Clone, Default)]
pub struct EditableMapHost {
    pub map: Option<EditableMap>,
    pub history: MapHistory,
    pub selected_reaction: Option<String>,
    pub selected_entity: Option<SelectedEntity>,
    pub selected_geometry: Option<GeometrySelection>,
    pub multi_selection: Selection,
    pub view: HostView,
    /// What the Cell tool can draw.
    pub cells: Vec<CellTemplate>,
    /// The compartments of the models.
    pub compartments: Vec<CompartmentTemplate>,
    /// `store.cellPlacementSpeciesId`.
    pub cell_placement: Option<String>,
    /// `store.compartmentPlacementKey`: `species|compartment`.
    pub compartment_placement_key: Option<String>,
    /// The picked cross-feeding line: `(from, base, to, at)`.
    pub cross_feeding_pick: Option<(String, String, String, Option<Point>)>,
    /// `store.emit(reason)`, in order.
    pub emitted: Vec<String>,
    /// The fluxes the map is drawn with.
    pub render: Option<RenderState>,
}

impl EditableMapHost {
    pub fn new(map: EditableMap) -> EditableMapHost {
        EditableMapHost { map: Some(map), ..Default::default() }
    }

    fn emit(&mut self, reason: &str) {
        self.emitted.push(reason.to_string());
    }

    /// `availableMapCells()`: the species not drawn yet.
    pub fn available_map_cells(&self) -> Vec<&CellTemplate> {
        let Some(map) = &self.map else { return Vec::new() };
        self.cells.iter().filter(|c| !map.cells().iter().any(|d| d.id == c.species_id)).collect()
    }

    /// `availableMapCompartments()`: compartments whose parent is drawn and
    /// which are not, below the cell boundary.
    pub fn available_map_compartments(&self) -> Vec<&CompartmentTemplate> {
        let Some(map) = &self.map else { return Vec::new() };
        let mut out: Vec<&CompartmentTemplate> = self
            .compartments
            .iter()
            .filter(|c| {
                let Some(cell) = map.cells().iter().find(|d| d.id == c.species_id) else { return false };
                let drawn = |id: &str| cell.compartments.iter().any(|k| k.id == id);
                !c.parent_id.is_empty() && !drawn(&c.compartment_id) && drawn(&c.parent_id)
            })
            .collect();
        out.sort_by(|a, b| {
            coxeter_core::js::locale_compare(&a.species_id, &b.species_id)
                .then(a.depth.cmp(&b.depth))
                .then(coxeter_core::js::locale_compare(&a.compartment_id, &b.compartment_id))
        });
        out
    }

    fn selection_of(&mut self, kind: SelectKind) -> &mut indexmap::IndexSet<String> {
        match kind {
            SelectKind::Node => &mut self.multi_selection.nodes,
            SelectKind::Reaction => &mut self.multi_selection.reactions,
            SelectKind::Text => &mut self.multi_selection.texts,
        }
    }

    fn record(&mut self, before: &serde_json::Value, label: &str) -> bool {
        let Some(map) = &self.map else { return false };
        self.history.record(before, &map.to_json(), label)
    }
}

impl MapHost for EditableMapHost {
    fn map(&self) -> Option<&EditableMap> {
        self.map.as_ref()
    }
    fn selected_reaction(&self) -> Option<String> {
        self.selected_reaction.clone()
    }
    fn selected_entity(&self) -> Option<SelectedEntity> {
        self.selected_entity.clone()
    }
    fn selected_geometry(&self) -> Option<GeometrySelection> {
        self.selected_geometry.clone()
    }
    fn multi_selection(&self) -> Selection {
        self.multi_selection.clone()
    }
    fn multi_selection_size(&self) -> usize {
        selection_size(&self.multi_selection)
    }
    fn is_multi_selected(&self, kind: SelectKind, id: &str) -> bool {
        match kind {
            SelectKind::Node => self.multi_selection.nodes.contains(id),
            SelectKind::Reaction => self.multi_selection.reactions.contains(id),
            SelectKind::Text => self.multi_selection.texts.contains(id),
        }
    }
    fn geometry_edit(&self) -> bool {
        self.view.geometry_edit
    }
    fn multi_select(&self) -> bool {
        self.view.multi_select
    }
    fn text_tool(&self) -> bool {
        self.view.text_tool
    }
    fn scroll_input(&self) -> ScrollInput {
        self.view.scroll_input
    }
    fn text_style(&self) -> TextStyle {
        self.view.text_style.clone()
    }
    fn show_currency(&self) -> bool {
        self.view.show_currency
    }

    fn render_state(&self, opts: RenderOptions) -> RenderState {
        let mut state = self.render.clone().unwrap_or_default();
        state.dark = self.view.dark;
        state.selected_reaction = self.selected_reaction.clone();
        state.selected_geometry = self.selected_geometry.clone();
        state.geometry_edit = self.view.geometry_edit;
        state.reaction_edit = opts.reaction_edit;
        state.show_currency = self.view.show_currency;
        state.multi_selection = Some(MultiSelection {
            nodes: self.multi_selection.nodes.iter().cloned().collect(),
            reactions: self.multi_selection.reactions.iter().cloned().collect(),
            texts: self.multi_selection.texts.iter().cloned().collect(),
            compartments: self.multi_selection.compartments.iter().cloned().collect(),
        });
        state.min_hit_radius = opts.min_hit_radius;
        state
    }

    fn begin_map_edit(&mut self, label: &str) -> Option<MapEdit> {
        let map = self.map.as_ref()?;
        Some(MapEdit { label: label.to_string(), before: map.to_json() })
    }

    fn commit_map_edit(&mut self, edit: Option<MapEdit>) -> bool {
        let Some(edit) = edit else { return false };
        if self.map.is_none() {
            return false;
        }
        let changed = self.record(&edit.before, &edit.label);
        if changed {
            self.emit("history");
        }
        changed
    }

    fn restore_map_edit(&mut self, edit: &MapEdit) {
        if let Ok(map) = EditableMap::from_json(&edit.before) {
            self.map = Some(map);
            self.emit("map");
        }
    }

    fn undo_map(&mut self) -> bool {
        let Some(map) = &self.map else { return false };
        let Some(entry) = self.history.undo(&map.to_json()) else { return false };
        let Ok(restored) = EditableMap::from_json(&entry.doc) else { return false };
        self.map = Some(restored);
        self.selected_reaction = None;
        self.selected_entity = None;
        self.emit("map");
        true
    }

    fn redo_map(&mut self) -> bool {
        let Some(map) = &self.map else { return false };
        let Some(entry) = self.history.redo(&map.to_json()) else { return false };
        let Ok(restored) = EditableMap::from_json(&entry.doc) else { return false };
        self.map = Some(restored);
        self.selected_reaction = None;
        self.selected_entity = None;
        self.emit("map");
        true
    }

    fn can_move_node(&self, node_id: &str) -> bool {
        self.map.as_ref().is_some_and(|m| m.can_move_node(node_id))
    }
    fn move_node(&mut self, node_id: &str, x: f64, y: f64) -> Option<Point> {
        self.map.as_mut()?.move_node(node_id, x, y).map(|n| Point::new(n.x, n.y))
    }
    fn merge_overlapping_metabolite(&mut self, node_id: &str) -> Option<String> {
        let result = self.map.as_mut()?.merge_overlapping_metabolite(node_id)?;
        let target = result.target.id.clone();
        if let Some(e) = &mut self.selected_entity
            && e.kind == "metabolite" && e.id == node_id {
                e.id = target.clone();
            }
        Some(target)
    }
    fn can_move_reaction(&self, reaction_id: &str) -> bool {
        self.map.as_ref().is_some_and(|m| m.can_move_reaction(reaction_id))
    }
    fn move_reaction(&mut self, reaction_id: &str, x: f64, y: f64) -> Option<Point> {
        self.map.as_mut()?.move_reaction(reaction_id, x, y).map(|r| Point::new(r.x, r.y))
    }
    fn can_rotate_reaction(&self, reaction_id: &str) -> bool {
        self.map.as_ref().is_some_and(|m| m.can_rotate_reaction(reaction_id))
    }
    fn set_reaction_angle(&mut self, reaction_id: &str, angle: f64) -> bool {
        self.map.as_mut().is_some_and(|m| m.set_reaction_angle(reaction_id, angle))
    }
    fn reaction_control_points(&mut self, reaction_id: &str) -> Vec<ControlPoint> {
        self.map.as_mut().map(|m| m.reaction_control_points(reaction_id)).unwrap_or_default()
    }
    fn move_branch_control(&mut self, reaction_id: &str, node_id: &str, handle: Handle, x: f64, y: f64) -> Option<Point> {
        self.map.as_mut()?.move_branch_control(reaction_id, node_id, handle, x, y)
    }
    fn set_metabolite_label_angle(&mut self, node_id: &str, angle: f64) -> bool {
        self.map.as_mut().is_some_and(|m| m.set_metabolite_label_angle(node_id, angle).is_some())
    }
    fn restore_metabolite_label_angle(&mut self, node_id: &str, angle: Option<f64>) {
        if let Some(map) = &mut self.map {
            map.edit_graph(|g| {
                if let Some(n) = g.nodes.get_mut(node_id) {
                    n.label_angle = angle;
                }
            });
        }
    }
    fn move_map_text(&mut self, id: &str, x: f64, y: f64) -> Option<Point> {
        self.map.as_mut()?.move_text(id, x, y).map(|t| Point::new(t.x, t.y))
    }
    fn move_compartment_label(&mut self, cell_id: Option<&str>, compartment_id: &str, x: f64, y: f64) -> Option<Point> {
        self.map.as_mut()?.set_compartment_label_position(cell_id, compartment_id, x, y)
    }
    fn edit_geometry_rect(&mut self, cell_id: Option<&str>, compartment_id: &str, rect: Rect) -> Option<Rect> {
        self.map.as_mut()?.edit_geometry_rect(cell_id, compartment_id, rect).map(|e| e.rect)
    }
    fn edit_compartment_shape(&mut self, cell_id: Option<&str>, compartment_id: &str, handle: &str, amount: f64) -> bool {
        let patch = coxeter_map::geometry_edit::ShapePatch::handle(handle, amount);
        self.map.as_mut().is_some_and(|m| m.edit_compartment_shape(cell_id, compartment_id, patch).is_some())
    }
    fn preview_canvas_size(&self, width: f64, height: f64) -> Option<(f64, f64)> {
        let map = self.map.as_ref()?;
        let min = map.minimum_canvas_size(40.0);
        // `Number(width) || this.map.canvas.width`.
        let or = |v: f64, d: f64| if v == 0.0 || v.is_nan() { d } else { v };
        Some((
            coxeter_core::js::max(min.width, or(width, map.width())),
            coxeter_core::js::max(min.height, or(height, map.height())),
        ))
    }
    fn set_map_canvas_size(&mut self, width: f64, height: f64, shift_x: f64, shift_y: f64) -> Option<(f64, f64)> {
        let c = self.map.as_mut()?.set_canvas_size(Some(width), Some(height), shift_x, shift_y);
        Some((c.width, c.height))
    }

    fn toggle_multi_selection(&mut self, kind: SelectKind, id: &str) {
        if id.is_empty() {
            return;
        }
        let bucket = self.selection_of(kind);
        if !bucket.shift_remove(id) {
            bucket.insert(id.to_string());
        }
        self.emit("multi-selection");
    }
    fn select_in_rect(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, replace: bool) {
        let Some(map) = &self.map else { return };
        let found = map.selectable_in_rect(SelectionRect::from_corners(x1, y1, x2, y2), self.view.show_currency);
        if replace {
            self.multi_selection = Selection::default();
        }
        self.multi_selection.nodes.extend(found.nodes);
        self.multi_selection.reactions.extend(found.reactions);
        self.multi_selection.texts.extend(found.texts);
        self.multi_selection.compartments.extend(found.compartments);
        self.emit("multi-selection");
    }
    fn move_multi_selection(&mut self, dx: f64, dy: f64) -> Option<Moved> {
        let selection = self.multi_selection.clone();
        self.map.as_mut()?.drag_selection(&selection, dx, dy)
    }
    fn clear_multi_selection(&mut self) -> bool {
        if selection_size(&self.multi_selection) == 0 {
            return false;
        }
        self.multi_selection = Selection::default();
        self.emit("multi-selection");
        true
    }
    fn remove_multi_selection(&mut self) -> bool {
        let Some(map) = &mut self.map else { return false };
        if selection_size(&self.multi_selection) == 0 {
            return false;
        }
        let before = map.to_json();
        let reaction_ids: Vec<String> = self.multi_selection.reactions.iter().cloned().collect();
        let node_ids: Vec<String> = self.multi_selection.nodes.iter().cloned().collect();
        let text_ids: Vec<String> = self.multi_selection.texts.iter().cloned().collect();
        let (mut reactions, mut nodes, mut texts) = (Vec::new(), Vec::new(), Vec::new());
        // Whole compartments first, with what is drawn in them.
        let mut compartments = Vec::new();
        for (cell_id, compartment_id) in self.multi_selection.compartments.clone() {
            if let Some(result) = map.remove_compartment(&cell_id, &compartment_id) {
                compartments.extend(result.compartment_ids);
                reactions.extend(result.reactions);
                texts.extend(result.texts);
            }
        }
        // Reactions next: their reaction-local nodes may go with them.
        for id in reaction_ids {
            if map.remove_reaction(&id).is_some() {
                reactions.push(id);
            }
        }
        for id in node_ids {
            if !map.has_node(&id) || !map.can_remove_node(&id) {
                continue;
            }
            if map.remove_node(&id).is_some() {
                nodes.push(id);
            }
        }
        for id in text_ids {
            if map.remove_text(&id).is_some() {
                texts.push(id);
            }
        }
        if reactions.is_empty() && nodes.is_empty() && texts.is_empty() && compartments.is_empty() {
            return false;
        }
        let after = map.to_json();
        let node_gone = |id: &str| !map.has_node(id);
        let entity_gone = self.selected_entity.as_ref().is_some_and(|e| {
            (e.kind == "reaction" && reactions.contains(&e.id)) || (e.kind == "metabolite" && node_gone(&e.id)) || (e.kind == "text" && texts.contains(&e.id))
        });
        self.history.record(&before, &after, "Remove selection");
        self.multi_selection = Selection::default();
        if self.selected_reaction.as_ref().is_some_and(|r| reactions.contains(r)) {
            self.selected_reaction = None;
        }
        if entity_gone {
            self.selected_entity = None;
        }
        self.emit("multi-selection");
        self.emit("map");
        true
    }

    fn select(&mut self, reaction_id: Option<&str>) {
        self.selected_reaction = reaction_id.map(str::to_string);
        self.selected_entity = reaction_id.map(|id| SelectedEntity { kind: "reaction".into(), id: id.into(), cell_id: None });
        self.emit("selection");
    }
    fn select_entity(&mut self, kind: &str, id: &str, cell_id: Option<&str>) {
        if kind.is_empty() || id.is_empty() {
            self.select(None);
            return;
        }
        self.selected_reaction = (kind == "reaction").then(|| id.to_string());
        self.selected_geometry = None;
        self.selected_entity = Some(SelectedEntity { kind: kind.into(), id: id.into(), cell_id: cell_id.map(str::to_string) });
        self.emit("selection");
    }
    fn select_geometry(&mut self, cell_id: Option<&str>, compartment_id: &str) {
        let exists = self.map.as_ref().and_then(|m| m.geometry_target(cell_id, compartment_id)).is_some();
        if !exists {
            self.selected_geometry = None;
            self.selected_entity = None;
            self.emit("geometry-selection");
            return;
        }
        self.selected_geometry = Some(GeometrySelection { cell_id: cell_id.map(str::to_string), compartment_id: compartment_id.into() });
        self.selected_reaction = None;
        self.selected_entity =
            Some(SelectedEntity { kind: "compartment".into(), id: compartment_id.into(), cell_id: cell_id.map(str::to_string) });
        self.emit("geometry-selection");
    }
    fn select_cell_geometry(&mut self, cell_id: &str) {
        let Some(map) = &self.map else { return };
        let Some(cell) = map.cells().iter().find(|c| c.id == cell_id) else { return };
        let outer = cell
            .compartments
            .iter()
            .find(|c| Some(&c.id) == cell.outer_compartment.as_ref())
            .or_else(|| cell.compartments.iter().find(|c| c.parent_id.is_none()));
        let Some(outer) = outer else { return };
        self.selected_reaction = None;
        self.selected_geometry = Some(GeometrySelection { cell_id: Some(cell.id.clone()), compartment_id: outer.id.clone() });
        self.selected_entity = Some(SelectedEntity { kind: "cell".into(), id: cell.id.clone(), cell_id: None });
        self.emit("geometry-selection");
    }
    fn clear_geometry_selection(&mut self) {
        if self.selected_geometry.is_none() {
            return;
        }
        self.selected_geometry = None;
        if self.selected_entity.as_ref().is_some_and(|e| e.kind == "compartment") {
            self.selected_entity = None;
        }
        self.emit("geometry-selection");
    }
    fn toggle_cross_feeding_link(&mut self, from: &str, base: &str, to: &str, at: Option<Point>) {
        let same = self.cross_feeding_pick.as_ref().is_some_and(|(f, b, t, _)| f == from && b == base && t == to);
        self.cross_feeding_pick = if same { None } else { Some((from.into(), base.into(), to.into(), at)) };
        self.emit("cross-feeding-explorer");
    }
    fn clear_cross_feeding_pick(&mut self) {
        if self.cross_feeding_pick.take().is_some() {
            self.emit("cross-feeding-explorer");
        }
    }
    fn set_view(&mut self, patch: ViewPatch) {
        if let Some(v) = patch.text_tool {
            self.view.text_tool = v;
        }
        if let Some(v) = patch.multi_select {
            self.view.multi_select = v;
        }
        if let Some(v) = patch.geometry_edit {
            self.view.geometry_edit = v;
            if v {
                self.selected_reaction = None;
                self.selected_entity = None;
            } else {
                self.selected_geometry = None;
            }
        }
        self.emit("view");
    }

    fn cell_placement_species_id(&self) -> Option<String> {
        let available = self.available_map_cells();
        self.cell_placement.clone().filter(|s| !s.is_empty()).or_else(|| available.first().map(|c| c.species_id.clone()))
    }
    fn add_map_cell(&mut self, species_id: &str, rect: Rect) -> Option<String> {
        let spec = {
            let available = self.available_map_cells();
            available
                .iter()
                .find(|c| c.species_id == species_id)
                .or_else(|| available.iter().find(|c| Some(&c.species_id) == self.cell_placement.as_ref()))
                .or_else(|| available.first())
                .map(|c| (*c).clone())?
        };
        let map = self.map.as_mut()?;
        let before = map.to_json();
        let added = map.add_cell(&CellSpec {
            species_id: spec.species_id.clone(),
            outer_compartment_id: Some(spec.outer_compartment_id.clone()),
            outer_label: Some(spec.outer_label.clone()),
            cell_name: Some(spec.cell_name.clone()),
            organism: Some(spec.organism.clone()),
            note: Some(spec.note.clone()),
            color: Some(if spec.color.is_empty() { "#64748b".into() } else { spec.color.clone() }),
            abundance: spec.abundance,
            rect: Some(rect),
            ..Default::default()
        })?;
        self.record(&before, "Create cell");
        self.cell_placement = self.available_map_cells().first().map(|c| c.species_id.clone());
        self.selected_reaction = None;
        self.selected_geometry =
            Some(GeometrySelection { cell_id: Some(added.cell.id.clone()), compartment_id: added.compartment.id.clone() });
        self.selected_entity = Some(SelectedEntity { kind: "cell".into(), id: added.cell.id.clone(), cell_id: None });
        self.emit("map");
        Some(added.cell.id)
    }
    fn current_compartment_placement(&mut self) -> Option<CompartmentPlacement> {
        let selected = {
            let available = self.available_map_compartments();
            let key = self.compartment_placement_key.clone();
            available
                .iter()
                .find(|c| Some(format!("{}|{}", c.species_id, c.compartment_id)) == key)
                .or_else(|| available.first())
                .map(|c| (*c).clone())
        }?;
        if self.compartment_placement_key.is_none() {
            self.compartment_placement_key = Some(format!("{}|{}", selected.species_id, selected.compartment_id));
        }
        Some(CompartmentPlacement {
            species_id: selected.species_id,
            compartment_id: selected.compartment_id,
            parent_id: Some(selected.parent_id),
        })
    }
    fn compartment_parent_rect(&self, species_id: &str, compartment_id: &str) -> Option<Rect> {
        let available = self.available_map_compartments();
        let spec = available.iter().find(|c| c.species_id == species_id && c.compartment_id == compartment_id)?;
        let cell = self.map.as_ref()?.cells().iter().find(|c| c.id == species_id)?;
        cell.compartments.iter().find(|c| c.id == spec.parent_id).map(|c| c.rect)
    }
    fn add_map_compartment(&mut self, species_id: &str, compartment_id: &str, rect: Rect) -> bool {
        let Some(spec) = self.compartments.iter().find(|c| c.species_id == species_id && c.compartment_id == compartment_id).cloned()
        else {
            return false;
        };
        let Some(map) = self.map.as_mut() else { return false };
        if spec.parent_id.is_empty() || !map.cells().iter().any(|c| c.id == species_id) {
            return false;
        }
        let before = map.to_json();
        let added = map.add_compartment(&CompartmentSpec {
            species_id: species_id.into(),
            compartment_id: compartment_id.into(),
            parent_id: Some(spec.parent_id.clone()),
            label: Some(spec.label.clone()),
            cell_name: Some(if spec.cell_name.is_empty() { species_id.to_string() } else { spec.cell_name.clone() }),
            organism: Some(spec.organism.clone()),
            note: Some(spec.note.clone()),
            color: Some(if spec.color.is_empty() { "#64748b".into() } else { spec.color.clone() }),
            abundance: spec.abundance,
            rect: Some(rect),
            ..Default::default()
        });
        if added.is_none() {
            return false;
        }
        self.record(&before, "Agregar compartimento");
        self.selected_reaction = None;
        self.selected_geometry = Some(GeometrySelection { cell_id: Some(species_id.into()), compartment_id: compartment_id.into() });
        self.selected_entity =
            Some(SelectedEntity { kind: "compartment".into(), id: compartment_id.into(), cell_id: Some(species_id.into()) });
        self.compartment_placement_key =
            self.available_map_compartments().first().map(|c| format!("{}|{}", c.species_id, c.compartment_id));
        self.emit("map");
        true
    }

    fn remove_map_reaction(&mut self, reaction_id: &str) -> bool {
        let Some(map) = self.map.as_mut() else { return false };
        let before = map.to_json();
        let Some(result) = map.remove_reaction(reaction_id) else { return false };
        self.record(&before, "Remove reaction");
        if self.selected_reaction.as_deref() == Some(reaction_id) {
            self.selected_reaction = None;
            self.selected_entity = None;
        }
        self.multi_selection.reactions.shift_remove(reaction_id);
        for id in &result.nodes {
            self.multi_selection.nodes.shift_remove(id);
        }
        self.emit("map");
        true
    }
    fn can_remove_map_node(&self, node_id: &str) -> bool {
        self.map.as_ref().is_some_and(|m| m.can_remove_node(node_id))
    }
    fn remove_map_node(&mut self, node_id: &str) -> bool {
        let Some(map) = self.map.as_mut() else { return false };
        let before = map.to_json();
        if map.remove_node(node_id).is_none() {
            return false;
        }
        self.record(&before, "Eliminar metabolito");
        if self.selected_entity.as_ref().is_some_and(|e| e.kind == "metabolite" && e.id == node_id) {
            self.selected_entity = None;
        }
        self.emit("map");
        true
    }

    fn remove_map_compartment(&mut self, cell_id: &str, compartment_id: &str) -> bool {
        let Some(map) = self.map.as_mut() else { return false };
        let before = map.to_json();
        let Some(result) = map.remove_compartment(cell_id, compartment_id) else { return false };
        self.record(&before, if result.cell_removed { "Remove cell" } else { "Remove compartment" });
        self.selected_geometry = None;
        let map = self.map.as_ref().expect("checked");
        if self.selected_entity.as_ref().is_some_and(|e| {
            matches!(e.kind.as_str(), "compartment" | "cell")
                || (e.kind == "reaction" && result.reactions.contains(&e.id))
                || (e.kind == "metabolite" && !map.has_node(&e.id))
        }) {
            self.selected_entity = None;
        }
        if self.selected_reaction.as_ref().is_some_and(|r| !map.has_reaction(r)) {
            self.selected_reaction = None;
        }
        self.multi_selection = Selection::default();
        self.emit("map");
        true
    }

    fn add_map_text(&mut self, text: &str, x: f64, y: f64, style: &TextStyle) -> bool {
        let Some(map) = self.map.as_mut() else { return false };
        let before = map.to_json();
        let opts = TextOptions { id: None, font_size: Some(style.font_size), font_family: Some(style.font_family.clone()) };
        if map.add_text(text, x, y, &opts).is_none() {
            return false;
        }
        self.record(&before, "Add text");
        self.emit("map");
        true
    }
    fn update_map_text(&mut self, id: &str, text: &str, style: Option<&TextStyle>) -> bool {
        let Some(map) = self.map.as_mut() else { return false };
        let before = map.to_json();
        if map.update_text(id, text).is_none() {
            return false;
        }
        if let Some(style) = style {
            map.style_text(id, Some(style.font_size), Some(&style.font_family));
        }
        self.record(&before, "Edit text");
        self.emit("map");
        true
    }
    fn remove_map_text(&mut self, id: &str) -> bool {
        let Some(map) = self.map.as_mut() else { return false };
        let before = map.to_json();
        if map.remove_text(id).is_none() {
            return false;
        }
        self.record(&before, "Eliminar texto");
        if self.selected_entity.as_ref().is_some_and(|e| e.kind == "text" && e.id == id) {
            self.selected_entity = None;
        }
        self.emit("map");
        true
    }
    fn set_text_style(&mut self, style: &TextStyle) {
        self.view.text_style =
            TextStyle { font_size: clamp_text_size(style.font_size), font_family: normalize_text_font(&style.font_family).to_string() };
    }
}
