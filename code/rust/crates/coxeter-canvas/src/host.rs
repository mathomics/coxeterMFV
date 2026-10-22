//! What the map widget needs from the application: the store methods its
//! gestures call (`src/ui/store.js`, by their JavaScript names in snake_case),
//! and read access to the map and the selection.
//!
//! The widget never owns the map. In JavaScript the gestures of `app.js` act
//! through the global `store`; here they act through a [`MapHost`], which the
//! Store of `coxeter-state` implements. [`crate::test_host::EditableMapHost`]
//! implements it over a bare [`EditableMap`] and [`MapHistory`], with the JS
//! semantics of each method, so the gestures are tested on their own.
//!
//! Return values carry what the gestures read from the objects the JS methods
//! return (a moved node's new position, the target of a merge), not the
//! objects themselves.
//!
//! [`MapHistory`]: coxeter_map::history::MapHistory

use coxeter_core::mapdata::{Point, Rect};
use coxeter_map::EditableMap;
use coxeter_map::multi_selection::{Moved, Selection};
use coxeter_map::reaction_edit::{ControlPoint, Handle};
use coxeter_render::{GeometrySelection, RenderState};
use serde_json::Value;

/// What `mapRenderState()` reads from the interface rather than the store:
/// the active tool and the camera.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RenderOptions {
    /// `ui.tool === 'edit'`: show the control points of every drawn reaction.
    pub reaction_edit: bool,
    /// `MIN_HIT_PX / camera.scale`: the smallest pointer target in map units.
    pub min_hit_radius: f64,
}

/// What `store.beginMapEdit(label)` returns: the map before a multi-step edit
/// such as a drag, and the label the history will show for it.
#[derive(Debug, Clone, PartialEq)]
pub struct MapEdit {
    pub label: String,
    /// `map.toJSON()` when the edit began.
    pub before: Value,
}

/// `store.selectedEntity`: what the inspector shows.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectedEntity {
    /// `reaction`, `metabolite`, `text`, `cell` or `compartment`.
    pub kind: String,
    pub id: String,
    /// The cell of a selected compartment (`details.cellId`).
    pub cell_id: Option<String>,
}

/// The kinds a multi-selection holds (`node`, `reaction`, `text`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SelectKind {
    Node,
    Reaction,
    Text,
}

impl SelectKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SelectKind::Node => "node",
            SelectKind::Reaction => "reaction",
            SelectKind::Text => "text",
        }
    }
}

/// `store.currentCompartmentPlacement()`: the compartment the Compartment tool
/// draws next, inside the drawn parent it belongs in.
#[derive(Debug, Clone, PartialEq)]
pub struct CompartmentPlacement {
    pub species_id: String,
    pub compartment_id: String,
    pub parent_id: Option<String>,
}

/// `store.view.textStyle`: the size and font new texts are set in.
#[derive(Debug, Clone, PartialEq)]
pub struct TextStyle {
    pub font_size: f64,
    /// A key of `TEXT_FONTS` (`sans`, `serif`, …).
    pub font_family: String,
}

impl Default for TextStyle {
    fn default() -> Self {
        TextStyle {
            font_size: coxeter_core::text_style::TEXT_SIZE.default,
            font_family: coxeter_core::text_style::DEFAULT_TEXT_FONT.to_string(),
        }
    }
}

/// `store.view.scrollInput`: how wheel events are read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScrollInput {
    #[default]
    Auto,
    Mouse,
    Touchpad,
}

impl ScrollInput {
    pub fn as_str(self) -> &'static str {
        match self {
            ScrollInput::Auto => "auto",
            ScrollInput::Mouse => "mouse",
            ScrollInput::Touchpad => "touchpad",
        }
    }
}

/// The part of `store.setView(patch)` the canvas uses (`activateTool`, Escape).
/// A field left `None` is not in the patch.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ViewPatch {
    pub text_tool: Option<bool>,
    pub geometry_edit: Option<bool>,
    pub multi_select: Option<bool>,
}

/// The store, as the map widget sees it.
///
/// Every method is the store method of the same name in `src/ui/store.js`
/// and has its semantics: the `move_*`, `set_*_angle`, `edit_geometry_rect`
/// and `set_map_canvas_size` calls of a drag neither record history nor emit
/// (the gesture records one edit through [`MapHost::begin_map_edit`] /
/// [`MapHost::commit_map_edit`] when it ends); the selection methods emit.
pub trait MapHost {
    // Reading

