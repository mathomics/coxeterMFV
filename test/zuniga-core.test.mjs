/** Synthetic fixtures authored for tests; no upstream models or datasets. */
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import {
  evaluateZunigaGeneRule, convertZunigaMatModel, canonicalZunigaModel,
  createZunigaScenario, restoreZunigaScenario, solveZunigaScenario,
  loadZunigaStudy,
} from '../src/communities/zuniga.js';
import { Model } from '../src/core/model.js';
import { initGLPK } from '../src/solver/glpk.js';

const EC_BIOMASS = 'BIOMASS_Ec_iJO1366_core_53p95M';
const canonicalEcBiomass = 'Ec@' + EC_BIOMASS;
globalThis.__GLPK_FACTORY__ = (await import('../assets/vendor/glpk.js')).default;
globalThis.__GLPK_WASM__ = new Uint8Array(await readFile(new URL('../assets/vendor/glpk.wasm', import.meta.url)));
assert.equal(await initGLPK(), true);

function sparseMatrix(mets, reactions) {
  const ir = [], jc = [0], values = [];
  const rows = new Map(mets.map((id, i) => [id, i]));
  for (const reaction of reactions) {
    const entries = Object.entries(reaction.metabolites).sort((a, b) => rows.get(a[0]) - rows.get(b[0]));
    for (const [mid, coefficient] of entries) {
      ir.push(rows.get(mid)); values.push(coefficient);
    }
    jc.push(values.length);
  }
  return { ir: Uint32Array.from(ir), jc: Uint32Array.from(jc), values: Float64Array.from(values), shape: [mets.length, reactions.length] };
}

function fixtureRaw(coculture = false) {
  const suffix = coculture ? '_D' : '';
  const food = 'food_c' + suffix, pre = 'pre_c' + suffix, phe = 'phe__L_c' + suffix;
  const mets = [food, pre, phe];
  const r = (id, metabolites, lb = 0, ub = 1000, gpr = '', objective = 0) => ({
    id: id + suffix, metabolites, lb, ub, gpr, objective,
  });
  const reactions = [
    r('SOURCE', { [food]: 1 }, 0, 3),
    r('CHORM', { [food]: -1, [pre]: 1 }, 0, 1000, '(b2599 or isozyme)'),
    r('PPNDH', { [pre]: -1, [phe]: 1 }, 0, 1000, 'b2599'),
    r(EC_BIOMASS, { [phe]: -1 }, 0, 1000, '', 1),
    r('BROKEN_TEXT', { [food]: -1 }, 0, 0, '((other)'),
  ];
  if (coculture) {
    mets.push('phe__L_c_A', 'phe__L_[smp]');
    reactions.push(
      { id: 'SOURCE_A', metabolites: { phe__L_c_A: 1 }, lb: 0, ub: 4, gpr: '', objective: 0 },
      { id: 'BOF_A', metabolites: { phe__L_c_A: -1 }, lb: 1, ub: 1, gpr: '', objective: 1 },
      { id: 'PHELtex_A', metabolites: { phe__L_c_A: 1, 'phe__L_[smp]': -1 }, lb: -1000, ub: 1000, gpr: '', objective: 0 },
      { id: 'PHEtex_D', metabolites: { 'phe__L_[smp]': -1, phe__L_c_D: 1 }, lb: -1000, ub: 1000, gpr: '', objective: 0 },
      { id: 'EX_phe__L_[smp]', metabolites: { 'phe__L_[smp]': -1 }, lb: 0, ub: 0, gpr: '', objective: 0 },
    );
  }
  return {
    rxns: reactions.map(r => r.id), mets, genes: ['b2599', 'isozyme', 'other'],
    rxnNames: reactions.map(r => r.id), metNames: mets,
    lb: Float64Array.from(reactions.map(r => r.lb)),
    ub: Float64Array.from(reactions.map(r => r.ub)),
    c: Float64Array.from(reactions.map(r => r.objective)),
    b: Float64Array.from(mets.map(() => 0)), csense: 'E'.repeat(mets.length),
    grRules: reactions.map(r => r.gpr),
    rules: reactions.map(r => r.id.startsWith('BROKEN_TEXT') ? 'x(3)' : ''),
    S: sparseMatrix(mets, reactions), description: 'Own synthetic validation fixture',
  };
}

