/**
 * SteadyCom: maximum community growth rate at community steady state.
 *
 * Chan SHJ, Simons MN, Maranas CD (2017) SteadyCom: Predicting microbial
 * abundances while ensuring community stability. PLoS Comput Biol 13(5):
 * e1005539. https://doi.org/10.1371/journal.pcbi.1005539
 *
 * The formulation the paper states is
 *
 *     max  mu
 *     s.t. sum_j S^k_ij V^k_j = 0                  for each metabolite i of member k
 *          LB^k_j X^k <= V^k_j <= UB^k_j X^k       for each reaction j of member k
 *          sum_k V^k_ex(i) + u^com_i >= 0          for each community metabolite i
 *          V^k_biomass = X^k mu                    for each member k
 *          sum_k X^k = 1
 *          X^k, mu >= 0
 *
 * which is not an LP, because `X^k mu` multiplies two variables. It becomes one
 * the moment mu is fixed, and the maximum total biomass is non-increasing in mu,
 * so the paper solves a sequence of LPs and brackets the largest mu that still
 * supports the required total biomass. That is what this module does: bisection
 * on mu, with each step an ordinary LP.
 *
 * Note on units, because it is the easiest thing to get wrong here: V is an
 * absolute rate (mmol/h) for the whole population of that member, not a specific
 * rate, which is why the bounds are scaled by X^k. Divide by X^k to recover the
 * per-gDW fluxes an individual FBA would report.
 */

import { SparseColumns, LP_INFEASIBLE, LP_OPTIMAL } from '../solver/simplex.js';
import { glpkAvailable, solveWithGLPK } from '../solver/glpk.js';

/**
 * @typedef {object} SteadyComMember
 * @property {string} id
 * @property {import('./model.js').Model} model
 * @property {string} biomassReaction        id within that member's model
 * @property {Object<string,number>} [uptake] community metabolite -> max uptake
 *                                            rate for this member (positive)
 */

/**
 * How close the capped bisection is allowed to get to the maximum growth rate.
 * See the note in steadyCom(): nearer than this the LP can cycle in GLPK and
 * never return, and glpk.js cannot interrupt it.
 */
const CAPPED_FLOOR = 1e-5;

/**
 * The maximum community growth rate and the abundances and fluxes that reach it,
 * by bisection on mu. Needs the GLPK backend.
 *
 * @param {object} spec
 * @param {SteadyComMember[]} spec.members
 * @param {Object<string,number>} spec.communityUptake  base metabolite -> budget
 *        available to the whole community (positive mmol/h per gDW of community)
 * @param {number} [spec.totalBiomass=1]
 * @param {object} [options] tolerance (1e-6, in 1/h), maxIterations (60 LPs),
 *        upperGuess (2 /h), biomassCap (null: fixed-total formulation),
 *        timeLimitSeconds (600 per LP) and onProgress({iterations, lo, hi, mu})
 * @returns {{status:string, growthRate:number, abundances:Map<string,number>,
 *            fluxes:Map<string,number>, iterations:number, message:string}}
 *          plus the verified range and resolution when optimal
 */
