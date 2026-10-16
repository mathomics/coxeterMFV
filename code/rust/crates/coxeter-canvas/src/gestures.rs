//! The pointer gestures of `src/ui/app.js` and the stage's pointer plumbing:
//! `pointerdown` / `pointermove` / `pointerup` / `pointercancel` choose a
//! gesture by tool and target, and every gesture is a begin / update / finish
//! triple acting on the store through [`MapHost`].
//!
//! Ported: pan (Pan tool, middle button, and Space+drag), reaction pick,
//! node drag, metabolite label drag, text drag, reaction drag, rotate,
//! control point, cell draw, compartment draw, canvas resize, compartment
//! label drag, geometry drag (handles, the shared medium's and the
//! compartments' membranes), selection rectangle, group drag, multi-select
//! toggle, pinch, the click logic of `pointerup`, `dblclick`, the canvas keys
//! of the window `keydown` handler, `activateTool`, and the in-place text
//! editor's state (`openTextEditor`, `commitTextEditor`, `cancelTextEditor`).
//!
//! Two gesture variables JavaScript keeps outside the map are here too: the
//! pointer the stage has captured (`setPointerCapture`, which retargets every
//! later event of that pointer to the stage), and the stage classes
//! (`is-dragging-node`, `is-drawing-tool`, …) the stylesheets read.
//!
//! What the panels do (context menus, the status line, the inspector's
//! focus, the tool grid) is returned as [`CanvasEvent`]s.

use coxeter_core::js;
use coxeter_core::mapdata::{Point, Rect};
use coxeter_core::metabolite_label::metabolite_label_angle;
use coxeter_map::reaction_edit::Handle;
use indexmap::IndexMap;

use crate::camera::{self, Camera, PinchGesture, StageRect};
use crate::hit::{self, HitFilter};
use crate::host::{MapEdit, MapHost, SelectKind, TextStyle, ViewPatch};
use crate::scene::Scene;
use crate::targets::{self, GeometryHandle, Target};

// Input

/// The tools of the tool grid (`ui.tool`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Tool {
    #[default]
    Select,
    Pan,
    Edit,
    Reaction,
    Metabolite,
    Text,
    Compartment,
    Cell,
}

impl Tool {
    pub const ALL: [Tool; 8] =
        [Tool::Select, Tool::Pan, Tool::Edit, Tool::Reaction, Tool::Metabolite, Tool::Text, Tool::Compartment, Tool::Cell];

    /// The `data-tool` value.
    pub fn as_str(self) -> &'static str {
        match self {
            Tool::Select => "select",
            Tool::Pan => "pan",
            Tool::Edit => "edit",
            Tool::Reaction => "reaction",
            Tool::Metabolite => "metabolite",
            Tool::Text => "text",
            Tool::Compartment => "compartment",
            Tool::Cell => "cell",
        }
    }

    pub fn parse(s: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|t| t.as_str() == s)
    }

    /// The toolbar's name for it (`#active-tool-label`).
    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Pan => "Pan",
            Tool::Edit => "Edit",
            Tool::Reaction => "Add Reaction",
            Tool::Metabolite => "Metabolite",
            Tool::Text => "Text",
            Tool::Cell => "Cell",
            Tool::Compartment => "Compartment",
        }
    }
}

/// `event.pointerType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PointerType {
    #[default]
    Mouse,
    Touch,
    Pen,
}

pub type PointerId = u64;

/// The fields of a `PointerEvent` the stage reads.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PointerInput {
    pub pointer_id: PointerId,
    pub pointer_type: PointerType,
    /// `event.button`: 0 primary, 1 middle, 2 secondary (`-1` on moves).
    pub button: i16,
    /// Client coordinates (egui points).
    pub client: Point,
    pub shift: bool,
    /// `event.target`. `None`: found by hit-testing the scene at `client`
    /// (or the stage, when the stage has captured this pointer).
    pub target: Option<Target>,
    /// The press or release is on the open text editor (`.map-text-editor`).
    pub on_text_editor: bool,
}

impl PointerInput {
    /// A mouse event with the primary button.
    pub fn mouse(client: Point) -> PointerInput {
        PointerInput { pointer_id: 1, pointer_type: PointerType::Mouse, button: 0, client, ..Default::default() }
    }

    pub fn with_button(mut self, button: i16) -> PointerInput {
        self.button = button;
        self
    }

    pub fn with_shift(mut self, shift: bool) -> PointerInput {
        self.shift = shift;
        self
    }

    pub fn touch(id: PointerId, client: Point) -> PointerInput {
        PointerInput { pointer_id: id, pointer_type: PointerType::Touch, button: 0, client, ..Default::default() }
    }
}

/// The keys the stage answers (`KeyboardEvent.key`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanvasKey {
    Escape,
    Delete,
    Backspace,
    /// `0`: fit the map to the stage.
    Zero,
    /// `+` or `=`.
    Plus,
    Minus,
    Z,
    Y,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
}

/// Modifier keys held with a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeyMods {
    /// Ctrl, or Cmd on a Mac.
    pub command: bool,
    pub shift: bool,
}

// Output

/// The four map menus (`boundaryMenu`, `metaboliteMenu`, `compartmentMenu`,
/// `metaboliteAddMenu`); the panels own them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MapMenu {
    Boundary,
    Metabolite,
    Compartment,
    MetaboliteAdd,
}

/// What a menu is opened for.
#[derive(Debug, Clone, PartialEq)]
pub enum MenuRequest {
    /// `boundaryMenu.open(boundaryId, event)`: the reactions that can be added
    /// to a membrane.
    Boundary { boundary_id: String },
    /// `metaboliteMenu.open(nodeId, event)`: the reactions of a metabolite.
    Metabolite { node_id: String },
    /// `compartmentMenu.open(cellId, compartmentId, event)`.
    Compartment { cell_id: String, compartment_id: String },
    /// `metaboliteAddMenu.open(cellId, compartmentId, event)`.
    MetaboliteAdd { cell_id: String, compartment_id: String },
}

impl MenuRequest {
    pub fn menu(&self) -> MapMenu {
        match self {
            MenuRequest::Boundary { .. } => MapMenu::Boundary,
            MenuRequest::Metabolite { .. } => MapMenu::Metabolite,
            MenuRequest::Compartment { .. } => MapMenu::Compartment,
            MenuRequest::MetaboliteAdd { .. } => MapMenu::MetaboliteAdd,
        }
    }
}

/// What the stage asks of the rest of the application.
#[derive(Debug, Clone, PartialEq)]
pub enum CanvasEvent {
    /// Open a menu at a client point (the press that asked for it).
    OpenMenu { menu: MenuRequest, at: Point },
    CloseMenu(MapMenu),
    /// A message for the status line (`store.status = {state: 'ok', message}`).
    StatusHint(String),
    /// A double click on a reaction: put the keyboard in the inspector's
    /// lower-bound field.
    FocusLowerBound,
    /// The tool changed without the tool grid (Escape, a drawing finished).
    ToolChanged(Tool),
    /// A button of the empty-map card (`map-full`, `map-core-overview`).
    Action(&'static str),
    /// The in-place text editor opened (for a text, or a new one).
    TextEditorOpened { id: Option<String> },
    /// The text editor closed (written or dropped).
    TextEditorClosed,
}

// The stage

/// What a gesture works on besides the store: the camera, the stage's box in
/// client coordinates, and the map as drawn (for what is under a point).
pub struct Stage<'a> {
    pub camera: &'a mut Camera,
    pub rect: StageRect,
    pub scene: Option<&'a Scene>,
}

impl Stage<'_> {
    /// `screenToMap(clientX, clientY)`.
    pub fn screen_to_map(&self, client: Point) -> Point {
        camera::screen_to_map(self.camera, self.rect, client)
    }

    /// Whether a client point is on the stage.
    pub fn contains(&self, client: Point) -> bool {
        let r = self.rect;
        client.x >= r.left && client.y >= r.top && client.x <= r.left + r.width && client.y <= r.top + r.height
    }

    /// `event.target` for an uncaptured pointer: the topmost element of the
    /// map at a client point, the map's own `<svg>` on the canvas, the stage
    /// elsewhere.
    pub fn target_at(&self, client: Point, filter: HitFilter) -> Target {
        let Some(scene) = self.scene else { return Target::stage() };
        if filter.pan_tool {
            return Target::stage();
        }
        let at = self.screen_to_map(client);
        match hit::hit_test(scene, at, filter) {
            Some(i) => Target::of(scene, i),
            None if on_canvas(scene, at) => Target::of(scene, 0),
            None => Target::stage(),
        }
    }

    /// `document.elementsFromPoint(clientX, clientY)` inside the map.
    pub fn elements_from_point(&self, client: Point, filter: HitFilter) -> Vec<Target> {
        let Some(scene) = self.scene else { return Vec::new() };
        let at = self.screen_to_map(client);
        hit::hits_at(scene, at, filter).into_iter().map(|i| Target::of(scene, i)).collect()
    }
}

fn on_canvas(scene: &Scene, at: Point) -> bool {
    let (w, h) = scene.canvas_size();
    at.x >= 0.0 && at.y >= 0.0 && at.x <= w && at.y <= h
}

// Gestures

/// `panGesture`.
#[derive(Debug, Clone, PartialEq)]
pub struct PanGesture {
    pub x: f64,
    pub y: f64,
    pub cx: f64,
    pub cy: f64,
    pub moved: bool,
    /// What was under the pointer when it went down: capturing the pointer
    /// retargets every later event to the stage.
    pub target: Target,
}

/// `reactionPickGesture`: a press on a metabolite with Add Reaction.
#[derive(Debug, Clone, PartialEq)]
pub struct ReactionPick {
    pub node_id: String,
    pub pointer_id: PointerId,
    pub x: f64,
    pub y: f64,
    pub moved: bool,
}

/// `nodeDragGesture`.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeDrag {
    pub node_id: String,
    pub pointer_id: PointerId,
    /// Where on the circle the pointer grabbed it, so the node does not snap
    /// its centre to the cursor on the first move.
    pub grab_x: f64,
    pub grab_y: f64,
    pub start_x: f64,
    pub start_y: f64,
    pub moved: bool,
    pub history: Option<MapEdit>,
    pub merged_into: Option<String>,
}

/// `metaboliteLabelDragGesture`.
#[derive(Debug, Clone, PartialEq)]
pub struct LabelDrag {
    pub node_id: String,
    pub pointer_id: PointerId,
    pub start_angle: f64,
    pub label_angle_before: Option<f64>,
    pub pointer_angle: f64,
    pub moved: bool,
    pub history: Option<MapEdit>,
}

/// `textDragGesture`.
#[derive(Debug, Clone, PartialEq)]
pub struct TextDrag {
    pub text_id: String,
    pub grab_x: f64,
    pub grab_y: f64,
    pub start_x: f64,
    pub start_y: f64,
    pub moved: bool,
    pub history: Option<MapEdit>,
}

/// `reactionDragGesture`: a drag if the pointer travels, a selection if not.
#[derive(Debug, Clone, PartialEq)]
pub struct ReactionDrag {
    pub reaction_id: String,
    /// False for a press that may only select.
    pub movable: bool,
    pub client_x: f64,
    pub client_y: f64,
    pub dragging: bool,
    pub grab_x: f64,
    pub grab_y: f64,
    pub start_x: f64,
    pub start_y: f64,
    pub moved: bool,
    pub history: Option<MapEdit>,
}

/// `rotateDragGesture`.
#[derive(Debug, Clone, PartialEq)]
pub struct RotateDrag {
    pub reaction_id: String,
    pub start_angle: f64,
    pub moved: bool,
    pub history: Option<MapEdit>,
}

/// `controlDragGesture`.
#[derive(Debug, Clone, PartialEq)]
pub struct ControlDrag {
    pub reaction_id: String,
    pub node_id: String,
    pub handle: Handle,
    pub start_x: f64,
    pub start_y: f64,
    pub moved: bool,
    pub history: Option<MapEdit>,
}

/// `cellDrawGesture` and `compartmentDrawGesture`.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawGesture {
    pub species_id: String,
    /// The compartment drawn (compartment tool only).
    pub compartment_id: Option<String>,
    pub parent_id: Option<String>,
    pub parent_rect: Option<Rect>,
    pub pointer_id: PointerId,
    pub start_map: Point,
    pub current_map: Point,
    pub start_client: Point,
    pub current_client: Point,
    pub moved: bool,
}