function fixtureStudy() {
  const mono = convertZunigaMatModel(fixtureRaw(), { id: 'toy-mono' });
  const joint = convertZunigaMatModel(fixtureRaw(true), { id: 'toy-co' });
  const se = { id: 'toy-se', metabolites: [{ id: 'phe__L_c', compartment: 'c' }],
    reactions: [{ id: 'SOURCE', metabolites: { phe__L_c: 1 }, lower_bound: 0, upper_bound: 4 }] };
  return { sourceModels: { mono, joint }, models: { Ec: new Model(mono), Se: new Model(se) },
    sourceHashes: { ownFixture: 'not-an-upstream-model' } };
}

test('GPR knockout preserves alternative isozymes and parses nested AND/OR without skipping terms', () => {
  const deleted = new Set(['b2599']);
  assert.equal(evaluateZunigaGeneRule('b2599', deleted), false);
  assert.equal(evaluateZunigaGeneRule('b2599 or isozyme', deleted), true);
  assert.equal(evaluateZunigaGeneRule('(b2599 and other) or (isozyme and other)', deleted), true);
  assert.equal(evaluateZunigaGeneRule('b2599 and (isozyme or other)', deleted), false);
  assert.equal(evaluateZunigaGeneRule('', deleted), true);
  for (const rule of ['b2599 or', '(b2599', 'b2599 )', 'b2599 other', 'and b2599']) {
    assert.throws(() => evaluateZunigaGeneRule(rule, deleted));
  }
});

test('MAT converter preserves complete sparse coefficients and uses the valid indexed original GPR', () => {
  const raw = fixtureRaw(true);
  const doc = convertZunigaMatModel(raw);
  assert.equal(doc.metabolites.length, raw.mets.length);
  assert.equal(doc.reactions.length, raw.rxns.length);
  assert.equal(doc.genes.length, raw.genes.length);
  const rule = doc.reactions.find(r => r.id === 'BROKEN_TEXT_D');
  assert.equal(rule.gene_reaction_rule, 'other');
  assert.equal(rule.notes.original_gr_rule, '((other)');
  assert.equal(rule.notes.original_rules, 'x(3)');
  assert.deepEqual(doc.reactions[2].metabolites, { pre_c_D: -1, phe__L_c_D: 1 });
  assert.deepEqual(doc.reactions.map(r => r.objective_coefficient), Array.from(raw.c));
  assert.deepEqual(doc.reactions.map(r => r.lower_bound), Array.from(raw.lb));
  assert.deepEqual(doc.reactions.map(r => r.upper_bound), Array.from(raw.ub));
});

test('Source converter rejects nonzero mass-balance RHS, non-equality rows, and invalid bound vectors', () => {
  const rhs = fixtureRaw(); rhs.b[0] = 1;
  assert.throws(() => convertZunigaMatModel(rhs), /RHS/);
  const sense = fixtureRaw(); sense.csense = 'LEE';
  assert.throws(() => convertZunigaMatModel(sense), /sense/);
  const bound = fixtureRaw(); bound.lb[0] = 4;
  assert.throws(() => convertZunigaMatModel(bound), /bounds/);
  const vector = fixtureRaw(); vector.ub = [3];
  assert.throws(() => convertZunigaMatModel(vector), /vectors/);
});

test('Malformed source GPR needs its valid indexed expression and never invents a replacement', () => {
  const absent = fixtureRaw(); absent.rules[4] = '';
  assert.throws(() => convertZunigaMatModel(absent), /No valid original GPR/);
  const unknown = fixtureRaw(); unknown.rules[4] = 'x(9999)';
  assert.throws(() => convertZunigaMatModel(unknown), /unknown gene/);
});

