// Colouring lines by |flux| alone, as Escher's absolute-value option does:
// by default white at no flux to red at the largest |flux| on the map. The
// points are editable, and the colours can follow log |flux| instead.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import { EditableMap } from '../src/map/editable-map.js';
import { FluxScale, absoluteLogFloor, normalizeAbsoluteFluxColorStops } from '../src/render/theme.js';
import { buildReactionColoring, isFluxColorMode, normalizeReactionColorMode } from '../src/render/reaction-colors.js';
import { renderMap } from '../src/render/mapview.js';
import { renderMapQuickControls, renderWorkspace } from '../src/ui/panels.js';
import { createMapDocument } from '../src/map/map-document.js';
import { Store } from '../src/ui/store.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

async function makeStore() {
  const store = new Store('.');
  store.models.set('demo', new Model({
    id: 'demo', compartments: { c: 'Cytosol', e: 'Extracellular' },
    metabolites: [{ id: 'a_e', compartment: 'e' }, { id: 'a_c', compartment: 'c' }],
    reactions: [
      { id: 'EX_a_e', metabolites: { a_e: -1 }, lower_bound: -10, upper_bound: 1000 },
      { id: 'At', metabolites: { a_e: -1, a_c: 1 }, lower_bound: 0, upper_bound: 1000 },
      { id: 'BIOMASS', metabolites: { a_c: -1 }, lower_bound: 0, upper_bound: 1000, objective_coefficient: 1 },
    ],
  }));
  store.scenarios = [{
    id: 'demo', name: 'Demo', objective: 'equal-growth',
    species: [{ id: 'demo', model: 'demo', abundance: 1 }], medium: { a: 10 },
  }];
  await store.useScenario('demo');
  return store;
}

const blue = [{ value: 0, color: '#ffffff' }, { value: 10, color: '#0000ff' }];

test('absolute flux runs linearly from white at 0 to red at the largest |flux|, whatever the sign', () => {
  const scale = new FluxScale([-1000, -3, 0, 250, 500]);
  assert.equal(scale.absoluteColor(0), '#ffffff');
  assert.equal(scale.absoluteColor(1000), '#dc2626');
  assert.equal(scale.absoluteColor(-1000), '#dc2626');
  assert.equal(scale.absoluteColor(500), '#ee9393', 'halfway is halfway, not log-compressed');
  assert.equal(scale.absoluteColor(-500), scale.absoluteColor(500));
  assert.equal(scale.absoluteColor(1e9), '#dc2626');
  assert.deepEqual(scale.absoluteLegendStops().map(({ t, value, color }) => ({ t, value, color })), [
    { t: 0, value: 0, color: '#ffffff' },
    { t: 1, value: 1000, color: '#dc2626' },
  ]);
});

test('absolute flux ignores custom signed stops, and with no flux keeps the theme grey', () => {
  const custom = new FluxScale([-10, 10], { colorStops: [{ value: -10, color: '#0000ff' }, { value: 10, color: '#00ff00' }] });
  assert.equal(custom.absoluteColor(-10), '#dc2626');
  assert.equal(new FluxScale([]).absoluteColor(0), '#cbd5e1');
  assert.equal(new FluxScale([0], { dark: true }).absoluteColor(0), '#475569');
});

test('absflux is a flux colour mode with no categories', () => {
  assert.equal(normalizeReactionColorMode('absflux'), 'absflux');
  assert.ok(isFluxColorMode('absflux') && isFluxColorMode('flux'));
  assert.ok(!isFluxColorMode('pathway') && !isFluxColorMode('subsystem'));
  const model = new Model({
    id: 'm', compartments: { c: 'c' },
    metabolites: [{ id: 'a_c', compartment: 'c' }],
    reactions: [{ id: 'R1', metabolites: { a_c: -1 }, subsystem: 'S' }],
  });
  const coloring = buildReactionColoring(model, 'absflux');
  assert.equal(coloring.mode, 'absflux');
  assert.equal(coloring.colors.size, 0);
  assert.deepEqual(coloring.legend, []);
});

