import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import { EditableMap } from '../src/map/editable-map.js';
import {
  emptySelection,
  alignSelection,
  distributeSelection,
  selectableInRect,
  selectionSize,
  textCentre,
  translateSelection,
} from '../src/map/multi-selection.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

async function setup() {
  const raw = JSON.parse(await readFile(join(root, 'models', 'ecoli_core_mc.json'), 'utf8'));
  const community = new CommunityModel({
    species: [{ id: 'eco', model: new Model(raw), abundance: 1 }],
    medium: {},
  });
  const map = EditableMap.emptyFromCommunity(community);
  map.addReaction(community, 'eco@TALA');
  return { community, map };
}

test('empty selection starts with no objects', () => {
  assert.equal(selectionSize(emptySelection()), 0);
});

test('rectangle selection includes metabolite centres and reaction centres', async () => {
  const { map } = await setup();
  const rxn = map.reactions.get('eco@TALA');
  const node = [...map.nodes.values()].find((n) => n.kind === 'metabolite');
  assert.ok(rxn && node);

  const rect = {
    x1: Math.min(rxn.x, node.x) - 5,
    y1: Math.min(rxn.y, node.y) - 5,
    x2: Math.max(rxn.x, node.x) + 5,
    y2: Math.max(rxn.y, node.y) + 5,
  };
  const selected = selectableInRect(map, rect);
  assert.ok(selected.reactions.has('eco@TALA'));
  assert.ok(selected.nodes.has(node.id));
});

test('hidden metabolites are excluded from rectangle selection', async () => {
  const { map } = await setup();
  const node = [...map.nodes.values()].find((n) => n.kind === 'metabolite');
  map.setNodeHidden(node.id, true);
  const selected = selectableInRect(map, {
    x1: node.x - 2, y1: node.y - 2, x2: node.x + 2, y2: node.y + 2,
  });
  assert.equal(selected.nodes.has(node.id), false);
});

test('group translation moves a selected metabolite and reaction together', async () => {
  const { map } = await setup();
  const rid = 'eco@TALA';
  const rxn = map.reactions.get(rid);
  const node = [...map.nodes.values()].find((n) =>
    n.kind === 'metabolite' && map.segmentsAt(n.id).some((s) => s.reactionId === rid)
  );
  assert.ok(rxn && node);

  const beforeRxn = { x: rxn.x, y: rxn.y };
  const beforeNode = { x: node.x, y: node.y };
  const selection = { nodes: new Set([node.id]), reactions: new Set([rid]) };

  const moved = translateSelection(map, selection, 12, 8);
  assert.ok(moved.nodes.includes(node.id));
  assert.ok(moved.reactions.includes(rid));
  assert.ok(Math.abs((node.x - beforeNode.x) - 12) < 1e-6);
  assert.ok(Math.abs((node.y - beforeNode.y) - 8) < 1e-6);
  assert.ok(Math.abs((rxn.x - beforeRxn.x) - 12) < 1e-6);
  assert.ok(Math.abs((rxn.y - beforeRxn.y) - 8) < 1e-6);
});


test('Delete-style group removal removes selected map instances but not the biological model', async () => {
  const { community, map } = await setup();
  const rid = 'eco@TALA';
  const node = [...map.nodes.values()].find((n) =>
    n.kind === 'metabolite' && map.segmentsAt(n.id).some((s) => s.reactionId === rid)
  );
  assert.ok(node);

  const modelReactionCount = community.merged.reactions.size;
  const modelMetaboliteCount = community.merged.metabolites.size;
  const selection = { nodes: new Set([node.id]), reactions: new Set([rid]) };

  // Mirror Store.removeMultiSelection ordering: reactions first, then selected
  // node instances that still exist and are removable.
  for (const reactionId of selection.reactions) map.removeReaction(reactionId);
  for (const nodeId of selection.nodes) {
    if (map.nodes.has(nodeId) && map.canRemoveNode(nodeId)) map.removeNode(nodeId);
  }

  assert.equal(map.reactions.has(rid), false);
  assert.equal(community.merged.reactions.has(rid), true);
  assert.equal(community.merged.reactions.size, modelReactionCount);
  assert.equal(community.merged.metabolites.size, modelMetaboliteCount);
});

