/** Saved baselines follow serialized work while later edits and replacement contexts stay unsaved. */
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { Store } from '../src/ui/store.js';
import { Model } from '../src/core/model.js';

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

test('Scenario save completion records the written inputs and preserves later edits', async () => {
  const store = await makeStore();
  store.setMediumLimit('glc__D', '4');
  const snapshot = store.workSnapshot();
  const document = JSON.parse(store.stateJSON());
  store.setMediumLimit('glc__D', '3');
  store.markScenarioSaved(snapshot);
  assert.equal(store.unsavedWork().scenario, true);
  store.setMediumLimit('glc__D', String(document.medium.glc__D));
  assert.equal(store.unsavedWork().scenario, false);
});

test('Scenario save completion preserves shared pathway edits made after serialization', async () => {
  const store = await makeStore();
  const snapshot = store.workSnapshot();
  store.createManualPathway({ name: 'Later route', color: '#123456', reactionIds: ['eco@PGI'] });
  store.markScenarioSaved(snapshot);
  assert.equal(store.unsavedWork().map, true);
  store.undoMap();
  assert.equal(store.unsavedWork().map, false);
});

test('Map save completion records the written undo version and shared pathway settings', async () => {
  const store = await makeStore();
  store.addMapText('Written annotation', 200, 200);
  const snapshot = store.workSnapshot();
  store.addMapText('Later annotation', 250, 250);
  store.createManualPathway({ name: 'Later route', color: '#123456', reactionIds: ['eco@PGI'] });
  store.markMapSaved(snapshot);
  assert.equal(store.unsavedWork().map, true);
  store.undoMap();
  store.undoMap();
  assert.equal(store.unsavedWork().map, false);
  store.redoMap();
  assert.equal(store.unsavedWork().map, true);
});

test('a completed old Scenario save cannot replace the baseline of a reopened context with the same ID', async () => {
  const store = await makeStore();
  store.setMediumLimit('glc__D', '4');
  const snapshot = store.workSnapshot();
  const reopened = JSON.parse(store.stateJSON());
  reopened.medium.glc__D = 9;
  await store.loadState(JSON.stringify(reopened));
  store.setMediumLimit('glc__D', '8');
  store.markScenarioSaved(snapshot);
  assert.equal(store.unsavedWork().scenario, true);
  store.setMediumLimit('glc__D', '9');
  assert.equal(store.unsavedWork().scenario, false);
});

test('a completed old Map save cannot save a replacement context drawing', async () => {
  const store = await makeStore();
  store.addMapText('Previous context', 200, 200);
  const snapshot = store.workSnapshot();
  await store.useScenario('solo-ecoli');
  store.addMapText('Replacement context', 250, 250);
  store.markMapSaved(snapshot);
  assert.equal(store.unsavedWork().map, true);
  store.undoMap();
  assert.equal(store.unsavedWork().map, false);
});

test('clearing drawing history retires earlier Map save snapshots within the same context', async () => {
  const store = await makeStore();
  store.addMapText('Previous history', 200, 200);
  const snapshot = store.workSnapshot();
  store.mapHistory.clear();
  store.addMapText('Replacement history', 250, 250);
  store.markMapSaved(snapshot);
  assert.equal(store.unsavedWork().map, true);
  store.undoMap();
  assert.equal(store.unsavedWork().map, false);
});

test('model save completion records only the biological edits serialized into the file', async () => {
  const store = await makeStore();
  await store.createModelMetabolite({ id: 'written_c', name: 'Written metabolite', compartment: 'c' });
  const snapshot = store.modelSaveSnapshot();
  await store.createModelMetabolite({ id: 'later_c', name: 'Later metabolite', compartment: 'c' });
  store.markModelSaved(snapshot.model, snapshot.revision);
  assert.equal(store.unsavedWork().model, true);
  store.markModelSaved();
  assert.equal(store.unsavedWork().model, false);
});

test('a model save records the serialized model even when the editor selects another species', async () => {
  const store = await makeStore();
  await store.useScenario('aerobio-mixto');
  store.setModelEditorSpecies(store.species[0].id);
  await store.createModelMetabolite({ id: 'written_c', name: 'Written member', compartment: 'c' });
  const snapshot = store.modelSaveSnapshot();
  store.setModelEditorSpecies(store.species[1].id);
  await store.createModelMetabolite({ id: 'later_c', name: 'Other member', compartment: 'c' });
  store.markModelSaved(snapshot.model, snapshot.revision);
  assert.equal(store.modelHasUnsavedEdits(snapshot.model), false);
  assert.equal(store.modelHasUnsavedEdits(store.modelEditorModel()), true);
});

test('out-of-order save completion follows the last file written rather than the current inputs', async () => {
  const store = await makeStore();
  store.setMediumLimit('glc__D', '4');
  const earlier = store.workSnapshot();
  store.setMediumLimit('glc__D', '3');
  const later = store.workSnapshot();
  store.markScenarioSaved(later);
  assert.equal(store.unsavedWork().scenario, false);
  store.markScenarioSaved(earlier);
  assert.equal(store.unsavedWork().scenario, true);
  store.setMediumLimit('glc__D', '4');
  assert.equal(store.unsavedWork().scenario, false);
});

test('a rejected Scenario preload preserves the pending save snapshot of the current context', async () => {
  const store = await makeStore();
  store.setMediumLimit('glc__D', '4');
  const snapshot = store.workSnapshot();
  const document = JSON.parse(store.stateJSON());
  document.species[0].file = 'missing-save-timing-model';
  store.loadModel = async () => {
    throw new Error('Missing save timing model');
  };
  await assert.rejects(store.loadState(JSON.stringify(document)), /Missing save timing model/);
  store.setMediumLimit('glc__D', '3');
  store.markScenarioSaved(snapshot);
  assert.equal(store.unsavedWork().scenario, true);
  store.setMediumLimit('glc__D', '4');
  assert.equal(store.unsavedWork().scenario, false);
});
