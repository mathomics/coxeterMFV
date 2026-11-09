/**
 * Bundles keep one coherent model and drawing snapshot. Malformed archives are
 * rejected before replacing the open models, solution or editable drawing.
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { Model } from '../src/core/model.js';
import { Store } from '../src/ui/store.js';
import { sha256, verifyReproducibilityBundle } from '../src/core/reproducibility.js';

async function makeStore() {
  const raw = JSON.parse(await readFile(new URL('../models/ecoli_core_mc.json', import.meta.url), 'utf8'));
  const scenarios = JSON.parse(await readFile(new URL('../models/scenarios.json', import.meta.url), 'utf8'));
  const store = new Store('.');
  store.models.set('ecoli_core_mc', new Model(raw));
  store.scenarios = scenarios.scenarios;
  await store.useScenario('solo-ecoli');
  return store;
}

function drawingState(store) {
  return {
    map: store.map.toJSON(),
    presentation: {
      mapMode: store.mapMode, pathwayManager: store.pathwayManagerState(),
      fluxColorStops: store.view.fluxColorStops, absoluteFluxColors: store.absoluteFluxColors(),
      pathwayRibbonMode: store.view.pathwayRibbonMode, reactionColorMode: store.view.reactionColorMode,
      showCrossFeeding: store.view.showCrossFeeding, showCurrency: store.view.showCurrency,
    },
    layout: store.layoutSettings,
    widthByFlux: store.view.widthByFlux,
    dashNoFlux: store.view.dashNoFlux,
    lineHops: store.view.lineHops,
    crossFeedingWidth: store.view.crossFeedingWidth,
    ribbonWidth: store.view.pathwayRibbonWidth,
    ribbonOpacity: store.view.pathwayRibbonOpacity,
    omics: store.omicsState(),
  };
}

async function seal(bundle) {
  bundle.modelHashes = {};
  for (const [id, model] of Object.entries(bundle.models)) {
    bundle.modelHashes[id] = await sha256(model);
  }
  delete bundle.integrity;
  bundle.integrity = { algorithm: 'SHA-256', payloadHash: await sha256(bundle) };
  return bundle;
}

async function refuseWithoutChanges(store, bundle) {
  const before = {
    state: store.stateJSON(), map: store.map, problem: store.problem, solution: store.solution,
    keys: [...store.models.keys()], uploaded: [...store.uploadedModelKeys],
    history: store.mapHistory.undoStack.length, cleared: store.solutionCleared, unsaved: store.unsavedWork(),
  };
  await assert.rejects(store.importReproducibilityBundle(await seal(bundle)), /bundle|model|species|medium|bounds|map|GIMME|method|tolerance|palette|colou?r|version|presentation/i);
  assert.ok(store.stateJSON() === before.state, 'the scenario settings are preserved');
  assert.equal(store.map, before.map);
  assert.equal(store.problem, before.problem);
  assert.equal(store.solution, before.solution);
  assert.deepEqual([...store.models.keys()], before.keys);
  assert.deepEqual([...store.uploadedModelKeys], before.uploaded);
  assert.equal(store.mapHistory.undoStack.length, before.history);
  assert.equal(store.solutionCleared, before.cleared);
  assert.deepEqual(store.unsavedWork(), before.unsaved);
}

async function editableStore() {
  const store = await makeStore();
  const text = store.modelJSON();
  await store.importModelFiles([{ name: 'current.json', text }]);
  store.addMapText('Keep this drawing', 300, 300);
  store.clearSolution();
  return store;
}

test('a bundle restores drawing palettes, pathways, layout and map omics', async () => {
  const source = await makeStore();
  source.setLayoutPreset('compact');
  source.createManualPathway({ name: 'Saved route', color: '#123456', reactionIds: ['eco@TALA'] });
  source.setFluxColorStops([{ value: 0, color: '#ffffff' }, { value: 10, color: '#0000ff' }]);
  source.restoreAbsoluteFluxColors({ stops: [{ value: 0, color: '#ffffff' }, { value: 10, color: '#00ff00' }], log: true });
  source.setView({ reactionColorMode: 'absflux', pathwayRibbonMode: 'pathway', pathwayRibbonWidth: 72,
    pathwayRibbonOpacity: 0.3, widthByFlux: true, dashNoFlux: false, lineHops: false,
    crossFeedingWidth: 1.5, showCrossFeeding: true, showCurrency: false });
  source.importOmicsCSV('species,gene,Control,Treatment\neco,b0008,1,4\n');
  source.addMapText('Archived drawing', 300, 300);
  const expected = drawingState(source);
  const bundle = await source.reproducibilityBundle();
  const target = await makeStore();
  target.createManualPathway({ name: 'Unrelated route', reactionIds: ['eco@PGI'] });
  const result = await target.importReproducibilityBundle(bundle);
  assert.equal(result.valid, true);
  assert.equal(result.replay.pass, true);
  assert.deepEqual(drawingState(target), expected);
  assert.equal(target.canUndoMap, false);
  assert.deepEqual(target.unsavedWork(), { map: false, scenario: false, model: false });
});

test('older bundles load without inheriting unrelated pathway and map-omics annotations', async () => {
  const source = await makeStore();
  const original = await source.reproducibilityBundle();
  for (const version of [1, 2]) {
    const bundle = structuredClone(original);
    bundle.version = version;
    delete bundle.presentation;
    const target = await makeStore();
    target.createManualPathway({ name: 'Unrelated route', reactionIds: ['eco@PGI'] });
    target.importOmicsCSV('gene,Control\nb0008,1\n');
    const result = await target.importReproducibilityBundle(await seal(bundle));
    assert.equal(result.replay.pass, true);
    assert.equal(target.manualPathways.size, 0);
    assert.equal(target.omicsDataset, null);
  }
});

test('a bundle export captures its state before another community starts loading', async () => {
  const source = await makeStore();
  source.scenarios.push({ ...source.scenario, id: 'another-ecoli', map: null,
    species: [{ ...source.scenario.species[0], id: 'second' }] });
  const pending = source.reproducibilityBundle();
  const switching = source.useScenario('another-ecoli');
  const bundle = await pending;
  await switching;
  assert.equal(bundle.community.species[0].id, 'eco');
  assert.equal(bundle.community.scenario, 'solo-ecoli');
  assert.deepEqual(Object.keys(bundle.models), ['eco']);
  assert.equal((await verifyReproducibilityBundle(bundle)).valid, true);
});

test('missing or duplicate bundle species are refused without replacing uploaded models', async () => {
  const source = await makeStore();
  const original = await source.reproducibilityBundle();
  const missing = structuredClone(original);
  delete missing.models.eco;
  const duplicate = structuredClone(original);
  duplicate.community.species.push({ ...duplicate.community.species[0] });
  const absent = structuredClone(original);
  absent.community.species = [];
  for (const bundle of [missing, duplicate, absent]) {
    await refuseWithoutChanges(await editableStore(), bundle);
  }
});

test('an invalid embedded model is refused before any current model is deleted', async () => {
  const source = await makeStore();
  const bundle = await source.reproducibilityBundle();
  bundle.community.species.push({ id: 'broken', file: 'unused', abundance: 1, enabled: true });
  bundle.models.broken = { id: 'broken', metabolites: [], reactions: [{ id: 'R', metabolites: { missing: -1 } }] };
  await refuseWithoutChanges(await editableStore(), bundle);
});

test('invalid bundle medium and reaction bounds are refused before replacing the open work', async () => {
  const source = await makeStore();
  const original = await source.reproducibilityBundle();
  const invalid = [
    { medium: { glc__D: 'ten' } },
    { medium: { glc__D: -1 } },
    { reactionOverrides: { 'eco@ATPM': { lower: 10, upper: 1 } } },
    { reactionOverrides: { 'eco@ATPM': { lower: 0, upper: null } } },
  ];
  for (const patch of invalid) {
    const bundle = structuredClone(original);
    Object.assign(bundle.constraints, patch);
    await refuseWithoutChanges(await editableStore(), bundle);
  }
});

test('a malformed bundle drawing is refused before replacing the open work', async () => {
  const source = await makeStore();
  const bundle = await source.reproducibilityBundle();
  bundle.map.nodes = {};
  await refuseWithoutChanges(await editableStore(), bundle);
});

test('invalid analysis methods, missing GIMME data and replay tolerances are refused before replacement', async () => {
  const source = await makeStore();
  const original = await source.reproducibilityBundle();
  for (const method of ['GIMME', 'unknown-analysis']) {
    const bundle = structuredClone(original);
    bundle.analysis.method = method;
    bundle.omics = null;
    await refuseWithoutChanges(await editableStore(), bundle);
  }
  const bundle = structuredClone(original);
  bundle.replay.tolerances.absolute = -1;
  await refuseWithoutChanges(await editableStore(), bundle);
});

test('invalid presentation colors are refused before replacing the open work', async () => {
  const source = await makeStore();
  const bundle = await source.reproducibilityBundle();
  bundle.presentation = { fluxColorStops: [{ value: -1, color: 'broken' }] };
  await refuseWithoutChanges(await editableStore(), bundle);
});

test('malformed pathway reaction lists are refused before replacing the open work', async () => {
  const source = await makeStore();
  const bundle = await source.reproducibilityBundle();
  bundle.presentation = {
    pathwayManager: {
      manual: [{ id: 'bad-route', name: 'Invalid route', reactionIds: 'eco@TALA' }], settings: [],
    },
  };
  await refuseWithoutChanges(await editableStore(), bundle);
});

test('bundle versions must be supported positive integers', async () => {
  const source = await makeStore();
  const original = await source.reproducibilityBundle();
  for (const version of [null, '2', 0, -1, 1.5]) {
    const bundle = structuredClone(original);
    bundle.version = version;
    await assert.rejects(verifyReproducibilityBundle(await seal(bundle)), /version/i);
  }
});

test('a pFVA bundle restores its parsimonious solution and archived flux ranges', async () => {
  const source = await makeStore();
  await source.runAnalysis('pfva');
  clearTimeout(source._fvaTimer);
  source._fvaTimer = null;
  await source.runFVA();
  const ranges = [...source.fvaRanges];
  const bundle = await source.reproducibilityBundle();
  const target = await makeStore();
  const result = await target.importReproducibilityBundle(bundle);
  assert.equal(target.parsimonious, true);
  assert.equal(target.pfvaEnabled, true);
  assert.equal(target.fvaEnabled, true);
  assert.equal(typeof target.solution.totalFlux, 'number');
  assert.ok(Math.abs(target.solution.totalFlux - source.solution.totalFlux) < 1e-6);
  assert.deepEqual([...target.fvaRanges], ranges);
  assert.equal(result.replay.pass, true);
});

test('malformed community omics are refused before replacing the open work', async () => {
  const source = await makeStore();
  const bundle = await source.reproducibilityBundle();
  bundle.omics = { conditions: ['Control'], values: [], settings: {} };
  await refuseWithoutChanges(await editableStore(), bundle);
});

test('GIMME archives with unusable expression inputs are refused before replacing the open work', async () => {
  const source = await makeStore();
  const original = await source.reproducibilityBundle();
  original.analysis.method = 'GIMME';
  original.omics = {
    conditions: ['Control'], values: { unknown: { id: 'unmapped', species: 'eco', values: { Control: 4 } } },
    settings: { conditionA: 'Control' },
  };
  for (const gimme of [
    { condition: 'Control', objectiveFraction: 0.8, threshold: 5 },
    { condition: 'missing', objectiveFraction: 0.8, threshold: 5 },
    { condition: 'Control', objectiveFraction: -1, threshold: 5 },
    { condition: 'Control', objectiveFraction: 0.8, threshold: 'bad' },
  ]) {
    const bundle = structuredClone(original);
    bundle.analysis.gimme = gimme;
    await refuseWithoutChanges(await editableStore(), bundle);
  }
});
