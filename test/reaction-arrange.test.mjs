import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import { EditableMap } from '../src/map/editable-map.js';
import { MapHistory } from '../src/map/history.js';
import { Store } from '../src/ui/store.js';
import { renderMap } from '../src/render/mapview.js';
import { renderInspector, renderViewControls } from '../src/ui/panels.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const RID = 'eco@PFK';

async function community() {
  const raw = JSON.parse(await readFile(join(root, 'models', 'ecoli_core_mc.json'), 'utf8'));
  return new CommunityModel({
    species: [{ id: 'eco', model: new Model(raw), abundance: 1 }],
    medium: {},
  });
}

async function pfkMap(extra = []) {
  const com = await community();
  const map = EditableMap.emptyFromCommunity(com);
  for (const rid of [RID, ...extra]) assert.ok(map.addReaction(com, rid), `${rid} is placed`);
  return { com, map };
}

const branches = (map, rid = RID) => map.segments.filter((seg) => seg.reactionId === rid && seg.nodeId);
const metaboliteEnd = (seg) => (seg.side === 'substrate' ? seg.from : seg.to);
const reactionEnd = (seg) => (seg.side === 'substrate' ? seg.to : seg.from);
const near = (a, b, eps = 1e-6) => Math.hypot(a.x - b.x, a.y - b.y) < eps;
const nodeOf = (map, base) => [...map.nodes.values()].find((n) => n.base === base);

/** A reaction is still one drawing: every branch ends on its node and on its backbone. */
function assertIntact(map, rid = RID) {
  const rxn = map.reactions.get(rid);
  for (const seg of branches(map, rid)) {
    const node = map.nodes.get(seg.nodeId);
    assert.ok(near(metaboliteEnd(seg), node), `${node.base}: the branch ends on its metabolite`);
    assert.ok(Math.hypot(reactionEnd(seg).x - rxn.x, reactionEnd(seg).y - rxn.y) < 30,
      `${node.base}: the branch leaves from the backbone`);
  }
  const axis = map.segments.find((seg) => seg.reactionId === rid && seg.kind === 'axis');
  assert.equal(axis.from, rxn.axis.a1);
  assert.equal(axis.to, rxn.axis.a2);
}

// Undo history

test('edits made in place are recorded, so one undo takes back one edit', async () => {
  /*
   * The history is given the document from before an edit and compares it with
   * the one after, so that document has to be a copy. Were it the live nodes
   * and segments, a drag (which changes them where they stand) would compare
   * equal to itself and be dropped. Only edits that add or remove something
   * would be kept, and the first undo would go straight back past every move
   * to the last of those.
   */
  const { map } = await pfkMap();
  const history = new MapHistory();
  const edit = (label, change) => {
    const before = map.toJSON();
    change();
    return history.record(before, map.toJSON(), label);
  };

  const node = nodeOf(map, 'f6p');
  const start = { x: node.x, y: node.y };
  assert.equal(edit('move metabolite', () => map.moveNode(node.id, node.x + 40, node.y + 25)), true);
  const rxn = map.reactions.get(RID);
  assert.equal(edit('move reaction', () => map.moveReaction(RID, rxn.x + 30, rxn.y)), true);
  const seg = branches(map)[0];
  assert.equal(edit('move control point', () =>
    map.moveBranchControl(RID, seg.nodeId, 'b1', seg.b1.x + 9, seg.b1.y + 9)), true);
  assert.equal(edit('make secondary', () => map.setNodePrimary(node.id, false)), true);
  assert.equal(history.undoStack.length, 4);

  // Undoing the last one leaves the three before it standing.
  const undone = EditableMap.fromJSON(history.undo(map.toJSON()).doc);
  const again = nodeOf(undone, 'f6p');
  assert.notEqual(again.primary, false, 'the role change is undone');
  assert.ok(Math.abs(again.x - (start.x + 40)) < 1e-9, 'and the move before it is not');
});

test('a saved document does not change when the map it came from is edited', async () => {
  const { map } = await pfkMap();
  const doc = map.toJSON();
  const node = nodeOf(map, 'f6p');
  const saved = doc.nodes.find((n) => n.id === node.id);
  const at = { x: saved.x, y: saved.y };
  map.moveNode(node.id, node.x + 80, node.y + 80);
  assert.deepEqual({ x: saved.x, y: saved.y }, at);
});

test('the store records a drag and a turn as separate undo steps', async () => {
  const com = await community();
  const store = new Store('.');
  store.community = com;
  store.map = EditableMap.emptyFromCommunity(com);
  store.addMapReaction(RID);
  const depth = () => store.mapHistory.undoStack.length;
  assert.equal(depth(), 1);

  const node = nodeOf(store.map, 'fdp');
  const drag = store.beginMapEdit('Move metabolite');
  store.moveNode(node.id, node.x + 50, node.y + 10);
  assert.equal(store.commitMapEdit(drag), true, 'a drag is an edit');
  assert.equal(depth(), 2);

  const untouched = store.beginMapEdit('Move metabolite');
  assert.equal(store.commitMapEdit(untouched), false, 'a press that moved nothing is not');

  assert.ok(store.rotateMapReaction(RID, Math.PI / 2));
  assert.equal(depth(), 3);

  const moved = { x: node.x, y: node.y };
  store.undoMap();
  const after = nodeOf(store.map, 'fdp');
  assert.deepEqual({ x: after.x, y: after.y }, moved, 'undo takes back the turn, not the drag');
  assert.equal(depth(), 2);
});

// Rotation

