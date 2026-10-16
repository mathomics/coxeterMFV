/**
 * Map layout for a multi-cell, multi-compartment community.
 *
 * The layout is built strictly outside-in, so every boundary in the drawing
 * corresponds to a real boundary in the model:
 *
 *   environment (shared medium)
 *     +-- cell                      one per species
 *           +-- outermost compartment (usually `e`, the cell surface layer)
 *                 +-- ... nested organelles, from the model's compartment tree
 *
 * A reaction is placed *inside* a compartment when all of its metabolites live
 * there, and *on the boundary* between two compartments when it crosses one.
 * That is what turns a membrane from decoration into the thing transporters
 * actually sit on.
 */

import { namespaced, sharedMetabolite, communityExchange, SHARED_COMPARTMENT, LINK_COMPARTMENT } from '../core/community.js';
import {
  compartmentName, cellCompartmentName, membraneStyle, compartmentLayoutProfile,
  cellCompartmentVisualId,
} from '../core/compartments.js';
import {
  rect, inset, center, right, bottom, gridCells, perimeterPoint,
  closestPerimeterPoint, clampToRect, compartmentCorners,
} from './geometry.js';
import { relax, makeRandom } from './force.js';

const clamp = (v, lo, hi) => Math.min(Math.max(v, lo), hi);

/** Node and marker sizes in map pixels. `reactionAxis` is half a reaction axis's length. */
export const SIZES = {
  // Every metabolite circle uses exactly one of these two radii.
  primaryRadius: 13,
  mediumRadius: 13,
  currencyRadius: 7,
  markerRadius: 5,
  reactionAxis: 17,
};

/**
 * Clearances the automatic placement keeps around a node. They are exported
 * because manual dragging has to obey exactly the same rules: a node the user
 * moves must end up somewhere the relaxation would have been willing to put it,
 * otherwise the next automatic relayout would visibly jump it.
 */
export const CLEARANCE = {
  bounds: 6,        // beyond the node radius, inside its own compartment
  hole: 10,         // beyond the node radius, away from a nested compartment
  environment: 12,  // how far inside the shared environment a medium node stays
  cell: 10,         // how far medium nodes stay clear of a cell rectangle
};

const MIN_SHELL = 44;
const NODE_AREA = 1500;
const CELL_GAP = 170;  // leaves a corridor of shared medium between cells
const ENV_PAD = 26;

function normaliseLayoutTuning(opts = {}) {
  return {
    cellGap: clamp(Number(opts.cellGap) || CELL_GAP, 80, 420),
    minShell: clamp(Number(opts.minShell) || MIN_SHELL, 28, 110),
    nodeSpacing: clamp(Number(opts.nodeSpacing) || 1, 0.65, 1.9),
    cellAreaMultiplier: clamp(Number(opts.cellAreaMultiplier) || 1, 1, 8),
  };
}

/**
 * Lay out a whole community map from scratch: the environment, one cell per
 * active species with its nested compartments, the shared-medium nodes, and
 * every metabolite, reaction marker and segment. Deterministic for a given
 * `opts.seed`. The tuning options (cellGap, minShell, nodeSpacing,
 * cellAreaMultiplier) come in `opts` as layoutEngineOptions() in
 * layout-settings.js produces them.
 *
 * @param {import('../core/community.js').CommunityModel} community
 * @param {object} [opts]
 * @param {number} [opts.width]   canvas size in map pixels, by default sized
 * @param {number} [opts.height]  to the model (see autoCanvas) at `opts.aspect`
 * @param {number} [opts.aspect]  width / height of the automatic canvas, default 1.34
 * @param {number} [opts.seed]
 * @param {'all'|'none'} [opts.contents]  'none' packs the cells, the nested
 *   compartments and the environment but places nothing inside them: no
 *   metabolite nodes, no reaction markers, no segments. The shells come out
 *   identical to a full layout, because their sizes are derived from the model
 *   rather than from what happens to be drawn.
 * @returns {{width, height, environment, cells, medium, nodes: Map, reactions: Map, segments: object[]}}
 */
export function layoutCommunity(community, opts = {}) {
  const seed = opts.seed ?? 20240501;
  const contents = opts.contents === 'none' ? 'none' : 'all';
  const tuning = normaliseLayoutTuning(opts);
  const auto = autoCanvas(community, opts.aspect ?? 1.34, tuning);
  const width = opts.width ?? auto.width;
  const height = opts.height ?? auto.height;

  const envRect = rect(ENV_PAD, ENV_PAD, width - 2 * ENV_PAD, height - 2 * ENV_PAD);
  const species = community.activeSpecies;

  const nodes = new Map();
  const reactions = new Map();
  const cells = [];

  // Cells get area in proportion to how much they have to show.
  const cellWeights = species.map((sp) =>
    cellTargetArea(sp.model, tuning.nodeSpacing, tuning.cellAreaMultiplier));
  const margin = Math.min(170, Math.max(82, width * 0.055));
  const cellRects = weightedGrid(
    inset(envRect, margin * 0.62, margin, margin * 0.62, margin),
    cellWeights,
    tuning.cellGap,
  );

  // The medium is laid out first because it only needs the cell rectangles, and
  // each cell then places its transfers facing the medium node they trade with.
  // Doing it the other way round leaves a cell's surface metabolites stranded on
  // the far side, with their transfer line drawn straight through the cytosol.
  const medium = contents === 'none'
    ? { nodes: [], exchanges: [] }
    : layoutMedium({
      community, envRect, cellRects, nodes, reactions, seed: seed + 7919, tuning,
    });

  species.forEach((sp, index) => {
    const cell = layoutCell({
      community,
      species: sp,
      rect: cellRects[index],
      nodes,
      reactions,
      seed: seed + index * 977,
      contents,
      tuning,
    });
    cells.push(cell);
  });

  const segments = contents === 'none'
    ? []
    : buildSegments({ community, nodes, reactions });

  return {
    width,
    height,
    environment: {
      rect: envRect,
      id: SHARED_COMPARTMENT,
      label: compartmentName(SHARED_COMPARTMENT, { s: 'Shared medium' }),
    },
    cells,
    medium,
    nodes,
    reactions,
    segments,
  };
}

