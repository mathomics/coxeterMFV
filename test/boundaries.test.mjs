/**
 * Boundary identity and transport detection.
 *
 * The thing under test is a claim about biology, not about strings: a reaction
 * crosses a membrane when its metabolites sit on both sides of it. Several of
 * these tests exist specifically to prove that no reaction name is consulted.
 */

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import { FluxProblem } from '../src/solver/fba.js';
import {
  compartmentSubtrees, reactionBoundaries, isTransportReaction, modelBoundaryIndex,
  communityBoundaries, reactionBoundaryKeys, reactionsByBoundary, reactionsCrossing,
  crossesEnvironment, communityReactionPlaces, modelReactionPlaces,
  boundaryKey, ENVIRONMENT_KEY, BOUNDARY_KIND,
} from '../src/map/boundaries.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

async function loadModel(name) {
  return new Model(JSON.parse(await readFile(join(root, 'models', `${name}.json`), 'utf8')));
}

async function buildCommunity(names = [['eco', 'ecoli_core_mc'], ['sce', 'yeast_core_mc']]) {
  const species = [];
  for (const [id, file] of names) {
    species.push({ id, model: await loadModel(file), abundance: 1 / names.length });
  }
  return new CommunityModel({ species, medium: { glc__D: { lower: -10 }, o2: { lower: -20 } } });
}

// Hierarchy

test('a compartment subtree is the compartment plus everything nested in it', async () => {
  const yeast = await loadModel('yeast_core_mc');
  const subtrees = compartmentSubtrees(yeast.compartmentTree());
  // e > c > im > m
  assert.deepEqual([...subtrees.get('m')].sort(), ['m']);
  assert.deepEqual([...subtrees.get('im')].sort(), ['im', 'm']);
  assert.deepEqual([...subtrees.get('c')].sort(), ['c', 'im', 'm']);
  assert.deepEqual([...subtrees.get('e')].sort(), ['c', 'e', 'im', 'm']);

  const ecoli = await loadModel('ecoli_core_mc');
  const eco = compartmentSubtrees(ecoli.compartmentTree());
  // e > p > c
  assert.deepEqual([...eco.get('c')].sort(), ['c']);
  assert.deepEqual([...eco.get('p')].sort(), ['c', 'p']);
  assert.deepEqual([...eco.get('e')].sort(), ['c', 'e', 'p']);
});

// Model-level crossing

test('an internal reaction crosses nothing', async () => {
  const model = await loadModel('ecoli_core_mc');
  const subtrees = compartmentSubtrees(model.compartmentTree());
  let checked = 0;
  for (const rxn of model.reactions.values()) {
    if (rxn.compartments.length !== 1) continue;
    assert.deepEqual(reactionBoundaries(rxn, model, subtrees), [],
      `${rxn.id} lives in ${rxn.compartments} yet was called transport`);
    assert.equal(isTransportReaction(rxn, model, subtrees), false, rxn.id);
    checked += 1;
  }
  assert.ok(checked > 40, `only checked ${checked} single-compartment reactions`);
});

test('transport detection agrees with the compartment span of every reaction', async () => {
  // An independent cross-check: a reaction spans more than one compartment
  // exactly when it crosses at least one membrane. Neither side reads a name.
  for (const name of ['ecoli_core_mc', 'yeast_core_mc', 'cyano_core_mc', 'lactobacillus_core_mc']) {
    const model = await loadModel(name);
    const subtrees = compartmentSubtrees(model.compartmentTree());
    for (const rxn of model.reactions.values()) {
      const spans = new Set(modelReactionPlaces(rxn, model).map((p) => p.compartment));
      assert.equal(isTransportReaction(rxn, model, subtrees), spans.size > 1,
        `${name}/${rxn.id} spans ${[...spans]} but crossing said otherwise`);
    }
  }
});

