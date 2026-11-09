import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile, access } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  PUBLISHED_COMMUNITIES, FRAMEWORKS, findCommunity,
  reproducedCommunities, blockedCommunities,
} from '../src/communities/published.js';
import {
  EXPECTED_GROWTH_RATE, STRAINS, EX_RATE, EXCHANGE_UPPER_BOUND, PROVENANCE,
} from '../src/communities/steadycom-ecoli.js';
import { Store } from '../src/ui/store.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

const scenarios = JSON.parse(await readFile(join(root, 'models', 'scenarios.json'), 'utf8'));

// The registry

test('every community carries the metadata a citation needs', () => {
  for (const c of PUBLISHED_COMMUNITIES) {
    assert.ok(c.id, 'missing id');
    assert.ok(c.name, `${c.id}: missing name`);
    assert.ok(c.description?.length > 40, `${c.id}: description is too thin to be useful`);
    const p = c.paper;
    for (const field of ['title', 'authors', 'year', 'journal', 'doi', 'framework']) {
      assert.ok(p?.[field], `${c.id}: paper.${field} is missing`);
    }
    assert.match(String(p.doi), /^10\.\d{4,9}\/\S+$/, `${c.id}: ${p.doi} is not a DOI`);
    assert.ok(p.year >= 2000 && p.year <= new Date().getFullYear(), `${c.id}: year ${p.year}`);
    assert.ok(Object.values(FRAMEWORKS).includes(p.framework),
      `${c.id}: ${p.framework} is not one of the known frameworks`);
  }
});

test('ids are unique', () => {
  const ids = PUBLISHED_COMMUNITIES.map((c) => c.id);
  assert.equal(new Set(ids).size, ids.length);
});

test('a citation says whether it was verified, one way or the other', () => {
  // The field has to be present and explicit. Leaving it undefined would let an
  // unchecked DOI pass for a checked one, which is the whole thing this guards.
  for (const c of PUBLISHED_COMMUNITIES) {
    assert.ok('doiVerifiedFrom' in c.paper, `${c.id}: paper.doiVerifiedFrom is not stated`);
    const v = c.paper.doiVerifiedFrom;
    assert.ok(v === null || typeof v === 'string', `${c.id}: doiVerifiedFrom should name a file or be null`);
  }
});

test('species entries are complete and referenced consistently', () => {
  for (const c of PUBLISHED_COMMUNITIES) {
    const ids = c.species.map((s) => s.id);
    assert.equal(new Set(ids).size, ids.length, `${c.id}: duplicate species id`);
    for (const sp of c.species) {
      assert.ok(sp.id && sp.name && sp.organism && sp.role,
        `${c.id}/${sp.id}: a species needs an id, name, organism and role`);
      assert.ok('model' in sp, `${c.id}/${sp.id}: say which model, or null`);
      assert.ok('abundance' in sp, `${c.id}/${sp.id}: say the abundance, or null`);
    }
    // An exchange may only name a species of this community, or the medium.
    for (const x of c.exchanges ?? []) {
      assert.ok(ids.includes(x.from), `${c.id}: exchange from unknown "${x.from}"`);
      for (const to of [].concat(x.to ?? [])) {
        if (to === null) continue;
        assert.ok(ids.includes(to), `${c.id}: exchange to unknown "${to}"`);
      }
      assert.ok('verified' in x, `${c.id}: exchange ${x.metabolite} does not say if it is verified`);
    }
  }
});

test('a model reference always says where the file came from', () => {
  for (const c of PUBLISHED_COMMUNITIES) {
    const models = [...(c.models ?? []), ...c.species.map((s) => s.model).filter(Boolean)];
    for (const m of models) {
      if (!m.id) continue;             // a bare { ref } points at the community's own models
      assert.ok(m.reference, `${c.id}/${m.id}: no reference for the reconstruction`);
      assert.ok(m.source, `${c.id}/${m.id}: no source for the file`);
      assert.ok(m.file, `${c.id}/${m.id}: no file name`);
    }
    // A { ref } has to resolve against this community's own model list.
    for (const sp of c.species) {
      if (!sp.model?.ref) continue;
      assert.ok((c.models ?? []).some((m) => m.id === sp.model.ref),
        `${c.id}/${sp.id}: refers to model ${sp.model.ref}, which the community does not list`);
    }
  }
});