// Cells

function layoutCell({
  community, species: sp, rect: cellRect, nodes, reactions, seed,
  contents = 'all', tuning = normaliseLayoutTuning(),
}) {
  const model = sp.model;
  const { roots, byId } = model.compartmentTree();
  const layoutProfile = compartmentLayoutProfile(model.compartmentIds(), model.compartmentNames);

  // How much room each compartment needs
  const own = new Map();
  const internalReactions = new Map();   // compartment id -> reaction[]
  const crossing = [];                   // transport reactions
  const exchanges = [];                  // cell <-> shared medium transfers
  const primaryMets = new Map();         // compartment id -> metabolite[]

  for (const id of byId.keys()) {
    own.set(id, 0);
    internalReactions.set(id, []);
    primaryMets.set(id, []);
  }

  for (const met of model.metabolites.values()) {
    if (met.compartment === LINK_COMPARTMENT) continue;
    if (!primaryMets.has(met.compartment)) continue;
    if (met.isPrimary) primaryMets.get(met.compartment).push(met);
  }

  for (const rxn of model.reactions.values()) {
    const comps = rxn.compartments.filter((c) => byId.has(c));
    if (rxn.kind === 'exchange') { exchanges.push(rxn); continue; }
    if (comps.length === 1) internalReactions.get(comps[0]).push(rxn);
    else if (comps.length > 1) crossing.push({ rxn, comps });
  }

  for (const id of byId.keys()) {
    own.set(id, primaryMets.get(id).length + internalReactions.get(id).length * 1.15 + 1);
  }
  const total = new Map();
  const computeTotal = (node) => {
    let t = own.get(node.id);
    for (const ch of node.children) t += computeTotal(ch);
    total.set(node.id, t);
    return t;
  };
  roots.forEach(computeTotal);

  // Nested packing
  const compartments = [];
  const rectOf = new Map();
  const rootWeights = roots.map((r) => total.get(r.id));
  // What a compartment needs before it is cramped: room for its own nodes,
  // three times over, since a map drawn by hand is never packed tight.
  const roomFor = (node) => total.get(node.id) * NODE_AREA * tuning.nodeSpacing * tuning.nodeSpacing * 3;
  const rootRects = roots.length === 1
    ? [cellRect]
    : packCompartments(cellRect, rootWeights, 16, roots.map(roomFor));
  roots.forEach((root, i) => packNested(root, rootRects[i]));

  function packNested(node, r) {
    rectOf.set(node.id, r);
    const visualId = cellCompartmentVisualId(node.id, model.compartmentNames);
    compartments.push({
      id: node.id,
      rect: r,
      depth: node.depth,
      parentId: node.parent,
      label: cellCompartmentName(node.id, model.compartmentNames),
      membrane: membraneStyle(visualId),
      visualId,
    });
    if (!node.children.length) return;

    const childTotal = node.children.reduce((s, ch) => s + total.get(ch.id), 0);
    const ownWeight = own.get(node.id);

    // Give the nested compartments a share of the area proportional to how much
    // they have to show, so a cytosol with 70 nodes is not squeezed into a thin
    // ring around a mitochondrion with 20. The shell is what is left over, with
    // a floor so a membrane always has room for the metabolites sitting on it.
    const innerFrac = clamp(childTotal / (ownWeight + childTotal), 0.12, 0.9);
    const scale = Math.sqrt(innerFrac);
    // The floor scales with the compartment so deep nesting does not run out of
    // room: four stacked membranes at a fixed 44 px would eat a small cell.
    const floor = clamp(Math.min(r.w, r.h) * 0.09, 15, tuning.minShell);
    const shellX = clamp((r.w * (1 - scale)) / 2, floor, r.w * 0.34);
    const shellY = clamp((r.h * (1 - scale)) / 2, floor, r.h * 0.34);
    let inner = inset(r, shellY, shellX, shellY, shellX);

    // Last resort: if the ring is still too thin for this compartment's own
    // nodes, take an extra band off the long axis so the parent keeps one
    // contiguous area to lay a pathway out in.
    const needed = ownWeight * NODE_AREA * tuning.nodeSpacing * tuning.nodeSpacing;
    const ringArea = r.w * r.h - inner.w * inner.h;
    if (ringArea < needed) {
      const deficit = needed - ringArea;
      if (inner.w >= inner.h) {
        const band = Math.min(deficit / Math.max(inner.h, 1), inner.w * 0.5);
        inner = { ...inner, x: inner.x + band, w: inner.w - band };
      } else {
        const band = Math.min(deficit / Math.max(inner.w, 1), inner.h * 0.5);
        inner = { ...inner, y: inner.y + band, h: inner.h - band };
      }
    }
    // Yeast9 has multiple organelles that coexist as islands in the cytosol.
    // Treating them as a tiled grid makes the cytosol look like a mosaic and
    // visually erases the fact that these are separate organelles. Spread the
    // organelles through the cytosolic space instead. Their own children (ER
    // lumen, mitochondrial interior, Golgi and vacuole) remain normally nested.
    if (layoutProfile === 'yeast9' && node.id === 'c') {
      const pad = clamp(Math.min(r.w, r.h) * 0.045, 24, 84);
      inner = inset(r, pad);
    }

    const parts = node.children.length === 1
      ? [inner]
      : layoutProfile === 'yeast9' && node.id === 'c'
        ? yeast9OrganelleRects(inner, node.children)
        : packCompartments(
          inner, node.children.map((ch) => total.get(ch.id)), 18, node.children.map(roomFor),
        );
    node.children.forEach((ch, i) => packNested(ch, parts[i]));
  }

  const childRects = new Map();
  for (const c of compartments) {
    childRects.set(c.id, compartments.filter((o) => o.parentId === c.id).map((o) => o.rect));
  }

  // An empty map stops here: the cell, its membranes and the compartments
  // nested inside them are the whole drawing. Everything below this point
  // places contents, and contents are exactly what an empty map has none of.
  if (contents === 'none') return describeCell(sp, cellRect, compartments);

  // Primary metabolites and single-compartment reactions
  const items = [];
  const itemIndex = new Map();
  const rand = makeRandom(seed);

  const pushItem = (key, node, compartmentId, radius, hint) => {
    const r = rectOf.get(compartmentId);
    const usable = inset(r, radius + 10 * tuning.nodeSpacing);
    const start = hintToPoint(hint, usable, rand);
    const item = {
      key,
      x: start.x,
      y: start.y,
      r: radius,
      bounds: r,
      holes: childRects.get(compartmentId) || [],
      anchorY: start.anchorY,
      anchorX: start.anchorX,
      node,
    };
    itemIndex.set(key, items.length);
    items.push(item);
    return item;
  };

  const hintsByCompartment = collectHints(model, byId);

  for (const [compId, list] of internalReactions) {
    for (const rxn of list) {
      const hint = normaliseHint(rxn.layout, hintsByCompartment.get(compId));
      pushItem(`rxn:${rxn.id}`, { kind: 'reaction', rxn }, compId, SIZES.markerRadius, hint);
    }
  }
  for (const [compId, list] of primaryMets) {
    for (const met of list) {
      const hint = metaboliteHint(met, model, hintsByCompartment.get(compId));
      pushItem(`met:${met.id}`, { kind: 'metabolite', met }, compId, SIZES.primaryRadius, hint);
    }
  }

  const edges = [];
  for (const [compId, list] of internalReactions) {
    for (const rxn of list) {
      const ri = itemIndex.get(`rxn:${rxn.id}`);
      for (const mid of Object.keys(rxn.metabolites)) {
        const mi = itemIndex.get(`met:${mid}`);
        if (mi === undefined) continue;
        edges.push({ a: ri, b: mi, length: 62 * tuning.nodeSpacing, strength: 1 });
      }
    }
  }

  relax({
    items, edges, iterations: 360, seed,
    repulsion: 1.25 * tuning.nodeSpacing * tuning.nodeSpacing,
    rankStrength: 0.10,
  });

  /**
   * Pin a reaction to a membrane and let the relaxation pull its metabolites
   * towards it. Returns the edges the next pass should honour.
   *
   * Low-degree partners are also *moved* next to the marker rather than only
   * being pulled: a metabolite sitting in a thin shell cannot reach the far side
   * of its compartment on a spring alone, because the straight line runs through
   * the organelle inside it and the containment force cancels the pull. Seeding
   * the position instead of fighting for it is what keeps a transfer from being
   * drawn right across the cytosol.
   */
  const pinToMembrane = (placed) => {
    const extra = [];
    for (const t of placed) {
      itemIndex.set(t.key, items.length);
      const index = items.length;
      items.push({ ...t, fixed: true, bounds: null, holes: [] });

      for (const mid of Object.keys(t.node.rxn.metabolites)) {
        const mi = itemIndex.get(`met:${mid}`);
        if (mi === undefined) continue;
        extra.push({ a: index, b: mi, length: 56 * tuning.nodeSpacing, strength: 1.4 });

        const met = model.metabolites.get(mid);
        if (!met || met.reactions.length > 3) continue;   // hubs keep their place
        const item = items[mi];
        if (item.seeded) continue;
        // Inside the membrane if the metabolite belongs to it, outside if it
        // belongs to the compartment this one is nested in.
        const inside = met.compartment === t.membrane ? -1 : 1;
        const offset = (item.r + 22 * tuning.nodeSpacing) * inside;
        item.x = t.x + t.nx * offset;
        item.y = t.y + t.ny * offset;
        item.seeded = true;
      }
    }
    return extra;
  };

  // Transfers to the shared medium, on the cell's outer boundary.
  // These go first because the medium node on the other side is already fixed,
  // so they are the one end of the chain that is not free to move. Each surface
  // metabolite is then pulled to the side of the cell that faces its partner.
  const outerId = roots[0]?.id ?? 'e';
  const outerRect = rectOf.get(outerId) ?? cellRect;
  const outerCompartment = compartments.find(compartment => compartment.id === outerId);
  const outerRadius = compartmentCorners(outerCompartment ?? { id: outerId, rect: outerRect });
  const usedOuter = [];
  const transferItems = [];
  const aimed = exchanges.map((rxn) => {
    const localId = Object.keys(rxn.metabolites)[0];
    const met = model.metabolites.get(localId);
    const target = met ? nodes.get(`met:${sharedMetabolite(met.base)}`) : null;
    const anchor = target ?? center(outerRect);
    return { rxn, t: perimeterParam(outerRect, closestPerimeterPoint(outerRect, anchor)) };
  });
  aimed.sort((a, b) => a.t - b.t || a.rxn.id.localeCompare(b.rxn.id));
  for (const { rxn, t } of aimed) {
    const slot = spreadAlongPerimeter(t, usedOuter, minSpacing(outerRect, 46 * tuning.nodeSpacing));
    usedOuter.push(slot);
    const perimeter = perimeterPoint(outerRect, slot);
    const p = closestPerimeterPoint(outerRect, perimeter, outerRadius);
    transferItems.push({
      key: `rxn:${rxn.id}`,
      x: p.x, y: p.y, nx: p.nx, ny: p.ny,
      r: SIZES.markerRadius,
      node: { kind: 'reaction', rxn },
      membrane: outerId,
      onMembrane: true,
      transfer: true,
    });
  }

  const edges2 = [...edges, ...pinToMembrane(transferItems)];
  relax({
    items, edges: edges2, iterations: 230, seed: seed + 31,
    repulsion: 1.25 * tuning.nodeSpacing * tuning.nodeSpacing, rankStrength: 0,
  });

  // Transporters on the membranes they cross.
  // Placed after the metabolites have settled against the outer boundary, so an
  // inner transporter lands on the side its partners actually ended up on.
  const membraneGroups = new Map();
  for (const { rxn, comps } of crossing) {
    const target = innermostOf(comps, byId);
    if (!membraneGroups.has(target)) membraneGroups.set(target, []);
    membraneGroups.get(target).push(rxn);
  }

  const transporterItems = [];
  for (const [compId, list] of membraneGroups) {
    const r = rectOf.get(compId);
    const compartment = compartments.find(compartment => compartment.id === compId);
    const radius = compartmentCorners(compartment);
    list.sort((a, b) => (a.layout?.lane ?? 0) - (b.layout?.lane ?? 0) || a.id.localeCompare(b.id));
    const used = [];
    list.forEach((rxn, i) => {
      // Aim at the metabolites this reaction actually touches, then slide along
      // the perimeter until it stops colliding with its neighbours.
      const partners = Object.keys(rxn.metabolites)
        .map((mid) => items[itemIndex.get(`met:${mid}`)])
        .filter(Boolean);
      let t;
      if (partners.length) {
        const cx = partners.reduce((s, p) => s + p.x, 0) / partners.length;
        const cy = partners.reduce((s, p) => s + p.y, 0) / partners.length;
        t = perimeterParam(r, closestPerimeterPoint(r, { x: cx, y: cy }));
      } else {
        t = i / Math.max(list.length, 1);
      }
      t = spreadAlongPerimeter(t, used, minSpacing(r, 54 * tuning.nodeSpacing));
      used.push(t);
      const perimeter = perimeterPoint(r, t);
      const p = closestPerimeterPoint(r, perimeter, radius);
      transporterItems.push({
        key: `rxn:${rxn.id}`,
        x: p.x, y: p.y, nx: p.nx, ny: p.ny,
        r: SIZES.markerRadius,
        node: { kind: 'reaction', rxn },
        membrane: compId,
        onMembrane: true,
      });
    });
  }

  const edges3 = [...edges2, ...pinToMembrane(transporterItems)];
  relax({
    items, edges: edges3, iterations: 180, seed: seed + 61,
    repulsion: 1.25 * tuning.nodeSpacing * tuning.nodeSpacing, rankStrength: 0,
  });

  // Commit to the shared node and reaction tables
  for (const item of items) {
    if (item.node.kind === 'metabolite') {
      const met = item.node.met;
      const merged = namespaced(sp.id, met.id);
      nodes.set(`met:${merged}`, {
        id: `met:${merged}`,
        kind: 'metabolite',
        mergedId: merged,
        localId: met.id,
        base: met.base,
        name: met.name,
        formula: met.formula,
        compartment: met.compartment,
        visualCompartment: cellCompartmentVisualId(met.compartment, model.compartmentNames),
        cellId: sp.id,
        x: item.x, y: item.y, r: SIZES.primaryRadius,
        primary: true,
      });
    } else {
      const rxn = item.node.rxn;
      const mergedId = namespaced(sp.id, rxn.id);
      // A transfer's stoichiometry, bounds and compartments only exist in the
      // merged model: in the species' own model it is still a boundary exchange.
      const mergedRxn = community.merged.reactions.get(mergedId);
      const source = item.transfer && mergedRxn ? mergedRxn : rxn;
      reactions.set(mergedId, {
        id: mergedId,
        localId: rxn.id,
        name: rxn.name,
        subsystem: item.transfer ? 'Intercambio con el medio' : rxn.subsystem,
        cellId: sp.id,
        kind: item.transfer ? 'transfer' : rxn.kind,
        compartments: source.compartments,
        membrane: item.membrane ?? null,
        membraneVisualId: item.membrane
          ? cellCompartmentVisualId(item.membrane, model.compartmentNames)
          : null,
        onMembrane: Boolean(item.onMembrane),
        normal: item.onMembrane ? { x: item.nx, y: item.ny } : null,
        x: item.x, y: item.y,
        equation: source.equation(),
        bounds: { lower: source.lowerBound, upper: source.upperBound },
      });
    }
  }

  return describeCell(sp, cellRect, compartments);
}

