/**
 * Flux balance analysis.
 *
 * Everything works against a plain `Model`: the community merger in
 * src/core/community.js produces a normal COBRA model with namespaced ids, so
 * single-organism and multi-organism problems take exactly the same path.
 *
 * Two LP backends sit behind one interface:
 *   simplex  the bundled dense-inverse simplex, no dependencies, good to a few
 *            hundred metabolites
 *   glpk     GLPK's sparse LU simplex via WebAssembly, the same solver EscherFBA
 *            uses, which handles genome-scale models in ~100 ms
 *
 * `backend: 'auto'` (the default) picks GLPK whenever it has been initialised
 * and the bundled simplex otherwise, so the project works with no dependencies
 * installed.
 */

import { SparseColumns, solveLP, LP_OPTIMAL } from './simplex.js';
import { glpkAvailable, solveWithGLPK } from './glpk.js';
import { solveBinaryMILP } from './milp.js';

const BIG = 1e6;

/** Replace ±Infinity with a large finite number the simplex is comfortable with. */
function finite(v, fallback) {
  if (v === null || v === undefined || Number.isNaN(Number(v))) return fallback;
  const n = Number(v);
  if (n === Infinity) return BIG;
  if (n === -Infinity) return -BIG;
  return n;
}

/**
 * A reusable LP view of a model. Bounds and objective can be tweaked between
 * solves without rebuilding the stoichiometric matrix.
 */
export class FluxProblem {
  /**
   * @param {import('../core/model.js').Model} model
   * @param {object} [opts]
   * @param {Object<string,number>} [opts.objective] reaction id -> coefficient
   * @param {'auto'|'simplex'|'glpk'} [opts.backend='auto']
   */
  constructor(model, opts = {}) {
    this.model = model;
    this.backendChoice = opts.backend ?? 'auto';
    this.metIds = [...model.metabolites.keys()];
    this.rxnIds = [...model.reactions.keys()];
    this.metIndex = new Map(this.metIds.map((id, i) => [id, i]));
    this.rxnIndex = new Map(this.rxnIds.map((id, i) => [id, i]));

    const m = this.metIds.length;
    const n = this.rxnIds.length;
    this.A = new SparseColumns(m, n);
    this.lb = new Float64Array(n);
    this.ub = new Float64Array(n);
    this.c = new Float64Array(n);
    this.b = new Float64Array(m);

    this.rxnIds.forEach((rid, j) => {
      const rxn = model.reactions.get(rid);
      for (const [mid, coef] of Object.entries(rxn.metabolites)) {
        if (!coef) continue;
        this.A.add(this.metIndex.get(mid), j, coef);
      }
      this.lb[j] = finite(rxn.lowerBound, -BIG);
      this.ub[j] = finite(rxn.upperBound, BIG);
    });

    this.setObjective(opts.objective || defaultObjective(model));
  }

  /** @param {Object<string,number>} coefficients */
  setObjective(coefficients) {
    this.c.fill(0);
    this.objective = { ...coefficients };
    for (const [rid, coef] of Object.entries(coefficients)) {
      const j = this.rxnIndex.get(rid);
      if (j === undefined) continue;
      this.c[j] = Number(coef) || 0;
    }
    return this;
  }

  /**
   * Set one reaction's bounds. A null or undefined side is left as it is, and
   * a lower bound above the upper one is pulled down to it. Throws for an
   * unknown reaction id.
   */
  setBounds(reactionId, lower, upper) {
    const j = this.rxnIndex.get(reactionId);
    if (j === undefined) throw new Error(`Unknown reaction ${reactionId}`);
    if (lower !== null && lower !== undefined) this.lb[j] = finite(lower, this.lb[j]);
    if (upper !== null && upper !== undefined) this.ub[j] = finite(upper, this.ub[j]);
    if (this.lb[j] > this.ub[j]) this.lb[j] = this.ub[j];
    return this;
  }

  /** One reaction's current bounds, or null for an unknown id. */
  getBounds(reactionId) {
    const j = this.rxnIndex.get(reactionId);
    if (j === undefined) return null;
    return { lower: this.lb[j], upper: this.ub[j] };
  }

  /** Snapshot of all bounds, for undo. */
  snapshotBounds() { return { lb: Float64Array.from(this.lb), ub: Float64Array.from(this.ub) }; }
  restoreBounds(snap) { this.lb.set(snap.lb); this.ub.set(snap.ub); return this; }

