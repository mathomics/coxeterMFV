/**
 * GLPK backend.
 *
 * The bundled simplex in simplex.js keeps the basis inverse dense, which is fine
 * for the core-scale models this project draws but scales as roughly m^3.5: a
 * genome-scale model such as iAF1260 (1668 metabolites) needs minutes, and a
 * nine-species community is out of reach entirely. GLPK uses a sparse LU
 * factorisation and solves the same problems in about 100 ms.
 *
 * This is the same solver EscherFBA uses (`glpk.js`, the GLPK port to
 * WebAssembly). We load the direct build rather than the Worker-backed default
 * export, because the direct build solves synchronously: only instantiating the
 * WASM module is asynchronous, so the rest of the pipeline stays synchronous.
 *
 * The backend is optional. When it is not installed, or has not been
 * initialised, FluxProblem falls back to the bundled simplex, so the project
 * still runs with no dependencies at all.
 */

import { LP_OPTIMAL, LP_INFEASIBLE, LP_UNBOUNDED, LP_ITERATION_LIMIT } from './simplex.js';

/** Resolved GLPK instance, or null when unavailable. */
let glpk = null;
let initPromise = null;
let unavailableReason = '';

/**
 * Load GLPK once. Safe to call repeatedly and from several places at once.
 * @returns {Promise<boolean>} whether the backend is usable
 */
export function initGLPK() {
  if (glpk) return Promise.resolve(true);
  if (initPromise) return initPromise;

  initPromise = (async () => {
    for (const load of loaders()) {
      try {
        const { factory, wasmBinary } = await load();
        if (typeof factory !== 'function') continue;
        // In a browser the Emscripten glue would try to fetch the .wasm
        // synchronously, which is not allowed. Handing it the bytes up front is
        // the documented way around that.
        const instance = await factory(wasmBinary ? { wasmBinary } : undefined);
        // The Worker-backed build returns a Promise from solve(), and the
        // callers expect a synchronous result.
        if (typeof instance?.solve !== 'function') continue;
        glpk = instance;
        return true;
      } catch (error) {
        unavailableReason = error?.message ?? String(error);
      }
    }
    return false;
  })();

  return initPromise;
}

/**
 * Ways to reach GLPK, in order of preference:
 *   1. a factory the page already put on the global, which is how the
 *      single-file build ships it with the WASM inlined as a data URL
 *   2. the npm package, for Node
 *   3. the vendored copy next to the app, for the browser without a bundler
 */
function loaders() {
  const out = [];

  if (typeof globalThis !== 'undefined' && typeof globalThis.__GLPK_FACTORY__ === 'function') {
    out.push(async () => ({
      factory: globalThis.__GLPK_FACTORY__,
      wasmBinary: globalThis.__GLPK_WASM__ ?? null,
    }));
  }

  out.push(async () => ({ factory: (await import(/* @vite-ignore */ 'glpk.js/node')).default }));

  if (typeof document !== 'undefined' || typeof WorkerGlobalScope !== 'undefined') {
    out.push(async () => {
      const base = new URL('../../assets/vendor/', import.meta.url);
      const mod = await import(/* @vite-ignore */ new URL('glpk.js', base).href);
      const res = await fetch(new URL('glpk.wasm', base).href);
      if (!res.ok) throw new Error(`glpk.wasm: HTTP ${res.status}`);
      return { factory: mod.default ?? mod, wasmBinary: await res.arrayBuffer() };
    });
  }

  return out;
}

/** Whether initGLPK() has loaded a usable GLPK. */
export function glpkAvailable() { return glpk !== null; }
/** The last error met while loading GLPK, or '' when there was none. */
export function glpkUnavailableReason() { return unavailableReason; }

/** Status codes GLPK reports, mapped onto the ones simplex.js already uses. */
function mapStatus(code) {
  if (!glpk) return LP_INFEASIBLE;
  switch (code) {
    case glpk.GLP_OPT: return LP_OPTIMAL;
    // A feasible point does not prove that an objective has reached its maximum.
    // In particular, capped SteadyCom must not infer impossibility from an
    // unfinished maximisation of total biomass.
    case glpk.GLP_FEAS: return LP_ITERATION_LIMIT;
    case glpk.GLP_NOFEAS: return LP_INFEASIBLE;
    // An infeasible current basis is not proof that the LP has no solution.
    case glpk.GLP_INFEAS: return LP_ITERATION_LIMIT;
    case glpk.GLP_UNBND: return LP_UNBOUNDED;
    case glpk.GLP_UNDEF: return LP_ITERATION_LIMIT;
    default: return LP_ITERATION_LIMIT;
  }
}

/**
 * Solve a problem in the same shape solveLP() takes. Throws when GLPK has not
 * been initialised.
 *
 * @param {object} problem  { A: SparseColumns, b, c, lb, ub, maximize,
 *                            timeLimitSeconds, rowLower, rowUpper, presolve,
 *                            binaries }, the last four optional: row ranges
 *                          that replace `b`, presolve (default true), and the
 *                          column indices restricted to 0 or 1, which make it
 *                          a MILP solved by GLPK's branch and cut
 * @param {object} [names]  { rows: string[], cols: string[] } for readable duals
 * @returns {{status:string, objective:number, x:Float64Array|null, duals:Float64Array|null, message:string}}
 */