test('turning a reaction turns its backbone and what it owns, and nothing else', async () => {
  const { map } = await pfkMap();
  const rxn = map.reactions.get(RID);
  const dir = { ...rxn.axis.dir };
  const centre = { x: rxn.x, y: rxn.y };
  const own = [...map.nodes.values()].filter((n) => n.reactionId === RID);
  const free = [...map.nodes.values()].filter((n) => !n.reactionId).map((n) => [n.id, n.x, n.y]);
  const radius = own.map((n) => Math.hypot(n.x - rxn.x, n.y - rxn.y));

  assert.ok(map.rotateReaction(RID, Math.PI / 2));

  assert.deepEqual({ x: rxn.x, y: rxn.y }, centre, 'it turns about its own centre');
  assert.ok(Math.abs(rxn.axis.dir.x - -dir.y) < 1e-9 && Math.abs(rxn.axis.dir.y - dir.x) < 1e-9,
    'the backbone has made a quarter turn');
  assert.deepEqual(
    [...map.nodes.values()].filter((n) => !n.reactionId).map((n) => [n.id, n.x, n.y]),
    free,
    'metabolites that stand on their own stay where they are',
  );
  own.forEach((n, i) => assert.ok(Math.abs(Math.hypot(n.x - rxn.x, n.y - rxn.y) - radius[i]) < 1e-6,
    `${n.base} went round with the backbone`));
  assertIntact(map);
});

test('a backbone can be set to an angle, and a transporter keeps the one its membrane gives it', async () => {
  const { map, com } = await pfkMap();
  assert.ok(map.setReactionAngle(RID, 0));
  assert.ok(Math.abs(map.reactionAngle(RID)) < 1e-9);
  assert.ok(Math.abs(map.reactions.get(RID).axis.dir.x - 1) < 1e-9);

  assert.ok(map.addReaction(com, 'eco@PIt2rpp'));
  assert.equal(map.reactions.get('eco@PIt2rpp').onMembrane, true);
  assert.equal(map.canRotateReaction('eco@PIt2rpp'), false);
  assert.equal(map.rotateReaction('eco@PIt2rpp', 1), null);
});

// Straight reactions

test('a linear reaction lies on the line between its substrate and its product', async () => {
  const { map } = await pfkMap();
  assert.equal(map.canLinearizeReaction(RID), true);
  assert.ok(map.setReactionLinear(RID, true));

  const rxn = map.reactions.get(RID);
  const from = map.nodes.get(rxn.linear.from);
  const to = map.nodes.get(rxn.linear.to);
  // It runs between the two metabolites the reaction is about, not its cofactors.
  assert.deepEqual([from.base, to.base], ['f6p', 'fdp']);
  assert.ok(near(rxn, { x: (from.x + to.x) / 2, y: (from.y + to.y) / 2 }), 'the marker is midway');

  const length = Math.hypot(to.x - from.x, to.y - from.y);
  const along = { x: (to.x - from.x) / length, y: (to.y - from.y) / length };
  assert.ok(near(rxn.axis.dir, along), 'and turned along the line');

  // Both branches are that line: every point of them is on it.
  const off = (p) => Math.abs((p.x - from.x) * -along.y + (p.y - from.y) * along.x);
  for (const seg of branches(map).filter((s) => s.nodeId === from.id || s.nodeId === to.id)) {
    for (const p of [seg.from, seg.b1, seg.b2, seg.to]) assert.ok(off(p) < 1e-6);
  }
  assertIntact(map);
});

test('moving a metabolite takes a linear reaction with it', async () => {
  const { map } = await pfkMap();
  map.setReactionLinear(RID, true);
  const rxn = map.reactions.get(RID);
  const from = map.nodes.get(rxn.linear.from);
  const to = map.nodes.get(rxn.linear.to);
  const cofactor = [...map.nodes.values()].find((n) => n.reactionId === RID);
  const reach = Math.hypot(cofactor.x - rxn.x, cofactor.y - rxn.y);

  map.moveNode(from.id, from.x + 130, from.y + 70);

  assert.ok(near(rxn, { x: (from.x + to.x) / 2, y: (from.y + to.y) / 2 }),
    'the marker is midway between where the two are now');
  assert.ok(Math.abs(Math.hypot(cofactor.x - rxn.x, cofactor.y - rxn.y) - reach) < 1e-6,
    'and its cofactors came along at the same distance');
  assertIntact(map);

  // The free reaction it is compared with: the marker stays behind.
  const other = await pfkMap();
  const still = { x: other.map.reactions.get(RID).x, y: other.map.reactions.get(RID).y };
  const node = nodeOf(other.map, 'f6p');
  other.map.moveNode(node.id, node.x + 130, node.y + 70);
  assert.deepEqual({ x: other.map.reactions.get(RID).x, y: other.map.reactions.get(RID).y }, still);
});

