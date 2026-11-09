/**
 * Exercise signed objectives and secondary optimizations on the bundled E. coli
 * network. A requested relaxation must stay feasible and match its flux ranges.
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import factory from '../assets/vendor/glpk.js';
import { Model } from '../src/core/model.js';
import { FluxProblem } from '../src/solver/fba.js';
import { initGLPK } from '../src/solver/glpk.js';
import { runGIMME } from '../src/omics/gimme.js';
import { Store } from '../src/ui/store.js';

test.before(async () => {
  globalThis.__GLPK_FACTORY__ = factory;
  globalThis.__GLPK_WASM__ = await readFile(new URL('../assets/vendor/glpk.wasm', import.meta.url));
  assert.equal(await initGLPK(), true);
});

async function model() {
  const raw = JSON.parse(await readFile(new URL('../models/ecoli_core_mc.json', import.meta.url), 'utf8'));
  raw.reactions.find(reaction => reaction.id === 'GAPD').gene_reaction_rule = 'test_gap';
  return new Model(raw);
}

async function makeStore() {
  const store = new Store('.');
  store.models.set('ecoli_core_mc', await model());
  store.scenarios = JSON.parse(await readFile(new URL('../models/scenarios.json', import.meta.url), 'utf8')).scenarios;
  await store.useScenario('solo-ecoli');
  return store;
}

function objective(store, id, coefficient, sense) {
  for (const reaction of Object.keys(store.objectiveCoefficients())) {
    store.setObjectiveCoefficient(reaction, 0);
  }
  store.setObjectiveCoefficient(id, coefficient);
  store.setObjectiveSense(sense);
}

async function finishRanges(store) {
  clearTimeout(store._fvaTimer);
  store._fvaTimer = null;
  await store.runFVA();
}

function checkRanges(store) {
  assert.equal(store.lastRun.phase, 'done');
  assert.equal(store.fvaRanges.size, store.problem.rxnIds.length);
  for (const [id, range] of store.fvaRanges) {
    const flux = store.solution.fluxes.get(id);
    assert.ok(Number.isFinite(range.min) && Number.isFinite(range.max), id);
    assert.ok(flux >= range.min - 1e-5 && flux <= range.max + 1e-5,
      `${id}: ${flux} outside [${range.min}, ${range.max}]`);
  }
}

test('pFBA relaxes a negative maximized objective in the feasible direction', async () => {
  for (const backend of ['simplex', 'glpk']) {
    const problem = new FluxProblem(await model(), { backend, objective: { ATPM: -1 } });
    const result = problem.parsimonious({ fraction: 0.9 });
    assert.equal(result.status, 'optimal', backend);
    assert.ok(Number.isFinite(result.totalFlux), backend);
    assert.ok(Math.abs(result.objectiveValue + 9.229) < 1e-6, backend);
    assert.ok(Math.abs(result.fluxes.get('ATPM') - 9.229) < 1e-6, backend);
    const band = problem.parsimonious({ fraction: 0.9, exactObjective: false });
    assert.ok(Number.isFinite(band.totalFlux), backend);
    assert.ok(Math.abs(band.objectiveValue + 8.39) < 1e-6, backend);
    problem.setObjective({ ATPM: 1 });
    const minimizedBand = problem.parsimonious({ fraction: 0.9, maximize: false, exactObjective: false });
    assert.ok(Number.isFinite(minimizedBand.totalFlux), backend);
    assert.ok(Math.abs(minimizedBand.objectiveValue - 8.39) < 1e-6, backend);
  }
});

test('pFBA relaxes a negative minimized objective in the feasible direction', async () => {
  for (const backend of ['simplex', 'glpk']) {
    const problem = new FluxProblem(await model(), { backend, objective: { BIOMASS_Ec_core: -1 } });
    const first = problem.optimize({ maximize: false });
    const result = problem.parsimonious({ fraction: 0.9, maximize: false });
    assert.equal(result.status, 'optimal', backend);
    assert.ok(Number.isFinite(result.totalFlux), backend);
    assert.ok(Math.abs(result.objectiveValue - 0.9 * first.objectiveValue) < 1e-6, backend);
    assert.ok(result.objectiveValue > first.objectiveValue, backend);
  }
});

test('Store pFBA honors positive and negative custom objectives in either sense', async () => {
  for (const [id, coefficient, sense, multiplier] of [
    ['eco@BIOMASS_Ec_core', 1, 'max', 0.9],
    ['eco@ATPM', -1, 'max', 1.1],
    ['eco@ATPM', 1, 'min', 1.1],
    ['eco@BIOMASS_Ec_core', -1, 'min', 0.9],
  ]) {
    const store = await makeStore();
    objective(store, id, coefficient, sense);
    const first = store.solution.objectiveValue;
    store.setPFBABiomassPercent(90);
    await store.runAnalysis('pfba');
    assert.equal(store.lastRun.phase, 'done', `${id} ${sense}`);
    assert.ok(Number.isFinite(store.solution.totalFlux), `${id} ${sense}`);
    assert.ok(Math.abs(store.solution.objectiveValue - multiplier * first) < 1e-6, `${id} ${sense}`);
    assert.ok(Math.abs(store.solution.objectiveValue - coefficient * store.solution.fluxes.get(id)) < 1e-6);
    assert.equal(store.exportMetadata().analysis.method, 'pFBA');
  }
});

test('GIMME retains feasibility when its unconstrained maximum is negative', async () => {
  const problem = new FluxProblem(await model(), { objective: { ATPM: -1 } });
  const result = runGIMME(problem, new Map([['GAPD', 1]]), { threshold: 5, objectiveFraction: 0.9 });
  assert.equal(result.status, 'optimal');
  assert.ok(Math.abs(result.requiredObjective + 9.229) < 1e-6);
  assert.ok(result.objectiveValue >= result.requiredObjective - 1e-6);
  assert.ok(Math.abs(result.objectiveValue + result.fluxes.get('ATPM')) < 1e-6);
});

test('Store GIMME relaxes a positive minimized objective and replays it', async () => {
  const store = await makeStore();
  objective(store, 'eco@ATPM', 1, 'min');
  store.importMetatranscriptomics('MAG,gene,condition,TPM\neco,test_gap,Control,1\n');
  const result = store.runCommunityGIMME({ gimmeThreshold: 5, gimmeObjectiveFraction: 0.9 });
  assert.equal(result.status, 'optimal');
  assert.ok(Math.abs(result.requiredObjective - 9.229) < 1e-6);
  assert.ok(result.objectiveValue <= result.requiredObjective + 1e-6);
  const report = await store.methodsProvenanceReport();
  assert.match(report.text, /Objective retention allows deterioration/);
  const bundle = report.bundle;
  await store.importReproducibilityBundle(bundle);
  assert.equal(store.reproducibilityReplay.replay.pass, true);
});

test('pFVA displays a solution inside its own ranges for a positive minimized objective', async () => {
  const store = await makeStore();
  objective(store, 'eco@ATPM', 1, 'min');
  store.setPFBABiomassPercent(90);
  await store.runAnalysis('pfva');
  await finishRanges(store);
  checkRanges(store);
  assert.ok(Math.abs(store.solution.objectiveValue - 8.39) < 1e-6);
  const bundle = await store.reproducibilityBundle();
  await store.importReproducibilityBundle(bundle);
  assert.equal(store.reproducibilityReplay.replay.pass, true);
});

test('pFVA displays a parsimonious solution inside its own ranges for a negative maximum', async () => {
  const store = await makeStore();
  objective(store, 'eco@ATPM', -1, 'max');
  store.setPFBABiomassPercent(90);
  await store.runAnalysis('pfva');
  await finishRanges(store);
  assert.ok(Number.isFinite(store.solution.totalFlux));
  checkRanges(store);
  assert.ok(Math.abs(store.solution.objectiveValue + 8.39) < 1e-6);
});

test('ordinary FVA keeps a valid relaxation band for a negative maximum', async () => {
  const problem = new FluxProblem(await model(), { objective: { ATPM: -1 } });
  for (const fraction of [0, 0.9, 1]) {
    const range = problem.variability(['ATPM'], { fraction }).get('ATPM');
    assert.ok(Math.abs(range.min - 8.39) < 1e-6);
    assert.ok(Math.abs(range.max - (2 - fraction) * 8.39) < 1e-6);
  }
});

test('an unreachable exact pFBA target reports its FBA fallback', async () => {
  const problem = new FluxProblem(await model());
  problem.setBounds('BIOMASS_Ec_core', 0.5, 0.5);
  const result = problem.parsimonious({ fraction: 0.9 });
  assert.equal(result.status, 'optimal');
  assert.ok(Math.abs(result.objectiveValue - 0.5) < 1e-6);
  assert.equal(result.totalFlux, undefined);
  assert.match(result.message, /pFBA could not.*FBA solution is shown/);
});

test('an unreachable pFBA run stays visibly failed until a feasible request recovers', async () => {
  const store = await makeStore();
  store.setReactionBounds('eco@BIOMASS_Ec_core', 0.5, 0.5);
  store.setPFBABiomassPercent(90);
  store.setParsimonious(true);
  assert.equal(store.status.state, 'warn');
  assert.match(store.status.message, /FBA solution is shown/);
  await store.runAnalysis('pfba');
  assert.equal(store.lastRun.phase, 'failed');
  assert.equal(store.status.state, 'error');
  assert.match(store.status.message, /pFBA did not finish:.*FBA solution is shown/);
  assert.equal((await store.reproducibilityBundle()).analysis.method, 'FBA');
  store.setPFBABiomassPercent(100);
  await store.runAnalysis('pfba');
  assert.equal(store.lastRun.phase, 'done');
  assert.ok(Number.isFinite(store.solution.totalFlux));
});

test('a zero objective remains valid for pFBA and GIMME fractions', async () => {
  const problem = new FluxProblem(await model());
  problem.setObjective({});
  for (const fraction of [0, 0.9, 1]) {
    const result = problem.parsimonious({ fraction });
    assert.equal(result.status, 'optimal');
    assert.ok(Number.isFinite(result.totalFlux));
    assert.equal(result.objectiveValue, 0);
    const gimme = runGIMME(problem, new Map([['GAPD', 1]]), { threshold: 5, objectiveFraction: fraction });
    assert.equal(gimme.status, 'optimal');
    assert.equal(gimme.requiredObjective, 0);
  }
});

test('invalid pFBA fractions report a fallback without changing the problem bounds', async () => {
  const problem = new FluxProblem(await model());
  const before = problem.snapshotBounds();
  for (const fraction of [NaN, Infinity, -0.1, 1.1]) {
    const result = problem.parsimonious({ fraction });
    assert.equal(result.status, 'optimal');
    assert.equal(result.totalFlux, undefined);
    assert.match(result.message, /pFBA objective fraction must be between 0 and 1/);
    assert.deepEqual(problem.snapshotBounds(), before);
  }
});
