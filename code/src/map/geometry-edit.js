/**
 * Direct editing of cell/compartment geometry.
 *
 * Geometry edits are graphical only. They transform the selected compartment,
 * its descendants and the map content biologically located inside that subtree.
 * The biological Model and FBA are never touched.
 */

import { updateConnectedSegments } from '../layout/drag.js';
import { compartmentShape } from '../layout/geometry.js';
import { moveReaction } from '../layout/reaction-edit.js';
import { ENVIRONMENT_KEY } from './boundaries.js';

const MIN_W = 90;
const MIN_H = 64;
const PAD = 10;
// How near a corner a reaction riding a resized side may be pushed.
const SIDE_MARGIN = 18;

/**
 * Resolve a geometry target. An empty `cellId` with the environment's id names
 * the environment.
 *
 * @returns {{cell: object|null, compartment: object, environment: boolean}|null}
 */
export function geometryTarget(map, cellId, compartmentId) {
  const isEnvironment = (cellId === null || cellId === undefined || cellId === '')
    && compartmentId === map?.environment?.id;
  if (isEnvironment && map?.environment?.rect) {
    return { cell: null, compartment: map.environment, environment: true };
  }

  const cell = map?.cells?.find((c) => c.id === cellId);
  const compartment = cell?.compartments?.find((c) => c.id === compartmentId);
  return cell && compartment ? { cell, compartment, environment: false } : null;
}

/**
 * Give a compartment (or the environment) a new rectangle in map units. Same
 * size means a move of the whole subtree. A different size is a resize that
 * keeps the content inside and carries the reactions on the membrane along.
 *
 * @param {{x: number, y: number, w: number, h: number}} requested
 * @returns {{cellId: string|null, compartmentId: string, rect: object}|null}
 *   the rectangle actually applied, or null when the target or rect is invalid
 */
export function editGeometryRect(map, cellId, compartmentId, requested) {
  const target = geometryTarget(map, cellId, compartmentId);
  if (!target || !validRect(requested)) return null;
  if (target.environment) return editEnvironmentRect(map, requested);

  const { cell, compartment } = target;
  const parent = compartment.parentId
    ? cell.compartments.find((c) => c.id === compartment.parentId)?.rect
    : map.environment?.rect;
  if (!parent) return null;

  const oldRect = { ...compartment.rect };
  const parentPad = compartment.parentId ? PAD : 18;
  const moving = nearlyEqual(requested.w, oldRect.w) && nearlyEqual(requested.h, oldRect.h);

  // A move keeps the size and is shifted back inside the parent. A resize stops
  // each side at the parent instead, because shifting would also move the side
  // that is not being pulled.
  let nextRect = clampRect(moving ? requested : clipRect(requested, parent, parentPad), parent, parentPad);
  if (!nextRect) return null;

  const subtree = descendantIds(cell, compartmentId);

  if (moving) {
    const dx = nextRect.x - oldRect.x;
    const dy = nextRect.y - oldRect.y;
    translateSubtree(map, cell, compartmentId, dx, dy,
      compartment.parentId === null ? nextRect : null);
  } else {
    // Resizing creates or removes free drawing space. Descendant compartments,
    // metabolites and internal reactions keep their size and absolute position.
    // The selected parent is not allowed to shrink through any of them.
    // Reactions physically attached to the resized boundary ride it instead.
    const attached = attachedReactions(map, cellId, compartmentId);

    const required = requiredBounds(map, cell, compartmentId, subtree, oldRect, attached);
    if (required) nextRect = containRequired(nextRect, required, parent, parentPad, oldRect);

    compartment.rect = { ...nextRect };
    if (compartment.parentId === null) cell.rect = { ...nextRect };

    followBoundary(map, attached, oldRect, nextRect);
  }

  map.refreshBoundaries?.();
  return { cellId, compartmentId, rect: { ...nextRect } };
}

/* The reactions that sit on a compartment's membrane and ride it. */
function attachedReactions(map, cellId, compartmentId) {
  const boundaryId = `${cellId}/${compartmentId}`;
  const attached = new Set();
  for (const rxn of map.reactions.values()) {
    if (rxn.cellId !== cellId) continue;
    if (rxn.membrane === compartmentId || rxn.boundaryId === boundaryId) attached.add(rxn.id);
  }
  return attached;
}