// The reproduced / blocked line

test('status is one of the two the registry defines', () => {
  for (const c of PUBLISHED_COMMUNITIES) {
    assert.ok(['reproduced', 'blocked'].includes(c.reproduction.status), `${c.id}`);
  }
  assert.ok(reproducedCommunities().length + blockedCommunities().length
    === PUBLISHED_COMMUNITIES.length);
});

test('a reproduced community says what it reproduces, what it adapts, and how to run it', () => {
  for (const c of reproducedCommunities()) {
    const rep = c.reproduction;
    assert.ok(rep.reproduces.length, `${c.id}: claims to reproduce the paper but lists nothing`);
    assert.ok(rep.adaptations.length,
      `${c.id}: no adaptations listed — a reproduction that departs from the source in no way `
      + 'at all is unlikely, and silence here is what the registry is meant to prevent');
    assert.ok(rep.expected, `${c.id}: no expected values to check against`);
    assert.ok(rep.builder && rep.runner, `${c.id}: no builder or runner to point a reader at`);
    assert.equal(rep.needs.length, 0, `${c.id}: reproduced, yet still lists what it needs`);
    assert.ok(c.conditions, `${c.id}: reproduced, so the conditions must be on record`);
  }
});

test('a blocked community states what is missing and claims nothing', () => {
  for (const c of blockedCommunities()) {
    const rep = c.reproduction;
    assert.ok(rep.needs.length, `${c.id}: blocked but does not say what is missing`);
    assert.equal(rep.reproduces.length, 0, `${c.id}: blocked, yet claims to reproduce something`);
    assert.equal(rep.expected, null, `${c.id}: blocked, yet carries expected values`);
    // Exchanges may be listed from the paper's prose, but none may pass as verified.
    for (const x of c.exchanges ?? []) {
      assert.equal(x.verified, false,
        `${c.id}: exchange ${x.metabolite} is marked verified in a community whose source `
        + 'could not be retrieved');
    }
  }
});

test('the four benchmark papers are all present', () => {
  // The set that ships models and a map. A missing one should fail loudly rather
  // than quietly shrink the collection.
  for (const id of [
    'stolyar-2007-syntrophy',
    'steadycom-ecoli-auxotrophs',
    'steadycom-gut-nine',
    'zuniga-2020-phea-rescue',
  ]) {
    assert.ok(findCommunity(id), `${id} is not in the registry`);
  }
});

// The builder and the file

test('the registry agrees with the SteadyCom builder it points at', () => {
  const c = findCommunity('steadycom-ecoli-auxotrophs');
  assert.equal(c.reproduction.expected.growthRate, EXPECTED_GROWTH_RATE);
  assert.equal(c.paper.doi, PROVENANCE.paper.doi);
  assert.equal(c.species.length, STRAINS.length);

  // Each strain's description must match the bounds the builder actually sets,
  // so the card cannot drift away from the code it describes.
  for (const strain of STRAINS) {
    const sp = c.species.find((s) => s.id === strain.id);
    assert.ok(sp, `${strain.id} is in the builder but not the registry`);
    for (const aa of strain.auxotrophFor) {
      assert.match(sp.role, new RegExp(aa, 'i'), `${strain.id}: role omits its auxotrophy for ${aa}`);
    }
    assert.match(sp.role, new RegExp(strain.cannotExport, 'i'),
      `${strain.id}: role omits what it cannot export`);
    assert.match(sp.role, new RegExp(strain.supplies, 'i'),
      `${strain.id}: role omits what it supplies`);
  }

  // And the cross-feeding listed is the cross-feeding the strains imply.
  for (const strain of STRAINS) {
    const supplied = c.exchanges.find((x) => x.from === strain.id);
    assert.ok(supplied, `${strain.id} supplies nothing in the registry`);
    assert.match(supplied.metabolite, new RegExp(strain.supplies, 'i'));
    // Everyone auxotrophic for it should be listed as a consumer.
    const consumers = [].concat(supplied.to);
    for (const other of STRAINS) {
      if (other.auxotrophFor.includes(strain.supplies)) {
        assert.ok(consumers.includes(other.id),
          `${supplied.metabolite}: ${other.id} needs it but is not listed as a consumer`);
      }
    }
  }

  assert.match(c.conditions.crossFeedingUptake, new RegExp(String(EX_RATE)));
  assert.match(c.conditions.secretion, new RegExp(String(EXCHANGE_UPPER_BOUND)));
});

