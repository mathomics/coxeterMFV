import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import { EditableMap } from '../src/map/editable-map.js';
import { nearestRectEdge, rectFromHandle } from '../src/map/geometry-edit.js';
import { ENVIRONMENT_KEY } from '../src/map/boundaries.js';
import { renderMap } from '../src/render/mapview.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

async function community(name = 'yeast_core_mc') {
  const raw = JSON.parse(await readFile(join(root, 'models', `${name}.json`), 'utf8'));
  return new CommunityModel({
    species: [{ id: 'sp', model: new Model(raw), abundance: 1 }],
    medium: {},
  });
}

test('nearest rectangle edge resolves the side of a shared-medium boundary drag', () => {
  const rect = { x: 100, y: 80, w: 500, h: 300 };
  assert.equal(nearestRectEdge(rect, { x: 320, y: 82 }), 'n');
  assert.equal(nearestRectEdge(rect, { x: 598, y: 200 }), 'e');
  assert.equal(nearestRectEdge(rect, { x: 360, y: 379 }), 's');
  assert.equal(nearestRectEdge(rect, { x: 101, y: 180 }), 'w');
});

test('corner handles produce resized rectangles and the centre handle produces movement', () => {
  const r = { x: 100, y: 100, w: 300, h: 200 };
  assert.deepEqual(
    rectFromHandle(r, 'move', { x: 0, y: 0 }, { x: 20, y: -10 }),
    { x: 120, y: 90, w: 300, h: 200 },
  );
  assert.deepEqual(
    rectFromHandle(r, 'se', { x: 400, y: 300 }, { x: 440, y: 330 }),
    { x: 100, y: 100, w: 340, h: 230 },
  );
});

test('moving the outer cell moves all of its compartment rectangles', async () => {
  const com = await community();
  const map = EditableMap.emptyFromCommunity(com);
  const cell = map.cells[0];
  const outer = cell.compartments.find((c) => c.parentId === null);
  assert.ok(outer);

  const before = new Map(cell.compartments.map((c) => [c.id, { ...c.rect }]));
  const target = { ...outer.rect, x: outer.rect.x + 25, y: outer.rect.y + 18 };
  assert.ok(map.editGeometryRect(cell.id, outer.id, target));

  for (const comp of cell.compartments) {
    const old = before.get(comp.id);
    assert.ok(Math.abs(comp.rect.x - (old.x + 25)) < 1e-6, comp.id);
    assert.ok(Math.abs(comp.rect.y - (old.y + 18)) < 1e-6, comp.id);
  }
  assert.deepEqual(cell.rect, outer.rect);
});

test('resizing a parent keeps nested compartments inside it', async () => {
  const com = await community();
  const map = EditableMap.emptyFromCommunity(com);
  const cell = map.cells[0];
  const outer = cell.compartments.find((c) => c.parentId === null);
  assert.ok(outer);

  const target = {
    x: outer.rect.x,
    y: outer.rect.y,
    w: outer.rect.w * 0.82,
    h: outer.rect.h * 0.84,
  };
  map.editGeometryRect(cell.id, outer.id, target);

  for (const comp of cell.compartments.filter((c) => c.parentId)) {
    const parent = cell.compartments.find((p) => p.id === comp.parentId);
    assert.ok(comp.rect.x >= parent.rect.x - 1e-6, comp.id);
    assert.ok(comp.rect.y >= parent.rect.y - 1e-6, comp.id);
    assert.ok(comp.rect.x + comp.rect.w <= parent.rect.x + parent.rect.w + 1e-6, comp.id);
    assert.ok(comp.rect.y + comp.rect.h <= parent.rect.y + parent.rect.h + 1e-6, comp.id);
  }
});

