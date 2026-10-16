//! Manual editing of reaction geometry (port of `src/layout/reaction-edit.js`).
//!
//! Escher-style branch geometry uses a cubic Bézier per metabolite connection:
//!
//! ```text
//!   metabolite P0 -- b1 (metabolite-side) -- b2 (reaction-side) -- P3 backbone
//! ```
//!
//! `b1` follows the metabolite when that metabolite moves. `b2` and the
//! anchor follow the reaction skeleton when the reaction moves. The map shows
//! b1 and b2 as handles. The anchors are not offered one by one, because
//! pulling a single branch off the backbone bends a reaction out of shape.
//! The backbone is turned as a whole instead ([`rotate_reaction`]), which
//! takes every anchor with it.
//!
//! The same module holds the operations that rearrange one reaction as a
//! drawing: keeping it straight between two metabolites, exchanging the
//! places of two of its metabolites, and mirroring it. All of them are
//! geometry only.
//!
//! Where the JavaScript moves an axis point *in place* (`translateReaction`),
//! every segment end that is that same object moves too; the map's segment
//! links ([`EditableMap::segment_links`]) say which those are.

use std::collections::HashSet;
use std::f64::consts::{FRAC_1_SQRT_2, PI};

use coxeter_core::geometry::{closest_perimeter_point, closest_perimeter_point_corners, compartment_corners};
use coxeter_core::js;
use coxeter_core::mapdata::{AxisEnd, Linear, LineEnds, MapReaction, NodeKind, Point, Segment, SegmentKind};
use serde_json::{Map, Value};

use crate::drag::{Moving, Placeable, clamp_node_position, node_fits_at, update_connected_segments};
use crate::editable_map::{Boundary, EditableMap, SegEnd};
use crate::util::{lerp, point_from_value, shifted};

const EPS: f64 = 1e-9;

/// How close to either end a straight reaction's marker may be slid.
const LINEAR_T_MIN: f64 = 0.12;
const LINEAR_T_MAX: f64 = 0.88;

/// Where a reaction's own cofactor nodes stand by default, measured from its
/// centre: out along the backbone and off to one side of it. The first pair
/// on a side stands close in beside the stem, one on each hand of it; further
/// ones fan out from them, wider and nearer the marker. On the end that
/// points down the page the name of the metabolite the backbone runs to lies
/// between it and the marker, so the cofactors keep nearer the marker there.
const SATELLITE_ALONG: f64 = 34.0;
const SATELLITE_ALONG_LEAN: f64 = 6.0;
const SATELLITE_ACROSS: f64 = 26.0;
const SATELLITE_ALONG_STEP: f64 = 12.0;
const SATELLITE_ACROSS_STEP: f64 = 22.0;
const SATELLITE_ALONG_MIN: f64 = 16.0;

/// The corner a branch to an off-axis metabolite turns: how far along the
/// stem its reaction-side handle lies, as a share of the way out to the
/// metabolite's level, and how far in from the metabolite its other handle.
const BRANCH_TURN_ALONG: f64 = 0.8;
const BRANCH_TURN_ACROSS: f64 = 0.55;

/// The least a metabolite is set from the marker when a reaction is aligned.
const ALIGN_ARM_MIN: f64 = 56.0;

/// The eight ways a reaction can be laid from its substrate to its product.
pub fn compass(direction: &str) -> Option<Point> {
    const D: f64 = FRAC_1_SQRT_2;
    Some(match direction {
        "N" => Point::new(0.0, -1.0),
        "NE" => Point::new(D, -D),
        "E" => Point::new(1.0, 0.0),
        "SE" => Point::new(D, D),
        "S" => Point::new(0.0, 1.0),
        "SW" => Point::new(-D, D),
        "W" => Point::new(-1.0, 0.0),
        "NW" => Point::new(-D, -D),
        _ => return None,
    })
}

/// The compass directions, in the JavaScript's order.
pub const COMPASS_DIRECTIONS: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];

// Small helpers

/// A branch of `reaction_id`: a segment to one of its metabolites, not its
/// axis, a boundary stub or a transport lane.
pub(crate) fn is_branch(seg: &Segment, reaction_id: &str) -> bool {
    seg.reaction_id == reaction_id
        && seg.node_id.as_deref().is_some_and(|n| !n.is_empty())
        && !matches!(seg.kind, SegmentKind::Axis | SegmentKind::Boundary | SegmentKind::TransportLane)
}

/// The metabolite end of a branch: `from` for a substrate, `to` for a product.
pub(crate) fn metabolite_endpoint(seg: &Segment) -> Option<Point> {
    if seg.is_substrate() {
        Some(seg.from)
    } else if seg.is_product() {
        Some(seg.to)
    } else {
        None
    }
}

/// The reaction end of a branch: `to` for a substrate, `from` for a product.
pub(crate) fn reaction_endpoint(seg: &Segment) -> Option<Point> {
    if seg.is_substrate() {
        Some(seg.to)
    } else if seg.is_product() {
        Some(seg.from)
    } else {
        None
    }
}

fn branches(map: &EditableMap, reaction_id: &str) -> Vec<usize> {
    (0..map.g.segments.len()).filter(|&i| is_branch(&map.g.segments[i], reaction_id)).collect()
}

fn branch_segment(map: &EditableMap, reaction_id: &str, node_id: &str) -> Option<usize> {
    (0..map.g.segments.len())
        .find(|&i| is_branch(&map.g.segments[i], reaction_id) && map.g.segments[i].node_id.as_deref() == Some(node_id))
}

fn delete_control(seg: &mut Segment) {
    seg.extra.shift_remove("control");
}

/// The reaction-side end of a branch: `to` for a substrate, `from` otherwise.
fn reaction_end_key(seg: &Segment) -> SegEnd {
    if seg.is_substrate() { SegEnd::To } else { SegEnd::From }
}

fn set_reaction_end(map: &mut EditableMap, i: usize, p: Point) {
    let key = if map.g.segments[i].is_substrate() { SegEnd::To } else { SegEnd::From };
    map.set_seg(i, key, p);
}

/// `ensureCubicControls(seg)`: convert legacy quadratic control geometry to an
/// equivalent cubic Bézier, or give a branch with no handles a neutral cubic
/// along the connection. For a quadratic P0-Q-P3: C1 = P0 + 2/3(Q-P0),
/// C2 = P3 + 2/3(Q-P3).
pub fn ensure_cubic_controls(seg: &mut Segment) {
    if seg.b1.is_some() && seg.b2.is_some() {
        return;
    }
    let (Some(p0), Some(p3)) = (metabolite_endpoint(seg), reaction_endpoint(seg)) else { return };
    if let Some(q) = seg.extra.get("control").filter(|v| crate::util::truthy(Some(v))) {
        let q = point_from_value(q).unwrap_or(Point::new(f64::NAN, f64::NAN));
        let k = 2.0 / 3.0;
        seg.b1 = Some(Point::new(p0.x + k * (q.x - p0.x), p0.y + k * (q.y - p0.y)));
        seg.b2 = Some(Point::new(p3.x + k * (q.x - p3.x), p3.y + k * (q.y - p3.y)));
        return;
    }
    // New/default branch: smooth, neutral cubic along the connection.
    seg.b1 = Some(lerp(p0, p3, 0.34));
    seg.b2 = Some(lerp(p0, p3, 0.72));
}

