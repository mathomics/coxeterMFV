/**
 * SteadyCom cancellation and request ordering preserve the current community result.
 * Numerical tasks use the bundled E. coli network.
 */
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
  const raw = JSON.parse(await readFile(new URL('../models/ecoli_core_mc.json', import.meta.url), 'utf8'));
  store.models.set('ecoli_core_mc', new Model(raw));
  store.scenarios = JSON.parse(await readFile(new URL('../models/scenarios.json', import.meta.url), 'utf8')).scenarios;
  await store.useScenario('solo-ecoli');
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

function holdFailingPreparation(store) {
  let started;
  let release;
  const ready = new Promise(resolve => {
    started = resolve;
  });
  const held = new Promise(resolve => {
    release = resolve;
  });
  store.steadyComProblem = async () => {
    started();
    await held;
    throw new Error('The earlier model preparation failed.');
  };
  return { ready, release };
}

test('cancelling a range retains the ordinary solution and immediately permits its export', async () => {
  const store = await makeStore();
  const held = holdNextTask(store);
  const pending = store.steadyComRange();
  const rejection = assert.rejects(pending, { name: 'AbortError' });
  await held.ready;
  const solution = store.solution;
  const drawing = store.map.toJSON();
  store.cancelSteadyCom();
  assert.equal(store._steadyComController, null);
  assert.equal(store.status.state, 'ok');
  assert.equal((await store.reproducibilityBundle()).analysis.method, 'FBA');
  held.release();
  await rejection;
  assert.equal(store.solution, solution);
  assert.deepEqual(store.map.toJSON(), drawing);
  assert.equal(store.steadyComState.max, null);
});

test('cancelling plain and parsimonious fixed growth retains the installed composition and saved analysis', async () => {
  for (const parsimonious of [false, true]) {
    const store = await makeStore();
    await store.solveSteadyComAt(0.1);
    store.markScenarioSaved();
    const solution = store.solution;
    const composition = store.species.map(species => species.abundance);
    store.setSteadyComParsimonious(parsimonious);
    const held = holdNextTask(store);
    const pending = store.solveSteadyComAt(0.2);
    const rejection = assert.rejects(pending, { name: 'AbortError' });
    await held.ready;
    store.cancelSteadyCom();
    const document = JSON.parse(store.stateJSON());
    assert.equal(document.steadyCom.mu, 0.1);
    assert.equal(store.status.state, 'ok');
    held.release();
    await rejection;
    assert.equal(store.solution, solution);
    assert.deepEqual(store.species.map(species => species.abundance), composition);
    assert.equal(store.solution.steadyCom.parsimonious, false);
  }
});

test('a preparation failure after cancellation is retired as an abort for both range and fixed growth', async () => {
  for (const method of ['range', 'fixed']) {
    const store = await makeStore();
    if (method === 'fixed') {
      await store.steadyComRange();
    }
    const held = holdFailingPreparation(store);
    const pending = method === 'range' ? store.steadyComRange() : store.solveSteadyComAt(0.1);
    const rejection = assert.rejects(pending, { name: 'AbortError' });
    await held.ready;
    store.cancelSteadyCom();
    const status = { ...store.status };
    held.release();
    await rejection;
    assert.deepEqual(store.status, status);
  }
});

test('a preparation failure from an older context is retired without presenting an error for the newer scenario', async () => {
  for (const method of ['range', 'fixed']) {
    const store = await makeStore();
    if (method === 'fixed') {
      await store.steadyComRange();
    }
    const held = holdFailingPreparation(store);
    const pending = method === 'range' ? store.steadyComRange() : store.solveSteadyComAt(0.1);
    const rejection = assert.rejects(pending, { name: 'AbortError' });
    await held.ready;
    await store.useScenario('solo-ecoli');
    const solution = store.solution;
    const status = { ...store.status };
    held.release();
    await rejection;
    assert.equal(store.solution, solution);
    assert.deepEqual(store.status, status);
  }
});

test('a newer fixed growth request supersedes an ordinary analysis still waiting to paint', async () => {
  for (const cached of [false, true]) {
    for (const method of ['fba', 'pfba', 'loopless', 'cyclefree', 'fva', 'pfva']) {
      const store = await makeStore();
      if (cached) {
        await store.steadyComRange();
      }
      const older = store.runAnalysis(method);
      const newer = store.solveSteadyComAt(0.1);
      await Promise.all([older, newer]);
      assert.equal(store.solution.steadyCom?.mu, 0.1, `${method}, cached range: ${cached}`);
      assert.equal(store.lastRun, null, method);
      assert.equal(store._steadyComController, null);
      store.setFVA(false);
    }
  }
});

test('a current preparation failure remains visible and keeps the installed solution', async () => {
  for (const method of ['range', 'fixed']) {
    const store = await makeStore();
    if (method === 'fixed') {
      await store.steadyComRange();
    }
    const solution = store.solution;
    const held = holdFailingPreparation(store);
    const pending = method === 'range' ? store.steadyComRange() : store.solveSteadyComAt(0.1);
    const rejection = assert.rejects(pending, /earlier model preparation failed/);
    await held.ready;
    held.release();
    await rejection;
    assert.equal(store.status.state, 'error');
    assert.match(store.status.message, /earlier model preparation failed/);
    assert.equal(store.solution, solution);
    assert.equal(store._steadyComController, null);
  }
});

test('cancelling a replacement fixed-growth request preserves previously completed FVA and pFVA ranges', async () => {
  for (const method of ['fva', 'pfva']) {
    const store = await makeStore();
    await store.steadyComRange();
    await store.runAnalysis(method);
    clearTimeout(store._fvaTimer);
    store._fvaTimer = null;
    await store.runFVA();
    const solution = store.solution;
    const ranges = new Map(store.fvaRanges);
    const held = holdNextTask(store);
    const pending = store.solveSteadyComAt(0.1);
    const rejection = assert.rejects(pending, { name: 'AbortError' });
    await held.ready;
    store.cancelSteadyCom();
    const bundle = await store.reproducibilityBundle();
    assert.equal(bundle.analysis.method, method === 'pfva' ? 'pFVA' : 'FBA');
    assert.equal(bundle.analysis.fvaEnabled, true);
    assert.equal(Object.keys(bundle.solution.fva).length, ranges.size);
    held.release();
    await rejection;
    assert.equal(store.solution, solution);
    assert.deepEqual(store.fvaRanges, ranges);
  }
});

test('an invalid growth request leaves a current range request running and able to finish', async () => {
  const store = await makeStore();
  const held = holdNextTask(store);
  const pending = store.steadyComRange();
  await held.ready;
  const controller = store._steadyComController;
  await assert.rejects(store.solveSteadyComAt(-1), /finite growth rate/);
  assert.equal(store._steadyComController, controller);
  assert.equal(controller.signal.aborted, false);
  held.release();
  const state = await pending;
  assert.ok(state.max > 0.8);
  assert.equal(store.status.state, 'ok');
});
