import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import factory from '../assets/vendor/glpk.js';
import { initGLPK } from '../src/solver/glpk.js';
import { Model } from '../src/core/model.js';
import { Store } from '../src/ui/store.js';

test.before(async () => {
  globalThis.__GLPK_FACTORY__ = factory;
  globalThis.__GLPK_WASM__ = await readFile(new URL('../assets/vendor/glpk.wasm', import.meta.url));
  assert.equal(await initGLPK(), true, 'Store integration requires the shipped GLPK backend');
});

function member(id, makes, needs) {
  return new Model({
    id, compartments: { e: 'Extracellular', c: 'Cytosol' },
    metabolites: ['glc', makes, needs].flatMap(base => ['e', 'c'].map(compartment => ({ id: `${base}_${compartment}`, compartment }))),
    reactions: [
      { id: 'EX_glc_e', metabolites: { glc_e: -1 }, lower_bound: -10, upper_bound: 1000 },
      { id: 'GLCt', metabolites: { glc_e: -1, glc_c: 1 }, lower_bound: 0, upper_bound: 100 },
      { id: 'BIOMASS', metabolites: { glc_c: -1, [`${needs}_c`]: -1 }, lower_bound: 0, upper_bound: 100 },
      { id: `MK_${makes}`, metabolites: { glc_c: -1, [`${makes}_c`]: 1 }, lower_bound: 0, upper_bound: 100 },
      { id: `T_${makes}`, metabolites: { [`${makes}_c`]: -1, [`${makes}_e`]: 1 }, lower_bound: 0, upper_bound: 100 },
      { id: `EX_${makes}_e`, metabolites: { [`${makes}_e`]: -1 }, lower_bound: 0, upper_bound: 1000 },
      { id: `T_${needs}`, metabolites: { [`${needs}_e`]: -1, [`${needs}_c`]: 1 }, lower_bound: 0, upper_bound: 100 },
      { id: `EX_${needs}_e`, metabolites: { [`${needs}_e`]: -1 }, lower_bound: -100, upper_bound: 1000 },
    ], objective: { BIOMASS: 1 },
  });
}

async function pairStore() {
  const store = new Store('.');
  store.models.set('member-A', member('A', 'x', 'y'));
  store.models.set('member-B', member('B', 'y', 'x'));
  const species = [{ id: 'A', model: 'member-A', abundance: 0.5 }, { id: 'B', model: 'member-B', abundance: 0.5 }];
  store.scenarios = [
    { id: 'mutualists', name: 'Mutualists', objective: 'equal-growth', medium: { glc: 10 }, species },
    { id: 'lean-mutualists', name: 'Lean mutualists', objective: 'equal-growth', medium: { glc: 2 }, species },
  ];
  await store.useScenario('mutualists');
  return store;
}

function assertFixedSolution(store, mu) {
  assert.equal(store.solution.status, 'optimal');
  assert.equal(store.solution.steadyCom.mu, mu);
  assert.equal(store.solution.objectiveValue, mu);
  assert.equal(store.steadyComState.key, store.steadyComKey());
  assert.ok(Math.abs([...store.solution.steadyCom.abundances.values()].reduce((sum, value) => sum + value, 0) - 1) < 1e-7);
  for (const id of ['A', 'B']) {
    assert.ok(store.solution.steadyCom.abundances.get(id) > 0, `${id} must be present in this obligate mutualism`);
    assert.ok(Math.abs(store.solution.fluxes.get(`${id}@BIOMASS`) - mu) < 1e-7, `${id} specific biomass flux equals requested μ`);
  }
}

function deferredTask(store) {
  let resolveTask, ready;
  const called = new Promise(resolve => { ready = resolve; });
  store.runSteadyComTask = (spec, request, controls) => new Promise(resolve => {
    resolveTask = resolve;
    ready({ spec, request, controls });
  });
  return { called, resolve: result => resolveTask(result) };
}

