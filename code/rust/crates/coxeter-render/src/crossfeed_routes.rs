//! Routes for the cross-feeding lines, drawn like the lines of a metro map
//! (`src/render/crossfeed-routes.js`).
//!
//! A line runs from the producer's medium node of the metabolite to the
//! consumer's, through the medium between the cells and never across or behind
//! one. It is made of straight horizontal and vertical runs: it leaves each node
//! straight out from the membrane, and its corners are rounded when drawn.
//!
//! Routing has two steps. First every line finds its way on a coarse grid that
//! prefers the middle of the space between cells and few corners; lines that go
//! the same way therefore share the same corridors and form bundles. Then each
//! straight run gets a lane of its own, the free lane nearest the middle of its
//! corridor, so the lines of a bundle run side by side and stay straight for the
//! whole run instead of stepping round each other.
//!
//! Routing depends only on the geometry and on which links exist, not on the
//! flux they carry, so a new solution with the same exchanges reuses the routes.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex};

use coxeter_core::community::CrossFeedingLink;
use coxeter_core::js;
use coxeter_core::mapdata::{MapCell, MapGraph, Node, NodeKind, Point, Rect};
use indexmap::IndexMap;
use regex::Regex;

/// Exchanged metabolites that get no line: protons and water go everywhere.
const SKIP_BASES: [&str; 5] = ["h", "h+", "h2o", "cpd00001", "cpd00067"];

/// `isDrawnCrossFeedingBase(base)`.
pub fn is_drawn_cross_feeding_base(base: &str) -> bool {
    let lower = base.to_lowercase();
    !SKIP_BASES.contains(&lower.as_str())
}

// Four directions, clockwise from east; d & 1 is the axis (0 horizontal, 1 vertical).
const DX: [i64; 4] = [1, 0, -1, 0];
const DY: [i64; 4] = [0, 1, 0, -1];
const COARSE: f64 = 4.0; // fine cells per coarse cell
const TURN: f64 = 6.0; // a corner costs this many coarse steps
const MEDIAL: f64 = 1.5; // extra cost per step for each cell closer to a cell than CLEAR
const CLEAR: f64 = 3.0; // coarse cells from the nearest cell that count as the middle
const MAX_POPS: usize = 400000;
const MAX_LANE: i64 = 40; // lanes tried on each side of the middle
const CROWD: f64 = 0.12; // extra cost per line already routed through a coarse cell the same way
const OVER: f64 = 4.0; // extra cost per line beyond the lanes a corridor has room for

/// Grid size the router uses for this map; lanes are a little closer than this.
pub fn route_grid(map: &MapGraph) -> f64 {
    js::max(24.0, js::min(120.0, js::round(js::max(map.width(), map.height()) / 280.0)))
}

/// `CROSSFEED_WIDTH_RANGE`: the slider's range for the cross-feeding line
/// width, as a multiple of the default. Wider than 1.5 the lanes no longer fit
/// the corridors between the cells of a crowded map, and lines are pushed into
/// each other's lanes.
pub struct CrossFeedWidthRange {
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub default: f64,
}

pub const CROSSFEED_WIDTH_RANGE: CrossFeedWidthRange = CrossFeedWidthRange { min: 0.25, max: 1.5, step: 0.05, default: 1.0 };

/// `crossFeedingLineWidth(layout, scale)`: how wide a cross-feeding line is
/// drawn, in map units, at `scale` times the default. The default is 0.8 of the
/// routing grid, which with the whole map in view is about as thick on screen
/// as a cell's boundary.
pub fn cross_feeding_line_width(map: &MapGraph, scale: f64) -> f64 {
    let k = if scale.is_finite() { js::max(CROSSFEED_WIDTH_RANGE.min, js::min(CROSSFEED_WIDTH_RANGE.max, scale)) } else { 1.0 };
    0.8 * route_grid(map) * k
}

/// `laneSpacing(grid, lineWidth)`: distance between neighbouring lanes, in map
/// units: half a line's width of gap between two lines side by side, and never
/// closer than 0.6 grid, so thin lines keep the bundles they always had.
pub fn lane_spacing(grid: f64, line_width: f64) -> f64 {
    let w = if line_width.is_nan() { 0.0 } else { line_width };
    js::round(js::max(0.6 * grid, 1.5 * w))
}

/// Where a line starts or ends: a medium node (or a point beside a cell),
/// with the outward normal of the membrane it faces and how far out from it
/// it sits.
#[derive(Debug, Clone, PartialEq)]
pub struct Endpoint {
    pub x: f64,
    pub y: f64,
    pub nx: f64,
    pub ny: f64,
    pub gap: f64,
    pub r: f64,
    /// The medium node's name; none for a point on a cell's side.
    pub name: Option<String>,
}

/// One routed line: `points` runs from node centre to node centre through
/// the corners.
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    pub points: Vec<Point>,
    pub start: Endpoint,
    pub end: Endpoint,
}

/// Routes keyed `from|base|to`.
pub type Routes = IndexMap<String, Route>;

