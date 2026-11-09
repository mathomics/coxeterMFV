/** Canvas transactions belong to one drawing history and keep their snapshots detached. */
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import factory from '../assets/vendor/glpk.js';
import { initGLPK } from '../src/solver/glpk.js';
import { Model } from '../src/core/model.js';
import { EditableMap } from '../src/map/editable-map.js';
import { Store } from '../src/ui/store.js';

test.before(async () => {
  globalThis.__GLPK_FACTORY__ = factory;
  globalThis.__GLPK_WASM__ = await readFile(new URL('../assets/vendor/glpk.wasm', import.meta.url));
  assert.equal(await initGLPK(), true);
});

async function fixture() {
  const store = new Store('.');
  const raw = JSON.parse(await readFile(new URL('../models/ecoli_core_mc.json', import.meta.url), 'utf8'));
  store.models.set('ecoli_core_mc', new Model(raw));
  store.scenarios = JSON.parse(await readFile(new URL('../models/scenarios.json', import.meta.url), 'utf8')).scenarios;
  await store.useScenario('solo-ecoli');
  store.map = EditableMap.emptyFromCommunity(store.community);
  store.addMapReaction('eco@PGI');
  store.mapHistory.clear();
  const node = store.map.nodesForMetabolite('eco@g6p_c')[0];
  return { store, node };
}

test('a canvas transaction can commit only once', async () => {
  const { store, node } = await fixture();
  const transaction = store.beginMapEdit('Move metabolite');
  store.moveNode(node.id, node.x + 30, node.y + 20);
  assert.equal(store.commitMapEdit(transaction), true);
  const version = store.mapHistory.version;
  assert.equal(store.commitMapEdit(transaction), false);
  assert.equal(store.mapHistory.version, version);
  assert.equal(store.mapHistory.undoStack.length, 1);
});

test('a canvas transaction cannot record against a replacement full map', async () => {
  const { store } = await fixture();
  const transaction = store.beginMapEdit('Move metabolite');
  store.showFullMap();
  const before = store.map.toJSON();
  const version = store.mapHistory.version;
  assert.equal(store.commitMapEdit(transaction), false);
  assert.deepEqual(store.map.toJSON(), before);
  assert.equal(store.mapHistory.version, version);
});

test('a canvas transaction cannot record against a reloaded scenario history', async () => {
  const { store } = await fixture();
  const transaction = store.beginMapEdit('Move metabolite');
  await store.useScenario('solo-ecoli');
  const before = store.map.toJSON();
  const solution = store.solution;
  const version = store.mapHistory.version;
  assert.equal(store.commitMapEdit(transaction), false);
  assert.deepEqual(store.map.toJSON(), before);
  assert.equal(store.mapHistory.version, version);
  assert.equal(store.mapHistory.undoStack.length, 0);
  assert.equal(store.solution, solution);
});

test('a canvas transaction cannot replace Redo after another history step is undone', async () => {
  const { store } = await fixture();
  store.addMapText('Keep this note', 300, 300);
  const transaction = store.beginMapEdit('Move metabolite');
  store.undoMap();
  const version = store.mapHistory.version;
  assert.equal(store.mapHistory.canRedo, true);
  assert.equal(store.commitMapEdit(transaction), false);
  assert.equal(store.mapHistory.canRedo, true);
  assert.equal(store.mapHistory.version, version);
});

test('a canvas transaction snapshot stays detached while its attached geometry moves', async () => {
  const { store, node } = await fixture();
  const transaction = store.beginMapEdit('Move metabolite');
  const before = structuredClone(transaction.before);
  store.moveNode(node.id, node.x + 30, node.y + 20);
  assert.deepEqual(transaction.before, before);
});

test('an ordinary canvas transaction preserves the model and supports exact Undo and Redo', async () => {
  const { store, node } = await fixture();
  const biology = store.modelEditorModel().toJSON();
  const bounds = store.problem.snapshotBounds();
  const solution = store.solution;
  const before = store.map.toJSON();
  const transaction = store.beginMapEdit('Move metabolite');
  store.moveNode(node.id, node.x + 30, node.y + 20);
  const after = store.map.toJSON();
  assert.equal(store.commitMapEdit(transaction), true);
  store.undoMap();
  assert.deepEqual(store.map.toJSON(), before);
  store.redoMap();
  assert.deepEqual(store.map.toJSON(), after);
  assert.deepEqual(store.modelEditorModel().toJSON(), biology);
  assert.deepEqual(store.problem.snapshotBounds(), bounds);
  assert.equal(store.solution, solution);
});