test('a reaction crosses the membrane of the compartment it actually leaves', async () => {
  const model = await loadModel('ecoli_core_mc');
  const subtrees = compartmentSubtrees(model.compartmentTree());
  const boundariesOf = (id) => reactionBoundaries(model.reactions.get(id), model, subtrees);

  // Nested e > p > c. A cytosol/periplasm carrier crosses the inner membrane.
  assert.deepEqual(boundariesOf('GLCptspp'), ['c']);
  assert.deepEqual(boundariesOf('CO2tpp'), ['c']);
  // A periplasm/extracellular porin crosses the outer one instead.
  assert.deepEqual(boundariesOf('GLCtex'), ['p']);
  assert.deepEqual(boundariesOf('CO2tex'), ['p']);
  // A purely cytosolic reaction crosses neither.
  assert.deepEqual(boundariesOf('PGI'), []);
});

test('one reaction can cross more than one boundary', async () => {
  const model = await loadModel('yeast_core_mc');
  const subtrees = compartmentSubtrees(model.compartmentTree());
  // Nested e > c > im > m. A cytosol/matrix carrier passes the inner membrane
  // and the intermembrane space's membrane, but never leaves the cytosol.
  for (const id of ['ATPtm', 'CO2tm', 'PYRt2m', 'O2tm']) {
    assert.deepEqual(reactionBoundaries(model.reactions.get(id), model, subtrees), ['im', 'm'],
      `${id} should cross both mitochondrial membranes`);
  }
  // Something that only reaches the intermembrane space crosses just one.
  const oneOnly = [...model.reactions.values()]
    .filter((r) => reactionBoundaries(r, model, subtrees).length === 1);
  assert.ok(oneOnly.length > 0, 'expected single-boundary transport as well');
});

test('the boundary index lists every compartment and nothing else', async () => {
  const model = await loadModel('ecoli_core_mc');
  const index = modelBoundaryIndex(model);
  assert.deepEqual([...index.keys()].sort(), ['c', 'e', 'p']);
  // The outermost compartment is not crossed from inside a single model: an
  // exchange there is a half-reaction with everything on one side.
  assert.deepEqual(index.get('e'), []);
  assert.ok(index.get('c').includes('GLCptspp'));
  assert.ok(index.get('p').includes('GLCtex'));
  assert.ok(!index.get('c').includes('PGI'));
  // Every listed reaction really exists, and the lists are sorted.
  for (const [, ids] of index) {
    assert.deepEqual(ids, [...ids].sort());
    for (const id of ids) assert.ok(model.reactions.has(id), `${id} is not in the model`);
  }
});

test('detection follows compartments, not reaction names', () => {
  // Deliberately misleading ids: the "transporter-looking" one is internal and
  // the innocuous one is the real carrier. Compartments decide.
  const model = new Model({
    id: 'trap',
    compartments: { c: 'Cytosol', e: 'Extracellular' },
    metabolites: [
      { id: 'a_c', compartment: 'c', formula: 'C', charge: 0 },
      { id: 'b_c', compartment: 'c', formula: 'C', charge: 0 },
      { id: 'a_e', compartment: 'e', formula: 'C', charge: 0 },
    ],
    reactions: [
      // Looks like a transporter by every naming convention. Is not one.
      { id: 'GLCt2pp', metabolites: { a_c: -1, b_c: 1 }, lower_bound: 0, upper_bound: 1000 },
      // Looks like an ordinary metabolic step. Actually crosses the membrane.
      { id: 'FOO', metabolites: { a_e: -1, a_c: 1 }, lower_bound: 0, upper_bound: 1000 },
      { id: 'BARtex', metabolites: { b_c: -1, a_c: 1 }, lower_bound: 0, upper_bound: 1000 },
    ],
  });
  const subtrees = compartmentSubtrees(model.compartmentTree());
  assert.deepEqual(reactionBoundaries(model.reactions.get('GLCt2pp'), model, subtrees), [],
    'a name that looks like transport must not make it transport');
  assert.deepEqual(reactionBoundaries(model.reactions.get('BARtex'), model, subtrees), []);
  assert.deepEqual(reactionBoundaries(model.reactions.get('FOO'), model, subtrees), ['c'],
    'a plain name must not hide real transport');
});