/** The cell record the map is built from, identical for full and empty maps. */
function describeCell(sp, cellRect, compartments) {
  return {
    id: sp.id,
    name: sp.model.cell.name,
    organism: sp.model.cell.organism,
    note: sp.model.cell.note,
    color: sp.color,
    abundance: sp.abundance,
    rect: cellRect,
    compartments,
    outerCompartment: compartments.find((c) => c.parentId === null)?.id ?? 'e',
  };
}

// Medium

function layoutMedium({
  community, envRect, cellRects, nodes, reactions, seed,
  tuning = normaliseLayoutTuning(),
}) {
  const bases = [...community.transfersByBase.keys()].sort();
  if (!bases.length) return { nodes: [], exchanges: [] };

  const items = [];
  const rand = makeRandom(seed);
  const cellCentres = cellRects.map(center);

  bases.forEach((base, i) => {
    const transfers = community.transfersByBase.get(base);
    const partners = transfers
      .map((t) => community.activeSpecies.findIndex((s) => s.id === t.speciesId))
      .filter((k) => k >= 0)
      .map((k) => cellCentres[k]);
    const cx = partners.length
      ? partners.reduce((s, p) => s + p.x, 0) / partners.length
      : center(envRect).x;
    const cy = partners.length
      ? partners.reduce((s, p) => s + p.y, 0) / partners.length
      : center(envRect).y;
    items.push({
      key: `med:${base}`,
      base,
      x: cx + (rand() - 0.5) * 120 * tuning.nodeSpacing,
      y: cy + (rand() - 0.5) * 120 * tuning.nodeSpacing,
      r: SIZES.mediumRadius,
      bounds: inset(envRect, CLEARANCE.environment * tuning.nodeSpacing),
      holes: cellRects.map((r) => inset(r, -CLEARANCE.cell * tuning.nodeSpacing)),
      degree: transfers.length,
    });
  });

  relax({
    items, edges: [], iterations: 380, seed,
    repulsion: 1.7 * tuning.nodeSpacing * tuning.nodeSpacing,
    rankStrength: 0,
  });

  for (const item of items) {
    const merged = sharedMetabolite(item.base);
    nodes.set(`met:${merged}`, {
      id: `met:${merged}`,
      kind: 'medium',
      mergedId: merged,
      localId: item.base,
      base: item.base,
      name: item.base,
      compartment: SHARED_COMPARTMENT,
      cellId: null,
      x: item.x, y: item.y, r: SIZES.mediumRadius,
      primary: true,
      degree: item.degree,
    });
  }

  // Community boundary exchanges: a short stub pointing out of the environment.
  const exchanges = [];
  for (const base of bases) {
    const exId = communityExchange(base);
    if (!community.merged.reactions.has(exId)) continue;
    const node = nodes.get(`met:${sharedMetabolite(base)}`);
    const p = closestPerimeterPoint(envRect, node);
    const dx = p.x - node.x;
    const dy = p.y - node.y;
    const len = Math.hypot(dx, dy) || 1;
    const rxn = community.merged.reactions.get(exId);
    reactions.set(exId, {
      id: exId,
      localId: exId,
      name: rxn.name,
      subsystem: 'Community exchange',
      cellId: null,
      kind: 'exchange',
      compartments: [SHARED_COMPARTMENT],
      membrane: null,
      onMembrane: false,
      normal: { x: dx / len, y: dy / len },
      x: node.x + (dx / len) * Math.min(len * 0.5, 46),
      y: node.y + (dy / len) * Math.min(len * 0.5, 46),
      equation: rxn.equation(),
      bounds: { lower: rxn.lowerBound, upper: rxn.upperBound },
      // A short stub, not a line all the way to the canvas edge: the exchange
      // only needs to read as "in or out of the system".
      boundary: {
        x: node.x + (dx / len) * Math.min(len, 92),
        y: node.y + (dy / len) * Math.min(len, 92),
      },
    });
    exchanges.push(exId);
  }

  return { nodes: bases.map((b) => `met:${sharedMetabolite(b)}`), exchanges };
}

