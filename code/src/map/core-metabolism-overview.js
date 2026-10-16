/**
 * Minimal pedagogical glycolysis overview.
 *
 * The Core metabolism view intentionally shows only glycolysis, arranged as a
 * vertical pathway from top to bottom. Every cell uses the same normalised
 * coordinates so strain-to-strain comparison is immediate and uncluttered.
 */

import { EditableMap } from './editable-map.js';
import { namespaced } from '../core/community.js';
import { layoutEngineOptions } from '../layout/layout-settings.js';

/** Pathway groups the overview draws, with their local (per-species) reaction ids. */
export const CORE_OVERVIEW_GROUPS = Object.freeze([
  group('glycolysis', 'Glycolysis', [
    'PGI',
    'PFK',
    'FBA',
    'TPI',
    'GAPD',
    'PGK',
    'PGM',
    'ENO',
    'PYK',
  ]),
]);

const GLYCOLYSIS_REACTIONS = CORE_OVERVIEW_GROUPS[0].reactions;

const REACTION_FLOW = Object.freeze({
  PGI:  ['g6p_c', 'f6p_c'],
  PFK:  ['f6p_c', 'fdp_c'],
  FBA:  ['fdp_c', 'g3p_c'],
  TPI:  ['dhap_c', 'g3p_c'],
  GAPD: ['g3p_c', '13dpg_c'],
  PGK:  ['13dpg_c', '3pg_c'],
  PGM:  ['3pg_c', '2pg_c'],
  ENO:  ['2pg_c', 'pep_c'],
  PYK:  ['pep_c', 'pyr_c'],
});

/**
 * Fixed vertical topology.
 *
 * FBA branches FBP into DHAP and G3P. TPI returns DHAP to G3P, after which the
 * pathway continues vertically to pyruvate.
 */
const METABOLITE_TEMPLATE = Object.freeze({
  g6p_c:     p(0.50, 0.10, true),
  f6p_c:     p(0.50, 0.19, true),
  fdp_c:     p(0.50, 0.28, true),

  dhap_c:    p(0.36, 0.38),
  g3p_c:     p(0.50, 0.42, true),

  '13dpg_c': p(0.50, 0.52),
  '3pg_c':   p(0.50, 0.61, true),
  '2pg_c':   p(0.50, 0.70),
  pep_c:     p(0.50, 0.79, true),
  pyr_c:     p(0.50, 0.89, true),
});

const LABELS = Object.freeze([
  ['G6P',   0.535, 0.10],
  ['F6P',   0.535, 0.19],
  ['FBP',   0.535, 0.28],
  ['DHAP',  0.315, 0.38],
  ['G3P',   0.535, 0.42],
  ['1,3-BPG',0.535,0.52],
  ['3PG',   0.535, 0.61],
  ['2PG',   0.535, 0.70],
  ['PEP',   0.535, 0.79],
  ['PYR',   0.535, 0.89],
]);

/**
 * How well the active species cover the overview's glycolysis reactions.
 * The overview is supported when at least 75% of them are present, counted
 * over all active species by local reaction id.
 *
 * @returns {{supported: boolean, coverage: number, groups: number}} coverage
 *   in [0, 1], groups = number of active species
 */
export function coreOverviewSupport(community) {
  if (!community?.activeSpecies?.length) {
    return { supported: false, coverage: 0, groups: 0 };
  }

  let hits = 0;
  let total = 0;
  for (const sp of community.activeSpecies) {
    for (const id of GLYCOLYSIS_REACTIONS) {
      total += 1;
      if (sp.model.reactions.has(id)) hits += 1;
    }
  }

  const coverage = total ? hits / total : 0;
  return {
    supported: coverage >= 0.75,
    coverage,
    groups: community.activeSpecies.length,
  };
}

/**
 * Build a fresh map that draws glycolysis in the cytosol (or the largest
 * compartment) of every active cell, on the fixed template above.
 *
 * @param {import('../core/community.js').CommunityModel} community
 * @param {object} [layoutSettings] layout settings; cell area, node spacing and
 *   cell gap are raised to the minimums the template needs
 * @returns {{map: EditableMap, pathwayGroups: object[], placed: number,
 *   perCell: object[], support: object}|null} null without a community
 */
