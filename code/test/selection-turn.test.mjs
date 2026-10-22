import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { Store } from '../src/ui/store.js';
import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import { EditableMap } from '../src/map/editable-map.js';
import { parseMapDocument } from '../src/map/map-document.js';
import {
  compartmentKey, emptySelection, pointingAngle, rotateSelection, selectionMainAxis,
  spaceSelectionEvenly, spacingItems, TURN_REFUSAL,
} from '../src/map/multi-selection.js';
import { DEFAULT_METABOLITE_LABEL_ANGLE, metaboliteLabelAngle } from '../src/map/metabolite-label.js';
import { OPERATION_DOMAIN, operationDomain } from '../src/ui/operation-boundary.js';
import { renderInspector, renderWorkspace, showsSelectionInspector } from '../src/ui/panels.js';

const near = (a, b, eps = 1e-6) => Math.abs(a - b) <= eps;
const CHAIN = ['eco@PGI', 'eco@PFK', 'eco@FBA'];

async function ecoliModel() {
  return new Model(JSON.parse(await readFile(new URL('../models/ecoli_core_mc.json', import.meta.url), 'utf8')));
}

async function ecoliStore() {
  const store = new Store('.');
  store.models.set('ecoli_core_mc', await ecoliModel());
  store.scenarios = [{
    id: 'ecoli', name: 'E. coli', objective: 'equal-growth',
    species: [{ id: 'eco', model: 'ecoli_core_mc', abundance: 1 }],
    medium: { glc__D: 10, o2: 20 },
  }];
  await store.useScenario('ecoli');
  return store;
}

/** A chain placed reaction by reaction on an empty cell: curved reactions with their own cofactors. */
async function placedChain() {
  const com = new CommunityModel({ species: [{ id: 'eco', model: await ecoliModel(), abundance: 1 }], medium: {} });
  const map = EditableMap.emptyFromCommunity(com);
  for (const rid of CHAIN) assert.ok(map.addReaction(com, rid), `${rid} is placed`);
  const selection = emptySelection();
  for (const rid of CHAIN) selection.reactions.add(rid);
  for (const seg of map.segments) if (selection.reactions.has(seg.reactionId) && seg.nodeId) selection.nodes.add(seg.nodeId);
  return { map, selection };
}

/** Selection of some reactions and every metabolite drawn to them, cofactors included. */
function chainSelection(map, reactions) {
  const selection = emptySelection();
  for (const rid of reactions) selection.reactions.add(rid);
  for (const seg of map.segments) if (selection.reactions.has(seg.reactionId) && seg.nodeId) selection.nodes.add(seg.nodeId);
  return selection;
}

/**
 * Every point a selection's drawing is made of: its metabolites (cofactors of
 * its reactions included), markers, axis ends, and the ends and handles of its
 * reactions' curves.
 */
function drawingPoints(map, selection) {
  const out = [];
  const nodes = new Set(selection.nodes);
  for (const node of map.nodes.values()) {
    if (nodes.has(node.id) || selection.reactions.has(node.reactionId)) out.push([`node ${node.id}`, node]);
  }
  for (const rid of selection.reactions) {
    const rxn = map.reactions.get(rid);
    out.push([`marker ${rid}`, rxn], [`a1 ${rid}`, rxn.axis.a1], [`a2 ${rid}`, rxn.axis.a2]);
  }
  map.segments.forEach((seg, i) => {
    if (!selection.reactions.has(seg.reactionId)) return;
    for (const key of ['from', 'to', 'b1', 'b2']) if (seg[key]) out.push([`seg ${i} ${key}`, seg[key]]);
  });
  return out.map(([name, p]) => [name, p.x, p.y]);
}

function assertRigid(before, after, label) {
  assert.equal(before.length, after.length);
  assert.ok(before.length > 20, `${label}: enough points to mean something`);
  let worst = 0;
  for (let i = 0; i < before.length; i += 1) {
    for (let j = i + 1; j < before.length; j += 1) {
      const d0 = Math.hypot(before[i][1] - before[j][1], before[i][2] - before[j][2]);
      const d1 = Math.hypot(after[i][1] - after[j][1], after[i][2] - after[j][2]);
      worst = Math.max(worst, Math.abs(d0 - d1));
    }
  }
  assert.ok(worst < 1e-6, `${label}: every distance kept (worst change ${worst})`);
}