test('editing geometry on a populated map transforms cellular content', async () => {
  const com = await community('ecoli_core_mc');
  const map = EditableMap.fromCommunity(com);
  const cell = map.cells[0];
  const outer = cell.compartments.find((c) => c.parentId === null);
  const node = [...map.nodes.values()].find((n) => n.cellId === cell.id && n.kind === 'metabolite');
  const rxn = [...map.reactions.values()].find((r) => r.cellId === cell.id);
  assert.ok(node && rxn);

  const beforeNode = { x: node.x, y: node.y };
  const beforeRxn = { x: rxn.x, y: rxn.y };
  map.editGeometryRect(cell.id, outer.id, {
    ...outer.rect,
    x: outer.rect.x + 30,
    y: outer.rect.y + 20,
  });

  assert.ok(Math.abs(node.x - (beforeNode.x + 30)) < 1e-6);
  assert.ok(Math.abs(node.y - (beforeNode.y + 20)) < 1e-6);
  assert.ok(Math.abs(rxn.x - (beforeRxn.x + 30)) < 1e-6);
  assert.ok(Math.abs(rxn.y - (beforeRxn.y + 20)) < 1e-6);
});

test('geometry edits refresh membrane boundary geometry', async () => {
  const com = await community();
  const map = EditableMap.emptyFromCommunity(com);
  const cell = map.cells[0];
  const outer = cell.compartments.find((c) => c.parentId === null);
  const boundaryId = `${cell.id}/${outer.id}`;
  const before = { ...map.boundary(boundaryId).rect };

  map.editGeometryRect(cell.id, outer.id, {
    ...outer.rect,
    x: outer.rect.x + 15,
    y: outer.rect.y + 12,
  });

  const after = map.boundary(boundaryId).rect;
  assert.notDeepEqual(after, before);
  assert.deepEqual(after, outer.rect);
});

test('geometry survives EditableMap JSON save/load', async () => {
  const com = await community();
  const map = EditableMap.emptyFromCommunity(com);
  const cell = map.cells[0];
  const outer = cell.compartments.find((c) => c.parentId === null);
  map.editGeometryRect(cell.id, outer.id, {
    x: outer.rect.x + 10,
    y: outer.rect.y + 10,
    w: outer.rect.w - 20,
    h: outer.rect.h - 20,
  });

  const restored = EditableMap.fromJSON(JSON.parse(JSON.stringify(map.toJSON())));
  assert.deepEqual(restored.toJSON(), map.toJSON());
});


test('shared medium is a first-class geometry target that can move the whole map', async () => {
  const com = await community();
  const map = EditableMap.emptyFromCommunity(com);
  const beforeEnv = { ...map.environment.rect };
  const beforeCell = { ...map.cells[0].rect };
  const target = map.geometryTarget(null, map.environment.id);
  assert.ok(target?.environment);
  assert.equal(target.compartment, map.environment);

  const moved = map.editGeometryRect(null, map.environment.id, {
    ...beforeEnv,
    x: beforeEnv.x + 35,
    y: beforeEnv.y + 24,
  });
  assert.ok(moved);
  assert.equal(map.environment.rect.x, beforeEnv.x + 35);
  assert.equal(map.environment.rect.y, beforeEnv.y + 24);
  assert.equal(map.cells[0].rect.x, beforeCell.x + 35);
  assert.equal(map.cells[0].rect.y, beforeCell.y + 24);
  assert.deepEqual(map.boundary(ENVIRONMENT_KEY).rect, map.environment.rect);
});

test('shared medium boundary can be expanded without moving the cell', async () => {
  const com = await community();
  const map = EditableMap.emptyFromCommunity(com);
  const beforeCell = { ...map.cells[0].rect };
  const before = { ...map.environment.rect };

  const resized = map.editGeometryRect(null, map.environment.id, {
    x: before.x,
    y: before.y,
    w: before.w + 260,
    h: before.h + 180,
  });

  assert.ok(resized);
  assert.ok(map.environment.rect.w >= before.w + 260);
  assert.ok(map.environment.rect.h >= before.h + 180);
  assert.deepEqual(map.cells[0].rect, beforeCell);
  assert.ok(map.canvas.width >= map.environment.rect.x + map.environment.rect.w);
  assert.ok(map.canvas.height >= map.environment.rect.y + map.environment.rect.h);
});

