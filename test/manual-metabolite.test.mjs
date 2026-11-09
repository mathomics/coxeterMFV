/** Manual metabolite insertion respects drawn compartments, preserves biology and reports failed insertion. */
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import factory from '../assets/vendor/glpk.js';
import { initGLPK } from '../src/solver/glpk.js';
import { Model } from '../src/core/model.js';
import { EditableMap } from '../src/map/editable-map.js';
import { nodeFitsAt } from '../src/layout/drag.js';
import { Store } from '../src/ui/store.js';

test.before(async () => {
  globalThis.__GLPK_FACTORY__ = factory;
  globalThis.__GLPK_WASM__ = await readFile(new URL('../assets/vendor/glpk.wasm', import.meta.url));
  assert.equal(await initGLPK(), true);
});

async function makeStore() {
  const store = new Store('.');
  const raw = JSON.parse(await readFile(new URL('../models/ecoli_core_mc.json', import.meta.url), 'utf8'));
  store.models.set('ecoli_core_mc', new Model(raw));
  store.scenarios = JSON.parse(await readFile(new URL('../models/scenarios.json', import.meta.url), 'utf8')).scenarios;
  await store.useScenario('solo-ecoli');
  store.map = EditableMap.emptyFromCommunity(store.community);
  store.mapHistory.clear();
  return store;
}

test('manual insertion near an outer compartment edge obeys the same constraints as dragging', async () => {
  const store = await makeStore();
  const rect = store.map.cells[0].compartments.find(compartment => compartment.id === 'c').rect;
  const solution = store.solution;
  const node = store.addMapMetabolite('eco@g6p_c', { x: rect.x + 1, y: rect.y + rect.h / 2 });
  assert.ok(node);
  assert.ok(nodeFitsAt(store.map, node, node.x, node.y));
  assert.equal(store.solution, solution);
});

test('manual periplasmic insertion stays clear of the nested cytosol', async () => {
  const store = await makeStore();
  const cytosol = store.map.cells[0].compartments.find(compartment => compartment.id === 'c').rect;
  const node = store.addMapMetabolite('eco@glc__D_p', {
    x: cytosol.x - 1, y: cytosol.y + cytosol.h / 2,
  });
  assert.ok(node);
  assert.ok(nodeFitsAt(store.map, node, node.x, node.y));
});

test('manual insertion cannot create an orphan when its compartment is absent from the drawing', async () => {
  const store = await makeStore();
  store.removeMapCompartment('eco', 'c');
  const drawing = store.map.toJSON();
  const version = store.mapHistory.version;
  assert.equal(store.addMapMetabolite('eco@g6p_c', { x: 500, y: 500 }), null);
  assert.deepEqual(store.map.toJSON(), drawing);
  assert.equal(store.mapHistory.version, version);
  assert.deepEqual(store.metabolitesAvailableInCompartment('eco', 'c'), []);
});

test('malformed or nonfinite manual coordinates cannot mutate the drawing or history', async () => {
  const store = await makeStore();
  const drawing = store.map.toJSON();
  const version = store.mapHistory.version;
  for (const value of [true, false, null, '', [], [500], '500', NaN, Infinity, undefined]) {
    for (const coordinates of [{ x: value, y: 500 }, { x: 500, y: value }]) {
      assert.equal(store.addMapMetabolite('eco@g6p_c', coordinates), null);
      assert.deepEqual(store.map.toJSON(), drawing);
      assert.equal(store.mapHistory.version, version);
    }
  }
});

test('manual duplicates have unique identities and round trip through history without changing biology', async () => {
  const store = await makeStore();
  const rect = store.map.cells[0].compartments.find(compartment => compartment.id === 'c').rect;
  const point = { x: rect.x + rect.w / 2, y: rect.y + rect.h / 2 };
  const biology = store.modelEditorModel().toJSON();
  const bounds = store.problem.snapshotBounds();
  const solution = store.solution;
  const first = store.addMapMetabolite('eco@g6p_c', point);
  const second = store.addMapMetabolite('eco@g6p_c', point);
  assert.notEqual(first.id, second.id);
  assert.equal(first.mergedId, second.mergedId);
  const drawing = store.map.toJSON();
  store.undoMap();
  assert.equal(store.map.nodes.has(first.id), true);
  assert.equal(store.map.nodes.has(second.id), false);
  store.redoMap();
  assert.deepEqual(store.map.toJSON(), drawing);
  store.loadMapJSON(store.mapJSON());
  assert.equal(store.map.nodesForMetabolite('eco@g6p_c').length, 2);
  assert.deepEqual(store.modelEditorModel().toJSON(), biology);
  assert.deepEqual(store.problem.snapshotBounds(), bounds);
  assert.equal(store.solution, solution);
});

test('a known undrawn reaction reports missing compartments or lack of room without changing the drawing', async () => {
  const store = await makeStore();
  store.showEmptyMap();
  const drawing = store.map.toJSON();
  const version = store.mapHistory.version;
  const solution = store.solution;
  assert.equal(store.addMapReaction('eco@PFK'), null);
  assert.equal(store.status.state, 'warn');
  assert.match(store.status.message, /PFK.*not drawn.*compartments.*space/i);
  assert.deepEqual(store.map.toJSON(), drawing);
  assert.equal(store.mapHistory.version, version);
  assert.equal(store.solution, solution);
});

test('a known metabolite reports missing drawn geometry without creating an orphan', async () => {
  const store = await makeStore();
  store.showEmptyMap();
  const drawing = store.map.toJSON();
  const version = store.mapHistory.version;
  const solution = store.solution;
  assert.equal(store.addMapMetabolite('eco@g6p_c', { x: 500, y: 500 }), null);
  assert.equal(store.status.state, 'warn');
  assert.match(store.status.message, /g6p_c.*not drawn.*compartment.*point/i);
  assert.deepEqual(store.map.toJSON(), drawing);
  assert.equal(store.mapHistory.version, version);
  assert.equal(store.solution, solution);
});

test('unknown identifiers and an already drawn reaction remain quiet no-ops', async () => {
  const store = await makeStore();
  store.addMapReaction('eco@PGI');
  const status = { ...store.status };
  const drawing = store.map.toJSON();
  const version = store.mapHistory.version;
  assert.equal(store.addMapReaction('eco@PGI'), null);
  assert.equal(store.addMapReaction('eco@missing'), null);
  assert.equal(store.addMapMetabolite('eco@missing_c', { x: 500, y: 500 }), null);
  assert.deepEqual(store.status, status);
  assert.deepEqual(store.map.toJSON(), drawing);
  assert.equal(store.mapHistory.version, version);
});
