/**
 * How reaction lines and metabolite circles are drawn: hops where lines cross,
 * grey dashes for reactions without flux, width by flux, colours set on the
 * map, and the hover text and metadata of an exported SVG.
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { findLineHops, hoppedPath, curvePoint } from '../src/render/line-hops.js';
import { renderMap } from '../src/render/mapview.js';
import { Store } from '../src/ui/store.js';
import { Model } from '../src/core/model.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const p = (x, y) => ({ x, y });

// Line hops

test('of two reactions whose lines cross, the one drawn earlier is cut where the later one passes over', () => {
  const items = [
    { curve: [p(0, 50), p(100, 50)], reactionId: 'A', width: 7.5 },
    { curve: [p(50, 0), p(50, 100)], reactionId: 'B', width: 7.5 },
  ];
  const hops = findLineHops(items);
  assert.deepEqual([...hops.keys()], [0]);
  const [{ t, half }] = hops.get(0);
  assert.ok(Math.abs(t - 0.5) < 1e-9);
  // The cut clears the top line's half width, a gap and its own round cap.
  assert.ok(half > 7.5 / 2 + 7.5 / 2, `gap half-length ${half}`);
});

test('a shallow crossing gets a longer gap than a square one', () => {
  const square = findLineHops([
    { curve: [p(0, 50), p(100, 50)], reactionId: 'A', width: 7.5 },
    { curve: [p(50, 0), p(50, 100)], reactionId: 'B', width: 7.5 },
  ]).get(0)[0].half;
  const shallow = findLineHops([
    { curve: [p(0, 50), p(200, 50)], reactionId: 'A', width: 7.5 },
    { curve: [p(0, 0), p(200, 100)], reactionId: 'B', width: 7.5 },
  ]).get(0)[0].half;
  assert.ok(shallow > square * 1.5, `${shallow} against ${square}`);
});

test('a line ending by a crossing stays whole and the other line is cut instead', () => {
  const hops = findLineHops([
    { curve: [p(0, 50), p(60, 50)], reactionId: 'A', width: 7.5 },
    { curve: [p(50, 0), p(50, 100)], reactionId: 'B', width: 7.5 },
  ]);
  assert.deepEqual([...hops.keys()], [1], 'A runs on to its metabolite uncut');
});

test('a line that ends on another touches it and cuts nothing', () => {
  assert.equal(findLineHops([
    { curve: [p(0, 50), p(53, 50)], reactionId: 'A', width: 7.5 },
    { curve: [p(50, 0), p(50, 100)], reactionId: 'B', width: 7.5 },
  ]).size, 0);
});

test('a cubic branch under a line is cut where it crosses', () => {
  const curve = [p(0, 0), p(40, 120), p(80, -40), p(120, 80)];
  const hops = findLineHops([
    { curve, reactionId: 'B', width: 7.5 },
    { curve: [p(0, 40), p(120, 40)], reactionId: 'A', width: 7.5 },
  ]);
  assert.ok(hops.get(0)?.length >= 1);
  for (const { t } of hops.get(0)) assert.ok(Math.abs(curvePoint(curve, t).y - 40) < 2, 'on the line it crosses');
});

test('lines of one reaction, and lines that only meet where they end, are not cut', () => {
  assert.equal(findLineHops([
    { curve: [p(0, 50), p(100, 50)], reactionId: 'A', width: 7.5 },
    { curve: [p(50, 0), p(50, 100)], reactionId: 'A', width: 7.5 },
  ]).size, 0, 'a reaction’s own branches meet at its centre');
  assert.equal(findLineHops([
    { curve: [p(0, 0), p(100, 100)], reactionId: 'A', width: 7.5 },
    { curve: [p(100, 101), p(200, 0)], reactionId: 'B', width: 7.5 },
  ]).size, 0, 'two lines ending at one metabolite');
});

test('past the limit, a map gets no gaps at all', () => {
  const items = [
    { curve: [p(0, 50), p(100, 50)], reactionId: 'A', width: 7.5 },
    { curve: [p(50, 0), p(50, 100)], reactionId: 'B', width: 7.5 },
  ];
  assert.equal(findLineHops(items, { limit: 0 }).size, 0);
});

test('a cut path starts and ends where the line does and leaves a gap where it is crossed', () => {
  const d = hoppedPath([p(0, 50), p(100, 50)], [{ t: 0.5, half: 10 }]);
  assert.equal(d, 'M 0 50 L 40 50 M 60 50 L 100 50');
  const curve = [p(0, 0), p(30, 90), p(70, 90), p(100, 0)];
  const cut = hoppedPath(curve, [{ t: 0.5, half: 8 }]);
  assert.match(cut, /^M 0 0 C [^M]* M [\d.]+ [\d.]+ C .*, 100 0$/);
  assert.equal(hoppedPath([p(0, 50), p(100, 50)], [{ t: 0.03, half: 10 }]), 'M 0 50 L 100 50', 'no gap runs off the end');
});

// A model with a solution

const miniModel = {
  id: 'mini',
  compartments: { e: 'extracellular', c: 'cytoplasm' },
  metabolites: [
    { id: 'glc_e', name: 'glucose', compartment: 'e' },
    { id: 'glc_c', name: 'glucose', compartment: 'c', formula: 'C6H12O6' },
    { id: 'g6p_c', name: 'glucose 6-phosphate', compartment: 'c' },
    { id: 'dead_c', name: 'dead end', compartment: 'c' },
  ],
  reactions: [
    { id: 'EX_glc_e', metabolites: { glc_e: -1 }, lower_bound: -10, upper_bound: 1000 },
    { id: 'GLCt', name: 'glucose transport', metabolites: { glc_e: -1, glc_c: 1 }, lower_bound: 0, upper_bound: 1000 },
    { id: 'HEX', name: 'hexokinase', metabolites: { glc_c: -1, g6p_c: 1 }, lower_bound: 0, upper_bound: 1000, subsystem: 'Glycolysis' },
    // Nothing takes dead_c away, so in steady state this carries nothing.
    { id: 'DEAD', name: 'dead end', metabolites: { g6p_c: -1, dead_c: 1 }, lower_bound: 0, upper_bound: 1000, subsystem: 'Glycolysis' },
    { id: 'BIO', name: 'growth', metabolites: { g6p_c: -1 }, lower_bound: 0, upper_bound: 1000, objective_coefficient: 1 },
  ],
};

async function miniStore() {
  const store = new Store('.');
  store.models.set('mini', new Model(miniModel));
  store.scenarios = [{
    id: 'mini', name: 'Mini', objective: 'equal-growth',
    species: [{ id: 'sp', model: 'mini', abundance: 1 }], medium: { glc: 10 },
  }];
  await store.useScenario('mini');
  assert.equal(store.solution.status, 'optimal');
  return store;
}

function stateFor(store, extra = {}) {
  return {
    fluxes: store.displayFluxes(), solvedFluxes: store.solution.fluxes, scale: store.scale,
    reactionColorMode: store.view.reactionColorMode, reactionColors: store.reactionColoring().colors,
    ...extra,
  };
}

/** The <path> elements a reaction's lines are drawn with. */
function segmentsOf(svg, rid) {
  return [...svg.matchAll(/<path class="emc-seg[^"]*"[^>]*>/g)].map((m) => m[0])
    .filter((tag) => tag.includes(`data-rxn="${rid}"`));
}

test('with a solution, a reaction that carries no flux is drawn grey and dashed', async () => {
  const store = await miniStore();
  const svg = renderMap(store.map, stateFor(store, { offStyle: true }));
  const dead = segmentsOf(svg, 'sp@DEAD');
  const hex = segmentsOf(svg, 'sp@HEX');
  assert.ok(dead.length && hex.length);
  for (const tag of dead) {
    assert.match(tag, /is-off/);
    assert.match(tag, /stroke="#9b9da5"/);
    assert.match(tag, /stroke-dasharray="/);
  }
  for (const tag of hex) assert.doesNotMatch(tag, /is-off|stroke-dasharray/);
  // Its name, where names are shown, is greyed with it.
  const named = renderMap(store.map, stateFor(store, { offStyle: true, labels: 'all' }));
  assert.match(named, /<text class="emc-label emc-label-rxn is-off[^"]*"[^>]*data-rxn="sp@DEAD"/);
  assert.doesNotMatch(named, /<text class="emc-label emc-label-rxn is-off[^"]*"[^>]*data-rxn="sp@HEX"/);

  const plain = renderMap(store.map, stateFor(store, { offStyle: false }));
  assert.doesNotMatch(plain, /is-off/, 'switched off, every line keeps its own colour');
});

test('line width follows flux only when asked to', async () => {
  const store = await miniStore();
  const widths = (svg) => new Set([...svg.matchAll(/<path class="emc-seg[^"]*"[^>]*stroke-width="([\d.]+)"/g)].map((m) => m[1]));
  assert.equal(widths(renderMap(store.map, stateFor(store))).size, 1);
  assert.ok(widths(renderMap(store.map, stateFor(store, { reactionWidthMode: 'flux' }))).size > 1);
});

test('a reaction’s own colour is used over its pathway’s, but not over flux colours', async () => {
  const store = await miniStore();
  store.setMapReactionColor(['sp@HEX'], '#123456');
  store.setView({ reactionColorMode: 'subsystem' });
  const bySubsystem = renderMap(store.map, stateFor(store));
  for (const tag of segmentsOf(bySubsystem, 'sp@HEX')) assert.match(tag, /stroke="#123456"/);
  assert.doesNotMatch(segmentsOf(bySubsystem, 'sp@DEAD')[0], /#123456/);

  store.setView({ reactionColorMode: 'flux' });
  const byFlux = renderMap(store.map, stateFor(store));
  for (const tag of segmentsOf(byFlux, 'sp@HEX')) assert.doesNotMatch(tag, /#123456/);
});

test('metabolite circles take the map’s colours, and one metabolite its own', async () => {
  const store = await miniStore();
  const circles = (svg) => [...svg.matchAll(/<circle class="emc-node[^"]*"[^>]*>/g)].map((m) => m[0]);
  store.setMapNodeStyle({ fill: '#aabbcc', stroke: '#112233' });
  const nodeId = [...store.map.nodes.values()].find((node) => node.mergedId === 'sp@g6p_c').id;
  store.setMapNodeColors([nodeId], { fill: '#ff0000' });
  const svg = renderMap(store.map, stateFor(store));
  const own = circles(svg).find((tag) => tag.includes(`data-node="${nodeId}"`));
  assert.match(own, /fill="#ff0000"/);
  assert.match(own, /stroke="#112233"/, 'what it does not set comes from the map');
  assert.ok(circles(svg).filter((tag) => !tag.includes(nodeId)).every((tag) => tag.includes('fill="#aabbcc"')));
});

test('colour edits are undoable and travel in the Map JSON', async () => {
  const store = await miniStore();
  const nodeId = [...store.map.nodes.values()].find((node) => node.mergedId === 'sp@g6p_c').id;
  store.setMapNodeStyle({ fill: '#aabbcc' });
  store.setMapNodeColors([nodeId], { fill: '#ff0000' });
  store.setMapReactionColor(['sp@HEX'], '#123456');
  const saved = store.mapJSON();

  store.undoMap();
  assert.equal(store.map.reactions.get('sp@HEX').color, undefined);
  store.undoMap();
  assert.equal(store.map.nodes.get(nodeId).fill, undefined);
  store.undoMap();
  assert.equal(store.map.nodeStyle, null);

  store.loadMapJSON(saved);
  assert.deepEqual(store.map.nodeStyle, { fill: '#aabbcc' });
  assert.equal(store.map.nodes.get(nodeId).fill, '#ff0000');
  assert.equal(store.map.reactions.get('sp@HEX').color, '#123456');

  store.setMapNodeColors([nodeId], { fill: null, stroke: null });
  store.setMapReactionColor(['sp@HEX'], null);
  assert.equal(store.map.nodes.get(nodeId).fill, undefined, 'reset gives it back to the map');
  assert.equal(store.map.reactions.get('sp@HEX').color, undefined);
});

test('the line switches are kept in the Scenario JSON', async () => {
  const store = await miniStore();
  assert.deepEqual([store.view.widthByFlux, store.view.dashNoFlux, store.view.lineHops], [false, true, true]);
  store.setView({ widthByFlux: 1, dashNoFlux: 0, lineHops: false });
  const doc = JSON.parse(store.stateJSON());
  assert.deepEqual([doc.widthByFlux, doc.dashNoFlux, doc.lineHops], [true, false, false]);

  store.setView({ widthByFlux: false, dashNoFlux: true, lineHops: true });
  await store.loadState(JSON.stringify(doc));
  assert.deepEqual([store.view.widthByFlux, store.view.dashNoFlux, store.view.lineHops], [true, false, false]);
});

test('on a real map, gaps are drawn without adding or losing a line', async () => {
  const store = new Store('.');
  const index = JSON.parse(await readFile(join(root, 'models/index.json'), 'utf8'));
  for (const entry of index.models) {
    store.models.set(entry.file.replace(/\.json$/, ''), new Model(JSON.parse(await readFile(join(root, 'models', entry.file), 'utf8'))));
  }
  store.scenarios = JSON.parse(await readFile(join(root, 'models/scenarios.json'), 'utf8')).scenarios;
  await store.useScenario('solo-levadura');
  const count = (svg) => (svg.match(/<path class="emc-seg/g) ?? []).length;
  const plain = renderMap(store.map, stateFor(store));
  const hopped = renderMap(store.map, stateFor(store, { lineHops: true }));
  assert.equal(count(hopped), count(plain));
  assert.ok((hopped.match(/<path class="emc-seg[^>]* d="M [^"]* M /g) ?? []).length > 0, 'the yeast core map has crossings');
});

// SVG export

test('an exported map says what each reaction and metabolite is, and what drew it', async () => {
  const store = await miniStore();
  const tips = store.exportTooltips();
  const reaction = tips.reaction('sp@HEX');
  assert.match(reaction, /^HEX\nhexokinase\nSubsystem: Glycolysis\nBounds: \[0, 1000\]\nFlux: 10/);
  const nodeId = [...store.map.nodes.values()].find((node) => node.mergedId === 'sp@glc_c').id;
  assert.equal(tips.node(nodeId), 'glc\nglucose\nID: glc_c\nFormula: C6H12O6\nCompartment: cytoplasm');

  const svg = renderMap(store.map, stateFor(store, { forExport: true, tooltips: tips }));
  assert.match(svg, /<title>HEX\nhexokinase/);

  const about = store.exportMetadata();
  assert.equal(about.generator, 'Coxeter Multicell Flux Visualizer');
  assert.equal(about.scenario.id, 'mini');
  assert.deepEqual(about.species.map((sp) => sp.model), ['mini']);
  assert.equal(about.analysis.status, 'optimal');
  assert.deepEqual(about.map.labels, { metabolite: 'base', reaction: 'id' });
  assert.equal(about.map.lineHops, true);
});
