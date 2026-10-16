/**
 * Manual node placement.
 *
 * Everything here runs on a real layout built from the bundled models, with no
 * browser: the drag module is a pure mutation of the layout object, so the parts
 * that can actually be wrong — the coordinate conversion, the compartment
 * constraint and the segment bookkeeping — are all testable outside the DOM.
 */

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import { layoutCommunity, CLEARANCE } from '../src/layout/layout.js';
import { renderMap } from '../src/render/mapview.js';
import { FluxScale } from '../src/render/theme.js';
import { signedDistance, center, inset } from '../src/layout/geometry.js';
import {
  stageToMap, isDraggableNode, nodeConstraint, clampNodePosition,
  moveLayoutNode, updateConnectedSegments, DRAGGABLE_NODE_KINDS,
} from '../src/layout/drag.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

async function loadModel(name) {
  return new Model(JSON.parse(await readFile(join(root, 'models', `${name}.json`), 'utf8')));
}

async function buildLayout(names = [['eco', 'ecoli_core_mc'], ['sce', 'yeast_core_mc']]) {
  const species = [];
  for (const [id, file] of names) {
    species.push({ id, model: await loadModel(file), abundance: 1 / names.length });
  }
  const com = new CommunityModel({
    species,
    medium: {
      glc__D: { lower: -10 }, o2: { lower: -20 }, nh4: { lower: -1000 },
      pi: { lower: -1000 }, h2o: { lower: -1000 }, h: { lower: -1000 },
      co2: { lower: -1000 }, photon: { lower: -80 },
    },
  });
  return { com, layout: layoutCommunity(com) };
}

const nodesOfKind = (layout, kind) =>
  [...layout.nodes.values()].filter((node) => node.kind === kind);

/** A node with at least `room` of slack on every side, so a test move is not clamped. */
function roomyNode(layout, kind, room) {
  return nodesOfKind(layout, kind).find((node) => {
    const c = nodeConstraint(layout, node);
    if (!c) return false;
    if (signedDistance(c.bounds, node) > -(c.pad + room)) return false;
    return c.holes.every((hole) => signedDistance(hole, node) > c.clearance + room);
  });
}

// Pointer conversion

test('stage coordinates convert to map coordinates at any zoom and pan', () => {
  const box = { left: 64, top: 120 };
  for (const camera of [
    { x: 0, y: 0, scale: 1 },
    { x: 37, y: -210, scale: 0.2718 },     // fitted on a big canvas
    { x: -1450, y: -880, scale: 3.4 },     // zoomed in, panned away from origin
    { x: 12, y: 8, scale: 0.05 },          // zoomed all the way out
  ]) {
    for (const map of [{ x: 0, y: 0 }, { x: 512.5, y: 91.25 }, { x: 2400, y: 1600 }]) {
      // Forward: the camera transform the renderer applies to the SVG element.
      const client = {
        x: map.x * camera.scale + camera.x + box.left,
        y: map.y * camera.scale + camera.y + box.top,
      };
      const back = stageToMap(client, camera, box);
      assert.ok(Math.abs(back.x - map.x) < 1e-6, `x ${back.x} != ${map.x} at ${camera.scale}`);
      assert.ok(Math.abs(back.y - map.y) < 1e-6, `y ${back.y} != ${map.y} at ${camera.scale}`);
    }
  }
});

test('a zero scale does not produce a non-finite map coordinate', () => {
  const p = stageToMap({ x: 10, y: 10 }, { x: 0, y: 0, scale: 0 }, { left: 0, top: 0 });
  assert.ok(Number.isFinite(p.x) && Number.isFinite(p.y), JSON.stringify(p));
});

// Eligibility

test('primary, shared-medium and secondary metabolites are draggable', async () => {
  const { layout } = await buildLayout();
  assert.deepEqual([...DRAGGABLE_NODE_KINDS].sort(), ['currency', 'medium', 'metabolite']);

  const seen = new Set();
  for (const node of layout.nodes.values()) {
    seen.add(node.kind);
    assert.equal(isDraggableNode(node), ['metabolite', 'medium', 'currency'].includes(node.kind),
      `${node.id} (${node.kind}) has the wrong eligibility`);
  }
  for (const kind of ['metabolite', 'medium', 'currency']) {
    assert.ok(seen.has(kind), `no ${kind} nodes on the map`);
  }
  assert.equal(isDraggableNode(null), false);
  assert.equal(isDraggableNode(undefined), false);
});

test('a secondary currency metabolite can be moved by the layout API', async () => {
  const { layout } = await buildLayout();
  const node = nodesOfKind(layout, 'currency')[0];
  assert.ok(node, 'no currency nodes on the map');
  const before = { x: node.x, y: node.y };

  const moved = moveLayoutNode(layout, node.id, before.x + 35, before.y + 25);
  assert.equal(moved, node);
  assert.notDeepEqual({ x: node.x, y: node.y }, before);
});

