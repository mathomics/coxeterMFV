/**
 * Manual editing of reaction geometry.
 *
 * Escher-style branch geometry uses a cubic Bézier per metabolite connection:
 *
 *   metabolite P0 -- b1 (metabolite-side) -- b2 (reaction-side) -- P3 backbone
 *
 * Each branch has three points that shape it:
 *   b1      the metabolite-side Bézier handle
 *   b2      the reaction-side Bézier handle
 *   anchor  the branch attachment point on the reaction backbone
 *
 * b1 follows the metabolite when that metabolite moves. b2 and anchor follow
 * the reaction skeleton when the reaction moves. The map shows b1 and b2 as
 * handles but does not offer the anchors one by one, because pulling a single
 * branch off the backbone bends a reaction out of shape. The backbone is
 * turned as a whole instead (rotateReaction), which takes every anchor with it.
 *
 * The same module holds the operations that rearrange one reaction as a
 * drawing: keeping it straight between two metabolites, exchanging the places
 * of two of its metabolites, and mirroring it. All of them are geometry only.
 */

import { clampNodePosition, nodeFitsAt, updateConnectedSegments } from './drag.js';
import { closestBoundaryPoint } from './geometry.js';
import { turnMetaboliteLabel } from '../map/metabolite-label.js';

const EPS = 1e-9;

/** How close to either end a straight reaction's marker may be slid. */
const LINEAR_T_MIN = 0.12;
const LINEAR_T_MAX = 0.88;

/**
 * Where a reaction's own cofactor nodes stand by default, measured from its
 * centre: out along the backbone and off to one side of it. The first pair on
 * a side stands close in beside the stem, one on each hand of it, between the
 * marker and the metabolite the backbone runs to; further ones fan out from
 * them, wider and nearer the marker.
 *
 * How far out along the backbone depends on which way that end of it points on
 * the page. A metabolite's name is written above it, so on the end that points
 * down the name of the one the backbone runs to lies between it and the
 * marker, and the cofactors keep below that name, nearer the marker. On the
 * end that points up there is nothing in the way and they stand further out.
 */
const SATELLITE_ALONG = 34;
const SATELLITE_ALONG_LEAN = 6;
const SATELLITE_ACROSS = 26;
const SATELLITE_ALONG_STEP = 12;
const SATELLITE_ACROSS_STEP = 22;
const SATELLITE_ALONG_MIN = 16;

/**
 * The corner a branch to an off-axis metabolite turns: how far along the stem
 * its reaction-side handle lies, as a share of the way out to the metabolite's
 * level, and how far in from the metabolite its other handle, as a share of
 * the way across to the stem.
 */
const BRANCH_TURN_ALONG = 0.8;
const BRANCH_TURN_ACROSS = 0.55;

/** The least a metabolite is set from the marker when a reaction is aligned. */
const ALIGN_ARM_MIN = 56;

/** The eight ways a reaction can be laid from its substrate to its product. */
const DIAGONAL = Math.SQRT1_2;
export const COMPASS = Object.freeze({
  N: Object.freeze({ x: 0, y: -1 }),
  NE: Object.freeze({ x: DIAGONAL, y: -DIAGONAL }),
  E: Object.freeze({ x: 1, y: 0 }),
  SE: Object.freeze({ x: DIAGONAL, y: DIAGONAL }),
  S: Object.freeze({ x: 0, y: 1 }),
  SW: Object.freeze({ x: -DIAGONAL, y: DIAGONAL }),
  W: Object.freeze({ x: -1, y: 0 }),
  NW: Object.freeze({ x: -DIAGONAL, y: -DIAGONAL }),
});

/** Whether `reactionId` is drawn on the map, and so can be dragged. */
export function canMoveReaction(layout, reactionId) {
  return Boolean(layout?.reactions?.has(reactionId));
}

/**
 * Drag a reaction's marker towards `(x, y)`, taking its own geometry along. A
 * transporter slides round its membrane and turns with it, a straight reaction
 * slides along its line, and an internal one is held inside its compartment.
 *
 * @returns {object|null} the reaction, or null if it could not be moved
 */
export function moveReaction(layout, reactionId, x, y) {
  const rxn = layout?.reactions?.get(reactionId);
  if (!rxn || !Number.isFinite(x) || !Number.isFinite(y)) return null;
  ensureReactionBackbone(layout, reactionId);
  if (rxn.onMembrane && rxn.boundaryId) return moveMembraneReaction(layout, rxn, x, y);
  if (linearEnds(layout, rxn)) return slideLinearReaction(layout, rxn, x, y);
  return moveInternalReaction(layout, rxn, x, y);
}

// Rotation

/** Direction of the reaction backbone, in radians, as the map draws it. */
export function reactionAngle(layout, reactionId) {
  const dir = layout?.reactions?.get(reactionId)?.axis?.dir;
  return dir ? Math.atan2(dir.y, dir.x) : null;
}

/** Can this reaction's backbone be turned by hand? */
export function canRotateReaction(layout, reactionId) {
  const rxn = layout?.reactions?.get(reactionId);
  // A transporter points across the membrane it sits on, and a straight
  // reaction along the line it is kept on: neither has a direction to choose.
  return Boolean(rxn?.axis) && !rxn.onMembrane && !linearEnds(layout, rxn);
}

/**
 * Turn the backbone of an internal reaction about its centre.
 *
 * Everything that belongs to the reaction itself turns with it: the axis, the
 * points where the branches leave it, their reaction-side handles, the small
 * cofactor nodes the reaction owns and a name moved off its default place.
 * Metabolites that stand on their own stay where they are, and their branches
 * are redrawn to the turned backbone.
 *
 * @param {number} angle radians; positive is clockwise on screen
 */
export function rotateReaction(layout, reactionId, angle) {
  const rxn = layout?.reactions?.get(reactionId);
  if (!Number.isFinite(angle) || !canRotateReaction(layout, reactionId)) return null;
  ensureReactionBackbone(layout, reactionId);
  if (Math.abs(angle) < EPS) return rxn;

  const cos = Math.cos(angle);
  const sin = Math.sin(angle);
  const centre = { x: rxn.x, y: rxn.y };
  const turn = (v) => ({ x: v.x * cos - v.y * sin, y: v.x * sin + v.y * cos });
  transformSkeleton(layout, rxn, {
    point: (p) => rotateAbout(p, centre, centre, cos, sin),
    vector: turn,
  });
  if (rxn.labelOffset) rxn.labelOffset = turn(rxn.labelOffset);
  return rxn;
}

