/**
 * Replacing a drawing restores its mode and annotations through history and
 * drops selections from the old drawing without replacing the model.
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { Store } from '../src/ui/store.js';
import { Model } from '../src/core/model.js';

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
    drawing: store.map.toJSON(),
    mode: store.mapMode,
    pathways: store.pathwayManagerState(),
    ribbons: store.view.pathwayRibbonMode,
    crossFeeding: store.view.showCrossFeeding,
    currency: store.view.showCurrency,
  };
}

function selectObjects(store) {
  assert.ok(store.selectGeometry('eco', 'c'));
  store.toggleMultiSelection('reaction', 'eco@TALA');
  assert.equal(store.multiSelectionSize, 1);
}

function assertNoSelection(store) {
  assert.equal(store.selectedGeometry, null);
  assert.equal(store.selectedReaction, null);
  assert.equal(store.selectedEntity, null);
  assert.equal(store.multiSelectionSize, 0);
}

test('overview undo and redo restore the drawing mode, pathways and display choices', async () => {
  const store = await makeStore();
  store.setView({ pathwayRibbonMode: 'pathway', showCrossFeeding: true, showCurrency: false });
  const before = drawingState(store);
  const community = store.community;
  const solution = store.solution;
  assert.ok(store.showCoreMetabolismOverview());
  const overview = drawingState(store);
  assert.equal(store.mapHistory.undoStack.length, 1);
  assert.ok(store.undoMap());
  assert.deepEqual(drawingState(store), before);
  assert.ok(store.redoMap());
  assert.deepEqual(drawingState(store), overview);
  assert.equal(store.community, community);
  assert.equal(store.solution, solution);
});

test('undoing a full drawing restores the overview and its pathways', async () => {
  const store = await makeStore();
  store.showCoreMetabolismOverview();
  const overview = drawingState(store);
  store.showFullMap();
  const full = drawingState(store);
  assert.ok(store.undoMap());
  assert.deepEqual(drawingState(store), overview);
  assert.ok(store.redoMap());
  assert.deepEqual(drawingState(store), full);
});

test('undoing a clear drawing restores the overview mode for subsequent layouts', async () => {
  const store = await makeStore();
  store.showCoreMetabolismOverview();
  const overview = drawingState(store);
  store.showEmptyMap();
  assert.ok(store.undoMap());
  assert.deepEqual(drawingState(store), overview);
  store.setLayoutPreset('compact');
  assert.equal(store.map.reactions.size, 9);
  assert.equal(store.mapMode, 'core-overview');
});

test('undo and redo drop stale selections while keeping drawn objects selected', async () => {
  const store = await makeStore();
  const text = store.addMapText('History edit', 300, 300);
  selectObjects(store);
  store.toggleMultiSelection('text', text.id);
  assert.ok(store.undoMap());
  assert.equal(store.selectedGeometry, null);
  assert.equal(store.multiSelection.reactions.has('eco@TALA'), true);
  assert.equal(store.multiSelection.texts.size, 0);
  store.showEmptyMap();
  assert.ok(store.undoMap());
  selectObjects(store);
  store.multiSelection.nodes.add(store.map.nodes.keys().next().value);
  store.multiSelection.compartments.add(JSON.stringify(['eco', 'c']));
  assert.ok(store.redoMap());
  assertNoSelection(store);
});

test('loading a map discards geometry and multi-selections from the previous drawing', async () => {
  const store = await makeStore();
  const doc = store.mapDocument();
  selectObjects(store);
  store.loadMapJSON(JSON.stringify(doc));
  assertNoSelection(store);
});

test('drawing the full model discards the previous geometry selection', async () => {
  const store = await makeStore();
  selectObjects(store);
  store.showFullMap();
  assertNoSelection(store);
});

test('opening a normal scenario clears old selections and the overview mode', async () => {
  const store = await makeStore();
  store.showCoreMetabolismOverview();
  selectObjects(store);
  await store.useScenario('solo-ecoli');
  assertNoSelection(store);
  assert.equal(store.mapMode, 'full');
  store.setLayoutPreset('compact');
  assert.ok(store.map.reactions.size > 9);
});

test('loading a full map after an overview resets the mode and undo restores it', async () => {
  const store = await makeStore();
  const doc = store.mapDocument();
  store.showCoreMetabolismOverview();
  const overview = drawingState(store);
  store.loadMapJSON(JSON.stringify(doc));
  assert.equal(store.mapMode, 'full');
  assert.ok(store.undoMap());
  assert.deepEqual(drawingState(store), overview);
});

test('opening a Scenario file clears old selections and the overview mode', async () => {
  const store = await makeStore();
  const state = store.stateJSON();
  store.showCoreMetabolismOverview();
  selectObjects(store);
  await store.loadState(state);
  assertNoSelection(store);
  assert.equal(store.mapMode, 'full');
  store.setLayoutPreset('compact');
  assert.ok(store.map.reactions.size > 9);
});
