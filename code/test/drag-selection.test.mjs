/**
 * Dragging a multi-selection (dragSelection): an organelle selected with what
 * is round it moves whole, with everything in it, and a cofactor selected with
 * its reaction is not moved twice.
 *
 * Everything runs on the yeast layout, which has a mitochondrion inside a
 * mitochondrial membrane inside the cytosol.
 */

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import { layoutCommunity } from '../src/layout/layout.js';
import { EditableMap } from '../src/map/editable-map.js';
import {
  compartmentKey, emptySelection, selectableInRect, selectionSize,
} from '../src/map/multi-selection.js';
import { Store } from '../src/ui/store.js';
import { renderMap } from '../src/render/mapview.js';
import { FluxScale } from '../src/render/theme.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

async function yeast() {
  const raw = JSON.parse(await readFile(join(root, 'models', 'yeast_core_mc.json'), 'utf8'));
  const com = new CommunityModel({ species: [{ id: 'sce', model: new Model(raw), abundance: 1 }], medium: {} });
  return EditableMap.fromLayout(layoutCommunity(com));
}

/*
 * The innermost compartment that still holds one: the mitochondrial membrane
 * round the mitochondrion (the cytosol holds compartments too, but sits higher).
 */
function organelle(map) {
  const cell = map.cells[0];
  const depth = (c) => {
    let d = 0;
    for (let q = c; q?.parentId; q = cell.compartments.find((k) => k.id === q.parentId)) d += 1;
    return d;
  };
  const org = cell.compartments
    .filter((c) => c.parentId && cell.compartments.some((k) => k.parentId === c.id))
    .sort((a, b) => depth(b) - depth(a))[0];
  assert.ok(org, 'yeast has an organelle with a compartment inside it');
  assert.ok(depth(org) >= 2, `expected the mitochondrial membrane, got ${org.id}`);
  const parent = cell.compartments.find((c) => c.id === org.parentId);
  return { cell, org: { ...org, rect: { ...org.rect } }, parent: { ...parent, rect: { ...parent.rect } } };
}

function around(r, margin) {
  return { x: r.x - margin, y: r.y - margin, w: r.w + 2 * margin, h: r.h + 2 * margin };
}

function rectOf(map, cellId, id) {
  return map.cells.find((c) => c.id === cellId).compartments.find((c) => c.id === id).rect;
}

/* Nodes the organelle carries: inside it, or inside what it holds. */
function inside(map, cell, org) {
  const held = new Set(cell.compartments
    .filter((k) => k.id === org.id || k.parentId === org.id).map((k) => k.id));
  return [...map.nodes.values()].filter((n) => held.has(n.compartment)).map((n) => n.id);
}

test('a rectangle round an organelle selects it and what it holds', async () => {
  const map = await yeast();
  const { cell, org } = organelle(map);
  const selection = selectableInRect(map, around(org.rect, 4));
  assert.ok(selection.compartments.has(compartmentKey(cell.id, org.id)));
  const child = cell.compartments.find((k) => k.parentId === org.id);
  assert.ok(selection.compartments.has(compartmentKey(cell.id, child.id)));
  assert.ok(selectionSize(selection) > selection.nodes.size + selection.reactions.size);

  // A rectangle that leaves part of it out does not take the membrane.
  const half = { x: org.rect.x - 4, y: org.rect.y - 4, w: org.rect.w / 2, h: org.rect.h + 8 };
  assert.ok(!selectableInRect(map, half).compartments.has(compartmentKey(cell.id, org.id)));
});

