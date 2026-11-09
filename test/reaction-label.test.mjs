import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import { EditableMap } from '../src/map/editable-map.js';
import { normalizeReactionLabelOffset, reactionLabelPlacement } from '../src/map/reaction-label.js';
import { Store } from '../src/ui/store.js';
import { OPERATION_DOMAIN, operationDomain } from '../src/ui/operation-boundary.js';
import { renderMap } from '../src/render/mapview.js';
import { renderInspector } from '../src/ui/panels.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const RID = 'eco@PFK';

async function community() {
  const raw = JSON.parse(await readFile(join(root, 'models', 'ecoli_core_mc.json'), 'utf8'));
  return new CommunityModel({
    species: [{ id: 'eco', model: new Model(raw), abundance: 1 }],
    medium: {},
  });
}

async function pfkMap() {
  const com = await community();
  const map = EditableMap.emptyFromCommunity(com);
  assert.ok(map.addReaction(com, RID));
  return { com, map };
}

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

const render = (map) => renderMap(map, {
  fluxes: new Map(),
  scale: { color: () => '#94a3b8', width: () => 1, reference: 1 },
  showCurrency: true,
  labels: 'all',
});

/** The drawn name of a reaction: its attributes and its class list. */
function drawnName(map, rid = RID) {
  const svg = render(map);
  const text = [...svg.matchAll(/<text\b([^>]*)>[^<]*<\/text>/g)]
    .find((m) => m[1].includes(`data-rxn="${rid}"`));
  assert.ok(text, 'the name is drawn');
  const attrs = Object.fromEntries([...text[1].matchAll(/([\w-]+)="([^"]*)"/g)].map((m) => [m[1], m[2]]));
  return { x: Number(attrs.x), y: Number(attrs.y), anchor: attrs['text-anchor'], classes: attrs.class.split(' ') };
}

const near = (a, b, eps = 0.006) => Math.abs(a - b) <= eps;

test('without an offset the name stands where it always has, in each of the three default places', async () => {
  const { map } = await pfkMap();
  const rxn = map.reactions.get(RID);
  const check = (placement, anchor) => {
    const place = reactionLabelPlacement(map, rxn);
    assert.equal(place.placement, placement);
    assert.equal(place.anchor, anchor);
    const drawn = drawnName(map);
    assert.ok(near(drawn.x, place.x) && near(drawn.y, place.y), 'the renderer draws it there');
    assert.equal(drawn.anchor, anchor);
    assert.ok(drawn.classes.includes(placement));
    return place;
  };

  // Beside the marker: 8 past its right edge, which for a free internal
  // reaction is 3 |cos| + 9 |sin| of its angle.
  assert.ok(map.setReactionAngle(RID, Math.PI / 2));
  const beside = check('emc-label-rxn-adjacent', 'start');
  assert.ok(Math.abs(beside.x - (rxn.x + 9 + 8)) < 1e-9 && beside.y === rxn.y);

  // Straight and on its side: over or under the bar.
  assert.ok(map.alignReaction(RID, 'E'));
  const over = check('emc-label-rxn-above', 'middle');
  assert.ok(over.x === rxn.x && over.y < rxn.y);
  assert.ok(map.alignReaction(RID, 'W'));
  const under = check('emc-label-rxn-below', 'middle');
  assert.ok(under.x === rxn.x && under.y > rxn.y);
  assert.equal(Object.hasOwn(rxn, 'labelOffset'), false, 'nothing was stored');
});

test('a moved name is written from the marker plus its offset, aligned away from the marker', () => {
  const rxn = { id: 'r', x: 100, y: 50, axis: { dir: { x: 0, y: 1 } } };
  const cases = [
    [{ x: 30, y: 0 }, 'start', 'emc-label-rxn-adjacent'],
    [{ x: -30, y: 0 }, 'end', 'emc-label-rxn-adjacent'],
    [{ x: 0, y: -30 }, 'middle', 'emc-label-rxn-above'],
    [{ x: 0, y: 30 }, 'middle', 'emc-label-rxn-below'],
    [{ x: 20, y: -20 }, 'start', 'emc-label-rxn-above'],
    [{ x: -20, y: 20 }, 'end', 'emc-label-rxn-below'],
    // Within 0.3 of square across or along, the name is centred that way.
    [{ x: Math.cos(Math.acos(0.25)), y: Math.sin(Math.acos(0.25)) }, 'middle', 'emc-label-rxn-below'],
    [{ x: 1, y: -0.25 }, 'start', 'emc-label-rxn-adjacent'],
    [{ x: 0, y: 0 }, 'middle', 'emc-label-rxn-adjacent'],
  ];
  for (const [offset, anchor, placement] of cases) {
    const place = reactionLabelPlacement({ segments: [], nodes: new Map() }, { ...rxn, labelOffset: offset });
    assert.deepEqual(place, { x: 100 + offset.x, y: 50 + offset.y, anchor, placement }, JSON.stringify(offset));
  }
});

test('each default place read as an offset gives the same place, so a picked-up name does not jump', async () => {
  const { map } = await pfkMap();
  const rxn = map.reactions.get(RID);
  for (const lay of [() => map.setReactionAngle(RID, 0.4), () => map.alignReaction(RID, 'E'), () => map.alignReaction(RID, 'W')]) {
    assert.ok(lay());
    delete rxn.labelOffset;
    const before = reactionLabelPlacement(map, rxn);
    rxn.labelOffset = { x: before.x - rxn.x, y: before.y - rxn.y };
    const after = reactionLabelPlacement(map, rxn);
    assert.ok(near(after.x, before.x, 1e-9) && near(after.y, before.y, 1e-9));
    assert.equal(after.anchor, before.anchor);
    assert.equal(after.placement, before.placement);
  }
});

test('the drawn name follows its offset, and the offset follows the reaction', async () => {
  const { map } = await pfkMap();
  const rxn = map.reactions.get(RID);
  assert.ok(map.setReactionLabelOffset(RID, { x: -40, y: 25 }));
  let drawn = drawnName(map);
  assert.ok(near(drawn.x, rxn.x - 40) && near(drawn.y, rxn.y + 25));
  assert.equal(drawn.anchor, 'end');
  assert.ok(drawn.classes.includes('emc-label-rxn-below'));

  // Moved, the reaction takes its name along; straightened, it leaves it where it was put.
  map.moveReaction(RID, rxn.x + 30, rxn.y - 20);
  drawn = drawnName(map);
  assert.ok(near(drawn.x, rxn.x - 40) && near(drawn.y, rxn.y + 25));
  assert.ok(map.setReactionLinear(RID, true));
  assert.deepEqual(rxn.labelOffset, { x: -40, y: 25 });
});

test('turning a reaction turns its moved name with it', async () => {
  const { map } = await pfkMap();
  const rxn = map.reactions.get(RID);
  assert.ok(map.setReactionLabelOffset(RID, { x: 30, y: 0 }));
  assert.ok(map.rotateReaction(RID, Math.PI / 2));
  assert.ok(near(rxn.labelOffset.x, 0, 1e-9) && near(rxn.labelOffset.y, 30, 1e-9), 'a quarter turn clockwise on screen');
  assert.equal(reactionLabelPlacement(map, rxn).placement, 'emc-label-rxn-below');
});

test('offsets are saved and loaded, and a damaged one puts the name back in its place', async () => {
  const { map } = await pfkMap();
  map.setReactionLabelOffset(RID, { x: 12.5, y: -7 });
  const doc = JSON.parse(JSON.stringify(map.toJSON()));
  assert.deepEqual(doc.reactions.find((r) => r.id === RID).labelOffset, { x: 12.5, y: -7 });
  assert.deepEqual(EditableMap.fromJSON(doc).reactions.get(RID).labelOffset, { x: 12.5, y: -7 });

  for (const bad of [null, 3, 'x', [], { x: 1 }, { x: 'a', y: 2 }, { x: NaN, y: 0 }, { x: Infinity, y: 0 }]) {
    const copy = structuredClone(doc);
    copy.reactions.find((r) => r.id === RID).labelOffset = bad;
    const loaded = EditableMap.fromJSON(copy).reactions.get(RID);
    assert.equal(Object.hasOwn(loaded, 'labelOffset'), false, JSON.stringify(bad));
  }
  const extra = structuredClone(doc);
  extra.reactions.find((r) => r.id === RID).labelOffset = { x: 1, y: 2, z: 3 };
  assert.deepEqual(EditableMap.fromJSON(extra).reactions.get(RID).labelOffset, { x: 1, y: 2 });

  // A subset of the map is a copy: moving the name there leaves this one alone.
  const subset = map.subsetByReactions([RID]);
  subset.setReactionLabelOffset(RID, { x: 0, y: 40 });
  assert.deepEqual(map.reactions.get(RID).labelOffset, { x: 12.5, y: -7 });

  assert.equal(normalizeReactionLabelOffset({ x: -0.5, y: 2 }).x, -0.5);
  assert.equal(map.setReactionLabelOffset(RID, { x: NaN, y: 1 }), null);
  assert.equal(map.setReactionLabelOffset('missing', { x: 1, y: 1 }), null);
});

test('the store moves a name without history, records the drag once, and resets it as one step', async () => {
  const store = await storeWithPFK();
  const rxn = () => store.map.reactions.get(RID);
  const depth = () => store.mapHistory.undoStack.length;
  const start = depth();

  const drag = store.beginMapEdit('Move reaction label');
  for (const x of [5, 15, 25]) assert.ok(store.setReactionLabelOffset(RID, { x, y: -10 }));
  assert.equal(depth(), start, 'moving it records nothing');
  assert.equal(store.commitMapEdit(drag), true);
  assert.equal(depth(), start + 1);

  assert.equal(store.undoMap().label, 'Move reaction label');
  assert.equal(Object.hasOwn(rxn(), 'labelOffset'), false, 'undo puts it back in its default place');
  assert.equal(store.redoMap().label, 'Move reaction label');
  assert.deepEqual(rxn().labelOffset, { x: 25, y: -10 });

  store.select(RID);
  assert.match(renderInspector(store), /data-action="reaction-label-reset"/);
  assert.ok(store.resetReactionLabel(RID));
  assert.equal(Object.hasOwn(rxn(), 'labelOffset'), false);
  assert.equal(depth(), start + 2);
  assert.doesNotMatch(renderInspector(store), /data-action="reaction-label-reset"/, 'offered only for a moved name');
  assert.equal(store.resetReactionLabel(RID), null, 'nothing to reset');
  assert.equal(depth(), start + 2);
  store.undoMap();
  assert.deepEqual(rxn().labelOffset, { x: 25, y: -10 });

  for (const name of ['setReactionLabelOffset', 'resetReactionLabel']) {
    assert.equal(operationDomain(name), OPERATION_DOMAIN.MAP);
  }
});