export function steadyCom(spec, options = {}) {
  const {
    tolerance = 1e-6,
    maxIterations = 60,
    upperGuess = 2.0,
    biomassCap = null,
    timeLimitSeconds = 600,
    onProgress = null,
  } = options;

  validateSpec(spec);
  positiveNumber(tolerance, 'Growth-rate tolerance');
  positiveNumber(upperGuess, 'Initial upper growth-rate guess');
  positiveNumber(timeLimitSeconds, 'LP time limit');
  if (!Number.isInteger(maxIterations) || maxIterations < 1) {
    throw new Error('The maximum LP count must be a positive integer.');
  }
  if (onProgress !== null && typeof onProgress !== 'function') {
    throw new Error('The progress callback must be a function.');
  }

  if (!glpkAvailable()) {
    return fail('SteadyCom needs the GLPK backend: the problem uses inequality '
      + 'rows and reaches genome scale. Call initGLPK() first.',
    { status: 'solver-error', zeroFeasible: null });
  }

  // With a cap, each step maximises the total biomass and the bisection compares
  // what it reached against what is required. Without one, each step asks
  // directly whether the required total is feasible. Same question, and the same
  // answer. The difference is cost. On a nine-species genome-scale community,
  // proving one LP infeasible took fifteen to twenty-six seconds against two and
  // a half for a feasible one, and half of a bisection's steps are infeasible by
  // construction. Capping turns every step into the cheap case.
  const cap = biomassCap ?? spec.biomassCap ?? null;
  validateCap(cap, spec.totalBiomass ?? 1);
  const problem = buildAny(cap === null ? spec : { ...spec, biomassCap: cap });
  problem.timeLimitSeconds = timeLimitSeconds;
  const biomassObjective = cap === null ? null : {
    entries: [...problem.biomassCols.values()].map((col) => [col, 1]),
    maximize: true,
  };

  // The capped formulation is the cheap one, but it is an optimisation, and
  // within a whisker of the maximum growth rate the problem is degenerate enough
  // that GLPK's simplex can cycle and never return. Measured on a two-member test
  // community whose maximum is exactly 5 /h: every rate down to mu* + 2^-18
  // solved in under ten milliseconds, and mu* + 2^-19 never came back. glpk.js
  // exposes no time limit that interrupts it, so a cycling solve cannot be
  // recovered from inside the process. It has to be avoided.
  //
  // Avoided means not asked. The bisection stops refining at CAPPED_FLOOR rather
  // than probing arbitrarily close to the edge, so it never reaches the rates
  // that stall. The growth rate is then known to 1e-5, which is still finer than
  // anything these papers report, and a caller that wants the last decimals can
  // leave the cap off and pay for the exact formulation instead.
  //
  // Running the last steps on the fixed formulation instead is much worse: on the
  // nine-species community its infeasible solves cost more than the entire capped
  // bisection, and points that finish in five minutes with the cap alone did not
  // finish at all.
  const effectiveTolerance = cap === null ? tolerance : Math.max(tolerance, CAPPED_FLOOR);
  let fallback = null;
  const fixedProblem = () => {
    if (!fallback) {
      fallback = buildAny({ ...spec, biomassCap: null });
      fallback.timeLimitSeconds = timeLimitSeconds;
      fallback.growthRowScale = 1000;
    }
    return fallback;
  };

  // This search covers a range starting at zero only after checking that zero
  // is feasible. A positive biomass lower bound can exclude zero even when
  // positive growth is possible. That is a failed search premise, not a verdict
  // that the community cannot grow at any rate.
  let lo = 0;
  let hi = upperGuess;
  let iterations = 0;
  let failure = null;
  let failureStatus = 'solver-error';
  const run = (inner, mu, objective) => {
    if (iterations >= maxIterations) {
      failureStatus = 'iteration-limit';
      failure = 'The LP count limit was reached before the growth-rate range was verified.';
      return null;
    }
    const result = solveAt(inner, mu, spec.totalBiomass ?? 1, objective);
    iterations += result.lpEvaluations ?? 1;
    onProgress?.({ iterations, lo, hi, mu });
    return result;
  };

  // A step only narrows the bracket when the solver actually answered. GLPK
  // reports GLP_UNDEF ("no solution available") when it gives up on a badly
  // scaled problem, and reading that as "the community cannot grow this fast"
  // walks the bisection down to zero and returns it as an optimal answer.
  //
  // What counts as an answer differs between the two formulations, and conflating
  // them is its own bug. Fixed: the verdict IS feasibility, so a step reporting
  // infeasible has answered and anything else has not. Capped: the LP is always
  // feasible (every member can simply be absent), and the verdict is the
  // objective value, so an optimal solve has answered even when the total it
  // reached falls short of the one required. Requiring infeasibility there would
  // reject every growth rate above the maximum as a solver failure.

  const step = (mu) => {
    const capped = cap !== null;

    let res = run(problem, mu, biomassObjective);
    if (!res) return null;

    const ok = capped ? res.status === LP_OPTIMAL : (res.feasible || res.status === LP_INFEASIBLE);
    if (ok) return res;

    // The capped solve did not come back with an answer. Ask the same question
    // the other way before giving up on this step.
    if (capped) {
      res = run(fixedProblem(), mu, null);
      if (!res) return null;
      if (res.feasible || res.status === LP_INFEASIBLE) return res;
    }

    failure = `the LP at mu = ${mu} did not solve (${res.status}: ${res.message}). `
      + 'The result would be meaningless, so no growth rate is reported.';
    return null;
  };

  const failed = (message = failure, status = failureStatus, extra = {}) => fail(message, {
    status, iterations, zeroFeasible: null, ...extra,
  });
  const feasibleAtZero = step(0);
  if (!feasibleAtZero) return failed();
  if (!feasibleAtZero.feasible) {
    return failed('The range search requires a feasible solution at µ = 0, but zero growth '
      + 'cannot sustain the required biomass. Check the medium and positive lower bounds; '
      + 'a fixed solve at a positive growth rate may still be feasible.',
    'zero-infeasible', { zeroFeasible: false });
  }

  let best = feasibleAtZero;
  let bracketed = false;
  for (let i = 0; i < 20; i++) {
    const res = step(hi);
    if (!res) return failed(failure, failureStatus, { zeroFeasible: true });
    if (!res.feasible) { bracketed = true; break; }
    lo = hi;
    best = res;
    hi *= 2;
    if (!Number.isFinite(hi)) break;
  }
  if (!bracketed) {
    return failed('No finite upper growth-rate bound was established. The maximum may '
      + 'be unbounded, or the initial guess needs adjustment; no optimum is reported.',
    'iteration-limit', { zeroFeasible: true });
  }

  while (hi - lo > effectiveTolerance && iterations < maxIterations) {
    const mid = (lo + hi) / 2;
    if (!(mid > lo && mid < hi)) break;
    const res = step(mid);
    if (!res) return failed(failure, failureStatus, { zeroFeasible: true });
    if (res.feasible) { lo = mid; best = res; } else { hi = mid; }
  }
  if (hi - lo > effectiveTolerance) {
    return failed('The search stopped before its requested growth-rate resolution was '
      + 'reached; no verified maximum is reported.', 'iteration-limit', { zeroFeasible: true });
  }

  // Certify the usable endpoint with fixed total biomass and tighter growth
  // equalities. Tiny populations can otherwise hide a biomass error inside
  // GLPK's absolute row tolerance, which becomes large after dividing by X.
  for (let attempt = 0; attempt < 3; attempt++) {
    const verified = run(fixedProblem(), lo, null);
    if (!verified) return failed(failure, failureStatus, { zeroFeasible: true });
    if (verified.status === LP_OPTIMAL && verified.feasible) { best = verified; break; }
    if (verified.status !== LP_INFEASIBLE) {
      return failed(`The fixed-biomass endpoint at µ = ${lo} was not verified (${verified.status}).`, 'solver-error', { zeroFeasible: true });
    }
    if (attempt === 2 || lo === 0) return failed('No numerically verified endpoint was found.', 'solver-error', { zeroFeasible: true });
    lo = Math.max(0, lo - effectiveTolerance);
  }

  // The capped formulation maximises the total, so at a growth rate just below
  // the maximum it comes back with more biomass than was asked for, and
  // abundances that sum to that, not to one. Scaling the whole solution down is
  // exact rather than cosmetic: every constraint here is homogeneous in (V, X)
  // except the community exchange bounds, and those are intervals containing
  // zero, so shrinking a feasible solution leaves it feasible. The paper's
  // abundances sum to the total biomass by equation (15), and so do these.
  const achieved = best.total ?? (spec.totalBiomass ?? 1);
  const scale = achieved > 0 ? (spec.totalBiomass ?? 1) / achieved : 1;

  return {
    status: 'optimal',
    growthRate: lo,
    rangeLower: 0,
    rangeUpper: lo,
    upperBound: hi,
    zeroFeasible: true,
    resolution: Math.max(effectiveTolerance, hi - lo),
    abundances: scaled(best.abundances, scale),
    fluxes: scaled(best.fluxes, scale),
    iterations,
    message: '',
  };
}

