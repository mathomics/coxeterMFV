/** Scenario files restore SteadyCom work without hiding edits or inheriting another community. */
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
  await store.useScenario('aerobio-mixto');
  return store;
}

function holdNextTask(store) {
  const run = store.runSteadyComTask;
  let started;
  let release;
  const ready = new Promise(resolve => {
    started = resolve;
  });
  const held = new Promise(resolve => {
    release = resolve;
  });
  let first = true;
  store.runSteadyComTask = async (...args) => {
    const result = await run(...args);
    if (first) {
      first = false;
      started();
      await held;
    }
    return result;
  };
  return { ready, release };
}

test('a status message cannot allow reports during a fixed growth calculation', async () => {
  const store = await makeStore();
  const held = holdNextTask(store);
  const pending = store.solveSteadyComAt(0.2);
  await held.ready;
  try {
    store.status = { ...store.status, state: 'warn', message: 'A different action changed the notice.' };
    await assert.rejects(store.reproducibilityBundle(), /Wait for the current analysis/);
    await assert.rejects(store.methodsProvenanceReport(), /Wait for the current analysis/);
  } finally {
    held.release();
    await pending;
  }
});

test('a scenario opened at fixed growth is saved after its composition is recalculated', async () => {
  const store = await makeStore();
  const document = JSON.parse(store.stateJSON());
  document.steadyCom = { mu: 0.1 };
  await store.loadState(JSON.stringify(document));
  assert.equal(store.solution.steadyCom.mu, 0.1);
  assert.notDeepEqual(store.species.map(species => species.abundance), document.species.map(species => species.abundance));
  assert.equal(store.unsavedWork().scenario, false);
});

test('changing fixed growth marks the scenario unsaved even when composition stays the same', async () => {
  const store = await makeStore();
  await store.useScenario('solo-ecoli');
  await store.solveSteadyComAt(0.1);
  store.markScenarioSaved();
  const composition = store.species.map(species => species.abundance);
  await store.solveSteadyComAt(0.2);
  assert.deepEqual(store.species.map(species => species.abundance), composition);
  assert.equal(store.unsavedWork().scenario, true);
});

test('clearing fixed growth marks its removed scenario analysis unsaved', async () => {
  const store = await makeStore();
  await store.solveSteadyComAt(0.1);
  store.markScenarioSaved();
  store.clearSolution();
  assert.equal(JSON.parse(store.stateJSON()).steadyCom, null);
  assert.equal(store.unsavedWork().scenario, true);
});

test('edits made during scenario recalculation stay unsaved and can be undone back to saved', async () => {
  const store = await makeStore();
  const document = JSON.parse(store.stateJSON());
  document.steadyCom = { mu: 0.1 };
  const width = store.view.widthByFlux;
  const held = holdNextTask(store);
  const pending = store.loadState(JSON.stringify(document));
  await held.ready;
  store.setView({ widthByFlux: !width });
  held.release();
  await pending;
  assert.equal(store.unsavedWork().scenario, true);
  store.setView({ widthByFlux: width });
  assert.equal(store.unsavedWork().scenario, false);
});

test('an automatic opening solve preserves edits made while its result is pending', async () => {
  const store = await makeStore();
  store.scenario = { ...store.scenario, id: 'published:steadycom-ecoli-auxotrophs', published: 'steadycom-ecoli-auxotrophs' };
  store.publishedCommunityId = 'steadycom-ecoli-auxotrophs';
  store.markScenarioSaved();
  const width = store.view.widthByFlux;
  const held = holdNextTask(store);
  const pending = store.solveSteadyComOnOpen();
  await held.ready;
  store.setView({ widthByFlux: !width });
  held.release();
  await pending;
  assert.equal(store.unsavedWork().scenario, true);
  store.setView({ widthByFlux: width });
  assert.equal(store.unsavedWork().scenario, false);
});

test('an imported scenario outside the catalogue keeps its own identity and original medium', async () => {
  const store = await makeStore();
  const document = JSON.parse(store.stateJSON());
  document.scenario = 'custom-steadycom';
  document.medium.glc__D = 3;
  await store.loadState(JSON.stringify(document));
  assert.equal(store.scenario.id, document.scenario);
  assert.equal(store.scenario.name, 'Imported scenario');
  assert.equal(store.baseMedium.glc__D, 3);
  assert.equal(store.unsavedWork().scenario, false);
});

test('an imported scenario outside the catalogue does not inherit a published formulation', async () => {
  const store = await makeStore();
  const document = JSON.parse(store.stateJSON());
  document.scenario = 'custom-steadycom';
  document.steadyCom = { mu: 0.1 };
  store.scenario = { ...store.scenario, id: 'published:steadycom-gut-nine', published: 'steadycom-gut-nine' };
  store.publishedCommunityId = 'steadycom-gut-nine';
  await store.loadState(JSON.stringify(document));
  assert.equal(store.scenario.id, document.scenario);
  assert.equal(store.scenario.published, undefined);
  assert.equal(store.publishedCommunityId, null);
  assert.equal(store.steadyComSupport().kind, 'separate');
  assert.equal(store.solution.steadyCom.mu, 0.1);
});

test('fixed-growth results use Scenario JSON and return to ordinary FBA after a cleared save', async () => {
  const store = await makeStore();
  await store.solveSteadyComAt(0.1);
  const solution = store.solution;
  await assert.rejects(store.reproducibilityBundle(), /Scenario JSON/);
  await assert.rejects(store.methodsProvenanceReport(), /Scenario JSON/);
  assert.equal(store.solution, solution);
  const document = store.stateJSON();
  await store.loadState(document);
  assert.equal(store.solution.steadyCom.mu, 0.1);
  store.clearSolution();
  await store.loadState(store.stateJSON());
  assert.equal(store.solution.steadyCom, undefined);
  assert.equal(store.solution.status, 'optimal');
});
