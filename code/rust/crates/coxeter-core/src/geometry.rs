//! Rectangle helpers used by the compartment packer, the drag rules and the
//! renderer (`src/layout/geometry.js`), and the size constants of
//! `src/layout/layout.js` that every one of them shares.

use crate::js;
use crate::mapdata::{MapCompartment, Point, Rect};

/// Every metabolite circle uses exactly one of the two radii.
#[derive(Debug, Clone, Copy)]
pub struct Sizes {
    pub primary_radius: f64,
    pub medium_radius: f64,
    pub currency_radius: f64,
    pub marker_radius: f64,
    pub reaction_axis: f64,
}

pub const SIZES: Sizes = Sizes {
    primary_radius: 13.0,
    medium_radius: 13.0,
    currency_radius: 7.0,
    marker_radius: 5.0,
    reaction_axis: 17.0,
};

/// Clearances the automatic placement keeps around a node. Manual dragging
/// obeys the same rules, so a node the user moves ends up somewhere the
/// relaxation would have been willing to put it.
#[derive(Debug, Clone, Copy)]
pub struct Clearance {
    /// Beyond the node radius, inside its own compartment.
    pub bounds: f64,
    /// Beyond the node radius, away from a nested compartment.
    pub hole: f64,
    /// How far inside the shared environment a medium node stays.
    pub environment: f64,
    /// How far medium nodes stay clear of a cell rectangle.
    pub cell: f64,
}

pub const CLEARANCE: Clearance = Clearance { bounds: 6.0, hole: 10.0, environment: 12.0, cell: 10.0 };

pub const fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
    Rect { x, y, w, h }
}

/// `inset(r, top, right = top, bottom = top, left = right)`.
pub fn inset(r: &Rect, top: f64, right: Option<f64>, bottom: Option<f64>, left: Option<f64>) -> Rect {
    let right_v = right.unwrap_or(top);
    let bottom_v = bottom.unwrap_or(top);
    let left_v = left.unwrap_or(right_v);
    Rect {
        x: r.x + left_v,
        y: r.y + top,
        w: js::max(0.0, r.w - left_v - right_v),
        h: js::max(0.0, r.h - top - bottom_v),
    }
}

/// `inset(r, d)`: the same margin on every side.
pub fn inset_all(r: &Rect, d: f64) -> Rect {
    inset(r, d, None, None, None)
}

pub fn center(r: &Rect) -> Point {
    Point::new(r.x + r.w / 2.0, r.y + r.h / 2.0)
}
pub fn area(r: &Rect) -> f64 {
    r.w * r.h
}
pub fn right(r: &Rect) -> f64 {
    r.x + r.w
}
pub fn bottom(r: &Rect) -> f64 {
    r.y + r.h
}

/// Organelles drawn with strongly rounded corners.
pub const ROUND_ORGANELLES: [&str; 9] = ["m", "u", "n", "v", "x", "g", "l", "cx", "im"];

/// Whether a compartment drawn as `visual_id` is one of the [`ROUND_ORGANELLES`].
pub fn is_round_organelle(visual_id: &str) -> bool {
    ROUND_ORGANELLES.contains(&visual_id)
}

/// The id a compartment is drawn as (`visualId ?? id`).
pub fn drawn_id(c: &MapCompartment) -> &str {
    if c.visual_id.is_empty() { &c.id } else { &c.visual_id }
}

/// The corner a compartment gets until its shape is edited: round
/// organelles, square-ish shells (`defaultCornerRadius`).
pub fn corner_radius_for(visual_id: &str, r: &Rect) -> f64 {
    if is_round_organelle(visual_id) {
        return js::min(r.w, r.h) * 0.36;
    }
    js::min(40.0, js::min(r.w, r.h) * 0.16)
}

/// `Math.min(1, Math.max(0, Number(v) || 0))`.
fn unit(v: f64) -> f64 {
    js::min(1.0, js::max(0.0, if v.is_nan() { 0.0 } else { v }))
}

