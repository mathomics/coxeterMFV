import test from 'node:test';
import assert from 'node:assert/strict';
import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import { EditableMap } from '../src/map/editable-map.js';
import {
  DEFAULT_METABOLITE_LABEL_ANGLE,
  metaboliteLabelAngle,
  metaboliteLabelPosition,
} from '../src/map/metabolite-label.js';
import { FluxProblem } from '../src/solver/fba.js';
import { Store } from '../src/ui/store.js';
import { OPERATION_DOMAIN, operationDomain, isMapOperation } from '../src/ui/operation-boundary.js';
import { renderMap } from '../src/render/mapview.js';
import { FluxScale } from '../src/render/theme.js';

const PI = Math.PI;

function near(actual, expected, message = '') {
  assert.ok(Math.abs(actual - expected) < 1e-10, `${message}: ${actual} != ${expected}`);
}

function visualMap() {
  const nodes = [
    { id: 'primary', kind: 'metabolite', primary: true, mergedId: 'eco@glc_c', base: 'PrimaryLabel' },
    { id: 'secondary', kind: 'metabolite', primary: false, mergedId: 'eco@p_c', base: 'SecondaryLabel' },
    { id: 'medium-primary', kind: 'medium', primary: true, mergedId: 'M@glc_s', base: 'MediumLabel' },
    { id: 'medium-secondary', kind: 'medium', primary: false, mergedId: 'M@p_s', base: 'SmallMediumLabel' },
    { id: 'currency', kind: 'currency', mergedId: 'eco@atp_c', base: 'CurrencyLabel' },
    { id: 'hidden', kind: 'metabolite', mergedId: 'eco@hidden_c', base: 'HiddenLabel', hidden: true },
    { id: 'furniture', kind: 'marker', base: 'Furniture', x: 75, y: 75 },
  ].map((node, i) => ({ x: 100 + i * 50, y: 120, compartment: 'c', ...node }));
  return new EditableMap({
    canvas: { width: 700, height: 300 },
    environment: { id: 's', hidden: true, rect: { x: 0, y: 0, w: 700, h: 300 } },
    nodes: new Map(nodes.map((node) => [node.id, node])),
    cells: [],
    reactions: new Map(),
    segments: [],
  });
}

function biologyFixture() {
  const model = new Model({
    id: 'label-toy',
    compartments: { e: 'Extracellular', c: 'Cytosol' },
    metabolites: [
      { id: 'glc_e', name: 'Glucose', compartment: 'e' },
      { id: 'glc_c', name: 'Glucose', compartment: 'c' },
      { id: 'p_c', name: 'Product', compartment: 'c' },
    ],
    reactions: [
      { id: 'EX_glc', metabolites: { glc_e: -1 }, lower_bound: -10, upper_bound: 1000 },
      { id: 'T_glc', metabolites: { glc_e: -1, glc_c: 1 }, lower_bound: 0, upper_bound: 1000 },
      { id: 'MAKE_p', metabolites: { glc_c: -1, p_c: 1 }, lower_bound: 0, upper_bound: 1000 },
      { id: 'BIOMASS', metabolites: { p_c: -1 }, lower_bound: 0, upper_bound: 1000, objective_coefficient: 1 },
    ],
  });
  const community = new CommunityModel({
    species: [{ id: 'eco', model, abundance: 1 }],
    medium: { glc: { lower: -10 } },
  });
  const problem = new FluxProblem(community.merged);
  const map = EditableMap.fromCommunity(community);
  return { model, community, problem, map };
}

function biologySnapshot({ model, community, problem }) {
  return structuredClone({
    model: model.toJSON(),
    merged: community.merged.toJSON(),
    reactionOwner: community.reactionOwner,
    metaboliteOwner: community.metaboliteOwner,
    metIds: problem.metIds,
    rxnIds: problem.rxnIds,
    matrix: problem.A,
    lb: problem.lb,
    ub: problem.ub,
    objective: problem.c,
    rhs: problem.b,
  });
}

function withoutAngles(doc) {
  const copy = structuredClone(doc);
  for (const node of copy.nodes) delete node.labelAngle;
  return copy;
}

