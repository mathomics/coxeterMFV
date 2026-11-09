import test from 'node:test';
import assert from 'node:assert/strict';
import { SparseColumns } from '../src/solver/simplex.js';
import { solveBinaryMILP } from '../src/solver/milp.js';

test('binary MILP branch-and-bound enforces integrality', () => {
  // max x + y, x + 2y = 2.5, x/y binary. LP optimum is fractional,
  // but the only binary-feasible point under equality is intentionally absent.
  const A = new SparseColumns(1, 2);
  A.add(0, 0, 1);
  A.add(0, 1, 2);
  const result = solveBinaryMILP({
    A,
    b: new Float64Array([2.5]),
    c: new Float64Array([1, 1]),
    lb: new Float64Array([0, 0]),
    ub: new Float64Array([1, 1]),
    maximize: true,
  }, [0, 1], { nodeLimit: 100, timeLimitMs: 1000 });
  assert.equal(result.status, 'infeasible');
});

test('binary MILP returns an optimal integer solution', () => {
  // x + y + s = 1.5, s >= 0. max x+y with binaries -> optimum 1.
  const A = new SparseColumns(1, 3);
  A.add(0, 0, 1);
  A.add(0, 1, 1);
  A.add(0, 2, 1);
  const result = solveBinaryMILP({
    A,
    b: new Float64Array([1.5]),
    c: new Float64Array([1, 1, 0]),
    lb: new Float64Array([0, 0, 0]),
    ub: new Float64Array([1, 1, Infinity]),
    maximize: true,
  }, [0, 1], { nodeLimit: 100, timeLimitMs: 1000 });
  assert.equal(result.status, 'optimal');
  assert.equal(result.objective, 1);
  assert.ok([0, 1].includes(Math.round(result.x[0])));
  assert.ok([0, 1].includes(Math.round(result.x[1])));
});
