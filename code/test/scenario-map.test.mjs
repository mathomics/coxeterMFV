/**
 * A scenario that names a curated map (`map`, a path under models/) opens with
 * it; without one, or when the file is not there, the automatic layout draws it.
 * A published community whose manifest names a map opens with that map.
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { Store } from '../src/ui/store.js';
import { Model } from '../src/core/model.js';
import { createMapDocument } from '../src/map/map-document.js';

const demoModel = {
  id: 'demo',
  compartments: { c: 'Cytosol', e: 'Extracellular' },
  metabolites: [{ id: 'a_e', compartment: 'e' }, { id: 'a_c', compartment: 'c' }],
  reactions: [
    { id: 'EX_a_e', metabolites: { a_e: -1 }, lower_bound: -10, upper_bound: 1000 },
    { id: 'At', metabolites: { a_e: -1, a_c: 1 }, lower_bound: 0, upper_bound: 1000 },
    { id: 'BIOMASS', metabolites: { a_c: -1 }, lower_bound: 0, upper_bound: 1000, objective_coefficient: 1 },
  ],
};

test('a scenario opens with its curated map, and with the automatic layout without one', async (t) => {
  const files = new Map();
  const previousFetch = globalThis.fetch;
  globalThis.fetch = async (url) => {
    const text = files.get(String(url));
    return text === undefined
      ? { ok: false, status: 404 }
      : { ok: true, status: 200, json: async () => JSON.parse(text) };
  };
  t.after(() => { globalThis.fetch = previousFetch; });

  const store = new Store('.');
  store.models.set('demo', new Model(demoModel));
  store.scenarios = [{
    id: 'demo', name: 'Demo', objective: 'equal-growth',
    species: [{ id: 'd', model: 'demo', abundance: 1 }],
    medium: { a: 10 },
  }];

  await store.useScenario('demo');
  const automatic = store.map.reactionIds().sort();
  assert.ok(automatic.length > 1);
  assert.equal(store.scenarioMapFile, null);

  const kept = automatic[0];
  const drawing = createMapDocument(store.map.subsetByReactions([kept]), { species: store.species, scenario: 'demo' });
  files.set('./models/maps/demo.json', JSON.stringify(drawing));
  store.scenarios[0].map = 'maps/demo.json';
  await store.useScenario('demo');
  assert.deepEqual(store.map.reactionIds(), [kept]);
  assert.equal(store.scenarioMapFile, 'maps/demo.json');
  assert.equal(store.mapLoadNote, '', 'the curated map fits its scenario as it is');
  assert.equal(store.mapHistory.canUndo, false, 'and opens unedited');
  assert.equal(store.solution?.status, 'optimal');

  store.scenarios[0].map = 'maps/missing.json';
  await store.useScenario('demo');
  assert.deepEqual(store.map.reactionIds().sort(), automatic);
  assert.equal(store.scenarioMapFile, null);
});

test('a published community whose manifest names a map opens with it and keeps it when a member is switched off', async (t) => {
  const previousFetch = globalThis.fetch;
  globalThis.fetch = async (url) => {
    try {
      const text = await readFile(new URL(`../${String(url).replace(/^\.\//, '')}`, import.meta.url), 'utf8');
      return { ok: true, status: 200, json: async () => JSON.parse(text), text: async () => text };
    } catch {
      return { ok: false, status: 404 };
    }
  };
  t.after(() => { globalThis.fetch = previousFetch; });

  const id = 'stolyar-2007-syntrophy';
  const drawn = JSON.parse(await readFile(new URL(`../models/published/${id}/map.json`, import.meta.url), 'utf8'));
  const store = new Store('.');
  store.publishedCommunities = [{ id, name: 'Stolyar et al. (2007)', description: '' }];
  await store.usePublishedCommunity(id);
  assert.equal(store.map.reactions.size, drawn.map.reactions.length);
  assert.equal(store.map.texts.size, drawn.map.texts.length, 'the curated map, not the automatic layout');
  assert.equal(store.mapLoadNote, '', 'it fits the member models as they are');
  assert.equal(store.solution?.status, 'optimal');

  await store.setSpeciesEnabled('Mm', false);
  assert.equal(store.map.reactions.size, drawn.map.reactions.length, 'switching a member off keeps the map');
});