  /**
   * Which backend this problem will actually use. GLPK is faster at every size
   * we measured, so `auto` takes it whenever it loaded and falls back to the
   * bundled simplex only when it did not.
   */
  backend() {
    if (this.backendChoice === 'glpk') return 'glpk';
    if (this.backendChoice === 'simplex') return 'simplex';
    return glpkAvailable() ? 'glpk' : 'simplex';
  }

  /** Dispatch one LP to the selected backend. */
  _solve(spec) {
    if (this.backend() === 'glpk') {
      return solveWithGLPK(spec, { rows: spec.rowNames, cols: spec.colNames });
    }
    return solveLP(spec, {});
  }

  /**
   * Solve the current problem. Returns the status, objectiveValue, and Maps of
   * fluxes and reduced costs by reaction id and shadow prices by metabolite id
   * (empty unless optimal).
   */
  optimize({ maximize = true, objective = null } = {}) {
    if (objective) this.setObjective(objective);
    const res = this._solve({
      A: this.A, b: this.b, c: this.c, lb: this.lb, ub: this.ub, maximize,
      rowNames: this.metIds, colNames: this.rxnIds,
    });
    return this._wrap(res);
  }

  _wrap(res) {
    const fluxes = new Map();
    if (res.status === LP_OPTIMAL) {
      this.rxnIds.forEach((rid, j) => fluxes.set(rid, res.x[j]));
    }
    const shadowPrices = new Map();
    if (res.status === LP_OPTIMAL && res.duals) {
      this.metIds.forEach((mid, i) => shadowPrices.set(mid, res.duals[i]));
    }
    const reducedCosts = new Map();
    if (res.status === LP_OPTIMAL && res.reducedCosts) {
      this.rxnIds.forEach((rid, j) => reducedCosts.set(rid, res.reducedCosts[j]));
    }
    const reducedCostScale = Math.max(0, ...[...reducedCosts.values()].map((v) => Math.abs(v)));
    const scaledReducedCosts = new Map([...reducedCosts].map(([rid, value]) => [
      rid, reducedCostScale > 0 ? value / reducedCostScale : 0,
    ]));
    return {
      status: res.status,
      objectiveValue: res.objective,
      fluxes,
      shadowPrices,
      reducedCosts,
      scaledReducedCosts,
      reducedCostScale,
      dualAnalysis: res.status === LP_OPTIMAL ? { available:true, source:'optimal LP basis', interpretation:'primary objective' } : { available:false },
      iterations: res.iterations,
      message: res.message,
    };
  }

  /**
   * Exact loopless FBA using the standard thermodynamic MILP formulation.
   *
   * Internal reversible reactions receive a binary direction variable. Chemical
   * potential variables enforce a feasible sign of ΔG for every active internal
   * direction. Boundary exchanges, sinks, biomass and the virtual community
   * growth link are excluded from the thermodynamic constraints.
   *
   * This is substantially more expensive than LP FBA because it is a MILP.
   */
  loopless({
    maximize = true,
    epsilon = 1,
    thermodynamicBigM = 1000,
    nodeLimit = 20000,
    timeLimitMs = 8000,
  } = {}) {
    const built = this._looplessProblem({
      maximize,
      epsilon,
      thermodynamicBigM,
    });

    const result = solveBinaryMILP(
      built.problem,
      built.binaryIndices,
      { nodeLimit, timeLimitMs },
    );

    const fluxes = new Map();
    if (result.x) {
      this.rxnIds.forEach((rid, j) => {
        const value = result.x[j];
        fluxes.set(rid, Math.abs(value) < 1e-9 ? 0 : value);
      });
    }

    let objectiveValue = NaN;
    if (result.x) {
      objectiveValue = 0;
      for (let j = 0; j < this.rxnIds.length; j++) objectiveValue += this.c[j] * result.x[j];
    }

    return {
      status: result.status,
      objectiveValue,
      fluxes,
      shadowPrices: new Map(),
      reducedCosts: new Map(),
      scaledReducedCosts: new Map(),
      reducedCostScale: null,
      dualAnalysis: { available:false, reason:'Dual values are not reported for the loopless MILP solution.' },
      iterations: result.iterations ?? 0,
      nodes: result.nodes ?? 0,
      binaryVariables: built.binaryIndices.length,
      looplessInternalReactions: built.internalCount,
      provenOptimal: result.provenOptimal === true,
      message: result.message ?? '',
    };
  }

