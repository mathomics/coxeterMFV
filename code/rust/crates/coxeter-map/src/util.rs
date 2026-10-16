//! Small helpers shared by the modules of this crate: blank map records and
//! the JavaScript idioms (`a || b`, `Number(x)`) the ported code leans on.

use coxeter_core::mapdata::{MapReaction, Node, NodeKind, Point, Rect, Segment, SegmentKind};
use serde_json::{Map, Value};

/// A node with every optional field unset.
pub fn blank_node(id: &str, kind: NodeKind) -> Node {
    Node {
        id: id.to_string(),
        kind,
        merged_id: None,
        local_id: None,
        base: None,
        name: None,
        formula: None,
        compartment: None,
        visual_compartment: None,
        cell_id: None,
        reaction_id: None,
        x: 0.0,
        y: 0.0,
        r: 0.0,
        primary: None,
        visual_role: None,
        escher_secondary: None,
        degree: None,
        reaction_local_for: None,
        reaction_local: None,
        shared_instance: None,
        label_angle: None,
        hidden: None,
        duplicate_of: None,
        manual_duplicate: None,
        extra: Map::new(),
    }
}

/// A reaction record with every optional field unset.
pub fn blank_reaction(id: &str) -> MapReaction {
    MapReaction {
        id: id.to_string(),
        local_id: None,
        name: String::new(),
        subsystem: None,
        cell_id: None,
        kind: String::new(),
        compartments: Vec::new(),
        membrane: None,
        on_membrane: false,
        normal: None,
        x: 0.0,
        y: 0.0,
        equation: None,
        bounds: None,
        boundary: None,
        axis: None,
        membrane_visual_id: None,
        boundary_id: None,
        placement_direction: None,
        axis_direction_hint: None,
        terminal_metabolite_id: None,
        transport_mode: None,
        transport_lanes: None,
        line_ends: None,
        linear: None,
        reversible: None,
        sbml_reversible: None,
        genes: None,
        extra: Map::new(),
    }
}

/// A segment with every optional field unset.
pub fn blank_segment(reaction_id: &str, kind: SegmentKind, from: Point, to: Point, arrow: bool) -> Segment {
    Segment {
        id: None,
        reaction_id: reaction_id.to_string(),
        kind,
        from,
        to,
        arrow,
        node_id: None,
        coefficient: None,
        side: None,
        backbone_anchor: false,
        b1: None,
        b2: None,
        transport_base: None,
        transport_lane: false,
        transport_direction: None,
        extra: Map::new(),
    }
}

/// `x || fallback` for a number: 0 and NaN are falsy.
pub fn num_or(x: f64, fallback: f64) -> f64 {
    if x == 0.0 || x.is_nan() { fallback } else { x }
}

/// `Number(v)` of an optional number: `undefined` is NaN.
pub fn num(v: Option<f64>) -> f64 {
    v.unwrap_or(f64::NAN)
}

/// `Math.min(Math.max(v, lo), hi)`.
pub fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    coxeter_core::js::min(coxeter_core::js::max(v, lo), hi)
}

pub fn rect_of(x: f64, y: f64, w: f64, h: f64) -> Rect {
    Rect::new(x, y, w, h)
}

/// `{ x: a.x + (b.x - a.x) * t, … }`.
pub fn lerp(a: Point, b: Point, t: f64) -> Point {
    Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
}

/// `{ x: p.x + dx, y: p.y + dy }`.
pub fn shifted(p: Point, dx: f64, dy: f64) -> Point {
    Point::new(p.x + dx, p.y + dy)
}

/// A JSON number, written as JavaScript would (an integer where it is one).
pub fn json_num(x: f64) -> Value {
    let mut v = serde_json::json!(x);
    coxeter_core::js::normalize_json_numbers(&mut v);
    v
}

/// The `{x, y}` JSON of a point.
pub fn point_value(p: Point) -> Value {
    serde_json::json!({ "x": p.x, "y": p.y })
}

/// Read `{x, y}` out of a JSON value, if it has two numbers.
pub fn point_from_value(v: &Value) -> Option<Point> {
    let x = v.get("x")?.as_f64()?;
    let y = v.get("y")?.as_f64()?;
    Some(Point::new(x, y))
}

/// `true` for a JSON value JavaScript would call truthy.
pub fn truthy(v: Option<&Value>) -> bool {
    v.is_some_and(coxeter_core::js::truthy)
}
