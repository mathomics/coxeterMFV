/**
 * Line hops, drawn as gaps: where two reactions' lines cross, the line drawn
 * underneath is cut on both sides of the one on top, which stays whole. A
 * crossing then reads as one line passing over another and not as a junction
 * where they meet. On thick, curved lines a gap stays clean where an arc
 * hopping over the other line would look like a bubble.
 *
 * Works on each segment as drawn (a line or a cubic Bézier, already clipped
 * to its metabolite and arrowhead) and leaves the map's geometry alone: the
 * gap is in the picture only.
 */

const point = (x, y) => ({ x, y });
const mix = (a, b, t) => point(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t);
const distance = (a, b) => Math.hypot(a.x - b.x, a.y - b.y);
const n = (v) => (Math.round(v * 100) / 100).toString();

// Pieces a cubic is cut into to find where it crosses another: enough to follow
// the bends a branch is drawn with, few enough to run on every repaint.
const CURVE_PIECES = 12;
// Side of the grid squares crossings are looked for in, in map units.
const CELL = 80;
// More crossings than this is not a drawing anyone laid out to be read (a
// genome-scale model drawn whole), and gaps all over it only add noise.
export const MAX_LINE_HOPS = 1500;

// Clear space between each cut end of the lower line and the edge of the line
// on top, in map units.
const GAP_CLEAR = 3.5;
// A line crossing at a shallow angle runs beside the other for longer, so its
// gap is longer by 1/sin of the angle, which is floored here to cap the gap.
const GAP_MIN_SINE = 0.3;
// A gap keeps this far from either end of a line, so it never cuts into the
// metabolite or arrowhead there.
const END_MARGIN = 4;

/** The point at `t` on a line (2 points) or a cubic Bézier (4 points). */
export function curvePoint(curve, t) {
  if (curve.length === 2) return mix(curve[0], curve[1], t);
  const a = mix(curve[0], curve[1], t);
  const b = mix(curve[1], curve[2], t);
  const c = mix(curve[2], curve[3], t);
  return mix(mix(a, b, t), mix(b, c, t), t);
}

/**
 * Where the lines cross, and the gap each crossing cuts in the lower line.
 *
 * @param {{curve:{x:number,y:number}[], reactionId:string, width:number}[]} items
 *   segments in drawing order, widths in map units: of two that cross, the
 *   earlier one is underneath and is the one that gets the gap
 * @returns {Map<number, {t:number, half:number}[]>} gaps by item index, each at
 *   curve parameter `t` and `half` map units long on either side of it. Empty
 *   when there are more than `limit` (default MAX_LINE_HOPS) crossings
 */
export function findLineHops(items, { limit = MAX_LINE_HOPS } = {}) {
  const pieces = [];
  items.forEach((item, index) => {
    const curve = item.curve;
    if (!curve || curve.length < 2) return;
    const steps = curve.length === 4 ? CURVE_PIECES : 1;
    let previous = curve[0];
    let length = 0;
    const own = [];
    for (let k = 1; k <= steps; k += 1) {
      const t = k / steps;
      const next = curvePoint(curve, t);
      const piece = { index, t0: (k - 1) / steps, t1: t, a: previous, b: next, before: length };
      length += distance(previous, next);
      own.push(piece);
      previous = next;
    }
    for (const piece of own) piece.total = length;
    pieces.push(...own);
  });

  const grid = new Map();
  const cellOf = (v) => Math.floor(v / CELL);
  for (const piece of pieces) {
    const x0 = cellOf(Math.min(piece.a.x, piece.b.x));
    const x1 = cellOf(Math.max(piece.a.x, piece.b.x));
    const y0 = cellOf(Math.min(piece.a.y, piece.b.y));
    const y1 = cellOf(Math.max(piece.a.y, piece.b.y));
    for (let gx = x0; gx <= x1; gx += 1) {
      for (let gy = y0; gy <= y1; gy += 1) {
        const key = `${gx},${gy}`;
        let bucket = grid.get(key);
        if (!bucket) grid.set(key, (bucket = []));
        bucket.push(piece);
      }
    }
  }

  const hops = new Map();
  let count = 0;
  for (const [key, bucket] of grid) {
    if (bucket.length < 2) continue;
    const [gx, gy] = key.split(',').map(Number);
    for (let i = 0; i < bucket.length; i += 1) {
      const p = bucket[i];
      for (let j = i + 1; j < bucket.length; j += 1) {
        const q = bucket[j];
        if (p.index === q.index) continue;
        const first = items[p.index];
        const second = items[q.index];
        // A reaction's own branches meet at its centre, which is not a crossing.
        if (first.reactionId === second.reactionId) continue;
        const hit = intersect(p.a, p.b, q.a, q.b);
        if (!hit) continue;
        // Each crossing is counted in the one grid square it lies in, although
        // both pieces are listed in every square they pass over.
        if (cellOf(hit.x) !== gx || cellOf(hit.y) !== gy) continue;

        const [under, over] = p.index < q.index ? [p, q] : [q, p];
        const overWidth = items[over.index].width ?? 7.5;
        const underWidth = items[under.index].width ?? 7.5;
        // The cut has to clear the top line's half width and a gap, measured
        // along the lower line, plus the round cap each cut end is drawn with.
        const sine = Math.max(GAP_MIN_SINE, crossingSine(p.a, p.b, q.a, q.b));
        const half = (overWidth / 2 + GAP_CLEAR) / sine + underWidth / 2;
        // Lines that only meet where they end (at a metabolite they share, at
        // the reaction they both run into) are not crossing each other.
        if (nearEnd(under, hit, half) || nearEnd(over, hit, underWidth / 2 + GAP_CLEAR)) continue;
        const s = projection(under.a, under.b, hit);
        const t = under.t0 + (under.t1 - under.t0) * s;
        if (!hops.has(under.index)) hops.set(under.index, []);
        hops.get(under.index).push({ t, half });
        count += 1;
        if (count > limit) return new Map();
      }
    }
  }
  return hops;
}