  _looplessProblem({ maximize, epsilon, thermodynamicBigM }) {
    const n = this.rxnIds.length;
    const m = this.metIds.length;
    const internal = [];

    for (let j = 0; j < n; j++) {
      const rid = this.rxnIds[j];
      const rxn = this.model.reactions.get(rid);
      if (!rxn || !isLooplessInternalReaction(rxn, rid)) continue;
      const lower = this.lb[j];
      const upper = this.ub[j];
      if (Math.abs(lower) < 1e-12 && Math.abs(upper) < 1e-12) continue;

      const direction = lower < 0 && upper > 0
        ? 'reversible'
        : upper > 0
          ? 'forward'
          : lower < 0
            ? 'reverse'
            : 'blocked';
      if (direction !== 'blocked') internal.push({ j, rid, lower, upper, direction });
    }

    let inequalityRows = 0;
    for (const item of internal) inequalityRows += item.direction === 'reversible' ? 4 : 1;
    const rows = m + inequalityRows;
    const reversible = internal.filter((x) => x.direction === 'reversible');

    const fluxOffset = 0;
    const muOffset = n;
    const binaryOffset = muOffset + m;
    const slackOffset = binaryOffset + reversible.length;
    const cols = slackOffset + inequalityRows;

    const A = new SparseColumns(rows, cols);
    const b = new Float64Array(rows);
    const c = new Float64Array(cols);
    const lb = new Float64Array(cols);
    const ub = new Float64Array(cols);

    for (let j = 0; j < n; j++) {
      const col = this.A.column(j);
      for (let k = 0; k < col.length; k += 2) A.add(col[k], fluxOffset + j, col[k + 1]);
      lb[fluxOffset + j] = this.lb[j];
      ub[fluxOffset + j] = this.ub[j];
      c[fluxOffset + j] = this.c[j];
    }

    for (let i = 0; i < m; i++) {
      lb[muOffset + i] = -thermodynamicBigM;
      ub[muOffset + i] = thermodynamicBigM;
    }

    const binaryIndices = [];
    const binaryByReaction = new Map();
    reversible.forEach((item, index) => {
      const col = binaryOffset + index;
      binaryIndices.push(col);
      binaryByReaction.set(item.j, col);
      lb[col] = 0;
      ub[col] = 1;
    });

    let row = m;
    let slack = slackOffset;

    const addPotential = (targetRow, reactionIndex) => {
      const stoich = this.A.column(reactionIndex);
      for (let k = 0; k < stoich.length; k += 2) {
        const metaboliteRow = stoich[k];
        A.add(targetRow, muOffset + metaboliteRow, stoich[k + 1]);
      }
    };
    const addSlack = (targetRow, sign) => {
      A.add(targetRow, slack, sign);
      lb[slack] = 0;
      ub[slack] = Infinity;
      slack += 1;
    };

    for (const item of internal) {
      const { j, lower, upper, direction } = item;

      if (direction === 'reversible') {
        const y = binaryByReaction.get(j);

        // v <= U y
        A.add(row, fluxOffset + j, 1);
        A.add(row, y, -Math.max(upper, 0));
        addSlack(row, 1);
        b[row] = 0;
        row += 1;

        // v >= L (1-y)  =>  v + L y - s = L
        A.add(row, fluxOffset + j, 1);
        A.add(row, y, lower);
        addSlack(row, -1);
        b[row] = lower;
        row += 1;

        // ΔG <= M - (M+eps)y
        addPotential(row, j);
        A.add(row, y, thermodynamicBigM + epsilon);
        addSlack(row, 1);
        b[row] = thermodynamicBigM;
        row += 1;

        // ΔG >= eps - (M+eps)y
        addPotential(row, j);
        A.add(row, y, thermodynamicBigM + epsilon);
        addSlack(row, -1);
        b[row] = epsilon;
        row += 1;
      } else if (direction === 'forward') {
        // Fixed forward direction: ΔG <= -eps.
        addPotential(row, j);
        addSlack(row, 1);
        b[row] = -epsilon;
        row += 1;
      } else if (direction === 'reverse') {
        // Fixed reverse direction: ΔG >= eps.
        addPotential(row, j);
        addSlack(row, -1);
        b[row] = epsilon;
        row += 1;
      }
    }

    return {
      problem: { A, b, c, lb, ub, maximize },
      binaryIndices,
      internalCount: internal.length,
    };
  }

