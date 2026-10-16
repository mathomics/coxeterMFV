import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import { EditableMap } from '../src/map/editable-map.js';
import { renderMap } from '../src/render/mapview.js';
import { FluxScale } from '../src/render/theme.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

async function map() {
  const raw = JSON.parse(await readFile(join(root, 'models', 'ecoli_core_mc.json'), 'utf8'));
  const com = new CommunityModel({
    species: [{ id: 'eco', model: new Model(raw), abundance: 1 }],
    medium: { glc__D: { lower: -10 }, o2: { lower: -20 } },
  });
  return EditableMap.fromCommunity(com);
}

function primaryNode(m) {
  return [...m.nodes.values()].find((n) => n.kind === 'metabolite');
}

test('metabolite circles have only primary and secondary sizes', async () => {
  const m = await map();
  const node = primaryNode(m);
  assert.ok(node);

  assert.equal(m.setNodeRadius(node.id, 7).r, 7);
  assert.equal(node.visualRole, 'secondary');
  assert.equal(m.setNodeRadius(node.id, 31).r, 13);
  assert.equal(node.visualRole, 'primary');
  assert.equal(m.setNodeRadius(node.id, -100).r, 7);
  assert.equal(m.setNodeRadius(node.id, 1000).r, 13);
});

test('hiding a metabolite keeps it in the map data and reaction membership', async () => {
  const m = await map();
  const node = primaryNode(m);
  const reactions = m.segmentsAt(node.id).map((s) => s.reactionId);
  assert.ok(reactions.length > 0);

  const beforeStats = m.stats();
  assert.equal(m.setNodeHidden(node.id, true), node);
  assert.equal(node.hidden, true);
  assert.ok(m.hasNode(node.id));
  assert.deepEqual(m.stats(), beforeStats);

  for (const rid of reactions) assert.ok(m.hasReaction(rid));
});

test('a hidden metabolite and its attached branches are not rendered', async () => {
  const m = await map();
  const node = primaryNode(m);
  assert.ok(m.segmentsAt(node.id).length > 0);
  m.setNodeHidden(node.id, true);

  const svg = renderMap(m, { fluxes: new Map(), scale: new FluxScale([]) });
  assert.ok(!svg.includes(`data-node="${node.id}"`));

  for (const seg of m.segmentsAt(node.id)) {
    const marker = `data-rxn="${seg.reactionId}" data-node="${node.id}"`;
    assert.ok(!svg.includes(marker), marker);
  }
});

test('showAllHiddenNodes restores hidden metabolites', async () => {
  const m = await map();
  const nodes = [...m.nodes.values()].filter((n) => ['metabolite', 'medium'].includes(n.kind)).slice(0, 3);
  nodes.forEach((n) => m.setNodeHidden(n.id, true));
  assert.equal(m.hiddenNodeCount(), nodes.length);

  const restored = m.showAllHiddenNodes();
  assert.equal(restored.length, nodes.length);
  assert.equal(m.hiddenNodeCount(), 0);
  nodes.forEach((n) => assert.equal(n.hidden, false));
});

test('visual role size and hidden state survive EditableMap JSON save/load', async () => {
  const m = await map();
  const node = primaryNode(m);
  m.setNodePrimary(node.id, false);
  m.setNodeHidden(node.id, true);

  const back = EditableMap.fromJSON(JSON.parse(JSON.stringify(m.toJSON())));
  const restored = back.nodes.get(node.id);
  assert.equal(restored.r, 7);
  assert.equal(restored.visualRole, 'secondary');
  assert.equal(restored.hidden, true);
});
