/**
 * Multi-selection helpers for EditableMap.
 *
 * Selection is graphical UI state. It never changes the Model or FBA.
 */

import { isSecondaryNode } from './metabolite-label.js';
import { textFontAdvance } from '../core/text-style.js';
import { descendantIds, reactionInSubtree, translateSubtree } from './geometry-edit.js';
import { isDraggableNode, nodeFitsAt, updateConnectedSegments } from '../layout/drag.js';
import { reactionCentreFits, straightenLinearReactions } from '../layout/reaction-edit.js';

/** A selection with nothing in it: Sets of node, reaction and text ids and of compartment keys. */
export function emptySelection() {
  return { nodes: new Set(), reactions: new Set(), texts: new Set(), compartments: new Set() };
}

/** How many items a selection holds, over all four kinds. */
export function selectionSize(selection) {
  return (selection?.nodes?.size ?? 0)
    + (selection?.reactions?.size ?? 0)
    + (selection?.texts?.size ?? 0)
    + (selection?.compartments?.size ?? 0);
}

/**
 * A compartment in a selection: its cell and its own id, as one Set entry.
 * Compartment ids repeat from cell to cell, so the id alone is not enough.
 */
export function compartmentKey(cellId, compartmentId) {
  return JSON.stringify([cellId ?? null, compartmentId]);
}

/** The inverse of compartmentKey: `{ cellId, compartmentId }`, or null for a malformed key. */
export function parseCompartmentKey(key) {
  try {
    const [cellId, compartmentId] = JSON.parse(key);
    return { cellId, compartmentId };
  } catch {
    return null;
  }
}

/*
 * Where a piece of free text sits, for selecting, moving and aligning it.
 *
 * The renderer anchors annotations at the start of the alphabetic baseline, so
 * (x, y) is the bottom-left of the glyphs, not their middle. Everything else on
 * the map is selected by its centre falling inside the rectangle, and text has
 * to answer the same question, so the box is reconstructed here. The width is an
 * estimate (measuring glyphs needs a DOM) from the average character width
 * of the text's own font (text-style.js), a little generous, which is the safe
 * direction, since it only ever makes a label easier to catch.
 */
export function textBox(annotation) {
  const size = Number(annotation?.fontSize ?? 22);
  const width = String(annotation?.text ?? '').length * size * textFontAdvance(annotation?.fontFamily);
  return {
    left: annotation.x,
    right: annotation.x + width,
    top: annotation.y - size,
    bottom: annotation.y,
  };
}

/** The middle of a text's estimated box (see textBox). */
export function textCentre(annotation) {
  const box = textBox(annotation);
  return { x: (box.left + box.right) / 2, y: (box.top + box.bottom) / 2 };
}

/**
 * What a rectangle catches. With `includeSecondary` false (the Cofactors toggle
 * off) secondary nodes are not on screen, so they are not caught either: a
 * rectangle followed by Delete must not remove what nobody could see.
 */
export function selectableInRect(map, rect, { includeSecondary = true } = {}) {
  const box = normalizeRect(rect);
  const nodes = new Set();
  const reactions = new Set();

  if (!map || !box) return { nodes, reactions };

  for (const node of map.nodes.values()) {
    if (node.hidden) continue;
    if (!['metabolite', 'medium'].includes(node.kind)) continue;
    if (!includeSecondary && isSecondaryNode(node)) continue;
    if (contains(box, node.x, node.y)) nodes.add(node.id);
  }

  for (const [id, rxn] of map.reactions) {
    if (contains(box, rxn.x, rxn.y)) reactions.add(id);
  }

  // Annotations are part of the drawing, so a rectangle dragged over a pathway
  // takes its labels with it. Without them, deleting a pathway would leave its
  // metabolite names floating over nothing, with no way to catch them in bulk.
  const texts = new Set();
  for (const [id, annotation] of map.texts ?? []) {
    const centre = textCentre(annotation);
    if (contains(box, centre.x, centre.y)) texts.add(id);
  }

  // A compartment is caught only when the rectangle takes in all of it. What
  // lies inside is caught already; its membrane should come along with it, not
  // stay behind and hold its contents back. The shared medium is the frame of
  // the whole drawing and is never caught.
  const compartments = new Set();
  for (const cell of map.cells ?? []) {
    for (const comp of cell.compartments ?? []) {
      if (encloses(box, comp.rect)) compartments.add(compartmentKey(cell.id, comp.id));
    }
  }

  return { nodes, reactions, texts, compartments };
}

