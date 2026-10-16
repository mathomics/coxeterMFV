import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Model } from '../src/core/model.js';
import { CommunityModel, LINK_COMPARTMENT } from '../src/core/community.js';
import { EditableMap } from '../src/map/editable-map.js';
import { FluxProblem } from '../src/solver/fba.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

async function buildCommunity() {
  const raw = JSON.parse(await readFile(join(root, 'models', 'ecoli_core_mc.json'), 'utf8'));
  return new CommunityModel({
    species: [{ id: 'eco', model: new Model(raw), abundance: 1 }],
    medium: {
      glc__D: { lower: -10 },
      o2: { lower: -20 },
      nh4: { lower: -1000 },
      pi: { lower: -1000 },
    },
  });
}

function firstInternalReaction(com) {
  for (const [rid, rxn] of com.merged.reactions) {
    const real = Object.keys(rxn.metabolites).filter((mid) => {
      const met = com.merged.metabolites.get(mid);
      return met && met.compartment !== LINK_COMPARTMENT;
    });
    if (real.length >= 2 && rxn.kind === 'metabolic') return rid;
  }
  return null;
}

function anotherReactionFor(com, mergedId, excluded) {
  for (const [rid, rxn] of com.merged.reactions) {
    if (rid === excluded) continue;
    if (Object.prototype.hasOwnProperty.call(rxn.metabolites, mergedId)) return rid;
  }
  return null;
}

test('duplicating a metabolite creates a second visual instance with the same biological identity', async () => {
  const com = await buildCommunity();
  const map = EditableMap.emptyFromCommunity(com);
  const rid = firstInternalReaction(com);
  map.addReaction(com, rid);

  const original = [...map.nodes.values()].find((node) =>
    ['metabolite', 'medium'].includes(node.kind)
  );
  assert.ok(original);

  const biologicalCount = com.merged.metabolites.size;
  const copy = map.duplicateMetaboliteNode(original.id);
  assert.ok(copy);
  assert.notEqual(copy.id, original.id);
  assert.equal(copy.mergedId, original.mergedId);
  assert.equal(copy.localId, original.localId);
  assert.equal(copy.manualDuplicate, true);
  assert.equal(map.nodesForMetabolite(original.mergedId).length, 2);
  assert.equal(com.merged.metabolites.size, biologicalCount);
});

test('multiple duplicates receive unique node IDs', async () => {
  const com = await buildCommunity();
  const map = EditableMap.emptyFromCommunity(com);
  const rid = firstInternalReaction(com);
  map.addReaction(com, rid);
  const original = [...map.nodes.values()].find((node) => node.kind === 'metabolite');
  assert.ok(original);

  const a = map.duplicateMetaboliteNode(original.id);
  const b = map.duplicateMetaboliteNode(original.id);
  const c = map.duplicateMetaboliteNode(a.id);

  assert.equal(new Set([original.id, a.id, b.id, c.id]).size, 4);
  assert.equal(a.mergedId, original.mergedId);
  assert.equal(b.mergedId, original.mergedId);
  assert.equal(c.mergedId, original.mergedId);
});

test('moving and styling one visual copy does not alter the original', async () => {
  const com = await buildCommunity();
  const map = EditableMap.emptyFromCommunity(com);
  map.addReaction(com, firstInternalReaction(com));
  const original = [...map.nodes.values()].find((node) => node.kind === 'metabolite');
  const before = { x: original.x, y: original.y, r: original.r, hidden: Boolean(original.hidden) };

  const copy = map.duplicateMetaboliteNode(original.id);
  map.moveNode(copy.id, copy.x + 25, copy.y + 18);
  map.setNodePrimary(copy.id, false);
  map.setNodeHidden(copy.id, true);

  assert.deepEqual(
    { x: original.x, y: original.y, r: original.r, hidden: Boolean(original.hidden) },
    before,
  );
  assert.equal(copy.r, 7);
  assert.equal(copy.visualRole, 'secondary');
  assert.equal(copy.hidden, true);
});

