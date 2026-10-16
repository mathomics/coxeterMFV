/**
 * Routes for the cross-feeding lines, drawn like the lines of a metro map.
 *
 * A line runs from the producer's medium node of the metabolite to the
 * consumer's, through the medium between the cells and never across or behind
 * one. It is made of straight horizontal and vertical runs: it leaves each node
 * straight out from the membrane, and its corners are rounded when drawn.
 *
 * Routing has two steps. First every line finds its way on a coarse grid that
 * prefers the middle of the space between cells and few corners; lines that go
 * the same way therefore share the same corridors and form bundles. Then each
 * straight run gets a lane of its own, the free lane nearest the middle of its
 * corridor, so the lines of a bundle run side by side and stay straight for the
 * whole run instead of stepping round each other.
 *
 * Routing depends only on the geometry and on which links exist, not on the
 * flux they carry, so a new solution with the same exchanges reuses the routes.
 */

/** Exchanged metabolites that get no line: protons and water go everywhere. */
const SKIP_BASES = new Set(['h', 'h+', 'h2o', 'cpd00001', 'cpd00067']);

/** Whether a link for this exchanged metabolite base gets a line at all. */
export function isDrawnCrossFeedingBase(base) {
  return !SKIP_BASES.has(String(base ?? '').toLowerCase());
}

// Four directions, clockwise from east. d & 1 is the axis (0 horizontal, 1 vertical).
const DX = [1, 0, -1, 0];
const DY = [0, 1, 0, -1];
const COARSE = 4;        // fine cells per coarse cell
const TURN = 6;          // a corner costs this many coarse steps
const MEDIAL = 1.5;      // extra cost per step for each cell closer to a cell than CLEAR
const CLEAR = 3;         // coarse cells from the nearest cell that count as the middle
const MAX_POPS = 400000;
const MAX_LANE = 40;     // lanes tried on each side of the middle
const CROWD = 0.12;      // extra cost per line already routed through a coarse cell the same way
const OVER = 4;          // extra cost per line beyond the lanes a corridor has room for

let cache = { key: '', routes: null };

/** Grid size the router uses for this map, in map units. Lanes are a little closer than this. */
export function routeGrid(layout) {
  return Math.max(24, Math.min(120, Math.round(Math.max(layout.width, layout.height) / 280)));
}

/**
 * The medium node each cell exchanges a metabolite through, keyed
 * `${cellId}|${base}`, with the outward normal of the membrane it faces and
 * how far out from it the node sits.
 */
export function mediumEndpoints(layout) {
  const cells = new Map((layout.cells ?? []).map((c) => [c.id, c]));
  const reactions = layout.reactions instanceof Map ? layout.reactions
    : new Map((layout.reactions ?? []).map((r) => [r.id, r]));
  const nodes = layout.nodes instanceof Map ? layout.nodes
    : new Map((layout.nodes ?? []).map((nd) => [nd.id, nd]));
  const out = new Map();
  for (const seg of layout.segments ?? []) {
    const node = seg.nodeId ? nodes.get(seg.nodeId) : null;
    if (!node || node.kind !== 'medium') continue;
    const cell = cells.get(reactions.get(seg.reactionId)?.cellId);
    if (!cell?.rect) continue;
    const key = `${cell.id}|${mediumBase(node)}`;
    if (out.has(key)) continue;
    out.set(key, { x: node.x, y: node.y, ...outwardNormal(cell.rect, node), r: node.r ?? 17, name: node.name ?? null });
  }
  return out;
}

function mediumBase(node) {
  const m = /^M@(.+)_s$/.exec(node.mergedId ?? '');
  return m ? m[1] : (node.base ?? node.localId ?? node.id);
}

function outwardNormal(rect, p) {
  const gaps = [
    [p.x - (rect.x + rect.w), 1, 0],
    [rect.x - p.x, -1, 0],
    [p.y - (rect.y + rect.h), 0, 1],
    [rect.y - p.y, 0, -1],
  ];
  gaps.sort((a, b) => b[0] - a[0]);
  return { nx: gaps[0][1], ny: gaps[0][2], gap: gaps[0][0] };
}

/**
 * Route every drawable cross-feeding link. The last result is reused while the
 * map size, the cells and the link endpoints stay the same.
 *
 * @param {object} layout the map being drawn
 * @param {{from: string, to: string, base: string}[]} links
 * @returns {Map<string, {points: {x:number,y:number}[], start: object, end: object}>}
 *   keyed `${from}|${base}|${to}`. `points` runs from node centre to node
 *   centre through the corners, and `start` and `end` are the nodes with their
 *   outward normals. A link left out could not be routed.
 */