test('Source loader rejects failed downloads and unverified bytes before importing any model', async () => {
  const manifest = { sources: [{ file: 'M-models.mat', url: 'https://example.invalid/own-fixture.mat', sha256: '0'.repeat(64) }] };
  await assert.rejects(loadZunigaStudy(manifest, { fetcher: async () => new Response('', { status: 503 }) }), /HTTP 503/);
  await assert.rejects(loadZunigaStudy(manifest, {
    fetcher: async () => new Response(Uint8Array.from([1, 2, 3, 4]), { status: 200 }),
  }), /integrity check failed/);
});

test('Canonical IDs keep independent species compartments and shared phenylalanine without a trailing underscore', () => {
  const original = convertZunigaMatModel(fixtureRaw(true));
  const before = JSON.stringify(original);
  const canonical = canonicalZunigaModel(original);
  assert.equal(JSON.stringify(original), before, 'Canonicalization must not mutate source models');
  assert.ok(canonical.metabolites.some(m => m.id === 'Ec@phe__L_c' && m.compartment === 'c'));
  assert.ok(canonical.metabolites.some(m => m.id === 'Se@phe__L_c' && m.compartment === 'c'));
  assert.ok(canonical.metabolites.some(m => m.id === 'M@phe__L' && m.base === 'phe__L' && m.compartment === 's'));
  assert.deepEqual(canonical.reactions.find(r => r.id === 'Se@PHELtex').metabolites,
    { 'Se@phe__L_c': 1, 'M@phe__L': -1 });
  assert.equal(canonical.reactions.find(r => r.id === 'EX_phe__L_s').notes.source_reaction_id, 'EX_phe__L_[smp]');
  const collision = structuredClone(original);
  collision.metabolites.push({ ...original.metabolites.at(-1), id: 'phe__L[smp]' });
  assert.throws(() => canonicalZunigaModel(collision), /collision/);
});

test('Scenario preparation changes exactly the GPR knockout and leaves source objective and nutrition intact', () => {
  const study = fixtureStudy();
  const before = JSON.stringify(study.sourceModels);
  const mutant = createZunigaScenario(study, 'mutant-coculture');
  assert.deepEqual(mutant.knockouts, ['Ec@PPNDH']);
  assert.equal(mutant.model.reactions.get('Ec@PPNDH').upperBound, 0);
  assert.equal(mutant.model.reactions.get('Ec@CHORM').upperBound, 1000);
  assert.deepEqual(mutant.model.objectiveReactions(), [
    { id: canonicalEcBiomass, coefficient: 1 }, { id: 'Se@BOF', coefficient: 1 },
  ]);
  assert.equal(mutant.model.reactions.get('Se@BOF').lowerBound, 1);
  assert.equal(mutant.model.reactions.get('EX_phe__L_s').upperBound, 0);
  assert.equal(JSON.stringify(study.sourceModels), before, 'Preparing a knockout must not alter saved sources');
  const wt = createZunigaScenario(study, 'wt-monoculture');
  assert.equal(wt.model.reactions.get('Ec@PPNDH').upperBound, 1000);
  assert.deepEqual(wt.knockouts, []);
  assert.throws(() => createZunigaScenario(study, 'missing'));
});

test('Synthetic stationary model reproduces WT growth, auxotroph arrest, and a donor-dependent rescue', () => {
  const study = fixtureStudy();
  const expected = [
    ['wt-monoculture', 3, { Ec: 3 }],
    ['mutant-monoculture', 0, { Ec: 0 }],
    ['mutant-coculture', 4, { Ec: 3, Se: 1 }],
  ];
  for (const [id, objective, growth] of expected) {
    const prepared = createZunigaScenario(study, id);
    const result = solveZunigaScenario(prepared);
    assert.equal(result.status, 'optimal');
    assert.equal(result.objectiveValue, objective);
    assert.deepEqual(Object.fromEntries(prepared.community.growthRates(result.fluxes)), growth);
    assert.equal(result.zuniga.source, id.endsWith('coculture') ? 'CM-modelsKO.mat:modelCo2{2}' : 'M-models.mat:SingleM{4}');
    if (id === 'mutant-coculture') {
      assert.equal(result.fluxes.get('Ec@PHEtex'), 3);
      assert.equal(result.fluxes.get('Se@PHELtex'), -3);
      assert.equal(result.fluxes.get('EX_phe__L_s'), 0);
      assert.equal(result.zuniga.originalFluxes.PHEtex_D, 3);
      assert.equal(result.zuniga.originalFluxes.PHELtex_A, -3);
      const links = prepared.community.crossFeeding(result.fluxes).links;
      assert.deepEqual(links, [{ base: 'phe__L', from: 'Se', to: 'Ec', flux: 3 }]);
    }
  }
});