/// `canvasResizeGesture`.
#[derive(Debug, Clone, PartialEq)]
pub struct CanvasResize {
    pub edge: String,
    pub pointer_id: PointerId,
    pub start_client: Point,
    pub start_width: f64,
    pub start_height: f64,
    /// How much of the drawing's shift is already applied. Each move
    /// recomputes the target from the start of the gesture, so only the
    /// difference may be applied or the drawing would travel once per event.
    pub shifted: Point,
    pub moved: bool,
    pub history: Option<MapEdit>,
}

/// `compartmentLabelDragGesture`.
#[derive(Debug, Clone, PartialEq)]
pub struct CompartmentLabelDrag {
    pub cell_id: Option<String>,
    pub compartment_id: String,
    pub pointer_id: PointerId,
    pub start_map: Point,
    pub start_client: Point,
    pub anchor: Point,
    pub moved: bool,
    pub history: Option<MapEdit>,
}

/// `geometryDragGesture`.
#[derive(Debug, Clone, PartialEq)]
pub struct GeometryDrag {
    pub info: GeometryHandle,
    pub pointer_id: PointerId,
    pub start_point: Point,
    pub start_client: Point,
    pub start_rect: Rect,
    /// A shape diamond taken hold of: its spacing (the corner handle's side
    /// at the zoom the drag started at) and where the pointer was on it.
    pub shape_grab: Option<ShapeGrab>,
    pub moved: bool,
    pub history: Option<MapEdit>,
}

/// `shapeGrab` of `geometryDragGesture`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapeGrab {
    pub side: f64,
    pub offset: Point,
}

/// `selectionGesture`.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectionGesture {
    pub pointer_id: PointerId,
    pub start_map: Point,
    pub current_map: Point,
    pub start_client: Point,
    pub current_client: Point,
    pub moved: bool,
}

/// `groupDragGesture`.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupDrag {
    /// The object pressed (none for the selection's border).
    pub hit: Option<(SelectKind, String)>,
    pub pointer_id: PointerId,
    /// Where the press was, in map coordinates.
    pub start_map: Point,
    /// How far the selection has really gone. A wall can hold it back, so
    /// each step asks for what is still owed to the pointer, and the group
    /// catches up once the pointer comes back past where it stopped.
    pub applied: Point,
    pub moved: bool,
    pub toggle_on_click: bool,
    pub history: Option<MapEdit>,
}

/// `textEditor`: the in-place editor of a map text.
#[derive(Debug, Clone, PartialEq)]
pub struct TextEditorState {
    /// The text edited; `None` for a new one.
    pub id: Option<String>,
    pub x: f64,
    pub y: f64,
    pub value: String,
    pub font_size: f64,
    pub font_family: String,
    /// Drawn at least once: the first showing takes the keyboard.
    pub shown: bool,
}

/// A cross-feeding line pressed: `data-crossfeed-from`, `data-crossfeed`,
/// `data-crossfeed-to`.
#[derive(Debug, Clone, PartialEq)]
struct CrossFeedLine {
    from: Option<String>,
    base: Option<String>,
    to: Option<String>,
}

impl CrossFeedLine {
    fn of(e: &coxeter_render::Element) -> CrossFeedLine {
        CrossFeedLine {
            from: targets::data(e, "data-crossfeed-from"),
            base: targets::data(e, "data-crossfeed"),
            to: targets::data(e, "data-crossfeed-to"),
        }
    }
}

/// Angles a turned backbone settles on when it is brought close to one.
pub const ROTATE_SNAP_STEP: f64 = std::f64::consts::PI / 4.0;
/// How close (4°) a turned backbone must come to one of those angles to settle on it.
pub const ROTATE_SNAP_WITHIN: f64 = (4.0 * std::f64::consts::PI) / 180.0;
/// Smallest rectangle that counts as a deliberate drawing, in map units.
pub const MIN_DRAWN_SIZE: f64 = 24.0;
/// The edges a canvas grip can stand for (`data-canvas-resize`).
pub const CANVAS_EDGES: [&str; 5] = ["right", "bottom", "left", "top", "corner"];
/// What a nudge moves by, in map units (ten with Shift).
pub const NUDGE_STEP: f64 = 1.0;

/// How long and how far apart two clicks make a double click (egui's rule).
const DOUBLE_CLICK_MS: f64 = 300.0;
const DOUBLE_CLICK_PX: f64 = 6.0;

/// The marks a repaint puts back on the drawing (`markDraggedNode`,
/// `markDraggedMetaboliteLabel`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Marks {
    /// `.emc-node.is-dragging`.
    pub dragging_node: Option<String>,
    /// `.emc-node.is-merge-target`: where the dragged node would join.
    pub merge_target: Option<String>,
    /// `.emc-label-draggable.is-dragging`.
    pub dragging_label: Option<String>,
}

/// Every gesture variable of `app.js`, and the stage's own state.
#[derive(Debug, Clone, Default)]
pub struct Gestures {
    /// `ui.tool`.
    pub tool: Tool,
    /// The stage's `is-drawing-tool` class (`activateTool`): the pointer
    /// halos step aside for the canvas underneath.
    pub drawing_tool: bool,
    /// Space is held: a primary drag pans.
    pub space_held: bool,
    pub pan: Option<PanGesture>,
    pub node_drag: Option<NodeDrag>,
    pub metabolite_label_drag: Option<LabelDrag>,
    pub compartment_label_drag: Option<CompartmentLabelDrag>,
    pub reaction_pick: Option<ReactionPick>,
    pub reaction_drag: Option<ReactionDrag>,
    pub rotate_drag: Option<RotateDrag>,
    pub control_drag: Option<ControlDrag>,
    pub geometry_drag: Option<GeometryDrag>,
    pub canvas_resize: Option<CanvasResize>,
    pub cell_draw: Option<DrawGesture>,
    pub compartment_draw: Option<DrawGesture>,
    pub selection: Option<SelectionGesture>,
    pub group_drag: Option<GroupDrag>,
    pub text_drag: Option<TextDrag>,
    pub text_editor: Option<TextEditorState>,
    pub multi_toggle_pointer: Option<PointerId>,
    cross_feed_press: Option<CrossFeedLine>,
    pub pinch: Option<PinchGesture>,
    suppress_touch_until_clear: bool,
    touch_pointers: IndexMap<PointerId, Point>,
    /// The pointer the stage has captured (`setPointerCapture`).
    pub captured: Option<PointerId>,
    /// A pointer whose gesture Escape cancelled: its release does nothing.
    swallow: Option<PointerId>,
    /// The last click, for the double click: time (ms) and client point.
    last_click: Option<(f64, Point)>,
    /// Where the primary button went down: a release far from it is no click.
    press: Option<(PointerId, Point)>,
    /// Requests for the rest of the application.
    events: Vec<CanvasEvent>,
    /// `schedulePaint()` was called: the drawing changed.
    paint: bool,
    /// The camera moved (`applyCamera()`).
    camera_moved: bool,
}

/// `{ label, before }` captured with the map as it was: the drag mutates in
/// place, the snapshot stays detached (JS clones it with `structuredClone`;
/// a `MapEdit` is already a copy).
fn begin(host: &mut dyn MapHost, label: &str) -> Option<MapEdit> {
    host.begin_map_edit(label)
}

fn moved_by(a: Point, b: Point) -> f64 {
    (a.x - b.x).abs() + (a.y - b.y).abs()
}

/// `pointInRect(point, rect, pad)`.
pub fn point_in_rect(point: Point, rect: Option<&Rect>, pad: f64) -> bool {
    let Some(r) = rect else { return false };
    point.x >= r.x + pad && point.y >= r.y + pad && point.x <= r.x + r.w - pad && point.y <= r.y + r.h - pad
}

/// `clampPointToRect(point, rect, pad = 12)`.
pub fn clamp_point_to_rect(point: Point, rect: &Rect, pad: f64) -> Point {
    Point::new(
        js::min(js::max(point.x, rect.x + pad), rect.x + rect.w - pad),
        js::min(js::max(point.y, rect.y + pad), rect.y + rect.h - pad),
    )
}

/// The rectangle two corners span (`{x: min, y: min, w: |dx|, h: |dy|}`).
fn span(a: Point, b: Point) -> Rect {
    Rect::new(js::min(a.x, b.x), js::min(a.y, b.y), (b.x - a.x).abs(), (b.y - a.y).abs())
}

impl Gestures {
    pub fn new() -> Gestures {
        Gestures::default()
    }

    /// The requests made since the last call.
    pub fn take_events(&mut self) -> Vec<CanvasEvent> {
        std::mem::take(&mut self.events)
    }

    /// Whether the drawing changed since the last call (`schedulePaint`).
    pub fn take_paint(&mut self) -> bool {
        std::mem::take(&mut self.paint)
    }

    /// Whether the camera moved since the last call.
    pub fn take_camera_moved(&mut self) -> bool {
        std::mem::take(&mut self.camera_moved)
    }

    /// Ask for a repaint (the drawing changed outside a gesture).
    pub fn schedule_paint(&mut self) {
        self.paint = true;
    }

    /// The stage's pointer filter: what the stylesheets let take a press.
    pub fn hit_filter(&self) -> HitFilter {
        HitFilter { pan_tool: self.tool == Tool::Pan, drawing_tool: self.drawing_tool }
    }

    /// Whether a pointer gesture is live.
    pub fn active(&self) -> bool {
        self.pan.is_some()
            || self.node_drag.is_some()
            || self.metabolite_label_drag.is_some()
            || self.compartment_label_drag.is_some()
            || self.reaction_pick.is_some()
            || self.reaction_drag.is_some()
            || self.rotate_drag.is_some()
            || self.control_drag.is_some()
            || self.geometry_drag.is_some()
            || self.canvas_resize.is_some()
            || self.cell_draw.is_some()
            || self.compartment_draw.is_some()
            || self.selection.is_some()
            || self.group_drag.is_some()
            || self.text_drag.is_some()
            || self.pinch.is_some()
            || self.multi_toggle_pointer.is_some()
    }

    /// `mapDragInProgress()`: is a drag changing the map right now? Undo
    /// swaps the whole map out from under it: a drag that went on would edit
    /// the restored map and then commit against the snapshot it started
    /// from, so undoing the drag would bring back the edit just undone and
    /// lose the one before.
    pub fn map_drag_in_progress(&self) -> bool {
        self.node_drag.is_some()
            || self.compartment_label_drag.is_some()
            || self.reaction_drag.is_some()
            || self.rotate_drag.is_some()
            || self.control_drag.is_some()
            || self.geometry_drag.is_some()
            || self.canvas_resize.is_some()
            || self.cell_draw.is_some()
            || self.compartment_draw.is_some()
            || self.group_drag.is_some()
            || self.text_drag.is_some()
    }

    /// The stage's classes, as the stylesheets see them.
    pub fn stage_classes(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.pan.is_some() {
            out.push("is-panning");
        }
        if self.node_drag.is_some() {
            out.push("is-dragging-node");
        }
        if self.metabolite_label_drag.is_some() {
            out.push("is-dragging-metabolite-label");
        }
        if self.compartment_label_drag.is_some() {
            out.push("is-dragging-compartment-label");
        }
        if self.text_drag.is_some() {
            out.push("is-dragging-text");
        }
        if self.group_drag.is_some() {
            out.push("is-dragging-group");
        }
        if self.cell_draw.is_some() {
            out.push("is-drawing-cell");
        }
        if self.compartment_draw.is_some() {
            out.push("is-drawing-compartment");
        }
        if self.canvas_resize.is_some() {
            out.push("is-resizing-canvas");
        }
        if self.drawing_tool {
            out.push("is-drawing-tool");
        }
        out
    }

    /// What `markDraggedNode` and `markDraggedMetaboliteLabel` mark.
    pub fn marks(&self, host: &dyn MapHost) -> Marks {
        let mut marks = Marks::default();
        if let Some(g) = &self.node_drag {
            marks.dragging_node = Some(g.node_id.clone());
            // Match the merge on release, using the clamped position and the
            // nearest copy.
            if g.moved {
                marks.merge_target =
                    host.map().and_then(|m| m.overlapping_metabolite_target(&g.node_id)).map(|n| n.id.clone());
            }
        }
        marks.dragging_label = self.metabolite_label_drag.as_ref().map(|g| g.node_id.clone());
        marks
    }

    fn emit(&mut self, e: CanvasEvent) {
        self.events.push(e);
    }

    /// Queue a request made by something other than a gesture (the
    /// widget's overlays).
    pub(crate) fn push_event(&mut self, e: CanvasEvent) {
        self.events.push(e);
    }

