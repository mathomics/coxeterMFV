//! Direct editing of cell/compartment geometry (port of
//! `src/map/geometry-edit.js`).
//!
//! Geometry edits are graphical only. They transform the selected
//! compartment, its descendants and the map content biologically located
//! inside that subtree. The biological model and FBA are never touched.

use std::collections::{HashMap, HashSet};

use coxeter_core::boundaries::environment_key;
use coxeter_core::geometry::{CompartmentShape, compartment_shape};
use coxeter_core::js;
use coxeter_core::mapdata::{AxisEnd, MapCell, MapReaction, Point, Rect, SegmentKind};
use indexmap::{IndexMap, IndexSet};

use crate::drag::update_connected_segments;
use crate::editable_map::{EditableMap, GeometryTarget, SegEnd};
use crate::util::{point_from_value, point_value};

const MIN_W: f64 = 90.0;
const MIN_H: f64 = 64.0;
const PAD: f64 = 10.0;
/// How near a corner a reaction riding a resized side may be pushed.
const SIDE_MARGIN: f64 = 18.0;

/// What `editGeometryRect` returns: what was edited and the rectangle it got.
#[derive(Debug, Clone, PartialEq)]
pub struct GeometryEdit {
    pub cell_id: Option<String>,
    pub compartment_id: String,
    pub rect: Rect,
}

/// `geometryTarget(map, cellId, compartmentId)`: the environment when no
/// cell is named and the compartment is the environment's own id, else the
/// named compartment of the named cell.
pub fn geometry_target(map: &EditableMap, cell_id: Option<&str>, compartment_id: &str) -> Option<GeometryTarget> {
    let g = map.graph();
    if cell_id.is_none_or(str::is_empty) && compartment_id == g.environment.id {
        return Some(GeometryTarget::Environment);
    }
    let cell = g.cells.iter().position(|c| Some(c.id.as_str()) == cell_id)?;
    let compartment = g.cells[cell].compartments.iter().position(|c| c.id == compartment_id)?;
    Some(GeometryTarget::Compartment { cell, compartment })
}

/// A translation, as the JavaScript's general transform reads it.
#[derive(Debug, Clone, Copy)]
struct Transform {
    dx: f64,
    dy: f64,
}

fn translation(dx: f64, dy: f64) -> Transform {
    Transform { dx, dy }
}

/// `transformPoint(p, t)`: `t.to.x + (p.x - t.from.x) * t.sx` with from at the
/// origin and unit scale.
fn transform_point(p: Point, t: Transform) -> Point {
    Point::new(t.dx + (p.x - 0.0) * 1.0, t.dy + (p.y - 0.0) * 1.0)
}

fn translate_rect(r: Rect, dx: f64, dy: f64) -> Rect {
    Rect::new(r.x + dx, r.y + dy, r.w, r.h)
}

fn valid_rect(r: &Rect) -> bool {
    [r.x, r.y, r.w, r.h].iter().all(|v| v.is_finite()) && r.w > 0.0 && r.h > 0.0
}

fn nearly_equal(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-7
}

/// `editGeometryRect(map, cellId, compartmentId, requested)`.
///
/// Moving a compartment moves its entire visual subtree and all graphical
/// content that belongs to it, preserving relative geometry exactly.
/// Resizing creates or removes free drawing space: descendant compartments,
/// metabolites and internal reactions keep their size and absolute position,
/// and the compartment is not allowed to shrink through any of them; the
/// reactions attached to its membrane ride it (`follow_side`). A resized
/// side stops at the parent rather than shifting the rectangle.
pub fn edit_geometry_rect(map: &mut EditableMap, cell_id: Option<&str>, compartment_id: &str, requested: Rect) -> Option<GeometryEdit> {
    let target = geometry_target(map, cell_id, compartment_id)?;
    if !valid_rect(&requested) {
        return None;
    }
    let (ci, ki) = match target {
        GeometryTarget::Environment => return Some(edit_environment_rect(map, requested)),
        GeometryTarget::Compartment { cell, compartment } => (cell, compartment),
    };
    let cell_id = map.g.cells[ci].id.clone();
    let comp = map.g.cells[ci].compartments[ki].clone();
    let parent = match &comp.parent_id {
        Some(pid) => map.g.cells[ci].compartments.iter().find(|c| &c.id == pid).map(|c| c.rect),
        None => Some(map.g.environment.rect),
    }?;

    let old_rect = comp.rect;
    let parent_pad = if comp.parent_id.is_some() { PAD } else { 18.0 };
    let moving = nearly_equal(requested.w, old_rect.w) && nearly_equal(requested.h, old_rect.h);
    // A move keeps the size and is shifted back inside the parent. A resize
    // stops each side at the parent instead, since shifting it would move the
    // side not pulled.
    let wanted = if moving { requested } else { clip_rect(&requested, &parent, parent_pad) };
    let mut next_rect = clamp_rect(&wanted, &parent, parent_pad);
    let subtree = descendant_ids(&map.g.cells[ci], compartment_id);

    if moving {
        let dx = next_rect.x - old_rect.x;
        let dy = next_rect.y - old_rect.y;
        translate_subtree(map, ci, compartment_id, dx, dy, comp.parent_id.is_none().then_some(next_rect));
    } else {
        // Resizing creates or removes free drawing space. Descendant
        // compartments, metabolites and internal reactions keep their size
        // and absolute position, and the compartment is not allowed to shrink
        // through any of them. Reactions physically attached to the resized
        // boundary ride it instead.
        let attached = attached_reactions(map, &cell_id, compartment_id);

        if let Some(required) = required_bounds(map, &map.g.cells[ci], compartment_id, &subtree, &old_rect, &attached) {
            next_rect = contain_required(next_rect, required, &parent, parent_pad, &old_rect);
        }
        map.g.cells[ci].compartments[ki].rect = next_rect;
        if comp.parent_id.is_none() {
            map.g.cells[ci].rect = next_rect;
        }
        follow_boundary(map, &attached, &old_rect, &next_rect);
    }

    map.refresh_boundaries();
    Some(GeometryEdit { cell_id: Some(cell_id), compartment_id: compartment_id.to_string(), rect: next_rect })
}