static MEDIUM_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^M@(.+)_s$").unwrap());

fn medium_base(node: &Node) -> String {
    if let Some(m) = node.merged_id.as_deref().and_then(|id| MEDIUM_ID.captures(id)) {
        return m[1].to_string();
    }
    node.base.clone().or_else(|| node.local_id.clone()).unwrap_or_else(|| node.id.clone())
}

fn outward_normal(rect: &Rect, x: f64, y: f64) -> (f64, f64, f64) {
    let mut gaps = [
        (x - (rect.x + rect.w), 1.0, 0.0),
        (rect.x - x, -1.0, 0.0),
        (y - (rect.y + rect.h), 0.0, 1.0),
        (rect.y - y, 0.0, -1.0),
    ];
    // Stable, largest gap first, as `gaps.sort((a, b) => b[0] - a[0])`.
    gaps.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    (gaps[0].1, gaps[0].2, gaps[0].0)
}

/// `mediumEndpoints(layout)`: the medium node each cell exchanges a
/// metabolite through, keyed `cellId|base`.
pub fn medium_endpoints(map: &MapGraph) -> IndexMap<String, Endpoint> {
    let cells: HashMap<&str, &MapCell> = map.cells.iter().map(|c| (c.id.as_str(), c)).collect();
    let mut out = IndexMap::new();
    for seg in &map.segments {
        let Some(node) = seg.node_id.as_deref().and_then(|id| map.nodes.get(id)) else { continue };
        if node.kind != NodeKind::Medium {
            continue;
        }
        let Some(cell) = map
            .reactions
            .get(&seg.reaction_id)
            .and_then(|r| r.cell_id.as_deref())
            .and_then(|id| cells.get(id))
        else {
            continue;
        };
        let key = format!("{}|{}", cell.id, medium_base(node));
        if out.contains_key(&key) {
            continue;
        }
        let (nx, ny, gap) = outward_normal(&cell.rect, node.x, node.y);
        out.insert(key, Endpoint { x: node.x, y: node.y, nx, ny, gap, r: node.r, name: node.name.clone() });
    }
    out
}

struct Wanted {
    key: String,
    a: Endpoint,
    b: Endpoint,
}

struct Cache {
    key: String,
    routes: Arc<Routes>,
}

static CACHE: LazyLock<Mutex<Option<Cache>>> = LazyLock::new(|| Mutex::new(None));

/// `routeCrossFeeding(layout, links, { lineWidth })`: a route for every link
/// that can be routed, keyed `from|base|to`; a link left out could not be
/// routed. `line_width` is what the lines are drawn at, in map units (the JS
/// default is [`cross_feeding_line_width`] at 1), and sets how far apart lanes are.
///
/// The last result is kept: asked again with the same geometry, links and
/// width, the same routes come back (the same `Arc`), whatever the fluxes now are.
pub fn route_cross_feeding(map: &MapGraph, links: &[CrossFeedingLink], line_width: f64) -> Arc<Routes> {
    let endpoints = medium_endpoints(map);
    let cells: Vec<&MapCell> = map.cells.iter().collect();
    let mut wanted = Vec::new();
    for link in links {
        let (Some(from), Some(to)) = (link.from.as_deref(), link.to.as_deref()) else { continue };
        if from.is_empty() || to.is_empty() || !is_drawn_cross_feeding_base(&link.base) {
            continue;
        }
        let a = endpoints.get(&format!("{from}|{}", link.base)).cloned().or_else(|| perimeter_endpoint(&cells, from, to));
        let b = endpoints.get(&format!("{to}|{}", link.base)).cloned().or_else(|| perimeter_endpoint(&cells, to, from));
        let (Some(a), Some(b)) = (a, b) else { continue };
        wanted.push(Wanted { key: format!("{from}|{}|{to}", link.base), a, b });
    }
    wanted.sort_by(|p, q| js::code_unit_compare(&p.key, &q.key));
    let lane = lane_spacing(route_grid(map), line_width);
    let mut key = format!(
        "{}x{};{};",
        js::number_to_string(map.width()),
        js::number_to_string(map.height()),
        js::number_to_string(lane)
    );
    key.push_str(
        &cells
            .iter()
            .map(|c| {
                format!(
                    "{}:{},{},{},{}",
                    c.id,
                    js::number_to_string(c.rect.x),
                    js::number_to_string(c.rect.y),
                    js::number_to_string(c.rect.w),
                    js::number_to_string(c.rect.h)
                )
            })
            .collect::<Vec<_>>()
            .join(";"),
    );
    key.push(';');
    key.push_str(
        &wanted
            .iter()
            .map(|w| {
                format!(
                    "{}@{},{},{},{}",
                    w.key,
                    js::number_to_string(js::round(w.a.x)),
                    js::number_to_string(js::round(w.a.y)),
                    js::number_to_string(js::round(w.b.x)),
                    js::number_to_string(js::round(w.b.y))
                )
            })
            .collect::<Vec<_>>()
            .join(";"),
    );
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(c) = cache.as_ref().filter(|c| c.key == key) {
        return c.routes.clone();
    }
    let routes = Arc::new(compute_routes(map, &cells, &wanted, &endpoints, lane));
    *cache = Some(Cache { key, routes: routes.clone() });
    routes
}