test('the map strokes a reaction by its |flux| in absflux mode, over its own colour', async () => {
  const raw = JSON.parse(await readFile(join(root, 'models', 'ecoli_core_mc.json'), 'utf8'));
  const community = new CommunityModel({
    species: [{ id: 'eco', model: new Model(raw), abundance: 1 }],
    medium: { glc__D: { lower: -10 }, o2: { lower: -20 } },
  });
  const map = EditableMap.fromCommunity(community);
  const [a, b] = [...new Set(map.segments.map((seg) => seg.reactionId))];
  map.reactions.get(a).color = '#123456';
  const fluxes = new Map([[a, -500], [b, 1000]]);
  const svg = renderMap(map, { fluxes, scale: new FluxScale(fluxes.values()), reactionColorMode: 'absflux', showCrossFeeding: false });
  assert.ok(svg.includes('stroke="#ee9393"'), 'a flux of -500 against a largest of 1000 is half red');
  assert.ok(svg.includes('stroke="#dc2626"'));
  assert.ok(!svg.includes('stroke="#123456"'));
});

test('absolute flux is offered where lines are coloured, and the palette card can switch to it', async () => {
  const store = await makeStore();
  assert.match(renderMapQuickControls(store), /<option value="absflux">Color: absolute flux<\/option>/);

  store.setView({ reactionColorMode: 'absflux' });
  assert.match(renderMapQuickControls(store), /<option value="absflux" selected>/);
  const card = renderWorkspace(store, 'map');
  assert.match(card, /data-action="flux-colors-absolute" type="button" aria-pressed="true"/);
  assert.match(card, /data-action="flux-colors-reset" type="button" aria-pressed="false"/);
  assert.match(card, /data-action="color-by-flux" checked/);
  assert.match(card, /aria-label="Absolute flux color gradient, 0 to 10"/);

  // A new pathway turns ribbons on without taking the lines back to signed flux.
  store.createManualPathway({ name: 'Uptake', reactionIds: [[...store.community.merged.reactions.keys()][0]] });
  assert.equal(store.view.pathwayRibbonMode, 'pathway');
  assert.equal(store.view.reactionColorMode, 'absflux');
});

test('the absolute points are editable, in |flux|, and none may be negative', () => {
  const scale = new FluxScale([10], { absoluteStops: blue });
  assert.equal(scale.absoluteColor(5), '#8080ff');
  assert.equal(scale.absoluteColor(-10), '#0000ff');
  assert.equal(scale.absoluteColor(20), '#0000ff', 'beyond the top point keeps its colour');
  assert.deepEqual(scale.absolutePalette(), blue);
  assert.throws(() => normalizeAbsoluteFluxColorStops([{ value: -1, color: '#ffffff' }]), /zero or more/);
  assert.equal(normalizeAbsoluteFluxColorStops(null), null);
});

test('a log scale spreads the four decades below the top point, so one large flux does not wash out the rest', () => {
  const linear = new FluxScale([10, 1, 0.1, 0.01]);
  assert.equal(linear.absoluteColor(1), '#fce9e9', 'linear: a tenth of the top is nearly white');
  const log = new FluxScale([10, 1, 0.1, 0.01], { absoluteLog: true });
  assert.deepEqual([10, 1, 0.1, 0.01, 0.001, 0.0001, 0].map((v) => log.absoluteColor(v)),
    ['#dc2626', '#e55c5c', '#ee9393', '#f6c9c9', '#ffffff', '#ffffff', '#ffffff']);
  assert.deepEqual(log.absoluteLegendStops().map(({ t, value, color }) => ({ t, value, color })), [
    { t: 0, value: 0.001, color: '#ffffff' },
    { t: 1, value: 10, color: '#dc2626' },
  ]);
  const wide = new FluxScale([1000, 1], { absoluteLog: true });
  assert.deepEqual([1, 10, 100].map((v) => wide.absoluteColor(v)), ['#f6c9c9', '#ee9393', '#e55c5c']);
});

test('the log floor reaches below the smallest positive point, and needs one', () => {
  assert.equal(absoluteLogFloor([{ value: 0 }, { value: 1000 }]), 0.1);
  assert.equal(absoluteLogFloor([{ value: 0 }, { value: 0.001 }, { value: 1000 }]), 0.0001);
  assert.equal(absoluteLogFloor([{ value: 0 }]), null);
  const flat = new FluxScale([5], { absoluteStops: [{ value: 0, color: '#123456' }], absoluteLog: true });
  assert.equal(flat.absoluteColor(5), '#123456', 'no positive point: log falls back to the points as they are');
});

