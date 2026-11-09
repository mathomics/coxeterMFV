/** Verify pFVA constraints and progressive solver dispatch on the bundled E. coli model. */

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { Model } from '../src/core/model.js';
import { FluxProblem } from '../src/solver/fba.js';
import { fluxRangeDomain, renderFluxRanges } from '../src/render/flux-ranges.js';
import { Store } from '../src/ui/store.js';
import { analysisProvenance } from '../src/core/reproducibility.js';
import { renderResults, renderTaskPanel } from '../src/ui/panels.js';

async function problem() {
  const text = await readFile(new URL('../models/ecoli_core_mc.json', import.meta.url), 'utf8');
  return new FluxProblem(new Model(JSON.parse(text)));
}

test('pFVA removes surplus cycle flux and retains the pFBA solution', async () => {
  const p = await problem();
  const ids = ['FRD7', 'SUCDi', 'PGI', 'EX_glc__D_e'];
  const ordinary = p.variability(ids);
  const strict = p.variability(ids, { pfbaFactor: 1 });
  const relaxed = p.variability(ids, { pfbaFactor: 1.1 });
  const solution = p.parsimonious();
  assert.ok(ordinary.get('FRD7').max > 100);
  assert.ok(Math.abs(strict.get('FRD7').max) < 1e-5);
  assert.ok(relaxed.get('FRD7').max > strict.get('FRD7').max + 1);
  for (const id of ids) {
    const range = strict.get(id);
    const flux = solution.fluxes.get(id);
    assert.ok(range.min >= ordinary.get(id).min - 1e-5, id);
    assert.ok(range.max <= ordinary.get(id).max + 1e-5, id);
    assert.ok(flux >= range.min - 1e-5 && flux <= range.max + 1e-5, id);
  }
});

test('progressive pFVA uses the chosen backend and matches synchronous pFVA', async () => {
  const p = await problem();
  const ids = ['FRD7', 'SUCDi', 'PGI'];
  const expected = p.variability(ids, { pfbaFactor: 1.00001 });
  const solve = p._solve.bind(p);
  let calls = 0;
  p._solve = (spec) => {
    calls += 1;
    return solve(spec);
  };
  const actual = await p.variabilityAsync(ids, { pfbaFactor: 1.00001, chunkSize: 1 });
  assert.equal(calls, 2 + 2 * ids.length);
  for (const id of ids) {
    assert.ok(Math.abs(actual.get(id).min - expected.get(id).min) < 1e-6);
    assert.ok(Math.abs(actual.get(id).max - expected.get(id).max) < 1e-6);
  }
});

test('FVA at ninety percent allows growth between ninety and one hundred percent', async () => {
  const p = await problem();
  const optimum = p.optimize().objectiveValue;
  const id = p.model.biomassReactionId();
  const range = p.variability([id], { fraction: 0.9 }).get(id);
  assert.ok(Math.abs(range.min - 0.9 * optimum) < 1e-6);
  assert.ok(Math.abs(range.max - optimum) < 1e-6);
});

test('the interval plot retains zero ranges and marks unsolved reactions', () => {
  const rows = [
    { id: 'reverse', min: -4, max: -2, flux: -3 },
    { id: '<zero>', min: 0, max: 0 },
    { id: 'failed', min: NaN, max: NaN },
  ];
  assert.deepEqual(fluxRangeDomain(rows), { min: -4, max: 0 });
  const svg = renderFluxRanges(rows);
  assert.match(svg, /&lt;zero&gt;/);
  assert.match(svg, /Unsolved/);
  assert.equal((svg.match(/<circle /g) ?? []).length, 4);
  assert.doesNotMatch(svg, /="NaN"/);
});

test('pFVA is a separate analysis that persists its flux budget and finishes after its ranges', async () => {
  const store = new Store('.');
  let options = null;
  const result = {
    status: 'optimal', objectiveValue: 1, fluxes: new Map([['R', 1]]),
    shadowPrices: new Map(), iterations: 1,
  };
  store.problem = {
    rxnIds: ['R'],
    optimize() {
      return result;
    },
    parsimonious() {
      return result;
    },
    async variabilityAsync(_ids, opts) {
      options = opts;
      return new Map([['R', { min: 1, max: 1 }]]);
    },
  };
  store.community = {
    reactionOwner: new Map(),
    merged: (await problem()).model,
    crossFeeding() {
      return { links: [], perMetabolite: [] };
    },
    growthRates() {
      return new Map();
    },
  };
  await store.runAnalysis('pfva');
  assert.equal(store.lastRun.phase, 'running');
  clearTimeout(store._fvaTimer);
  store._fvaTimer = null;
  await store.runFVA();
  assert.equal(options.pfbaFactor, 1.00001);
  assert.equal(store.lastRun.phase, 'done');
  assert.equal(store.pfvaEnabled, true);
  assert.equal(JSON.parse(store.stateJSON()).pfvaEnabled, true);
  assert.equal(analysisProvenance(store).method, 'pFVA');
  assert.equal(analysisProvenance(store).pfbaFactor, 1.00001);
  assert.match(renderTaskPanel(store, { leftTab: 'analysis' }), /run-pfva/);
  assert.match(renderResults(store, 'fva'), /pFVA flux intervals/);
  store.setFVA(false);
  assert.equal(store.pfvaEnabled, false);
  store.parsimonious = true;
  store.setFVA(true);
  clearTimeout(store._fvaTimer);
  store._fvaTimer = null;
  await store.runFVA();
  assert.equal(options.pfbaFactor, null);
  store.setFVA(false);
});
