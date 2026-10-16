/**
 * Bounded-variable revised simplex.
 *
 * Solves   max (or min)  cᵀx
 *          s.t.          A x = b
 *                        l ≤ x ≤ u        (l may be -∞, u may be +∞)
 *
 * Genome-scale-lite problems (a few thousand columns) are well within reach:
 * the basis inverse is kept dense (m × m) and updated with product-form etas,
 * with a periodic Gauss–Jordan refactorisation to stop error from piling up.
 *
 * A is passed column-major and sparse, which is what a stoichiometric matrix
 * naturally is:  columns[j] = [rowIndex, value, rowIndex, value, ...]
 */

// Status strings every LP and MILP backend here reports.
export const LP_OPTIMAL = 'optimal';
export const LP_INFEASIBLE = 'infeasible';
export const LP_UNBOUNDED = 'unbounded';
export const LP_ITERATION_LIMIT = 'iteration_limit';

const AT_BASIC = 0;
const AT_LOWER = 1;
const AT_UPPER = 2;
const AT_FREE = 3;

const INF = Infinity;

/** Column-major sparse matrix builder. */
export class SparseColumns {
  constructor(rows, cols) {
    this.rows = rows;
    this.cols = cols;
    /** @type {number[][]} flat [row, val, row, val, ...] per column */
    this.data = Array.from({ length: cols }, () => []);
  }

  /** Set entry (row, col). A zero is skipped and does not clear an existing entry. */
  set(row, col, value) {
    if (value === 0) return;
    const c = this.data[col];
    for (let k = 0; k < c.length; k += 2) {
      if (c[k] === row) { c[k + 1] = value; return; }
    }
    c.push(row, value);
  }

  /** Add `value` to entry (row, col). */
  add(row, col, value) {
    if (value === 0) return;
    const c = this.data[col];
    for (let k = 0; k < c.length; k += 2) {
      if (c[k] === row) { c[k + 1] += value; return; }
    }
    c.push(row, value);
  }

  column(j) { return this.data[j]; }

  /** Append a column described as a flat [row, val, ...] array. */
  pushColumn(flat) {
    this.data.push(flat);
    this.cols += 1;
    return this.cols - 1;
  }
}

/**
 * Solve the LP above with a two-phase bounded simplex.
 *
 * Returns { status, objective, x, duals, reducedCosts, iterations, message }:
 * one dual per row of A and one reduced cost per column, in the sign
 * convention of the objective as given. x, duals and reducedCosts are null
 * unless the status is LP_OPTIMAL.
 *
 * @param {object} problem
 * @param {SparseColumns} problem.A
 * @param {Float64Array|number[]} problem.b
 * @param {Float64Array|number[]} problem.c
 * @param {Float64Array|number[]} problem.lb
 * @param {Float64Array|number[]} problem.ub
 * @param {boolean} [problem.maximize=true]
 * @param {object} [options] maxIterations, pivotTolerance, feasibilityTolerance,
 *   optimalityTolerance, refactorEvery (defaults below)
 */
