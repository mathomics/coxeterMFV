/** Model downloads must not replace newer contexts or install out-of-date responses. */
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { Store } from '../src/ui/store.js';
import { Model } from '../src/core/model.js';
import { toSBML } from '../src/core/sbml-write.js';
import { YEAST9_SOURCE } from '../src/core/yeast9.js';

async function makeStore() {
  const store = new Store('.');
  for (const id of ['ecoli_core_mc', 'yeast_core_mc']) {
    const raw = JSON.parse(await readFile(new URL(`../models/${id}.json`, import.meta.url), 'utf8'));
    store.models.set(id, new Model(raw));
  }
  store.scenarios = JSON.parse(await readFile(new URL('../models/scenarios.json', import.meta.url), 'utf8')).scenarios;
  await store.useScenario('solo-ecoli');
  return store;
}

function fixtureScenario(id = 'download-fixture') {
  return {
    id,
    name: 'Bundled core download fixture',
    loader: 'yeast9',
    objective: 'sum-biomass',
    species: [{ id: 'sce', model: 'yeast9-9-1-1', abundance: 1 }],
  };
}

function holdDownload(store, status = 200, stage = 'headers') {
  const fetch = globalThis.fetch;
  const xml = toSBML(store.models.get('ecoli_core_mc'));
  let release;
  const held = new Promise(resolve => {
    release = resolve;
  });
  let started;
  const ready = new Promise(resolve => {
    started = resolve;
  });
  let calls = 0;
  globalThis.fetch = async (url, ...options) => {
    if (url !== YEAST9_SOURCE.url) {
      return fetch(url, ...options);
    }
    calls += 1;
    const call = calls;
    if (call === 1 && stage === 'headers') {
      started();
      await held;
    }
    let responseStatus = 200;
    if (call === 1) {
      responseStatus = status;
    }
    if (responseStatus === 0) {
      throw new Error('Deliberate transport failure');
    }
    return {
      ok: responseStatus === 200,
      status: responseStatus,
      async text() {
        if (call === 1 && stage === 'body') {
          started();
          await held;
        }
        return xml;
      },
    };
  };
  return {
    release,
    ready,
    restore() {
      release();
      globalThis.fetch = fetch;
    },
  };
}

function download(store, scenario = fixtureScenario()) {
  return store.useYeast9Scenario(scenario).then(value => ({ value }), error => ({ error }));
}

test('a current downloaded core fixture still installs and leaves no old numerical result', async () => {
  const store = await makeStore();
  const held = holdDownload(store);
  try {
    const pending = download(store);
    held.release();
    const outcome = await pending;
    assert.equal(outcome.error, undefined);
    assert.equal(store.scenario.id, 'download-fixture');
    assert.equal(store.species[0].file, 'yeast9-9-1-1');
    assert.equal(store.solution, null);
  } finally {
    held.restore();
  }
});

for (const context of ['scenario', 'Scenario file', 'model file']) {
  test(`a late model download cannot replace a newer ${context}`, async () => {
    const store = await makeStore();
    const held = holdDownload(store);
    try {
      const pending = download(store);
      if (context === 'scenario') {
        await store.useScenario('aerobio-mixto');
      } else if (context === 'Scenario file') {
        const state = JSON.parse(store.stateJSON());
        state.medium.glc__D = 3;
        await store.loadState(JSON.stringify(state));
      } else {
        const raw = store.models.get('ecoli_core_mc').toJSON();
        raw.id = 'replacement-file';
        await store.importModelFiles([{ name: 'replacement.json', text: JSON.stringify(raw) }]);
      }
      const scenario = store.scenario;
      const solution = store.solution;
      const map = store.map;
      const status = { ...store.status };
      held.release();
      const outcome = await pending;
      assert.equal(outcome.error?.name, 'AbortError');
      assert.equal(store.scenario, scenario);
      assert.equal(store.solution, solution);
      assert.equal(store.map, map);
      assert.deepEqual(store.status, status);
    } finally {
      held.restore();
    }
  });
}

test('a late download failure is superseded rather than reported against the newer context', async () => {
  const store = await makeStore();
  const held = holdDownload(store, 503);
  try {
    const pending = download(store);
    await store.useScenario('aerobio-mixto');
    const status = { ...store.status };
    held.release();
    const outcome = await pending;
    assert.equal(outcome.error?.name, 'AbortError');
    assert.deepEqual(store.status, status);
  } finally {
    held.restore();
  }
});