export function routeCrossFeeding(layout, links) {
  const endpoints = mediumEndpoints(layout);
  const cells = (layout.cells ?? []).filter((c) => c.rect);
  const wanted = [];
  for (const link of links) {
    if (!link.from || !link.to || !isDrawnCrossFeedingBase(link.base)) continue;
    const a = endpoints.get(`${link.from}|${link.base}`) ?? perimeterEndpoint(cells, link.from, link.to);
    const b = endpoints.get(`${link.to}|${link.base}`) ?? perimeterEndpoint(cells, link.to, link.from);
    if (!a || !b) continue;
    wanted.push({ key: `${link.from}|${link.base}|${link.to}`, a, b });
  }
  wanted.sort((p, q) => (p.key < q.key ? -1 : p.key > q.key ? 1 : 0));
  const key = layout.width + 'x' + layout.height + ';'
    + cells.map((c) => `${c.id}:${c.rect.x},${c.rect.y},${c.rect.w},${c.rect.h}`).join(';') + ';'
    + wanted.map((w) => `${w.key}@${Math.round(w.a.x)},${Math.round(w.a.y)},${Math.round(w.b.x)},${Math.round(w.b.y)}`).join(';');
  if (cache.key === key) return cache.routes;
  const routes = computeRoutes(layout, cells, wanted, endpoints);
  cache = { key, routes };
  return routes;
}

/** A cell with no medium node for the metabolite: the middle of the side facing the partner. */
function perimeterEndpoint(cells, id, otherId) {
  const a = cells.find((c) => c.id === id);
  const b = cells.find((c) => c.id === otherId);
  if (!a || !b) return null;
  const ca = { x: a.rect.x + a.rect.w / 2, y: a.rect.y + a.rect.h / 2 };
  const cb = { x: b.rect.x + b.rect.w / 2, y: b.rect.y + b.rect.h / 2 };
  const dx = cb.x - ca.x;
  const dy = cb.y - ca.y;
  if (Math.abs(dx) * a.rect.h >= Math.abs(dy) * a.rect.w) {
    const sx = Math.sign(dx) || 1;
    return { x: ca.x + sx * (a.rect.w / 2 + 40), y: ca.y, nx: sx, ny: 0, gap: 40, r: 0 };
  }
  const sy = Math.sign(dy) || 1;
  return { x: ca.x, y: ca.y + sy * (a.rect.h / 2 + 40), nx: 0, ny: sy, gap: 40, r: 0 };
}

/** A grid over the map, with the cells (and a margin round them) blocked. */
function makeGrid(W, H, size, cells, margin) {
  const cols = Math.ceil(W / size) + 1;
  const rows = Math.ceil(H / size) + 1;
  const blocked = new Uint8Array(cols * rows);
  for (const c of cells) {
    const x0 = Math.max(0, Math.floor((c.rect.x - margin) / size));
    const x1 = Math.min(cols - 1, Math.ceil((c.rect.x + c.rect.w + margin) / size));
    const y0 = Math.max(0, Math.floor((c.rect.y - margin) / size));
    const y1 = Math.min(rows - 1, Math.ceil((c.rect.y + c.rect.h + margin) / size));
    for (let y = y0; y <= y1; y++) for (let x = x0; x <= x1; x++) blocked[y * cols + x] = 1;
  }
  return { size, cols, rows, blocked, n: cols * rows };
}

/** Steps from each grid cell to the nearest blocked cell or the edge of the map. */
function clearance(grid) {
  const { cols, rows, blocked, n } = grid;
  const dist = new Uint16Array(n).fill(0xffff);
  const queue = [];
  for (let c = 0; c < n; c++) {
    const x = c % cols;
    const y = (c / cols) | 0;
    if (blocked[c] || x === 0 || y === 0 || x === cols - 1 || y === rows - 1) { dist[c] = 0; queue.push(c); }
  }
  for (let i = 0; i < queue.length; i++) {
    const c = queue[i];
    const x = c % cols;
    const y = (c / cols) | 0;
    for (let d = 0; d < 4; d++) {
      const nx = x + DX[d];
      const ny = y + DY[d];
      if (nx < 0 || ny < 0 || nx >= cols || ny >= rows) continue;
      const nc = ny * cols + nx;
      if (dist[nc] > dist[c] + 1) { dist[nc] = dist[c] + 1; queue.push(nc); }
    }
  }
  return dist;
}