test('a reaction marker is not a node and cannot be moved', async () => {
  const { layout } = await buildLayout();
  const rid = [...layout.reactions.keys()][0];
  const rxn = layout.reactions.get(rid);
  const before = { x: rxn.x, y: rxn.y };
  assert.equal(moveLayoutNode(layout, rid, before.x + 90, before.y + 90), null);
  assert.deepEqual({ x: rxn.x, y: rxn.y }, before);
});

// Moves

test('moving a metabolite changes its x and y', async () => {
  const { layout } = await buildLayout();
  const node = roomyNode(layout, 'metabolite', 60);
  assert.ok(node, 'no metabolite with room to move');
  const before = { x: node.x, y: node.y };

  const moved = moveLayoutNode(layout, node.id, before.x + 40, before.y - 25);
  assert.equal(moved, node, 'the moved node was not reported back');
  assert.ok(Math.abs(node.x - (before.x + 40)) < 1e-6, `x is ${node.x}`);
  assert.ok(Math.abs(node.y - (before.y - 25)) < 1e-6, `y is ${node.y}`);
});

test('a move is rejected rather than writing NaN into the layout', async () => {
  const { layout } = await buildLayout();
  const node = roomyNode(layout, 'metabolite', 60);
  const before = { x: node.x, y: node.y };
  assert.equal(moveLayoutNode(layout, node.id, NaN, 10), null);
  assert.equal(moveLayoutNode(layout, node.id, 10, Infinity), null);
  assert.deepEqual({ x: node.x, y: node.y }, before);
});

test('an unknown node id is a no-op', async () => {
  const { layout } = await buildLayout();
  assert.equal(moveLayoutNode(layout, 'met:does-not-exist', 10, 10), null);
  assert.equal(updateConnectedSegments(layout, 'met:does-not-exist'), 0);
});

// Segments

test('connected segment endpoints follow the metabolite', async () => {
  const { layout } = await buildLayout();
  const node = roomyNode(layout, 'metabolite', 60);
  const attached = layout.segments.filter((seg) => seg.nodeId === node.id);
  assert.ok(attached.length > 0, `${node.id} has no segments`);

  const target = { x: node.x + 55, y: node.y + 30 };
  moveLayoutNode(layout, node.id, target.x, target.y);

  for (const seg of attached) {
    // Substrates run metabolite -> axis, products axis -> metabolite.
    const endpoint = seg.side === 'substrate' ? seg.from : seg.to;
    const other = seg.side === 'substrate' ? seg.to : seg.from;
    assert.ok(Math.abs(endpoint.x - target.x) < 1e-6 && Math.abs(endpoint.y - target.y) < 1e-6,
      `${seg.reactionId} ${seg.side} endpoint is at ${endpoint.x},${endpoint.y}`);
    // The reaction axis is deliberately left where it was.
    const axis = layout.reactions.get(seg.reactionId).axis;
    const anchor = seg.side === 'substrate' ? axis.a1 : axis.a2;
    assert.deepEqual({ x: other.x, y: other.y }, { x: anchor.x, y: anchor.y },
      `${seg.reactionId} axis endpoint moved`);
  }
});

test('a hub metabolite updates the segments of every reaction it touches', async () => {
  const { layout } = await buildLayout();
  // The busiest movable node, so more than one reaction is involved.
  const counts = new Map();
  for (const seg of layout.segments) {
    if (!seg.nodeId) continue;
    counts.set(seg.nodeId, (counts.get(seg.nodeId) ?? 0) + 1);
  }
  let best = null;
  for (const [id, count] of counts) {
    const node = layout.nodes.get(id);
    if (!isDraggableNode(node)) continue;
    if (!best || count > best.count) best = { id, count };
  }
  assert.ok(best && best.count > 2, `no well-connected movable node (${best?.count})`);

  const attached = layout.segments.filter((seg) => seg.nodeId === best.id);
  const reactions = new Set(attached.map((seg) => seg.reactionId));
  assert.ok(reactions.size > 1, `only ${reactions.size} reaction(s) on the hub`);

  const node = layout.nodes.get(best.id);
  moveLayoutNode(layout, best.id, node.x + 12, node.y + 12);
  for (const seg of attached) {
    const endpoint = seg.side === 'substrate' ? seg.from : seg.to;
    assert.ok(Math.abs(endpoint.x - node.x) < 1e-6 && Math.abs(endpoint.y - node.y) < 1e-6,
      `${seg.reactionId} did not follow the hub`);
  }
});