export function solveLP(problem, options = {}) {
  const {
    maxIterations = 200000,
    pivotTolerance = 1e-9,
    feasibilityTolerance = 1e-8,
    optimalityTolerance = 1e-8,
    refactorEvery = 100,
  } = options;

  const m = problem.A.rows;
  const nStructural = problem.A.cols;
  const maximize = problem.maximize !== false;

  // Working problem: structurals plus one artificial per row
  const n = nStructural + m;
  const cols = new Array(n);
  for (let j = 0; j < nStructural; j++) cols[j] = problem.A.column(j);

  const lb = new Float64Array(n);
  const ub = new Float64Array(n);
  const cObj = new Float64Array(n);
  const sign = maximize ? 1 : -1;
  for (let j = 0; j < nStructural; j++) {
    lb[j] = problem.lb[j];
    ub[j] = problem.ub[j];
    cObj[j] = sign * (problem.c[j] || 0);
    if (lb[j] > ub[j]) {
      return fail(LP_INFEASIBLE, `Reversed bounds on column ${j}: [${lb[j]}, ${ub[j]}]`);
    }
  }

  const x = new Float64Array(n);
  const status = new Int8Array(n);

  // Park every structural variable on a finite bound (free ones sit at 0).
  for (let j = 0; j < nStructural; j++) {
    if (Number.isFinite(lb[j])) { status[j] = AT_LOWER; x[j] = lb[j]; }
    else if (Number.isFinite(ub[j])) { status[j] = AT_UPPER; x[j] = ub[j]; }
    else { status[j] = AT_FREE; x[j] = 0; }
  }

  // Residual that the artificials must absorb.
  const residual = new Float64Array(m);
  for (let i = 0; i < m; i++) residual[i] = problem.b[i];
  for (let j = 0; j < nStructural; j++) {
    const v = x[j];
    if (v === 0) continue;
    const col = cols[j];
    for (let k = 0; k < col.length; k += 2) residual[col[k]] -= col[k + 1] * v;
  }

  const basis = new Int32Array(m);
  const artificialSign = new Float64Array(m);
  for (let i = 0; i < m; i++) {
    const j = nStructural + i;
    const s = residual[i] < 0 ? -1 : 1;
    artificialSign[i] = s;
    cols[j] = [i, s];
    lb[j] = 0;
    ub[j] = INF;
    cObj[j] = 0;
    basis[i] = j;
    status[j] = AT_BASIC;
    x[j] = Math.abs(residual[i]);
  }

  const state = {
    m, n, nStructural, cols, lb, ub, x, status, basis,
    binv: new Float64Array(m * m),
    pivotTolerance, feasibilityTolerance, optimalityTolerance, refactorEvery,
    iterations: 0, maxIterations,
  };
  for (let i = 0; i < m; i++) state.binv[i * m + i] = artificialSign[i];

  // Phase 1: drive the artificials to zero
  const phase1c = new Float64Array(n);
  for (let i = 0; i < m; i++) phase1c[nStructural + i] = -1; // maximise -Σ a

  let res = simplexLoop(state, phase1c);
  if (res.status === LP_ITERATION_LIMIT) return fail(LP_ITERATION_LIMIT, 'Phase 1 hit the iteration limit');
  if (res.status === LP_UNBOUNDED) return fail(LP_INFEASIBLE, 'Phase 1 reported an unbounded objective');

  let infeasibility = 0;
  for (let i = 0; i < m; i++) infeasibility += Math.abs(x[nStructural + i]);
  if (infeasibility > 1e-6 * Math.max(1, norm1(problem.b))) {
    return fail(LP_INFEASIBLE, `Residual infeasibility ${infeasibility.toExponential(3)}`);
  }

  // Pin the artificials at zero. The ones still in the basis are degenerate and
  // leave on their own if a pivot needs the row.
  for (let i = 0; i < m; i++) {
    const j = nStructural + i;
    ub[j] = 0;
    if (status[j] !== AT_BASIC) { status[j] = AT_LOWER; x[j] = 0; }
  }

  // Phase 2: the real objective
  res = simplexLoop(state, cObj);
  if (res.status === LP_UNBOUNDED) return fail(LP_UNBOUNDED, 'Objective is unbounded');
  if (res.status === LP_ITERATION_LIMIT) return fail(LP_ITERATION_LIMIT, 'Phase 2 hit the iteration limit');

  // Extract the solution
  const primal = new Float64Array(nStructural);
  for (let j = 0; j < nStructural; j++) primal[j] = clampTiny(x[j], lb[j], ub[j], feasibilityTolerance);

  const y = computeDuals(state, cObj);
  const duals = new Float64Array(m);
  for (let i = 0; i < m; i++) duals[i] = sign * y[i];

  const reduced = new Float64Array(nStructural);
  for (let j = 0; j < nStructural; j++) {
    if (status[j] === AT_BASIC) { reduced[j] = 0; continue; }
    reduced[j] = sign * (cObj[j] - dot(cols[j], y));
  }

  let objective = 0;
  for (let j = 0; j < nStructural; j++) objective += (problem.c[j] || 0) * primal[j];

  return {
    status: LP_OPTIMAL,
    objective,
    x: primal,
    duals,
    reducedCosts: reduced,
    iterations: state.iterations,
    message: '',
  };
}

