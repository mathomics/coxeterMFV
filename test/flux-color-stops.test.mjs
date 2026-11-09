import test from 'node:test';
import assert from 'node:assert/strict';
import { FluxScale, normalizeFluxColorStops } from '../src/render/theme.js';
import { Store } from '../src/ui/store.js';
import { Model } from '../src/core/model.js';

const stops = [
  { value: -20, color: '#ff0000' },
  { value: 0, color: '#ffffff' },
  { value: 5, color: '#0000ff' },
];

async function makeStore() {
  const store = new Store('.');
  store.models.set('palette-demo', new Model({
    id: 'palette-demo',
    compartments: { c: 'Cytosol', e: 'Extracellular' },
    metabolites: [
      { id: 'a_e', compartment: 'e' },
      { id: 'a_c', compartment: 'c' },
    ],
    reactions: [
      { id: 'EX_a_e', metabolites: { a_e: -1 }, lower_bound: -10, upper_bound: 1000 },
      { id: 'At', metabolites: { a_e: -1, a_c: 1 }, lower_bound: 0, upper_bound: 1000 },
      { id: 'BIOMASS', metabolites: { a_c: -1 }, lower_bound: 0, upper_bound: 1000, objective_coefficient: 1 },
    ],
  }));
  store.scenarios = [{
    id: 'palette-demo', name: 'Palette demo', objective: 'equal-growth',
    species: [{ id: 'demo', model: 'palette-demo', abundance: 1 }],
    medium: { a: 10 },
  }];
  await store.useScenario('palette-demo');
  assert.equal(store.solution.status, 'optimal');
  return store;
}

test('custom flux stops are sorted and owned copies, with no arbitrary point limit', () => {
  const input = [stops[2], stops[0], stops[1]].map(stop => ({ ...stop }));
  const original = structuredClone(input);
  const normalized = normalizeFluxColorStops(input);
  assert.deepEqual(normalized, stops);
  assert.deepEqual(input, original, 'normalizing must not reorder caller input');
  input[0].color = '#123456';
  assert.deepEqual(normalized, stops, 'normalized stops must not alias caller objects');
  const many = Array.from({ length: 50 }, (_, index) => ({ value: index - 25, color: '#123456' }));
  assert.equal(normalizeFluxColorStops(many).length, many.length);
  assert.equal(normalizeFluxColorStops(null), null);
});

test('invalid flux stops are rejected instead of becoming a misleading palette', () => {
  const invalid = [
    [],
    { value: 0, color: '#ffffff' },
    [{ value: 0, color: '#ffffff' }, { value: 0, color: '#000000' }],
    [{ value: NaN, color: '#ffffff' }],
    [{ value: Infinity, color: '#ffffff' }],
    [{ value: -Infinity, color: '#ffffff' }],
    [{ value: 'not a number', color: '#ffffff' }],
    [{ value: 0, color: '#fff' }],
    [{ value: 0, color: '#gg0000' }],
    [{ value: 0, color: 'red' }],
  ];
  for (const input of invalid) assert.throws(() => normalizeFluxColorStops(input), Error);
});

test('negative, zero and positive fluxes use their exact signed color anchors', () => {
  const scale = new FluxScale([-10000, 10000], { colorStops: stops });
  for (const stop of stops) assert.equal(scale.color(stop.value), stop.color);
  assert.notEqual(scale.color(-5), scale.color(5), 'negative flux must not become its absolute value');
  assert.deepEqual(scale.legendStops().map(({ value, color }) => ({ value, color })), stops);
});

test('RGB interpolation uses real flux distances rather than log or normalized magnitude', () => {
  const palette = [
    { value: -100, color: '#000000' },
    { value: 0, color: '#ff0000' },
    { value: 10, color: '#ffffff' },
  ];
  const scale = new FluxScale([-10000, 1, 10000], { colorStops: palette });
  assert.equal(scale.color(-25), '#bf0000', '-25 is three quarters through [-100, 0]');
  assert.equal(scale.color(5), '#ff8080', '5 is halfway through [0, 10]');
  const differentDataset = new FluxScale([0.0001, 0.001], { colorStops: palette });
  assert.equal(differentDataset.color(-25), scale.color(-25));
  assert.equal(differentDataset.color(5), scale.color(5));
});

