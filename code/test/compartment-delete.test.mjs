import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import { EditableMap } from '../src/map/editable-map.js';
import { Store } from '../src/ui/store.js';
import { FluxScale } from '../src/render/theme.js';
import { compartmentKey, emptySelection } from '../src/map/multi-selection.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

/* The yeast core model, laid out in full: reactions in the mitochondrion and on its membranes. */
async function yeastStore() {
  const raw = JSON.parse(await readFile(join(root, 'models', 'yeast_core_mc.json'), 'utf8'));
  const model = new Model(raw);
  const community = new CommunityModel({ species: [{ id: 'yeast', model, abundance: 1 }], medium: {} });
  const store = new Store('.');
  store.models.set('fixture', model);
  store.species = [{ id: 'yeast', file: 'fixture', abundance: 1, enabled: true }];
  store.community = community;
  store.map = EditableMap.fromCommunity(community);
  store.scale = new FluxScale([]);
  return store;
}

const ids = (cell) => cell.compartments.map((c) => c.id).sort();

test('deleting a compartment takes what is nested in it and drawn there, and nothing else', async () => {
  const store = await yeastStore();
  const map = store.map;
  const [cell] = map.cells;
  assert.ok(['im', 'm'].every((id) => ids(cell).includes(id)), 'the mitochondrion is drawn');
  const inside = new Set(['im', 'm']);
  const touches = (rxn) => (rxn.compartments ?? []).some((c) => inside.has(c)) || inside.has(rxn.membrane);
  const doomed = [...map.reactions.values()].filter(touches).map((r) => r.id);
  const kept = [...map.reactions.values()].filter((r) => !touches(r)).map((r) => r.id);
  assert.ok(doomed.length && kept.length);
  // A pathway name written inside it, and one written in the cytosol.
  const im = cell.compartments.find((c) => c.id === 'im').rect;
  const c = cell.compartments.find((k) => k.id === 'c').rect;
  const tca = map.addText('TCA cycle', im.x + im.w / 2, im.y + im.h / 2);
  const glycolysis = map.addText('Glycolysis', c.x + 20, c.y + 20);
  const json = JSON.stringify(map.toJSON());

  store.selectGeometry(cell.id, 'im');
  const result = store.removeMapCompartment(cell.id, 'im');
  assert.deepEqual(result.compartmentIds.sort(), ['im', 'm']);
  assert.equal(result.cellRemoved, false);
  assert.deepEqual(ids(store.map.cells[0]).filter((id) => inside.has(id)), []);
  assert.deepEqual(doomed.filter((id) => store.map.reactions.has(id)), [], 'its reactions are gone');
  assert.deepEqual(kept.filter((id) => !store.map.reactions.has(id)), [], 'the rest are all still there');
  assert.equal([...store.map.nodes.values()].filter((n) => inside.has(n.compartment)).length, 0, 'and its metabolites');
  assert.ok(store.map.segments.every((s) => !s.nodeId || store.map.nodes.has(s.nodeId)), 'no curve points at a removed metabolite');
  assert.ok(!store.map.boundaries.some((b) => b.cellId === cell.id && inside.has(b.inside)), 'its membranes are gone');
  assert.ok(!store.map.texts.has(tca.id), 'the name written inside it went with it');
  assert.ok(store.map.texts.has(glycolysis.id), 'the one outside stayed');
  assert.equal(store.selectedGeometry, null);
  assert.match(store.status.message, /^Removed .+ from the map, with 1 nested compartment, \d+ reactions?, \d+ metabolites?, 1 text\. The model is unchanged/);

  // It can be drawn again, and Ctrl+Z puts everything back as it was.
  assert.ok(store.availableMapCompartments().some((c) => c.compartmentId === 'im'));
  store.undoMap();
  assert.equal(JSON.stringify(store.map.toJSON()), json);
});

test('deleting a cell\'s outermost compartment deletes the cell', async () => {
  const store = await yeastStore();
  const [cell] = store.map.cells;
  const outer = cell.compartments.find((c) => !c.parentId);
  const result = store.removeMapCompartment(cell.id, outer.id);
  assert.equal(result.cellRemoved, true);
  assert.equal(store.map.cells.length, 0);
  assert.equal([...store.map.reactions.values()].filter((r) => r.cellId === cell.id).length, 0);
  assert.match(store.status.message, /^Removed the cell /);
  assert.ok(store.availableMapCells().some((c) => c.speciesId === cell.id), 'it can be drawn again');
  store.undoMap();
  assert.equal(store.map.cells.length, 1);
});

test('the shared medium is not a compartment that can be deleted', async () => {
  const store = await yeastStore();
  assert.equal(store.map.removeCompartment(null, store.map.environment.id), null);
  assert.equal(store.removeMapCompartment(null, store.map.environment.id), null);
});

test('a swept selection that holds a whole compartment deletes that compartment too', async () => {
  const store = await yeastStore();
  const [cell] = store.map.cells;
  store.multiSelection = emptySelection();
  store.multiSelection.compartments.add(compartmentKey(cell.id, 'm'));
  const removed = store.removeMultiSelection();
  assert.deepEqual(removed.compartments, ['m']);
  assert.ok(!ids(store.map.cells[0]).includes('m'));
  assert.ok(ids(store.map.cells[0]).includes('im'), 'its parent stays');
});

test('Delete removes the selected compartment or cell, read from what is selected', async () => {
  const source = await readFile(join(root, 'src/ui/app.js'), 'utf8');
  const at = source.indexOf("if (event.key === 'Delete' || event.key === 'Backspace') {");
  const body = source.slice(at, at + 1600);
  assert.match(body, /selected\?\.type === 'compartment' && selected\.cellId/);
  assert.match(body, /selected\?\.type === 'cell' \? cellRootOf\(selected\.id\)/);
  assert.match(body, /store\.removeMapCompartment\(compartment\.cellId, compartment\.compartmentId\)/);
  // The handles can outlive a click on a reaction: they must not decide.
  assert.doesNotMatch(body, /store\.selectedGeometry/);
});
