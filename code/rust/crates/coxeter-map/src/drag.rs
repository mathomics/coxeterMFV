//! Manual node placement (port of `src/layout/drag.js`).
//!
//! Dragging a metabolite is a *graphical* operation: it moves one node and
//! the reaction geometry hanging off it, and nothing else. It never touches
//! the model, the LP or the automatic layout.
//!
//! What a move has to keep consistent:
//!
//! ```text
//!   node.x / node.y        the canonical position the renderer reads
//!   map.segments           the endpoint of every segment attached to that node
//! ```
//!
//! Reaction markers and reaction axes stay where the reaction editor puts
//! them. Every metabolite circle, including reaction-local secondary/currency
//! nodes, is independently draggable; its attached branch geometry follows
//! the move.

use std::collections::HashSet;

use coxeter_core::geometry::{CLEARANCE, clamp_to_rect, closest_perimeter_point, inset_all, signed_distance};
use coxeter_core::mapdata::{Node, NodeKind, Point, Rect};

use crate::editable_map::{EditableMap, SegEnd};
use crate::util::num_or;

/// Node kinds a user may move. Primary, shared-medium and reaction-local
/// secondary/currency metabolites are all first-class editable map nodes.
pub const DRAGGABLE_NODE_KINDS: [&str; 3] = ["metabolite", "medium", "currency"];

/// `isDraggableNode(node)`.
pub fn is_draggable_node(node: Option<&Node>) -> bool {
    node.is_some_and(|n| matches!(n.kind, NodeKind::Metabolite | NodeKind::Medium | NodeKind::Currency))
}

/// The camera of the map view: `translate(x, y) scale(scale)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub x: f64,
    pub y: f64,
    pub scale: f64,
}

/// `stageToMap(point, camera, stageBox)`: convert a point in client space to
/// layout coordinates.
///
/// The map is drawn at its full canvas resolution and the camera is applied
/// as a transform about the stage's top-left corner, so one map unit is one
/// layout unit and the inverse of the camera is all that is needed:
/// `map = (client - stageOrigin - cameraTranslation) / cameraScale`.
pub fn stage_to_map(point: Point, camera: Camera, stage_left: f64, stage_top: f64) -> Point {
    let scale = num_or(camera.scale, 1.0);
    Point::new((point.x - stage_left - camera.x) / scale, (point.y - stage_top - camera.y) / scale)
}

/// What a node needs to say for its constraint to be found: the JavaScript
/// passes a node, or a node-shaped stand-in for a reaction marker.
#[derive(Debug, Clone, Copy)]
pub struct Placeable<'a> {
    pub kind: &'a NodeKind,
    pub r: f64,
    pub cell_id: Option<&'a str>,
    pub compartment: Option<&'a str>,
    pub x: f64,
    pub y: f64,
}

impl<'a> From<&'a Node> for Placeable<'a> {
    fn from(n: &'a Node) -> Self {
        Placeable { kind: &n.kind, r: n.r, cell_id: n.cell_id.as_deref(), compartment: n.compartment.as_deref(), x: n.x, y: n.y }
    }
}

/// The region a node is allowed to occupy: a bounding rectangle plus the
/// rectangles it must stay out of. This is the same description the force
/// relaxation places nodes against, rebuilt from the finished layout so
/// manual and automatic placement cannot disagree.
#[derive(Debug, Clone, PartialEq)]
pub struct Constraint {
    pub bounds: Rect,
    pub holes: Vec<Rect>,
    pub pad: f64,
    pub clearance: f64,
}

/// Cells and compartments about to move by `(dx, dy)` together with the
/// nodes being checked (a dragged selection): the holes they make are taken
/// where they will be, not where they are.
#[derive(Debug, Clone, Copy)]
pub struct Moving<'a> {
    pub dx: f64,
    pub dy: f64,
    /// Ids of whole cells that move.
    pub cells: &'a HashSet<String>,
    /// `(cell id, compartment id)` of every compartment that moves.
    pub compartments: &'a HashSet<(String, String)>,
}