    fn open_menu(&mut self, menu: MenuRequest, at: Point) {
        self.emit(CanvasEvent::OpenMenu { menu, at });
    }

    fn close_menu(&mut self, menu: MapMenu) {
        self.emit(CanvasEvent::CloseMenu(menu));
    }

    /// `closeMapMenus()`: the boundary, metabolite and compartment menus.
    pub fn close_map_menus(&mut self) {
        self.close_menu(MapMenu::Boundary);
        self.close_menu(MapMenu::Metabolite);
        self.close_menu(MapMenu::Compartment);
    }

    /// `movesThings()`: Select and Edit move what is already on the map.
    pub fn moves_things(&self) -> bool {
        matches!(self.tool, Tool::Select | Tool::Edit)
    }

    fn capture(&mut self, ev: &PointerInput) {
        self.captured = Some(ev.pointer_id);
    }

    fn release(&mut self, ev: &PointerInput) {
        if self.captured == Some(ev.pointer_id) {
            self.captured = None;
        }
    }

    /// `event.target` of an event.
    fn target(&self, stage: &Stage, ev: &PointerInput) -> Target {
        if let Some(t) = &ev.target {
            return t.clone();
        }
        if self.captured == Some(ev.pointer_id) {
            return Target::stage();
        }
        stage.target_at(ev.client, self.hit_filter())
    }

    // Add reaction pick

    fn begin_reaction_pick(&mut self, node_id: String, ev: &PointerInput) {
        self.reaction_pick =
            Some(ReactionPick { node_id, pointer_id: ev.pointer_id, x: ev.client.x, y: ev.client.y, moved: false });
        self.capture(ev);
    }

    fn update_reaction_pick(&mut self, ev: &PointerInput) {
        let Some(g) = &mut self.reaction_pick else { return };
        if ev.pointer_id != g.pointer_id {
            return;
        }
        if (ev.client.x - g.x).abs() + (ev.client.y - g.y).abs() > 4.0 {
            g.moved = true;
        }
    }

    fn finish_reaction_pick(&mut self, ev: &PointerInput, host: &mut dyn MapHost) -> Option<ReactionPick> {
        let g = self.reaction_pick.take()?;
        self.release(ev);
        if g.moved {
            return Some(g);
        }
        self.close_menu(MapMenu::Boundary);
        self.close_menu(MapMenu::Compartment);
        self.open_menu(MenuRequest::Metabolite { node_id: g.node_id.clone() }, ev.client);
        host.select_entity("metabolite", &g.node_id, None);
        Some(g)
    }

    // Node drag

    fn begin_node_drag(&mut self, node_id: String, ev: &PointerInput, stage: &Stage, host: &mut dyn MapHost) {
        let Some((nx, ny)) = host.map().and_then(|m| m.node(&node_id)).map(|n| (n.x, n.y)) else { return };
        let at = stage.screen_to_map(ev.client);
        let history = begin(host, "Move metabolite");
        self.node_drag = Some(NodeDrag {
            node_id,
            pointer_id: ev.pointer_id,
            grab_x: nx - at.x,
            grab_y: ny - at.y,
            start_x: nx,
            start_y: ny,
            moved: false,
            history,
            merged_into: None,
        });
        self.capture(ev);
        self.paint = true;
    }

    fn update_node_drag(&mut self, ev: &PointerInput, stage: &Stage, host: &mut dyn MapHost) {
        let Some(g) = &mut self.node_drag else { return };
        let at = stage.screen_to_map(ev.client);
        // Geometry only: no solve, no rebuild, no relayout. Repainting from the
        // updated coordinates is what makes the labels and the curves follow.
        let Some(node) = host.move_node(&g.node_id, at.x + g.grab_x, at.y + g.grab_y) else { return };
        if moved_by(node, Point::new(g.start_x, g.start_y)) > 0.5 {
            g.moved = true;
        }
        self.paint = true;
    }

    fn finish_node_drag(&mut self, ev: &PointerInput, host: &mut dyn MapHost) -> Option<NodeDrag> {
        let mut g = self.node_drag.take()?;
        self.release(ev);
        if g.moved {
            if let Some(merged) = host.merge_overlapping_metabolite(&g.node_id) {
                g.merged_into = Some(merged);
            }
            host.commit_map_edit(g.history.clone());
        }
        // A final paint, in case the pointer went up before the pending frame ran.
        self.paint = true;
        Some(g)
    }

    // Metabolite labels

    fn begin_metabolite_label_drag(&mut self, node_id: String, ev: &PointerInput, stage: &Stage, host: &mut dyn MapHost) {
        let Some((nx, ny, label_angle)) = host.map().and_then(|m| m.node(&node_id)).map(|n| (n.x, n.y, n.label_angle)) else {
            return;
        };
        self.close_map_menus();
        let at = stage.screen_to_map(ev.client);
        let history = begin(host, "Move metabolite label");
        self.metabolite_label_drag = Some(LabelDrag {
            node_id,
            pointer_id: ev.pointer_id,
            start_angle: metabolite_label_angle(label_angle),
            label_angle_before: label_angle,
            pointer_angle: js::atan2(at.y - ny, at.x - nx),
            moved: false,
            history,
        });
        self.capture(ev);
        self.paint = true;
    }

    fn update_metabolite_label_drag(&mut self, ev: &PointerInput, stage: &Stage, host: &mut dyn MapHost) {
        let Some(g) = &mut self.metabolite_label_drag else { return };
        if ev.pointer_id != g.pointer_id {
            return;
        }
        let Some((nx, ny)) = host.map().and_then(|m| m.node(&g.node_id)).map(|n| (n.x, n.y)) else { return };
        let at = stage.screen_to_map(ev.client);
        if js::hypot(at.x - nx, at.y - ny) < 1e-6 {
            return;
        }
        let pointer_angle = js::atan2(at.y - ny, at.x - nx);
        let angle = g.start_angle + pointer_angle - g.pointer_angle;
        let change = js::atan2(js::sin(angle - g.start_angle), js::cos(angle - g.start_angle));
        if change.abs() < 1e-4 && !g.moved {
            return;
        }
        if !host.set_metabolite_label_angle(&g.node_id, angle) {
            return;
        }
        g.moved = true;
        self.paint = true;
    }

    fn finish_metabolite_label_drag(&mut self, pointer_id: PointerId, cancel: bool, host: &mut dyn MapHost) -> Option<LabelDrag> {
        let g = self.metabolite_label_drag.as_ref()?;
        if g.pointer_id != pointer_id {
            return None;
        }
        let g = self.metabolite_label_drag.take()?;
        if self.captured == Some(pointer_id) {
            self.captured = None;
        }
        if cancel {
            host.restore_metabolite_label_angle(&g.node_id, g.label_angle_before);
        } else if g.moved {
            host.commit_map_edit(g.history.clone());
        }
        self.paint = true;
        Some(g)
    }

    // Text drag

    fn begin_text_drag(&mut self, text_id: String, ev: &PointerInput, stage: &Stage, host: &mut dyn MapHost) {
        let Some((tx, ty)) = host.map().and_then(|m| m.text(&text_id)).map(|t| (t.x, t.y)) else { return };
        let at = stage.screen_to_map(ev.client);
        let history = begin(host, "Move text");
        self.text_drag =
            Some(TextDrag { text_id, grab_x: tx - at.x, grab_y: ty - at.y, start_x: tx, start_y: ty, moved: false, history });
        self.capture(ev);
    }

    fn update_text_drag(&mut self, ev: &PointerInput, stage: &Stage, host: &mut dyn MapHost) {
        let Some(g) = &mut self.text_drag else { return };
        let at = stage.screen_to_map(ev.client);
        let Some(item) = host.move_map_text(&g.text_id, at.x + g.grab_x, at.y + g.grab_y) else { return };
        if moved_by(item, Point::new(g.start_x, g.start_y)) > 0.5 {
            g.moved = true;
        }
        self.paint = true;
    }

    fn finish_text_drag(&mut self, ev: &PointerInput, host: &mut dyn MapHost) -> Option<TextDrag> {
        let g = self.text_drag.take()?;
        self.release(ev);
        if g.moved {
            host.commit_map_edit(g.history.clone());
            self.paint = true;
        } else {
            let annotation = host.map().and_then(|m| m.text(&g.text_id)).cloned();
            if let Some(a) = annotation {
                host.select_entity("text", &a.id, None);
                self.open_text_editor(Some(a.id.clone()), a.x, a.y, a.text.clone(), a.font_size, a.font_family.clone(), host);
            }
        }
        Some(g)
    }

    // Reaction drag

    /// A press on a reaction: a drag if the pointer travels, a selection if
    /// it does not, decided when the pointer comes up. (Selecting on the
    /// press would repaint and put the control points under the pointer,
    /// and the release would deselect it again, so it would take two presses.)
    fn begin_reaction_drag(&mut self, reaction_id: String, ev: &PointerInput, movable: bool, stage: &Stage, host: &mut dyn MapHost) {
        let Some((rx, ry, on_membrane)) = host.map().and_then(|m| m.reaction(&reaction_id)).map(|r| (r.x, r.y, r.on_membrane))
        else {
            return;
        };
        let at = stage.screen_to_map(ev.client);
        let history = if movable { begin(host, if on_membrane { "Move transporter" } else { "Move reaction" }) } else { None };
        self.reaction_drag = Some(ReactionDrag {
            reaction_id,
            movable,
            client_x: ev.client.x,
            client_y: ev.client.y,
            dragging: false,
            grab_x: rx - at.x,
            grab_y: ry - at.y,
            start_x: rx,
            start_y: ry,
            moved: false,
            history,
        });
        self.capture(ev);
    }

    fn update_reaction_drag(&mut self, ev: &PointerInput, stage: &Stage, host: &mut dyn MapHost) {
        let Some(g) = &mut self.reaction_drag else { return };
        if !g.movable {
            return;
        }
        // A few pixels of slack, so the tremor of a click or a tap does not
        // nudge the reaction and get taken for a drag.
        if !g.dragging {
            if (ev.client.x - g.client_x).abs() + (ev.client.y - g.client_y).abs() <= 4.0 {
                return;
            }
            g.dragging = true;
        }
        let at = stage.screen_to_map(ev.client);
        let Some(rxn) = host.move_reaction(&g.reaction_id, at.x + g.grab_x, at.y + g.grab_y) else { return };
        if moved_by(rxn, Point::new(g.start_x, g.start_y)) > 0.5 {
            g.moved = true;
        }
        self.paint = true;
    }

    fn finish_reaction_drag(&mut self, ev: &PointerInput, host: &mut dyn MapHost) -> Option<ReactionDrag> {
        let g = self.reaction_drag.take()?;
        self.release(ev);
        if g.moved {
            host.commit_map_edit(g.history.clone());
        }
        self.paint = true;
        Some(g)
    }

    // Rotation

    fn begin_rotate_drag(&mut self, reaction_id: String, ev: &PointerInput, host: &mut dyn MapHost) {
        if host.map().and_then(|m| m.reaction(&reaction_id)).is_none() {
            return;
        }
        // `reactionAngle` is null without an axis, which arithmetic reads as 0.
        let start_angle = host.reaction_angle(&reaction_id).unwrap_or(0.0);
        let history = begin(host, "Rotate reaction");
        self.rotate_drag = Some(RotateDrag { reaction_id, start_angle, moved: false, history });
        self.capture(ev);
    }

    fn update_rotate_drag(&mut self, ev: &PointerInput, stage: &Stage, host: &mut dyn MapHost) {
        let Some(g) = &mut self.rotate_drag else { return };
        let Some((rx, ry)) = host.map().and_then(|m| m.reaction(&g.reaction_id)).map(|r| (r.x, r.y)) else { return };
        let at = stage.screen_to_map(ev.client);
        if js::hypot(at.x - rx, at.y - ry) < 1e-6 {
            return;
        }
        // The lever stands a quarter turn on from the backbone it turns.
        let mut angle = js::atan2(at.y - ry, at.x - rx) - std::f64::consts::PI / 2.0;
        let nearest = js::round(angle / ROTATE_SNAP_STEP) * ROTATE_SNAP_STEP;
        if (angle - nearest).abs() < ROTATE_SNAP_WITHIN {
            angle = nearest;
        }
        if !host.set_reaction_angle(&g.reaction_id, angle) {
            return;
        }
        let turned = host.reaction_angle(&g.reaction_id).unwrap_or(0.0) - g.start_angle;
        if js::atan2(js::sin(turned), js::cos(turned)).abs() > 1e-4 {
            g.moved = true;
        }
        self.paint = true;
    }