test('renaming every reaction changes no boundary answer', async () => {
  const original = await loadModel('yeast_core_mc');
  const doc = original.toJSON();
  // Scramble the ids into meaningless ones, keeping the stoichiometry.
  const rename = new Map([...original.reactions.keys()].map((id, i) => [id, `R${i}`]));
  doc.reactions = doc.reactions.map((r) => ({ ...r, id: rename.get(r.id) }));
  doc.objective = null;
  const renamed = new Model(doc);

  const a = compartmentSubtrees(original.compartmentTree());
  const b = compartmentSubtrees(renamed.compartmentTree());
  for (const [id, newId] of rename) {
    assert.deepEqual(
      reactionBoundaries(renamed.reactions.get(newId), renamed, b),
      reactionBoundaries(original.reactions.get(id), original, a),
      `${id} -> ${newId} changed its boundaries when only its name changed`,
    );
  }
});

// Community crossing

test('a community has one boundary per compartment per cell, plus the environment', async () => {
  const com = await buildCommunity();
  const list = communityBoundaries(com);
  const byId = new Map(list.map((b) => [b.id, b]));

  const env = byId.get(ENVIRONMENT_KEY);
  assert.ok(env, 'no environment boundary');
  assert.equal(env.kind, BOUNDARY_KIND.ENVIRONMENT);
  assert.equal(env.cellId, null);
  assert.equal(env.outside, null, 'the environment has nothing outside it');

  let expected = 1;
  for (const sp of com.activeSpecies) {
    for (const id of sp.model.compartmentIds()) {
      const b = byId.get(boundaryKey(sp.id, id));
      assert.ok(b, `no boundary for ${sp.id}/${id}`);
      assert.equal(b.cellId, sp.id);
      assert.equal(b.inside, id);
      expected += 1;
    }
  }
  assert.equal(list.length, expected, 'unexpected boundaries');

  // The outermost membrane of each cell is the one facing the shared medium.
  const outer = list.filter((b) => b.kind === BOUNDARY_KIND.CELL);
  assert.equal(outer.length, com.activeSpecies.length);
  for (const b of outer) {
    assert.equal(b.parentId, null);
    assert.equal(b.outside, 's');
  }
  // Every other cell boundary names a real parent compartment.
  for (const b of list.filter((k) => k.kind === BOUNDARY_KIND.MEMBRANE)) {
    assert.ok(b.parentId, `${b.id} has no parent`);
    assert.equal(b.outside, b.parentId);
  }
});

test('a transfer to the medium crosses exactly its cell boundary', async () => {
  const com = await buildCommunity();
  let checked = 0;
  for (const { reactionId, speciesId } of com.transferReactions()) {
    const keys = reactionBoundaryKeys(com, reactionId);
    assert.deepEqual(keys, [boundaryKey(speciesId, 'e')],
      `${reactionId} crossed ${keys} instead of its own cell boundary`);
    checked += 1;
  }
  assert.ok(checked > 10, `only checked ${checked} transfers`);
});

test('a community exchange crosses the environment boundary and nothing else', async () => {
  const com = await buildCommunity();
  let checked = 0;
  for (const [id, rxn] of com.merged.reactions) {
    const places = communityReactionPlaces(com, rxn);
    if (!crossesEnvironment(places)) continue;
    assert.deepEqual(reactionBoundaryKeys(com, id), [ENVIRONMENT_KEY], id);
    assert.ok(id.endsWith('_s'), `${id} was not expected to be a medium exchange`);
    checked += 1;
  }
  assert.ok(checked > 5, `only checked ${checked} community exchanges`);
});

test('internal reactions of a community are never classified as transport', async () => {
  const com = await buildCommunity();
  let checked = 0;
  for (const [id, rxn] of com.merged.reactions) {
    const owner = com.reactionOwner.get(id);
    if (!owner?.speciesId) continue;
    const local = com.species.find((s) => s.id === owner.speciesId).model.reactions.get(owner.originalId);
    if (!local || local.compartments.length !== 1 || local.kind === 'exchange') continue;
    assert.deepEqual(reactionBoundaryKeys(com, id), [], `${id} is internal to ${local.compartments[0]}`);
    checked += 1;
  }
  assert.ok(checked > 60, `only checked ${checked} internal reactions`);
});