/**
 * Turn an internal reaction's own drawing about `pivot`, marker included: the
 * axis and the direction it was placed in, where its branches leave the
 * backbone with their reaction-side handles, its cofactor nodes (labels and
 * all) and its name. This is one reaction's part of turning a whole selection
 * (rotateSelection, multi-selection.js), which is rigid, so nothing is clamped
 * here; the caller has checked there is room. Metabolites standing on their
 * own, and the metabolite ends of the branches to them, are the caller's.
 *
 * @param {number} angle radians; positive is clockwise on screen
 * @returns {object|null} the reaction, or null for a transporter or a straight
 *   reaction (one turns with its membrane, the other with its two metabolites)
 */
export function turnReactionAbout(layout, reactionId, pivot, angle) {
  const rxn = layout?.reactions?.get(reactionId);
  if (!rxn || rxn.onMembrane || linearEnds(layout, rxn) || !Number.isFinite(angle)
    || !Number.isFinite(pivot?.x) || !Number.isFinite(pivot?.y)) return null;
  ensureReactionBackbone(layout, reactionId);
  const cos = Math.cos(angle);
  const sin = Math.sin(angle);
  const point = (p) => rotateAbout(p, pivot, pivot, cos, sin);
  const turn = (v) => ({ x: v.x * cos - v.y * sin, y: v.x * sin + v.y * cos });
  transformSkeleton(layout, rxn, { point, vector: turn });
  const centre = point(rxn);
  rxn.x = centre.x;
  rxn.y = centre.y;
  for (const key of ['axisDirectionHint', 'placementDirection', 'normal', 'labelOffset']) {
    if (rxn[key]) rxn[key] = turn(rxn[key]);
  }
  for (const node of layout.nodes.values()) {
    if (node.reactionId === rxn.id) turnMetaboliteLabel(node, angle);
  }
  return rxn;
}

/** Turn the backbone so that it points along `angle`. */
export function setReactionAngle(layout, reactionId, angle) {
  const current = reactionAngle(layout, reactionId);
  if (current === null || !Number.isFinite(angle)) return null;
  return rotateReaction(layout, reactionId, normaliseAngle(angle - current));
}

// Straight reactions

/**
 * Keep a reaction straight between two of its metabolites, or stop doing so.
 *
 * A straight reaction has no shape of its own to keep in order: its marker
 * lies on the line from one metabolite to the other, turned along it, and both
 * branches are drawn as that line. Moving either metabolite takes the marker
 * with it, which is the point. Without this, every move of a substrate would
 * leave the marker behind and the reaction would have to be put right by hand.
 *
 * The two ends are remembered on the reaction (`linear.from`, `linear.to`) so
 * they survive saving, and so exchanging two metabolites can hand the role on.
 *
 * @param {{from?: string, to?: string}} [ends] node ids, when the caller has a preference
 * @returns {object|null} the reaction, or null if it cannot be made straight
 */
export function setReactionLinear(layout, reactionId, on = true, ends = {}) {
  const rxn = layout?.reactions?.get(reactionId);
  if (!rxn) return null;
  if (!on) {
    delete rxn.linear;
    return rxn;
  }
  if (rxn.onMembrane || !rxn.axis) return null;
  ensureReactionBackbone(layout, reactionId);

  const from = pickLinearEnd(layout, rxn, 'substrate', ends.from ?? rxn.linear?.from);
  const to = pickLinearEnd(layout, rxn, 'product', ends.to ?? rxn.linear?.to);
  if (!from || !to || from === to) return null;

  rxn.linear = { from, to, t: 0.5 };
  straightenReaction(layout, reactionId);
  return rxn;
}

/**
 * The two metabolites a reaction's line runs through, or would run through if
 * it were made straight now: the ones it is kept on, else the ones chosen for
 * it, else the ones it would pick itself.
 *
 * @returns {{from: string|null, to: string|null}}
 */
export function reactionLineEnds(layout, reactionId) {
  const rxn = layout?.reactions?.get(reactionId);
  if (!rxn?.axis) return { from: null, to: null };
  return {
    from: pickLinearEnd(layout, rxn, 'substrate', rxn.linear?.from),
    to: pickLinearEnd(layout, rxn, 'product', rxn.linear?.to),
  };
}

/**
 * Choose one of the two metabolites a reaction's line runs through. Which side
 * it replaces follows from which side of the reaction the metabolite is on.
 * A reaction that is being kept straight is straightened through it at once;
 * one that is not remembers the choice for when it is.
 */
export function setReactionLineEnd(layout, reactionId, nodeId) {
  const rxn = layout?.reactions?.get(reactionId);
  const seg = branchSegment(layout, reactionId, nodeId);
  if (!rxn?.axis || rxn.onMembrane || !seg) return null;
  const key = seg.side === 'substrate' ? 'from' : 'to';
  rxn.lineEnds = { ...(rxn.lineEnds ?? {}), [key]: nodeId };
  if (rxn.linear) {
    rxn.linear[key] = nodeId;
    straightenReaction(layout, reactionId);
  }
  return rxn;
}

/**
 * Lay a reaction out along one of the eight compass directions, substrate to
 * product, through its marker.
 *
 * The marker stays where it is. The two metabolites its line runs through are
 * set on either side of it along that direction, each as far from it as it
 * was, and the reaction is kept straight between them from then on. What else
 * the reaction draws is put in order around the new line: its own cofactors in
 * their places, every curve leaving the backbone along it.
 *
 * @param {'N'|'NE'|'E'|'SE'|'S'|'SW'|'W'|'NW'} direction
 * @param {{from?: string, to?: string}} [ends] node ids, to choose the two metabolites
 */
export function alignReaction(layout, reactionId, direction, ends = {}) {
  const rxn = layout?.reactions?.get(reactionId);
  const way = COMPASS[direction];
  if (!rxn?.axis || rxn.onMembrane || !way) return null;
  ensureReactionBackbone(layout, reactionId);

  const fromId = pickLinearEnd(layout, rxn, 'substrate', ends.from ?? rxn.linear?.from);
  const toId = pickLinearEnd(layout, rxn, 'product', ends.to ?? rxn.linear?.to);
  if (!fromId || !toId || fromId === toId) return null;
  const from = layout.nodes.get(fromId);
  const to = layout.nodes.get(toId);

  const centre = { x: rxn.x, y: rxn.y };
  const arm = (node) => Math.max(ALIGN_ARM_MIN, Math.hypot(node.x - centre.x, node.y - centre.y));
  const before = arm(from);
  const after = arm(to);
  placeNode(layout, from, { x: centre.x - way.x * before, y: centre.y - way.y * before });
  placeNode(layout, to, { x: centre.x + way.x * after, y: centre.y + way.y * after });

  keepStraightThrough(layout, reactionId, fromId, toId, centre);
  // Other straight reactions drawn to either metabolite follow it.
  straightenLinearReactions(layout, [fromId, toId]);
  return rxn;
}