/**
 * The inner LP on its own: the largest total biomass the medium supports if every
 * member grows at exactly `mu`. SteadyCom's answer is the largest mu at which this
 * still reaches the required total, so this is the function being bisected. That
 * makes it the thing to look at when an answer is surprising, and the thing a test
 * can assert against directly.
 *
 * @param {object} spec  same shape steadyCom() takes
 * @param {number} mu
 * @param {object} [options] biomassCap (null: total fixed), timeLimitSeconds (600)
 * @returns {{feasible:boolean, total:number, abundances:Map<string,number>,
 *            fluxes:Map<string,number>}}
 */
export function maxTotalBiomassAt(spec, mu, options = {}) {
  validateSpec(spec);
  if (!Number.isFinite(mu) || mu < 0) {
    throw new Error('The fixed growth rate must be a finite number of zero or more.');
  }
  const cap = options.biomassCap ?? spec.biomassCap ?? null;
  validateCap(cap, spec.totalBiomass ?? 1);
  const timeLimitSeconds = options.timeLimitSeconds ?? 600;
  positiveNumber(timeLimitSeconds, 'LP time limit');
  const problem = buildAny(cap === null ? spec : { ...spec, biomassCap: cap });
  problem.timeLimitSeconds = timeLimitSeconds;
  if (cap === null) problem.growthRowScale = 1000;
  const objective = cap === null ? null : {
    entries: [...problem.biomassCols.values()].map((col) => [col, 1]),
    maximize: true,
  };
  return solveAt(problem, mu, spec.totalBiomass ?? 1, objective);
}

/**
 * The most economical flux distribution among those that hold the community at
 * a fixed growth rate: parsimonious FBA, asked of SteadyCom.
 *
 * At a fixed mu the feasible set is a polytope, and a feasibility solve returns
 * whichever vertex the simplex reaches first. On the nine-species gut community
 * that vertex carried futile cycles at the flux bounds (a member at 0.15 % of
 * the biomass running a reaction at 1000 mmol/gDW/h) and loops that pass a
 * metabolite back and forth between two members, which no per-species loopless
 * constraint can see. Minimising the summed magnitude of every member flux picks
 * the cheapest of the solutions instead, the way pFBA does for one organism.
 *
 * The sum runs over member reactions only, in their absolute (X-scaled) units, so
 * a gram of community is charged the same for flux in any member. Community
 * uptake and export are not charged: they follow from the member exchanges by
 * mass balance. The composition is left free under the normalisation, so the
 * answer is the cheapest community at this growth rate. At the maximum growth
 * rate that composition is usually pinned anyway.
 *
 * @param {object} spec  same shape steadyCom() takes
 * @param {number} mu
 * @param {object} [options] timeLimitSeconds (600)
 * @returns {{feasible:boolean, total:number, abundances:Map<string,number>,
 *            fluxes:Map<string,number>, totalFlux:number, status:string}}
 */
