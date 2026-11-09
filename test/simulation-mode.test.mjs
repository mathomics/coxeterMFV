import test from 'node:test';
import assert from 'node:assert/strict';
import { Store } from '../src/ui/store.js';
import { Model } from '../src/core/model.js';
import { FluxProblem } from '../src/solver/fba.js';

function optimal(value) {
  return { status: 'optimal', objectiveValue: value, fluxes: new Map([['R', value]]), shadowPrices: new Map(), iterations: 1, message: '' };
}

function mockStore() {
  const store = new Store('.');
  let fbaCalls = 0;
  let pfbaCalls = 0;
  const pfbaFractions = [];
  store.problem = {
    optimize() { fbaCalls += 1; return optimal(1); },
    parsimonious(opts = {}) {
      pfbaCalls += 1;
      pfbaFractions.push(opts.fraction);
      return optimal(opts.fraction ?? 1);
    },
  };
  store.community = {
    crossFeeding() { return { links: [], perMetabolite: [] }; },
    growthRates() { return new Map(); },
  };
  return { store, calls: () => ({ fbaCalls, pfbaCalls }), pfbaFractions };
}

test('simulation defaults to standard FBA', () => {
  const { store, calls } = mockStore();
  assert.equal(store.parsimonious, false);
  store.solve();
  assert.deepEqual(calls(), { fbaCalls: 1, pfbaCalls: 0 });
});

test('checking pFBA switches solves to parsimonious FBA', () => {
  const { store, calls } = mockStore();
  store.setParsimonious(true);
  assert.equal(store.parsimonious, true);
  assert.deepEqual(calls(), { fbaCalls: 0, pfbaCalls: 1 });
  store.solve();
  assert.deepEqual(calls(), { fbaCalls: 0, pfbaCalls: 2 });
});

test('unchecking pFBA returns to standard FBA', () => {
  const { store, calls } = mockStore();
  store.setParsimonious(true);
  store.setParsimonious(false);
  assert.equal(store.parsimonious, false);
  assert.deepEqual(calls(), { fbaCalls: 1, pfbaCalls: 1 });
});

test('scenario JSON persists the selected mode', () => {
  const { store } = mockStore();
  store.parsimonious = true;
  const doc = JSON.parse(store.stateJSON());
  assert.equal(doc.parsimonious, true);
});


test('pFBA defaults to 100 percent of the primary optimum', () => {
  const { store, pfbaFractions } = mockStore();
  assert.equal(store.pfbaBiomassPercent, 100);
  store.setParsimonious(true);
  assert.equal(pfbaFractions.at(-1), 1);
});

test('user can set pFBA biomass target as a percentage', () => {
  const { store, pfbaFractions } = mockStore();
  store.setParsimonious(true);
  store.setPFBABiomassPercent(95);
  assert.equal(store.pfbaBiomassPercent, 95);
  assert.equal(pfbaFractions.at(-1), 0.95);

  store.setPFBABiomassPercent(87.5);
  assert.equal(store.pfbaBiomassPercent, 87.5);
  assert.equal(pfbaFractions.at(-1), 0.875);
});

test('pFBA biomass percentage is clamped to 0-100 and ignores temporary blank input', () => {
  const { store } = mockStore();
  store.setPFBABiomassPercent(120);
  assert.equal(store.pfbaBiomassPercent, 100);
  store.setPFBABiomassPercent(-5);
  assert.equal(store.pfbaBiomassPercent, 0);
  store.setPFBABiomassPercent('');
  assert.equal(store.pfbaBiomassPercent, 0);
});

test('scenario JSON persists the pFBA biomass target', () => {
  const { store } = mockStore();
  store.pfbaBiomassPercent = 92.5;
  const doc = JSON.parse(store.stateJSON());
  assert.equal(doc.pfbaBiomassPercent, 92.5);
});


test('real pFBA solver fixes the requested fraction of optimum', () => {
  const model = new Model({
    id: 'pfba_fraction_demo',
    compartments: { c: 'cytosol' },
    metabolites: [{ id: 'a_c', compartment: 'c' }],
    reactions: [
      {
        id: 'EX_a',
        metabolites: { a_c: -1 },
        lower_bound: -10,
        upper_bound: 0,
      },
      {
        id: 'BIOMASS',
        metabolites: { a_c: -1 },
        lower_bound: 0,
        upper_bound: 1000,
        objective_coefficient: 1,
      },
    ],
  });
  const problem = new FluxProblem(model);
  const fba = problem.optimize();
  const pfba90 = problem.parsimonious({ fraction: 0.9 });

  assert.equal(fba.status, 'optimal');
  assert.ok(Math.abs(fba.objectiveValue - 10) < 1e-8);
  assert.equal(pfba90.status, 'optimal');
  assert.ok(Math.abs(pfba90.objectiveValue - 9) < 1e-8);
});

test('running an analysis from the panel says when it has finished', async () => {
  const { store, calls } = mockStore();
  const seen = [];
  store.subscribe((s) => seen.push(s.lastRun?.phase));
  const run = store.runAnalysis('pfba');
  assert.equal(store.lastRun.phase, 'running');
  assert.equal(store.status.state, 'busy');
  assert.match(store.status.message, /Running pFBA/);
  await run;
  assert.deepEqual(calls(), { fbaCalls: 0, pfbaCalls: 1 });
  assert.equal(store.lastRun.phase, 'done');
  assert.equal(store.lastRun.outcome, 'optimal');
  assert.ok(Number.isFinite(store.lastRun.ms));
  assert.equal(store.status.state, 'ok');
  assert.match(store.status.message, /^pFBA completed · objective 1\.00000 · /);
  assert.ok(seen.includes('running') && seen.at(-1) === 'done');
  assert.ok(store.log.some((entry) => /pFBA completed/.test(entry.message)));
});

test('an analysis that fails is reported as not finished', async () => {
  const { store } = mockStore();
  store.problem.optimize = () => ({ status: 'infeasible', objectiveValue: NaN, fluxes: new Map(), message: 'No feasible point.' });
  await store.runAnalysis('fba');
  assert.equal(store.lastRun.phase, 'failed');
  assert.equal(store.lastRun.outcome, 'infeasible');
  assert.equal(store.status.state, 'error');
  assert.match(store.status.message, /^FBA did not finish: No feasible point\./);
});

test('an unknown analysis is refused', async () => {
  const { store } = mockStore();
  await assert.rejects(store.runAnalysis('sampling'), /Unknown analysis/);
});
