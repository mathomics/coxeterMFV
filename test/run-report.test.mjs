/**
 * Analysis reports describe the current run; later solves keep earlier reports
 * in the log without presenting them as results for a new model or medium.
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
  return store;
}

test('a new solve retires the previous run report while keeping it in the log', async () => {
  const store = await makeStore();
  await store.runAnalysis('fba');
  assert.equal(store.lastRun.phase, 'done');
  const objective = store.solution.objectiveValue;
  store.setMediumLimit('glc__D', 5);
  assert.equal(store.lastRun, null);
  assert.equal(store.solution.status, 'optimal');
  assert.ok(store.solution.objectiveValue > 0 && store.solution.objectiveValue < objective);
  assert.ok(store.log.some((entry) => entry.message.startsWith('FBA completed')));
});

test('opening a scenario retires the previous run report', async () => {
  const store = await makeStore();
  await store.runAnalysis('fba');
  assert.equal(store.lastRun.phase, 'done');
  await store.useScenario('solo-ecoli');
  assert.equal(store.lastRun, null);
  assert.equal(store.solution.status, 'optimal');
});

test('opening a scenario cancels an analysis that is still waiting to start', async () => {
  const store = await makeStore();
  const pending = store.runAnalysis('pfba');
  await store.useScenario('solo-ecoli');
  await pending;
  assert.equal(store.lastRun, null);
  assert.equal(store.parsimonious, false);
  assert.equal(store.solution.status, 'optimal');
});