test('editing a compartment leaves the bends of straight reactions it did not pull out of line', async () => {
  const { map } = await pfkMap();
  map.setReactionLinear(RID, true);
  const rxn = map.reactions.get(RID);
  const [cell] = map.cells;
  // A bend drawn by hand into one branch of the line.
  const seg = branches(map).find((s) => s.nodeId === rxn.linear.from);
  seg.b1 = { x: seg.b1.x + 40, y: seg.b1.y - 25 };
  const drawn = () => JSON.stringify(branches(map).map((s) => [s.b1, s.b2]));

  // Another compartment resized: nothing of the line moves, and it keeps its bend.
  const before = drawn();
  const p = cell.compartments.find((c) => c.id === 'p').rect;
  assert.ok(map.editGeometryRect(cell.id, 'p', { ...p, w: p.w + 60 }));
  assert.equal(drawn(), before, 'the bend survived an edit somewhere else');

  // Its own compartment moved, the whole line with it: the bend moves along, unchanged.
  const c = { ...cell.compartments.find((k) => k.id === 'c').rect };
  const b1 = { ...seg.b1 };
  assert.ok(map.editGeometryRect(cell.id, 'c', { ...c, x: c.x + 12, y: c.y + 9 }));
  const now = cell.compartments.find((k) => k.id === 'c').rect;
  const d = { x: now.x - c.x, y: now.y - c.y };
  assert.ok(d.x || d.y, 'the compartment moved');
  assert.ok(near(seg.b1, { x: b1.x + d.x, y: b1.y + d.y }), 'the bend came along with it, unchanged');
});

test('a linear reaction slides along its line, cannot be turned, and can be released', async () => {
  const { map } = await pfkMap();
  map.setReactionLinear(RID, true);
  const rxn = map.reactions.get(RID);
  const from = map.nodes.get(rxn.linear.from);
  const to = map.nodes.get(rxn.linear.to);

  // Dragged far off to one side, the marker goes as far as the line allows.
  map.moveReaction(RID, from.x + 900, from.y - 900);
  const t = Math.hypot(rxn.x - from.x, rxn.y - from.y) / Math.hypot(to.x - from.x, to.y - from.y);
  assert.ok(t > 0.1 && t < 0.9, `the marker stays between its ends (t = ${t.toFixed(2)})`);
  const cross = (rxn.x - from.x) * (to.y - from.y) - (rxn.y - from.y) * (to.x - from.x);
  assert.ok(Math.abs(cross) < 1e-6, 'and on the line');

  assert.equal(map.canRotateReaction(RID), false);
  assert.equal(map.rotateReaction(RID, 1), null);

  assert.ok(map.setReactionLinear(RID, false));
  assert.equal(rxn.linear, undefined);
  assert.equal(map.canRotateReaction(RID), true);
});

test('linear survives saving, and a merge hands the line on to the node that took over', async () => {
  const { map } = await pfkMap();
  map.setReactionLinear(RID, true);
  const saved = JSON.parse(JSON.stringify(map.toJSON()));
  const loaded = EditableMap.fromJSON(saved);
  const rxn = loaded.reactions.get(RID);
  assert.deepEqual(rxn.linear, map.reactions.get(RID).linear);

  const end = loaded.nodes.get(rxn.linear.to);
  const copy = loaded.duplicateMetaboliteNode(end.id);
  assert.ok(loaded.mergeMetaboliteNodes(end.id, copy.id), 'the end is merged into its copy');
  assert.equal(rxn.linear.to, copy.id);
  assert.ok(near(rxn, {
    x: (loaded.nodes.get(rxn.linear.from).x + copy.x) / 2,
    y: (loaded.nodes.get(rxn.linear.from).y + copy.y) / 2,
  }), 'and the reaction runs to where the copy is');
});

// Swap and mirror

test('two substrates trade places and the drawing stays as it was', async () => {
  const { map } = await pfkMap();
  const sides = map.reactionSides(RID);
  assert.deepEqual(sides.substrates.map((s) => s.label), ['atp', 'f6p']);
  assert.deepEqual(sides.products.map((s) => s.label), ['adp', 'fdp', 'h']);

  const [a, b] = sides.substrates;
  const placeA = { x: a.node.x, y: a.node.y };
  const placeB = { x: b.node.x, y: b.node.y };
  const curves = branches(map).filter((s) => s.side === 'substrate')
    .map((s) => [metaboliteEnd(s), s.b1, s.b2, reactionEnd(s)].map((p) => `${p.x.toFixed(4)},${p.y.toFixed(4)}`).join(' '));

  assert.ok(map.swapReactionMetabolites(RID, a.nodeId, b.nodeId));

  assert.deepEqual({ x: a.node.x, y: a.node.y }, placeB);
  assert.deepEqual({ x: b.node.x, y: b.node.y }, placeA);
  // The same two curves are still drawn; they have only changed owner.
  const after = branches(map).filter((s) => s.side === 'substrate')
    .map((s) => [metaboliteEnd(s), s.b1, s.b2, reactionEnd(s)].map((p) => `${p.x.toFixed(4)},${p.y.toFixed(4)}`).join(' '));
  assert.deepEqual([...after].sort(), [...curves].sort());
  assert.notDeepEqual(after, curves);
  assertIntact(map);
});

test('a swap is refused across the two sides of a reaction', async () => {
  const { map } = await pfkMap();
  const sides = map.reactionSides(RID);
  assert.equal(map.swapReactionMetabolites(RID, sides.substrates[0].nodeId, sides.products[0].nodeId), null);
  assert.equal(map.swapReactionMetabolites(RID, sides.substrates[0].nodeId, sides.substrates[0].nodeId), null);
});

test('a swap on a linear reaction hands the line to the metabolite now standing on it', async () => {
  const { map } = await pfkMap();
  map.setReactionLinear(RID, true);
  const rxn = map.reactions.get(RID);
  const [atp, f6p] = map.reactionSides(RID).substrates;
  assert.equal(rxn.linear.from, f6p.nodeId);
  assert.ok(map.swapReactionMetabolites(RID, atp.nodeId, f6p.nodeId));
  assert.equal(rxn.linear.from, atp.nodeId);
  const to = map.nodes.get(rxn.linear.to);
  assert.ok(near(rxn, { x: (atp.node.x + to.x) / 2, y: (atp.node.y + to.y) / 2 }));
});

