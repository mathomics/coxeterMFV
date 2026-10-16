//! What is under the pointer: `event.target` and `document.elementsFromPoint`
//! for the map, in the order the DOM gives them (the element painted last
//! wins), under the SVG rules of `pointer-events` and `visibility`.
//!
//! Geometry is the element's own: a circle's disc and ring, a rounded
//! rectangle, a path's (non-zero) interior and the band of its stroke, a
//! text's character cells. Zoom-dependent targets are taken at the live zoom,
//! as `applyCamera()` leaves them: the invisible halos (`updateHitTargets`:
//! `r = max(data-hit-base, MIN_HIT_PX / scale)`), the multi-selection border
//! (`stroke-width = max(12, MIN_HIT_PX / scale)`), the edit handles
//! (`updateEditHandles`), and strokes that do not scale.
//!
//! Two stage-level rules of the stylesheets, which the map's own tree cannot
//! express, are applied here: in Pan mode the map takes no pointer events
//! (`.stage[data-tool="pan"] svg`), and while a drawing tool is live the
//! halos do not either (`.stage.is-drawing-tool .emc-hit`).
//!
//! Dashes are not cut out of a stroke's band.

use coxeter_core::mapdata::Point;

use crate::camera::MIN_HIT_PX;
use crate::geom::{self, ShapeKind};
use crate::scene::{HitGeom, Scene};

/// The stage's state, as far as pointer events go.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct HitFilter {
    /// Pan mode: nothing on the map is a target.
    pub pan_tool: bool,
    /// A drawing tool (cell, compartment) is live: the halos step aside.
    pub drawing_tool: bool,
}

/// The topmost element at a map point, if any (`event.target` inside the map).
pub fn hit_test(scene: &Scene, p: Point, filter: HitFilter) -> Option<usize> {
    if filter.pan_tool || !in_viewport(scene, p) {
        return None;
    }
    (1..scene.elems.len()).rev().find(|&i| hits(scene, i, p, filter))
}

/// Every element at a map point, topmost first (`elementsFromPoint`).
pub fn hits_at(scene: &Scene, p: Point, filter: HitFilter) -> Vec<usize> {
    if filter.pan_tool || !in_viewport(scene, p) {
        return Vec::new();
    }
    (1..scene.elems.len()).rev().filter(|&i| hits(scene, i, p, filter)).collect()
}

/// Whether a map point is inside the `<svg>`'s own box: an outer SVG element
/// clips what overflows it (`overflow: hidden`), and what is clipped away
/// takes no pointer either.
pub fn in_viewport(scene: &Scene, p: Point) -> bool {
    let (w, h) = scene.canvas_size();
    p.x >= 0.0 && p.y >= 0.0 && p.x <= w && p.y <= h
}