/// `shapeBranch(seg, rxn)`: give one branch its base curve.
///
/// At the reaction it runs along the backbone, as part of the stem every
/// branch of that end shares. For the metabolite the backbone runs straight
/// to, that makes a straight line. For one standing off to the side (a
/// cofactor, or a second product), the stem is followed out to about the
/// metabolite's level and the curve then turns square across to it: a C, the
/// rounded corner a hand-drawn map gives a cofactor.
pub fn shape_branch(seg: &mut Segment, rxn: &MapReaction) {
    let (p0, p3) = (metabolite_endpoint(seg), reaction_endpoint(seg));
    let (Some(p0), Some(p3), Some(axis)) = (p0, p3, rxn.axis) else {
        ensure_cubic_controls(seg);
        return;
    };
    let dir = axis.dir;
    // Which way along the backbone this branch leaves it: the way its end of
    // the backbone lies from the centre, or failing that the way its
    // metabolite does.
    let mut outward = (p3.x - rxn.x) * dir.x + (p3.y - rxn.y) * dir.y;
    if outward.abs() < 1e-6 {
        outward = (p0.x - rxn.x) * dir.x + (p0.y - rxn.y) * dir.y;
    }
    if outward.abs() < 1e-6 {
        outward = if seg.is_substrate() { -1.0 } else { 1.0 };
    }
    let u = if outward < 0.0 { Point::new(-dir.x, -dir.y) } else { Point::new(dir.x, dir.y) };

    let vx = p0.x - p3.x;
    let vy = p0.y - p3.y;
    let along = vx * u.x + vy * u.y;
    let side = -vx * u.y + vy * u.x; // signed: which hand of the stem
    let across = side.abs();
    if across < 4.0 && along > 0.0 {
        // Dead ahead: handles in order on the line itself.
        seg.b1 = Some(lerp(p0, p3, 0.34));
        seg.b2 = Some(lerp(p0, p3, 0.72));
    } else {
        // Unit vector from the stem out to the metabolite's hand of it.
        let hand = if side < 0.0 { -1.0 } else { 1.0 };
        let n = Point::new(-u.y * hand, u.x * hand);
        // A metabolite beside or behind its end of the backbone still gets a
        // curve that sets out along the stem before it comes round.
        let reach = js::max_n([along, 0.5 * across, 14.0]);
        seg.b1 = Some(Point::new(p0.x - n.x * across * BRANCH_TURN_ACROSS, p0.y - n.y * across * BRANCH_TURN_ACROSS));
        seg.b2 = Some(Point::new(p3.x + u.x * reach * BRANCH_TURN_ALONG, p3.y + u.y * reach * BRANCH_TURN_ALONG));
    }
    delete_control(seg);
}

/// `satellitePlace(rxn, side, index)`: where the `index`-th cofactor of one
/// side of a reaction stands by default. `side` is -1 for the substrate end of
/// the backbone, +1 for the product end.
pub fn satellite_place(rxn: &MapReaction, side: f64, index: usize) -> Point {
    let dir = rxn.axis.map(|a| a.dir).unwrap_or(Point::new(f64::NAN, f64::NAN));
    let row = (index / 2) as f64;
    let hand = if index % 2 == 0 { 1.0 } else { -1.0 };
    // dir.y * side is how far down the page this end of the backbone points.
    let first = SATELLITE_ALONG - SATELLITE_ALONG_LEAN * dir.y * side;
    let along = js::max(SATELLITE_ALONG_MIN, first - row * SATELLITE_ALONG_STEP) * side;
    let across = (SATELLITE_ACROSS + row * SATELLITE_ACROSS_STEP) * hand;
    Point::new(rxn.x + dir.x * along - dir.y * across, rxn.y + dir.y * along + dir.x * across)
}

fn axis_half(rxn: &MapReaction) -> f64 {
    match rxn.axis {
        None => 18.0,
        Some(a) => js::max(1.0, js::hypot(a.a2.x - a.a1.x, a.a2.y - a.a1.y) / 2.0),
    }
}

/// `placeNode`: put a node somewhere, held inside its compartment, and bring
/// along the branches of every reaction drawn to it, the same as a drag to
/// that point.
fn place_node(map: &mut EditableMap, node_id: &str, point: Point) {
    let Some(node) = map.g.nodes.get(node_id) else { return };
    let before = node.pos();
    let p = clamp_node_position(map, Placeable::from(node), point.x, point.y);
    let node = map.g.nodes.get_mut(node_id).expect("present");
    node.x = p.x;
    node.y = p.y;
    update_connected_segments(map, node_id, Some((p.x - before.x, p.y - before.y)));
}

/// `ensureReactionBackbone`: give every branch of an internal reaction its
/// own attachment point along the backbone (Escher-style), once.
fn ensure_reaction_backbone(map: &mut EditableMap, reaction_id: &str) {
    let Some(rxn) = map.g.reactions.get(reaction_id) else { return };
    if rxn.axis.is_none() || rxn.on_membrane {
        return;
    }
    let rxn = rxn.clone();
    let all = branches(map, reaction_id);
    let subs: Vec<usize> = all.iter().copied().filter(|&i| map.g.segments[i].is_substrate()).collect();
    let prods: Vec<usize> = all.iter().copied().filter(|&i| map.g.segments[i].is_product()).collect();
    let dir = rxn.axis.expect("checked").dir;
    let half = axis_half(&rxn);
    for (list, side) in [(subs, -1.0), (prods, 1.0)] {
        let count = list.len();
        for (k, &i) in list.iter().enumerate() {
            ensure_cubic_controls(&mut map.g.segments[i]);
            if map.g.segments[i].backbone_anchor {
                continue;
            }
            let old = reaction_endpoint(&map.g.segments[i]).expect("a side");
            let t = if count == 1 { 0.72 } else { 0.28 + 0.58 * (k as f64 / (count - 1) as f64) };
            let d = half * t * side;
            let anchor = Point::new(rxn.x + dir.x * d, rxn.y + dir.y * d);
            let (dx, dy) = (anchor.x - old.x, anchor.y - old.y);
            set_reaction_end(map, i, anchor);
            let seg = &mut map.g.segments[i];
            if let Some(b2) = seg.b2 {
                seg.b2 = Some(shifted(b2, dx, dy));
            }
            seg.backbone_anchor = true;
            delete_control(seg);
        }
    }
}

// Moving

/// `canMoveReaction`.
pub fn can_move_reaction(map: &EditableMap, reaction_id: &str) -> bool {
    map.g.reactions.contains_key(reaction_id)
}

/// `moveReaction(layout, id, x, y)`: move a reaction's marker. A transporter
/// slides along its membrane; a straight reaction slides along its line;
/// anything else moves with its skeleton, held inside its compartment.
pub fn move_reaction(map: &mut EditableMap, reaction_id: &str, x: f64, y: f64) -> bool {
    if !map.g.reactions.contains_key(reaction_id) || !x.is_finite() || !y.is_finite() {
        return false;
    }
    ensure_reaction_backbone(map, reaction_id);
    let rxn = &map.g.reactions[reaction_id];
    if rxn.on_membrane && rxn.boundary_id.as_deref().is_some_and(|b| !b.is_empty()) {
        return move_membrane_reaction(map, reaction_id, x, y);
    }
    if linear_ends(map, reaction_id).is_some() {
        return slide_linear_reaction(map, reaction_id, x, y);
    }
    move_internal_reaction(map, reaction_id, x, y);
    true
}

fn internal_compartment(map: &EditableMap, rxn: &MapReaction) -> Option<String> {
    let cell = map.g.cells.iter().find(|c| Some(&c.id) == rxn.cell_id.as_ref())?;
    let available: HashSet<&str> = cell.compartments.iter().map(|c| c.id.as_str()).collect();
    rxn.compartments
        .iter()
        .find(|id| available.contains(id.as_str()))
        .cloned()
        .or_else(|| cell.compartments.iter().find(|c| c.depth == 0.0).map(|c| c.id.clone()))
}

/// The kind the centre proxy of an internal reaction stands as.
static PROXY_KIND: NodeKind = NodeKind::Metabolite;

/// `reactionCentreFits(layout, reactionId, x, y, moving)`: could the centre
/// of `reaction_id` move to `(x, y)` without being pulled back? An internal
/// reaction's centre is kept where a small metabolite of its compartment could
/// stand. A transporter keeps to its membrane and a straight reaction to its
/// two metabolites, so neither can refuse a place.
pub fn reaction_centre_fits(map: &EditableMap, reaction_id: &str, x: f64, y: f64, moving: Option<&Moving>) -> bool {
    let Some(rxn) = map.g.reactions.get(reaction_id) else { return true };
    if (rxn.on_membrane && rxn.boundary_id.as_deref().is_some_and(|b| !b.is_empty())) || linear_ends(map, reaction_id).is_some() {
        return true;
    }
    let Some(compartment) = internal_compartment(map, rxn) else { return true };
    let proxy = Placeable { kind: &PROXY_KIND, r: 10.0, cell_id: rxn.cell_id.as_deref(), compartment: Some(&compartment), x: rxn.x, y: rxn.y };
    node_fits_at(map, proxy, x, y, moving)
}