test('Monoculture has no enabled S. elongatus growth and replay preserves the full prepared model', () => {
  const study = fixtureStudy();
  const prepared = createZunigaScenario(study, 'mutant-monoculture');
  assert.deepEqual(prepared.community.activeSpecies.map(s => s.id), ['Ec']);
  assert.deepEqual(Object.fromEntries(prepared.community.growthRates(new Map([
    [canonicalEcBiomass, 0], ['Se@BOF', 100],
  ]))), { Ec: 0 });
  const replay = restoreZunigaScenario(study, prepared.scenario.id, prepared.model.toJSON());
  assert.deepEqual(replay.model.objectiveReactions(), prepared.model.objectiveReactions());
  assert.deepEqual(replay.problem.rxnIds, prepared.problem.rxnIds);
  assert.deepEqual([...replay.problem.lb], [...prepared.problem.lb]);
  assert.deepEqual([...replay.problem.ub], [...prepared.problem.ub]);
  assert.deepEqual(replay.knockouts, ['Ec@PPNDH']);
});

test('Cross-feeding reports net transfer between cells rather than simultaneous transport circulation', () => {
  const study = fixtureStudy();
  study.sourceModels.joint.reactions.push(
    { id: 'PHEreturn_D', metabolites: { phe__L_c_D: -1, 'phe__L_[smp]': 1 },
      lower_bound: 0, upper_bound: 1000, objective_coefficient: 0,
      gene_reaction_rule: '', notes: { source_reaction_id: 'PHEreturn_D' } },
    { id: 'PHEreuptake_A', metabolites: { 'phe__L_[smp]': -1, phe__L_c_A: 1 },
      lower_bound: 0, upper_bound: 1000, objective_coefficient: 0,
      gene_reaction_rule: '', notes: { source_reaction_id: 'PHEreuptake_A' } },
  );
  const prepared = createZunigaScenario(study, 'mutant-coculture');
  const fluxes = new Map([...prepared.model.reactions.keys()].map(id => [id, 0]));
  fluxes.set('Se@SOURCE', 4);
  fluxes.set('Se@BOF', 1);
  fluxes.set(canonicalEcBiomass, 3);
  fluxes.set('Se@PHELtex', -5);
  fluxes.set('Se@PHEreuptake', 2);
  fluxes.set('Ec@PHEtex', 4);
  fluxes.set('Ec@PHEreturn', 1);
  // This is a feasible optimum of the own synthetic LP, including the cycles.
  for (const mid of prepared.model.metabolites.keys()) {
    const balance = [...prepared.model.reactions.values()].reduce(
      (sum, reaction) => sum + (reaction.metabolites[mid] ?? 0) * fluxes.get(reaction.id), 0,
    );
    assert.equal(balance, 0);
  }
  const feeding = prepared.community.crossFeeding(fluxes);
  assert.deepEqual(feeding.links, [{ base: 'phe__L', from: 'Se', to: 'Ec', flux: 3 }]);
  const phenylalanine = feeding.perMetabolite.find(entry => entry.base === 'phe__L');
  assert.deepEqual(phenylalanine.producers, [{ speciesId: 'Se', flux: 3 }]);
  assert.deepEqual(phenylalanine.consumers, [{ speciesId: 'Ec', flux: 3 }]);
});
