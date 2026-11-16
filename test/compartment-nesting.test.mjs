/**
 * A cell's own nesting of its compartments, set in the Edit compartments
 * window and saved in the map: the presets it starts from, the edits that
 * change it, and how the drawing follows (compartments fitted into their new
 * parent, missing parents drawn, transporters moved to the membrane they now
 * cross). The model is never touched.
 */

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import { nestingProblem, presetNesting } from '../src/core/compartments.js';
import { EditableMap } from '../src/map/editable-map.js';
import { communityReactionPlaces, mapSubtrees, reactionBoundaryKeys } from '../src/map/boundaries.js';
import {
  MAP_DOCUMENT_VERSION, createMapDocument, parseMapDocument,
} from '../src/map/map-document.js';
import { reconcileCompartments } from '../src/map/reconcile-map.js';
import { Store } from '../src/ui/store.js';
import { FluxScale } from '../src/render/theme.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

/* A store over one bundled model, with a blank map and the cell drawn. */
async function withCell(name, speciesId) {
  const raw = JSON.parse(await readFile(join(root, 'models', `${name}.json`), 'utf8'));
  const model = new Model(raw);
  const community = new CommunityModel({ species: [{ id: speciesId, model, abundance: 1 }], medium: {} });
  const store = new Store('.');
  store.models.set('fixture', model);
  store.species = [{ id: speciesId, file: 'fixture', abundance: 1, enabled: true }];
  store.community = community;
  store.map = EditableMap.blank({ width: 1300, height: 900 });
  store.scale = new FluxScale([]);
  store.addMapCell(speciesId, { x: 40, y: 40, w: 1180, h: 800 });
  return { model, community, store };
}

const compartment = (store, id) => store.map.cells[0].compartments.find((c) => c.id === id);

const inside = (inner, outer) => inner.x >= outer.x && inner.y >= outer.y
  && inner.x + inner.w <= outer.x + outer.w && inner.y + inner.h <= outer.y + outer.h;

const overlap = (a, b) => !(a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y);

test('a cell\'s saved nesting overrides the model\'s tree for what can be drawn and where', async () => {
  const { store } = await withCell('ecoli_core_mc', 'eco');
  assert.deepEqual(store.availableMapCompartments().map((x) => [x.compartmentId, x.parentId]), [['p', 'e']]);

  store.applyNestingPreset('eco', 'empty');
  assert.deepEqual(store.map.cells[0].nesting, { c: 'e', e: null, p: 'e' });
  assert.deepEqual(
    store.availableMapCompartments().map((x) => [x.compartmentId, x.parentId]),
    [['c', 'e'], ['p', 'e']],
  );
  const view = store.compartmentNesting('eco');
  assert.equal(view.custom, true);
  assert.deepEqual(view.rows.map((r) => [r.id, r.parentId]), [['e', null], ['c', 'e'], ['p', 'e']]);

  store.applyNestingPreset('eco', 'auto');
  assert.equal(store.map.cells[0].nesting, undefined);
  assert.equal(store.compartmentNesting('eco').custom, false);
});

test('moving a drawn compartment under another fits it inside its new parent and moves its transporters to the new membrane', async () => {
  const { model, community, store } = await withCell('yeast_core_mc', 'yeast');
  const modelBefore = JSON.stringify(model.toJSON());
  store.addMapCompartment('yeast', 'c', { x: 80, y: 80, w: 1100, h: 720 });
  store.addMapCompartment('yeast', 'im', { x: 120, y: 120, w: 380, h: 320 });
  store.addMapCompartment('yeast', 'm', { x: 170, y: 170, w: 280, h: 220 });

  // A transporter between the cytosol and the matrix.
  let transporter = null;
  for (const [id, reaction] of community.merged.reactions) {
    const places = new Set(communityReactionPlaces(community, reaction).map((p) => p.compartment));
    if (places.size === 2 && places.has('c') && places.has('m')) {
      transporter = id;
      break;
    }
  }
  assert.ok(transporter, 'the yeast model moves something between cytosol and matrix');
  assert.ok(store.map.addReaction(community, transporter));
  assert.deepEqual(reactionBoundaryKeys(community, transporter), ['yeast/im', 'yeast/m']);

  assert.ok(store.setCompartmentParent('yeast', 'm', 'c'));
  const m = compartment(store, 'm');
  assert.equal(m.parentId, 'c');
  assert.ok(inside(m.rect, compartment(store, 'c').rect), 'the matrix is inside the cytosol');
  assert.ok(!overlap(m.rect, compartment(store, 'im').rect), 'and beside the intermembrane space, not on it');

  const crossed = reactionBoundaryKeys(community, transporter, mapSubtrees(community, store.map));
  assert.deepEqual(crossed, ['yeast/m'], 'cytosol to matrix now crosses one membrane');
  const rxn = store.map.reactions.get(transporter);
  assert.equal(rxn.boundaryId, 'yeast/m');
  assert.equal(rxn.onMembrane, true);
  assert.equal(JSON.stringify(model.toJSON()), modelBefore, 'the model is not touched');
});