fn move_internal_reaction(map: &mut EditableMap, reaction_id: &str, x: f64, y: f64) {
    let rxn = map.g.reactions[reaction_id].clone();
    let compartment = internal_compartment(map, &rxn);
    let p = match &compartment {
        Some(c) => {
            let fake = Placeable { kind: &PROXY_KIND, r: 10.0, cell_id: rxn.cell_id.as_deref(), compartment: Some(c), x: rxn.x, y: rxn.y };
            clamp_node_position(map, fake, x, y)
        }
        None => Point::new(x, y),
    };
    let dx = p.x - rxn.x;
    let dy = p.y - rxn.y;
    if dx.abs() + dy.abs() < EPS {
        return;
    }
    translate_reaction(map, reaction_id, dx, dy);
}

/// `closestBoundaryPoint(layout, boundary, point)`.
pub fn closest_boundary_point(map: &EditableMap, boundary: &Boundary, point: Point) -> coxeter_core::geometry::PerimeterPoint {
    let compartment = map
        .g
        .cells
        .iter()
        .find(|c| Some(&c.id) == boundary.cell_id.as_ref())
        .and_then(|c| c.compartments.iter().find(|k| k.id == boundary.inside));
    let rect = boundary.rect.unwrap_or_default();
    match compartment {
        Some(c) => closest_perimeter_point_corners(&rect, point, compartment_corners(c)),
        None => closest_perimeter_point(&rect, point, 0.0),
    }
}

fn move_membrane_reaction(map: &mut EditableMap, reaction_id: &str, x: f64, y: f64) -> bool {
    let rxn = map.g.reactions[reaction_id].clone();
    let Some(boundary) = rxn.boundary_id.as_deref().and_then(|b| map.boundary(b)).filter(|b| b.rect.is_some()).cloned()
    else {
        return false;
    };
    let p = closest_boundary_point(map, &boundary, Point::new(x, y));
    let old_centre = Point::new(rxn.x, rxn.y);
    let old_normal = rxn.normal.unwrap_or(Point::new(1.0, 0.0));
    let new_normal = Point::new(p.nx, p.ny);
    let dot = old_normal.x * new_normal.x + old_normal.y * new_normal.y;
    let cross = old_normal.x * new_normal.y - old_normal.y * new_normal.x;
    let tp = |q: Point| {
        let dx = q.x - old_centre.x;
        let dy = q.y - old_centre.y;
        Point::new(p.x + dx * dot - dy * cross, p.y + dx * cross + dy * dot)
    };
    let tv = |v: Point| Point::new(v.x * dot - v.y * cross, v.x * cross + v.y * dot);

    let mut moved_currency = HashSet::new();
    for node in map.g.nodes.values_mut() {
        if node.reaction_id.as_deref() != Some(reaction_id) {
            continue;
        }
        let q = tp(node.pos());
        node.x = q.x;
        node.y = q.y;
        moved_currency.insert(node.id.clone());
    }

    if let Some(axis) = rxn.axis {
        map.replace_axis_point(reaction_id, AxisEnd::A1, tp(axis.a1));
        map.replace_axis_point(reaction_id, AxisEnd::A2, tp(axis.a2));
        map.g.reactions[reaction_id].axis.as_mut().expect("axis").dir = tv(axis.dir);
    }
    transform_lanes(map, reaction_id, &tp);
    transform_reaction_segments(map, reaction_id, &tp, &moved_currency, true);

    let r = &mut map.g.reactions[reaction_id];
    r.x = p.x;
    r.y = p.y;
    r.normal = Some(new_normal);
    true
}

/// Move every transport lane point of a reaction through `tp`.
fn transform_lanes(map: &mut EditableMap, reaction_id: &str, tp: &dyn Fn(Point) -> Point) {
    if let Some(lanes) = map.g.reactions[reaction_id].transport_lanes.as_mut() {
        for lane in lanes {
            lane.inside_point = Some(tp(lane.inside_point.unwrap_or(Point::new(f64::NAN, f64::NAN))));
            lane.outside_point = Some(tp(lane.outside_point.unwrap_or(Point::new(f64::NAN, f64::NAN))));
        }
    }
}

/// The lane of a reaction a `transport-lane` segment draws.
fn lane_points(map: &EditableMap, reaction_id: &str, base: Option<&str>) -> Option<(Point, Point)> {
    let lane = map.g.reactions.get(reaction_id)?.transport_lanes.as_ref()?.iter().find(|l| Some(l.base.as_str()) == base)?;
    Some((lane.inside_point.unwrap_or(Point::new(f64::NAN, f64::NAN)), lane.outside_point.unwrap_or(Point::new(f64::NAN, f64::NAN))))
}

/// The segment pass shared by `moveMembraneReaction` (lanes aware) and
/// `transformSkeleton`.
fn transform_reaction_segments(
    map: &mut EditableMap,
    reaction_id: &str,
    tp: &dyn Fn(Point) -> Point,
    carried: &HashSet<String>,
    lanes_aware: bool,
) {
    let has_axis = map.g.reactions[reaction_id].axis.is_some();
    for i in 0..map.g.segments.len() {
        if map.g.segments[i].reaction_id != reaction_id {
            continue;
        }
        match map.g.segments[i].kind {
            SegmentKind::Axis => {
                if has_axis {
                    map.alias_seg(i, SegEnd::From, AxisEnd::A1);
                    map.alias_seg(i, SegEnd::To, AxisEnd::A2);
                } else {
                    let (f, t) = (map.g.segments[i].from, map.g.segments[i].to);
                    map.set_seg(i, SegEnd::From, tp(f));
                    map.set_seg(i, SegEnd::To, tp(t));
                }
            }
            SegmentKind::TransportLane if lanes_aware => {
                let base = map.g.segments[i].transport_base.clone();
                match lane_points(map, reaction_id, base.as_deref()) {
                    Some((inside, outside)) => {
                        map.set_seg(i, SegEnd::From, inside);
                        map.set_seg(i, SegEnd::To, outside);
                    }
                    None => {
                        let (f, t) = (map.g.segments[i].from, map.g.segments[i].to);
                        map.set_seg(i, SegEnd::From, tp(f));
                        map.set_seg(i, SegEnd::To, tp(t));
                    }
                }
            }
            SegmentKind::Boundary | SegmentKind::TransportLane => {
                let (f, t) = (map.g.segments[i].from, map.g.segments[i].to);
                map.set_seg(i, SegEnd::From, tp(f));
                map.set_seg(i, SegEnd::To, tp(t));
            }
            _ => {
                ensure_cubic_controls(&mut map.g.segments[i]);
                let seg = &map.g.segments[i];
                if seg.is_substrate() {
                    let t = seg.to;
                    map.set_seg(i, SegEnd::To, tp(t));
                } else if seg.is_product() {
                    let f = seg.from;
                    map.set_seg(i, SegEnd::From, tp(f));
                }
                if let Some(b2) = map.g.segments[i].b2 {
                    map.g.segments[i].b2 = Some(tp(b2));
                }
                // Reaction-owned currency nodes move with the skeleton, so b1
                // does too.
                let node_id = map.g.segments[i].node_id.clone().unwrap_or_default();
                if carried.contains(&node_id) {
                    let at = map.g.nodes[&node_id].pos();
                    let seg = &map.g.segments[i];
                    if seg.is_substrate() {
                        map.set_seg(i, SegEnd::From, at);
                    } else if seg.is_product() {
                        map.set_seg(i, SegEnd::To, at);
                    }
                    if let Some(b1) = map.g.segments[i].b1 {
                        map.g.segments[i].b1 = Some(tp(b1));
                    }
                }
                delete_control(&mut map.g.segments[i]);
            }
        }
    }
}