// Shape

/** The two shape handles: `corner` softens the corners, `oval` rounds the whole outline. */
export const SHAPE_HANDLES = ['corner', 'oval'];

/*
 * The track a shape handle runs along, from the top-left corner: the corner
 * handle along the top edge, the oval handle down the left side. It starts
 * clear of the corner handle and stops clear of the side handle in the middle,
 * so no two handles sit on one another, and where along it the handle stands
 * is its amount, 0 at the corner end. `handleSize` is the corner handle's side
 * in map units, which is what the gaps are made of.
 */
function shapeTrack(rect, which, handleSize) {
  const vertical = which === 'oval';
  const start = vertical ? rect.y : rect.x;
  const length = vertical ? rect.h : rect.w;
  const a = start + handleSize * 1.2;
  const b = Math.max(a + handleSize, start + length / 2 - handleSize * 1.6);
  return { vertical, a, b, at: vertical ? rect.x : rect.y };
}

/** Where a shape handle stands for an amount from 0 to 1. */
export function shapeHandlePoint(rect, which, amount, handleSize) {
  const t = shapeTrack(rect, which, handleSize);
  const v = t.a + Math.min(1, Math.max(0, amount)) * (t.b - t.a);
  return t.vertical ? { x: t.at, y: v } : { x: v, y: t.at };
}

/** The amount, from 0 to 1, a shape handle dragged to `point` stands for. */
export function shapeAmountAt(rect, which, point, handleSize) {
  const t = shapeTrack(rect, which, handleSize);
  const v = t.vertical ? point.y : point.x;
  return Math.min(1, Math.max(0, (v - t.a) / (t.b - t.a)));
}

/**
 * Give a compartment of a cell a new shape: `{ corner, oval }`, either or both,
 * each from 0 to 1 (see compartmentShape). Its rectangle does not change. The
 * transporters on its membrane move onto the new outline and turn with it, and
 * take their own metabolites along.
 */
export function editCompartmentShape(map, cellId, compartmentId, shape = {}) {
  const target = geometryTarget(map, cellId, compartmentId);
  if (!target || target.environment) return null;
  const { compartment } = target;

  const next = compartmentShape(compartment);
  for (const key of SHAPE_HANDLES) {
    const value = Number(shape?.[key]);
    if (Number.isFinite(value)) next[key] = Math.min(1, Math.max(0, value));
  }
  compartment.shape = next;

  for (const id of attachedReactions(map, cellId, compartmentId)) {
    const rxn = map.reactions.get(id);
    if (!rxn?.onMembrane || !rxn.boundaryId) continue;
    const owned = new Map();
    for (const node of map.nodes.values()) {
      if (node.reactionId === id) owned.set(node.id, { x: node.x, y: node.y });
    }
    moveReaction(map, id, rxn.x, rxn.y);
    const moved = new Map();
    for (const [nodeId, before] of owned) {
      const node = map.nodes.get(nodeId);
      moved.set(nodeId, { x: node.x - before.x, y: node.y - before.y });
    }
    carryHandles(map, new Set([id]), (nodeId) => moved.get(nodeId));
    for (const nodeId of owned.keys()) updateConnectedSegments(map, nodeId);
  }

  map.refreshBoundaries?.();
  return { cellId, compartmentId, shape: { ...next } };
}

/**
 * Move a compartment, its entire visual subtree and all graphical content that
 * belongs to it by (dx, dy), preserving relative geometry exactly. Nothing is
 * clamped and the boundaries are not refreshed: the caller has already decided
 * the move is allowed, and refreshes once it has moved everything it moves. A
 * cell's own (root) compartment also moves the cell, to `cellRect`.
 */
