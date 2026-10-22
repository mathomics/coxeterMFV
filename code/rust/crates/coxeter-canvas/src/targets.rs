//! What a press is on, as `src/ui/app.js` reads it: `event.target` walked up
//! with `closest()` to the `data-*` attributes the renderer puts on its
//! elements (`mapContentAt`, `draggableNodeAt`, `reactionAt`,
//! `draggableReactionAt`, `reactionMarkerAt`, `rotateHandleAt`,
//! `multiSelectableAt`, `mapTextAt`, `metaboliteLabelAt`, `reactionLabelAt`,
//! `controlHandleAt`, `editHandleUnder`,
//! `canvasResizeAt`, `geometryHandleAt`, `sharedMediumBoundaryHandleAt`,
//! `compartmentBoundaryHandleAt` and `drawTargetAt`).
//!
//! A [`Target`] is the element under the pointer with its ancestors, copied
//! without their children. A repaint during a gesture replaces the scene the
//! element was found in; JavaScript keeps the detached element alive (a pan
//! remembers the target it began on and reads it on `pointerup`), and so does
//! the copy.
//!
//! `dataset` values are strings, possibly empty: where JavaScript tests one
//! for truth, an empty one counts as absent; where it passes it on with
//! `??`, it is kept.

use std::sync::LazyLock;

use coxeter_core::mapdata::{Point, Rect};
use coxeter_map::editable_map::GeometryTarget;
use coxeter_render::Element;

use crate::doc::Selector;
use crate::host::{MapHost, SelectKind};
use crate::scene::Scene;

/// `event.target` and its ancestors, nearest first, without children. Empty
/// for the stage itself: a press outside the map, or one the map does not
/// take (Pan mode), or one a captured pointer delivers.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Target {
    chain: Vec<Element>,
    /// The element's index in the scene it was found in.
    pub index: Option<usize>,
}

impl Target {
    /// The stage: no element of the map.
    pub fn stage() -> Target {
        Target::default()
    }

    /// Element `i` of a scene.
    pub fn of(scene: &Scene, i: usize) -> Target {
        let path = scene.doc.path(i);
        let chain = path.iter().rev().map(|e| shallow(e)).collect();
        Target { chain, index: Some(i) }
    }

    /// Whether this is the stage rather than an element of the map.
    pub fn is_stage(&self) -> bool {
        self.chain.is_empty()
    }

    /// The element itself.
    pub fn element(&self) -> Option<&Element> {
        self.chain.first()
    }

    /// The element and its ancestors, nearest first.
    pub fn chain(&self) -> &[Element] {
        &self.chain
    }

    /// `target.closest(selector)`: where in the chain the nearest match is.
    pub fn closest_index(&self, sel: &Selector) -> Option<usize> {
        self.chain.iter().position(|e| sel.matches(e))
    }

    /// `target.closest(selector)`.
    pub fn closest(&self, sel: &Selector) -> Option<&Element> {
        self.closest_index(sel).map(|k| &self.chain[k])
    }

    /// `hit.closest(selector)` for the element at `k` of the chain.
    pub fn closest_from(&self, k: usize, sel: &Selector) -> Option<&Element> {
        self.chain.get(k..)?.iter().find(|e| sel.matches(e))
    }

    /// `target.closest(selector)?.dataset[name]`.
    pub fn data(&self, sel: &Selector, name: &str) -> Option<String> {
        self.closest(sel)?.get(name).map(str::to_string)
    }
}

fn shallow(e: &Element) -> Element {
    Element { name: e.name, attrs: e.attrs.clone(), children: Vec::new(), close_ws: e.close_ws, paired: e.paired }
}

/// `dataset[name]` of an element.
pub fn data(e: &Element, name: &str) -> Option<String> {
    e.get(name).map(str::to_string)
}

/// A string JavaScript would take as true.
fn truthy(s: &Option<String>) -> Option<&str> {
    s.as_deref().filter(|v| !v.is_empty())
}

macro_rules! selector {
    ($name:ident, $text:expr) => {
        pub static $name: LazyLock<Selector> = LazyLock::new(|| Selector::parse($text));
    };
}