test('legacy label coordinates remain exactly above every supported metabolite role', () => {
  const map = visualMap();
  assert.equal(DEFAULT_METABOLITE_LABEL_ANGLE, -PI / 2);
  for (const id of ['primary', 'secondary', 'medium-primary', 'medium-secondary', 'currency']) {
    const node = map.nodes.get(id);
    const gap = node.primary === false ? 3.5 : 5;
    assert.equal(metaboliteLabelAngle(node), -PI / 2);
    assert.deepEqual(metaboliteLabelPosition(node), {
      x: node.x, y: node.y - node.r - gap, anchor: 'middle', baseline: 'auto',
    }, id);
    assert.equal(Object.hasOwn(node, 'labelAngle'), false);
  }
});

test('labels orbit at the role radius and align outward in each quadrant', () => {
  const map = visualMap();
  const directions = [
    [0, 'start', 'middle'],
    [PI / 2, 'middle', 'hanging'],
    [PI, 'end', 'middle'],
    [-PI / 2, 'middle', 'auto'],
    [PI / 4, 'start', 'hanging'],
    [3 * PI / 4, 'end', 'hanging'],
    [-3 * PI / 4, 'end', 'auto'],
    [-PI / 4, 'start', 'auto'],
  ];
  for (const id of ['primary', 'secondary', 'medium-primary', 'medium-secondary', 'currency']) {
    const node = map.nodes.get(id);
    const distance = node.r + (node.primary === false ? 3.5 : 5);
    for (const [angle, anchor, baseline] of directions) {
      map.setMetaboliteLabelAngle(id, angle);
      const position = metaboliteLabelPosition(node);
      near(position.x, node.x + distance * Math.cos(angle), `${id}: x`);
      near(position.y, node.y + distance * Math.sin(angle), `${id}: y`);
      near(Math.hypot(position.x - node.x, position.y - node.y), distance, `${id}: orbit`);
      assert.equal(position.anchor, anchor, `${id}: anchor at ${angle}`);
      assert.equal(position.baseline, baseline, `${id}: baseline at ${angle}`);
    }
  }
  const vertical = metaboliteLabelPosition({ x: 0, y: 0, labelAngle: Math.acos(0.2) });
  assert.equal(vertical.anchor, 'middle');
  assert.equal(vertical.baseline, 'hanging');
  const horizontal = metaboliteLabelPosition({ x: 0, y: 0, labelAngle: Math.asin(-0.2) });
  assert.equal(horizontal.anchor, 'start');
  assert.equal(horizontal.baseline, 'middle');
});

test('angle reads normalize complete turns and reject nonnumeric or nonfinite state', () => {
  for (const [angle, expected] of [
    [PI, -PI], [5 * PI, -PI], [-5 * PI, -PI],
    [3 * PI / 2, -PI / 2], [-3 * PI / 2, PI / 2],
    [2 * PI, 0], [-2 * PI, 0], [0.25, 0.25],
  ]) {
    const node = { labelAngle: angle };
    near(metaboliteLabelAngle(node), expected, `angle ${angle}`);
    assert.equal(node.labelAngle, angle, 'reading must not mutate the node');
  }
  for (const angle of [NaN, Infinity, -Infinity, '0', '', null, undefined, true, {}, new Number(0)]) {
    assert.equal(metaboliteLabelAngle({ labelAngle: angle }), DEFAULT_METABOLITE_LABEL_ANGLE);
  }
  assert.equal(metaboliteLabelAngle(null), DEFAULT_METABOLITE_LABEL_ANGLE);
});

test('setting an eligible label angle changes only its angle, including hidden nodes', () => {
  const map = visualMap();
  const before = withoutAngles(map.toJSON());
  for (const id of ['primary', 'secondary', 'medium-primary', 'medium-secondary', 'currency', 'hidden']) {
    const node = map.nodes.get(id);
    assert.equal(map.setMetaboliteLabelAngle(id, 9 * PI / 2), node);
    near(node.labelAngle, PI / 2);
  }
  assert.deepEqual(withoutAngles(map.toJSON()), before);
  assert.equal(map.nodes.get('hidden').hidden, true);
});

test('invalid angle input and nonmetabolite ids are no-ops', () => {
  const map = visualMap();
  map.setMetaboliteLabelAngle('primary', 0.7);
  const before = map.toJSON();
  for (const angle of [NaN, Infinity, -Infinity, '1', '', null, undefined, false, [], new Number(1)]) {
    assert.equal(map.setMetaboliteLabelAngle('primary', angle), null);
  }
  assert.equal(map.setMetaboliteLabelAngle('missing', 0), null);
  assert.equal(map.setMetaboliteLabelAngle('furniture', 0), null);
  assert.deepEqual(map.toJSON(), before);
});