test('unrelated segments are left untouched', async () => {
  const { layout } = await buildLayout();
  const node = roomyNode(layout, 'metabolite', 60);

  const snapshot = layout.segments
    .filter((seg) => seg.nodeId !== node.id)
    .map((seg) => ({
      seg,
      from: { x: seg.from.x, y: seg.from.y },
      to: { x: seg.to.x, y: seg.to.y },
    }));
  assert.ok(snapshot.length > 100, `only ${snapshot.length} other segments`);

  moveLayoutNode(layout, node.id, node.x + 45, node.y + 45);

  for (const { seg, from, to } of snapshot) {
    assert.deepEqual({ x: seg.from.x, y: seg.from.y }, from,
      `${seg.reactionId}/${seg.kind} 'from' moved`);
    assert.deepEqual({ x: seg.to.x, y: seg.to.y }, to,
      `${seg.reactionId}/${seg.kind} 'to' moved`);
  }
});

test('other nodes and every reaction marker stay where they were', async () => {
  const { layout } = await buildLayout();
  const node = roomyNode(layout, 'metabolite', 60);
  const others = [...layout.nodes.values()]
    .filter((other) => other.id !== node.id)
    .map((other) => ({ other, x: other.x, y: other.y }));
  const markers = [...layout.reactions.values()].map((rxn) => ({ rxn, x: rxn.x, y: rxn.y }));

  moveLayoutNode(layout, node.id, node.x + 30, node.y + 30);

  for (const { other, x, y } of others) {
    assert.deepEqual({ x: other.x, y: other.y }, { x, y }, `${other.id} moved`);
  }
  for (const { rxn, x, y } of markers) {
    assert.deepEqual({ x: rxn.x, y: rxn.y }, { x, y }, `marker ${rxn.id} moved`);
  }
});

// Constraints

test('a cellular metabolite is clamped to its own compartment', async () => {
  const { layout } = await buildLayout();
  let checked = 0;

  for (const node of nodesOfKind(layout, 'metabolite')) {
    const c = nodeConstraint(layout, node);
    assert.ok(c, `no constraint for ${node.id}`);

    // Aim far outside the cell, in all four directions.
    for (const [dx, dy] of [[-9e4, 0], [9e4, 0], [0, -9e4], [0, 9e4], [9e4, 9e4]]) {
      const p = clampNodePosition(layout, node, node.x + dx, node.y + dy);
      assert.ok(signedDistance(c.bounds, p) <= -(c.pad - 1e-6),
        `${node.id} escaped ${node.compartment} towards ${dx},${dy}`);
      for (const hole of c.holes) {
        assert.ok(signedDistance(hole, p) >= c.clearance - 1e-6,
          `${node.id} entered a nested compartment towards ${dx},${dy}`);
      }
    }
    checked += 1;
  }
  assert.ok(checked > 50, `only checked ${checked} nodes`);
});

test('a metabolite cannot be dragged into a nested child compartment', async () => {
  // Yeast has a mitochondrion inside a mitochondrial membrane inside the cytosol,
  // so its cytosolic metabolites have real holes to be kept out of.
  const { layout } = await buildLayout([['sce', 'yeast_core_mc']]);
  const cell = layout.cells[0];
  const nested = cell.compartments.filter((comp) => comp.parentId);
  assert.ok(nested.length >= 2, `expected nested compartments, got ${nested.length}`);

  let checked = 0;
  for (const child of nested) {
    const parentNodes = nodesOfKind(layout, 'metabolite')
      .filter((node) => node.compartment === child.parentId);
    if (!parentNodes.length) continue;
    const aim = center(child.rect);
    for (const node of parentNodes) {
      // Drop it straight onto the middle of the organelle.
      const p = clampNodePosition(layout, node, aim.x, aim.y);
      assert.ok(signedDistance(child.rect, p) >= node.r + CLEARANCE.hole - 1e-6,
        `${node.id} landed inside ${child.id} at ${p.x.toFixed(1)},${p.y.toFixed(1)}`);
      checked += 1;
    }
  }
  assert.ok(checked > 10, `only checked ${checked} placements`);
});

test('a shared-medium metabolite cannot be dragged inside a cell', async () => {
  const { layout } = await buildLayout();
  const mediums = nodesOfKind(layout, 'medium');
  assert.ok(mediums.length > 3, `only ${mediums.length} medium nodes`);

  let checked = 0;
  for (const node of mediums) {
    for (const cell of layout.cells) {
      // Aim at the middle of the cell, and at each of its corners.
      const targets = [
        center(cell.rect),
        { x: cell.rect.x + 4, y: cell.rect.y + 4 },
        { x: cell.rect.x + cell.rect.w - 4, y: cell.rect.y + cell.rect.h - 4 },
      ];
      for (const target of targets) {
        const p = clampNodePosition(layout, node, target.x, target.y);
        assert.ok(signedDistance(cell.rect, p) > 0,
          `${node.base} landed inside cell ${cell.id} at ${p.x.toFixed(1)},${p.y.toFixed(1)}`);
        checked += 1;
      }
    }
  }
  assert.ok(checked > 20, `only checked ${checked} placements`);
});

