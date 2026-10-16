//! Line hops (`src/render/line-hops.js`): where two reactions' lines cross,
//! the one drawn over the other jumps it with a small arc, so a crossing reads
//! as two lines passing and not as a junction where they meet.
//!
//! Works on each segment as drawn (a line or a cubic Bézier, already clipped
//! to its metabolite and arrowhead) and leaves the map's geometry alone: the
//! hop is in the picture only.

use std::collections::HashMap;

use coxeter_core::js;
use coxeter_core::mapdata::Point;
use indexmap::IndexMap;

use crate::format::n;

fn mix(a: Point, b: Point, t: f64) -> Point {
    Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
}

fn distance(a: Point, b: Point) -> f64 {
    js::hypot(a.x - b.x, a.y - b.y)
}

// Pieces a cubic is cut into to find where it crosses another: enough to
// follow the bends a branch is drawn with, few enough to run on every repaint.
const CURVE_PIECES: usize = 12;
// Side of the grid squares crossings are looked for in, in map units.
const CELL: f64 = 80.0;
/// More crossings than this is not a drawing anyone laid out to be read (a
/// genome-scale model drawn whole), and hops all over it only add noise.
pub const MAX_LINE_HOPS: usize = 1500;

/// `curvePoint(curve, t)`: the point at `t` on a line (2 points) or a cubic
/// Bézier (4 points).
pub fn curve_point(curve: &[Point], t: f64) -> Point {
    if curve.len() == 2 {
        return mix(curve[0], curve[1], t);
    }
    let a = mix(curve[0], curve[1], t);
    let b = mix(curve[1], curve[2], t);
    let c = mix(curve[2], curve[3], t);
    mix(mix(a, b, t), mix(b, c, t), t)
}

/// One segment as drawn, for [`find_line_hops`]: its line or cubic, the
/// reaction it belongs to and its stroke width.
#[derive(Debug, Clone, PartialEq)]
pub struct HopItem {
    pub curve: Vec<Point>,
    pub reaction_id: String,
    pub width: f64,
}

/// Where along a segment it hops, and how wide.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hop {
    pub t: f64,
    pub radius: f64,
}

#[derive(Debug, Clone, Copy)]
struct Piece {
    index: usize,
    t0: f64,
    t1: f64,
    a: Point,
    b: Point,
    before: f64,
    total: f64,
}

/// `findLineHops(items, { limit })`: where the lines cross, and which of each
/// pair hops. `items` are the segments in drawing order: of two that cross,
/// the later one is on top and is the one that hops. Hops by item index;
/// empty when there are more than `limit` crossings.
pub fn find_line_hops(items: &[HopItem], limit: usize) -> HashMap<usize, Vec<Hop>> {
    let mut pieces: Vec<Piece> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let curve = &item.curve;
        if curve.len() < 2 {
            continue;
        }
        let steps = if curve.len() == 4 { CURVE_PIECES } else { 1 };
        let mut previous = curve[0];
        let mut length = 0.0;
        let start = pieces.len();
        for k in 1..=steps {
            let t = k as f64 / steps as f64;
            let next = curve_point(curve, t);
            pieces.push(Piece { index, t0: (k - 1) as f64 / steps as f64, t1: t, a: previous, b: next, before: length, total: 0.0 });
            length += distance(previous, next);
            previous = next;
        }
        for piece in &mut pieces[start..] {
            piece.total = length;
        }
    }

    // The grid, in the order squares are first filled (a JS `Map`).
    let cell_of = |v: f64| (v / CELL).floor() as i64;
    let mut grid: IndexMap<(i64, i64), Vec<usize>> = IndexMap::new();
    for (k, piece) in pieces.iter().enumerate() {
        let x0 = cell_of(js::min(piece.a.x, piece.b.x));
        let x1 = cell_of(js::max(piece.a.x, piece.b.x));
        let y0 = cell_of(js::min(piece.a.y, piece.b.y));
        let y1 = cell_of(js::max(piece.a.y, piece.b.y));
        for gx in x0..=x1 {
            for gy in y0..=y1 {
                grid.entry((gx, gy)).or_default().push(k);
            }
        }
    }

    let mut hops: HashMap<usize, Vec<Hop>> = HashMap::new();
    let mut count = 0usize;
    for (&(gx, gy), bucket) in &grid {
        if bucket.len() < 2 {
            continue;
        }
        for i in 0..bucket.len() {
            let p = &pieces[bucket[i]];
            for &qk in &bucket[i + 1..] {
                let q = &pieces[qk];
                if p.index == q.index {
                    continue;
                }
                // A reaction's own branches meet at its centre; that is not a crossing.
                if items[p.index].reaction_id == items[q.index].reaction_id {
                    continue;
                }
                let Some(hit) = intersect(p.a, p.b, q.a, q.b) else { continue };
                // Each crossing is counted in the one grid square it lies in,
                // although both pieces are listed in every square they pass over.
                if cell_of(hit.x) != gx || cell_of(hit.y) != gy {
                    continue;
                }
                let (under, over) = if p.index < q.index { (p, q) } else { (q, p) };
                let radius = js::max(7.0, items[under.index].width / 2.0 + items[over.index].width / 2.0 + 2.0);
                // Lines that only meet where they end (at a metabolite they
                // share, or at the reaction they both run into) are not crossing
                // each other.
                if near_end(under, hit, radius) || near_end(over, hit, radius) {
                    continue;
                }
                let s = projection(over.a, over.b, hit);
                let t = over.t0 + (over.t1 - over.t0) * s;
                hops.entry(over.index).or_default().push(Hop { t, radius });
                count += 1;
                if count > limit {
                    return HashMap::new();
                }
            }
        }
    }
    hops
}

