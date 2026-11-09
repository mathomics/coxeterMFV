//! Small deterministic force relaxation for placing nodes inside a
//! compartment (port of `src/layout/force.js`).
//!
//! Three forces, in decreasing priority:
//!
//! ```text
//!   containment  keep every node inside its compartment and out of the organelles
//!                nested within it
//!   springs      pull a reaction marker towards the metabolites it touches
//!   repulsion    push overlapping nodes apart
//! ```
//!
//! Deterministic: the seeded PRNG only breaks exact ties, so the same model
//! always produces the same map.
//!
//! Repulsion only acts between nodes closer than `(r_a + r_b) * 3.2 + 14`.
//! The JavaScript compares every pair; here a uniform grid finds the pairs that
//! can be that close, and they are visited in exactly the order the all-pairs
//! loop visits them (by `i`, then by `j`), so the result is bit-identical while
//! the work grows with the number of close pairs instead of `n²`.

use coxeter_core::geometry::{clamp_to_rect, closest_perimeter_point, signed_distance};
use coxeter_core::js;
use coxeter_core::mapdata::{Point, Rect};

/// Mulberry32: tiny, seedable, good enough for jitter. Produces exactly the
/// sequence `makeRandom(seed)` produces in JavaScript.
#[derive(Debug, Clone)]
pub struct Random {
    a: u32,
}

/// JavaScript `ToUint32` (`x >>> 0`).
pub fn to_uint32(x: f64) -> u32 {
    if !x.is_finite() {
        return 0;
    }
    let t = x.trunc();
    let m = t.rem_euclid(4_294_967_296.0);
    m as u32
}

impl Random {
    /// A generator seeded with `seed`, taken as `seed >>> 0`.
    pub fn new(seed: f64) -> Random {
        Random { a: to_uint32(seed) }
    }

    /// The next number in [0, 1) (one call of the JavaScript closure).
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> f64 {
        self.a = self.a.wrapping_add(0x6d2b_79f5);
        let a = self.a;
        let mut t = (a ^ (a >> 15)).wrapping_mul(1 | a);
        t = t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t)) ^ t;
        f64::from(t ^ (t >> 14)) / 4_294_967_296.0
    }
}

/// `makeRandom(seed)`.
pub fn make_random(seed: f64) -> Random {
    Random::new(seed)
}

/// One body the relaxation moves.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Item {
    pub x: f64,
    pub y: f64,
    pub r: f64,
    /// Fixed items push and pull the others but never move.
    pub fixed: bool,
    /// The rectangle it must stay inside.
    pub bounds: Option<Rect>,
    /// Rectangles it must stay out of (nested organelles).
    pub holes: Vec<Rect>,
    /// Where rank anchoring pulls it, if anywhere.
    pub anchor_x: Option<f64>,
    pub anchor_y: Option<f64>,
}

impl Item {
    fn pos(&self) -> Point {
        Point::new(self.x, self.y)
    }
}

/// A spring between items `a` and `b`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Edge {
    pub a: usize,
    pub b: usize,
    pub length: f64,
    pub strength: f64,
}

/// The parameters of [`relax`], with the JavaScript defaults.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RelaxOptions {
    pub iterations: usize,
    pub repulsion: f64,
    pub seed: f64,
    pub rank_strength: f64,
}

impl Default for RelaxOptions {
    fn default() -> Self {
        RelaxOptions { iterations: 260, repulsion: 1.0, seed: 12345.0, rank_strength: 0.10 }
    }
}

/// `x || fallback` for a number: 0 and NaN are falsy.
fn or_else(x: f64, fallback: f64) -> f64 {
    if x == 0.0 || x.is_nan() { fallback } else { x }
}