// Segments

/**
 * Turn each reaction into Escher-style geometry: substrates converge on one end
 * of a short reaction axis, products leave from the other, and every non-primary
 * metabolite gets its own satellite node next to the marker.
 */
function buildSegments({ community, nodes, reactions }) {
  const segments = [];

  for (const [rid, rxn] of reactions) {
    const model = community.merged;
    const mrxn = model.reactions.get(rid);
    if (!mrxn) continue;

    const subs = [];
    const prods = [];
    for (const [mid, coef] of Object.entries(mrxn.metabolites)) {
      const met = model.metabolites.get(mid);
      if (!met || met.compartment === LINK_COMPARTMENT) continue;
      const node = nodes.get(`met:${mid}`);
      (coef < 0 ? subs : prods).push({ mid, coef, node, met });
    }

    // Direction of the reaction axis: from substrates towards products.
    const dir = axisDirection(rxn, subs, prods);
    const half = SIZES.reactionAxis;
    const a1 = { x: rxn.x - dir.x * half, y: rxn.y - dir.y * half };
    const a2 = { x: rxn.x + dir.x * half, y: rxn.y + dir.y * half };
    rxn.axis = { a1, a2, dir };

    segments.push({ reactionId: rid, kind: 'axis', from: a1, to: a2, arrow: false });

    let curIndex = 0;
    const placeSatellite = (entry, side) => {
      const perp = { x: -dir.y, y: dir.x };
      const slot = Math.floor(curIndex / 2) + 1;
      const sign = curIndex % 2 === 0 ? 1 : -1;
      curIndex += 1;
      const anchor = side < 0 ? a1 : a2;
      const dist = 26 + slot * 17;
      const px = anchor.x + perp.x * sign * dist + dir.x * side * 12;
      const py = anchor.y + perp.y * sign * dist + dir.y * side * 12;
      const id = `cur:${rid}:${entry.mid}`;
      nodes.set(id, {
        id,
        kind: 'currency',
        mergedId: entry.mid,
        localId: entry.met.id,
        base: entry.met.base,
        name: entry.met.name,
        formula: entry.met.formula,
        compartment: entry.met.compartment,
        cellId: rxn.cellId,
        reactionId: rid,
        x: px, y: py, r: SIZES.currencyRadius,
        primary: false,
      });
      return nodes.get(id);
    };

    for (const entry of subs) {
      const node = entry.node ?? placeSatellite(entry, -1);
      segments.push({
        reactionId: rid, kind: entry.node ? 'primary' : 'currency',
        from: { x: node.x, y: node.y }, to: a1,
        nodeId: node.id, coefficient: entry.coef, arrow: false, side: 'substrate',
      });
    }
    for (const entry of prods) {
      const node = entry.node ?? placeSatellite(entry, 1);
      segments.push({
        reactionId: rid, kind: entry.node ? 'primary' : 'currency',
        from: a2, to: { x: node.x, y: node.y },
        nodeId: node.id, coefficient: entry.coef, arrow: true, side: 'product',
      });
    }

    if (rxn.boundary) {
      segments.push({ reactionId: rid, kind: 'boundary', from: a2, to: rxn.boundary, arrow: true });
    }
  }

  return segments;
}

