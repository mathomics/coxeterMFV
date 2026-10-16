/**
 * Incremental placement of a single reaction onto an existing EditableMap.
 *
 * This module never calls layoutCommunity(), never solves FBA and never moves
 * geometry that was already on the map.
 */

import { SHARED_COMPARTMENT, LINK_COMPARTMENT, splitId } from '../core/community.js';
import { stripCompartment } from '../core/model.js';
import { SIZES } from './layout.js';
import { center, closestBoundaryPoint } from './geometry.js';
import { reactionBoundaryKeys } from '../map/boundaries.js';
import { shapeBranch, satellitePlace } from './reaction-edit.js';

/** Which way a reaction runs when nothing says otherwise: down the page. */
const BASE_DIRECTION = Object.freeze({ x: 0, y: 1 });

/** How far from the marker the metabolite on the backbone's line stands. */
const PRIMARY_DISTANCE = 72;

/**
 * Where the `index`-th further primary metabolite of a side stands: off to
 * alternate hands of the backbone, starting on the hand the cofactors do not
 * start on, each pair a little wider and a little nearer the marker.
 */
function besideBackbone(index) {
  const row = Math.floor(index / 2) + 1;
  const hand = index % 2 === 0 ? -1 : 1;
  return { along: Math.max(40, PRIMARY_DISTANCE - 10 * row), across: hand * 62 * row };
}

/**
 * Escher-style secondary/currency metabolites. These are duplicated locally
 * beside each internal reaction instead of being shared as long-range hubs.
 */
export const ESCHER_SECONDARY_BASES = Object.freeze(new Set([
  'nad', 'nadh', 'nadp', 'nadph',
  'h', 'h2o', 'co2', 'coa',
  'atp', 'adp', 'amp', 'pi',
]));

/**
 * Whether a merged metabolite is an Escher-style secondary, judged by its
 * species-local base id. `metabolite` saves the lookup when the caller has it.
 */
export function isEscherSecondaryMetabolite(community, mergedId, metabolite = null) {
  const met = metabolite ?? community?.merged?.metabolites?.get?.(mergedId) ?? null;
  if (!met) return false;
  const base = localMetaboliteBase(community, mergedId, met);
  return ESCHER_SECONDARY_BASES.has(String(base ?? '').toLowerCase());
}

/**
 * Draw one model reaction (merged id) on `map`: its marker inside its
 * compartment or on the membrane it crosses, fresh reaction-local nodes for its
 * metabolites and the segments joining them.
 *
 * @param {object} [opts]
 * @param {string} [opts.anchorNodeId] a node already on the map to start from,
 *   reused instead of duplicated
 * @param {{x:number,y:number}} [opts.anchor] map point to place the marker near
 * @param {string} [opts.boundaryId] which of the crossed boundaries to sit on
 * @param {{x:number,y:number}} [opts.placementDirection] where the reaction
 *   should run from the anchor node
 * @param {string} [opts.terminalMetaboliteId] the metabolite to put on the
 *   backbone's line
 * @returns {{reaction, nodes: string[], segments: object[], anchorNode}|null}
 *   null when the reaction is already drawn, unknown, or has nowhere to go
 */