export function buildCoreMetabolismOverview(community, layoutSettings = {}) {
  if (!community) return null;

  const engine = layoutEngineOptions(layoutSettings);
  engine.cellAreaMultiplier = Math.max(6, Number(engine.cellAreaMultiplier) || 1);
  engine.nodeSpacing = Math.max(1.15, Number(engine.nodeSpacing) || 1);
  engine.cellGap = Math.max(220, Number(engine.cellGap) || 170);

  const map = EditableMap.emptyFromCommunity(community, engine);
  const pathwayGroups = [];
  const perCell = [];
  let placed = 0;

  for (const sp of community.activeSpecies) {
    const cell = map.cells.find((item) => item.id === sp.id);
    if (!cell) continue;

    const cytosol = cell.compartments.find((item) => item.id === 'c')
      ?? largestCompartment(cell.compartments);
    if (!cytosol?.rect) continue;

    const reactionIds = [];
    for (const localId of GLYCOLYSIS_REACTIONS) {
      if (!sp.model.reactions.has(localId)) continue;
      const reactionId = namespaced(sp.id, localId);
      const result = map.addReaction(community, reactionId, {
        anchor: normalizedPoint(cytosol.rect, 0.50, 0.50),
      });
      if (!result) continue;
      reactionIds.push(reactionId);
      placed += 1;
    }

    applyMetaboliteTemplate(map, sp.id, cytosol.rect);
    placeReactionsBetweenLandmarks(map, community, sp);

    const title = normalizedPoint(cytosol.rect, 0.50, 0.045);
    map.addText('GLYCOLYSIS', title.x, title.y, {
      id: `overview:${sp.id}:glycolysis`,
      fontSize: 28,
    });

    for (const [text, x, y] of LABELS) {
      const q = normalizedPoint(cytosol.rect, x, y);
      map.addText(text, q.x, q.y, {
        id: `overview:${sp.id}:met:${text}`,
        fontSize: 16,
      });
    }

    pathwayGroups.push({
      id: `${sp.id}-glycolysis`,
      speciesId: sp.id,
      groupId: 'glycolysis',
      name: 'Glycolysis',
      reactionIds,
    });
    perCell.push({
      speciesId: sp.id,
      groups: [{ id: 'glycolysis', count: reactionIds.length }],
    });
  }

  return {
    map,
    pathwayGroups,
    placed,
    perCell,
    support: coreOverviewSupport(community),
  };
}

function applyMetaboliteTemplate(map, speciesId, rect) {
  for (const [localId, spec] of Object.entries(METABOLITE_TEMPLATE)) {
    const mergedId = namespaced(speciesId, localId);
    const nodes = map.nodesForMetabolite?.(mergedId) ?? [];
    if (!nodes.length) continue;

    const q = normalizedPoint(rect, spec.x, spec.y);
    for (const node of nodes) {
      map.moveNode(node.id, q.x, q.y);
      if (spec.landmark) {
        map.setNodeRadius(node.id, Math.max(node.r ?? 13, 16));
      }
    }
  }
}

function placeReactionsBetweenLandmarks(map, community, species) {
  for (const localId of GLYCOLYSIS_REACTIONS) {
    const reactionId = namespaced(species.id, localId);
    const rxn = map.reactions.get(reactionId);
    const merged = community.merged.reactions.get(reactionId);
    if (!rxn || !merged) continue;

    const participants = semanticParticipants(species.id, merged);
    const nodes = participants.flatMap((item) =>
      map.nodesForMetabolite(namespaced(species.id, item.localId))
    );
    if (!nodes.length) continue;

    const q = centroid(nodes);
    map.moveReaction(reactionId, q.x, q.y);
    orientReaction(map, reactionId, localId, species.id, participants);
  }
}

function semanticParticipants(speciesId, reaction) {
  const out = [];
  for (const [mergedMetId, coef] of Object.entries(reaction.metabolites ?? {})) {
    const localId = mergedMetId.startsWith(`${speciesId}@`)
      ? mergedMetId.slice(speciesId.length + 1)
      : mergedMetId;
    if (!Object.prototype.hasOwnProperty.call(METABOLITE_TEMPLATE, localId)) continue;
    out.push({ localId, coef });
  }
  return out;
}

