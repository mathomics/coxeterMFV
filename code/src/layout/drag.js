/**
 * Manual node placement.
 *
 * Dragging a metabolite is a *graphical* operation: it moves one node and the
 * reaction geometry hanging off it, and nothing else. It never touches the
 * model, the LP or the automatic layout. Keeping that promise is why this lives
 * in its own module with no DOM and no store: everything here is a pure
 * mutation of a layout object, so it can be tested outside a browser.
 *
 * What a move has to keep consistent:
 *
 *   node.x / node.y        the canonical position the renderer reads
 *   layout.segments        the endpoint of every segment attached to that node
 *
 * Reaction markers and reaction axes stay where the reaction editor puts them.
 * Every metabolite circle, including reaction-local secondary/currency nodes,
 * is independently draggable; its attached branch geometry follows the move.
 */

import { inset, signedDistance, closestPerimeterPoint, clampToRect } from './geometry.js';
import { CLEARANCE } from './layout.js';

/**
 * Node kinds a user may move. Primary, shared-medium and reaction-local
 * secondary/currency metabolites are all first-class editable map nodes.
 */
export const DRAGGABLE_NODE_KINDS = new Set(['metabolite', 'medium', 'currency']);

/**
 * Whether the user may drag `node` (one of DRAGGABLE_NODE_KINDS).
 * @param {{kind?: string}|null|undefined} node
 */
export function isDraggableNode(node) {
  return Boolean(node) && DRAGGABLE_NODE_KINDS.has(node.kind);
}

// Pointer conversion

/**
 * Convert a point in stage (client/screen) space to layout coordinates.
 *
 * The map is drawn at its full canvas resolution and the camera is applied as a
 * CSS transform on the SVG element, `translate(camera.x, camera.y) scale(s)`
 * about the element's top-left corner. One SVG user unit is therefore one
 * layout unit, and the inverse of the camera is all that is needed:
 *
 *   map = (client - stageOrigin - cameraTranslation) / cameraScale
 *
 * Kept pure and separate from the event handlers so the conversion can be
 * checked at any zoom and pan without a browser.
 *
 * @param {{x:number, y:number}} point     client coordinates
 * @param {{x:number, y:number, scale:number}} camera
 * @param {{left:number, top:number}} stageBox  the stage's bounding box
 */
export function stageToMap(point, camera, stageBox) {
  const scale = camera.scale || 1;
  return {
    x: (point.x - stageBox.left - camera.x) / scale,
    y: (point.y - stageBox.top - camera.y) / scale,
  };
}

// Constraints

/**
 * The region a node is allowed to occupy, as a bounding rectangle plus the
 * rectangles it must stay out of. This is the same description the force
 * relaxation places nodes against (`bounds` + `holes` in force.js), rebuilt from
 * the finished layout so manual and automatic placement cannot disagree.
 *
 * Returns `null` when the node has no compartment on the map, in which case the
 * caller leaves it unconstrained rather than guessing.
 *
 * `moving`, when given, describes cells and compartments that are about to be
 * moved by (`dx`, `dy`) together with the node (a dragged selection): the holes
 * they make are taken where they will be, not where they are.
 *
 * @param {{dx: number, dy: number, cells?: Set<string>, compartments?: Set<string>}|null} [moving]
 *   `cells` holds cell ids, `compartments` keys `${cellId}/${compartmentId}`
 * @returns {{bounds: object, holes: object[], pad: number, clearance: number}|null}
 */
export function nodeConstraint(layout, node, moving = null) {
  if (!layout || !node) return null;
  const pad = node.r + CLEARANCE.bounds;
  const clearance = node.r + CLEARANCE.hole;
  const shifted = (rect, moves) => (moves && rect
    ? { ...rect, x: rect.x + moving.dx, y: rect.y + moving.dy }
    : rect);

  if (node.kind === 'medium') {
    // Shared-medium metabolites live in the environment and around the cells,
    // never inside one. The cell rectangles are grown by the same margin the
    // automatic layout uses, so a medium node keeps a visible gap from a cell
    // wall instead of resting against it.
    return {
      bounds: inset(layout.environment.rect, CLEARANCE.environment),
      holes: layout.cells.map((cell) => inset(
        shifted(cell.rect, moving?.cells?.has(cell.id)), -CLEARANCE.cell,
      )),
      pad,
      clearance,
    };
  }

  const cell = layout.cells.find((c) => c.id === node.cellId);
  if (!cell) return null;
  const own = cell.compartments.find((c) => c.id === node.compartment);
  if (!own) return null;
  // Its own compartment is the cage; the compartments nested directly inside it
  // are the holes. Deeper descendants are already inside those, so listing the
  // direct children is enough.
  return {
    bounds: own.rect,
    holes: cell.compartments
      .filter((c) => c.parentId === own.id)
      .map((c) => shifted(c.rect, moving?.compartments?.has(`${cell.id}/${c.id}`))),
    pad,
    clearance,
  };
}

/**
 * Is `(x, y)` a place `node` may be: inside its compartment and out of the
 * organelles nested in it? A node with no compartment on the map fits
 * anywhere. `moving` is as for `nodeConstraint`.
 */
export function nodeFitsAt(layout, node, x, y, moving = null) {
  const c = nodeConstraint(layout, node, moving);
  return !c || satisfies({ x, y }, c);
}

const EPS = 1e-6;