/// Whether element `i` takes a pointer at map point `p`.
pub fn hits(scene: &Scene, i: usize, p: Point, filter: HitFilter) -> bool {
    let d = &scene.elems[i];
    let Some(hit) = &d.hit else { return false };
    if !d.shown(&scene.zoom) {
        return false;
    }
    let style = &d.style;
    let (fill_ok, stroke_ok) = (style.hits_fill(), style.hits_stroke());
    if !fill_ok && !stroke_ok {
        return false;
    }
    let live = d.live_hit;
    if filter.drawing_tool && matches!(live, LiveHit::Halo(_)) {
        return false;
    }
    // Into the group's base geometry, and back by the element's shift to
    // the geometry its hit shapes were made from.
    let (px, py, stroke_factor) = if d.group == 0 {
        (p.x, p.y, 1.0)
    } else {
        let g = &scene.groups[d.group as usize];
        let (x, y) = g.unplace(p.x, p.y);
        (x, y, if g.k != 0.0 { g.m / g.k } else { 1.0 })
    };
    let (px, py) = (px - d.shift[0], py - d.shift[1]);
    let scale = if scene.scale > 0.0 { scene.scale } else { 1.0 };
    // Broad phase.
    if !matches!(live, LiveHit::Halo(_) | LiveHit::MultiBorder)
        && let Some(b) = d.hit_bounds {
            let pad = 1e-9;
            if px < b[0] - pad || px > b[2] + pad || py < b[1] - pad || py > b[3] + pad {
                return false;
            }
        }
    match hit {
        // A text takes the pointer in its character cells, and where its
        // stroke is painted (the halo of a label) the band that adds around
        // them, as browsers hit-test the runs they paint.
        HitGeom::Text { quad, scale: text_scale } => {
            let painted_stroke = stroke_ok && style.visible && !style.stroke.is_none() && style.stroke_width > 0.0;
            let half = if painted_stroke { style.stroke_width / 2.0 * stroke_factor * text_scale } else { 0.0 };
            (fill_ok || painted_stroke) && (in_quad(quad, px, py) || (half > 0.0 && quad_distance(quad, px, py) <= half))
        }
        HitGeom::Shape { kind, inv, matrix, flat, fillable } => {
            let ms = geom::mean_scale(matrix);
            // The stroke width in the shape's own units, at the live zoom.
            let mut sw = style.stroke_width;
            if style.non_scaling_stroke {
                sw = style.stroke_width / scale / if ms > 0.0 { ms } else { 1.0 };
            }
            if let LiveHit::MultiBorder = live {
                sw = 12f64.max(MIN_HIT_PX / scale);
            }
            sw *= stroke_factor;
            let half = sw / 2.0;
            let fill_ok = fill_ok && *fillable;
            let (lx, ly) = geom::apply(inv, px, py);
            match kind {
                ShapeKind::Circle { cx, cy, r } => {
                    let r = match live {
                        LiveHit::Halo(base) => base.max(MIN_HIT_PX / scale),
                        _ => *r,
                    };
                    let dist = coxeter_core::js::hypot(lx - cx, ly - cy);
                    (fill_ok && dist <= r) || (stroke_ok && (dist - r).abs() <= half)
                }
                ShapeKind::Ellipse { cx, cy, rx, ry } => {
                    let nx = (lx - cx) / rx;
                    let ny = (ly - cy) / ry;
                    let n = (nx * nx + ny * ny).sqrt();
                    let dist = (n - 1.0) * rx.min(*ry);
                    (fill_ok && n <= 1.0) || (stroke_ok && dist.abs() <= half)
                }
                ShapeKind::Rect { x, y, w, h, rx, ry } => {
                    let sd = rounded_rect_sdf(lx, ly, *x, *y, *w, *h, *rx, *ry);
                    (fill_ok && sd <= 0.0) || (stroke_ok && sd.abs() <= half)
                }
                ShapeKind::Line { x1, y1, x2, y2 } => stroke_ok && segment_distance(lx, ly, *x1, *y1, *x2, *y2) <= half,
                ShapeKind::Poly { .. } | ShapeKind::Path => {
                    // Flattened in map coordinates.
                    let half_map = half * ms;
                    (fill_ok && winding(flat, px, py) != 0)
                        || (stroke_ok
                            && flat.iter().any(|(pts, closed)| polyline_distance(pts, *closed, px, py) <= half_map))
                }
            }
        }
    }
}

/// How an element's pointer target follows the zoom.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum LiveHit {
    #[default]
    Fixed,
    /// `.emc-hit`: radius `max(data-hit-base, MIN_HIT_PX / scale)`.
    Halo(f64),
    /// `.emc-multi-drag-hit`: stroke `max(12, MIN_HIT_PX / scale)`.
    MultiBorder,
}

/// Signed distance to a rounded rectangle (negative inside).
#[allow(clippy::too_many_arguments)]
pub fn rounded_rect_sdf(px: f64, py: f64, x: f64, y: f64, w: f64, h: f64, rx: f64, ry: f64) -> f64 {
    let cx = x + w / 2.0;
    let cy = y + h / 2.0;
    let qx = (px - cx).abs() - w / 2.0;
    let qy = (py - cy).abs() - h / 2.0;
    let r = rx.min(ry).max(0.0);
    if r <= 0.0 {
        let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
        return outside + qx.max(qy).min(0.0);
    }
    // Elliptical corners approximated by their smaller radius when rx ≠ ry.
    let ax = qx + r;
    let ay = qy + r;
    let outside = (ax.max(0.0).powi(2) + ay.max(0.0).powi(2)).sqrt();
    outside + ax.max(ay).min(0.0) - r
}

