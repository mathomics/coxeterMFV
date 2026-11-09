/** Metabolite placement follows the displayed compartment outline and preserves the biological calculation. */
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import factory from '../assets/vendor/glpk.js';
import { initGLPK } from '../src/solver/glpk.js';
import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import { EditableMap } from '../src/map/editable-map.js';
import { clampNodePosition, nodeFitsAt } from '../src/layout/drag.js';
import { compartmentCorners, COMPARTMENT_SHAPES } from '../src/layout/geometry.js';
import { layoutCommunity } from '../src/layout/layout.js';
import { Store } from '../src/ui/store.js';

test.before(async () => {
  globalThis.__GLPK_FACTORY__ = factory;
  globalThis.__GLPK_WASM__ = await readFile(new URL('../assets/vendor/glpk.wasm', import.meta.url));
  assert.equal(await initGLPK(), true);
});

async function makeStore() {
  const store = new Store('.');
  const raw = JSON.parse(await readFile(new URL('../models/ecoli_core_mc.json', import.meta.url), 'utf8'));
  store.models.set('ecoli_core_mc', new Model(raw));
  store.scenarios = JSON.parse(await readFile(new URL('../models/scenarios.json', import.meta.url), 'utf8')).scenarios;
  await store.useScenario('solo-ecoli');
  store.map = EditableMap.emptyFromCommunity(store.community);
  store.mapHistory.clear();
  return store;
}

function inside(compartment, point) {
  const rect = compartment.rect;
  const dx = Math.abs(point.x - rect.x - rect.w / 2);
  const dy = Math.abs(point.y - rect.y - rect.h / 2);
  if (dx > rect.w / 2 + 1e-7 || dy > rect.h / 2 + 1e-7) {
    return false;
  }
  const { rx, ry } = compartmentCorners(compartment);
  if (!rx || !ry) {
    return true;
  }
  const u = Math.max(0, dx - (rect.w / 2 - rx)) / rx;
  const v = Math.max(0, dy - (rect.h / 2 - ry)) / ry;
  return u * u + v * v <= 1 + 1e-7;
}

function diskInside(compartment, node, padding = 6) {
  for (let index = 0; index < 64; index += 1) {
    const angle = index * Math.PI / 32;
    const radius = node.r + padding;
    const point = { x: node.x + radius * Math.cos(angle), y: node.y + radius * Math.sin(angle) };
    if (!inside(compartment, point)) {
      return false;
    }
  }
  return true;
}

test('manual insertion and dragging keep circles inside all four displayed compartment shapes', async () => {
  for (const shape of Object.values(COMPARTMENT_SHAPES)) {
    const store = await makeStore();
    store.setCompartmentShape('eco', 'c', shape);
    const compartment = store.map.cells[0].compartments.find(item => item.id === 'c');
    const point = { x: compartment.rect.x + 30, y: compartment.rect.y + 30 };
    const node = store.addMapMetabolite('eco@g6p_c', point);
    assert.ok(node);
    assert.ok(diskInside(compartment, node), `insertion ${JSON.stringify(shape)}`);
    const transaction = store.beginMapEdit('Drag metabolite');
    store.map.moveNode(node.id, compartment.rect.x + compartment.rect.w - 30, compartment.rect.y + 30);
    store.commitMapEdit(transaction);
    assert.ok(diskInside(compartment, node), `drag ${JSON.stringify(shape)}`);
  }
});

test('reaction insertion contains its marker, primary nodes and secondary nodes in an oval', async () => {
  const store = await makeStore();
  store.setCompartmentShape('eco', 'c', COMPARTMENT_SHAPES.oval);
  const compartment = store.map.cells[0].compartments.find(item => item.id === 'c');
  const result = store.addMapReaction('eco@PFK', {
    anchor: { x: compartment.rect.x + 35, y: compartment.rect.y + 35 },
  });
  assert.ok(result);
  assert.ok(diskInside(compartment, { ...result.reaction, r: 5 }), 'marker');
  for (const node of store.map.nodes.values()) {
    assert.ok(diskInside(compartment, node), node.id);
  }
  for (const segment of store.map.segments) {
    if (segment.nodeId) {
      const node = store.map.nodes.get(segment.nodeId);
      const point = segment.side === 'substrate' ? segment.from : segment.to;
      assert.ok(Math.hypot(node.x - point.x, node.y - point.y) < 1e-8);
    }
  }
});