function axisDirection(rxn, subs, prods) {
  if (rxn.onMembrane && rxn.normal) {
    // Transporters point across the membrane they sit on.
    return { x: rxn.normal.x, y: rxn.normal.y };
  }
  const from = centroidOf(subs) ?? { x: rxn.x, y: rxn.y - 1 };
  const to = centroidOf(prods) ?? { x: rxn.x, y: rxn.y + 1 };
  let dx = to.x - from.x;
  let dy = to.y - from.y;
  const len = Math.hypot(dx, dy);
  if (len < 1e-6) return { x: 0, y: 1 };
  return { x: dx / len, y: dy / len };
}

function centroidOf(list) {
  const withNodes = list.filter((e) => e.node);
  if (!withNodes.length) return null;
  return {
    x: withNodes.reduce((s, e) => s + e.node.x, 0) / withNodes.length,
    y: withNodes.reduce((s, e) => s + e.node.y, 0) / withNodes.length,
  };
}

// Helpers

/** Room a single cell needs, from how much of its network has to be drawn. */
function cellTargetArea(model, nodeSpacing = 1, cellAreaMultiplier = 1) {
  const spacingArea = nodeSpacing * nodeSpacing;
  const baseArea = 230000 + 5600 * spacingArea * model.reactions.size;

  // The outer canvas has fixed margins and inter-cell corridors. Multiplying
  // the raw target by 6 would therefore yield substantially more than 6× of
  // realised cell rectangle area (about 7.7× in a four-cell community).
  // Compensate for that geometry so the user-facing multiplier describes the
  // area they actually see, not an internal pre-margin number.
  const realisedScale = 1 + (cellAreaMultiplier - 1) * 0.74;
  return baseArea * realisedScale;
}