export function parsimoniousAt(spec, mu, options = {}) {
  validateSpec(spec);
  if (!Number.isFinite(mu) || mu < 0) {
    throw new Error('The fixed growth rate must be a finite number of zero or more.');
  }
  const timeLimitSeconds = options.timeLimitSeconds ?? 600;
  positiveNumber(timeLimitSeconds, 'LP time limit');
  const problem = buildAny({ ...spec, biomassCap: null });
  problem.timeLimitSeconds = timeLimitSeconds;
  problem.growthRowScale = 1000;
  const entries = absoluteFluxTerms(problem, spec);
  const result = solveAt(problem, mu, spec.totalBiomass ?? 1, { entries, maximize: false });
  return { ...result, totalFlux: result.status === LP_OPTIMAL ? result.objective : NaN };
}

/**
 * Objective terms whose sum is the total magnitude of the member fluxes.
 *
 * A reaction whose lower bound is at least zero can only carry V >= LB*X >= 0,
 * so |V| is V itself. One whose upper bound is at most zero has |V| = -V. Only a
 * reversible reaction needs an auxiliary t >= |V|, written as t - V >= 0 and
 * t + V >= 0, and minimising t makes it equal |V| at the optimum.
 */
function absoluteFluxTerms(problem, spec) {
  const communityTag = spec.communityTag ?? 'comm';
  const reactions = [];
  if (spec.reactions) {
    for (const [id, rxn] of spec.reactions) {
      if (rxn.species !== communityTag) reactions.push([problem.fluxCols.get(id), rxn]);
    }
  } else {
    for (const member of spec.members) {
      for (const [rid, rxn] of member.model.reactions) {
        reactions.push([problem.fluxCols.get(`${member.id}\u0000${rid}`), rxn]);
      }
    }
  }
  const entries = [];
  for (const [col, rxn] of reactions) {
    if (rxn.lowerBound >= 0) { entries.push([col, 1]); continue; }
    if (rxn.upperBound <= 0) { entries.push([col, -1]); continue; }
    const name = problem.cols[col].name;
    problem.cols.push({ name: `abs|${name}`, lb: 0, ub: Infinity });
    const t = problem.cols.length - 1;
    problem.rows.push({ name: `abs|${name}|+`, entries: [[t, 1], [col, -1]], lo: 0, hi: Infinity });
    problem.rows.push({ name: `abs|${name}|-`, entries: [[t, 1], [col, 1]], lo: 0, hi: Infinity });
    entries.push([t, 1]);
  }
  return entries;
}

/**
 * Two community shapes reach the same LP.
 *
 * `members[].model` + `exchangeFor` is the shape the four-auxotroph community
 * uses: separate single-organism models whose own exchange reactions are summed
 * into one community budget row per metabolite.
 *
 * `reactions` + `metabolites` is a joint model that already carries its
 * community structure: a shared `[u]` compartment with its own mass balance,
 * species-to-community exchange reactions and community-to-environment uptake
 * and export reactions. The nine-species gut model is distributed that way, and
 * it is also the formulation the paper's own code builds.
 */
function buildAny(spec) {
  return spec.reactions ? buildJointProblem(spec) : buildProblem(spec);
}

// Building

/**
 * Lay the community out once: one column per member reaction, one column per
 * member biomass variable X^k, and rows for mass balance, the bound coupling and
 * the community uptake budget. Only the few coefficients that depend on mu are
 * rewritten between bisection steps.
 */