const fakeRange = () => ({ status: 'optimal', zeroFeasible: true, rangeLower: 0, rangeUpper: 99, upperBound: 100, resolution: 0.001, iterations: 1 });

test('community export bounds constrain the separate-member SteadyCom interval', async () => {
  const store = new Store('.');
  store.models.set('waste-maker', new Model({ id: 'A', compartments: { e: 'Extracellular' },
    metabolites: [{ id: 'a_e', compartment: 'e' }, { id: 'w_e', compartment: 'e' }],
    reactions: [
      { id: 'EX_a_e', metabolites: { a_e: -1 }, lower_bound: -10, upper_bound: 100 },
      { id: 'BIOMASS', metabolites: { a_e: -1, w_e: 1 }, lower_bound: 0, upper_bound: 100 },
      { id: 'EX_w_e', metabolites: { w_e: -1 }, lower_bound: 0, upper_bound: 100 },
    ], objective: { BIOMASS: 1 } }));
  store.scenarios = [{ id: 'waste', name: 'Waste maker', species: [{ id: 'A', model: 'waste-maker', abundance: 1 }], medium: { a: 10 } }];
  await store.useScenario('waste');
  const initial = await store.steadyComRange();
  assert.ok(Math.abs(initial.max - 10) < 1e-4);
  store.setReactionBounds('EX_w_s', 0, 2);
  const limited = await store.steadyComRange();
  assert.ok(Math.abs(limited.max - 2) < 1e-4, `export cap must limit growth to 2, got ${limited.max}`);
  const problem = await store.steadyComProblem();
  assert.deepEqual(problem.communityExchangeBounds.w, { lower: 0, upper: 2 });
  await store.solveSteadyComAt(1);
  assert.ok(Math.abs(store.solution.fluxes.get('A@EX_w_e') - 1) < 1e-7);
});

test('Store computes and caches a real permitted interval without rewriting member bounds', async () => {
  const store = await pairStore();
  const documents = [...store.models].map(([id, model]) => [id, model.toJSON()]);
  const bounds = store.problem.snapshotBounds();
  const range = await store.steadyComRange();
  assert.equal(range.source, 'computed');
  assert.equal(range.min, 0);
  assert.ok(Math.abs(range.max - 5) < 2e-5, `two glucose costs per biomass give μ≤5, received ${range.max}`);
  assert.ok(range.iterations > 1);
  assert.equal(range.phase, null);
  assert.deepEqual(store.problem.snapshotBounds(), bounds);
  assert.deepEqual([...store.models].map(([id, model]) => [id, model.toJSON()]), documents);
  store.runSteadyComTask = () => { throw new Error('Cached range must not dispatch another task'); };
  assert.equal(await store.steadyComRange(), range);
});

test('the first fixed-μ solve computes its range before returning usable state and population-specific fluxes', async () => {
  const store = await pairStore();
  const task = store.runSteadyComTask;
  const requests = [];
  store.runSteadyComTask = (...args) => { requests.push(args[1].type); return task(...args); };
  const before = store.problem.snapshotBounds();
  await store.solveSteadyComAt(2);
  assert.deepEqual(requests, ['range', 'fixed']);
  assert.ok(Number.isFinite(store.steadyComState.max) && store.steadyComState.max > 2);
  assert.equal(store.steadyComState.phase, null);
  assertFixedSolution(store, 2);
  assert.deepEqual(store.problem.snapshotBounds(), before);
  assert.equal(store.parsimonious, false);
  assert.equal(store.loopless, false);
});

test('invalid and out-of-range μ never replace the last valid fixed solution', async () => {
  const store = await pairStore();
  await store.solveSteadyComAt(2);
  const solution = store.solution;
  for (const mu of [NaN, Infinity, -1, '', '2', store.steadyComState.max + 1]) {
    await assert.rejects(store.solveSteadyComAt(mu), /finite growth rate|computed range/);
    assert.equal(store.solution, solution);
  }
  assertFixedSolution(store, 2);
});

