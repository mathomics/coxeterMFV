/**
 * Inspector edits keep selections valid and drawing history separate from biology.
 * The bundled E. coli model supplies the map and numerical result.
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import factory from '../assets/vendor/glpk.js';
import { initGLPK } from '../src/solver/glpk.js';
import { Model } from '../src/core/model.js';
import { Store } from '../src/ui/store.js';
import { renderInspector } from '../src/ui/panels.js';

test.before(async () => {
  globalThis.__GLPK_FACTORY__ = factory;
  globalThis.__GLPK_WASM__ = await readFile(new URL('../assets/vendor/glpk.wasm', import.meta.url));
  assert.equal(await initGLPK(), true);
});

async function makeStore() {
  const raw = JSON.parse(await readFile(new URL('../models/ecoli_core_mc.json', import.meta.url), 'utf8'));
  const store = new Store('.');
  store.models.set('ecoli_core_mc', new Model(raw));
  store.scenarios = JSON.parse(await readFile(new URL('../models/scenarios.json', import.meta.url), 'utf8')).scenarios;
  await store.useScenario('solo-ecoli');
  return store;
}

test('selecting compartment geometry replaces the previous multi-selection without editing the drawing', async () => {
  const store = await makeStore();
  const text = store.addMapText('Selection note', 200, 200);
  store.toggleMultiSelection('node', 'met:eco@g6p_c');
  store.toggleMultiSelection('reaction', 'eco@PGI');
  store.toggleMultiSelection('text', text.id);
  const drawing = store.map.toJSON();
  const version = store.mapHistory.version;
  const solution = store.solution;
  store.selectGeometry('eco', 'c');
  assert.equal(store.multiSelectionSize, 0);
  assert.equal(store.selectedEntity.type, 'compartment');
  assert.equal(store.selectedEntity.id, 'c');
  assert.doesNotMatch(renderInspector(store), /data-action="selection-delete"/);
  assert.deepEqual(store.map.toJSON(), drawing);
  assert.equal(store.mapHistory.version, version);
  assert.equal(store.solution, solution);
});

test('selecting a reaction or clearing selection removes the previous geometry handles', async () => {
  for (const reaction of ['eco@PGI', null]) {
    const store = await makeStore();
    store.selectGeometry('eco', 'c');
    store.select(reaction);
    assert.equal(store.selectedGeometry, null, String(reaction));
    assert.equal(store.selectedReaction, reaction);
    if (reaction) {
      assert.equal(store.selectedEntity.type, 'reaction');
    } else {
      assert.equal(store.selectedEntity, null);
    }
  }
});

test('deleting one annotation or metabolite removes its stale selection while preserving unrelated picks', async () => {
  const store = await makeStore();
  const first = store.addMapText('Remove this note', 200, 200);
  const second = store.addMapText('Keep this note', 220, 220);
  const node = store.duplicateMapMetabolite('met:eco@g6p_c', { offset: 40 });
  assert.equal(store.canRemoveMapNode(node.id), true);
  store.toggleMultiSelection('text', first.id);
  store.toggleMultiSelection('text', second.id);
  store.toggleMultiSelection('node', node.id);
  store.toggleMultiSelection('reaction', 'eco@PGI');
  const solution = store.solution;
  store.removeMapText(first.id);
  assert.equal(store.multiSelection.texts.has(first.id), false);
  assert.equal(store.multiSelectionSize, 3);
  store.removeMapNode(node.id);
  assert.equal(store.multiSelection.nodes.has(node.id), false);
  assert.equal(store.multiSelectionSize, 2);
  assert.equal(store.multiSelection.texts.has(second.id), true);
  assert.equal(store.multiSelection.reactions.has('eco@PGI'), true);
  store.undoMap();
  store.redoMap();
  assert.equal(store.multiSelectionSize, 2);
  assert.equal(store.solution, solution);
});

test('merging a selected metabolite retargets its multi-selection and deduplicates an already selected survivor', async () => {
  for (const pickedTarget of [false, true]) {
    const store = await makeStore();
    const target = store.map.nodes.get('met:eco@g6p_c');
    const source = store.duplicateMapMetabolite(target.id, { offset: 40 });
    assert.ok(target && source);
    store.selectEntity('metabolite', source.id);
    store.toggleMultiSelection('node', source.id);
    if (pickedTarget) {
      store.toggleMultiSelection('node', target.id);
    }
    const edit = store.beginMapEdit('Merge metabolites');
    store.moveNode(source.id, target.x, target.y);
    const result = store.mergeOverlappingMetabolite(source.id);
    assert.equal(result.target.id, target.id);
    store.commitMapEdit(edit);
    assert.equal(store.multiSelection.nodes.has(source.id), false);
    assert.deepEqual([...store.multiSelection.nodes], [target.id]);
    assert.equal(store.selectedEntity.id, target.id);
  }
});

test('annotation and metabolite deletion produce English Undo labels', async () => {
  const store = await makeStore();
  const text = store.addMapText('Delete this note', 200, 200);
  store.removeMapText(text.id);
  assert.equal(store.undoMap().label, 'Remove text');
  const node = store.duplicateMapMetabolite('met:eco@g6p_c', { offset: 40 });
  store.removeMapNode(node.id);
  assert.equal(store.undoMap().label, 'Remove metabolite');
});

test('mixed inspector styles each record one Undo step and preserve the biological model and solution', async () => {
  const store = await makeStore();
  const text = store.addMapText('Original note', 200, 200);
  store.mapHistory.clear();
  const model = store.models.get('ecoli_core_mc').toJSON();
  const bounds = store.problem.snapshotBounds();
  const solution = store.solution;
  const expected = [store.map.toJSON()];
  const actions = [
    () => store.updateMapText(text.id, 'Edited note'),
    () => store.styleMapText(text.id, { fontSize: 28, fontFamily: 'times' }),
    () => store.setMapGeometryColors('eco', null, { fill: '#123456' }),
    () => store.setMapGeometryColors('eco', 'c', { fill: '#fedcba' }),
    () => store.setMapNodeColors(['met:eco@g6p_c'], { fill: '#aabbcc', stroke: '#112233' }),
    () => store.setMapReactionColor(['eco@PGI'], '#765432'),
    () => store.setMapNodePrimary('met:eco@g6p_c', false),
    () => store.setMapNodeHidden('met:eco@g6p_c', true),
    () => store.showAllHiddenMetabolites(),
  ];
  for (const action of actions) {
    const count = store.mapHistory.undoStack.length;
    action();
    assert.equal(store.mapHistory.undoStack.length, count + 1);
    expected.push(store.map.toJSON());
  }
  for (let index = actions.length - 1; index >= 0; index -= 1) {
    store.undoMap();
    assert.deepEqual(store.map.toJSON(), expected[index]);
  }
  for (let index = 1; index < expected.length; index += 1) {
    store.redoMap();
    assert.deepEqual(store.map.toJSON(), expected[index]);
  }
  assert.deepEqual(store.models.get('ecoli_core_mc').toJSON(), model);
  assert.deepEqual(store.problem.snapshotBounds(), bounds);
  assert.equal(store.solution, solution);
});

test('repeated inspector values and missing targets do not add history or discard an existing Redo', async () => {
  const store = await makeStore();
  const text = store.addMapText('Stable note', 200, 200);
  store.styleMapText(text.id, { fontSize: 28, fontFamily: 'times' });
  store.setMapReactionColor(['eco@PGI'], '#765432');
  store.undoMap();
  const version = store.mapHistory.version;
  const drawing = store.map.toJSON();
  const actions = [
    ['unchanged text', () => store.updateMapText(text.id, 'Stable note')],
    ['unchanged text style', () => store.styleMapText(text.id, { fontSize: 28, fontFamily: 'times' })],
    ['already visible metabolite', () => store.setMapNodeHidden('met:eco@g6p_c', false)],
    ['missing cell', () => store.setMapGeometryColors('missing-cell', null, { fill: '#123456' })],
    ['missing node', () => store.setMapNodeColors(['missing-node'], { fill: '#123456' })],
    ['missing reaction', () => store.setMapReactionColor(['missing-reaction'], '#123456')],
  ];
  for (const [name, action] of actions) {
    action();
    assert.equal(store.mapHistory.version, version, name);
  }
  assert.equal(store.mapHistory.canRedo, true);
  assert.deepEqual(store.map.toJSON(), drawing);
});

test('the metabolite inspector counts actual drawn connections on the real map', async () => {
  const store = await makeStore();
  const id = 'met:eco@g6p_c';
  store.selectEntity('metabolite', id);
  const connected = store.map.segmentsAt(id).length;
  assert.ok(connected > 0);
  const html = renderInspector(store);
  assert.match(html, new RegExp(`<dt[^>]*>Connected paths</dt>\\s*<dd[^>]*>${connected}</dd>`));
});

test('metabolite reaction links target existing merged reaction identifiers without adding another species prefix', async () => {
  const store = await makeStore();
  let count = 0;
  for (const node of store.map.nodes.values()) {
    if (!store.community.merged.metabolites.has(node.mergedId)) {
      continue;
    }
    store.selectEntity('metabolite', node.id);
    const html = renderInspector(store);
    const links = [...html.matchAll(/data-action="select-model-reaction" data-rxn="([^"]+)"/g)];
    for (const link of links) {
      assert.equal(store.community.merged.reactions.has(link[1]), true, `${node.id}: ${link[1]}`);
      assert.equal(link[1].includes('eco@eco@'), false);
      count += 1;
    }
  }
  assert.ok(count > 0);
});