function buildProblem(spec) {
  const members = spec.members;
  const cols = [];            // { name, lb, ub }
  const colIndex = new Map();
  const pushCol = (name, lb, ub) => {
    colIndex.set(name, cols.length);
    cols.push({ name, lb, ub });
    return cols.length - 1;
  };

  // Columns: member fluxes, then one biomass variable per member.
  const fluxCols = new Map();   // `${memberId}\u0000${rxnId}` -> column
  for (const member of members) {
    for (const rid of member.model.reactions.keys()) {
      // The absolute rate is unbounded here. The LB*X <= V <= UB*X rows do the
      // bounding, which is the whole point of the formulation.
      fluxCols.set(`${member.id}\u0000${rid}`, pushCol(`${member.id}|${rid}`, -Infinity, Infinity));
    }
  }
  // Bounded by the community total, or by the cap when the total is being
  // maximised instead of fixed. Leaving these free lets the LP drift to a total
  // of ten-odd gDW with fluxes to match, which is where GLPK returns GLP_UNDEF.
  const totalBiomass = spec.totalBiomass ?? 1;
  const biomassCap = spec.biomassCap ?? null;
  const biomassCols = new Map();
  for (const member of members) {
    biomassCols.set(member.id, pushCol(`X|${member.id}`, 0, biomassCap ?? totalBiomass));
  }

  const rows = [];            // { name, entries: [[col, coef]], lo, hi }
  const pushRow = (name, entries, lo, hi) => {
    rows.push({ name, entries, lo, hi });
    return rows.length - 1;
  };

  // Mass balance per member: S^k V^k = 0
  for (const member of members) {
    const perMet = new Map();
    for (const [rid, rxn] of member.model.reactions) {
      const col = fluxCols.get(`${member.id}\u0000${rid}`);
      for (const [mid, coef] of Object.entries(rxn.metabolites)) {
        if (!coef) continue;
        if (!perMet.has(mid)) perMet.set(mid, []);
        perMet.get(mid).push([col, coef]);
      }
    }
    for (const [mid, entries] of perMet) {
      pushRow(`${member.id}|${mid}`, entries, 0, 0);
    }
  }

  // Bound coupling: V - UB*X <= 0 and V - LB*X >= 0
  const couplingRows = [];
  for (const member of members) {
    const xCol = biomassCols.get(member.id);
    for (const [rid, rxn] of member.model.reactions) {
      const col = fluxCols.get(`${member.id}\u0000${rid}`);
      if (Number.isFinite(rxn.upperBound)) {
        couplingRows.push(pushRow(`${member.id}|${rid}|ub`,
          [[col, 1], [xCol, -rxn.upperBound]], -Infinity, 0));
      }
      if (Number.isFinite(rxn.lowerBound)) {
        couplingRows.push(pushRow(`${member.id}|${rid}|lb`,
          [[col, 1], [xCol, -rxn.lowerBound]], 0, Infinity));
      }
    }
  }

  // Community exchange space: sum_k V^k_ex(i) + u^com_i >= 0
  //
  // One row for EVERY metabolite any member can exchange, not only the ones
  // the medium supplies. A metabolite the medium does not provide gets a
  // budget of zero, which is exactly what forces cross-feeding: a member can
  // only take it up to the extent that another member secretes it. Without
  // the zero-budget rows each member draws from nowhere and an auxotroph
  // appears to grow alone.
  const exchangeBases = new Set();
  for (const member of members) {
    for (const base of Object.keys(member.exchangeFor ?? {})) exchangeBases.add(base);
  }

  const budgets = spec.communityUptake ?? {};
  for (const base of exchangeBases) {
    const entries = [];
    for (const member of members) {
      const rid = member.exchangeFor?.[base];
      if (!rid) continue;
      const col = fluxCols.get(`${member.id}\u0000${rid}`);
      if (col !== undefined) entries.push([col, 1]);
    }
    if (!entries.length) continue;
    const budget = Math.abs(budgets[base] ?? 0);
    const bounds = spec.communityExchangeBounds?.[base];
    pushRow(`community|${base}`, entries, bounds?.lower ?? -budget, bounds?.upper ?? Infinity);
  }

  // Normalisation: sum_k X^k = total biomass. Equation (15) of the paper, and
  // the reason the abundances it reports are relative.
  //
  // Fixing the total makes each bisection step a feasibility question. Capping
  // it instead makes the step an optimisation (maximise the total, then see
  // whether it reached the required one), which is the paper's own algorithm
  // and answers the same question far more cheaply, because proving a large LP
  // infeasible costs an order of magnitude more than solving a feasible one.
  // Either way every X stays bounded, which is what keeps the LP conditioned.
  // Letting them run free makes GLPK give up.
  pushRow('normalisation', [...biomassCols.values()].map((col) => [col, 1]),
    biomassCap === null ? totalBiomass : 0, biomassCap === null ? totalBiomass : biomassCap);

  // Growth coupling, rewritten per bisection step: V_biomass - mu*X = 0
  const growthRows = members.map((member) => pushRow(
    `growth|${member.id}`,
    [[fluxCols.get(`${member.id}\u0000${member.biomassReaction}`), 1],
      [biomassCols.get(member.id), 0]],
    0, 0,
  ));

  return { members, cols, rows, fluxCols, biomassCols, growthRows };
}

/**
 * Solve the LP for one fixed growth rate.
 *
 * With no objective this is a feasibility question (can the community carry the
 * required biomass at this growth rate?), which is what the bisection asks. With
 * one it becomes the FVA step: the same feasible set, minimised or maximised
 * over a linear target.
 *
 * @param {object} [objective] { entries: [[col, coef]], maximize }
 */
