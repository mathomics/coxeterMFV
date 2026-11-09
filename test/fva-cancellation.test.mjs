/**
 * Interrupted variability runs retain their primary solution without claiming complete ranges.
 * The bundled E. coli network supplies the numerical results.
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import factory from '../assets/vendor/glpk.js';
import { Model } from '../src/core/model.js';
import { Store } from '../src/ui/store.js';
import { initGLPK } from '../src/solver/glpk.js';
import { renderWorkspace } from '../src/ui/panels.js';

test.before(async () => {
  globalThis.__GLPK_FACTORY__ = factory;
  globalThis.__GLPK_WASM__ = await readFile(new URL('../assets/vendor/glpk.wasm', import.meta.url));
  assert.equal(await initGLPK(), true);
});

async function makeStore() {
  const model = JSON.parse(await readFile(new URL('../models/ecoli_core_mc.json', import.meta.url), 'utf8'));
  const store = new Store('.');
  store.models.set('ecoli_core_mc', new Model(model));
  store.scenarios = JSON.parse(await readFile(new URL('../models/scenarios.json', import.meta.url), 'utf8')).scenarios;
  await store.useScenario('solo-ecoli');
  return store;
}

async function startRanges(store, method) {
  await store.runAnalysis(method);
  clearTimeout(store._fvaTimer);
  store._fvaTimer = null;
  assert.equal(store.fvaProgress.running, true);
}

test('cancelling queued FVA or pFVA immediately finishes the report and permits the primary result export', async () => {
  for (const method of ['fva', 'pfva']) {
    const store = await makeStore();
    await startRanges(store, method);
    const solution = store.solution;
    store.cancelFVA();
    assert.equal(store.lastRun.phase, 'cancelled', method);
    assert.equal(store.status.state, 'ok', method);
    assert.equal(store.solution, solution);
    const bundle = await store.reproducibilityBundle();
    assert.equal(bundle.analysis.method, method === 'pfva' ? 'pFBA' : 'FBA');
    assert.equal(bundle.analysis.fvaEnabled, false);
    assert.equal(bundle.solution.fva, null);
  }
});

test('cancelling partial variability preserves visible ranges without archiving them as a completed analysis', async () => {
  for (const method of ['fva', 'pfva']) {
    const store = await makeStore();
    await startRanges(store, method);
    const solution = store.solution;
    const variability = store.problem.variabilityAsync.bind(store.problem);
    store.problem.variabilityAsync = (ids, options) => variability(ids, {
      ...options,
      onProgress: (partial, done, total) => {
        options.onProgress(partial, done, total);
        store.cancelFVA();
      },
    });
    await store.runFVA();
    assert.ok(store.fvaRanges.size > 0);
    assert.ok(store.fvaRanges.size < store.problem.rxnIds.length);
    assert.equal(store.solution, solution);
    assert.equal(store.lastRun.phase, 'cancelled');
    const bundle = await store.reproducibilityBundle();
    assert.equal(bundle.analysis.method, method === 'pfva' ? 'pFBA' : 'FBA');
    assert.equal(bundle.analysis.fvaEnabled, false);
    assert.equal(bundle.solution.fva, null);
    const target = await makeStore();
    await target.importReproducibilityBundle(bundle);
    assert.equal(target.fvaEnabled, false);
    assert.equal(target.fvaRanges.size, 0);
    assert.equal((await target.reproducibilityBundle()).analysis.method, bundle.analysis.method);
  }
});

test('incomplete variability results fail their report and export only the completed primary solution', async () => {
  for (const method of ['fva', 'pfva']) {
    const store = await makeStore();
    await startRanges(store, method);
    const variability = store.problem.variabilityAsync.bind(store.problem);
    store.problem.variabilityAsync = (ids, options) => variability(ids.slice(0, 20), {
      ...options,
      onProgress: (partial, done) => {
        options.onProgress(partial, done, ids.length);
      },
    });
    await store.runFVA();
    assert.equal(store.fvaRanges.size, 20);
    assert.equal(store.lastRun.phase, 'failed');
    const bundle = await store.reproducibilityBundle();
    assert.equal(bundle.analysis.method, method === 'pfva' ? 'pFBA' : 'FBA');
    assert.equal(bundle.analysis.fvaEnabled, false);
    assert.equal(bundle.solution.fva, null);
  }
});

test('complete FVA and pFVA still export and restore every computed range', async () => {
  for (const method of ['fva', 'pfva']) {
    const store = await makeStore();
    await startRanges(store, method);
    await store.runFVA();
    assert.equal(store.lastRun.phase, 'done');
    const bundle = await store.reproducibilityBundle();
    assert.equal(bundle.analysis.method, method === 'pfva' ? 'pFVA' : 'FBA');
    assert.equal(bundle.analysis.fvaEnabled, true);
    assert.equal(Object.keys(bundle.solution.fva).length, store.problem.rxnIds.length);
    const target = await makeStore();
    await target.importReproducibilityBundle(bundle);
    assert.deepEqual(target.fvaRanges, store.fvaRanges);
    assert.equal((await target.reproducibilityBundle()).analysis.method, bundle.analysis.method);
  }
});

test('a partial variability count is labelled incomplete in the Scenario workspace', async () => {
  const store = await makeStore();
  await startRanges(store, 'fva');
  await store.runFVA();
  store.fvaRanges = new Map([...store.fvaRanges].slice(0, 20));
  store.fvaProgress.done = 20;
  const html = renderWorkspace(store, 'scenario');
  assert.match(html, /FVA incomplete:/);
  assert.doesNotMatch(html, /FVA done:/);
  assert.match(html, new RegExp(`20/${store.problem.rxnIds.length}`));
});

test('a primary-only pFVA archive replays its objective interval rather than an exact pFBA target', async () => {
  for (const [coefficient, sense] of [[1, 'min'], [-1, 'max']]) {
    const store = await makeStore();
    for (const id of Object.keys(store.objectiveCoefficients())) {
      store.setObjectiveCoefficient(id, 0);
    }
    store.setObjectiveCoefficient('eco@ATPM', coefficient);
    store.setObjectiveSense(sense);
    store.setPFBABiomassPercent(90);
    await startRanges(store, 'pfva');
    const variability = store.problem.variabilityAsync.bind(store.problem);
    store.problem.variabilityAsync = (ids, options) => variability(ids.slice(0, 20), options);
    await store.runFVA();
    const objective = store.solution.objectiveValue;
    assert.ok(Math.abs(objective - coefficient * 8.39) < 1e-6);
    const bundle = await store.reproducibilityBundle();
    const target = await makeStore();
    await target.importReproducibilityBundle(bundle);
    assert.ok(Math.abs(target.solution.objectiveValue - objective) < 1e-6, `${sense}: ${target.solution.objectiveValue}`);
    assert.equal(bundle.analysis.method, 'pFBA');
    assert.equal(bundle.analysis.pfbaExactObjective, false);
    const replayed = await target.methodsProvenanceReport();
    assert.equal(replayed.bundle.analysis.pfbaExactObjective, false);
    assert.match(replayed.text, /pFBA minimized total absolute flux within/);
    assert.doesNotMatch(replayed.text, /pFBA fixed the primary objective/);
  }
});
