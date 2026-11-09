/**
 * Reports and figure metadata describe the numerical result being shown.
 * Configuring another analysis must not relabel an older result during its run.
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import factory from '../assets/vendor/glpk.js';
import { Model } from '../src/core/model.js';
import { FluxProblem } from '../src/solver/fba.js';
import { Store } from '../src/ui/store.js';
import { initGLPK } from '../src/solver/glpk.js';

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
  return store.runCommunityGIMME({ gimmeThreshold: 5, gimmeObjectiveFraction: 0.8 });
}

function minimiseMaintenance(store) {
  store.setObjectiveCoefficient('eco@BIOMASS_Ec_core', 0);
  store.setObjectiveCoefficient('eco@ATPM', 1);
  store.setObjectiveSense('min');
}

test('FBA, pFBA, CycleFreeFlux and Loopless results record their actual backend', async () => {
  const store = await makeStore();
  const model = store.models.get('ecoli_core_mc');
  for (const backend of ['simplex', 'glpk']) {
    const problem = new FluxProblem(model, { backend });
    for (const method of ['optimize', 'parsimonious', 'cycleFree']) {
      const result = problem[method]();
      assert.equal(result.status, 'optimal');
      assert.equal(result.solver, backend, method);
    }
  }
  assert.equal(new FluxProblem(model, { backend: 'glpk' }).loopless().solver, 'glpk');
});

test('bundle and report solver names belong to the result rather than a later backend choice', async () => {
  const store = await makeStore();
  store.problem.backendChoice = 'simplex';
  store.solve();
  const solved = store.solution;
  store.problem.backendChoice = 'glpk';
  const report = await store.methodsProvenanceReport();
  assert.ok(store.solution === solved);
  assert.equal(report.bundle.solution.solver, 'simplex');
  assert.equal(report.bundle.analysis.solver, 'Local simplex.js');
  assert.match(report.text, /Solver: Local simplex\.js\./);
});

test('GIMME provenance names the primary and contextual solvers independently', async () => {
  const store = await makeStore();
  const result = gimme(store);
  assert.equal(result.solver, 'simplex');
  assert.equal(result.primarySolver, 'glpk');
  const report = await store.methodsProvenanceReport();
  assert.equal(report.bundle.analysis.solver, 'Local simplex.js');
  assert.equal(report.bundle.analysis.gimme.primarySolver, 'GLPK (WebAssembly)');
  assert.equal(report.bundle.analysis.gimme.contextualSolver, 'Local simplex.js');
  assert.match(report.text, /primary objective: GLPK \(WebAssembly\)/);
  assert.match(report.text, /penalty minimisation: Local simplex\.js/);
});

test('figure metadata identifies a GIMME result and its solver', async () => {
  const store = await makeStore();
  gimme(store);
  const metadata = store.exportMetadata();
  const bundle = await store.reproducibilityBundle();
  assert.equal(metadata.analysis.method, 'GIMME');
  assert.equal(metadata.analysis.solver, bundle.analysis.solver);
  assert.equal(metadata.analysis.objectiveValue, store.solution.objectiveValue);
});

test('figure metadata keeps the displayed method while another analysis is starting', async () => {
  const store = await makeStore();
  gimme(store);
  const value = store.solution.objectiveValue;
  const pending = store.runAnalysis('pfba');
  try {
    const metadata = store.exportMetadata();
    assert.equal(metadata.analysis.method, 'GIMME');
    assert.equal(metadata.analysis.objectiveValue, value);
  } finally {
    await pending;
  }
  assert.equal(store.exportMetadata().analysis.method, 'pFBA');
});

test('Methods reports and figures name the custom objective actually minimised', async () => {
  const store = await makeStore();
  minimiseMaintenance(store);
  const report = await store.methodsProvenanceReport();
  assert.match(report.text, /Custom objective \(min\): 1 eco@ATPM\./);
  assert.doesNotMatch(report.text, /Community objective: sum-biomass\./);
  const metadata = store.exportMetadata();
  assert.equal(metadata.analysis.objectiveSense, 'min');
  assert.deepEqual(metadata.analysis.customObjective, { sense: 'min', coefficients: { 'eco@ATPM': 1 } });
  assert.ok(Math.abs(metadata.analysis.objectiveValue - store.solution.fluxes.get('eco@ATPM')) < 1e-7);
});

test('figure metadata keeps the solved objective while a replacement model is rebuilding', async () => {
  const store = await makeStore();
  minimiseMaintenance(store);
  const value = store.solution.objectiveValue;
  const pending = store.setObjective('equal-growth');
  try {
    const metadata = store.exportMetadata();
    assert.equal(metadata.analysis.objectiveSense, 'min');
    assert.deepEqual(metadata.analysis.customObjective, { sense: 'min', coefficients: { 'eco@ATPM': 1 } });
    assert.equal(metadata.analysis.objectiveValue, value);
  } finally {
    await pending;
  }
  assert.equal(store.exportMetadata().analysis.customObjective, null);
});

test('product-objective figure metadata preserves the solved target during a rebuild', async () => {
  const store = await makeStore();
  await store.setObjective('product', 'ac');
  assert.equal(store.exportMetadata().analysis.productTarget, 'ac');
  const pending = store.setObjective('product', 'succ');
  try {
    assert.equal(store.exportMetadata().analysis.productTarget, 'ac');
  } finally {
    await pending;
  }
  assert.equal(store.exportMetadata().analysis.productTarget, 'succ');
});

test('Methods reports distinguish disabled models from simulated species', async () => {
  const store = await makeStore();
  store.scenarios.push({ ...store.scenario, id: 'held-cell', map: null, species: [
    { id: 'eco', model: 'ecoli_core_mc', abundance: 0.5, enabled: true },
    { id: 'other', model: 'ecoli_core_mc', abundance: 0.5, enabled: false },
  ] });
  await store.useScenario('held-cell');
  const report = await store.methodsProvenanceReport();
  assert.match(report.text, /- eco:.*; enabled;/);
  assert.match(report.text, /- other:.*; disabled;/);
});

test('cleared figure metadata does not claim that a numerical analysis is present', async () => {
  const store = await makeStore();
  gimme(store);
  store.clearSolution();
  const analysis = store.exportMetadata().analysis;
  assert.equal(analysis.method, null);
  assert.equal(analysis.solver, null);
  assert.equal(analysis.status, null);
  assert.equal(analysis.objectiveValue, null);
});

test('completed pFVA figure metadata agrees with its bundle analysis', async () => {
  const store = await makeStore();
  await store.runAnalysis('pfva');
  clearTimeout(store._fvaTimer);
  store._fvaTimer = null;
  await store.runFVA();
  const metadata = store.exportMetadata();
  const bundle = await store.reproducibilityBundle();
  assert.equal(metadata.analysis.method, bundle.analysis.method);
  assert.equal(metadata.analysis.method, 'pFVA');
  assert.equal(metadata.analysis.solver, bundle.analysis.solver);
});