/// A compartment's shape, as two amounts from 0 to 1. `corner` is how soft
/// its corners are: 0 square, 1 a corner as round as its shorter side allows
/// (a pill). `oval` is how far the whole outline is pulled towards an oval,
/// whose corners grow with the rectangle in both directions. On a full oval
/// `corner` still counts, as how round that oval is: 1 the ellipse inside the
/// rectangle, 0 a boxy oval with flat sides (see [`compartment_corners`]).
/// Kept as fractions, so a resized compartment keeps its look.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompartmentShape {
    pub corner: f64,
    pub oval: f64,
}

impl CompartmentShape {
    /// `{ corner, oval }`, as saved on a compartment.
    pub fn to_json(&self) -> serde_json::Value {
        let mut v = serde_json::json!({ "corner": self.corner, "oval": self.oval });
        js::normalize_json_numbers(&mut v);
        v
    }
}

/// A finite number in a saved shape (`Number.isFinite(s[key])`).
fn shape_amount(shape: &serde_json::Value, key: &str) -> Option<f64> {
    shape.get(key).and_then(serde_json::Value::as_f64).filter(|v| v.is_finite())
}

/// `compartmentShape(compartment)`. A compartment whose shape was never edited
/// gets the default corner of its kind.
pub fn compartment_shape(c: &MapCompartment) -> CompartmentShape {
    shape_of(c.shape.as_ref(), drawn_id(c), &c.rect)
}

/// The shape of a compartment drawn as `visual_id` in `r`, from what is saved.
pub fn shape_of(saved: Option<&serde_json::Value>, visual_id: &str, r: &Rect) -> CompartmentShape {
    if let Some(s) = saved {
        let corner = shape_amount(s, "corner");
        let oval = shape_amount(s, "oval");
        if corner.is_some() || oval.is_some() {
            return CompartmentShape { corner: unit(corner.unwrap_or(0.0)), oval: unit(oval.unwrap_or(0.0)) };
        }
    }
    let half = js::min(r.w, r.h) / 2.0;
    CompartmentShape { corner: if half > 0.0 { unit(corner_radius_for(visual_id, r) / half) } else { 0.0 }, oval: 0.0 }
}

/// Ready-made shapes, by name, as [`compartment_shape`] amounts
/// (`COMPARTMENT_SHAPES`).
pub const COMPARTMENT_SHAPES: [(&str, CompartmentShape); 4] = [
    ("rectangle", CompartmentShape { corner: 0.0, oval: 0.0 }),
    ("rounded", CompartmentShape { corner: 0.3, oval: 0.0 }),
    ("capsule", CompartmentShape { corner: 1.0, oval: 0.0 }),
    ("oval", CompartmentShape { corner: 1.0, oval: 1.0 }),
];

/// `COMPARTMENT_SHAPES[name]`.
pub fn compartment_shape_preset(name: &str) -> Option<CompartmentShape> {
    COMPARTMENT_SHAPES.iter().find(|(n, _)| *n == name).map(|(_, s)| *s)
}

// How far a full oval's corners reach along each side, as a share of half that
// side, at its boxiest (corner 0) and roundest (corner 1, the ellipse).
const OVAL_BOXY: f64 = 0.3;

/// The radii of a compartment's corners, which may be elliptical.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Corners {
    pub rx: f64,
    pub ry: f64,
}

impl Corners {
    /// Circular corners of `radius`.
    pub fn circular(radius: f64) -> Corners {
        Corners { rx: radius, ry: radius }
    }
}

/// `compartmentCorners(compartment)`: what SVG's rx and ry draw, and what a
/// boundary projects onto. A rounded rectangle has circular corners. An oval's
/// corners are elliptical, the same share of the width and of the height, and
/// the `oval` amount blends one into the other.
pub fn compartment_corners(c: &MapCompartment) -> Corners {
    corners_of(&compartment_shape(c), &c.rect)
}

/// The corners a shape gives the rectangle `r`.
pub fn corners_of(shape: &CompartmentShape, r: &Rect) -> Corners {
    let CompartmentShape { corner, oval } = *shape;
    let base = corner * js::min(r.w, r.h) / 2.0;
    let reach = OVAL_BOXY + (1.0 - OVAL_BOXY) * corner;
    Corners { rx: base + oval * (reach * r.w / 2.0 - base), ry: base + oval * (reach * r.h / 2.0 - base) }
}