/**
 * Keep a reaction straight between two of its metabolites from now on, with
 * its marker where it is along that line, and put the rest of its drawing in
 * order around it: its own cofactors in their places, every curve leaving the
 * backbone along it.
 *
 * @param {{x: number, y: number}} [at] where the marker is, if it has been
 *   measured before its metabolites were moved
 */
export function keepStraightThrough(layout, reactionId, fromId, toId, at = null) {
  const rxn = layout?.reactions?.get(reactionId);
  const from = layout?.nodes?.get(fromId);
  const to = layout?.nodes?.get(toId);
  if (!rxn?.axis || rxn.onMembrane || !from || !to || fromId === toId) return null;
  const marker = at ?? { x: rxn.x, y: rxn.y };
  // Where the marker is between the two, so that straightening leaves it there.
  const dx = to.x - from.x;
  const dy = to.y - from.y;
  const squared = dx * dx + dy * dy;
  const t = squared > 1e-9
    ? ((marker.x - from.x) * dx + (marker.y - from.y) * dy) / squared
    : 0.5;
  rxn.linear = { from: fromId, to: toId, t };
  rxn.lineEnds = { from: fromId, to: toId };
  straightenReaction(layout, reactionId);
  tidyReaction(layout, reactionId);
  return rxn;
}

// Base shape

/**
 * Where the `index`-th cofactor of one side of a reaction stands by default.
 *
 * @param {number} side  -1 for the substrate end of the backbone, +1 for the product end
 */
export function satellitePlace(rxn, side, index) {
  const dir = rxn.axis.dir;
  const row = Math.floor(index / 2);
  const hand = index % 2 === 0 ? 1 : -1;
  // dir.y * side is how far down the page this end of the backbone points.
  const first = SATELLITE_ALONG - SATELLITE_ALONG_LEAN * dir.y * side;
  const along = Math.max(SATELLITE_ALONG_MIN, first - row * SATELLITE_ALONG_STEP) * side;
  const across = (SATELLITE_ACROSS + row * SATELLITE_ACROSS_STEP) * hand;
  return {
    x: rxn.x + dir.x * along - dir.y * across,
    y: rxn.y + dir.y * along + dir.x * across,
  };
}

/**
 * Give one branch its base curve.
 *
 * At the reaction it runs along the backbone, as part of the stem every branch
 * of that end shares. For the metabolite the backbone runs straight to, that
 * makes a straight line. For one standing off to the side (a cofactor, or a
 * second product) the stem is followed out to about the metabolite's level
 * and the curve then turns square across to it: a C, the rounded corner a
 * hand-drawn map gives a cofactor. It arrives at the metabolite from the side
 * of the stem, not from above or below it, which would make the curve an S and
 * leave the cofactor looking as if it hung off its own little stalk.
 */
export function shapeBranch(seg, rxn) {
  const p0 = metaboliteEndpoint(seg);
  const p3 = reactionEndpoint(seg);
  if (!p0 || !p3 || !rxn?.axis) return ensureCubicControls(seg);
  const dir = rxn.axis.dir;

  // Which way along the backbone this branch leaves it: the way its end of the
  // backbone lies from the centre, or failing that the way its metabolite does.
  let outward = (p3.x - rxn.x) * dir.x + (p3.y - rxn.y) * dir.y;
  if (Math.abs(outward) < 1e-6) outward = (p0.x - rxn.x) * dir.x + (p0.y - rxn.y) * dir.y;
  if (Math.abs(outward) < 1e-6) outward = seg.side === 'substrate' ? -1 : 1;
  const u = outward < 0 ? { x: -dir.x, y: -dir.y } : { x: dir.x, y: dir.y };

  const vx = p0.x - p3.x;
  const vy = p0.y - p3.y;
  const along = vx * u.x + vy * u.y;
  const side = -vx * u.y + vy * u.x;          // signed: which hand of the stem
  const across = Math.abs(side);
  if (across < 4 && along > 0) {
    // Dead ahead: handles in order on the line itself.
    seg.b1 = lerp(p0, p3, 0.34);
    seg.b2 = lerp(p0, p3, 0.72);
  } else {
    // Unit vector from the stem out to the metabolite's hand of it.
    const hand = side < 0 ? -1 : 1;
    const n = { x: -u.y * hand, y: u.x * hand };
    // A metabolite beside or behind its end of the backbone still gets a curve
    // that sets out along the stem before it comes round.
    const reach = Math.max(along, 0.5 * across, 14);
    seg.b1 = { x: p0.x - n.x * across * BRANCH_TURN_ACROSS, y: p0.y - n.y * across * BRANCH_TURN_ACROSS };
    seg.b2 = { x: p3.x + u.x * reach * BRANCH_TURN_ALONG, y: p3.y + u.y * reach * BRANCH_TURN_ALONG };
  }
  delete seg.control;
  return seg;
}

/**
 * Put a reaction's drawing back in its base shape, where it stands.
 *
 * The cofactor nodes the reaction owns go to their default places around the
 * backbone, the branches are attached along the backbone in order again, and
 * every curve is given its base form. Metabolites that stand on their own are
 * not moved: only the curves to them are redrawn.
 */
export function tidyReaction(layout, reactionId) {
  const rxn = layout?.reactions?.get(reactionId);
  if (!rxn?.axis) return null;
  ensureReactionBackbone(layout, reactionId);
  const branches = layout.segments.filter((seg) => isBranch(seg, reactionId));

  if (!rxn.onMembrane) {
    const dir = rxn.axis.dir;
    const half = axisHalf(rxn);
    for (const [side, sign] of [['substrate', -1], ['product', 1]]) {
      const list = branches.filter((seg) => seg.side === side);
      let cofactor = 0;
      list.forEach((seg, i) => {
        const t = list.length === 1 ? 0.72 : 0.28 + 0.58 * (i / (list.length - 1));
        const anchor = { x: rxn.x + dir.x * half * t * sign, y: rxn.y + dir.y * half * t * sign };
        if (side === 'substrate') seg.to = anchor;
        else seg.from = anchor;
        seg.backboneAnchor = true;

        // Its own cofactors, but not one the reaction's line has been run to.
        const node = layout.nodes.get(seg.nodeId);
        if (node?.reactionId === reactionId
          && node.id !== rxn.linear?.from && node.id !== rxn.linear?.to) {
          placeNode(layout, node, satellitePlace(rxn, sign, cofactor));
          cofactor += 1;
        }
      });
    }
  }

  for (const seg of branches) {
    const node = layout.nodes.get(seg.nodeId);
    if (node) {
      if (seg.side === 'substrate') seg.from = { x: node.x, y: node.y };
      else seg.to = { x: node.x, y: node.y };
    }
    shapeBranch(seg, rxn);
  }
  if (rxn.linear) straightenReaction(layout, reactionId);
  return rxn;
}