test('adding a reaction without a selected anchor never auto-links to an existing visual instance', async () => {
  const com = await buildCommunity();
  const map = EditableMap.emptyFromCommunity(com);

  let sourceReaction = null;
  let sourceMid = null;
  let targetReaction = null;

  for (const [rid, rxn] of com.merged.reactions) {
    for (const mid of Object.keys(rxn.metabolites)) {
      const met = com.merged.metabolites.get(mid);
      if (!met || met.compartment === LINK_COMPARTMENT) continue;
      const target = anotherReactionFor(com, mid, rid);
      if (target) {
        sourceReaction = rid;
        sourceMid = mid;
        targetReaction = target;
        break;
      }
    }
    if (sourceReaction) break;
  }

  assert.ok(sourceReaction && sourceMid && targetReaction);
  assert.ok(map.addReaction(com, sourceReaction));

  const original = map.nodesForMetabolite(sourceMid)[0];
  assert.ok(original);
  const copy = map.duplicateMetaboliteNode(original.id, {
    x: original.x + 55,
    y: original.y + 35,
  });
  assert.ok(copy);

  const result = map.addReaction(com, targetReaction, {
    anchor: { x: copy.x + 82, y: copy.y },
  });
  assert.ok(result);

  const reused = map.segmentsOf(targetReaction).find((seg) =>
    [copy.id, original.id].includes(seg.nodeId)
    && map.nodes.get(seg.nodeId)?.mergedId === sourceMid
  );
  assert.equal(reused, undefined, 'new reaction reused a pre-existing metabolite node');

  const fresh = map.nodesForMetabolite(sourceMid).find((node) =>
    node.reactionLocalFor === targetReaction
  );
  assert.ok(fresh, 'new reaction did not create its own metabolite instance');
  assert.notEqual(fresh.id, original.id);
  assert.notEqual(fresh.id, copy.id);
  assert.ok(
    map.segmentsOf(targetReaction).some((seg) => seg.nodeId === fresh.id),
    'new reaction is not connected to its fresh metabolite instance',
  );
});

test('a manual duplicate survives removal of a reaction that used it', async () => {
  const com = await buildCommunity();
  const map = EditableMap.emptyFromCommunity(com);

  const source = firstInternalReaction(com);
  map.addReaction(com, source);
  const original = [...map.nodes.values()].find((node) => node.kind === 'metabolite');
  const target = anotherReactionFor(com, original.mergedId, source);
  assert.ok(target);

  const copy = map.duplicateMetaboliteNode(original.id);
  map.addReaction(com, target, {
    anchor: { x: copy.x + 82, y: copy.y },
    anchorNodeId: copy.id,
  });

  assert.ok(
    map.segmentsOf(target).some((seg) => seg.nodeId === copy.id),
    'selected metabolite was not reused as the reaction anchor',
  );
  const fresh = map.nodesForMetabolite(original.mergedId).find((node) =>
    node.reactionLocalFor === target
  );
  assert.equal(fresh, undefined, 'selected anchor was duplicated');

  map.removeReaction(target);
  assert.ok(map.hasNode(copy.id), 'pre-existing selected anchor was pruned with the reaction');
  assert.equal(map.segmentsAt(copy.id).length, 0);
});

test('duplicate visual instances survive map JSON round trip', async () => {
  const com = await buildCommunity();
  const map = EditableMap.emptyFromCommunity(com);
  map.addReaction(com, firstInternalReaction(com));
  const original = [...map.nodes.values()].find((node) => node.kind === 'metabolite');
  const copy = map.duplicateMetaboliteNode(original.id);
  map.setNodePrimary(copy.id, false);

  const restored = EditableMap.fromJSON(JSON.parse(JSON.stringify(map.toJSON())));
  const restoredCopy = restored.nodes.get(copy.id);

  assert.ok(restoredCopy);
  assert.equal(restoredCopy.mergedId, original.mergedId);
  assert.equal(restoredCopy.manualDuplicate, true);
  assert.equal(restoredCopy.r, 7);
  assert.equal(restoredCopy.visualRole, 'secondary');
  assert.equal(restored.nodesForMetabolite(original.mergedId).length, 2);
});