// Even spacing

/** Metabolites on their own on an open map, so nothing holds them back. */
function looseNodes(points) {
  const nodes = points.map(([x, y], i) => ({ id: `n${i}`, kind: 'metabolite', primary: true, x, y, r: 13 }));
  const map = new EditableMap({
    canvas: { width: 2000, height: 2000 },
    nodes: new Map(nodes.map((node) => [node.id, node])),
    reactions: new Map(),
    segments: [],
    cells: [],
  });
  const selection = emptySelection();
  for (const node of nodes) selection.nodes.add(node.id);
  return { map, selection };
}

test('even spacing works along a diagonal, keeping the ends and how far each stands off the line', () => {
  // A chain running down to the right, unevenly spaced, one of them a little off it.
  const { map, selection } = looseNodes([[100, 100], [130, 130], [300, 300], [356, 344], [500, 500]]);
  const axis = selectionMainAxis(map, selection);
  assert.ok(near(Math.abs(axis.x), Math.SQRT1_2, 0.01) && near(axis.x, axis.y, 0.01), 'the line is the diagonal');
  // Off the selection's own line, which is all but the diagonal.
  const across = (p) => p.y * axis.x - p.x * axis.y;
  const offBefore = ['n1', 'n2', 'n3'].map((id) => across(map.nodes.get(id)));

  const result = spaceSelectionEvenly(map, selection);
  assert.equal(result.moved.length, 3);
  const along = (id) => {
    const p = map.nodes.get(id);
    return p.x * result.axis.x + p.y * result.axis.y;
  };
  const steps = ['n0', 'n1', 'n2', 'n3', 'n4'].map(along);
  for (let i = 1; i < steps.length; i += 1) assert.ok(near(steps[i] - steps[i - 1], result.step), `step ${i}`);
  assert.deepEqual(['n0', 'n4'].map((id) => [map.nodes.get(id).x, map.nodes.get(id).y]), [[100, 100], [500, 500]],
    'the two ends stay');
  ['n1', 'n2', 'n3'].forEach((id, i) => assert.ok(near(across(map.nodes.get(id)), offBefore[i]),
    `${id} stays as far off the line`));
});

test('even spacing needs three items, and leaves two alone', () => {
  const { map, selection } = looseNodes([[0, 0], [10, 90]]);
  const before = map.toJSON();
  assert.deepEqual(spaceSelectionEvenly(map, selection), { moved: [], count: 2 });
  assert.deepEqual(map.toJSON(), before);
});

test('even spacing counts metabolites and markers, not cofactors or straight reactions on a selected end', async () => {
  const store = await ecoliStore();
  store.multiSelection = chainSelection(store.map, ['eco@PGI', 'eco@PFK']);
  assert.ok(store.arrangeMultiSelection('align-vertical').changed);
  const items = spacingItems(store.map, store.multiSelection);
  assert.ok(items.every((item) => item.kind === 'node'), 'the two straight reactions follow their ends');
  assert.ok(items.every((item) => item.object.kind !== 'currency' && item.object.primary !== false));
  assert.deepEqual(new Set(items.map((item) => item.id)), new Set(['met:eco@g6p_c', 'met:eco@f6p_c', 'met:eco@fdp_c']));

  // Pull the middle one off its place, then space them again: one undo step.
  const f6p = store.map.nodes.get('met:eco@f6p_c');
  store.map.moveNode(f6p.id, f6p.x, f6p.y - 25);
  const depth = store.mapHistory.undoStack.length;
  const result = store.spaceMultiSelectionEvenly();
  assert.ok(result.changed);
  assert.equal(store.mapHistory.undoStack.length, depth + 1);
  const [g6p, mid, fdp] = ['met:eco@g6p_c', 'met:eco@f6p_c', 'met:eco@fdp_c'].map((id) => store.map.nodes.get(id));
  assert.ok(near(mid.y - g6p.y, fdp.y - mid.y, 1e-6), 'equal steps');
  const pfk = store.map.reactions.get('eco@PFK');
  assert.ok(near(pfk.x, mid.x, 1e-6) && pfk.y > Math.min(mid.y, fdp.y) && pfk.y < Math.max(mid.y, fdp.y),
    'the straight reaction stayed on its two metabolites');
  assert.equal(operationDomain('spaceMultiSelectionEvenly'), OPERATION_DOMAIN.MAP);
});