export function placeReaction(map, community, reactionId, opts = {}) {
  if (!map || !community) return null;
  if (map.reactions.has(reactionId)) return null;

  const modelReaction = community.merged.reactions.get(reactionId);
  if (!modelReaction) return null;

  // Add Reaction mode may start from one already-drawn metabolite. That one
  // explicit node is the only visual instance we are allowed to reuse; every
  // other metabolite keeps the normal "fresh instance" insertion rule.
  const anchorNode = selectedReactionAnchor(map, modelReaction, opts.anchorNodeId);

  const owner = community.reactionOwner?.get(reactionId) ?? null;
  const cellId = owner?.speciesId ?? splitId(reactionId).species ?? null;
  const places = reactionMetabolites(community, modelReaction, cellId);
  if (!places.length) return null;

  const crossed = reactionBoundaryKeys(community, reactionId);
  const requestedBoundary = opts.boundaryId && crossed.includes(opts.boundaryId)
    ? opts.boundaryId : null;
  const boundaryId = requestedBoundary ?? chooseBoundary(map, crossed, opts.anchor ?? null);

  const placement = chooseReactionPosition(
    map, cellId, places, boundaryId, opts.anchor ?? null,
  );
  if (!placement) return null;

  const placementDirection = unitDirection(opts.placementDirection);
  const anchorCoefficient = anchorNode
    ? Number(modelReaction.metabolites[anchorNode.mergedId])
    : 0;
  // placementDirection says where the new reaction should appear relative to
  // the selected metabolite. The biochemical axis must flip when that selected
  // metabolite is a product so its branch still begins at the chosen node.
  const directed = placementDirection && anchorNode && anchorCoefficient > 0
    ? { x: -placementDirection.x, y: -placementDirection.y }
    : placementDirection;
  // With nothing to say which way it should run, a reaction that is not being
  // attached to a metabolite already on the map is drawn in its base shape:
  // straight down the page, substrate above the marker and product below.
  const axisDirectionHint = directed ?? (anchorNode || placement.onMembrane ? null : { ...BASE_DIRECTION });
  const mapReaction = {
    id: reactionId,
    localId: owner?.originalId ?? splitId(reactionId).id,
    name: modelReaction.name,
    subsystem: modelReaction.subsystem ?? '',
    cellId,
    kind: owner?.role ?? modelReaction.kind ?? 'reaction',
    compartments: modelReaction.compartments ?? uniqueCompartments(places),
    membrane: placement.membrane,
    boundaryId: placement.boundaryId ?? null,
    onMembrane: placement.onMembrane,
    normal: placement.normal,
    placementDirection,
    axisDirectionHint,
    x: placement.x,
    y: placement.y,
    equation: typeof modelReaction.equation === 'function' ? modelReaction.equation() : '',
    terminalMetaboliteId: opts.terminalMetaboliteId ?? null,
    bounds: { lower: modelReaction.lowerBound, upper: modelReaction.upperBound },
  };

  map.reactions.set(reactionId, mapReaction);
  const addedNodeIds = ensureReactionNodes(
    map, community, mapReaction, modelReaction, anchorNode,
  );
  const newSegments = buildSingleReactionSegments(map, community, reactionId, {
    anchorNodeId: opts.anchorNodeId ?? null,
  });
  map.segments.push(...newSegments);
  if (anchorNode) anchorNode.sharedInstance = true;

  return {
    reaction: mapReaction,
    nodes: addedNodeIds,
    segments: newSegments,
    anchorNode: anchorNode ?? null,
  };
}