/// The corners of a compartment drawn as `visual_id` in `r`, never edited.
pub fn default_corners(visual_id: &str, r: &Rect) -> Corners {
    corners_of(&shape_of(None, visual_id, r), r)
}

/// `compartmentCornerRadius(compartment)`: the corner radius of a
/// compartment whose corners are circular.
pub fn compartment_corner_radius(c: &MapCompartment) -> f64 {
    let Corners { rx, ry } = compartment_corners(c);
    js::min(rx, ry)
}

/// `isRoundCompartment(compartment)`: whether a compartment reads as round,
/// and gets its name astride the top edge.
pub fn is_round_compartment(c: &MapCompartment) -> bool {
    if c.shape.is_none() {
        return is_round_organelle(drawn_id(c));
    }
    let CompartmentShape { corner, oval } = compartment_shape(c);
    oval >= 0.25 || corner >= 0.6
}

// Shape handles

// From src/map/geometry-edit.js, here so the renderer can place them too.

/// The two shape handles (`SHAPE_HANDLES`): `corner` softens the corners,
/// `oval` rounds the whole outline.
pub const SHAPE_HANDLES: [&str; 2] = ["corner", "oval"];

/// Whether a geometry handle is one of the two shape handles.
pub fn is_shape_handle(handle: &str) -> bool {
    SHAPE_HANDLES.contains(&handle)
}

/// The track a shape handle runs along (`shapeTrack`).
struct ShapeTrack {
    vertical: bool,
    a: f64,
    b: f64,
    at: f64,
}

/// The track a shape handle runs along, from the top-left corner: the corner
/// handle along the top edge, the oval handle down the left side. It starts
/// clear of the corner handle and stops clear of the side handle in the
/// middle, so no two handles sit on one another, and where along it the
/// handle stands is its amount, 0 at the corner end. `handle_size` is the
/// corner handle's side in map units, which is what the gaps are made of.
fn shape_track(rect: &Rect, which: &str, handle_size: f64) -> ShapeTrack {
    let vertical = which == "oval";
    let start = if vertical { rect.y } else { rect.x };
    let length = if vertical { rect.h } else { rect.w };
    let a = start + handle_size * 1.2;
    let b = js::max(a + handle_size, start + length / 2.0 - handle_size * 1.6);
    ShapeTrack { vertical, a, b, at: if vertical { rect.x } else { rect.y } }
}

/// `shapeHandlePoint(rect, which, amount, handleSize)`: where a shape handle
/// stands for an amount from 0 to 1.
pub fn shape_handle_point(rect: &Rect, which: &str, amount: f64, handle_size: f64) -> Point {
    let t = shape_track(rect, which, handle_size);
    let v = t.a + js::min(1.0, js::max(0.0, amount)) * (t.b - t.a);
    if t.vertical { Point::new(t.at, v) } else { Point::new(v, t.at) }
}

/// `shapeAmountAt(rect, which, point, handleSize)`: the amount, from 0 to 1,
/// a shape handle dragged to `point` stands for.
pub fn shape_amount_at(rect: &Rect, which: &str, point: Point, handle_size: f64) -> f64 {
    let t = shape_track(rect, which, handle_size);
    let v = if t.vertical { point.y } else { point.x };
    js::min(1.0, js::max(0.0, (v - t.a) / (t.b - t.a)))
}

/// Whether `p` lies inside `r` shrunk by `pad` on every side, edges included.
pub fn contains(r: &Rect, p: Point, pad: f64) -> bool {
    p.x >= r.x + pad && p.x <= right(r) - pad && p.y >= r.y + pad && p.y <= bottom(r) - pad
}