/// `translateReaction`: the axis points move *in place*, and with them every
/// segment end that is the same object; each segment is then shifted as the
/// JavaScript shifts it.
fn translate_reaction(map: &mut EditableMap, reaction_id: &str, dx: f64, dy: f64) {
    {
        let r = &mut map.g.reactions[reaction_id];
        r.x += dx;
        r.y += dy;
    }
    let has_axis = map.g.reactions[reaction_id].axis.is_some();
    if has_axis {
        map.shift_axis_in_place(reaction_id, dx, dy);
    }
    let mut moved_currency = HashSet::new();
    for node in map.g.nodes.values_mut() {
        if node.reaction_id.as_deref() == Some(reaction_id) {
            node.x += dx;
            node.y += dy;
            moved_currency.insert(node.id.clone());
        }
    }
    for i in 0..map.g.segments.len() {
        if map.g.segments[i].reaction_id != reaction_id {
            continue;
        }
        match map.g.segments[i].kind {
            SegmentKind::Axis => {
                if has_axis {
                    map.alias_seg(i, SegEnd::From, AxisEnd::A1);
                    map.alias_seg(i, SegEnd::To, AxisEnd::A2);
                } else {
                    let (f, t) = (map.g.segments[i].from, map.g.segments[i].to);
                    map.set_seg(i, SegEnd::From, shifted(f, dx, dy));
                    map.set_seg(i, SegEnd::To, shifted(t, dx, dy));
                }
            }
            SegmentKind::Boundary | SegmentKind::TransportLane => {
                let (f, t) = (map.g.segments[i].from, map.g.segments[i].to);
                map.set_seg(i, SegEnd::From, shifted(f, dx, dy));
                map.set_seg(i, SegEnd::To, shifted(t, dx, dy));
            }
            _ => {
                ensure_cubic_controls(&mut map.g.segments[i]);
                // Preserve the individual backbone attachment point for each branch.
                let seg = &map.g.segments[i];
                if seg.is_substrate() {
                    let t = seg.to;
                    map.set_seg(i, SegEnd::To, shifted(t, dx, dy));
                } else if seg.is_product() {
                    let f = seg.from;
                    map.set_seg(i, SegEnd::From, shifted(f, dx, dy));
                }
                if let Some(b2) = map.g.segments[i].b2 {
                    map.g.segments[i].b2 = Some(shifted(b2, dx, dy));
                }
                let node_id = map.g.segments[i].node_id.clone().unwrap_or_default();
                if moved_currency.contains(&node_id) {
                    let at = map.g.nodes[&node_id].pos();
                    let seg = &map.g.segments[i];
                    if seg.is_substrate() {
                        map.set_seg(i, SegEnd::From, at);
                    } else if seg.is_product() {
                        map.set_seg(i, SegEnd::To, at);
                    }
                    if let Some(b1) = map.g.segments[i].b1 {
                        map.g.segments[i].b1 = Some(shifted(b1, dx, dy));
                    }
                }
                delete_control(&mut map.g.segments[i]);
            }
        }
    }
}

// Rotation

/// `reactionAngle`: direction of the backbone, in radians, as drawn.
pub fn reaction_angle(map: &EditableMap, reaction_id: &str) -> Option<f64> {
    let dir = map.g.reactions.get(reaction_id)?.axis?.dir;
    Some(js::atan2(dir.y, dir.x))
}

/// `canRotateReaction`: a transporter points across the membrane it sits on,
/// and a straight reaction along the line it is kept on: neither has a
/// direction to choose.
pub fn can_rotate_reaction(map: &EditableMap, reaction_id: &str) -> bool {
    let Some(rxn) = map.g.reactions.get(reaction_id) else { return false };
    rxn.axis.is_some() && !rxn.on_membrane && linear_ends(map, reaction_id).is_none()
}

/// `rotateReaction(layout, id, angle)`: turn the backbone of an internal
/// reaction about its centre (radians; positive is clockwise on screen).
///
/// Everything that belongs to the reaction itself turns with it: the axis,
/// the points where the branches leave it, their reaction-side handles and
/// the small cofactor nodes the reaction owns. Metabolites that stand on
/// their own stay where they are, and their branches are redrawn to the
/// turned backbone.
pub fn rotate_reaction(map: &mut EditableMap, reaction_id: &str, angle: f64) -> bool {
    if !angle.is_finite() || !can_rotate_reaction(map, reaction_id) {
        return false;
    }
    ensure_reaction_backbone(map, reaction_id);
    if angle.abs() < EPS {
        return true;
    }
    let cos = js::cos(angle);
    let sin = js::sin(angle);
    let rxn = &map.g.reactions[reaction_id];
    let centre = Point::new(rxn.x, rxn.y);
    transform_skeleton(
        map,
        reaction_id,
        &|p| rotate_about(p, centre, centre, cos, sin),
        &|v| Point::new(v.x * cos - v.y * sin, v.x * sin + v.y * cos),
        &HashSet::new(),
    );
    true
}

/// `setReactionAngle`: turn the backbone so that it points along `angle`.
pub fn set_reaction_angle(map: &mut EditableMap, reaction_id: &str, angle: f64) -> bool {
    let Some(current) = reaction_angle(map, reaction_id) else { return false };
    if !angle.is_finite() {
        return false;
    }
    rotate_reaction(map, reaction_id, normalise_angle(angle - current))
}

/// Turn `p` about `from` and set it down relative to `to`.
fn rotate_about(p: Point, from: Point, to: Point, cos: f64, sin: f64) -> Point {
    let dx = p.x - from.x;
    let dy = p.y - from.y;
    Point::new(to.x + dx * cos - dy * sin, to.y + dx * sin + dy * cos)
}

fn normalise_angle(angle: f64) -> f64 {
    let mut a = angle % (2.0 * PI);
    if a > PI {
        a -= 2.0 * PI;
    }
    if a < -PI {
        a += 2.0 * PI;
    }
    a
}

/// `transformSkeleton`: move everything that belongs to an internal reaction
/// itself through one rigid transform: its axis, the points its branches
/// leave it at with their reaction-side handles, and the cofactor nodes it
/// owns (except those in `keep`). The centre is left to the caller.
fn transform_skeleton(
    map: &mut EditableMap,
    reaction_id: &str,
    point: &dyn Fn(Point) -> Point,
    vector: &dyn Fn(Point) -> Point,
    keep: &HashSet<String>,
) {
    if let Some(axis) = map.g.reactions[reaction_id].axis {
        map.replace_axis_point(reaction_id, AxisEnd::A1, point(axis.a1));
        map.replace_axis_point(reaction_id, AxisEnd::A2, point(axis.a2));
        map.g.reactions[reaction_id].axis.as_mut().expect("axis").dir = vector(axis.dir);
    }
    if let Some(b) = map.g.reactions[reaction_id].boundary {
        map.g.reactions[reaction_id].boundary = Some(point(b));
    }
    let mut carried = HashSet::new();
    for node in map.g.nodes.values_mut() {
        if node.reaction_id.as_deref() != Some(reaction_id) || keep.contains(&node.id) {
            continue;
        }
        let q = point(node.pos());
        node.x = q.x;
        node.y = q.y;
        carried.insert(node.id.clone());
    }
    transform_reaction_segments(map, reaction_id, point, &carried, false);
}

// Straight reactions

/// The two end nodes of a straight reaction, or None if it is not (or no
/// longer) one.
fn linear_ends(map: &EditableMap, reaction_id: &str) -> Option<(String, String)> {
    let rxn = map.g.reactions.get(reaction_id)?;
    let linear = rxn.linear.as_ref()?;
    if rxn.on_membrane || rxn.axis.is_none() {
        return None;
    }
    let from = linear.from.as_deref().filter(|id| map.g.nodes.contains_key(*id))?;
    let to = linear.to.as_deref().filter(|id| map.g.nodes.contains_key(*id))?;
    Some((from.to_string(), to.to_string()))
}

/// `pickLinearEnd`: which metabolite a straight reaction runs to on one side.
///
/// A preference is honoured if it is on that side. Otherwise the metabolite
/// the reaction is about: one that stands on its own before a cofactor the
/// reaction carries with it, a primary before a secondary, and of those the
/// one already closest to the line of the backbone.
fn pick_linear_end(map: &EditableMap, reaction_id: &str, side: &str, preferred: Option<&str>) -> Option<String> {
    let rxn = map.g.reactions.get(reaction_id)?;
    let list: Vec<&Segment> =
        map.g.segments.iter().filter(|s| is_branch(s, reaction_id) && s.side.as_deref() == Some(side)).collect();
    let remembered = rxn.line_ends.as_ref().and_then(|e| if side == "substrate" { e.from.as_deref() } else { e.to.as_deref() });
    for wanted in [preferred, remembered].into_iter().flatten() {
        if !wanted.is_empty() && list.iter().any(|s| s.node_id.as_deref() == Some(wanted)) {
            return Some(wanted.to_string());
        }
    }
    let dir = rxn.axis?.dir;
    let mut best: Option<(String, f64, f64)> = None;
    for seg in list {
        let Some(node) = seg.node_id.as_deref().and_then(|id| map.g.nodes.get(id)) else { continue };
        if node.hidden == Some(true) {
            continue;
        }
        let off = ((node.x - rxn.x) * -dir.y + (node.y - rxn.y) * dir.x).abs();
        let rank = (if node.kind == NodeKind::Currency { 2.0 } else { 0.0 }) + (if node.primary == Some(false) { 1.0 } else { 0.0 });
        let better = match &best {
            None => true,
            Some((_, br, bo)) => rank < *br || (rank == *br && off < *bo),
        };
        if better {
            best = Some((node.id.clone(), rank, off));
        }
    }
    best.map(|b| b.0)
}