/// A cell with no medium node for the metabolite: the middle of the side facing the partner.
fn perimeter_endpoint(cells: &[&MapCell], id: &str, other_id: &str) -> Option<Endpoint> {
    let a = cells.iter().find(|c| c.id == id)?;
    let b = cells.iter().find(|c| c.id == other_id)?;
    let ca = Point::new(a.rect.x + a.rect.w / 2.0, a.rect.y + a.rect.h / 2.0);
    let cb = Point::new(b.rect.x + b.rect.w / 2.0, b.rect.y + b.rect.h / 2.0);
    let dx = cb.x - ca.x;
    let dy = cb.y - ca.y;
    let sign_or_one = |v: f64| {
        let s = js::sign(v);
        if s == 0.0 || s.is_nan() { 1.0 } else { s }
    };
    if dx.abs() * a.rect.h >= dy.abs() * a.rect.w {
        let sx = sign_or_one(dx);
        return Some(Endpoint { x: ca.x + sx * (a.rect.w / 2.0 + 40.0), y: ca.y, nx: sx, ny: 0.0, gap: 40.0, r: 0.0, name: None });
    }
    let sy = sign_or_one(dy);
    Some(Endpoint { x: ca.x, y: ca.y + sy * (a.rect.h / 2.0 + 40.0), nx: 0.0, ny: sy, gap: 40.0, r: 0.0, name: None })
}

/// A grid over the map, with the cells (and a margin round them) blocked.
struct Grid {
    size: f64,
    cols: usize,
    rows: usize,
    blocked: Vec<u8>,
    n: usize,
}

fn make_grid(w: f64, h: f64, size: f64, cells: &[&MapCell], margin: f64) -> Grid {
    let cols = (w / size).ceil() as usize + 1;
    let rows = (h / size).ceil() as usize + 1;
    let mut blocked = vec![0u8; cols * rows];
    for c in cells {
        let x0 = js::max(0.0, ((c.rect.x - margin) / size).floor());
        let x1 = js::min(cols as f64 - 1.0, ((c.rect.x + c.rect.w + margin) / size).ceil());
        let y0 = js::max(0.0, ((c.rect.y - margin) / size).floor());
        let y1 = js::min(rows as f64 - 1.0, ((c.rect.y + c.rect.h + margin) / size).ceil());
        if !(x0 <= x1 && y0 <= y1) {
            continue;
        }
        for y in (y0 as usize)..=(y1 as usize) {
            for x in (x0 as usize)..=(x1 as usize) {
                blocked[y * cols + x] = 1;
            }
        }
    }
    Grid { size, cols, rows, blocked, n: cols * rows }
}

/// Steps from each grid cell to the nearest blocked cell or the edge of the map.
#[allow(clippy::needless_range_loop)]
fn clearance(grid: &Grid) -> Vec<u16> {
    let Grid { cols, rows, n, .. } = *grid;
    let mut dist = vec![0xffffu16; n];
    let mut queue = Vec::new();
    for c in 0..n {
        let x = c % cols;
        let y = c / cols;
        if grid.blocked[c] != 0 || x == 0 || y == 0 || x == cols - 1 || y == rows - 1 {
            dist[c] = 0;
            queue.push(c);
        }
    }
    let mut i = 0;
    while i < queue.len() {
        let c = queue[i];
        i += 1;
        let x = (c % cols) as i64;
        let y = (c / cols) as i64;
        for d in 0..4 {
            let nx = x + DX[d];
            let ny = y + DY[d];
            if nx < 0 || ny < 0 || nx >= cols as i64 || ny >= rows as i64 {
                continue;
            }
            let nc = ny as usize * cols + nx as usize;
            if (dist[nc] as u32) > dist[c] as u32 + 1 {
                dist[nc] = dist[c] + 1;
                queue.push(nc);
            }
        }
    }
    dist
}

/// The cross-sections of the free space: for a horizontal run through a grid
/// cell, the free stretch of the cell's column; for a vertical run, the free
/// stretch of its row. `id[axis][cell]` names the cross-section and
/// `lanes[axis][id]` is how many lanes fit across it, less the margins along
/// the cells.
struct Sections {
    id: [Vec<i32>; 2],
    lanes: [Vec<f64>; 2],
}

