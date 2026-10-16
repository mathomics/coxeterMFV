import { SIZES } from '../layout/layout.js';

const GAP = 4;
const point = (x, y) => ({ x, y });
const mix = (a, b, t) => point(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t);
const distance = (a, b) => Math.hypot(a.x - b.x, a.y - b.y);
const n = (value) => (Math.round(value * 100) / 100).toString();

/**
 * Render-only geometry, oriented from the reaction towards the endpoint.
 * Clipping a Bézier with de Casteljau preserves its edited shape and leaves
 * model coordinates and editable control points untouched.
 *
 * Returns the SVG path `d`, the arrowhead (or null) and the drawn `curve`. `d`
 * is empty when the branch lies entirely inside its node.
 */
export function reactionEndpointGeometry(points, { node = null, arrow = false, width = 7.5 } = {}) {
  const cubic = points.length === 4;
  const at = (t) => {
    if (!cubic) return mix(points[0], points[1], t);
    const a = mix(points[0], points[1], t);
    const b = mix(points[1], points[2], t);
    const c = mix(points[2], points[3], t);
    return mix(mix(a, b, t), mix(b, c, t), t);
  };
  const endpoint = points.at(-1);
  const radius = node
    ? (['metabolite', 'medium', 'currency'].includes(node.kind)
      ? (node.kind === 'currency' || node.primary === false ? SIZES.currencyRadius : SIZES.primaryRadius)
      : Math.max(0, Number(node.r) || 0))
    : 0;
  const inset = node ? radius + GAP : (arrow ? 2 : 0);
  const tipT = inset > 0 ? outsideCircle(at, 1, endpoint, inset) : 1;
  if (tipT === null) return { d: '', arrow: null };
  const tip = at(tipT);
  const size = Math.max(7, width * 1.9);
  let shaftT = tipT;
  let head = null;
  if (arrow) {
    const baseT = outsideCircle(at, tipT, tip, size * 1.25);
    shaftT = baseT ?? 0;
    const base = at(shaftT);
    const length = distance(tip, base);
    if (length > 1e-6) {
      const dx = (tip.x - base.x) / length;
      const dy = (tip.y - base.y) / length;
      const half = size * 0.62 * Math.min(1, length / (size * 1.25));
      head = {
        tip, base,
        points: [tip, point(base.x - dy * half, base.y + dx * half), point(base.x + dy * half, base.y - dx * half)],
      };
    }
  }
  const end = at(shaftT);
  // `curve` is the line or cubic the path draws, for what is worked out on
  // the picture itself (line hops).
  let d;
  let curve;
  if (cubic) {
    const a = mix(points[0], points[1], shaftT);
    const b = mix(points[1], points[2], shaftT);
    const first = a;
    const second = mix(a, b, shaftT);
    d = `M ${n(points[0].x)} ${n(points[0].y)} C ${n(first.x)} ${n(first.y)}, ${n(second.x)} ${n(second.y)}, ${n(end.x)} ${n(end.y)}`;
    curve = [points[0], first, second, end];
  } else {
    d = `M ${n(points[0].x)} ${n(points[0].y)} L ${n(end.x)} ${n(end.y)}`;
    curve = [points[0], end];
  }
  return { d, arrow: head, curve };
}

/** Last crossing, so loops and strongly bent branches clip at the node end. */
function outsideCircle(at, endT, center, radius) {
  if (distance(at(endT), center) >= radius) return endT;
  let inside = endT;
  for (let i = 1; i <= 32; i += 1) {
    const outside = endT * (1 - i / 32);
    if (distance(at(outside), center) >= radius) {
      let low = outside;
      let high = inside;
      for (let step = 0; step < 30; step += 1) {
        const mid = (low + high) / 2;
        if (distance(at(mid), center) >= radius) low = mid;
        else high = mid;
      }
      return (low + high) / 2;
    }
    inside = outside;
  }
  return null;
}