test('nesting a drawn compartment under an undrawn one draws the parent, and any missing ancestors, around it', async () => {
  const { store } = await withCell('ecoli_core_mc', 'eco');
  store.applyNestingPreset('eco', 'empty');
  store.addMapCompartment('eco', 'c', { x: 300, y: 250, w: 400, h: 300 });

  assert.ok(store.setCompartmentParent('eco', 'c', 'p'));
  const p = compartment(store, 'p');
  const c = compartment(store, 'c');
  assert.ok(p, 'the periplasm is drawn');
  assert.equal(p.parentId, 'e');
  assert.equal(c.parentId, 'p');
  assert.ok(inside(c.rect, p.rect));

  const yeast = await withCell('yeast_core_mc', 'yeast');
  yeast.store.applyNestingPreset('yeast', 'empty');
  yeast.store.addMapCompartment('yeast', 'm', { x: 400, y: 300, w: 300, h: 220 });
  assert.ok(yeast.store.setCompartmentNesting('yeast', { c: 'e', e: null, im: 'c', m: 'im' }));
  const at = (id) => compartment(yeast.store, id);
  assert.deepEqual(['c', 'im', 'm'].map((id) => at(id)?.parentId), ['e', 'c', 'im']);
  assert.ok(inside(at('m').rect, at('im').rect));
  assert.ok(inside(at('im').rect, at('c').rect));
  assert.ok(inside(at('c').rect, at('e').rect));
});

test('the outer compartment cannot move, and a compartment cannot go inside its own descendant', async () => {
  const { store } = await withCell('yeast_core_mc', 'yeast');
  store.addMapCompartment('yeast', 'c', { x: 80, y: 80, w: 1100, h: 720 });
  const json = JSON.stringify(store.map.toJSON());

  assert.equal(store.setCompartmentParent('yeast', 'e', 'c'), null);
  assert.match(store.status.message, /stays outermost/);
  assert.equal(store.setCompartmentParent('yeast', 'c', 'm'), null);
  assert.match(store.status.message, /cannot go inside itself/);
  assert.equal(store.nestCompartment('yeast', 'c', 'out'), null, 'nothing moves out of the outer compartment');
  assert.equal(JSON.stringify(store.map.toJSON()), json);
});

test('the Yeast9, BiGG and Empty presets keep every compartment inside the outer compartment', () => {
  const ids = ['c', 'ce', 'e', 'f', 'm', 'mm', 'p', 'tm'];
  for (const preset of ['yeast9', 'bigg', 'empty']) {
    assert.equal(nestingProblem(ids, 'e', presetNesting(ids, 'e', preset)), null, preset);
  }
  const yeast9 = presetNesting(ids, 'e', 'yeast9');
  assert.deepEqual([yeast9.ce, yeast9.c, yeast9.mm, yeast9.m, yeast9.p, yeast9.f], ['e', 'ce', 'c', 'mm', 'c', 'e']);
  const bigg = presetNesting(ids, 'e', 'bigg');
  assert.deepEqual([bigg.p, bigg.c, bigg.m, bigg.f, bigg.tm], ['e', 'p', 'c', 'e', 'e']);
  const empty = presetNesting(ids, 'e', 'empty');
  assert.ok(ids.filter((id) => id !== 'e').every((id) => empty[id] === 'e'));
});