/** Can this reaction be kept straight: is there a metabolite on each side? */
export function canLinearizeReaction(layout, reactionId) {
  const rxn = layout?.reactions?.get(reactionId);
  if (!rxn?.axis || rxn.onMembrane) return false;
  const from = pickLinearEnd(layout, rxn, 'substrate');
  const to = pickLinearEnd(layout, rxn, 'product');
  return Boolean(from && to && from !== to);
}

/**
 * Put a straight reaction back on its line. Called after anything that may
 * have moved one of its ends. A reaction that is not straight is left alone.
 */
export function straightenReaction(layout, reactionId) {
  const rxn = layout?.reactions?.get(reactionId);
  const ends = linearEnds(layout, rxn);
  if (!ends) return null;
  ensureReactionBackbone(layout, reactionId);

  const { from, to } = ends;
  const t = Math.min(LINEAR_T_MAX, Math.max(LINEAR_T_MIN, Number(rxn.linear.t ?? 0.5)));
  const target = lerp(from, to, t);
  const length = Math.hypot(to.x - from.x, to.y - from.y);
  const old = rxn.axis.dir;
  // Two metabolites on top of each other give no direction; keep the old one.
  const dir = length > 1e-6
    ? { x: (to.x - from.x) / length, y: (to.y - from.y) / length }
    : { x: old.x, y: old.y };
  const cos = old.x * dir.x + old.y * dir.y;
  const sin = old.x * dir.y - old.y * dir.x;
  const centre = { x: rxn.x, y: rxn.y };

  transformSkeleton(layout, rxn, {
    point: (p) => rotateAbout(p, centre, target, cos, sin),
    vector: () => ({ x: dir.x, y: dir.y }),
    keep: new Set([from.id, to.id]),
  });
  rxn.x = target.x;
  rxn.y = target.y;

  // The two branches that carry the line are the line: handles on it, and the
  // point they leave the backbone at pulled back onto it.
  for (const seg of layout.segments) {
    if (!isBranch(seg, reactionId) || (seg.nodeId !== from.id && seg.nodeId !== to.id)) continue;
    const node = seg.nodeId === from.id ? from : to;
    const anchor = reactionEndpoint(seg);
    const along = (anchor.x - target.x) * dir.x + (anchor.y - target.y) * dir.y;
    const onAxis = { x: target.x + dir.x * along, y: target.y + dir.y * along };
    const at = { x: node.x, y: node.y };
    if (seg.side === 'substrate') { seg.from = at; seg.to = onAxis; }
    else { seg.from = onAxis; seg.to = at; }
    seg.b1 = lerp(at, onAxis, 1 / 3);
    seg.b2 = lerp(at, onAxis, 2 / 3);
    delete seg.control;
  }
  return rxn;
}

/**
 * Straighten every straight reaction that ends at one of these nodes, or all
 * of them when no nodes are given.
 */
export function straightenLinearReactions(layout, nodeIds = null) {
  const wanted = nodeIds ? new Set(nodeIds) : null;
  const done = [];
  for (const [id, rxn] of layout?.reactions ?? []) {
    if (!rxn.linear) continue;
    if (wanted && !wanted.has(rxn.linear.from) && !wanted.has(rxn.linear.to)) continue;
    if (straightenReaction(layout, id)) done.push(id);
  }
  return done;
}

/** A node took over from another one (a merge): keep straight reactions attached. */
export function replaceLinearEnd(layout, oldNodeId, newNodeId) {
  for (const rxn of layout?.reactions?.values?.() ?? []) {
    if (!rxn.linear) continue;
    if (rxn.linear.from === oldNodeId) rxn.linear.from = newNodeId;
    if (rxn.linear.to === oldNodeId) rxn.linear.to = newNodeId;
  }
  for (const rxn of layout?.reactions?.values?.() ?? []) {
    if (!rxn.lineEnds) continue;
    if (rxn.lineEnds.from === oldNodeId) rxn.lineEnds.from = newNodeId;
    if (rxn.lineEnds.to === oldNodeId) rxn.lineEnds.to = newNodeId;
  }
}

/** Dragging a straight reaction's marker slides it along its line. */
function slideLinearReaction(layout, rxn, x, y) {
  const { from, to } = linearEnds(layout, rxn);
  const dx = to.x - from.x;
  const dy = to.y - from.y;
  const squared = dx * dx + dy * dy;
  if (squared > 1e-9) {
    const t = ((x - from.x) * dx + (y - from.y) * dy) / squared;
    rxn.linear.t = Math.min(LINEAR_T_MAX, Math.max(LINEAR_T_MIN, t));
  }
  return straightenReaction(layout, rxn.id);
}

/** The two end nodes of a straight reaction, or null if it is not (or no longer) one. */
function linearEnds(layout, rxn) {
  if (!rxn?.linear || rxn.onMembrane || !rxn.axis) return null;
  const from = layout.nodes.get(rxn.linear.from);
  const to = layout.nodes.get(rxn.linear.to);
  if (!from || !to) return null;
  return { from, to };
}

/**
 * Which metabolite a straight reaction runs to on one side.
 *
 * A preference is honoured if it is on that side. Otherwise the metabolite the
 * reaction is about: one that stands on its own before a cofactor the reaction
 * carries with it, a primary before a secondary, and of those the one already
 * closest to the line of the backbone, which is the one drawn as its extension.
 */
function pickLinearEnd(layout, rxn, side, preferred = null) {
  const branches = layout.segments.filter((seg) => isBranch(seg, rxn.id) && seg.side === side);
  for (const wanted of [preferred, rxn.lineEnds?.[side === 'substrate' ? 'from' : 'to']]) {
    if (wanted && branches.some((seg) => seg.nodeId === wanted)) return wanted;
  }

  const dir = rxn.axis.dir;
  let best = null;
  for (const seg of branches) {
    const node = layout.nodes.get(seg.nodeId);
    if (!node || node.hidden) continue;
    const off = Math.abs((node.x - rxn.x) * -dir.y + (node.y - rxn.y) * dir.x);
    const rank = (node.kind === 'currency' ? 2 : 0) + (node.primary === false ? 1 : 0);
    if (!best || rank < best.rank || (rank === best.rank && off < best.off)) {
      best = { id: node.id, rank, off };
    }
  }
  return best?.id ?? null;
}

