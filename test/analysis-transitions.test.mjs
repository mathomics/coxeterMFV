/**
 * Analysis choices stay consistent with their numerical results and exports.
 * GIMME uses the bundled E. coli network with two test gene annotations.
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import factory from '../assets/vendor/glpk.js';
import { Model } from '../src/core/model.js';
import { Store } from '../src/ui/store.js';
import { analysisProvenance } from '../src/core/reproducibility.js';
import { reactionOmicsScores } from '../src/omics/overlay.js';
import { runGIMME } from '../src/omics/gimme.js';
import { initGLPK } from '../src/solver/glpk.js';

test.before(async () => {
  globalThis.__GLPK_FACTORY__ = factory;
  globalThis.__GLPK_WASM__ = await readFile(new URL('../assets/vendor/glpk.wasm', import.meta.url));
  assert.equal(await initGLPK(), true);
});

async function makeStore() {
  const raw = JSON.parse(await readFile(new URL('../models/ecoli_core_mc.json', import.meta.url), 'utf8'));
  raw.reactions.find(reaction => reaction.id === 'GAPD').gene_reaction_rule = 'test_gap';
  raw.reactions.find(reaction => reaction.id === 'PGI').gene_reaction_rule = 'test_pgi';
  const store = new Store('.');
  store.models.set('ecoli_core_mc', new Model(raw));
  store.scenarios = JSON.parse(await readFile(new URL('../models/scenarios.json', import.meta.url), 'utf8')).scenarios;
  await store.useScenario('solo-ecoli');
  store.importMetatranscriptomics('MAG,gene,condition,TPM\neco,test_gap,Control,1\neco,test_pgi,Control,8\neco,test_gap,Treatment,8\neco,test_pgi,Treatment,1\n');
  store.setCommunityOmics({ gimmeThreshold: 5, gimmeObjectiveFraction: 0.8 });
  return store;
}

async function finishRanges(store) {
  clearTimeout(store._fvaTimer);
  store._fvaTimer = null;
  await store.runFVA();
}

test('GIMME replaces the previous FBA-family method and its pending ranges', async () => {
  for (const method of ['pfba', 'loopless', 'cyclefree', 'fva', 'pfva']) {
    const store = await makeStore();
    await store.runAnalysis(method);
    const result = store.runCommunityGIMME();
    assert.equal(result.status, 'optimal', method);
    assert.equal(store.omics.gimmeEnabled, true);
    assert.equal(store.parsimonious, false, method);
    assert.equal(store.loopless, false, method);
    assert.equal(store.cycleFree, false, method);
    assert.equal(store.fvaEnabled, false, method);
    assert.equal(store.pfvaEnabled, false, method);
    assert.equal(store.lastRun, null, method);
    assert.equal(store.fvaProgress.running, false, method);
    assert.equal(analysisProvenance(store).method, 'GIMME');
  }
});

test('each ordinary analysis replaces GIMME in the result and exported provenance', async () => {
  for (const [method, archived] of [
    ['fba', 'FBA'], ['pfba', 'pFBA'], ['loopless', 'loopless FBA'],
    ['cyclefree', 'CycleFreeFlux'], ['fva', 'FBA'], ['pfva', 'pFVA'],
  ]) {
    const store = await makeStore();
    store.runCommunityGIMME();
    await store.runAnalysis(method);
    if (store.fvaEnabled) {
      await finishRanges(store);
    }
    assert.equal(store.solution.status, 'optimal', method);
    assert.equal(Boolean(store.solution.gimme), false, method);
    assert.equal(store.omics.gimmeEnabled, false, method);
    assert.equal(store.omics.gimmeResult, null, method);
    assert.equal(analysisProvenance(store).method, archived, method);
    assert.equal((await store.reproducibilityBundle()).analysis.method, archived, method);
    store.setFVA(false);
  }
});

test('Return FBA from GIMME runs ordinary FBA regardless of the earlier method', async () => {
  for (const method of ['pfba', 'loopless', 'cyclefree']) {
    const store = await makeStore();
    const optimum = store.solution.objectiveValue;
    await store.runAnalysis(method);
    store.runCommunityGIMME();
    store.disableGIMME();
    assert.equal(store.omics.gimmeEnabled, false);
    assert.equal(store.parsimonious, false, method);
    assert.equal(store.loopless, false, method);
    assert.equal(store.cycleFree, false, method);
    assert.equal(store.fvaEnabled, false, method);
    assert.equal(analysisProvenance(store).method, 'FBA');
    assert.ok(Math.abs(store.solution.objectiveValue - optimum) < 1e-7);
  }
});

test('medium, bounds and gene edits recompute the selected GIMME analysis', async () => {
  for (const edit of ['medium', 'bounds', 'gene']) {
    const store = await makeStore();
    store.runCommunityGIMME();
    const drawing = JSON.stringify(store.map.toJSON());
    if (edit === 'medium') {
      store.setMediumLimit('glc__D', 5);
    } else if (edit === 'bounds') {
      store.setReactionBounds('eco@PGI', 0, 0);
      store.solve();
    } else {
      store.toggleGeneKnockout('eco@test_pgi');
    }
    const scores = reactionOmicsScores(store.community, store.species, store.models, store.omics.dataset, 'Control');
    const expected = runGIMME(store.problem, scores, { threshold: 5, objectiveFraction: 0.8 });
    assert.equal(store.solution.status, expected.status, edit);
    assert.equal(store.solution.gimme, true, edit);
    assert.equal(store.omics.gimmeEnabled, true, edit);
    assert.ok(Math.abs(store.solution.objectiveValue - expected.objectiveValue) < 1e-7, edit);
    assert.equal(JSON.stringify(store.map.toJSON()), drawing, edit);
  }
});

test('an abundance rebuild recomputes GIMME without replacing the drawing', async () => {
  const store = await makeStore();
  store.runCommunityGIMME();
  const drawing = JSON.stringify(store.map.toJSON());
  await store.setAbundance('eco', 0.5);
  assert.equal(store.solution.gimme, true);
  assert.equal(store.omics.gimmeEnabled, true);
  assert.equal(JSON.stringify(store.map.toJSON()), drawing);
});

test('GIMME supersedes an ordinary analysis that is still starting', async () => {
  const store = await makeStore();
  const pending = store.runAnalysis('pfba');
  const result = store.runCommunityGIMME();
  await pending;
  assert.ok(store.solution === result);
  assert.equal(store.solution.gimme, true);
  assert.equal(store.omics.gimmeEnabled, true);
  assert.equal(store.lastRun, null);
});

test('opening another scenario retires the previous GIMME method', async () => {
  const store = await makeStore();
  store.runCommunityGIMME();
  store.scenarios.push({ ...store.scenario, id: 'second-cell', map: null,
    species: [{ id: 'other', model: 'ecoli_core_mc', abundance: 1 }] });
  await store.useScenario('second-cell');
  assert.equal(store.solution.status, 'optimal');
  assert.equal(Boolean(store.solution.gimme), false);
  assert.equal(store.omics.gimmeEnabled, false);
  assert.equal(store.omics.gimmeResult, null);
  assert.equal(analysisProvenance(store).method, 'FBA');
});

test('GIMME resumes after Clear solution and its bundle replays the same analysis', async () => {
  const store = await makeStore();
  store.runCommunityGIMME();
  store.clearSolution();
  store.runCommunityGIMME();
  assert.equal(store.solutionCleared, false);
  assert.equal(store.solution.gimme, true);
  const bundle = await store.reproducibilityBundle();
  const target = await makeStore();
  const check = await target.importReproducibilityBundle(bundle);
  assert.equal(check.replay.pass, true);
  assert.equal(target.solution.gimme, true);
  assert.equal(target.omics.gimmeEnabled, true);
});

test('an infeasible edit keeps GIMME selected so Undo recomputes the same analysis', async () => {
  const store = await makeStore();
  store.runCommunityGIMME();
  store.setReactionBounds('eco@BIOMASS_Ec_core', 10, 10);
  store.solve();
  assert.equal(store.solution.status, 'infeasible');
  assert.equal(store.solution.gimme, true);
  assert.equal(store.omics.gimmeEnabled, true);
  await store.undoConstraints();
  assert.equal(store.solution.status, 'optimal');
  assert.equal(store.solution.gimme, true);
  assert.equal(store.omics.gimmeEnabled, true);
});

test('direct analysis switches replace GIMME and recompute their own primary solution', async () => {
  for (const mode of ['parsimonious', 'loopless', 'cyclefree', 'fva']) {
    const store = await makeStore();
    store.runCommunityGIMME();
    if (mode === 'parsimonious') {
      store.setParsimonious(true);
    } else if (mode === 'loopless') {
      store.setLoopless(true);
    } else if (mode === 'cyclefree') {
      store.setCycleFree(true);
    } else {
      store.setFVA(true);
      await finishRanges(store);
    }
    assert.equal(store.solution.status, 'optimal', mode);
    assert.equal(Boolean(store.solution.gimme), false, mode);
    assert.equal(store.omics.gimmeEnabled, false, mode);
    assert.equal(store.omics.gimmeResult, null, mode);
    store.setFVA(false);
  }
});

test('a newer GIMME request supersedes an earlier GIMME result', async () => {
  const store = await makeStore();
  store.runCommunityGIMME({ gimmeObjectiveFraction: 0.8 });
  const newer = store.runCommunityGIMME({ gimmeObjectiveFraction: 0.6 });
  assert.ok(store.solution === newer);
  assert.equal(store.solution.objectiveFraction, 0.6);
});

test('GIMME reports a minimised custom objective with its original sign', async () => {
  const store = await makeStore();
  store.setObjectiveCoefficient('eco@BIOMASS_Ec_core', -1);
  store.setObjectiveSense('min');
  const optimum = store.solution.objectiveValue;
  const result = store.runCommunityGIMME({ gimmeObjectiveFraction: 1 });
  const achieved = -result.fluxes.get('eco@BIOMASS_Ec_core');
  assert.equal(result.status, 'optimal');
  assert.ok(Math.abs(result.objectiveValue - achieved) < 1e-7);
  assert.ok(Math.abs(result.objectiveValue - optimum) < 1e-7);
  assert.ok(Math.abs(result.requiredObjective - optimum) < 1e-7);
  const check = await (await makeStore()).importReproducibilityBundle(await store.reproducibilityBundle());
  assert.equal(check.replay.pass, true);
});

test('committed GIMME parameters recompute the active result and replay consistently', async () => {
  const store = await makeStore();
  store.runCommunityGIMME();
  const drawing = JSON.stringify(store.map.toJSON());
  for (const patch of [{ gimmeObjectiveFraction: 0.6 }, { gimmeThreshold: 0 }, { gimmeThreshold: null }]) {
    const before = store.solution;
    store.setCommunityOmics(patch);
    assert.ok(store.solution !== before);
    assert.equal(store.solution.gimme, true);
    assert.equal(store.solution.objectiveFraction, store.omics.gimmeObjectiveFraction);
    const check = await (await makeStore()).importReproducibilityBundle(await store.reproducibilityBundle());
    assert.equal(check.replay.pass, true);
  }
  assert.equal(JSON.stringify(store.map.toJSON()), drawing);
});

test('changing GIMME condition recomputes its fluxes while display settings leave the solve alone', async () => {
  const store = await makeStore();
  store.runCommunityGIMME();
  const before = store.solution;
  store.setCommunityOmics({ conditionA: 'Treatment' });
  assert.ok(store.solution !== before);
  const expected = runGIMME(store.problem,
    reactionOmicsScores(store.community, store.species, store.models, store.omics.dataset, 'Treatment'),
    { threshold: 5, objectiveFraction: 0.8 });
  assert.deepEqual([...store.solution.fluxes], [...expected.fluxes]);
  const solved = store.solution;
  store.setCommunityOmics({ enabled: false, mode: 'compare', conditionB: 'Control', metric: 'difference', pseudocount: 2 });
  assert.ok(store.solution === solved);
  const check = await (await makeStore()).importReproducibilityBundle(await store.reproducibilityBundle());
  assert.equal(check.replay.pass, true);
});

test('replacing active GIMME expression data recomputes the result before bundle export', async () => {
  const store = await makeStore();
  store.runCommunityGIMME();
  const before = store.solution;
  store.importMetatranscriptomics('MAG,gene,condition,TPM\neco,test_gap,New,9\neco,test_pgi,New,1\n', 'replacement.csv');
  assert.ok(store.solution !== before);
  assert.equal(store.solution.gimme, true);
  assert.equal(store.omics.conditionA, 'New');
  const check = await (await makeStore()).importReproducibilityBundle(await store.reproducibilityBundle());
  assert.equal(check.replay.pass, true);
});

test('unmapped replacement data and invalid GIMME settings preserve the active work', async () => {
  const store = await makeStore();
  store.runCommunityGIMME();
  const before = { solution: store.solution, omics: store.omics, state: store.stateJSON() };
  assert.throws(() => store.importMetatranscriptomics('MAG,gene,condition,TPM\nunknown,unmapped,New,4\n'), /GIMME/);
  assert.ok(store.omics === before.omics);
  for (const patch of [
    { conditionA: 'missing' }, { gimmeThreshold: Infinity }, { gimmeThreshold: 'bad' },
    { gimmeObjectiveFraction: -1 }, { gimmeObjectiveFraction: NaN },
  ]) {
    assert.throws(() => store.setCommunityOmics(patch), /GIMME/);
    assert.throws(() => store.runCommunityGIMME(patch), /GIMME/);
    assert.ok(store.omics === before.omics);
    assert.ok(store.solution === before.solution);
    assert.equal(store.stateJSON(), before.state);
  }
});

test('GIMME input edits respect Clear solution until the user runs another analysis', async () => {
  const store = await makeStore();
  store.runCommunityGIMME();
  store.clearSolution();
  store.setCommunityOmics({ gimmeObjectiveFraction: 0.6, conditionA: 'Treatment' });
  store.importMetatranscriptomics('MAG,gene,condition,TPM\neco,test_gap,New,9\neco,test_pgi,New,1\n');
  assert.equal(store.solution, null);
  assert.equal(store.solutionCleared, true);
  store.runCommunityGIMME();
  assert.equal(store.solution.gimme, true);
  assert.equal(store.solution.objectiveFraction, 0.6);
  assert.equal(store.omics.conditionA, 'New');
});

test('an unfinished analysis cannot be exported with an older numerical result', async () => {
  const store = await makeStore();
  const pending = store.runAnalysis('pfba');
  await assert.rejects(store.reproducibilityBundle(), /finish|running|wait/i);
  await pending;
  assert.equal((await store.reproducibilityBundle()).analysis.method, 'pFBA');
});

test('automatic GIMME thresholds survive bundle reload and adapt to replacement expression', async () => {
  const store = await makeStore();
  store.runCommunityGIMME({ gimmeThreshold: null });
  const target = await makeStore();
  await target.importReproducibilityBundle(await store.reproducibilityBundle());
  assert.equal(target.omics.gimmeThreshold, null);
  const cutoff = target.solution.threshold;
  target.importMetatranscriptomics('MAG,gene,condition,TPM\neco,test_gap,New,40\neco,test_pgi,New,90\n');
  assert.equal(target.omics.gimmeThreshold, null);
  assert.ok(target.solution.threshold > cutoff);
  assert.equal(target.solution.gimme, true);
});