    fn finish_rotate_drag(&mut self, ev: &PointerInput, host: &mut dyn MapHost) -> Option<RotateDrag> {
        let g = self.rotate_drag.take()?;
        self.release(ev);
        if g.moved {
            host.commit_map_edit(g.history.clone());
        }
        self.paint = true;
        Some(g)
    }

    // Curve drag

    fn begin_control_drag(&mut self, handle: targets::ControlHandle, ev: &PointerInput, host: &mut dyn MapHost) {
        let Some(which) = Handle::parse(&handle.handle) else { return };
        let current = host
            .reaction_control_points(&handle.reaction_id)
            .into_iter()
            .find(|p| p.node_id == handle.node_id && p.handle == which);
        let Some(current) = current else { return };
        let label = match handle.handle.as_str() {
            "b1" => "Move metabolite control point",
            "b2" => "Move reaction control point",
            _ => "Move reaction backbone control point",
        };
        let history = begin(host, label);
        self.control_drag = Some(ControlDrag {
            reaction_id: handle.reaction_id,
            node_id: handle.node_id,
            handle: which,
            start_x: current.x,
            start_y: current.y,
            moved: false,
            history,
        });
        self.capture(ev);
    }

    fn update_control_drag(&mut self, ev: &PointerInput, stage: &Stage, host: &mut dyn MapHost) {
        let Some(g) = &mut self.control_drag else { return };
        let at = stage.screen_to_map(ev.client);
        let Some(point) = host.move_branch_control(&g.reaction_id, &g.node_id, g.handle, at.x, at.y) else { return };
        if moved_by(point, Point::new(g.start_x, g.start_y)) > 0.5 {
            g.moved = true;
        }
        self.paint = true;
    }

    fn finish_control_drag(&mut self, ev: &PointerInput, host: &mut dyn MapHost) -> Option<ControlDrag> {
        let g = self.control_drag.take()?;
        self.release(ev);
        if g.moved {
            host.commit_map_edit(g.history.clone());
        }
        self.paint = true;
        Some(g)
    }

    // Draw Cell

    fn begin_cell_draw(&mut self, ev: &PointerInput, stage: &Stage, host: &mut dyn MapHost) -> bool {
        let species_id = host.cell_placement_species_id().unwrap_or_default();
        if species_id.is_empty() || host.map().is_none() {
            return false;
        }
        let map_point = stage.screen_to_map(ev.client);
        self.cell_draw = Some(DrawGesture {
            species_id,
            compartment_id: None,
            parent_id: None,
            parent_rect: None,
            pointer_id: ev.pointer_id,
            start_map: map_point,
            current_map: map_point,
            start_client: ev.client,
            current_client: ev.client,
            moved: false,
        });
        self.close_map_menus();
        self.capture(ev);
        self.paint = true;
        true
    }

    fn update_cell_draw(&mut self, ev: &PointerInput, stage: &Stage) {
        let Some(g) = &mut self.cell_draw else { return };
        g.current_map = stage.screen_to_map(ev.client);
        g.current_client = ev.client;
        if moved_by(g.current_client, g.start_client) > 5.0 {
            g.moved = true;
        }
        self.paint = true;
    }

    fn finish_cell_draw(&mut self, ev: &PointerInput, host: &mut dyn MapHost) -> Option<String> {
        let g = self.cell_draw.take()?;
        self.release(ev);
        let rect = if g.moved {
            span(g.start_map, g.current_map)
        } else {
            Rect::new(g.start_map.x - 180.0, g.start_map.y - 120.0, 360.0, 240.0)
        };
        let result = host.add_map_cell(&g.species_id, rect);
        match &result {
            Some(cell_id) => {
                host.select_cell_geometry(cell_id);
                self.set_tool_quietly(Tool::Cell);
            }
            None => self.paint = true,
        }
        result
    }

    // Draw Compartment

    fn begin_compartment_draw(&mut self, ev: &PointerInput, stage: &Stage, host: &mut dyn MapHost) -> bool {
        let Some(spec) = host.current_compartment_placement() else { return false };
        if host.map().is_none() {
            return false;
        }
        let Some(parent_rect) = host.compartment_parent_rect(&spec.species_id, &spec.compartment_id) else { return false };
        let at = stage.screen_to_map(ev.client);
        if !point_in_rect(at, Some(&parent_rect), 10.0) {
            return false;
        }
        let start = clamp_point_to_rect(at, &parent_rect, 12.0);
        self.compartment_draw = Some(DrawGesture {
            species_id: spec.species_id,
            compartment_id: Some(spec.compartment_id),
            parent_id: spec.parent_id,
            parent_rect: Some(parent_rect),
            pointer_id: ev.pointer_id,
            start_map: start,
            current_map: start,
            start_client: ev.client,
            current_client: ev.client,
            moved: false,
        });
        self.close_map_menus();
        self.capture(ev);
        self.paint = true;
        true
    }

    fn update_compartment_draw(&mut self, ev: &PointerInput, stage: &Stage) {
        let Some(g) = &mut self.compartment_draw else { return };
        let Some(parent) = g.parent_rect else { return };
        let raw = stage.screen_to_map(ev.client);
        g.current_map = clamp_point_to_rect(raw, &parent, 12.0);
        g.current_client = Point::new(
            stage.camera.x + g.current_map.x * stage.camera.scale + stage.rect.left,
            stage.camera.y + g.current_map.y * stage.camera.scale + stage.rect.top,
        );
        if moved_by(g.current_client, g.start_client) > 5.0 {
            g.moved = true;
        }
        self.paint = true;
    }

    fn finish_compartment_draw(&mut self, ev: &PointerInput, host: &mut dyn MapHost) -> bool {
        let Some(g) = self.compartment_draw.take() else { return false };
        self.release(ev);
        // A bare click does not drop a default-sized box: a tool meant for
        // drawing would then place compartments by itself. Nothing is
        // created unless a rectangle was actually pulled out, and the status
        // line says so rather than leaving a silent no-op.
        let rect = span(g.start_map, g.current_map);
        if !g.moved || rect.w < MIN_DRAWN_SIZE || rect.h < MIN_DRAWN_SIZE {
            self.emit(CanvasEvent::StatusHint(
                "Drag to draw the compartment: press inside the highlighted parent and pull out a rectangle.".into(),
            ));
            self.paint = true;
            return false;
        }
        let compartment_id = g.compartment_id.clone().unwrap_or_default();
        let result = host.add_map_compartment(&g.species_id, &compartment_id, rect);
        if result {
            host.select_geometry(Some(&g.species_id), &compartment_id);
            self.set_tool_quietly(Tool::Compartment);
        } else {
            self.paint = true;
        }
        result
    }

    // Canvas resize

    fn begin_canvas_resize(&mut self, edge: String, ev: &PointerInput, host: &mut dyn MapHost) {
        let Some((w, h)) = host.map().map(|m| (m.width(), m.height())) else { return };
        if !CANVAS_EDGES.contains(&edge.as_str()) {
            return;
        }
        let history = begin(host, "Resize canvas");
        self.canvas_resize = Some(CanvasResize {
            edge,
            pointer_id: ev.pointer_id,
            start_client: ev.client,
            start_width: w,
            start_height: h,
            shifted: Point::new(0.0, 0.0),
            moved: false,
            history,
        });
        self.capture(ev);
    }

    fn update_canvas_resize(&mut self, ev: &PointerInput, stage: &Stage, host: &mut dyn MapHost) {
        let Some(g) = &mut self.canvas_resize else { return };
        // `camera.scale || 1`.
        let s = if stage.camera.scale == 0.0 || stage.camera.scale.is_nan() { 1.0 } else { stage.camera.scale };
        let dx = (ev.client.x - g.start_client.x) / s;
        let dy = (ev.client.y - g.start_client.y) / s;
        // The edge being dragged is the one that moves; the opposite one stays
        // put. Pulling the left edge leftwards is a negative dx and a wider canvas.
        let horizontal = match g.edge.as_str() {
            "right" | "corner" => dx,
            "left" => -dx,
            _ => 0.0,
        };
        let vertical = match g.edge.as_str() {
            "bottom" | "corner" => dy,
            "top" => -dy,
            _ => 0.0,
        };
        let width = g.start_width + horizontal;
        let height = g.start_height + vertical;
        // Clamping decides the real size, so the shift is derived from what the
        // canvas actually became rather than from what the pointer asked for.
        let Some((tw, th)) = host.preview_canvas_size(width, height) else { return };
        let want_x = if g.edge == "left" { tw - g.start_width } else { 0.0 };
        let want_y = if g.edge == "top" { th - g.start_height } else { 0.0 };
        let Some((rw, rh)) = host.set_map_canvas_size(width, height, want_x - g.shifted.x, want_y - g.shifted.y) else {
            return;
        };
        g.shifted = Point::new(want_x, want_y);
        if (rw - g.start_width).abs() + (rh - g.start_height).abs() > 0.5 {
            g.moved = true;
        }
        self.paint = true;
    }

    fn finish_canvas_resize(&mut self, ev: &PointerInput, host: &mut dyn MapHost) -> Option<CanvasResize> {
        let g = self.canvas_resize.take()?;
        self.release(ev);
        if g.moved {
            host.commit_map_edit(g.history.clone());
        }
        self.paint = true;
        Some(g)
    }

    // Compartment label drag

    fn begin_compartment_label_drag(&mut self, target: &Target, k: usize, ev: &PointerInput, stage: &Stage, host: &mut dyn MapHost) {
        self.close_map_menus();
        let hit = &target.chain()[k];
        let cell_id = target.closest_from(k, &targets::CELL).and_then(|c| targets::data(c, "data-cell"));
        let compartment_id = targets::data(hit, "data-compartment-label").unwrap_or_default();
        // `hit.transform.baseVal.consolidate()?.matrix`: its own transform attribute.
        let matrix = hit.get("transform").and_then(crate::geom::parse_transform);
        let Some(matrix) = matrix else { return };
        if host.map().and_then(|m| m.geometry_target(cell_id.as_deref(), &compartment_id)).is_none() {
            return;
        }
        let history = begin(host, "Move compartment name");
        self.compartment_label_drag = Some(CompartmentLabelDrag {
            cell_id,
            compartment_id,
            pointer_id: ev.pointer_id,
            start_map: stage.screen_to_map(ev.client),
            start_client: ev.client,
            anchor: Point::new(matrix[4], matrix[5]),
            moved: false,
            history,
        });
        self.capture(ev);
    }

    fn update_compartment_label_drag(&mut self, ev: &PointerInput, stage: &Stage, host: &mut dyn MapHost) {
        let Some(g) = &mut self.compartment_label_drag else { return };
        let distance = js::hypot(ev.client.x - g.start_client.x, ev.client.y - g.start_client.y);
        if !g.moved && distance < 4.0 {
            return;
        }
        let point = stage.screen_to_map(ev.client);
        let position = host.move_compartment_label(
            g.cell_id.as_deref(),
            &g.compartment_id,
            g.anchor.x + point.x - g.start_map.x,
            g.anchor.y + point.y - g.start_map.y,
        );
        if position.is_some() {
            g.moved = true;
        }
        self.paint = true;
    }

    fn finish_compartment_label_drag(&mut self, pointer_id: PointerId, host: &mut dyn MapHost) {
        let Some(g) = self.compartment_label_drag.take() else { return };
        if self.captured == Some(pointer_id) {
            self.captured = None;
        }
        if g.moved {
            host.commit_map_edit(g.history.clone());
        } else {
            host.select_entity("compartment", &g.compartment_id, g.cell_id.as_deref());
        }
        self.paint = true;
    }

    // Geometry drag