fn cross_sections(grid: &Grid, margin: f64, lane: f64) -> Sections {
    let Grid { cols, rows, size, n, .. } = *grid;
    let mut id = [vec![-1i32; n], vec![-1i32; n]];
    let mut lanes: [Vec<f64>; 2] = [Vec::new(), Vec::new()];
    let mut fill = |cells_of: &dyn Fn(usize) -> usize, count: usize, axis: usize| {
        let mut k = 0;
        while k < count {
            if grid.blocked[cells_of(k)] != 0 {
                k += 1;
                continue;
            }
            let mut e = k;
            while e < count && grid.blocked[cells_of(e)] == 0 {
                e += 1;
            }
            let sid = lanes[axis].len() as i32;
            lanes[axis].push(js::max(1.0, (((e - k) as f64 * size - 2.0 * margin) / lane).floor()));
            for j in k..e {
                id[axis][cells_of(j)] = sid;
            }
            k = e;
        }
    };
    for x in 0..cols {
        fill(&|y| y * cols + x, rows, 0);
    }
    for y in 0..rows {
        fill(&|x| y * cols + x, cols, 1);
    }
    Sections { id, lanes }
}

/// The binary heap of the A* search, keyed by cost. A port of the JS one, so
/// that ties are broken in the same order.
#[derive(Default)]
struct MinHeap {
    ids: Vec<usize>,
    keys: Vec<f64>,
}

impl MinHeap {
    fn len(&self) -> usize {
        self.ids.len()
    }
    fn clear(&mut self) {
        self.ids.clear();
        self.keys.clear();
    }
    fn push(&mut self, id: usize, key: f64) {
        let mut i = self.ids.len();
        self.ids.push(id);
        self.keys.push(key);
        while i > 0 {
            let p = (i - 1) >> 1;
            if self.keys[p] <= key {
                break;
            }
            self.ids[i] = self.ids[p];
            self.keys[i] = self.keys[p];
            i = p;
        }
        self.ids[i] = id;
        self.keys[i] = key;
    }
    fn pop(&mut self) -> usize {
        let top = self.ids[0];
        let id = self.ids.pop().expect("non-empty heap");
        let key = self.keys.pop().expect("non-empty heap");
        let n = self.ids.len();
        if n > 0 {
            let mut i = 0;
            loop {
                let l = 2 * i + 1;
                if l >= n {
                    break;
                }
                let r = l + 1;
                let c = if r < n && self.keys[r] < self.keys[l] { r } else { l };
                if self.keys[c] >= key {
                    break;
                }
                self.ids[i] = self.ids[c];
                self.keys[i] = self.keys[c];
                i = c;
            }
            self.ids[i] = id;
            self.keys[i] = key;
        }
        top
    }
}

struct SearchBuf {
    g: Vec<f64>,
    seen: Vec<u32>,
    prev: Vec<i64>,
    heap: MinHeap,
    stamp: u32,
}

/// A* over (grid cell, direction) states, from `start` leaving in `start_dir` to
/// `goal`, arriving in any direction but straight out of `goal_dir`.
/// `extra(cell, dir)` prices stepping into a cell. Returns the states, or None.
fn search(
    grid: &Grid,
    buf: &mut SearchBuf,
    start: usize,
    start_dir: usize,
    goal: usize,
    goal_dir: usize,
    extra: &dyn Fn(usize, usize) -> f64,
) -> Option<Vec<usize>> {
    let Grid { cols, rows, .. } = *grid;
    buf.stamp += 1;
    let stamp = buf.stamp;
    buf.heap.clear();
    let tx = (goal % cols) as i64;
    let ty = (goal / cols) as i64;
    let h = |c: usize| ((c % cols) as i64 - tx).abs() as f64 + ((c / cols) as i64 - ty).abs() as f64;
    let s0 = start * 4 + start_dir;
    buf.g[s0] = 0.0;
    buf.seen[s0] = stamp;
    buf.prev[s0] = -1;
    buf.heap.push(s0, h(start));
    let mut pops = 0;
    while buf.heap.len() > 0 {
        let s = buf.heap.pop();
        pops += 1;
        if pops > MAX_POPS {
            return None;
        }
        let cell = s >> 2;
        let d = s & 3;
        if cell == goal {
            if d == (goal_dir + 2) % 4 {
                continue;
            }
            let mut out = Vec::new();
            let mut k = s as i64;
            while k >= 0 {
                out.push(k as usize);
                k = buf.prev[k as usize];
            }
            out.reverse();
            return Some(out);
        }
        let gs = buf.g[s];
        let cx = (cell % cols) as i64;
        let cy = (cell / cols) as i64;
        for nd in 0..4 {
            if nd == (d + 2) % 4 {
                continue;
            }
            let nx = cx + DX[nd];
            let ny = cy + DY[nd];
            if nx < 0 || ny < 0 || nx >= cols as i64 || ny >= rows as i64 {
                continue;
            }
            let nc = ny as usize * cols + nx as usize;
            if nc != goal && grid.blocked[nc] != 0 {
                continue;
            }
            let ns = nc * 4 + nd;
            let ng = gs + 1.0 + (if nd == d { 0.0 } else { TURN }) + extra(nc, nd);
            if buf.seen[ns] == stamp && buf.g[ns] <= ng {
                continue;
            }
            buf.seen[ns] = stamp;
            buf.g[ns] = ng;
            buf.prev[ns] = s as i64;
            buf.heap.push(ns, ng + h(nc));
        }
    }
    None
}