/**
 * The cross-sections of the free space: for a horizontal run through a grid
 * cell, the free stretch of the cell's column; for a vertical run, the free
 * stretch of its row. `id[axis][cell]` names the cross-section and
 * `lanes[axis][id]` is how many lanes fit across it, less the margins along
 * the cells.
 */
function crossSections(grid, margin, lane) {
  const { cols, rows, blocked, size, n } = grid;
  const id = [new Int32Array(n).fill(-1), new Int32Array(n).fill(-1)];
  const lanes = [[], []];
  const fill = (cellsOf, count, axis) => {
    let k = 0;
    while (k < count) {
      if (blocked[cellsOf(k)]) { k++; continue; }
      let e = k;
      while (e < count && !blocked[cellsOf(e)]) e++;
      const sid = lanes[axis].length;
      lanes[axis].push(Math.max(1, Math.floor(((e - k) * size - 2 * margin) / lane)));
      for (let j = k; j < e; j++) id[axis][cellsOf(j)] = sid;
      k = e;
    }
  };
  for (let x = 0; x < cols; x++) fill((y) => y * cols + x, rows, 0);
  for (let y = 0; y < rows; y++) fill((x) => y * cols + x, cols, 1);
  return { id, lanes };
}

/**
 * A* over (grid cell, direction) states, from `start` leaving in `startDir` to
 * `goal`, arriving in any direction but straight out of `goalDir`.
 * `extra(cell, dir)` prices stepping into a cell. Returns the states, or null.
 */
function search(grid, buf, start, startDir, goal, goalDir, extra) {
  const { cols, rows, blocked } = grid;
  const { g, seen, prev, heap } = buf;
  const stamp = ++buf.stamp;
  heap.clear();
  const tx = goal % cols;
  const ty = (goal / cols) | 0;
  const h = (c) => Math.abs((c % cols) - tx) + Math.abs(((c / cols) | 0) - ty);
  const s0 = start * 4 + startDir;
  g[s0] = 0; seen[s0] = stamp; prev[s0] = -1;
  heap.push(s0, h(start));
  let pops = 0;
  while (heap.size) {
    const s = heap.pop();
    if (++pops > MAX_POPS) return null;
    const cell = s >> 2;
    const d = s & 3;
    if (cell === goal) {
      if (d === (goalDir + 2) % 4) continue;
      const out = [];
      for (let k = s; k >= 0; k = prev[k]) out.push(k);
      return out.reverse();
    }
    const gs = g[s];
    const cx = cell % cols;
    const cy = (cell / cols) | 0;
    for (let nd = 0; nd < 4; nd++) {
      if (nd === (d + 2) % 4) continue;
      const nx = cx + DX[nd];
      const ny = cy + DY[nd];
      if (nx < 0 || ny < 0 || nx >= cols || ny >= rows) continue;
      const nc = ny * cols + nx;
      if (nc !== goal && blocked[nc]) continue;
      const ns = nc * 4 + nd;
      const ng = gs + 1 + (nd === d ? 0 : TURN) + extra(nc, nd);
      if (seen[ns] === stamp && g[ns] <= ng) continue;
      seen[ns] = stamp; g[ns] = ng; prev[ns] = s;
      heap.push(ns, ng + h(nc));
    }
  }
  return null;
}

