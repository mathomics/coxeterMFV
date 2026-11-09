import test from 'node:test';
import assert from 'node:assert/strict';
import { SparseColumns, solveLP, LP_OPTIMAL, LP_INFEASIBLE, LP_UNBOUNDED } from '../src/solver/simplex.js';

/** Brute-force reference for tiny LPs: enumerate the vertices of a 2-D problem. */
function referenceMax2D(A, b, c, lb, ub, samples = 4001) {
  let best = -Infinity;
  for (let i = 0; i < samples; i++) {
    const x0 = lb[0] + ((ub[0] - lb[0]) * i) / (samples - 1);
    // One equality constraint: solve for x1 given x0.
    const a0 = A[0][0];
    const a1 = A[0][1];
    if (a1 === 0) continue;
    const x1 = (b[0] - a0 * x0) / a1;
    if (x1 < lb[1] - 1e-9 || x1 > ub[1] + 1e-9) continue;
    best = Math.max(best, c[0] * x0 + c[1] * x1);
  }
  return best;
}

test('solves a bounded two-variable problem', () => {
  const A = new SparseColumns(1, 2);
  A.set(0, 0, 1); A.set(0, 1, 1);
  const res = solveLP({ A, b: [4], c: [3, 2], lb: [0, 0], ub: [3, 3] });
  assert.equal(res.status, LP_OPTIMAL);
  assert.ok(Math.abs(res.objective - 11) < 1e-9, `objective ${res.objective}`);
  assert.ok(Math.abs(res.x[0] - 3) < 1e-9);
  assert.ok(Math.abs(res.x[1] - 1) < 1e-9);
});

test('minimisation flips the optimum', () => {
  const A = new SparseColumns(1, 2);
  A.set(0, 0, 1); A.set(0, 1, 1);
  const res = solveLP({ A, b: [4], c: [3, 2], lb: [0, 0], ub: [3, 3], maximize: false });
  assert.equal(res.status, LP_OPTIMAL);
  assert.ok(Math.abs(res.objective - 9) < 1e-9);
});

test('detects infeasibility', () => {
  const A = new SparseColumns(1, 2);
  A.set(0, 0, 1); A.set(0, 1, 1);
  const res = solveLP({ A, b: [10], c: [1, 1], lb: [0, 0], ub: [3, 3] });
  assert.equal(res.status, LP_INFEASIBLE);
});

test('detects unboundedness', () => {
  const A = new SparseColumns(1, 2);
  A.set(0, 0, 1); A.set(0, 1, -1);
  const res = solveLP({ A, b: [0], c: [1, 1], lb: [0, 0], ub: [Infinity, Infinity] });
  assert.equal(res.status, LP_UNBOUNDED);
});

test('handles negative lower bounds and free variables', () => {
  const A = new SparseColumns(1, 2);
  A.set(0, 0, 2); A.set(0, 1, 1);
  // max x0 - x1  s.t. 2 x0 + x1 = 1, -5 <= x0 <= 5, x1 free
  const res = solveLP({
    A, b: [1], c: [1, -1],
    lb: [-5, -Infinity], ub: [5, Infinity],
  });
  assert.equal(res.status, LP_OPTIMAL);
  // x1 = 1 - 2 x0, objective = x0 - 1 + 2 x0 = 3 x0 - 1, maximised at x0 = 5.
  assert.ok(Math.abs(res.objective - 14) < 1e-7, `objective ${res.objective}`);
});

test('agrees with a brute-force scan on random dense problems', () => {
  let seed = 7;
  const rand = () => {
    seed = (seed * 1103515245 + 12345) & 0x7fffffff;
    return seed / 0x7fffffff;
  };
  for (let trial = 0; trial < 60; trial++) {
    const a0 = rand() * 4 - 2;
    const a1 = rand() * 4 - 2;
    if (Math.abs(a1) < 0.2 || Math.abs(a0) < 0.2) continue;
    const bb = rand() * 4 - 2;
    const c0 = rand() * 4 - 2;
    const c1 = rand() * 4 - 2;
    const lb = [-3, -3];
    const ub = [3, 3];

    const A = new SparseColumns(1, 2);
    A.set(0, 0, a0); A.set(0, 1, a1);
    const res = solveLP({ A, b: [bb], c: [c0, c1], lb, ub });
    const expected = referenceMax2D([[a0, a1]], [bb], [c0, c1], lb, ub);

    if (!Number.isFinite(expected)) {
      assert.equal(res.status, LP_INFEASIBLE, `trial ${trial} should be infeasible`);
      continue;
    }
    assert.equal(res.status, LP_OPTIMAL, `trial ${trial}: ${res.message}`);
    assert.ok(
      res.objective >= expected - 1e-4,
      `trial ${trial}: simplex ${res.objective} below scan ${expected}`,
    );
  }
});

test('respects equality constraints exactly on a larger sparse problem', () => {
  // A small flow network: 6 nodes, conservation at each, maximise throughput.
  const m = 4;
  const n = 7;
  const A = new SparseColumns(m, n);
  const edges = [[0, 1], [0, 2], [1, 2], [1, 3], [2, 3], [3, 0], [2, 0]];
  edges.forEach(([from, to], j) => { A.set(from, j, -1); A.set(to, j, 1); });
  const res = solveLP({
    A,
    b: new Float64Array(m),
    c: [0, 0, 0, 1, 0, 0, 0],
    lb: new Float64Array(n).fill(0),
    ub: new Float64Array(n).fill(5),
  });
  assert.equal(res.status, LP_OPTIMAL);
  // Conservation must hold at every node.
  const residual = new Float64Array(m);
  edges.forEach(([from, to], j) => { residual[from] -= res.x[j]; residual[to] += res.x[j]; });
  for (let i = 0; i < m; i++) assert.ok(Math.abs(residual[i]) < 1e-8, `node ${i} residual ${residual[i]}`);
});