test('duplicating a metabolite leaves FBA completely unchanged', async () => {
  const com = await buildCommunity();
  const problem = new FluxProblem(com.merged);
  const before = problem.optimize();

  const map = EditableMap.emptyFromCommunity(com);
  map.addReaction(com, firstInternalReaction(com));
  const original = [...map.nodes.values()].find((node) => node.kind === 'metabolite');
  map.duplicateMetaboliteNode(original.id);

  const after = problem.optimize();
  assert.equal(after.status, before.status);
  assert.equal(after.objectiveValue, before.objectiveValue);
  assert.equal(after.fluxes.size, before.fluxes.size);
  for (const [rid, value] of before.fluxes) {
    assert.ok(Math.abs(value - after.fluxes.get(rid)) < 1e-9, rid);
  }
});


test('model JSON still exports one biological metabolite despite multiple visual copies', async () => {
  const com = await buildCommunity();
  const map = EditableMap.emptyFromCommunity(com);
  map.addReaction(com, firstInternalReaction(com));

  const original = [...map.nodes.values()].find((node) => node.kind === 'metabolite');
  assert.ok(original);
  map.duplicateMetaboliteNode(original.id);
  map.duplicateMetaboliteNode(original.id);

  const owner = com.metaboliteOwner.get(original.mergedId);
  const species = com.activeSpecies.find((entry) => entry.id === owner?.speciesId);
  assert.ok(species?.model);

  const localId = owner.originalId;
  const exported = species.model.toJSON();
  assert.equal(exported.metabolites.filter((met) => met.id === localId).length, 1);
});


test('drag-over merge rewires both reactions to one metabolite instance', async () => {
  const com = await buildCommunity();
  const map = EditableMap.emptyFromCommunity(com);

  const sourceReaction = firstInternalReaction(com);
  assert.ok(map.addReaction(com, sourceReaction));
  const sourceNode = [...map.nodes.values()].find((node) =>
    ['metabolite', 'medium'].includes(node.kind)
    && anotherReactionFor(com, node.mergedId, sourceReaction)
  );
  assert.ok(sourceNode, 'no shared primary metabolite found');

  const targetReaction = anotherReactionFor(com, sourceNode.mergedId, sourceReaction);
  assert.ok(targetReaction);
  assert.ok(map.addReaction(com, targetReaction));

  const copies = map.nodesForMetabolite(sourceNode.mergedId);
  const first = copies.find((node) => node.reactionLocalFor === sourceReaction);
  const second = copies.find((node) => node.reactionLocalFor === targetReaction);
  assert.ok(first && second);
  assert.notEqual(first.id, second.id, 'reactions were auto-linked before the drag');

  assert.ok(map.moveNode(second.id, first.x, first.y));
  const merged = map.mergeOverlappingMetabolite(second.id);
  assert.ok(merged);
  assert.equal(merged.target.id, first.id);
  assert.equal(map.hasNode(second.id), false);
  assert.ok(map.hasNode(first.id));

  for (const rid of [sourceReaction, targetReaction]) {
    const branch = map.segmentsOf(rid).find((seg) =>
      seg.nodeId && map.nodes.get(seg.nodeId)?.mergedId === sourceNode.mergedId
    );
    assert.ok(branch, `${rid} lost the merged metabolite branch`);
    assert.equal(branch.nodeId, first.id, `${rid} did not rewire to the merge target`);
  }

  map.removeReaction(sourceReaction);
  assert.ok(map.hasNode(first.id), 'shared merged node was deleted with its original reaction');
  assert.equal(map.nodes.get(first.id)?.reactionLocalFor, undefined);
  assert.ok(map.segmentsOf(targetReaction).some((seg) => seg.nodeId === first.id));
});

test('overlapping different metabolites never merge', async () => {
  const com = await buildCommunity();
  const map = EditableMap.emptyFromCommunity(com);
  assert.ok(map.addReaction(com, firstInternalReaction(com)));

  const nodes = [...map.nodes.values()].filter((node) =>
    ['metabolite', 'medium'].includes(node.kind)
  );
  const a = nodes[0];
  const b = nodes.find((node) =>
    node.mergedId !== a?.mergedId
    && node.cellId === a?.cellId
    && node.compartment === a?.compartment
  );
  assert.ok(a && b);

  assert.ok(map.moveNode(b.id, a.x, a.y));
  assert.equal(map.mergeOverlappingMetabolite(b.id), null);
  assert.ok(map.hasNode(a.id));
  assert.ok(map.hasNode(b.id));
});