function ensureReactionNodes(map, community, mapReaction, modelReaction, anchorNode = null) {
  const added = [];
  let index = 0;
  const lateralByBase = membraneLateralOffsets(community, mapReaction, modelReaction);
  const beside = { '-1': 0, 1: 0 };   // metabolites already set beside the backbone, per side

  for (const [mid, coefficient] of Object.entries(modelReaction.metabolites)) {
    const met = community.merged.metabolites.get(mid);
    if (!met || met.compartment === LINK_COMPARTMENT) continue;

    // The metabolite explicitly selected in Add Reaction mode is the single
    // exception to duplication: the new reaction must start from that exact
    // visual node. All other metabolites remain fresh reaction-local instances.
    if (anchorNode?.mergedId === mid) continue;

    // Internal Escher-style secondary metabolites are reaction-local satellites.
    // Without an explicit selected anchor they are also duplicated per reaction.
    if (!mapReaction.onMembrane && isEscherSecondaryMetabolite(community, mid, met)) {
      continue;
    }

    const primaryId = nextReactionNodeId(map.nodes, mapReaction.id, mid);

    const owner = community.metaboliteOwner?.get(mid);
    const { compartment, cellId, shared } = metabolitePlace(community, mid, met, mapReaction.cellId);
    const base = localMetaboliteBase(community, mid, met);
    // The first primary metabolite on each side stands on the backbone's own
    // line, which is what makes a reaction read as substrate, marker, product.
    // The rest of that side stand off to either hand of it, a little nearer the
    // marker, where their curves come down the stem and out to them.
    const axial = !mapReaction.onMembrane
      && isAxialPrimary(community, modelReaction, mid, coefficient, anchorNode, mapReaction.terminalMetaboliteId);
    let offset = null;
    if (!mapReaction.onMembrane && !axial) {
      const side = coefficient < 0 ? -1 : 1;
      offset = besideBackbone(beside[side]);
      beside[side] += 1;
    }
    const position = positionNewMetabolite(
      map, mapReaction, compartment, cellId, coefficient, index,
      lateralByBase.get(base) ?? null,
      { axial, offset },
    );

    map.nodes.set(primaryId, {
      id: primaryId,
      kind: shared ? 'medium' : 'metabolite',
      mergedId: mid,
      localId: owner?.originalId ?? met.id,
      // The label. Taken from the species' own id: the merged one carries the
      // species in front, and the node would be named `eco@g6p`.
      base,
      name: met.name,
      formula: met.formula,
      compartment,
      cellId,
      reactionLocalFor: mapReaction.id,
      reactionLocal: true,
      x: position.x,
      y: position.y,
      r: shared ? SIZES.mediumRadius : SIZES.primaryRadius,
      primary: true,
      visualRole: 'primary',
    });

    if (shared && map.medium) {
      if (!Array.isArray(map.medium.nodes)) map.medium.nodes = [];
      if (!map.medium.nodes.includes(primaryId)) map.medium.nodes.push(primaryId);
    }

    added.push(primaryId);
    index += 1;
  }

  return added;
}

function positionNewMetabolite(
  map, reaction, compartment, cellId, coefficient, index, lateralOverride = null, opts = {},
) {
  const dir = reaction.normal ?? reaction.axisDirectionHint ?? reaction.placementDirection ?? { x: 0, y: 1 };
  // On a membrane, geometry follows biological location rather than the sign of
  // the stoichiometric coefficient: inside is opposite the outward normal and
  // outside is along it. Internal reactions put substrates before the marker
  // and products after it.
  const side = reaction.onMembrane
    ? (isInsideBoundary(map, reaction, cellId, compartment) ? -1 : 1)
    : (coefficient < 0 ? -1 : 1);
  const perp = { x: -dir.y, y: dir.x };
  const distance = reaction.onMembrane ? 42 : (opts.offset?.along ?? PRIMARY_DISTANCE);
  const lateral = opts.axial
    ? 0
    : (opts.offset?.across ?? lateralOverride ?? (((index % 3) - 1) * 34));

  let point = {
    x: reaction.x + dir.x * side * distance + perp.x * lateral,
    y: reaction.y + dir.y * side * distance + perp.y * lateral,
  };

  const compartmentRect = findCompartmentRect(map, cellId, compartment);

  if (isSharedPlace(cellId, compartment)) {
    return clampInsideRect(point, map.environment?.rect, SIZES.mediumRadius + 8);
  }
  if (compartmentRect) {
    point = clampInsideRect(point, compartmentRect, SIZES.primaryRadius + 8);
  }
  return point;
}


function isAxialPrimary(community, modelReaction, targetMid, targetCoefficient, anchorNode = null, preferredMid = null) {
  if (preferredMid) return targetMid === preferredMid;

  const targetSide = targetCoefficient < 0 ? -1 : 1;
  for (const [mid, coefficient] of Object.entries(modelReaction.metabolites)) {
    if ((coefficient < 0 ? -1 : 1) !== targetSide) continue;
    if (anchorNode?.mergedId === mid) return false;
    const met = community.merged.metabolites.get(mid);
    if (!met || met.compartment === LINK_COMPARTMENT) continue;
    if (isEscherSecondaryMetabolite(community, mid, met)) continue;
    return mid === targetMid;
  }
  return false;
}

