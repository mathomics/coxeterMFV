import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import { EditableMap } from '../src/map/editable-map.js';
import { Store } from '../src/ui/store.js';
import { renderMap } from '../src/render/mapview.js';
import { compartmentCornerRadius } from '../src/layout/geometry.js';

async function fixture() {
  const source = await readFile(new URL('../models/yeast_core_mc.json', import.meta.url), 'utf8');
  const model = new Model(JSON.parse(source));
  const community = new CommunityModel({
    species: [{ id: 'yeast', model, abundance: 1 }],
    medium: {},
  });
  const map = EditableMap.emptyFromCommunity(community);
  const compartment = map.cells[0].compartments.find(item => item.id === 'm');
  map.addReaction(community, 'yeast@PYRt2m', {
    boundaryId: 'yeast/m',
    anchor: { x: compartment.rect.x, y: compartment.rect.y },
  });
  const store = new Store('.');
  store.map = map;
  store.community = community;
  return { model, community, map, compartment, store };
}

function geometry(map) {
  const document = map.toJSON();
  delete document.environment.labelPosition;
  for (const cell of document.cells) {
    for (const compartment of cell.compartments) {
      delete compartment.labelPosition;
    }
  }
  return document;
}

test('dragging a compartment name follows straight edges and rounded corners without changing map geometry', async () => {
  const { map, compartment, model, community } = await fixture();
  const before = geometry(map);
  const biology = JSON.stringify([model.toJSON(), community.merged.toJSON()]);
  const rect = compartment.rect;
  const radius = compartmentCornerRadius(compartment);
  for (const x of [rect.x, rect.x + rect.w / 2, rect.x + rect.w]) {
    for (const y of [rect.y, rect.y + rect.h / 2, rect.y + rect.h]) {
      map.setCompartmentLabelPosition('yeast', 'm', x, y);
      const position = compartment.labelPosition;
      const px = rect.x + position.x * rect.w;
      const py = rect.y + position.y * rect.h;
      const nearestX = Math.max(rect.x + radius, Math.min(rect.x + rect.w - radius, px));
      const nearestY = Math.max(rect.y + radius, Math.min(rect.y + rect.h - radius, py));
      assert.ok(Math.abs(Math.hypot(px - nearestX, py - nearestY) - radius) < 1e-8);
      assert.deepEqual(geometry(map), before);
    }
  }
  assert.equal(JSON.stringify([model.toJSON(), community.merged.toJSON()]), biology);
});

test('compartment name positions survive JSON save and load and one drag makes one undo step', async () => {
  const { map, compartment, store } = await fixture();
  const before = map.toJSON();
  const rect = compartment.rect;
  const history = store.beginMapEdit('Move compartment name');
  store.moveCompartmentLabel('yeast', 'm', rect.x, rect.y);
  store.moveCompartmentLabel('yeast', 'm', rect.x, rect.y + rect.h / 2);
  store.commitMapEdit(history);
  const moved = map.toJSON();
  assert.deepEqual(EditableMap.fromJSON(moved).toJSON(), moved);
  assert.equal(store.mapHistory.undoStack.length, 1);
  store.undoMap();
  assert.deepEqual(store.map.toJSON(), before);
  store.redoMap();
  assert.deepEqual(store.map.toJSON(), moved);
});

test('a moved compartment name renders on its actual membrane above the reaction targets', async () => {
  const { map, compartment } = await fixture();
  const rect = compartment.rect;
  map.setCompartmentLabelPosition('yeast', 'm', rect.x, rect.y + rect.h / 2);
  const svg = renderMap(map, { scale: { color: () => '#94a3b8', width: () => 1, reference: 1 } });
  const round = value => String(Math.round(value * 100) / 100);
  assert.ok(svg.includes(`data-compartment-label="m" transform="translate(${round(rect.x)} ${round(rect.y + rect.h / 2)})"`));
  assert.ok(svg.indexOf('emc-layer-compartment-labels') > svg.indexOf('emc-layer-hit'));
});

test('invalid compartment-name moves are ignored', async () => {
  const { map } = await fixture();
  const before = map.toJSON();
  assert.equal(map.setCompartmentLabelPosition('missing', 'm', 10, 10), null);
  assert.equal(map.setCompartmentLabelPosition('yeast', 'm', NaN, 10), null);
  assert.deepEqual(map.toJSON(), before);
});
