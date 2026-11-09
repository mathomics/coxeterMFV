/**
 * A map opened before the model it was drawn over.
 *
 * A model imported from a file lives only as long as the page, so a map drawn
 * over it and opened again after a reload finds nothing to sit on. It is kept
 * instead of refused, and drawn once that model is loaded.
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Store } from '../src/ui/store.js';
import { Model } from '../src/core/model.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const yeastText = await readFile(join(root, 'models/yeast_core_mc.json'), 'utf8');

const otherModel = {
  id: 'other',
  compartments: { c: 'Cytosol', e: 'Extracellular' },
  metabolites: [{ id: 'a_e', compartment: 'e' }, { id: 'a_c', compartment: 'c' }],
  reactions: [
    { id: 'EX_a_e', metabolites: { a_e: -1 }, lower_bound: -10, upper_bound: 1000 },
    { id: 'At', metabolites: { a_e: -1, a_c: 1 }, lower_bound: 0, upper_bound: 1000 },
    { id: 'BIOMASS', metabolites: { a_c: -1 }, lower_bound: 0, upper_bound: 1000, objective_coefficient: 1 },
  ],
};

/* A page as it opens: a teaching scenario that has nothing to do with the map. */
async function freshStore() {
  const store = new Store('.');
  store.models.set('other', new Model(otherModel));
  store.scenarios = [{
    id: 'demo', name: 'Demo', objective: 'equal-growth',
    species: [{ id: 'demo', model: 'other', abundance: 1 }],
    medium: { a: 10 },
  }];
  await store.useScenario('demo');
  return store;
}

/* A map drawn by hand over an imported yeast, with one metabolite moved. */
async function savedYeastMap() {
  const store = await freshStore();
  await store.importModelFiles([{ name: 'my_yeast.json', text: yeastText }]);
  const [nodeId, node] = [...store.map.nodes].find(([, n]) => n.mergedId);
  const tx = store.beginMapEdit('Move');
  store.moveNode(nodeId, node.x + 123, node.y + 45);
  store.commitMapEdit(tx);
  return { text: store.mapJSON(), nodeId, x: store.map.nodes.get(nodeId).x, reactions: store.map.reactions.size };
}

test('a map whose imported model is not loaded waits, and is drawn when that model is imported', async () => {
  const saved = await savedYeastMap();
  const store = await freshStore();
  const before = store.map.toJSON();

  assert.equal(await store.loadMapDocument(saved.text, { name: 'yeast_map.json' }), null);
  assert.equal(store.waitingMap?.name, 'yeast_map.json');
  assert.equal(store.waitingMap.needs, '“my_yeast”', 'named after the file the model came from');
  assert.deepEqual(store.map.toJSON(), before, 'the drawing on screen is untouched while the map waits');

  await store.importModelFiles([{ name: 'my_yeast.json', text: yeastText }]);
  assert.equal(store.waitingMap, null);
  assert.equal(store.map.reactions.size, saved.reactions);
  assert.equal(store.map.nodes.get(saved.nodeId).x, saved.x, 'the hand-placed metabolite is where it was saved');
  assert.match(store.status.message, /Imported 1 model\(s\).*Drew “yeast_map\.json”, which was waiting for this model\./);
});

test('the model under a re-exported file name still takes its waiting map', async () => {
  const saved = await savedYeastMap();
  const store = await freshStore();
  await store.loadMapDocument(saved.text, { name: 'yeast_map.json' });
  await store.importModelFiles([{ name: 'my_yeast_v2.json', text: yeastText }]);
  assert.equal(store.waitingMap, null);
  assert.equal(store.map.nodes.get(saved.nodeId).x, saved.x);
});

test('a model the map does not fit leaves it waiting, and so does switching scenario', async () => {
  const saved = await savedYeastMap();
  const store = await freshStore();
  await store.loadMapDocument(saved.text, { name: 'yeast_map.json' });

  await store.useScenario('demo');
  assert.ok(store.waitingMap, 'browsing scenarios is not loading the map’s model');

  await store.importModelFiles([{ name: 'other.json', text: JSON.stringify(otherModel) }]);
  assert.ok(store.waitingMap);
  assert.equal(store.status.state, 'warn');
  assert.match(store.status.message, /“yeast_map\.json” is still waiting: it does not fit this model/);

  await store.importModelFiles([{ name: 'my_yeast.json', text: yeastText }]);
  assert.equal(store.waitingMap, null);
});

test('a waiting map can be forgotten, and is replaced by the next map opened', async () => {
  const saved = await savedYeastMap();
  const store = await freshStore();
  await store.loadMapDocument(saved.text, { name: 'first.json' });
  await store.loadMapDocument(saved.text, { name: 'second.json' });
  assert.equal(store.waitingMap.name, 'second.json');

  store.discardWaitingMap();
  assert.equal(store.waitingMap, null);
  await store.importModelFiles([{ name: 'my_yeast.json', text: yeastText }]);
  assert.notEqual(store.map.nodes.get(saved.nodeId)?.x, saved.x, 'a forgotten map is not drawn');
});

test('a file that is not a map is still refused, not kept', async () => {
  const store = await freshStore();
  const broken = JSON.stringify({ format: 'EscherMCMCMap', version: 3, metadata: null, map: { reactions: 'x' } });
  await assert.rejects(store.loadMapDocument(broken, { name: 'broken.json' }), /EditableMap structure/);
  assert.equal(store.waitingMap, null);
});
