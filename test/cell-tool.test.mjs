import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import { EditableMap } from '../src/map/editable-map.js';
import { Store } from '../src/ui/store.js';
import { renderMap } from '../src/render/mapview.js';
import { FluxScale } from '../src/render/theme.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

async function fixture() {
  const raw = JSON.parse(await readFile(join(root, 'models', 'ecoli_core_mc.json'), 'utf8'));
  const model = new Model(raw);
  const community = new CommunityModel({
    species: [{ id: 'eco', model, abundance: 1 }],
    medium: { glc__D: { lower: -10 }, o2: { lower: -20 } },
  });
  return { model, community };
}

function storeFor(model, community) {
  const store = new Store('.');
  store.models.set('eco-file', model);
  store.species = [{ id: 'eco', file: 'eco-file', abundance: 1, enabled: true }];
  store.community = community;
  store.map = EditableMap.blank({ width: 1000, height: 700 });
  store.scale = new FluxScale([]);
  return store;
}

test('Cell creation draws the species outer extracellular boundary on a blank map', async () => {
  const { model, community } = await fixture();
  const store = storeFor(model, community);

  assert.deepEqual(store.availableMapCells().map((x) => x.speciesId), ['eco']);
  assert.deepEqual(store.availableMapCompartments(), []);

  const result = store.addMapCell('eco', { x: 120, y: 100, w: 520, h: 360 });
  assert.ok(result);
  assert.equal(store.map.cells.length, 1);

  const cell = store.map.cells[0];
  assert.equal(cell.id, 'eco');
  assert.equal(cell.outerCompartment, 'e');
  assert.equal(cell.compartments.length, 1);
  assert.equal(cell.compartments[0].id, 'e');
  assert.deepEqual(cell.rect, cell.compartments[0].rect);

  const boundary = store.map.boundary('eco/e');
  assert.ok(boundary);
  assert.equal(boundary.kind, 'cell');
  assert.equal(boundary.inside, 'e');
  assert.equal(boundary.outside, 's');

  assert.deepEqual(store.availableMapCells(), []);
  assert.deepEqual(store.availableMapCompartments().map((x) => x.compartmentId), ['p']);
});

test('Cell boundary is rendered as an explicit selectable boundary', async () => {
  const { model, community } = await fixture();
  const store = storeFor(model, community);
  store.addMapCell('eco', { x: 120, y: 100, w: 520, h: 360 });

  const svg = renderMap(store.map, {
    fluxes: new Map(),
    scale: store.scale,
    growth: new Map(),
  });

  assert.match(svg, /class="emc-cell-boundary"/);
  assert.match(svg, /data-cell-boundary="eco"/);
  assert.match(svg, /data-boundary="eco\/e"/);
});

test('resizing a Cell creates free space without scaling child compartments', async () => {
  const { model, community } = await fixture();
  const store = storeFor(model, community);
  store.addMapCell('eco', { x: 100, y: 90, w: 520, h: 360 });
  store.addMapCompartment('eco', 'p');

  const cell = store.map.cells[0];
  const outer = cell.compartments.find((c) => c.id === 'e');
  const child = cell.compartments.find((c) => c.id === 'p');
  const childBefore = { ...child.rect };

  const edited = store.map.editGeometryRect('eco', 'e', {
    x: outer.rect.x,
    y: outer.rect.y,
    w: outer.rect.w + 180,
    h: outer.rect.h + 120,
  });

  assert.ok(edited);
  assert.deepEqual(child.rect, childBefore);
  assert.deepEqual(cell.rect, outer.rect);
  assert.ok(outer.rect.w > 520);
  assert.ok(outer.rect.h > 360);
});

test('moving a Cell moves its child compartments by the same translation', async () => {
  const { model, community } = await fixture();
  const store = storeFor(model, community);
  store.addMapCell('eco', { x: 100, y: 90, w: 520, h: 360 });
  store.addMapCompartment('eco', 'p');

  const cell = store.map.cells[0];
  const outer = cell.compartments.find((c) => c.id === 'e');
  const child = cell.compartments.find((c) => c.id === 'p');
  const before = { ...child.rect };

  store.map.editGeometryRect('eco', 'e', {
    ...outer.rect,
    x: outer.rect.x + 40,
    y: outer.rect.y + 25,
  });

  assert.equal(child.rect.x, before.x + 40);
  assert.equal(child.rect.y, before.y + 25);
  assert.equal(child.rect.w, before.w);
  assert.equal(child.rect.h, before.h);
});

test('a child compartment cannot be resized beyond its parent boundary', async () => {
  const { model, community } = await fixture();
  const store = storeFor(model, community);
  store.addMapCell('eco', { x: 100, y: 90, w: 520, h: 360 });
  store.addMapCompartment('eco', 'p');

  const cell = store.map.cells[0];
  const outer = cell.compartments.find((c) => c.id === 'e');
  const p = cell.compartments.find((c) => c.id === 'p');

  store.map.editGeometryRect('eco', 'p', {
    x: outer.rect.x - 500,
    y: outer.rect.y - 500,
    w: outer.rect.w * 4,
    h: outer.rect.h * 4,
  });

  assert.ok(p.rect.x >= outer.rect.x + 10 - 1e-6);
  assert.ok(p.rect.y >= outer.rect.y + 10 - 1e-6);
  assert.ok(p.rect.x + p.rect.w <= outer.rect.x + outer.rect.w - 10 + 1e-6);
  assert.ok(p.rect.y + p.rect.h <= outer.rect.y + outer.rect.h - 10 + 1e-6);
});

test('Cell creation is map-only and does not mutate the biological model or solver', async () => {
  const { model, community } = await fixture();
  const store = storeFor(model, community);
  const reactions = community.merged.reactions.size;
  const metabolites = community.merged.metabolites.size;

  store.addMapCell('eco', { x: 100, y: 90, w: 520, h: 360 });

  assert.equal(community.merged.reactions.size, reactions);
  assert.equal(community.merged.metabolites.size, metabolites);
  assert.equal(model.compartmentIds().includes('e'), true);
});