// Turning

test('turning a selection is rigid: every distance in its drawing is kept, cofactors and handles included', async () => {
  const { map, selection } = await placedChain();
  // Labels and a moved name, to see them turn too.
  const node = map.nodes.get([...selection.nodes][0]);
  const cofactor = [...map.nodes.values()].find((n) => n.reactionId === 'eco@PFK');
  assert.ok(cofactor, 'PFK has cofactors of its own');
  map.setReactionLabelOffset('eco@PFK', { x: 20, y: 0 });
  const before = drawingPoints(map, selection);

  const result = rotateSelection(map, selection, Math.PI / 4);
  assert.equal(result.turned, true);
  assertRigid(before, drawingPoints(map, selection), 'curved chain');
  assert.ok(near(metaboliteLabelAngle(node), DEFAULT_METABOLITE_LABEL_ANGLE + Math.PI / 4), 'a label turns round its metabolite');
  assert.ok(near(metaboliteLabelAngle(cofactor), DEFAULT_METABOLITE_LABEL_ANGLE + Math.PI / 4), 'and a cofactor\'s with it');
  const offset = map.reactions.get('eco@PFK').labelOffset;
  assert.ok(near(offset.x, 20 * Math.SQRT1_2) && near(offset.y, 20 * Math.SQRT1_2), 'the moved name turns with its reaction');

  // A turn and its opposite bring everything back.
  rotateSelection(map, selection, -Math.PI / 4, result.pivot);
  drawingPoints(map, selection).forEach(([name, x, y], i) => {
    assert.ok(near(x, before[i][1], 1e-6) && near(y, before[i][2], 1e-6), `${name} is back`);
  });
});

test('a straight pathway turns rigidly too, kept straight on its metabolites', async () => {
  const store = await ecoliStore();
  store.multiSelection = chainSelection(store.map, ['eco@PGI', 'eco@PFK']);
  store.arrangeMultiSelection('align-vertical');
  const selection = chainSelection(store.map, ['eco@PGI', 'eco@PFK']);
  store.multiSelection = selection;
  const before = drawingPoints(store.map, selection);
  const depth = store.mapHistory.undoStack.length;

  const result = store.rotateMultiSelection(Math.PI / 4);
  assert.ok(result.turned && result.changed);
  assert.equal(store.mapHistory.undoStack.length, depth + 1, 'one undo step');
  assertRigid(before, drawingPoints(store.map, selection), 'straight chain');
  for (const rid of ['eco@PGI', 'eco@PFK']) assert.ok(store.map.reactions.get(rid).linear, `${rid} is still straight`);
  store.undoMap();
  assert.ok(near(store.map.reactions.get('eco@PGI').x, before.find(([name]) => name === 'marker eco@PGI')[1]));
  assert.equal(operationDomain('rotateMultiSelection'), OPERATION_DOMAIN.MAP);
});