function simplexLoop(state, c) {
  const { m, n, cols, lb, ub, x, status, basis } = state;
  let sinceRefactor = 0;
  let degenerateStreak = 0;
  let bland = false;

  const alpha = new Float64Array(m);

  while (state.iterations < state.maxIterations) {
    state.iterations += 1;

    if (sinceRefactor >= state.refactorEvery) {
      if (!refactorize(state)) return { status: LP_INFEASIBLE };
      sinceRefactor = 0;
    }

    const y = computeDuals(state, c);

    // Pricing
    let enter = -1;
    let enterDir = 0;
    let best = state.optimalityTolerance;
    for (let j = 0; j < n; j++) {
      const st = status[j];
      if (st === AT_BASIC) continue;
      if (lb[j] === ub[j]) continue; // fixed column, never useful
      const d = c[j] - dot(cols[j], y);
      let dir = 0;
      if (st === AT_LOWER && d > state.optimalityTolerance) dir = 1;
      else if (st === AT_UPPER && d < -state.optimalityTolerance) dir = -1;
      else if (st === AT_FREE && Math.abs(d) > state.optimalityTolerance) dir = d > 0 ? 1 : -1;
      if (!dir) continue;
      if (bland) { enter = j; enterDir = dir; break; }
      const score = Math.abs(d);
      if (score > best) { best = score; enter = j; enterDir = dir; }
    }

    if (enter < 0) return { status: LP_OPTIMAL };

    // alpha = B⁻¹ A_q
    btranColumn(state, cols[enter], alpha);

    // Harris-style two-pass ratio test
    const span = ub[enter] - lb[enter];
    let tMax = Number.isFinite(span) ? span : INF;
    const feas = state.feasibilityTolerance;

    for (let i = 0; i < m; i++) {
      const a = alpha[i];
      if (Math.abs(a) < state.pivotTolerance) continue;
      const delta = -enterDir * a; // change of x_B[i] per unit of t
      const bi = basis[i];
      let limit;
      if (delta > 0) {
        if (!Number.isFinite(ub[bi])) continue;
        limit = (ub[bi] - x[bi] + feas) / delta;
      } else {
        if (!Number.isFinite(lb[bi])) continue;
        limit = (lb[bi] - x[bi] - feas) / delta;
      }
      if (limit < tMax) tMax = Math.max(limit, 0);
    }

    if (!Number.isFinite(tMax)) return { status: LP_UNBOUNDED };

    let leave = -1;
    let leaveToUpper = false;
    let bestPivot = state.pivotTolerance;
    let tActual = Number.isFinite(span) ? span : INF;

    for (let i = 0; i < m; i++) {
      const a = alpha[i];
      if (Math.abs(a) < state.pivotTolerance) continue;
      const delta = -enterDir * a;
      const bi = basis[i];
      let ratio;
      if (delta > 0) {
        if (!Number.isFinite(ub[bi])) continue;
        ratio = (ub[bi] - x[bi]) / delta;
      } else {
        if (!Number.isFinite(lb[bi])) continue;
        ratio = (lb[bi] - x[bi]) / delta;
      }
      if (ratio > tMax) continue;
      const magnitude = Math.abs(a);
      if (bland) {
        if (leave < 0 || bi < basis[leave]) { leave = i; leaveToUpper = delta > 0; tActual = Math.max(ratio, 0); }
      } else if (magnitude > bestPivot) {
        bestPivot = magnitude;
        leave = i;
        leaveToUpper = delta > 0;
        tActual = Math.max(ratio, 0);
      }
    }

    // Nothing blocks before the entering variable reaches its opposite bound:
    // just flip it between bounds, the basis is unchanged.
    if (leave < 0 || (Number.isFinite(span) && span <= tActual)) {
      if (!Number.isFinite(span)) return { status: LP_UNBOUNDED };
      const t = span;
      applyStep(state, alpha, enter, enterDir, t);
      x[enter] = enterDir > 0 ? ub[enter] : lb[enter];
      status[enter] = enterDir > 0 ? AT_UPPER : AT_LOWER;
      degenerateStreak = t < 1e-12 ? degenerateStreak + 1 : 0;
      if (degenerateStreak > 80) bland = true;
      continue;
    }

    const t = tActual;
    applyStep(state, alpha, enter, enterDir, t);

    const leavingVar = basis[leave];
    x[leavingVar] = leaveToUpper ? ub[leavingVar] : lb[leavingVar];
    status[leavingVar] = leaveToUpper ? AT_UPPER : AT_LOWER;
    if (lb[leavingVar] === ub[leavingVar]) status[leavingVar] = AT_LOWER;

    basis[leave] = enter;
    status[enter] = AT_BASIC;
    updateBinv(state, alpha, leave);
    sinceRefactor += 1;

    degenerateStreak = t < 1e-12 ? degenerateStreak + 1 : 0;
    if (degenerateStreak > 80) bland = true;
    else if (degenerateStreak === 0) bland = false;
  }

  return { status: LP_ITERATION_LIMIT };
}

/** x_B ← x_B − t·dir·alpha and x_enter ← x_enter + t·dir. */
function applyStep(state, alpha, enter, dir, t) {
  if (t === 0) return;
  const { m, x, basis } = state;
  for (let i = 0; i < m; i++) {
    const a = alpha[i];
    if (a !== 0) x[basis[i]] -= t * dir * a;
  }
  x[enter] += t * dir;
}

/** y = c_Bᵀ B⁻¹ */
function computeDuals(state, c) {
  const { m, binv, basis } = state;
  const y = new Float64Array(m);
  for (let k = 0; k < m; k++) {
    const cb = c[basis[k]];
    if (cb === 0) continue;
    const off = k * m;
    for (let i = 0; i < m; i++) y[i] += cb * binv[off + i];
  }
  return y;
}