  /**
   * Parsimonious FBA: keep the optimal objective (within `fraction`) and then
   * minimise total flux. This often removes gratuitous cycle flux, but it does
   * not enforce thermodynamic looplessness. Use loopless() for that.
   */
  parsimonious({ fraction = 1.0, maximize = true } = {}) {
    const first = this.optimize({ maximize });
    if (first.status !== LP_OPTIMAL) return first;

    const target = first.objectiveValue * (maximize ? fraction : (2 - fraction));
    const n = this.rxnIds.length;
    const m = this.metIds.length;

    // Split every flux into forward/reverse halves so |v| becomes linear.
    const A = new SparseColumns(m + 1, 2 * n);
    const lb = new Float64Array(2 * n);
    const ub = new Float64Array(2 * n);
    const c = new Float64Array(2 * n);
    const b = new Float64Array(m + 1);

    for (let j = 0; j < n; j++) {
      const col = this.A.column(j);
      for (let k = 0; k < col.length; k += 2) {
        A.add(col[k], j, col[k + 1]);
        A.add(col[k], n + j, -col[k + 1]);
      }
      lb[j] = Math.max(0, this.lb[j]);
      ub[j] = Math.max(0, this.ub[j]);
      lb[n + j] = Math.max(0, -this.ub[j]);
      ub[n + j] = Math.max(0, -this.lb[j]);
      c[j] = 1;
      c[n + j] = 1;
      if (this.c[j]) {
        A.add(m, j, this.c[j]);
        A.add(m, n + j, -this.c[j]);
      }
    }
    b[m] = target;

    // The objective row is an equality at `target`. With fraction < 1 that is a
    // deliberate choice: hit exactly that share of the optimum.
    const res = this._solve({
      A, b, c, lb, ub, maximize: false,
      rowNames: [...this.metIds, 'pfba_objective'],
      colNames: [...this.rxnIds.map((r) => `${r}#fwd`), ...this.rxnIds.map((r) => `${r}#rev`)],
    });
    if (res.status !== LP_OPTIMAL) return first;

    const fluxes = new Map();
    this.rxnIds.forEach((rid, j) => {
      const v = res.x[j] - res.x[n + j];
      fluxes.set(rid, Math.abs(v) < 1e-9 ? 0 : v);
    });

    let objectiveValue = 0;
    this.rxnIds.forEach((rid, j) => { objectiveValue += this.c[j] * fluxes.get(rid); });

    return {
      status: LP_OPTIMAL,
      objectiveValue,
      fluxes,
      shadowPrices: first.shadowPrices,
      reducedCosts: first.reducedCosts,
      scaledReducedCosts: first.scaledReducedCosts,
      reducedCostScale: first.reducedCostScale,
      dualAnalysis: { available:true, source:'primary FBA optimum', interpretation:'primary objective before parsimonious secondary minimization' },
      iterations: first.iterations + res.iterations,
      message: '',
      totalFlux: res.objective,
    };
  }

  /**
   * Flux variability for a subset of reactions while holding the objective at
   * `fraction` of its optimum.
   * @param {string[]} reactionIds
   */
  variability(reactionIds, { fraction = 1.0 } = {}) {
    const base = this.optimize();
    const out = new Map();
    if (base.status !== LP_OPTIMAL) return out;

    const m = this.metIds.length;
    const n = this.rxnIds.length;
    const A = new SparseColumns(m + 1, n);
    const b = new Float64Array(m + 1);
    for (let j = 0; j < n; j++) {
      const col = this.A.column(j);
      for (let k = 0; k < col.length; k += 2) A.add(col[k], j, col[k + 1]);
      if (this.c[j]) A.add(m, j, this.c[j]);
    }
    b[m] = base.objectiveValue * fraction;

    // Relax the objective row into a range [fraction·opt, opt] using a slack.
    const slackLow = Math.min(0, base.objectiveValue * fraction - base.objectiveValue);
    const slackHigh = Math.max(0, base.objectiveValue - base.objectiveValue * fraction);
    const slackCol = A.pushColumn([m, 1]);
    const lb = new Float64Array(n + 1);
    const ub = new Float64Array(n + 1);
    lb.set(this.lb.subarray(0, n));
    ub.set(this.ub.subarray(0, n));
    lb[slackCol] = slackLow;
    ub[slackCol] = slackHigh;

    const c = new Float64Array(n + 1);
    for (const rid of reactionIds) {
      const j = this.rxnIndex.get(rid);
      if (j === undefined) continue;
      c.fill(0);
      c[j] = 1;
      const spec = {
        A, b, c, lb, ub,
        rowNames: [...this.metIds, 'fva_objective'],
        colNames: [...this.rxnIds, 'fva_slack'],
      };
      const hi = this._solve({ ...spec, maximize: true });
      const lo = this._solve({ ...spec, maximize: false });
      out.set(rid, {
        min: lo.status === LP_OPTIMAL ? lo.objective : NaN,
        max: hi.status === LP_OPTIMAL ? hi.objective : NaN,
      });
    }
    return out;
  }