selector!(
    MAP_CONTENT,
    ".emc-node[data-node], .emc-marker[data-rxn], .emc-seg[data-rxn], \
     .emc-hit-reaction[data-rxn], .emc-hit-node[data-node], \
     [data-reaction-handle], [data-rotate-handle], [data-control-handle], [data-map-text], \
     [data-geometry-handle], [data-cell-plate], [data-metabolite-label], .emc-label-rxn[data-rxn]"
);
selector!(NODE_CIRCLE, ".emc-node[data-node], .emc-hit-node[data-node]");
selector!(NODE_ONLY, ".emc-node[data-node]");
selector!(REACTION_HANDLE, "[data-reaction-handle]");
selector!(ROTATE_HANDLE, "[data-rotate-handle]");
selector!(CONTROL_RXN, "[data-control-rxn]");
selector!(
    REACTION_VISUAL,
    ".emc-marker[data-rxn], .emc-seg[data-rxn], .emc-hit-reaction[data-rxn], .emc-label-rxn[data-rxn]"
);
selector!(REACTION_MARKER, ".emc-marker[data-rxn], .emc-hit-reaction[data-rxn], [data-reaction-handle]");
selector!(REACTION_SELECTABLE, ".emc-marker[data-rxn], .emc-seg[data-rxn]");
selector!(MAP_TEXT, "[data-map-text]");
selector!(METABOLITE_LABEL, "[data-metabolite-label]");
selector!(REACTION_LABEL, ".emc-label-rxn[data-rxn]");
selector!(EDIT_HANDLE, "[data-rotate-handle], [data-control-handle], [data-reaction-handle]");
selector!(CONTROL_HANDLE, "[data-control-rxn][data-control-node][data-control-handle]");
selector!(CANVAS_RESIZE, "[data-canvas-resize]");
selector!(GEOMETRY_HANDLE, "[data-geometry-handle][data-geometry-cell][data-geometry-comp]");
selector!(GEOMETRY_HANDLE_ANY, "[data-geometry-handle]");
selector!(BOUNDARY, "[data-boundary]");
selector!(COMPARTMENT_LABEL, "[data-compartment-label]");
selector!(CELL, "[data-cell]");
selector!(COMPARTMENT_SPACE, "[data-compartment-space]");
selector!(CELL_PLATE, "[data-cell-plate]");
selector!(SELECTION_DRAG, "[data-selection-drag]");
selector!(CROSSFEED_ROUTE, "[data-crossfeed-route]");
selector!(ANY_RXN, "[data-rxn]");
selector!(ANY_NODE, "[data-node]");
selector!(
    CELL_GEOMETRY,
    "[data-cell-space], [data-cell-boundary], [data-cell-plate], [data-geometry-handle], [data-boundary]"
);

/// `mapContentAt(target)`: whatever is drawn inside a compartment and has an
/// editing behaviour of its own (metabolites, reaction geometry and its
/// handles, text, and the geometry handles themselves). The compartment's own
/// membrane is deliberately absent: grabbing that is how a compartment is
/// resized.
pub fn map_content_at(t: &Target) -> bool {
    t.closest(&MAP_CONTENT).is_some()
}

/// `draggableNodeAt(target)`: the visible circle and the invisible hit circle
/// around it, and only those. Reaction segments carry `data-node` as well, so
/// a curve can be traced back to the metabolite it ends at, and a press on a
/// curve must not drag that node.
pub fn draggable_node_at(t: &Target, host: &dyn MapHost) -> Option<String> {
    let id = t.data(&NODE_CIRCLE, "data-node");
    truthy(&id).filter(|id| host.can_move_node(id)).map(str::to_string)
}

/// `reactionAt(target)`. A control point or the rotation lever is part of the
/// reaction it shapes: a click that ends on one is still a click on that
/// reaction. The hit layer is the generous target the map draws around every
/// reaction, resolved back to its reaction like the thin visible geometry. So
/// does the reaction's name: with a tool that does not move it, a press on it
/// is a press on the reaction.
pub fn reaction_at(t: &Target) -> Option<String> {
    let explicit = t
        .data(&REACTION_HANDLE, "data-reaction-handle")
        .or_else(|| t.data(&ROTATE_HANDLE, "data-rotate-handle"))
        .or_else(|| t.data(&CONTROL_RXN, "data-control-rxn"));
    explicit.or_else(|| t.data(&REACTION_VISUAL, "data-rxn"))
}

