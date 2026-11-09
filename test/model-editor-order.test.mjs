/** Model creation belongs to its original context even when asynchronous rebuilding finishes later. */
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import factory from '../assets/vendor/glpk.js';
import { initGLPK } from '../src/solver/glpk.js';
import { Model } from '../src/core/model.js';
import { Store } from '../src/ui/store.js';

test.before(async () => {
  globalThis.__GLPK_FACTORY__ = factory;
  globalThis.__GLPK_WASM__ = await readFile(new URL('../assets/vendor/glpk.wasm', import.meta.url));
  assert.equal(await initGLPK(), true);
});

async function makeStore() {
  const store = new Store('.');
  for (const id of ['ecoli_core_mc', 'yeast_core_mc']) {
    const raw = JSON.parse(await readFile(new URL(`../models/${id}.json`, import.meta.url), 'utf8'));
    store.models.set(id, new Model(raw));
  }
  store.scenarios = JSON.parse(await readFile(new URL('../models/scenarios.json', import.meta.url), 'utf8')).scenarios;
  await store.useScenario('solo-ecoli');
  return store;
}

function holdNext(store, method, after) {
  const original = store[method].bind(store);
  let release;
  let started;
  const held = new Promise(resolve => {
    release = resolve;
  });
  const ready = new Promise(resolve => {
    started = resolve;
  });
  let first = true;
  store[method] = async (...args) => {
    if (!first) {
      return original(...args);
    }
    first = false;
    if (!after) {
      started();
      await held;
      return original(...args);
    }
    const result = await original(...args);
    started();
    await held;
    return result;
  };
  return { ready, release };
}

function create(store, kind) {
  if (kind === 'metabolite') {
    return store.createModelMetabolite({ id: 'qa_metabolite_c', name: 'QA metabolite', compartment: 'c' });
  }
  return store.createModelReaction({
    id: 'QA_PGI', metabolites: 'g6p_c:-1, f6p_c:1', lowerBound: -10, upperBound: 10,
  }, { addToMap: true });
}

for (const kind of ['metabolite', 'reaction']) {
  for (const stage of ['during', 'after']) {
    test(`a ${kind} creation finishing ${stage} rebuilding cannot replace a newer scenario`, async () => {
      const store = await makeStore();
      const held = holdNext(store, stage === 'during' ? 'loadModel' : 'rebuild', stage === 'after');
      const pending = create(store, kind);
      await held.ready;
      await store.useScenario('solo-levadura');
      const community = store.community;
      const problem = store.problem;
      const map = store.map;
      const solution = store.solution;
      const status = JSON.stringify(store.status);
      held.release();
      await pending;
      assert.equal(store.community, community);
      assert.equal(store.problem, problem);
      assert.equal(store.map, map);
      assert.equal(store.solution, solution);
      assert.equal(JSON.stringify(store.status), status);
      assert.equal(store.scenario.id, 'solo-levadura');
      assert.equal(store.models.get('ecoli_core_mc').reactions.has('QA_PGI'), kind === 'reaction');
      assert.equal(store.models.get('ecoli_core_mc').metabolites.has('qa_metabolite_c'), kind === 'metabolite');
    });
  }
}

for (const stage of ['during', 'after']) {
  test(`returning to the original scenario cannot revive a reaction creation held ${stage} rebuilding`, async () => {
    const store = await makeStore();
    const held = holdNext(store, stage === 'during' ? 'loadModel' : 'rebuild', stage === 'after');
    const pending = create(store, 'reaction');
    await held.ready;
    await store.useScenario('solo-levadura');
    await store.useScenario('solo-ecoli');
    const map = store.map;
    const drawing = JSON.stringify(map.toJSON());
    const solution = store.solution;
    const version = store.mapHistory.version;
    const status = JSON.stringify(store.status);
    held.release();
    await pending;
    assert.equal(store.map, map);
    assert.equal(JSON.stringify(store.map.toJSON()), drawing);
    assert.equal(store.solution, solution);
    assert.equal(store.mapHistory.version, version);
    assert.equal(JSON.stringify(store.status), status);
    assert.equal(store.problem.rxnIndex.has('eco@QA_PGI'), true);
    assert.equal(store.unsavedWork().model, true);
  });
}

test('overlapping valid creations in one context retain both biological and drawing edits', async () => {
  const store = await makeStore();
  const held = holdNext(store, 'loadModel', false);
  const pending = create(store, 'reaction');
  await held.ready;
  try {
    await store.createModelReaction({
      id: 'QA_SECOND', metabolites: 'g6p_c:-1, f6p_c:1', lowerBound: -5, upperBound: 5,
    }, { addToMap: true });
  } finally {
    held.release();
    await pending;
  }
  for (const id of ['QA_PGI', 'QA_SECOND']) {
    assert.equal(store.modelEditorModel().reactions.has(id), true);
    assert.equal(store.problem.rxnIndex.has(`eco@${id}`), true);
    assert.equal(store.map.reactions.has(`eco@${id}`), true);
  }
  assert.equal(store.solution.status, 'optimal');
  assert.equal(store.unsavedWork().model, true);
});

test('a rejected coefficient edit preserves another creation waiting in the same context', async () => {
  const store = await makeStore();
  const held = holdNext(store, 'loadModel', false);
  const pending = create(store, 'reaction');
  await held.ready;
  const model = store.modelEditorModel();
  const biology = model.toJSON();
  const revision = store.modelEdits.get(model);
  const problem = store.problem;
  const solution = store.solution;
  try {
    await assert.rejects(store.createModelReaction({
      id: 'QA_REJECTED', metabolites: { g6p_c: true, f6p_c: 1 }, lowerBound: 0, upperBound: 10,
    }), /coefficient.*g6p_c/i);
    assert.deepEqual(model.toJSON(), biology);
    assert.equal(store.modelEdits.get(model), revision);
    assert.equal(store.problem, problem);
    assert.equal(store.solution, solution);
  } finally {
    held.release();
    await pending;
  }
  assert.equal(store.problem.rxnIndex.has('eco@QA_PGI'), true);
  assert.equal(store.problem.rxnIndex.has('eco@QA_REJECTED'), false);
  assert.equal(store.solution.status, 'optimal');
});