test('another reaction drawn to a swapped metabolite follows it', async () => {
  const { map, com } = await pfkMap();
  // FBA shares fdp with PFK once it is added from that node.
  const fdp = nodeOf(map, 'fdp');
  assert.ok(map.addReaction(com, 'eco@FBA', { anchorNodeId: fdp.id }));
  const other = branches(map, 'eco@FBA').find((s) => s.nodeId === fdp.id);
  assert.ok(other, 'FBA is drawn from the same fdp node');

  const products = map.reactionSides(RID).products;
  const adp = products.find((p) => p.label === 'adp');
  assert.ok(map.swapReactionMetabolites(RID, fdp.id, adp.nodeId));
  assert.ok(near(metaboliteEnd(other), fdp), 'the other reaction still ends on fdp');
});

test('swapping substrates and products turns the reaction end for end', async () => {
  const { map } = await pfkMap();
  const rxn = map.reactions.get(RID);
  const dir = { ...rxn.axis.dir };
  const side = (node) => (node.x - rxn.x) * dir.x + (node.y - rxn.y) * dir.y;
  const across = (node) => (node.x - rxn.x) * -dir.y + (node.y - rxn.y) * dir.x;
  const before = new Map([...map.nodes.values()].map((n) => [n.id, { along: side(n), across: across(n) }]));

  assert.ok(map.flipReaction(RID));

  assert.ok(near(rxn.axis.dir, { x: -dir.x, y: -dir.y }), 'the backbone points the other way');
  for (const node of map.nodes.values()) {
    const was = before.get(node.id);
    assert.ok(Math.abs(side(node) + was.along) < 1e-6, `${node.base} is on the other end`);
    assert.ok(Math.abs(across(node) - was.across) < 1e-6, `${node.base} is on the same side as before`);
  }
  assertIntact(map);
});

test('mirroring moves what hangs off one side to the other', async () => {
  const { map } = await pfkMap();
  const rxn = map.reactions.get(RID);
  const dir = { ...rxn.axis.dir };
  const along = (node) => (node.x - rxn.x) * dir.x + (node.y - rxn.y) * dir.y;
  const across = (node) => (node.x - rxn.x) * -dir.y + (node.y - rxn.y) * dir.x;
  const before = new Map([...map.nodes.values()].map((n) => [n.id, { along: along(n), across: across(n) }]));

  assert.ok(map.mirrorReaction(RID));

  assert.ok(near(rxn.axis.dir, dir), 'the backbone does not move');
  for (const node of map.nodes.values()) {
    const was = before.get(node.id);
    assert.ok(Math.abs(along(node) - was.along) < 1e-6);
    assert.ok(Math.abs(across(node) + was.across) < 1e-6, `${node.base} changed sides`);
  }
  assertIntact(map);
});

test('a transporter can be mirrored but its two sides stay on their sides of the membrane', async () => {
  const { map, com } = await pfkMap();
  const rid = 'eco@PIt2rpp';
  assert.ok(map.addReaction(com, rid));
  const compartments = new Map([...map.nodes.values()].map((n) => [n.id, n.compartment]));
  assert.equal(map.flipReaction(rid), null, 'the sides of a transporter are the sides of a membrane');
  assert.equal(map.canLinearizeReaction(rid), false);
  assert.ok(map.mirrorReaction(rid));
  for (const node of map.nodes.values()) assert.equal(node.compartment, compartments.get(node.id));
  for (const seg of branches(map, rid)) {
    assert.ok(near(metaboliteEnd(seg), map.nodes.get(seg.nodeId)));
  }
});

// Handles

const render = (map, state = {}) => renderMap(map, {
  fluxes: new Map(),
  scale: { color: () => '#94a3b8', width: () => 1, reference: 1 },
  showCurrency: true,
  ...state,
});

test('selecting a reaction does not put control points over its marker', async () => {
  /*
   * Shown on selection, they would catch the release of the very click that
   * selected the reaction, which reads as a click on nothing and deselects it:
   * the inspector would flash open and shut, and a reaction could not be
   * dragged because it was never still selected.
   */
  const { map } = await pfkMap();
  const svg = render(map, { selectedReaction: RID });
  assert.doesNotMatch(svg, /data-control-handle=/);
  assert.doesNotMatch(svg, /data-reaction-handle=/);
  assert.match(svg, /emc-select-ring/, 'the selection is still shown');
});

test('Edit Control Points shows the two curve handles and a lever, and no backbone points', async () => {
  const { map } = await pfkMap();
  const svg = render(map, { reactionEdit: true });
  assert.match(svg, /data-control-handle="b1"/);
  assert.match(svg, /data-control-handle="b2"/);
  assert.doesNotMatch(svg, /data-control-handle="anchor"/);
  assert.doesNotMatch(svg, /emc-control-anchor/);
  assert.match(svg, new RegExp(`data-rotate-handle="${RID}"`));
});

test('a linear reaction offers no lever and no handles on the two branches it keeps straight', async () => {
  const { map } = await pfkMap();
  const all = (render(map, { reactionEdit: true }).match(/class="emc-control-handle emc-control-b1"/g) ?? []).length;
  map.setReactionLinear(RID, true);
  const svg = render(map, { reactionEdit: true });
  assert.doesNotMatch(svg, /data-rotate-handle=/);
  assert.equal((svg.match(/class="emc-control-handle emc-control-b1"/g) ?? []).length, all - 2);
});

// Inspector