// Exchanging and mirroring

/**
 * The metabolites drawn on each side of a reaction, in a stable order.
 *
 * @returns {{substrates: object[], products: object[]}} each entry carries the
 *   node, its id and label, and `group`, which names what it may trade places
 *   with: two metabolites exchange only within the same compartment of the same
 *   cell, so nothing is ever moved across a membrane.
 */
export function reactionSides(layout, reactionId) {
  const out = { substrates: [], products: [] };
  if (!layout?.reactions?.has(reactionId)) return out;
  for (const seg of layout.segments) {
    if (!isBranch(seg, reactionId)) continue;
    const node = layout.nodes.get(seg.nodeId);
    if (!node) continue;
    (seg.side === 'substrate' ? out.substrates : out.products).push({
      node,
      nodeId: node.id,
      label: node.base ?? node.localId ?? node.mergedId ?? node.id,
      group: `${node.kind === 'medium' ? '*' : (node.cellId ?? '*')}/${node.compartment ?? ''}`,
    });
  }
  return out;
}

/**
 * Exchange the places of two metabolites on the same side of a reaction.
 *
 * The drawing stays exactly as it was and the two metabolites trade positions
 * in it: each takes the other's place and the other's curve. Other reactions
 * drawn to either node follow it, as they would if it had been dragged there.
 *
 * @returns {{a: object, b: object}|null}
 */
export function swapReactionMetabolites(layout, reactionId, nodeIdA, nodeIdB) {
  const rxn = layout?.reactions?.get(reactionId);
  if (!rxn || !nodeIdA || !nodeIdB || nodeIdA === nodeIdB) return null;
  ensureReactionBackbone(layout, reactionId);

  const segA = branchSegment(layout, reactionId, nodeIdA);
  const segB = branchSegment(layout, reactionId, nodeIdB);
  const a = layout.nodes.get(nodeIdA);
  const b = layout.nodes.get(nodeIdB);
  if (!segA || !segB || !a || !b || segA.side !== segB.side) return null;
  // Never across a membrane: the two must share a compartment.
  if ((a.kind === 'medium') !== (b.kind === 'medium')) return null;
  if (a.kind !== 'medium' && (a.cellId !== b.cellId || a.compartment !== b.compartment)) return null;

  ensureCubicControls(segA);
  ensureCubicControls(segB);
  const curve = (seg) => ({
    b1: { ...seg.b1 },
    b2: { ...seg.b2 },
    anchor: { ...reactionEndpoint(seg) },
  });
  const curveA = curve(segA);
  const curveB = curve(segB);
  const placeA = { x: a.x, y: a.y };
  const placeB = { x: b.x, y: b.y };

  placeNode(layout, a, placeB);
  placeNode(layout, b, placeA);

  // A branch that ends on a transporter lane belongs to that lane, so only the
  // metabolite's own end of the curve changes hands there.
  const whole = !segA.transportLane && !segB.transportLane;
  const take = (seg, node, from) => {
    const at = { x: node.x, y: node.y };
    const anchor = whole ? from.anchor : reactionEndpoint(seg);
    if (seg.side === 'substrate') { seg.from = at; seg.to = anchor; }
    else { seg.from = anchor; seg.to = at; }
    seg.b1 = from.b1;
    if (whole) seg.b2 = from.b2;
    delete seg.control;
  };
  take(segA, a, curveB);
  take(segB, b, curveA);

  // The metabolite now standing where the line ran to is the one it runs to.
  const trade = (id) => (id === nodeIdA ? nodeIdB : id === nodeIdB ? nodeIdA : id);
  if (rxn.linear) {
    rxn.linear.from = trade(rxn.linear.from);
    rxn.linear.to = trade(rxn.linear.to);
  }
  if (rxn.lineEnds) {
    rxn.lineEnds = { from: trade(rxn.lineEnds.from), to: trade(rxn.lineEnds.to) };
  }
  straightenLinearReactions(layout, [nodeIdA, nodeIdB]);
  return { a, b };
}

/**
 * Exchange the substrate side and the product side of a reaction.
 *
 * Everything the reaction is drawn with is reflected in the line through its
 * centre, across the backbone: substrates end up where the products were and
 * the backbone points the other way, while what was on the left stays on the
 * left. The metabolites move, shared ones included, and other reactions drawn
 * to them follow.
 */
export function flipReaction(layout, reactionId) {
  const rxn = layout?.reactions?.get(reactionId);
  // A transporter's two sides are the two sides of a membrane.
  if (!rxn?.axis || rxn.onMembrane) return null;
  ensureReactionBackbone(layout, reactionId);
  const dir = { x: rxn.axis.dir.x, y: rxn.axis.dir.y };
  return reflectReaction(layout, rxn, dir, { everyNode: true, reverse: true });
}

/**
 * Mirror a reaction in its own backbone: what hung off to one side hangs off to
 * the other. Metabolites other reactions are drawn to keep their place.
 */
export function mirrorReaction(layout, reactionId) {
  const rxn = layout?.reactions?.get(reactionId);
  if (!rxn?.axis) return null;
  ensureReactionBackbone(layout, reactionId);
  const dir = rxn.axis.dir;
  return reflectReaction(layout, rxn, { x: -dir.y, y: dir.x }, { everyNode: false, reverse: false });
}

/**
 * Reflect a reaction's drawing in the line through its centre with the given
 * normal.
 *
 * @param {{x:number,y:number}} normal unit vector across the mirror line
 * @param {object} opts
 * @param {boolean} opts.everyNode move metabolites other reactions use too
 * @param {boolean} opts.reverse   the backbone ends trade places
 */
