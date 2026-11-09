/**
 * Distinguish archived inputs, failed attempts and optimal numerical results.
 * Reopening a bundle calculates its configured method without inventing a comparison.
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import factory from '../assets/vendor/glpk.js';
import { Model } from '../src/core/model.js';
import { Store } from '../src/ui/store.js';
import { initGLPK } from '../src/solver/glpk.js';
import { compareReplayedSolution } from '../src/core/reproducibility.js';
import { renderResults } from '../src/ui/panels.js';

test.before(async () => {
  globalThis.__GLPK_FACTORY__ = factory;
  globalThis.__GLPK_WASM__ = await readFile(new URL('../assets/vendor/glpk.wasm', import.meta.url));
  assert.equal(await initGLPK(), true);
});

async function makeStore() {
  const raw = JSON.parse(await readFile(new URL('../models/ecoli_core_mc.json', import.meta.url), 'utf8'));
  raw.reactions.find(reaction => reaction.id === 'GAPD').gene_reaction_rule = 'test_gap';
  const store = new Store('.');
  store.models.set('ecoli_core_mc', new Model(raw));
  store.scenarios = JSON.parse(await readFile(new URL('../models/scenarios.json', import.meta.url), 'utf8')).scenarios;
  await store.useScenario('solo-ecoli');
  return store;
}

function gimme(store) {
  store.importMetatranscriptomics('MAG,gene,condition,TPM\neco,test_gap,Control,1\n');
  store.runCommunityGIMME({ gimmeThreshold: 5, gimmeObjectiveFraction: 0.8 });
}

function infeasible(store) {
  store.setMediumLimit('glc__D', 0);
  store.setMediumLimit('o2', 0);
  assert.equal(store.solution.status, 'infeasible');
}

async function finishRanges(store) {
  clearTimeout(store._fvaTimer);
  store._fvaTimer = null;
  await store.runFVA();
}

test('an input-only bundle recalculates without claiming a numerical replay', async () => {
  const store = await makeStore();
  const drawing = store.map.toJSON();
  store.clearSolution();
  const bundle = await store.reproducibilityBundle();
  assert.equal(bundle.solution, null);
  const check = await store.importReproducibilityBundle(bundle);
  assert.equal(check.replay, null);
  assert.equal(store.reproducibilityReplay.replay, null);
  assert.equal(store.solution.status, 'optimal');
  assert.equal(store.status.state, 'ok');
  assert.match(store.status.message, /no archived numerical result to compare/);
  assert.deepEqual(store.map.toJSON(), drawing);
  const html = renderResults(store, 'repro');
  assert.match(html, /No numerical solution was archived/);
  assert.doesNotMatch(html, /Reproducibility verified|Reproducibility warning/);
  assert.doesNotMatch(html, /data-analysis-tab="repro"[^>]* disabled/);
});

test('an input-only GIMME archive excludes retired scores and calculated solver stages', async () => {
  const store = await makeStore();
  gimme(store);
  assert.ok(store.solution.inconsistencyScore > 0);
  store.clearSolution();
  store.setCommunityOmics({ gimmeThreshold: 0 });
  const bundle = await store.reproducibilityBundle();
  assert.equal(bundle.solution, null);
  assert.equal(bundle.analysis.method, 'GIMME');
  assert.equal(bundle.analysis.gimme.threshold, 0);
  assert.equal(bundle.analysis.gimme.inconsistencyScore, null);
  assert.equal(bundle.analysis.gimme.primarySolver, null);
  assert.equal(bundle.analysis.gimme.contextualSolver, null);
});

test('an input-only GIMME bundle recalculates its configured parameters without a false mismatch', async () => {
  const store = await makeStore();
  gimme(store);
  store.clearSolution();
  store.setCommunityOmics({ gimmeThreshold: 0, gimmeObjectiveFraction: 0.6 });
  const bundle = await store.reproducibilityBundle();
  const check = await store.importReproducibilityBundle(bundle);
  assert.equal(check.replay, null);
  assert.equal(store.solution.status, 'optimal');
  assert.equal(store.solution.gimme, true);
  assert.equal(store.solution.threshold, 0);
  assert.equal(store.solution.objectiveFraction, 0.6);
  assert.equal(store.status.state, 'ok');
});

test('an input-only pFVA bundle restores and recomputes its selected ranges', async () => {
  const store = await makeStore();
  store.setPFBABiomassPercent(90);
  await store.runAnalysis('pfva');
  await finishRanges(store);
  store.clearSolution();
  const bundle = await store.reproducibilityBundle();
  const check = await store.importReproducibilityBundle(bundle);
  assert.equal(check.replay, null);
  assert.equal(store.pfvaEnabled, true);
  assert.equal(store.fvaEnabled, true);
  await finishRanges(store);
  assert.equal(store.fvaRanges.size, store.problem.rxnIds.length);
  assert.equal(store.exportMetadata().analysis.method, 'pFVA');
  store.setFVA(false);
});

test('an infeasible bundle reproduces its solver status without a false objective mismatch', async () => {
  const store = await makeStore();
  infeasible(store);
  const bundle = await store.reproducibilityBundle();
  assert.equal(bundle.solution.objectiveValue, null);
  const check = await store.importReproducibilityBundle(bundle);
  assert.equal(check.replay.pass, true);
  assert.equal(check.replay.checks.status, true);
  assert.match(store.status.message, /analysis status reproduced: infeasible/);
  assert.doesNotMatch(store.status.message, /numerical replay OK|numerical differences/);
});

test('the replay view names an infeasible result without verifying nonexistent numerical values', async () => {
  const store = await makeStore();
  infeasible(store);
  await store.importReproducibilityBundle(await store.reproducibilityBundle());
  const html = renderResults(store, 'repro');
  assert.match(html, /Analysis status reproduced/);
  assert.match(html, /Solver status/);
  assert.match(html, /infeasible/);
  assert.match(html, /No optimal solution was obtained/);
  assert.doesNotMatch(html, /Reproducibility verified/);
  assert.doesNotMatch(html, /<strong>Fluxes<\/strong>/);
});

test('Methods describes input-only archives as configured work without completed-analysis claims', async () => {
  for (const method of ['fba', 'pfba', 'cyclefree', 'gimme']) {
    const store = await makeStore();
    if (method === 'gimme') {
      gimme(store);
    } else {
      await store.runAnalysis(method);
    }
    store.clearSolution();
    const report = await store.methodsProvenanceReport();
    assert.match(report.text, /Analysis settings were prepared/, method);
    assert.match(report.text, /Configured method:/, method);
    assert.doesNotMatch(report.text, /Metabolic simulations were performed|CycleFreeFlux .* removed the internal cycles|GIMME used condition/, method);
  }
});

test('Methods distinguishes a failed analysis attempt from an optimal calculation', async () => {
  const store = await makeStore();
  infeasible(store);
  const report = await store.methodsProvenanceReport();
  assert.match(report.text, /An analysis was attempted/);
  assert.match(report.text, /No optimal solution was obtained/);
  assert.doesNotMatch(report.text, /Metabolic simulations were performed/);
});

test('missing optimal objectives still fail comparison while unavailable failed objectives agree', () => {
  const failed = compareReplayedSolution(
    { status: 'infeasible', objectiveValue: null, fluxes: {} },
    { status: 'infeasible', objectiveValue: NaN, fluxes: new Map() },
  );
  assert.equal(failed.pass, true);
  const invalid = compareReplayedSolution(
    { status: 'optimal', objectiveValue: null, fluxes: {} },
    { status: 'optimal', objectiveValue: NaN, fluxes: new Map() },
  );
  assert.equal(invalid.pass, false);
  const differentStatus = compareReplayedSolution(
    { status: 'infeasible', objectiveValue: null, fluxes: {} },
    { status: 'unbounded', objectiveValue: NaN, fluxes: new Map() },
  );
  assert.equal(differentStatus.pass, false);
});