/// One straight run of a line, and the lane it gets.
struct Run {
    axis: usize,
    want: f64,
    coord: Option<f64>,
    fixed: bool,
    line: usize,
    i: usize,
}

struct Line {
    w: usize,
    runs: Vec<usize>,
}

/// A run's claim on its lane, over the stretch `lo..hi` along it.
#[derive(Clone, Copy)]
struct Entry {
    run: usize,
    coord: f64,
    lo: f64,
    hi: f64,
}

fn dir_of(nx: f64, ny: f64) -> usize {
    for d in 0..4 {
        if DX[d] as f64 == js::sign(nx) && DY[d] as f64 == js::sign(ny) {
            return d;
        }
    }
    0
}

/// The lane assignment (the second routing step), over the runs of every line.
struct Lanes<'a> {
    wanted: &'a [Wanted],
    lines: Vec<Line>,
    runs: Vec<Run>,
    placed: [Vec<Entry>; 2],
    fine: &'a Grid,
    g: f64,
    lane: f64,
}

impl Lanes<'_> {
    fn coord_of(&self, r: usize) -> f64 {
        self.runs[r].coord.unwrap_or(self.runs[r].want)
    }

    fn extent(&self, r: usize, est: bool) -> (f64, f64) {
        let run = &self.runs[r];
        let line = &self.lines[run.line];
        let w = &self.wanted[line.w];
        let n = line.runs.len();
        let end = |e: &Endpoint| if run.axis == 0 { e.x } else { e.y };
        let neighbour = |j: usize| if est { self.coord_of(j) } else { self.runs[j].coord.unwrap_or(0.0) };
        let p = if run.i == 0 { end(&w.a) } else { neighbour(line.runs[run.i - 1]) };
        let q = if run.i == n - 1 { end(&w.b) } else { neighbour(line.runs[run.i + 1]) };
        (js::min(p, q), js::max(p, q))
    }

    fn clashes(&self, r: usize, coord: f64, lo: f64, hi: f64) -> usize {
        let lane = self.lane;
        self.placed[self.runs[r].axis]
            .iter()
            .filter(|e| e.run != r && (e.coord - coord).abs() < lane * 0.95 && e.lo < hi + lane / 2.0 && lo < e.hi + lane / 2.0)
            .count()
    }

    fn open(&self, axis: usize, coord: f64, lo: f64, hi: f64) -> bool {
        let fine = self.fine;
        let g = self.g;
        let mut t = lo;
        while t <= hi + 1e-6 {
            let x = if axis == 0 { t } else { coord };
            let y = if axis == 0 { coord } else { t };
            let gx = js::round(x / g);
            let gy = js::round(y / g);
            if gx < 0.0 || gy < 0.0 || gx >= fine.cols as f64 || gy >= fine.rows as f64 {
                return false;
            }
            // A NaN index reads `undefined` in JS: not blocked.
            if !gx.is_nan() && !gy.is_nan() && fine.blocked[gy as usize * fine.cols + gx as usize] != 0 {
                return false;
            }
            t += g / 2.0;
        }
        true
    }

    /// Which side of its corridor a run leans to: the side its line comes from and
    /// the side it turns to. A run between two turns to the same side (a U) keeps
    /// to that side, so it does not cut across the lines that go straight on.
    fn side(&self, r: usize) -> f64 {
        let run = &self.runs[r];
        let line = &self.lines[run.line];
        let w = &self.wanted[line.w];
        let len = line.runs.len() as i64;
        let far = |j: usize, toward: i64| -> f64 {
            // the far end of the perpendicular run j, beyond its corner with r
            let t = if toward < 0 { 0 } else { len - 1 };
            let k = j as i64 + toward;
            if k < 0 || k >= len {
                return if run.axis == 0 {
                    if t == 0 { w.a.y } else { w.b.y }
                } else if t == 0 {
                    w.a.x
                } else {
                    w.b.x
                };
            }
            self.coord_of(line.runs[k as usize])
        };
        let mut s = 0.0;
        if run.i > 0 {
            s += js::sign(far(run.i - 1, -1) - self.coord_of(r));
        }
        if (run.i as i64) < len - 1 {
            s += js::sign(far(run.i + 1, 1) - self.coord_of(r));
        }
        s
    }

    fn place(&mut self, r: usize, est: bool) {
        let (lo, hi) = self.extent(r, est);
        let s = self.side(r);
        let lean = if s < 0.0 { -1.0 } else { 1.0 };
        // a U tries its own side first, then the other; anything else alternates
        let u = s.abs() == 2.0;
        let offset = |k: i64| -> f64 {
            if u {
                if k <= MAX_LANE { lean * k as f64 } else { -lean * (k - MAX_LANE) as f64 }
            } else if k % 2 != 0 {
                (k + 1) as f64 / 2.0
            } else {
                -(k as f64) / 2.0
            }
        };
        // the nearest lane that is free; failing that, the open lane with fewest lines on it
        let axis = self.runs[r].axis;
        let want = self.runs[r].want;
        let mut best: Option<f64> = None;
        let mut fallback: Option<f64> = None;
        let mut fewest = usize::MAX;
        let mut k = 0;
        while k <= 2 * MAX_LANE && best.is_none() {
            let coord = want + offset(k) * self.lane;
            k += 1;
            if !self.open(axis, coord, lo, hi) {
                continue;
            }
            let n = self.clashes(r, coord, lo, hi);
            if n == 0 {
                best = Some(coord);
            } else if n < fewest {
                fewest = n;
                fallback = Some(coord);
            }
        }
        let coord = best.or(fallback).unwrap_or(want);
        self.runs[r].coord = Some(coord);
        self.placed[axis].push(Entry { run: r, coord, lo, hi });
    }

    fn entry_index(&self, r: usize) -> usize {
        let axis = self.runs[r].axis;
        self.placed[axis].iter().position(|e| e.run == r).expect("every run has its lane")
    }
}