fn segment_distance(px: f64, py: f64, x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    let dx = x2 - x1;
    let dy = y2 - y1;
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((px - x1) * dx + (py - y1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    let (cx, cy) = (x1 + t * dx, y1 + t * dy);
    ((px - cx).powi(2) + (py - cy).powi(2)).sqrt()
}

fn polyline_distance(pts: &[[f64; 2]], closed: bool, px: f64, py: f64) -> f64 {
    let mut best = f64::INFINITY;
    for w in pts.windows(2) {
        best = best.min(segment_distance(px, py, w[0][0], w[0][1], w[1][0], w[1][1]));
    }
    if closed && pts.len() > 2 {
        let (a, b) = (pts[pts.len() - 1], pts[0]);
        best = best.min(segment_distance(px, py, a[0], a[1], b[0], b[1]));
    }
    if pts.len() == 1 {
        best = ((px - pts[0][0]).powi(2) + (py - pts[0][1]).powi(2)).sqrt();
    }
    best
}

/// The winding number of the (implicitly closed) sub-paths about a point.
fn winding(subpaths: &[(Vec<[f64; 2]>, bool)], px: f64, py: f64) -> i32 {
    let mut w = 0;
    for (pts, _) in subpaths {
        let n = pts.len();
        if n < 3 {
            continue;
        }
        for k in 0..n {
            let a = pts[k];
            let b = pts[(k + 1) % n];
            if a[1] <= py {
                if b[1] > py && cross(a, b, px, py) > 0.0 {
                    w += 1;
                }
            } else if b[1] <= py && cross(a, b, px, py) < 0.0 {
                w -= 1;
            }
        }
    }
    w
}

fn cross(a: [f64; 2], b: [f64; 2], px: f64, py: f64) -> f64 {
    (b[0] - a[0]) * (py - a[1]) - (px - a[0]) * (b[1] - a[1])
}

/// The distance from a point to a quadrilateral's outline.
fn quad_distance(q: &[[f64; 2]; 4], px: f64, py: f64) -> f64 {
    let pts: Vec<[f64; 2]> = q.to_vec();
    polyline_distance(&pts, true, px, py)
}

fn in_quad(q: &[[f64; 2]; 4], px: f64, py: f64) -> bool {
    let mut sign = 0.0f64;
    for k in 0..4 {
        let c = cross(q[k], q[(k + 1) % 4], px, py);
        if c != 0.0 {
            if sign == 0.0 {
                sign = c.signum();
            } else if c.signum() != sign {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounded_rectangles() {
        assert!(rounded_rect_sdf(5.0, 5.0, 0.0, 0.0, 10.0, 10.0, 3.0, 3.0) < 0.0);
        assert!(rounded_rect_sdf(0.2, 0.2, 0.0, 0.0, 10.0, 10.0, 3.0, 3.0) > 0.0, "the corner is cut");
        assert!((rounded_rect_sdf(12.0, 5.0, 0.0, 0.0, 10.0, 10.0, 0.0, 0.0) - 2.0).abs() < 1e-12);
        let sq = vec![(vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]], true)];
        assert_eq!(winding(&sq, 5.0, 5.0).abs(), 1);
        assert_eq!(winding(&sq, 15.0, 5.0), 0);
        assert!(in_quad(&[[0.0, 0.0], [4.0, 0.0], [4.0, 2.0], [0.0, 2.0]], 1.0, 1.0));
        assert!(!in_quad(&[[0.0, 0.0], [4.0, 0.0], [4.0, 2.0], [0.0, 2.0]], 5.0, 1.0));
    }
}