test('the store keeps the absolute palette and log scale, and Scenario and Map files carry them', async () => {
  const store = await makeStore();
  store.setView({ reactionColorMode: 'absflux', absoluteFluxColorStops: blue, absoluteFluxLog: true });
  assert.equal(store.scale.absoluteColor(10), '#0000ff');
  assert.equal(store.scale.absoluteLog, true);
  assert.throws(() => store.setView({ absoluteFluxColorStops: [{ value: -1, color: '#ffffff' }] }), /zero or more/);
  assert.deepEqual(store.view.absoluteFluxColorStops, blue, 'a refused edit keeps the last palette');

  const scenario = JSON.parse(store.stateJSON());
  const map = JSON.parse(store.mapJSON());
  assert.deepEqual(scenario.absoluteFluxColors, { stops: blue, log: true });
  assert.deepEqual(map.absoluteFluxColors, { stops: blue, log: true });

  const restored = await makeStore();
  await restored.loadState(JSON.stringify(scenario));
  assert.deepEqual(restored.absoluteFluxColors(), { stops: blue, log: true });
  assert.equal(restored.scale.absoluteLog, true);
  delete scenario.absoluteFluxColors;
  await restored.loadState(JSON.stringify(scenario));
  assert.deepEqual(restored.absoluteFluxColors(), { stops: null, log: false }, 'older files get the default');

  restored.loadMapJSON(JSON.stringify(map));
  assert.deepEqual(restored.absoluteFluxColors(), { stops: blue, log: true });
  restored.loadMapJSON(JSON.stringify(map.map));
  assert.deepEqual(restored.absoluteFluxColors(), { stops: null, log: false }, 'raw legacy maps get the default');
});

test('the palette card shows the absolute points, the scale switch and a reset', async () => {
  const store = await makeStore();
  store.setView({ reactionColorMode: 'absflux' });
  let card = renderWorkspace(store, 'map');
  assert.match(card, /data-action="absolute-flux-scale" data-scale="linear" type="button" aria-pressed="true"/);
  assert.match(card, /data-action="flux-color-value" data-flux-color-index="1"/);
  assert.match(card, /value="10" data-action="flux-color-value"/, 'unedited points follow the largest |flux|');
  assert.match(card, /data-action="absolute-flux-reset" type="button" title="[^"]*" disabled>/);

  store.setView({ absoluteFluxColorStops: blue, absoluteFluxLog: true });
  card = renderWorkspace(store, 'map');
  assert.match(card, /data-action="absolute-flux-scale" data-scale="log" type="button" aria-pressed="true"/);
  assert.match(card, /data-action="absolute-flux-reset" type="button" title="[^"]*">/);
  assert.match(card, /value="#0000ff" data-action="flux-color-color"/);
  assert.match(card, /\|flux\|, log mmol/);
});

test('opening a Scenario file keeps its palettes even when the scenario redraws a curated map', async (t) => {
  const files = new Map();
  const previousFetch = globalThis.fetch;
  globalThis.fetch = async (url) => {
    const text = files.get(String(url));
    return text === undefined ? { ok: false, status: 404 } : { ok: true, status: 200, json: async () => JSON.parse(text) };
  };
  t.after(() => { globalThis.fetch = previousFetch; });

  const store = await makeStore();
  // The curated map carries the default palettes, as the shipped ones do.
  files.set('./models/maps/demo.json', JSON.stringify(createMapDocument(store.map, { species: store.species, scenario: 'demo' })));
  store.scenarios[0].map = 'maps/demo.json';
  await store.useScenario('demo');
  assert.equal(store.scenarioMapFile, 'maps/demo.json');
  const signed = [{ value: -1, color: '#000000' }, { value: 1, color: '#ff0000' }];
  store.setView({ reactionColorMode: 'absflux', fluxColorStops: signed, absoluteFluxColorStops: blue, absoluteFluxLog: true });
  const saved = store.stateJSON();

  await store.useScenario('demo');
  assert.deepEqual(store.absoluteFluxColors(), { stops: null, log: false });
  await store.loadState(saved);
  assert.deepEqual(store.view.fluxColorStops, signed);
  assert.deepEqual(store.absoluteFluxColors(), { stops: blue, log: true });
  assert.equal(store.scale.absoluteLog, true);
  assert.equal(store.scenarioMapFile, 'maps/demo.json');
});
