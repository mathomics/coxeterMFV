import test from 'node:test';
import assert from 'node:assert/strict';
import { SparseColumns, LP_ITERATION_LIMIT } from '../src/solver/simplex.js';
import { initGLPK, solveWithGLPK } from '../src/solver/glpk.js';
import { Model } from '../src/core/model.js';
import { steadyCom } from '../src/core/steadycom.js';

let code = 2;
globalThis.__GLPK_FACTORY__ = async () => ({
  GLP_OPT: 5, GLP_FEAS: 2, GLP_INFEAS: 3, GLP_NOFEAS: 4, GLP_UNBND: 6, GLP_UNDEF: 1,
  GLP_MIN: 1, GLP_MAX: 2, GLP_FR: 1, GLP_LO: 2, GLP_UP: 3, GLP_DB: 4, GLP_FX: 5, GLP_MSG_OFF: 0,
  solve: () => ({ result: { status: code, z: 0, vars: {}, dual: {} } }),
});
test.before(async () => { assert.equal(await initGLPK(), true); });

test('feasible-but-unproven and an infeasible basis are unfinished, not optimal or impossible', () => {
  const problem = { A: new SparseColumns(0, 1), b: new Float64Array(0), c: new Float64Array([1]),
    lb: new Float64Array([0]), ub: new Float64Array([1]) };
  for (const current of [2, 3]) {
    code = current;
    const result = solveWithGLPK(problem);
    assert.equal(result.status, LP_ITERATION_LIMIT);
    assert.equal(result.x, null);
  }
});

test('unfinished capped objectives cannot generate a maximum or a zero-infeasible verdict', () => {
  code = 2;
  const model = new Model({ id: 'A', compartments: {}, metabolites: [], reactions: [
    { id: 'BIOMASS', metabolites: {}, lower_bound: 0, upper_bound: 1 },
  ], objective: { BIOMASS: 1 } });
  const result = steadyCom({ members: [{ id: 'A', model, biomassReaction: 'BIOMASS' }], totalBiomass: 1 }, { biomassCap: 2 });
  assert.equal(result.status, 'solver-error');
  assert.equal(result.zeroFeasible, null);
  assert.equal(result.rangeUpper, undefined);
  assert.equal(result.fluxes.size, 0);
});
