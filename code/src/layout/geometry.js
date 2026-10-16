/** Rectangle helpers used by the nested compartment packer. */

/** @typedef {{x:number, y:number, w:number, h:number}} Rect */

export const rect = (x, y, w, h) => ({ x, y, w, h });

/**
 * Shrink `r` by margins given in CSS order (top, right, bottom, left, each
 * defaulting as in CSS). Negative margins grow it. Width and height never go
 * below 0.
 */
export const inset = (r, top, right = top, bottom = top, left = right) => ({
  x: r.x + left,
  y: r.y + top,
  w: Math.max(0, r.w - left - right),
  h: Math.max(0, r.h - top - bottom),
});

export const center = (r) => ({ x: r.x + r.w / 2, y: r.y + r.h / 2 });
export const area = (r) => r.w * r.h;
export const right = (r) => r.x + r.w;
export const bottom = (r) => r.y + r.h;

/** Compartment ids (or visual ids) drawn round until their shape is edited. */
export const ROUND_ORGANELLES = new Set(['m', 'u', 'n', 'v', 'x', 'g', 'l', 'cx', 'im']);

/** The corner a compartment gets until its shape is edited: round organelles, square-ish shells. */
function defaultCornerRadius(compartment) {
  const r = compartment.rect;
  if (ROUND_ORGANELLES.has(compartment.visualId ?? compartment.id)) {
    return Math.min(r.w, r.h) * 0.36;
  }
  return Math.min(40, Math.min(r.w, r.h) * 0.16);
}

const unit = (v) => Math.min(1, Math.max(0, Number(v) || 0));

/**
 * A compartment's shape, as two amounts from 0 to 1. `corner` is how soft its
 * corners are: 0 square, 1 a corner as round as its shorter side allows (a
 * pill). `oval` is how far the whole outline is pulled towards an oval, whose
 * corners grow with the rectangle in both directions. On a full oval `corner`
 * still counts, as how round that oval is: 1 the ellipse inside the rectangle,
 * 0 a boxy oval with flat sides (see compartmentCorners). Kept as fractions,
 * so a resized compartment keeps its look. A compartment whose shape was never
 * edited gets its default corner and no oval.
 */
export function compartmentShape(compartment) {
  const s = compartment.shape;
  if (s && (Number.isFinite(s.corner) || Number.isFinite(s.oval))) {
    return { corner: unit(s.corner), oval: unit(s.oval) };
  }
  const half = Math.min(compartment.rect.w, compartment.rect.h) / 2;
  return { corner: half > 0 ? unit(defaultCornerRadius(compartment) / half) : 0, oval: 0 };
}

/** Ready-made shapes, by name, as compartmentShape amounts. */
export const COMPARTMENT_SHAPES = {
  rectangle: { corner: 0, oval: 0 },
  rounded: { corner: 0.3, oval: 0 },
  capsule: { corner: 1, oval: 0 },
  oval: { corner: 1, oval: 1 },
};

// How far a full oval's corners reach along each side, as a share of half that
// side, at its boxiest (corner 0) and roundest (corner 1, the ellipse).
const OVAL_BOXY = 0.3;

/**
 * The radii of a compartment's corners, which may be elliptical: what SVG's
 * rx and ry draw, and what a boundary projects onto. A rounded rectangle has
 * circular corners. An oval's corners are elliptical, the same share of the
 * width and of the height, and the `oval` amount blends one into the other.
 */
export function compartmentCorners(compartment) {
  const r = compartment.rect;
  const { corner, oval } = compartmentShape(compartment);
  const base = corner * Math.min(r.w, r.h) / 2;
  const reach = OVAL_BOXY + (1 - OVAL_BOXY) * corner;
  return {
    rx: base + oval * (reach * r.w / 2 - base),
    ry: base + oval * (reach * r.h / 2 - base),
  };
}

/** The corner radius of a compartment whose corners are circular. */
export function compartmentCornerRadius(compartment) {
  const { rx, ry } = compartmentCorners(compartment);
  return Math.min(rx, ry);
}

/** Whether a compartment reads as round, and gets its name astride the top edge. */
export function isRoundCompartment(compartment) {
  if (!compartment.shape) return ROUND_ORGANELLES.has(compartment.visualId ?? compartment.id);
  const { corner, oval } = compartmentShape(compartment);
  return oval >= 0.25 || corner >= 0.6;
}

/**
 * Closest point to `point` on a map boundary's outline, with the outward
 * normal, following the rounded corners of the compartment inside it.
 */
export function closestBoundaryPoint(layout, boundary, point) {
  const cell = layout.cells?.find(cell => cell.id === boundary.cellId);
  const compartment = cell?.compartments.find(compartment => compartment.id === boundary.inside);
  return closestPerimeterPoint(boundary.rect, point, compartment ? compartmentCorners(compartment) : 0);
}

/** Whether `p` lies inside `r` shrunk by `pad` on every side. */
export function contains(r, p, pad = 0) {
  return p.x >= r.x + pad && p.x <= right(r) - pad && p.y >= r.y + pad && p.y <= bottom(r) - pad;
}

/**
 * The point nearest `p` inside `r` shrunk by `pad` on every side. A rect too
 * small for the padding collapses to its centre.
 */
export function clampToRect(p, r, pad = 0) {
  const w = Math.max(0, r.w / 2 - pad);
  const h = Math.max(0, r.h / 2 - pad);
  const c = center(r);
  return {
    x: Math.min(Math.max(p.x, c.x - w), c.x + w),
    y: Math.min(Math.max(p.y, c.y - h), c.y + h),
  };
}

