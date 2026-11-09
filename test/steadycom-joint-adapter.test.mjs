import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Model } from '../src/core/model.js';
import { FluxProblem } from '../src/solver/fba.js';
import {
  jointSteadyComSpec, steadyComSpec, exchangeIndex, parseMediaCSV, toMergedFluxes, communityBalances,
  serializeSteadyComSpec, restoreSteadyComSpec,
  jointCommunityProjection,
} from '../src/communities/steadycom-runtime.js';

const dir = join(dirname(fileURLToPath(import.meta.url)), '..', 'models', 'published', 'steadycom-gut-nine');
const manifest = JSON.parse(await readFile(join(dir, 'manifest.json'), 'utf8'));
const bounds = await readFile(join(dir, 'bounds.csv'), 'utf8');
const members = await Promise.all(manifest.members.map(async ({ id, model }) => ({
  id, model: new Model(JSON.parse(await readFile(join(dir, model), 'utf8'))),
})));
const spec = jointSteadyComSpec(members, bounds);

test('published E. coli medium aliases resolve through their exchange reaction to the actual metabolite', async () => {
  const ecoliDir = join(dir, '..', 'steadycom-ecoli-auxotrophs');
  const model = new Model(JSON.parse(await readFile(join(ecoliDir, 'model-Ec1.json'), 'utf8')));
  const medium = parseMediaCSV(await readFile(join(ecoliDir, 'media.csv'), 'utf8'), [{ id: 'Ec1', model }]);
  assert.equal(medium['glc-D'], 8);
  assert.equal(medium.glc, undefined);
  assert.equal(medium.o2, 18.5);
  assert.equal(Object.keys(medium).length, 22);
  const problem = steadyComSpec([{ id: 'Ec1', model }], medium);
  assert.equal(problem.members[0].exchangeFor['glc-D'], 'EX_glc_e');
  assert.throws(() => parseMediaCSV('metabolite,exchange_reaction,lower_bound\nx,EX_missing,-1', [{ id: 'Ec1', model }]), /no matching member exchange/);
});

test('the complete nine-member assets reconstruct every joint reaction and coefficient', () => {
  assert.equal(spec.metadata.kind, 'joint');
  assert.equal(spec.reactions.size, 11009);
  assert.equal(spec.metabolites.size, 8160);
  assert.equal([...spec.reactions.values()].reduce((sum, reaction) => sum + Object.keys(reaction.metabolites).length, 0), 42420);
  assert.equal(spec.members.length, 9);
  assert.equal(spec.communityTag, 'comm');
  assert.equal(spec.totalBiomass, 1);
  assert.deepEqual(spec.members.map(({ id, biomassReaction }) => [id, biomassReaction]), [
    ['Bt', 'Biomass_BT_v2_Bt'], ['Fp', 'Biomass_FP_Fp'], ['Kp', 'Biomass_Kp'],
    ['St', 'biomass_STR_St'], ['Ba', 'Biomass_Ba'], ['Ec', 'Ec_biomass_iJO1366_WT_53p95M_Ec'],
    ['Ef', 'R_BIOMASS_Ef'], ['Er', 'r55_Er'], ['Lc', 'bio06001_Lc'],
  ]);
  for (const line of bounds.trim().split(/\r?\n/).slice(1)) {
    const [id, owner, lower, upper, objective] = line.split(',');
    const reaction = spec.reactions.get(id);
    assert.equal(reaction.species, owner, id);
    assert.equal(reaction.lowerBound, Number(lower), id);
    assert.equal(reaction.upperBound, Number(upper), id);
    assert.equal(reaction.objective, Number(objective), id);
  }
});

