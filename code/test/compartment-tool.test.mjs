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

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

async function loaded(name, speciesId = 'sp') {
  const raw = JSON.parse(await readFile(join(root, 'models', `${name}.json`), 'utf8'));
  const model = new Model(raw);
  const community = new CommunityModel({
    species: [{ id: speciesId, model, abundance: 1 }],
    medium: {},
  });
  const store = new Store('.');
  store.models.set('fixture', model);
  store.species = [{ id: speciesId, file: 'fixture', abundance: 1, enabled: true }];
  store.community = community;
  store.map = EditableMap.blank({ width: 1200, height: 850 });
  store.scale = new FluxScale([]);
  return { model, community, store };
}

test('E. coli compartments become drawable in biological containment order e -> p -> c', async () => {
  const { store } = await loaded('ecoli_core_mc', 'eco');
  store.addMapCell('eco', { x: 80, y: 70, w: 760, h: 560 });

  let available = store.availableMapCompartments();
  assert.deepEqual(available.map((x) => [x.compartmentId, x.parentId]), [['p', 'e']]);

  const p = store.addMapCompartment('eco', 'p', { x: 130, y: 120, w: 650, h: 450 });
  assert.ok(p);
  available = store.availableMapCompartments();
  assert.deepEqual(available.map((x) => [x.compartmentId, x.parentId]), [['c', 'p']]);

  const c = store.addMapCompartment('eco', 'c', { x: 175, y: 165, w: 540, h: 350 });
  assert.ok(c);

  const cell = store.map.cells[0];
  const periplasm = cell.compartments.find((x) => x.id === 'p');
  const cytosol = cell.compartments.find((x) => x.id === 'c');
  assert.equal(cytosol.parentId, 'p');
  assert.ok(cytosol.rect.x > periplasm.rect.x);
  assert.ok(cytosol.rect.y > periplasm.rect.y);
  assert.ok(cytosol.rect.x + cytosol.rect.w < periplasm.rect.x + periplasm.rect.w);
  assert.ok(cytosol.rect.y + cytosol.rect.h < periplasm.rect.y + periplasm.rect.h);
});

test('yeast mitochondrial intermembrane space surrounds mitochondrial matrix when both exist', async () => {
  const { store } = await loaded('yeast_core_mc', 'yeast');
  store.addMapCell('yeast', { x: 80, y: 70, w: 900, h: 650 });

  assert.deepEqual(
    store.availableMapCompartments().map((x) => [x.compartmentId, x.parentId]),
    [['c', 'e']],
  );

  store.addMapCompartment('yeast', 'c', { x: 130, y: 120, w: 800, h: 540 });
  const afterC = store.availableMapCompartments();
  const imSpec = afterC.find((x) => x.compartmentId === 'im');
  assert.ok(imSpec);
  assert.equal(imSpec.parentId, 'c');

  store.addMapCompartment('yeast', 'im', { x: 250, y: 220, w: 360, h: 260 });
  const mSpec = store.availableMapCompartments().find((x) => x.compartmentId === 'm');
  assert.ok(mSpec);
  assert.equal(mSpec.parentId, 'im');

  store.addMapCompartment('yeast', 'm', { x: 285, y: 255, w: 285, h: 190 });
  const cell = store.map.cells[0];
  const im = cell.compartments.find((x) => x.id === 'im');
  const m = cell.compartments.find((x) => x.id === 'm');

  assert.equal(m.parentId, 'im');
  assert.ok(m.rect.x > im.rect.x);
  assert.ok(m.rect.y > im.rect.y);
  assert.ok(m.rect.x + m.rect.w < im.rect.x + im.rect.w);
  assert.ok(m.rect.y + m.rect.h < im.rect.y + im.rect.h);
});

test('custom compartment_tree governs ER-like nested compartments without hardcoding names', () => {
  const model = new Model({
    id: 'custom',
    name: 'custom',
    compartments: {
      e: 'Extracellular',
      c: 'Cytosol',
      r: 'Endoplasmic reticulum',
      rl: 'ER lumen',
    },
    compartment_tree: { e: null, c: 'e', r: 'c', rl: 'r' },
    metabolites: [
      { id: 'a_e', name: 'a', compartment: 'e' },
      { id: 'a_c', name: 'a', compartment: 'c' },
      { id: 'a_r', name: 'a', compartment: 'r' },
      { id: 'a_rl', name: 'a', compartment: 'rl' },
    ],
    reactions: [],
    genes: [],
    cell: { name: 'Custom cell', color: '#64748b' },
  });
  const tree = model.compartmentTree();
  assert.equal(tree.byId.get('c').parent, 'e');
  assert.equal(tree.byId.get('r').parent, 'c');
  assert.equal(tree.byId.get('rl').parent, 'r');
});

test('a requested compartment rectangle is clamped inside its biological parent', async () => {
  const { store } = await loaded('ecoli_core_mc', 'eco');
  store.addMapCell('eco', { x: 100, y: 100, w: 520, h: 360 });

  const result = store.addMapCompartment('eco', 'p', {
    x: -500,
    y: -400,
    w: 5000,
    h: 4000,
  });
  assert.ok(result);

  const cell = store.map.cells[0];
  const e = cell.compartments.find((x) => x.id === 'e');
  const p = cell.compartments.find((x) => x.id === 'p');
  assert.ok(p.rect.x > e.rect.x);
  assert.ok(p.rect.y > e.rect.y);
  assert.ok(p.rect.x + p.rect.w < e.rect.x + e.rect.w);
  assert.ok(p.rect.y + p.rect.h < e.rect.y + e.rect.h);
});

test('a compartment cannot be added before its biological parent exists', async () => {
  const { store } = await loaded('yeast_core_mc', 'yeast');
  store.addMapCell('yeast', { x: 80, y: 70, w: 900, h: 650 });

  assert.equal(store.addMapCompartment('yeast', 'm', { x: 200, y: 200, w: 200, h: 150 }), null);
  assert.equal(store.addMapCompartment('yeast', 'im', { x: 200, y: 200, w: 250, h: 180 }), null);
});

test('editing a child compartment still cannot cross its parent after creation', async () => {
  const { store } = await loaded('ecoli_core_mc', 'eco');
  store.addMapCell('eco', { x: 100, y: 100, w: 700, h: 500 });
  store.addMapCompartment('eco', 'p', { x: 140, y: 140, w: 600, h: 400 });
  store.addMapCompartment('eco', 'c', { x: 190, y: 190, w: 480, h: 290 });

  const cell = store.map.cells[0];
  const p = cell.compartments.find((x) => x.id === 'p');
  const c = cell.compartments.find((x) => x.id === 'c');

  store.map.editGeometryRect('eco', 'c', {
    x: p.rect.x - 1000,
    y: p.rect.y - 1000,
    w: p.rect.w * 5,
    h: p.rect.h * 5,
  });

  assert.ok(c.rect.x >= p.rect.x + 10 - 1e-6);
  assert.ok(c.rect.y >= p.rect.y + 10 - 1e-6);
  assert.ok(c.rect.x + c.rect.w <= p.rect.x + p.rect.w - 10 + 1e-6);
  assert.ok(c.rect.y + c.rect.h <= p.rect.y + p.rect.h - 10 + 1e-6);
});