test('models/scenarios.json carries the registry and keeps the hand-written scenarios', () => {
  assert.ok(Array.isArray(scenarios.scenarios) && scenarios.scenarios.length >= 5,
    'the teaching scenarios must survive regeneration');
  const published = scenarios.publishedCommunities;
  assert.ok(Array.isArray(published), 'publishedCommunities is missing from scenarios.json');
  assert.equal(published.length, PUBLISHED_COMMUNITIES.length);

  for (const c of PUBLISHED_COMMUNITIES) {
    const copy = published.find((p) => p.id === c.id);
    assert.ok(copy, `${c.id} was not written out`);
    // `computed` is added by the runner, so compare everything else.
    const { computed, ...rest } = copy;
    assert.deepEqual(rest, JSON.parse(JSON.stringify(c)),
      `${c.id}: scenarios.json is stale — rerun tools/build-published-communities.mjs`);
  }
});

test('the recorded result still matches what the paper reports', () => {
  // The numbers in scenarios.json come from an actual solve, so they are the
  // artefact a regression would show up in without anyone rerunning a long job.
  for (const c of reproducedCommunities()) {
    const copy = scenarios.publishedCommunities.find((p) => p.id === c.id);
    assert.ok(copy.computed, `${c.id}: no recorded result — run the builder with --run`);
    const expected = c.reproduction.expected;

    // A community reproduced against a single published number is checked
    // against it. One reproduced against a sweep — where the published figure's
    // own values are not available offline — is checked against the range that
    // figure's axes span, which is what the registry records instead.
    if (expected.growthRate !== undefined) {
      assert.ok(Math.abs(copy.computed.growthRate - expected.growthRate) <= expected.tolerance,
        `${c.id}: recorded ${copy.computed.growthRate}, paper reports ${expected.growthRate}`);
    } else if (expected.growthRates) {
      // A community whose two members grow at different rates records one per
      // member rather than a single community rate.
      for (const [member, rate] of Object.entries(expected.growthRates)) {
        const got = copy.computed.growthRates?.[member];
        assert.equal(typeof got, 'number', `${c.id}: no recorded rate for ${member}`);
        assert.ok(Math.abs(got - rate) <= expected.tolerance,
          `${c.id}/${member}: recorded ${got}, expected ${rate}`);
      }
      if (expected.scenarioCount) {
        assert.equal(copy.computed.scenarios.length, expected.scenarioCount);
        const mutant = copy.computed.scenarios.find(s => s.id === 'mutant-monoculture');
        assert.equal(mutant.growth.Ec, expected.mutantMonoculture);
        assert.ok(Math.abs(copy.computed.phenylalanineTransfer - expected.phenylalanineTransfer) <= expected.tolerance);
      } else {
        assert.ok(copy.computed.sweepPoints > 0, `${c.id}: the recorded sweep is empty`);
      }
    } else {
      assert.ok(Array.isArray(expected.growthRateRange),
        `${c.id}: expected neither a growth rate nor a range`);
      const [lo, hi] = copy.computed.growthRateRange;
      assert.ok(lo >= expected.growthRateRange[0] && hi <= expected.growthRateRange[1],
        `${c.id}: recorded ${lo}..${hi}, outside ${expected.growthRateRange.join('..')}`);
      assert.ok(copy.computed.sweepPoints > 0, `${c.id}: the recorded sweep is empty`);
    }

    // SteadyCom solves for relative abundances, which have to sum to one. A
    // joint-FBA model does not have them at all: its two biomass fluxes are
    // absolute and their ratio is whatever the weighted objective makes it.
    if (copy.computed.abundances) {
      const total = Object.values(copy.computed.abundances).reduce((s, x) => s + x, 0);
      assert.ok(Math.abs(total - 1) < 1e-4, `${c.id}: recorded abundances sum to ${total}`);
    } else {
      assert.notEqual(c.paper.framework, FRAMEWORKS.STEADYCOM,
        `${c.id}: a SteadyCom reproduction has to record the abundances it solved for`);
    }
  }
});