fn compute_routes(map: &MapGraph, cells: &[&MapCell], wanted: &[Wanted], endpoints: &IndexMap<String, Endpoint>, lane: f64) -> Routes {
    let mut routes = Routes::new();
    if wanted.is_empty() {
        return routes;
    }
    let g = route_grid(map);
    // How far out the medium nodes sit; the lines keep further out than that.
    let mut gaps: Vec<f64> = endpoints.values().map(|e| e.gap).filter(|x| *x > 0.0).collect();
    gaps.sort_by(|p, q| p.partial_cmp(q).unwrap_or(std::cmp::Ordering::Equal));
    let node_gap = if gaps.is_empty() { 0.0 } else { gaps[gaps.len() / 2] };
    let margin = node_gap + 1.6 * g;
    let fine = make_grid(map.width(), map.height(), g, cells, margin);
    // The coarse grid blocks the cells alone: rounding to its large squares
    // already adds a margin, and the lines must still be able to reach it.
    let coarse = make_grid(map.width(), map.height(), g * COARSE, cells, 0.0);
    let gc = coarse.size;
    let clear = clearance(&coarse);
    // lines through each coarse cell, per axis, against the lanes there is room
    // for: later lines spread to quieter corridors, and leave full ones alone
    let mut crowd = [vec![0u16; coarse.n], vec![0u16; coarse.n]];
    let sections = cross_sections(&coarse, margin, lane);
    let mut used = [vec![0u16; sections.lanes[0].len()], vec![0u16; sections.lanes[1].len()]];
    let mut buf = SearchBuf {
        g: vec![0.0; coarse.n * 4],
        seen: vec![0; coarse.n * 4],
        prev: vec![0; coarse.n * 4],
        heap: MinHeap::default(),
        stamp: 0,
    };
    let to_coarse = |x: f64, y: f64| -> usize {
        let row = js::max(0.0, js::min(coarse.rows as f64 - 1.0, (y / gc).floor()));
        let col = js::max(0.0, js::min(coarse.cols as f64 - 1.0, (x / gc).floor()));
        row as usize * coarse.cols + col as usize
    };
    // the first free coarse cell straight out from a node
    let exit_cell = |e: &Endpoint| -> usize {
        let mut x = e.x + e.nx * (margin + g);
        let mut y = e.y + e.ny * (margin + g);
        let mut c = to_coarse(x, y);
        let mut k = 0;
        while k < 60 && coarse.blocked[c] != 0 {
            x += e.nx * gc;
            y += e.ny * gc;
            c = to_coarse(x, y);
            k += 1;
        }
        c
    };

    // Coarse routes, cut into straight runs. Short links first: they take the
    // direct way, and the long ones spread round them.
    let mut lines: Vec<Line> = Vec::new();
    let mut runs: Vec<Run> = Vec::new();
    let reach = |w: &Wanted| (w.a.x - w.b.x).abs() + (w.a.y - w.b.y).abs();
    let mut order: Vec<usize> = (0..wanted.len()).collect();
    order.sort_by(|&p, &q| reach(&wanted[p]).partial_cmp(&reach(&wanted[q])).unwrap_or(std::cmp::Ordering::Equal));
    for wi in order {
        let w = &wanted[wi];
        let da = dir_of(w.a.nx, w.a.ny);
        let inward = (dir_of(w.b.nx, w.b.ny) + 2) % 4;
        let states = {
            let price = |c: usize, d: usize| -> f64 {
                let a = d & 1;
                let sid = sections.id[a][c];
                let over = if sid < 0 {
                    0.0
                } else {
                    js::max(0.0, used[a][sid as usize] as f64 + 1.0 - sections.lanes[a][sid as usize])
                };
                MEDIAL * js::max(0.0, CLEAR - clear[c] as f64) + CROWD * crowd[a][c] as f64 + OVER * over
            };
            search(&coarse, &mut buf, exit_cell(&w.a), da, exit_cell(&w.b), inward, &price)
        };
        let Some(states) = states else { continue };
        // a line counts once in each cross-section it passes through
        let mut counted: HashSet<(usize, i32)> = HashSet::new();
        for &s in &states {
            let a = s & 1;
            let c = s >> 2;
            crowd[a][c] = crowd[a][c].wrapping_add(1);
            let sid = sections.id[a][c];
            if sid >= 0 && counted.insert((a, sid)) {
                used[a][sid as usize] = used[a][sid as usize].wrapping_add(1);
            }
        }
        let mut moves: Vec<(usize, usize)> = states.iter().map(|&s| (s & 3, s >> 2)).collect();
        let last_c = moves[moves.len() - 1].1;
        moves.push((inward, last_c));
        let line_index = lines.len();
        let mut line_runs: Vec<(usize, f64)> = Vec::new();
        for (d, c) in moves {
            let axis = d & 1;
            if line_runs.last().is_some_and(|r| r.0 == axis) {
                continue;
            }
            let want = if axis == 0 {
                ((c / coarse.cols) as f64 + 0.5) * gc
            } else {
                ((c % coarse.cols) as f64 + 0.5) * gc
            };
            line_runs.push((axis, want));
        }
        let along = |e: &Endpoint, axis: usize| if axis == 0 { e.y } else { e.x };
        if line_runs.len() == 1 {
            // straight across: a jog in the middle when the nodes are not in line
            let axis = line_runs[0].0;
            if (along(&w.a, axis) - along(&w.b, axis)).abs() > 1.0 {
                let mid = if axis == 0 { (w.a.x + w.b.x) / 2.0 } else { (w.a.y + w.b.y) / 2.0 };
                line_runs.push((1 - axis, mid));
                line_runs.push((axis, 0.0));
            }
        }
        let first_id = runs.len();
        let count = line_runs.len();
        for (i, (axis, want)) in line_runs.into_iter().enumerate() {
            runs.push(Run { axis, want, coord: None, fixed: false, line: line_index, i });
        }
        let last_id = first_id + count - 1;
        runs[first_id].fixed = true;
        runs[first_id].coord = Some(along(&w.a, runs[first_id].axis));
        runs[last_id].fixed = true;
        runs[last_id].coord = Some(along(&w.b, runs[last_id].axis));
        lines.push(Line { w: wi, runs: (first_id..=last_id).collect() });
    }

    // A lane for every run
    let mut lanes = Lanes { wanted, lines, runs, placed: [Vec::new(), Vec::new()], fine: &fine, g, lane };
    let mut free: Vec<usize> = Vec::new();
    for li in 0..lanes.lines.len() {
        for k in 0..lanes.lines[li].runs.len() {
            let r = lanes.lines[li].runs[k];
            if lanes.runs[r].fixed {
                let (lo, hi) = lanes.extent(r, true);
                let coord = lanes.runs[r].coord.expect("fixed runs have their coordinate");
                let axis = lanes.runs[r].axis;
                lanes.placed[axis].push(Entry { run: r, coord, lo, hi });
            } else {
                free.push(r);
            }
        }
    }
    // long runs first: the trunks of the bundles take the middle lanes
    let spans: HashMap<usize, f64> = free
        .iter()
        .map(|&r| {
            let (lo, hi) = lanes.extent(r, true);
            (r, hi - lo)
        })
        .collect();
    free.sort_by(|p, q| spans[q].partial_cmp(&spans[p]).unwrap_or(std::cmp::Ordering::Equal));
    for &r in &free {
        lanes.place(r, true);
    }
    // the extents are now known exactly: re-place any run that turned out to clash
    for _pass in 0..3 {
        for axis in 0..2 {
            for k in 0..lanes.placed[axis].len() {
                let run = lanes.placed[axis][k].run;
                let (lo, hi) = lanes.extent(run, false);
                lanes.placed[axis][k].lo = lo;
                lanes.placed[axis][k].hi = hi;
            }
        }
        let mut moved = 0;
        for &r in &free {
            let axis = lanes.runs[r].axis;
            let entry = lanes.placed[axis][lanes.entry_index(r)];
            let coord = lanes.runs[r].coord.expect("placed");
            if lanes.clashes(r, coord, entry.lo, entry.hi) == 0 && lanes.open(axis, coord, entry.lo, entry.hi) {
                continue;
            }
            let at = lanes.entry_index(r);
            lanes.placed[axis].remove(at);
            lanes.runs[r].want = coord;
            lanes.place(r, false);
            moved += 1;
        }
        if moved == 0 {
            break;
        }
    }

    // Corners
    for line in &lanes.lines {
        let w = &wanted[line.w];
        let mut pts = vec![Point::new(w.a.x, w.a.y)];
        for i in 0..line.runs.len().saturating_sub(1) {
            let r = &lanes.runs[line.runs[i]];
            let s = &lanes.runs[line.runs[i + 1]];
            let (rc, sc) = (r.coord.unwrap_or(f64::NAN), s.coord.unwrap_or(f64::NAN));
            pts.push(if r.axis == 0 { Point::new(sc, rc) } else { Point::new(rc, sc) });
        }
        pts.push(Point::new(w.b.x, w.b.y));
        routes.insert(w.key.clone(), Route { points: dedupe(&pts), start: w.a.clone(), end: w.b.clone() });
    }
    routes
}