export function translateSubtree(map, cell, compartmentId, dx, dy, cellRect = null) {
  const cellId = cell.id;
  const subtree = descendantIds(cell, compartmentId);
  const transform = translation(dx, dy);

  for (const comp of cell.compartments) {
    if (!subtree.has(comp.id)) continue;
    comp.rect = translateRect(comp.rect, dx, dy);
  }

  if (cellRect) cell.rect = { ...cellRect };

  const affectedNodes = new Set();
  for (const node of map.nodes.values()) {
    if (node.cellId !== cellId || !subtree.has(node.compartment)) continue;
    node.x += dx;
    node.y += dy;
    affectedNodes.add(node.id);
  }

  const affectedReactions = new Set();
  for (const rxn of map.reactions.values()) {
    if (!reactionInSubtree(rxn, cellId, subtree)) continue;

    transformReaction(rxn, transform);
    affectedReactions.add(rxn.id);

    for (const node of map.nodes.values()) {
      if (node.reactionId !== rxn.id || affectedNodes.has(node.id)) continue;
      node.x += dx;
      node.y += dy;
      affectedNodes.add(node.id);
    }
  }

  transformSegments(map, affectedReactions, transform, affectedNodes);
  const delta = { x: dx, y: dy };
  carryHandles(map, affectedReactions, (id) => (affectedNodes.has(id) ? delta : null));
  for (const nodeId of affectedNodes) updateConnectedSegments(map, nodeId);
}

/**
 * Whether a reaction travels with a compartment subtree of its cell when that
 * subtree is moved: it runs in one of those compartments, sits on one of their
 * membranes, or is attached to one of their boundaries.
 */
export function reactionInSubtree(rxn, cellId, subtree) {
  if (rxn.cellId !== cellId) return false;
  return (rxn.compartments ?? []).some((id) => subtree.has(id))
    || subtree.has(rxn.membrane)
    || boundaryBelongs(rxn.boundaryId, cellId, subtree);
}

function editEnvironmentRect(map, requested) {
  const environment = map.environment;
  const oldRect = { ...environment.rect };
  const moving = nearlyEqual(requested.w, oldRect.w) && nearlyEqual(requested.h, oldRect.h);

  if (moving) {
    const nextRect = {
      x: Math.max(0, requested.x),
      y: Math.max(0, requested.y),
      w: oldRect.w,
      h: oldRect.h,
    };
    const dx = nextRect.x - oldRect.x;
    const dy = nextRect.y - oldRect.y;
    if (Math.abs(dx) + Math.abs(dy) > 1e-9) translateWholeMap(map, dx, dy);
    environment.rect = nextRect;
    map.canvas.width = Math.max(map.canvas.width, nextRect.x + nextRect.w + 24);
    map.canvas.height = Math.max(map.canvas.height, nextRect.y + nextRect.h + 24);
  } else {
    const attached = new Set();
    for (const rxn of map.reactions.values()) {
      if (rxn.boundaryId === ENVIRONMENT_KEY) attached.add(rxn.id);
    }

    // As for a compartment: what rides the boundary does not hold it out, and
    // content nearer the edge than the margin only stops it where it already is.
    const required = wholeMapRequiredBounds(map, attached);
    let x1 = Math.max(0, requested.x);
    let y1 = Math.max(0, requested.y);
    let x2 = requested.x + requested.w;
    let y2 = requested.y + requested.h;

    if (required) {
      x1 = Math.min(x1, Math.max(0, required.x1 - 18, oldRect.x));
      y1 = Math.min(y1, Math.max(0, required.y1 - 18, oldRect.y));
      x2 = Math.max(x2, Math.min(required.x2 + 18, oldRect.x + oldRect.w));
      y2 = Math.max(y2, Math.min(required.y2 + 18, oldRect.y + oldRect.h));
    }

    const w = Math.max(MIN_W, x2 - x1);
    const h = Math.max(MIN_H, y2 - y1);
    const nextRect = { x: x1, y: y1, w, h };

    followBoundary(map, attached, oldRect, nextRect);

    environment.rect = nextRect;
    map.canvas.width = Math.max(map.canvas.width, nextRect.x + nextRect.w + 24);
    map.canvas.height = Math.max(map.canvas.height, nextRect.y + nextRect.h + 24);
  }

  map.refreshBoundaries?.();
  return { cellId: null, compartmentId: environment.id, rect: { ...environment.rect } };
}

/**
 * Shift every cell, compartment, node, reaction, segment and text by (dx, dy)
 * map units. The environment rect and the canvas size are left to the caller.
 */
