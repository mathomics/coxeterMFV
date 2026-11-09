import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Model } from '../src/core/model.js';
import { CommunityModel, OBJECTIVES, namespaced, sharedMetabolite, communityExchange, SHARED_COMPARTMENT } from '../src/core/community.js';
import { FluxProblem } from '../src/solver/fba.js';
import { buildCompartmentTree } from '../src/core/compartments.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

async function loadModel(name) {
  return new Model(JSON.parse(await readFile(join(root, 'models', `${name}.json`), 'utf8')));
}

const AEROBIC = {
  glc__D: { lower: -10 }, o2: { lower: -20 }, nh4: { lower: -1000 },
  pi: { lower: -1000 }, h2o: { lower: -1000 }, h: { lower: -1000 },
  co2: { lower: -1000 },
};

const ANAEROBIC = { ...AEROBIC, o2: { lower: 0 } };

async function community(names, opts = {}) {
  const species = [];
  for (const [id, file] of names) {
    species.push({ id, model: await loadModel(file), abundance: 1 / names.length });
  }
  return new CommunityModel({
    species,
    objective: opts.objective ?? OBJECTIVES.EQUAL_GROWTH,
    medium: opts.medium ?? AEROBIC,
  });
}

test('merging keeps every species namespaced and adds one shared medium', async () => {
  const com = await community([['eco', 'ecoli_core_mc'], ['sce', 'yeast_core_mc']]);
  const merged = com.merged;

  assert.ok(merged.reactions.has(namespaced('eco', 'PGI')));
  assert.ok(merged.reactions.has(namespaced('sce', 'PGI')));
  assert.notEqual(namespaced('eco', 'PGI'), namespaced('sce', 'PGI'));

  // Shared medium metabolites exist exactly once and live in compartment `s`.
  const shared = [...merged.metabolites.values()].filter((m) => m.compartment === SHARED_COMPARTMENT);
  assert.ok(shared.length > 5);
  const bases = shared.map((m) => m.id);
  assert.equal(new Set(bases).size, bases.length, 'duplicate medium metabolite');
  assert.ok(merged.metabolites.has(sharedMetabolite('glc__D')));
  assert.ok(merged.reactions.has(communityExchange('glc__D')));
});

test('the merged model is structurally sound', async () => {
  const com = await community([['eco', 'ecoli_core_mc'], ['mba', 'methanogen_core_mc']]);
  for (const rxn of com.merged.reactions.values()) {
    for (const mid of Object.keys(rxn.metabolites)) {
      assert.ok(com.merged.metabolites.has(mid), `${rxn.id} references missing ${mid}`);
    }
  }
});

test('the community solution conserves every metabolite, medium included', async () => {
  const com = await community(
    [['eco', 'ecoli_core_mc'], ['mba', 'methanogen_core_mc']],
    { medium: ANAEROBIC },
  );
  const res = new FluxProblem(com.merged).optimize();
  assert.equal(res.status, 'optimal');

  const residual = new Map();
  for (const [rid, v] of res.fluxes) {
    if (!v) continue;
    for (const [mid, coef] of Object.entries(com.merged.reactions.get(rid).metabolites)) {
      residual.set(mid, (residual.get(mid) ?? 0) + coef * v);
    }
  }
  for (const [mid, value] of residual) {
    assert.ok(Math.abs(value) < 1e-6, `${mid} unbalanced by ${value}`);
  }
});

test('equal-growth gives every species the same growth rate', async () => {
  const com = await community(
    [['eco', 'ecoli_core_mc'], ['sce', 'yeast_core_mc'], ['mba', 'methanogen_core_mc']],
    { medium: ANAEROBIC, objective: OBJECTIVES.EQUAL_GROWTH },
  );
  const res = new FluxProblem(com.merged).optimize();
  assert.equal(res.status, 'optimal');
  const rates = [...com.growthRates(res.fluxes).values()];
  assert.equal(rates.length, 3);
  const spread = Math.max(...rates) - Math.min(...rates);
  assert.ok(spread < 1e-6, `growth rates differ by ${spread}: ${rates}`);
  assert.ok(rates[0] > 1e-4, 'the community should actually grow');
});

test('sum-biomass lets species grow at different rates', async () => {
  const com = await community(
    [['eco', 'ecoli_core_mc'], ['mba', 'methanogen_core_mc']],
    { medium: ANAEROBIC, objective: OBJECTIVES.SUM_BIOMASS },
  );
  const res = new FluxProblem(com.merged).optimize();
  assert.equal(res.status, 'optimal');
  const rates = [...com.growthRates(res.fluxes).values()];
  assert.ok(rates.some((r) => r > 1e-4));
});

