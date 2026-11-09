import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Model } from '../src/core/model.js';
import { Store } from '../src/ui/store.js';
import {
  MAP_DOCUMENT_FORMAT,
  MAP_DOCUMENT_VERSION,
  createMapDocument,
  parseMapDocument,
  validateMapContext,
} from '../src/map/map-document.js';

function fakeMap() {
  return {
    reactions: new Map([['r1', { id: 'r1' }]]),
    nodes: new Map([['n1', { id: 'n1' }]]),
    segments: [{ reactionId: 'r1' }],
    toJSON() {
      return {
        version: 1,
        cells: [{ id: 'eco' }],
        reactions: [{ id: 'r1' }],
        nodes: [{ id: 'n1' }],
        segments: [{ reactionId: 'r1' }],
      };
    },
  };
}

test('V0.8 creates a versioned EscherMCMCMap document', () => {
  const doc = createMapDocument(fakeMap(), {
    scenario: 'demo',
    species: [{ id: 'eco', file: 'ecoli_core_mc', enabled: true }],
  });

  assert.equal(doc.format, MAP_DOCUMENT_FORMAT);
  assert.equal(doc.version, MAP_DOCUMENT_VERSION);
  assert.equal(doc.metadata.scenario, 'demo');
  assert.deepEqual(doc.metadata.species, [{ id: 'eco', model: 'ecoli_core_mc' }]);
  assert.equal(doc.metadata.reactionCount, 1);
  assert.ok(doc.map);
});

test('V0.8 parses its own document format', () => {
  const created = createMapDocument(fakeMap(), {
    species: [{ id: 'eco', file: 'ecoli_core_mc', enabled: true }],
  });
  const parsed = parseMapDocument(JSON.stringify(created));

  assert.equal(parsed.legacy, false);
  assert.equal(parsed.version, MAP_DOCUMENT_VERSION);
  assert.deepEqual(parsed.map, created.map);
});

test('legacy raw EditableMap JSON remains loadable', () => {
  const raw = fakeMap().toJSON();
  const parsed = parseMapDocument(JSON.stringify(raw));

  assert.equal(parsed.legacy, true);
  assert.equal(parsed.format, 'EditableMap');
  assert.deepEqual(parsed.map, raw);
});

test('future unsupported document versions are rejected', () => {
  assert.throws(
    () => parseMapDocument({
      format: MAP_DOCUMENT_FORMAT,
      version: MAP_DOCUMENT_VERSION + 1,
      map: {},
    }),
    /only understands up to version/,
  );
});

test('map context catches a different biological model', () => {
  const parsed = parseMapDocument(createMapDocument(fakeMap(), {
    species: [{ id: 'eco', file: 'ecoli_core_mc', enabled: true }],
  }));

  assert.throws(
    () => validateMapContext(parsed, {
      species: [{ id: 'eco', file: 'another_model', enabled: true }],
    }),
    /different model/,
  );
});

test('map context accepts the same active species and model', () => {
  const parsed = parseMapDocument(createMapDocument(fakeMap(), {
    species: [{ id: 'eco', file: 'ecoli_core_mc', enabled: true }],
  }));

  assert.equal(validateMapContext(parsed, {
    species: [{ id: 'eco', file: 'ecoli_core_mc', enabled: true }],
  }), true);
});

test('a published community map opens whichever build saved it', () => {
  /*
   * A member of a published community is served from two files: the full model
   * and the map excerpt. Which one the runtime loads depends on the build — the
   * single-file one embeds only the excerpts — so the same drawing records
   * `preview-Ec1` when saved there and `model-Ec1` when saved from a server.
   *
   * Comparing the file names refused one of those two, which is a map that the
   * app itself had just written being rejected by the app that wrote it.
   */
  const saved = JSON.stringify({
    format: 'EscherMCMCMap',
    version: 3,
    metadata: {
      scenario: 'published:steadycom-ecoli-auxotrophs',
      species: ['Ec1', 'Ec2'].map((id) => ({
        id, model: `published/steadycom-ecoli-auxotrophs/preview-${id}`,
      })),
    },
    map: { version: 1, canvas: { width: 100, height: 100 }, cells: [], nodes: [], reactions: [] },
  });
  const parsed = parseMapDocument(saved);

  for (const kind of ['preview', 'model']) {
    assert.equal(validateMapContext(parsed, {
      species: ['Ec1', 'Ec2'].map((id) => ({
        id, file: `published/steadycom-ecoli-auxotrophs/${kind}-${id}`, enabled: true,
      })),
    }), true, `a map saved as preview must open against ${kind}`);
  }

  // The guard still does its job: another community is still another community.
  assert.throws(() => validateMapContext(parsed, {
    species: ['Ec1', 'Ec2'].map((id) => ({
      id, file: `published/steadycom-gut-nine/preview-${id}`, enabled: true,
    })),
  }), /different model/);
});

// Opening a map from another page

const docRoot = join(dirname(fileURLToPath(import.meta.url)), '..');

async function twoScenarioStore() {
  const store = new Store('.');
  for (const name of ['ecoli_core_mc', 'yeast_core_mc']) {
    store.models.set(name, new Model(JSON.parse(await readFile(join(docRoot, 'models', `${name}.json`), 'utf8'))));
  }
  store.scenarios = [
    {
      id: 'just-ecoli', name: 'Just E. coli', medium: { glc__D: 10, o2: 20 },
      species: [{ id: 'eco', model: 'ecoli_core_mc' }],
    },
    {
      id: 'just-yeast', name: 'Just yeast', medium: { glc__D: 10, o2: 20 },
      species: [{ id: 'sce', model: 'yeast_core_mc' }],
    },
  ];
  return store;
}

test('a map opens in the scenario it was saved in, not only the active one', async () => {
  /*
   * Save a map, close the page, open it again: the app is back on its default
   * scenario, and the file belongs to another one. Refusing it there would make
   * saving a map useless across sessions.
   */
  const store = await twoScenarioStore();
  await store.useScenario('just-yeast');
  const drawn = store.map.reactions.size;
  const saved = store.mapJSON();

  await store.useScenario('just-ecoli');
  await store.loadMapDocument(saved);

  assert.equal(store.scenario.id, 'just-yeast', 'the saved scenario is restored');
  assert.equal(store.map.reactions.size, drawn, 'the drawing arrives intact');
  assert.equal(store.species[0].id, 'sce', 'and the model underneath it is the right one');
});

test('a map that fits the open scenario loads without moving anywhere', async () => {
  const store = await twoScenarioStore();
  await store.useScenario('just-ecoli');
  const saved = store.mapJSON();
  const rebuilds = [];
  store.subscribe((_, reason) => rebuilds.push(reason));

  await store.loadMapDocument(saved);

  assert.equal(store.scenario.id, 'just-ecoli');
  assert.ok(!rebuilds.includes('rebuild'), 'no scenario switch happens when none is needed');
});

test('a map whose scenario this build does not have waits, and says which', async () => {
  const store = await twoScenarioStore();
  await store.useScenario('just-yeast');
  const saved = JSON.parse(store.mapJSON());
  saved.metadata.scenario = 'a-scenario-that-went-away';

  await store.useScenario('just-ecoli');
  const open = JSON.stringify(store.map.toJSON());
  assert.equal(await store.loadMapDocument(JSON.stringify(saved)), null);
  assert.match(store.waitingMap.reason, /a-scenario-that-went-away/);
  assert.equal(store.scenario.id, 'just-ecoli', 'and nothing is disturbed on the way');
  assert.equal(JSON.stringify(store.map.toJSON()), open);
});