test('shared stoichiometry supplies exact exchange ownership even for unrelated member identifiers', () => {
  assert.equal(spec.metadata.speciesExchange.size, 1355);
  assert.equal(spec.metadata.bases.size, 558);
  assert.equal(spec.metadata.communityUptake.size, 558);
  assert.equal(spec.metadata.communityExport.size, 558);
  assert.deepEqual(spec.reactions.get('EX_12ethd[u]tr_Bt').metabolites, { '12ethd[e_Bt]': -1, '12ethd[u]': 1 });
  assert.equal(spec.metadata.speciesExchange.get('glc-D[u]\u0000Er'), 'glcIn_Er');
  assert.equal(spec.metadata.speciesExchange.get('2mba[u]\u0000Ef'), 'R_Ex_2mbut__e_Ef');
  assert.equal(spec.metadata.bases.get('glc-D[u]'), 'glc-D');
  for (const met of ['ad[u]', 'biomassPg[u]', 'h2co3[u]', 'pime[u]']) assert.equal(spec.metabolites.get(met).species, 'comm');
  assert.equal([...spec.reactions.values()].filter(reaction => reaction.species === 'comm').length, 1116);
});

test('the full diet and ATP maintenance survive without using the narrowed preview medium', () => {
  const open = [...spec.metadata.communityUptake.values()].filter(id => spec.reactions.get(id).upperBound > 0);
  assert.equal(open.length, 88);
  assert.equal(Object.keys(manifest.medium).length, 8);
  assert.equal(spec.reactions.get('UT_glc-D(u)').upperBound, 0.03832823);
  assert.equal(spec.reactions.get('UT_adocbl(u)').upperBound, 0.1);
  assert.deepEqual(spec.reactions.get('UT_adocbl(u)').metabolites, { 'adocbl[u]': 1 });
  assert.deepEqual(spec.reactions.get('EX_adocbl(u)').metabolites, { 'adocbl[u]': -1 });
  for (const id of spec.metadata.communityExport.values()) {
    assert.equal(spec.reactions.get(id).lowerBound, 0);
    assert.equal(spec.reactions.get(id).upperBound, 10000);
  }
  for (const id of ['ATPM_Bt', 'ATPM_Fp', 'ATPM_Kp', 'ATPM_St', 'ATPM_Ba', 'ATPM_Ec', 'R_ATPM_Ef', 'r54_Er', 'rxn00062_Lc']) {
    assert.equal(spec.reactions.get(id).lowerBound, 1, id);
    assert.equal(spec.reactions.get(id).upperBound, 1000, id);
  }
});

test('an explicit Figure 4 fiber condition adds the original uptake-only constraints', () => {
  const conditioned = jointSteadyComSpec(members, bounds, {
    totalBiomass: 2, fiber: { bacteroides: 10, faecalibacterium: 5 },
  });
  assert.equal(conditioned.totalBiomass, 2);
  assert.equal(conditioned.constraints.length, 2);
  assert.deepEqual(conditioned.constraints.map(constraint => constraint.terms.length), [12, 5]);
  assert.ok(conditioned.constraints.every(constraint => constraint.hi === 0 && constraint.terms.every(term => term.part === 'uptake')));
  assert.deepEqual(conditioned.constraints[0].biomass, [{ species: 'Bt', coef: -1 }]);
  assert.deepEqual(conditioned.constraints[1].biomass, [{ species: 'Fp', coef: -1 }]);
  assert.equal(conditioned.constraints[0].terms.find(term => term.reaction === 'EX_pect[u]tr_Bt').coef, 253.5);
  assert.deepEqual(spec.constraints, [], 'the unconditioned spec must remain independent');
  const original = members[0].model.reactions.get('ATPM_Bt').lowerBound;
  conditioned.reactions.get('ATPM_Bt').lowerBound = 0;
  assert.equal(members[0].model.reactions.get('ATPM_Bt').lowerBound, original, 'joint reactions must not alias member models');
});

test('joint flux ownership maps species rates to drawing IDs while community columns stay absolute', () => {
  const values = new Map([
    ['glcIn_Er', -2], ['Biomass_BT_v2_Bt', 0.005], ['UT_glc-D(u)', 0.01], ['EX_glc-D(u)', 0.003],
  ]);
  const mapped = toMergedFluxes(values, '@', spec.metadata.reactionOwners);
  assert.equal(mapped.get('Er@glcIn_Er'), -2);
  assert.equal(mapped.get('Bt@Biomass_BT_v2_Bt'), 0.005);
  assert.equal(mapped.get('UT_glc-D(u)'), 0.01);
  assert.equal(mapped.get('EX_glc-D(u)'), 0.003);
  assert.equal(mapped.size, values.size);
  assert.equal(toMergedFluxes(new Map([['A|R', 4]])).get('A@R'), 4, 'separate-member mapping stays compatible');
});

