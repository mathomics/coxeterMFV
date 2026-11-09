/** Reaction controls preserve completed work on no-ops and keep gene closures separate from bounds. */
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import factory from '../assets/vendor/glpk.js';
import { initGLPK } from '../src/solver/glpk.js';
import { Model } from '../src/core/model.js';
import { Store } from '../src/ui/store.js';
import { renderInspector } from '../src/ui/panels.js';

test.before(async () => {
  globalThis.__GLPK_FACTORY__ = factory;
  globalThis.__GLPK_WASM__ = await readFile(new URL('../assets/vendor/glpk.wasm', import.meta.url));
  assert.equal(await initGLPK(), true);
});

async function makeStore() {
  const raw = JSON.parse(await readFile(new URL('../models/ecoli_core_mc.json', import.meta.url), 'utf8'));
  raw.reactions.find(reaction => reaction.id === 'PGI').gene_reaction_rule = 'qa_pgi and qa_partner';
  const store = new Store('.');
  store.models.set('ecoli_core_mc', new Model(raw));
  store.scenarios = JSON.parse(await readFile(new URL('../models/scenarios.json', import.meta.url), 'utf8')).scenarios;
  await store.useScenario('solo-ecoli');
  return store;
}

function unchanged(store, solution, snapshot, replay) {
  assert.equal(store.solution, solution);
  assert.deepEqual(store.constraintSnapshot(), snapshot);
  assert.equal(store.reproducibilityReplay, replay);
}

for (const action of ['bounds', 'reset', 'reset-all', 'medium', 'knockout']) {
  test(`an unchanged ${action} control preserves a completed fixed growth result`, async () => {
    const store = await makeStore();
    if (action === 'knockout') {
      store.knockout('eco@PGI');
    }
    await store.solveSteadyComAt(0.1);
    assert.equal(store.solution.status, 'optimal');
    assert.equal(store.solution.steadyCom.mu, 0.1);
    const solution = store.solution;
    const snapshot = store.constraintSnapshot();
    const replay = { replay: { pass: true } };
    store.reproducibilityReplay = replay;
    if (action === 'bounds') {
      const bounds = store.currentReactionBounds('eco@PGI');
      store.setReactionBounds('eco@PGI', bounds.lower, bounds.upper);
    } else if (action === 'reset') {
      store.resetReaction('eco@PGI');
    } else if (action === 'reset-all') {
      store.resetAllReactions();
    } else if (action === 'medium') {
      store.setMediumLimit('glc__D', store.medium.glc__D);
    } else {
      store.knockout('eco@PGI');
    }
    unchanged(store, solution, snapshot, replay);
  });
}

test('unknown reaction controls cannot add invisible constraints or retire a result', async () => {
  const store = await makeStore();
  await store.solveSteadyComAt(0.1);
  const solution = store.solution;
  const snapshot = store.constraintSnapshot();
  const replay = { replay: { pass: true } };
  store.reproducibilityReplay = replay;
  store.setReactionBounds('eco@missing', -20, 30);
  store.knockout('eco@missing');
  store.resetReaction('eco@missing');
  unchanged(store, solution, snapshot, replay);
});

test('bounds and reaction knockout cannot hide edits underneath a gene closure', async () => {
  const store = await makeStore();
  store.setReactionBounds('eco@PGI', -15, 20);
  store.toggleGeneKnockout('eco@qa_pgi');
  const solution = store.solution;
  const snapshot = store.constraintSnapshot();
  store.setReactionBounds('eco@PGI', -2, 5);
  store.knockout('eco@PGI');
  assert.equal(store.solution, solution);
  assert.deepEqual(store.constraintSnapshot(), snapshot);
  store.toggleGeneKnockout('eco@qa_pgi');
  assert.deepEqual(store.currentReactionBounds('eco@PGI'), { lower: -15, upper: 20 });
});

test('resetting reaction bounds leaves gene knockouts in force and gene restoration reopens the model bounds', async () => {
  const store = await makeStore();
  store.setReactionBounds('eco@PGI', -15, 20);
  store.toggleGeneKnockout('eco@qa_pgi');
  store.resetReaction('eco@PGI');
  assert.equal(store.geneKnockouts.has('eco@qa_pgi'), true);
  assert.equal(store.overrides.has('eco@PGI'), false);
  assert.deepEqual(store.currentReactionBounds('eco@PGI'), { lower: 0, upper: 0 });
  store.toggleGeneKnockout('eco@qa_pgi');
  assert.deepEqual(store.currentReactionBounds('eco@PGI'), store.originalReactionBounds('eco@PGI'));
});