/**
 * Move every member of a selection by (dx, dy), each the way dragging it alone
 * would: a metabolite is kept in its compartment, a transporter on its
 * membrane, a straight reaction between its two metabolites.
 *
 * `rigid` is for a selection dragged as one (dragSelection), whose step is
 * already one every member may take: metabolites then move by exactly that
 * step, with no clamp of their own (which would shove one that starts out of
 * place sideways while the rest go straight), and a transporter goes only as
 * far as the step runs along its membrane. Pushed into the membrane instead,
 * its projection back onto the outline would walk it round the corners.
 */
export function translateSelection(map, selection, dx, dy, { rigid = false } = {}) {
  if (!map || !selection || !Number.isFinite(dx) || !Number.isFinite(dy)) return null;
  if (dx === 0 && dy === 0) return { nodes: [], reactions: [] };

  const movedNodes = [];
  const movedReactions = [];

  // Move metabolite endpoints first (moves b1), then reaction skeletons (moves b2).
  // When both ends of one branch are selected this translates the whole curve.
  // A straight reaction follows its two metabolites, and carries the cofactor
  // nodes it owns. With one of its ends in the selection it has therefore moved
  // already by the time its own turn comes, and so have those nodes: moving
  // either again would count the same drag twice.
  const selectedNodes = new Set(selection.nodes ?? []);
  const carried = (rxn) => Boolean(rxn?.linear)
    && (selectedNodes.has(rxn.linear.from) || selectedNodes.has(rxn.linear.to));

  for (const id of selection.nodes ?? []) {
    const node = map.nodes.get(id);
    if (!node || !map.canMoveNode(id)) continue;
    if (node.reactionId && carried(map.reactions.get(node.reactionId))
      && node.id !== map.reactions.get(node.reactionId).linear.from
      && node.id !== map.reactions.get(node.reactionId).linear.to) continue;
    if (rigid) {
      node.x += dx;
      node.y += dy;
      updateConnectedSegments(map, id, { x: dx, y: dy });
      movedNodes.push(id);
      continue;
    }
    const before = { x: node.x, y: node.y };
    const moved = map.moveNode(id, node.x + dx, node.y + dy);
    if (moved && (moved.x !== before.x || moved.y !== before.y)) movedNodes.push(id);
  }
  // A straight reaction goes where its two metabolites go (moveNode does this
  // for each node it moves).
  if (rigid && movedNodes.length) straightenLinearReactions(map, movedNodes);

  for (const id of selection.reactions ?? []) {
    const rxn = map.reactions.get(id);
    if (!rxn || !map.canMoveReaction(id)) continue;
    if (carried(rxn)) { movedReactions.push(id); continue; }
    const before = { x: rxn.x, y: rxn.y };
    let [tx, ty] = [rxn.x + dx, rxn.y + dy];
    if (rigid && rxn.onMembrane && rxn.boundaryId) {
      const n = rxn.normal ?? { x: 1, y: 0 };
      const along = -dx * n.y + dy * n.x;
      if (Math.abs(along) < 1e-9) continue;
      [tx, ty] = [rxn.x - n.y * along, rxn.y + n.x * along];
    }
    const moved = map.moveReaction(id, tx, ty);
    if (moved && (moved.x !== before.x || moved.y !== before.y)) movedReactions.push(id);
  }

  const movedTexts = [];
  for (const id of selection.texts ?? []) {
    const annotation = map.texts?.get(id);
    if (!annotation) continue;
    if (map.moveText(id, annotation.x + dx, annotation.y + dy)) movedTexts.push(id);
  }

  return { nodes: movedNodes, reactions: movedReactions, texts: movedTexts };
}

/**
 * Move a selection by (dx, dy) the way the map's drag does.
 *
 * `translateSelection` alone pulls a selection round an organelle apart: the
 * organelle stays where it is, so what is selected inside it stops at its
 * membrane while the rest goes on, and what is selected around it runs into it.
 * A cofactor node selected with its reaction also goes twice as fast: it moves,
 * and then its reaction carries it. Here:
 *
 * - Selected compartments move first, whole, with everything that belongs to
 *   them (the same move as dragging a compartment's centre handle). They are
 *   then already where they are going when the rest of the selection follows,
 *   so nothing in the selection collides with them.
 * - The selection goes only as far as all of it can: its compartments stay
 *   inside their parents and off the compartments beside them that are not
 *   moving, and its metabolites and reaction centres inside their compartments
 *   and out of the organelles there. When one would hit a wall the whole
 *   selection stops, or slides along it, together. (Moving each member as far
 *   as it alone could go would pile the ones at the wall up against it while
 *   the rest went on, and stretch the curves between them across the map.)
 * - A cofactor node owned by a selected reaction is left to that reaction.
 * - Everything else moves as `translateSelection` moves it.
 *
 * Returns what moved, plus `compartments` (keys) and the step actually taken
 * (`dx`, `dy`), which is less than asked when a wall held the selection back.
 */
