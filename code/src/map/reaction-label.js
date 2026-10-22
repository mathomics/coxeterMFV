/**
 * Where a reaction's name is written.
 *
 * By default the map places it: beside the marker, or over and under the bar
 * of a straight reaction lying on its side, where its own arms would be in the
 * way. A name dragged elsewhere keeps where it was put as `rxn.labelOffset`,
 * the point it is written from relative to the marker, so it follows the
 * reaction when the reaction is moved. Like a metabolite's label, the text
 * itself stays horizontal.
 */

/**
 * A usable offset, or null: an object with a finite `x` and `y` (map units).
 * Anything else (a number, a string, NaN) is not one, so a damaged file loads
 * with the name back in its default place rather than nowhere.
 */
export function normalizeReactionLabelOffset(value) {
  if (!value || typeof value !== 'object') return null;
  const { x, y } = value;
  if (typeof x !== 'number' || typeof y !== 'number' || !Number.isFinite(x) || !Number.isFinite(y)) return null;
  return { x, y };
}

/** The metabolite nodes each reaction reaches. */
export function reactionNodes(layout) {
  const byReaction = new Map();
  for (const seg of layout.segments) {
    const node = seg.nodeId ? layout.nodes.get(seg.nodeId) : null;
    if (!node || !seg.reactionId) continue;
    if (!byReaction.has(seg.reactionId)) byReaction.set(seg.reactionId, []);
    byReaction.get(seg.reactionId).push(node);
  }
  return byReaction;
}

/**
 * Where the name of `rxn` is written: the point, the text anchor, and the
 * class that sets its baseline (`emc-label-rxn-adjacent` centres it on the
 * point, `-above` writes it above, `-below` below).
 *
 * `nodesOf(rid)` gives the metabolite nodes a reaction reaches. Only a straight
 * reaction lying on its side needs them, so a caller drawing every name passes
 * one that builds `reactionNodes` once; the default builds it on demand.
 *
 * A moved name is written from the marker plus its offset, aligned away from
 * the marker the way a metabolite's label is aligned away from its circle. The
 * two default places, read as offsets, give the anchor and the baseline they
 * have, so a name picked up does not jump.
 *
 * @returns {{x: number, y: number, anchor: 'start'|'middle'|'end', placement: string}}
 */
export function reactionLabelPlacement(layout, rxn, nodesOf = (rid) => reactionNodes(layout).get(rid)) {
  const offset = normalizeReactionLabelOffset(rxn.labelOffset);
  if (offset) {
    const length = Math.hypot(offset.x, offset.y);
    const cos = length > 1e-9 ? offset.x / length : 0;
    const sin = length > 1e-9 ? offset.y / length : 0;
    return {
      x: rxn.x + offset.x,
      y: rxn.y + offset.y,
      anchor: cos > 0.3 ? 'start' : cos < -0.3 ? 'end' : 'middle',
      placement: sin < -0.3 ? 'emc-label-rxn-above' : sin > 0.3 ? 'emc-label-rxn-below' : 'emc-label-rxn-adjacent',
    };
  }

  // Place the ID and its value beside the marker, leaving a gap even
  // for rotated transporters with several lanes.
  const angle = rxn.axis ? Math.atan2(rxn.axis.dir.y, rxn.axis.dir.x) : 0;
  const halfWidth = rxn.onMembrane ? 13 : 3;
  const halfHeight = rxn.onMembrane
    ? ((rxn.transportLanes?.length ?? 0) >= 2 ? (14 + rxn.transportLanes.length * 10) / 2 : 11)
    : 9;
  const markerRight = halfWidth * Math.abs(Math.cos(angle)) + halfHeight * Math.abs(Math.sin(angle));
  // A reaction kept straight and lying on its side has its own arm where
  // the name would go, so there the name stands over the bar, on the side
  // with fewer of its metabolites.
  if (rxn.linear && rxn.axis && !rxn.onMembrane && Math.abs(Math.sin(angle)) < 0.5) {
    const markerTop = halfWidth * Math.abs(Math.sin(angle)) + halfHeight * Math.abs(Math.cos(angle));
    const below = emptierSide(nodesOf(rxn.id), rxn) > 0;
    return {
      x: rxn.x,
      y: rxn.y + (markerTop + 5) * (below ? 1 : -1),
      anchor: 'middle',
      placement: below ? 'emc-label-rxn-below' : 'emc-label-rxn-above',
    };
  }
  return { x: rxn.x + markerRight + 8, y: rxn.y, anchor: 'start', placement: 'emc-label-rxn-adjacent' };
}

/** Above (-1) or below (+1) a reaction lying on its side: where fewer of its metabolites stand. */
function emptierSide(nodes = [], rxn) {
  let above = 0;
  let below = 0;
  for (const node of nodes) {
    if (node.y < rxn.y - 4) above += 1;
    else if (node.y > rxn.y + 4) below += 1;
  }
  return below < above ? 1 : -1;
}