/// `setReactionLinear(layout, id, on, ends)`: keep a reaction straight
/// between two of its metabolites, or stop doing so.
///
/// A straight reaction has no shape of its own to keep in order: its marker
/// lies on the line from one metabolite to the other, turned along it, and
/// both branches are drawn as that line. Moving either metabolite takes the
/// marker with it. The two ends are remembered on the reaction
/// (`linear.from`, `linear.to`) so they survive saving.
pub fn set_reaction_linear(map: &mut EditableMap, reaction_id: &str, on: bool, from: Option<&str>, to: Option<&str>) -> bool {
    let Some(rxn) = map.g.reactions.get_mut(reaction_id) else { return false };
    if !on {
        rxn.linear = None;
        return true;
    }
    if rxn.on_membrane || rxn.axis.is_none() {
        return false;
    }
    ensure_reaction_backbone(map, reaction_id);
    let linear = map.g.reactions[reaction_id].linear.clone();
    let pref_from = from.map(str::to_string).or_else(|| linear.as_ref().and_then(|l| l.from.clone()));
    let pref_to = to.map(str::to_string).or_else(|| linear.as_ref().and_then(|l| l.to.clone()));
    let f = pick_linear_end(map, reaction_id, "substrate", pref_from.as_deref());
    let t = pick_linear_end(map, reaction_id, "product", pref_to.as_deref());
    let (Some(f), Some(t)) = (f, t) else { return false };
    if f == t {
        return false;
    }
    map.g.reactions[reaction_id].linear = Some(Linear { from: Some(f), to: Some(t), t: Some(0.5), extra: Map::new() });
    straighten_reaction(map, reaction_id);
    true
}

/// `reactionLineEnds`: the two metabolites a reaction's line runs through, or
/// would run through if it were made straight now.
pub fn reaction_line_ends(map: &EditableMap, reaction_id: &str) -> (Option<String>, Option<String>) {
    let Some(rxn) = map.g.reactions.get(reaction_id) else { return (None, None) };
    if rxn.axis.is_none() {
        return (None, None);
    }
    let lf = rxn.linear.as_ref().and_then(|l| l.from.clone());
    let lt = rxn.linear.as_ref().and_then(|l| l.to.clone());
    (
        pick_linear_end(map, reaction_id, "substrate", lf.as_deref()),
        pick_linear_end(map, reaction_id, "product", lt.as_deref()),
    )
}

/// `setReactionLineEnd`: choose one of the two metabolites a reaction's line
/// runs through. A reaction kept straight is straightened through it at
/// once; one that is not remembers the choice for when it is.
pub fn set_reaction_line_end(map: &mut EditableMap, reaction_id: &str, node_id: &str) -> bool {
    let Some(i) = branch_segment(map, reaction_id, node_id) else { return false };
    let Some(rxn) = map.g.reactions.get_mut(reaction_id) else { return false };
    if rxn.axis.is_none() || rxn.on_membrane {
        return false;
    }
    let substrate = map.g.segments[i].is_substrate();
    let ends = rxn.line_ends.get_or_insert_with(|| LineEnds { from: None, to: None, extra: Map::new() });
    if substrate {
        ends.from = Some(node_id.to_string());
    } else {
        ends.to = Some(node_id.to_string());
    }
    if let Some(linear) = rxn.linear.as_mut() {
        if substrate {
            linear.from = Some(node_id.to_string());
        } else {
            linear.to = Some(node_id.to_string());
        }
        straighten_reaction(map, reaction_id);
    }
    true
}

/// `alignReaction(layout, id, direction, ends)`: lay a reaction out along one
/// of the eight compass directions, substrate to product, through its marker.
///
/// The marker stays where it is. The two metabolites its line runs through
/// are set on either side of it along that direction, each as far from it as
/// it was, and the reaction is kept straight between them from then on.
pub fn align_reaction(map: &mut EditableMap, reaction_id: &str, direction: &str, from: Option<&str>, to: Option<&str>) -> bool {
    let Some(way) = compass(direction) else { return false };
    let Some(rxn) = map.g.reactions.get(reaction_id) else { return false };
    if rxn.axis.is_none() || rxn.on_membrane {
        return false;
    }
    ensure_reaction_backbone(map, reaction_id);
    let linear = map.g.reactions[reaction_id].linear.clone();
    let pf = from.map(str::to_string).or_else(|| linear.as_ref().and_then(|l| l.from.clone()));
    let pt = to.map(str::to_string).or_else(|| linear.as_ref().and_then(|l| l.to.clone()));
    let from_id = pick_linear_end(map, reaction_id, "substrate", pf.as_deref());
    let to_id = pick_linear_end(map, reaction_id, "product", pt.as_deref());
    let (Some(from_id), Some(to_id)) = (from_id, to_id) else { return false };
    if from_id == to_id {
        return false;
    }
    let rxn = &map.g.reactions[reaction_id];
    let centre = Point::new(rxn.x, rxn.y);
    let arm = |id: &str| {
        let n = &map.g.nodes[id];
        js::max(ALIGN_ARM_MIN, js::hypot(n.x - centre.x, n.y - centre.y))
    };
    let before = arm(&from_id);
    let after = arm(&to_id);
    place_node(map, &from_id, Point::new(centre.x - way.x * before, centre.y - way.y * before));
    place_node(map, &to_id, Point::new(centre.x + way.x * after, centre.y + way.y * after));

    keep_straight_through(map, reaction_id, &from_id, &to_id, Some(centre));
    // Other straight reactions drawn to either metabolite follow it.
    straighten_linear_reactions(map, Some(&[from_id, to_id]));
    true
}

/// `keepStraightThrough`: keep a reaction straight between two of its
/// metabolites from now on, with its marker where it is along that line
/// (`at`, if it was measured before its metabolites were moved), and put the
/// rest of its drawing in order around it.
pub fn keep_straight_through(map: &mut EditableMap, reaction_id: &str, from_id: &str, to_id: &str, at: Option<Point>) -> bool {
    let Some(rxn) = map.g.reactions.get(reaction_id) else { return false };
    let (Some(from), Some(to)) = (map.g.nodes.get(from_id), map.g.nodes.get(to_id)) else { return false };
    if rxn.axis.is_none() || rxn.on_membrane || from_id == to_id {
        return false;
    }
    let marker = at.unwrap_or(Point::new(rxn.x, rxn.y));
    // Where the marker is between the two, so that straightening leaves it there.
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let squared = dx * dx + dy * dy;
    let t = if squared > 1e-9 { ((marker.x - from.x) * dx + (marker.y - from.y) * dy) / squared } else { 0.5 };
    let rxn = &mut map.g.reactions[reaction_id];
    rxn.linear = Some(Linear { from: Some(from_id.into()), to: Some(to_id.into()), t: Some(t), extra: Map::new() });
    rxn.line_ends = Some(LineEnds { from: Some(from_id.into()), to: Some(to_id.into()), extra: Map::new() });
    straighten_reaction(map, reaction_id);
    tidy_reaction(map, reaction_id);
    true
}