test('duplicate instances of one biological metabolite keep independent label angles', () => {
  const map = visualMap();
  const original = map.nodes.get('primary');
  const copy = map.duplicateMetaboliteNode(original.id, { x: original.x + 45, y: original.y + 30 });
  assert.ok(copy);
  assert.equal(copy.mergedId, original.mergedId);
  const positions = [original, copy].map(({ x, y }) => ({ x, y }));
  map.setMetaboliteLabelAngle(original.id, 0);
  map.setMetaboliteLabelAngle(copy.id, PI / 2);
  assert.equal(original.labelAngle, 0);
  assert.equal(copy.labelAngle, PI / 2);
  assert.deepEqual([original, copy].map(({ x, y }) => ({ x, y })), positions);
  assert.equal(metaboliteLabelPosition(original).anchor, 'start');
  assert.equal(metaboliteLabelPosition(copy).baseline, 'hanging');
});

test('moving a label leaves geometry, reaction membership, biological models and LP untouched', () => {
  const fixture = biologyFixture();
  const { map, community, problem } = fixture;
  const beforeBiology = biologySnapshot(fixture);
  const beforeMap = withoutAngles(map.toJSON());
  const node = [...map.nodes.values()].find((item) => item.kind === 'metabolite');
  assert.ok(node);
  assert.ok(map.segmentsAt(node.id).length > 0, 'fixture must include attached reaction geometry');
  for (const angle of [0, PI / 4, PI / 2, PI]) map.setMetaboliteLabelAngle(node.id, angle);
  assert.deepEqual(withoutAngles(map.toJSON()), beforeMap);
  assert.deepEqual(biologySnapshot(fixture), beforeBiology);
  assert.equal(problem.model, community.merged);
});

test('angles persist independently through detached map JSON save and load', () => {
  const map = visualMap();
  map.setMetaboliteLabelAngle('primary', -0.8);
  map.setMetaboliteLabelAngle('medium-secondary', 1.4);
  map.setMetaboliteLabelAngle('currency', 0);
  const saved = map.toJSON();
  map.setMetaboliteLabelAngle('primary', 2);
  const restored = EditableMap.fromJSON(JSON.parse(JSON.stringify(saved)));
  assert.equal(restored.nodes.get('primary').labelAngle, -0.8);
  assert.equal(restored.nodes.get('medium-secondary').labelAngle, 1.4);
  assert.equal(restored.nodes.get('currency').labelAngle, 0);
  assert.equal(Object.hasOwn(restored.nodes.get('secondary'), 'labelAngle'), false);
  restored.setMetaboliteLabelAngle('primary', -2);
  assert.equal(map.nodes.get('primary').labelAngle, 2);
  assert.equal(saved.nodes.find(({ id }) => id === 'primary').labelAngle, -0.8);
});

test('legacy and invalid saved angles load with the original top placement', () => {
  const original = visualMap().toJSON();
  for (const invalid of [undefined, null, '1', false, NaN, Infinity, {}, []]) {
    const doc = structuredClone(original);
    const node = doc.nodes.find(({ id }) => id === 'primary');
    if (invalid !== undefined) node.labelAngle = invalid;
    const restored = EditableMap.fromJSON(doc).nodes.get('primary');
    assert.equal(Object.hasOwn(restored, 'labelAngle'), false);
    assert.equal(metaboliteLabelAngle(restored), DEFAULT_METABOLITE_LABEL_ANGLE);
    assert.deepEqual(metaboliteLabelPosition(restored), {
      x: restored.x, y: restored.y - 18, anchor: 'middle', baseline: 'auto',
    });
  }
  const doc = structuredClone(original);
  doc.nodes.find(({ id }) => id === 'primary').labelAngle = 5 * PI;
  const restored = EditableMap.fromJSON(doc).nodes.get('primary');
  assert.equal(restored.labelAngle, -PI);
});

