import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { EditableMap } from '../src/map/editable-map.js';
import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import { Store } from '../src/ui/store.js';
import { renderMap } from '../src/render/mapview.js';
import { FluxScale } from '../src/render/theme.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

async function ecoli() {
  const raw = JSON.parse(await readFile(join(root, 'models', 'ecoli_core_mc.json'), 'utf8'));
  const model = new Model(raw);
  const community = new CommunityModel({
    species: [{ id: 'eco', model, abundance: 1 }],
    medium: { glc__D: { lower: -10 }, o2: { lower: -20 } },
  });
  return { model, community };
}

test('blank map contains only a drawing canvas and no biological geometry', () => {
  const map = EditableMap.blank({ width: 900, height: 600 });
  assert.equal(map.width, 900);
  assert.equal(map.height, 600);
  assert.equal(map.cells.length, 0);
  assert.equal(map.nodes.size, 0);
  assert.equal(map.reactions.size, 0);
  assert.equal(map.segments.length, 0);
  assert.equal(map.environment.hidden, true);
  assert.equal(map.isEmpty(), true);
});

test('blank map renderer shows the white canvas but no environment or compartments', () => {
  const map = EditableMap.blank({ width: 900, height: 600 });
  const svg = renderMap(map, {
    fluxes: new Map(),
    scale: new FluxScale([]),
    growth: new Map(),
  });
  assert.ok(svg.includes('emc-canvas-frame'));
  assert.ok(svg.includes('data-canvas-resize="right"'));
  assert.ok(svg.includes('data-canvas-resize="bottom"'));
  assert.ok(svg.includes('data-canvas-resize="corner"'));
  assert.ok(!svg.includes('emc-layer-env'));
  assert.ok(!svg.includes('emc-compartment-space'));
});

test('showEmptyMap now produces a truly blank map rather than the structural shell', async () => {
  const { community } = await ecoli();
  const store = new Store('.');
  store.community = community;
  store.map = EditableMap.fromCommunity(community);
  store.scale = new FluxScale([]);
  const width = store.map.width;
  const height = store.map.height;

  store.showEmptyMap();

  assert.equal(store.map.cells.length, 0);
  assert.equal(store.map.nodes.size, 0);
  assert.equal(store.map.reactions.size, 0);
  assert.equal(store.map.environment.hidden, true);
  assert.equal(store.map.width, width);
  assert.equal(store.map.height, height);
});

test('manual structure follows the biological compartment hierarchy', async () => {
  const { model, community } = await ecoli();
  const store = new Store('.');
  store.models.set('eco-file', model);
  store.species = [{ id: 'eco', file: 'eco-file', abundance: 1, enabled: true }];
  store.community = community;
  store.map = EditableMap.blank({ width: 1000, height: 700 });

  let available = store.availableMapCompartments();
  assert.deepEqual(available, []);
  assert.deepEqual(store.availableMapCells().map((x) => x.speciesId), ['eco']);

  const cell = store.addMapCell('eco', { x: 80, y: 70, w: 520, h: 360 });
  assert.ok(cell);
  assert.equal(store.map.cells.length, 1);
  assert.equal(store.map.cells[0].compartments.length, 1);
  assert.equal(store.map.cells[0].compartments[0].id, 'e');

  available = store.availableMapCompartments();
  assert.deepEqual(available.map((x) => x.compartmentId), ['p']);

  const p = store.addMapCompartment('eco', 'p');
  assert.ok(p);
  assert.equal(p.compartment.parentId, 'e');

  available = store.availableMapCompartments();
  assert.deepEqual(available.map((x) => x.compartmentId), ['c']);

  const c = store.addMapCompartment('eco', 'c');
  assert.ok(c);
  assert.equal(c.compartment.parentId, 'p');
  assert.equal(store.availableMapCompartments().length, 0);
});

test('adding map compartments does not change the biological model', async () => {
  const { model, community } = await ecoli();
  const reactionCount = community.merged.reactions.size;
  const metaboliteCount = community.merged.metabolites.size;
  const sourceCompartments = model.compartmentIds();

  const map = EditableMap.blank({ width: 1000, height: 700 });
  map.addCell({
    speciesId: 'eco',
    outerCompartmentId: 'e',
    outerLabel: model.compartmentNames.e,
    cellName: model.cell.name,
    color: model.cell.color,
    rect: { x: 100, y: 80, w: 520, h: 360 },
  });

  assert.equal(community.merged.reactions.size, reactionCount);
  assert.equal(community.merged.metabolites.size, metaboliteCount);
  assert.deepEqual(model.compartmentIds(), sourceCompartments);
});

test('canvas can grow and cannot shrink through existing content', () => {
  const map = EditableMap.blank({ width: 800, height: 500 });
  let size = map.setCanvasSize(1200, 900);
  assert.deepEqual(size, { width: 1200, height: 900 });

  map.addCell({
    speciesId: 'eco',
    outerCompartmentId: 'e',
    cellName: 'E. coli',
    rect: { x: 700, y: 500, w: 350, h: 250 },
  });

  const min = map.minimumCanvasSize();
  size = map.setCanvasSize(400, 300);
  assert.ok(size.width >= min.width);
  assert.ok(size.height >= min.height);
  assert.ok(size.width >= 1090);
  assert.ok(size.height >= 790);
});

test('blank-map canvas size and manually added compartments survive JSON round trip', () => {
  const map = EditableMap.blank({ width: 1100, height: 750 });
  map.addCell({
    speciesId: 'eco',
    outerCompartmentId: 'e',
    outerLabel: 'Extracelular',
    cellName: 'E. coli',
    color: '#0284c7',
    rect: { x: 100, y: 80, w: 520, h: 360 },
  });
  map.setCanvasSize(1300, 900);

  const back = EditableMap.fromJSON(JSON.parse(JSON.stringify(map.toJSON())));
  assert.equal(back.width, 1300);
  assert.equal(back.height, 900);
  assert.equal(back.environment.hidden, true);
  assert.equal(back.cells.length, 1);
  assert.equal(back.cells[0].compartments[0].id, 'e');
});
