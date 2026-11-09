/** Model creation validates scientific inputs before changing an open calculation or drawing. */
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import factory from '../assets/vendor/glpk.js';
import { initGLPK } from '../src/solver/glpk.js';
import { Model } from '../src/core/model.js';
import { addReactionToModel, geneIdsFromGPR } from '../src/core/model-edit.js';
import { geneRuleGenes } from '../src/core/gene-rules.js';
import { Store } from '../src/ui/store.js';
import { renderWorkspace } from '../src/ui/panels.js';

test.before(async () => {
  globalThis.__GLPK_FACTORY__ = factory;
  globalThis.__GLPK_WASM__ = await readFile(new URL('../assets/vendor/glpk.wasm', import.meta.url));
  assert.equal(await initGLPK(), true);
});

async function makeStore() {
  const raw = JSON.parse(await readFile(new URL('../models/ecoli_core_mc.json', import.meta.url), 'utf8'));
  const store = new Store('.');
  store.models.set('ecoli_core_mc', new Model(raw));
  store.scenarios = JSON.parse(await readFile(new URL('../models/scenarios.json', import.meta.url), 'utf8')).scenarios;
  await store.useScenario('solo-ecoli');
  return store;
}

function reactionSpec() {
  return {
    id: 'QA_PGI',
    name: 'QA parallel glucose isomerase',
    metabolites: 'g6p_c:-1, f6p_c:1',
    lowerBound: -10,
    upperBound: 10,
    geneReactionRule: 'qa_a and (qa_b or qa_c)',
  };
}

function savedWork(store) {
  return {
    model: JSON.stringify(store.modelEditorModel().toJSON()),
    drawing: JSON.stringify(store.map.toJSON()),
    problem: store.problem,
    solution: store.solution,
    work: store.unsavedWork(),
    history: store.mapHistory.version,
  };
}

function preserved(store, before) {
  assert.equal(JSON.stringify(store.modelEditorModel().toJSON()), before.model);
  assert.equal(JSON.stringify(store.map.toJSON()), before.drawing);
  assert.equal(store.problem, before.problem);
  assert.equal(store.solution, before.solution);
  assert.deepEqual(store.unsavedWork(), before.work);
  assert.equal(store.mapHistory.version, before.history);
}

test('cleared or nonnumeric required bounds cannot silently create a zero bound', async () => {
  const store = await makeStore();
  const before = savedWork(store);
  for (const side of ['lowerBound', 'upperBound']) {
    for (const value of ['', '  ', null, undefined, true, [], [1], {}]) {
      const spec = reactionSpec();
      spec[side] = value;
      await assert.rejects(store.createModelReaction(spec), /bounds must be numbers/);
      preserved(store, before);
    }
  }
});

test('malformed new gene rules are rejected before reactions or genes are inserted', async () => {
  const store = await makeStore();
  const before = savedWork(store);
  for (const rule of ['qa_a and (qa_b', 'qa_a or', 'qa_a qa_b', 'and qa_a', 'qa_a and ()', 'not qa_a']) {
    const spec = reactionSpec();
    spec.geneReactionRule = rule;
    await assert.rejects(store.createModelReaction(spec), /gene rule/i);
    preserved(store, before);
  }
});

test('empty and spaced metabolite identifiers report English validation errors', async () => {
  const store = await makeStore();
  const before = savedWork(store);
  for (const id of ['', 'qa metabolite']) {
    await assert.rejects(store.createModelMetabolite({ id, name: 'QA metabolite', compartment: 'c' }), /metabolite ID/);
    preserved(store, before);
  }
});

test('gene metadata agrees with knockout parsing for every accepted gene identifier', async () => {
  const store = await makeStore();
  const spec = reactionSpec();
  spec.geneReactionRule = 'not or (true and false)';
  const model = store.modelEditorModel();
  addReactionToModel(model, spec);
  const genes = geneRuleGenes(spec.geneReactionRule);
  assert.deepEqual(geneIdsFromGPR(spec.geneReactionRule), genes);
  for (const id of genes) {
    assert.ok(model.genes.some(gene => gene.id === id), id);
  }
});

test('other invalid creation fields preserve the installed calculation and drawing', async () => {
  const store = await makeStore();
  const before = savedWork(store);
  const badReactions = [
    { id: 'PGI' },
    { metabolites: 'missing_c:-1, f6p_c:1' },
    { metabolites: 'g6p_c:-1, g6p_c:1' },
    { metabolites: 'g6p_c:0, f6p_c:1' },
    { lowerBound: 20, upperBound: 10 },
  ];
  for (const fields of badReactions) {
    await assert.rejects(store.createModelReaction({ ...reactionSpec(), ...fields }));
    preserved(store, before);
  }
  const badMetabolites = [
    { id: 'g6p_c' },
    { compartment: 'missing' },
    { name: '' },
    { charge: 'not a charge' },
  ];
  for (const fields of badMetabolites) {
    await assert.rejects(store.createModelMetabolite({
      id: 'qa_metabolite_c', name: 'QA metabolite', compartment: 'c', ...fields,
    }));
    preserved(store, before);
  }
});

test('valid creation reaches the model and LP while map Undo leaves the biological edit intact', async () => {
  const store = await makeStore();
  const result = await store.createModelReaction(reactionSpec(), { addToMap: true });
  assert.equal(result.mergedId, 'eco@QA_PGI');
  assert.ok(result.mapResult);
  assert.equal(store.problem.rxnIndex.has(result.mergedId), true);
  assert.ok(store.map.reactions.has(result.mergedId));
  assert.equal(store.solution.status, 'optimal');
  const biology = JSON.stringify(store.modelEditorModel().toJSON());
  store.undoMap();
  assert.equal(store.map.reactions.has(result.mergedId), false);
  assert.equal(store.problem.rxnIndex.has(result.mergedId), true);
  assert.deepEqual(store.modelEditorModel().toJSON(), JSON.parse(biology));
  store.redoMap();
  assert.ok(store.map.reactions.has(result.mergedId));
  store.toggleGeneKnockout('eco@qa_b');
  assert.equal(store.geneClosedReactions().has(result.mergedId), false);
  store.toggleGeneKnockout('eco@qa_c');
  assert.equal(store.geneClosedReactions().has(result.mergedId), true);
  await store.importReproducibilityBundle(await store.reproducibilityBundle());
  assert.equal(store.reproducibilityReplay.replay.pass, true);
  assert.equal(store.geneClosedReactions().has(result.mergedId), true);
  const exported = store.modelEditorModel().toJSON();
  const original = JSON.parse(biology);
  assert.deepEqual(exported.reactions, original.reactions);
  assert.deepEqual(exported.metabolites, original.metabolites);
  assert.deepEqual(exported.genes.map(gene => [gene.id, gene.name]), original.genes.map(gene => [gene.id, gene.name]));
});

test('the model overview counts GPR genes in each species including undrawn reactions', async () => {
  const store = await makeStore();
  await store.createModelReaction(reactionSpec());
  assert.equal(store.map.reactions.has('eco@QA_PGI'), false);
  assert.match(renderWorkspace(store, 'model'), /Genes \(GPR\)<\/dt><dd>3<\/dd>/);
  const scenario = structuredClone(store.scenario);
  scenario.id = 'model-editor-pair';
  scenario.species.push({ id: 'other', model: 'ecoli_core_mc', abundance: 0.5 });
  scenario.species[0].abundance = 0.5;
  store.scenarios.push(scenario);
  await store.useScenario(scenario.id);
  assert.match(renderWorkspace(store, 'model'), /Genes \(GPR\)<\/dt><dd>6<\/dd>/);
});