test('dragging a selection carries its organelle and everything in it', async () => {
  const map = await yeast();
  const { cell, org, parent } = organelle(map);
  const selection = selectableInRect(map, around(org.rect, 4));
  const carried = inside(map, cell, org);
  assert.ok(carried.length > 5, `the organelle should hold metabolites, got ${carried.length}`);
  const before = carried.map((id) => ({ x: map.nodes.get(id).x, y: map.nodes.get(id).y }));

  const roomLeft = org.rect.x - (parent.rect.x + 10);
  const dx = roomLeft > 2 ? -Math.min(roomLeft / 2, 30) : 10;
  const moved = map.dragSelection(selection, dx, 0);

  assert.ok(moved.compartments.includes(compartmentKey(cell.id, org.id)));
  assert.notEqual(moved.dx, 0, 'the organelle should have moved');
  const r = rectOf(map, cell.id, org.id);
  assert.ok(Math.abs(r.x - (org.rect.x + moved.dx)) < 1e-9 && Math.abs(r.y - org.rect.y) < 1e-9);
  carried.forEach((id, i) => {
    const node = map.nodes.get(id);
    assert.ok(Math.abs(node.x - (before[i].x + moved.dx)) < 1e-9 && Math.abs(node.y - before[i].y) < 1e-9,
      `${id} went to ${node.x},${node.y} instead of moving with its organelle`);
  });
});

test('the old translation leaves an organelle behind and squeezes what is in it', async () => {
  // What dragSelection fixes: translateSelection on the same selection moves
  // the metabolites but not the membrane, so pushed far enough they pile up at
  // the membrane instead of keeping their shape.
  const map = await yeast();
  const { cell, org } = organelle(map);
  const selection = selectableInRect(map, around(org.rect, 4));
  const carried = inside(map, cell, org).filter((id) => selection.nodes.has(id));
  const before = carried.map((id) => map.nodes.get(id).x);
  map.translateSelection(selection, -100000, 0);
  assert.deepEqual(rectOf(map, cell.id, org.id), org.rect, 'the membrane stayed where it was');
  const spreadBefore = Math.max(...before) - Math.min(...before);
  const after = carried.map((id) => map.nodes.get(id).x);
  assert.ok(Math.max(...after) - Math.min(...after) < spreadBefore, 'the contents were squeezed');
});

test('a selection stops together when its organelle meets its parent', async () => {
  const map = await yeast();
  const { cell, org, parent } = organelle(map);
  const selection = selectableInRect(map, around(org.rect, 4));
  const carried = inside(map, cell, org);
  const before = carried.map((id) => map.nodes.get(id).x);

  const moved = map.dragSelection(selection, -100000, 0);
  assert.ok(moved.dx > -100000 && moved.dx <= 0);
  const r = rectOf(map, cell.id, org.id);
  assert.ok(r.x >= parent.rect.x + 10 - 1e-6, `the organelle left its parent: ${r.x}`);
  carried.forEach((id, i) => {
    assert.ok(Math.abs(map.nodes.get(id).x - (before[i] + moved.dx)) < 1e-9, `${id} did not stop with its organelle`);
  });
});

/* Where every branch's ends and handles are, by segment index. */
function curves(map) {
  return map.segments.map((seg) => ({
    nodeId: seg.nodeId,
    reactionId: seg.reactionId,
    b1: seg.b1 && { ...seg.b1 },
    b2: seg.b2 && { ...seg.b2 },
  }));
}

function shiftedBy(a, b, d) {
  return Math.abs(b.x - a.x - d.x) < 1e-6 && Math.abs(b.y - a.y - d.y) < 1e-6;
}