function computeRoutes(layout, cells, wanted, endpoints) {
  const routes = new Map();
  if (!wanted.length) return routes;
  const G = routeGrid(layout);
  const LANE = Math.round(0.6 * G);
  // How far out the medium nodes sit. The lines keep further out than that.
  const gaps = [...endpoints.values()].map((e) => e.gap).filter((x) => x > 0).sort((p, q) => p - q);
  const nodeGap = gaps.length ? gaps[Math.floor(gaps.length / 2)] : 0;
  const margin = nodeGap + 1.6 * G;
  const fine = makeGrid(layout.width, layout.height, G, cells, margin);
  // The coarse grid blocks the cells alone: rounding to its large squares
  // already adds a margin, and the lines must still be able to reach it.
  const coarse = makeGrid(layout.width, layout.height, G * COARSE, cells, 0);
  const Gc = coarse.size;
  const clear = clearance(coarse);
  // Lines through each coarse cell, per axis, against the lanes there is room
  // for. Later lines spread to quieter corridors and leave full ones alone.
  const crowd = [new Uint16Array(coarse.n), new Uint16Array(coarse.n)];
  const sections = crossSections(coarse, margin, LANE);
  const used = [new Uint16Array(sections.lanes[0].length), new Uint16Array(sections.lanes[1].length)];
  const price = (c, d) => {
    const a = d & 1;
    const sid = sections.id[a][c];
    const over = sid < 0 ? 0 : Math.max(0, used[a][sid] + 1 - sections.lanes[a][sid]);
    return MEDIAL * Math.max(0, CLEAR - clear[c]) + CROWD * crowd[a][c] + OVER * over;
  };
  const buf = {
    g: new Float64Array(coarse.n * 4), seen: new Uint32Array(coarse.n * 4),
    prev: new Int32Array(coarse.n * 4), heap: new MinHeap(), stamp: 0,
  };
  const toCoarse = (x, y) => Math.max(0, Math.min(coarse.rows - 1, Math.floor(y / Gc))) * coarse.cols
    + Math.max(0, Math.min(coarse.cols - 1, Math.floor(x / Gc)));
  // the first free coarse cell straight out from a node
  const exitCell = (e) => {
    let x = e.x + e.nx * (margin + G);
    let y = e.y + e.ny * (margin + G);
    let c = toCoarse(x, y);
    for (let k = 0; k < 60 && coarse.blocked[c]; k++) {
      x += e.nx * Gc; y += e.ny * Gc;
      c = toCoarse(x, y);
    }
    return c;
  };

  // Coarse routes, cut into straight runs. Short links first: they take the
  // direct way, and the long ones spread round them.
  const lines = [];
  const reach = (w) => Math.abs(w.a.x - w.b.x) + Math.abs(w.a.y - w.b.y);
  for (const w of [...wanted].sort((p, q) => reach(p) - reach(q))) {
    const da = dirOf(w.a.nx, w.a.ny);
    const inward = (dirOf(w.b.nx, w.b.ny) + 2) % 4;
    const states = search(coarse, buf, exitCell(w.a), da, exitCell(w.b), inward, price);
    if (!states) continue;
    // a line counts once in each cross-section it passes through
    const counted = new Set();
    for (const s of states) {
      const a = s & 1;
      const c = s >> 2;
      crowd[a][c]++;
      const sid = sections.id[a][c];
      if (sid >= 0 && !counted.has(a + ':' + sid)) { counted.add(a + ':' + sid); used[a][sid]++; }
    }
    const moves = states.map((s) => ({ d: s & 3, c: s >> 2 }));
    moves.push({ d: inward, c: moves[moves.length - 1].c });
    const runs = [];
    for (const { d, c } of moves) {
      const axis = d & 1;
      if (runs.length && runs[runs.length - 1].axis === axis) continue;
      const want = axis === 0 ? (Math.floor(c / coarse.cols) + 0.5) * Gc : ((c % coarse.cols) + 0.5) * Gc;
      runs.push({ axis, want, coord: null, fixed: false });
    }
    const along = (e, axis) => (axis === 0 ? e.y : e.x);
    if (runs.length === 1) {
      // straight across: a jog in the middle when the nodes are not in line
      const r = runs[0];
      if (Math.abs(along(w.a, r.axis) - along(w.b, r.axis)) > 1) {
        const mid = r.axis === 0 ? (w.a.x + w.b.x) / 2 : (w.a.y + w.b.y) / 2;
        runs.push({ axis: 1 - r.axis, want: mid, coord: null, fixed: false }, { axis: r.axis, want: 0, coord: null, fixed: false });
      }
    }
    const first = runs[0];
    const last = runs[runs.length - 1];
    first.fixed = true; first.coord = along(w.a, first.axis);
    last.fixed = true; last.coord = along(w.b, last.axis);
    const line = { w, runs };
    runs.forEach((r, i) => { r.line = line; r.i = i; });
    lines.push(line);
  }

  // A lane for every run
  const coordOf = (r) => (r.coord ?? r.want);
  const extent = (r, est) => {
    const { runs, w } = r.line;
    const n = runs.length;
    const end = (e) => (r.axis === 0 ? e.x : e.y);
    const p = r.i === 0 ? end(w.a) : (est ? coordOf(runs[r.i - 1]) : runs[r.i - 1].coord);
    const q = r.i === n - 1 ? end(w.b) : (est ? coordOf(runs[r.i + 1]) : runs[r.i + 1].coord);
    return [Math.min(p, q), Math.max(p, q)];
  };
  const placed = [[], []];
  const clashes = (r, coord, lo, hi) => placed[r.axis].filter((e) => e.run !== r
    && Math.abs(e.coord - coord) < LANE * 0.95 && e.lo < hi + LANE / 2 && lo < e.hi + LANE / 2).length;
  const conflicts = (r, coord, lo, hi) => clashes(r, coord, lo, hi) > 0;
  const open = (axis, coord, lo, hi) => {
    for (let t = lo; t <= hi + 1e-6; t += G / 2) {
      const x = axis === 0 ? t : coord;
      const y = axis === 0 ? coord : t;
      const gx = Math.round(x / G);
      const gy = Math.round(y / G);
      if (gx < 0 || gy < 0 || gx >= fine.cols || gy >= fine.rows || fine.blocked[gy * fine.cols + gx]) return false;
    }
    return true;
  };
  // Which side of its corridor a run leans to: the side its line comes from and
  // the side it turns to. A run between two turns to the same side (a U) keeps
  // to that side, so it does not cut across the lines that go straight on.
  const side = (r) => {
    const { runs, w } = r.line;
    const far = (j, toward) => {
      // the far end of the perpendicular run j, beyond its corner with r
      const t = toward < 0 ? 0 : runs.length - 1;
      const k = j + toward;
      if (k < 0 || k >= runs.length) return r.axis === 0 ? (t === 0 ? w.a.y : w.b.y) : (t === 0 ? w.a.x : w.b.x);
      return coordOf(runs[k]);
    };
    let s = 0;
    if (r.i > 0) s += Math.sign(far(r.i - 1, -1) - coordOf(r));
    if (r.i < runs.length - 1) s += Math.sign(far(r.i + 1, 1) - coordOf(r));
    return s;
  };
  const place = (r, est) => {
    const [lo, hi] = extent(r, est);
    const s = side(r);
    const lean = s < 0 ? -1 : 1;
    // a U tries its own side first, then the other, and anything else alternates
    const offset = Math.abs(s) === 2
      ? (k) => (k <= MAX_LANE ? lean * k : -lean * (k - MAX_LANE))
      : (k) => (k % 2 ? (k + 1) / 2 : -k / 2);
    // the nearest free lane, or failing that the open lane with fewest lines on it
    let best = null;
    let fallback = null;
    let fewest = Infinity;
    for (let k = 0; k <= 2 * MAX_LANE && best === null; k++) {
      const coord = r.want + offset(k) * LANE;
      if (!open(r.axis, coord, lo, hi)) continue;
      const n = clashes(r, coord, lo, hi);
      if (n === 0) best = coord;
      else if (n < fewest) { fewest = n; fallback = coord; }
    }
    r.coord = best ?? fallback ?? r.want;
    r.entry = { run: r, coord: r.coord, lo, hi };
    placed[r.axis].push(r.entry);
  };
  const free = [];
  for (const line of lines) {
    for (const r of line.runs) {
      if (r.fixed) {
        const [lo, hi] = extent(r, true);
        r.entry = { run: r, coord: r.coord, lo, hi };
        placed[r.axis].push(r.entry);
      } else free.push(r);
    }
  }
  // long runs first: the trunks of the bundles take the middle lanes
  const span = (r) => { const [lo, hi] = extent(r, true); return hi - lo; };
  free.sort((p, q) => span(q) - span(p));
  for (const r of free) place(r, true);
  // the extents are now known exactly: re-place any run that turned out to clash
  for (let pass = 0; pass < 3; pass++) {
    for (const axis of [0, 1]) for (const e of placed[axis]) [e.lo, e.hi] = extent(e.run, false);
    let moved = 0;
    for (const r of free) {
      if (!conflicts(r, r.coord, r.entry.lo, r.entry.hi) && open(r.axis, r.coord, r.entry.lo, r.entry.hi)) continue;
      const list = placed[r.axis];
      list.splice(list.indexOf(r.entry), 1);
      r.want = r.coord;
      place(r, false);
      moved++;
    }
    if (!moved) break;
  }

  // Corners
  for (const { w, runs } of lines) {
    const pts = [{ x: w.a.x, y: w.a.y }];
    for (let i = 0; i + 1 < runs.length; i++) {
      const r = runs[i];
      const s = runs[i + 1];
      pts.push(r.axis === 0 ? { x: s.coord, y: r.coord } : { x: r.coord, y: s.coord });
    }
    pts.push({ x: w.b.x, y: w.b.y });
    routes.set(w.key, { points: dedupe(pts), start: w.a, end: w.b });
  }
  return routes;
}