test('a reaction closed by a gene cannot offer an ineffective Restore action', async () => {
  const store = await makeStore();
  store.toggleGeneKnockout('eco@qa_pgi');
  store.select('eco@PGI');
  const html = renderInspector(store);
  const restore = html.match(/<button[^>]*data-action="reset-reaction"[^>]*>Restore<\/button>/)?.[0];
  assert.ok(restore);
  assert.match(restore, /disabled/);
  assert.match(restore, /Restore the gene/);
});

test('real bounds changes still retire fixed growth work and undo restores the input bounds', async () => {
  const store = await makeStore();
  await store.solveSteadyComAt(0.1);
  const original = store.currentReactionBounds('eco@PGI');
  store.setReactionBounds('eco@PGI', 0, 0);
  assert.equal(store.solution, null);
  assert.equal(store.steadyComState, null);
  await store.undoConstraints();
  assert.deepEqual(store.currentReactionBounds('eco@PGI'), original);
  assert.equal(store.solution.status, 'optimal');
});

test('an unchanged bound preserves an imported replay and available Redo', async () => {
  const store = await makeStore();
  store.setReactionBounds('eco@PGI', -15, 20);
  await store.undoConstraints();
  const snapshot = store.constraintSnapshot();
  await store.importReproducibilityBundle(await store.reproducibilityBundle());
  assert.equal(store.reproducibilityReplay.replay.pass, true);
  const solution = store.solution;
  const replay = store.reproducibilityReplay;
  store.setReactionBounds('eco@PGI', null, store.currentReactionBounds('eco@PGI').upper);
  unchanged(store, solution, snapshot, replay);
  store.setReactionBounds('eco@PGI', -15, 20);
  await store.undoConstraints();
  store.setReactionBounds('eco@PGI', null, store.currentReactionBounds('eco@PGI').upper);
  assert.equal(store.canRedoConstraints, true);
});

test('gene closures and underlying bounds survive Scenario and reproducibility round trips', async () => {
  const store = await makeStore();
  store.setReactionBounds('eco@PGI', -15, 20);
  store.toggleGeneKnockout('eco@qa_pgi');
  const snapshot = store.constraintSnapshot();
  await store.loadState(store.stateJSON());
  assert.deepEqual(store.constraintSnapshot(), snapshot);
  await store.importReproducibilityBundle(await store.reproducibilityBundle());
  assert.deepEqual(store.constraintSnapshot(), snapshot);
  assert.equal(store.reproducibilityReplay.replay.pass, true);
  store.toggleGeneKnockout('eco@qa_pgi');
  assert.deepEqual(store.currentReactionBounds('eco@PGI'), { lower: -15, upper: 20 });
});

test('held-out species retain earlier reaction bounds and refuse hidden bound controls', async () => {
  const store = await makeStore();
  const scenario = structuredClone(store.scenario);
  scenario.id = 'reaction-constraints-pair';
  scenario.species.push({ id: 'other', model: 'ecoli_core_mc', abundance: 0.5 });
  scenario.species[0].abundance = 0.5;
  store.scenarios.push(scenario);
  await store.useScenario(scenario.id);
  store.setReactionBounds('eco@PGI', -15, 20);
  await store.setSpeciesEnabled('eco', false);
  const snapshot = store.constraintSnapshot();
  const solution = store.solution;
  store.setReactionBounds('eco@PGI', -2, 5);
  store.knockout('eco@PGI');
  store.resetReaction('eco@PGI');
  assert.deepEqual(store.constraintSnapshot(), snapshot);
  assert.equal(store.solution, solution);
  await store.setSpeciesEnabled('eco', true);
  assert.deepEqual(store.currentReactionBounds('eco@PGI'), { lower: -15, upper: 20 });
  assert.deepEqual(store.currentReactionBounds('other@PGI'), store.originalReactionBounds('other@PGI'));
  store.toggleGeneKnockout('eco@qa_pgi');
  assert.equal(store.geneClosedReactions().has('eco@PGI'), true);
  assert.equal(store.geneClosedReactions().has('other@PGI'), false);
});