async function storeWithPFK() {
  const com = await community();
  const store = new Store('.');
  store.community = com;
  store.species = [{ id: 'eco', file: 'ecoli_core_mc', abundance: 1, enabled: true }];
  store.models.set('ecoli_core_mc', com.species[0].model);
  store.map = EditableMap.emptyFromCommunity(com);
  store.problem = { getBounds: () => ({ lower: 0, upper: 1000 }) };
  store.defaultReactionBounds = () => ({ lower: 0, upper: 1000 });
  store.currentReactionBounds = () => ({ lower: 0, upper: 1000 });
  store.addMapReaction(RID);
  store.select(RID);
  return store;
}

test('the reaction inspector offers linear, swap, mirror and rotate as map options', async () => {
  const store = await storeWithPFK();
  const html = renderInspector(store);
  for (const action of ['reaction-free', 'reaction-linear', 'reaction-swap', 'reaction-flip',
    'reaction-mirror', 'reaction-rotate']) {
    assert.match(html, new RegExp(`data-action="${action}"`), `${action} is offered`);
  }
  // One swap between each pair of neighbours: 1 for two substrates, 2 for three products.
  assert.equal((html.match(/data-action="reaction-swap"/g) ?? []).length, 3);
  assert.match(html, /Shift-click two metabolites to pick them/);
});

test('the inspector options are undoable edits that leave the model alone', async () => {
  const store = await storeWithPFK();
  const equation = store.community.merged.reactions.get(RID).equation();
  const depth = () => store.mapHistory.undoStack.length;
  const start = depth();

  assert.ok(store.setMapReactionLinear(RID, true));
  assert.match(renderInspector(store), /Kept straight between f6p and fdp/);
  const [a, b] = store.map.reactionSides(RID).products;
  assert.ok(store.swapMapReactionMetabolites(RID, a.nodeId, b.nodeId));
  assert.ok(store.mirrorMapReaction(RID));
  assert.ok(store.flipMapReaction(RID));
  assert.equal(depth(), start + 4);
  assert.equal(store.community.merged.reactions.get(RID).equation(), equation);

  store.undoMap(); store.undoMap(); store.undoMap(); store.undoMap();
  assert.equal(store.map.reactions.get(RID).linear, undefined);
});

// Compass alignment

const COMPASS = {
  N: [0, -1], NE: [1, -1], E: [1, 0], SE: [1, 1], S: [0, 1], SW: [-1, 1], W: [-1, 0], NW: [-1, -1],
};

test('a reaction can be laid out in each of the eight directions, substrate to product', async () => {
  for (const [name, [dx, dy]] of Object.entries(COMPASS)) {
    const { map } = await pfkMap();
    const rxn = map.reactions.get(RID);
    const centre = { x: rxn.x, y: rxn.y };
    assert.ok(map.alignReaction(RID, name), `${name} is accepted`);

    const from = map.nodes.get(rxn.linear.from);
    const to = map.nodes.get(rxn.linear.to);
    assert.deepEqual([from.base, to.base], ['f6p', 'fdp']);
    const length = Math.hypot(dx, dy);
    const way = { x: dx / length, y: dy / length };

    // The marker has not moved, and both metabolites are in line with it.
    assert.ok(near(rxn, centre, 1e-6), `${name}: the marker stays where it was`);
    const off = (p) => Math.abs((p.x - centre.x) * -way.y + (p.y - centre.y) * way.x);
    const along = (p) => (p.x - centre.x) * way.x + (p.y - centre.y) * way.y;
    assert.ok(off(from) < 1e-6 && off(to) < 1e-6, `${name}: both are on the line through the marker`);
    assert.ok(along(from) < 0 && along(to) > 0, `${name}: the substrate is behind the marker and the product ahead`);
    assert.ok(near(rxn.axis.dir, way, 1e-9), `${name}: the backbone points that way`);
    assertIntact(map);
  }
  const { map } = await pfkMap();
  assert.equal(map.alignReaction(RID, 'NNE'), null, 'only the eight compass points exist');
});

test('aligning keeps each metabolite as far from the marker as it was', async () => {
  const { map } = await pfkMap();
  const rxn = map.reactions.get(RID);
  const f6p = nodeOf(map, 'f6p');
  const fdp = nodeOf(map, 'fdp');
  map.moveNode(f6p.id, rxn.x - 150, rxn.y - 90);
  const arms = [f6p, fdp].map((n) => Math.hypot(n.x - rxn.x, n.y - rxn.y));

  assert.ok(map.alignReaction(RID, 'E'));
  assert.ok(Math.abs(Math.hypot(f6p.x - rxn.x, f6p.y - rxn.y) - arms[0]) < 1e-6);
  assert.ok(Math.abs(Math.hypot(fdp.x - rxn.x, fdp.y - rxn.y) - arms[1]) < 1e-6);
  assert.ok(f6p.x < rxn.x && fdp.x > rxn.x && Math.abs(f6p.y - rxn.y) < 1e-6 && Math.abs(fdp.y - rxn.y) < 1e-6);
  // And from then on the reaction is kept straight between them.
  map.moveNode(fdp.id, fdp.x + 60, fdp.y + 80);
  const t = rxn.linear.t;
  assert.ok(near(rxn, { x: f6p.x + (fdp.x - f6p.x) * t, y: f6p.y + (fdp.y - f6p.y) * t }, 1e-6));
});