/// Whether `relax` finds close pairs with the neighbour grid (the default) or
/// visits every pair as the JavaScript does.
static NEIGHBOUR_GRID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Turn the neighbour grid of [`relax`] on or off for the whole process.
/// Both give the same bits; this exists to measure the difference and to
/// prove the equality on real layouts. Not for normal use.
#[doc(hidden)]
pub fn set_neighbour_grid(enabled: bool) {
    NEIGHBOUR_GRID.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

/// Relax `items` in place (`relax(spec)`).
pub fn relax(items: &mut [Item], edges: &[Edge], opts: RelaxOptions) {
    relax_impl(items, edges, opts, NEIGHBOUR_GRID.load(std::sync::atomic::Ordering::Relaxed));
}

/// [`relax`] exactly as the JavaScript loops: every pair, every iteration.
pub fn relax_all_pairs(items: &mut [Item], edges: &[Edge], opts: RelaxOptions) {
    relax_impl(items, edges, opts, false);
}

/// `relax` with the neighbour grid on or off; both give the same bits.
fn relax_impl(items: &mut [Item], edges: &[Edge], opts: RelaxOptions, use_grid: bool) {
    let n = items.len();
    if n == 0 {
        return;
    }
    let RelaxOptions { iterations, repulsion, seed, rank_strength } = opts;
    let mut rand = make_random(seed);
    let mut vx = vec![0.0f64; n];
    let mut vy = vec![0.0f64; n];

    // Break exact coincidences so repulsion has a direction to work with.
    for it in items.iter_mut() {
        it.x += (rand.next() - 0.5) * 0.5;
        it.y += (rand.next() - 0.5) * 0.5;
    }

    let mut grid = NeighbourGrid::default();
    let iters = iterations as f64;
    for iter in 0..iterations {
        let cooling = 1.0 - iter as f64 / iters;
        let step = 0.55 * cooling + 0.08;
        vx.fill(0.0);
        vy.fill(0.0);

        // Springs
        for e in edges {
            if e.a >= n || e.b >= n {
                continue;
            }
            let (a, b) = (&items[e.a], &items[e.b]);
            let mut dx = b.x - a.x;
            let mut dy = b.y - a.y;
            let mut d = js::hypot(dx, dy);
            if d < 1e-6 {
                dx = 0.7;
                dy = 0.3;
                d = 0.76;
            }
            let k = e.strength * 0.13;
            let f = ((d - e.length) / d) * k;
            vx[e.a] += dx * f;
            vy[e.a] += dy * f;
            vx[e.b] -= dx * f;
            vy[e.b] -= dy * f;
        }

        // Repulsion, only between pairs within reach of each other
        grid.rebuild(items, use_grid);
        grid.for_each_close_pair(items, |i, j| {
            let (a, b) = (&items[i], &items[j]);
            let mut dx = b.x - a.x;
            let mut dy = b.y - a.y;
            let d2 = dx * dx + dy * dy;
            let want = (a.r + b.r) * 3.2 + 14.0;
            if d2 > want * want {
                return;
            }
            let mut d = d2.sqrt();
            if d < 1e-6 {
                dx = rand.next() - 0.5;
                dy = rand.next() - 0.5;
                d = 0.5;
            }
            let push = ((want - d) / d) * 0.85 * repulsion;
            vx[i] -= dx * push;
            vy[i] -= dy * push;
            vx[j] += dx * push;
            vy[j] += dy * push;
        });

        // Rank anchoring, which keeps pathways flowing top to bottom
        if rank_strength > 0.0 {
            for (i, it) in items.iter().enumerate() {
                let Some(anchor_y) = it.anchor_y else { continue };
                vy[i] += (anchor_y - it.y) * rank_strength * cooling;
                if let Some(anchor_x) = it.anchor_x {
                    vx[i] += (anchor_x - it.x) * rank_strength * 0.6 * cooling;
                }
            }
        }

        // Integration and containment
        for (i, it) in items.iter_mut().enumerate() {
            if it.fixed {
                continue;
            }
            it.x += js::max(-24.0, js::min(24.0, vx[i])) * step;
            it.y += js::max(-24.0, js::min(24.0, vy[i])) * step;

            if let Some(bounds) = it.bounds {
                let p = clamp_to_rect(it.pos(), &bounds, it.r + 6.0);
                it.x = p.x;
                it.y = p.y;
            }
            // Push out of nested organelles so the parent's nodes never sit on
            // top of a child compartment. This is a soft, integrated force
            // rather than a teleport: snapping every intruder to the nearest
            // edge piles them all up on the same stretch of membrane.
            for hi in 0..it.holes.len() {
                let hole = it.holes[hi];
                let clearance = it.r + 12.0;
                let sd = signed_distance(&hole, it.pos());
                if sd >= clearance {
                    continue;
                }
                let edge = closest_perimeter_point(&hole, it.pos(), 0.0);
                let mut ox = edge.nx;
                let mut oy = edge.ny;
                if !js_truthy_num(ox) && !js_truthy_num(oy) {
                    ox = if it.x < edge.x { -1.0 } else { 1.0 };
                    oy = 0.0;
                }
                let depth = clearance - sd;
                let gain = if sd < 0.0 { 0.55 } else { 0.22 };
                it.x += ox * depth * gain;
                it.y += oy * depth * gain;
            }
        }
    }

    // Final pass: anything still overlapping an organelle gets placed just
    // outside it, spread around the edge it is closest to.
    for it in items.iter_mut() {
        if it.fixed {
            continue;
        }
        for hi in 0..it.holes.len() {
            let hole = it.holes[hi];
            let clearance = it.r + 10.0;
            if signed_distance(&hole, it.pos()) >= clearance {
                continue;
            }
            let edge = closest_perimeter_point(&hole, it.pos(), 0.0);
            let ox = or_else(edge.nx, if it.x < edge.x { -1.0 } else { 1.0 });
            let oy = or_else(edge.ny, if it.y < edge.y { -1.0 } else { 1.0 });
            it.x = edge.x + ox * clearance;
            it.y = edge.y + oy * clearance;
            if let Some(bounds) = it.bounds {
                let p = clamp_to_rect(it.pos(), &bounds, it.r + 6.0);
                it.x = p.x;
                it.y = p.y;
            }
        }
    }
}

fn js_truthy_num(x: f64) -> bool {
    x != 0.0 && !x.is_nan()
}

/// The all-pairs loop of `relax`, `for i { for j > i { … } }`, restricted to
/// the pairs that can be within repulsion reach.
///
/// Items are bucketed in a uniform grid whose cell is the largest reach any
/// pair can have, so two items within reach lie in the same or adjacent cells.
/// For every `i` the candidates `j > i` from the 3×3 neighbourhood are sorted,
/// which is the order the nested loop would meet them in. Pairs the grid skips
/// are exactly the ones the loop would skip with `d2 > want²`; non-finite
/// coordinates (which make that test false in JavaScript) fall back to the
/// plain loop.
#[derive(Default)]
struct NeighbourGrid {
    cell: f64,
    min_x: f64,
    min_y: f64,
    cols: usize,
    rows: usize,
    /// Item indices per cell, ascending.
    buckets: Vec<Vec<usize>>,
    /// Cell of each item.
    cell_of: Vec<usize>,
    /// Fall back to visiting every pair.
    all_pairs: bool,
    scratch: Vec<usize>,
}

/// Below this many items the plain double loop is as fast as the grid.
const GRID_MIN_ITEMS: usize = 48;

impl NeighbourGrid {
    fn rebuild(&mut self, items: &[Item], use_grid: bool) {
        let n = items.len();
        self.all_pairs = !use_grid || n < GRID_MIN_ITEMS;
        if self.all_pairs {
            return;
        }
        let mut max_r = 0.0f64;
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        for it in items {
            if !(it.x.is_finite() && it.y.is_finite() && it.r.is_finite()) {
                self.all_pairs = true;
                return;
            }
            max_r = max_r.max(it.r);
            min_x = min_x.min(it.x);
            min_y = min_y.min(it.y);
            max_x = max_x.max(it.x);
            max_y = max_y.max(it.y);
        }
        // The largest `want` any pair can have, padded so rounding at a cell
        // edge can never hide a pair that is within reach.
        let cell = (2.0 * max_r * 3.2 + 14.0) * 1.0001 + 1e-9;
        let cols = (((max_x - min_x) / cell).floor() as usize).saturating_add(1);
        let rows = (((max_y - min_y) / cell).floor() as usize).saturating_add(1);
        // A degenerate spread (everything in one cell, or far too many cells)
        // gains nothing from the grid.
        if cols.saturating_mul(rows) > 4 * n + 1024 || cols * rows == 1 {
            self.all_pairs = true;
            return;
        }
        self.cell = cell;
        self.min_x = min_x;
        self.min_y = min_y;
        self.cols = cols;
        self.rows = rows;
        if self.buckets.len() < cols * rows {
            self.buckets.resize_with(cols * rows, Vec::new);
        }
        for b in &mut self.buckets[..cols * rows] {
            b.clear();
        }
        self.cell_of.clear();
        for (i, it) in items.iter().enumerate() {
            let cx = (((it.x - min_x) / cell).floor() as usize).min(cols - 1);
            let cy = (((it.y - min_y) / cell).floor() as usize).min(rows - 1);
            let c = cy * cols + cx;
            self.buckets[c].push(i);
            self.cell_of.push(c);
        }
    }

    fn for_each_close_pair(&mut self, items: &[Item], mut visit: impl FnMut(usize, usize)) {
        let n = items.len();
        if self.all_pairs {
            for i in 0..n {
                for j in (i + 1)..n {
                    visit(i, j);
                }
            }
            return;
        }
        let mut scratch = std::mem::take(&mut self.scratch);
        for i in 0..n {
            let c = self.cell_of[i];
            let (cx, cy) = (c % self.cols, c / self.cols);
            scratch.clear();
            for gy in cy.saturating_sub(1)..=(cy + 1).min(self.rows - 1) {
                for gx in cx.saturating_sub(1)..=(cx + 1).min(self.cols - 1) {
                    let bucket = &self.buckets[gy * self.cols + gx];
                    // Buckets are ascending: skip everything up to i.
                    let start = bucket.partition_point(|&j| j <= i);
                    scratch.extend_from_slice(&bucket[start..]);
                }
            }
            scratch.sort_unstable();
            for &j in &scratch {
                visit(i, j);
            }
        }
        self.scratch = scratch;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mulberry32_matches_javascript() {
        // node -e "…makeRandom(1)…" printed these.
        let mut r = make_random(1.0);
        let got: Vec<f64> = (0..3).map(|_| r.next()).collect();
        assert_eq!(got, [0.6270739405881613, 0.002735721180215478, 0.5274470399599522]);
        let mut r = make_random(20240501.0);
        assert_eq!(r.next(), 0.7720305563416332);
        let mut r = make_random(4294967296.0 + 5.0);
        assert_eq!(r.next(), 0.6897749109193683);
    }

    fn scatter(n: usize, seed: f64, spread: f64) -> Vec<Item> {
        let mut r = make_random(seed);
        (0..n)
            .map(|i| Item {
                x: r.next() * spread,
                y: r.next() * spread,
                r: if i % 3 == 0 { 5.0 } else { 13.0 },
                bounds: Some(Rect::new(0.0, 0.0, spread, spread)),
                ..Default::default()
            })
            .collect()
    }

    #[test]
    fn grid_gives_the_all_pairs_result_bit_for_bit() {
        for (n, spread) in [(60usize, 300.0), (200, 900.0), (400, 3000.0)] {
            let mut a = scatter(n, 7.0, spread);
            let mut b = a.clone();
            let edges: Vec<Edge> = (0..n / 2).map(|i| Edge { a: i, b: (i * 7 + 3) % n, length: 62.0, strength: 1.0 }).collect();
            let opts = RelaxOptions { iterations: 60, repulsion: 1.4, seed: 99.0, rank_strength: 0.0 };
            relax(&mut a, &edges, opts);
            relax_impl(&mut b, &edges, opts, false);
            for (p, q) in a.iter().zip(&b) {
                assert_eq!((p.x.to_bits(), p.y.to_bits()), (q.x.to_bits(), q.y.to_bits()));
            }
        }
    }
}