function dirOf(nx, ny) {
  for (let d = 0; d < 4; d++) if (DX[d] === Math.sign(nx) && DY[d] === Math.sign(ny)) return d;
  return 0;
}

/** Drops repeated points and points in the middle of a straight stretch. */
function dedupe(all) {
  const out = [all[0]];
  for (let k = 1; k < all.length; k++) {
    const p = out[out.length - 1];
    const q = all[k];
    if (Math.hypot(q.x - p.x, q.y - p.y) < 1e-6) continue;
    if (out.length >= 2) {
      const o = out[out.length - 2];
      const cross = (p.x - o.x) * (q.y - p.y) - (p.y - o.y) * (q.x - p.x);
      const dot = (p.x - o.x) * (q.x - p.x) + (p.y - o.y) * (q.y - p.y);
      if (Math.abs(cross) < 1e-6 && dot >= 0) { out[out.length - 1] = q; continue; }
    }
    out.push(q);
  }
  return out;
}

/** The point of a route nearest to `at`, or the middle of its longest run. */
export function routeAnchor(points, at = null) {
  let best = null;
  for (let k = 1; k < points.length; k++) {
    const p = points[k - 1];
    const q = points[k];
    const len = Math.hypot(q.x - p.x, q.y - p.y);
    if (len < 1e-6) continue;
    if (!at) {
      if (!best || len > best.len) best = { len, x: (p.x + q.x) / 2, y: (p.y + q.y) / 2 };
      continue;
    }
    const t = Math.max(0, Math.min(1, ((at.x - p.x) * (q.x - p.x) + (at.y - p.y) * (q.y - p.y)) / (len * len)));
    const x = p.x + t * (q.x - p.x);
    const y = p.y + t * (q.y - p.y);
    const d = Math.hypot(at.x - x, at.y - y);
    if (!best || d < best.d) best = { d, x, y };
  }
  return best ? { x: best.x, y: best.y } : points[0];
}

