/** Private-case imports validate first, and leaving them retires pending loaders and case views. */
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import factory from '../assets/vendor/glpk.js';
import { initGLPK } from '../src/solver/glpk.js';
import { Model } from '../src/core/model.js';
import { Store } from '../src/ui/store.js';
import { ZUNIGA_ID } from '../src/communities/zuniga.js';
import { sha256 } from '../src/core/reproducibility.js';
import { zunigaCoreFixture } from './helpers/zuniga-core-fixture.mjs';

test.before(async () => {
  globalThis.__GLPK_FACTORY__ = factory;
  globalThis.__GLPK_WASM__ = await readFile(new URL('../assets/vendor/glpk.wasm', import.meta.url));
  assert.equal(await initGLPK(), true);
});

async function ordinaryStore() {
  const raw = JSON.parse(await readFile(new URL('../models/ecoli_core_mc.json', import.meta.url), 'utf8'));
  const store = new Store('.');
  store.models.set('ecoli_core_mc', new Model(raw));
  store.scenarios = JSON.parse(await readFile(new URL('../models/scenarios.json', import.meta.url), 'utf8')).scenarios;
  await store.useScenario('solo-ecoli');
  return store;
}

async function privateStore() {
  const store = await ordinaryStore();
  const raw = store.models.get('ecoli_core_mc').toJSON();
  const fixture = zunigaCoreFixture(raw);
  store.publishedManifests.set(ZUNIGA_ID, fixture.study.manifest);
  globalThis.__ESCHER_MCMC_DATA__ = { [`models/published/${ZUNIGA_ID}/map.json`]: fixture.map.toJSON() };
  await store.useZunigaStudy({ study: fixture.study });
  return store;
}

test('loading an ordinary Scenario retires the previous private case and restores its view', async () => {
  const ordinary = await ordinaryStore();
  const store = await privateStore();
  store.setView({ showCrossFeeding: true });
  await store.loadState(ordinary.stateJSON());
  assert.equal(store.isZunigaStudy, false);
  assert.equal(store.zunigaStudy, null);
  assert.equal(store.zunigaPrepared, null);
  assert.equal(store.zunigaSolutions.size, 0);
  assert.equal(store.view.showCrossFeeding, false);
  assert.equal(store.solution.zuniga, undefined);
  const objective = store.solution.objectiveValue;
  store.setMediumLimit('glc__D', 5);
  assert.ok(store.solution.objectiveValue < objective);
});

test('a missing ordinary model leaves the private case available and unchanged', async () => {
  const ordinary = await ordinaryStore();
  const state = JSON.parse(ordinary.stateJSON());
  state.species[0].file = 'missing-private-context-model';
  const store = await privateStore();
  const solution = store.solution;
  const study = store.zunigaStudy;
  await assert.rejects(store.loadState(JSON.stringify(state)));
  assert.equal(store.zunigaStudy, study);
  assert.equal(store.solution, solution);
  assert.equal(store.isZunigaStudy, true);
});

test('an ordinary Scenario supersedes a pending private load and its late progress', async () => {
  const ordinary = await ordinaryStore();
  const store = await privateStore();
  const study = store.zunigaStudy;
  let release;
  let progress;
  let started;
  const ready = new Promise(resolve => {
    started = resolve;
  });
  store.loadZunigaStudy = (_manifest, options) => {
    progress = options.onProgress;
    started();
    return new Promise(resolve => {
      release = resolve;
    });
  };
  const pending = store.useZunigaStudy();
  await ready;
  await store.loadState(ordinary.stateJSON());
  const solution = store.solution;
  const message = store.status.message;
  progress('Late private download progress');
  release(study);
  await assert.rejects(pending, { name: 'AbortError' });
  assert.equal(store.solution, solution);
  assert.equal(store.status.message, message);
  assert.equal(store.scenario.id, 'solo-ecoli');
});

test('invalid private Scenario metadata is refused before downloading or replacing ordinary work', async () => {
  const source = await privateStore();
  const valid = JSON.parse(source.stateJSON());
  for (const patch of [{ comparison: 42 }, { scenarioId: 'invented-case' }, { comparison: 'invented-comparison' }, { fluxReference: 999 }]) {
    const store = await ordinaryStore();
    store.publishedManifests.set(ZUNIGA_ID, source.zunigaStudy.manifest);
    let downloads = 0;
    store.loadZunigaStudy = async () => {
      downloads += 1;
      return source.zunigaStudy;
    };
    const solution = store.solution;
    const state = JSON.stringify({ ...valid, study: { ...valid.study, ...patch } });
    await assert.rejects(store.loadState(state));
    assert.equal(downloads, 0);
    assert.equal(store.solution, solution);
    assert.equal(store.scenario.id, 'solo-ecoli');
  }
});

