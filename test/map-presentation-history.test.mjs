/**
 * Map imports and pathway annotations undo their presentation together with the
 * drawing, without replacing the biological model or its solution.
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

function presentation(store) {
  return {
    pathways: store.pathwayManagerState(),
    fluxColorStops: structuredClone(store.view.fluxColorStops),
    absoluteFluxColors: store.absoluteFluxColors(),
    pathwayRibbonMode: store.view.pathwayRibbonMode,
    reactionColorMode: store.view.reactionColorMode,
  };
}

test('loading a map restores its pathways and palettes through undo and redo', async () => {
  const store = await makeStore();
  store.createManualPathway({ name: 'Original route', color: '#ff0000', reactionIds: ['eco@TALA'] });
  store.mapHistory.clear();
  const before = presentation(store);
  const originalDrawing = structuredClone(store.map.toJSON());
  const solution = store.solution;
  const community = store.community;
  const doc = store.mapDocument();
  doc.map.texts.push({ id: 'import-label', text: 'Imported drawing', x: 300, y: 300 });
  doc.pathwayManager = {
    manual: [{ id: 'imported', name: 'Imported route', color: '#0000ff', visible: true, reactionIds: ['eco@PGI'] }],
    settings: [],
  };
  doc.fluxColorStops = [{ value: 0, color: '#ffffff' }, { value: 10, color: '#00ff00' }];
  doc.absoluteFluxColors = { stops: [{ value: 0, color: '#ffffff' }, { value: 10, color: '#0000ff' }], log: true };
  store.loadMapJSON(JSON.stringify(doc));
  const imported = presentation(store);
  const importedDrawing = structuredClone(store.map.toJSON());
  assert.notDeepEqual(imported, before);
  assert.ok(store.undoMap());
  assert.deepEqual(presentation(store), before);
  assert.deepEqual(store.map.toJSON(), originalDrawing);
  assert.ok(store.redoMap());
  assert.deepEqual(presentation(store), imported);
  assert.deepEqual(store.map.toJSON(), importedDrawing);
  assert.equal(store.solution, solution);
  assert.equal(store.community, community);
});

test('a map import that changes only pathways or palettes is one undoable edit', async () => {
  const store = await makeStore();
  const before = presentation(store);
  const doc = store.mapDocument();
  doc.absoluteFluxColors = { stops: [{ value: 0, color: '#ffffff' }, { value: 10, color: '#0000ff' }], log: false };
  store.loadMapJSON(JSON.stringify(doc));
  assert.equal(store.mapHistory.undoStack.length, 1);
  const imported = presentation(store);
  assert.ok(store.undoMap());
  assert.deepEqual(presentation(store), before);
  assert.ok(store.redoMap());
  assert.deepEqual(presentation(store), imported);
});

test('manual pathway edits are single undo steps and keep model annotations intact', async () => {
  const store = await makeStore();
  const biology = JSON.stringify(store.community.merged.toJSON());
  const before = presentation(store);
  const route = store.createManualPathway({ name: 'Custom route', color: '#123456', reactionIds: ['eco@TALA'] });
  assert.equal(store.mapHistory.undoStack.length, 1);
  const created = presentation(store);
  assert.ok(store.undoMap());
  assert.deepEqual(presentation(store), before);
  assert.ok(store.redoMap());
  assert.deepEqual(presentation(store), created);

  store.updateManualPathway(route.id, { name: 'Renamed route', color: '#654321', visible: false });
  assert.equal(store.mapHistory.undoStack.length, 2);
  const renamed = presentation(store);
  assert.ok(store.undoMap());
  assert.deepEqual(presentation(store), created);
  assert.ok(store.redoMap());
  assert.deepEqual(presentation(store), renamed);

  store.deleteManualPathway(route.id);
  assert.equal(store.mapHistory.undoStack.length, 3);
  assert.ok(store.undoMap());
  assert.deepEqual(presentation(store), renamed);
  assert.equal(JSON.stringify(store.community.merged.toJSON()), biology);
});

test('model pathway display changes undo without changing the model pathway', async () => {
  const store = await makeStore();
  const subsystem = store.community.merged.reactions.get('eco@TALA').subsystem;
  const key = `model:${subsystem}`;
  const before = presentation(store);
  store.setPathwayColor(key, '#123456');
  assert.equal(store.mapHistory.undoStack.length, 1);
  const colored = presentation(store);
  store.setPathwayVisible(key, false);
  assert.equal(store.mapHistory.undoStack.length, 2);
  assert.ok(store.undoMap());
  assert.deepEqual(presentation(store), colored);
  assert.ok(store.undoMap());
  assert.deepEqual(presentation(store), before);
  assert.equal(store.community.merged.reactions.get('eco@TALA').subsystem, subsystem);
});

test('undoing drawing edits does not roll back later palette changes', async () => {
  const store = await makeStore();
  store.addMapText('Drawing edit', 300, 300);
  const stops = [{ value: 0, color: '#ffffff' }, { value: 10, color: '#0000ff' }];
  store.setFluxColorStops(stops);
  assert.ok(store.undoMap());
  assert.deepEqual(store.view.fluxColorStops, stops);
  assert.ok(store.redoMap());
  assert.deepEqual(store.view.fluxColorStops, stops);
});

test('unchanged pathway settings and rejected edits add no undo steps', async () => {
  const store = await makeStore();
  const route = store.createManualPathway({ name: 'Custom route', color: '#123456', reactionIds: ['eco@TALA'] });
  store.mapHistory.clear();
  store.updateManualPathway(route.id, { name: route.name, color: route.color, visible: route.visible });
  assert.equal(store.canUndoMap, false);
  assert.throws(() => store.updateManualPathway(route.id, { name: ' ' }), /cannot be empty/);
  assert.equal(store.canUndoMap, false);
  assert.equal(store.deleteManualPathway('missing'), false);
  assert.equal(store.canUndoMap, false);
});

test('undoing a pathway edit keeps later palette and line-color choices', async () => {
  const store = await makeStore();
  store.createManualPathway({ name: 'Custom route', reactionIds: ['eco@TALA'] });
  const stops = [{ value: 0, color: '#ffffff' }, { value: 10, color: '#0000ff' }];
  store.setFluxColorStops(stops);
  store.setView({ reactionColorMode: 'absflux' });
  assert.ok(store.undoMap());
  assert.equal(store.manualPathways.size, 0);
  assert.deepEqual(store.view.fluxColorStops, stops);
  assert.equal(store.view.reactionColorMode, 'absflux');
  assert.ok(store.redoMap());
  assert.equal(store.manualPathways.size, 1);
  assert.deepEqual(store.view.fluxColorStops, stops);
  assert.equal(store.view.reactionColorMode, 'absflux');
});
