//! The geometry of an SVG element: `path` (`d`, every command, relative ones
//! and arcs included, through `svgtypes`), `rect` (with `rx`/`ry`), `circle`,
//! `ellipse`, `line`, `polyline` and `polygon`, built as lyon paths in the
//! element's own coordinates, the way an SVG renderer builds them (a circle
//! starts at its rightmost point and turns clockwise on screen, a rectangle at
//! the end of its top-left corner, so that dashes start where they do in a
//! browser).
//!
//! Also 2D affine transforms (`[a, b, c, d, e, f]`, `x' = a·x + c·y + e`).

use lyon_tessellation::geom::point;
use lyon_tessellation::geom::vector;
use lyon_tessellation::path::builder::SvgPathBuilder;
use lyon_tessellation::path::{ArcFlags, Path};
use lyon_tessellation::geom::Angle;
use coxeter_render::Element;

/// An affine transform `[a, b, c, d, e, f]`.
pub type Mat = [f64; 6];

pub const IDENTITY: Mat = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `outer × inner`: apply `inner` first.
pub fn mul(outer: &Mat, inner: &Mat) -> Mat {
    [
        outer[0] * inner[0] + outer[2] * inner[1],
        outer[1] * inner[0] + outer[3] * inner[1],
        outer[0] * inner[2] + outer[2] * inner[3],
        outer[1] * inner[2] + outer[3] * inner[3],
        outer[0] * inner[4] + outer[2] * inner[5] + outer[4],
        outer[1] * inner[4] + outer[3] * inner[5] + outer[5],
    ]
}

