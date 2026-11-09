/** Default drawing placement separates additions and reports optional drawing failures without losing biology. */
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
  return store;
}

function reactionSpec() {
  return { id: 'QA_PGI', metabolites: 'g6p_c:-1, f6p_c:1', lowerBound: -10, upperBound: 10 };
}

test('creation on an empty drawing reports that requested map insertion did not happen', async () => {
  const store = await makeStore();
  store.showEmptyMap();
  const drawing = store.map.toJSON();
  const version = store.mapHistory.version;
  const result = await store.createModelReaction(reactionSpec(), { addToMap: true });
  assert.equal(result.mapResult, null);
  assert.equal(store.status.state, 'warn');
  assert.match(store.status.message, /created.*not drawn/i);
  assert.deepEqual(store.map.toJSON(), drawing);
  assert.equal(store.mapHistory.version, version);
  assert.equal(store.modelEditorModel().reactions.has('QA_PGI'), true);
  assert.equal(store.problem.rxnIndex.has('eco@QA_PGI'), true);
  assert.equal(store.solution.status, 'optimal');
});

test('creation without map insertion on an empty drawing remains an ordinary success', async () => {
  const store = await makeStore();
  store.showEmptyMap();
  const drawing = store.map.toJSON();
  const result = await store.createModelReaction(reactionSpec(), { addToMap: false });
  assert.equal(result.mapResult, null);
  assert.equal(store.status.state, 'ok');
  assert.deepEqual(store.map.toJSON(), drawing);
  assert.equal(store.problem.rxnIndex.has('eco@QA_PGI'), true);
  assert.equal(store.solution.status, 'optimal');
});

test('default internal additions use separate space while preserving all previous geometry', async () => {
  const store = await makeStore();
  store.map = EditableMap.emptyFromCommunity(store.community);
  const solution = store.solution;
  for (const rid of ['eco@PGI', 'eco@PFK', 'eco@GAPD']) {
    const before = store.map.toJSON();
    const priorNodes = [...store.map.nodes.values()].map(node => ({ ...node }));
    const result = store.addMapReaction(rid);
    assert.ok(result);
    const current = result.reaction;
    for (const reaction of before.reactions) {
      assert.ok(Math.hypot(current.x - reaction.x, current.y - reaction.y) >= 20);
    }
    for (const id of result.nodes) {
      const node = store.map.nodes.get(id);
      for (const prior of priorNodes) {
        assert.ok(Math.hypot(node.x - prior.x, node.y - prior.y) >= (node.r ?? 10) + (prior.r ?? 10));
      }
    }
    const after = store.map.toJSON();
    assert.deepEqual(after.cells, before.cells);
    assert.deepEqual(after.segments.slice(0, before.segments.length), before.segments);
    for (const node of before.nodes) {
      assert.deepEqual(after.nodes.find(current => current.id === node.id), node);
    }
    for (const reaction of before.reactions) {
      assert.deepEqual(after.reactions.find(current => current.id === reaction.id), reaction);
    }
  }
  assert.equal(store.solution, solution);
});

test('default transport additions remain on their membrane without stacking their markers', async () => {
  const store = await makeStore();
  store.map = EditableMap.emptyFromCommunity(store.community);
  const first = store.addMapReaction('eco@GLCptspp');
  const second = store.addMapReaction('eco@H2Otpp');
  assert.ok(first && second);
  assert.equal(first.reaction.onMembrane, true);
  assert.equal(second.reaction.onMembrane, true);
  assert.equal(first.reaction.boundaryId, second.reaction.boundaryId);
  assert.ok(Math.hypot(first.reaction.x - second.reaction.x, first.reaction.y - second.reaction.y) >= 20);
});

test('an explicit requested position is honored even when another marker is already there', async () => {
  const store = await makeStore();
  store.map = EditableMap.emptyFromCommunity(store.community);
  const rect = store.map.cells[0].compartments.find(compartment => compartment.id === 'c').rect;
  const anchor = { x: rect.x + rect.w / 2, y: rect.y + rect.h / 2 };
  const first = store.addMapReaction('eco@PGI', { anchor });
  const second = store.addMapReaction('eco@PFK', { anchor });
  assert.deepEqual({ x: first.reaction.x, y: first.reaction.y }, anchor);
  assert.deepEqual({ x: second.reaction.x, y: second.reaction.y }, anchor);
});

test('nonfinite or malformed requested positions cannot add broken drawing objects', async () => {
  const store = await makeStore();
  store.map = EditableMap.emptyFromCommunity(store.community);
  const drawing = store.map.toJSON();
  const version = store.mapHistory.version;
  const anchors = [
    { x: NaN, y: 10 }, { x: 10, y: Infinity }, { x: '10', y: 10 }, { x: null, y: 10 },
    {}, [10, 20], false, 0, '',
  ];
  for (const anchor of anchors) {
    assert.equal(store.addMapReaction('eco@PFK', { anchor }), null);
    assert.deepEqual(store.map.toJSON(), drawing);
    assert.equal(store.mapHistory.version, version);
  }
});

test('default placement can add every reaction that fits at an explicit position', async () => {
  const store = await makeStore();
  const automatic = EditableMap.emptyFromCommunity(store.community);
  const explicit = EditableMap.emptyFromCommunity(store.community);
  const missing = [];
  for (const id of store.community.merged.reactions.keys()) {
    if (!explicit.addReaction(store.community, id, { anchor: { x: 500, y: 500 } })) {
      continue;
    }
    if (!automatic.addReaction(store.community, id)) {
      missing.push(id);
    }
  }
  assert.deepEqual(missing, []);
});