export function dragSelection(map, selection, dx, dy) {
  if (!map || !selection || !Number.isFinite(dx) || !Number.isFinite(dy)) return null;
  const none = { nodes: [], reactions: [], texts: [], compartments: [], dx: 0, dy: 0 };
  if (dx === 0 && dy === 0) return none;

  const plan = planGroupMove(map, selection);
  const step = allowedStep(map, plan, dx, dy);
  if (step.dx === 0 && step.dy === 0) return none;

  const compartments = [];
  for (const { cell, compartment } of plan.compartments) {
    const cellRect = compartment.parentId
      ? null
      : { ...cell.rect, x: cell.rect.x + step.dx, y: cell.rect.y + step.dy };
    translateSubtree(map, cell, compartment.id, step.dx, step.dy, cellRect);
    compartments.push(compartmentKey(cell.id, compartment.id));
  }
  if (compartments.length) map.refreshBoundaries?.();

  // Selected metabolites drawn inside a moving organelle without belonging to
  // it go exactly with it. They are already where their compartment says they
  // may not be, so moving them the ordinary way pulls them out of it.
  const riders = [];
  for (const id of plan.riders) {
    const node = map.nodes.get(id);
    if (!node) continue;
    const before = { x: node.x, y: node.y };
    node.x = before.x + step.dx;
    node.y = before.y + step.dy;
    updateConnectedSegments(map, id, { x: node.x - before.x, y: node.y - before.y });
    riders.push(id);
  }
  if (riders.length) straightenLinearReactions(map, riders);

  const moved = translateSelection(map, plan.rest, step.dx, step.dy, { rigid: true })
    ?? { nodes: [], reactions: [], texts: [] };
  return {
    nodes: [...riders, ...moved.nodes],
    reactions: [...moved.reactions, ...plan.carriedReactions],
    texts: moved.texts ?? [],
    compartments,
    dx: step.dx,
    dy: step.dy,
  };
}

/*
 * Who moves what in a dragged selection: the selected compartments with no
 * selected compartment round them (each carries its subtree), and the rest,
 * which `translateSelection` moves member by member.
 */