test('when both ends of a TALA branch move, both cubic handles translate', async () => {
  const { map } = await setup();
  const rid = 'eco@TALA';
  const controls = map.reactionControlPoints(rid);
  const b1 = controls.find((p) => p.handle === 'b1');
  const b2 = controls.find((p) => p.nodeId === b1.nodeId && p.handle === 'b2');

  translateSelection(map, {
    nodes: new Set([b1.nodeId]),
    reactions: new Set([rid]),
  }, 10, 6);

  const after = map.reactionControlPoints(rid);
  const a1 = after.find((p) => p.nodeId === b1.nodeId && p.handle === 'b1');
  const a2 = after.find((p) => p.nodeId === b1.nodeId && p.handle === 'b2');

  assert.ok(Math.abs(a1.x - (b1.x + 10)) < 1e-6);
  assert.ok(Math.abs(a1.y - (b1.y + 6)) < 1e-6);
  assert.ok(Math.abs(a2.x - (b2.x + 10)) < 1e-6);
  assert.ok(Math.abs(a2.y - (b2.y + 6)) < 1e-6);
});


function simpleArrangeMap() {
  const nodes = new Map([
    ['n1', { id: 'n1', kind: 'metabolite', x: 10, y: 20, r: 5 }],
    ['n2', { id: 'n2', kind: 'metabolite', x: 40, y: 50, r: 10 }],
    ['n3', { id: 'n3', kind: 'metabolite', x: 100, y: 80, r: 5 }],
  ]);
  const reactions = new Map([
    ['r1', { id: 'r1', x: 70, y: 35, onMembrane: false }],
  ]);
  return {
    nodes,
    reactions,
    canMoveNode(id) { return nodes.has(id); },
    canMoveReaction(id) { return reactions.has(id); },
    moveNode(id, x, y) {
      const node = nodes.get(id);
      if (!node) return null;
      node.x = x; node.y = y;
      return node;
    },
    moveReaction(id, x, y) {
      const reaction = reactions.get(id);
      if (!reaction) return null;
      reaction.x = x; reaction.y = y;
      return reaction;
    },
  };
}

test('horizontal alignment puts selected centres on the same y coordinate', () => {
  const map = simpleArrangeMap();
  const selection = { nodes: new Set(['n1', 'n2', 'n3']), reactions: new Set() };

  const result = alignSelection(map, selection, 'horizontal');
  assert.equal(result.moved.length, 3);
  assert.equal(map.nodes.get('n1').y, 50);
  assert.equal(map.nodes.get('n2').y, 50);
  assert.equal(map.nodes.get('n3').y, 50);
});

test('vertical alignment works across metabolite and reaction objects', () => {
  const map = simpleArrangeMap();
  const selection = { nodes: new Set(['n1', 'n2']), reactions: new Set(['r1']) };

  alignSelection(map, selection, 'vertical');
  const expected = (10 + 40 + 70) / 3;
  assert.equal(map.nodes.get('n1').x, expected);
  assert.equal(map.nodes.get('n2').x, expected);
  assert.equal(map.reactions.get('r1').x, expected);
});

test('left alignment uses visual edges rather than only object centres', () => {
  const map = simpleArrangeMap();
  const selection = { nodes: new Set(['n1', 'n2']), reactions: new Set(['r1']) };

  alignSelection(map, selection, 'left');
  const leftEdges = [
    map.nodes.get('n1').x - map.nodes.get('n1').r,
    map.nodes.get('n2').x - map.nodes.get('n2').r,
    map.reactions.get('r1').x - 9,
  ];
  assert.ok(leftEdges.every((x) => Math.abs(x - leftEdges[0]) < 1e-9));
});