test('medium and member-bound changes invalidate the previous interval and are included in the new solve', async () => {
  const store = await pairStore();
  const initial = await store.steadyComRange();
  store.setMediumLimit('glc', 4);
  assert.equal(store.steadyComState, null);
  assert.equal(store._steadyComSpec, null);
  const lean = await store.steadyComRange();
  assert.notEqual(lean.key, initial.key);
  assert.ok(Math.abs(lean.max - 2) < 2e-5);
  store.setReactionBounds('A@MK_x', 0, 0.1);
  assert.equal(store.steadyComState, null);
  const constrained = await store.steadyComRange();
  assert.ok(constrained.max < lean.max - 0.1, 'limiting the cross-fed product must reduce the sustainable growth rate');
  assert.equal((await store.steadyComProblem()).members.find(member => member.id === 'A').model.reactions.get('MK_x').upperBound, 0.1);
  assert.equal(store.models.get('member-A').reactions.get('MK_x').upperBound, 100, 'simulation overrides must not edit the catalog model');
});

test('a cancelled calculation cannot apply a late successful range result', async () => {
  const store = await pairStore();
  const deferred = deferredTask(store);
  const pending = store.steadyComRange();
  const call = await deferred.called;
  const ordinarySolution = store.solution;
  store.cancelSteadyCom();
  assert.equal(call.controls.signal.aborted, true);
  deferred.resolve(fakeRange());
  await assert.rejects(pending, { name: 'AbortError' });
  assert.equal(store.solution, ordinarySolution);
  assert.notEqual(store.steadyComState?.max, 99);
});

test('changing medium while a range task runs makes its late result stale', async () => {
  const store = await pairStore();
  const deferred = deferredTask(store);
  const pending = store.steadyComRange();
  const call = await deferred.called;
  store.setMediumLimit('glc', 4);
  const currentSolution = store.solution;
  deferred.resolve(fakeRange());
  await assert.rejects(pending, { name: 'AbortError' });
  assert.equal(call.controls.signal.aborted, true);
  assert.equal(store.steadyComState, null);
  assert.equal(store.solution, currentSolution);
  assert.equal(store.medium.glc, 4);
});

test('switching scenarios during a fixed task cannot install its old composition or fluxes', async () => {
  const store = await pairStore();
  await store.steadyComRange();
  const deferred = deferredTask(store);
  const pending = store.solveSteadyComAt(2);
  const call = await deferred.called;
  assert.equal(call.request.type, 'fixed');
  await store.useScenario('lean-mutualists');
  const currentSolution = store.solution;
  deferred.resolve({ status: 'optimal', feasible: true, total: 1, abundances: new Map([['A', 0.9], ['B', 0.1]]), fluxes: new Map([['A|BIOMASS', 1.8], ['B|BIOMASS', 0.2]]) });
  await assert.rejects(pending, { name: 'AbortError' });
  assert.equal(call.controls.signal.aborted, true);
  assert.equal(store.scenario.id, 'lean-mutualists');
  assert.equal(store.solution, currentSolution);
  assert.equal(store.solution.steadyCom, undefined);
  assert.deepEqual(store.species.map(species => species.abundance), [0.5, 0.5]);
});

test('scenario persistence recomputes and restores fixed μ in a new context; legacy scenarios return to ordinary FBA', async () => {
  const source = await pairStore();
  await source.solveSteadyComAt(1.5);
  const document = JSON.parse(source.stateJSON());
  assert.deepEqual(document.steadyCom, { mu: 1.5 });
  const restored = await pairStore();
  const task = restored.runSteadyComTask;
  const requests = [];
  restored.runSteadyComTask = (...args) => { requests.push(args[1].type); return task(...args); };
  await restored.loadState(JSON.stringify(document));
  assert.deepEqual(requests, ['range', 'fixed']);
  assertFixedSolution(restored, 1.5);
  assert.notEqual(restored.steadyComState.key, source.steadyComState.key);
  delete document.steadyCom;
  delete document.steadyComConfig;
  await restored.loadState(JSON.stringify(document));
  assert.equal(restored.steadyComState, null);
  assert.equal(restored.solution.steadyCom, undefined);
  assert.equal(restored.solution.status, 'optimal');
});