function membraneLateralOffsets(community, reaction, modelReaction) {
  const out = new Map();
  if (!reaction.onMembrane) return out;

  const bases = [];
  for (const mid of Object.keys(modelReaction.metabolites)) {
    const met = community.merged.metabolites.get(mid);
    if (!met || met.compartment === LINK_COMPARTMENT) continue;
    const base = localMetaboliteBase(community, mid, met);
    if (!bases.includes(base)) bases.push(base);
  }

  if (bases.length < 2) return out;
  const spacing = 34;
  const centre = (bases.length - 1) / 2;
  bases.forEach((base, i) => out.set(base, (i - centre) * spacing));
  return out;
}

function chooseReactionPosition(map, cellId, places, boundaryId, anchor) {
  if (boundaryId) {
    const boundary = map.boundary?.(boundaryId)
      ?? map.boundaries?.find((b) => b.id === boundaryId);
    if (!boundary?.rect) return null;

    const p = closestBoundaryPoint(map, boundary, anchor ?? center(boundary.rect));
    return {
      x: p.x,
      y: p.y,
      onMembrane: true,
      membrane: boundary.inside,
      boundaryId: boundary.id,
      normal: { x: p.nx, y: p.ny },
    };
  }

  const place = chooseInternalPlace(places);
  const r = place
    ? findCompartmentRect(map, isSharedPlace(place.cellId, place.compartment) ? null : (place.cellId ?? cellId), place.compartment)
    : null;
  if (!r) return null;
  const raw = anchor ?? center(r);
  const p = clampInsideRect(raw, r, SIZES.markerRadius + 14);

  return { x: p.x, y: p.y, onMembrane: false, membrane: null, boundaryId: null, normal: null };
}

function chooseBoundary(map, crossed, anchor = null) {
  const candidates = crossed
    .map((id) => map.boundary?.(id) ?? map.boundaries?.find((b) => b.id === id))
    .filter((b) => b?.rect);

  if (!candidates.length) return null;
  if (anchor) {
    candidates.sort((a, b) => {
      const pa = closestBoundaryPoint(map, a, anchor);
      const pb = closestBoundaryPoint(map, b, anchor);
      const da = (pa.x - anchor.x) ** 2 + (pa.y - anchor.y) ** 2;
      const db = (pb.x - anchor.x) ** 2 + (pb.y - anchor.y) ** 2;
      return da - db || (b.depth ?? -1) - (a.depth ?? -1);
    });
    return candidates[0].id;
  }
  candidates.sort((a, b) => (b.depth ?? -1) - (a.depth ?? -1));
  return candidates[0].id;
}

/**
 * Build the segments of one reaction already in `map.reactions`: its axis, or
 * one lane per transported metabolite for a symport or antiport, and a branch
 * to each metabolite. Escher secondaries of an internal reaction get
 * reaction-local currency nodes, added to `map.nodes`. Sets the reaction's
 * axis, transportMode and transportLanes. `opts.anchorNodeId` names a node to
 * use for its metabolite instead of the reaction's own.
 *
 * @returns {object[]} the new segments, not yet added to map.segments
 */