test('community balances report signed population exchange and actual external rates, not bounds', () => {
  const er = spec.metadata.speciesExchange.get('glc-D[u]\u0000Er');
  const bt = spec.metadata.speciesExchange.get('glc-D[u]\u0000Bt');
  const result = communityBalances(spec, new Map([
    [er, -0.2], [bt, 0.3], ['UT_glc-D(u)', 0.012], ['EX_glc-D(u)', 0.112],
  ]));
  assert.equal(result.speciesExchanges.get('glc-D[u]').get('Er'), -0.2);
  assert.equal(result.speciesExchanges.get('glc-D[u]').get('Bt'), 0.3);
  assert.equal(result.communityUptake.get('glc-D[u]'), 0.012);
  assert.equal(result.communityExport.get('glc-D[u]'), 0.112);
  assert.equal(result.communityUptake.get('adocbl[u]'), 0, 'absent flux columns are zero, not their diet bounds');
});

test('joint worker snapshots preserve maps and constraints without retaining Model instances', () => {
  const conditioned = jointSteadyComSpec(members, bounds, { fiber: { bacteroides: 10, faecalibacterium: 5 } });
  const restored = restoreSteadyComSpec(structuredClone(serializeSteadyComSpec(conditioned)));
  assert.ok(restored.reactions instanceof Map);
  assert.ok(restored.metabolites instanceof Map);
  assert.ok(restored.metadata.reactionOwners instanceof Map);
  assert.ok(restored.metadata.speciesExchange instanceof Map);
  assert.equal(restored.reactions.size, 11009);
  assert.equal(restored.metabolites.size, 8160);
  assert.ok(restored.members.every(member => !('model' in member)));
  assert.deepEqual(restored.constraints, conditioned.constraints);
  restored.reactions.get('ATPM_Bt').lowerBound = 0;
  assert.equal(conditioned.reactions.get('ATPM_Bt').lowerBound, 1);
});

test('separate worker snapshots reconstruct models and derive exchanges from metabolite identity', () => {
  const model = new Model({
    id: 'generic', compartments: { e: 'Extracellular', c: 'Cytosol' },
    metabolites: [{ id: 'arg-L_e', compartment: 'e' }, { id: 'arg-L_c', compartment: 'c' }],
    reactions: [
      { id: 'EX_arg_L_e', metabolites: { 'arg-L_e': -1 }, lower_bound: -5, upper_bound: 100 },
      { id: 'T', metabolites: { 'arg-L_e': -1, 'arg-L_c': 1 }, lower_bound: 0, upper_bound: 100 },
      { id: 'BIOMASS', metabolites: { 'arg-L_c': -1 }, lower_bound: 0, upper_bound: 100 },
    ], objective: { BIOMASS: 1 },
  });
  assert.deepEqual(exchangeIndex(model), { 'arg-L': 'EX_arg_L_e' });
  const original = steadyComSpec([{ id: 'generic', model }], { 'arg-L': 5 });
  const snapshot = serializeSteadyComSpec(original);
  assert.equal(snapshot.members[0].model instanceof Model, false);
  const restored = restoreSteadyComSpec(structuredClone(snapshot));
  assert.equal(restored.metadata.kind, 'separate');
  assert.ok(restored.members[0].model instanceof Model);
  assert.deepEqual(restored.members[0].model.toJSON(), model.toJSON());
  assert.equal(restored.members[0].model.biomassReactionId(), 'BIOMASS');
  restored.members[0].model.reactions.get('T').upperBound = 7;
  assert.equal(model.reactions.get('T').upperBound, 100);
});