function reflectReaction(layout, rxn, normal, { everyNode, reverse }) {
  const centre = { x: rxn.x, y: rxn.y };
  const reflect = (p) => {
    const d = (p.x - centre.x) * normal.x + (p.y - centre.y) * normal.y;
    return { x: p.x - 2 * d * normal.x, y: p.y - 2 * d * normal.y };
  };

  const branches = layout.segments.filter((seg) => isBranch(seg, rxn.id));
  for (const seg of branches) ensureCubicControls(seg);

  // Where each metabolite ends up, and by how much its compartment held it back.
  const moved = new Map();
  for (const seg of branches) {
    const node = layout.nodes.get(seg.nodeId);
    if (!node || moved.has(node.id)) continue;
    const exclusive = layout.segments.every((other) =>
      other.nodeId !== node.id || other.reactionId === rxn.id);
    if (!everyNode && !exclusive) continue;
    const ideal = reflect(node);
    const before = { x: node.x, y: node.y };
    placeNode(layout, node, ideal);
    moved.set(node.id, { x: node.x - ideal.x, y: node.y - ideal.y, before });
  }

  if (reverse) {
    const a1 = reflect(rxn.axis.a1);
    const a2 = reflect(rxn.axis.a2);
    rxn.axis.a1 = a1;
    rxn.axis.a2 = a2;
    rxn.axis.dir = { x: -rxn.axis.dir.x, y: -rxn.axis.dir.y };
    for (const key of ['axisDirectionHint', 'placementDirection']) {
      if (rxn[key]) rxn[key] = { x: -rxn[key].x, y: -rxn[key].y };
    }
  }
  if (rxn.boundary) rxn.boundary = reflect(rxn.boundary);
  for (const lane of rxn.transportLanes ?? []) {
    lane.insidePoint = reflect(lane.insidePoint);
    lane.outsidePoint = reflect(lane.outsidePoint);
    if (Number.isFinite(lane.offset)) lane.offset = -lane.offset;
  }

  for (const seg of layout.segments) {
    if (seg.reactionId !== rxn.id) continue;
    if (seg.kind === 'axis') {
      seg.from = rxn.axis.a1;
      seg.to = rxn.axis.a2;
    } else if (seg.kind === 'transport-lane') {
      const lane = rxn.transportLanes?.find((l) => l.base === seg.transportBase);
      seg.from = lane ? lane.insidePoint : reflect(seg.from);
      seg.to = lane ? lane.outsidePoint : reflect(seg.to);
    } else if (seg.kind === 'boundary') {
      seg.from = reflect(seg.from);
      seg.to = reflect(seg.to);
    } else if (isBranch(seg, rxn.id)) {
      const node = layout.nodes.get(seg.nodeId);
      const shift = moved.get(seg.nodeId);
      const anchor = reflect(reactionEndpoint(seg));
      const at = node ? { x: node.x, y: node.y } : metaboliteEndpoint(seg);
      if (seg.side === 'substrate') { seg.from = at; seg.to = anchor; }
      else { seg.from = anchor; seg.to = at; }
      seg.b2 = reflect(seg.b2);
      // placeNode() already carried b1 along with the node; start again from
      // where it was so the handle is mirrored, not mirrored and moved.
      if (shift) {
        const original = shifted(seg.b1, shift.before.x - node.x, shift.before.y - node.y);
        seg.b1 = shifted(reflect(original), shift.x, shift.y);
      }
      delete seg.control;
    }
  }

  straightenLinearReactions(layout, [...moved.keys()]);
  if (rxn.linear) straightenReaction(layout, rxn.id);
  return rxn;
}

/**
 * Put a node somewhere, held inside its compartment, and bring along the
 * branches of every reaction drawn to it, the same as a drag to that point.
 */
function placeNode(layout, node, point) {
  const before = { x: node.x, y: node.y };
  const p = clampNodePosition(layout, node, point.x, point.y);
  node.x = p.x;
  node.y = p.y;
  updateConnectedSegments(layout, node.id, { x: p.x - before.x, y: p.y - before.y });
  return node;
}

/**
 * Move everything that belongs to an internal reaction itself through one
 * rigid transform: its axis, the points its branches leave it at with their
 * reaction-side handles, and the cofactor nodes it owns. The centre is left to
 * the caller, which knows where it is going.
 *
 * @param {object} t
 * @param {(p:{x:number,y:number}) => {x:number,y:number}} t.point
 * @param {(v:{x:number,y:number}) => {x:number,y:number}} t.vector
 * @param {Set<string>} [t.keep] owned nodes that must stay put
 */
function transformSkeleton(layout, rxn, { point, vector, keep = null }) {
  if (rxn.axis) {
    rxn.axis.a1 = point(rxn.axis.a1);
    rxn.axis.a2 = point(rxn.axis.a2);
    rxn.axis.dir = vector(rxn.axis.dir);
  }
  if (rxn.boundary) rxn.boundary = point(rxn.boundary);

  const carried = new Set();
  for (const node of layout.nodes.values()) {
    if (node.reactionId !== rxn.id || keep?.has(node.id)) continue;
    const q = point(node);
    node.x = q.x;
    node.y = q.y;
    carried.add(node.id);
  }

  for (const seg of layout.segments) {
    if (seg.reactionId !== rxn.id) continue;
    if (seg.kind === 'axis') {
      seg.from = rxn.axis?.a1 ?? point(seg.from);
      seg.to = rxn.axis?.a2 ?? point(seg.to);
    } else if (seg.kind === 'boundary' || seg.kind === 'transport-lane') {
      seg.from = point(seg.from);
      seg.to = point(seg.to);
    } else {
      ensureCubicControls(seg);
      if (seg.side === 'substrate') seg.to = point(seg.to);
      else if (seg.side === 'product') seg.from = point(seg.from);
      if (seg.b2) seg.b2 = point(seg.b2);

      if (carried.has(seg.nodeId)) {
        const node = layout.nodes.get(seg.nodeId);
        if (seg.side === 'substrate') seg.from = { x: node.x, y: node.y };
        else if (seg.side === 'product') seg.to = { x: node.x, y: node.y };
        if (seg.b1) seg.b1 = point(seg.b1);
      }
      delete seg.control;
    }
  }
}

/** Turn `p` about `from` and set it down relative to `to`. */
function rotateAbout(p, from, to, cos, sin) {
  const dx = p.x - from.x;
  const dy = p.y - from.y;
  return { x: to.x + dx * cos - dy * sin, y: to.y + dx * sin + dy * cos };
}

function normaliseAngle(angle) {
  let a = angle % (2 * Math.PI);
  if (a > Math.PI) a -= 2 * Math.PI;
  if (a < -Math.PI) a += 2 * Math.PI;
  return a;
}

/** Return the three Escher-style control points for one branch. */
export function branchControlPoints(layout, reactionId, nodeId) {
  ensureReactionBackbone(layout, reactionId);
  const seg = branchSegment(layout, reactionId, nodeId);
  if (!seg) return null;
  ensureCubicControls(seg);
  const anchor = reactionEndpoint(seg);
  return {
    b1: { ...seg.b1 },
    b2: { ...seg.b2 },
    anchor: anchor ? { ...anchor } : null,
  };
}