test('the two metabolites a reaction is aligned through can be chosen', async () => {
  const { map } = await pfkMap();
  const rxn = map.reactions.get(RID);
  const sides = map.reactionSides(RID);
  const atp = sides.substrates.find((e) => e.label === 'atp');
  const adp = sides.products.find((e) => e.label === 'adp');

  // Left to itself the line runs through the two the reaction is about.
  const own = map.reactionLineEnds(RID);
  assert.deepEqual([map.nodes.get(own.from).base, map.nodes.get(own.to).base], ['f6p', 'fdp']);

  // Chosen one at a time, as in the inspector: nothing moves until it is used.
  const before = JSON.stringify(map.toJSON().nodes);
  assert.ok(map.setReactionLineEnd(RID, atp.nodeId));
  assert.equal(JSON.stringify(map.toJSON().nodes), before);
  assert.equal(map.reactionLineEnds(RID).from, atp.nodeId);
  assert.equal(map.reactionLineEnds(RID).to, own.to, 'the other end is left alone');

  // Or both at once, as from a selection on the map.
  assert.ok(map.alignReaction(RID, 'S', { from: atp.nodeId, to: adp.nodeId }));
  assert.deepEqual([rxn.linear.from, rxn.linear.to], [atp.nodeId, adp.nodeId]);
  assert.ok(Math.abs(atp.node.x - rxn.x) < 1e-6 && atp.node.y < rxn.y, 'atp is straight above the marker');
  assert.ok(Math.abs(adp.node.x - rxn.x) < 1e-6 && adp.node.y > rxn.y, 'adp is straight below it');

  // A metabolite of the wrong side is not an end of that side.
  assert.equal(map.setReactionLineEnd(RID, 'no-such-node'), null);
});

test('metabolites picked out on the map are the ones Linear and the compass use', async () => {
  const store = await storeWithPFK();
  const sides = store.map.reactionSides(RID);
  const atp = sides.substrates.find((e) => e.label === 'atp');
  const h = sides.products.find((e) => e.label === 'h');
  assert.deepEqual(store.selectedLineEnds(RID), {});

  store.toggleMultiSelection('node', atp.nodeId);
  store.toggleMultiSelection('node', h.nodeId);
  assert.deepEqual(store.selectedLineEnds(RID), { from: atp.nodeId, to: h.nodeId });
  assert.match(renderInspector(store), new RegExp(`reaction-line-end is-active" data-action="reaction-line-end"\\s+data-rxn="${RID}" data-node="${atp.nodeId.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}"`));

  assert.ok(store.alignMapReaction(RID, 'W'));
  const rxn = store.map.reactions.get(RID);
  assert.deepEqual([rxn.linear.from, rxn.linear.to], [atp.nodeId, h.nodeId]);
  assert.ok(atp.node.x > rxn.x && h.node.x < rxn.x, 'substrate to product runs to the left');

  // Two picked on the same side say nothing about that side.
  store.toggleMultiSelection('node', sides.substrates.find((e) => e.label === 'f6p').nodeId);
  assert.deepEqual(store.selectedLineEnds(RID), { to: h.nodeId });
});

// Base shape

test('a newly placed reaction is straight, with its cofactors close beside the stem', async () => {
  const { map } = await pfkMap();
  const rxn = map.reactions.get(RID);
  const dir = rxn.axis.dir;
  assert.deepEqual([dir.x, dir.y], [0, 1], 'drawn down the page');
  const along = (p) => (p.x - rxn.x) * dir.x + (p.y - rxn.y) * dir.y;
  const across = (p) => (p.x - rxn.x) * -dir.y + (p.y - rxn.y) * dir.x;
  // (+ 0 turns the -0 that rounding a tiny negative leaves into a plain 0.)
  const place = (base) => { const n = nodeOf(map, base); return [Math.round(along(n)) + 0, Math.round(across(n)) + 0]; };

  // Substrate, marker and product in one line.
  assert.deepEqual(place('f6p'), [-72, 0]);
  assert.deepEqual(place('fdp'), [72, 0]);
  // One substrate cofactor and two product ones: the first on each side stands
  // on the same hand of the stem, the second across from it. The end that
  // points up the page has them further out than the end that points down,
  // where the name of the metabolite below has to fit above it.
  assert.deepEqual(place('atp'), [-40, 26]);
  assert.deepEqual(place('adp'), [28, 26]);
  assert.deepEqual(place('h'), [28, -26]);
  assertIntact(map);
});