export function translateWholeMap(map, dx, dy) {
  for (const cell of map.cells) {
    cell.rect = translateRect(cell.rect, dx, dy);
    for (const comp of cell.compartments) comp.rect = translateRect(comp.rect, dx, dy);
  }

  for (const node of map.nodes.values()) {
    node.x += dx;
    node.y += dy;
  }

  const transform = translation(dx, dy);
  const affectedReactions = new Set();
  for (const rxn of map.reactions.values()) {
    transformReaction(rxn, transform);
    affectedReactions.add(rxn.id);
  }
  transformSegments(map, affectedReactions, transform);
  for (const node of map.nodes.values()) updateConnectedSegments(map, node.id);

  for (const text of map.texts?.values?.() ?? []) {
    if (Number.isFinite(text.x)) text.x += dx;
    if (Number.isFinite(text.y)) text.y += dy;
  }
}

function wholeMapRequiredBounds(map, attached) {
  const bounds = [];
  for (const cell of map.cells) {
    bounds.push({
      x1: cell.rect.x,
      y1: cell.rect.y,
      x2: cell.rect.x + cell.rect.w,
      y2: cell.rect.y + cell.rect.h,
    });
  }
  for (const node of map.nodes.values()) {
    if (node.reactionId && attached.has(node.reactionId)) continue;
    const r = Number(node.r ?? 6);
    bounds.push({ x1: node.x - r, y1: node.y - r, x2: node.x + r, y2: node.y + r });
  }
  for (const rxn of map.reactions.values()) {
    if (attached.has(rxn.id)) continue;
    bounds.push({ x1: rxn.x - 18, y1: rxn.y - 18, x2: rxn.x + 18, y2: rxn.y + 18 });
  }
  for (const text of map.texts?.values?.() ?? []) {
    if (!Number.isFinite(text.x) || !Number.isFinite(text.y)) continue;
    bounds.push({ x1: text.x - 8, y1: text.y - 16, x2: text.x + 120, y2: text.y + 8 });
  }
  return unionBounds(bounds);
}

/** The side of `rect` nearest to `point`: 'n', 'e', 's' or 'w', or null for bad input. */
export function nearestRectEdge(rect, point) {
  if (!validRect(rect) || !point || !Number.isFinite(Number(point.x)) || !Number.isFinite(Number(point.y))) return null;
  const x = Number(point.x);
  const y = Number(point.y);
  const distances = [
    ['n', Math.abs(y - rect.y)],
    ['e', Math.abs(x - (rect.x + rect.w))],
    ['s', Math.abs(y - (rect.y + rect.h))],
    ['w', Math.abs(x - rect.x)],
  ];
  distances.sort((a, b) => a[1] - b[1]);
  return distances[0][0];
}

/**
 * The rectangle a handle drag from `startPoint` to `point` asks for. `handle` is
 * 'move' or a combination of 'n', 'e', 's', 'w'. A resize keeps at least
 * MIN_W x MIN_H by holding back the dragged side. Not clamped to any parent.
 */
export function rectFromHandle(startRect, handle, startPoint, point) {
  if (!validRect(startRect) || !startPoint || !point) return null;
  const dx = point.x - startPoint.x;
  const dy = point.y - startPoint.y;

  if (handle === 'move') {
    return { x: startRect.x + dx, y: startRect.y + dy, w: startRect.w, h: startRect.h };
  }

  let x1 = startRect.x;
  let y1 = startRect.y;
  let x2 = startRect.x + startRect.w;
  let y2 = startRect.y + startRect.h;

  if (handle.includes('w')) x1 += dx;
  if (handle.includes('e')) x2 += dx;
  if (handle.includes('n')) y1 += dy;
  if (handle.includes('s')) y2 += dy;

  if (x2 - x1 < MIN_W) {
    if (handle.includes('w')) x1 = x2 - MIN_W;
    else x2 = x1 + MIN_W;
  }
  if (y2 - y1 < MIN_H) {
    if (handle.includes('n')) y1 = y2 - MIN_H;
    else y2 = y1 + MIN_H;
  }

  return { x: x1, y: y1, w: x2 - x1, h: y2 - y1 };
}

/** `rootId` and the ids of every compartment of `cell` nested under it, as a Set. */
export function descendantIds(cell, rootId) {
  const out = new Set([rootId]);
  let changed = true;
  while (changed) {
    changed = false;
    for (const comp of cell.compartments) {
      if (comp.parentId && out.has(comp.parentId) && !out.has(comp.id)) {
        out.add(comp.id);
        changed = true;
      }
    }
  }
  return out;
}