test('a shared-medium metabolite stays inside the shared environment', async () => {
  const { layout } = await buildLayout();
  const bounds = inset(layout.environment.rect, CLEARANCE.environment);

  for (const node of nodesOfKind(layout, 'medium')) {
    for (const [dx, dy] of [[-9e4, -9e4], [9e4, -9e4], [-9e4, 9e4], [9e4, 9e4]]) {
      const p = clampNodePosition(layout, node, node.x + dx, node.y + dy);
      assert.ok(signedDistance(bounds, p) <= -(node.r + CLEARANCE.bounds - 1e-6),
        `${node.base} left the environment towards ${dx},${dy}`);
    }
  }
});

test('a clamped move still only ever moves the node it was asked about', async () => {
  const { layout } = await buildLayout();
  const node = nodesOfKind(layout, 'metabolite')[0];
  const others = [...layout.nodes.values()]
    .filter((other) => other.id !== node.id)
    .map((other) => ({ other, x: other.x, y: other.y }));

  moveLayoutNode(layout, node.id, -5e4, -5e4);   // far outside everything
  assert.ok(Number.isFinite(node.x) && Number.isFinite(node.y), `${node.x},${node.y}`);
  for (const { other, x, y } of others) {
    assert.deepEqual({ x: other.x, y: other.y }, { x, y }, `${other.id} moved`);
  }
});

test('a node with no compartment on the map is left unconstrained', async () => {
  const { layout } = await buildLayout();
  const orphan = { id: 'met:ghost', kind: 'metabolite', cellId: 'nope', compartment: 'c', r: 13, x: 0, y: 0 };
  assert.equal(nodeConstraint(layout, orphan), null);
  assert.deepEqual(clampNodePosition(layout, orphan, 77, 88), { x: 77, y: 88 });
});

// Integration

test('the moved position survives a repaint and lands in the SVG', async () => {
  const { layout } = await buildLayout();
  const node = roomyNode(layout, 'metabolite', 60);
  moveLayoutNode(layout, node.id, node.x + 37, node.y + 21);

  const svg = renderMap(layout, { fluxes: new Map(), scale: new FluxScale([]) });
  const round = (v) => (Math.round(v * 100) / 100).toString();
  assert.ok(
    svg.includes(`data-node="${node.id}" data-met="${node.mergedId}" cx="${round(node.x)}" cy="${round(node.y)}"`),
    `the circle for ${node.id} was not drawn at ${round(node.x)},${round(node.y)}`,
  );
  // The label is placed from the same coordinates, so it follows automatically.
  assert.ok(svg.includes(`x="${round(node.x)}" y="${round(node.y - node.r - 5)}"`),
    'the label did not follow the node');
  assert.ok(!/NaN|undefined|Infinity/.test(svg), 'a numeric attribute did not resolve');
});

test('dragging never disturbs the model, the bounds or the reaction set', async () => {
  const { com, layout } = await buildLayout();
  const before = {
    reactions: com.merged.reactions.size,
    metabolites: com.merged.metabolites.size,
    bounds: [...com.merged.reactions.values()].map((r) => `${r.id}:${r.lowerBound}:${r.upperBound}`),
    drawn: [...layout.reactions.keys()].sort(),
    segments: layout.segments.length,
    nodes: layout.nodes.size,
  };

  for (const node of [...layout.nodes.values()].filter(isDraggableNode).slice(0, 25)) {
    moveLayoutNode(layout, node.id, node.x + 33, node.y - 19);
  }

  assert.equal(com.merged.reactions.size, before.reactions);
  assert.equal(com.merged.metabolites.size, before.metabolites);
  assert.deepEqual([...com.merged.reactions.values()].map((r) => `${r.id}:${r.lowerBound}:${r.upperBound}`),
    before.bounds, 'a reaction bound changed');
  assert.deepEqual([...layout.reactions.keys()].sort(), before.drawn);
  assert.equal(layout.segments.length, before.segments, 'segments were added or removed');
  assert.equal(layout.nodes.size, before.nodes, 'nodes were added or removed');
});

test('repeated moves are deterministic', async () => {
  const runs = [];
  for (let i = 0; i < 2; i++) {
    const { layout } = await buildLayout([['eco', 'ecoli_core_mc']]);
    const movable = [...layout.nodes.values()].filter(isDraggableNode);
    for (const node of movable) moveLayoutNode(layout, node.id, node.x + 400, node.y + 400);
    runs.push(movable.map((node) => `${node.id}:${node.x.toFixed(6)},${node.y.toFixed(6)}`));
  }
  assert.deepEqual(runs[0], runs[1]);
});