/** The sine of the angle between segments a–b and c–d, from 0 (parallel) to 1. */
function crossingSine(a, b, c, d) {
  const ux = b.x - a.x;
  const uy = b.y - a.y;
  const vx = d.x - c.x;
  const vy = d.y - c.y;
  const lengths = Math.hypot(ux, uy) * Math.hypot(vx, vy);
  return lengths > 1e-12 ? Math.abs(ux * vy - uy * vx) / lengths : 0;
}

/** Whether `hit` lies within `reach` (plus END_MARGIN) of either end of its line. */
function nearEnd(piece, hit, reach) {
  const along = piece.before + distance(piece.a, hit);
  const margin = reach + END_MARGIN;
  return along < margin || piece.total - along < margin;
}

function projection(a, b, p) {
  const len = distance(a, b);
  return len > 1e-9 ? distance(a, p) / len : 0;
}

/** Where segments a–b and c–d cross, or null. Touching and parallel lines do not count. */
function intersect(a, b, c, d) {
  const rx = b.x - a.x;
  const ry = b.y - a.y;
  const sx = d.x - c.x;
  const sy = d.y - c.y;
  const denom = rx * sy - ry * sx;
  if (Math.abs(denom) < 1e-9) return null;
  const qx = c.x - a.x;
  const qy = c.y - a.y;
  const u = (qx * sy - qy * sx) / denom;
  const v = (qx * ry - qy * rx) / denom;
  if (u <= 0 || u >= 1 || v <= 0 || v >= 1) return null;
  return point(a.x + u * rx, a.y + u * ry);
}

/**
 * The path of a segment with its gaps: the same line or curve, cut where
 * another passes over it and picked up again on the other side.
 *
 * @param {{x:number,y:number}[]} curve 2 or 4 points
 * @param {{t:number, half:number}[]} hops the gaps from findLineHops
 * @returns {string} SVG path data
 */
export function hoppedPath(curve, hops) {
  const intervals = [];
  for (const hop of [...hops].sort((a, b) => a.t - b.t)) {
    const speed = derivativeLength(curve, hop.t);
    if (speed < 1e-9) continue;
    const dt = hop.half / speed;
    const from = Math.max(0, hop.t - dt);
    const to = Math.min(1, hop.t + dt);
    if (from <= 0 || to >= 1) continue;
    const last = intervals.at(-1);
    // Two crossings closer than a gap apart make one longer gap.
    if (last && from <= last.to) last.to = Math.max(last.to, to);
    else intervals.push({ from, to });
  }

  const start = curve[0];
  const parts = [`M ${n(start.x)} ${n(start.y)}`];
  let cursor = 0;
  for (const { from, to } of intervals) {
    parts.push(piecePath(curve, cursor, from));
    const resume = curvePoint(curve, to);
    parts.push(`M ${n(resume.x)} ${n(resume.y)}`);
    cursor = to;
  }
  parts.push(piecePath(curve, cursor, 1));
  return parts.join(' ');
}

/** The stretch of the curve from `a` to `b`, as a path command continuing from its start. */
function piecePath(curve, a, b) {
  if (curve.length === 2) {
    const end = curvePoint(curve, b);
    return `L ${n(end.x)} ${n(end.y)}`;
  }
  const [, c1, c2, end] = subCubic(curve, a, b);
  return `C ${n(c1.x)} ${n(c1.y)}, ${n(c2.x)} ${n(c2.y)}, ${n(end.x)} ${n(end.y)}`;
}

/** The cubic between parameters a and b of another, by de Casteljau. */
function subCubic(curve, a, b) {
  const [left] = split(curve, b);
  if (b <= 1e-12) return [curve[0], curve[0], curve[0], curve[0]];
  const [, right] = split(left, a / b);
  return right;
}

function split([p0, p1, p2, p3], t) {
  const a = mix(p0, p1, t);
  const b = mix(p1, p2, t);
  const c = mix(p2, p3, t);
  const d = mix(a, b, t);
  const e = mix(b, c, t);
  const f = mix(d, e, t);
  return [[p0, a, d, f], [f, e, c, p3]];
}

function derivativeLength(curve, t) {
  if (curve.length === 2) return distance(curve[0], curve[1]);
  const [p0, p1, p2, p3] = curve;
  const u = 1 - t;
  const dx = 3 * u * u * (p1.x - p0.x) + 6 * u * t * (p2.x - p1.x) + 3 * t * t * (p3.x - p2.x);
  const dy = 3 * u * u * (p1.y - p0.y) + 6 * u * t * (p2.y - p1.y) + 3 * t * t * (p3.y - p2.y);
  return Math.hypot(dx, dy);
}