test('Store delegates angle updates without emitting, solving, rebuilding or recording history', () => {
  const fixture = biologyFixture();
  const store = new Store('.');
  Object.assign(store, { map: fixture.map, community: fixture.community, problem: fixture.problem });
  store.solution = { status: 'optimal', fluxes: new Map([['eco@BIOMASS', 10]]) };
  const solution = store.solution;
  const beforeBiology = biologySnapshot(fixture);
  const calls = [];
  for (const name of ['emit', 'solve', 'rebuild', 'relayoutMap', 'beginMapEdit', 'commitMapEdit']) {
    store[name] = (...args) => { calls.push([name, args]); };
  }
  store.mapHistory.record = (...args) => { calls.push(['history', args]); };
  const node = [...store.map.nodes.values()].find((item) => item.kind === 'metabolite');
  const delegate = store.map.setMetaboliteLabelAngle.bind(store.map);
  const delegated = [];
  store.map.setMetaboliteLabelAngle = (...args) => {
    delegated.push(args);
    return delegate(...args);
  };
  assert.equal(store.setMetaboliteLabelAngle(node.id, 0.9), node);
  assert.equal(store.setMetaboliteLabelAngle(node.id, NaN), null);
  assert.equal(node.labelAngle, 0.9);
  assert.deepEqual(delegated, [[node.id, 0.9], [node.id, NaN]]);
  assert.deepEqual(calls, []);
  assert.equal(store.solution, solution);
  assert.equal(store.problem, fixture.problem);
  assert.equal(store.community, fixture.community);
  assert.deepEqual(biologySnapshot(fixture), beforeBiology);
  assert.deepEqual(store.mapHistory.undoStack, []);
  store.map = null;
  assert.equal(store.setMetaboliteLabelAngle(node.id, 0), null);
  assert.deepEqual(calls, []);
});

test('a sequence of label updates forms one undoable map gesture when committed', () => {
  const store = new Store('.');
  store.map = visualMap();
  const nodeId = 'primary';
  const transaction = store.beginMapEdit('Move metabolite label');
  for (const angle of [0.2, 0.8, 1.5]) store.setMetaboliteLabelAngle(nodeId, angle);
  assert.equal(store.mapHistory.undoStack.length, 0);
  assert.equal(store.commitMapEdit(transaction), true);
  assert.equal(store.mapHistory.undoStack.length, 1);
  assert.equal(store.undoMap().label, 'Move metabolite label');
  assert.equal(metaboliteLabelAngle(store.map.nodes.get(nodeId)), DEFAULT_METABOLITE_LABEL_ANGLE);
  assert.equal(store.redoMap().label, 'Move metabolite label');
  assert.equal(store.map.nodes.get(nodeId).labelAngle, 1.5);
});

test('label angle editing is explicitly registered as a MAP operation', () => {
  assert.equal(operationDomain('setMetaboliteLabelAngle'), OPERATION_DOMAIN.MAP);
  assert.equal(isMapOperation('setMetaboliteLabelAngle'), true);
  const store = new Store('.');
  assert.equal(store.editingOperationDomain('setMetaboliteLabelAngle'), OPERATION_DOMAIN.MAP);
  assert.equal(store.assertMapOperation('setMetaboliteLabelAngle'), true);
});

test('SVG renders orbit positions and outward alignment without rotating label text', () => {
  const map = visualMap();
  const cases = [
    ['primary', 0, 'start', 'middle'],
    ['secondary', PI, 'end', 'middle'],
    ['medium-primary', PI / 2, 'middle', 'hanging'],
    ['medium-secondary', -PI / 2, 'middle', 'auto'],
    ['currency', PI / 4, 'start', 'hanging'],
  ];
  for (const [id, angle] of cases) map.setMetaboliteLabelAngle(id, angle);
  const svg = renderMap(map, {
    fluxes: new Map(), scale: new FluxScale([]), labels: 'all',
    showCurrency: true, currencyLabels: true, showCrossFeeding: false,
  });
  for (const [id, , anchor, baseline] of cases) {
    const node = map.nodes.get(id);
    const text = [...svg.matchAll(/<text\b([^>]*)>([^<]*)<\/text>/g)]
      .find((match) => match[2] === node.base);
    assert.ok(text, `${id}: missing metabolite label`);
    const attributes = Object.fromEntries([...text[1].matchAll(/([\w-]+)="([^"]*)"/g)]
      .map((match) => [match[1], match[2]]));
    const position = metaboliteLabelPosition(node);
    assert.ok(Math.abs(Number(attributes.x) - position.x) <= 0.005, `${id}: rendered x`);
    assert.ok(Math.abs(Number(attributes.y) - position.y) <= 0.005, `${id}: rendered y`);
    assert.equal(attributes['text-anchor'], anchor, `${id}: rendered anchor`);
    assert.equal(attributes['dominant-baseline'] ?? 'auto', baseline, `${id}: rendered baseline`);
    assert.equal(attributes.transform, undefined, `${id}: text remains horizontal`);
    assert.equal(attributes.rotate, undefined, `${id}: text remains horizontal`);
  }
  assert.equal(svg.includes('>HiddenLabel</text>'), false);
});