test('a branch to a metabolite beside the stem is a C: along the stem, then square across to it', async () => {
  const { map } = await pfkMap();
  const rxn = map.reactions.get(RID);
  const dir = rxn.axis.dir;
  const along = (p) => (p.x - rxn.x) * dir.x + (p.y - rxn.y) * dir.y;
  const across = (p) => (p.x - rxn.x) * -dir.y + (p.y - rxn.y) * dir.x;

  for (const seg of branches(map)) {
    const node = map.nodes.get(seg.nodeId);
    const end = reactionEnd(seg);
    const out = Math.sign(along(node));
    // At the reaction it runs along the stem, away from the marker.
    assert.ok(Math.abs(across(seg.b2)) < 1e-6, `${node.base}: the reaction-side handle is on the stem`);
    assert.ok((along(seg.b2) - along(end)) * out > 0, `${node.base}: it sets out away from the marker`);
    if (Math.abs(across(node)) < 1e-6) continue;   // the one the stem runs straight to

    // At the metabolite it comes in from the stem's side: the handle is level
    // with the metabolite and between it and the stem, not above or below it.
    assert.ok(Math.abs(along(seg.b1) - along(node)) < 1e-6, `${node.base}: the metabolite-side handle is level with it`);
    assert.ok(Math.abs(across(seg.b1)) < Math.abs(across(node)) && across(seg.b1) * across(node) > 0,
      `${node.base}: and lies between it and the stem`);
    // The stem is followed nearly out to the metabolite's level before the turn.
    assert.ok((along(node) - along(seg.b2)) * out > 0, `${node.base}: the turn comes before its level is reached`);
    assert.ok(Math.abs(along(seg.b2) - along(end)) > 0.6 * Math.abs(along(node) - along(end)));

    // The curve never crosses to the other hand of the stem, and never doubles
    // back along it: no S.
    const at = (t) => {
      const [p0, p1, p2, p3] = seg.side === 'substrate'
        ? [seg.from, seg.b1, seg.b2, seg.to] : [seg.to, seg.b1, seg.b2, seg.from];
      const k = (a, b, c, d) => (1 - t) ** 3 * a + 3 * (1 - t) ** 2 * t * b + 3 * (1 - t) * t ** 2 * c + t ** 3 * d;
      return { x: k(p0.x, p1.x, p2.x, p3.x), y: k(p0.y, p1.y, p2.y, p3.y) };
    };
    let last = at(0);
    for (let i = 1; i <= 20; i += 1) {
      const p = at(i / 20);
      assert.ok(across(p) * across(node) >= -1e-9, `${node.base}: the curve stays on its own hand of the stem`);
      assert.ok(Math.abs(across(p)) <= Math.abs(across(last)) + 1e-9, `${node.base}: it only ever closes in on the stem`);
      assert.ok((along(last) - along(p)) * out >= -1e-9, `${node.base}: and only ever heads for the marker`);
      last = p;
    }
  }
});

test('cofactors stand the same distance out on both ends of a reaction lying on its side', async () => {
  const { map } = await pfkMap();
  assert.ok(map.alignReaction(RID, 'E'));
  const rxn = map.reactions.get(RID);
  const along = (n) => Math.round((n.x - rxn.x) * rxn.axis.dir.x + (n.y - rxn.y) * rxn.axis.dir.y);
  assert.deepEqual([along(nodeOf(map, 'atp')), along(nodeOf(map, 'adp')), along(nodeOf(map, 'h'))], [-34, 34, 34]);
  // Turned upside down, the further end is again the one that points up.
  assert.ok(map.alignReaction(RID, 'N'));
  assert.deepEqual([along(nodeOf(map, 'atp')), along(nodeOf(map, 'adp'))], [-28, 40]);
});

test('the name of a straight reaction lying on its side stands clear of its arm', async () => {
  const { map } = await pfkMap();
  const rxn = map.reactions.get(RID);
  const name = () => {
    const m = render(map, { labels: 'all' })
      .match(/<text class="emc-label emc-label-rxn ([\w-]+)" x="([-\d.]+)" y="([-\d.]+)" text-anchor="(\w+)"[^>]*data-rxn="eco@PFK"/);
    return { placement: m[1], dx: +m[2] - rxn.x, dy: +m[3] - rxn.y, anchor: m[4] };
  };

  // Top to bottom, as placed: beside the marker, where nothing of the reaction is.
  let at = name();
  assert.deepEqual([at.placement, at.anchor, at.dy], ['emc-label-rxn-adjacent', 'start', 0]);
  assert.ok(at.dx > 3);

  // Lying east the product arm is there. Two cofactors are below and one above.
  assert.ok(map.alignReaction(RID, 'E'));
  at = name();
  assert.deepEqual([at.placement, at.anchor, at.dx], ['emc-label-rxn-above', 'middle', 0]);
  assert.ok(at.dy < -9, 'above the bar, which reaches 9 either side of the stem');

  // Turned end for end the two are above, so the name goes below.
  assert.ok(map.alignReaction(RID, 'W'));
  at = name();
  assert.deepEqual([at.placement, at.anchor, at.dx], ['emc-label-rxn-below', 'middle', 0]);
  assert.ok(at.dy > 9);

  // On a diagonal the arm clears the name, and a reaction not kept straight is left as it was.
  assert.ok(map.alignReaction(RID, 'NE'));
  assert.equal(name().placement, 'emc-label-rxn-adjacent');
  assert.ok(map.alignReaction(RID, 'E'));
  map.setReactionLinear(RID, false);
  assert.equal(name().placement, 'emc-label-rxn-adjacent');
});

test('tidy puts a pulled-about reaction back in its base shape without moving its metabolites', async () => {
  const { map } = await pfkMap();
  const rxn = map.reactions.get(RID);
  const shape = () => JSON.stringify(branches(map).map((seg) => [seg.nodeId, seg.from, seg.to, seg.b1, seg.b2]));
  const base = shape();
  const own = [...map.nodes.values()].filter((n) => !n.reactionId).map((n) => [n.id, n.x, n.y]);

  // Cofactors dragged off, a handle pulled, a branch taken off the backbone.
  const atp = nodeOf(map, 'atp');
  map.moveNode(atp.id, atp.x + 70, atp.y - 45);
  const seg = branches(map).find((x) => map.nodes.get(x.nodeId).base === 'fdp');
  map.moveBranchControl(RID, seg.nodeId, 'b2', seg.b2.x + 40, seg.b2.y + 30);
  map.moveBranchControl(RID, seg.nodeId, 'anchor', rxn.x + 25, rxn.y + 25);
  assert.notEqual(shape(), base);

  assert.ok(map.tidyReaction(RID));
  assert.equal(shape(), base);
  assert.deepEqual([...map.nodes.values()].filter((n) => !n.reactionId).map((n) => [n.id, n.x, n.y]), own);
});

