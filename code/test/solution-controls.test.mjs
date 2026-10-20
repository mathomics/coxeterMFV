import test from 'node:test';
import assert from 'node:assert/strict';
import { Store } from '../src/ui/store.js';
import { Model } from '../src/core/model.js';
import { renderAddReactionPanel } from '../src/ui/add-reaction-panel.js';

// One cell: a is taken up and turned into b by R1 (held at 6) and R3 (the
// remaining 2, as 2 a -> 2 b), with R2 and R4 idle; b feeds biomass.
async function makeStore() {
  const store = new Store('.');
  store.models.set('routes', new Model({
    id: 'routes',
    metabolites: [{ id: 'a_e', compartment: 'e' }, { id: 'a_c', compartment: 'c' }, { id: 'b_c', compartment: 'c' }],
    reactions: [
      { id: 'EX_a_e', metabolites: { a_e: -1 }, lower_bound: -10, upper_bound: 1000 },
      { id: 'At', metabolites: { a_e: -1, a_c: 1 }, lower_bound: 0, upper_bound: 1000 },
      { id: 'R2', metabolites: { a_c: -1, b_c: 1 }, lower_bound: 0, upper_bound: 0 },
      { id: 'R1', metabolites: { a_c: -1, b_c: 1 }, lower_bound: 6, upper_bound: 6 },
      { id: 'R3', metabolites: { a_c: -2, b_c: 2 }, lower_bound: 0, upper_bound: 1000 },
      { id: 'R4', metabolites: { a_c: -1, b_c: 1 }, lower_bound: 0, upper_bound: 0 },
      { id: 'BIOMASS', metabolites: { b_c: -1 }, lower_bound: 0, upper_bound: 1000, objective_coefficient: 1 },
    ],
  }));
  store.scenarios = [{
    id: 'routes', name: 'Routes', objective: 'equal-growth',
    species: [{ id: 'demo', model: 'routes', abundance: 1 }], medium: { a: 10 },
  }];
  await store.useScenario('routes');
  assert.equal(store.solution.status, 'optimal');
  return store;
}

test('Add Reaction lists flux-carrying reactions first, by |flux|, and shows the flux', async () => {
  const store = await makeStore();
  for (const id of ['R1', 'R2', 'R3', 'R4']) store.map.removeReaction(`demo@${id}`);
  const nodeId = store.map.nodesForMetabolite('demo@a_c')[0].id;

  assert.deepEqual(store.reactionsAvailableFromMetabolite(nodeId), ['demo@R1', 'demo@R3', 'demo@R2', 'demo@R4']);
  const html = renderAddReactionPanel({ store, reactionIds: ['demo@R1'], fluxes: store.solution.fluxes });
  assert.match(html, /class="add-reaction-flux"[^>]*>6</, 'the flux is shown in its row');

  store.clearSolution();
  assert.deepEqual(store.reactionsAvailableFromMetabolite(nodeId), ['demo@R2', 'demo@R1', 'demo@R3', 'demo@R4'],
    'without a solution the list keeps the model order');
  assert.doesNotMatch(renderAddReactionPanel({ store, reactionIds: ['demo@R1'] }), /add-reaction-flux/);
});

test('Clear solution empties the solution and holds it through bound edits until an analysis runs', async () => {
  const store = await makeStore();
  assert.equal(store.clearSolution(), true);
  assert.equal(store.solution, null);
  assert.equal(store.growth.size, 0);
  assert.deepEqual(store.crossFeeding.links, []);
  assert.equal(store.fvaRanges.size, 0);

  store.setReactionBounds('demo@R1', 4, 4);
  store.solve();
  store.setMediumLimit('a', 8);
  assert.equal(store.solution, null, 'edits made while cleared do not solve');
  assert.equal(store.clearSolution(), false, 'nothing left to clear');

  await store.runAnalysis('fba');
  assert.equal(store.solution.status, 'optimal');
  assert.ok(Math.abs(store.solution.objectiveValue - 8) < 1e-6, 'the run sees the edits');
  store.setMediumLimit('a', 7);
  assert.ok(Math.abs(store.solution.objectiveValue - 7) < 1e-6, 'and edits re-solve again');
});