test('the growth coupling pseudo-compartment is not mistaken for a place', async () => {
  const com = await buildCommunity();
  // Biomass reactions carry a metabolite in the virtual `link` compartment, and
  // COMMUNITY_GROWTH consists of nothing else. Neither is transport.
  assert.deepEqual(reactionBoundaryKeys(com, 'COMMUNITY_GROWTH'), []);
  assert.deepEqual(communityReactionPlaces(com, com.merged.reactions.get('COMMUNITY_GROWTH')), []);
  for (const [, bioId] of com.biomassIds) {
    assert.deepEqual(reactionBoundaryKeys(com, bioId), [], `${bioId} was called transport`);
  }
});

test('a cytosolic carrier crosses only the membranes between it and its target', async () => {
  const com = await buildCommunity();
  // E. coli: nested e > p > c, so the two stages of uptake cross different ones.
  assert.deepEqual(reactionBoundaryKeys(com, 'eco@GLCptspp'), [boundaryKey('eco', 'c')]);
  assert.deepEqual(reactionBoundaryKeys(com, 'eco@GLCtex'), [boundaryKey('eco', 'p')]);
  // Yeast: a mitochondrial carrier crosses two membranes at once.
  assert.deepEqual(reactionBoundaryKeys(com, 'sce@ATPtm'),
    [boundaryKey('sce', 'im'), boundaryKey('sce', 'm')]);
  // And a cell's membranes belong to that cell only.
  for (const key of reactionBoundaryKeys(com, 'sce@ATPtm')) {
    assert.ok(key.startsWith('sce/'), `${key} leaked into another cell`);
  }
});

test('reactionsByBoundary groups every crossing exactly once', async () => {
  const com = await buildCommunity();
  const grouped = reactionsByBoundary(com);

  // Every declared boundary is present, even when nothing crosses it.
  for (const b of communityBoundaries(com)) {
    assert.ok(grouped.has(b.id), `${b.id} missing from the grouping`);
  }
  // The grouping agrees with the per-reaction answer, in both directions.
  for (const id of com.merged.reactions.keys()) {
    for (const key of reactionBoundaryKeys(com, id)) {
      assert.ok(grouped.get(key).includes(id), `${id} missing from ${key}`);
    }
  }
  for (const [key, ids] of grouped) {
    assert.deepEqual(ids, [...ids].sort(), `${key} is not sorted`);
    assert.equal(new Set(ids).size, ids.length, `${key} lists a reaction twice`);
    for (const id of ids) {
      assert.ok(reactionBoundaryKeys(com, id).includes(key), `${id} should not be on ${key}`);
    }
  }
  assert.deepEqual(reactionsCrossing(com, boundaryKey('eco', 'c')), grouped.get(boundaryKey('eco', 'c')));
  assert.deepEqual(reactionsCrossing(com, 'nonsense/x'), []);
});

test('asking about boundaries does not disturb the model or the LP', async () => {
  const com = await buildCommunity();
  const before = new FluxProblem(com.merged).parsimonious();
  const bounds = [...com.merged.reactions.values()].map((r) => `${r.id}:${r.lowerBound}:${r.upperBound}`);

  reactionsByBoundary(com);
  communityBoundaries(com);
  for (const id of com.merged.reactions.keys()) reactionBoundaryKeys(com, id);

  const after = new FluxProblem(com.merged).parsimonious();
  assert.equal(after.status, before.status);
  assert.ok(Math.abs(after.objectiveValue - before.objectiveValue) < 1e-12,
    `${before.objectiveValue} -> ${after.objectiveValue}`);
  assert.deepEqual([...com.merged.reactions.values()].map((r) => `${r.id}:${r.lowerBound}:${r.upperBound}`),
    bounds, 'a reaction bound moved');
});