function solveAt(problem, mu, totalBiomass, objective = null) {
  const { members, cols, rows, growthRows, biomassCols, fluxCols } = problem;

  // Only the growth-coupling coefficient changes with mu.
  members.forEach((member, k) => {
    rows[growthRows[k]].entries[1][1] = -mu;
  });

  const m = rows.length;
  const n = cols.length;
  const A = new SparseColumns(m, n);
  const rowLower = new Float64Array(m);
  const rowUpper = new Float64Array(m);

  // Equilibrate each row: divide it by its largest coefficient. The bound
  // coupling contributes `V - 1000*X <= 0` while a biomass reaction contributes
  // stoichiometry around 1e-5, so the untouched matrix spans eight orders of
  // magnitude and GLPK, which exposes no scaling option of its own through
  // glpk.js, answers GLP_UNDEF instead of solving. Scaling a row changes nothing
  // about its solution set, exactly rather than approximately: every bound here
  // is either zero or scaled with the row. It does scale that row's dual by the
  // same factor, which is why nothing in this module reads duals.
  rows.forEach((row, i) => {
    let largest = 0;
    for (const [, coef] of row.entries) largest = Math.max(largest, Math.abs(coef));
    const scale = (largest > 0 ? 1 / largest : 1)
      * (row.name.startsWith('growth|') ? problem.growthRowScale ?? 1 : 1);
    for (const [col, coef] of row.entries) A.add(i, col, coef * scale);
    rowLower[i] = Number.isFinite(row.lo) ? row.lo * scale : row.lo;
    rowUpper[i] = Number.isFinite(row.hi) ? row.hi * scale : row.hi;
  });

  const lb = new Float64Array(n);
  const ub = new Float64Array(n);
  const c = new Float64Array(n);
  cols.forEach((col, j) => { lb[j] = col.lb; ub[j] = col.ub; });
  // Without an objective the total is already fixed by the normalisation row, so
  // there is nothing left to optimise: any feasible point is an answer and the
  // question is only whether one exists.
  for (const [col, coef] of objective?.entries ?? []) c[col] += coef;

  const res = solveWithGLPK(
    {
      A, b: new Float64Array(m), c, lb, ub, rowLower, rowUpper,
      maximize: objective ? objective.maximize !== false : true,
      timeLimitSeconds: problem.timeLimitSeconds,
      presolve: problem.presolve,
    },
    { rows: rows.map((r) => r.name), cols: cols.map((col) => col.name) },
  );

  if (res.status !== 'optimal') {
    return {
      feasible: false, total: NaN, status: res.status, message: res.message,
      abundances: new Map(), fluxes: new Map(),
    };
  }

  let total = 0;
  for (const col of biomassCols.values()) total += res.x[col];
  const feasible = total >= totalBiomass - 1e-7;

  const abundances = new Map();
  for (const [id, col] of biomassCols) abundances.set(id, res.x[col]);
  // A tiny population can meet its scaled growth row within GLPK's tolerance
  // while its specific growth V/X is off. Solve again without presolve, then
  // with the growth rows weighted more heavily, before giving up.
  if (problem.growthRowScale) {
    const inaccurate = members.some((member, index) => {
      const x = abundances.get(member.id);
      const biomassColumn = rows[growthRows[index]].entries[0][0];
      return x > 1e-9 && Math.abs(res.x[biomassColumn] / x - mu) > 1e-7;
    });
    if (inaccurate) {
      if (problem.presolve !== false) {
        problem.presolve = false;
        const refined = solveAt(problem, mu, totalBiomass, objective);
        return { ...refined, lpEvaluations: (refined.lpEvaluations ?? 1) + 1 };
      }
      if (problem.growthRowScale < 1e7) {
        problem.growthRowScale *= 100;
        const refined = solveAt(problem, mu, totalBiomass, objective);
        return { ...refined, lpEvaluations: (refined.lpEvaluations ?? 1) + 1 };
      }
      return { feasible: false, total: NaN, status: 'iteration_limit',
        message: 'The solver did not resolve the specific growth of tiny populations accurately enough.',
        abundances: new Map(), fluxes: new Map() };
    }
  }
  // Keys are `member\0reaction` when the members came in as separate models, and
  // a bare reaction id for a joint model whose reactions are already unique.
  const fluxes = new Map();
  for (const [key, col] of fluxCols) {
    const sep = key.indexOf('\u0000');
    fluxes.set(sep < 0 ? key : `${key.slice(0, sep)}|${key.slice(sep + 1)}`, res.x[col]);
  }

  return {
    feasible, total, status: res.status, message: '',
    objective: res.objective, abundances, fluxes,
  };
}

/** A solution rescaled by a constant, used to normalise the total biomass. */
function scaled(values, factor) {
  if (factor === 1) return values;
  const out = new Map();
  for (const [key, value] of values) out.set(key, value * factor);
  return out;
}

function fail(message, details = {}) {
  return {
    status: 'infeasible',
    growthRate: 0,
    abundances: new Map(),
    fluxes: new Map(),
    iterations: 0,
    message,
    ...details,
  };
}

function positiveNumber(value, name) {
  if (!Number.isFinite(value) || value <= 0) throw new Error(`${name} must be a finite positive number.`);
}

function validateCap(cap, total) {
  if (cap === null) return;
  positiveNumber(cap, 'The biomass cap');
  if (cap < total) throw new Error('The biomass cap cannot be below the required total biomass.');
}