test('missing, duplicate or invalid source rows cannot produce an apparently complete joint problem', () => {
  assert.throws(() => jointSteadyComSpec(members, bounds.replace(/^ATPM_Bt,.*\r?\n/m, '')), /ATPM_Bt.*matching bounds/);
  assert.throws(() => jointSteadyComSpec(members, `${bounds.trim()}\nATPM_Bt,Bt,1,1000,0\n`), /Duplicate.*ATPM_Bt/);
  assert.throws(() => jointSteadyComSpec(members, bounds.replace('UT_adocbl(u),comm,0,0.1,0', 'UT_adocbl(u),comm,0,NaN,0')), /invalid joint bounds/);
  assert.throws(() => jointSteadyComSpec(members, bounds, { totalBiomass: 0 }), /positive and finite/);
  assert.throws(() => jointSteadyComSpec(members, bounds, { fiber: { bacteroides: 0, faecalibacterium: 5 } }), /fiber uptake limits.*positive and finite/);
});

function tinyJoint() {
  const pair = ['A', 'B'].map(id => ({
    id, model: new Model({
      id, compartments: { e: 'Extracellular', c: 'Cytosol', u: 'Shared medium' },
      metabolites: [
        { id: `a[e_${id}]`, compartment: 'e' },
        { id: `a[c_${id}]`, compartment: 'c' },
        { id: 'a[u]', compartment: 'u' },
      ],
      reactions: [
        { id: `EX_${id}`, metabolites: { [`a[e_${id}]`]: -1, 'a[u]': 1 }, lower_bound: -20, upper_bound: 1000 },
        { id: `T_${id}`, metabolites: { [`a[e_${id}]`]: -1, [`a[c_${id}]`]: 1 }, lower_bound: 0, upper_bound: 1000 },
        { id: `BIO_${id}`, metabolites: { [`a[c_${id}]`]: -1 }, lower_bound: 0, upper_bound: 1000 },
      ], objective: { [`BIO_${id}`]: 1 },
    }),
  }));
  const csv = [
    'reaction,species,lower_bound,upper_bound,objective_coefficient',
    ...pair.flatMap(({ id }) => [`EX_${id},${id},-20,1000,0`, `T_${id},${id},0,1000,0`, `BIO_${id},${id},0,1000,1`]),
    'UT_a(u),comm,0,10,0', 'EX_a(u),comm,0,10000,0',
  ].join('\n');
  return jointSteadyComSpec(pair, csv);
}

function assertMassBalanced(model, fluxes) {
  const balances = new Map();
  for (const reaction of model.reactions.values()) {
    for (const [mid, coefficient] of Object.entries(reaction.metabolites)) {
      balances.set(mid, (balances.get(mid) ?? 0) + coefficient * (fluxes.get(reaction.id) ?? 0));
    }
  }
  for (const [mid, residual] of balances) assert.ok(Math.abs(residual) < 1e-7, `${mid} mass residual ${residual}`);
}

test('unequal fixed abundances scale only shared coefficients and give a balanced conventional FBA', () => {
  const joint = tinyJoint();
  joint.constraints = [{ name: 'steadycom-only', terms: [{ reaction: 'EX_A', coef: 1, part: 'uptake' }], hi: 0 }];
  const projection = jointCommunityProjection(joint, new Map([['A', 0.25], ['B', 0.75]]));
  assert.deepEqual(projection.model.reactions.get('A@EX_A').metabolites, { 'A@a[e_A]': -1, 'M@a_s': 0.25 });
  assert.deepEqual(projection.model.reactions.get('B@EX_B').metabolites, { 'B@a[e_B]': -1, 'M@a_s': 0.75 });
  assert.deepEqual(projection.model.reactions.get('UT_a(u)').metabolites, { 'M@a_s': 1 });
  assert.equal(projection.model.reactions.get('UT_a(u)').upperBound, 10);
  assert.equal(projection.model.metabolites.get('M@a_s').compartment, 's');
  const result = new FluxProblem(projection.model).optimize();
  assert.equal(result.status, 'optimal');
  assert.ok(Math.abs(result.objectiveValue - 10) < 1e-7);
  assert.ok(Math.abs(result.fluxes.get('A@BIO_A') - 10) < 1e-7);
  assert.ok(Math.abs(result.fluxes.get('B@BIO_B') - 10) < 1e-7);
  assertMassBalanced(projection.model, result.fluxes);
  assert.equal(joint.reactions.get('EX_A').metabolites['a[u]'], 1, 'projection must not mutate joint source coefficients');
});

