import test from 'node:test';
import assert from 'node:assert/strict';
import { EditableMap } from '../src/map/editable-map.js';
import { renderMap } from '../src/render/mapview.js';
import { Store } from '../src/ui/store.js';

function emptyMap() {
  return new EditableMap({
    canvas: { width: 600, height: 400 },
    environment: {
      id: 'u',
      label: 'Medium',
      rect: { x: 0, y: 0, w: 600, h: 400 },
    },
    cells: [],
    medium: { nodes: [], exchanges: [] },
    nodes: new Map(),
    reactions: new Map(),
    segments: [],
  });
}

const scale = {
  reference: 1,
  width() { return 1; },
  color() { return '#64748b'; },
};

test('free text can be created, edited, moved and removed', () => {
  const map = emptyMap();
  const text = map.addText('Glicólisis', 120, 90);

  assert.ok(text);
  assert.equal(text.id, 'text-1');
  assert.equal(text.text, 'Glicólisis');
  assert.equal(text.x, 120);
  assert.equal(text.y, 90);

  map.updateText(text.id, 'Glucólisis central');
  assert.equal(map.texts.get(text.id).text, 'Glucólisis central');

  map.moveText(text.id, 180, 140);
  assert.equal(map.texts.get(text.id).x, 180);
  assert.equal(map.texts.get(text.id).y, 140);

  assert.equal(map.removeText(text.id).id, text.id);
  assert.equal(map.texts.has(text.id), false);
});

test('text IDs increment without colliding', () => {
  const map = emptyMap();
  assert.equal(map.addText('A', 1, 1).id, 'text-1');
  assert.equal(map.addText('B', 2, 2).id, 'text-2');
  map.removeText('text-1');
  assert.equal(map.addText('C', 3, 3).id, 'text-1');
});

test('blank text is rejected', () => {
  const map = emptyMap();
  assert.equal(map.addText('   ', 10, 10), null);
  const item = map.addText('Valid', 10, 10);
  assert.equal(map.updateText(item.id, '   '), null);
  assert.equal(map.texts.get(item.id).text, 'Valid');
});

test('free text survives EditableMap JSON round trip', () => {
  const map = emptyMap();
  const item = map.addText('Glicólisis', 200, 150, { fontSize: 28 });
  map.moveText(item.id, 240, 180);

  const restored = EditableMap.fromJSON(JSON.parse(JSON.stringify(map.toJSON())));
  assert.deepEqual(restored.toJSON(), map.toJSON());
  assert.equal(restored.texts.get(item.id).text, 'Glicólisis');
  assert.equal(restored.texts.get(item.id).fontSize, 28);
});

test('renderer emits editable map text and escapes its content', () => {
  const map = emptyMap();
  const item = map.addText('A < B & C', 160, 100);

  const svg = renderMap(map, {
    fluxes: new Map(),
    scale,
    showCrossFeeding: false,
  });

  assert.ok(svg.includes(`data-map-text="${item.id}"`));
  assert.ok(svg.includes('A &lt; B &amp; C'));
  assert.ok(!svg.includes('A < B & C'));
});

test('store records add/edit/delete text in map history', () => {
  const store = new Store('.');
  store.map = emptyMap();

  const item = store.addMapText('Glicólisis', 100, 100);
  assert.ok(item);
  assert.equal(store.canUndoMap, true);

  store.updateMapText(item.id, 'Glicólisis / PPP');
  assert.equal(store.map.texts.get(item.id).text, 'Glicólisis / PPP');

  store.removeMapText(item.id);
  assert.equal(store.map.texts.has(item.id), false);

  store.undoMap();
  assert.equal(store.map.texts.get(item.id).text, 'Glicólisis / PPP');
});

test('deleting a selection removes the annotations caught in it', () => {
  /*
   * The report: a whole pathway was selected and deleted, and the metabolite
   * names persisted. In the core-metabolism overview those names are map
   * annotations, so a selection that did not hold annotations could not take
   * them, and Delete left them labelling reactions that had gone.
   */
  const store = new Store('.');
  store.map = emptyMap();
  const kept = store.addMapText('Far away', 580, 380);
  const doomed = store.addMapText('G6P', 100, 100);

  store.selectInRect({ x1: 60, y1: 40, x2: 300, y2: 140 });
  assert.ok(store.isMultiSelected('text', doomed.id), 'the rectangle caught it');
  assert.ok(!store.isMultiSelected('text', kept.id), 'and left the other alone');

  const removed = store.removeMultiSelection();

  assert.deepEqual(removed.texts, [doomed.id]);
  assert.equal(store.map.texts.has(doomed.id), false);
  assert.equal(store.map.texts.has(kept.id), true);

  store.undoMap();
  assert.equal(store.map.texts.get(doomed.id).text, 'G6P');
});

test('a selected annotation is drawn as selected', () => {
  const store = new Store('.');
  store.map = emptyMap();
  const item = store.addMapText('G6P', 100, 100);
  store.toggleMultiSelection('text', item.id);

  const svg = renderMap(store.map, {
    fluxes: new Map(), scale, multiSelection: store.multiSelection,
  });

  assert.ok(svg.includes('emc-map-text-selected'), 'the plate behind it is drawn');
  assert.ok(svg.includes('emc-map-text is-multi'), 'and the text itself is marked');
});
