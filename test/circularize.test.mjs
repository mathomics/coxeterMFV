import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import { EditableMap } from '../src/map/editable-map.js';
import { Store } from '../src/ui/store.js';
import { FluxScale } from '../src/render/theme.js';
import { emptySelection } from '../src/map/multi-selection.js';
import { isSecondaryNode } from '../src/map/metabolite-label.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

/* The E. coli core model, laid out in full: its TCA cycle, with FRD7 running beside SUCDi. */
async function ecoliStore() {
  const raw = JSON.parse(await readFile(join(root, 'models', 'ecoli_core_mc.json'), 'utf8'));
  const model = new Model(raw);
  const community = new CommunityModel({ species: [{ id: 'ecoli', model, abundance: 1 }], medium: {} });
  const store = new Store('.');
  store.models.set('fixture', model);
  store.species = [{ id: 'ecoli', file: 'fixture', abundance: 1, enabled: true }];
  store.community = community;
  store.map = EditableMap.fromCommunity(community);
  store.scale = new FluxScale([]);
  return store;
}

const TCA = ['CS', 'ACONT', 'ICDHyr', 'AKGDH', 'SUCOAS', 'SUCDi', 'FUM', 'MDH'];
const id = (store, local) => [...store.map.reactions.keys()].find((rid) => rid.endsWith(`@${local}`));

function pick(store, locals) {
  store.multiSelection = emptySelection();
  for (const local of locals) store.multiSelection.reactions.add(id(store, local));
}

/** The main metabolite nodes a reaction is drawn to. */
const mainNodes = (map, rid) => map.segments
  .filter((seg) => seg.reactionId === rid && seg.nodeId && !isSecondaryNode(map.nodes.get(seg.nodeId)))
  .map((seg) => seg.nodeId);

const distance = (p, c) => Math.hypot(p.x - c.x, p.y - c.y);

function bezierMiddle(seg) {
  const [p0, p3] = seg.side === 'substrate' ? [seg.from, seg.to] : [seg.to, seg.from];
  const at = (k) => (p0[k] + 3 * seg.b1[k] + 3 * seg.b2[k] + p3[k]) / 8;
  return { x: at('x'), y: at('y') };
}

test('the TCA cycle is found among the selected reactions, and a chain is not a cycle', async () => {
  const store = await ecoliStore();
  pick(store, [...TCA, 'FRD7']);
  const cycle = store.map.selectionCycle(store.multiSelection);
  assert.equal(cycle.reactions.length, 8);
  assert.equal(new Set(cycle.metabolites).size, 8);
  assert.ok(store.canCircularizeSelection());

  pick(store, ['PGI', 'PFK', 'FBA']);
  assert.equal(store.map.selectionCycle(store.multiSelection), null);
  assert.equal(store.canCircularizeSelection(), false);
  const json = JSON.stringify(store.map.toJSON());
  const refused = store.circularizeMultiSelection();
  assert.deepEqual(refused, { circled: false, reason: 'cycle' });
  assert.equal(store.status.message, 'The selected reactions do not close a cycle.');
  assert.equal(JSON.stringify(store.map.toJSON()), json, 'a refusal changes nothing');
});