/**
 * Pick a canvas big enough that every cell gets its target area and the shared
 * medium still has room between the cells. The SVG viewBox scales this down to
 * whatever the screen is, so a large canvas costs nothing but gives the layout
 * space to breathe.
 */
function autoCanvas(community, aspect, tuning = normaliseLayoutTuning()) {
  const cellArea = community.activeSpecies.reduce(
    (sum, sp) => sum + cellTargetArea(
      sp.model,
      tuning.nodeSpacing,
      tuning.cellAreaMultiplier,
    ),
    0,
  );

  // Spacious layouts intentionally devote more of the viewport to cells rather
  // than merely scaling the whole canvas up. This makes nodes and labels
  // genuinely easier to read after Fit-to-screen instead of cancelling the
  // extra spacing through a lower zoom factor.
  const density = (tuning.nodeSpacing - 0.65) / (1.9 - 0.65);
  const cellCoverage = clamp(0.48 + density * 0.14, 0.48, 0.62);
  const gapAllowance = Math.max(0, community.activeSpecies.length - 1)
    * tuning.cellGap * tuning.cellGap * 0.42;
  const canvasArea = Math.max(1.6e6, cellArea / cellCoverage + gapAllowance);
  const width = Math.round(Math.sqrt(canvasArea * aspect));
  return { width, height: Math.round(width / aspect) };
}

const YEAST9_ORGANELLE_SLOTS = Object.freeze({
  // Fractions of the cytosolic rectangle. They deliberately leave broad
  // cytosolic corridors between organelles so pathways can run around them.
  erm: { x: 0.05, y: 0.05, w: 0.50, h: 0.20 }, // ER membrane + ER lumen
  gm:  { x: 0.67, y: 0.07, w: 0.25, h: 0.18 }, // Golgi membrane + Golgi
  n:   { x: 0.34, y: 0.31, w: 0.28, h: 0.29 }, // nucleus
  p:   { x: 0.08, y: 0.36, w: 0.17, h: 0.17 }, // peroxisome (Yeast9 p)
  lp:  { x: 0.76, y: 0.33, w: 0.14, h: 0.12 }, // lipid particle
  mm:  { x: 0.05, y: 0.67, w: 0.39, h: 0.20 }, // mitochondrial membrane + interior
  vm:  { x: 0.63, y: 0.50, w: 0.29, h: 0.34 }, // vacuolar membrane + vacuole
});