function clampRect(r, parent, pad) {
  // The minimum size adapts to very small parents so containment always wins:
  // a child compartment must never be allowed to protrude beyond its parent.
  const innerW = Math.max(1, parent.w - pad * 2);
  const innerH = Math.max(1, parent.h - pad * 2);
  const minW = Math.min(MIN_W, innerW);
  const minH = Math.min(MIN_H, innerH);
  const w = Math.min(Math.max(minW, r.w), innerW);
  const h = Math.min(Math.max(minH, r.h), innerH);
  const x = Math.min(Math.max(r.x, parent.x + pad), parent.x + parent.w - pad - w);
  const y = Math.min(Math.max(r.y, parent.y + pad), parent.y + parent.h - pad - h);
  return { x, y, w, h };
}

function clipRect(r, parent, pad) {
  const x1 = Math.max(r.x, parent.x + pad);
  const y1 = Math.max(r.y, parent.y + pad);
  const x2 = Math.min(r.x + r.w, parent.x + parent.w - pad);
  const y2 = Math.min(r.y + r.h, parent.y + parent.h - pad);
  return { x: x1, y: y1, w: x2 - x1, h: y2 - y1 };
}

function nearlyEqual(a, b, eps = 1e-7) {
  return Math.abs(Number(a) - Number(b)) <= eps;
}

function translation(dx, dy) {
  return {
    from: { x: 0, y: 0, w: 1, h: 1 },
    to: { x: dx, y: dy, w: 1, h: 1 },
    sx: 1,
    sy: 1,
  };
}

function transformPoint(p, t) {
  return {
    x: t.to.x + (p.x - t.from.x) * t.sx,
    y: t.to.y + (p.y - t.from.y) * t.sy,
  };
}

function translateRect(r, dx, dy) {
  return { x: r.x + dx, y: r.y + dy, w: r.w, h: r.h };
}

function unionBounds(bounds) {
  const items = bounds.filter(Boolean);
  if (!items.length) return null;
  return {
    x1: Math.min(...items.map((b) => b.x1)),
    y1: Math.min(...items.map((b) => b.y1)),
    x2: Math.max(...items.map((b) => b.x2)),
    y2: Math.max(...items.map((b) => b.y2)),
  };
}

/*
 * What a resized compartment must keep inside: its descendants and what is
 * drawn inside it now. The reactions attached to its own boundary and their
 * metabolites are left out, since they ride the boundary, and so is whatever
 * already lies outside it (a transporter's far side, say). Counting either one
 * would hold the boundary out beyond them, and since an attached reaction then
 * follows the boundary, every step of a resize would push it out again.
 */
function requiredBounds(map, cell, rootId, subtree, rect, attached) {
  const bounds = [];
  const inside = (p) => p.x >= rect.x && p.x <= rect.x + rect.w
    && p.y >= rect.y && p.y <= rect.y + rect.h;

  for (const comp of cell.compartments) {
    if (comp.id === rootId || !subtree.has(comp.id)) continue;
    bounds.push({
      x1: comp.rect.x,
      y1: comp.rect.y,
      x2: comp.rect.x + comp.rect.w,
      y2: comp.rect.y + comp.rect.h,
    });
  }

  for (const node of map.nodes.values()) {
    if (node.cellId !== cell.id || !subtree.has(node.compartment)) continue;
    if (node.reactionId && attached.has(node.reactionId)) continue;
    if (!inside(node)) continue;
    const r = Number(node.r ?? 6);
    bounds.push({ x1: node.x - r, y1: node.y - r, x2: node.x + r, y2: node.y + r });
  }

  for (const rxn of map.reactions.values()) {
    if (!reactionInSubtree(rxn, cell.id, subtree)) continue;
    if (attached.has(rxn.id) || !inside(rxn)) continue;
    bounds.push({ x1: rxn.x - 18, y1: rxn.y - 18, x2: rxn.x + 18, y2: rxn.y + 18 });
  }

  return unionBounds(bounds);
}

