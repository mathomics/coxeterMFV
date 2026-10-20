import test from 'node:test';
import assert from 'node:assert/strict';
import { Store } from '../src/ui/store.js';
import { Model } from '../src/core/model.js';
import { evaluateGeneRule, geneKnockoutReactions } from '../src/core/gene-rules.js';

// One cell: a takes two routes to b (R1, and R2 or R3), b feeds biomass.
async function makeStore() {
  const store = new Store('.');
  store.models.set('gpr-demo', new Model({
    id: 'gpr-demo',
    metabolites: [{ id: 'a_e', compartment: 'e' }, { id: 'a_c', compartment: 'c' }, { id: 'b_c', compartment: 'c' }],
    reactions: [
      { id: 'EX_a_e', metabolites: { a_e: -1 }, lower_bound: -10, upper_bound: 1000 },
      { id: 'At', metabolites: { a_e: -1, a_c: 1 }, lower_bound: 0, upper_bound: 1000, gene_reaction_rule: 'g2 or g3' },
      { id: 'R1', metabolites: { a_c: -1, b_c: 1 }, lower_bound: 0, upper_bound: 1000, gene_reaction_rule: 'g1' },
      { id: 'R2', metabolites: { a_c: -1, b_c: 1 }, lower_bound: 0, upper_bound: 1000, gene_reaction_rule: '(g1 and g4) or (g5 and g6)' },
      { id: 'R3', metabolites: { a_c: -1, b_c: 1 }, lower_bound: 0, upper_bound: 1000, gene_reaction_rule: 'g1 and g7' },
      { id: 'BIOMASS', metabolites: { b_c: -1 }, lower_bound: 0, upper_bound: 1000, objective_coefficient: 1 },
    ],
    genes: ['g1', 'g2', 'g3', 'g4', 'g5', 'g6', 'g7'],
  }));
  store.scenarios = [{
    id: 'gpr-demo', name: 'GPR demo', objective: 'equal-growth',
    species: [{ id: 'demo', model: 'gpr-demo', abundance: 1 }], medium: { a: 10 },
  }];
  await store.useScenario('gpr-demo');
  assert.equal(store.solution.status, 'optimal');
  return store;
}

test('gene rules: and, or, nesting and isozymes', () => {
  const deleted = (...genes) => new Set(genes);
  assert.equal(evaluateGeneRule('', deleted('g1')), true, 'no rule, nothing to lose');
  assert.equal(evaluateGeneRule('g1 or g2', deleted('g1')), true, 'an isozyme is left');
  assert.equal(evaluateGeneRule('g1 or g2', deleted('g1', 'g2')), false);
  assert.equal(evaluateGeneRule('g1 and g2', deleted('g2')), false, 'a complex loses a subunit');
  assert.equal(evaluateGeneRule('(g1 and g4) or (g5 and g6)', deleted('g1')), true);
  assert.equal(evaluateGeneRule('(g1 and g4) or (g5 and g6)', deleted('g1', 'g6')), false);
  assert.equal(evaluateGeneRule('g1 OR (g2 AND (g3 or g4))', deleted('g1', 'g3')), true);
  assert.throws(() => evaluateGeneRule('g1 and (g2', deleted()), /Unbalanced/);
  const { closed, unreadable } = geneKnockoutReactions(
    [['A', 'g1'], ['B', 'g1 or g2'], ['C', 'g1 and g2'], ['D', 'g3'], ['E', 'g1 and (']], new Set(['g1']));
  assert.deepEqual([...closed], [['A', ['g1']], ['C', ['g1']]]);
  assert.deepEqual(unreadable, ['E'], 'an unreadable rule is left open and reported');
});

test('a gene knockout closes exactly the reactions its rules lose, and undo, reset and the scenario file follow it', async () => {
  const store = await makeStore();
  assert.deepEqual(store.geneSearch('g1')[0].closes, ['demo@R1', 'demo@R3'], 'the preview names what it would close');

  store.toggleGeneKnockout('demo@g1');
  assert.deepEqual([...store.geneClosedReactions().keys()], ['demo@R1', 'demo@R3']);
  assert.deepEqual(store.currentReactionBounds('demo@R1'), { lower: 0, upper: 0 });
  assert.equal(store.reactionKnockedOut('demo@R2'), false, 'g5 and g6 still make R2');
  assert.equal(store.solution.status, 'optimal');
  assert.ok(Math.abs(store.solution.fluxes.get('demo@R1')) < 1e-9);
  const row = store.reactionTableRows().find((r) => r.id === 'demo@R3');
  assert.deepEqual([row.status, row.geneKnockouts], ['knocked-out', ['demo@g1']]);
  assert.deepEqual(store.geneKnockoutChanges(), [{ key: 'demo@g1', gene: 'g1', speciesId: 'demo', title: 'g1', reactions: ['demo@R1', 'demo@R3'] }]);
  assert.deepEqual(store.constraintChanges().filter((c) => c.knockedOut).map((c) => c.id), ['demo@R1', 'demo@R3'], 'the map greys them out');

  const growth = store.solution.objectiveValue;
  store.setGeneKnockouts(['demo@g2', 'demo@g3']);
  assert.ok(Math.abs(store.solution.objectiveValue) < 1e-9, 'both transporter isozymes gone: no growth');
  await store.undoConstraints();
  assert.ok(Math.abs(store.solution.objectiveValue - growth) < 1e-9);
  assert.deepEqual([...store.geneKnockouts], ['demo@g1']);

  const file = store.stateJSON();
  assert.deepEqual(JSON.parse(file).geneKnockouts, ['demo@g1']);
  const other = await makeStore();
  await other.loadState(file);
  assert.deepEqual([...other.geneClosedReactions().keys()], ['demo@R1', 'demo@R3']);
  const bundle = await store.reproducibilityBundle();
  assert.deepEqual(bundle.constraints.geneKnockouts, ['demo@g1']);
  const replayed = await (await makeStore()).importReproducibilityBundle(bundle);
  assert.equal(replayed.replay.pass, true, 'the bundle replays with the gene knockout');

  await store.undoConstraints();
  assert.equal(store.reactionKnockedOut('demo@R1'), false, 'undo reopens what the gene closed');
  await store.redoConstraints();
  store.resetAllReactions();
  assert.equal(store.geneKnockouts.size, 0);
  assert.deepEqual(store.constraintChanges(), []);
});