/// The point of `r`, shrunk by `pad` on every side, nearest to `p`. A side
/// shorter than `2 * pad` collapses to the rectangle's centre line.
pub fn clamp_to_rect(p: Point, r: &Rect, pad: f64) -> Point {
    let w = js::max(0.0, r.w / 2.0 - pad);
    let h = js::max(0.0, r.h / 2.0 - pad);
    let c = center(r);
    Point::new(js::min(js::max(p.x, c.x - w), c.x + w), js::min(js::max(p.y, c.y - h), c.y + h))
}

/// Split a rect into parts along its longer axis, weighted.
pub fn split_weighted(r: &Rect, weights: &[f64], gap: f64) -> Vec<Rect> {
    let sum: f64 = weights.iter().fold(0.0, |a, b| a + b);
    let total = if sum == 0.0 || sum.is_nan() { 1.0 } else { sum };
    let horizontal = r.w >= r.h;
    let span = (if horizontal { r.w } else { r.h }) - gap * (weights.len() as f64 - 1.0);
    let mut out = Vec::with_capacity(weights.len());
    let mut cursor = if horizontal { r.x } else { r.y };
    for w in weights {
        let size = (w / total) * span;
        out.push(if horizontal { Rect::new(cursor, r.y, size, r.h) } else { Rect::new(r.x, cursor, r.w, size) });
        cursor += size + gap;
    }
    out
}

/// Lay `n` rects out on a grid inside `r`, preferring near-square cells.
pub fn grid_cells(r: &Rect, n: usize, gap: f64, target_aspect: f64) -> Vec<Rect> {
    if n == 0 {
        return vec![];
    }
    let nf = n as f64;
    // Search the column counts rather than deriving one: a 4-cell map on a
    // wide canvas should come out 2x2, not 3 + 1 stranded underneath.
    let mut cols = 1usize;
    let mut best = f64::INFINITY;
    for c in 1..=n {
        let cf = c as f64;
        let rws = (nf / cf).ceil();
        let cw = (r.w - gap * (cf - 1.0)) / cf;
        let ch = (r.h - gap * (rws - 1.0)) / rws;
        if cw <= 0.0 || ch <= 0.0 {
            continue;
        }
        let aspect_cost = js::log((cw / ch) / target_aspect).abs();
        let ragged_cost = ((cf * rws - nf) / nf) * 0.9;
        let score = aspect_cost + ragged_cost;
        if score < best {
            best = score;
            cols = c;
        }
    }
    let colsf = cols as f64;
    let rows = (nf / colsf).ceil();
    let cw = (r.w - gap * (colsf - 1.0)) / colsf;
    let ch = (r.h - gap * (rows - 1.0)) / rows;
    (0..n)
        .map(|i| {
            let col = (i % cols) as f64;
            let row = (i / cols) as f64;
            // Centre the last, possibly short, row.
            let in_row = js::min(colsf, nf - row * colsf);
            let offset = ((colsf - in_row) * (cw + gap)) / 2.0;
            Rect::new(r.x + offset + col * (cw + gap), r.y + row * (ch + gap), cw, ch)
        })
        .collect()
}

/// A point on a perimeter with its outward normal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PerimeterPoint {
    pub x: f64,
    pub y: f64,
    pub nx: f64,
    pub ny: f64,
}

impl PerimeterPoint {
    pub fn point(&self) -> Point {
        Point::new(self.x, self.y)
    }
}

/// Walk the perimeter of a rectangle: `t` runs 0..1 clockwise from the middle
/// of the top edge.
pub fn perimeter_point(r: &Rect, t: f64) -> PerimeterPoint {
    let p = ((t % 1.0) + 1.0) % 1.0;
    let perim = 2.0 * (r.w + r.h);
    let mut d = p * perim;
    let half_top = r.w / 2.0;
    if d < half_top {
        return PerimeterPoint { x: r.x + r.w / 2.0 + d, y: r.y, nx: 0.0, ny: -1.0 };
    }
    d -= half_top;
    if d < r.h {
        return PerimeterPoint { x: right(r), y: r.y + d, nx: 1.0, ny: 0.0 };
    }
    d -= r.h;
    if d < r.w {
        return PerimeterPoint { x: right(r) - d, y: bottom(r), nx: 0.0, ny: 1.0 };
    }
    d -= r.w;
    if d < r.h {
        return PerimeterPoint { x: r.x, y: bottom(r) - d, nx: -1.0, ny: 0.0 };
    }
    d -= r.h;
    PerimeterPoint { x: r.x + d, y: r.y, nx: 0.0, ny: -1.0 }
}

