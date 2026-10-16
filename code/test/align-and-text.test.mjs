import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { Store } from '../src/ui/store.js';
import { Model } from '../src/core/model.js';
import { EditableMap } from '../src/map/editable-map.js';
import { renderMap } from '../src/render/mapview.js';
import { renderInspector, renderWorkspace } from '../src/ui/panels.js';
import { emptySelection } from '../src/map/multi-selection.js';
import { TEXT_SIZE, textFontStack } from '../src/core/text-style.js';

const near = (a, b, eps = 1e-6) => Math.abs(a - b) <= eps;

async function ecoliStore() {
  const store = new Store('.');
  const raw = JSON.parse(await readFile(new URL('../models/ecoli_core_mc.json', import.meta.url), 'utf8'));
  store.models.set('ecoli_core_mc', new Model(raw));
  store.scenarios = [{
    id: 'ecoli', name: 'E. coli', objective: 'equal-growth',
    species: [{ id: 'eco', model: 'ecoli_core_mc', abundance: 1 }],
    medium: { glc__D: 10, o2: 20 },
  }];
  await store.useScenario('ecoli');
  return store;
}

function select(store, { reactions = [], nodes = [] } = {}) {
  store.multiSelection = emptySelection();
  for (const id of reactions) store.multiSelection.reactions.add(id);
  for (const id of nodes) store.multiSelection.nodes.add(id);
}

// Escher-style line alignment

test('Align vertical puts selected reactions and their metabolites on one straight line', async () => {
  const store = await ecoliStore();
  const map = () => store.map;
  const reactions = ['eco@PGI', 'eco@PFK'];
  const line = ['met:eco@g6p_c', 'met:eco@f6p_c', 'met:eco@fdp_c'];
  const orderBefore = [...line].sort((a, b) => map().nodes.get(a).y - map().nodes.get(b).y);
  const cofactor = [...map().nodes.values()].find((n) => n.kind === 'currency' && n.reactionId === 'eco@PFK');
  select(store, { reactions });

  const result = store.arrangeMultiSelection('align-vertical');
  assert.ok(result?.changed, 'the map changed and was recorded');
  const x = map().nodes.get(line[0]).x;
  for (const id of line) assert.ok(near(map().nodes.get(id).x, x), `${id} on the line`);
  for (const id of reactions) assert.ok(near(map().reactions.get(id).x, x), `${id} marker on the line`);
  const orderAfter = [...line].sort((a, b) => map().nodes.get(a).y - map().nodes.get(b).y);
  assert.ok(orderBefore.length === 3);
  assert.ok(JSON.stringify(orderAfter) === JSON.stringify(line) || JSON.stringify(orderAfter) === JSON.stringify([...line].reverse()),
    `the pathway reads in its own order along the line, not folded back (${orderAfter.join(' → ')})`);
  for (let i = 1; i < orderAfter.length; i += 1) {
    assert.ok(map().nodes.get(orderAfter[i]).y - map().nodes.get(orderAfter[i - 1]).y >= 80 - 1e-6,
      'neighbours are spread to leave room for a marker between them');
  }
  for (const id of reactions) {
    const { from, to } = map().reactionLineEnds(id);
    const mid = (map().nodes.get(from).y + map().nodes.get(to).y) / 2;
    assert.ok(near(map().reactions.get(id).y, mid, 1e-3), `${id} marker half way between its metabolites`);
  }
  for (const id of reactions) {
    assert.ok(map().reactions.get(id).linear, `${id} is kept straight`);
    for (const seg of map().segments.filter((s) => s.reactionId === id && line.includes(s.nodeId))) {
      for (const point of [seg.b1, seg.b2].filter(Boolean)) assert.ok(near(point.x, x, 1e-3), `${id} branch to ${seg.nodeId} is straight`);
    }
  }
  if (cofactor) {
    const placed = map().nodes.get(cofactor.id);
    assert.ok(!near(placed.x, x, 0.5), 'cofactors are placed beside the line, not stacked on it');
    assert.ok(Math.hypot(placed.x - map().reactions.get('eco@PFK').x, placed.y - map().reactions.get('eco@PFK').y) < 120,
      'and stay by their reaction');
  }

  store.undoMap();
  assert.ok(!near(store.map.nodes.get(line[0]).x, x) || !near(store.map.nodes.get(line[2]).x, x), 'one Undo puts it back');
});

test('Align horizontal does the same along a horizontal line', async () => {
  const store = await ecoliStore();
  select(store, { reactions: ['eco@PGI'] });
  store.arrangeMultiSelection('align-horizontal');
  const y = store.map.reactions.get('eco@PGI').y;
  assert.ok(near(store.map.nodes.get('met:eco@g6p_c').y, y));
  assert.ok(near(store.map.nodes.get('met:eco@f6p_c').y, y));
});