/// `attachedReactions`: the reactions that sit on a compartment's membrane
/// and ride it.
fn attached_reactions(map: &EditableMap, cell_id: &str, compartment_id: &str) -> IndexSet<String> {
    let boundary_id = format!("{cell_id}/{compartment_id}");
    map.g
        .reactions
        .iter()
        .filter(|(_, r)| {
            r.cell_id.as_deref() == Some(cell_id)
                && (r.membrane.as_deref() == Some(compartment_id) || r.boundary_id.as_deref() == Some(boundary_id.as_str()))
        })
        .map(|(id, _)| id.clone())
        .collect()
}

// Shape

// The two shape handles and the tracks they run along live in
// `coxeter_core::geometry`, which the renderer shares (`SHAPE_HANDLES`,
// `shapeHandlePoint`, `shapeAmountAt` of geometry-edit.js).
pub use coxeter_core::geometry::{SHAPE_HANDLES, is_shape_handle, shape_amount_at, shape_handle_point};

/// What `editCompartmentShape` is asked for: either amount or both.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ShapePatch {
    pub corner: Option<f64>,
    pub oval: Option<f64>,
}

impl ShapePatch {
    /// Both amounts of a shape.
    pub fn of(shape: CompartmentShape) -> ShapePatch {
        ShapePatch { corner: Some(shape.corner), oval: Some(shape.oval) }
    }
    /// One handle's amount (`{ [handle]: amount }`).
    pub fn handle(which: &str, amount: f64) -> ShapePatch {
        match which {
            "corner" => ShapePatch { corner: Some(amount), oval: None },
            "oval" => ShapePatch { corner: None, oval: Some(amount) },
            _ => ShapePatch::default(),
        }
    }
}

/// What `editCompartmentShape` returns.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeEdit {
    pub cell_id: String,
    pub compartment_id: String,
    pub shape: CompartmentShape,
}

/// `editCompartmentShape(map, cellId, compartmentId, shape)`: give a
/// compartment of a cell a new shape, `{ corner, oval }`, either or both, each
/// from 0 to 1 (see `compartment_shape`). Its rectangle does not change. The
/// transporters on its membrane move onto the new outline and turn with it,
/// and take their own metabolites along. The shared medium has no shape.
pub fn edit_compartment_shape(map: &mut EditableMap, cell_id: Option<&str>, compartment_id: &str, shape: ShapePatch) -> Option<ShapeEdit> {
    let (ci, ki) = match geometry_target(map, cell_id, compartment_id)? {
        GeometryTarget::Environment => return None,
        GeometryTarget::Compartment { cell, compartment } => (cell, compartment),
    };
    let cell_id = map.g.cells[ci].id.clone();
    let compartment = &mut map.g.cells[ci].compartments[ki];
    let mut next = compartment_shape(compartment);
    if let Some(v) = shape.corner.filter(|v| v.is_finite()) {
        next.corner = js::min(1.0, js::max(0.0, v));
    }
    if let Some(v) = shape.oval.filter(|v| v.is_finite()) {
        next.oval = js::min(1.0, js::max(0.0, v));
    }
    compartment.shape = Some(next.to_json());

    for id in attached_reactions(map, &cell_id, compartment_id) {
        let Some(rxn) = map.g.reactions.get(&id) else { continue };
        if !rxn.on_membrane || rxn.boundary_id.as_deref().is_none_or(str::is_empty) {
            continue;
        }
        let (x, y) = (rxn.x, rxn.y);
        let owned: IndexMap<String, Point> =
            map.g.nodes.values().filter(|n| n.reaction_id.as_deref() == Some(id.as_str())).map(|n| (n.id.clone(), n.pos())).collect();
        crate::reaction_edit::move_reaction(map, &id, x, y);
        let moved: HashMap<String, Point> = owned
            .iter()
            .filter_map(|(node_id, before)| map.g.nodes.get(node_id).map(|n| (node_id.clone(), Point::new(n.x - before.x, n.y - before.y))))
            .collect();
        carry_handles(map, &IndexSet::from([id.clone()]), |node_id| moved.get(node_id).copied());
        for node_id in owned.keys() {
            update_connected_segments(map, node_id, None);
        }
    }

    map.refresh_boundaries();
    Some(ShapeEdit { cell_id, compartment_id: compartment_id.to_string(), shape: next })
}