function planGroupMove(map, selection) {
  const chosen = selection.compartments ?? new Set();
  const compartments = [];
  const carried = new Map();
  for (const key of chosen) {
    const ref = parseCompartmentKey(key);
    const cell = ref && (map.cells ?? []).find((c) => c.id === ref.cellId);
    const compartment = cell?.compartments?.find((c) => c.id === ref.compartmentId);
    if (!compartment) continue;
    // Nested in another selected compartment: that one carries it.
    let parentId = compartment.parentId;
    let nested = false;
    for (let i = 0; parentId && i <= cell.compartments.length; i += 1) {
      if (chosen.has(compartmentKey(cell.id, parentId))) { nested = true; break; }
      parentId = cell.compartments.find((c) => c.id === parentId)?.parentId;
    }
    if (nested) continue;
    compartments.push({ cell, compartment });
    const subtree = carried.get(cell.id) ?? new Set();
    for (const id of descendantIds(cell, compartment.id)) subtree.add(id);
    carried.set(cell.id, subtree);
  }

  const reactionCarried = (rid) => {
    const rxn = map.reactions.get(rid);
    const subtree = rxn && carried.get(rxn.cellId);
    return Boolean(subtree) && reactionInSubtree(rxn, rxn.cellId, subtree);
  };
  // Straight reactions that follow a selected end, and the nodes that are some
  // straight reaction's end, are translateSelection's business.
  const selectedNodes = new Set(selection.nodes ?? []);
  const followsEnd = (rid) => {
    const line = map.reactions.get(rid)?.linear;
    return Boolean(line) && (selectedNodes.has(line.from) || selectedNodes.has(line.to));
  };
  const lineEnds = new Set();
  for (const rxn of map.reactions.values()) {
    if (rxn.linear?.from) lineEnds.add(rxn.linear.from);
    if (rxn.linear?.to) lineEnds.add(rxn.linear.to);
  }
  const selectedReactions = new Set(selection.reactions ?? []);

  // Inside the outline of a compartment that moves, whatever it is drawn as
  // belonging to: a cytosolic metabolite of an inner-membrane transporter
  // drawn in the intermembrane space, say.
  const insideMoving = (p) => compartments.some(({ compartment: { rect: r } }) => r
    && p.x > r.x && p.x < r.x + r.w && p.y > r.y && p.y < r.y + r.h);

  const rest = {
    nodes: new Set(),
    reactions: new Set(),
    texts: new Set(selection.texts ?? []),
    compartments: new Set(),
  };
  const riders = [];
  for (const id of selection.nodes ?? []) {
    const node = map.nodes.get(id);
    if (!node) continue;
    if (carried.get(node.cellId)?.has(node.compartment)) continue;
    if (node.reactionId) {
      if (reactionCarried(node.reactionId)) continue;
      // Its reaction is selected and carries it.
      if (selectedReactions.has(node.reactionId) && map.reactions.has(node.reactionId)
        && !followsEnd(node.reactionId) && !lineEnds.has(id)) continue;
    }
    if (insideMoving(node) && isDraggableNode(node) && map.canMoveNode?.(id) !== false
      && !lineEnds.has(id)) {
      riders.push(id);
      continue;
    }
    rest.nodes.add(id);
  }
  const carriedReactions = [];
  for (const id of selection.reactions ?? []) {
    if (reactionCarried(id)) carriedReactions.push(id);
    else rest.reactions.add(id);
  }

  // What the walls hold back: every metabolite and reaction centre the drag
  // moves one by one that stands where it may now. One already out of place
  // (left there by hand before) is not held to it, or it would pin the whole
  // selection.
  const held = { nodes: [], reactions: [] };
  for (const id of rest.nodes) {
    const node = map.nodes.get(id);
    if (node && isDraggableNode(node) && map.canMoveNode?.(id) !== false
      && nodeFitsAt(map, node, node.x, node.y)) held.nodes.push(node);
  }
  for (const id of rest.reactions) {
    const rxn = map.reactions.get(id);
    if (rxn && reactionCentreFits(map, id, rxn.x, rxn.y)) held.reactions.push(rxn);
  }
  // The cells and compartments that move along, so the holes they make are
  // looked for where they will be.
  const movingCells = new Set(compartments.filter(({ compartment }) => !compartment.parentId)
    .map(({ cell }) => cell.id));
  const movingKeys = new Set();
  for (const [cellId, ids] of carried) for (const id of ids) movingKeys.add(compartmentKey(cellId, id));
  return { compartments, carried, carriedReactions, riders, rest, held, movingCells, movingKeys };
}

const FIT_EPS = 1e-6;

/* `r` lies inside `parent` with `pad` to spare on every side. */
function within(r, parent, pad) {
  return r.x >= parent.x + pad - FIT_EPS
    && r.y >= parent.y + pad - FIT_EPS
    && r.x + r.w <= parent.x + parent.w - pad + FIT_EPS
    && r.y + r.h <= parent.y + parent.h - pad + FIT_EPS;
}

/* The two rectangles share some area (touching edges do not count). */
function overlaps(a, b) {
  return a.x < b.x + b.w - FIT_EPS && b.x < a.x + a.w - FIT_EPS
    && a.y < b.y + b.h - FIT_EPS && b.y < a.y + a.h - FIT_EPS;
}

/*
 * Whether every carried compartment still fits after moving by (dx, dy): inside
 * its parent with the margin editGeometryRect keeps, and off whatever shares
 * that parent and stays put. One that does not fit now is not held to it.
 */
function compartmentsFit(map, plan, dx, dy) {
  const carries = (cellId, id) => plan.carried.get(cellId)?.has(id) ?? false;
  return plan.compartments.every(({ cell, compartment }) => {
    const now = compartment.rect;
    const next = { ...now, x: now.x + dx, y: now.y + dy };
    const parent = compartment.parentId
      ? cell.compartments.find((c) => c.id === compartment.parentId)?.rect
      : map.environment?.rect;
    if (!parent) return true;
    const pad = compartment.parentId ? 10 : 18;
    if (within(now, parent, pad) && !within(next, parent, pad)) return false;
    const siblings = compartment.parentId
      ? cell.compartments
        .filter((c) => c.parentId === compartment.parentId && !carries(cell.id, c.id))
        .map((c) => c.rect)
      // A whole cell: the other cells are beside it.
      : (map.cells ?? [])
        .filter((c) => c.id !== cell.id
          && !(c.compartments ?? []).some((k) => !k.parentId && carries(c.id, k.id)))
        .map((c) => c.rect);
    return siblings.every((s) => !s || !overlaps(next, s) || overlaps(now, s));
  });
}