fn near_end(piece: &Piece, hit: Point, radius: f64) -> bool {
    let along = piece.before + distance(piece.a, hit);
    let margin = radius * 1.6;
    along < margin || piece.total - along < margin
}

fn projection(a: Point, b: Point, p: Point) -> f64 {
    let len = distance(a, b);
    if len > 1e-9 { distance(a, p) / len } else { 0.0 }
}

/// Where segments a–b and c–d cross, or `None`; touching and parallel lines
/// do not count.
fn intersect(a: Point, b: Point, c: Point, d: Point) -> Option<Point> {
    let rx = b.x - a.x;
    let ry = b.y - a.y;
    let sx = d.x - c.x;
    let sy = d.y - c.y;
    let denom = rx * sy - ry * sx;
    if denom.abs() < 1e-9 {
        return None;
    }
    let qx = c.x - a.x;
    let qy = c.y - a.y;
    let u = (qx * sy - qy * sx) / denom;
    let v = (qx * ry - qy * rx) / denom;
    if u <= 0.0 || u >= 1.0 || v <= 0.0 || v >= 1.0 {
        return None;
    }
    Some(Point::new(a.x + u * rx, a.y + u * ry))
}

/// `hoppedPath(curve, hops)`: the path of a segment with its hops: the same
/// line or curve, broken where it crosses another and joined again over it by
/// a half circle.
pub fn hopped_path(curve: &[Point], hops: &[Hop]) -> String {
    let mut sorted = hops.to_vec();
    sorted.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
    let mut intervals: Vec<(f64, f64)> = Vec::new();
    for hop in sorted {
        let speed = derivative_length(curve, hop.t);
        if speed < 1e-9 {
            continue;
        }
        let dt = hop.radius / speed;
        let from = js::max(0.0, hop.t - dt);
        let to = js::min(1.0, hop.t + dt);
        if from <= 0.0 || to >= 1.0 {
            continue;
        }
        // Two crossings closer than a hop apart make one longer hop.
        match intervals.last_mut() {
            Some(last) if from <= last.1 => last.1 = js::max(last.1, to),
            _ => intervals.push((from, to)),
        }
    }

    let start = curve[0];
    let mut parts = vec![format!("M {} {}", n(start.x), n(start.y))];
    let mut cursor = 0.0;
    for (from, to) in intervals {
        parts.push(piece_path(curve, cursor, from));
        let p1 = curve_point(curve, from);
        let p2 = curve_point(curve, to);
        let r = distance(p1, p2) / 2.0;
        parts.push(format!("A {} {} 0 0 {} {} {}", n(r), n(r), arc_sweep(p1, p2), n(p2.x), n(p2.y)));
        cursor = to;
    }
    parts.push(piece_path(curve, cursor, 1.0));
    parts.join(" ")
}

/// Hops bulge upward, or to the left on a vertical line, wherever the line runs.
fn arc_sweep(p1: Point, p2: Point) -> u8 {
    let dx = p2.x - p1.x;
    let dy = p2.y - p1.y;
    if dx.abs() > dy.abs() * 0.05 {
        return if dx > 0.0 { 1 } else { 0 };
    }
    if dy > 0.0 { 0 } else { 1 }
}

/// The stretch of the curve from `a` to `b`, as a path command continuing
/// from its start.
fn piece_path(curve: &[Point], a: f64, b: f64) -> String {
    if curve.len() == 2 {
        let end = curve_point(curve, b);
        return format!("L {} {}", n(end.x), n(end.y));
    }
    let [_, c1, c2, end] = sub_cubic(curve, a, b);
    format!("C {} {}, {} {}, {} {}", n(c1.x), n(c1.y), n(c2.x), n(c2.y), n(end.x), n(end.y))
}

/// The cubic between parameters `a` and `b` of another, by de Casteljau.
fn sub_cubic(curve: &[Point], a: f64, b: f64) -> [Point; 4] {
    let (left, _) = split([curve[0], curve[1], curve[2], curve[3]], b);
    if b <= 1e-12 {
        return [curve[0]; 4];
    }
    split(left, a / b).1
}

