import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import { EditableMap } from '../src/map/editable-map.js';
import { reactionBoundaryKeys } from '../src/map/boundaries.js';
import { renderMap } from '../src/render/mapview.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

async function community() {
  const raw = JSON.parse(await readFile(join(root, 'models', 'ecoli_core_mc.json'), 'utf8'));
  return new CommunityModel({
    species: [{ id: 'eco', model: new Model(raw), abundance: 1 }],
    medium: {},
  });
}

async function talaMap() {
  const com = await community();
  const map = EditableMap.emptyFromCommunity(com);
  const rid = 'eco@TALA';
  assert.ok(com.merged.reactions.has(rid));
  assert.ok(map.addReaction(com, rid));
  return { com, map, rid };
}

test('an internal reaction marker can be moved without moving primary metabolites', async () => {
  const { map, rid } = await talaMap();
  const rxn = map.reactions.get(rid);
  const before = [...map.nodes.values()]
    .filter((n) => !n.reactionId)
    .map((n) => [n.id, n.x, n.y]);
  const x0 = rxn.x;
  const y0 = rxn.y;

  assert.ok(map.moveReaction(rid, x0 + 35, y0 + 25));
  assert.notEqual(rxn.x, x0);
  assert.notEqual(rxn.y, y0);
  assert.deepEqual(
    [...map.nodes.values()].filter((n) => !n.reactionId).map((n) => [n.id, n.x, n.y]),
    before,
  );
});

test('a transporter drag stays on its original membrane', async () => {
  const com = await community();
  const rid = [...com.merged.reactions.keys()]
    .find((id) => reactionBoundaryKeys(com, id).some((k) => k.startsWith('eco/')));
  assert.ok(rid);

  const key = reactionBoundaryKeys(com, rid).find((k) => k.startsWith('eco/'));
  const map = EditableMap.emptyFromCommunity(com);
  const b = map.boundary(key);
  map.addReaction(com, rid, { boundaryId: key, anchor: { x: b.rect.x, y: b.rect.y + 20 } });

  const rxn = map.reactions.get(rid);
  assert.ok(map.moveReaction(rid, b.rect.x + b.rect.w / 2, b.rect.y - 300));

  const onTop = Math.abs(rxn.y - b.rect.y) < 1e-9;
  const onBottom = Math.abs(rxn.y - (b.rect.y + b.rect.h)) < 1e-9;
  const onLeft = Math.abs(rxn.x - b.rect.x) < 1e-9;
  const onRight = Math.abs(rxn.x - (b.rect.x + b.rect.w)) < 1e-9;
  assert.ok(onTop || onBottom || onLeft || onRight);
  assert.equal(rxn.boundaryId, key);
});

test('TALA exposes three Escher-style control points per branch', async () => {
  const { map, rid } = await talaMap();
  const controls = map.reactionControlPoints(rid);

  assert.equal(controls.length, 12);
  assert.equal(controls.filter((p) => p.handle === 'b1').length, 4);
  assert.equal(controls.filter((p) => p.handle === 'b2').length, 4);
  assert.equal(controls.filter((p) => p.handle === 'anchor').length, 4);
  assert.equal(new Set(controls.map((p) => p.nodeId)).size, 4);
});

test('TALA branches attach to four distinct reaction-backbone points', async () => {
  const { map, rid } = await talaMap();
  map.reactionControlPoints(rid); // upgrades legacy/full geometry when needed

  const branches = map.segmentsOf(rid).filter((seg) => seg.nodeId && seg.backboneAnchor);
  assert.equal(branches.length, 4);

  const anchors = branches.map((seg) =>
    seg.side === 'substrate' ? seg.to : seg.from
  );
  const unique = new Set(anchors.map((p) => `${p.x.toFixed(6)}:${p.y.toFixed(6)}`));
  assert.equal(unique.size, 4);
});

test('moving b1 changes only the metabolite-side handle of that branch', async () => {
  const { map, rid } = await talaMap();
  const before = map.reactionControlPoints(rid);
  const chosen = before.find((p) => p.handle === 'b1');
  const partner = before.find((p) => p.nodeId === chosen.nodeId && p.handle === 'b2');

  map.moveBranchControl(rid, chosen.nodeId, 'b1', chosen.x + 25, chosen.y - 18);
  const after = map.reactionControlPoints(rid);
  const moved = after.find((p) => p.nodeId === chosen.nodeId && p.handle === 'b1');
  const samePartner = after.find((p) => p.nodeId === chosen.nodeId && p.handle === 'b2');

  assert.equal(moved.x, chosen.x + 25);
  assert.equal(moved.y, chosen.y - 18);
  assert.deepEqual(samePartner, partner);
});