test('a species that cannot grow alone grows on what its neighbour secretes', async () => {
  // The methanogen has no glucose transporter: on glucose alone it is stuck.
  const alone = await community([['mba', 'methanogen_core_mc']], { medium: ANAEROBIC });
  const soloRes = new FluxProblem(alone.merged).optimize();
  const soloGrowth = soloRes.status === 'optimal'
    ? (alone.growthRates(soloRes.fluxes).get('mba') ?? 0) : 0;
  assert.ok(soloGrowth < 1e-6, `methanogen should not grow alone, got ${soloGrowth}`);

  // Paired with a fermenter that supplies acetate, H2 and CO2, it does.
  const paired = await community(
    [['eco', 'ecoli_core_mc'], ['mba', 'methanogen_core_mc']],
    { medium: ANAEROBIC },
  );
  const res = new FluxProblem(paired.merged).optimize();
  assert.equal(res.status, 'optimal');
  const growth = paired.growthRates(res.fluxes).get('mba');
  assert.ok(growth > 1e-4, `expected cross-fed growth, got ${growth}`);

  // And methane only appears because of that pairing.
  assert.ok((res.fluxes.get(communityExchange('ch4')) ?? 0) > 1e-3, 'expected methane output');
});

test('cross-feeding links point from the producer to the consumer', async () => {
  const com = await community(
    [['eco', 'ecoli_core_mc'], ['mba', 'methanogen_core_mc']],
    { medium: ANAEROBIC },
  );
  const res = new FluxProblem(com.merged).parsimonious();
  const { links } = com.crossFeeding(res.fluxes, 1e-5);
  const speciesLinks = links.filter((l) => l.from && l.to);
  assert.ok(speciesLinks.length > 0, 'expected at least one species-to-species link');
  for (const link of speciesLinks) {
    assert.ok(link.flux > 0);
    assert.notEqual(link.from, link.to);
  }
  assert.ok(speciesLinks.some((l) => l.from === 'eco' && l.to === 'mba'),
    `expected E. coli to feed the methanogen: ${JSON.stringify(speciesLinks)}`);
});

test('abundance rescales what a species contributes to the medium', async () => {
  const build = async (abundance) => {
    const eco = await loadModel('ecoli_core_mc');
    const mba = await loadModel('methanogen_core_mc');
    return new CommunityModel({
      objective: OBJECTIVES.SUM_BIOMASS,
      medium: ANAEROBIC,
      species: [
        { id: 'eco', model: eco, abundance },
        { id: 'mba', model: mba, abundance: 1 - abundance },
      ],
    });
  };
  const low = await build(0.2);
  const high = await build(0.8);
  const ch4 = (com) => {
    const res = new FluxProblem(com.merged).optimize();
    return res.status === 'optimal' ? (res.fluxes.get(communityExchange('ch4')) ?? 0) : 0;
  };
  // More fermenter means more substrate reaching the methanogen.
  assert.ok(ch4(high) > ch4(low), `${ch4(high)} should exceed ${ch4(low)}`);
});

test('disabling a species removes it from the merged model', async () => {
  const eco = await loadModel('ecoli_core_mc');
  const sce = await loadModel('yeast_core_mc');
  const com = new CommunityModel({
    species: [
      { id: 'eco', model: eco, abundance: 0.5 },
      { id: 'sce', model: sce, abundance: 0.5, enabled: false },
    ],
    medium: AEROBIC,
  });
  assert.ok(com.merged.reactions.has(namespaced('eco', 'PGI')));
  assert.ok(!com.merged.reactions.has(namespaced('sce', 'PGI')));
  assert.equal(com.activeSpecies.length, 1);
});

test('compartment nesting skips levels the model does not declare', () => {
  // Yeast has no periplasm or cell wall, so the cytosol must fall back to `e`.
  const { byId } = buildCompartmentTree(['e', 'c', 'im', 'm']);
  assert.equal(byId.get('c').parent, 'e');
  assert.equal(byId.get('im').parent, 'c');
  assert.equal(byId.get('m').parent, 'im');
  assert.equal(byId.get('e').parent, null);

  // A model with a mitochondrion but no intermembrane space nests m inside c.
  const flat = buildCompartmentTree(['e', 'c', 'm']);
  assert.equal(flat.byId.get('m').parent, 'c');

  // Explicit trees win over the defaults.
  const explicit = buildCompartmentTree(['e', 'c', 'x'], { x: 'e', c: 'e', e: null });
  assert.equal(explicit.byId.get('x').parent, 'e');
});