test('a periplasmic metabolite can occupy free space outside an oval cytosol but inside its bounding box', async () => {
  const store = await makeStore();
  store.setCompartmentShape('eco', 'c', COMPARTMENT_SHAPES.oval);
  const cytosol = store.map.cells[0].compartments.find(item => item.id === 'c');
  const point = { x: cytosol.rect.x + 40, y: cytosol.rect.y + 40 };
  assert.equal(inside(cytosol, point), false);
  const node = store.addMapMetabolite('eco@glc__D_p', point);
  assert.ok(node);
  assert.ok(Math.hypot(node.x - point.x, node.y - point.y) < 1e-8);
  assert.equal(nodeFitsAt(store.map, node, point.x, point.y), true);
});

test('a shared-medium node can use space outside an oval cell corner without crossing its outline', async () => {
  const store = await makeStore();
  store.setCompartmentShape('eco', 'e', COMPARTMENT_SHAPES.oval);
  const outer = store.map.cells[0].compartments.find(item => item.id === 'e');
  const point = { x: outer.rect.x + 35, y: outer.rect.y + 35 };
  const node = { kind: 'medium', cellId: null, compartment: 'medium', r: 13, ...point };
  assert.equal(inside(outer, point), false);
  assert.equal(nodeFitsAt(store.map, node, point.x, point.y), true);
  const result = clampNodePosition(store.map, node, point.x, point.y);
  assert.ok(Math.hypot(result.x - point.x, result.y - point.y) < 1e-8);
});

test('shape-aware insertion and drag history preserve the model, bounds and installed result', async () => {
  const store = await makeStore();
  const biology = store.modelEditorModel().toJSON();
  const bounds = store.problem.snapshotBounds();
  const solution = store.solution;
  store.setCompartmentShape('eco', 'c', COMPARTMENT_SHAPES.oval);
  const compartment = store.map.cells[0].compartments.find(item => item.id === 'c');
  const node = store.addMapMetabolite('eco@g6p_c', {
    x: compartment.rect.x + 30, y: compartment.rect.y + 30,
  });
  const before = store.map.toJSON();
  const transaction = store.beginMapEdit('Drag metabolite');
  store.map.moveNode(node.id, compartment.rect.x + compartment.rect.w - 30, compartment.rect.y + 30);
  store.commitMapEdit(transaction);
  const after = store.map.toJSON();
  store.undoMap();
  assert.deepEqual(store.map.toJSON(), before);
  store.redoMap();
  assert.deepEqual(store.map.toJSON(), after);
  assert.deepEqual(store.modelEditorModel().toJSON(), biology);
  assert.deepEqual(store.problem.snapshotBounds(), bounds);
  assert.equal(store.solution, solution);
});

test('automatic layout contains secondary circles in their displayed compartments and connects their branches', async () => {
  for (const file of ['ecoli_core_mc', 'yeast_core_mc']) {
    const raw = JSON.parse(await readFile(new URL(`../models/${file}.json`, import.meta.url), 'utf8'));
    const model = new Model(raw);
    const community = new CommunityModel({ species: [{ id: 'species', model, abundance: 1 }], medium: {} });
    const map = layoutCommunity(community);
    const nodes = [...map.nodes.values()].filter(node => node.kind === 'currency');
    assert.ok(nodes.length > 0);
    for (const node of nodes) {
      const cell = map.cells.find(cell => cell.id === node.cellId);
      const compartment = cell.compartments.find(compartment => compartment.id === node.compartment);
      assert.ok(diskInside(compartment, node), `${file}: ${node.id}`);
      assert.ok(nodeFitsAt(map, node, node.x, node.y), `${file}: ${node.id} region`);
    }
    for (const segment of map.segments) {
      if (segment.kind === 'currency') {
        const node = map.nodes.get(segment.nodeId);
        const point = segment.side === 'substrate' ? segment.from : segment.to;
        assert.ok(Math.hypot(node.x - point.x, node.y - point.y) < 1e-8);
      }
    }
  }
});