fn edit_environment_rect(map: &mut EditableMap, requested: Rect) -> GeometryEdit {
    let old_rect = map.g.environment.rect;
    let moving = nearly_equal(requested.w, old_rect.w) && nearly_equal(requested.h, old_rect.h);
    if moving {
        let next = Rect::new(js::max(0.0, requested.x), js::max(0.0, requested.y), old_rect.w, old_rect.h);
        let dx = next.x - old_rect.x;
        let dy = next.y - old_rect.y;
        if dx.abs() + dy.abs() > 1e-9 {
            translate_whole_map(map, dx, dy);
        }
        map.g.environment.rect = next;
        map.g.canvas.width = js::max(map.g.canvas.width, next.x + next.w + 24.0);
        map.g.canvas.height = js::max(map.g.canvas.height, next.y + next.h + 24.0);
    } else {
        let env_key = environment_key();
        let attached: IndexSet<String> =
            map.g.reactions.iter().filter(|(_, r)| r.boundary_id.as_deref() == Some(env_key.as_str())).map(|(id, _)| id.clone()).collect();

        // As for a compartment: what rides the boundary does not hold it out,
        // and content nearer the edge than the margin only stops it where it
        // already is.
        let required = whole_map_required_bounds(map, &attached);
        let mut x1 = js::max(0.0, requested.x);
        let mut y1 = js::max(0.0, requested.y);
        let mut x2 = requested.x + requested.w;
        let mut y2 = requested.y + requested.h;
        if let Some(r) = required {
            x1 = js::min(x1, js::max(js::max(0.0, r.x1 - 18.0), old_rect.x));
            y1 = js::min(y1, js::max(js::max(0.0, r.y1 - 18.0), old_rect.y));
            x2 = js::max(x2, js::min(r.x2 + 18.0, old_rect.x + old_rect.w));
            y2 = js::max(y2, js::min(r.y2 + 18.0, old_rect.y + old_rect.h));
        }
        let w = js::max(MIN_W, x2 - x1);
        let h = js::max(MIN_H, y2 - y1);
        let next = Rect::new(x1, y1, w, h);

        follow_boundary(map, &attached, &old_rect, &next);

        map.g.environment.rect = next;
        map.g.canvas.width = js::max(map.g.canvas.width, next.x + next.w + 24.0);
        map.g.canvas.height = js::max(map.g.canvas.height, next.y + next.h + 24.0);
    }
    map.refresh_boundaries();
    GeometryEdit { cell_id: None, compartment_id: map.g.environment.id.clone(), rect: map.g.environment.rect }
}