/// Closest point on the perimeter of `r` to `p` (corners rounded by
/// `radius`), with the outward normal.
pub fn closest_perimeter_point(r: &Rect, p: Point, radius: f64) -> PerimeterPoint {
    let a = js::max(0.0, js::min(js::min(if radius.is_nan() { 0.0 } else { radius }, r.w / 2.0), r.h / 2.0));
    closest_perimeter_point_with(r, p, a, a)
}

/// `closestPerimeterPoint(r, p, {rx, ry})`: the same with elliptical corners
/// ([`compartment_corners`]). On an elliptical corner the point is where the
/// line from that corner's centre meets the ellipse, near enough to the
/// closest point for placing a transporter, and the normal is the ellipse's
/// own.
pub fn closest_perimeter_point_corners(r: &Rect, p: Point, corners: Corners) -> PerimeterPoint {
    let nz = |v: f64| if v.is_nan() { 0.0 } else { v };
    let ax = js::max(0.0, js::min(nz(corners.rx), r.w / 2.0));
    let ay = js::max(0.0, js::min(nz(corners.ry), r.h / 2.0));
    closest_perimeter_point_with(r, p, ax, ay)
}

fn closest_perimeter_point_with(r: &Rect, p: Point, ax: f64, ay: f64) -> PerimeterPoint {
    if ax > 0.0 && ay > 0.0 {
        let nearest_x = js::max(r.x + ax, js::min(right(r) - ax, p.x));
        let nearest_y = js::max(r.y + ay, js::min(bottom(r) - ay, p.y));
        let u = (p.x - nearest_x) / ax;
        let v = (p.y - nearest_y) / ay;
        let length = js::hypot(u, v);
        if length > 0.0 {
            let ex = (ax * u) / length;
            let ey = (ay * v) / length;
            let gx = ex / (ax * ax);
            let gy = ey / (ay * ay);
            let g = js::hypot(gx, gy);
            return PerimeterPoint { x: nearest_x + ex, y: nearest_y + ey, nx: gx / g, ny: gy / g };
        }
    }
    let c = center(r);
    let dx = p.x - c.x;
    let dy = p.y - c.y;
    let sx = r.w / 2.0;
    let sy = r.h / 2.0;
    if dx.abs() * sy > dy.abs() * sx {
        let nx = if dx >= 0.0 { 1.0 } else { -1.0 };
        return PerimeterPoint { x: c.x + nx * sx, y: c.y + js::max(-sy, js::min(sy, dy)), nx, ny: 0.0 };
    }
    let ny = if dy >= 0.0 { 1.0 } else { -1.0 };
    PerimeterPoint { x: c.x + js::max(-sx, js::min(sx, dx)), y: c.y + ny * sy, nx: 0.0, ny }
}

