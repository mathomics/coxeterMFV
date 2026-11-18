/**
 * The merged community model: one COBRA JSON file holding every species, the
 * shared medium and the compartment hierarchy, which opens again as the same
 * species, so a map saved over the separate models opens on it.
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import {
  COMMUNITY_MODEL_VERSION,
  communityModelDocument,
  splitCommunityModel,
} from '../src/core/community-json.js';
import { prepareModelUploads } from '../src/core/model-import.js';
import { Store } from '../src/ui/store.js';

const MODEL_KEYS = ['cyano_core_mc', 'ecoli_core_mc', 'yeast_core_mc'];

async function readModel(key) {
  return JSON.parse(await readFile(new URL(`../models/${key}.json`, import.meta.url), 'utf8'));
}

/** The bundled three-species scenario: a cyanobacterium, E. coli and yeast. */
async function threeSpeciesStore() {
  const scenarios = JSON.parse(await readFile(new URL('../models/scenarios.json', import.meta.url), 'utf8'));
  const store = new Store('.');
  for (const key of MODEL_KEYS) {
    store.models.set(key, new Model(await readModel(key)));
  }
  store.scenarios = scenarios.scenarios;
  await store.useScenario('phototroph-heterotroph');
  return store;
}

async function openMerged(text) {
  const store = new Store('.');
  await store.importModelFiles([{ name: 'community_merged-model.json', text }]);
  return store;
}

test('a three-species community saves as one model, nested inside the shared medium', async () => {
  const store = await threeSpeciesStore();
  const doc = JSON.parse(store.mergedModelJSON());

  assert.equal(doc.reactions.length, store.community.merged.reactions.size);
  assert.equal(doc.metabolites.length, store.community.merged.metabolites.size);
  assert.ok(doc.reactions.some((r) => r.id === 'eco@PGI'));
  assert.deepEqual(doc.community.members.map((m) => m.id), ['syn', 'eco', 'sce']);

  // Every metabolite sits in a compartment the file declares.
  for (const met of doc.metabolites) {
    assert.ok(Object.hasOwn(doc.compartments, met.compartment), met.id);
  }
  assert.equal(doc.compartment_tree.s, null);
  assert.equal(doc.compartment_tree['eco@e'], 's');
  assert.equal(doc.compartment_tree['eco@c'], 'eco@p');
  assert.equal(doc.compartment_tree['sce@m'], 'sce@im');
  assert.equal(doc.compartment_tree['syn@u'], 'syn@c');

  // The community objective is the only one left in the merged model.
  const objective = doc.reactions.filter((r) => r.objective_coefficient);
  assert.deepEqual(objective.map((r) => r.id), ['COMMUNITY_GROWTH']);
});

test('the merged model opens as the same species, models, medium and growth', async () => {
  const store = await threeSpeciesStore();
  await store.runAnalysis('fba');
  const reopened = await openMerged(store.mergedModelJSON());
  await reopened.runAnalysis('fba');

  assert.deepEqual(reopened.species.map((sp) => [sp.id, sp.abundance]), store.species.map((sp) => [sp.id, sp.abundance]));
  for (let i = 0; i < store.species.length; i += 1) {
    const before = store.models.get(store.species[i].file).toJSON();
    const after = reopened.models.get(reopened.species[i].file).toJSON();
    assert.deepEqual(after, before, store.species[i].id);
  }
  assert.deepEqual(reopened.medium, store.medium);
  assert.equal(reopened.objective, store.objective);
  assert.equal(reopened.solution.status, 'optimal');
  for (const [id, rate] of store.growth) {
    assert.ok(Math.abs(reopened.growth.get(id) - rate) < 1e-9, id);
  }
});

test('a map saved over the separate models opens on the merged model, before or after it', async () => {
  const store = await threeSpeciesStore();
  assert.ok(store.applyNestingPreset('sce', 'bigg'));
  const mapText = store.mapJSON();
  const mergedText = store.mergedModelJSON();
  const drawn = [...store.map.reactions.keys()].sort();

  // The merged model first, then the map.
  const first = await openMerged(mergedText);
  await first.loadMapDocument(mapText);
  assert.deepEqual([...first.map.reactions.keys()].sort(), drawn);
  assert.deepEqual(first.map.cells.find((cell) => cell.id === 'sce').nesting, store.map.cells.find((cell) => cell.id === 'sce').nesting);

  // The map first, which waits for its models, then the merged model.
  const second = new Store('.');
  assert.equal(await second.loadMapDocument(mapText, { name: 'map.json' }), null);
  await second.importModelFiles([{ name: 'community_merged-model.json', text: mergedText }]);
  assert.equal(second.waitingMap, null);
  assert.deepEqual([...second.map.reactions.keys()].sort(), drawn);
});

test('a nesting set by hand goes into the merged tree and back into the species model', async () => {
  const yeast = new Model(await readModel('yeast_core_mc'));
  const community = new CommunityModel({ species: [{ id: 'sce', model: yeast, abundance: 1 }] });
  const nesting = { e: null, c: 'e', im: 'e', m: 'c' };
  const doc = communityModelDocument(community, { cells: [{ id: 'sce', outerCompartment: 'e', nesting }] });

  assert.equal(doc.compartment_tree['sce@e'], 's');
  assert.equal(doc.compartment_tree['sce@im'], 'sce@e');
  assert.equal(doc.compartment_tree['sce@m'], 'sce@c');

  const [member] = splitCommunityModel(JSON.parse(JSON.stringify(doc))).members;
  assert.deepEqual(member.model.compartment_tree, nesting);
});

test('genes are namespaced per species and the models come back as they were written', async () => {
  const raw = await readModel('ecoli_core_mc');
  raw.reactions.find((r) => r.id === 'PGI').gene_reaction_rule = 'b4025 or (b0001 and  b0002)';
  raw.genes = [{ id: 'b4025', name: 'pgi' }, { id: 'b0001', name: 'thrL' }, { id: 'b0002', name: 'thrA' }];
  // An objective given only as reaction coefficients stays that way.
  delete raw.objective;
  const community = new CommunityModel({
    species: [{ id: 'eco', model: new Model(raw), abundance: 1 }, { id: 'eco2', model: new Model(raw), abundance: 1 }],
  });
  const doc = communityModelDocument(community);

  // cobrapy reads a gene rule as Python, where `@` is an operator, so genes take a `:`.
  assert.equal(doc.reactions.find((r) => r.id === 'eco2@PGI').gene_reaction_rule, 'eco2:b4025 or (eco2:b0001 and  eco2:b0002)');
  assert.deepEqual(doc.genes.map((g) => g.id), ['eco:b4025', 'eco:b0001', 'eco:b0002', 'eco2:b4025', 'eco2:b0001', 'eco2:b0002']);

  const prepared = await prepareModelUploads([{ name: 'two.json', text: JSON.stringify(doc) }]);
  for (const entry of prepared.entries) {
    assert.equal(entry.model.reactions.get('PGI').geneReactionRule, 'b4025 or (b0001 and  b0002)');
    assert.deepEqual(entry.model.toJSON(), new Model(raw).toJSON());
  }
});

test('a merged model from a newer build is refused', async () => {
  const store = await threeSpeciesStore();
  const doc = JSON.parse(store.mergedModelJSON());
  doc.community.version = COMMUNITY_MODEL_VERSION + 1;
  await assert.rejects(prepareModelUploads([{ name: 'new.json', text: JSON.stringify(doc) }]), /version .* is not supported/);
});