test('one selected reaction is enough to align, and the panel offers it', async () => {
  const store = await ecoliStore();
  select(store, { reactions: ['eco@PGI'] });
  assert.equal(store.canArrangeSelection('align-vertical'), true);
  assert.equal(store.canArrangeSelection('align-left'), false, 'edge alignment still needs two objects');
  store.view.multiSelect = true;
  const html = renderWorkspace(store, 'map');
  assert.match(html, /data-action="align-vertical"(?![^>]*disabled)[^>]*title="Put the selected reactions and their metabolites on one vertical line/);
  assert.match(html, /data-action="align-left" disabled/);
});

test('a selection without reactions keeps the plain alignment of centres', async () => {
  const store = await ecoliStore();
  select(store, { nodes: ['met:eco@g6p_c', 'met:eco@fdp_c'] });
  const pgiBefore = { ...store.map.reactions.get('eco@PGI') };
  store.arrangeMultiSelection('align-vertical');
  assert.ok(near(store.map.nodes.get('met:eco@g6p_c').x, store.map.nodes.get('met:eco@fdp_c').x));
  assert.equal(store.map.reactions.get('eco@PGI').linear, pgiBefore.linear, 'no reaction was made straight');
});

test('transporters stay on their membranes when a selection is aligned', async () => {
  const store = await ecoliStore();
  const transporter = [...store.map.reactions.values()].find((r) => r.onMembrane);
  assert.ok(transporter, 'the fixture has a transporter');
  const before = { x: transporter.x, y: transporter.y };
  select(store, { reactions: [transporter.id, 'eco@PGI'] });
  store.arrangeMultiSelection('align-vertical');
  const after = store.map.reactions.get(transporter.id);
  assert.ok(near(after.x, before.x) && near(after.y, before.y));
});

test('a reaction aligned on its own keeps the way it runs', async () => {
  const store = await ecoliStore();
  const ends = store.map.reactionLineEnds('eco@PGI');
  const wasBelow = store.map.nodes.get(ends.to).y >= store.map.nodes.get(ends.from).y;
  store.alignMapReactionAxis('eco@PGI', 'vertical');
  const from = store.map.nodes.get(ends.from);
  const to = store.map.nodes.get(ends.to);
  const marker = store.map.reactions.get('eco@PGI');
  assert.ok(near(from.x, marker.x) && near(to.x, marker.x), 'on one vertical line through the marker');
  assert.equal(to.y >= from.y, wasBelow, 'the product stays on the side it was');

  store.alignMapReactionAxis('eco@PGI', 'horizontal');
  assert.ok(near(store.map.nodes.get(ends.from).y, marker.y) && near(store.map.nodes.get(ends.to).y, marker.y));
});

test('the reaction inspector offers Vertical and Horizontal', async () => {
  const store = await ecoliStore();
  store.selectedReaction = 'eco@PGI';
  const html = renderInspector(store);
  assert.match(html, /data-action="reaction-align-axis"[^>]*data-axis="vertical"/);
  assert.match(html, /data-action="reaction-align-axis"[^>]*data-axis="horizontal"/);
});

// Text size and font

test('map text keeps a size and a font through saving and drawing', () => {
  const map = EditableMap.blank({ width: 800, height: 600 });
  const text = map.addText('Glycolysis', 100, 100, { fontSize: 30, fontFamily: 'serif' });
  assert.equal(text.fontSize, 30);
  assert.equal(text.fontFamily, 'serif');
  assert.equal(map.addText('x', 1, 1, { fontFamily: 'comic' }).fontFamily, 'sans', 'an unknown font falls back');
  map.styleText(text.id, { fontSize: 500 });
  assert.equal(text.fontSize, TEXT_SIZE.max, 'sizes are kept within range');
  map.styleText(text.id, { fontSize: '', fontFamily: 'mono' });
  assert.equal(text.fontSize, TEXT_SIZE.max, 'an empty size changes nothing');
  assert.equal(text.fontFamily, 'mono');

  const restored = EditableMap.fromJSON(map.toJSON());
  assert.equal(restored.texts.get(text.id).fontFamily, 'mono');
  const svg = renderMap(restored, { fluxes: new Map(), scale: { reference: 1, width: () => 2, color: () => '#000' } });
  assert.match(svg, new RegExp(`data-map-text="${text.id}" style="font-family: [^"]*monospace`));
  assert.ok(svg.includes(`font-size="${TEXT_SIZE.max}"`));
  assert.doesNotMatch(svg.split(`data-map-text="${text.id}"`)[0].split('<text').pop() ?? '', /style=/,
    'texts in the default font carry no override');
  assert.ok(textFontStack('serif').includes('Georgia'));
});

test('restyling a text is one Undo step and becomes the style of the next text', async () => {
  const store = await ecoliStore();
  const first = store.addMapText('Pathway', 50, 50);
  assert.equal(first.fontSize, TEXT_SIZE.default);
  store.styleMapText(first.id, { fontSize: 40, fontFamily: 'times' });
  assert.equal(store.map.texts.get(first.id).fontFamily, 'times');
  assert.deepEqual(store.view.textStyle, { fontSize: 40, fontFamily: 'times' });

  const second = store.addMapText('Next', 60, 60);
  assert.equal(second.fontSize, 40, 'the next text uses the last size');
  assert.equal(second.fontFamily, 'times', 'and the last font');

  store.undoMap();   // undoes adding the second text
  store.undoMap();   // undoes the restyle
  assert.equal(store.map.texts.get(first.id).fontSize, TEXT_SIZE.default);

  store.updateMapText(first.id, 'Renamed', { fontSize: 18, fontFamily: 'courier' });
  const edited = store.map.texts.get(first.id);
  assert.equal(edited.text, 'Renamed');
  assert.equal(edited.fontFamily, 'courier');
  store.undoMap();
  assert.equal(store.map.texts.get(first.id).text, 'Pathway', 'words and style are one step');
});

test('the text inspector offers a size and a font', async () => {
  const store = await ecoliStore();
  const text = store.addMapText('Label', 50, 50, { fontSize: 26, fontFamily: 'serif' });
  store.selectedEntity = { type: 'text', id: text.id };
  const html = renderInspector(store);
  assert.match(html, /data-action="text-font-size"[^>]*value="26"/);
  assert.match(html, /<option value="serif" selected>Serif<\/option>/);
});

test('a pathway drawn across the line is not folded back on itself', async () => {
  const store = await ecoliStore();
  const map = store.map;
  // Lay g6p, f6p and fdp side by side, f6p a little below g6p: projected onto a
  // vertical line by position alone, g6p would land between f6p and fdp.
  map.moveNode('met:eco@fdp_c', 600, 300);
  map.moveNode('met:eco@g6p_c', 400, 500);
  map.moveNode('met:eco@f6p_c', 500, 520);
  select(store, { reactions: ['eco@PGI', 'eco@PFK'] });
  store.arrangeMultiSelection('align-vertical');
  const y = (id) => store.map.nodes.get(id).y;
  const ordered = ['met:eco@g6p_c', 'met:eco@f6p_c', 'met:eco@fdp_c'].sort((a, b) => y(a) - y(b));
  assert.equal(ordered[1], 'met:eco@f6p_c', 'f6p stays between g6p and fdp');
  for (const id of ['eco@PGI', 'eco@PFK']) {
    const { from, to } = store.map.reactionLineEnds(id);
    const [low, high] = [y(from), y(to)].sort((a, b) => a - b);
    const marker = store.map.reactions.get(id).y;
    assert.ok(marker > low && marker < high, `${id} sits between its own metabolites`);
    for (const other of ['met:eco@g6p_c', 'met:eco@f6p_c', 'met:eco@fdp_c'].filter((n) => n !== from && n !== to)) {
      assert.ok(!(y(other) > low && y(other) < high), `${id}'s line does not run over ${other}`);
    }
  }
});

// Alignment edge cases

test('a metabolite a membrane stops is reported, and spacing follows where it landed', async () => {
  const store = await ecoliStore();
  const cell = store.map.cells[0];
  const cytosol = cell.compartments.find((c) => c.id === 'c');
  const periplasm = cell.compartments.find((c) => c.id === 'p');
  const outside = [...store.map.nodes.values()].find((n) => n.compartment === 'p' && n.kind === 'metabolite' && n.primary !== false);
  assert.ok(cytosol && periplasm && outside, 'fixture has a cytosol, a periplasm and a periplasmic metabolite');
  // Beside the cytosol, inside the periplasm band, at the cytosol's height.
  store.map.moveNode(outside.id, (periplasm.rect.x + cytosol.rect.x) / 2, cytosol.rect.y + cytosol.rect.h / 2);
  select(store, { reactions: ['eco@PGI'], nodes: [outside.id] });
  const result = store.arrangeMultiSelection('align-vertical');
  assert.ok(result.held.includes(outside.id), 'the periplasmic metabolite could not cross into the cytosol');
  assert.equal(store.status.state, 'warn');
  assert.match(store.status.message, /could not reach the line/);
  const landed = store.map.nodes.get(outside.id);
  assert.ok(landed.x < cytosol.rect.x, 'it stayed outside the cytosol');
});

test('a cofactor helper is never put on the line', async () => {
  const store = await ecoliStore();
  const ends = store.map.reactionLineEnds('eco@TPI');
  assert.equal(store.map.nodes.get(ends.to).kind, 'currency', 'TPI runs to its own g3p helper in this map');
  select(store, { reactions: ['eco@TPI'] });
  store.arrangeMultiSelection('align-vertical');
  const x = store.map.reactions.get('eco@TPI').x;
  assert.ok(near(store.map.nodes.get(ends.from).x, x), 'dhap is on the line');
  assert.ok(!near(store.map.nodes.get(ends.to).x, x, 0.5), 'the helper is placed beside it');
  assert.ok(!store.map.reactions.get('eco@TPI').linear, 'no straight line is drawn to a helper');
});

test('a transporter-only selection is aligned as plain objects, as before', async () => {
  const store = await ecoliStore();
  const transporter = [...store.map.reactions.values()].find((r) => r.onMembrane);
  select(store, { reactions: [transporter.id], nodes: ['met:eco@g6p_c'] });
  assert.equal(store.map.selectionHasLineReactions(store.multiSelection), false);
  const result = store.arrangeMultiSelection('align-vertical');
  assert.ok('count' in result && !('held' in result), 'the plain centre alignment ran');
  select(store, { reactions: [transporter.id] });
  assert.equal(store.canArrangeSelection('align-vertical'), false, 'one transporter alone offers nothing to align');
});

test('a selected title joins the line instead of pulling the pathway to it', async () => {
  const store = await ecoliStore();
  const ends = store.map.reactionLineEnds('eco@PGI');
  const anchors = [store.map.nodes.get(ends.from), store.map.nodes.get(ends.to), store.map.reactions.get('eco@PGI')];
  const expected = anchors.reduce((sum, p) => sum + p.x, 0) / anchors.length;
  const title = store.addMapText('Glycolysis', expected - 2000, 100);
  select(store, { reactions: ['eco@PGI'] });
  store.multiSelection.texts.add(title.id);
  store.arrangeMultiSelection('align-vertical');
  assert.ok(near(store.map.reactions.get('eco@PGI').x, expected, 1e-6), 'the line is where the pathway was');
  const { textCentre } = await import('../src/map/multi-selection.js');
  assert.ok(near(textCentre(store.map.texts.get(title.id)).x, expected, 1e-6), 'and the title is centred on it');
});

test('unselected reactions drawn to a moved metabolite are put in order where they stand', async () => {
  const store = await ecoliStore();
  const neighbour = 'eco@G6PDH2r';
  // A curve shaped by hand earlier, its handles now far from both its ends.
  for (const seg of store.map.segments.filter((s) => s.reactionId === neighbour && s.nodeId === 'met:eco@g6p_c')) {
    seg.b1 = { x: 0, y: 0 };
    seg.b2 = { x: 5, y: 5 };
  }
  select(store, { reactions: ['eco@PGI'] });
  store.arrangeMultiSelection('align-vertical');
  const rxn = store.map.reactions.get(neighbour);
  const g6p = store.map.nodes.get('met:eco@g6p_c');
  const box = {
    left: Math.min(rxn.x, g6p.x) - 90, right: Math.max(rxn.x, g6p.x) + 90,
    top: Math.min(rxn.y, g6p.y) - 90, bottom: Math.max(rxn.y, g6p.y) + 90,
  };
  for (const seg of store.map.segments.filter((s) => s.reactionId === neighbour && s.nodeId === g6p.id)) {
    for (const p of [seg.b1, seg.b2].filter(Boolean)) {
      assert.ok(p.x >= box.left && p.x <= box.right && p.y >= box.top && p.y <= box.bottom,
        `${neighbour}'s curve to g6p stays between its marker and g6p`);
    }
  }
});

test('text boxes follow the font they are drawn in', async () => {
  const { textBox } = await import('../src/map/multi-selection.js');
  const base = { text: 'Glycolysis pathway', fontSize: 40, x: 0, y: 0 };
  const courier = textBox({ ...base, fontFamily: 'courier' });
  const times = textBox({ ...base, fontFamily: 'times' });
  const plain = textBox(base);
  assert.ok(courier.right > plain.right && plain.right > times.right, 'a monospace label is wider than a Times one');
});