    fn begin_geometry_drag(&mut self, info: GeometryHandle, ev: &PointerInput, stage: &Stage, host: &mut dyn MapHost) {
        let Some((rect, environment)) = targets::geometry_rect(host, info.cell_id.as_deref(), &info.compartment_id) else {
            return;
        };
        let moving = info.handle == "move";
        let start_point = stage.screen_to_map(ev.client);
        let shaping = coxeter_map::geometry_edit::is_shape_handle(&info.handle) && !environment;
        let shape_grab = if shaping {
            // Spaced as drawn, at the zoom the drag starts at; the offset keeps
            // the diamond where it was taken hold of instead of jumping under
            // the pointer.
            let scale = stage.camera.scale;
            let side = coxeter_render::geometry_handle_side(crate::camera::MIN_HIT_PX / if scale > 0.0 { scale } else { 1.0 });
            let compartment = host.map().and_then(|m| {
                m.cells()
                    .iter()
                    .find(|c| Some(c.id.as_str()) == info.cell_id.as_deref())
                    .and_then(|c| c.compartments.iter().find(|k| k.id == info.compartment_id))
                    .cloned()
            });
            compartment.map(|comp| {
                let shape = coxeter_core::geometry::compartment_shape(&comp);
                let amount = if info.handle == "oval" { shape.oval } else { shape.corner };
                let at = coxeter_map::geometry_edit::shape_handle_point(&rect, &info.handle, amount, side);
                ShapeGrab { side, offset: Point::new(start_point.x - at.x, start_point.y - at.y) }
            })
        } else {
            None
        };
        let label = match (environment, moving) {
            (true, true) => "Move shared medium",
            (true, false) => "Resize shared medium",
            (false, _) if shaping && info.handle == "oval" => "Make compartment oval",
            (false, _) if shaping => "Soften compartment corners",
            (false, true) => "Move compartment",
            (false, false) => "Resize compartment",
        };
        let history = begin(host, label);
        self.geometry_drag = Some(GeometryDrag {
            info,
            pointer_id: ev.pointer_id,
            start_point,
            start_client: ev.client,
            start_rect: rect,
            shape_grab,
            moved: false,
            history,
        });
        self.capture(ev);
    }

    fn update_geometry_drag(&mut self, ev: &PointerInput, stage: &Stage, host: &mut dyn MapHost) {
        let Some(g) = &mut self.geometry_drag else { return };
        // Do not mutate geometry for tiny pointer jitter. Besides feeling more
        // deliberate, this preserves ordinary click semantics on compartments
        // and boundaries when the user only meant to select them.
        let drag_distance = js::hypot(ev.client.x - g.start_client.x, ev.client.y - g.start_client.y);
        if !g.moved && drag_distance < 3.0 {
            return;
        }
        let at = stage.screen_to_map(ev.client);
        if let Some(ShapeGrab { side, offset }) = g.shape_grab {
            let point = Point::new(at.x - offset.x, at.y - offset.y);
            let amount = coxeter_map::geometry_edit::shape_amount_at(&g.start_rect, &g.info.handle, point, side);
            if !host.edit_compartment_shape(g.info.cell_id.as_deref(), &g.info.compartment_id, &g.info.handle, amount) {
                return;
            }
            g.moved = true;
            self.paint = true;
            return;
        }
        if coxeter_map::geometry_edit::is_shape_handle(&g.info.handle) {
            // A shape handle of something that has no shape (the shared medium).
            return;
        }
        let Some(requested) = coxeter_map::geometry_edit::rect_from_handle(&g.start_rect, &g.info.handle, g.start_point, at) else {
            return;
        };
        if host.edit_geometry_rect(g.info.cell_id.as_deref(), &g.info.compartment_id, requested).is_none() {
            return;
        }
        g.moved = true;
        self.paint = true;
    }

    fn finish_geometry_drag(&mut self, ev: &PointerInput, host: &mut dyn MapHost) -> Option<GeometryDrag> {
        let g = self.geometry_drag.take()?;
        self.release(ev);
        if g.moved {
            host.commit_map_edit(g.history.clone());
        }
        self.paint = true;
        Some(g)
    }

    // Multi-selection

    fn begin_selection_rect(&mut self, ev: &PointerInput, stage: &Stage) {
        self.close_map_menus();
        let at = stage.screen_to_map(ev.client);
        self.selection = Some(SelectionGesture {
            pointer_id: ev.pointer_id,
            start_map: at,
            current_map: at,
            start_client: ev.client,
            current_client: ev.client,
            moved: false,
        });
        self.capture(ev);
        self.paint = true;
    }

    fn update_selection_rect(&mut self, ev: &PointerInput, stage: &Stage) {
        let Some(g) = &mut self.selection else { return };
        g.current_map = stage.screen_to_map(ev.client);
        g.current_client = ev.client;
        if moved_by(g.current_client, g.start_client) > 4.0 {
            g.moved = true;
        }
        self.paint = true;
    }

    fn finish_selection_rect(&mut self, ev: &PointerInput, host: &mut dyn MapHost) -> Option<SelectionGesture> {
        let g = self.selection.take()?;
        self.release(ev);
        if g.moved {
            host.select_in_rect(g.start_map.x, g.start_map.y, g.current_map.x, g.current_map.y, true);
        } else {
            host.clear_multi_selection();
        }
        self.paint = true;
        Some(g)
    }

    fn begin_group_drag(&mut self, hit: Option<(SelectKind, String)>, ev: &PointerInput, toggle_on_click: bool, stage: &Stage, host: &mut dyn MapHost) {
        self.close_map_menus();
        let at = stage.screen_to_map(ev.client);
        let history = begin(host, "Move selection");
        self.group_drag = Some(GroupDrag {
            hit,
            pointer_id: ev.pointer_id,
            start_map: at,
            applied: Point::default(),
            moved: false,
            toggle_on_click,
            history,
        });
        self.capture(ev);
    }

    fn update_group_drag(&mut self, ev: &PointerInput, stage: &Stage, host: &mut dyn MapHost) {
        let Some(g) = &mut self.group_drag else { return };
        let at = stage.screen_to_map(ev.client);
        let dx = (at.x - g.start_map.x) - g.applied.x;
        let dy = (at.y - g.start_map.y) - g.applied.y;
        if let Some(m) = host.move_multi_selection(dx, dy) {
            g.applied.x += m.dx;
            g.applied.y += m.dy;
            if m.any() {
                g.moved = true;
            }
        }
        self.paint = true;
    }

    /// A press on the empty inside of a compartment the multi-selection
    /// carries (it, or one around it, is selected): that grabs the group, as
    /// pressing inside a selected shape does in any drawing program.
    fn in_selected_compartment(target: &Target, host: &dyn MapHost) -> bool {
        let Some(space) = target.closest(&targets::COMPARTMENT_SPACE) else { return false };
        let selection = host.multi_selection();
        if selection.compartments.is_empty() {
            return false;
        }
        let (cell_id, comp_id) = space_ids(space);
        let Some(cell) = host.map().and_then(|m| m.cells().iter().find(|c| c.id == cell_id)) else { return false };
        let mut at = Some(comp_id);
        for _ in 0..=cell.compartments.len() {
            let Some(id) = at else { break };
            if selection.compartments.contains(&(cell_id.clone(), id.clone())) {
                return true;
            }
            at = cell.compartments.iter().find(|c| c.id == id).and_then(|c| c.parent_id.clone());
        }
        false
    }

    fn finish_group_drag(&mut self, ev: &PointerInput, host: &mut dyn MapHost) -> Option<GroupDrag> {
        let g = self.group_drag.take()?;
        self.release(ev);
        if g.moved {
            host.commit_map_edit(g.history.clone());
        } else if g.toggle_on_click
            && let Some((kind, id)) = &g.hit {
                host.toggle_multi_selection(*kind, id);
            }
        self.paint = true;
        Some(g)
    }

    // Pinch zoom

    fn begin_pinch_if_ready(&mut self, ev: &PointerInput, stage: &mut Stage, host: &mut dyn MapHost) -> bool {
        if ev.pointer_type != PointerType::Touch {
            return false;
        }
        self.touch_pointers.insert(ev.pointer_id, ev.client);
        if self.touch_pointers.len() < 2 {
            return false;
        }
        self.close_map_menus();
        self.suspend_single_pointer_gesture_for_pinch(host);
        let mut points = self.touch_pointers.values().copied();
        let (first, second) = (points.next().unwrap_or_default(), points.next().unwrap_or_default());
        self.pinch = camera::begin_pinch(stage.camera, first, second, stage.rect);
        self.suppress_touch_until_clear = true;
        self.capture(ev);
        self.pinch.is_some()
    }

    fn update_touch_pointer(&mut self, ev: &PointerInput, stage: &mut Stage) -> bool {
        if ev.pointer_type != PointerType::Touch || !self.touch_pointers.contains_key(&ev.pointer_id) {
            return false;
        }
        self.touch_pointers.insert(ev.pointer_id, ev.client);
        let Some(pinch) = self.pinch else { return self.suppress_touch_until_clear };
        let points: Vec<Point> = self.touch_pointers.values().copied().collect();
        if points.len() >= 2 {
            camera::update_pinch(stage.camera, &pinch, points[0], points[1], stage.rect);
            self.camera_moved = true;
        }
        true
    }

    fn finish_touch_pointer(&mut self, ev: &PointerInput) -> bool {
        if ev.pointer_type != PointerType::Touch || !self.touch_pointers.contains_key(&ev.pointer_id) {
            return false;
        }
        let was_pinching = self.pinch.is_some() || self.suppress_touch_until_clear;
        self.touch_pointers.shift_remove(&ev.pointer_id);
        self.release(ev);
        if self.pinch.is_some() && self.touch_pointers.len() < 2 {
            self.pinch = None;
        }
        if self.touch_pointers.is_empty() {
            self.suppress_touch_until_clear = false;
        }
        was_pinching
    }

    /// `suspendSinglePointerGestureForPinch()`: a second finger turns
    /// whatever the first one began into a pinch; what it moved is kept.
    fn suspend_single_pointer_gesture_for_pinch(&mut self, host: &mut dyn MapHost) {
        self.multi_toggle_pointer = None;
        if let Some(g) = &self.compartment_label_drag {
            let id = g.pointer_id;
            self.finish_compartment_label_drag(id, host);
        }
        if let Some(g) = &self.metabolite_label_drag {
            let id = g.pointer_id;
            self.finish_metabolite_label_drag(id, false, host);
        }
        if let Some(g) = self.text_drag.take()
            && g.moved {
                host.commit_map_edit(g.history);
            }
        self.selection = None;
        if let Some(g) = self.group_drag.take()
            && g.moved {
                host.commit_map_edit(g.history);
            }
        if let Some(g) = self.node_drag.take() {
            self.paint = true;
            if g.moved {
                host.commit_map_edit(g.history);
            }
        }
        if let Some(g) = self.reaction_drag.take()
            && g.moved {
                host.commit_map_edit(g.history);
            }
        if let Some(g) = self.rotate_drag.take()
            && g.moved {
                host.commit_map_edit(g.history);
            }
        if let Some(g) = self.control_drag.take()
            && g.moved {
                host.commit_map_edit(g.history);
            }
        self.cell_draw = None;
        self.compartment_draw = None;
        if let Some(g) = self.canvas_resize.take()
            && g.moved {
                host.commit_map_edit(g.history);
            }
        if let Some(g) = self.geometry_drag.take()
            && g.moved {
                host.commit_map_edit(g.history);
            }
        self.pan = None;
    }

    // Pan

    fn begin_pan(&mut self, ev: &PointerInput, target: Target, stage: &Stage) {
        // Remember what was under the pointer now: capturing the pointer
        // retargets every later event to the stage, so by pointerup the
        // reaction that was clicked is no longer the event target.
        self.pan = Some(PanGesture {
            x: ev.client.x,
            y: ev.client.y,
            cx: stage.camera.x,
            cy: stage.camera.y,
            moved: false,
            target,
        });
        self.capture(ev);
    }

    fn end_pan(&mut self, ev: &PointerInput) -> Option<PanGesture> {
        let g = self.pan.take();
        self.release(ev);
        g
    }

    // Pointer plumbing