/// Signed distance from a point to the rect edge, negative inside.
pub fn signed_distance(r: &Rect, p: Point) -> f64 {
    let dx = js::max_n([r.x - p.x, 0.0, p.x - right(r)]);
    let dy = js::max_n([r.y - p.y, 0.0, p.y - bottom(r)]);
    let outside = js::hypot(dx, dy);
    if outside > 0.0 {
        return outside;
    }
    -js::min_n([p.x - r.x, right(r) - p.x, p.y - r.y, bottom(r) - p.y])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_prefers_square_cells() {
        let cells = grid_cells(&Rect::new(0.0, 0.0, 1000.0, 1000.0), 4, 24.0, 1.08);
        assert_eq!(cells.len(), 4);
        assert_eq!(cells[1].y, 0.0);
        assert_eq!(cells[2].x, 0.0);
    }

    #[test]
    fn perimeter_walk_and_projection() {
        let r = Rect::new(0.0, 0.0, 100.0, 50.0);
        assert_eq!(perimeter_point(&r, 0.0).point(), Point::new(50.0, 0.0));
        let p = closest_perimeter_point(&r, Point::new(120.0, 25.0), 0.0);
        assert_eq!((p.x, p.y, p.nx, p.ny), (100.0, 25.0, 1.0, 0.0));
        assert_eq!(signed_distance(&r, Point::new(50.0, 25.0)), -25.0);
        assert_eq!(signed_distance(&r, Point::new(103.0, 54.0)), 5.0);
    }

    fn comp(id: &str, rect: Rect, shape: Option<serde_json::Value>) -> MapCompartment {
        MapCompartment { id: id.into(), rect, shape, ..Default::default() }
    }

    #[test]
    fn an_unedited_compartment_keeps_the_corners_it_always_had() {
        let m = comp("m", Rect::new(0.0, 0.0, 300.0, 200.0), None);
        let c = comp("c", Rect::new(0.0, 0.0, 800.0, 500.0), None);
        assert!((compartment_corners(&m).rx - 200.0 * 0.36).abs() < 1e-9, "round organelle");
        assert!((compartment_corners(&c).rx - js::min(40.0, 500.0 * 0.16)).abs() < 1e-9, "square-ish shell");
        assert_eq!(compartment_shape(&m).oval, 0.0);
        assert!(is_round_compartment(&m) && !is_round_compartment(&c));
    }

    #[test]
    fn the_oval_amount_pulls_the_corners_out_and_the_corner_amount_still_shapes_the_oval() {
        let rect = Rect::new(100.0, 50.0, 600.0, 300.0);
        let corners = |corner: f64, oval: f64| corners_of(&CompartmentShape { corner, oval }, &rect);
        let close = |a: Corners, rx: f64, ry: f64| (a.rx - rx).abs() < 1e-9 && (a.ry - ry).abs() < 1e-9;
        assert_eq!(corners(0.0, 0.0), Corners { rx: 0.0, ry: 0.0 });
        assert_eq!(corners(1.0, 0.0), Corners { rx: 150.0, ry: 150.0 });
        assert_eq!(corners(1.0, 1.0), Corners { rx: 300.0, ry: 150.0 });
        assert!(close(corners(0.0, 1.0), 90.0, 45.0));
        let boxier = corners(0.4, 1.0);
        assert!(boxier.rx < 300.0 && boxier.rx > 90.0 && (boxier.rx / boxier.ry - 2.0).abs() < 1e-9);
        assert!(close(corners(0.5, 0.5), 75.0 + 0.5 * (195.0 - 75.0), 75.0 + 0.5 * (97.5 - 75.0)));

        // Projected onto the full ellipse, any point lands on it, with the ellipse's own normal.
        let ellipse = corners(1.0, 1.0);
        for (x, y) in [(90.0, 40.0), (650.0, 330.0), (400.0, 0.0), (300.0, 200.0), (800.0, 200.0)] {
            let q = closest_perimeter_point_corners(&rect, Point::new(x, y), ellipse);
            let on = js::hypot((q.x - 400.0) / 300.0, (q.y - 200.0) / 150.0) - 1.0;
            assert!(on.abs() < 1e-9, "{q:?}");
            let gx = (q.x - 400.0) / (300.0 * 300.0);
            let gy = (q.y - 200.0) / (150.0 * 150.0);
            let g = js::hypot(gx, gy);
            assert!((q.nx - gx / g).abs() < 1e-9 && (q.ny - gy / g).abs() < 1e-9);
        }
        // A saved shape is read back, clamped to 0..1, and named presets exist.
        let shaped = comp("x", rect, Some(serde_json::json!({ "corner": 2, "oval": 0.5 })));
        assert_eq!(compartment_shape(&shaped), CompartmentShape { corner: 1.0, oval: 0.5 });
        assert!(is_round_compartment(&shaped));
        assert_eq!(compartment_shape_preset("oval"), Some(CompartmentShape { corner: 1.0, oval: 1.0 }));
        assert_eq!(CompartmentShape { corner: 1.0, oval: 0.25 }.to_json().to_string(), r#"{"corner":1,"oval":0.25}"#);
    }
}