/*
 * Whether every held metabolite and reaction centre still stands where it may
 * after moving by (dx, dy), with the compartments that move along moved too.
 */
function membersFit(map, plan, dx, dy) {
  const moving = { dx, dy, cells: plan.movingCells, compartments: plan.movingKeys };
  return plan.held.nodes.every((n) => nodeFitsAt(map, n, n.x + dx, n.y + dy, moving))
    && plan.held.reactions.every((r) => reactionCentreFits(map, r.id, r.x + dx, r.y + dy, moving));
}

/*
 * The step the selection allows towards (dx, dy): all of it, or else as far
 * as all of it can go along x, then along y, which lets the selection slide
 * along a wall rather than stick to it.
 */
function allowedStep(map, plan, dx, dy) {
  const fits = (x, y) => compartmentsFit(map, plan, x, y) && membersFit(map, plan, x, y);
  if (fits(dx, dy)) return { dx, dy };
  const furthest = (ok) => {
    if (ok(1)) return 1;
    let lo = 0;
    let hi = 1;
    for (let i = 0; i < 30; i += 1) {
      const mid = (lo + hi) / 2;
      if (ok(mid)) lo = mid;
      else hi = mid;
    }
    return lo;
  };
  const x = dx ? dx * furthest((t) => fits(dx * t, 0)) : 0;
  const y = dy ? dy * furthest((t) => fits(x, dy * t)) : 0;
  return { dx: x, dy: y };
}

/**
 * A rectangle given as `{x1, y1, x2, y2}` or `{x, y, w, h}`, dragged in any
 * direction, as `{x1, y1, x2, y2}` with x1 <= x2 and y1 <= y2. Null when a
 * coordinate is not finite.
 */
export function normalizeRect(rect) {
  if (!rect) return null;
  const x1 = Number(rect.x1 ?? rect.x ?? 0);
  const y1 = Number(rect.y1 ?? rect.y ?? 0);
  const x2 = Number(rect.x2 ?? ((rect.x ?? 0) + (rect.w ?? 0)));
  const y2 = Number(rect.y2 ?? ((rect.y ?? 0) + (rect.h ?? 0)));
  if (![x1, y1, x2, y2].every(Number.isFinite)) return null;
  return {
    x1: Math.min(x1, x2),
    y1: Math.min(y1, y2),
    x2: Math.max(x1, x2),
    y2: Math.max(y1, y2),
  };
}

function contains(rect, x, y) {
  return x >= rect.x1 && x <= rect.x2 && y >= rect.y1 && y <= rect.y2;
}

function encloses(box, r) {
  return Boolean(r) && [r.x, r.y, r.w, r.h].every(Number.isFinite)
    && contains(box, r.x, r.y) && contains(box, r.x + r.w, r.y + r.h);
}


// Alignment and distribution

/**
 * Align selected objects. Centre modes use object centres. Edge modes use a
 * lightweight visual bounding box (metabolite radius / reaction marker size).
 *
 * Modes:
 *   horizontal  same y centre
 *   vertical    same x centre
 *   left/right/top/bottom visual edges
 */
export function alignSelection(map, selection, mode) {
  const objects = selectedObjects(map, selection);
  if (objects.length < 2) return { moved: [], count: objects.length };

  const boxes = objects.map((item) => ({
    item, box: objectBox(item.object, item.kind), centre: itemCentre(item),
  }));
  let target;

  switch (mode) {
    case 'horizontal':
      target = average(boxes.map((x) => x.centre.y));
      break;
    case 'vertical':
      target = average(boxes.map((x) => x.centre.x));
      break;
    case 'left':
      target = Math.min(...boxes.map((x) => x.box.left));
      break;
    case 'right':
      target = Math.max(...boxes.map((x) => x.box.right));
      break;
    case 'top':
      target = Math.min(...boxes.map((x) => x.box.top));
      break;
    case 'bottom':
      target = Math.max(...boxes.map((x) => x.box.bottom));
      break;
    default:
      return { moved: [], count: objects.length };
  }

  const moved = [];
  for (const { item, box, centre } of boxes) {
    let x = centre.x;
    let y = centre.y;

    if (mode === 'horizontal') y = target;
    else if (mode === 'vertical') x = target;
    else if (mode === 'left') x += target - box.left;
    else if (mode === 'right') x += target - box.right;
    else if (mode === 'top') y += target - box.top;
    else if (mode === 'bottom') y += target - box.bottom;

    if (moveItem(map, item, x, y)) moved.push({ kind: item.kind, id: item.id });
  }

  return { moved, count: objects.length };
}