/** SVG path through the corners, each corner rounded off. */
export function roundedPath(points, radius) {
  if (points.length < 2) return '';
  const f = (v) => Math.round(v * 10) / 10;
  let d = `M ${f(points[0].x)} ${f(points[0].y)}`;
  for (let k = 1; k < points.length - 1; k++) {
    const p = points[k - 1];
    const q = points[k];
    const r = points[k + 1];
    const l1 = Math.hypot(q.x - p.x, q.y - p.y);
    const l2 = Math.hypot(r.x - q.x, r.y - q.y);
    if (l1 < 1e-6 || l2 < 1e-6) continue;
    const rr = Math.min(radius, l1 / 2, l2 / 2);
    const a = { x: q.x - (q.x - p.x) / l1 * rr, y: q.y - (q.y - p.y) / l1 * rr };
    const b = { x: q.x + (r.x - q.x) / l2 * rr, y: q.y + (r.y - q.y) / l2 * rr };
    d += ` L ${f(a.x)} ${f(a.y)} Q ${f(q.x)} ${f(q.y)} ${f(b.x)} ${f(b.y)}`;
  }
  const last = points[points.length - 1];
  return d + ` L ${f(last.x)} ${f(last.y)}`;
}

class MinHeap {
  constructor() { this.ids = []; this.keys = []; }
  get size() { return this.ids.length; }
  clear() { this.ids.length = 0; this.keys.length = 0; }
  push(id, key) {
    const ids = this.ids;
    const keys = this.keys;
    let i = ids.length;
    ids.push(id); keys.push(key);
    while (i > 0) {
      const p = (i - 1) >> 1;
      if (keys[p] <= key) break;
      ids[i] = ids[p]; keys[i] = keys[p];
      i = p;
    }
    ids[i] = id; keys[i] = key;
  }
  pop() {
    const ids = this.ids;
    const keys = this.keys;
    const top = ids[0];
    const id = ids.pop();
    const key = keys.pop();
    const n = ids.length;
    if (n) {
      let i = 0;
      for (;;) {
        const l = 2 * i + 1;
        if (l >= n) break;
        const r = l + 1;
        const c = r < n && keys[r] < keys[l] ? r : l;
        if (keys[c] >= key) break;
        ids[i] = ids[c]; keys[i] = keys[c];
        i = c;
      }
      ids[i] = id; keys[i] = key;
    }
    return top;
  }
}