/** Split a rect into `n` parts along its longer axis, weighted. */
export function splitWeighted(r, weights, gap = 0) {
  const total = weights.reduce((a, b) => a + b, 0) || 1;
  const horizontal = r.w >= r.h;
  const span = (horizontal ? r.w : r.h) - gap * (weights.length - 1);
  const out = [];
  let cursor = horizontal ? r.x : r.y;
  for (const w of weights) {
    const size = (w / total) * span;
    out.push(horizontal
      ? { x: cursor, y: r.y, w: size, h: r.h }
      : { x: r.x, y: cursor, w: r.w, h: size });
    cursor += size + gap;
  }
  return out;
}

/** Lay `n` rects out on a grid inside `r`, preferring near-square cells. */
export function gridCells(r, n, gap = 24, targetAspect = 1.08) {
  if (n <= 0) return [];
  // Search the column counts rather than deriving one: a 4-cell map on a wide
  // canvas should come out 2x2, not 3 + 1 stranded underneath.
  let cols = 1;
  let best = Infinity;
  for (let c = 1; c <= n; c++) {
    const rws = Math.ceil(n / c);
    const cw = (r.w - gap * (c - 1)) / c;
    const ch = (r.h - gap * (rws - 1)) / rws;
    if (cw <= 0 || ch <= 0) continue;
    const aspectCost = Math.abs(Math.log((cw / ch) / targetAspect));
    const raggedCost = ((c * rws - n) / n) * 0.9;
    const score = aspectCost + raggedCost;
    if (score < best) { best = score; cols = c; }
  }
  const rows = Math.ceil(n / cols);
  const cw = (r.w - gap * (cols - 1)) / cols;
  const ch = (r.h - gap * (rows - 1)) / rows;
  const out = [];
  for (let i = 0; i < n; i++) {
    const col = i % cols;
    const row = Math.floor(i / cols);
    // Centre the last, possibly short, row.
    const inRow = Math.min(cols, n - row * cols);
    const offset = ((cols - inRow) * (cw + gap)) / 2;
    out.push({ x: r.x + offset + col * (cw + gap), y: r.y + row * (ch + gap), w: cw, h: ch });
  }
  return out;
}

/**
 * Walk the perimeter of a rectangle. `t` runs 0..1 clockwise from the middle
 * of the top edge. Returns the point and the outward normal, which is what the
 * renderer needs to orient a transporter glyph.
 */
export function perimeterPoint(r, t) {
  const p = ((t % 1) + 1) % 1;
  const perim = 2 * (r.w + r.h);
  let d = p * perim;
  const halfTop = r.w / 2;

  // Start at the middle of the top edge and go clockwise.
  if (d < halfTop) return { x: r.x + r.w / 2 + d, y: r.y, nx: 0, ny: -1 };
  d -= halfTop;
  if (d < r.h) return { x: right(r), y: r.y + d, nx: 1, ny: 0 };
  d -= r.h;
  if (d < r.w) return { x: right(r) - d, y: bottom(r), nx: 0, ny: 1 };
  d -= r.w;
  if (d < r.h) return { x: r.x, y: bottom(r) - d, nx: -1, ny: 0 };
  d -= r.h;
  return { x: r.x + d, y: r.y, nx: 0, ny: -1 };
}

/**
 * Closest point on the perimeter of `r` to `p`, with the outward normal.
 * `radius` rounds the corners: a number for circular corners, or `{rx, ry}`
 * (compartmentCorners) for elliptical ones. On an elliptical corner the point
 * is where the line from that corner's centre meets the ellipse, near enough
 * to the closest point for placing a transporter, and the normal is the
 * ellipse's own.
 */
export function closestPerimeterPoint(r, p, radius = 0) {
  let ax;
  let ay;
  if (typeof radius === 'object' && radius) {
    ax = Math.max(0, Math.min(Number(radius.rx) || 0, r.w / 2));
    ay = Math.max(0, Math.min(Number(radius.ry) || 0, r.h / 2));
  } else {
    ax = ay = Math.max(0, Math.min(Number(radius) || 0, r.w / 2, r.h / 2));
  }
  if (ax > 0 && ay > 0) {
    const nearestX = Math.max(r.x + ax, Math.min(right(r) - ax, p.x));
    const nearestY = Math.max(r.y + ay, Math.min(bottom(r) - ay, p.y));
    const u = (p.x - nearestX) / ax;
    const v = (p.y - nearestY) / ay;
    const length = Math.hypot(u, v);
    if (length > 0) {
      const ex = (ax * u) / length;
      const ey = (ay * v) / length;
      const gx = ex / (ax * ax);
      const gy = ey / (ay * ay);
      const g = Math.hypot(gx, gy);
      return { x: nearestX + ex, y: nearestY + ey, nx: gx / g, ny: gy / g };
    }
  }
  const c = center(r);
  const dx = p.x - c.x;
  const dy = p.y - c.y;
  const sx = r.w / 2;
  const sy = r.h / 2;
  if (Math.abs(dx) * sy > Math.abs(dy) * sx) {
    const nx = dx >= 0 ? 1 : -1;
    return { x: c.x + nx * sx, y: c.y + Math.max(-sy, Math.min(sy, dy)), nx, ny: 0 };
  }
  const ny = dy >= 0 ? 1 : -1;
  return { x: c.x + Math.max(-sx, Math.min(sx, dx)), y: c.y + ny * sy, nx: 0, ny };
}

/** Signed distance from a point to the rect edge; negative means inside. */
export function signedDistance(r, p) {
  const dx = Math.max(r.x - p.x, 0, p.x - right(r));
  const dy = Math.max(r.y - p.y, 0, p.y - bottom(r));
  const outside = Math.hypot(dx, dy);
  if (outside > 0) return outside;
  return -Math.min(p.x - r.x, right(r) - p.x, p.y - r.y, bottom(r) - p.y);
}