/** Move b1, b2, or the branch attachment point on the reaction backbone. */
export function moveBranchControl(layout, reactionId, nodeId, handle, x, y) {
  if (!Number.isFinite(x) || !Number.isFinite(y)) return null;
  if (!['b1', 'b2', 'anchor'].includes(handle)) return null;
  ensureReactionBackbone(layout, reactionId);
  const seg = branchSegment(layout, reactionId, nodeId);
  if (!seg) return null;
  ensureCubicControls(seg);

  if (handle === 'anchor') {
    const old = reactionEndpoint(seg);
    if (!old) return null;
    const dx = x - old.x;
    const dy = y - old.y;
    const next = { x, y };
    if (seg.side === 'substrate') seg.to = next;
    else seg.from = next;
    // Keep the reaction-side Bézier geometry attached to the moved anchor.
    seg.b2 = shifted(seg.b2, dx, dy);
    seg.backboneAnchor = true;
    delete seg.control;
    return next;
  }

  seg[handle] = { x, y };
  // Once upgraded, the legacy quadratic field must not win during rendering.
  delete seg.control;
  return seg[handle];
}

/**
 * Flat list used by the UI. Each branch exposes three draggable control points:
 * b1, b2 and the backbone anchor. Four branches therefore expose twelve handles.
 */
export function reactionControlPoints(layout, reactionId) {
  ensureReactionBackbone(layout, reactionId);
  const out = [];
  for (const seg of layout?.segments ?? []) {
    if (!isBranch(seg, reactionId)) continue;
    ensureCubicControls(seg);
    out.push({
      reactionId,
      nodeId: seg.nodeId,
      handle: 'b1',
      role: 'metabolite',
      x: seg.b1.x,
      y: seg.b1.y,
    });
    out.push({
      reactionId,
      nodeId: seg.nodeId,
      handle: 'b2',
      role: 'reaction',
      x: seg.b2.x,
      y: seg.b2.y,
    });
    const anchor = reactionEndpoint(seg);
    if (anchor) {
      out.push({
        reactionId,
        nodeId: seg.nodeId,
        handle: 'anchor',
        role: 'backbone',
        x: anchor.x,
        y: anchor.y,
      });
    }
  }
  return out;
}

/**
 * Make sure a branch has the cubic handles b1 and b2, and return it. Quadratic
 * control geometry from V0.7 maps (`seg.control`) is converted to the
 * equivalent cubic Bézier. For a quadratic P0-Q-P3:
 *   C1 = P0 + 2/3(Q-P0)
 *   C2 = P3 + 2/3(Q-P3)
 * A branch with neither gets a neutral cubic along the connection.
 */
export function ensureCubicControls(seg) {
  if (seg.b1 && seg.b2) return seg;

  const p0 = metaboliteEndpoint(seg);
  const p3 = reactionEndpoint(seg);
  if (!p0 || !p3) return seg;

  if (seg.control) {
    const q = seg.control;
    seg.b1 = {
      x: p0.x + (2 / 3) * (q.x - p0.x),
      y: p0.y + (2 / 3) * (q.y - p0.y),
    };
    seg.b2 = {
      x: p3.x + (2 / 3) * (q.x - p3.x),
      y: p3.y + (2 / 3) * (q.y - p3.y),
    };
    return seg;
  }

  // New/default branch: smooth, neutral cubic along the connection.
  seg.b1 = lerp(p0, p3, 0.34);
  seg.b2 = lerp(p0, p3, 0.72);
  return seg;
}

/**
 * Give every branch of an internal reaction its own point on the backbone and
 * its two cubic handles, as edits here expect. A map laid out by the automatic
 * layout has neither: its branches meet at the axis ends, as straight lines.
 */
export function ensureReactionBackbone(layout, reactionId) {
  const rxn = layout?.reactions?.get(reactionId);
  if (!rxn?.axis || rxn.onMembrane) return;

  const branches = (layout.segments ?? []).filter((seg) => isBranch(seg, reactionId));
  const substrates = branches.filter((seg) => seg.side === 'substrate');
  const products = branches.filter((seg) => seg.side === 'product');

  const assign = (list, side) => {
    const count = list.length;
    if (!count) return;
    list.forEach((seg, i) => {
      ensureCubicControls(seg);
      if (seg.backboneAnchor) return;

      const old = reactionEndpoint(seg);
      const t = count === 1 ? 0.72 : 0.28 + 0.58 * (i / (count - 1));
      const d = axisHalf(rxn) * t * side;
      const anchor = {
        x: rxn.x + rxn.axis.dir.x * d,
        y: rxn.y + rxn.axis.dir.y * d,
      };
      const dx = anchor.x - old.x;
      const dy = anchor.y - old.y;

      if (seg.side === 'substrate') seg.to = anchor;
      else seg.from = anchor;
      if (seg.b2) seg.b2 = shifted(seg.b2, dx, dy);
      seg.backboneAnchor = true;
      delete seg.control;
    });
  };

  assign(substrates, -1);
  assign(products, 1);
}

function axisHalf(rxn) {
  if (!rxn.axis) return 18;
  return Math.max(1, Math.hypot(
    rxn.axis.a2.x - rxn.axis.a1.x,
    rxn.axis.a2.y - rxn.axis.a1.y,
  ) / 2);
}

/*
 * An internal reaction's centre is kept where a small metabolite of its
 * compartment could stand: this stands in for it.
 */
function centreProxy(layout, rxn) {
  const compartment = internalCompartment(layout, rxn);
  return compartment
    ? { kind: 'metabolite', r: 10, cellId: rxn.cellId, compartment, x: rxn.x, y: rxn.y }
    : null;
}

/**
 * Could the centre of `reactionId` move to `(x, y)` without being pulled back?
 * Only an internal reaction's centre is held: a transporter keeps to its
 * membrane and a straight reaction to its two metabolites, so neither can
 * refuse a place. `moving` is as for `nodeConstraint` (drag.js).
 */
export function reactionCentreFits(layout, reactionId, x, y, moving = null) {
  const rxn = layout?.reactions?.get(reactionId);
  if (!rxn || (rxn.onMembrane && rxn.boundaryId) || linearEnds(layout, rxn)) return true;
  const proxy = centreProxy(layout, rxn);
  return !proxy || nodeFitsAt(layout, proxy, x, y, moving);
}

function moveInternalReaction(layout, rxn, x, y) {
  const fake = centreProxy(layout, rxn);
  const p = fake ? clampNodePosition(layout, fake, x, y) : { x, y };
  const dx = p.x - rxn.x;
  const dy = p.y - rxn.y;
  if (Math.abs(dx) + Math.abs(dy) < EPS) return rxn;
  translateReaction(layout, rxn, dx, dy);
  return rxn;
}

