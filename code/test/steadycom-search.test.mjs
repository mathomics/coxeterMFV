import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import factory from '../assets/vendor/glpk.js';
import { Model } from '../src/core/model.js';
import { initGLPK } from '../src/solver/glpk.js';
import { steadyCom, maxTotalBiomassAt } from '../src/core/steadycom.js';

test.before(async () => {
  globalThis.__GLPK_FACTORY__ = factory;
  globalThis.__GLPK_WASM__ = await readFile(new URL('../assets/vendor/glpk.wasm', import.meta.url));
  assert.equal(await initGLPK(), true);
});

function single({ biomassLower = 0, unbounded = false } = {}) {
  const model = new Model({
    id: 'A', compartments: { e: 'Extracellular' },
    metabolites: [{ id: 'a_e', compartment: 'e' }],
    reactions: [
      { id: 'EX_a_e', metabolites: { a_e: -1 }, lower_bound: -1, upper_bound: 0 },
      { id: 'BIOMASS', metabolites: unbounded ? {} : { a_e: -1 }, lower_bound: biomassLower,
        upper_bound: unbounded ? Infinity : 10 },
    ], objective: { BIOMASS: 1 },
  });
  if (unbounded) model.reactions.get('BIOMASS').upperBound = Infinity;
  return { members: [{ id: 'A', model, biomassReaction: 'BIOMASS', exchangeFor: { a: 'EX_a_e' } }],
    communityUptake: { a: 1 }, totalBiomass: 1 };
}

test('a feasible bracketing point retains fluxes at its reported growth rate', () => {
  const result = steadyCom(single(), { upperGuess: 1, tolerance: 0.001 });
  assert.equal(result.status, 'optimal');
  assert.equal(result.growthRate, 1);
  assert.equal(result.fluxes.get('A|BIOMASS'), result.growthRate * result.abundances.get('A'));
});

test('only a resolved bracket supplies the verified zero-to-maximum range', () => {
  const progress = [];
  const result = steadyCom(single(), { upperGuess: 2, biomassCap: 2, onProgress: (p) => progress.push(p) });
  assert.equal(result.status, 'optimal', result.message);
  assert.equal(result.zeroFeasible, true);
  assert.equal(result.rangeLower, 0);
  assert.equal(result.rangeUpper, result.growthRate);
  assert.ok(result.upperBound >= result.rangeUpper);
  assert.ok(result.upperBound - result.rangeUpper <= result.resolution);
  assert.ok(Math.abs(result.rangeUpper - 1) < 1e-4);
  assert.equal(progress.length, result.iterations);
  assert.deepEqual(progress.map((p) => p.iterations), progress.map((_, i) => i + 1));
  for (const p of progress) for (const key of ['lo', 'hi', 'mu']) assert.ok(Number.isFinite(p[key]));
});

test('an exhausted LP budget cannot be labelled optimal', () => {
  const result = steadyCom(single(), { upperGuess: 2, maxIterations: 2 });
  assert.equal(result.status, 'iteration-limit');
  assert.equal(result.zeroFeasible, true);
  assert.equal(result.iterations, 2);
  assert.equal(result.rangeUpper, undefined);
  assert.equal(result.fluxes.size, 0);
});

test('failure to find an upper bracket cannot invent a finite optimum', () => {
  const result = steadyCom(single({ unbounded: true }), { upperGuess: 2 });
  assert.equal(result.status, 'iteration-limit');
  assert.match(result.message, /No finite upper/);
  assert.equal(result.rangeUpper, undefined);
});

test('an excluded zero does not imply that positive growth is impossible', () => {
  const spec = single({ biomassLower: 0.5 });
  assert.equal(maxTotalBiomassAt(spec, 0).status, 'infeasible');
  assert.equal(maxTotalBiomassAt(spec, 0.5).feasible, true);
  const result = steadyCom(spec, { upperGuess: 2 });
  assert.equal(result.status, 'zero-infeasible');
  assert.equal(result.zeroFeasible, false);
  assert.match(result.message, /positive growth rate may still be feasible/);
  assert.equal(result.rangeLower, undefined);
});

test('invalid rates and search options are refused before solving', () => {
  for (const mu of [NaN, Infinity, -1, '0']) assert.throws(() => maxTotalBiomassAt(single(), mu), /finite number/);
  for (const options of [{ tolerance: 0 }, { tolerance: Infinity }, { upperGuess: NaN },
    { upperGuess: -1 }, { maxIterations: 1.5 }, { timeLimitSeconds: 0 }, { biomassCap: 0.5 }]) {
    assert.throws(() => steadyCom(single(), options));
  }
  assert.throws(() => steadyCom({ ...single(), totalBiomass: 0 }), /Total community biomass/);
});
