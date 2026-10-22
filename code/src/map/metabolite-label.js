import { SIZES } from '../layout/layout.js';

/** Labels orbit their metabolite; SVG text itself remains horizontal. */
export const DEFAULT_METABOLITE_LABEL_ANGLE = -Math.PI / 2;

/** Read a finite instance angle, keeping it in [-π, π). */
export function metaboliteLabelAngle(node) {
  const angle = node?.labelAngle;
  if (!Number.isFinite(angle)) return DEFAULT_METABOLITE_LABEL_ANGLE;
  if (angle >= -Math.PI && angle < Math.PI) return angle;
  const turn = 2 * Math.PI;
  const wrapped = angle % turn;
  return wrapped >= Math.PI ? wrapped - turn : wrapped < -Math.PI ? wrapped + turn : wrapped;
}

const METABOLITE_LIKE = new Set(['metabolite', 'medium', 'currency']);

/**
 * A secondary metabolite instance: a small helper node the layout places beside
 * a reaction (ATP, NADH, H⁺…), or any instance set to Secondary by hand. It is
 * drawn smaller, and it is what the Cofactors toggles hide.
 */
export function isSecondaryNode(node) {
  return Boolean(node) && METABOLITE_LIKE.has(node.kind)
    && (node.kind === 'currency' || node.primary === false);
}

/**
 * Position and align an unrotated label just outside the visible circle.
 * `secondaryRadius` is how large a secondary node is drawn (smaller on a map
 * with quiet cofactors).
 */
export function metaboliteLabelPosition(node, { secondaryRadius = SIZES.currencyRadius } = {}) {
  const secondary = isSecondaryNode(node);
  const radius = secondary ? secondaryRadius : SIZES.primaryRadius;
  const distance = radius + (secondary ? 3.5 : 5);
  const angle = metaboliteLabelAngle(node);
  // Snap near-zero components so cardinal directions land on exact coordinates,
  // matching the default placement.
  const rawCos = Math.cos(angle);
  const rawSin = Math.sin(angle);
  const cos = Math.abs(rawCos) < 1e-12 ? 0 : rawCos;
  const sin = Math.abs(rawSin) < 1e-12 ? 0 : rawSin;
  return {
    x: node.x + distance * cos,
    y: node.y + distance * sin,
    anchor: cos > 0.3 ? 'start' : cos < -0.3 ? 'end' : 'middle',
    baseline: sin < -0.3 ? 'auto' : sin > 0.3 ? 'hanging' : 'middle',
  };
}