test('the recorded cross-feeding has every blocked exporter at exactly zero', () => {
  // The structural claim of the SteadyCom community: a strain cannot release the
  // amino acid whose exporter was knocked out, so that flux must be zero, and it
  // must be releasing the one it can.
  const c = findCommunity('steadycom-ecoli-auxotrophs');
  const recorded = scenarios.publishedCommunities.find((p) => p.id === c.id).computed;
  for (const strain of STRAINS) {
    const row = recorded.exchanges[strain.id];
    assert.equal(row[strain.cannotExport], 0,
      `${strain.id} cannot export ${strain.cannotExport}, yet the flux is ${row[strain.cannotExport]}`);
    assert.ok(row[strain.supplies] > 0,
      `${strain.id} should be supplying ${strain.supplies}, flux is ${row[strain.supplies]}`);
    for (const aa of strain.auxotrophFor) {
      assert.ok(row[aa] < 0, `${strain.id} is auxotrophic for ${aa} but is not taking it up`);
    }
  }
});


test('the four-E. coli published community keeps full models separate from map previews', async () => {
  const manifest = JSON.parse(await readFile(
    join(root, 'models', 'published', 'steadycom-ecoli-auxotrophs', 'manifest.json'), 'utf8'
  ));

  for (const member of manifest.members) {
    const full = JSON.parse(await readFile(
      join(root, 'models', 'published', 'steadycom-ecoli-auxotrophs', member.model), 'utf8'
    ));
    const preview = JSON.parse(await readFile(
      join(root, 'models', 'published', 'steadycom-ecoli-auxotrophs', member.preview), 'utf8'
    ));

    const acald = full.reactions.find((reaction) => reaction.id === 'ACALD');
    assert.ok(acald, `${member.id}: full model is missing ACALD`);
    assert.equal(acald.metabolites.accoa_c, 1,
      `${member.id}: ACALD should contain cytosolic acetyl-CoA`);
    assert.ok(full.metabolites.some((metabolite) => metabolite.id === 'accoa_c'),
      `${member.id}: full model is missing acetyl-CoA`);
    assert.equal(preview.reactions.some((reaction) => reaction.id === 'ACALD'), false,
      `${member.id}: regression fixture expects ACALD to be absent from the curated preview`);
  }

  const storeSource = await readFile(join(root, 'src', 'ui', 'store.js'), 'utf8');
  assert.match(storeSource, /publishedMemberFileForRuntime\(id, m\)/,
    'published communities must resolve the member file according to the active runtime');
  assert.match(storeSource, /return full \?\? preview/,
    'hosted builds must continue to prefer the full biological model');
  assert.match(storeSource, /contents: curatedCoreMap \|\| jointMap \|\| ownMap \? 'none' : 'all'/,
    'the E. coli preview remains a map concern when the full biological model is available');
});