test('moving a metabolite carries b1 but leaves b2 attached to the reaction', async () => {
  const { map, rid } = await talaMap();
  const controls = map.reactionControlPoints(rid);
  const c1 = controls.find((p) => p.handle === 'b1');
  const c2 = controls.find((p) => p.nodeId === c1.nodeId && p.handle === 'b2');
  const node = map.nodes.get(c1.nodeId);
  const oldNode = { x: node.x, y: node.y };

  map.moveNode(node.id, node.x + 18, node.y + 11);
  const dx = node.x - oldNode.x;
  const dy = node.y - oldNode.y;
  const after = map.reactionControlPoints(rid);
  const b1 = after.find((p) => p.nodeId === node.id && p.handle === 'b1');
  const b2 = after.find((p) => p.nodeId === node.id && p.handle === 'b2');

  assert.ok(Math.abs(b1.x - (c1.x + dx)) < 1e-9);
  assert.ok(Math.abs(b1.y - (c1.y + dy)) < 1e-9);
  assert.deepEqual(b2, c2);
});

test('moving the reaction carries b2/backbone but leaves b1 with the metabolite', async () => {
  const { map, rid } = await talaMap();
  const before = map.reactionControlPoints(rid);
  const c1 = before.find((p) => p.handle === 'b1');
  const c2 = before.find((p) => p.nodeId === c1.nodeId && p.handle === 'b2');
  const rxn = map.reactions.get(rid);
  const old = { x: rxn.x, y: rxn.y };

  map.moveReaction(rid, rxn.x + 20, rxn.y + 15);
  const dx = rxn.x - old.x;
  const dy = rxn.y - old.y;
  const after = map.reactionControlPoints(rid);
  const b1 = after.find((p) => p.nodeId === c1.nodeId && p.handle === 'b1');
  const b2 = after.find((p) => p.nodeId === c1.nodeId && p.handle === 'b2');

  assert.deepEqual(b1, c1);
  assert.ok(Math.abs(b2.x - (c2.x + dx)) < 1e-9);
  assert.ok(Math.abs(b2.y - (c2.y + dy)) < 1e-9);
});

test('old quadratic controls upgrade to two cubic controls', async () => {
  const { map, rid } = await talaMap();
  const seg = map.segmentsOf(rid).find((x) => x.nodeId);
  delete seg.b1;
  delete seg.b2;
  seg.control = {
    x: (seg.from.x + seg.to.x) / 2 + 10,
    y: (seg.from.y + seg.to.y) / 2 - 7,
  };

  const handles = map.reactionControlPoints(rid).filter((p) => p.nodeId === seg.nodeId);
  assert.equal(handles.length, 3);
  assert.ok(handles.some((p) => p.handle === 'b1'));
  assert.ok(handles.some((p) => p.handle === 'b2'));
  assert.ok(handles.some((p) => p.handle === 'anchor'));
});

test('cubic handles and backbone anchors survive map JSON save/load', async () => {
  const { map, rid } = await talaMap();
  const c = map.reactionControlPoints(rid).find((p) => p.handle === 'b1');
  map.moveBranchControl(rid, c.nodeId, 'b1', c.x + 11, c.y + 13);

  const back = EditableMap.fromJSON(JSON.parse(JSON.stringify(map.toJSON())));
  assert.deepEqual(back.toJSON(), map.toJSON());
  assert.equal(back.reactionControlPoints(rid).length, 12);
});


test('moving the third anchor control moves the branch attachment and carries b2', async () => {
  const { map, rid } = await talaMap();
  const before = map.reactionControlPoints(rid);
  const anchor = before.find((p) => p.handle === 'anchor');
  const b1 = before.find((p) => p.nodeId === anchor.nodeId && p.handle === 'b1');
  const b2 = before.find((p) => p.nodeId === anchor.nodeId && p.handle === 'b2');

  const dx = 14;
  const dy = -9;
  map.moveBranchControl(rid, anchor.nodeId, 'anchor', anchor.x + dx, anchor.y + dy);

  const after = map.reactionControlPoints(rid);
  const movedAnchor = after.find((p) => p.nodeId === anchor.nodeId && p.handle === 'anchor');
  const movedB1 = after.find((p) => p.nodeId === anchor.nodeId && p.handle === 'b1');
  const movedB2 = after.find((p) => p.nodeId === anchor.nodeId && p.handle === 'b2');

  assert.equal(movedAnchor.x, anchor.x + dx);
  assert.equal(movedAnchor.y, anchor.y + dy);
  assert.deepEqual(movedB1, b1);
  assert.equal(movedB2.x, b2.x + dx);
  assert.equal(movedB2.y, b2.y + dy);
});


test('global Edit mode exposes controls for every drawn reaction without selecting them one by one', async () => {
  const com = await community();
  const map = EditableMap.emptyFromCommunity(com);
  for (const rid of ['eco@TALA', 'eco@GAPD']) assert.ok(map.addReaction(com, rid));

  const svg = renderMap(map, {
    fluxes: new Map(),
    scale: { color: () => '#94a3b8', width: () => 1 },
    selectedReaction: null,
    reactionEdit: true,
    showCurrency: true,
  });

  assert.match(svg, /data-reaction-handle="eco@TALA"/);
  assert.match(svg, /data-reaction-handle="eco@GAPD"/);
  assert.match(svg, /data-control-rxn="eco@TALA"/);
  assert.match(svg, /data-control-rxn="eco@GAPD"/);
});
