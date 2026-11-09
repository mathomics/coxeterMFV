/** New drawing objects respect their compartment while existing geometry and biology stay intact. */
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

function assertConnected(map) {
  for (const segment of map.segments) {
    if (!segment.nodeId) {
      continue;
    }
    const node = map.nodes.get(segment.nodeId);
    const point = segment.side === 'substrate' ? segment.from : segment.to;
    assert.ok(Math.hypot(node.x - point.x, node.y - point.y) < 1e-8, segment.nodeId);
  }
}

test('reaction insertion stays inside an imported compartment smaller than its padding', async () => {
  const store = await makeStore();
  const drawing = store.map.toJSON();
  const rect = { x: 300, y: 300, w: 12, h: 10 };
  drawing.cells[0].compartments.find(compartment => compartment.id === 'c').rect = rect;
  store.loadMapJSON(JSON.stringify(drawing));
  const solution = store.solution;
  const added = store.addMapReaction('eco@PGI');
  assert.ok(added);
  assert.deepEqual({ x: added.reaction.x, y: added.reaction.y }, { x: 306, y: 305 });
  for (const node of store.map.nodes.values()) {
    assert.ok(node.x >= rect.x && node.x <= rect.x + rect.w, node.id);
    assert.ok(node.y >= rect.y && node.y <= rect.y + rect.h, node.id);
  }
  assertConnected(store.map);
  assert.equal(store.solution, solution);
});

test('outer membrane insertion keeps metabolites out of the nested cytosol', async () => {
  const store = await makeStore();
  const solution = store.solution;
  for (const id of ['eco@GLCtex', 'eco@H2Otex']) {
    assert.ok(store.addMapReaction(id));
  }
  for (const node of store.map.nodes.values()) {
    assert.ok(nodeFitsAt(store.map, node, node.x, node.y), node.id);
  }
  assertConnected(store.map);
  assert.equal(store.solution, solution);
});

test('an internal periplasm reaction keeps its marker and metabolites out of the cytosol', async () => {
  const store = await makeStore();
  await store.createModelReaction({
    id: 'QA_PERIPLASM', metabolites: 'glc__D_p:-1, h_p:1', lowerBound: 0, upperBound: 1000,
  });
  const added = store.addMapReaction('eco@QA_PERIPLASM');
  assert.ok(added);
  const marker = { ...added.reaction, kind: 'metabolite', compartment: 'p', r: 5 };
  assert.ok(nodeFitsAt(store.map, marker, marker.x, marker.y), 'reaction marker');
  for (const node of store.map.nodes.values()) {
    assert.ok(nodeFitsAt(store.map, node, node.x, node.y), node.id);
  }
  assertConnected(store.map);
  assert.equal(store.solution.status, 'optimal');
});

test('insertion near a compartment edge contains secondary metabolites and their branch endpoints', async () => {
  const store = await makeStore();
  const rect = store.map.cells[0].compartments.find(compartment => compartment.id === 'c').rect;
  const anchor = { x: rect.x + rect.w - 20, y: rect.y + rect.h - 20 };
  assert.ok(store.addMapReaction('eco@PFK', { anchor }));
  for (const node of store.map.nodes.values()) {
    assert.ok(nodeFitsAt(store.map, node, node.x, node.y), node.id);
  }
  assertConnected(store.map);
});

test('insertion repairs only new objects and preserves the explicitly reused metabolite', async () => {
  const store = await makeStore();
  store.addMapReaction('eco@PGI');
  const anchor = [...store.map.nodes.values()].find(node => node.mergedId === 'eco@f6p_c');
  const drawing = store.map.toJSON();
  const biology = store.modelEditorModel().toJSON();
  const solution = store.solution;
  const added = store.addMapReaction('eco@PFK', {
    anchor: { x: anchor.x + 96, y: anchor.y }, anchorNodeId: anchor.id,
    placementDirection: { x: 1, y: 0 },
  });
  assert.ok(added);
  assert.equal(added.anchorNode, anchor);
  for (const previous of drawing.nodes) {
    const expected = { ...previous };
    if (previous.id === anchor.id) {
      expected.sharedInstance = true;
    }
    assert.deepEqual(store.map.nodes.get(previous.id), expected);
  }
  assert.deepEqual(store.map.segments.slice(0, drawing.segments.length), drawing.segments);
  assert.deepEqual(store.modelEditorModel().toJSON(), biology);
  assert.equal(store.solution, solution);
  assertConnected(store.map);
});

test('an automatic insertion with no distinct marker position preserves the drawing and Redo', async () => {
  const store = await makeStore();
  store.map.cells[0].compartments.find(compartment => compartment.id === 'c').rect = {
    x: 300, y: 300, w: 40, h: 40,
  };
  assert.ok(store.addMapReaction('eco@PGI'));
  assert.ok(store.addMapReaction('eco@GLCtex'));
  store.undoMap();
  const drawing = store.map.toJSON();
  const version = store.mapHistory.version;
  const solution = store.solution;
  assert.equal(store.addMapReaction('eco@PFK'), null);
  assert.deepEqual(store.map.toJSON(), drawing);
  assert.equal(store.mapHistory.version, version);
  assert.equal(store.mapHistory.canRedo, true);
  assert.equal(store.solution, solution);
});