function containRequired(rect, required, parent, parentPad, current) {
  const pad = PAD;
  const parentLeft = parent.x + parentPad;
  const parentTop = parent.y + parentPad;
  const parentRight = parent.x + parent.w - parentPad;
  const parentBottom = parent.y + parent.h - parentPad;

  // Content already nearer the boundary than the margin stops it where it is
  // and does not push it out: a side the user is not pulling stays put.
  const left = Math.max(required.x1 - pad, current.x);
  const top = Math.max(required.y1 - pad, current.y);
  const right = Math.min(required.x2 + pad, current.x + current.w);
  const bottom = Math.min(required.y2 + pad, current.y + current.h);

  let x1 = Math.min(rect.x, left);
  let y1 = Math.min(rect.y, top);
  let x2 = Math.max(rect.x + rect.w, right);
  let y2 = Math.max(rect.y + rect.h, bottom);

  x1 = Math.max(parentLeft, x1);
  y1 = Math.max(parentTop, y1);
  x2 = Math.min(parentRight, x2);
  y2 = Math.min(parentBottom, y2);

  if (x2 - x1 < MIN_W) x2 = Math.min(parentRight, x1 + MIN_W);
  if (y2 - y1 < MIN_H) y2 = Math.min(parentBottom, y1 + MIN_H);

  // Existing descendants are already valid inside the parent, so these final
  // corrections can always preserve them while staying within the parent.
  if (x1 > left) x1 = Math.max(parentLeft, left);
  if (y1 > top) y1 = Math.max(parentTop, top);
  if (x2 < right) x2 = Math.min(parentRight, right);
  if (y2 < bottom) y2 = Math.min(parentBottom, bottom);

  return { x: x1, y: y1, w: x2 - x1, h: y2 - y1 };
}

/*
 * Bring the reactions attached to a resized boundary, with their own
 * metabolites, along with it (see followSide).
 */
function followBoundary(map, reactionIds, from, to) {
  const affectedReactions = new Set();
  const transforms = new Map();
  const moved = new Map();
  for (const id of reactionIds) {
    const rxn = map.reactions.get(id);
    if (!rxn) continue;
    const next = followSide(from, to, rxn);
    const dx = next.x - rxn.x;
    const dy = next.y - rxn.y;
    if (Math.abs(dx) + Math.abs(dy) < 1e-9) continue;
    const transform = translation(dx, dy);
    transformReaction(rxn, transform);
    transforms.set(rxn.id, transform);
    affectedReactions.add(rxn.id);

    for (const node of map.nodes.values()) {
      if (node.reactionId !== rxn.id) continue;
      node.x += dx;
      node.y += dy;
      moved.set(node.id, { x: dx, y: dy });
    }
  }
  const movedNodes = new Set(moved.keys());
  transformSegmentsByReaction(map, affectedReactions, transforms, movedNodes);
  carryHandles(map, affectedReactions, (id) => moved.get(id));
  for (const nodeId of movedNodes) updateConnectedSegments(map, nodeId);
}

/*
 * A metabolite that moved takes its b1 along on the curves of the reactions
 * that did not move as well: a metabolite one reaction owns can be drawn into
 * the next one too, whose curve would otherwise keep its old bend.
 */
function carryHandles(map, affectedReactions, deltaOf) {
  for (const seg of map.segments) {
    if (!seg.b1 || !seg.nodeId || affectedReactions.has(seg.reactionId)) continue;
    const d = deltaOf(seg.nodeId);
    if (d) seg.b1 = { x: seg.b1.x + d.x, y: seg.b1.y + d.y };
  }
}

/*
 * Where a point on a resized rect's nearest side goes: across, by as far as that
 * side moved, so it stays on the membrane; along it, nowhere, unless the side
 * has become too short for it. Spreading it along in proportion to the new
 * length would slide every transporter away from the metabolites drawn
 * against it.
 */
function followSide(from, to, point) {
  const distances = [
    ['left', Math.abs(point.x - from.x)],
    ['right', Math.abs(point.x - (from.x + from.w))],
    ['top', Math.abs(point.y - from.y)],
    ['bottom', Math.abs(point.y - (from.y + from.h))],
  ].sort((a, b) => a[1] - b[1]);
  const side = distances[0][0];

  if (side === 'left' || side === 'right') {
    const dx = side === 'left' ? to.x - from.x : (to.x + to.w) - (from.x + from.w);
    return { x: point.x + dx, y: keepAlong(point.y, from.y, from.h, to.y, to.h) };
  }
  const dy = side === 'top' ? to.y - from.y : (to.y + to.h) - (from.y + from.h);
  return { x: keepAlong(point.x, from.x, from.w, to.x, to.w), y: point.y + dy };
}