    /// `store.map` (alias `store.layout`): what is drawn, if anything.
    fn map(&self) -> Option<&EditableMap>;
    /// `store.selectedReaction`.
    fn selected_reaction(&self) -> Option<String>;
    /// `store.selectedEntity`.
    fn selected_entity(&self) -> Option<SelectedEntity>;
    /// `store.selectedGeometry`.
    fn selected_geometry(&self) -> Option<GeometrySelection>;
    /// `store.multiSelection`.
    fn multi_selection(&self) -> Selection;
    /// `store.multiSelectionSize`.
    fn multi_selection_size(&self) -> usize;
    /// `store.isMultiSelected(kind, id)`.
    fn is_multi_selected(&self, kind: SelectKind, id: &str) -> bool;
    /// `store.view.geometryEdit`.
    fn geometry_edit(&self) -> bool;
    /// `store.view.multiSelect`.
    fn multi_select(&self) -> bool;
    /// `store.view.textTool`.
    fn text_tool(&self) -> bool;
    /// `store.view.scrollInput`.
    fn scroll_input(&self) -> ScrollInput;
    /// `store.view.textStyle`.
    fn text_style(&self) -> TextStyle;
    /// `store.view.showCurrency`.
    fn show_currency(&self) -> bool;
    /// `mapRenderState()`: everything the renderer needs to draw the map as
    /// it stands (`store.view.dark` included, which picks the page theme).
    fn render_state(&self, opts: RenderOptions) -> RenderState;
    /// `store.community?.merged?.reactions?.size ?? 0`: what the empty-map
    /// card says is waiting to be drawn.
    fn available_reactions(&self) -> usize {
        0
    }
    /// `store.coreMetabolismOverviewSupport?.()?.supported`.
    fn core_overview_supported(&self) -> bool {
        false
    }

    // Edit history

    /// `beginMapEdit(label)`: capture the drawing before a multi-step edit.
    fn begin_map_edit(&mut self, label: &str) -> Option<MapEdit>;
    /// `commitMapEdit(transaction)`: record it if it changed the map.
    fn commit_map_edit(&mut self, edit: Option<MapEdit>) -> bool;
    /// Put the map back as it was when `edit` began, recording nothing: a
    /// drag Escape cancelled. (Not a store method of the JavaScript app, which
    /// cannot cancel a drag; it is `store.map = EditableMap.fromJSON(before)`
    /// and an emit.)
    fn restore_map_edit(&mut self, edit: &MapEdit);
    /// `undoMap()`: whether there was anything to undo.
    fn undo_map(&mut self) -> bool;
    /// `redoMap()`: whether there was anything to redo.
    fn redo_map(&mut self) -> bool;

    // Moving things

    /// `canMoveNode(nodeId)`.
    fn can_move_node(&self, node_id: &str) -> bool;
    /// `moveNode(nodeId, x, y)`: where the node ended up, or `None`.
    fn move_node(&mut self, node_id: &str, x: f64, y: f64) -> Option<Point>;
    /// `mergeOverlappingMetabolite(nodeId)`: the id of the node it joined.
    fn merge_overlapping_metabolite(&mut self, node_id: &str) -> Option<String>;
    /// `canMoveReaction(reactionId)`.
    fn can_move_reaction(&self, reaction_id: &str) -> bool;
    /// `moveReaction(reactionId, x, y)`: where the reaction ended up.
    fn move_reaction(&mut self, reaction_id: &str, x: f64, y: f64) -> Option<Point>;
    /// `canRotateReaction(reactionId)`.
    fn can_rotate_reaction(&self, reaction_id: &str) -> bool;
    /// `setReactionAngle(reactionId, angle)`.
    fn set_reaction_angle(&mut self, reaction_id: &str, angle: f64) -> bool;
    /// `layout.reactionAngle(reactionId)`.
    fn reaction_angle(&self, reaction_id: &str) -> Option<f64> {
        self.map()?.reaction_angle(reaction_id)
    }
    /// `layout.reactionControlPoints(reactionId)` (upgrades legacy geometry, as in JS).
    fn reaction_control_points(&mut self, reaction_id: &str) -> Vec<ControlPoint>;
    /// `moveBranchControl(reactionId, nodeId, handle, x, y)`: the point's new place.
    fn move_branch_control(&mut self, reaction_id: &str, node_id: &str, handle: Handle, x: f64, y: f64) -> Option<Point>;
    /// `setMetaboliteLabelAngle(nodeId, angle)`.
    fn set_metabolite_label_angle(&mut self, node_id: &str, angle: f64) -> bool;
    /// Put a node's `labelAngle` back as it was (the JS cancel path writes the
    /// node directly: `delete node.labelAngle` or `node.labelAngle = before`).
    fn restore_metabolite_label_angle(&mut self, node_id: &str, angle: Option<f64>);
    /// `setReactionLabelOffset(reactionId, offset)`.
    fn set_reaction_label_offset(&mut self, reaction_id: &str, offset: Point) -> bool;
    /// Put a reaction's `labelOffset` back as it was (the JS cancel path
    /// writes the reaction directly: `delete rxn.labelOffset` or
    /// `rxn.labelOffset = before`).
    fn restore_reaction_label_offset(&mut self, reaction_id: &str, offset: Option<Point>);
    /// `moveMapText(id, x, y)`: where the text ended up.
    fn move_map_text(&mut self, id: &str, x: f64, y: f64) -> Option<Point>;
    /// `moveCompartmentLabel(cellId, compartmentId, x, y)`: the label's position
    /// as fractions of its compartment.
    fn move_compartment_label(&mut self, cell_id: Option<&str>, compartment_id: &str, x: f64, y: f64) -> Option<Point>;
    /// `editGeometryRect(cellId, compartmentId, rect)`: the rectangle it got.
    fn edit_geometry_rect(&mut self, cell_id: Option<&str>, compartment_id: &str, rect: Rect) -> Option<Rect>;
    /// `editCompartmentShape(cellId, compartmentId, { [handle]: amount })`
    /// while a shape handle is dragged (no history): whether it changed.
    fn edit_compartment_shape(&mut self, cell_id: Option<&str>, compartment_id: &str, handle: &str, amount: f64) -> bool;
    /// `previewCanvasSize(width, height)`: `(width, height)` it would settle on.
    fn preview_canvas_size(&self, width: f64, height: f64) -> Option<(f64, f64)>;
    /// `setMapCanvasSize(width, height, { shiftX, shiftY })`: the size it got.
    fn set_map_canvas_size(&mut self, width: f64, height: f64, shift_x: f64, shift_y: f64) -> Option<(f64, f64)>;