test('a newer model download keeps its authority when the older response arrives last', async () => {
  const store = await makeStore();
  const held = holdDownload(store);
  try {
    const older = download(store, fixtureScenario('older-download'));
    const newer = await download(store, fixtureScenario('newer-download'));
    assert.equal(newer.error, undefined);
    const map = store.map;
    held.release();
    const outcome = await older;
    assert.equal(outcome.error?.name, 'AbortError');
    assert.equal(store.scenario.id, 'newer-download');
    assert.equal(store.map, map);
  } finally {
    held.restore();
  }
});

test('a rejected Scenario preload leaves the current model download eligible to finish', async () => {
  const store = await makeStore();
  const held = holdDownload(store);
  try {
    const pending = download(store);
    const state = JSON.parse(store.stateJSON());
    state.species[0].file = 'missing-download-order-model';
    const loadModel = store.loadModel;
    store.loadModel = async () => {
      throw new Error('Missing download-order model');
    };
    await assert.rejects(store.loadState(JSON.stringify(state)), /Missing download-order model/);
    store.loadModel = loadModel;
    held.release();
    const outcome = await pending;
    assert.equal(outcome.error, undefined);
    assert.equal(store.scenario.id, 'download-fixture');
  } finally {
    held.restore();
  }
});

test('a late response body cannot install after its context was replaced', async () => {
  const store = await makeStore();
  const held = holdDownload(store, 200, 'body');
  try {
    const pending = download(store);
    await held.ready;
    await store.useScenario('aerobio-mixto');
    const map = store.map;
    held.release();
    const outcome = await pending;
    assert.equal(outcome.error?.name, 'AbortError');
    assert.equal(store.scenario.id, 'aerobio-mixto');
    assert.equal(store.map, map);
  } finally {
    held.restore();
  }
});

test('a late transport failure is superseded without changing the newer status', async () => {
  const store = await makeStore();
  const held = holdDownload(store, 0);
  try {
    const pending = download(store);
    await store.useScenario('aerobio-mixto');
    const status = { ...store.status };
    held.release();
    const outcome = await pending;
    assert.equal(outcome.error?.name, 'AbortError');
    assert.deepEqual(store.status, status);
  } finally {
    held.restore();
  }
});

test('a failed current download preserves a deliberately cleared solution', async () => {
  for (const catalogue of [false, true]) {
    const store = await makeStore();
    store.clearSolution();
    store.publishedCommunityId = 'previous-published-fixture';
    store.scenario = { ...store.scenario, published: 'previous-published-fixture' };
    store.steadyComConfig.parsimonious = true;
    const map = store.map;
    const held = holdDownload(store, 503);
    try {
      let pending;
      if (catalogue) {
        store.scenarios.push(fixtureScenario());
        pending = store.useScenario('download-fixture').then(value => ({ value }), error => ({ error }));
      } else {
        pending = download(store);
      }
      held.release();
      const outcome = await pending;
      assert.match(outcome.error?.message, /HTTP 503/);
      assert.equal(store.solutionCleared, true);
      assert.equal(store.solution, null);
      assert.equal(store.map, map);
      assert.equal(store.publishedCommunityId, 'previous-published-fixture');
      assert.equal(store.steadyComConfig.parsimonious, true);
    } finally {
      held.restore();
    }
  }
});

test('ordinary model imports and downloads clear the previous published identity and formulation settings', async () => {
  for (const route of ['model file', 'download']) {
    const store = await makeStore();
    store.publishedCommunityId = 'previous-published-fixture';
    store.scenario = { ...store.scenario, published: 'previous-published-fixture' };
    store.steadyComConfig.parsimonious = true;
    if (route === 'model file') {
      const raw = store.models.get('ecoli_core_mc').toJSON();
      await store.importModelFiles([{ name: 'ordinary-model.json', text: JSON.stringify(raw) }]);
    } else {
      const held = holdDownload(store);
      try {
        const pending = download(store);
        held.release();
        assert.equal((await pending).error, undefined);
      } finally {
        held.restore();
      }
    }
    assert.equal(store.publishedCommunityId, null);
    assert.equal(store.scenario.published, undefined);
    assert.deepEqual(store.steadyComConfig, { fiber: null, parsimonious: false });
  }
});