test('custom flux colors clamp outside anchors and one anchor gives a constant color', () => {
  const scale = new FluxScale([1], { colorStops: stops });
  assert.equal(scale.color(-1e9), stops[0].color);
  assert.equal(scale.color(1e9), stops.at(-1).color);
  const constant = new FluxScale([-10, 0, 10], { colorStops: [{ value: -3, color: '#123456' }] });
  for (const value of [-1e9, -3, 0, 1e9]) assert.equal(constant.color(value), '#123456');
});

test('automatic colors keep their existing behavior and custom stops never change flux widths', () => {
  const values = [-20, 0, 5, 100];
  const automatic = new FluxScale(values);
  const explicitAutomatic = new FluxScale(values, { colorStops: null });
  const custom = new FluxScale(values, { colorStops: stops });
  assert.equal(automatic.color(0), '#cbd5e1');
  assert.equal(new FluxScale(values, { dark: true }).color(0), '#475569');
  assert.equal(automatic.color(20), automatic.color(-20));
  assert.equal(automatic.color(automatic.reference), '#be123c');
  for (const value of [-1000, -20, 0, 5, 1000]) {
    assert.equal(explicitAutomatic.color(value), automatic.color(value));
    assert.equal(custom.width(value), automatic.width(value));
  }
});

test('changing palette is presentation only and invalid edits preserve the previous palette', async () => {
  const store = await makeStore();
  assert.equal(store.view.fluxColorStops, null);
  const before = {
    community: store.community, problem: store.problem, solution: store.solution, map: store.map,
    bounds: store.problem.snapshotBounds(), fluxes: [...store.solution.fluxes],
  };
  store.setFluxColorStops(stops);
  assert.deepEqual(store.view.fluxColorStops, stops);
  assert.equal(store.scale.color(0), '#ffffff');
  assert.equal(store.community, before.community);
  assert.equal(store.problem, before.problem);
  assert.equal(store.solution, before.solution);
  assert.equal(store.map, before.map);
  assert.deepEqual(store.problem.snapshotBounds(), before.bounds);
  assert.deepEqual([...store.solution.fluxes], before.fluxes);
  assert.throws(() => store.setFluxColorStops([{ value: 1, color: '#fff' }]), Error);
  assert.deepEqual(store.view.fluxColorStops, stops);
  store.solve();
  assert.equal(store.scale.color(5), '#0000ff', 'solving must keep the chosen presentation palette');
  store.setView({ dark: true });
  assert.equal(store.scale.color(5), '#0000ff', 'changing theme must keep the chosen palette');
  store.setFluxColorStops(null);
  assert.equal(store.view.fluxColorStops, null);
  assert.equal(store.scale.color(0), '#475569');
});

test('scenario export and import preserve custom stops; older scenarios restore automatic colors', async () => {
  const source = await makeStore();
  source.setFluxColorStops(stops);
  const doc = JSON.parse(source.stateJSON());
  assert.deepEqual(doc.fluxColorStops, stops);
  const restored = await makeStore();
  await restored.loadState(JSON.stringify(doc));
  assert.deepEqual(restored.view.fluxColorStops, stops);
  assert.equal(restored.scale.color(-20), '#ff0000');
  assert.equal(restored.scale.color(5), '#0000ff');
  delete doc.fluxColorStops;
  await restored.loadState(JSON.stringify(doc));
  assert.equal(restored.view.fluxColorStops, null, 'old scenarios must not inherit an unrelated custom palette');
  assert.equal(restored.scale.color(0), '#cbd5e1');
});

test('map export and import preserve custom stops; older wrapped and raw maps restore automatic colors', async () => {
  const source = await makeStore();
  source.setFluxColorStops(stops);
  const doc = JSON.parse(source.mapJSON());
  assert.deepEqual(doc.fluxColorStops, stops);
  const restored = await makeStore();
  const solution = restored.solution;
  restored.loadMapJSON(JSON.stringify(doc));
  assert.deepEqual(restored.view.fluxColorStops, stops);
  assert.equal(restored.scale.color(0), '#ffffff');
  assert.equal(restored.solution, solution, 'loading map presentation must not run FBA');
  delete doc.fluxColorStops;
  restored.loadMapJSON(JSON.stringify(doc));
  assert.equal(restored.view.fluxColorStops, null);
  assert.equal(restored.scale.color(0), '#cbd5e1');
  restored.setFluxColorStops(stops);
  restored.loadMapJSON(JSON.stringify(doc.map));
  assert.equal(restored.view.fluxColorStops, null, 'legacy raw EditableMap JSON defaults to automatic colors');
});