/// `draggableReactionAt(target)`.
pub fn draggable_reaction_at(t: &Target, host: &dyn MapHost) -> Option<String> {
    let id = reaction_at(t);
    truthy(&id).filter(|id| host.can_move_reaction(id)).map(str::to_string)
}

/// `reactionMarkerAt(target)`: was the press on the reaction's marker rather
/// than on one of its curves? The marker is the thing a reaction is picked up
/// by; a press on a curve is far more often the start of a sweep.
pub fn reaction_marker_at(t: &Target) -> bool {
    t.closest(&REACTION_MARKER).is_some()
}

/// `rotateHandleAt(target)`.
pub fn rotate_handle_at(t: &Target, host: &dyn MapHost) -> Option<String> {
    let id = t.data(&ROTATE_HANDLE, "data-rotate-handle");
    truthy(&id).filter(|id| host.can_rotate_reaction(id)).map(str::to_string)
}

/// `multiSelectableAt(target)`: what a shift-press adds to or takes from the
/// selection.
pub fn multi_selectable_at(t: &Target, host: &dyn MapHost) -> Option<(SelectKind, String)> {
    let node_id = t.data(&NODE_ONLY, "data-node").or_else(|| metabolite_label_at(t, host));
    if let Some(id) = truthy(&node_id).filter(|id| host.can_move_node(id)) {
        return Some((SelectKind::Node, id.to_string()));
    }
    let reaction_id = t.data(&REACTION_SELECTABLE, "data-rxn").or_else(|| reaction_label_at(t, host));
    if let Some(id) = truthy(&reaction_id).filter(|id| host.can_move_reaction(id)) {
        return Some((SelectKind::Reaction, id.to_string()));
    }
    let text_id = map_text_at(t);
    truthy(&text_id).map(|id| (SelectKind::Text, id.to_string()))
}

/// `mapTextAt(target)`.
pub fn map_text_at(t: &Target) -> Option<String> {
    t.data(&MAP_TEXT, "data-map-text")
}

/// `metaboliteLabelAt(target)`: the node whose label this is, if it is a
/// metabolite's.
pub fn metabolite_label_at(t: &Target, host: &dyn MapHost) -> Option<String> {
    let id = t.data(&METABOLITE_LABEL, "data-metabolite-label");
    let id = truthy(&id)?;
    let node = host.map()?.node(id)?;
    node.is_metabolite_kind().then(|| id.to_string())
}

/// `reactionLabelAt(target)`: the reaction whose name is under the pointer,
/// if it is one drawn on the map.
pub fn reaction_label_at(t: &Target, host: &dyn MapHost) -> Option<String> {
    let id = t.data(&REACTION_LABEL, "data-rxn");
    let id = truthy(&id)?;
    host.map()?.reaction(id).map(|_| id.to_string())
}

/// `editHandleUnder(event)`: in Edit the curve handles and the lever are drawn
/// under the names, and a name beside a marker can lie over one of them. The
/// handle wins: the name can be moved in Select, but a handle hidden under it
/// could not be reached at all. `stack` is `elementsFromPoint`, topmost first.
pub fn edit_handle_under(stack: &[Target]) -> Option<&Target> {
    stack.iter().find(|t| t.element().is_some_and(|e| EDIT_HANDLE.matches(e)))
}

/// A reaction's control point, as `controlHandleAt` reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct ControlHandle {
    pub reaction_id: String,
    pub node_id: String,
    /// `b1`, `b2` or `anchor`.
    pub handle: String,
}

/// `controlHandleAt(target)`.
pub fn control_handle_at(t: &Target) -> Option<ControlHandle> {
    let hit = t.closest(&CONTROL_HANDLE)?;
    Some(ControlHandle {
        reaction_id: data(hit, "data-control-rxn").unwrap_or_default(),
        node_id: data(hit, "data-control-node").unwrap_or_default(),
        handle: data(hit, "data-control-handle").unwrap_or_default(),
    })
}

/// `canvasResizeAt(target)`: the edge a canvas grip stands for.
pub fn canvas_resize_at(t: &Target) -> Option<String> {
    t.data(&CANVAS_RESIZE, "data-canvas-resize")
}