fn split([p0, p1, p2, p3]: [Point; 4], t: f64) -> ([Point; 4], [Point; 4]) {
    let a = mix(p0, p1, t);
    let b = mix(p1, p2, t);
    let c = mix(p2, p3, t);
    let d = mix(a, b, t);
    let e = mix(b, c, t);
    let f = mix(d, e, t);
    ([p0, a, d, f], [f, e, c, p3])
}

fn derivative_length(curve: &[Point], t: f64) -> f64 {
    if curve.len() == 2 {
        return distance(curve[0], curve[1]);
    }
    let (p0, p1, p2, p3) = (curve[0], curve[1], curve[2], curve[3]);
    let u = 1.0 - t;
    let dx = 3.0 * u * u * (p1.x - p0.x) + 6.0 * u * t * (p2.x - p1.x) + 3.0 * t * t * (p3.x - p2.x);
    let dy = 3.0 * u * u * (p1.y - p0.y) + 6.0 * u * t * (p2.y - p1.y) + 3.0 * t * t * (p3.y - p2.y);
    js::hypot(dx, dy)
}

#[cfg(test)]
mod tests {
    //! The line-hop tests of `test/line-styles.test.mjs`.
    use super::*;

    fn p(x: f64, y: f64) -> Point {
        Point::new(x, y)
    }

    fn item(curve: Vec<Point>, reaction: &str) -> HopItem {
        HopItem { curve, reaction_id: reaction.into(), width: 7.5 }
    }

    #[test]
    fn of_two_reactions_whose_lines_cross_the_one_drawn_later_hops_the_other() {
        let items = [item(vec![p(0.0, 50.0), p(100.0, 50.0)], "A"), item(vec![p(50.0, 0.0), p(50.0, 100.0)], "B")];
        let hops = find_line_hops(&items, MAX_LINE_HOPS);
        assert_eq!(hops.keys().copied().collect::<Vec<_>>(), [1]);
        let hop = hops[&1][0];
        assert!((hop.t - 0.5).abs() < 1e-9);
        assert!(hop.radius >= 7.0);
    }

    #[test]
    fn a_cubic_branch_crossing_a_line_is_found_where_it_crosses() {
        let curve = vec![p(0.0, 0.0), p(40.0, 120.0), p(80.0, -40.0), p(120.0, 80.0)];
        let hops = find_line_hops(&[item(vec![p(0.0, 40.0), p(120.0, 40.0)], "A"), item(curve.clone(), "B")], MAX_LINE_HOPS);
        let found = hops.get(&1).expect("a hop");
        assert!(!found.is_empty());
        for hop in found {
            assert!((curve_point(&curve, hop.t).y - 40.0).abs() < 2.0, "on the line it crosses");
        }
    }

    #[test]
    fn lines_of_one_reaction_and_lines_that_only_meet_where_they_end_do_not_hop() {
        let own = [item(vec![p(0.0, 50.0), p(100.0, 50.0)], "A"), item(vec![p(50.0, 0.0), p(50.0, 100.0)], "A")];
        assert!(find_line_hops(&own, MAX_LINE_HOPS).is_empty(), "a reaction's own branches meet at its centre");
        let ends = [item(vec![p(0.0, 0.0), p(100.0, 100.0)], "A"), item(vec![p(100.0, 101.0), p(200.0, 0.0)], "B")];
        assert!(find_line_hops(&ends, MAX_LINE_HOPS).is_empty(), "two lines ending at one metabolite");
    }

    #[test]
    fn past_the_limit_a_map_gets_no_hops_at_all() {
        let items = [item(vec![p(0.0, 50.0), p(100.0, 50.0)], "A"), item(vec![p(50.0, 0.0), p(50.0, 100.0)], "B")];
        assert!(find_line_hops(&items, 0).is_empty());
    }

    #[test]
    fn a_hopped_path_starts_and_ends_where_the_line_does_and_jumps_with_a_half_circle() {
        let d = hopped_path(&[p(0.0, 50.0), p(100.0, 50.0)], &[Hop { t: 0.5, radius: 10.0 }]);
        assert_eq!(d, "M 0 50 L 40 50 A 10 10 0 0 1 60 50 L 100 50", "bulging upward on a line running right");
        let curve = [p(0.0, 0.0), p(30.0, 90.0), p(70.0, 90.0), p(100.0, 0.0)];
        let hopped = hopped_path(&curve, &[Hop { t: 0.5, radius: 8.0 }]);
        let re = regex::Regex::new(r"^M 0 0 C [^A]* A [\d.]+ [\d.]+ 0 0 [01] [\d.]+ [\d.]+ C .*, 100 0$").unwrap();
        assert!(re.is_match(&hopped), "{hopped}");
    }
}