/// `tidyReaction`: put a reaction's drawing back in its base shape, where it
/// stands. The cofactor nodes the reaction owns go to their default places
/// around the backbone, the branches are attached along the backbone in
/// order again, and every curve is given its base form. Metabolites that
/// stand on their own are not moved: only the curves to them are redrawn.
pub fn tidy_reaction(map: &mut EditableMap, reaction_id: &str) -> bool {
    match map.g.reactions.get(reaction_id) {
        Some(r) if r.axis.is_some() => {}
        _ => return false,
    }
    ensure_reaction_backbone(map, reaction_id);
    let all = branches(map, reaction_id);
    let rxn = map.g.reactions[reaction_id].clone();

    if !rxn.on_membrane {
        let dir = rxn.axis.expect("checked").dir;
        let half = axis_half(&rxn);
        let linear_from = rxn.linear.as_ref().and_then(|l| l.from.clone());
        let linear_to = rxn.linear.as_ref().and_then(|l| l.to.clone());
        for (side, sign) in [("substrate", -1.0), ("product", 1.0)] {
            let list: Vec<usize> = all.iter().copied().filter(|&i| map.g.segments[i].side.as_deref() == Some(side)).collect();
            let mut cofactor = 0;
            let len = list.len();
            for (k, &i) in list.iter().enumerate() {
                let t = if len == 1 { 0.72 } else { 0.28 + 0.58 * (k as f64 / (len - 1) as f64) };
                let anchor = Point::new(rxn.x + dir.x * half * t * sign, rxn.y + dir.y * half * t * sign);
                let end = if side == "substrate" { SegEnd::To } else { SegEnd::From };
                map.set_seg(i, end, anchor);
                map.g.segments[i].backbone_anchor = true;

                // Its own cofactors, but not one the reaction's line has been run to.
                let node_id = map.g.segments[i].node_id.clone().unwrap_or_default();
                let owned = map.g.nodes.get(&node_id).is_some_and(|n| n.reaction_id.as_deref() == Some(reaction_id));
                if owned && Some(&node_id) != linear_from.as_ref() && Some(&node_id) != linear_to.as_ref() {
                    let place = satellite_place(&rxn, sign, cofactor);
                    place_node(map, &node_id, place);
                    cofactor += 1;
                }
            }
        }
    }

    for i in all {
        let node_id = map.g.segments[i].node_id.clone().unwrap_or_default();
        if let Some(at) = map.g.nodes.get(&node_id).map(|n| n.pos()) {
            let end = if map.g.segments[i].is_substrate() { SegEnd::From } else { SegEnd::To };
            map.set_seg(i, end, at);
        }
        let rxn = &map.g.reactions[reaction_id];
        shape_branch(&mut map.g.segments[i], rxn);
    }
    if map.g.reactions[reaction_id].linear.is_some() {
        straighten_reaction(map, reaction_id);
    }
    true
}

/// `canLinearizeReaction`: is there a metabolite on each side to keep it
/// straight between?
pub fn can_linearize_reaction(map: &EditableMap, reaction_id: &str) -> bool {
    let Some(rxn) = map.g.reactions.get(reaction_id) else { return false };
    if rxn.axis.is_none() || rxn.on_membrane {
        return false;
    }
    let from = pick_linear_end(map, reaction_id, "substrate", None);
    let to = pick_linear_end(map, reaction_id, "product", None);
    matches!((from, to), (Some(f), Some(t)) if f != t)
}

/// `straightenReaction`: put a straight reaction back on its line. Called
/// after anything that may have moved one of its ends. A reaction that is not
/// straight is left alone (and `false` comes back).
pub fn straighten_reaction(map: &mut EditableMap, reaction_id: &str) -> bool {
    let Some((from_id, to_id)) = linear_ends(map, reaction_id) else { return false };
    ensure_reaction_backbone(map, reaction_id);
    let from = map.g.nodes[&from_id].pos();
    let to = map.g.nodes[&to_id].pos();
    let rxn = &map.g.reactions[reaction_id];
    let t = js::min(LINEAR_T_MAX, js::max(LINEAR_T_MIN, rxn.linear.as_ref().and_then(|l| l.t).unwrap_or(0.5)));
    let target = lerp(from, to, t);
    let length = js::hypot(to.x - from.x, to.y - from.y);
    let old = rxn.axis.expect("linear ends need an axis").dir;
    // Two metabolites on top of each other give no direction; keep the old one.
    let dir = if length > 1e-6 { Point::new((to.x - from.x) / length, (to.y - from.y) / length) } else { old };
    let cos = old.x * dir.x + old.y * dir.y;
    let sin = old.x * dir.y - old.y * dir.x;
    let centre = Point::new(rxn.x, rxn.y);

    let keep: HashSet<String> = [from_id.clone(), to_id.clone()].into_iter().collect();
    transform_skeleton(map, reaction_id, &|p| rotate_about(p, centre, target, cos, sin), &|_| dir, &keep);
    {
        let r = &mut map.g.reactions[reaction_id];
        r.x = target.x;
        r.y = target.y;
    }

    // The two branches that carry the line are the line: handles on it, and
    // the point they leave the backbone at pulled back onto it.
    for i in 0..map.g.segments.len() {
        let seg = &map.g.segments[i];
        if !is_branch(seg, reaction_id) {
            continue;
        }
        let node_id = seg.node_id.as_deref().unwrap_or("");
        if node_id != from_id && node_id != to_id {
            continue;
        }
        let node = if node_id == from_id { from } else { to };
        let anchor = reaction_endpoint(seg).unwrap_or(Point::new(f64::NAN, f64::NAN));
        let along = (anchor.x - target.x) * dir.x + (anchor.y - target.y) * dir.y;
        let on_axis = Point::new(target.x + dir.x * along, target.y + dir.y * along);
        let at = node;
        if seg.is_substrate() {
            map.set_seg(i, SegEnd::From, at);
            map.set_seg(i, SegEnd::To, on_axis);
        } else {
            map.set_seg(i, SegEnd::From, on_axis);
            map.set_seg(i, SegEnd::To, at);
        }
        let seg = &mut map.g.segments[i];
        seg.b1 = Some(lerp(at, on_axis, 1.0 / 3.0));
        seg.b2 = Some(lerp(at, on_axis, 2.0 / 3.0));
        delete_control(seg);
    }
    true
}

/// `straightenLinearReactions`: straighten every straight reaction that ends
/// at one of these nodes, or all of them when `None`. The ids done.
pub fn straighten_linear_reactions(map: &mut EditableMap, node_ids: Option<&[String]>) -> Vec<String> {
    let wanted: Option<HashSet<&str>> = node_ids.map(|ids| ids.iter().map(String::as_str).collect());
    let ids: Vec<String> = map.g.reactions.keys().cloned().collect();
    let mut done = Vec::new();
    for id in ids {
        let Some(linear) = map.g.reactions.get(&id).and_then(|r| r.linear.as_ref()) else { continue };
        if let Some(w) = &wanted {
            let hit = linear.from.as_deref().is_some_and(|f| w.contains(f)) || linear.to.as_deref().is_some_and(|t| w.contains(t));
            if !hit {
                continue;
            }
        }
        if straighten_reaction(map, &id) {
            done.push(id);
        }
    }
    done
}

/// `replaceLinearEnd`: a node took over from another one (a merge): keep
/// straight reactions attached.
pub fn replace_linear_end(map: &mut EditableMap, old_id: &str, new_id: &str) {
    for rxn in map.g.reactions.values_mut() {
        if let Some(l) = rxn.linear.as_mut() {
            if l.from.as_deref() == Some(old_id) {
                l.from = Some(new_id.into());
            }
            if l.to.as_deref() == Some(old_id) {
                l.to = Some(new_id.into());
            }
        }
    }
    for rxn in map.g.reactions.values_mut() {
        if let Some(e) = rxn.line_ends.as_mut() {
            if e.from.as_deref() == Some(old_id) {
                e.from = Some(new_id.into());
            }
            if e.to.as_deref() == Some(old_id) {
                e.to = Some(new_id.into());
            }
        }
    }
}

/// Dragging a straight reaction's marker slides it along its line.
fn slide_linear_reaction(map: &mut EditableMap, reaction_id: &str, x: f64, y: f64) -> bool {
    let Some((from_id, to_id)) = linear_ends(map, reaction_id) else { return false };
    let from = map.g.nodes[&from_id].pos();
    let to = map.g.nodes[&to_id].pos();
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let squared = dx * dx + dy * dy;
    if squared > 1e-9 {
        let t = ((x - from.x) * dx + (y - from.y) * dy) / squared;
        if let Some(l) = map.g.reactions[reaction_id].linear.as_mut() {
            l.t = Some(js::min(LINEAR_T_MAX, js::max(LINEAR_T_MIN, t)));
        }
    }
    straighten_reaction(map, reaction_id)
}

// Exchanging and mirroring

/// One metabolite drawn on a side of a reaction (`reactionSides`).
#[derive(Debug, Clone, PartialEq)]
pub struct SideEntry {
    pub node_id: String,
    pub label: String,
    /// What it may trade places with: two metabolites exchange only within
    /// the same compartment of the same cell, so nothing is ever moved
    /// across a membrane.
    pub group: String,
}