/// A point through a transform.
pub fn apply(m: &Mat, x: f64, y: f64) -> (f64, f64) {
    (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
}

/// The linear part only (a direction or an offset).
pub fn apply_vec(m: &Mat, x: f64, y: f64) -> (f64, f64) {
    (m[0] * x + m[2] * y, m[1] * x + m[3] * y)
}

/// The inverse transform, `None` when there is none.
pub fn invert(m: &Mat) -> Option<Mat> {
    let det = m[0] * m[3] - m[1] * m[2];
    if det == 0.0 || !det.is_finite() {
        return None;
    }
    let inv = 1.0 / det;
    Some([
        m[3] * inv,
        -m[1] * inv,
        -m[2] * inv,
        m[0] * inv,
        (m[2] * m[5] - m[3] * m[4]) * inv,
        (m[1] * m[4] - m[0] * m[5]) * inv,
    ])
}

/// How much a transform scales lengths (the geometric mean of its axes).
pub fn mean_scale(m: &Mat) -> f64 {
    (m[0] * m[3] - m[1] * m[2]).abs().sqrt()
}

pub fn is_identity(m: &Mat) -> bool {
    *m == IDENTITY
}

/// A number attribute (`0` when missing or not a number, as SVG reads it).
pub fn num_attr(e: &Element, name: &str) -> f64 {
    e.get(name).map(parse_number).unwrap_or(0.0)
}

/// An optional number attribute.
pub fn opt_num_attr(e: &Element, name: &str) -> Option<f64> {
    e.get(name).map(parse_number).filter(|v| v.is_finite())
}

fn parse_number(s: &str) -> f64 {
    let t = s.trim();
    let t = t.strip_suffix("px").unwrap_or(t);
    t.parse::<f64>().unwrap_or(f64::NAN)
}

/// The kinds of shape, with the numbers the hit-tester reads directly.
#[derive(Debug, Clone, PartialEq)]
pub enum ShapeKind {
    Rect { x: f64, y: f64, w: f64, h: f64, rx: f64, ry: f64 },
    Circle { cx: f64, cy: f64, r: f64 },
    Ellipse { cx: f64, cy: f64, rx: f64, ry: f64 },
    Line { x1: f64, y1: f64, x2: f64, y2: f64 },
    Poly { points: Vec<(f64, f64)>, closed: bool },
    Path,
}

/// An element's geometry in its own coordinates.
#[derive(Debug, Clone)]
pub struct Shape {
    pub kind: ShapeKind,
    pub path: Path,
    /// Whether filling it paints anything (a line never does).
    pub fillable: bool,
}

fn p(x: f64, y: f64) -> lyon_tessellation::geom::Point<f32> {
    point(x as f32, y as f32)
}

/// The shape of a `rect`, `circle`, `ellipse`, `line`, `polyline`,
/// `polygon` or `path`; `None` for anything else, or a shape that draws
/// nothing (a rectangle without area, a circle of radius 0).
pub fn shape_of(e: &Element) -> Option<Shape> {
    match e.name {
        "rect" => {
            let x = num_attr(e, "x");
            let y = num_attr(e, "y");
            let w = num_attr(e, "width");
            let h = num_attr(e, "height");
            if !(w > 0.0 && h > 0.0) || !x.is_finite() || !y.is_finite() {
                return None;
            }
            // `rx` alone sets `ry` too, and the other way round; each is held to
            // half its side.
            let rx_attr = opt_num_attr(e, "rx").filter(|v| *v >= 0.0);
            let ry_attr = opt_num_attr(e, "ry").filter(|v| *v >= 0.0);
            let (rx, ry) = match (rx_attr, ry_attr) {
                (Some(rx), Some(ry)) => (rx, ry),
                (Some(r), None) | (None, Some(r)) => (r, r),
                (None, None) => (0.0, 0.0),
            };
            let rx = rx.min(w / 2.0);
            let ry = ry.min(h / 2.0);
            Some(Shape { kind: ShapeKind::Rect { x, y, w, h, rx, ry }, path: rect_path(x, y, w, h, rx, ry), fillable: true })
        }
        "circle" => {
            let cx = num_attr(e, "cx");
            let cy = num_attr(e, "cy");
            let r = num_attr(e, "r");
            if !(r > 0.0) || !cx.is_finite() || !cy.is_finite() {
                return None;
            }
            Some(Shape { kind: ShapeKind::Circle { cx, cy, r }, path: ellipse_path(cx, cy, r, r), fillable: true })
        }
        "ellipse" => {
            let cx = num_attr(e, "cx");
            let cy = num_attr(e, "cy");
            let rx = num_attr(e, "rx");
            let ry = num_attr(e, "ry");
            if !(rx > 0.0 && ry > 0.0) {
                return None;
            }
            Some(Shape { kind: ShapeKind::Ellipse { cx, cy, rx, ry }, path: ellipse_path(cx, cy, rx, ry), fillable: true })
        }
        "line" => {
            let (x1, y1, x2, y2) = (num_attr(e, "x1"), num_attr(e, "y1"), num_attr(e, "x2"), num_attr(e, "y2"));
            if ![x1, y1, x2, y2].iter().all(|v| v.is_finite()) {
                return None;
            }
            let mut b = Path::builder();
            b.begin(p(x1, y1));
            b.line_to(p(x2, y2));
            b.end(false);
            Some(Shape { kind: ShapeKind::Line { x1, y1, x2, y2 }, path: b.build(), fillable: false })
        }
        "polyline" | "polygon" => {
            let closed = e.name == "polygon";
            let points = parse_points(e.get("points").unwrap_or(""));
            if points.len() < 2 {
                return None;
            }
            let mut b = Path::builder();
            b.begin(p(points[0].0, points[0].1));
            for &(x, y) in &points[1..] {
                b.line_to(p(x, y));
            }
            b.end(closed);
            Some(Shape { kind: ShapeKind::Poly { points, closed }, path: b.build(), fillable: true })
        }
        "path" => {
            let path = path_data(e.get("d").unwrap_or(""))?;
            Some(Shape { kind: ShapeKind::Path, path, fillable: true })
        }
        _ => None,
    }
}

/// `points="x,y x,y …"` (commas and spaces alike); an odd number drops the last.
pub fn parse_points(s: &str) -> Vec<(f64, f64)> {
    let nums: Vec<f64> = s
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|t| !t.is_empty())
        .map_while(|t| t.parse::<f64>().ok())
        .collect();
    nums.as_chunks::<2>().0.iter().map(|c| (c[0], c[1])).collect()
}

/// A rounded rectangle as SVG draws it: from the end of the top-left corner,
/// clockwise on screen.
pub fn rect_path(x: f64, y: f64, w: f64, h: f64, rx: f64, ry: f64) -> Path {
    let mut b = Path::builder().with_svg();
    if rx > 0.0 && ry > 0.0 {
        let radii = vector(rx as f32, ry as f32);
        let flags = ArcFlags { large_arc: false, sweep: true };
        b.move_to(p(x + rx, y));
        b.line_to(p(x + w - rx, y));
        b.arc_to(radii, Angle::radians(0.0), flags, p(x + w, y + ry));
        b.line_to(p(x + w, y + h - ry));
        b.arc_to(radii, Angle::radians(0.0), flags, p(x + w - rx, y + h));
        b.line_to(p(x + rx, y + h));
        b.arc_to(radii, Angle::radians(0.0), flags, p(x, y + h - ry));
        b.line_to(p(x, y + ry));
        b.arc_to(radii, Angle::radians(0.0), flags, p(x + rx, y));
        b.close();
    } else {
        b.move_to(p(x, y));
        b.line_to(p(x + w, y));
        b.line_to(p(x + w, y + h));
        b.line_to(p(x, y + h));
        b.close();
    }
    b.build()
}