test('curves keep their shape: b1 goes with its metabolite, b2 with its reaction', async () => {
  // Moving an organelle takes its transporters along. A curve from one of them
  // to a cytosolic metabolite that stays must stretch, not loop: its metabolite
  // end handle (b1) stays with the metabolite. One whose metabolite comes too
  // moves whole.
  const map = await yeast();
  const { org } = organelle(map);
  // A drawn layout leaves handles to the renderer; a map saved after editing
  // has them on every branch, and those are what a move can bend.
  const lerp = (a, b, t) => ({ x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t });
  for (const seg of map.segments) {
    if (!seg.nodeId || !seg.from || !seg.to || seg.b1) continue;
    const [m, r] = seg.side === 'product' ? [seg.to, seg.from] : [seg.from, seg.to];
    seg.b1 = lerp(m, r, 1 / 3);
    seg.b2 = lerp(m, r, 2 / 3);
  }
  // Tight round the organelle, so most cytosolic partners are left behind.
  const selection = selectableInRect(map, around(org.rect, 1));
  const before = curves(map);
  const nodes = new Map([...map.nodes.values()].map((n) => [n.id, { x: n.x, y: n.y }]));
  const reactions = new Map([...map.reactions.values()].map((r) => [r.id, { x: r.x, y: r.y }]));
  const moved = map.dragSelection(selection, -15, 10);
  const d = { x: moved.dx, y: moved.dy };
  assert.ok(d.x !== 0 || d.y !== 0);
  const zero = { x: 0, y: 0 };

  let stretched = 0;
  let whole = 0;
  map.segments.forEach((seg, i) => {
    const was = before[i];
    if (!was.nodeId || !was.b1 || !was.b2) return;
    const node = map.nodes.get(was.nodeId);
    const rxn = map.reactions.get(was.reactionId);
    if (!node || !rxn) return;
    const nodeMoved = shiftedBy(nodes.get(was.nodeId), node, d);
    const nodeStayed = shiftedBy(nodes.get(was.nodeId), node, zero);
    const rxnMoved = shiftedBy(reactions.get(was.reactionId), rxn, d);
    if (rxnMoved && nodeStayed) {
      stretched += 1;
      assert.ok(shiftedBy(was.b1, seg.b1, zero), `b1 of ${was.reactionId} → ${was.nodeId} left its metabolite`);
      assert.ok(shiftedBy(was.b2, seg.b2, d), `b2 of ${was.reactionId} → ${was.nodeId} did not follow its reaction`);
    } else if (rxnMoved && nodeMoved) {
      whole += 1;
      assert.ok(shiftedBy(was.b1, seg.b1, d) && shiftedBy(was.b2, seg.b2, d),
        `curve ${was.reactionId} → ${was.nodeId} did not move whole`);
    }
  });
  assert.ok(stretched > 0, 'some curve reaches a metabolite left behind');
  assert.ok(whole > 0, 'some curve moves whole');
});

test('a cytosolic metabolite drawn inside the moving organelle rides with it', async () => {
  // Inner-membrane transporters are drawn with their cytosolic side in the
  // intermembrane band: inside the organelle's outline, but not of it. Moved the
  // ordinary way such a node is pushed out of the organelle and left behind.
  const map = await yeast();
  const { cell, org } = organelle(map);
  const inner = cell.compartments.find((k) => k.parentId === org.id).rect;
  const cytosolic = [...map.nodes.values()].find((n) => n.compartment === org.parentId && n.kind === 'metabolite' && !n.reactionId);
  assert.ok(cytosolic, 'a cytosolic metabolite to place');
  cytosolic.x = (org.rect.x + inner.x) / 2;
  cytosolic.y = inner.y + inner.h / 2;
  const before = { x: cytosolic.x, y: cytosolic.y };

  const selection = selectableInRect(map, around(org.rect, 4));
  assert.ok(selection.nodes.has(cytosolic.id));
  const moved = map.dragSelection(selection, 12, 9);
  assert.ok(moved.dx !== 0 || moved.dy !== 0);
  const node = map.nodes.get(cytosolic.id);
  assert.ok(Math.abs(node.x - before.x - moved.dx) < 1e-9 && Math.abs(node.y - before.y - moved.dy) < 1e-9,
    `it went to ${node.x},${node.y} instead of riding along from ${before.x},${before.y}`);
});

test('without compartments or cofactors a drag is one rigid move by the step it takes', async () => {
  const a = await yeast();
  const b = await yeast();
  const { org } = organelle(a);
  const found = selectableInRect(a, around(org.rect, 4));
  const selection = {
    nodes: new Set([...found.nodes].filter((id) => !a.nodes.get(id).reactionId)),
    reactions: found.reactions,
    texts: found.texts,
    compartments: new Set(),
  };
  // Every member goes by the one step the whole selection can take; none is
  // held back, or pulled in, on its own.
  const step = a.dragSelection(selection, 7, -5);
  b.translateSelection(selection, step.dx, step.dy, { rigid: true });
  assert.deepEqual(a.toJSON(), b.toJSON());
});