test('invalid private Map metadata is refused before downloading or replacing ordinary work', async () => {
  const source = await privateStore();
  const map = source.mapDocument();
  map.study.scenarioId = 'invented-case';
  const store = await ordinaryStore();
  store.publishedManifests.set(ZUNIGA_ID, source.zunigaStudy.manifest);
  let downloads = 0;
  store.loadZunigaStudy = async () => {
    downloads += 1;
    return source.zunigaStudy;
  };
  const solution = store.solution;
  const drawing = store.map;
  await assert.rejects(store.loadMapDocument(JSON.stringify(map)));
  assert.equal(downloads, 0);
  assert.equal(store.solution, solution);
  assert.equal(store.map, drawing);
});

test('a private bundle with an unusable source LP is refused before replacing ordinary work', async () => {
  const source = await privateStore();
  for (const includeMap of [true, false]) {
    const bundle = await source.reproducibilityBundle();
    bundle.study.sourceModels.mono.reactions.find(reaction => reaction.id === 'PPNDH').gene_reaction_rule = '';
    if (!includeMap) {
      bundle.map = null;
    }
    delete bundle.integrity;
    bundle.integrity = { algorithm: 'SHA-256', payloadHash: await sha256(bundle) };
    const store = await ordinaryStore();
    const solution = store.solution;
    const drawing = store.map;
    await assert.rejects(store.importReproducibilityBundle(bundle), /PPNDH knockout/);
    assert.equal(store.solution, solution);
    assert.equal(store.map, drawing);
    assert.equal(store.scenario.id, 'solo-ecoli');
  }
});

test('valid private bundles restore their exact source LP and permit a later ordinary Scenario', async () => {
  const source = await privateStore();
  source.setZunigaComparison('difference');
  const bundle = await source.reproducibilityBundle();
  const store = await ordinaryStore();
  const ordinary = store.stateJSON();
  const check = await store.importReproducibilityBundle(bundle);
  assert.equal(check.replay.pass, true);
  assert.equal(store.problem.rxnIds.length, source.problem.rxnIds.length);
  assert.equal(store.zunigaComparison, 'difference');
  await store.loadState(ordinary);
  assert.equal(store.solution.status, 'optimal');
  assert.equal(store.scenario.id, 'solo-ecoli');
});

test('valid private Scenario and Map metadata load from an ordinary context', async () => {
  const source = await privateStore();
  source.useZunigaScenario('mutant-monoculture');
  source.setZunigaComparison('difference');
  for (const format of ['scenario', 'map']) {
    const store = await ordinaryStore();
    store.publishedManifests.set(ZUNIGA_ID, source.zunigaStudy.manifest);
    let downloads = 0;
    store.loadZunigaStudy = async () => {
      downloads += 1;
      return source.zunigaStudy;
    };
    if (format === 'scenario') {
      await store.loadState(source.stateJSON());
    } else {
      await store.loadMapDocument(source.mapJSON());
    }
    assert.equal(downloads, 1);
    assert.equal(store.isZunigaStudy, true);
    assert.equal(store.zunigaScenarioId, 'mutant-monoculture');
    assert.equal(store.zunigaComparison, 'difference');
    assert.equal(store.solution.status, 'optimal');
  }
});

test('a private source LP that is infeasible is refused before replacing ordinary work', async () => {
  const source = await privateStore();
  const bundle = await source.reproducibilityBundle();
  for (const reaction of bundle.study.sourceModels.mono.reactions) {
    if (reaction.id.startsWith('EX_') && reaction.lower_bound < 0) {
      reaction.lower_bound = 0;
    }
  }
  delete bundle.integrity;
  bundle.integrity = { algorithm: 'SHA-256', payloadHash: await sha256(bundle) };
  const store = await ordinaryStore();
  const solution = store.solution;
  const drawing = store.map;
  let failure;
  try {
    await store.importReproducibilityBundle(bundle);
  } catch (error) {
    failure = error;
  }
  assert.ok(failure);
  assert.equal(store.solution, solution);
  assert.equal(store.map, drawing);
  assert.equal(store.scenario.id, 'solo-ecoli');
  assert.match(failure.message, /infeasible/);
});