test('shared medium boundary is clickable and gets geometry handles when selected', async () => {
  const com = await community();
  const map = EditableMap.emptyFromCommunity(com);
  const svg = renderMap(map, {
    geometryEdit: true,
    selectedGeometry: { cellId: null, compartmentId: map.environment.id },
  });

  assert.ok(svg.includes(`data-boundary="${ENVIRONMENT_KEY}"`));
  assert.match(svg, /data-geometry-cell="" data-geometry-comp="s"/);
  assert.match(svg, /data-geometry-handle="move"/);
});

test('a selected compartment has a handle in the middle of each side as well as at each corner', async () => {
  const map = EditableMap.fromCommunity(await community());
  const cell = map.cells[0];
  const comp = cell.compartments[0];
  const svg = renderMap(map, {
    scale: { reference: 1, width: () => 2, color: () => '#000' },
    geometryEdit: true,
    selectedGeometry: { cellId: cell.id, compartmentId: comp.id },
  });
  const handles = [...svg.matchAll(/<rect class="emc-geometry-handle[^>]*data-geometry-handle="(\w+)" x="([\d.-]+)" y="([\d.-]+)"\s+width="([\d.-]+)" height="([\d.-]+)"/g)]
    .map(([, handle, x, y, w, h]) => ({ handle, cx: +x + +w / 2, cy: +y + +h / 2 }));
  assert.deepEqual(handles.map((h) => h.handle).sort(), ['e', 'n', 'ne', 'nw', 's', 'se', 'sw', 'w']);
  assert.match(svg, /data-geometry-handle="move"/);

  const r = comp.rect;
  const at = Object.fromEntries(handles.map((h) => [h.handle, h]));
  const near = (a, b) => Math.abs(a - b) < 0.1;
  assert.ok(near(at.n.cx, r.x + r.w / 2) && near(at.n.cy, r.y), 'n sits mid-top');
  assert.ok(near(at.s.cx, r.x + r.w / 2) && near(at.s.cy, r.y + r.h), 's sits mid-bottom');
  assert.ok(near(at.w.cx, r.x) && near(at.w.cy, r.y + r.h / 2), 'w sits mid-left');
  assert.ok(near(at.e.cx, r.x + r.w) && near(at.e.cy, r.y + r.h / 2), 'e sits mid-right');
});

// Resizing with transporters

/*
 * Resize as a corner or side handle does: every pointer move asks for the rect the
 * pointer now implies, worked out from the rect the drag started with.
 */
function dragCorner(map, cellId, compartmentId, handle, dx, dy, steps = 20) {
  const start = { ...map.geometryTarget(cellId, compartmentId).compartment.rect };
  const corner = {
    x: start.x + (handle.includes('e') ? start.w : 0),
    y: start.y + (handle.includes('s') ? start.h : 0),
  };
  for (let i = 1; i <= steps; i += 1) {
    const at = { x: corner.x + (dx * i) / steps, y: corner.y + (dy * i) / steps };
    map.editGeometryRect(cellId, compartmentId, rectFromHandle(start, handle, corner, at));
  }
  return start;
}

function sideOf(rect, p) {
  return [
    ['left', Math.abs(p.x - rect.x)],
    ['right', Math.abs(p.x - (rect.x + rect.w))],
    ['top', Math.abs(p.y - rect.y)],
    ['bottom', Math.abs(p.y - (rect.y + rect.h))],
  ].sort((a, b) => a[1] - b[1])[0][0];
}

/* The cytosol of the yeast layout, with a transporter on each of its sides. */
async function yeastCytosol() {
  const map = EditableMap.fromCommunity(await community());
  const cell = map.cells[0];
  const cytosol = cell.compartments.find((c) => c.parentId === cell.compartments.find((k) => k.parentId === null).id);
  const transporters = [...map.reactions.values()].filter((r) => r.membrane === cytosol.id);
  const sides = new Set(transporters.map((r) => sideOf(cytosol.rect, r)));
  assert.deepEqual([...sides].sort(), ['bottom', 'left', 'top'], 'yeast has cytosol transporters on three sides');

  // Bend every curve, so it shows if a handle is left behind or taken twice.
  for (const seg of map.segments) {
    const node = map.nodes.get(seg.nodeId);
    const rxn = map.reactions.get(seg.reactionId);
    if (!node || !rxn) continue;
    seg.b1 = { x: node.x + 7, y: node.y - 11 };
    seg.b2 = { x: rxn.x - 13, y: rxn.y + 5 };
  }
  return { map, cell, cytosol, transporters };
}

function positions(map) {
  const pt = (p) => ({ x: p.x, y: p.y });
  return {
    nodes: new Map([...map.nodes.values()].map((n) => [n.id, pt(n)])),
    reactions: new Map([...map.reactions.values()].map((r) => [r.id, pt(r)])),
    segments: map.segments.map((s) => ({ b1: s.b1 && pt(s.b1), b2: s.b2 && pt(s.b2) })),
  };
}

/* Each b1 keeps its offset from its metabolite and each b2 from its reaction. */
function assertHandlesFollow(map, before) {
  const near = (a, b) => Math.abs(a.x - b.x) < 1e-6 && Math.abs(a.y - b.y) < 1e-6;
  map.segments.forEach((seg, i) => {
    const old = before.segments[i];
    const n0 = before.nodes.get(seg.nodeId);
    const r0 = before.reactions.get(seg.reactionId);
    const n1 = map.nodes.get(seg.nodeId);
    const r1 = map.reactions.get(seg.reactionId);
    if (!old?.b1 || !n0 || !r0) return;
    assert.ok(near({ x: seg.b1.x - n1.x, y: seg.b1.y - n1.y }, { x: old.b1.x - n0.x, y: old.b1.y - n0.y }),
      `b1 of ${seg.reactionId} at ${seg.nodeId} left its metabolite`);
    assert.ok(near({ x: seg.b2.x - r1.x, y: seg.b2.y - r1.y }, { x: old.b2.x - r0.x, y: old.b2.y - r0.y }),
      `b2 of ${seg.reactionId} at ${seg.nodeId} left its reaction`);
  });
}

/*
 * The yeast cytosol has room to come in at the top and on either side; its
 * bottom has metabolites right against it.
 */
test('dragging one corner of a compartment leaves its other sides where they are', async () => {
  const { map, cell, cytosol } = await yeastCytosol();
  const start = dragCorner(map, cell.id, cytosol.id, 'ne', -40, 20);
  const r = cytosol.rect;

  // Transporters must not hold every side out a margin beyond them: they
  // follow the side there, so each step of the drag would push all four out.
  assert.equal(r.x, start.x);
  assert.equal(r.y + r.h, start.y + start.h);
  assert.ok(Math.abs((r.x + r.w) - (start.x + start.w - 40)) < 1e-6, 'the right side came in');
  assert.ok(Math.abs(r.y - (start.y + 20)) < 1e-6, 'the top came down');
});

test('a side handle changes only the width or only the height, however the pointer wanders', async () => {
  const { map, cell, cytosol } = await yeastCytosol();
  const before = positions(map);
  let start = dragCorner(map, cell.id, cytosol.id, 'e', -40, 25);
  let r = cytosol.rect;
  assert.deepEqual([r.x, r.y, r.h], [start.x, start.y, start.h]);
  assert.ok(Math.abs(r.w - (start.w - 40)) < 1e-6, 'the right side came in');

  start = dragCorner(map, cell.id, cytosol.id, 'n', -30, 20);
  r = cytosol.rect;
  assert.deepEqual([r.x, r.w, r.y + r.h], [start.x, start.w, start.y + start.h]);
  assert.ok(Math.abs(r.y - (start.y + 20)) < 1e-6, 'the top came down');
  assertHandlesFollow(map, before);
});

test('a transporter on a resized side moves only across it, by as far as the side moved', async () => {
  const { map, cell, cytosol, transporters } = await yeastCytosol();
  const before = positions(map);
  const start = dragCorner(map, cell.id, cytosol.id, 'nw', 30, 20);
  const r = cytosol.rect;
  const moved = {
    left: { x: r.x - start.x, y: 0 },
    right: { x: (r.x + r.w) - (start.x + start.w), y: 0 },
    top: { x: 0, y: r.y - start.y },
    bottom: { x: 0, y: (r.y + r.h) - (start.y + start.h) },
  };
  assert.ok(moved.left.x > 0 && moved.top.y > 0, 'the left side and the top both came in');

  let slid = 0;
  for (const rxn of transporters) {
    const old = before.reactions.get(rxn.id);
    const side = sideOf(start, old);
    const [along, across] = side === 'top' || side === 'bottom' ? ['x', 'y'] : ['y', 'x'];
    assert.ok(Math.abs(rxn[across] - old[across] - moved[side][across]) < 1e-6,
      `${rxn.id} on the ${side} moved across it by ${rxn[across] - old[across]}`);

    // Along it, nowhere: spread in proportion to its new length, as before,
    // every transporter slid away from the metabolites drawn against it. Only
    // one the next side came in past moves, to keep clear of the new corner.
    const lo = along === 'x' ? r.x : r.y;
    const hi = lo + (along === 'x' ? r.w : r.h);
    const want = Math.min(Math.max(old[along], lo + 18), hi - 18);
    assert.ok(Math.abs(rxn[along] - want) < 1e-6,
      `${rxn.id} on the ${side} slid along it by ${rxn[along] - old[along]}`);
    if (want !== old[along]) slid += 1;
  }
  assert.equal(slid, 1, 'O2t, near the corner the left side came in past');
  assertHandlesFollow(map, before);
});

test('a resized side pulled past the parent stops there instead of shifting the far side', async () => {
  const { map, cell, cytosol } = await yeastCytosol();
  const parent = cell.compartments.find((c) => c.id === cytosol.parentId).rect;
  const start = dragCorner(map, cell.id, cytosol.id, 'se', 400, 300);
  const r = cytosol.rect;
  assert.equal(r.x, start.x);
  assert.equal(r.y, start.y);
  assert.ok(Math.abs((r.x + r.w) - (parent.x + parent.w - 10)) < 1e-6);
  assert.ok(Math.abs((r.y + r.h) - (parent.y + parent.h - 10)) < 1e-6);
});

test('a metabolite carried by a resized boundary takes its handle on other curves too', async () => {
  const { map, cell, cytosol, transporters } = await yeastCytosol();
  // A transporter on the top, with a metabolite of its own drawn into a
  // second reaction that stays where it is.
  const rxn = transporters.find((t) => sideOf(cytosol.rect, t) === 'top'
    && [...map.nodes.values()].some((n) => n.reactionId === t.id));
  const own = [...map.nodes.values()].find((n) => n.reactionId === rxn.id);
  const other = [...map.reactions.values()].find((r) => r.cellId === cell.id && !r.membrane);
  const shared = { ...map.segments.find((s) => s.nodeId === own.id), reactionId: other.id };
  shared.b1 = { x: own.x + 7, y: own.y - 11 };
  shared.b2 = { x: other.x - 13, y: other.y + 5 };
  map.segments.push(shared);

  const before = { y: own.y, b1: { ...shared.b1 } };
  dragCorner(map, cell.id, cytosol.id, 'ne', 0, 20);
  const dy = own.y - before.y;
  assert.ok(dy > 0, 'the transporter came down with the top');
  assert.deepEqual(shared.b1, { x: before.b1.x, y: before.b1.y + dy });
});