function validateSpec(spec) {
  if (!spec || !Array.isArray(spec.members) || !spec.members.length) {
    throw new Error('A SteadyCom problem needs at least one member.');
  }
  positiveNumber(spec.totalBiomass ?? 1, 'Total community biomass');
  const ids = new Set();
  for (const member of spec.members) {
    if (!member?.id || ids.has(member.id)) throw new Error('SteadyCom member identifiers must be nonempty and unique.');
    ids.add(member.id);
    const reactions = spec.reactions ?? member.model?.reactions;
    if (!(reactions instanceof Map) || !reactions.has(member.biomassReaction)) {
      throw new Error(`Member ${member.id} has no valid biomass reaction ${member.biomassReaction ?? ''}.`);
    }
  }
}

// The joint-model formulation

/**
 * Lay out a community that already carries its own community compartment.
 *
 * The difference from buildProblem() is where the community lives. There, each
 * member keeps its own exchange reactions and a budget row sums them. Here the
 * shared medium is in the model: community metabolites have their own mass
 * balance, species-to-community exchange reactions move material in and out of
 * it, and community uptake and export reactions connect it to the environment.
 *
 * That changes which fluxes scale with abundance. A species reaction is a
 * specific rate and is bounded by LB*X <= V <= UB*X. A community exchange
 * reaction is an absolute community rate (the diet is so many mmol per hour for
 * the whole community, not per gram of any member), so it keeps its own bounds
 * and is coupled to nothing.
 */
function buildJointProblem(spec) {
  const members = spec.members;
  const totalBiomass = spec.totalBiomass ?? 1;
  const biomassCap = spec.biomassCap ?? null;
  const communityTag = spec.communityTag ?? 'comm';

  const cols = [];
  const pushCol = (name, lb, ub) => { cols.push({ name, lb, ub }); return cols.length - 1; };

  const fluxCols = new Map();
  for (const [id, rxn] of spec.reactions) {
    fluxCols.set(id, rxn.species === communityTag
      ? pushCol(id, rxn.lowerBound, rxn.upperBound)
      : pushCol(id, -Infinity, Infinity));
  }

  const biomassCols = new Map();
  for (const member of members) {
    biomassCols.set(member.id, pushCol(`X|${member.id}`, 0, biomassCap ?? totalBiomass));
  }

  const rows = [];
  const pushRow = (name, entries, lo, hi) => { rows.push({ name, entries, lo, hi }); return rows.length - 1; };

  // Mass balance, for species and community metabolites alike.
  const perMet = new Map();
  for (const [id, rxn] of spec.reactions) {
    const col = fluxCols.get(id);
    for (const [mid, coef] of Object.entries(rxn.metabolites)) {
      if (!coef) continue;
      if (!perMet.has(mid)) perMet.set(mid, []);
      perMet.get(mid).push([col, coef]);
    }
  }
  for (const [mid, entries] of perMet) pushRow(mid, entries, 0, 0);

  // Bound coupling, for species reactions only.
  for (const [id, rxn] of spec.reactions) {
    if (rxn.species === communityTag) continue;
    const xCol = biomassCols.get(rxn.species);
    if (xCol === undefined) {
      throw new Error(`reaction ${id} belongs to species ${rxn.species}, which is not a member`);
    }
    const col = fluxCols.get(id);
    if (Number.isFinite(rxn.upperBound)) {
      pushRow(`${id}|ub`, [[col, 1], [xCol, -rxn.upperBound]], -Infinity, 0);
    }
    if (Number.isFinite(rxn.lowerBound)) {
      pushRow(`${id}|lb`, [[col, 1], [xCol, -rxn.lowerBound]], 0, Infinity);
    }
  }

  // Normalisation. See the note in buildProblem: fixed when there is no cap,
  // capped when the caller means to maximise the total instead.
  pushRow('normalisation', [...biomassCols.values()].map((col) => [col, 1]),
    biomassCap === null ? totalBiomass : 0, biomassCap === null ? totalBiomass : biomassCap);

  // Any extra linear constraints the caller asked for, each {name, terms:
  // [{reaction, coef, part?}], biomass: [{species, coef}], lo, hi}.
  for (const constraint of spec.constraints ?? []) {
    const entries = [];
    for (const term of constraint.terms ?? []) {
      const col = fluxCols.get(term.reaction);
      if (col === undefined) throw new Error(`constraint ${constraint.name}: no reaction ${term.reaction}`);
      if (term.part !== 'uptake') { entries.push([col, term.coef]); continue; }
      // Only the uptake side of this flux counts. Split it: u >= 0 and u >= -V,
      // then use u. Nothing pushes u above max(0, -V), because u appears only
      // here and only with a positive coefficient in a <= constraint, so the
      // bound is tight at the optimum and this is exact rather than a relaxation.
      const uptakeCol = pushCol(`uptake|${term.reaction}`, 0, Infinity);
      pushRow(`uptake|${term.reaction}|split`, [[col, 1], [uptakeCol, 1]], 0, Infinity);
      entries.push([uptakeCol, term.coef]);
    }
    for (const term of constraint.biomass ?? []) {
      const col = biomassCols.get(term.species);
      if (col === undefined) throw new Error(`constraint ${constraint.name}: no member ${term.species}`);
      entries.push([col, term.coef]);
    }
    if (entries.length) {
      pushRow(constraint.name ?? `constraint${rows.length}`, entries,
        constraint.lo ?? -Infinity, constraint.hi ?? Infinity);
    }
  }

  // Growth coupling, rewritten per bisection step.
  const growthRows = members.map((member) => {
    const col = fluxCols.get(member.biomassReaction);
    if (col === undefined) {
      throw new Error(`member ${member.id}: no biomass reaction ${member.biomassReaction}`);
    }
    return pushRow(`growth|${member.id}`, [[col, 1], [biomassCols.get(member.id), 0]], 0, 0);
  });

  return { members, cols, rows, fluxCols, biomassCols, growthRows };
}