export function buildSingleReactionSegments(map, community, reactionId, opts = {}) {
  const rxn = map.reactions.get(reactionId);
  const mrxn = community.merged.reactions.get(reactionId);
  if (!rxn || !mrxn) return [];

  const substrates = [];
  const products = [];
  const anchorNode = selectedReactionAnchor(map, mrxn, opts.anchorNodeId);

  for (const [mid, coef] of Object.entries(mrxn.metabolites)) {
    const met = community.merged.metabolites.get(mid);
    if (!met || met.compartment === LINK_COMPARTMENT) continue;
    const useLocalSecondary = !rxn.onMembrane
      && isEscherSecondaryMetabolite(community, mid, met);
    const node = anchorNode?.mergedId === mid
      ? anchorNode
      : (useLocalSecondary ? null : reactionVisualNode(map, reactionId, mid));
    const entry = {
      mid,
      coef,
      node,
      met,
      transportBase: localMetaboliteBase(community, mid, met),
    };
    (coef < 0 ? substrates : products).push(entry);
  }

  const dir = normalizeDirection(axisDirection(rxn, substrates, products));
  const half = SIZES.reactionAxis;
  const a1 = { x: rxn.x - dir.x * half, y: rxn.y - dir.y * half };
  const a2 = { x: rxn.x + dir.x * half, y: rxn.y + dir.y * half };
  rxn.axis = { a1, a2, dir };

  const transport = rxn.onMembrane
    ? transportStreams(map, community, rxn, [...substrates, ...products])
    : null;

  if (transport?.lanes.length >= 2) {
    rxn.transportMode = transport.mode;
    rxn.transportLanes = transport.lanes;
    const segments = buildMultiTransportSegments(rxn, reactionId, substrates, products, transport);
    initializeBranchGeometry(segments, rxn);
    return segments;
  }

  rxn.transportMode = rxn.onMembrane ? 'uniport' : null;
  rxn.transportLanes = null;

  const segments = [
    { reactionId, kind: 'axis', from: a1, to: a2, arrow: false },
  ];

  const substrateAnchors = backboneAnchors(rxn, substrates.length, -1);
  const productAnchors = backboneAnchors(rxn, products.length, 1);

  // Cofactors are counted on each side of the reaction separately, so the first
  // on either side takes the same place and a pair reads as one arc past the
  // marker (ATP in, ADP out) rather than as two unrelated spokes.
  const placed = { '-1': 0, 1: 0 };
  const satellite = (entry, side) => {
    const at = satellitePlace(rxn, side, placed[side]);
    placed[side] += 1;
    const id = `cur:${reactionId}:${entry.mid}`;

    const node = {
      id,
      kind: 'currency',
      mergedId: entry.mid,
      localId: entry.met.id,
      base: entry.transportBase ?? entry.met.base,
      name: entry.met.name,
      formula: entry.met.formula,
      compartment: entry.met.compartment,
      cellId: rxn.cellId,
      reactionId,
      x: at.x,
      y: at.y,
      r: SIZES.currencyRadius,
      primary: false,
      visualRole: 'secondary',
      escherSecondary: true,
    };

    map.nodes.set(id, node);
    return node;
  };

  substrates.forEach((entry, i) => {
    const node = entry.node ?? satellite(entry, -1);
    segments.push({
      reactionId,
      kind: entry.node?.kind === 'currency' ? 'currency' : (entry.node ? 'primary' : 'currency'),
      from: { x: node.x, y: node.y },
      to: substrateAnchors[i] ?? a1,
      nodeId: node.id,
      coefficient: entry.coef,
      arrow: false,
      side: 'substrate',
      backboneAnchor: true,
    });
  });

  products.forEach((entry, i) => {
    const node = entry.node ?? satellite(entry, 1);
    segments.push({
      reactionId,
      kind: entry.node?.kind === 'currency' ? 'currency' : (entry.node ? 'primary' : 'currency'),
      from: productAnchors[i] ?? a2,
      to: { x: node.x, y: node.y },
      nodeId: node.id,
      coefficient: entry.coef,
      arrow: true,
      side: 'product',
      backboneAnchor: true,
    });
  });

  if (rxn.boundary) {
    segments.push({
      reactionId, kind: 'boundary', from: a2, to: rxn.boundary, arrow: true,
    });
  }

  initializeBranchGeometry(segments, rxn);
  return segments;
}

function backboneAnchors(rxn, count, side) {
  if (!count || !rxn.axis) return [];
  const { dir } = rxn.axis;
  const half = SIZES.reactionAxis;
  if (count === 1) {
    const d = half * 0.72 * side;
    return [{ x: rxn.x + dir.x * d, y: rxn.y + dir.y * d }];
  }

  return Array.from({ length: count }, (_, i) => {
    const t = i / (count - 1);
    const d = half * (0.28 + 0.58 * t) * side;
    return { x: rxn.x + dir.x * d, y: rxn.y + dir.y * d };
  });
}

/**
 * Give every branch of a newly placed reaction its base curve: along the
 * backbone at both ends, see shapeBranch().
 */