function orientReaction(map, reactionId, localReactionId, speciesId, semantic) {
  const rxn = map.reactions.get(reactionId);
  if (!rxn?.axis || semantic.length < 2) return;

  // Reversible GEM reactions are not guaranteed to be stored in the intuitive
  // glycolytic direction (PGK and PGM are commonly written backwards). The
  // pedagogical overview therefore uses a canonical top-to-bottom pathway
  // direction rather than raw stoichiometric sign.
  const canonical = REACTION_FLOW[localReactionId];
  let a;
  let b;

  if (canonical) {
    const start = map.nodesForMetabolite(namespaced(speciesId, canonical[0]))[0];
    const end = map.nodesForMetabolite(namespaced(speciesId, canonical[1]))[0];
    if (start && end) {
      a = { x: start.x, y: start.y };
      b = { x: end.x, y: end.y };
    }
  }

  if (!a || !b) {
    const substrates = semantic.filter((item) => item.coef < 0);
    const products = semantic.filter((item) => item.coef > 0);
    if (!substrates.length || !products.length) return;

    const subNodes = substrates
      .flatMap((item) => map.nodesForMetabolite(namespaced(rxn.cellId, item.localId)))
      .filter(Boolean);
    const prodNodes = products
      .flatMap((item) => map.nodesForMetabolite(namespaced(rxn.cellId, item.localId)))
      .filter(Boolean);
    if (!subNodes.length || !prodNodes.length) return;

    a = centroid(subNodes);
    b = centroid(prodNodes);
  }
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const norm = Math.hypot(dx, dy);
  if (norm < 1e-6) return;

  const dir = { x: dx / norm, y: dy / norm };
  const half = Math.max(
    10,
    Math.hypot(
      rxn.axis.a2.x - rxn.axis.a1.x,
      rxn.axis.a2.y - rxn.axis.a1.y,
    ) / 2,
  );

  rxn.axis.dir = dir;
  rxn.axis.a1 = { x: rxn.x - dir.x * half, y: rxn.y - dir.y * half };
  rxn.axis.a2 = { x: rxn.x + dir.x * half, y: rxn.y + dir.y * half };

  for (const seg of map.segments) {
    if (seg.reactionId !== reactionId) continue;

    if (seg.kind === 'axis') {
      seg.from = rxn.axis.a1;
      seg.to = rxn.axis.a2;
      continue;
    }

    if (!seg.nodeId || (seg.kind !== 'branch' && seg.side == null)) continue;
    const node = map.nodes.get(seg.nodeId);
    if (!node) continue;

    const end = seg.side === 'substrate' ? rxn.axis.a1 : rxn.axis.a2;
    if (seg.side === 'substrate') {
      seg.from = { x: node.x, y: node.y };
      seg.to = end;
    } else {
      seg.from = end;
      seg.to = { x: node.x, y: node.y };
    }

    seg.b1 = lerp({ x: node.x, y: node.y }, end, 0.34);
    seg.b2 = lerp({ x: node.x, y: node.y }, end, 0.72);
    delete seg.control;
  }
}

function group(id, label, reactions) {
  return Object.freeze({
    id,
    label,
    reactions: Object.freeze(reactions),
  });
}

function p(x, y, landmark = false) {
  return Object.freeze({ x, y, landmark });
}

function normalizedPoint(rect, x, y) {
  return {
    x: rect.x + rect.w * x,
    y: rect.y + rect.h * y,
  };
}

function centroid(points) {
  return {
    x: points.reduce((sum, q) => sum + q.x, 0) / points.length,
    y: points.reduce((sum, q) => sum + q.y, 0) / points.length,
  };
}

function lerp(a, b, t) {
  return {
    x: a.x + (b.x - a.x) * t,
    y: a.y + (b.y - a.y) * t,
  };
}

function largestCompartment(compartments = []) {
  return [...compartments].sort((a, b) =>
    (b.rect?.w ?? 0) * (b.rect?.h ?? 0)
    - (a.rect?.w ?? 0) * (a.rect?.h ?? 0)
  )[0] ?? null;
}