test('reactions dragged into a wall stop there together, keeping their shape', async () => {
  // Held back by the wall one member at a time, the ones that reached it
  // would stop while the rest went on, and the curves between them would be
  // dragged across the map.
  const map = await yeast();
  const cytosol = map.cells[0].compartments.find((c) => !c.parentId).rect;
  const block = {
    x: cytosol.x + cytosol.w * 0.05, y: cytosol.y + cytosol.h * 0.2,
    w: cytosol.w * 0.35, h: cytosol.h * 0.3,
  };
  const selection = selectableInRect(map, block);
  selection.compartments = new Set();
  assert.ok(selection.reactions.size >= 5, 'the block holds several reactions');
  map.dragSelection(selection, -8, 0); // the first move settles every backbone
  const where = () => new Map([
    ...[...selection.nodes].map((id) => [id, { ...map.nodes.get(id) }]),
    ...[...selection.reactions].map((id) => [id, { ...map.reactions.get(id) }]),
  ]);
  const start = where();
  let total = 0;
  for (let i = 0; i < 120; i += 1) total += map.dragSelection(selection, -8, 0).dx;
  assert.ok(total > -960, 'the wall stopped the selection');
  for (const [id, p] of where()) {
    const q = start.get(id);
    assert.ok(Math.abs(p.x - q.x - total) < 1e-6 && Math.abs(p.y - q.y) < 1e-6,
      `${id} moved by ${p.x - q.x},${p.y - q.y}, the selection by ${total},0`);
  }
});

test('a cofactor selected with its reaction moves once, with its reaction', async () => {
  const map = await yeast();
  let rid = null;
  let cofactor = null;
  for (const [id, rxn] of map.reactions) {
    if (rxn.onMembrane) continue;
    const owned = [...map.nodes.values()].find((n) => n.reactionId === id);
    if (owned) { rid = id; cofactor = owned.id; break; }
  }
  assert.ok(rid, 'some reaction draws its own cofactor');
  const selection = emptySelection();
  selection.reactions.add(rid);
  selection.nodes.add(cofactor);
  const offset = (m) => {
    const n = m.nodes.get(cofactor);
    const r = m.reactions.get(rid);
    return { x: n.x - r.x, y: n.y - r.y };
  };

  const start = offset(map);
  map.dragSelection(selection, 3, 2);
  const after = offset(map);
  assert.ok(Math.abs(after.x - start.x) < 1e-9 && Math.abs(after.y - start.y) < 1e-9,
    `the cofactor drifted off its reaction: ${JSON.stringify(start)} -> ${JSON.stringify(after)}`);

  // translateSelection moves it, and then the reaction carries it: twice the drag.
  const port = await yeast();
  port.translateSelection(selection, 3, 2);
  const doubled = offset(port);
  assert.ok(Math.abs(doubled.x - start.x) > 1 || Math.abs(doubled.y - start.y) > 1);
});

test('the store selects compartments by rectangle and its drag moves them', async () => {
  const store = new Store('.');
  store.map = await yeast();
  const { cell, org } = organelle(store.map);
  store.selectInRect(around(org.rect, 4));
  assert.ok(store.multiSelection.compartments.has(compartmentKey(cell.id, org.id)));
  const moved = store.moveMultiSelection(0, 5);
  assert.ok(moved.compartments.length > 0);
  assert.equal(rectOf(store.map, cell.id, org.id).y, org.rect.y + moved.dy);
});

test('a selected compartment is outlined in the drawing', async () => {
  const map = await yeast();
  const { org } = organelle(map);
  const selection = selectableInRect(map, around(org.rect, 4));
  const svg = renderMap(map, { fluxes: new Map(), scale: new FluxScale([]), multiSelection: selection });
  assert.match(svg, /class="emc-multi-compartment"/);
  const none = renderMap(map, { fluxes: new Map(), scale: new FluxScale([]), multiSelection: emptySelection() });
  assert.doesNotMatch(none, /emc-multi-compartment/);
});