/** Does `p` satisfy every part of `c`? */
function satisfies(p, c) {
  const inBounds = clampToRect(p, c.bounds, c.pad);
  if (Math.abs(inBounds.x - p.x) > EPS || Math.abs(inBounds.y - p.y) > EPS) return false;
  return c.holes.every((hole) => signedDistance(hole, p) >= c.clearance - EPS);
}

/**
 * Pull `p` into `c`: clamp it to the bounding rectangle, then push it out of
 * whichever hole it is deepest inside, worst first, and clamp again. A handful
 * of passes is enough for the shapes this layout produces; if the point still
 * violates something afterwards, `null` says so rather than returning a
 * position that only looks legal.
 */
function resolve(p, c) {
  let out = clampToRect(p, c.bounds, c.pad);
  for (let pass = 0; pass < 6; pass++) {
    let worst = null;
    let depth = 0;
    for (const hole of c.holes) {
      const d = c.clearance - signedDistance(hole, out);
      if (d > depth) { depth = d; worst = hole; }
    }
    if (!worst) return out;
    // Leave by the nearest wall, exactly as the relaxation's final pass does.
    const edge = closestPerimeterPoint(worst, out);
    const ox = edge.nx || (out.x < edge.x ? -1 : 1);
    const oy = edge.ny || (out.y < edge.y ? -1 : 1);
    out = clampToRect(
      { x: edge.x + ox * c.clearance, y: edge.y + oy * c.clearance },
      c.bounds,
      c.pad,
    );
  }
  return satisfies(out, c) ? out : null;
}

/**
 * Where a node is actually allowed to go when the user asks for `(x, y)`.
 *
 * Documented behaviour, in order of preference:
 *
 *   1. the requested point, pulled inside the compartment and out of any
 *      organelle nested in it;
 *   2. failing that, the requested movement along one axis only, which lets a
 *      node slide along an obstacle instead of sticking to it;
 *   3. failing that, the position the node already has. A drag into a place
 *      with no valid answer (the crevice where two organelles meet, say) stops
 *      at the obstacle rather than teleporting the node somewhere surprising.
 *
 * A node with no compartment on the map is returned unconstrained.
 */
export function clampNodePosition(layout, node, x, y) {
  const c = nodeConstraint(layout, node);
  if (!c) return { x, y };
  const here = { x: node.x, y: node.y };
  const attempts = [
    { x, y },
    { x, y: here.y },     // slide horizontally along the obstacle
    { x: here.x, y },     // slide vertically along it
  ];
  for (const attempt of attempts) {
    const p = resolve(attempt, c);
    if (p) return p;
  }
  return here;
}

// Segments

/**
 * Is this the endpoint of `seg` that belongs to its metabolite?
 *
 * `buildSegments` draws substrates as *metabolite -> axis* and products as
 * *axis -> metabolite*, and records which it is in `seg.side`. That flag is the
 * contract; when it is missing the reaction's own axis endpoints are used to
 * work out which end is pinned to the marker.
 */
function nodeEndpointKey(seg, layout) {
  if (seg.side === 'substrate') return 'from';
  if (seg.side === 'product') return 'to';
  const axis = layout.reactions.get(seg.reactionId)?.axis;
  if (!axis) return null;
  if (seg.to === axis.a1 || seg.to === axis.a2) return 'from';
  if (seg.from === axis.a1 || seg.from === axis.a2) return 'to';
  return null;
}

/**
 * Move the endpoint of every segment attached to `nodeId` onto the node's
 * current position.
 *
 * A metabolite can be shared by many reactions, so every segment is considered,
 * not just the ones belonging to one reaction. The reaction axes are left
 * untouched: the marker keeps its place and only the curve leading to it is
 * re-shaped. The endpoint is replaced rather than mutated, so a segment can
 * never write through an alias into the axis geometry. `delta`, when given, is
 * how far the node moved, and shifts each segment's metabolite-side handle b1
 * by as much.
 *
 * @returns {number} how many segments were updated
 */
export function updateConnectedSegments(layout, nodeId, delta = null) {
  const node = layout?.nodes?.get(nodeId);
  if (!node) return 0;
  let touched = 0;
  for (const seg of layout.segments) {
    if (seg.nodeId !== nodeId) continue;
    const key = nodeEndpointKey(seg, layout);
    if (!key) continue;
    seg[key] = { x: node.x, y: node.y };
    // Escher-style b1 is attached to the metabolite side of the curve.
    if (delta && seg.b1) {
      seg.b1 = { x: seg.b1.x + delta.x, y: seg.b1.y + delta.y };
    }
    touched += 1;
  }
  return touched;
}

// Move

/**
 * Move a node to `(x, y)`, clamped to its compartment, and bring the reaction
 * geometry attached to it along.
 *
 * This is the single entry point for manual metabolite placement. Reaction
 * markers are handled separately by the reaction editor.
 *
 * @returns {object|null} the node that moved, or null if nothing did
 */
export function moveLayoutNode(layout, nodeId, x, y) {
  const node = layout?.nodes?.get(nodeId);
  if (!isDraggableNode(node)) return null;
  if (!Number.isFinite(x) || !Number.isFinite(y)) return null;
  const old = { x: node.x, y: node.y };
  const p = clampNodePosition(layout, node, x, y);
  node.x = p.x;
  node.y = p.y;
  updateConnectedSegments(layout, nodeId, { x: p.x - old.x, y: p.y - old.y });
  return node;
}