function moveMembraneReaction(layout, rxn, x, y) {
  const boundary = layout.boundary?.(rxn.boundaryId)
    ?? layout.boundaries?.find((b) => b.id === rxn.boundaryId);
  if (!boundary?.rect) return null;

  const p = closestBoundaryPoint(layout, boundary, { x, y });
  const oldCentre = { x: rxn.x, y: rxn.y };
  const oldNormal = rxn.normal ?? { x: 1, y: 0 };
  const newNormal = { x: p.nx, y: p.ny };

  const dot = oldNormal.x * newNormal.x + oldNormal.y * newNormal.y;
  const cross = oldNormal.x * newNormal.y - oldNormal.y * newNormal.x;
  const transformPoint = (point) => {
    const dx = point.x - oldCentre.x;
    const dy = point.y - oldCentre.y;
    return {
      x: p.x + dx * dot - dy * cross,
      y: p.y + dx * cross + dy * dot,
    };
  };
  const transformVector = (v) => ({
    x: v.x * dot - v.y * cross,
    y: v.x * cross + v.y * dot,
  });

  const movedCurrency = new Set();
  for (const node of layout.nodes.values()) {
    if (node.reactionId !== rxn.id) continue;
    const q = transformPoint(node);
    node.x = q.x;
    node.y = q.y;
    movedCurrency.add(node.id);
  }

  if (rxn.axis) {
    rxn.axis.a1 = transformPoint(rxn.axis.a1);
    rxn.axis.a2 = transformPoint(rxn.axis.a2);
    rxn.axis.dir = transformVector(rxn.axis.dir);
  }

  if (Array.isArray(rxn.transportLanes)) {
    for (const lane of rxn.transportLanes) {
      lane.insidePoint = transformPoint(lane.insidePoint);
      lane.outsidePoint = transformPoint(lane.outsidePoint);
    }
  }

  for (const seg of layout.segments) {
    if (seg.reactionId !== rxn.id) continue;

    if (seg.kind === 'axis') {
      seg.from = rxn.axis?.a1 ?? transformPoint(seg.from);
      seg.to = rxn.axis?.a2 ?? transformPoint(seg.to);
    } else if (seg.kind === 'transport-lane') {
      const lane = rxn.transportLanes?.find((l) => l.base === seg.transportBase);
      if (lane) {
        seg.from = lane.insidePoint;
        seg.to = lane.outsidePoint;
      } else {
        seg.from = transformPoint(seg.from);
        seg.to = transformPoint(seg.to);
      }
    } else if (seg.kind === 'boundary') {
      seg.from = transformPoint(seg.from);
      seg.to = transformPoint(seg.to);
    } else {
      ensureCubicControls(seg);
      // Transform only the reaction-side endpoint and reaction-side handle.
      if (seg.side === 'substrate') seg.to = transformPoint(seg.to);
      else if (seg.side === 'product') seg.from = transformPoint(seg.from);
      if (seg.b2) seg.b2 = transformPoint(seg.b2);

      // Reaction-owned currency nodes move with the skeleton, so b1 does too.
      if (movedCurrency.has(seg.nodeId)) {
        const node = layout.nodes.get(seg.nodeId);
        if (seg.side === 'substrate') seg.from = { x: node.x, y: node.y };
        else if (seg.side === 'product') seg.to = { x: node.x, y: node.y };
        if (seg.b1) seg.b1 = transformPoint(seg.b1);
      }
      delete seg.control;
    }
  }

  rxn.x = p.x;
  rxn.y = p.y;
  rxn.normal = newNormal;
  return rxn;
}

function translateReaction(layout, rxn, dx, dy) {
  rxn.x += dx;
  rxn.y += dy;

  if (rxn.axis) {
    rxn.axis.a1.x += dx;
    rxn.axis.a1.y += dy;
    rxn.axis.a2.x += dx;
    rxn.axis.a2.y += dy;
  }

  const movedCurrency = new Set();
  for (const node of layout.nodes.values()) {
    if (node.reactionId === rxn.id) {
      node.x += dx;
      node.y += dy;
      movedCurrency.add(node.id);
    }
  }

  for (const seg of layout.segments) {
    if (seg.reactionId !== rxn.id) continue;

    if (seg.kind === 'axis') {
      if (rxn.axis) {
        seg.from = rxn.axis.a1;
        seg.to = rxn.axis.a2;
      } else {
        seg.from = shifted(seg.from, dx, dy);
        seg.to = shifted(seg.to, dx, dy);
      }
    } else if (seg.kind === 'boundary' || seg.kind === 'transport-lane') {
      seg.from = shifted(seg.from, dx, dy);
      seg.to = shifted(seg.to, dx, dy);
    } else {
      ensureCubicControls(seg);
      // Preserve the individual backbone attachment point for each branch.
      if (seg.side === 'substrate') seg.to = shifted(seg.to, dx, dy);
      else if (seg.side === 'product') seg.from = shifted(seg.from, dx, dy);
      if (seg.b2) seg.b2 = shifted(seg.b2, dx, dy);

      if (movedCurrency.has(seg.nodeId)) {
        const node = layout.nodes.get(seg.nodeId);
        if (seg.side === 'substrate') seg.from = { x: node.x, y: node.y };
        else if (seg.side === 'product') seg.to = { x: node.x, y: node.y };
        if (seg.b1) seg.b1 = shifted(seg.b1, dx, dy);
      }
      delete seg.control;
    }
  }
}

function internalCompartment(layout, rxn) {
  const cell = layout.cells?.find((c) => c.id === rxn.cellId);
  if (!cell) return null;
  const available = new Set(cell.compartments.map((c) => c.id));
  return (rxn.compartments ?? []).find((id) => available.has(id))
    ?? cell.compartments.find((c) => c.depth === 0)?.id
    ?? null;
}

function branchSegment(layout, reactionId, nodeId) {
  return (layout?.segments ?? []).find((seg) => isBranch(seg, reactionId) && seg.nodeId === nodeId) ?? null;
}

function isBranch(seg, reactionId) {
  return seg.reactionId === reactionId
    && Boolean(seg.nodeId)
    && seg.kind !== 'axis'
    && seg.kind !== 'boundary'
    && seg.kind !== 'transport-lane';
}

function metaboliteEndpoint(seg) {
  if (seg.side === 'substrate') return seg.from;
  if (seg.side === 'product') return seg.to;
  return null;
}

function reactionEndpoint(seg) {
  if (seg.side === 'substrate') return seg.to;
  if (seg.side === 'product') return seg.from;
  return null;
}

const lerp = (a, b, t) => ({ x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t });
const shifted = (p, dx, dy) => ({ x: p.x + dx, y: p.y + dy });