function yeast9OrganelleRects(r, children) {
  const fallback = weightedGrid(r, children.map(() => 1), 18);
  return children.map((child, i) => {
    const slot = YEAST9_ORGANELLE_SLOTS[child.id];
    if (!slot) return fallback[i];
    return {
      x: r.x + slot.x * r.w,
      y: r.y + slot.y * r.h,
      w: Math.max(24, slot.w * r.w),
      h: Math.max(24, slot.h * r.h),
    };
  });
}

function weightedGrid(r, weights, gap) {
  const n = weights.length;
  if (n === 0) return [];
  if (n === 1) return [r];
  const cells = gridCells(r, n, gap);
  // gridCells gives equal cells, so nudge sizes towards the weights within each row.
  const total = weights.reduce((a, b) => a + b, 0) / n;
  const TARGET_ASPECT = 1.18;
  return cells.map((c, i) => {
    const scale = Math.sqrt(Math.min(1.25, Math.max(0.55, weights[i] / total)));
    const wanted = c.w * scale * c.h * scale;
    // Keep the area the weight earned, but square the cell up: a 2.2:1 cell
    // leaves a third of the cytosol empty no matter how the nodes relax.
    let w = Math.sqrt(wanted * TARGET_ASPECT);
    let h = wanted / w;
    if (w > c.w) { w = c.w; h = wanted / w; }
    if (h > c.h) { h = c.h; w = wanted / h; }
    w = Math.min(w, c.w);
    h = Math.min(h, c.h);
    return { x: c.x + (c.w - w) / 2, y: c.y + (c.h - h) / 2, w, h };
  });
}

const MIN_SIBLING_SHARE = 0.012;
const MIN_SIBLING_AREA = 60000;
const DOMINANT_SHARE = 0.6;
const BLOCK_ASPECT = 1.6;
const MAX_SIBLING_ASPECT = 2.4;

/**
 * Share a rectangle between sibling compartments in proportion to what each has
 * to show.
 *
 * weightedGrid() starts from equal cells and only nudges them, which suits the
 * cells of a community: they are of one kind and of similar size. Sibling
 * compartments are not. A genome-scale alga has a chloroplast with 1300 things
 * to draw next to a Golgi with one. From equal cells the Golgi would come out
 * three quarters the size of the chloroplast, and a flagellum declared beside
 * the cell would take half of it.
 *
 * Areas follow the weights, with a floor so that a compartment holding a single
 * metabolite is still a box a reaction can be put into. The tiling is a
 * squarified treemap (Bruls, Huizing and van Wijk, 2000), which keeps shapes
 * close to square, with one case taken out of its hands: when one sibling
 * outweighs all the others together, a treemap gives it a full-height column
 * and leaves the rest a sliver the length of their neighbour. There the small
 * ones are gathered into a compact block along one side instead, and what the
 * block does not use is left to the parent.
 *
 * @param {number[]} [minAreas] the least area each sibling can be drawn in
 */
function packCompartments(r, weights, gap, minAreas = []) {
  const n = weights.length;
  if (n === 0) return [];
  if (n === 1) return [r];

  const area = r.w * r.h;
  const sum = weights.reduce((a, b) => a + Math.max(b, 0), 0) || n;
  // No sibling may be floored into taking the place over: the floors are capped
  // so that together they can never ask for more than the rectangle has.
  const cap = area / (n * 1.5);
  const wanted = weights.map((w, i) => Math.max(
    (area * Math.max(w, 0)) / sum,
    Math.min(cap, Math.max(area * MIN_SIBLING_SHARE, MIN_SIBLING_AREA, minAreas[i] ?? 0)),
  ));
  const scale = area / wanted.reduce((a, b) => a + b, 0);
  const items = wanted.map((a, i) => ({ i, area: a * scale }));

  const out = new Array(n);
  tileSiblings(r, items, out);

  return out.map((cell) => {
    const box = inset(cell, gap / 2);
    let { w, h } = box;
    if (w > h * MAX_SIBLING_ASPECT) w = h * MAX_SIBLING_ASPECT;
    else if (h > w * MAX_SIBLING_ASPECT) h = w * MAX_SIBLING_ASPECT;
    return { x: box.x + (box.w - w) / 2, y: box.y + (box.h - h) / 2, w, h };
  });
}