test('zero-abundance members are closed without forcing community growth to zero', () => {
  const projection = jointCommunityProjection(tinyJoint(), { A: 1, B: 0 });
  for (const id of ['B@EX_B', 'B@T_B', 'B@BIO_B']) {
    assert.equal(projection.model.reactions.get(id).lowerBound, 0, id);
    assert.equal(projection.model.reactions.get(id).upperBound, 0, id);
  }
  assert.equal(projection.model.metabolites.has('GROWTHLINK@B'), false);
  assert.deepEqual(projection.activeSpecies.map(member => member.id), ['A']);
  const result = new FluxProblem(projection.model).optimize();
  assert.equal(result.status, 'optimal');
  assert.ok(Math.abs(result.objectiveValue - 10) < 1e-7);
  assert.equal(result.fluxes.get('B@BIO_B'), 0);
  assertMassBalanced(projection.model, result.fluxes);
});

test('fixed projection supports weighted biomass and net secretion and rejects unsupported targets', () => {
  const joint = tinyJoint();
  const biomass = jointCommunityProjection(joint, { A: 0.25, B: 0.75 }, { objective: 'sum-biomass' });
  assert.deepEqual(biomass.model.rawObjective, { 'A@BIO_A': 0.25, 'B@BIO_B': 0.75 });
  assert.equal(biomass.model.reactions.has('COMMUNITY_GROWTH'), false);
  const product = jointCommunityProjection(joint, { A: 0.25, B: 0.75 }, { objective: 'product', productTarget: 'a' });
  assert.deepEqual(product.model.rawObjective, { 'EX_a(u)': 1, 'UT_a(u)': -1 });
  const result = new FluxProblem(product.model).optimize();
  assert.equal(result.status, 'optimal');
  assert.ok(Math.abs(result.objectiveValue) < 1e-7, 'importing and immediately exporting substrate is not net product');
  assertMassBalanced(product.model, result.fluxes);
  assert.throws(() => jointCommunityProjection(joint, { A: 0.5, B: 0.5 }, { objective: 'unknown' }), /Unsupported.*objective/);
  assert.throws(() => jointCommunityProjection(joint, { A: 0.5, B: 0.5 }, { objective: 'product', productTarget: 'missing' }), /No community boundary/);
  assert.throws(() => jointCommunityProjection(joint, { A: 0, B: 0 }), /positive abundance/);
});

test('full GUT9 projection keeps every biological column and canonical cross-feeding alias', () => {
  const projection = jointCommunityProjection(spec, new Map(spec.members.map(member => [member.id, 1 / 9])));
  assert.equal(projection.model.reactions.size, 11010);
  assert.equal(projection.model.metabolites.size, 8169);
  assert.equal(projection.biomassIds.size, 9);
  assert.equal(projection.transfersByBase.size, 554);
  const transfer = projection.transfersByBase.get('glc-D').find(item => item.speciesId === 'Er');
  assert.equal(transfer.reactionId, 'Er@glcIn_Er');
  assert.equal(transfer.localId, 'Er@cpd00027[e_Er]');
  assert.deepEqual(projection.reactionOwner.get('Er@glcIn_Er'), { speciesId: 'Er', originalId: 'glcIn_Er', role: 'transfer' });
  assert.deepEqual(projection.reactionOwner.get('UT_glc-D(u)'), { speciesId: null, originalId: 'UT_glc-D(u)', role: 'community' });
  assert.deepEqual(projection.communityExchangeIds.get('glc-D'), { uptake: 'UT_glc-D(u)', export: 'EX_glc-D(u)' });
  assert.equal(projection.model.metabolites.get('M@glc-D_s').compartment, 's');
  assert.ok(projection.model.compartmentIds().every(compartment => ['c', 'e', 'p', 's', 'link'].includes(compartment)));
});