test('a nesting with a loop or a moved outer compartment is refused', () => {
  const ids = ['c', 'e', 'm'];
  const nesting = { e: null, c: 'e', m: 'c' };
  assert.equal(nestingProblem(ids, 'e', nesting), null);
  nesting.c = 'm';
  assert.equal(nestingProblem(ids, 'e', nesting), 'c cannot go inside itself.');
  nesting.c = 'e';
  nesting.e = 'c';
  assert.equal(nestingProblem(ids, 'e', nesting), 'e is the outer compartment and stays outermost.');
});

test('Tab nests a compartment under the one listed above it, and Shift+Tab moves it back out', async () => {
  const { store } = await withCell('yeast_core_mc', 'yeast');
  store.applyNestingPreset('yeast', 'empty');
  const parents = () => Object.fromEntries(store.compartmentNesting('yeast').rows.map((r) => [r.id, r.parentId]));

  store.nestCompartment('yeast', 'im', 'in');
  assert.equal(parents().im, 'c');
  store.nestCompartment('yeast', 'm', 'in');
  assert.equal(parents().m, 'c');
  store.nestCompartment('yeast', 'm', 'in');
  assert.equal(parents().m, 'im');
  store.nestCompartment('yeast', 'm', 'out');
  assert.equal(parents().m, 'c');
});

test('a nesting change is one undo step, and the map saves and loads byte-identical at version 4', async () => {
  const { community, store } = await withCell('yeast_core_mc', 'yeast');
  store.addMapCompartment('yeast', 'c', { x: 80, y: 80, w: 1100, h: 720 });
  store.addMapCompartment('yeast', 'im', { x: 120, y: 120, w: 380, h: 320 });
  store.addMapCompartment('yeast', 'm', { x: 170, y: 170, w: 280, h: 220 });
  const before = JSON.stringify(store.map.toJSON());

  store.setCompartmentParent('yeast', 'm', 'c');
  const after = JSON.stringify(store.map.toJSON());
  store.undoMap();
  assert.equal(JSON.stringify(store.map.toJSON()), before);
  store.redoMap();
  assert.equal(JSON.stringify(store.map.toJSON()), after);

  const doc = JSON.parse(JSON.stringify(createMapDocument(store.map, { species: store.species })));
  assert.equal(doc.version, MAP_DOCUMENT_VERSION);
  assert.equal(MAP_DOCUMENT_VERSION, 4);
  const reopened = EditableMap.fromJSON(parseMapDocument(doc).map);
  const report = reconcileCompartments(reopened, community);
  assert.deepEqual([report.wrapped, report.moved, report.left, report.reseated], [[], [], [], []]);
  assert.equal(JSON.stringify(reopened.toJSON()), after, 'loading keeps the nesting the user set');
});

test('a version 3 map loads with automatic nesting', async () => {
  const { store } = await withCell('yeast_core_mc', 'yeast');
  const doc = JSON.parse(JSON.stringify(createMapDocument(store.map, { species: store.species })));
  doc.version = 3;
  const parsed = parseMapDocument(doc);
  assert.equal(parsed.version, 3);
  store.map = EditableMap.fromJSON(parsed.map);
  assert.equal(store.map.cells[0].nesting, undefined);
  assert.equal(store.compartmentNesting('yeast').custom, false);
  assert.deepEqual(
    store.compartmentNesting('yeast').rows.map((r) => [r.id, r.parentId]),
    [['e', null], ['c', 'e'], ['im', 'c'], ['m', 'im']],
  );
});

test('re-laying out keeps the user\'s nesting', async () => {
  const { store } = await withCell('yeast_core_mc', 'yeast');
  store.applyNestingPreset('yeast', 'empty');
  const nesting = { ...store.map.cells[0].nesting };

  store.relayoutMap();
  const [cell] = store.map.cells;
  assert.deepEqual(cell.nesting, nesting);
  const parentOf = Object.fromEntries(cell.compartments.map((c) => [c.id, c.parentId]));
  assert.deepEqual(parentOf, { e: null, c: 'e', im: 'e', m: 'e' });
});