test('Circularize lays the cycle on one circle, in equal steps that grow with it', async () => {
  const store = await ecoliStore();
  pick(store, [...TCA, 'FRD7']);
  const cycle = store.map.selectionCycle(store.multiSelection);
  const json = JSON.stringify(store.map.toJSON());

  const result = store.circularizeMultiSelection();
  assert.equal(result.circled, true);
  assert.equal(result.reactions, 8);
  assert.ok(Math.abs(result.radius - (16 * 84) / (2 * Math.PI)) < 1e-9, 'sixteen steps of 84 px round');
  const { centre, radius } = result;
  const map = store.map;

  // Metabolite, reaction, metabolite, ...: each on the circle, a sixteenth of a turn apart.
  const ring = [];
  cycle.reactions.forEach((rid, i) => {
    const node = mainNodes(map, rid).map((n) => map.nodes.get(n)).find((n) => n.mergedId === cycle.metabolites[i]);
    ring.push(node, map.reactions.get(rid));
  });
  const step = (2 * Math.PI) / 16;
  ring.forEach((item, j) => {
    assert.ok(Math.abs(distance(item, centre) - radius) < 1e-6, `item ${j} on the circle`);
    const next = ring[(j + 1) % ring.length];
    const turn = Math.atan2(next.y - centre.y, next.x - centre.x) - Math.atan2(item.y - centre.y, item.x - centre.x);
    const wrapped = Math.abs(Math.atan2(Math.sin(turn), Math.cos(turn)));
    assert.ok(Math.abs(wrapped - step) < 1e-6, `item ${j} one step from the next`);
  });

  // Names: metabolites outside the circle, reactions inside.
  for (const item of ring) {
    const out = { x: item.x - centre.x, y: item.y - centre.y };
    if (item.axis) {
      assert.ok(item.labelOffset.x * out.x + item.labelOffset.y * out.y < 0, `${item.id} named inside`);
      // Its backbone runs along the circle.
      const along = item.axis.dir.x * out.x + item.axis.dir.y * out.y;
      assert.ok(Math.abs(along) < 1e-6 * radius, `${item.id} along the circle`);
    } else {
      assert.ok(Math.abs(Math.cos(item.labelAngle) * out.y - Math.sin(item.labelAngle) * out.x) < 1e-6 * radius
        && Math.cos(item.labelAngle) * out.x + Math.sin(item.labelAngle) * out.y > 0, `${item.base} named outside`);
    }
  }

  // The cycle's own curves follow the circle between marker and metabolite.
  const ringNodes = new Set(ring.filter((item) => !item.axis).map((n) => n.id));
  for (const rid of cycle.reactions) {
    for (const seg of map.segments.filter((s) => s.reactionId === rid && ringNodes.has(s.nodeId))) {
      assert.ok(Math.abs(distance(bezierMiddle(seg), centre) - radius) < 2, `${rid} curve on the circle`);
    }
  }

  // FRD7 and SUCDi both run between fumarate and succinate: the one not on the
  // circle stands outside it, beside the other, drawn to the same two nodes.
  const [beside] = ['FRD7', 'SUCDi'].map((local) => id(store, local)).filter((rid) => !cycle.reactions.includes(rid));
  const second = map.reactions.get(beside);
  // 90 px out, or as far as its compartment lets it.
  assert.ok(distance(second, centre) > radius + 30 && distance(second, centre) < radius + 90 + 1e-6);
  assert.equal(mainNodes(map, beside).filter((n) => ringNodes.has(n)).length, 2, 'drawn to the circle\'s own nodes');

  // One undoable step.
  assert.equal(map === store.map, true);
  store.undoMap();
  assert.equal(JSON.stringify(store.map.toJSON()), json);
});

test('copies of a cycle metabolite are joined into one node on the circle', async () => {
  const store = await ecoliStore();
  const map = store.map;
  const cs = id(store, 'CS');
  const seg = map.segments.find((s) => s.reactionId === cs && map.nodes.get(s.nodeId)?.mergedId?.endsWith('cit_c'));
  const source = map.nodes.get(seg.nodeId);
  const copy = map.duplicateMetaboliteNode(source.id, { x: source.x + 300, y: source.y });
  seg.nodeId = copy.id;
  const before = map.nodesForMetabolite(copy.mergedId).length;

  pick(store, TCA);
  const result = store.circularizeMultiSelection();
  assert.equal(result.circled, true);
  assert.equal(map.nodesForMetabolite(copy.mergedId).length, before - 1, 'one copy fewer');
  const acont = id(store, 'ACONT');
  const shared = mainNodes(map, cs).filter((n) => mainNodes(map, acont).includes(n));
  assert.equal(shared.length, 1, 'CS and ACONT meet at one citrate');
});

test('a circle with no room in its compartment is refused and changes nothing', async () => {
  const store = await ecoliStore();
  pick(store, TCA);
  const node = store.map.nodes.get(mainNodes(store.map, id(store, 'CS'))[0]);
  const cell = store.map.cells.find((c) => c.id === node.cellId);
  const compartment = cell.compartments.find((c) => c.id === node.compartment);
  compartment.rect = { ...compartment.rect, w: 200, h: 200 };
  const json = JSON.stringify(store.map.toJSON());
  assert.deepEqual(store.circularizeMultiSelection(), { circled: false, reason: 'room' });
  assert.equal(store.status.message, 'No room for the circle in its compartment.');
  assert.equal(JSON.stringify(store.map.toJSON()), json);
});

test('Ctrl-click adds to the selection what was picked before it, and a plain pick lets go', async () => {
  const store = await ecoliStore();
  const [a, b, c] = ['CS', 'ACONT', 'MDH'].map((local) => id(store, local));
  store.select(a);
  store.extendMultiSelection('reaction', b);
  assert.deepEqual([...store.multiSelection.reactions].sort(), [a, b].sort());
  assert.equal(store.selectedReaction, null, 'the inspector turns to the selection');
  store.extendMultiSelection('reaction', b);
  assert.deepEqual([...store.multiSelection.reactions], [a], 'a second Ctrl-click takes it out');
  store.select(c);
  assert.equal(store.multiSelectionSize, 0);
  assert.equal(store.selectedReaction, c);

  const source = await readFile(join(root, 'src/ui/app.js'), 'utf8');
  const selecting = 'const selecting = event.shiftKey || event.ctrlKey || event.metaKey || store.view.multiSelect;';
  assert.ok(source.includes(selecting), 'Shift, Ctrl and Cmd all add to the selection');
  assert.match(source, /store\.extendMultiSelection\(multiHit\.kind, multiHit\.id\)/);
});