test('standalone runtime loads an embedded published preview instead of fetching an absent full model', async () => {
  const preview = JSON.parse(await readFile(join(root, 'models', 'ecoli_core_mc.json'), 'utf8'));
  const previousEmbedded = globalThis.__ESCHER_MCMC_DATA__;

  globalThis.__ESCHER_MCMC_DATA__ = {
    'models/published/runtime-preview/preview-Ec.json': preview,
  };

  try {
    const store = new Store('.');
    store.publishedCommunities = [{
      id: 'runtime-preview',
      name: 'Runtime preview community',
      description: 'Regression fixture for the standalone file.',
    }];
    store.publishedManifest = async () => ({
      medium: { glc__D: 10, o2: 1000, nh4: 1000, pi: 1000, h2o: 1000, h: 1000, co2: 1000 },
      members: [{
        id: 'Ec',
        model: 'model-Ec.json',
        preview: 'preview-Ec.json',
      }],
    });

    await store.usePublishedCommunity('runtime-preview');

    assert.equal(store.scenario.id, 'published:runtime-preview');
    assert.equal(store.species[0].file, 'published/runtime-preview/preview-Ec');
    assert.ok(store.community, 'standalone preview did not become the active community');
    assert.ok(store.map, 'standalone preview did not create a map');
    assert.ok(store.map.nodes.size > 0, 'standalone preview map is empty');
  } finally {
    if (previousEmbedded === undefined) delete globalThis.__ESCHER_MCMC_DATA__;
    else globalThis.__ESCHER_MCMC_DATA__ = previousEmbedded;
  }
});

// The full solve, which is slow

const MODEL_DIR = process.env.COXETER_MODEL_DIR ?? '/home/user/mmc/sbml';

test('the four-auxotroph community solves to the published growth rate', async (t) => {
  // Four genome-scale models and 22 LPs: about two and a half minutes, so it is
  // opt-in rather than part of every run.
  //     COXETER_REPRODUCE=1 COXETER_MODEL_DIR=/path/to/sbml npm test
  if (process.env.COXETER_REPRODUCE !== '1') {
    return t.skip('set COXETER_REPRODUCE=1 to solve it (about 150 s)');
  }
  const file = join(MODEL_DIR, 'iAF1260.xml');
  try {
    await access(file);
  } catch {
    return t.skip(`${file} not found; set COXETER_MODEL_DIR to the directory holding iAF1260.xml`);
  }

  const { initGLPK } = await import('../src/solver/glpk.js');
  const { steadyCom } = await import('../src/core/steadycom.js');
  const { buildCommunity, AMINO_ACID_EXCHANGES } =
    await import('../src/communities/steadycom-ecoli.js');

  assert.ok(await initGLPK(), 'GLPK is required for a genome-scale community');
  const spec = buildCommunity(await readFile(file, 'utf8'));
  const result = steadyCom(spec, { upperGuess: 1.0 });

  assert.equal(result.status, 'optimal', result.message);
  assert.ok(Math.abs(result.growthRate - EXPECTED_GROWTH_RATE) < 1e-4,
    `got ${result.growthRate}, the tutorial reports ${EXPECTED_GROWTH_RATE}`);

  const total = [...result.abundances.values()].reduce((s, x) => s + x, 0);
  assert.ok(Math.abs(total - 1) < 1e-4, `abundances sum to ${total}`);

  for (const strain of STRAINS) {
    const flux = (aa) => result.fluxes.get(`${strain.id}|${AMINO_ACID_EXCHANGES[aa]}`) ?? 0;
    assert.ok(Math.abs(flux(strain.cannotExport)) < 1e-9,
      `${strain.id} exported ${strain.cannotExport}, which it cannot`);
    assert.ok(flux(strain.supplies) > 0, `${strain.id} is not supplying ${strain.supplies}`);
    for (const aa of strain.auxotrophFor) {
      assert.ok(flux(aa) < 0, `${strain.id} is not taking up the ${aa} it cannot make`);
    }
  }
});
