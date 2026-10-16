//! Where a reaction's line stops short of its metabolite, and its arrowhead
//! (`src/render/reaction-path.js`).
//!
//! Render-only geometry, oriented from the reaction towards the endpoint.
//! Clipping a Bézier with de Casteljau preserves its edited shape and leaves
//! model coordinates and editable control points untouched.

use coxeter_core::geometry::SIZES;
use coxeter_core::js;
use coxeter_core::mapdata::{Node, NodeKind, Point};

use crate::format::n;

const GAP: f64 = 4.0;

fn mix(a: Point, b: Point, t: f64) -> Point {
    Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
}

fn distance(a: Point, b: Point) -> f64 {
    js::hypot(a.x - b.x, a.y - b.y)
}

/// An arrowhead: its tip, the middle of its base, and the triangle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArrowHead {
    pub tip: Point,
    pub base: Point,
    pub points: [Point; 3],
}

/// The clipped path (`d`, empty when nothing is left to draw) and its head.
#[derive(Debug, Clone, PartialEq)]
pub struct EndpointGeometry {
    pub d: String,
    pub arrow: Option<ArrowHead>,
    /// The line or cubic the path draws, for what is worked out on the
    /// picture itself (line hops). Empty when nothing is drawn.
    pub curve: Vec<Point>,
}

/// The radius a line ending at this node stops short of: the drawn circle of
/// a metabolite (secondary ones are smaller), or the node's own radius.
pub fn endpoint_node_radius(node: &Node) -> f64 {
    match node.kind {
        NodeKind::Metabolite | NodeKind::Medium | NodeKind::Currency => {
            if node.kind == NodeKind::Currency || node.primary == Some(false) {
                SIZES.currency_radius
            } else {
                SIZES.primary_radius
            }
        }
        NodeKind::Other(_) => {
            let r = node.r;
            js::max(0.0, if r == 0.0 || r.is_nan() { 0.0 } else { r })
        }
    }
}

/// `reactionEndpointGeometry(points, { node, arrow, width })`.
///
/// `points` is a line (2 points) or a cubic Bézier (4), from the reaction to
/// the endpoint. `node_radius` is the radius of the node at the end, if one
/// is there (see [`endpoint_node_radius`]).
pub fn reaction_endpoint_geometry(points: &[Point], node_radius: Option<f64>, arrow: bool, width: f64) -> EndpointGeometry {
    let cubic = points.len() == 4;
    let at = |t: f64| -> Point {
        if !cubic {
            return mix(points[0], points[1], t);
        }
        let a = mix(points[0], points[1], t);
        let b = mix(points[1], points[2], t);
        let c = mix(points[2], points[3], t);
        mix(mix(a, b, t), mix(b, c, t), t)
    };
    let endpoint = points[points.len() - 1];
    let inset = match node_radius {
        Some(r) => r + GAP,
        None => {
            if arrow { 2.0 } else { 0.0 }
        }
    };
    let tip_t = if inset > 0.0 { outside_circle(&at, 1.0, endpoint, inset) } else { Some(1.0) };
    let Some(tip_t) = tip_t else {
        return EndpointGeometry { d: String::new(), arrow: None, curve: Vec::new() };
    };
    let tip = at(tip_t);
    let size = js::max(7.0, width * 1.9);
    let mut shaft_t = tip_t;
    let mut head = None;
    if arrow {
        let base_t = outside_circle(&at, tip_t, tip, size * 1.25);
        shaft_t = base_t.unwrap_or(0.0);
        let base = at(shaft_t);
        let length = distance(tip, base);
        if length > 1e-6 {
            let dx = (tip.x - base.x) / length;
            let dy = (tip.y - base.y) / length;
            let half = size * 0.62 * js::min(1.0, length / (size * 1.25));
            head = Some(ArrowHead {
                tip,
                base,
                points: [
                    tip,
                    Point::new(base.x - dy * half, base.y + dx * half),
                    Point::new(base.x + dy * half, base.y - dx * half),
                ],
            });
        }
    }
    let end = at(shaft_t);
    let mut curve = vec![points[0], end];
    let d = if cubic {
        let a = mix(points[0], points[1], shaft_t);
        let b = mix(points[1], points[2], shaft_t);
        let first = a;
        let second = mix(a, b, shaft_t);
        curve = vec![points[0], first, second, end];
        format!(
            "M {} {} C {} {}, {} {}, {} {}",
            n(points[0].x),
            n(points[0].y),
            n(first.x),
            n(first.y),
            n(second.x),
            n(second.y),
            n(end.x),
            n(end.y)
        )
    } else {
        format!("M {} {} L {} {}", n(points[0].x), n(points[0].y), n(end.x), n(end.y))
    };
    EndpointGeometry { d, arrow: head, curve }
}

/// Last crossing, so loops and strongly bent branches clip at the node end.
fn outside_circle(at: &impl Fn(f64) -> Point, end_t: f64, center: Point, radius: f64) -> Option<f64> {
    if distance(at(end_t), center) >= radius {
        return Some(end_t);
    }
    let mut inside = end_t;
    for i in 1..=32 {
        let outside = end_t * (1.0 - i as f64 / 32.0);
        if distance(at(outside), center) >= radius {
            let mut low = outside;
            let mut high = inside;
            for _ in 0..30 {
                let mid = (low + high) / 2.0;
                if distance(at(mid), center) >= radius {
                    low = mid;
                } else {
                    high = mid;
                }
            }
            return Some((low + high) / 2.0);
        }
        inside = outside;
    }
    None
}
