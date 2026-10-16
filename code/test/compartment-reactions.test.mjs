/**
 * Compartment-local reaction discovery.
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import { reactionsInCompartment } from '../src/map/compartment-reactions.js';
import { reactionBoundaryKeys } from '../src/map/boundaries.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

async function community() {
  const raw = JSON.parse(await readFile(join(root, 'models', 'ecoli_core_mc.json'), 'utf8'));
  return new CommunityModel({
    species: [{ id: 'eco', model: new Model(raw), abundance: 1 }],
    medium: {},
  });
}

test('compartment discovery returns only reactions whose real metabolites all live there', async () => {
  const com = await community();
  for (const compartmentId of ['c', 'e']) {
    const ids = reactionsInCompartment(com, 'eco', compartmentId);
    for (const id of ids) {
      const rxn = com.merged.reactions.get(id);
      for (const mid of Object.keys(rxn.metabolites)) {
        const owner = com.metaboliteOwner.get(mid);
        if (!owner) continue;
        assert.equal(owner.speciesId, 'eco', id);
        assert.equal(owner.compartment, compartmentId, id);
      }
    }
  }
});

test('transport reactions are not offered by compartment discovery', async () => {
  const com = await community();
  const internal = new Set([
    ...reactionsInCompartment(com, 'eco', 'c'),
    ...reactionsInCompartment(com, 'eco', 'e'),
  ]);
  for (const id of internal) {
    assert.equal(reactionBoundaryKeys(com, id).length, 0, id);
  }
});

test('the cytoplasm exposes at least one internal reaction in the E. coli fixture', async () => {
  const com = await community();
  assert.ok(reactionsInCompartment(com, 'eco', 'c').length > 0);
});


test('virtual link reactions are not offered as compartment reactions', async () => {
  const com = await community();
  const ids = new Set([
    ...reactionsInCompartment(com, 'eco', 'c'),
    ...reactionsInCompartment(com, 'eco', 'e'),
  ]);

  for (const id of ids) {
    const rxn = com.merged.reactions.get(id);
    for (const mid of Object.keys(rxn.metabolites)) {
      const owner = com.metaboliteOwner.get(mid);
      assert.notEqual(owner?.compartment, 'link', id);
    }
  }
});