export function solveWithGLPK(problem, names = {}) {
  if (!glpk) throw new Error(`GLPK backend not initialised: ${unavailableReason || 'call initGLPK() first'}`);

  const m = problem.A.rows;
  const n = problem.A.cols;
  const colNames = names.cols ?? Array.from({ length: n }, (_, j) => `v${j}`);
  const rowNames = names.rows ?? Array.from({ length: m }, (_, i) => `m${i}`);

  // GLPK's JSON interface is row-major, so invert the column-major matrix once.
  const rows = Array.from({ length: m }, () => []);
  for (let j = 0; j < n; j++) {
    const col = problem.A.column(j);
    for (let k = 0; k < col.length; k += 2) {
      rows[col[k]].push({ name: colNames[j], coef: col[k + 1] });
    }
  }

  const objectiveVars = [];
  for (let j = 0; j < n; j++) {
    const coef = problem.c[j];
    if (coef) objectiveVars.push({ name: colNames[j], coef });
  }

  // Rows are equalities (S·v = b) unless the caller asked for a range. Anything
  // beyond mass balance, such as SteadyCom's LB·X <= V <= UB·X or a community
  // uptake budget, needs inequalities, so `rowLower`/`rowUpper` override `b`
  // per row.
  const rowLower = problem.rowLower ?? null;
  const rowUpper = problem.rowUpper ?? null;

  const subjectTo = [];
  for (let i = 0; i < m; i++) {
    if (!rows[i].length) continue;             // empty row constrains nothing
    const rhs = problem.b[i] ?? 0;
    const lo = rowLower ? rowLower[i] : rhs;
    const hi = rowUpper ? rowUpper[i] : rhs;
    const loFinite = Number.isFinite(lo);
    const hiFinite = Number.isFinite(hi);
    let type = glpk.GLP_FR;
    if (loFinite && hiFinite) type = lo === hi ? glpk.GLP_FX : glpk.GLP_DB;
    else if (loFinite) type = glpk.GLP_LO;
    else if (hiFinite) type = glpk.GLP_UP;
    subjectTo.push({
      name: rowNames[i],
      vars: rows[i],
      bnds: { type, lb: loFinite ? lo : 0, ub: hiFinite ? hi : 0 },
    });
  }

  const bounds = [];
  for (let j = 0; j < n; j++) {
    const lo = problem.lb[j];
    const hi = problem.ub[j];
    const loFinite = Number.isFinite(lo);
    const hiFinite = Number.isFinite(hi);
    let type = glpk.GLP_FR;
    if (loFinite && hiFinite) type = lo === hi ? glpk.GLP_FX : glpk.GLP_DB;
    else if (loFinite) type = glpk.GLP_LO;
    else if (hiFinite) type = glpk.GLP_UP;
    bounds.push({ name: colNames[j], type, lb: loFinite ? lo : 0, ub: hiFinite ? hi : 0 });
  }

  const lp = {
    name: 'fba',
    objective: {
      direction: problem.maximize === false ? glpk.GLP_MIN : glpk.GLP_MAX,
      name: 'z',
      vars: objectiveVars,
    },
    subjectTo,
    bounds,
  };
  const binaries = problem.binaries ?? [];
  if (binaries.length) lp.binaries = binaries.map((j) => colNames[j]);

  // A time limit, because a degenerate LP can cycle in the simplex and never come
  // back, and one such LP should not hang the whole program. GLPK returns
  // whatever it has when the limit expires, which will not be GLP_OPT, so the
  // caller sees a non-optimal status and can decide. That is a bounded failure
  // it can report or work around, which an infinite wait is not.
  const options = { msglev: glpk.GLP_MSG_OFF, presol: problem.presolve !== false };
  if (problem.timeLimitSeconds) options.tmlim = problem.timeLimitSeconds;

  // The presolver is what makes GLPK quick on well-scaled problems, but on a
  // badly scaled one it gives up and reports GLP_UNDEF, "no solution", which is
  // not the same as "infeasible" and must never be read as one. The plain simplex
  // usually gets through, so retry once without the presolver before believing it.
  // A MILP that ran out of time without an integer point is also GLP_UNDEF, and
  // retrying it would only double the wait.
  let result = glpk.solve(lp, options);
  if (result?.result?.status === glpk.GLP_UNDEF && !binaries.length) {
    result = glpk.solve(lp, { ...options, presol: false });
  }

  const status = mapStatus(result?.result?.status);
  // A MILP stopped by its time limit keeps the best integer point it found.
  const incumbent = binaries.length > 0 && result?.result?.status === glpk.GLP_FEAS;
  if (status !== LP_OPTIMAL && !incumbent) {
    return { status, objective: NaN, x: null, duals: null, message: `GLPK status ${result?.result?.status}` };
  }

  const vars = result.result.vars;
  const x = new Float64Array(n);
  for (let j = 0; j < n; j++) x[j] = vars[colNames[j]] ?? 0;

  const dualMap = result.result.dual ?? {};
  const duals = new Float64Array(m);
  for (let i = 0; i < m; i++) duals[i] = dualMap[rowNames[i]] ?? 0;

  return { status, objective: result.result.z, x, duals, message: '' };
}

/**
 * Measured on this project's own models, GLPK is faster at every size, from a
 * 58-metabolite single cell (7 ms vs 10 ms) to a five-species community
 * (pFBA: 10 ms vs 137 ms). There is therefore no size threshold: `auto` uses
 * GLPK whenever it loaded, and the bundled simplex exists as a genuine fallback
 * for when it did not, not as a choice for small problems.
 */
export const BACKEND_SELECTION = 'glpk-when-available';