test('ordinary FVA refuses a fixed-μ result without changing the active solution', async () => {
  const store = await pairStore();
  await store.solveSteadyComAt(2);
  const solution = store.solution;
  store.setFVA(true);
  assert.equal(store.fvaEnabled, false);
  assert.equal(store.solution, solution);
  assert.match(store.status.message, /FVA.*fixed|fixed.*SteadyCom/);
});

test('GUT9 loads full biology and its diet, draws its curated map and leaves the solve to the worker', async t => {
  const previousFetch = globalThis.fetch;
  const requested = [];
  globalThis.fetch = async url => {
    const path = new URL(url, 'http://test.invalid/').pathname.replace(/^\//, '');
    assert.match(path, /^models\//, 'fixture fetches are restricted to repository model assets');
    requested.push(path);
    const text = await readFile(new URL(`../${path}`, import.meta.url), 'utf8');
    return { ok: true, status: 200, json: async () => JSON.parse(text), text: async () => text };
  };
  t.after(() => { globalThis.fetch = previousFetch; });
  const store = new Store('http://test.invalid');
  store.publishedCommunities = JSON.parse(await readFile(new URL('../models/scenarios.json', import.meta.url), 'utf8')).publishedCommunities;
  store.solve = () => { throw new Error('Selecting GUT9 must not run an expensive LP'); };
  const opening = new Promise((resolve) => {
    store.runSteadyComTask = (spec, request) => { resolve(request); return new Promise(() => {}); };
  });
  await store.usePublishedCommunity('steadycom-gut-nine');
  assert.equal(store.steadyComState.phase, 'fixed');
  assert.equal(store.steadyComSupport().supported, true);
  assert.equal(store.steadyComSupport().kind, 'joint');
  assert.equal(store.species.length, 9);
  assert.ok(store.species.every(species => /\/model-/.test(species.file)));
  assert.equal(store.models.get(store.species.find(species => species.id === 'Bt').file).reactions.size, 1443);
  assert.equal(store.community.merged.reactions.size, 11010);
  assert.equal(store.community.merged.metabolites.size, 8169);
  assert.equal(store.problem.rxnIds.length, 11010);
  // The drawn map has grown with each redrawn cell (1171 reactions after phase
  // 7); an excerpt is still a small part of the 11010 the community holds.
  assert.ok(store.map.reactions.size > 0 && store.map.reactions.size * 5 < store.community.merged.reactions.size,
    `drawing must stay an excerpt, got ${store.map.reactions.size}`);
  assert.equal(store.map.cells.length, 9);
  assert.equal(store.solution, null, 'the map is on screen before the solution arrives');
  assert.equal(Object.keys(store.medium).length, 88);
  assert.ok(requested.includes('models/published/steadycom-gut-nine/bounds.csv'));
  assert.ok(requested.includes('models/published/steadycom-gut-nine/model-Bt.json'));
  // The drawing is the curated map, not one assembled from the member previews.
  assert.ok(requested.includes('models/published/steadycom-gut-nine/map.json'));
  assert.ok(!requested.some((path) => /\/preview-/.test(path)));
  // It opens solved: one fixed-µ task at its default growth rate, not the range search.
  const request = await opening;
  assert.deepEqual([request.type, request.mu], ['fixed', 0.0796]);
  const spec = await store.steadyComProblem();
  assert.equal(spec.reactions.size, 11009);
  assert.equal(spec.metadata.speciesExchange.size, 1355);
});

/** Exercise the published-joint Store route with a small, exactly solvable model. */
async function tinyJointStore(t) {
  const store = new Store('http://joint-fixture.invalid');
  const published = 'steadycom-gut-nine';
  const rows = ['reaction,species,lower_bound,upper_bound,objective_coefficient'];
  const descriptors = [];
  for (const id of ['A', 'B']) {
    const doc = {
      id, compartments: { e: 'Extracellular', c: 'Cytosol', u: 'Shared medium' },
      metabolites: [
        { id: `a[e_${id}]`, compartment: 'e' }, { id: `a[c_${id}]`, compartment: 'c' },
        { id: 'a[u]', compartment: 'u' },
      ],
      reactions: [
        { id: `EX_${id}`, metabolites: { [`a[e_${id}]`]: -1, 'a[u]': 1 }, lower_bound: -20, upper_bound: 1000 },
        { id: `T_${id}`, metabolites: { [`a[e_${id}]`]: -1, [`a[c_${id}]`]: 1 }, lower_bound: 0, upper_bound: 1000 },
        { id: `BIO_${id}`, metabolites: { [`a[c_${id}]`]: -1 }, lower_bound: 0, upper_bound: 1000 },
      ], objective: { [`BIO_${id}`]: 1 },
    };
    store.models.set(`published/${published}/model-${id}`, new Model(doc));
    const preview = structuredClone(doc);
    preview.metabolites = preview.metabolites.filter(metabolite => metabolite.id !== 'a[u]');
    delete preview.compartments.u;
    delete preview.reactions[0].metabolites['a[u]'];
    store.models.set(`published/${published}/preview-${id}`, new Model(preview));
    rows.push(`EX_${id},${id},-20,1000,0`, `T_${id},${id},0,1000,0`, `BIO_${id},${id},0,1000,1`);
    descriptors.push({ id, model: `model-${id}.json`, preview: `preview-${id}.json` });
  }
  rows.push('UT_a(u),comm,0,10,0', 'EX_a(u),comm,0,10000,0');
  store.publishedCommunities = [{ id: published, name: 'Small joint Store fixture' }];
  store.publishedManifests.set(published, { members: descriptors, medium: { a: 10 } });
  const previousFetch = globalThis.fetch;
  globalThis.fetch = async url => {
    assert.equal(url, `http://joint-fixture.invalid/models/published/${published}/bounds.csv`);
    return { ok: true, status: 200, text: async () => rows.join('\n') };
  };
  t.after(() => { globalThis.fetch = previousFetch; });
  await store.usePublishedCommunity(published);
  return store;
}

test('a SteadyCom community opens solved, with its lines on, and gives the view back when it is left', async t => {
  const store = await tinyJointStore(t);
  const opened = await store.steadyComOnOpen;
  assert.equal(opened.mu, 0.0796);
  assert.equal(opened.max, null, 'the range is still computed only when asked');
  assert.equal(store.solution.steadyCom.mu, 0.0796);
  assert.equal(store.view.showCrossFeeding, true);
  assert.equal(store.unsavedWork().scenario, false, 'the solve is part of opening');
  // The user's controls work on it as before.
  assert.equal(store.clearSolution(), true);
  await store.solveSteadyComAt(2);
  assert.equal(store.solution.steadyCom.mu, 2);
  store.scenarios = [{ id: 'plain', name: 'Plain', species: [{ id: 'A', model: 'published/steadycom-gut-nine/model-A', abundance: 1 }], medium: {} }];
  await store.useScenario('plain');
  assert.equal(store.view.showCrossFeeding, false);
});

test('disabling a joint member preserves the excerpt and closes its entire fixed-composition biology', async t => {
  const store = await tinyJointStore(t);
  await store.setSpeciesEnabled('B', false);
  assert.equal(store.map.cells.length, 1);
  assert.equal(store.map.cells[0].id, 'A');
  assert.ok(store.map.reactions.size < 20);
  assert.equal(store.community.abundances.get('B') ?? 0, 0);
  for (const id of ['B@EX_B', 'B@T_B', 'B@BIO_B']) {
    assert.deepEqual(store.problem.getBounds(id), { lower: 0, upper: 0 }, id);
  }
  const spec = await store.steadyComProblem();
  assert.equal(spec.reactions.size, 8, 'the joint source must remain complete when one member is inactive');
  for (const mu of [0, 2]) {
    await store.solveSteadyComAt(mu);
    assert.equal(store.solution.steadyCom.abundances.get('B'), 0, `disabled B must remain absent even at μ=${mu}`);
    for (const id of ['B@EX_B', 'B@T_B', 'B@BIO_B']) assert.equal(store.solution.fluxes.get(id), 0, id);
  }
});

test('joint medium edits update the real uptake column used by ordinary FBA and the next SteadyCom task', async t => {
  const store = await tinyJointStore(t);
  store.setMediumLimit('a', 4);
  assert.deepEqual(store.problem.getBounds('UT_a(u)'), { lower: 0, upper: 4 });
  assert.equal(store.solution.status, 'optimal');
  assert.equal(store.solution.steadyCom, undefined);
  assert.ok(Math.abs(store.solution.objectiveValue - 4) < 1e-7);
  assert.equal((await store.steadyComProblem()).reactions.get('UT_a(u)').upperBound, 4);
});

test('a joint fixed-μ result rebuilds ordinary FBA with its calculated composition while keeping the drawing', async t => {
  const store = await tinyJointStore(t);
  const map = store.map;
  await store.solveSteadyComAt(2);
  const abundances = store.solution.steadyCom.abundances;
  assert.ok([...abundances.values()].some(amount => Math.abs(amount - 0.5) > 1e-6), 'this tiny LP returns a different vertex composition from the starting half/half mixture');
  assert.equal(store.map, map);
  for (const id of ['A', 'B']) {
    const coefficient = store.community.merged.reactions.get(`${id}@EX_${id}`).metabolites['M@a_s'] ?? 0;
    assert.ok(Math.abs(coefficient - abundances.get(id)) < 1e-7, `${id}: ordinary shared stoichiometry must use the newly solved abundance`);
    if (abundances.get(id) === 0) assert.deepEqual(store.problem.getBounds(`${id}@BIO_${id}`), { lower: 0, upper: 0 });
  }
  store.solve();
  assert.equal(store.solution.status, 'optimal');
  assert.equal(store.solution.steadyCom, undefined);
  assert.ok(Math.abs(store.solution.objectiveValue - 10) < 1e-7);
  const residuals = new Map();
  for (const reaction of store.community.merged.reactions.values()) {
    for (const [metabolite, coefficient] of Object.entries(reaction.metabolites)) {
      residuals.set(metabolite, (residuals.get(metabolite) ?? 0) + coefficient * (store.solution.fluxes.get(reaction.id) ?? 0));
    }
  }
  for (const [metabolite, residual] of residuals) assert.ok(Math.abs(residual) < 1e-7, `${metabolite}: ordinary FBA remains mass balanced`);
});

test('a bound edited on a joint member does not reopen it while it is switched off', async t => {
  const store = await tinyJointStore(t);
  store.setReactionBounds('B@T_B', null, '5');
  assert.equal(store.problem.getBounds('B@T_B').upper, 5);
  await store.setSpeciesEnabled('B', false);
  assert.deepEqual(store.problem.getBounds('B@T_B'), { lower: 0, upper: 0 }, 'the kept edit must not reopen a closed member');
  assert.equal(store.overrides.get('B@T_B')?.upper, 5, 'the edit itself is kept for when B returns');
  await store.setSpeciesEnabled('B', true);
  assert.equal(store.problem.getBounds('B@T_B').upper, 5, 'and applies again once B is back');
});

test('a bound edited on a joint member does not reopen it at zero abundance either', async t => {
  const store = await tinyJointStore(t);
  store.setReactionBounds('B@T_B', null, '5');
  await store.setAbundance('B', 0);
  assert.equal(store.speciesHeldOut('B'), true);
  assert.deepEqual(store.problem.getBounds('B@T_B'), { lower: 0, upper: 0 }, 'zero abundance closes B, and the edit must not reopen it');
  await store.setAbundance('B', 0.5);
  assert.equal(store.problem.getBounds('B@T_B').upper, 5, 'the edit applies again once B has an abundance');
});