    /// The stage's `pointerdown`. Menus are click-only: hover may highlight
    /// geometry, but it never opens, closes or switches a menu.
    pub fn pointer_down(&mut self, stage: &mut Stage, host: &mut dyn MapHost, ev: &PointerInput) {
        if self.text_editor.is_some() && !ev.on_text_editor {
            self.commit_text_editor(host);
            return;
        }
        if ev.on_text_editor {
            return;
        }
        if ev.button == 0 {
            self.press = Some((ev.pointer_id, ev.client));
        }
        let target = self.target(stage, ev);
        self.cross_feed_press = target.closest(&targets::CROSSFEED_ROUTE).map(CrossFeedLine::of);
        if self.begin_pinch_if_ready(ev, stage, host) {
            return;
        }
        if ev.pointer_type == PointerType::Touch && self.suppress_touch_until_clear {
            return;
        }

        // Desktop navigation is deliberately separated from editing:
        // middle-button drag always pans the canvas; left-button is reserved
        // for selection and the primary action of the active tool.
        if ev.pointer_type == PointerType::Mouse && ev.button == 1 {
            self.begin_pan(ev, target, stage);
            return;
        }
        if ev.pointer_type == PointerType::Mouse && ev.button != 0 {
            return;
        }

        // Pan mode: the drag the middle button does everywhere, on the primary
        // button, and nothing else. Every gesture below starts from this
        // handler, so not reaching them is what makes the mode unable to change
        // anything. Space held does the same for one drag.
        if self.tool == Tool::Pan || self.space_held {
            self.begin_pan(ev, target, stage);
            return;
        }

        if let Some(k) = target.closest_index(&targets::COMPARTMENT_LABEL)
            && self.moves_things() {
                self.begin_compartment_label_drag(&target, k, ev, stage, host);
                return;
            }

        if let Some(edge) = targets::canvas_resize_at(&target) {
            self.begin_canvas_resize(edge, ev, host);
            return;
        }

        if self.tool == Tool::Cell {
            let existing_cell_geometry = target.closest(&targets::CELL_GEOMETRY).is_some();
            if !existing_cell_geometry && self.begin_cell_draw(ev, stage, host) {
                return;
            }
        }

        if self.tool == Tool::Compartment {
            let handle = target.closest(&targets::GEOMETRY_HANDLE_ANY).is_some();
            let placement = host.current_compartment_placement();
            let stack = stage.elements_from_point(ev.client, self.hit_filter());
            let space = targets::draw_target_at(&target, &stack, &targets::COMPARTMENT_SPACE);
            let on_expected_parent = match (&placement, space) {
                (Some(p), Some(space)) => {
                    space.get("data-cell-space") == Some(p.species_id.as_str())
                        && space.get("data-compartment-space") == p.parent_id.as_deref()
                }
                _ => false,
            };
            if !handle && on_expected_parent && self.begin_compartment_draw(ev, stage, host) {
                return;
            }
        }

        let multi_hit = targets::multi_selectable_at(&target, host);
        let selecting = ev.shift || host.multi_select();
        let geometry_edit = host.geometry_edit();

        if target.closest(&targets::SELECTION_DRAG).is_some()
            && !geometry_edit
            && host.multi_selection_size() >= 2
            && self.moves_things()
        {
            self.begin_group_drag(None, ev, false, stage, host);
            return;
        }

        // The empty inside of a selected compartment moves the group it is
        // part of; its membrane still resizes it, and what is drawn inside
        // answers for itself below.
        if !geometry_edit
            && !selecting
            && host.multi_selection_size() >= 2
            && self.moves_things()
            && !targets::map_content_at(&target)
            && Self::in_selected_compartment(&target, host)
        {
            self.begin_group_drag(None, ev, false, stage, host);
            return;
        }

        // An annotation is dragged on its own by default, but it joins a group
        // like anything else: shift, the multi-select tool, or already being
        // in the group all have to reach the branches below.
        let text_id = targets::map_text_at(&target).filter(|t| !t.is_empty());
        if let Some(text_id) = text_id {
            let grouped = host.is_multi_selected(SelectKind::Text, &text_id) && host.multi_selection_size() >= 2 && self.moves_things();
            if !geometry_edit && !selecting && !grouped {
                // Every tool that moves things moves an annotation too, Select
                // included: otherwise a press on one under Select would do
                // nothing at all.
                if self.moves_things() {
                    self.begin_text_drag(text_id, ev, stage, host);
                }
                return;
            }
        }

        if host.text_tool() {
            self.begin_pan(ev, target, stage);
            return;
        }

        if let Some(label_node_id) = targets::metabolite_label_at(&target, host)
            && !geometry_edit && !selecting {
                if self.moves_things() {
                    self.begin_metabolite_label_drag(label_node_id, ev, stage, host);
                } else if self.tool == Tool::Reaction {
                    self.begin_reaction_pick(label_node_id, ev);
                } else if self.tool != Tool::Metabolite {
                    host.select_entity("metabolite", &label_node_id, None);
                }
                return;
            }

        if selecting && !geometry_edit {
            if let Some((kind, id)) = multi_hit {
                if host.is_multi_selected(kind, &id) && host.multi_selection_size() >= 2 && self.moves_things() {
                    self.begin_group_drag(Some((kind, id)), ev, true, stage, host);
                } else {
                    self.multi_toggle_pointer = Some(ev.pointer_id);
                    self.capture(ev);
                    host.toggle_multi_selection(kind, &id);
                }
                return;
            }
            self.begin_selection_rect(ev, stage);
            return;
        }

        if !geometry_edit
            && let Some((kind, id)) = &multi_hit
                && host.is_multi_selected(*kind, id) && host.multi_selection_size() >= 2 && self.moves_things() {
                    self.begin_group_drag(multi_hit.clone(), ev, false, stage, host);
                    return;
                }

        // Select edits compartment geometry from its edge, never from its
        // middle: a membrane resizes, its handles move; the space inside
        // belongs to what is drawn there and to the selection rectangle.
        // The handles come first and are not subject to the content guard: a
        // handle is drawn over the thing it belongs to.
        if !geometry_edit && self.tool == Tool::Select
            && let Some(handle) = targets::geometry_handle_at(&target) {
                self.begin_geometry_drag(handle, ev, stage, host);
                return;
            }

        if !geometry_edit && self.tool == Tool::Select && !targets::map_content_at(&target) {
            let at = stage.screen_to_map(ev.client);
            // The shared medium is a compartment like any other here: its
            // membrane resizes it, its handles move it.
            if let Some(h) = targets::shared_medium_boundary_handle_at(&target, host, at) {
                host.select_geometry(h.cell_id.as_deref(), &h.compartment_id);
                self.begin_geometry_drag(h, ev, stage, host);
                return;
            }
            if let Some(h) = targets::compartment_boundary_handle_at(&target, host, at) {
                self.begin_geometry_drag(h, ev, stage, host);
                return;
            }
        }

        // The Edit tool must also let the user grab the shared medium's
        // boundary directly: moving the system boundary should not require
        // discovering a second mode.
        if !geometry_edit && self.tool == Tool::Edit {
            let at = stage.screen_to_map(ev.client);
            if let Some(h) = targets::shared_medium_boundary_handle_at(&target, host, at) {
                self.begin_geometry_drag(h, ev, stage, host);
                return;
            }
        }

        if geometry_edit {
            if let Some(h) = targets::geometry_handle_at(&target) {
                self.begin_geometry_drag(h, ev, stage, host);
                return;
            }
            // The shared medium's boundary is itself a resize handle: on large
            // maps its corner handles can be off-screen or hard to hit.
            let at = stage.screen_to_map(ev.client);
            if let Some(h) = targets::shared_medium_boundary_handle_at(&target, host, at) {
                host.select_geometry(h.cell_id.as_deref(), &h.compartment_id);
                self.begin_geometry_drag(h, ev, stage, host);
                return;
            }
            self.begin_pan(ev, target, stage);
            return;
        }

        if let Some(rotating) = targets::rotate_handle_at(&target, host) {
            self.begin_rotate_drag(rotating, ev, host);
            return;
        }

        if let Some(control) = targets::control_handle_at(&target) {
            self.begin_control_drag(control, ev, host);
            return;
        }

        if let Some(reaction_id) = targets::draggable_reaction_at(&target, host) {
            // Add Metabolite acts on the compartment under the pointer, so
            // what is drawn there must not answer first. Handled on pointerup.
            if self.tool == Tool::Metabolite {
                return;
            }
            // One press does both: it drags if the pointer travels and selects
            // if it does not. The marker moves a reaction at once; a curve only
            // once the reaction is selected, so a sweep begun on a long branch
            // does not carry the reaction off with it.
            let movable = self.moves_things()
                && (targets::reaction_marker_at(&target) || host.selected_reaction().as_deref() == Some(reaction_id.as_str()));
            self.begin_reaction_drag(reaction_id, ev, movable, stage, host);
            return;
        }

        if let Some(node_id) = targets::draggable_node_at(&target, host) {
            if self.tool == Tool::Reaction {
                self.begin_reaction_pick(node_id, ev);
                return;
            }
            if self.tool == Tool::Metabolite {
                return;
            }
            // A press that moves drags the metabolite; one that does not still
            // selects it, which pointerup does from the finished gesture.
            if self.moves_things() {
                self.begin_node_drag(node_id, ev, stage, host);
            } else {
                host.select_entity("metabolite", &node_id, None);
            }
            return;
        }

        // Dragging the background in Select sweeps out a selection rectangle,
        // as every graphics editor does. Panning is still the middle button,
        // and has a tool of its own.
        if self.tool == Tool::Select && ev.pointer_type == PointerType::Mouse {
            self.begin_selection_rect(ev, stage);
            return;
        }
        if ev.pointer_type != PointerType::Mouse {
            self.begin_pan(ev, target, stage);
        }
    }

    /// The stage's `pointermove`.
    pub fn pointer_move(&mut self, stage: &mut Stage, host: &mut dyn MapHost, ev: &PointerInput) {
        if self.update_touch_pointer(ev, stage) {
            return;
        }
        if self.compartment_label_drag.is_some() {
            self.update_compartment_label_drag(ev, stage, host);
            return;
        }
        if self.metabolite_label_drag.is_some() {
            self.update_metabolite_label_drag(ev, stage, host);
            return;
        }
        if self.text_drag.is_some() {
            self.update_text_drag(ev, stage, host);
            return;
        }
        if self.selection.is_some() {
            self.update_selection_rect(ev, stage);
            return;
        }
        if self.group_drag.is_some() {
            self.update_group_drag(ev, stage, host);
            return;
        }
        if self.cell_draw.is_some() {
            self.update_cell_draw(ev, stage);
            return;
        }
        if self.compartment_draw.is_some() {
            self.update_compartment_draw(ev, stage);
            return;
        }
        if self.canvas_resize.is_some() {
            self.update_canvas_resize(ev, stage, host);
            return;
        }
        if self.geometry_drag.is_some() {
            self.update_geometry_drag(ev, stage, host);
            return;
        }
        if self.rotate_drag.is_some() {
            self.update_rotate_drag(ev, stage, host);
            return;
        }
        if self.control_drag.is_some() {
            self.update_control_drag(ev, stage, host);
            return;
        }
        if self.reaction_drag.is_some() {
            self.update_reaction_drag(ev, stage, host);
            return;
        }
        if self.reaction_pick.is_some() {
            self.update_reaction_pick(ev);
            return;
        }
        if self.node_drag.is_some() {
            self.update_node_drag(ev, stage, host);
            return;
        }
        let Some(pan) = &mut self.pan else { return };
        let dx = ev.client.x - pan.x;
        let dy = ev.client.y - pan.y;
        if dx.abs() + dy.abs() > 3.0 {
            pan.moved = true;
        }
        stage.camera.x = pan.cx + dx;
        stage.camera.y = pan.cy + dy;
        self.camera_moved = true;
    }

    /// The stage's `pointerup`: ends the gesture, and a press that never
    /// moved is a click, answered by what it was on.
    pub fn pointer_up(&mut self, stage: &mut Stage, host: &mut dyn MapHost, ev: &PointerInput, time_ms: f64) {
        if self.swallow == Some(ev.pointer_id) {
            self.swallow = None;
            self.release(ev);
            return;
        }
        let event_target = self.target(stage, ev);
        self.click_up(stage, host, ev, &event_target);
        // The browser lets a captured pointer go when it comes up.
        if self.captured == Some(ev.pointer_id) {
            self.captured = None;
        }
        // `dblclick`: two clicks close together, on what is under the second.
        let press = self.press.take();
        let click = ev.pointer_type == PointerType::Mouse
            && ev.button == 0
            && press.is_some_and(|(id, p)| id == ev.pointer_id && js::hypot(ev.client.x - p.x, ev.client.y - p.y) <= DOUBLE_CLICK_PX);
        if click {
            let double = self
                .last_click
                .is_some_and(|(t, p)| time_ms - t <= DOUBLE_CLICK_MS && js::hypot(ev.client.x - p.x, ev.client.y - p.y) <= DOUBLE_CLICK_PX);
            if double {
                self.last_click = None;
                let target = stage.target_at(ev.client, self.hit_filter());
                self.double_click(&target, host);
            } else {
                self.last_click = Some((time_ms, ev.client));
            }
        }
    }

    /// The stage's `dblclick`.
    pub fn double_click(&mut self, target: &Target, host: &mut dyn MapHost) {
        if let Some(reaction) = target.data(&targets::ANY_RXN, "data-rxn").filter(|r| !r.is_empty()) {
            host.select(Some(&reaction));
            self.emit(CanvasEvent::FocusLowerBound);
        }
    }

