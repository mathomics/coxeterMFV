import test from 'node:test';
import assert from 'node:assert/strict';
import { Model } from '../src/core/model.js';
import { FluxProblem } from '../src/solver/fba.js';
import { Store } from '../src/ui/store.js';

function forcedCycleModel() {
  return new Model({
    id: 'forced_cycle',
    compartments: { c: 'cytosol' },
    metabolites: [
      { id: 'a_c', compartment: 'c' },
      { id: 'b_c', compartment: 'c' },
    ],
    reactions: [
      {
        id: 'R1',
        metabolites: { a_c: -1, b_c: 1 },
        lower_bound: 1,
        upper_bound: 1000,
        objective_coefficient: 1,
      },
      {
        id: 'R2',
        metabolites: { b_c: -1, a_c: 1 },
        lower_bound: 1,
        upper_bound: 1000,
      },
    ],
  });
}

function feasiblePathwayModel() {
  return new Model({
    id: 'pathway',
    compartments: { c: 'cytosol' },
    metabolites: [
      { id: 'a_c', compartment: 'c' },
      { id: 'b_c', compartment: 'c' },
    ],
    reactions: [
      {
        id: 'EX_a',
        metabolites: { a_c: -1 },
        lower_bound: -10,
        upper_bound: 1000,
      },
      {
        id: 'R1',
        metabolites: { a_c: -1, b_c: 1 },
        lower_bound: 0,
        upper_bound: 1000,
      },
      {
        id: 'DM_b',
        metabolites: { b_c: -1 },
        lower_bound: 0,
        upper_bound: 1000,
        objective_coefficient: 1,
      },
    ],
  });
}

test('ordinary FBA permits a forced internal stoichiometric cycle', () => {
  const problem = new FluxProblem(forcedCycleModel());
  const result = problem.optimize();
  assert.equal(result.status, 'optimal');
  assert.ok(result.objectiveValue > 0);
});

test('loopless FBA rejects a forced thermodynamically impossible cycle', () => {
  const problem = new FluxProblem(forcedCycleModel());
  const result = problem.loopless({ nodeLimit: 1000, timeLimitMs: 2000 });
  assert.equal(result.status, 'infeasible');
});

test('loopless FBA preserves a feasible acyclic pathway optimum', () => {
  const problem = new FluxProblem(feasiblePathwayModel());
  const ordinary = problem.optimize();
  const loopless = problem.loopless({ nodeLimit: 1000, timeLimitMs: 2000 });

  assert.equal(ordinary.status, 'optimal');
  assert.equal(loopless.status, 'optimal');
  assert.ok(Math.abs(loopless.objectiveValue - ordinary.objectiveValue) < 1e-7);
  assert.equal(loopless.provenOptimal, true);
});

test('reversible internal reactions create MILP binary direction variables', () => {
  const model = feasiblePathwayModel();
  const r = model.reactions.get('R1');
  r.lowerBound = -1000;
  model.finalize();

  const problem = new FluxProblem(model);
  const result = problem.loopless({ nodeLimit: 1000, timeLimitMs: 2000 });
  assert.equal(result.status, 'optimal');
  assert.equal(result.binaryVariables, 1);
});

function mockStore() {
  const store = new Store('.');
  let normal = 0;
  let pfba = 0;
  let loopless = 0;

  const result = () => ({
    status: 'optimal',
    objectiveValue: 1,
    fluxes: new Map([['R', 1]]),
    shadowPrices: new Map(),
    iterations: 1,
  });

  store.problem = {
    optimize() { normal += 1; return result(); },
    parsimonious() { pfba += 1; return result(); },
    loopless() {
      loopless += 1;
      return { ...result(), nodes: 1, binaryVariables: 0, provenOptimal: true };
    },
  };
  store.community = {
    crossFeeding() { return { links: [], perMetabolite: [] }; },
    growthRates() { return new Map(); },
  };
  return { store, calls: () => ({ normal, pfba, loopless }) };
}

test('Loopless checkbox switches Store solves to loopless MILP', () => {
  const { store, calls } = mockStore();
  store.setLoopless(true);
  assert.equal(store.loopless, true);
  assert.equal(store.parsimonious, false);
  assert.deepEqual(calls(), { normal: 0, pfba: 0, loopless: 1 });
});

test('Loopless is mutually exclusive with pFBA and FVA', () => {
  const { store } = mockStore();
  store.parsimonious = true;
  store.fvaEnabled = true;
  store.fvaRanges = new Map([['R', { min: 0, max: 1 }]]);

  store.setLoopless(true);
  assert.equal(store.loopless, true);
  assert.equal(store.parsimonious, false);
  assert.equal(store.fvaEnabled, false);

  store.setParsimonious(true);
  assert.equal(store.loopless, false);
  assert.equal(store.parsimonious, true);
});

test('scenario JSON persists loopless mode', () => {
  const { store } = mockStore();
  store.loopless = true;
  const doc = JSON.parse(store.stateJSON());
  assert.equal(doc.loopless, true);
});