/// `nodeConstraint(layout, node)`: `None` when the node has no compartment on
/// the map, in which case the caller leaves it unconstrained.
pub fn node_constraint(map: &EditableMap, node: Placeable) -> Option<Constraint> {
    node_constraint_moving(map, node, None)
}

/// `nodeConstraint(layout, node, moving)`: as [`node_constraint`], with the
/// holes of what is `moving` taken where they will be.
pub fn node_constraint_moving(map: &EditableMap, node: Placeable, moving: Option<&Moving>) -> Option<Constraint> {
    let pad = node.r + CLEARANCE.bounds;
    let clearance = node.r + CLEARANCE.hole;
    let g = map.graph();
    let shifted = |r: Rect, moves: bool| match moving {
        Some(m) if moves => Rect::new(r.x + m.dx, r.y + m.dy, r.w, r.h),
        _ => r,
    };
    if *node.kind == NodeKind::Medium {
        // Shared-medium metabolites live in the environment and around the
        // cells, never inside one. The cell rectangles are grown by the same
        // margin the automatic layout uses, so a medium node keeps a visible
        // gap from a cell wall instead of resting against it.
        return Some(Constraint {
            bounds: inset_all(&g.environment.rect, CLEARANCE.environment),
            holes: g
                .cells
                .iter()
                .map(|c| inset_all(&shifted(c.rect, moving.is_some_and(|m| m.cells.contains(&c.id))), -CLEARANCE.cell))
                .collect(),
            pad,
            clearance,
        });
    }
    let cell = g.cells.iter().find(|c| Some(c.id.as_str()) == node.cell_id)?;
    let own = cell.compartments.iter().find(|c| Some(c.id.as_str()) == node.compartment)?;
    // Its own compartment is the cage; the compartments nested directly
    // inside it are the holes.
    Some(Constraint {
        bounds: own.rect,
        holes: cell
            .compartments
            .iter()
            .filter(|c| c.parent_id.as_deref() == Some(own.id.as_str()))
            .map(|c| shifted(c.rect, moving.is_some_and(|m| m.compartments.contains(&(cell.id.clone(), c.id.clone())))))
            .collect(),
        pad,
        clearance,
    })
}

/// `nodeFitsAt(layout, node, x, y, moving)`: is `(x, y)` a place `node` may
/// be, inside its compartment and out of the organelles nested in it? A node
/// with no compartment on the map fits anywhere.
pub fn node_fits_at(map: &EditableMap, node: Placeable, x: f64, y: f64, moving: Option<&Moving>) -> bool {
    node_constraint_moving(map, node, moving).is_none_or(|c| satisfies(Point::new(x, y), &c))
}

const EPS: f64 = 1e-6;

/// Does `p` satisfy every part of `c`?
fn satisfies(p: Point, c: &Constraint) -> bool {
    let in_bounds = clamp_to_rect(p, &c.bounds, c.pad);
    if (in_bounds.x - p.x).abs() > EPS || (in_bounds.y - p.y).abs() > EPS {
        return false;
    }
    c.holes.iter().all(|hole| signed_distance(hole, p) >= c.clearance - EPS)
}

/// Pull `p` into `c`: clamp it to the bounding rectangle, then push it out of
/// whichever hole it is deepest inside, worst first, and clamp again. If the
/// point still violates something afterwards, `None` says so rather than
/// returning a position that only looks legal.
fn resolve(p: Point, c: &Constraint) -> Option<Point> {
    let mut out = clamp_to_rect(p, &c.bounds, c.pad);
    for _ in 0..6 {
        let mut worst: Option<&Rect> = None;
        let mut depth = 0.0;
        for hole in &c.holes {
            let d = c.clearance - signed_distance(hole, out);
            if d > depth {
                depth = d;
                worst = Some(hole);
            }
        }
        let Some(worst) = worst else { return Some(out) };
        // Leave by the nearest wall, exactly as the relaxation's final pass does.
        let edge = closest_perimeter_point(worst, out, 0.0);
        let ox = num_or(edge.nx, if out.x < edge.x { -1.0 } else { 1.0 });
        let oy = num_or(edge.ny, if out.y < edge.y { -1.0 } else { 1.0 });
        out = clamp_to_rect(Point::new(edge.x + ox * c.clearance, edge.y + oy * c.clearance), &c.bounds, c.pad);
    }
    satisfies(out, c).then_some(out)
}