    fn click_up(&mut self, stage: &mut Stage, host: &mut dyn MapHost, ev: &PointerInput, event_target: &Target) {
        if self.finish_touch_pointer(ev) {
            return;
        }
        if self.compartment_label_drag.is_some() {
            self.finish_compartment_label_drag(ev.pointer_id, host);
            return;
        }
        if self.metabolite_label_drag.is_some() {
            if let Some(g) = self.finish_metabolite_label_drag(ev.pointer_id, false, host)
                && !g.moved {
                    host.select_entity("metabolite", &g.node_id, None);
                }
            return;
        }
        if self.text_drag.is_some() {
            self.finish_text_drag(ev, host);
            return;
        }
        if self.reaction_pick.is_some() {
            self.finish_reaction_pick(ev, host);
            return;
        }
        if self.multi_toggle_pointer == Some(ev.pointer_id) {
            self.multi_toggle_pointer = None;
            self.release(ev);
            return;
        }
        if self.selection.is_some() {
            // A press that never moved is a click on the background, which
            // deselects and closes menus like any other, so it falls through.
            if self.finish_selection_rect(ev, host).is_some_and(|g| g.moved) {
                return;
            }
        }
        if self.group_drag.is_some() {
            self.finish_group_drag(ev, host);
            return;
        }
        if self.cell_draw.is_some() {
            self.finish_cell_draw(ev, host);
            return;
        }
        if self.compartment_draw.is_some() {
            self.finish_compartment_draw(ev, host);
            return;
        }
        if self.canvas_resize.is_some() {
            self.finish_canvas_resize(ev, host);
            return;
        }

        if self.geometry_drag.is_some() {
            let g = self.finish_geometry_drag(ev, host);
            if let Some(g) = g
                && !g.moved && !host.geometry_edit() && self.tool == Tool::Select {
                    // Selecting a membrane selects the compartment it belongs
                    // to, so its handles appear. (selectGeometry records the
                    // compartment as the selected entity; selectEntity after it
                    // would clear the handles again.)
                    self.close_map_menus();
                    if !g.info.compartment_id.is_empty() {
                        host.select_geometry(g.info.cell_id.as_deref(), &g.info.compartment_id);
                    }
                }
            return;
        }

        // `event.target` was settled when the event was dispatched: the stage,
        // if a gesture had captured the pointer.
        let event_target = event_target.clone();

        if host.geometry_edit() {
            let gesture = self.end_pan(ev);
            if gesture.as_ref().is_some_and(|g| g.moved) {
                return;
            }
            let origin = gesture.map(|g| g.target).unwrap_or(event_target);
            self.geometry_edit_click(&origin, host);
            return;
        }

        if self.rotate_drag.is_some() {
            self.finish_rotate_drag(ev, host);
            return;
        }
        if self.control_drag.is_some() {
            self.finish_control_drag(ev, host);
            return;
        }

        if self.reaction_drag.is_some() {
            if let Some(g) = self.finish_reaction_drag(ev, host)
                && !g.moved {
                    self.close_map_menus();
                    host.select(Some(&g.reaction_id));
                }
            return;
        }

        if self.node_drag.is_some() {
            if let Some(g) = self.finish_node_drag(ev, host)
                && !g.moved {
                    self.close_map_menus();
                    host.select_entity("metabolite", &g.node_id, None);
                }
            return;
        }

        let gesture = self.end_pan(ev);
        if gesture.as_ref().is_some_and(|g| g.moved) {
            return;
        }
        let origin = gesture.map(|g| g.target).unwrap_or(event_target);

        // A press that never moved is a click, and in Pan mode a click selects
        // nothing: the mode is left by choosing another tool.
        if self.tool == Tool::Pan {
            return;
        }

        if host.text_tool() {
            let at = stage.screen_to_map(ev.client);
            self.open_text_editor(None, at.x, at.y, String::new(), None, None, host);
            return;
        }

        // Add Metabolite asks the compartment, not what happens to be drawn in it.
        if self.tool == Tool::Metabolite
            && let Some(space) = origin.closest(&targets::COMPARTMENT_SPACE) {
                let (cell_id, compartment_id) = space_ids(space);
                self.close_map_menus();
                self.open_menu(MenuRequest::MetaboliteAdd { cell_id, compartment_id }, ev.client);
                return;
            }

        // A click on a cross-feeding line pins the name of what it carries to
        // it; a second click on the same line takes the name away.
        let cross_feed_line = self.cross_feed_press.take().or_else(|| origin.closest(&targets::CROSSFEED_ROUTE).map(CrossFeedLine::of));
        if let Some(line) = cross_feed_line {
            self.close_map_menus();
            let at = stage.screen_to_map(ev.client);
            host.toggle_cross_feeding_link(
                line.from.as_deref().unwrap_or(""),
                line.base.as_deref().unwrap_or(""),
                line.to.as_deref().unwrap_or(""),
                Some(at),
            );
            return;
        }

        // Reaction geometry has priority over the membrane beneath it: the
        // segments of exchanges and transporters overlap a boundary on purpose.
        if let Some(reaction_id) = targets::reaction_at(&origin).filter(|r| !r.is_empty()) {
            self.close_menu(MapMenu::Boundary);
            self.close_menu(MapMenu::Metabolite);
            self.close_menu(MapMenu::Compartment);
            host.select(Some(&reaction_id));
            return;
        }
        if origin.closest(&targets::ANY_NODE).is_some() {
            return;
        }

        if let Some(hit) = origin.closest(&targets::BOUNDARY) {
            let boundary_id = targets::data(hit, "data-boundary").unwrap_or_default();
            // The membrane's reaction menu is Add Reaction's.
            if self.tool == Tool::Reaction {
                self.open_menu(MenuRequest::Boundary { boundary_id }, ev.client);
                return;
            }
            let boundary = host.map().and_then(|m| m.boundary(&boundary_id)).map(|b| (b.cell_id.clone(), b.compartment_id.clone()));
            self.close_map_menus();
            if let Some((cell_id, compartment_id)) = boundary.filter(|(_, c)| !c.is_empty()) {
                host.select_geometry(cell_id.as_deref(), &compartment_id);
            }
            return;
        }

        if let Some(cell) = origin.closest(&targets::CELL_PLATE) {
            let id = targets::data(cell, "data-cell-plate").unwrap_or_default();
            self.close_map_menus();
            host.select_entity("cell", &id, None);
            return;
        }

        if let Some(space) = origin.closest(&targets::COMPARTMENT_SPACE) {
            let (cell_id, compartment_id) = space_ids(space);
            self.close_menu(MapMenu::Boundary);
            self.close_menu(MapMenu::Metabolite);
            if self.tool == Tool::Metabolite {
                self.close_menu(MapMenu::Compartment);
                self.open_menu(MenuRequest::MetaboliteAdd { cell_id, compartment_id }, ev.client);
                return;
            }
            self.close_menu(MapMenu::MetaboliteAdd);
            if self.tool == Tool::Reaction {
                self.open_menu(MenuRequest::Compartment { cell_id: cell_id.clone(), compartment_id: compartment_id.clone() }, ev.client);
            } else {
                self.close_menu(MapMenu::Compartment);
            }
            host.select_entity("compartment", &compartment_id, Some(&cell_id));
            return;
        }

        self.close_menu(MapMenu::Boundary);
        self.close_menu(MapMenu::Metabolite);
        self.close_menu(MapMenu::Compartment);
        self.close_menu(MapMenu::MetaboliteAdd);
        host.clear_multi_selection();
        host.clear_cross_feeding_pick();
        host.select(None);
    }

    /// The click of `pointerup` with Geometry Edit on.
    fn geometry_edit_click(&mut self, origin: &Target, host: &mut dyn MapHost) {
        if self.tool == Tool::Cell {
            let cell_plate = origin.closest(&targets::CELL_PLATE).and_then(|e| targets::data(e, "data-cell-plate"));
            let boundary = origin
                .closest(&targets::BOUNDARY)
                .and_then(|e| targets::data(e, "data-boundary"))
                .and_then(|id| host.map().and_then(|m| m.boundary(&id)).map(|b| b.cell_id.clone()));
            let space_cell = origin.closest(&targets::COMPARTMENT_SPACE).and_then(|e| targets::data(e, "data-cell-space"));
            // `cellPlate?.dataset.cellPlate ?? boundary?.cellId ?? compartmentHit?.dataset.cellSpace ?? null`
            let cell_id = cell_plate.or_else(|| boundary.flatten()).or(space_cell);
            match cell_id.filter(|c| !c.is_empty()) {
                Some(cell_id) => {
                    host.select_cell_geometry(&cell_id);
                }
                None => host.clear_geometry_selection(),
            }
            return;
        }
        if let Some(hit) = origin.closest(&targets::BOUNDARY) {
            let id = targets::data(hit, "data-boundary").unwrap_or_default();
            let boundary = host.map().and_then(|m| m.boundary(&id)).map(|b| (b.cell_id.clone(), b.compartment_id.clone()));
            if let Some((cell_id, compartment_id)) = boundary.filter(|(_, c)| !c.is_empty()) {
                host.select_geometry(cell_id.as_deref(), &compartment_id);
            }
            return;
        }
        if let Some(space) = origin.closest(&targets::COMPARTMENT_SPACE) {
            let (cell_id, compartment_id) = space_ids(space);
            host.select_geometry(Some(&cell_id), &compartment_id);
            return;
        }
        host.clear_geometry_selection();
    }

    /// The stage's `pointercancel`: the browser took the pointer over, or the
    /// window lost focus. The gesture ends instead of staying live for the
    /// next unrelated move.
    pub fn pointer_cancel(&mut self, host: &mut dyn MapHost, ev: &PointerInput) {
        if self.swallow == Some(ev.pointer_id) {
            self.swallow = None;
        }
        self.cancel_inner(host, ev);
        if self.captured == Some(ev.pointer_id) {
            self.captured = None;
        }
    }

    fn cancel_inner(&mut self, host: &mut dyn MapHost, ev: &PointerInput) {
        if self.finish_touch_pointer(ev) {
            return;
        }
        if self.compartment_label_drag.is_some() {
            self.finish_compartment_label_drag(ev.pointer_id, host);
            return;
        }
        if self.metabolite_label_drag.is_some() {
            self.finish_metabolite_label_drag(ev.pointer_id, true, host);
            return;
        }
        if self.text_drag.is_some() {
            self.finish_text_drag(ev, host);
            return;
        }
        if self.reaction_pick.is_some() {
            self.reaction_pick = None;
            self.release(ev);
            return;
        }
        if self.multi_toggle_pointer == Some(ev.pointer_id) {
            self.multi_toggle_pointer = None;
            self.release(ev);
            return;
        }
        if self.selection.is_some() {
            self.finish_selection_rect(ev, host);
            return;
        }
        if self.group_drag.is_some() {
            self.finish_group_drag(ev, host);
            return;
        }
        if self.cell_draw.take().is_some() {
            self.release(ev);
            self.paint = true;
            return;
        }
        if self.compartment_draw.take().is_some() {
            self.release(ev);
            self.paint = true;
            return;
        }
        if self.canvas_resize.is_some() {
            self.finish_canvas_resize(ev, host);
            return;
        }
        if self.geometry_drag.is_some() {
            self.finish_geometry_drag(ev, host);
            return;
        }
        if self.rotate_drag.is_some() {
            self.finish_rotate_drag(ev, host);
            return;
        }
        if self.control_drag.is_some() {
            self.finish_control_drag(ev, host);
            return;
        }
        if self.reaction_drag.is_some() {
            self.finish_reaction_drag(ev, host);
            return;
        }
        if self.node_drag.is_some() {
            self.finish_node_drag(ev, host);
            return;
        }
        if self.pan.is_some() {
            self.end_pan(ev);
        }
    }