test('horizontal distribution keeps endpoints and spaces intermediate centres equally', () => {
  const map = simpleArrangeMap();
  const selection = { nodes: new Set(['n1', 'n2', 'n3']), reactions: new Set(['r1']) };

  distributeSelection(map, selection, 'horizontal');
  const xs = [
    map.nodes.get('n1').x,
    map.nodes.get('n2').x,
    map.reactions.get('r1').x,
    map.nodes.get('n3').x,
  ].sort((a, b) => a - b);

  const gaps = [xs[1] - xs[0], xs[2] - xs[1], xs[3] - xs[2]];
  assert.ok(gaps.every((gap) => Math.abs(gap - gaps[0]) < 1e-9));
  assert.equal(xs[0], 10);
  assert.equal(xs[3], 100);
});

test('distribution requires at least three selected objects', () => {
  const map = simpleArrangeMap();
  const before = { ...map.nodes.get('n1') };
  const result = distributeSelection(
    map,
    { nodes: new Set(['n1', 'n2']), reactions: new Set() },
    'vertical',
  );
  assert.equal(result.moved.length, 0);
  assert.deepEqual(map.nodes.get('n1'), before);
});

// Free text annotations

test('a rectangle catches the annotations under it, by their middle', async () => {
  const { map } = await setup();
  const inside = map.addText('G6P', 400, 400, { fontSize: 16 });
  const outside = map.addText('PEP', 4000, 4000, { fontSize: 16 });
  assert.ok(inside && outside);

  const centre = textCentre(inside);
  const found = selectableInRect(map, {
    x1: centre.x - 4, y1: centre.y - 4, x2: centre.x + 4, y2: centre.y + 4,
  });

  assert.ok(found.texts.has(inside.id), 'the annotation under the rectangle is caught');
  assert.ok(!found.texts.has(outside.id), 'one far away is not');
});

test('deleting a selection takes its metabolite names with it', async () => {
  /*
   * The case this covers: a whole pathway is selected and deleted, and its
   * metabolite names must not stay behind, labelling reactions that are no
   * longer there. In the core-metabolism overview those names are map
   * annotations, not node labels, so a rectangle selection has to take
   * annotations too.
   */
  const { map } = await setup();
  const rxn = map.reactions.get('eco@TALA');
  const label = map.addText('G6P', rxn.x, rxn.y, { fontSize: 16 });

  const centre = textCentre(label);
  const rect = {
    x1: Math.min(rxn.x, centre.x) - 40, y1: Math.min(rxn.y, centre.y) - 40,
    x2: Math.max(rxn.x, centre.x) + 40, y2: Math.max(rxn.y, centre.y) + 40,
  };
  const selection = selectableInRect(map, rect);

  assert.ok(selection.reactions.has('eco@TALA'));
  assert.ok(selection.texts.has(label.id));
  assert.equal(selectionSize(selection), selection.nodes.size + selection.reactions.size + 1);
});

test('moving a selection carries its annotations along', async () => {
  const { map } = await setup();
  const label = map.addText('PEP', 300, 300, { fontSize: 20 });
  const selection = emptySelection();
  selection.texts.add(label.id);

  const moved = translateSelection(map, selection, 25, -40);

  assert.deepEqual(moved.texts, [label.id]);
  assert.equal(map.texts.get(label.id).x, 325);
  assert.equal(map.texts.get(label.id).y, 260);
});

test('aligning mixes annotations and nodes by their middles, not their anchors', async () => {
  // A node sits at its centre and a text hangs off its baseline corner, so
  // aligning the two has to compare like with like or the text lands low.
  const { map } = await setup();
  const node = [...map.nodes.values()].find((n) => n.kind === 'metabolite');
  const label = map.addText('ALIGN', 900, 900, { fontSize: 20 });

  const selection = emptySelection();
  selection.nodes.add(node.id);
  selection.texts.add(label.id);
  alignSelection(map, selection, 'horizontal');

  const after = map.texts.get(label.id);
  const centreY = after.y - 20 / 2;
  assert.ok(Math.abs(centreY - node.y) < 1e-6,
    `text middle ${centreY} should meet the node at ${node.y}`);
});