    // Multi-selection

    /// `toggleMultiSelection(kind, id)`.
    fn toggle_multi_selection(&mut self, kind: SelectKind, id: &str);
    /// `selectInRect({x1, y1, x2, y2}, { replace })`.
    fn select_in_rect(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, replace: bool);
    /// `moveMultiSelection(dx, dy)`: what moved.
    fn move_multi_selection(&mut self, dx: f64, dy: f64) -> Option<Moved>;
    /// `clearMultiSelection()`.
    fn clear_multi_selection(&mut self) -> bool;
    /// `removeMultiSelection()`: whether anything went.
    fn remove_multi_selection(&mut self) -> bool;

    // Selection

    /// `select(rid)`.
    fn select(&mut self, reaction_id: Option<&str>);
    /// `selectEntity(type, id, details)`.
    fn select_entity(&mut self, kind: &str, id: &str, cell_id: Option<&str>);
    /// `selectGeometry(cellId, compartmentId)`.
    fn select_geometry(&mut self, cell_id: Option<&str>, compartment_id: &str);
    /// `selectCellGeometry(cellId)`.
    fn select_cell_geometry(&mut self, cell_id: &str);
    /// `clearGeometrySelection()`.
    fn clear_geometry_selection(&mut self);
    /// `toggleCrossFeedingLink({ from, base, to }, at)`.
    fn toggle_cross_feeding_link(&mut self, from: &str, base: &str, to: &str, at: Option<Point>);
    /// `clearCrossFeedingPick()`.
    fn clear_cross_feeding_pick(&mut self);
    /// `setView(patch)`, for the flags the tools switch.
    fn set_view(&mut self, patch: ViewPatch);

    // Cells and compartments

    /// The species the Cell tool draws: `cellPlacementSpeciesId()` of app.js,
    /// which reads the sidebar's choice, `store.cellPlacementSpeciesId`, or
    /// the first of `availableMapCells()`.
    fn cell_placement_species_id(&self) -> Option<String>;
    /// `addMapCell(speciesId, rect)`: the id of the cell drawn.
    fn add_map_cell(&mut self, species_id: &str, rect: Rect) -> Option<String>;
    /// `currentCompartmentPlacement()`.
    fn current_compartment_placement(&mut self) -> Option<CompartmentPlacement>;
    /// `compartmentParentRect(speciesId, compartmentId)`.
    fn compartment_parent_rect(&self, species_id: &str, compartment_id: &str) -> Option<Rect>;
    /// `addMapCompartment(speciesId, compartmentId, rect)`: whether it was drawn.
    fn add_map_compartment(&mut self, species_id: &str, compartment_id: &str, rect: Rect) -> bool;

    // Removing

    /// `removeMapReaction(reactionId)`.
    fn remove_map_reaction(&mut self, reaction_id: &str) -> bool;
    /// `canRemoveMapNode(nodeId)`.
    fn can_remove_map_node(&self, node_id: &str) -> bool;
    /// `removeMapNode(nodeId)`.
    fn remove_map_node(&mut self, node_id: &str) -> bool;
    /// `removeMapCompartment(cellId, compartmentId)`: the compartment, what is
    /// nested and drawn in it (the whole cell for its outermost one), in one
    /// undo step.
    fn remove_map_compartment(&mut self, cell_id: &str, compartment_id: &str) -> bool;

    // Text

    /// `addMapText(text, x, y, style)`.
    fn add_map_text(&mut self, text: &str, x: f64, y: f64, style: &TextStyle) -> bool;
    /// `updateMapText(id, text, style)`.
    fn update_map_text(&mut self, id: &str, text: &str, style: Option<&TextStyle>) -> bool;
    /// `removeMapText(id)`.
    fn remove_map_text(&mut self, id: &str) -> bool;
    /// `setTextStyle({ fontSize, fontFamily })`.
    fn set_text_style(&mut self, style: &TextStyle);
}