    /// Escape during a pointer gesture: the gesture is undone and the press
    /// that began it does nothing more when it comes up. (JavaScript cancels
    /// only the metabolite label drag this way; the native stage cancels every
    /// drag.) Returns whether there was one.
    pub fn cancel_gesture(&mut self, stage: &mut Stage, host: &mut dyn MapHost) -> bool {
        if !self.active() {
            return false;
        }
        // What the drag had changed: put back as it was when it began.
        let mut restore: Option<MapEdit> = None;
        let mut keep = |moved: bool, history: Option<MapEdit>| {
            if moved && history.is_some() {
                restore = history;
            }
        };
        if let Some(g) = self.metabolite_label_drag.take() {
            host.restore_metabolite_label_angle(&g.node_id, g.label_angle_before);
        }
        if let Some(g) = self.node_drag.take() {
            keep(g.moved, g.history);
        }
        if let Some(g) = self.text_drag.take() {
            keep(g.moved, g.history);
        }
        if let Some(g) = self.reaction_drag.take() {
            keep(g.moved, g.history);
        }
        if let Some(g) = self.rotate_drag.take() {
            keep(g.moved, g.history);
        }
        if let Some(g) = self.control_drag.take() {
            keep(g.moved, g.history);
        }
        if let Some(g) = self.canvas_resize.take() {
            keep(g.moved, g.history);
        }
        if let Some(g) = self.geometry_drag.take() {
            keep(g.moved, g.history);
        }
        if let Some(g) = self.group_drag.take() {
            keep(g.moved, g.history);
        }
        if let Some(g) = self.compartment_label_drag.take() {
            keep(g.moved, g.history);
        }
        self.selection = None;
        self.cell_draw = None;
        self.compartment_draw = None;
        self.reaction_pick = None;
        self.multi_toggle_pointer = None;
        if let Some(g) = self.pan.take() {
            stage.camera.x = g.cx;
            stage.camera.y = g.cy;
            self.camera_moved = true;
        }
        if let Some(edit) = restore {
            host.restore_map_edit(&edit);
        }
        // Every gesture captures its pointer; its release is swallowed.
        self.swallow = self.captured.take();
        self.cross_feed_press = None;
        self.press = None;
        self.paint = true;
        true
    }

    // Tools

    /// `activateTool(tool)`.
    pub fn activate_tool(&mut self, tool: Tool, host: &mut dyn MapHost) {
        self.tool = tool;
        // The pointer halos must not swallow presses meant for the canvas underneath.
        self.drawing_tool = matches!(tool, Tool::Cell | Tool::Compartment);
        self.close_map_menus();
        self.cancel_text_editor();
        host.clear_multi_selection();
        let patch = match tool {
            Tool::Text => ViewPatch { text_tool: Some(true), geometry_edit: Some(false), multi_select: Some(false) },
            Tool::Cell | Tool::Compartment => ViewPatch { text_tool: Some(false), geometry_edit: Some(true), multi_select: Some(false) },
            _ => ViewPatch { text_tool: Some(false), geometry_edit: Some(false), multi_select: Some(false) },
        };
        host.set_view(patch);
        self.paint = true;
    }

    /// `ui.tool = …` without `activateTool` (a drawing that finished, Escape).
    fn set_tool_quietly(&mut self, tool: Tool) {
        if self.tool != tool {
            self.tool = tool;
            self.emit(CanvasEvent::ToolChanged(tool));
        }
        self.paint = true;
    }

    // The keys

    /// The canvas keys of the window's `keydown` (outside text fields).
    /// Returns whether the key was used; `fit` and `zoom` are asked of the
    /// caller, which owns the stage's size.
    pub fn key_down(&mut self, key: CanvasKey, mods: KeyMods, stage: &mut Stage, host: &mut dyn MapHost) -> KeyOutcome {
        if key == CanvasKey::Escape && self.active() {
            self.cancel_gesture(stage, host);
            return KeyOutcome::Used;
        }
        // Undo and redo wait for a drag in progress to finish.
        if mods.command && matches!(key, CanvasKey::Z | CanvasKey::Y) && self.map_drag_in_progress() {
            return KeyOutcome::Used;
        }
        if mods.command && key == CanvasKey::Z {
            self.finish_metabolite_label_drag_for_history(host);
            self.close_map_menus();
            if mods.shift { host.redo_map() } else { host.undo_map() };
            self.paint = true;
            return KeyOutcome::Used;
        }
        if mods.command && key == CanvasKey::Y {
            self.finish_metabolite_label_drag_for_history(host);
            self.close_map_menus();
            host.redo_map();
            self.paint = true;
            return KeyOutcome::Used;
        }
        if mods.command {
            return KeyOutcome::Ignored;
        }
        match key {
            CanvasKey::Escape => {
                if self.text_editor.is_some() {
                    self.cancel_text_editor();
                    return KeyOutcome::Used;
                }
                self.close_map_menus();
                host.clear_multi_selection();
                if host.text_tool() {
                    host.set_view(ViewPatch { text_tool: Some(false), ..Default::default() });
                }
                // A selected compartment shows its handles in Select too, not
                // only in Geometry Edit, so Escape drops it either way.
                host.clear_geometry_selection();
                if !host.geometry_edit() {
                    host.select(None);
                }
                self.set_tool_quietly(Tool::Select);
                KeyOutcome::Used
            }
            CanvasKey::Delete | CanvasKey::Backspace => {
                if host.multi_selection_size() > 0 {
                    self.close_map_menus();
                    host.remove_multi_selection();
                    self.paint = true;
                    return KeyOutcome::Used;
                }
                // A selected compartment or cell goes with what is drawn in it.
                // Read from what is selected, not from the handles: they can
                // outlive a click on a reaction, which must delete the
                // reaction. The shared medium (no cell) is the whole map and is
                // not deleted.
                let compartment = host.selected_entity().and_then(|selected| match selected.kind.as_str() {
                    "compartment" => selected.cell_id.clone().filter(|c| !c.is_empty()).map(|c| (c, selected.id.clone())),
                    "cell" => cell_root_of(host, &selected.id),
                    _ => None,
                });
                if let Some((cell_id, compartment_id)) = compartment {
                    self.close_map_menus();
                    host.remove_map_compartment(&cell_id, &compartment_id);
                    self.paint = true;
                    return KeyOutcome::Used;
                }
                if let Some(selected) = host.selected_entity() {
                    match selected.kind.as_str() {
                        "reaction" => {
                            host.remove_map_reaction(&selected.id);
                        }
                        "metabolite" if host.can_remove_map_node(&selected.id) => {
                            host.remove_map_node(&selected.id);
                        }
                        "text" => {
                            host.remove_map_text(&selected.id);
                        }
                        _ => {}
                    }
                }
                self.paint = true;
                KeyOutcome::Used
            }
            CanvasKey::Zero => KeyOutcome::Fit,
            CanvasKey::Plus => KeyOutcome::Zoom(1.3),
            CanvasKey::Minus => KeyOutcome::Zoom(1.0 / 1.3),
            CanvasKey::ArrowLeft | CanvasKey::ArrowRight | CanvasKey::ArrowUp | CanvasKey::ArrowDown => {
                let step = if mods.shift { NUDGE_STEP * 10.0 } else { NUDGE_STEP };
                let (dx, dy) = match key {
                    CanvasKey::ArrowLeft => (-step, 0.0),
                    CanvasKey::ArrowRight => (step, 0.0),
                    CanvasKey::ArrowUp => (0.0, -step),
                    _ => (0.0, step),
                };
                if self.nudge(dx, dy, host) { KeyOutcome::Used } else { KeyOutcome::Ignored }
            }
            CanvasKey::Z | CanvasKey::Y => KeyOutcome::Ignored,
        }
    }

    /// Arrow keys: move the selection by a step, as one undoable edit. (Not
    /// in the JavaScript app; the native stage adds it.)
    pub fn nudge(&mut self, dx: f64, dy: f64, host: &mut dyn MapHost) -> bool {
        if self.active() || self.text_editor.is_some() || host.map().is_none() {
            return false;
        }
        if host.multi_selection_size() > 0 {
            let edit = host.begin_map_edit("Move selection");
            let moved = host.move_multi_selection(dx, dy);
            host.commit_map_edit(edit);
            self.paint = true;
            return moved.is_some();
        }
        let Some(selected) = host.selected_entity() else { return false };
        let id = selected.id.as_str();
        let done = match selected.kind.as_str() {
            "metabolite" if host.can_move_node(id) => {
                let Some((x, y)) = host.map().and_then(|m| m.node(id)).map(|n| (n.x, n.y)) else { return false };
                let edit = host.begin_map_edit("Move metabolite");
                let r = host.move_node(id, x + dx, y + dy).is_some();
                host.commit_map_edit(edit);
                r
            }
            "reaction" if host.can_move_reaction(id) => {
                let Some((x, y)) = host.map().and_then(|m| m.reaction(id)).map(|r| (r.x, r.y)) else { return false };
                let on_membrane = host.map().and_then(|m| m.reaction(id)).is_some_and(|r| r.on_membrane);
                let edit = host.begin_map_edit(if on_membrane { "Move transporter" } else { "Move reaction" });
                let r = host.move_reaction(id, x + dx, y + dy).is_some();
                host.commit_map_edit(edit);
                r
            }
            "text" => {
                let Some((x, y)) = host.map().and_then(|m| m.text(id)).map(|t| (t.x, t.y)) else { return false };
                let edit = host.begin_map_edit("Move text");
                let r = host.move_map_text(id, x + dx, y + dy).is_some();
                host.commit_map_edit(edit);
                r
            }
            _ => false,
        };
        self.paint |= done;
        done
    }

    /// `finishMetaboliteLabelDragForHistory()`: undo must not land in the
    /// middle of a label drag.
    pub fn finish_metabolite_label_drag_for_history(&mut self, host: &mut dyn MapHost) {
        if let Some(g) = &self.metabolite_label_drag {
            let id = g.pointer_id;
            self.finish_metabolite_label_drag(id, false, host);
        }
    }

    /// Space pressed or let go: while it is held a primary drag pans.
    pub fn set_space_held(&mut self, held: bool) {
        self.space_held = held;
    }

    // Text editor

    /// `openTextEditor({ id, x, y, value, fontSize, fontFamily })`. A new
    /// text starts in the last size and font chosen; an existing one in its
    /// own.
    #[allow(clippy::too_many_arguments)]
    pub fn open_text_editor(
        &mut self,
        id: Option<String>,
        x: f64,
        y: f64,
        value: String,
        font_size: Option<f64>,
        font_family: Option<String>,
        host: &dyn MapHost,
    ) {
        self.close_map_menus();
        let style = host.text_style();
        self.text_editor = Some(TextEditorState {
            id: id.clone(),
            x,
            y,
            value,
            font_size: font_size.unwrap_or(style.font_size),
            font_family: font_family.unwrap_or(style.font_family),
            shown: false,
        });
        self.emit(CanvasEvent::TextEditorOpened { id });
        self.paint = true;
    }

    /// `commitTextEditor()`: write the text, or drop an empty one.
    pub fn commit_text_editor(&mut self, host: &mut dyn MapHost) {
        let Some(editor) = self.text_editor.take() else { return };
        self.emit(CanvasEvent::TextEditorClosed);
        self.paint = true;
        let value = editor.value.trim().to_string();
        if value.is_empty() {
            return;
        }
        let style = TextStyle { font_size: editor.font_size, font_family: editor.font_family.clone() };
        host.set_text_style(&style);
        match &editor.id {
            Some(id) => {
                host.update_map_text(id, &value, Some(&style));
            }
            None => {
                host.add_map_text(&value, editor.x, editor.y, &style);
            }
        }
    }

    /// `cancelTextEditor()`.
    pub fn cancel_text_editor(&mut self) {
        if self.text_editor.take().is_some() {
            self.emit(CanvasEvent::TextEditorClosed);
            self.paint = true;
        }
    }

    /// The editor's delete button: the text goes, and the editor with it.
    pub fn delete_edited_text(&mut self, host: &mut dyn MapHost) {
        let id = self.text_editor.take().and_then(|e| e.id);
        self.emit(CanvasEvent::TextEditorClosed);
        if let Some(id) = id {
            host.remove_map_text(&id);
        }
        self.paint = true;
    }
}

/// What a key asks of the stage's owner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum KeyOutcome {
    Ignored,
    Used,
    /// `camera.fitted = false; fitToStage()`.
    Fit,
    /// `zoomBy(factor)` about the stage's centre.
    Zoom(f64),
}

/// `data-cell-space` and `data-compartment-space` of a compartment's space.
fn space_ids(space: &coxeter_render::Element) -> (String, String) {
    (
        targets::data(space, "data-cell-space").unwrap_or_default(),
        targets::data(space, "data-compartment-space").unwrap_or_default(),
    )
}

/// `cellRootOf(cellId)`: a cell's outermost compartment; deleting it deletes
/// the cell.
pub fn cell_root_of(host: &dyn MapHost, cell_id: &str) -> Option<(String, String)> {
    let cell = host.map()?.cell(cell_id)?;
    let root = cell
        .compartments
        .iter()
        .find(|c| Some(&c.id) == cell.outer_compartment.as_ref())
        .or_else(|| cell.compartments.iter().find(|c| c.parent_id.is_none()))?;
    Some((cell_id.to_string(), root.id.clone()))
}