/// An ellipse as SVG draws it: from its rightmost point, clockwise on screen.
pub fn ellipse_path(cx: f64, cy: f64, rx: f64, ry: f64) -> Path {
    let mut b = Path::builder().with_svg();
    let radii = vector(rx as f32, ry as f32);
    let flags = ArcFlags { large_arc: false, sweep: true };
    b.move_to(p(cx + rx, cy));
    b.arc_to(radii, Angle::radians(0.0), flags, p(cx, cy + ry));
    b.arc_to(radii, Angle::radians(0.0), flags, p(cx - rx, cy));
    b.arc_to(radii, Angle::radians(0.0), flags, p(cx, cy - ry));
    b.arc_to(radii, Angle::radians(0.0), flags, p(cx + rx, cy));
    b.close();
    b.build()
}

/// Path data (`d`) as a lyon path. Relative commands, smooth curves and
/// shorthand lines are made absolute, arcs become cubic curves (as usvg makes
/// them, with the same parser). Parsing stops at the first error and keeps
/// what came before it, as SVG renders a path "up to the error".
pub fn path_data(d: &str) -> Option<Path> {
    use svgtypes::SimplePathSegment as S;
    let mut b = Path::builder();
    let mut open = false;
    let mut any = false;
    let mut start = (0.0f64, 0.0f64);
    let mut current = (0.0f64, 0.0f64);
    for seg in svgtypes::SimplifyingPathParser::from(d) {
        let Ok(seg) = seg else { break };
        match seg {
            S::MoveTo { x, y } => {
                if open {
                    b.end(false);
                }
                b.begin(p(x, y));
                open = true;
                start = (x, y);
                current = (x, y);
            }
            S::LineTo { x, y } => {
                if !open {
                    b.begin(p(current.0, current.1));
                    open = true;
                }
                b.line_to(p(x, y));
                current = (x, y);
                any = true;
            }
            S::CurveTo { x1, y1, x2, y2, x, y } => {
                if !open {
                    b.begin(p(current.0, current.1));
                    open = true;
                }
                b.cubic_bezier_to(p(x1, y1), p(x2, y2), p(x, y));
                current = (x, y);
                any = true;
            }
            S::Quadratic { x1, y1, x, y } => {
                if !open {
                    b.begin(p(current.0, current.1));
                    open = true;
                }
                b.quadratic_bezier_to(p(x1, y1), p(x, y));
                current = (x, y);
                any = true;
            }
            S::ClosePath => {
                if open {
                    b.end(true);
                    open = false;
                }
                current = start;
            }
        }
    }
    if open {
        b.end(false);
    }
    any.then(|| b.build())
}

/// A transform attribute or computed transform text as a matrix.
pub fn parse_transform(text: &str) -> Option<Mat> {
    coxeter_render::css::parse_transform(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lyon_tessellation::path::Event;

    #[test]
    fn relative_paths_and_arcs_parse() {
        let path = path_data("M 10 10 l 5 0 h 5 v 5 a 5 5 0 0 1 -5 5 z m 1 1 L 2 2").unwrap();
        let mut ends = 0;
        let mut last = None;
        for ev in path.iter() {
            match ev {
                Event::End { .. } => ends += 1,
                Event::Line { to, .. } => last = Some(to),
                _ => {}
            }
        }
        assert_eq!(ends, 2);
        assert_eq!(last, Some(point(2.0, 2.0)));
        assert!(path_data("").is_none());
        assert!(path_data("M 1 1").is_none());
    }

    #[test]
    fn transforms_compose() {
        let t = parse_transform("translate(10 20) rotate(90)").unwrap();
        let (x, y) = apply(&t, 1.0, 0.0);
        assert!((x - 10.0).abs() < 1e-12 && (y - 21.0).abs() < 1e-12);
        let inv = invert(&t).unwrap();
        let (a, b) = apply(&inv, x, y);
        assert!((a - 1.0).abs() < 1e-12 && b.abs() < 1e-12);
        assert_eq!(mul(&IDENTITY, &t), t);
        assert_eq!(parse_points("1,2 3 4, 5"), vec![(1.0, 2.0), (3.0, 4.0)]);
    }
}