function initializeBranchGeometry(segments, rxn) {
  for (const seg of segments) {
    if (!seg.nodeId || seg.kind === 'axis' || seg.kind === 'boundary' || seg.kind === 'transport-lane') continue;
    shapeBranch(seg, rxn);
  }
}

function transportStreams(map, community, rxn, entries) {
  const groups = new Map();

  for (const entry of entries) {
    const { compartment, cellId } = metabolitePlace(community, entry.mid, entry.met, rxn.cellId);
    const inside = isInsideBoundary(map, rxn, cellId, compartment);
    const base = entry.transportBase ?? entry.met.base ?? entry.mid;

    if (!groups.has(base)) groups.set(base, []);
    groups.get(base).push({ ...entry, inside });
  }

  const streams = [];
  for (const [base, list] of groups) {
    const hasInside = list.some((e) => e.inside);
    const hasOutside = list.some((e) => !e.inside);
    const substrateInside = list.some((e) => e.coef < 0 && e.inside);
    const substrateOutside = list.some((e) => e.coef < 0 && !e.inside);
    const productInside = list.some((e) => e.coef > 0 && e.inside);
    const productOutside = list.some((e) => e.coef > 0 && !e.inside);

    if (!hasInside || !hasOutside) continue;

    let direction = 0;
    if (substrateInside && productOutside) direction = 1;   // outward at + flux
    else if (substrateOutside && productInside) direction = -1; // inward at + flux

    streams.push({ base, entries: list, direction });
  }

  if (streams.length < 2) return { mode: 'uniport', lanes: [] };

  const dirs = new Set(streams.map((s) => s.direction).filter(Boolean));
  const mode = dirs.size > 1 ? 'antiport' : 'symport';
  const normal = rxn.normal ?? rxn.axis?.dir ?? { x: 1, y: 0 };
  const perp = { x: -normal.y, y: normal.x };
  const spacing = 12;
  const centre = (streams.length - 1) / 2;
  const half = SIZES.reactionAxis;

  const lanes = streams.map((stream, i) => {
    const offset = (i - centre) * spacing;
    return {
      ...stream,
      offset,
      insidePoint: {
        x: rxn.x - normal.x * half + perp.x * offset,
        y: rxn.y - normal.y * half + perp.y * offset,
      },
      outsidePoint: {
        x: rxn.x + normal.x * half + perp.x * offset,
        y: rxn.y + normal.y * half + perp.y * offset,
      },
    };
  });

  return { mode, lanes };
}

function buildMultiTransportSegments(rxn, reactionId, substrates, products, transport) {
  const segments = [];
  const laneByBase = new Map(transport.lanes.map((lane) => [lane.base, lane]));

  for (const lane of transport.lanes) {
    segments.push({
      reactionId,
      kind: 'transport-lane',
      from: lane.insidePoint,
      to: lane.outsidePoint,
      arrow: false,
      transportBase: lane.base,
      transportDirection: lane.direction,
    });
  }

  const addEntry = (entry, side) => {
    const lane = laneByBase.get(entry.transportBase ?? entry.met.base);
    if (!lane || !entry.node) return;

    const ownerSideInside = lane.entries.find((e) => e.mid === entry.mid)?.inside ?? false;
    const endpoint = ownerSideInside ? lane.insidePoint : lane.outsidePoint;

    if (side === 'substrate') {
      segments.push({
        reactionId,
        kind: entry.node.kind === 'currency' ? 'currency' : 'primary',
        from: { x: entry.node.x, y: entry.node.y },
        to: endpoint,
        nodeId: entry.node.id,
        coefficient: entry.coef,
        arrow: false,
        side,
        transportBase: lane.base,
        transportLane: true,
      });
    } else {
      segments.push({
        reactionId,
        kind: entry.node.kind === 'currency' ? 'currency' : 'primary',
        from: endpoint,
        to: { x: entry.node.x, y: entry.node.y },
        nodeId: entry.node.id,
        coefficient: entry.coef,
        arrow: true,
        side,
        transportBase: lane.base,
        transportLane: true,
      });
    }
  };

  substrates.forEach((entry) => addEntry(entry, 'substrate'));
  products.forEach((entry) => addEntry(entry, 'product'));

  return segments;
}