/**
 * Evenly distribute selected object centres between the first and last item.
 * End points stay fixed. Requires at least three selected objects.
 */
export function distributeSelection(map, selection, axis) {
  const objects = selectedObjects(map, selection);
  if (objects.length < 3 || !['horizontal', 'vertical'].includes(axis)) {
    return { moved: [], count: objects.length };
  }

  const key = axis === 'horizontal' ? 'x' : 'y';
  const placed = objects.map((item) => ({ item, centre: itemCentre(item) }));
  const sorted = [...placed].sort((a, b) =>
    a.centre[key] - b.centre[key] || String(a.item.id).localeCompare(String(b.item.id))
  );

  const first = sorted[0].centre[key];
  const last = sorted[sorted.length - 1].centre[key];
  const step = (last - first) / (sorted.length - 1);
  const moved = [];

  for (let i = 1; i < sorted.length - 1; i++) {
    const { item, centre } = sorted[i];
    const x = axis === 'horizontal' ? first + step * i : centre.x;
    const y = axis === 'vertical' ? first + step * i : centre.y;
    if (moveItem(map, item, x, y)) moved.push({ kind: item.kind, id: item.id });
  }

  return { moved, count: objects.length, step };
}

/**
 * The selected nodes, reactions and texts that can be moved, as
 * `{ kind: 'node'|'reaction'|'text', id, object }`. Hidden nodes and items the
 * map will not move are left out. Compartments are not included.
 */
export function selectedObjects(map, selection) {
  if (!map || !selection) return [];
  const out = [];

  for (const id of selection.nodes ?? []) {
    const node = map.nodes.get(id);
    if (!node || node.hidden || !map.canMoveNode(id)) continue;
    out.push({ kind: 'node', id, object: node });
  }

  for (const id of selection.reactions ?? []) {
    const reaction = map.reactions.get(id);
    if (!reaction || !map.canMoveReaction(id)) continue;
    out.push({ kind: 'reaction', id, object: reaction });
  }

  for (const id of selection.texts ?? []) {
    const annotation = map.texts?.get(id);
    if (!annotation) continue;
    out.push({ kind: 'text', id, object: annotation });
  }

  return out;
}

/** Where an item's middle is, whatever corner it happens to be anchored by. */
function itemCentre(item) {
  if (item.kind === 'text') return textCentre(item.object);
  return { x: item.object.x, y: item.object.y };
}

/** Place an item so that its middle lands on (x, y). */
function moveItem(map, item, x, y) {
  if (item.kind === 'text') {
    // A text is positioned by the start of its baseline, so the offset from its
    // middle has to be put back before it is moved.
    const centre = textCentre(item.object);
    return Boolean(map.moveText(
      item.id,
      item.object.x + (x - centre.x),
      item.object.y + (y - centre.y),
    ));
  }
  const moved = item.kind === 'node'
    ? map.moveNode(item.id, x, y)
    : map.moveReaction(item.id, x, y);
  // Alignment/distribution reports the selected objects it successfully placed,
  // including an object that was already exactly at the target coordinate.
  return Boolean(moved);
}

function objectBox(object, kind) {
  if (kind === 'text') return textBox(object);
  if (kind === 'node') {
    const r = Number(object.r ?? 13);
    return {
      left: object.x - r,
      right: object.x + r,
      top: object.y - r,
      bottom: object.y + r,
    };
  }

  // Internal reaction markers are 18×6. Transporters are 26×22 or taller for
  // multi-lane transport. A conservative axis-aligned visual footprint keeps
  // alignment predictable regardless of marker rotation.
  const halfW = object.onMembrane ? 13 : 9;
  const halfH = object.onMembrane
    ? Math.max(11, 7 + (object.transportLanes?.length ?? 0) * 5)
    : 3;
  return {
    left: object.x - halfW,
    right: object.x + halfW,
    top: object.y - halfH,
    bottom: object.y + halfH,
  };
}

const average = (values) => values.reduce((sum, x) => sum + x, 0) / values.length;