/// `translateWholeMap(map, dx, dy)`: move everything drawn (cells,
/// compartments, nodes, reactions, segments and texts) but not the
/// environment rectangle.
pub fn translate_whole_map(map: &mut EditableMap, dx: f64, dy: f64) {
    for cell in &mut map.g.cells {
        cell.rect = translate_rect(cell.rect, dx, dy);
        for comp in &mut cell.compartments {
            comp.rect = translate_rect(comp.rect, dx, dy);
        }
    }
    for node in map.g.nodes.values_mut() {
        node.x += dx;
        node.y += dy;
    }
    let transform = translation(dx, dy);
    let ids: Vec<String> = map.g.reactions.keys().cloned().collect();
    let mut affected = IndexSet::new();
    for rid in ids {
        transform_reaction(map, &rid, transform);
        affected.insert(rid);
    }
    transform_segments(map, &affected, transform, None);
    let node_ids: Vec<String> = map.g.nodes.keys().cloned().collect();
    for id in node_ids {
        update_connected_segments(map, &id, None);
    }
    for text in map.g.texts.values_mut() {
        if text.x.is_finite() {
            text.x += dx;
        }
        if text.y.is_finite() {
            text.y += dy;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Bounds4 {
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
}

fn union_bounds(items: &[Bounds4]) -> Option<Bounds4> {
    if items.is_empty() {
        return None;
    }
    Some(Bounds4 {
        x1: js::min_n(items.iter().map(|b| b.x1)),
        y1: js::min_n(items.iter().map(|b| b.y1)),
        x2: js::max_n(items.iter().map(|b| b.x2)),
        y2: js::max_n(items.iter().map(|b| b.y2)),
    })
}

fn whole_map_required_bounds(map: &EditableMap, attached: &IndexSet<String>) -> Option<Bounds4> {
    let mut b = Vec::new();
    for cell in &map.g.cells {
        b.push(Bounds4 { x1: cell.rect.x, y1: cell.rect.y, x2: cell.rect.x + cell.rect.w, y2: cell.rect.y + cell.rect.h });
    }
    for node in map.g.nodes.values() {
        if owned_by(node.reaction_id.as_deref(), attached) {
            continue;
        }
        let r = node.r;
        b.push(Bounds4 { x1: node.x - r, y1: node.y - r, x2: node.x + r, y2: node.y + r });
    }
    for (id, rxn) in &map.g.reactions {
        if attached.contains(id) {
            continue;
        }
        b.push(Bounds4 { x1: rxn.x - 18.0, y1: rxn.y - 18.0, x2: rxn.x + 18.0, y2: rxn.y + 18.0 });
    }
    for t in map.g.texts.values() {
        if !t.x.is_finite() || !t.y.is_finite() {
            continue;
        }
        b.push(Bounds4 { x1: t.x - 8.0, y1: t.y - 16.0, x2: t.x + 120.0, y2: t.y + 8.0 });
    }
    union_bounds(&b)
}

/// Move compartment `compartment_id` of cell `ci`, everything nested in it
/// and all the graphical content that belongs to that subtree by `(dx, dy)`,
/// preserving relative geometry exactly. Nothing is clamped and the
/// boundaries are not refreshed: the caller has already decided the move is
/// allowed, and refreshes once it has moved everything it is moving. A cell's
/// own (root) compartment also moves the cell, to `cell_rect`.
pub(crate) fn translate_subtree(map: &mut EditableMap, ci: usize, compartment_id: &str, dx: f64, dy: f64, cell_rect: Option<Rect>) {
    let cell_id = map.g.cells[ci].id.clone();
    let subtree = descendant_ids(&map.g.cells[ci], compartment_id);
    let transform = translation(dx, dy);
    for c in &mut map.g.cells[ci].compartments {
        if subtree.contains(&c.id) {
            c.rect = translate_rect(c.rect, dx, dy);
        }
    }
    if let Some(r) = cell_rect {
        map.g.cells[ci].rect = r;
    }

    let mut affected_nodes: IndexSet<String> = IndexSet::new();
    for node in map.g.nodes.values_mut() {
        if node.cell_id.as_deref() != Some(cell_id.as_str()) || !node.compartment.as_ref().is_some_and(|c| subtree.contains(c)) {
            continue;
        }
        node.x += dx;
        node.y += dy;
        affected_nodes.insert(node.id.clone());
    }

    let mut affected_reactions: IndexSet<String> = IndexSet::new();
    let ids: Vec<String> = map.g.reactions.keys().cloned().collect();
    for rid in ids {
        if !reaction_in_subtree(&map.g.reactions[&rid], &cell_id, &subtree) {
            continue;
        }
        transform_reaction(map, &rid, transform);
        affected_reactions.insert(rid.clone());
        for node in map.g.nodes.values_mut() {
            if node.reaction_id.as_deref() != Some(rid.as_str()) || affected_nodes.contains(&node.id) {
                continue;
            }
            node.x += dx;
            node.y += dy;
            affected_nodes.insert(node.id.clone());
        }
    }
    transform_segments(map, &affected_reactions, transform, Some(&affected_nodes));
    let delta = Point::new(dx, dy);
    carry_handles(map, &affected_reactions, |id| affected_nodes.contains(id).then_some(delta));
    for node_id in &affected_nodes {
        update_connected_segments(map, node_id, None);
    }
}

/// A node owned by one of `reactions` (its `reactionId` is one of them).
fn owned_by(reaction_id: Option<&str>, reactions: &IndexSet<String>) -> bool {
    reaction_id.is_some_and(|r| !r.is_empty() && reactions.contains(r))
}

/// Whether a reaction travels with a compartment subtree of its cell when
/// that subtree is moved: it runs in one of those compartments, sits on one
/// of their membranes, or is attached to one of their boundaries.
pub fn reaction_in_subtree(rxn: &MapReaction, cell_id: &str, subtree: &HashSet<String>) -> bool {
    if rxn.cell_id.as_deref() != Some(cell_id) {
        return false;
    }
    rxn.compartments.iter().any(|c| subtree.contains(c))
        || rxn.membrane.as_ref().is_some_and(|m| subtree.contains(m))
        || boundary_belongs(rxn.boundary_id.as_deref(), cell_id, subtree)
}

/// `nearestRectEdge(rect, point)`: `n`, `e`, `s` or `w`.
pub fn nearest_rect_edge(rect: &Rect, point: Point) -> Option<&'static str> {
    if !valid_rect(rect) || !point.x.is_finite() || !point.y.is_finite() {
        return None;
    }
    let mut d = [
        ("n", (point.y - rect.y).abs()),
        ("e", (point.x - (rect.x + rect.w)).abs()),
        ("s", (point.y - (rect.y + rect.h)).abs()),
        ("w", (point.x - rect.x).abs()),
    ];
    d.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    Some(d[0].0)
}

/// `rectFromHandle(startRect, handle, startPoint, point)`: the rectangle a
/// drag of one of its handles (`move`, `n`, `ne`, … `w`) gives.
pub fn rect_from_handle(start: &Rect, handle: &str, start_point: Point, point: Point) -> Option<Rect> {
    if !valid_rect(start) {
        return None;
    }
    let dx = point.x - start_point.x;
    let dy = point.y - start_point.y;
    if handle == "move" {
        return Some(Rect::new(start.x + dx, start.y + dy, start.w, start.h));
    }
    let mut x1 = start.x;
    let mut y1 = start.y;
    let mut x2 = start.x + start.w;
    let mut y2 = start.y + start.h;
    if handle.contains('w') {
        x1 += dx;
    }
    if handle.contains('e') {
        x2 += dx;
    }
    if handle.contains('n') {
        y1 += dy;
    }
    if handle.contains('s') {
        y2 += dy;
    }
    if x2 - x1 < MIN_W {
        if handle.contains('w') {
            x1 = x2 - MIN_W;
        } else {
            x2 = x1 + MIN_W;
        }
    }
    if y2 - y1 < MIN_H {
        if handle.contains('n') {
            y1 = y2 - MIN_H;
        } else {
            y2 = y1 + MIN_H;
        }
    }
    Some(Rect::new(x1, y1, x2 - x1, y2 - y1))
}

/// The compartment and everything nested in it, by id.
pub fn descendant_ids(cell: &MapCell, root: &str) -> HashSet<String> {
    let mut out: HashSet<String> = HashSet::from([root.to_string()]);
    let mut changed = true;
    while changed {
        changed = false;
        for c in &cell.compartments {
            if let Some(p) = &c.parent_id {
                if out.contains(p) && !out.contains(&c.id) {
                    out.insert(c.id.clone());
                    changed = true;
                }
            }
        }
    }
    out
}

/// `[...descendantIds(cell, root)]`: the compartment and everything nested in
/// it, in the order they are found.
pub fn descendant_list(cell: &MapCell, root: &str) -> Vec<String> {
    let mut out: IndexSet<String> = IndexSet::from([root.to_string()]);
    let mut changed = true;
    while changed {
        changed = false;
        for c in &cell.compartments {
            if let Some(p) = &c.parent_id {
                if out.contains(p) && !out.contains(&c.id) {
                    out.insert(c.id.clone());
                    changed = true;
                }
            }
        }
    }
    out.into_iter().collect()
}

/// The minimum size adapts to very small parents so containment always
/// wins: a child compartment must never be allowed to protrude beyond its
/// parent.
fn clamp_rect(r: &Rect, parent: &Rect, pad: f64) -> Rect {
    let inner_w = js::max(1.0, parent.w - pad * 2.0);
    let inner_h = js::max(1.0, parent.h - pad * 2.0);
    let min_w = js::min(MIN_W, inner_w);
    let min_h = js::min(MIN_H, inner_h);
    let w = js::min(js::max(min_w, r.w), inner_w);
    let h = js::min(js::max(min_h, r.h), inner_h);
    let x = js::min(js::max(r.x, parent.x + pad), parent.x + parent.w - pad - w);
    let y = js::min(js::max(r.y, parent.y + pad), parent.y + parent.h - pad - h);
    Rect::new(x, y, w, h)
}

/// `clipRect(r, parent, pad)`: each side of `r` stopped at the parent's
/// inside edge.
fn clip_rect(r: &Rect, parent: &Rect, pad: f64) -> Rect {
    let x1 = js::max(r.x, parent.x + pad);
    let y1 = js::max(r.y, parent.y + pad);
    let x2 = js::min(r.x + r.w, parent.x + parent.w - pad);
    let y2 = js::min(r.y + r.h, parent.y + parent.h - pad);
    Rect::new(x1, y1, x2 - x1, y2 - y1)
}

/// What a resized compartment must keep inside: its descendants and what is
/// drawn inside it now. The reactions attached to its own boundary and their
/// metabolites are left out, since they ride the boundary, and so is whatever
/// already lies outside it (a transporter's far side, say). Counting either
/// one would hold the boundary out beyond them, and since an attached
/// reaction follows the boundary, every step of a resize would push it out
/// again.
fn required_bounds(
    map: &EditableMap,
    cell: &MapCell,
    root: &str,
    subtree: &HashSet<String>,
    rect: &Rect,
    attached: &IndexSet<String>,
) -> Option<Bounds4> {
    let inside = |x: f64, y: f64| x >= rect.x && x <= rect.x + rect.w && y >= rect.y && y <= rect.y + rect.h;
    let mut b = Vec::new();
    for c in &cell.compartments {
        if c.id == root || !subtree.contains(&c.id) {
            continue;
        }
        b.push(Bounds4 { x1: c.rect.x, y1: c.rect.y, x2: c.rect.x + c.rect.w, y2: c.rect.y + c.rect.h });
    }
    for node in map.g.nodes.values() {
        if node.cell_id.as_deref() != Some(cell.id.as_str()) || !node.compartment.as_ref().is_some_and(|c| subtree.contains(c)) {
            continue;
        }
        if owned_by(node.reaction_id.as_deref(), attached) || !inside(node.x, node.y) {
            continue;
        }
        let r = node.r;
        b.push(Bounds4 { x1: node.x - r, y1: node.y - r, x2: node.x + r, y2: node.y + r });
    }
    for (id, rxn) in &map.g.reactions {
        if !reaction_in_subtree(rxn, &cell.id, subtree) {
            continue;
        }
        if attached.contains(id) || !inside(rxn.x, rxn.y) {
            continue;
        }
        b.push(Bounds4 { x1: rxn.x - 18.0, y1: rxn.y - 18.0, x2: rxn.x + 18.0, y2: rxn.y + 18.0 });
    }
    union_bounds(&b)
}

fn contain_required(rect: Rect, required: Bounds4, parent: &Rect, parent_pad: f64, current: &Rect) -> Rect {
    let pad = PAD;
    let parent_left = parent.x + parent_pad;
    let parent_top = parent.y + parent_pad;
    let parent_right = parent.x + parent.w - parent_pad;
    let parent_bottom = parent.y + parent.h - parent_pad;

    // Content already nearer the boundary than the margin stops it where it
    // is and does not push it out: a side the user is not pulling stays put.
    let left = js::max(required.x1 - pad, current.x);
    let top = js::max(required.y1 - pad, current.y);
    let right = js::min(required.x2 + pad, current.x + current.w);
    let bottom = js::min(required.y2 + pad, current.y + current.h);

    let mut x1 = js::min(rect.x, left);
    let mut y1 = js::min(rect.y, top);
    let mut x2 = js::max(rect.x + rect.w, right);
    let mut y2 = js::max(rect.y + rect.h, bottom);

    x1 = js::max(parent_left, x1);
    y1 = js::max(parent_top, y1);
    x2 = js::min(parent_right, x2);
    y2 = js::min(parent_bottom, y2);

    if x2 - x1 < MIN_W {
        x2 = js::min(parent_right, x1 + MIN_W);
    }
    if y2 - y1 < MIN_H {
        y2 = js::min(parent_bottom, y1 + MIN_H);
    }
    // Existing descendants are already valid inside the parent, so these
    // final corrections can always preserve them while staying within the
    // parent.
    if x1 > left {
        x1 = js::max(parent_left, left);
    }
    if y1 > top {
        y1 = js::max(parent_top, top);
    }
    if x2 < right {
        x2 = js::min(parent_right, right);
    }
    if y2 < bottom {
        y2 = js::min(parent_bottom, bottom);
    }
    Rect::new(x1, y1, x2 - x1, y2 - y1)
}

/// `followBoundary(map, reactionIds, from, to)`: bring the reactions attached
/// to a resized boundary, with their own metabolites, along with it (see
/// `follow_side`).
fn follow_boundary(map: &mut EditableMap, reaction_ids: &IndexSet<String>, from: &Rect, to: &Rect) {
    let mut affected: IndexSet<String> = IndexSet::new();
    let mut transforms: HashMap<String, Transform> = HashMap::new();
    let mut moved: IndexMap<String, Point> = IndexMap::new();
    for id in reaction_ids {
        let Some(rxn) = map.g.reactions.get(id) else { continue };
        let next = follow_side(from, to, Point::new(rxn.x, rxn.y));
        let dx = next.x - rxn.x;
        let dy = next.y - rxn.y;
        if dx.abs() + dy.abs() < 1e-9 {
            continue;
        }
        let transform = translation(dx, dy);
        transform_reaction(map, id, transform);
        transforms.insert(id.clone(), transform);
        affected.insert(id.clone());
        for node in map.g.nodes.values_mut() {
            if node.reaction_id.as_deref() != Some(id.as_str()) {
                continue;
            }
            node.x += dx;
            node.y += dy;
            moved.insert(node.id.clone(), Point::new(dx, dy));
        }
    }
    let moved_nodes: IndexSet<String> = moved.keys().cloned().collect();
    transform_segments_by_reaction(map, &affected, &transforms, Some(&moved_nodes));
    carry_handles(map, &affected, |id| moved.get(id).copied());
    for id in &moved_nodes {
        update_connected_segments(map, id, None);
    }
}

/// `carryHandles(map, affectedReactions, deltaOf)`: a metabolite that moved
/// takes its b1 along on the curves of the reactions that did not move as
/// well: a metabolite one reaction owns can be drawn into the next one too,
/// whose curve would otherwise keep its old bend.
fn carry_handles(map: &mut EditableMap, affected: &IndexSet<String>, delta_of: impl Fn(&str) -> Option<Point>) {
    for seg in &mut map.g.segments {
        let Some(b1) = seg.b1 else { continue };
        let Some(node_id) = seg.node_id.as_deref().filter(|n| !n.is_empty()) else { continue };
        if affected.contains(&seg.reaction_id) {
            continue;
        }
        if let Some(d) = delta_of(node_id) {
            seg.b1 = Some(Point::new(b1.x + d.x, b1.y + d.y));
        }
    }
}

/// The side of `rect` nearest to `point`: `left`, `right`, `top` or
/// `bottom`.
fn nearest_side(rect: &Rect, point: Point) -> &'static str {
    let mut d = [
        ("left", (point.x - rect.x).abs()),
        ("right", (point.x - (rect.x + rect.w)).abs()),
        ("top", (point.y - rect.y).abs()),
        ("bottom", (point.y - (rect.y + rect.h)).abs()),
    ];
    // Array#sort is stable, and a NaN comparison counts as a tie: ties keep
    // the order above.
    d.sort_by(|a, b| {
        let x = a.1 - b.1;
        if x < 0.0 {
            std::cmp::Ordering::Less
        } else if x > 0.0 {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    });
    d[0].0
}

/// `followSide(from, to, point)`: where a point on a resized rect's nearest
/// side goes: across, by as far as that side moved, so it stays on the
/// membrane; along it, nowhere, unless the side has become too short for it.
/// Spreading it along in proportion to the new length would slide every
/// transporter away from the metabolites drawn against it.
fn follow_side(from: &Rect, to: &Rect, point: Point) -> Point {
    let side = nearest_side(from, point);
    if side == "left" || side == "right" {
        let dx = if side == "left" { to.x - from.x } else { (to.x + to.w) - (from.x + from.w) };
        return Point::new(point.x + dx, keep_along(point.y, from.y, from.h, to.y, to.h));
    }
    let dy = if side == "top" { to.y - from.y } else { (to.y + to.h) - (from.y + from.h) };
    Point::new(keep_along(point.x, from.x, from.w, to.x, to.w), point.y + dy)
}

/// `keepAlong(v, oldStart, oldLength, start, length)`: a coordinate along a
/// side that is now `[start, start + length]`: unchanged while the side still
/// reaches it with `SIDE_MARGIN` to spare, else as near as that allows. A
/// point already nearer a corner than the margin keeps the distance it had.
fn keep_along(v: f64, old_start: f64, old_length: f64, start: f64, length: f64) -> f64 {
    let lead = js::min(SIDE_MARGIN, js::max(0.0, v - old_start));
    let trail = js::min(SIDE_MARGIN, js::max(0.0, old_start + old_length - v));
    js::min(js::max(v, start + lead), start + length - trail)
}

fn transform_segments(map: &mut EditableMap, affected: &IndexSet<String>, transform: Transform, moved_nodes: Option<&IndexSet<String>>) {
    let transforms: HashMap<String, Transform> = affected.iter().map(|id| (id.clone(), transform)).collect();
    transform_segments_by_reaction(map, affected, &transforms, moved_nodes);
}

/// `moved_nodes`, when given, are the metabolites that moved with the
/// reactions. A branch's b1 belongs to its metabolite and b2 to its reaction,
/// as in Escher and as `reaction_edit` moves them: a reaction that moves away
/// from a metabolite that stays takes b2 with it and leaves b1. Taking b1 as
/// well would bend every curve to a metabolite left behind into a loop as
/// long as the move.
fn transform_segments_by_reaction(
    map: &mut EditableMap,
    affected: &IndexSet<String>,
    transforms: &HashMap<String, Transform>,
    moved_nodes: Option<&IndexSet<String>>,
) {
    for i in 0..map.g.segments.len() {
        let rid = map.g.segments[i].reaction_id.clone();
        if !affected.contains(&rid) {
            continue;
        }
        let (Some(rxn), Some(&t)) = (map.g.reactions.get(&rid), transforms.get(&rid)) else { continue };
        let has_axis = rxn.axis.is_some();
        let seg = &map.g.segments[i];
        if seg.kind == SegmentKind::Axis && has_axis {
            map.alias_seg(i, SegEnd::From, AxisEnd::A1);
            map.alias_seg(i, SegEnd::To, AxisEnd::A2);
        } else if seg.kind == SegmentKind::TransportLane {
            let lane = rxn
                .transport_lanes
                .as_ref()
                .and_then(|l| l.iter().find(|l| Some(l.base.as_str()) == seg.transport_base.as_deref()))
                .map(|l| (l.inside_point, l.outside_point));
            let nan = Point::new(f64::NAN, f64::NAN);
            match lane {
                Some((inside, outside)) => {
                    map.set_seg(i, SegEnd::From, inside.unwrap_or(nan));
                    map.set_seg(i, SegEnd::To, outside.unwrap_or(nan));
                }
                None => {
                    let (f, to) = (seg.from, seg.to);
                    map.set_seg(i, SegEnd::From, transform_point(f, t));
                    map.set_seg(i, SegEnd::To, transform_point(to, t));
                }
            }
        } else if seg.kind == SegmentKind::Boundary {
            let (f, to) = (seg.from, seg.to);
            map.set_seg(i, SegEnd::From, transform_point(f, t));
            map.set_seg(i, SegEnd::To, transform_point(to, t));
        } else if seg.transport_lane {
            if seg.is_substrate() {
                let to = seg.to;
                map.set_seg(i, SegEnd::To, transform_point(to, t));
            } else if seg.is_product() {
                let f = seg.from;
                map.set_seg(i, SegEnd::From, transform_point(f, t));
            }
        } else if has_axis {
            if seg.is_substrate() {
                map.alias_seg(i, SegEnd::To, AxisEnd::A1);
            } else if seg.is_product() {
                map.alias_seg(i, SegEnd::From, AxisEnd::A2);
            }
        }

        let seg = &mut map.g.segments[i];
        if let Some(c) = seg.extra.get("control").filter(|v| crate::util::truthy(Some(v))) {
            let q = point_from_value(c).unwrap_or(Point::new(f64::NAN, f64::NAN));
            seg.extra.insert("control".into(), point_value(transform_point(q, t)));
        }
        let metabolite_moved =
            moved_nodes.is_none_or(|m| seg.node_id.as_deref().filter(|n| !n.is_empty()).is_none_or(|n| m.contains(n)));
        if let Some(b1) = seg.b1.filter(|_| metabolite_moved) {
            seg.b1 = Some(transform_point(b1, t));
        }
        if let Some(b2) = seg.b2 {
            seg.b2 = Some(transform_point(b2, t));
        }
    }
}

/// `transformReaction(rxn, t)`: the marker, its axis (new point objects) and
/// its transport lanes.
fn transform_reaction(map: &mut EditableMap, rid: &str, t: Transform) {
    let rxn = &mut map.g.reactions[rid];
    let p = transform_point(Point::new(rxn.x, rxn.y), t);
    rxn.x = p.x;
    rxn.y = p.y;
    if let Some(axis) = rxn.axis {
        let a1 = transform_point(axis.a1, t);
        let a2 = transform_point(axis.a2, t);
        map.replace_axis_point(rid, AxisEnd::A1, a1);
        map.replace_axis_point(rid, AxisEnd::A2, a2);
        let dx = a2.x - a1.x;
        let dy = a2.y - a1.y;
        let len = crate::util::num_or(js::hypot(dx, dy), 1.0);
        map.g.reactions[rid].axis.as_mut().expect("axis").dir = Point::new(dx / len, dy / len);
    }
    if let Some(lanes) = map.g.reactions[rid].transport_lanes.as_mut() {
        let nan = Point::new(f64::NAN, f64::NAN);
        for lane in lanes {
            lane.inside_point = Some(transform_point(lane.inside_point.unwrap_or(nan), t));
            lane.outside_point = Some(transform_point(lane.outside_point.unwrap_or(nan), t));
        }
    }
}

fn boundary_belongs(boundary_id: Option<&str>, cell_id: &str, subtree: &HashSet<String>) -> bool {
    let Some(b) = boundary_id.filter(|b| !b.is_empty()) else { return false };
    let prefix = format!("{cell_id}/");
    match b.strip_prefix(&prefix) {
        Some(rest) => subtree.contains(rest),
        None => false,
    }
}