  /**
   * Progressive FVA for large models. Uses the same mathematical formulation as
   * variability(), but yields to the browser every `chunkSize` reactions so the
   * UI remains responsive.
   *
   * @param {string[]} reactionIds
   * @param {object} opts
   * @param {number} [opts.fraction=1]
   * @param {number} [opts.chunkSize=8]
   * @param {(partial:Map<string,{min:number,max:number}>,done:number,total:number)=>void} [opts.onProgress]
   * @param {()=>boolean} [opts.shouldContinue]
   */
  async variabilityAsync(reactionIds, {
    fraction = 1.0,
    chunkSize = 8,
    onProgress = null,
    shouldContinue = null,
  } = {}) {
    const base = this.optimize();
    const out = new Map();
    if (base.status !== LP_OPTIMAL) return out;

    const m = this.metIds.length;
    const n = this.rxnIds.length;
    const A = new SparseColumns(m + 1, n);
    const b = new Float64Array(m + 1);
    for (let j = 0; j < n; j++) {
      const col = this.A.column(j);
      for (let k = 0; k < col.length; k += 2) A.add(col[k], j, col[k + 1]);
      if (this.c[j]) A.add(m, j, this.c[j]);
    }
    b[m] = base.objectiveValue * fraction;

    const slackLow = Math.min(0, base.objectiveValue * fraction - base.objectiveValue);
    const slackHigh = Math.max(0, base.objectiveValue - base.objectiveValue * fraction);
    const slackCol = A.pushColumn([m, 1]);
    const lb = new Float64Array(n + 1);
    const ub = new Float64Array(n + 1);
    lb.set(this.lb.subarray(0, n));
    ub.set(this.ub.subarray(0, n));
    lb[slackCol] = slackLow;
    ub[slackCol] = slackHigh;

    const ids = reactionIds.filter((rid) => this.rxnIndex.has(rid));
    const c = new Float64Array(n + 1);
    const size = Math.max(1, Number(chunkSize) || 8);

    for (let i = 0; i < ids.length; i++) {
      if (shouldContinue && !shouldContinue()) break;

      const rid = ids[i];
      const j = this.rxnIndex.get(rid);
      c.fill(0);
      c[j] = 1;

      const hi = solveLP({ A, b, c, lb, ub, maximize: true }, {});
      const lo = solveLP({ A, b, c, lb, ub, maximize: false }, {});
      out.set(rid, {
        min: lo.status === LP_OPTIMAL ? lo.objective : NaN,
        max: hi.status === LP_OPTIMAL ? hi.objective : NaN,
      });

      const done = i + 1;
      if (onProgress && (done % size === 0 || done === ids.length)) {
        onProgress(out, done, ids.length);
      }
      if (done % size === 0 && done < ids.length) await yieldFrame();
    }

    return out;
  }

  /** Objective value with one reaction forced to zero. */
  knockout(reactionId) {
    const snap = this.snapshotBounds();
    this.setBounds(reactionId, 0, 0);
    const res = this.optimize();
    this.restoreBounds(snap);
    return res;
  }
}

function isLooplessInternalReaction(reaction, reactionId) {
  if (!reaction) return false;
  if (reactionId === 'COMMUNITY_GROWTH') return false;
  if ((reaction.compartments ?? []).includes('link')) return false;
  return reaction.kind === 'metabolic' || reaction.kind === 'transport';
}

function yieldFrame() {
  return new Promise((resolve) => {
    if (typeof requestAnimationFrame === 'function') requestAnimationFrame(() => resolve());
    else setTimeout(resolve, 0);
  });
}

function defaultObjective(model) {
  const objective = model.objectiveReactions();
  if (objective.length) {
    const out = {};
    for (const { id, coefficient } of objective) out[id] = coefficient;
    return out;
  }
  const biomass = model.biomassReactionId();
  return biomass ? { [biomass]: 1 } : {};
}

/** One-shot FBA: a FluxProblem built with `opts`, optimised once. */
export function fba(model, opts = {}) {
  return new FluxProblem(model, opts).optimize();
}

/** One-shot pFBA wrapper. */
export function pfba(model, opts = {}) {
  return new FluxProblem(model, opts).parsimonious(opts);
}

/** One-shot loopless FBA wrapper. */
export function looplessFba(model, opts = {}) {
  return new FluxProblem(model, opts).loopless(opts);
}

export { LP_OPTIMAL };