/// `reactionSides`: the metabolites drawn on each side of a reaction, in a
/// stable order.
pub fn reaction_sides(map: &EditableMap, reaction_id: &str) -> (Vec<SideEntry>, Vec<SideEntry>) {
    let mut subs = Vec::new();
    let mut prods = Vec::new();
    if !map.g.reactions.contains_key(reaction_id) {
        return (subs, prods);
    }
    for seg in &map.g.segments {
        if !is_branch(seg, reaction_id) {
            continue;
        }
        let Some(node) = seg.node_id.as_deref().and_then(|id| map.g.nodes.get(id)) else { continue };
        let entry = SideEntry {
            node_id: node.id.clone(),
            label: node.base.clone().or_else(|| node.local_id.clone()).or_else(|| node.merged_id.clone()).unwrap_or_else(|| node.id.clone()),
            group: format!(
                "{}/{}",
                if node.kind == NodeKind::Medium { "*" } else { node.cell_id.as_deref().unwrap_or("*") },
                node.compartment.as_deref().unwrap_or("")
            ),
        };
        if seg.is_substrate() { subs.push(entry) } else { prods.push(entry) }
    }
    (subs, prods)
}

/// `swapReactionMetabolites`: exchange the places of two metabolites on the
/// same side of a reaction. Each takes the other's place and the other's
/// curve; other reactions drawn to either node follow it, as they would if it
/// had been dragged there.
pub fn swap_reaction_metabolites(map: &mut EditableMap, reaction_id: &str, a_id: &str, b_id: &str) -> bool {
    if !map.g.reactions.contains_key(reaction_id) || a_id.is_empty() || b_id.is_empty() || a_id == b_id {
        return false;
    }
    ensure_reaction_backbone(map, reaction_id);
    let (Some(sa), Some(sb)) = (branch_segment(map, reaction_id, a_id), branch_segment(map, reaction_id, b_id)) else {
        return false;
    };
    let (Some(a), Some(b)) = (map.g.nodes.get(a_id), map.g.nodes.get(b_id)) else { return false };
    if map.g.segments[sa].side != map.g.segments[sb].side {
        return false;
    }
    // Never across a membrane: the two must share a compartment.
    if (a.kind == NodeKind::Medium) != (b.kind == NodeKind::Medium) {
        return false;
    }
    if a.kind != NodeKind::Medium && (a.cell_id != b.cell_id || a.compartment != b.compartment) {
        return false;
    }
    ensure_cubic_controls(&mut map.g.segments[sa]);
    ensure_cubic_controls(&mut map.g.segments[sb]);
    let curve = |seg: &Segment| (seg.b1, seg.b2, reaction_endpoint(seg));
    let curve_a = curve(&map.g.segments[sa]);
    let curve_b = curve(&map.g.segments[sb]);
    let place_a = map.g.nodes[a_id].pos();
    let place_b = map.g.nodes[b_id].pos();

    place_node(map, a_id, place_b);
    place_node(map, b_id, place_a);

    // A branch that ends on a transporter lane belongs to that lane, so only
    // the metabolite's own end of the curve changes hands there.
    let whole = !map.g.segments[sa].transport_lane && !map.g.segments[sb].transport_lane;
    let take = |map: &mut EditableMap, i: usize, node_id: &str, from: (Option<Point>, Option<Point>, Option<Point>)| {
        let at = map.g.nodes[node_id].pos();
        let substrate = map.g.segments[i].is_substrate();
        let (meta_end, rxn_end) = if substrate { (SegEnd::From, SegEnd::To) } else { (SegEnd::To, SegEnd::From) };
        map.set_seg(i, meta_end, at);
        if whole {
            map.set_seg(i, rxn_end, from.2.unwrap_or(Point::new(f64::NAN, f64::NAN)));
        }
        let seg = &mut map.g.segments[i];
        seg.b1 = from.0;
        if whole {
            seg.b2 = from.1;
        }
        delete_control(seg);
    };
    take(map, sa, a_id, curve_b);
    take(map, sb, b_id, curve_a);

    // The metabolite now standing where the line ran to is the one it runs to.
    let trade = |id: &Option<String>| -> Option<String> {
        id.as_ref().map(|v| if v == a_id { b_id.to_string() } else if v == b_id { a_id.to_string() } else { v.clone() })
    };
    let rxn = &mut map.g.reactions[reaction_id];
    if let Some(l) = rxn.linear.as_mut() {
        l.from = trade(&l.from);
        l.to = trade(&l.to);
    }
    if let Some(e) = rxn.line_ends.as_ref() {
        rxn.line_ends = Some(LineEnds { from: trade(&e.from), to: trade(&e.to), extra: Map::new() });
    }
    straighten_linear_reactions(map, Some(&[a_id.to_string(), b_id.to_string()]));
    true
}

/// `flipReaction`: exchange the substrate side and the product side.
///
/// Everything the reaction is drawn with is reflected in the line through
/// its centre, across the backbone: substrates end up where the products
/// were and the backbone points the other way, while what was on the left
/// stays on the left. The metabolites move, shared ones included, and other
/// reactions drawn to them follow.
pub fn flip_reaction(map: &mut EditableMap, reaction_id: &str) -> bool {
    let Some(rxn) = map.g.reactions.get(reaction_id) else { return false };
    // A transporter's two sides are the two sides of a membrane.
    if rxn.axis.is_none() || rxn.on_membrane {
        return false;
    }
    ensure_reaction_backbone(map, reaction_id);
    let dir = map.g.reactions[reaction_id].axis.expect("checked").dir;
    reflect_reaction(map, reaction_id, dir, true, true);
    true
}

/// `mirrorReaction`: mirror a reaction in its own backbone: what hung off to
/// one side hangs off to the other. Metabolites other reactions are drawn to
/// keep their place.
pub fn mirror_reaction(map: &mut EditableMap, reaction_id: &str) -> bool {
    match map.g.reactions.get(reaction_id) {
        Some(r) if r.axis.is_some() => {}
        _ => return false,
    }
    ensure_reaction_backbone(map, reaction_id);
    let dir = map.g.reactions[reaction_id].axis.expect("checked").dir;
    reflect_reaction(map, reaction_id, Point::new(-dir.y, dir.x), false, false);
    true
}

fn negate_point_value(v: &Value) -> Value {
    let x = v.get("x").map(js::to_number).unwrap_or(f64::NAN);
    let y = v.get("y").map(js::to_number).unwrap_or(f64::NAN);
    crate::util::point_value(Point::new(-x, -y))
}