test('a pathway picked by its reactions turns and spaces as one straight line, the metabolites at its ends included', async () => {
  // Lower glycolysis of E. coli on the curated anaerobic consortium map, a
  // straight vertical chain g3p - GAPD - 13dpg - PGK - 3pg - PGM - 2pg.
  const curated = parseMapDocument(await readFile(new URL('../models/maps/consorcio-anaerobio.json', import.meta.url), 'utf8'));
  const map = EditableMap.fromJSON(curated.map);
  const chain = ['eco@GAPD', 'eco@PGK', 'eco@PGM'];
  // As a rectangle round the reactions catches it: the inner metabolites, not g3p and 2pg at the ends.
  const picked = () => {
    const selection = emptySelection();
    for (const rid of chain) selection.reactions.add(rid);
    for (const id of ['met:eco@13dpg_c', 'met:eco@3pg_c']) selection.nodes.add(id);
    return selection;
  };
  const line = ['met:eco@g3p_c', 'met:eco@13dpg_c', 'met:eco@3pg_c', 'met:eco@2pg_c'];
  const at = (id) => map.nodes.get(id);
  const offLine = () => {
    const [a, b] = [at(line[0]), at(line.at(-1))];
    const length = Math.hypot(b.x - a.x, b.y - a.y);
    return Math.max(...line.map((id) => Math.abs((at(id).x - a.x) * (b.y - a.y) - (at(id).y - a.y) * (b.x - a.x)) / length));
  };
  assert.ok(chain.every((rid) => map.reactions.get(rid).linear) && offLine() < 1e-9, 'a straight chain to begin with');
  assert.deepEqual(spacingItems(map, picked()).map((item) => item.id).sort(), [...line].sort(),
    'the end metabolites are spaced too, and the straight reactions follow them');

  assert.equal(rotateSelection(map, picked(), Math.PI / 4).turned, true);
  assert.ok(offLine() < 1e-6, `still one straight line (${offLine()} off it)`);
  const [first, last] = [at(line[0]), at(line.at(-1))];
  assert.ok(near(last.x - first.x, -(last.y - first.y), 1e-6) && last.y > first.y, 'turned whole, now down to the left');
  for (const rid of chain) assert.ok(map.reactions.get(rid).linear, `${rid} is still straight`);

  assert.equal(map.pointSelection(picked(), 'E').turned, true);
  assert.ok(offLine() < 1e-6 && near(at(line[0]).y, at(line.at(-1)).y, 1e-6), 'pointed east as a line');
  map.moveNode('met:eco@13dpg_c', at('met:eco@13dpg_c').x + 30, at('met:eco@13dpg_c').y);
  assert.equal(spaceSelectionEvenly(map, picked()).moved.length, 2);
  const xs = line.map((id) => at(id).x);
  assert.ok(near(xs[1] - xs[0], xs[2] - xs[1], 1e-6) && near(xs[2] - xs[1], xs[3] - xs[2], 1e-6), 'equal steps from end to end');
});

test('the compass turns the selection to point the way its pathway runs', async () => {
  const store = await ecoliStore();
  store.multiSelection = chainSelection(store.map, ['eco@PGI', 'eco@PFK']);
  store.arrangeMultiSelection('align-vertical');
  store.multiSelection = chainSelection(store.map, ['eco@PGI', 'eco@PFK']);
  const g6p = () => store.map.nodes.get('met:eco@g6p_c');
  const fdp = () => store.map.nodes.get('met:eco@fdp_c');
  const way = () => {
    const dx = fdp().x - g6p().x;
    const dy = fdp().y - g6p().y;
    const length = Math.hypot(dx, dy);
    return { x: dx / length, y: dy / length };
  };

  for (const [direction, x, y] of [['E', 1, 0], ['SE', Math.SQRT1_2, Math.SQRT1_2], ['N', 0, -1], ['SW', -Math.SQRT1_2, Math.SQRT1_2], ['W', -1, 0]]) {
    const result = store.pointMultiSelection(direction);
    assert.equal(result.turned, true, direction);
    const w = way();
    assert.ok(near(w.x, x, 1e-6) && near(w.y, y, 1e-6), `${direction}: glucose 6-phosphate to fructose bisphosphate runs ${direction} (${w.x}, ${w.y})`);
  }
  // Already pointing that way: nothing to do, nothing recorded.
  const depth = store.mapHistory.undoStack.length;
  assert.equal(store.pointMultiSelection('W').changed, false);
  assert.equal(store.mapHistory.undoStack.length, depth);
  assert.equal(operationDomain('pointMultiSelection'), OPERATION_DOMAIN.MAP);
});

test('with no reactions to say which way it runs, the compass makes the smaller turn', () => {
  const { map, selection } = looseNodes([[100, 100], [200, 110], [300, 120]]);
  // Nearly level: E and W are each a small turn one way round or the other.
  assert.ok(Math.abs(pointingAngle(map, selection, 'E')) < 0.2);
  assert.ok(Math.abs(pointingAngle(map, selection, 'W')) < 0.2);
  assert.ok(near(Math.abs(pointingAngle(map, selection, 'S')), Math.PI / 2, 0.2));
  assert.equal(pointingAngle(looseNodes([[0, 0]]).map, looseNodes([[0, 0]]).selection, 'E'), null, 'one item gives no line');
});

