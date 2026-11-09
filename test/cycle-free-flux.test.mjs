import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { Model } from '../src/core/model.js';
import { FluxProblem } from '../src/solver/fba.js';
import { Store } from '../src/ui/store.js';

// A -> B -> C -> A, all irreversible, beside the pathway A -> P that carries the objective.
function cycleBesidePathway() {
  return new Model({
    id: 'cycle_beside_pathway',
    compartments: { c: 'cytosol' },
    metabolites: ['a_c', 'b_c', 'c_c', 'p_c'].map((id) => ({ id, compartment: 'c' })),
    reactions: [
      { id: 'EX_a', metabolites: { a_c: -1 }, lower_bound: -10, upper_bound: 1000 },
      { id: 'R_AB', metabolites: { a_c: -1, b_c: 1 }, lower_bound: 0, upper_bound: 1000 },
      { id: 'R_BC', metabolites: { b_c: -1, c_c: 1 }, lower_bound: 0, upper_bound: 1000 },
      { id: 'R_CA', metabolites: { c_c: -1, a_c: 1 }, lower_bound: 0, upper_bound: 1000 },
      { id: 'R_AP', metabolites: { a_c: -1, p_c: 1 }, lower_bound: 0, upper_bound: 1000 },
      { id: 'DM_p', metabolites: { p_c: -1 }, lower_bound: 0, upper_bound: 1000, objective_coefficient: 1 },
    ],
  });
}

test('CycleFreeFlux removes the cycle an FBA optimum carries, keeping objective and exchanges', () => {
  const problem = new FluxProblem(cycleBesidePathway());
  const fba = problem.optimize();
  // The same optimum with 50 running round the cycle: as good an FBA answer.
  const looped = new Map(fba.fluxes);
  for (const id of ['R_AB', 'R_BC', 'R_CA']) looped.set(id, looped.get(id) + 50);

  const free = problem.cycleFree({ fluxes: looped });
  assert.equal(free.status, 'optimal');
  assert.ok(Math.abs(free.objectiveValue - 10) < 1e-9);
  for (const id of ['R_AB', 'R_BC', 'R_CA']) assert.equal(free.fluxes.get(id), 0, id);
  for (const id of ['EX_a', 'DM_p']) assert.ok(Math.abs(free.fluxes.get(id) - looped.get(id)) < 1e-9, id);
  assert.ok(Math.abs(free.internalFluxBefore - 160) < 1e-9 && Math.abs(free.internalFlux - 10) < 1e-9);
});

test('CycleFreeFlux on E. coli core keeps the FBA objective and exchanges and never adds internal flux', async () => {
  const model = new Model(JSON.parse(await readFile(new URL('../models/ecoli_core_mc.json', import.meta.url), 'utf8')));
  const problem = new FluxProblem(model);
  const fba = problem.optimize();
  const free = problem.cycleFree();
  assert.equal(free.status, 'optimal');
  assert.ok(Math.abs(free.objectiveValue - fba.objectiveValue) < 1e-7);
  for (const [id, rxn] of model.reactions) {
    if (rxn.kind === 'metabolic' || rxn.kind === 'transport') continue;
    assert.ok(Math.abs(free.fluxes.get(id) - fba.fluxes.get(id)) < 1e-7, id);
  }
  assert.ok(free.internalFlux <= free.internalFluxBefore + 1e-7);
  assert.deepEqual([...free.shadowPrices], [...fba.shadowPrices], 'the duals of the FBA solved first');
});

test('CycleFreeFlux is run from the Simulate menu like the other methods and saved with the scenario', async () => {
  const store = new Store('.');
  let calls = 0;
  const result = () => ({ status: 'optimal', objectiveValue: 1, fluxes: new Map([['R', 1]]), shadowPrices: new Map(), iterations: 1 });
  store.problem = { optimize: result, parsimonious: result, loopless: result, cycleFree() { calls += 1; return result(); } };
  store.community = { crossFeeding() { return { links: [], perMetabolite: [] }; }, growthRates() { return new Map(); } };
  store.parsimonious = true;

  await store.runAnalysis('cyclefree');
  assert.equal(calls, 1);
  assert.equal(store.lastRun.label, 'CycleFreeFlux');
  assert.equal(store.lastRun.phase, 'done');
  assert.deepEqual([store.cycleFree, store.parsimonious], [true, false]);
  assert.equal(JSON.parse(store.stateJSON()).cycleFree, true);

  store.setLoopless(true);
  assert.equal(store.cycleFree, false, 'Loopless takes its place');
});
