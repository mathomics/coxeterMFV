import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import factory from '../assets/vendor/glpk.js';
import { Model } from '../src/core/model.js';
import { runSteadyComTask } from '../src/communities/steadycom-client.js';

const spec = () => ({
  members: [{ id: 'A', biomassReaction: 'BIOMASS', exchangeFor: { a: 'EX_a_e' }, model: new Model({
    id: 'A', compartments: { e: 'Extracellular' }, metabolites: [{ id: 'a_e', compartment: 'e' }],
    reactions: [
      { id: 'EX_a_e', metabolites: { a_e: -1 }, lower_bound: -1, upper_bound: 0 },
      { id: 'BIOMASS', metabolites: { a_e: -1 }, lower_bound: 0, upper_bound: 10 },
    ], objective: { BIOMASS: 1 },
  }) }], communityUptake: { a: 1 }, totalBiomass: 1,
});

test.before(async () => {
  globalThis.__GLPK_FACTORY__ = factory;
  globalThis.__GLPK_WASM__ = await readFile(new URL('../assets/vendor/glpk.wasm', import.meta.url));
});

test('Node fixed-µ fallback hydrates models and returns native Maps', async () => {
  const input = spec();
  const result = await runSteadyComTask(input, { type: 'fixed', mu: 0.5 });
  assert.equal(result.feasible, true);
  assert.ok(result.abundances instanceof Map);
  assert.ok(result.fluxes instanceof Map);
  assert.ok(Math.abs(result.fluxes.get('A|BIOMASS') / result.abundances.get('A') - 0.5) < 1e-8);
  assert.equal(input.members[0].model.reactions.get('BIOMASS').lowerBound, 0);
});

test('Node parsimonious fallback reports the least total flux', async () => {
  // One member, one route: biomass takes the substrate straight from the
  // exchange, so at mu = 0.5 each of the two fluxes is 0.5 and their sum is 1.
  const result = await runSteadyComTask(spec(), { type: 'parsimonious', mu: 0.5 });
  assert.equal(result.feasible, true);
  assert.ok(result.fluxes instanceof Map);
  assert.ok(Math.abs(result.totalFlux - 1) < 1e-8, `got ${result.totalFlux}`);
  await assert.rejects(runSteadyComTask(spec(), { type: 'parsimonious', mu: -1 }), /finite number/);
});

test('Node range search reports progress and a verified interval', async () => {
  const progress = [];
  const result = await runSteadyComTask(spec(), { type: 'range', options: { upperGuess: 2, tolerance: 0.001 } },
    { onProgress: (p) => progress.push(p) });
  assert.equal(result.status, 'optimal');
  assert.equal(result.rangeLower, 0);
  assert.ok(Math.abs(result.rangeUpper - 1) < 0.001);
  assert.equal(progress.length, result.iterations);
});

test('pre-cancelled requests and invalid values are refused', async () => {
  const controller = new AbortController();
  controller.abort();
  await assert.rejects(runSteadyComTask(spec(), { type: 'range' }, { signal: controller.signal }), { name: 'AbortError' });
  await assert.rejects(runSteadyComTask(spec(), { type: 'fixed', mu: NaN }), /finite number/);
  await assert.rejects(runSteadyComTask(spec(), { type: 'unknown' }), /range search/);
});

async function fakeBrowser(run, behavior) {
  const savedWindow = globalThis.window;
  const savedWorker = globalThis.Worker;
  const instances = [];
  globalThis.window = {};
  globalThis.Worker = class {
    constructor(url, options) { this.url = url; this.options = options; this.terminated = false; instances.push(this); }
    postMessage(data) { this.posted = data; behavior?.(this, data); }
    terminate() { this.terminated = true; }
  };
  try { await run(instances); }
  finally {
    if (savedWindow === undefined) delete globalThis.window; else globalThis.window = savedWindow;
    if (savedWorker === undefined) delete globalThis.Worker; else globalThis.Worker = savedWorker;
  }
}

test('browser tasks ignore stale messages and preserve progress/result Maps', async () => {
  await fakeBrowser(async (workers) => {
    const progress = [];
    const promise = runSteadyComTask(spec(), { type: 'fixed', mu: 0.5 }, { onProgress: (p) => progress.push(p) });
    const worker = workers[0];
    const { id } = worker.posted;
    worker.onmessage({ data: { id: id + 1, type: 'result', result: { wrong: true } } });
    worker.onmessage({ data: { id, type: 'progress', progress: { iterations: 1, lo: 0, hi: 1, mu: 0.5 } } });
    worker.onmessage({ data: { id, type: 'result', result: { feasible: true, abundances: new Map([['A', 1]]) } } });
    const result = await promise;
    assert.equal(result.wrong, undefined);
    assert.equal(result.abundances.get('A'), 1);
    assert.equal(progress.length, 1);
    assert.equal(worker.options.type, 'module');
    assert.equal(worker.terminated, true);
  });
});

test('browser cancel and timeout terminate the worker with AbortError', async () => {
  await fakeBrowser(async (workers) => {
    const controller = new AbortController();
    const cancelled = runSteadyComTask(spec(), { type: 'range' }, { signal: controller.signal });
    controller.abort();
    await assert.rejects(cancelled, { name: 'AbortError' });
    assert.equal(workers[0].terminated, true);
    const timedOut = runSteadyComTask(spec(), { type: 'range' }, { timeoutMs: 10 });
    await assert.rejects(timedOut, { name: 'AbortError' });
    assert.equal(workers[1].terminated, true);
  });
});

test('a Worker error rejects the task and closes its worker', async () => {
  await fakeBrowser(async (workers) => {
    const promise = runSteadyComTask(spec(), { type: 'range' });
    const worker = workers[0];
    worker.onmessage({ data: { id: worker.posted.id, type: 'error', error: { name: 'Error', message: 'Cannot solve LP' } } });
    await assert.rejects(promise, /Cannot solve LP/);
    assert.equal(worker.terminated, true);
  });
});