/// Drops repeated points and points in the middle of a straight stretch.
fn dedupe(all: &[Point]) -> Vec<Point> {
    let mut out = vec![all[0]];
    for &q in &all[1..] {
        let p = out[out.len() - 1];
        if js::hypot(q.x - p.x, q.y - p.y) < 1e-6 {
            continue;
        }
        if out.len() >= 2 {
            let o = out[out.len() - 2];
            let cross = (p.x - o.x) * (q.y - p.y) - (p.y - o.y) * (q.x - p.x);
            let dot = (p.x - o.x) * (q.x - p.x) + (p.y - o.y) * (q.y - p.y);
            if cross.abs() < 1e-6 && dot >= 0.0 {
                let last = out.len() - 1;
                out[last] = q;
                continue;
            }
        }
        out.push(q);
    }
    out
}

/// `routeAnchor(points, at)`: the point of a route nearest to `at`, or the
/// middle of its longest run.
pub fn route_anchor(points: &[Point], at: Option<Point>) -> Point {
    // (distance or length, x, y)
    let mut best: Option<(f64, f64, f64)> = None;
    for k in 1..points.len() {
        let p = points[k - 1];
        let q = points[k];
        let len = js::hypot(q.x - p.x, q.y - p.y);
        if len < 1e-6 {
            continue;
        }
        let Some(at) = at else {
            if best.is_none_or(|b| len > b.0) {
                best = Some((len, (p.x + q.x) / 2.0, (p.y + q.y) / 2.0));
            }
            continue;
        };
        let t = js::max(0.0, js::min(1.0, ((at.x - p.x) * (q.x - p.x) + (at.y - p.y) * (q.y - p.y)) / (len * len)));
        let x = p.x + t * (q.x - p.x);
        let y = p.y + t * (q.y - p.y);
        let d = js::hypot(at.x - x, at.y - y);
        if best.is_none_or(|b| d < b.0) {
            best = Some((d, x, y));
        }
    }
    match best {
        Some((_, x, y)) => Point::new(x, y),
        None => points[0],
    }
}

