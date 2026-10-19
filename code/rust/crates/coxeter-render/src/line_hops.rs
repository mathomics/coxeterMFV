//! Line hops drawn as gaps (`src/render/line-hops.js`): where two reactions'
//! lines cross, the line drawn underneath is cut on both sides of the one on
//! top, which stays whole. A crossing then reads as one line passing over
//! another and not as a junction where they meet. On thick, curved lines a gap
//! stays clean where an arc hopping over the other line would look like a
//! bubble.
//!
//! Works on each segment as drawn (a line or a cubic Bézier, already clipped
//! to its metabolite and arrowhead) and leaves the map's geometry alone: the
//! gap is in the picture only.

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
/// genome-scale model drawn whole), and gaps all over it only add noise.
pub const MAX_LINE_HOPS: usize = 1500;

// Clear space between each cut end of the lower line and the edge of the line
// on top, in map units.
const GAP_CLEAR: f64 = 3.5;
// A line crossing at a shallow angle runs beside the other for longer, so its
// gap is longer by 1/sin of the angle, which is floored here to cap the gap.
const GAP_MIN_SINE: f64 = 0.3;
// A gap keeps this far from either end of a line, so it never cuts into the
// metabolite or arrowhead there.
const END_MARGIN: f64 = 4.0;

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

/// A gap cut in a segment: centred at curve parameter `t`, `half` map units
/// long on either side.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hop {
    pub t: f64,
    pub half: f64,
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

/// `findLineHops(items, { limit })`: where the lines cross, and the gap each
/// crossing cuts in the lower line. `items` are the segments in drawing
/// order, widths in map units: of two that cross, the earlier one is
/// underneath and is the one that gets the gap. Gaps by item index, empty
/// when there are more than `limit` crossings.
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
                let over_width = items[over.index].width;
                let under_width = items[under.index].width;
                // The cut has to clear the top line's half width and a gap,
                // measured along the lower line, plus the round cap each cut
                // end is drawn with.
                let sine = js::max(GAP_MIN_SINE, crossing_sine(p.a, p.b, q.a, q.b));
                let half = (over_width / 2.0 + GAP_CLEAR) / sine + under_width / 2.0;
                // Lines that only meet where they end (at a metabolite they
                // share, or at the reaction they both run into) are not crossing
                // each other.
                if near_end(under, hit, half) || near_end(over, hit, under_width / 2.0 + GAP_CLEAR) {
                    continue;
                }
                let s = projection(under.a, under.b, hit);
                let t = under.t0 + (under.t1 - under.t0) * s;
                hops.entry(under.index).or_default().push(Hop { t, half });
                count += 1;
                if count > limit {
                    return HashMap::new();
                }
            }
        }
    }
    hops
}

/// The sine of the angle between segments a–b and c–d, from 0 (parallel) to 1.
fn crossing_sine(a: Point, b: Point, c: Point, d: Point) -> f64 {
    let (ux, uy) = (b.x - a.x, b.y - a.y);
    let (vx, vy) = (d.x - c.x, d.y - c.y);
    let lengths = js::hypot(ux, uy) * js::hypot(vx, vy);
    if lengths > 1e-12 { (ux * vy - uy * vx).abs() / lengths } else { 0.0 }
}

