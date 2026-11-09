/**
 * Scenario imports validate biological settings before replacing the open work.
 * Invalid files leave the drawing, solution and undo history available.
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { Store } from '../src/ui/store.js';
import { Model } from '../src/core/model.js';

async function makeStore() {
  const raw = JSON.parse(await readFile(new URL('../models/ecoli_core_mc.json', import.meta.url), 'utf8'));
  const scenarios = JSON.parse(await readFile(new URL('../models/scenarios.json', import.meta.url), 'utf8'));
  const store = new Store('.');
  store.models.set('ecoli_core_mc', new Model(raw));
  store.scenarios = scenarios.scenarios;
  await store.useScenario('solo-ecoli');
  store.removeMapReaction('eco@ATPM');
  store.clearSolution();
  return store;
}

async function refusedWithoutChanges(store, doc, message) {
  const state = store.stateJSON();
  const drawing = structuredClone(store.map.toJSON());
  const map = store.map;
  const problem = store.problem;
  const solution = store.solution;
  const history = store.mapHistory;
  const unsaved = store.unsavedWork();
  await assert.rejects(store.loadState(JSON.stringify(doc)), message);
  assert.equal(store.stateJSON(), state);
  assert.deepEqual(store.map.toJSON(), drawing);
  assert.equal(store.map, map);
  assert.equal(store.problem, problem);
  assert.equal(store.solution, solution);
  assert.equal(store.solutionCleared, true);
  assert.equal(store.mapHistory, history);
  assert.equal(store.canUndoMap, true);
  assert.deepEqual(store.unsavedWork(), unsaved);
}

test('a file without scenario species and medium is refused without changing the open work', async () => {
  const store = await makeStore();
  for (const doc of [null, [], {}, { species: [], medium: {} }, { species: 'eco', medium: {} }]) {
    await refusedWithoutChanges(store, doc, /Scenario|scenario/);
  }
});

test('invalid species and medium values are refused without changing the open work', async () => {
  const store = await makeStore();
  const valid = JSON.parse(store.stateJSON());
  const invalidSpecies = [
    [null],
    [{ ...valid.species[0], id: '' }],
    [{ ...valid.species[0], file: '' }],
    [{ ...valid.species[0], abundance: -1 }],
    [{ ...valid.species[0], abundance: 'one' }],
    [{ ...valid.species[0], enabled: 'false' }],
    [valid.species[0], { ...valid.species[0] }],
  ];
  for (const species of invalidSpecies) {
    await refusedWithoutChanges(store, { ...valid, species }, /species|Species/);
  }
  for (const medium of [null, [], 'glucose', { glc__D: -10 }, { glc__D: 'ten' }, { glc__D: null }]) {
    await refusedWithoutChanges(store, { ...valid, medium }, /medium|Medium/);
  }
});

test('invalid reaction overrides are refused without changing the open work', async () => {
  const store = await makeStore();
  const valid = JSON.parse(store.stateJSON());
  const bound = { id: 'eco@ATPM', lower: 0, upper: 10 };
  for (const overrides of [
    {},
    [null],
    [{ ...bound, id: '' }],
    [{ ...bound, lower: 'zero' }],
    [{ ...bound, upper: null }],
    [{ ...bound, lower: 11 }],
    [bound, { ...bound }],
  ]) {
    await refusedWithoutChanges(store, { ...valid, overrides }, /override|bounds/);
  }
});

test('an unavailable scenario model is refused before replacing the open work', async (t) => {
  const store = await makeStore();
  const previousFetch = globalThis.fetch;
  globalThis.fetch = async () => ({ ok: false, status: 404 });
  t.after(() => {
    globalThis.fetch = previousFetch;
  });
  const doc = JSON.parse(store.stateJSON());
  doc.species[0].file = 'missing-model';
  await refusedWithoutChanges(store, doc, /missing-model/);
});

test('valid scenario numbers keep zero supply, zero abundance and negative reversible bounds', async () => {
  const store = await makeStore();
  const doc = JSON.parse(store.stateJSON());
  doc.medium.o2 = 0;
  doc.species.push({ id: 'other', file: 'ecoli_core_mc', abundance: 0, enabled: false });
  doc.overrides = [{ id: 'eco@PGI', lower: -100, upper: 100 }];
  await store.loadState(JSON.stringify(doc));
  assert.equal(store.medium.o2, 0);
  assert.equal(store.species[1].abundance, 0);
  assert.equal(store.species[1].enabled, false);
  assert.deepEqual(store.currentReactionBounds('eco@PGI'), { lower: -100, upper: 100 });
  assert.equal(store.solution.status, 'optimal');
});