/** out = B⁻¹ · column */
function btranColumn(state, column, out) {
  const { m, binv } = state;
  out.fill(0);
  for (let k = 0; k < column.length; k += 2) {
    const row = column[k];
    const val = column[k + 1];
    for (let i = 0; i < m; i++) out[i] += binv[i * m + row] * val;
  }
}

/** Product-form update of B⁻¹ after `leave` is replaced. */
function updateBinv(state, alpha, leave) {
  const { m, binv } = state;
  const pivot = alpha[leave];
  const off = leave * m;
  const inv = 1 / pivot;
  for (let i = 0; i < m; i++) binv[off + i] *= inv;
  for (let r = 0; r < m; r++) {
    if (r === leave) continue;
    const a = alpha[r];
    if (a === 0) continue;
    const ro = r * m;
    for (let i = 0; i < m; i++) binv[ro + i] -= a * binv[off + i];
  }
}

/** Rebuild B⁻¹ from scratch with Gauss–Jordan and partial pivoting. */
function refactorize(state) {
  const { m, cols, basis } = state;
  const a = new Float64Array(m * m);
  const inv = new Float64Array(m * m);
  for (let k = 0; k < m; k++) {
    const col = cols[basis[k]];
    for (let t = 0; t < col.length; t += 2) a[col[t] * m + k] = col[t + 1];
    inv[k * m + k] = 1;
  }
  for (let col = 0; col < m; col++) {
    let pivotRow = -1;
    let bestVal = state.pivotTolerance;
    for (let r = col; r < m; r++) {
      const v = Math.abs(a[r * m + col]);
      if (v > bestVal) { bestVal = v; pivotRow = r; }
    }
    if (pivotRow < 0) return false; // singular basis: keep the old inverse
    if (pivotRow !== col) { swapRows(a, m, col, pivotRow); swapRows(inv, m, col, pivotRow); }
    const p = a[col * m + col];
    const ip = 1 / p;
    for (let i = 0; i < m; i++) { a[col * m + i] *= ip; inv[col * m + i] *= ip; }
    for (let r = 0; r < m; r++) {
      if (r === col) continue;
      const f = a[r * m + col];
      if (f === 0) continue;
      for (let i = 0; i < m; i++) {
        a[r * m + i] -= f * a[col * m + i];
        inv[r * m + i] -= f * inv[col * m + i];
      }
    }
  }
  state.binv.set(inv);
  // Recompute basic values so accumulated drift does not survive.
  recomputeBasics(state);
  return true;
}

function recomputeBasics(state) {
  const { m, n, cols, x, status, basis, lb, ub } = state;
  const rhs = new Float64Array(m);
  for (let i = 0; i < m; i++) rhs[i] = state.rhs ? state.rhs[i] : 0;
  // rhs is not tracked separately: reconstruct b from the current point.
  // b = A x, which holds exactly while the iterate is feasible.
  const b = new Float64Array(m);
  for (let j = 0; j < n; j++) {
    const v = x[j];
    if (v === 0) continue;
    const col = cols[j];
    for (let k = 0; k < col.length; k += 2) b[col[k]] += col[k + 1] * v;
  }
  const nb = new Float64Array(m);
  for (let j = 0; j < n; j++) {
    if (status[j] === AT_BASIC) continue;
    const v = x[j];
    if (v === 0) continue;
    const col = cols[j];
    for (let k = 0; k < col.length; k += 2) nb[col[k]] += col[k + 1] * v;
  }
  for (let i = 0; i < m; i++) rhs[i] = b[i] - nb[i];
  for (let k = 0; k < m; k++) {
    let s = 0;
    const off = k * m;
    for (let i = 0; i < m; i++) s += state.binv[off + i] * rhs[i];
    const bk = basis[k];
    if (s < lb[bk] - 1e-7) s = lb[bk];
    if (s > ub[bk] + 1e-7) s = ub[bk];
    x[bk] = s;
  }
}

function swapRows(mat, m, r1, r2) {
  const o1 = r1 * m; const o2 = r2 * m;
  for (let i = 0; i < m; i++) { const t = mat[o1 + i]; mat[o1 + i] = mat[o2 + i]; mat[o2 + i] = t; }
}

function dot(column, y) {
  let s = 0;
  for (let k = 0; k < column.length; k += 2) s += column[k + 1] * y[column[k]];
  return s;
}

function norm1(v) {
  let s = 0;
  for (let i = 0; i < v.length; i++) s += Math.abs(v[i]);
  return s;
}

function clampTiny(v, lo, hi, tol) {
  if (Math.abs(v) < tol) return 0;
  if (v < lo) return lo;
  if (v > hi) return hi;
  return v;
}

function fail(status, message) {
  return { status, objective: NaN, x: null, duals: null, reducedCosts: null, iterations: 0, message };
}