test('the inspector offers the compass, the metabolite choice and tidy', async () => {
  const store = await storeWithPFK();
  const html = renderInspector(store);
  for (const direction of Object.keys(COMPASS)) {
    assert.match(html, new RegExp(`data-action="reaction-align" data-rxn="${RID}" data-direction="${direction}"`));
  }
  assert.equal((html.match(/data-action="reaction-line-end"/g) ?? []).length, 5, 'one per drawn metabolite');
  assert.equal((html.match(/reaction-line-end is-active/g) ?? []).length, 2, 'the two the line would run through are marked');
  assert.match(html, /data-action="reaction-tidy"/);

  const depth = store.mapHistory.undoStack.length;
  assert.ok(store.alignMapReaction(RID, 'NE'));
  assert.ok(store.tidyMapReaction(RID) !== undefined);
  assert.equal(store.mapHistory.undoStack.length, depth + 1, 'aligning is one undo step; tidying what is already tidy is none');
  store.undoMap();
  assert.equal(store.map.reactions.get(RID).linear, undefined);
});

// Transporter arrows

async function yeast() {
  const raw = JSON.parse(await readFile(join(root, 'models', 'yeast_core_mc.json'), 'utf8'));
  return new CommunityModel({
    species: [{ id: 'sce', model: new Model(raw), abundance: 1 }],
    medium: {},
  });
}

/** Which way each lane arrow of a transporter points, relative to its membrane. */
function laneArrows(svg, rid) {
  const glyph = new RegExp(`<g class="emc-marker[^"]*emc-transporter[^"]*" data-rxn="${rid}"[^>]*>([\\s\\S]*?)</g>`).exec(svg);
  assert.ok(glyph, `${rid} has a transporter glyph`);
  return [...glyph[1].matchAll(/data-lane-direction="(out|in)"/g)].map((m) => m[1]);
}

test('a symporter shows one ordinary arrow per metabolite, pointing the way it carries them', async () => {
  /*
   * PIt2rpp takes a phosphate and a proton from the periplasm into the cytosol.
   * The glyph is not read against the reaction's axis, which points either
   * way across a membrane and for this one would point both arrows out.
   */
  const { map, com } = await pfkMap();
  const rid = 'eco@PIt2rpp';
  assert.ok(map.addReaction(com, rid));
  assert.equal(map.reactions.get(rid).transportMode, 'symport');
  assert.equal(com.merged.reactions.get(rid).equation(), 'eco@h_p + eco@pi_p <=> eco@h_c + eco@pi_c');
  assert.equal(map.reactions.get(rid).boundaryId, 'eco/c');

  const forward = render(map, { fluxes: new Map([[rid, 3]]) });
  assert.deepEqual(laneArrows(forward, rid), ['in', 'in'], 'into the cytosol at positive flux');
  assert.deepEqual(laneArrows(render(map, { fluxes: new Map([[rid, -3]]) }), rid), ['out', 'out'],
    'and out of it when the flux is reversed');

  // An arrow is a shaft with a head on it, not a bar.
  assert.equal((forward.match(/class="emc-lane-head"/g) ?? []).length, 2);
  assert.equal((forward.match(/class="emc-lane"/g) ?? []).length, 2);
  // And the lanes are not drawn a second time as stubs under the glyph.
  assert.doesNotMatch(forward, /emc-seg-transport-lane/);
  assert.ok(map.segments.some((seg) => seg.kind === 'transport-lane'), 'the lane geometry itself is kept');
});

test('an antiporter shows two arrows pointing opposite ways', async () => {
  const com = await yeast();
  const map = EditableMap.emptyFromCommunity(com);
  const rid = 'sce@ATPtm';
  assert.ok(map.addReaction(com, rid));
  assert.equal(map.reactions.get(rid).transportMode, 'antiport');
  const arrows = laneArrows(render(map), rid);
  assert.deepEqual([...arrows].sort(), ['in', 'out']);
});

test('the arrows stay on their lanes when a transporter is carried round a corner', async () => {
  const { map, com } = await pfkMap();
  const rid = 'eco@PIt2rpp';
  assert.ok(map.addReaction(com, rid));
  const rxn = map.reactions.get(rid);
  const box = map.boundary(rxn.boundaryId).rect;
  // From wherever it was put to the middle of the left-hand side.
  assert.ok(map.moveReaction(rid, box.x - 50, box.y + box.h / 2));
  assert.ok(Math.abs(rxn.normal.x + 1) < 1e-9, 'it is on the left membrane now');
  const svg = render(map, { fluxes: new Map([[rid, 3]]) });
  assert.deepEqual(laneArrows(svg, rid), ['in', 'in']);
  // The glyph is turned with the membrane, so "in" is still into the cytosol.
  assert.match(svg, new RegExp(`data-rxn="${rid}" transform="translate\\([^)]*\\) rotate\\(180\\)"`));
});

// Cofactor labels

test('cofactor labels start enabled and can be turned off from the View menu', async () => {
  const { map } = await pfkMap();
  const store = new Store('.');
  assert.equal(store.view.currencyLabels, true);
  assert.match(renderViewControls(store), /data-action="toggle-currency-labels" aria-pressed="true"/);
  store.setView({ currencyLabels: false });
  assert.match(renderViewControls(store), /data-action="toggle-currency-labels" aria-pressed="false"/);

  const named = (svg) => (svg.match(/emc-label-currency/g) ?? []).length;
  assert.equal(named(render(map, { labels: 'auto', currencyLabels: false })), 0);
  // atp, adp and h, each beside its own small node.
  assert.equal(named(render(map, { labels: 'auto', currencyLabels: true })), 3);
});