function selectedReactionAnchor(map, modelReaction, anchorNodeId) {
  if (!anchorNodeId) return null;
  const node = map.nodes.get(anchorNodeId);
  if (!node?.mergedId) return null;
  if (!['metabolite', 'medium', 'currency'].includes(node.kind)) return null;
  if (!Object.prototype.hasOwnProperty.call(modelReaction.metabolites, node.mergedId)) return null;
  return node;
}

function reactionVisualNode(map, reactionId, mergedId) {
  return [...map.nodes.values()].find((node) =>
    ['metabolite', 'medium'].includes(node.kind)
    && node.reactionLocalFor === reactionId
    && node.mergedId === mergedId
  ) ?? null;
}

function nextReactionNodeId(nodes, reactionId, mergedId) {
  const root = `met:${reactionId}:${mergedId}`;
  if (!nodes.has(root)) return root;
  let index = 2;
  let id = `${root}#${index}`;
  while (nodes.has(id)) {
    index += 1;
    id = `${root}#${index}`;
  }
  return id;
}

/**
 * What a metabolite is called inside its own species: the id without the
 * species in front and without the compartment behind. This is the label of a
 * node, and what groups the two ends of a transported metabolite into one lane.
 */
export function localMetaboliteBase(community, mergedId, metabolite) {
  const owner = community?.metaboliteOwner?.get(mergedId);
  // A shared-medium metabolite is recorded under its base already; running it
  // through the compartment stripper would cut a real suffix off `pe_hs`.
  if (owner && owner.speciesId == null && owner.compartment === SHARED_COMPARTMENT) {
    return owner.originalId;
  }
  const compartment = owner?.compartment ?? metabolite?.compartment ?? null;
  const localId = owner?.originalId ?? splitId(mergedId).id;
  return stripCompartment(localId, compartment);
}

/**
 * The shared medium is the compartment `s` that belongs to no cell.
 *
 * The id alone does not say so. BiGG gives the eyespot of an alga the letter
 * `s` too (iRC1080 and every model drafted from it), and a metabolite in that
 * organelle is an ordinary cellular metabolite: it lives in the cell's own `s`
 * box, not out in the medium between the cells. Who owns it is what tells the
 * two apart.
 */
function isSharedPlace(cellId, compartmentId) {
  return (cellId === null || cellId === undefined) && compartmentId === SHARED_COMPARTMENT;
}

/** Where one merged metabolite sits: its compartment and, unless shared, its cell. */
function metabolitePlace(community, mergedId, metabolite, fallbackCellId = null) {
  const owner = community?.metaboliteOwner?.get(mergedId);
  const compartment = owner?.compartment ?? metabolite?.compartment ?? null;
  const shared = owner
    ? isSharedPlace(owner.speciesId, compartment)
    : compartment === SHARED_COMPARTMENT;
  return {
    compartment,
    shared,
    cellId: shared ? null : (owner?.speciesId ?? fallbackCellId),
  };
}

function normalizeDirection(dir) {
  return {
    x: Object.is(dir?.x, -0) ? 0 : (dir?.x ?? 0),
    y: Object.is(dir?.y, -0) ? 0 : (dir?.y ?? 0),
  };
}

function unitDirection(dir) {
  const x = Number(dir?.x);
  const y = Number(dir?.y);
  if (!Number.isFinite(x) || !Number.isFinite(y)) return null;
  const length = Math.hypot(x, y);
  if (length < 1e-9) return null;
  return normalizeDirection({ x: x / length, y: y / length });
}

function reactionMetabolites(community, reaction, fallbackCellId) {
  const out = [];
  for (const mid of Object.keys(reaction.metabolites)) {
    const met = community.merged.metabolites.get(mid);
    if (!met || met.compartment === LINK_COMPARTMENT) continue;
    const { compartment, cellId } = metabolitePlace(community, mid, met, fallbackCellId);
    out.push({ id: mid, compartment, cellId });
  }
  return out;
}