test('a turn is refused, leaving the map as it was, for a transporter, a compartment or a wall', async () => {
  const store = await ecoliStore();
  const map = store.map;
  const transporter = [...map.reactions.values()].find((r) => r.onMembrane);
  const before = JSON.stringify(map.toJSON());

  const withTransporter = chainSelection(map, ['eco@PGI', transporter.id]);
  assert.deepEqual(rotateSelection(map, withTransporter, 1), { turned: false, reason: TURN_REFUSAL.transporter });

  const withCompartment = chainSelection(map, ['eco@PGI', 'eco@PFK']);
  withCompartment.compartments.add(compartmentKey(map.cells[0].id, 'c'));
  assert.deepEqual(rotateSelection(map, withCompartment, 1), { turned: false, reason: TURN_REFUSAL.compartment });

  // Swung half round a point far outside the cell, the pathway would leave its cytosol.
  const chain = chainSelection(map, ['eco@PGI', 'eco@PFK']);
  const far = { x: map.cells[0].rect.x - 400, y: map.cells[0].rect.y - 400 };
  assert.deepEqual(rotateSelection(map, chain, Math.PI, far), { turned: false, reason: TURN_REFUSAL.room });
  assert.equal(JSON.stringify(map.toJSON()), before, 'nothing moved');

  // Through the store: a short warning, and no undo step.
  const depth = store.mapHistory.undoStack.length;
  store.multiSelection = withTransporter;
  assert.equal(store.rotateMultiSelection(Math.PI / 4).turned, false);
  assert.equal(store.status.state, 'warn');
  assert.equal(store.status.message, 'Transporters stay on their membranes; leave them out to turn the selection.');
  store.multiSelection = withCompartment;
  assert.equal(store.pointMultiSelection('NE').turned, false);
  assert.equal(store.status.message, 'Compartments cannot be turned; leave them out.');
  assert.equal(store.mapHistory.undoStack.length, depth);
  assert.equal(JSON.stringify(store.map.toJSON()), before);
});

// Inspector

test('the inspector turns to a selection of two or more and offers its tools there', async () => {
  const store = await ecoliStore();
  store.select(null);
  assert.equal(showsSelectionInspector(store), false, 'nothing selected');
  store.multiSelection = chainSelection(store.map, ['eco@PGI', 'eco@PFK']);
  const nodes = store.multiSelection.nodes.size;
  assert.equal(showsSelectionInspector(store), true);
  const html = renderInspector(store);
  assert.match(html, /<h2>Selection<\/h2>/);
  assert.match(html, new RegExp(`2 reactions · ${nodes} metabolites`));
  for (const action of ['align-horizontal', 'align-vertical', 'align-left', 'distribute-vertical',
    'selection-space-evenly', 'selection-rotate', 'clear-multi-selection']) {
    assert.match(html, new RegExp(`data-action="${action}"`), `${action} is offered`);
  }
  for (const direction of ['N', 'NE', 'E', 'SE', 'S', 'SW', 'W', 'NW']) {
    assert.match(html, new RegExp(`data-action="selection-point" data-direction="${direction}"`));
  }
  assert.match(html, /⟲ 45°/);
  assert.match(html, /⟳ 45°/);

  // A reaction picked on its own keeps its inspector: it reads line ends from the selection.
  store.select('eco@PGI');
  assert.equal(showsSelectionInspector(store), false);
  assert.match(renderInspector(store), /data-action="reaction-align"/);

  // Map options keeps its selection card, without the arrange block.
  store.select(null);
  const workspace = renderWorkspace(store, 'map');
  assert.match(workspace, /Selection &amp; geometry|Selection & geometry/);
  assert.doesNotMatch(workspace, /data-action="align-|data-action="distribute-/);
  assert.match(workspace, /data-action="toggle-multi-select"/);
});