/// `roundedPath(points, radius)`: SVG path through the corners, each corner
/// rounded off.
pub fn rounded_path(points: &[Point], radius: f64) -> String {
    if points.len() < 2 {
        return String::new();
    }
    let f = |v: f64| js::number_to_string(js::round(v * 10.0) / 10.0);
    let mut d = format!("M {} {}", f(points[0].x), f(points[0].y));
    for k in 1..points.len() - 1 {
        let p = points[k - 1];
        let q = points[k];
        let r = points[k + 1];
        let l1 = js::hypot(q.x - p.x, q.y - p.y);
        let l2 = js::hypot(r.x - q.x, r.y - q.y);
        if l1 < 1e-6 || l2 < 1e-6 {
            continue;
        }
        let rr = js::min_n([radius, l1 / 2.0, l2 / 2.0]);
        let a = Point::new(q.x - (q.x - p.x) / l1 * rr, q.y - (q.y - p.y) / l1 * rr);
        let b = Point::new(q.x + (r.x - q.x) / l2 * rr, q.y + (r.y - q.y) / l2 * rr);
        d.push_str(&format!(" L {} {} Q {} {} {} {}", f(a.x), f(a.y), f(q.x), f(q.y), f(b.x), f(b.y)));
    }
    let last = points[points.len() - 1];
    d.push_str(&format!(" L {} {}", f(last.x), f(last.y)));
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    // From test/crossfeed-routes.test.mjs.
    #[test]
    fn protons_and_water_get_no_line() {
        assert!(!is_drawn_cross_feeding_base("h"));
        assert!(!is_drawn_cross_feeding_base("h2o"));
        assert!(!is_drawn_cross_feeding_base("cpd00067"));
        assert!(!is_drawn_cross_feeding_base("H2O"));
        assert!(is_drawn_cross_feeding_base("ac"));
    }

    #[test]
    fn a_picked_line_is_named_where_it_was_clicked_or_mid_way_along_its_longest_run() {
        let line = [Point::new(0.0, 0.0), Point::new(0.0, 100.0), Point::new(400.0, 100.0)];
        assert_eq!(route_anchor(&line, Some(Point::new(250.0, 130.0))), Point::new(250.0, 100.0));
        assert_eq!(route_anchor(&line, Some(Point::new(-20.0, 40.0))), Point::new(0.0, 40.0));
        assert_eq!(route_anchor(&line, None), Point::new(200.0, 100.0));
    }

    #[test]
    fn heap_pops_in_key_order() {
        let mut h = MinHeap::default();
        for (id, key) in [(1, 5.0), (2, 1.0), (3, 3.0), (4, 1.0), (5, 0.5)] {
            h.push(id, key);
        }
        let mut out = Vec::new();
        while h.len() > 0 {
            out.push(h.pop());
        }
        assert_eq!(out[0], 5);
        assert_eq!(out[4], 1);
    }
}
