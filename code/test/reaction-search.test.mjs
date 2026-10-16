import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import { EditableMap } from '../src/map/editable-map.js';
import { Store } from '../src/ui/store.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

async function setup() {
  const raw = JSON.parse(await readFile(join(root, 'models', 'ecoli_core_mc.json'), 'utf8'));
  const com = new CommunityModel({
    species: [{ id: 'eco', model: new Model(raw), abundance: 1 }],
    medium: {},
  });
  const store = new Store('.');
  store.community = com;
  store.map = EditableMap.emptyFromCommunity(com);
  return { store, com };
}

test('reaction search finds an exact reaction ID first', async () => {
  const { store } = await setup();
  const results = store.searchReactions('TALA');
  assert.ok(results.length > 0);
  assert.equal(results[0].localId, 'TALA');
  assert.equal(results[0].reactionId, 'eco@TALA');
  assert.equal(results[0].score, 0);
});

test('reaction search is case-insensitive and searches reaction names', async () => {
  const { store } = await setup();
  const results = store.searchReactions('transaldolase');
  assert.ok(results.some((r) => r.reactionId === 'eco@TALA'));
});

test('reaction search searches subsystem text', async () => {
  const { store } = await setup();
  const results = store.searchReactions('pentose phosphate');
  assert.ok(results.some((r) => r.reactionId === 'eco@TALA'));
});

test('reaction search reports whether a result is already drawn', async () => {
  const { store, com } = await setup();
  let tala = store.searchReactions('TALA')[0];
  assert.equal(tala.drawn, false);

  assert.ok(store.map.addReaction(com, 'eco@TALA'));
  tala = store.searchReactions('TALA')[0];
  assert.equal(tala.drawn, true);
});

test('reaction search respects its result limit', async () => {
  const { store } = await setup();
  const results = store.searchReactions('a', { limit: 5 });
  assert.ok(results.length <= 5);
});

test('empty search does not enumerate the full model', async () => {
  const { store } = await setup();
  assert.deepEqual(store.searchReactions(''), []);
  assert.deepEqual(store.searchReactions('   '), []);
});