/// Reflect a reaction's drawing in the line through its centre with the given
/// normal. `every_node`: move metabolites other reactions use too.
/// `reverse`: the backbone ends trade places.
fn reflect_reaction(map: &mut EditableMap, reaction_id: &str, normal: Point, every_node: bool, reverse: bool) {
    let rxn = map.g.reactions[reaction_id].clone();
    let centre = Point::new(rxn.x, rxn.y);
    let reflect = |p: Point| {
        let d = (p.x - centre.x) * normal.x + (p.y - centre.y) * normal.y;
        Point::new(p.x - 2.0 * d * normal.x, p.y - 2.0 * d * normal.y)
    };

    let all = branches(map, reaction_id);
    for &i in &all {
        ensure_cubic_controls(&mut map.g.segments[i]);
    }

    // Where each metabolite ends up, and by how much its compartment held it back.
    let mut moved: indexmap::IndexMap<String, (f64, f64, Point)> = indexmap::IndexMap::new();
    for &i in &all {
        let node_id = map.g.segments[i].node_id.clone().unwrap_or_default();
        let Some(node) = map.g.nodes.get(&node_id) else { continue };
        if moved.contains_key(&node_id) {
            continue;
        }
        let exclusive =
            map.g.segments.iter().all(|o| o.node_id.as_deref() != Some(node_id.as_str()) || o.reaction_id == reaction_id);
        if !every_node && !exclusive {
            continue;
        }
        let ideal = reflect(node.pos());
        let before = node.pos();
        place_node(map, &node_id, ideal);
        let node = &map.g.nodes[&node_id];
        moved.insert(node_id.clone(), (node.x - ideal.x, node.y - ideal.y, before));
    }

    if reverse {
        let axis = map.g.reactions[reaction_id].axis.expect("checked");
        map.replace_axis_point(reaction_id, AxisEnd::A1, reflect(axis.a1));
        map.replace_axis_point(reaction_id, AxisEnd::A2, reflect(axis.a2));
        let r = &mut map.g.reactions[reaction_id];
        r.axis.as_mut().expect("axis").dir = Point::new(-axis.dir.x, -axis.dir.y);
        if let Some(h) = r.axis_direction_hint {
            r.axis_direction_hint = Some(Point::new(-h.x, -h.y));
        }
        if let Some(pd) = r.placement_direction.as_ref().filter(|v| js::truthy(v)) {
            r.placement_direction = Some(negate_point_value(pd));
        }
    }
    {
        let r = &mut map.g.reactions[reaction_id];
        if let Some(b) = r.boundary {
            r.boundary = Some(reflect(b));
        }
        for lane in r.transport_lanes.iter_mut().flatten() {
            lane.inside_point = Some(reflect(lane.inside_point.unwrap_or(Point::new(f64::NAN, f64::NAN))));
            lane.outside_point = Some(reflect(lane.outside_point.unwrap_or(Point::new(f64::NAN, f64::NAN))));
            if lane.offset.is_finite() {
                lane.offset = -lane.offset;
            }
        }
    }

    for i in 0..map.g.segments.len() {
        if map.g.segments[i].reaction_id != reaction_id {
            continue;
        }
        match map.g.segments[i].kind {
            SegmentKind::Axis => {
                map.alias_seg(i, SegEnd::From, AxisEnd::A1);
                map.alias_seg(i, SegEnd::To, AxisEnd::A2);
            }
            SegmentKind::TransportLane => {
                let base = map.g.segments[i].transport_base.clone();
                let (f, t) = match lane_points(map, reaction_id, base.as_deref()) {
                    Some(points) => points,
                    None => (reflect(map.g.segments[i].from), reflect(map.g.segments[i].to)),
                };
                map.set_seg(i, SegEnd::From, f);
                map.set_seg(i, SegEnd::To, t);
            }
            SegmentKind::Boundary => {
                let (f, t) = (map.g.segments[i].from, map.g.segments[i].to);
                map.set_seg(i, SegEnd::From, reflect(f));
                map.set_seg(i, SegEnd::To, reflect(t));
            }
            _ if is_branch(&map.g.segments[i], reaction_id) => {
                let node_id = map.g.segments[i].node_id.clone().unwrap_or_default();
                let node = map.g.nodes.get(&node_id).map(|n| n.pos());
                let shift = moved.get(&node_id).copied();
                let seg = &map.g.segments[i];
                let anchor = reflect(reaction_endpoint(seg).unwrap_or(Point::new(f64::NAN, f64::NAN)));
                let substrate = seg.is_substrate();
                let (meta_end, rxn_end) = if substrate { (SegEnd::From, SegEnd::To) } else { (SegEnd::To, SegEnd::From) };
                if let Some(at) = node {
                    map.set_seg(i, meta_end, at);
                }
                map.set_seg(i, rxn_end, anchor);
                let seg = &mut map.g.segments[i];
                seg.b2 = Some(reflect(seg.b2.unwrap_or(Point::new(f64::NAN, f64::NAN))));
                // placeNode() already carried b1 along with the node; start
                // again from where it was so the handle is mirrored, not
                // mirrored and moved.
                if let (Some((sx, sy, before)), Some(at)) = (shift, node) {
                    let b1 = seg.b1.unwrap_or(Point::new(f64::NAN, f64::NAN));
                    let original = shifted(b1, before.x - at.x, before.y - at.y);
                    seg.b1 = Some(shifted(reflect(original), sx, sy));
                }
                delete_control(seg);
            }
            _ => {}
        }
    }

    let moved_ids: Vec<String> = moved.keys().cloned().collect();
    straighten_linear_reactions(map, Some(&moved_ids));
    if map.g.reactions[reaction_id].linear.is_some() {
        straighten_reaction(map, reaction_id);
    }
}

// Handles

/// The three Escher-style control points of one branch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BranchControls {
    pub b1: Point,
    pub b2: Point,
    pub anchor: Option<Point>,
}

/// `branchControlPoints(layout, reactionId, nodeId)`.
pub fn branch_control_points(map: &mut EditableMap, reaction_id: &str, node_id: &str) -> Option<BranchControls> {
    ensure_reaction_backbone(map, reaction_id);
    let i = branch_segment(map, reaction_id, node_id)?;
    ensure_cubic_controls(&mut map.g.segments[i]);
    let seg = &map.g.segments[i];
    Some(BranchControls {
        b1: seg.b1.unwrap_or(Point::new(f64::NAN, f64::NAN)),
        b2: seg.b2.unwrap_or(Point::new(f64::NAN, f64::NAN)),
        anchor: reaction_endpoint(seg),
    })
}

/// Which handle of a branch is moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    B1,
    B2,
    Anchor,
}

impl Handle {
    /// The handle named `b1`, `b2` or `anchor`, as the UI names them.
    pub fn parse(s: &str) -> Option<Handle> {
        match s {
            "b1" => Some(Handle::B1),
            "b2" => Some(Handle::B2),
            "anchor" => Some(Handle::Anchor),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Handle::B1 => "b1",
            Handle::B2 => "b2",
            Handle::Anchor => "anchor",
        }
    }
}

/// `moveBranchControl`: move b1, b2, or the branch attachment point on the
/// reaction backbone. Returns where the handle now is.
pub fn move_branch_control(map: &mut EditableMap, reaction_id: &str, node_id: &str, handle: Handle, x: f64, y: f64) -> Option<Point> {
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    ensure_reaction_backbone(map, reaction_id);
    let i = branch_segment(map, reaction_id, node_id)?;
    ensure_cubic_controls(&mut map.g.segments[i]);
    let next = Point::new(x, y);
    match handle {
        Handle::Anchor => {
            let old = reaction_endpoint(&map.g.segments[i])?;
            let (dx, dy) = (x - old.x, y - old.y);
            let end = reaction_end_key(&map.g.segments[i]);
            map.set_seg(i, end, next);
            let seg = &mut map.g.segments[i];
            // Keep the reaction-side Bézier geometry attached to the moved anchor.
            seg.b2 = Some(shifted(seg.b2.unwrap_or(Point::new(f64::NAN, f64::NAN)), dx, dy));
            seg.backbone_anchor = true;
            delete_control(seg);
        }
        Handle::B1 => {
            let seg = &mut map.g.segments[i];
            seg.b1 = Some(next);
            // Once upgraded, the legacy quadratic field must not win during rendering.
            delete_control(seg);
        }
        Handle::B2 => {
            let seg = &mut map.g.segments[i];
            seg.b2 = Some(next);
            delete_control(seg);
        }
    }
    Some(next)
}

/// One draggable handle, as the UI lists them.
#[derive(Debug, Clone, PartialEq)]
pub struct ControlPoint {
    pub reaction_id: String,
    pub node_id: String,
    pub handle: Handle,
    /// `metabolite`, `reaction` or `backbone`.
    pub role: &'static str,
    pub x: f64,
    pub y: f64,
}

/// `reactionControlPoints`: each branch exposes b1, b2 and its backbone
/// anchor. (Upgrades legacy geometry as a side effect, as in JavaScript.)
pub fn reaction_control_points(map: &mut EditableMap, reaction_id: &str) -> Vec<ControlPoint> {
    ensure_reaction_backbone(map, reaction_id);
    let mut out = Vec::new();
    for i in 0..map.g.segments.len() {
        if !is_branch(&map.g.segments[i], reaction_id) {
            continue;
        }
        ensure_cubic_controls(&mut map.g.segments[i]);
        let seg = &map.g.segments[i];
        let node_id = seg.node_id.clone().unwrap_or_default();
        let nan = Point::new(f64::NAN, f64::NAN);
        let b1 = seg.b1.unwrap_or(nan);
        let b2 = seg.b2.unwrap_or(nan);
        out.push(ControlPoint { reaction_id: reaction_id.into(), node_id: node_id.clone(), handle: Handle::B1, role: "metabolite", x: b1.x, y: b1.y });
        out.push(ControlPoint { reaction_id: reaction_id.into(), node_id: node_id.clone(), handle: Handle::B2, role: "reaction", x: b2.x, y: b2.y });
        if let Some(a) = reaction_endpoint(seg) {
            out.push(ControlPoint { reaction_id: reaction_id.into(), node_id, handle: Handle::Anchor, role: "backbone", x: a.x, y: a.y });
        }
    }
    out
}