/// `clampNodePosition(layout, node, x, y)`: where a node is actually allowed
/// to go when the user asks for `(x, y)`.
///
/// In order of preference: the requested point, pulled inside the
/// compartment and out of any organelle nested in it; failing that, the
/// requested movement along one axis only, which lets a node slide along an
/// obstacle instead of sticking to it; failing that, where the node already
/// is. A node with no compartment on the map is returned unconstrained.
pub fn clamp_node_position(map: &EditableMap, node: Placeable, x: f64, y: f64) -> Point {
    let Some(c) = node_constraint(map, node) else { return Point::new(x, y) };
    let here = Point::new(node.x, node.y);
    for attempt in [Point::new(x, y), Point::new(x, here.y), Point::new(here.x, y)] {
        if let Some(p) = resolve(attempt, &c) {
            return p;
        }
    }
    here
}

/// Which end of segment `i` belongs to its metabolite.
///
/// Substrates are drawn *metabolite -> axis* and products *axis ->
/// metabolite*, recorded in `side`. When that is missing, the end that is not
/// the reaction's own axis point is the metabolite's.
fn node_endpoint_key(map: &EditableMap, i: usize) -> Option<SegEnd> {
    let seg = &map.g.segments[i];
    if seg.is_substrate() {
        return Some(SegEnd::From);
    }
    if seg.is_product() {
        return Some(SegEnd::To);
    }
    map.g.reactions.get(&seg.reaction_id)?.axis?;
    let (from, to) = map.links[i];
    if to.is_some() {
        return Some(SegEnd::From);
    }
    if from.is_some() {
        return Some(SegEnd::To);
    }
    None
}

/// `updateConnectedSegments(layout, nodeId, delta)`: move the endpoint of
/// every segment attached to `node_id` onto the node's current position.
///
/// A metabolite can be shared by many reactions, so every segment is
/// considered. The reaction axes are left untouched: the endpoint is
/// replaced rather than mutated, so a segment can never write through an
/// alias into the axis geometry. With a `delta`, the metabolite-side handle
/// `b1` moves along. Returns how many segments were updated.
pub fn update_connected_segments(map: &mut EditableMap, node_id: &str, delta: Option<(f64, f64)>) -> usize {
    let Some(node) = map.g.nodes.get(node_id) else { return 0 };
    let at = node.pos();
    let mut touched = 0;
    for i in 0..map.g.segments.len() {
        if map.g.segments[i].node_id.as_deref() != Some(node_id) {
            continue;
        }
        let Some(key) = node_endpoint_key(map, i) else { continue };
        map.set_seg(i, key, at);
        // Escher-style b1 is attached to the metabolite side of the curve.
        if let (Some((dx, dy)), Some(b1)) = (delta, map.g.segments[i].b1) {
            map.g.segments[i].b1 = Some(Point::new(b1.x + dx, b1.y + dy));
        }
        touched += 1;
    }
    touched
}

/// `moveLayoutNode(layout, nodeId, x, y)`: move a node to `(x, y)`, clamped to
/// its compartment, and bring the reaction geometry attached to it along.
/// The single entry point for manual metabolite placement. Returns `false`
/// if nothing moved (not a node, not draggable, or not a finite point).
pub fn move_layout_node(map: &mut EditableMap, node_id: &str, x: f64, y: f64) -> bool {
    let Some(node) = map.g.nodes.get(node_id) else { return false };
    if !is_draggable_node(Some(node)) || !x.is_finite() || !y.is_finite() {
        return false;
    }
    let old = node.pos();
    let p = clamp_node_position(map, Placeable::from(node), x, y);
    let node = map.g.nodes.get_mut(node_id).expect("present");
    node.x = p.x;
    node.y = p.y;
    update_connected_segments(map, node_id, Some((p.x - old.x, p.y - old.y)));
    true
}
