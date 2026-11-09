/** Reproducibility verdicts describe the imported calculation until scientific work changes. */
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import factory from '../assets/vendor/glpk.js';
import { Model } from '../src/core/model.js';
import { Store } from '../src/ui/store.js';
import { initGLPK } from '../src/solver/glpk.js';
import { renderResults } from '../src/ui/panels.js';

test.before(async () => {
  globalThis.__GLPK_FACTORY__ = factory;
  globalThis.__GLPK_WASM__ = await readFile(new URL('../assets/vendor/glpk.wasm', import.meta.url));
  assert.equal(await initGLPK(), true);
});

async function replayedStore(gimme = false) {
  const raw = JSON.parse(await readFile(new URL('../models/ecoli_core_mc.json', import.meta.url), 'utf8'));
  raw.reactions.find(reaction => reaction.id === 'GAPD').gene_reaction_rule = 'test_gap';
  const store = new Store('.');
  store.models.set('ecoli_core_mc', new Model(raw));
  store.scenarios = JSON.parse(await readFile(new URL('../models/scenarios.json', import.meta.url), 'utf8')).scenarios;
  await store.useScenario('solo-ecoli');
  if (gimme) {
    store.importMetatranscriptomics('MAG,gene,condition,TPM\neco,test_gap,Control,1\neco,test_gap,Treatment,10\n');
    store.runCommunityGIMME({ gimmeThreshold: 5, gimmeObjectiveFraction: 0.8 });
  }
  await store.importReproducibilityBundle(await store.reproducibilityBundle());
  assert.equal(store.reproducibilityReplay.replay.pass, true);
  return store;
}

function retired(store) {
  assert.ok(!store.reproducibilityReplay, 'The previous replay verdict was retired.');
  const html = renderResults(store, 'repro');
  assert.match(html, /Import a reproducibility bundle to replay it/);
  assert.doesNotMatch(html, /Reproducibility verified/);
  assert.match(html, /data-analysis-tab="repro"[^>]* disabled/);
}

test('a changed medium retires the imported numerical verdict', async () => {
  const store = await replayedStore();
  const objective = store.solution.objectiveValue;
  store.setMediumLimit('glc__D', 5);
  assert.ok(store.solution.objectiveValue < objective);
  retired(store);
});

test('unsolved bound edits retire the verdict before another result exists', async () => {
  const store = await replayedStore();
  const solution = store.solution;
  store.setReactionBounds('eco@PGI', 0, 0);
  assert.equal(store.solution, solution);
  retired(store);
});

test('clearing the solution retires its imported numerical verdict', async () => {
  const store = await replayedStore();
  store.clearSolution();
  retired(store);
});

test('starting another analysis retires the verdict before the run finishes', async () => {
  const store = await replayedStore();
  const pending = store.runAnalysis('pfba');
  try {
    retired(store);
  } finally {
    await pending;
  }
  assert.equal(store.solution.status, 'optimal');
  retired(store);
});

test('replacing the model retires the imported verdict during the rebuild', async () => {
  const store = await replayedStore();
  const pending = store.useScenario('solo-ecoli');
  try {
    retired(store);
  } finally {
    await pending;
  }
  retired(store);
});

test('changed analysis settings retire the verdict even before they affect a solve', async () => {
  const store = await replayedStore();
  const solution = store.solution;
  store.setPFBABiomassPercent(75);
  assert.equal(store.solution, solution);
  retired(store);
});

test('enabling variability retires the earlier analysis verdict', async () => {
  const store = await replayedStore();
  store.setFVA(true);
  try {
    retired(store);
  } finally {
    store.setFVA(false);
  }
});

test('a no-op setting or rejected bound input preserves the imported verdict', async () => {
  const store = await replayedStore();
  const report = store.reproducibilityReplay;
  store.setPFBABiomassPercent(store.pfbaBiomassPercent);
  store.setReactionBounds('eco@PGI', '', 'invalid');
  assert.ok(store.reproducibilityReplay === report);
});

test('drawing edits and rejected file imports preserve the imported numerical verdict', async () => {
  const store = await replayedStore();
  const report = store.reproducibilityReplay;
  store.showEmptyMap();
  store.setView({ dark: !store.view.dark });
  await assert.rejects(store.loadState('{broken'));
  await assert.rejects(store.importReproducibilityBundle({}));
  assert.equal(store.reproducibilityReplay, report);
  assert.match(renderResults(store, 'repro'), /Reproducibility verified/);
});

test('another valid import installs a fresh verdict after scientific edits', async () => {
  const store = await replayedStore();
  const bundle = await store.reproducibilityBundle();
  const previous = store.reproducibilityReplay;
  store.setMediumLimit('glc__D', 5);
  await store.importReproducibilityBundle(bundle);
  assert.notEqual(store.reproducibilityReplay, previous);
  assert.equal(store.reproducibilityReplay.replay.pass, true);
});

test('omics display choices preserve the verdict and changed GIMME inputs retire it', async () => {
  const store = await replayedStore();
  const report = store.reproducibilityReplay;
  const solution = store.solution;
  store.setCommunityOmics({ enabled: true, mode: 'comparison' });
  assert.ok(store.reproducibilityReplay === report);
  store.setCommunityOmics({ gimmeThreshold: 5 });
  assert.equal(store.solution, solution);
  retired(store);
});

test('replacing expression data retires the imported input verdict', async () => {
  const store = await replayedStore();
  store.importMetatranscriptomics('MAG,gene,condition,TPM\neco,test_gap,Control,1\n');
  retired(store);
});

test('a new GIMME calculation retires its previous imported verdict', async () => {
  const store = await replayedStore(true);
  store.runCommunityGIMME();
  retired(store);
});
