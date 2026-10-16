/**
 * Incremental reaction placement tests.
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Model } from '../src/core/model.js';
import { CommunityModel, LINK_COMPARTMENT } from '../src/core/community.js';
import { FluxProblem } from '../src/solver/fba.js';
import { EditableMap } from '../src/map/editable-map.js';
import { reactionBoundaryKeys } from '../src/map/boundaries.js';
import { isEscherSecondaryMetabolite } from '../src/layout/place-reaction.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
async function loadModel(name) {
  return new Model(JSON.parse(await readFile(join(root, 'models', `${name}.json`), 'utf8')));
}
const MEDIUM = {
  glc__D: { lower: -10 }, o2: { lower: -20 }, nh4: { lower: -1000 },
  pi: { lower: -1000 }, h2o: { lower: -1000 }, h: { lower: -1000 },
  co2: { lower: -1000 },
};
async function buildCommunity() {
  return new CommunityModel({
    species: [{ id: 'eco', model: await loadModel('ecoli_core_mc'), abundance: 1 }],
    medium: MEDIUM,
  });
}
function findInternalReaction(com) {
  for (const [id, rxn] of com.merged.reactions) {
    const real = Object.keys(rxn.metabolites).filter((mid) => {
      const met = com.merged.metabolites.get(mid);
      return met && met.compartment !== LINK_COMPARTMENT;
    });
    if (real.length >= 2 && reactionBoundaryKeys(com, id).length === 0) return id;
  }
  return null;
}
function findTransportReaction(com) {
  let fallback = null;
  for (const id of com.merged.reactions.keys()) {
    const boundaries = reactionBoundaryKeys(com, id).filter((key) => key.startsWith('eco/'));
    if (boundaries.length === 1) return { id, boundaryId: boundaries[0] };
    if (boundaries.length && !fallback) fallback = { id, boundaryId: boundaries[0] };
  }
  return fallback;
}
function snapshot(map) {
  return {
    cells: map.cells.map((c) => ({
      id: c.id, rect: { ...c.rect },
      compartments: c.compartments.map((x) => ({ id: x.id, rect: { ...x.rect } })),
    })),
    nodes: [...map.nodes.values()].map((n) => ({ id: n.id, x: n.x, y: n.y })),
    reactions: [...map.reactions.values()].map((r) => ({ id: r.id, x: r.x, y: r.y })),
  };
}
function sameFluxes(a, b, tol = 1e-9) {
  assert.equal(a.status, b.status);
  assert.equal(a.fluxes.size, b.fluxes.size);
  for (const [id, value] of a.fluxes) {
    const other = b.fluxes.get(id);
    assert.ok(other !== undefined, `missing flux ${id}`);
    assert.ok(Math.abs(value - other) <= tol, `${id} changed from ${value} to ${other}`);
  }
}

test('an empty map can add one internal reaction', async () => {
  const com = await buildCommunity();
  const map = EditableMap.emptyFromCommunity(com);
  const id = findInternalReaction(com);
  assert.ok(id);
  const result = map.addReaction(com, id);
  assert.ok(result);
  assert.equal(map.reactions.size, 1);
  assert.ok(map.nodes.size > 0);
  assert.ok(map.segments.length > 0);
});

test('adding a reaction creates primary metabolites and local Escher-style secondary satellites', async () => {
  const com = await buildCommunity();
  const map = EditableMap.emptyFromCommunity(com);
  const id = findInternalReaction(com);
  const rxn = com.merged.reactions.get(id);
  map.addReaction(com, id);
  let checked = 0;
  for (const mid of Object.keys(rxn.metabolites)) {
    const met = com.merged.metabolites.get(mid);
    if (!met || met.compartment === LINK_COMPARTMENT) continue;
    checked += 1;
    if (isEscherSecondaryMetabolite(com, mid, met)) {
      const satellites = [...map.nodes.values()].filter((node) =>
        node.kind === 'currency' && node.mergedId === mid && node.reactionId === id
      );
      assert.equal(satellites.length, 1, `${mid} should be local to ${id}`);
      assert.equal(satellites[0].primary, false);
      assert.equal(satellites[0].visualRole, 'secondary');
      assert.ok(satellites[0].r < 13);
      assert.equal(map.hasNode(`met:${mid}`), false);
    } else {
      const local = [...map.nodes.values()].filter((node) =>
        ['metabolite', 'medium'].includes(node.kind)
        && node.mergedId === mid
        && node.reactionLocalFor === id
      );
      assert.equal(local.length, 1, `${mid} should have one fresh reaction-local node`);
      assert.equal(local[0].reactionLocal, true);
    }
  }
  assert.ok(checked >= 2);
});

test('the added reaction gets one axis and connected metabolite segments', async () => {
  const com = await buildCommunity();
  const map = EditableMap.emptyFromCommunity(com);
  const id = findInternalReaction(com);
  map.addReaction(com, id);
  assert.ok(map.reactions.get(id).axis);
  const segs = map.segmentsOf(id);
  assert.ok(segs.some((s) => s.kind === 'axis'));
  assert.ok(segs.some((s) => Boolean(s.nodeId)));
});

test('a transport reaction can be placed on the boundary it crosses', async () => {
  const com = await buildCommunity();
  const map = EditableMap.emptyFromCommunity(com);
  const t = findTransportReaction(com);
  assert.ok(t);
  const b = map.boundary(t.boundaryId);
  assert.ok(b);
  assert.ok(map.addReaction(com, t.id, { boundaryId: t.boundaryId }));
  const rxn = map.reactions.get(t.id);
  assert.equal(rxn.onMembrane, true);
  assert.equal(rxn.membrane, b.inside);
  assert.ok(rxn.normal);
});

test('a boundary reaction honours an explicit placement anchor', async () => {
  const com = await buildCommunity();
  const map = EditableMap.emptyFromCommunity(com);
  const t = findTransportReaction(com);
  const b = map.boundary(t.boundaryId);
  const anchor = { x: b.rect.x + b.rect.w * 0.25, y: b.rect.y };
  map.addReaction(com, t.id, { boundaryId: t.boundaryId, anchor });
  const rxn = map.reactions.get(t.id);
  assert.ok(Math.hypot(rxn.x - anchor.x, rxn.y - anchor.y) < 5);
});

test('adding the same reaction twice does not duplicate it', async () => {
  const com = await buildCommunity();
  const map = EditableMap.emptyFromCommunity(com);
  const id = findInternalReaction(com);
  assert.ok(map.addReaction(com, id));
  const before = map.stats();
  assert.equal(map.addReaction(com, id), null);
  assert.deepEqual(map.stats(), before);
});

test('adding a second reaction does not move existing geometry', async () => {
  const com = await buildCommunity();
  const full = EditableMap.fromCommunity(com);
  const map = full.subsetByReactions([full.reactionIds()[0]]);
  const before = snapshot(map);
  let id = null;
  for (const rid of com.merged.reactions.keys()) {
    if (!map.hasReaction(rid) && reactionBoundaryKeys(com, rid).length === 0) { id = rid; break; }
  }
  assert.ok(id);
  assert.ok(map.addReaction(com, id));
  const now = snapshot(map);
  const after = {
    cells: now.cells,
    nodes: now.nodes.filter((n) => before.nodes.some((o) => o.id === n.id)),
    reactions: now.reactions.filter((r) => before.reactions.some((o) => o.id === r.id)),
  };
  assert.deepEqual(after, before);
});

test('adding a reaction changes the map but not the biological model', async () => {
  const com = await buildCommunity();
  const map = EditableMap.emptyFromCommunity(com);
  const id = findInternalReaction(com);
  const r = com.merged.reactions.size;
  const m = com.merged.metabolites.size;
  const ids = [...com.merged.reactions.keys()];
  map.addReaction(com, id);
  assert.equal(com.merged.reactions.size, r);
  assert.equal(com.merged.metabolites.size, m);
  assert.deepEqual([...com.merged.reactions.keys()], ids);
});

test('adding a reaction to the map does not change FBA', async () => {
  const com = await buildCommunity();
  const before = new FluxProblem(com.merged).optimize();
  const map = EditableMap.emptyFromCommunity(com);
  map.addReaction(com, findInternalReaction(com));
  const after = new FluxProblem(com.merged).optimize();
  sameFluxes(before, after);
});

test('adding an unknown reaction is a safe no-op', async () => {
  const com = await buildCommunity();
  const map = EditableMap.emptyFromCommunity(com);
  const before = map.stats();
  assert.equal(map.addReaction(com, 'eco@THIS_REACTION_DOES_NOT_EXIST'), null);
  assert.deepEqual(map.stats(), before);
});

test('every segment created for an added reaction references valid geometry', async () => {
  const com = await buildCommunity();
  const map = EditableMap.emptyFromCommunity(com);
  const id = findInternalReaction(com);
  map.addReaction(com, id);
  for (const seg of map.segmentsOf(id)) {
    assert.ok(Number.isFinite(seg.from.x) && Number.isFinite(seg.from.y));
    assert.ok(Number.isFinite(seg.to.x) && Number.isFinite(seg.to.y));
    if (seg.nodeId) assert.ok(map.hasNode(seg.nodeId), seg.nodeId);
  }
});


test('transporters can be inserted on every side of the same membrane', async () => {
  const com = await buildCommunity();
  const t = findTransportReaction(com);
  assert.ok(t);

  const shell = EditableMap.emptyFromCommunity(com);
  const b = shell.boundary(t.boundaryId);
  assert.ok(b?.rect);
  const r = b.rect;
  const cases = [
    ['top',    { x: r.x + r.w / 2, y: r.y },             { x: 0, y: -1 }],
    ['right',  { x: r.x + r.w, y: r.y + r.h / 2 },       { x: 1, y: 0 }],
    ['bottom', { x: r.x + r.w / 2, y: r.y + r.h },       { x: 0, y: 1 }],
    ['left',   { x: r.x, y: r.y + r.h / 2 },             { x: -1, y: 0 }],
  ];

  for (const [side, anchor, normal] of cases) {
    const map = EditableMap.emptyFromCommunity(com);
    const result = map.addReaction(com, t.id, { boundaryId: t.boundaryId, anchor });
    assert.ok(result, side);
    const rxn = map.reactions.get(t.id);
    assert.ok(Math.hypot(rxn.x - anchor.x, rxn.y - anchor.y) < 1e-9, `${side} moved`);
    assert.deepEqual(rxn.normal, normal, `${side} normal`);
  }
});

test('removing a transporter removes only its graphical reaction and orphaned nodes', async () => {
  const com = await buildCommunity();
  const t = findTransportReaction(com);
  const map = EditableMap.emptyFromCommunity(com);
  map.addReaction(com, t.id, { boundaryId: t.boundaryId });
  assert.ok(map.hasReaction(t.id));
  assert.ok(map.segmentsOf(t.id).length > 0);

  const result = map.removeReaction(t.id);
  assert.ok(result);
  assert.equal(map.hasReaction(t.id), false);
  assert.equal(map.segmentsOf(t.id).length, 0);
  assert.equal(map.reactions.size, 0);
  assert.equal(map.segments.length, 0);
  assert.equal(map.nodes.size, 0);
  assert.ok(com.merged.reactions.has(t.id), 'the biological reaction was removed');
});

test('removing one reaction preserves metabolite nodes still used by another drawn reaction', async () => {
  const com = await buildCommunity();
  const full = EditableMap.fromCommunity(com);
  let pair = null;
  const ids = full.reactionIds();
  for (let i = 0; i < ids.length && !pair; i++) {
    const aNodes = new Set(full.segmentsOf(ids[i]).map((s) => s.nodeId).filter(Boolean));
    for (let j = i + 1; j < ids.length; j++) {
      const shared = full.segmentsOf(ids[j]).map((s) => s.nodeId).find((id) => id && aNodes.has(id));
      if (shared) { pair = [ids[i], ids[j], shared]; break; }
    }
  }
  assert.ok(pair, 'no reactions with a shared drawn metabolite');
  const [a, b, sharedNode] = pair;
  const map = full.subsetByReactions([a, b]);
  assert.ok(map.hasNode(sharedNode));
  map.removeReaction(a);
  assert.ok(map.hasReaction(b));
  assert.ok(map.hasNode(sharedNode), 'shared node was pruned while still referenced');
});

test('a removed transporter becomes available to add again on its membrane', async () => {
  const com = await buildCommunity();
  const t = findTransportReaction(com);
  const map = EditableMap.fromCommunity(com);
  assert.ok(map.hasReaction(t.id));
  map.removeReaction(t.id);
  assert.equal(map.hasReaction(t.id), false);
  assert.ok(reactionBoundaryKeys(com, t.id).includes(t.boundaryId));
  assert.ok(map.addReaction(com, t.id, { boundaryId: t.boundaryId }));
  assert.ok(map.hasReaction(t.id));
});

test('removing a reaction from the map leaves FBA unchanged', async () => {
  const com = await buildCommunity();
  const t = findTransportReaction(com);
  const before = new FluxProblem(com.merged).optimize();
  const map = EditableMap.fromCommunity(com);
  map.removeReaction(t.id);
  const after = new FluxProblem(com.merged).optimize();
  sameFluxes(before, after);
});


test('a membrane transporter chooses the lower-crossing orientation', async () => {
  const com = await buildCommunity();
  const t = findTransportReaction(com);
  assert.ok(t);
  const map = EditableMap.emptyFromCommunity(com);
  const b = map.boundary(t.boundaryId);
  const anchor = { x: b.rect.x, y: b.rect.y + b.rect.h / 2 };
  assert.ok(map.addReaction(com, t.id, { boundaryId: t.boundaryId, anchor }));

  const rxn = map.reactions.get(t.id);
  const modelRxn = com.merged.reactions.get(t.id);
  const half = 17;
  const cost = (dir) => {
    const a1 = { x: rxn.x - dir.x * half, y: rxn.y - dir.y * half };
    const a2 = { x: rxn.x + dir.x * half, y: rxn.y + dir.y * half };
    let total = 0;
    for (const [mid, coef] of Object.entries(modelRxn.metabolites)) {
      const node = map.nodes.get(`met:${mid}`);
      if (!node) continue;
      const end = coef < 0 ? a1 : a2;
      total += (node.x - end.x) ** 2 + (node.y - end.y) ** 2;
    }
    return total;
  };

  const chosen = rxn.axis.dir;
  const flipped = { x: -chosen.x, y: -chosen.y };
  assert.ok(cost(chosen) <= cost(flipped) + 1e-9,
    'transporter chose the orientation with longer/crossing connections');
});


test('an orphan metabolite can be removed safely', async () => {
  const com = await buildCommunity();
  const map = EditableMap.emptyFromCommunity(com);
  const id = findInternalReaction(com);
  map.addReaction(com, id);
  const nodeId = map.segmentsOf(id).find((s) => s.nodeId)?.nodeId;
  assert.ok(nodeId);
  assert.equal(map.canRemoveNode(nodeId), false, 'connected node should not be removable');
  map.removeReaction(id);
  // The normal reaction cleanup removes orphaned nodes. Add one harmless orphan
  // copy to exercise the explicit metabolite deletion API.
  const original = com.merged.metabolites.values().next().value;
  const orphanId = 'met:test-orphan';
  map.nodes.set(orphanId, {
    id: orphanId, kind: 'metabolite', mergedId: original.id, localId: original.id,
    base: original.base, name: original.name, compartment: original.compartment,
    cellId: 'eco', x: 10, y: 10, r: 13, primary: true,
  });
  assert.equal(map.canRemoveNode(orphanId), true);
  assert.ok(map.removeNode(orphanId));
  assert.equal(map.hasNode(orphanId), false);
});

test('a connected metabolite cannot be removed directly', async () => {
  const com = await buildCommunity();
  const map = EditableMap.emptyFromCommunity(com);
  const id = findInternalReaction(com);
  map.addReaction(com, id);
  const nodeId = map.segmentsOf(id).find((s) => s.nodeId)?.nodeId;
  assert.ok(nodeId);
  const before = map.stats();
  assert.equal(map.removeNode(nodeId), null);
  assert.deepEqual(map.stats(), before);
});

test('an internal reaction honors an explicit eight-way placement direction', async () => {
  const com = await buildCommunity();
  const map = EditableMap.emptyFromCommunity(com);
  const id = findInternalReaction(com);
  const cell = map.cells.find((item) => item.id === 'eco');
  const compartment = cell?.compartments.find((item) => item.id === 'c') ?? cell?.compartments[0];
  assert.ok(compartment?.rect);
  const anchor = {
    x: compartment.rect.x + compartment.rect.w / 2,
    y: compartment.rect.y + compartment.rect.h / 2,
  };

  assert.ok(map.addReaction(com, id, {
    anchor,
    placementDirection: { x: 1, y: 0 },
  }));

  const reaction = map.reactions.get(id);
  assert.ok(reaction?.axis?.dir);
  assert.ok(reaction.axis.dir.x > 0.99);
  assert.ok(Math.abs(reaction.axis.dir.y) < 1e-9);
  assert.deepEqual(reaction.placementDirection, { x: 1, y: 0 });
});


test('explicit eight-way placement keeps the terminal primary metabolite exactly on axis', async () => {
  const com = await buildCommunity();
  const rid = 'eco@PDH';
  assert.ok(com.merged.reactions.has(rid));
  const directions = [
    [1, 0], [1, 1], [0, 1], [-1, 1],
    [-1, 0], [-1, -1], [0, -1], [1, -1],
  ];

  for (const [x, y] of directions) {
    const map = EditableMap.emptyFromCommunity(com);
    const cell = map.cells.find((item) => item.id === 'eco');
    const compartment = cell?.compartments.find((item) => item.id === 'c') ?? cell?.compartments[0];
    const anchor = {
      x: compartment.rect.x + compartment.rect.w / 2,
      y: compartment.rect.y + compartment.rect.h / 2,
    };
    assert.ok(map.addReaction(com, rid, { anchor, placementDirection: { x, y } }));

    const reaction = map.reactions.get(rid);
    const product = map.nodesForMetabolite('eco@accoa_c')
      .find((node) => node.reactionLocalFor === rid);
    assert.ok(product, `missing acetyl-CoA for direction ${x},${y}`);

    const dx = product.x - reaction.x;
    const dy = product.y - reaction.y;
    const cross = dx * reaction.axis.dir.y - dy * reaction.axis.dir.x;
    const dot = dx * reaction.axis.dir.x + dy * reaction.axis.dir.y;
    assert.ok(Math.abs(cross) < 1e-7,
      `acetyl-CoA is off-axis for direction ${x},${y}: cross=${cross}`);
    assert.ok(dot > 0,
      `acetyl-CoA is not on the product side for direction ${x},${y}`);
  }
});


test('a preferred terminal primary metabolite of an internal reaction is placed exactly on the selected axis', async () => {
  const com = await buildCommunity();
  const rid = 'eco@FBA';
  const preferred = 'eco@g3p_c';
  const reaction = com.merged.reactions.get(rid);
  assert.ok(reaction, 'FBA fixture reaction is missing');
  assert.equal(reactionBoundaryKeys(com, rid).length, 0, 'FBA must remain an internal reaction');
  assert.ok(Number(reaction.metabolites[preferred]) > 0, 'g3p_c must remain an FBA product');

  const map = EditableMap.emptyFromCommunity(com);
  assert.ok(map.addReaction(com, rid, {
    placementDirection: { x: 1, y: 0 },
    terminalMetaboliteId: preferred,
  }));
  const rxn = map.reactions.get(rid);
  const node = map.nodesForMetabolite(preferred).find((item) => item.reactionLocalFor === rid);
  assert.ok(node);
  assert.ok(Math.abs(node.y - rxn.y) < 1e-7, 'preferred terminal is not on the horizontal axis');
  assert.ok(node.x > rxn.x, 'preferred terminal is not on the product side');
});

test('a reaction added from a selected metabolite reuses only that metabolite as its anchor', async () => {
  const com = await buildCommunity();
  const full = EditableMap.fromCommunity(com);
  let source = null;
  let targetReaction = null;
  for (const node of full.nodes.values()) {
    if (!['metabolite', 'medium'].includes(node.kind)) continue;
    for (const [rid, rxn] of com.merged.reactions) {
      if (full.hasReaction(rid) && Object.prototype.hasOwnProperty.call(rxn.metabolites, node.mergedId)) {
        source = node;
        targetReaction = rid;
        break;
      }
    }
    if (source) break;
  }
  assert.ok(source && targetReaction);

  const map = full.subsetByReactions([]);
  // Keep exactly one pre-existing visual metabolite as the Add Reaction origin.
  map.nodes.set(source.id, { ...source });
  const anchor = { x: source.x + 96, y: source.y };
  const result = map.addReaction(com, targetReaction, {
    anchor,
    anchorNodeId: source.id,
    placementDirection: { x: 1, y: 0 },
  });
  assert.ok(result);
  assert.equal(result.anchorNode?.id, source.id);
  assert.ok(
    map.segmentsOf(targetReaction).some((seg) => seg.nodeId === source.id),
    'new reaction does not start from the selected metabolite',
  );
  assert.equal(
    map.nodesForMetabolite(source.mergedId).filter((node) =>
      node.reactionLocalFor === targetReaction
    ).length,
    0,
    'selected metabolite was duplicated',
  );

  const target = com.merged.reactions.get(targetReaction);
  for (const mid of Object.keys(target.metabolites)) {
    if (mid === source.mergedId) continue;
    const met = com.merged.metabolites.get(mid);
    if (!met || met.compartment === LINK_COMPARTMENT) continue;
    if (isEscherSecondaryMetabolite(com, mid, met)) {
      assert.ok(
        [...map.nodes.values()].some((node) =>
          node.kind === 'currency' && node.mergedId === mid && node.reactionId === targetReaction
        ),
        `${mid} should still be duplicated as a reaction-local secondary metabolite`,
      );
    } else {
      assert.ok(
        map.nodesForMetabolite(mid).some((node) =>
          node.reactionLocalFor === targetReaction
        ),
        `${mid} should still get a fresh reaction-local instance`,
      );
    }
  }

  const rxn = map.reactions.get(targetReaction);
  assert.ok(Number.isFinite(rxn.x) && Number.isFinite(rxn.y));
});


test('ACt2rpp is rendered as a two-lane symporter without crossed solute lanes', async () => {
  const com = await buildCommunity();
  const rid = 'eco@ACt2rpp';
  assert.ok(com.merged.reactions.has(rid));

  const boundaries = reactionBoundaryKeys(com, rid).filter((key) => key.startsWith('eco/'));
  assert.ok(boundaries.length > 0);

  const map = EditableMap.emptyFromCommunity(com);
  const boundary = map.boundary(boundaries[0]);
  const anchor = { x: boundary.rect.x, y: boundary.rect.y + boundary.rect.h / 2 };
  assert.ok(map.addReaction(com, rid, { boundaryId: boundaries[0], anchor }));

  const rxn = map.reactions.get(rid);
  assert.equal(rxn.transportMode, 'symport');
  assert.equal(rxn.transportLanes.length, 2);

  const bases = new Set(rxn.transportLanes.map((lane) => lane.base));
  assert.ok(bases.has('ac'));
  assert.ok(bases.has('h'));

  const laneSegments = map.segmentsOf(rid).filter((seg) => seg.kind === 'transport-lane');
  assert.equal(laneSegments.length, 2);

  for (const base of ['ac', 'h']) {
    const branchSegments = map.segmentsOf(rid)
      .filter((seg) => seg.transportBase === base && seg.kind === 'primary');
    assert.equal(branchSegments.length, 2, base);
  }
});


test('GAPD duplicates NAD, NADH and H as small reaction-local secondary metabolites', async () => {
  const com = await buildCommunity();
  const rid = 'eco@GAPD';
  assert.ok(com.merged.reactions.has(rid));
  const map = EditableMap.emptyFromCommunity(com);
  assert.ok(map.addReaction(com, rid));

  for (const base of ['nad','nadh','h']) {
    const nodes = [...map.nodes.values()].filter((node) =>
      node.kind === 'currency' && node.reactionId === rid && node.base === base
    );
    assert.equal(nodes.length, 1, `expected one local ${base} satellite`);
    assert.ok(nodes[0].r < 13, `${base} should be visually secondary`);
    assert.equal(nodes[0].escherSecondary, true);
  }

});

test('Pi is treated as a small reaction-local Escher secondary metabolite', async () => {
  const com = await buildCommunity();
  const rid = 'eco@GAPD';
  assert.ok(com.merged.reactions.has(rid));
  const map = EditableMap.emptyFromCommunity(com);
  assert.ok(map.addReaction(com, rid));

  const pi = [...map.nodes.values()].filter((node) =>
    node.kind === 'currency' && node.reactionId === rid && node.base === 'pi'
  );
  assert.equal(pi.length, 1);
  assert.equal(pi[0].primary, false);
  assert.equal(pi[0].visualRole, 'secondary');
  assert.equal(pi[0].escherSecondary, true);
  assert.ok(pi[0].r < 13);
});

test('secondary metabolites are duplicated per reaction rather than shared between reactions', async () => {
  const com = await buildCommunity();
  const map = EditableMap.emptyFromCommunity(com);
  for (const rid of ['eco@GAPD','eco@ACALD']) {
    if (com.merged.reactions.has(rid)) assert.ok(map.addReaction(com, rid));
  }

  const nad = [...map.nodes.values()].filter((node) =>
    node.kind === 'currency' && node.base === 'nad'
  );
  assert.ok(nad.length >= 2);
  assert.equal(new Set(nad.map((node) => node.reactionId)).size, nad.length);
  assert.ok(nad.every((node) => node.primary === false && node.r < 13));
});