function tileSiblings(r, items, out) {
  if (items.length === 1) { out[items[0].i] = r; return; }

  const order = [...items].sort((a, b) => b.area - a.area || a.i - b.i);
  const total = order.reduce((a, item) => a + item.area, 0);
  const [largest, ...rest] = order;

  if (largest.area > total * DOMINANT_SHARE) {
    // The block of small siblings goes along the short side, so the large one
    // keeps the proportions of the rectangle it was given.
    const restArea = total - largest.area;
    const wide = r.w >= r.h;
    const along = wide ? r.h : r.w;
    let length = Math.sqrt(restArea * BLOCK_ASPECT);
    let thickness = restArea / length;
    if (length > along) { length = along; thickness = restArea / along; }
    const offset = (along - length) / 2;
    const block = wide
      ? { x: r.x + r.w - thickness, y: r.y + offset, w: thickness, h: length }
      : { x: r.x + offset, y: r.y + r.h - thickness, w: length, h: thickness };
    out[largest.i] = wide
      ? { x: r.x, y: r.y, w: r.w - thickness, h: r.h }
      : { x: r.x, y: r.y, w: r.w, h: r.h - thickness };
    tileSiblings(block, rest, out);
    return;
  }

  let free = { x: r.x, y: r.y, w: r.w, h: r.h };

  // Worst aspect ratio in a row laid along a side of the given length.
  const worst = (row, side) => {
    const rowArea = row.reduce((a, item) => a + item.area, 0);
    const big = Math.max(...row.map((item) => item.area));
    const small = Math.min(...row.map((item) => item.area));
    return Math.max(
      (side * side * big) / (rowArea * rowArea),
      (rowArea * rowArea) / (side * side * small),
    );
  };

  const place = (row) => {
    const rowArea = row.reduce((a, item) => a + item.area, 0);
    if (free.w >= free.h) {
      const w = rowArea / Math.max(free.h, 1e-9);
      let y = free.y;
      for (const item of row) {
        const h = item.area / Math.max(w, 1e-9);
        out[item.i] = { x: free.x, y, w, h };
        y += h;
      }
      free = { x: free.x + w, y: free.y, w: Math.max(0, free.w - w), h: free.h };
    } else {
      const h = rowArea / Math.max(free.w, 1e-9);
      let x = free.x;
      for (const item of row) {
        const w = item.area / Math.max(h, 1e-9);
        out[item.i] = { x, y: free.y, w, h };
        x += w;
      }
      free = { x: free.x, y: free.y + h, w: free.w, h: Math.max(0, free.h - h) };
    }
  };

  let row = [];
  for (const item of order) {
    const side = Math.min(free.w, free.h);
    if (row.length && worst([...row, item], side) > worst(row, side)) {
      place(row);
      row = [];
    }
    row.push(item);
  }
  if (row.length) place(row);
}

function collectHints(model, byId) {
  const perCompartment = new Map();
  for (const id of byId.keys()) perCompartment.set(id, { ranks: [], lanes: [] });
  for (const rxn of model.reactions.values()) {
    if (!rxn.layout) continue;
    for (const c of rxn.compartments) {
      const bucket = perCompartment.get(c);
      if (!bucket) continue;
      bucket.ranks.push(rxn.layout.rank ?? 0);
      bucket.lanes.push(rxn.layout.lane ?? 0);
    }
  }
  const out = new Map();
  for (const [id, b] of perCompartment) {
    out.set(id, {
      minRank: b.ranks.length ? Math.min(...b.ranks) : 0,
      maxRank: b.ranks.length ? Math.max(...b.ranks) : 1,
      minLane: b.lanes.length ? Math.min(...b.lanes) : 0,
      maxLane: b.lanes.length ? Math.max(...b.lanes) : 1,
    });
  }
  return out;
}

function normaliseHint(layout, range) {
  if (!layout || !range) return null;
  const rs = range.maxRank - range.minRank || 1;
  const ls = range.maxLane - range.minLane || 1;
  return {
    v: ((layout.rank ?? 0) - range.minRank) / rs,
    u: ((layout.lane ?? 0) - range.minLane) / ls,
  };
}

function metaboliteHint(met, model, range) {
  const hints = [];
  for (const rid of met.reactions) {
    const rxn = model.reactions.get(rid);
    if (rxn?.layout) hints.push(normaliseHint(rxn.layout, range));
  }
  const valid = hints.filter(Boolean);
  if (!valid.length) return null;
  return {
    v: valid.reduce((s, h) => s + h.v, 0) / valid.length,
    u: valid.reduce((s, h) => s + h.u, 0) / valid.length,
  };
}

function hintToPoint(hint, usable, rand) {
  if (!hint) {
    return {
      x: usable.x + rand() * usable.w,
      y: usable.y + rand() * usable.h,
      anchorY: undefined,
      anchorX: undefined,
    };
  }
  const x = usable.x + hint.u * usable.w;
  const y = usable.y + hint.v * usable.h;
  return { x, y, anchorY: y, anchorX: x };
}

/** The compartment whose membrane a transport reaction crosses. */
function innermostOf(comps, byId) {
  let best = comps[0];
  let bestDepth = -1;
  for (const c of comps) {
    const d = byId.get(c)?.depth ?? 0;
    if (d > bestDepth) { bestDepth = d; best = c; }
  }
  return best;
}

function perimeterParam(r, p) {
  const perim = 2 * (r.w + r.h);
  const halfTop = r.w / 2;
  if (p.ny < 0 && p.y <= r.y + 0.5) {
    const d = p.x - (r.x + r.w / 2);
    return (d >= 0 ? d : perim + d) / perim;
  }
  if (p.nx > 0) return (halfTop + (p.y - r.y)) / perim;
  if (p.ny > 0) return (halfTop + r.h + (right(r) - p.x)) / perim;
  return (halfTop + r.h + r.w + (bottom(r) - p.y)) / perim;
}

function minSpacing(r, px) {
  const perim = 2 * (r.w + r.h);
  return px / Math.max(perim, 1);
}

/** Slide `t` along the perimeter until it clears every already-used slot. */
function spreadAlongPerimeter(t, used, spacing) {
  let candidate = ((t % 1) + 1) % 1;
  for (let attempt = 0; attempt < 200; attempt++) {
    const clash = used.some((u) => circularDistance(u, candidate) < spacing);
    if (!clash) return candidate;
    const step = spacing * (attempt % 2 === 0 ? 1 : -1) * (Math.floor(attempt / 2) + 1);
    candidate = ((t + step) % 1 + 1) % 1;
  }
  return candidate;
}

function circularDistance(a, b) {
  const d = Math.abs(a - b) % 1;
  return Math.min(d, 1 - d);
}

export { clampToRect };