/// Whether `hit` lies within `reach` (plus `END_MARGIN`) of either end of its line.
fn near_end(piece: &Piece, hit: Point, reach: f64) -> bool {
    let along = piece.before + distance(piece.a, hit);
    let margin = reach + END_MARGIN;
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

/// `hoppedPath(curve, hops)`: the path of a segment with its gaps: the same
/// line or curve, cut where another passes over it and picked up again on the
/// other side.
pub fn hopped_path(curve: &[Point], hops: &[Hop]) -> String {
    let mut sorted = hops.to_vec();
    sorted.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
    let mut intervals: Vec<(f64, f64)> = Vec::new();
    for hop in sorted {
        let speed = derivative_length(curve, hop.t);
        if speed < 1e-9 {
            continue;
        }
        let dt = hop.half / speed;
        let from = js::max(0.0, hop.t - dt);
        let to = js::min(1.0, hop.t + dt);
        if from <= 0.0 || to >= 1.0 {
            continue;
        }
        // Two crossings closer than a gap apart make one longer gap.
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
        let resume = curve_point(curve, to);
        parts.push(format!("M {} {}", n(resume.x), n(resume.y)));
        cursor = to;
    }
    parts.push(piece_path(curve, cursor, 1.0));
    parts.join(" ")
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
    fn of_two_reactions_whose_lines_cross_the_one_drawn_earlier_is_cut_where_the_later_one_passes_over() {
        let items = [item(vec![p(0.0, 50.0), p(100.0, 50.0)], "A"), item(vec![p(50.0, 0.0), p(50.0, 100.0)], "B")];
        let hops = find_line_hops(&items, MAX_LINE_HOPS);
        assert_eq!(hops.keys().copied().collect::<Vec<_>>(), [0]);
        let gap = hops[&0][0];
        assert!((gap.t - 0.5).abs() < 1e-9);
        // The cut clears the top line's half width, a gap and its own round cap.
        assert!(gap.half > 7.5 / 2.0 + 7.5 / 2.0, "gap half-length {}", gap.half);
    }

    #[test]
    fn a_shallow_crossing_gets_a_longer_gap_than_a_square_one() {
        let square = find_line_hops(&[item(vec![p(0.0, 50.0), p(100.0, 50.0)], "A"), item(vec![p(50.0, 0.0), p(50.0, 100.0)], "B")], MAX_LINE_HOPS)[&0][0].half;
        let shallow = find_line_hops(&[item(vec![p(0.0, 50.0), p(200.0, 50.0)], "A"), item(vec![p(0.0, 0.0), p(200.0, 100.0)], "B")], MAX_LINE_HOPS)[&0][0].half;
        assert!(shallow > square * 1.5, "{shallow} against {square}");
    }

    #[test]
    fn a_cubic_branch_under_a_line_is_cut_where_it_crosses() {
        let curve = vec![p(0.0, 0.0), p(40.0, 120.0), p(80.0, -40.0), p(120.0, 80.0)];
        let hops = find_line_hops(&[item(curve.clone(), "B"), item(vec![p(0.0, 40.0), p(120.0, 40.0)], "A")], MAX_LINE_HOPS);
        let found = hops.get(&0).expect("a gap");
        assert!(!found.is_empty());
        for gap in found {
            assert!((curve_point(&curve, gap.t).y - 40.0).abs() < 2.0, "on the line it crosses");
        }
    }

    #[test]
    fn lines_of_one_reaction_and_lines_that_only_meet_where_they_end_are_not_cut() {
        let own = [item(vec![p(0.0, 50.0), p(100.0, 50.0)], "A"), item(vec![p(50.0, 0.0), p(50.0, 100.0)], "A")];
        assert!(find_line_hops(&own, MAX_LINE_HOPS).is_empty(), "a reaction's own branches meet at its centre");
        let ends = [item(vec![p(0.0, 0.0), p(100.0, 100.0)], "A"), item(vec![p(100.0, 101.0), p(200.0, 0.0)], "B")];
        assert!(find_line_hops(&ends, MAX_LINE_HOPS).is_empty(), "two lines ending at one metabolite");
    }

    #[test]
    fn past_the_limit_a_map_gets_no_gaps_at_all() {
        let items = [item(vec![p(0.0, 50.0), p(100.0, 50.0)], "A"), item(vec![p(50.0, 0.0), p(50.0, 100.0)], "B")];
        assert!(find_line_hops(&items, 0).is_empty());
    }

    #[test]
    fn a_cut_path_starts_and_ends_where_the_line_does_and_leaves_a_gap_where_it_is_crossed() {
        let d = hopped_path(&[p(0.0, 50.0), p(100.0, 50.0)], &[Hop { t: 0.5, half: 10.0 }]);
        assert_eq!(d, "M 0 50 L 40 50 M 60 50 L 100 50");
        let curve = [p(0.0, 0.0), p(30.0, 90.0), p(70.0, 90.0), p(100.0, 0.0)];
        let cut = hopped_path(&curve, &[Hop { t: 0.5, half: 8.0 }]);
        let re = regex::Regex::new(r"^M 0 0 C [^M]* M [\d.]+ [\d.]+ C .*, 100 0$").unwrap();
        assert!(re.is_match(&cut), "{cut}");
        assert_eq!(hopped_path(&[p(0.0, 50.0), p(100.0, 50.0)], &[Hop { t: 0.03, half: 10.0 }]), "M 0 50 L 100 50", "no gap runs off the end");
    }
}