function uniqueCompartments(places) {
  return [...new Set(places.map((p) => p.compartment))];
}

function chooseInternalPlace(places) {
  const local = places.filter((p) => !isSharedPlace(p.cellId, p.compartment));
  return local[0] ?? places[0] ?? null;
}

function findCompartmentRect(map, cellId, compartmentId) {
  if (isSharedPlace(cellId, compartmentId)) return map.environment?.rect ?? null;
  const cell = map.cells.find((c) => c.id === cellId);
  if (!cell) return null;
  return cell.compartments.find((c) => c.id === compartmentId)?.rect ?? null;
}

function clampInsideRect(point, r, padding) {
  if (!r) return point;
  return {
    x: Math.min(r.x + r.w - padding, Math.max(r.x + padding, point.x)),
    y: Math.min(r.y + r.h - padding, Math.max(r.y + padding, point.y)),
  };
}

function axisDirection(rxn, substrates, products) {
  if (!rxn.onMembrane && (rxn.axisDirectionHint || rxn.placementDirection)) {
    return rxn.axisDirectionHint ?? rxn.placementDirection;
  }
  if (rxn.onMembrane && rxn.normal) {
    // A membrane has two equally valid geometric orientations. Choose the one
    // that makes substrate branches reach a1 and product branches reach a2 with
    // the least total travel. This handles uptake, secretion, symport and
    // antiport without hard-coding reaction names or assuming directionality.
    const forward = { x: rxn.normal.x, y: rxn.normal.y };
    const reverse = { x: -rxn.normal.x, y: -rxn.normal.y };
    const forwardCost = membraneRoutingCost(rxn, substrates, products, forward);
    const reverseCost = membraneRoutingCost(rxn, substrates, products, reverse);
    return reverseCost + 1e-9 < forwardCost ? reverse : forward;
  }
  const from = centroid(substrates) ?? { x: rxn.x, y: rxn.y - 1 };
  const to = centroid(products) ?? { x: rxn.x, y: rxn.y + 1 };
  let dx = to.x - from.x;
  let dy = to.y - from.y;
  const length = Math.hypot(dx, dy);
  if (length < 1e-6) return { x: 0, y: 1 };
  dx /= length;
  dy /= length;
  return { x: dx, y: dy };
}

function membraneRoutingCost(rxn, substrates, products, dir) {
  const half = SIZES.reactionAxis;
  const a1 = { x: rxn.x - dir.x * half, y: rxn.y - dir.y * half };
  const a2 = { x: rxn.x + dir.x * half, y: rxn.y + dir.y * half };
  let cost = 0;
  for (const entry of substrates) cost += squaredDistance(entry.node, a1);
  for (const entry of products) cost += squaredDistance(entry.node, a2);
  return cost;
}

function squaredDistance(node, point) {
  if (!node) return 0;
  const dx = node.x - point.x;
  const dy = node.y - point.y;
  return dx * dx + dy * dy;
}

/** Is a compartment on the enclosed side of the membrane used by this reaction? */
function isInsideBoundary(map, reaction, cellId, compartmentId) {
  if (!reaction.boundaryId || isSharedPlace(cellId, compartmentId)) return false;
  const boundary = map.boundary?.(reaction.boundaryId)
    ?? map.boundaries?.find((b) => b.id === reaction.boundaryId);
  if (!boundary || boundary.cellId !== cellId) return false;

  const cell = map.cells.find((c) => c.id === cellId);
  if (!cell) return false;
  const byId = new Map(cell.compartments.map((comp) => [comp.id, comp]));
  let current = byId.get(compartmentId);
  while (current) {
    if (current.id === boundary.inside) return true;
    current = current.parentId ? byId.get(current.parentId) : null;
  }
  return false;
}

function centroid(entries) {
  const valid = entries.filter((entry) => entry.node);
  if (!valid.length) return null;
  return {
    x: valid.reduce((sum, entry) => sum + entry.node.x, 0) / valid.length,
    y: valid.reduce((sum, entry) => sum + entry.node.y, 0) / valid.length,
  };
}