/*
 * A coordinate along a side that is now [start, start + length]: unchanged
 * while the side still reaches it with SIDE_MARGIN to spare, else as near as
 * that allows. A point already nearer a corner than the margin keeps the
 * distance it had.
 */
function keepAlong(v, oldStart, oldLength, start, length) {
  const lead = Math.min(SIDE_MARGIN, Math.max(0, v - oldStart));
  const trail = Math.min(SIDE_MARGIN, Math.max(0, oldStart + oldLength - v));
  return Math.min(Math.max(v, start + lead), start + length - trail);
}

function transformSegments(map, affectedReactions, transform, movedNodes = null) {
  const transforms = new Map([...affectedReactions].map((id) => [id, transform]));
  transformSegmentsByReaction(map, affectedReactions, transforms, movedNodes);
}

/*
 * `movedNodes`, when given, are the metabolites that moved with the reactions.
 * A branch's b1 belongs to its metabolite and b2 to its reaction, as in Escher
 * and as reaction-edit.js moves them: a reaction that moves away from a
 * metabolite that stays takes b2 with it and leaves b1. Taking b1 as well would
 * bend every curve to a metabolite left behind into a loop as long as the move.
 */
function transformSegmentsByReaction(map, affectedReactions, transforms, movedNodes = null) {
  for (const seg of map.segments) {
    if (!affectedReactions.has(seg.reactionId)) continue;
    const rxn = map.reactions.get(seg.reactionId);
    const transform = transforms.get(seg.reactionId);
    if (!rxn || !transform) continue;

    if (seg.kind === 'axis' && rxn.axis) {
      seg.from = rxn.axis.a1;
      seg.to = rxn.axis.a2;
    } else if (seg.kind === 'transport-lane') {
      const lane = rxn.transportLanes?.find((l) => l.base === seg.transportBase);
      if (lane) {
        seg.from = lane.insidePoint;
        seg.to = lane.outsidePoint;
      } else {
        seg.from = transformPoint(seg.from, transform);
        seg.to = transformPoint(seg.to, transform);
      }
    } else if (seg.kind === 'boundary') {
      seg.from = transformPoint(seg.from, transform);
      seg.to = transformPoint(seg.to, transform);
    } else if (seg.transportLane) {
      if (seg.side === 'substrate') seg.to = transformPoint(seg.to, transform);
      else if (seg.side === 'product') seg.from = transformPoint(seg.from, transform);
    } else if (rxn.axis) {
      if (seg.side === 'substrate') seg.to = rxn.axis.a1;
      else if (seg.side === 'product') seg.from = rxn.axis.a2;
    }

    if (seg.control) seg.control = transformPoint(seg.control, transform);
    const metaboliteMoved = !movedNodes || !seg.nodeId || movedNodes.has(seg.nodeId);
    if (seg.b1 && metaboliteMoved) seg.b1 = transformPoint(seg.b1, transform);
    if (seg.b2) seg.b2 = transformPoint(seg.b2, transform);
  }
}

function transformReaction(rxn, t) {
  const p = transformPoint(rxn, t);
  rxn.x = p.x;
  rxn.y = p.y;

  if (rxn.axis) {
    rxn.axis.a1 = transformPoint(rxn.axis.a1, t);
    rxn.axis.a2 = transformPoint(rxn.axis.a2, t);
    const dx = rxn.axis.a2.x - rxn.axis.a1.x;
    const dy = rxn.axis.a2.y - rxn.axis.a1.y;
    const len = Math.hypot(dx, dy) || 1;
    rxn.axis.dir = { x: dx / len, y: dy / len };
  }

  if (Array.isArray(rxn.transportLanes)) {
    for (const lane of rxn.transportLanes) {
      lane.insidePoint = transformPoint(lane.insidePoint, t);
      lane.outsidePoint = transformPoint(lane.outsidePoint, t);
    }
  }
}

function boundaryBelongs(boundaryId, cellId, subtree) {
  if (!boundaryId || !boundaryId.startsWith(`${cellId}/`)) return false;
  return subtree.has(boundaryId.slice(cellId.length + 1));
}

function validRect(r) {
  return r && [r.x, r.y, r.w, r.h].every(Number.isFinite) && r.w > 0 && r.h > 0;
}
