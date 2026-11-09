/** Reaction drawing controls preserve deliberate geometry, history and biological calculations. */
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

async function makeStore() {
  const store = new Store('.');
  const raw = JSON.parse(await readFile(new URL('../models/ecoli_core_mc.json', import.meta.url), 'utf8'));
  store.models.set('ecoli_core_mc', new Model(raw));
  store.scenarios = JSON.parse(await readFile(new URL('../models/scenarios.json', import.meta.url), 'utf8')).scenarios;
  await store.useScenario('solo-ecoli');
  store.map = EditableMap.emptyFromCommunity(store.community);
  store.addMapReaction('eco@PFK');
  store.addMapReaction('eco@PGI');
  store.setMapReactionLinear('eco@PFK', true);
  store.mapHistory.clear();
  store.select('eco@PFK');
  return store;
}

function slide(store) {
  const reaction = store.map.reactions.get('eco@PFK');
  const from = store.map.nodes.get(reaction.linear.from);
  const to = store.map.nodes.get(reaction.linear.to);
  const transaction = store.beginMapEdit('Slide reaction');
  store.map.moveReaction(reaction.id, from.x + 0.25 * (to.x - from.x), from.y + 0.25 * (to.y - from.y));
  assert.equal(store.commitMapEdit(transaction), true);
  assert.ok(Math.abs(reaction.linear.t - 0.25) < 1e-9);
}

function bendCofactor(store) {
  const reaction = store.map.reactions.get('eco@PFK');
  const segment = store.map.segmentsOf(reaction.id).find(segment => segment.nodeId && segment.b1
    && segment.nodeId !== reaction.linear.from && segment.nodeId !== reaction.linear.to);
  assert.ok(segment);
  const transaction = store.beginMapEdit('Bend cofactor');
  assert.ok(store.map.moveBranchControl(reaction.id, segment.nodeId, 'b1', segment.b1.x + 17, segment.b1.y - 11));
  assert.equal(store.commitMapEdit(transaction), true);
}

function assertIntact(map) {
  for (const node of map.nodes.values()) {
    assert.ok(Number.isFinite(node.x) && Number.isFinite(node.y), node.id);
  }
  for (const segment of map.segments) {
    for (const point of [segment.from, segment.to, segment.b1, segment.b2]) {
      if (point) {
        assert.ok(Number.isFinite(point.x) && Number.isFinite(point.y), segment.id);
      }
    }
    if (segment.nodeId) {
      const node = map.nodes.get(segment.nodeId);
      assert.ok(node, segment.nodeId);
      const end = segment.side === 'substrate' ? segment.from : segment.to;
      assert.ok(Math.hypot(end.x - node.x, end.y - node.y) < 1e-6, segment.nodeId);
    }
  }
}

test('pressing active Linear preserves a slid marker, bent cofactors and Redo', async () => {
  const store = await makeStore();
  slide(store);
  bendCofactor(store);
  store.addMapText('Redo control', 10, 10);
  store.undoMap();
  const before = store.map.toJSON();
  const version = store.mapHistory.version;
  const solution = store.solution;
  assert.ok(store.mapHistory.canRedo);
  store.setMapReactionLinear('eco@PFK', true);
  assert.deepEqual(store.map.toJSON(), before);
  assert.equal(store.mapHistory.version, version);
  assert.equal(store.mapHistory.canRedo, true);
  assert.equal(store.solution, solution);
});

test('choosing the same explicit line endpoint preserves a manually bent cofactor', async () => {
  const store = await makeStore();
  const endpoint = store.map.reactions.get('eco@PFK').linear.from;
  store.setMapReactionLineEnd('eco@PFK', endpoint);
  bendCofactor(store);
  const before = store.map.toJSON();
  const version = store.mapHistory.version;
  store.setMapReactionLineEnd('eco@PFK', endpoint);
  assert.deepEqual(store.map.toJSON(), before);
  assert.equal(store.mapHistory.version, version);
});

test('active Linear still adopts different endpoints picked on the map', async () => {
  const store = await makeStore();
  slide(store);
  const sides = store.map.reactionSides('eco@PFK');
  const from = sides.substrates.find(entry => entry.label === 'atp').nodeId;
  const to = sides.products.find(entry => entry.label === 'adp').nodeId;
  store.toggleMultiSelection('node', from);
  store.toggleMultiSelection('node', to);
  const version = store.mapHistory.version;
  assert.ok(store.setMapReactionLinear('eco@PFK', true));
  const linear = store.map.reactions.get('eco@PFK').linear;
  assert.equal(linear.from, from);
  assert.equal(linear.to, to);
  assert.equal(linear.t, 0.5);
  assert.equal(store.mapHistory.version, version + 1);
  assertIntact(store.map);
});

test('unsupported compass names cannot introduce nonfinite drawing coordinates', async () => {
  for (const direction of ['constructor', '__proto__', 'toString', 'upwards']) {
    const store = await makeStore();
    const before = store.map.toJSON();
    const version = store.mapHistory.version;
    assert.equal(store.alignMapReaction('eco@PFK', direction), null);
    assertIntact(store.map);
    assert.deepEqual(store.map.toJSON(), before);
    assert.equal(store.mapHistory.version, version);
  }
});

test('mixed reaction geometry edits remain connected and undoable without changing the calculation', async () => {
  const store = await makeStore();
  const biology = store.models.get('ecoli_core_mc').toJSON();
  const solution = store.solution;
  const problem = store.problem;
  const bounds = problem.snapshotBounds();
  const before = store.map.toJSON();
  const rid = 'eco@PFK';
  store.setMapReactionLinear(rid, false);
  store.rotateMapReaction(rid, Math.PI / 2);
  store.mirrorMapReaction(rid);
  store.flipMapReaction(rid);
  for (const direction of ['N', 'NE', 'E', 'SE', 'S', 'SW', 'W', 'NW']) {
    store.alignMapReaction(rid, direction);
    assertIntact(store.map);
  }
  const sides = store.map.reactionSides(rid);
  const from = sides.substrates.find(entry => entry.label === 'atp').nodeId;
  store.setMapReactionLineEnd(rid, from);
  store.swapMapReactionMetabolites(rid, sides.products[0].nodeId, sides.products[1].nodeId);
  store.setMapNodeHidden(from, true);
  store.tidyMapReaction(rid);
  store.setMapNodeHidden(from, false);
  store.duplicateMapMetabolite(from, { offset: 25 });
  assertIntact(store.map);
  const after = store.map.toJSON();
  const restored = EditableMap.fromJSON(JSON.parse(JSON.stringify(after)));
  assert.deepEqual(restored.toJSON(), after);
  assertIntact(restored);
  const count = store.mapHistory.undoStack.length;
  for (let index = 0; index < count; index += 1) {
    assert.ok(store.undoMap());
    assertIntact(store.map);
  }
  assert.deepEqual(store.map.toJSON(), before);
  for (let index = 0; index < count; index += 1) {
    assert.ok(store.redoMap());
    assertIntact(store.map);
  }
  assert.deepEqual(store.map.toJSON(), after);
  assert.deepEqual(store.models.get('ecoli_core_mc').toJSON(), biology);
  assert.equal(store.problem, problem);
  assert.deepEqual(problem.snapshotBounds(), bounds);
  assert.equal(store.solution, solution);
});