/// A compartment edge or handle to drag: what `geometryHandleAt` and the two
/// boundary functions return.
#[derive(Debug, Clone, PartialEq)]
pub struct GeometryHandle {
    /// `None` for the shared medium.
    pub cell_id: Option<String>,
    pub compartment_id: String,
    /// `move`, `n`, `ne`, … (see `rectFromHandle`).
    pub handle: String,
    /// `Some("boundary")` when the membrane itself was grabbed.
    pub source: Option<&'static str>,
    pub boundary_id: Option<String>,
}

/// `geometryHandleAt(target)`.
pub fn geometry_handle_at(t: &Target) -> Option<GeometryHandle> {
    let hit = t.closest(&GEOMETRY_HANDLE)?;
    Some(GeometryHandle {
        // `hit.dataset.geometryCell || null`.
        cell_id: data(hit, "data-geometry-cell").filter(|v| !v.is_empty()),
        compartment_id: data(hit, "data-geometry-comp").unwrap_or_default(),
        handle: data(hit, "data-geometry-handle").unwrap_or_default(),
        source: None,
        boundary_id: None,
    })
}

/// `sharedMediumBoundaryHandleAt(target, event)`: the shared medium's
/// membrane, as a handle on its nearest edge (`at` is the press in map
/// coordinates).
pub fn shared_medium_boundary_handle_at(t: &Target, host: &dyn MapHost, at: Point) -> Option<GeometryHandle> {
    let hit = t.closest(&BOUNDARY)?;
    let map = host.map()?;
    let boundary_id = data(hit, "data-boundary").unwrap_or_default();
    let env = map.environment();
    let boundary = map.boundary(&boundary_id)?;
    let is_shared_medium = boundary.cell_id.is_none() && boundary.compartment_id == env.id;
    if !is_shared_medium {
        return None;
    }
    let handle = coxeter_map::geometry_edit::nearest_rect_edge(&env.rect, at)?;
    Some(GeometryHandle {
        cell_id: None,
        compartment_id: env.id.clone(),
        handle: handle.to_string(),
        source: Some("boundary"),
        boundary_id: Some(boundary_id),
    })
}

/// `compartmentBoundaryHandleAt(target, event)`: a cell compartment's
/// membrane, as a handle on its nearest edge.
pub fn compartment_boundary_handle_at(t: &Target, host: &dyn MapHost, at: Point) -> Option<GeometryHandle> {
    let hit = t.closest(&BOUNDARY)?;
    let map = host.map()?;
    let boundary_id = data(hit, "data-boundary").unwrap_or_default();
    let boundary = map.boundary(&boundary_id)?;
    let cell_id = boundary.cell_id.clone().filter(|c| !c.is_empty())?;
    if boundary.compartment_id.is_empty() {
        return None;
    }
    let rect = geometry_rect(host, Some(&cell_id), &boundary.compartment_id)?.0;
    let handle = coxeter_map::geometry_edit::nearest_rect_edge(&rect, at)?;
    Some(GeometryHandle {
        cell_id: Some(cell_id),
        compartment_id: boundary.compartment_id.clone(),
        handle: handle.to_string(),
        source: Some("boundary"),
        boundary_id: Some(boundary_id),
    })
}

/// `geometryTarget(cellId, compartmentId)`: the rectangle it edits, and
/// whether it is the shared medium (`target.environment`).
pub fn geometry_rect(host: &dyn MapHost, cell_id: Option<&str>, compartment_id: &str) -> Option<(Rect, bool)> {
    let map = host.map()?;
    match map.geometry_target(cell_id, compartment_id)? {
        GeometryTarget::Environment => Some((map.environment().rect, true)),
        GeometryTarget::Compartment { cell, compartment } => Some((map.cells()[cell].compartments[compartment].rect, false)),
    }
}

/// `drawTargetAt(event, selector)`: what the drawing tools are really aiming
/// at. The target is whatever happens to be painted at that pixel (a curve, a
/// metabolite, a pointer halo), so the whole stack at that point is consulted
/// (`stack`: `elementsFromPoint`, topmost first), not just the topmost
/// element.
pub fn draw_target_at<'a>(t: &'a Target, stack: &'a [Target], sel: &Selector) -> Option<&'a Element> {
    if let Some(direct) = t.closest(sel) {
        return Some(direct);
    }
    stack.iter().find_map(|el| el.closest(sel))
}
