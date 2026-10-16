import test from 'node:test';
import assert from 'node:assert/strict';
import { Store } from '../src/ui/store.js';
import {
  createMapDocument, parseMapDocument, MAP_DOCUMENT_VERSION,
} from '../src/map/map-document.js';

function makeStore() {
  const store = new Store('.');
  store.community = {
    merged: {
      reactions: new Map([
        ['R1', { id: 'R1', pathway: 'Glycolysis', subsystem: 'Central carbon' }],
        ['R2', { id: 'R2', pathway: 'Glycolysis', subsystem: 'Central carbon' }],
        ['R3', { id: 'R3', pathway: 'TCA cycle', subsystem: 'Central carbon' }],
      ]),
    },
  };
  const drawn = new Set(['R1', 'R2', 'R3']);
  store.map = {
    reactionIds: () => [...drawn],
    hasReaction: (id) => drawn.has(id),
  };
  return store;
}

test('manual pathway is created from selected reactions and activates ribbons', () => {
  const store = makeStore();
  store.multiSelection.reactions.add('R1');
  store.multiSelection.reactions.add('R2');

  const pathway = store.createManualPathway({ name: 'Upper glycolysis', color: '#123456' });

  assert.equal(pathway.name, 'Upper glycolysis');
  assert.equal(pathway.color, '#123456');
  assert.deepEqual([...pathway.reactionIds].sort(), ['R1', 'R2']);
  assert.equal(store.view.pathwayRibbonMode, 'pathway');
  assert.equal(store.view.reactionColorMode, 'flux');
});

test('manual pathways are exclusive annotations without altering model metadata', () => {
  const store = makeStore();
  const first = store.createManualPathway({ name: 'First', reactionIds: ['R1', 'R2'] });
  const second = store.createManualPathway({ name: 'Second', reactionIds: ['R2', 'R3'] });

  assert.deepEqual([...store.manualPathways.get(first.id).reactionIds], ['R1']);
  assert.deepEqual([...store.manualPathways.get(second.id).reactionIds].sort(), ['R2', 'R3']);
  assert.equal(store.community.merged.reactions.get('R2').pathway, 'Glycolysis');
});

test('manual pathway overrides model pathway colour in managed pathway view', () => {
  const store = makeStore();
  store.createManualPathway({ name: 'Custom route', color: '#abcdef', reactionIds: ['R1'] });

  const coloring = store.managedPathwayColoring({ drawnOnly: true });
  assert.equal(coloring.groups.get('R1'), 'Custom route');
  assert.equal(coloring.colors.get('R1'), '#abcdef');
  assert.equal(coloring.groups.get('R2'), 'Glycolysis');
});

test('pathway manager can hide model and manual pathways', () => {
  const store = makeStore();
  const manual = store.createManualPathway({ name: 'Manual TCA', color: '#ff0000', reactionIds: ['R3'] });

  store.setPathwayVisible('model:Glycolysis', false);
  store.setPathwayVisible(`manual:${manual.id}`, false);

  const coloring = store.managedPathwayColoring({ drawnOnly: true });
  assert.equal(coloring.groups.has('R1'), false);
  assert.equal(coloring.groups.has('R2'), false);
  assert.equal(coloring.groups.has('R3'), false);
});

test('pathway manager selects all drawn reactions in a pathway', () => {
  const store = makeStore();
  const ids = store.selectPathway('model:Glycolysis');

  assert.deepEqual(ids.sort(), ['R1', 'R2']);
  assert.deepEqual([...store.multiSelection.reactions].sort(), ['R1', 'R2']);
  assert.equal(store.view.multiSelect, true);
});

test('manual pathways and pathway display settings round-trip through manager state', () => {
  const store = makeStore();
  const manual = store.createManualPathway({ name: 'Custom', color: '#654321', reactionIds: ['R1'] });
  store.setPathwayColor('model:Glycolysis', '#112233');
  store.setPathwayVisible('model:TCA cycle', false);

  const state = store.pathwayManagerState();
  const restored = makeStore();
  restored.restorePathwayManagerState(state);

  assert.equal(restored.manualPathways.get(manual.id).color, '#654321');
  assert.deepEqual([...restored.manualPathways.get(manual.id).reactionIds], ['R1']);
  assert.equal(restored.pathwaySettings.get('Glycolysis').color, '#112233');
  assert.equal(restored.pathwaySettings.get('TCA cycle').visible, false);
});

test('map document v3 persists pathway manager data while older maps remain parseable', () => {
  const map = {
    reactions: new Map([['R1', {}]]),
    nodes: new Map(),
    segments: [],
    texts: new Map(),
    toJSON() {
      return { version: 2, reactions: [{ id: 'R1' }], nodes: [], segments: [], cells: [] };
    },
  };
  const manager = {
    manual: [{ id: 'custom', name: 'Custom', color: '#123456', visible: true, reactionIds: ['R1'] }],
    settings: [],
  };

  const doc = createMapDocument(map, { pathwayManager: manager });
  assert.equal(MAP_DOCUMENT_VERSION, 3);
  assert.deepEqual(parseMapDocument(doc).pathwayManager, manager);

  const legacy = parseMapDocument(map.toJSON());
  assert.equal(legacy.legacy, true);
  assert.equal(legacy.pathwayManager, null);
});