// FVA

/**
 * @typedef {object} SteadyComTarget
 * @property {string} name
 * @property {Object<string,number>} [reactions] reaction id -> coefficient
 * @property {Object<string,number>} [biomass]   member id -> coefficient on X^k
 */

/**
 * Flux variability at community steady state: the range each target can take
 * while the community still grows at its maximum rate.
 *
 * This is what SteadyComFVA does. The growth rate is found first, then fixed,
 * and each target minimised and maximised with the total biomass still fixed at
 * one: the paper's optGRpercent = 100 and optBMpercent = 100.
 *
 * @param {object} spec
 * @param {object} [options]
 * @param {SteadyComTarget[]} [options.targets]
 * @param {number} [options.growthFraction=1] fraction of the maximum growth rate
 *        to hold the community at, the paper's optGRpercent / 100
 * @returns {{status:string, growthRate:number, abundances:Map, ranges:object[], message:string}}
 *          plus `relaxations` and the names of `unsolved` targets
 */
export function steadyComFVA(spec, options = {}) {
  const { targets = [], growthFraction = 1, ...rest } = options;

  const base = steadyCom(spec, rest);
  if (base.status !== 'optimal') {
    return { status: base.status, growthRate: 0, abundances: new Map(), ranges: [], message: base.message };
  }

  const totalBiomass = spec.totalBiomass ?? 1;
  const mu = base.growthRate * growthFraction;
  // The ranges are taken with the total biomass fixed, the paper's
  // optBMpercent = 100, whatever cap the growth-rate search used.
  const problem = buildAny({ ...spec, biomassCap: null });

  // Held at exactly the maximum growth rate, the feasible set is a sliver: the
  // bisection found this mu as the largest it could verify, and one tolerance
  // further would have been infeasible. Some of these solves land on the wrong
  // side of that edge and come back non-optimal. Backing mu off by the bisection
  // tolerance (the precision to which the growth rate is known in the first
  // place) moves the slice off the edge without moving it anywhere the search
  // had not already verified. Each range records whether it needed that, so a
  // relaxed answer never passes for an unrelaxed one.
  const relaxed = Math.max(0, mu - (rest.tolerance ?? 1e-6));

  const ranges = [];
  let relaxations = 0;
  for (const target of targets) {
    const entries = targetEntries(problem, target);
    const solve = (maximize) => {
      const at = solveAt(problem, mu, totalBiomass, { entries, maximize });
      if (at.status === LP_OPTIMAL) return { ...at, relaxed: false };
      const retry = solveAt(problem, relaxed, totalBiomass, { entries, maximize });
      if (retry.status === LP_OPTIMAL) relaxations += 1;
      return { ...retry, relaxed: true };
    };

    const min = solve(false);
    const max = solve(true);
    ranges.push({
      name: target.name,
      min: min.status === LP_OPTIMAL ? min.objective : NaN,
      max: max.status === LP_OPTIMAL ? max.objective : NaN,
      minStatus: min.status,
      maxStatus: max.status,
      minRelaxed: min.relaxed,
      maxRelaxed: max.relaxed,
      minFluxes: min.fluxes,
      maxFluxes: max.fluxes,
    });
  }

  const unsolved = ranges.filter((r) => Number.isNaN(r.min) || Number.isNaN(r.max));

  return {
    status: 'optimal',
    growthRate: base.growthRate,
    abundances: base.abundances,
    ranges,
    relaxations,
    unsolved: unsolved.map((r) => r.name),
    iterations: base.iterations,
    message: unsolved.length
      ? `${unsolved.length} of ${targets.length} targets did not solve even relaxed: `
        + unsolved.map((r) => r.name).join(', ')
      : '',
  };
}

function targetEntries(problem, target) {
  const entries = [];
  for (const [rid, coef] of Object.entries(target.reactions ?? {})) {
    const col = problem.fluxCols.get(rid) ?? problem.fluxCols.get(`${target.species}\u0000${rid}`);
    if (col === undefined) throw new Error(`target ${target.name}: no reaction ${rid}`);
    entries.push([col, coef]);
  }
  for (const [id, coef] of Object.entries(target.biomass ?? {})) {
    const col = problem.biomassCols.get(id);
    if (col === undefined) throw new Error(`target ${target.name}: no member ${id}`);
    entries.push([col, coef]);
  }
  return entries;
}
