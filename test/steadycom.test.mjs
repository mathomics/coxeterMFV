import test from 'node:test';
import assert from 'node:assert/strict';
import { Model } from '../src/core/model.js';
import { initGLPK, glpkAvailable } from '../src/solver/glpk.js';
import { steadyCom, maxTotalBiomassAt, parsimoniousAt } from '../src/core/steadycom.js';
import { FluxProblem } from '../src/solver/fba.js';

/**
 * SteadyCom on a community small enough to solve on paper, so the assertions are
 * derivations rather than a recording of whatever the code happened to print.
 *
 * Two members in an obligate mutualism. Each makes a metabolite the other cannot
 * make and needs: A supplies x, B supplies y. Every reaction costs one glucose.
 *
 *   A:  glc -> biomass_A        B:  glc -> biomass_B
 *       glc -> x (secreted)        glc -> y (secreted)
 *       needs y from B             needs x from A
 *
 * With a community glucose budget of 10 mmol/h and a total biomass of 1 gDW:
 *
 *   A spends glucose on its own growth and on the x that B needs:
 *     glc_A = mu*X_A + V_bio_B = mu*X_A + mu*X_B = mu
 *   B likewise spends glc_B = mu. Together 2*mu <= 10, so mu <= 5.
 *
 *   Each member may take up at most 10 per gDW of itself, so glc_A <= 10*X_A,
 *   that is mu <= 10*X_A. At mu = 5 this forces X_A >= 0.5, and the same for B.
 *   Since the two sum to 1, both are exactly 0.5.
 *
 * So mu = 5 /h and X_A = X_B = 0.5, with no freedom left in either.
 */

const GLUCOSE_BUDGET = 10;
const EXPECTED_GROWTH = GLUCOSE_BUDGET / 2;

/** One member: `makes` is secreted, `needs` is taken up, both one per biomass. */
function member(id, makes, needs) {
  const doc = {
    id,
    name: id,
    compartments: { e: 'Extracelular', c: 'Citosol' },
    metabolites: [
      { id: 'glc_e', name: 'glucose', compartment: 'e' },
      { id: 'glc_c', name: 'glucose', compartment: 'c' },
      { id: `${makes}_c`, name: makes, compartment: 'c' },
      { id: `${makes}_e`, name: makes, compartment: 'e' },
      { id: `${needs}_c`, name: needs, compartment: 'c' },
      { id: `${needs}_e`, name: needs, compartment: 'e' },
    ],
    reactions: [
      // Uptake is negative flux, as everywhere else in the project.
      { id: 'EX_glc_e', metabolites: { glc_e: -1 }, lower_bound: -GLUCOSE_BUDGET, upper_bound: 1000 },
      { id: 'GLCt', metabolites: { glc_e: -1, glc_c: 1 }, lower_bound: 0, upper_bound: 100 },
      // Biomass needs glucose and the partner's metabolite, one each.
      {
        id: 'BIOMASS',
        metabolites: { glc_c: -1, [`${needs}_c`]: -1 },
        lower_bound: 0,
        upper_bound: 100,
      },
      { id: `MK_${makes}`, metabolites: { glc_c: -1, [`${makes}_c`]: 1 }, lower_bound: 0, upper_bound: 100 },
      { id: `T_${makes}`, metabolites: { [`${makes}_c`]: -1, [`${makes}_e`]: 1 }, lower_bound: 0, upper_bound: 100 },
      { id: `EX_${makes}_e`, metabolites: { [`${makes}_e`]: -1 }, lower_bound: 0, upper_bound: 1000 },
      { id: `T_${needs}`, metabolites: { [`${needs}_e`]: -1, [`${needs}_c`]: 1 }, lower_bound: 0, upper_bound: 100 },
      { id: `EX_${needs}_e`, metabolites: { [`${needs}_e`]: -1 }, lower_bound: -100, upper_bound: 1000 },
    ],
    objective: { BIOMASS: 1 },
  };
  const model = new Model(doc);
  return {
    id,
    model,
    biomassReaction: 'BIOMASS',
    exchangeFor: {
      glc: 'EX_glc_e',
      [makes]: `EX_${makes}_e`,
      [needs]: `EX_${needs}_e`,
    },
  };
}

/** The pair, plus the medium: glucose only. Neither x nor y is supplied. */
function mutualists(options = {}) {
  const a = member('A', 'x', 'y');
  const b = member('B', 'y', 'x');
  if (options.blockA) {
    a.model.reactions.get('MK_x').lowerBound = 0;
    a.model.reactions.get('MK_x').upperBound = 0;
  }
  return {
    members: [a, b],
    communityUptake: { glc: GLUCOSE_BUDGET },
    totalBiomass: options.totalBiomass ?? 1,
  };
}

test.before(async () => { await initGLPK(); });

test('SteadyCom reaches the growth rate and abundances the medium dictates', (t) => {
  if (!glpkAvailable()) return t.skip('GLPK not installed');
  const result = steadyCom(mutualists(), { upperGuess: 20 });

  assert.equal(result.status, 'optimal', result.message);
  assert.ok(Math.abs(result.growthRate - EXPECTED_GROWTH) < 1e-4,
    `expected mu = ${EXPECTED_GROWTH}, got ${result.growthRate}`);
  assert.ok(Math.abs(result.abundances.get('A') - 0.5) < 1e-4);
  assert.ok(Math.abs(result.abundances.get('B') - 0.5) < 1e-4);
});

test('abundances sum to the total biomass', (t) => {
  if (!glpkAvailable()) return t.skip('GLPK not installed');
  const result = steadyCom(mutualists(), { upperGuess: 20 });
  const total = [...result.abundances.values()].reduce((s, x) => s + x, 0);
  assert.ok(Math.abs(total - 1) < 1e-6, `total biomass ${total}`);
});

test('twice the biomass on the same budget grows half as fast', (t) => {
  if (!glpkAvailable()) return t.skip('GLPK not installed');
  // The glucose budget is an absolute rate, so doubling the biomass that has to
  // live on it halves the rate each gram can grow at. Nothing else changes.
  const one = steadyCom(mutualists({ totalBiomass: 1 }), { upperGuess: 20 });
  const two = steadyCom(mutualists({ totalBiomass: 2 }), { upperGuess: 20 });
  assert.ok(Math.abs(two.growthRate - one.growthRate / 2) < 1e-4,
    `${two.growthRate} is not half of ${one.growthRate}`);
});

test('neither member grows alone', (t) => {
  if (!glpkAvailable()) return t.skip('GLPK not installed');
  for (const m of mutualists().members) {
    // On its own, with no partner to supply what it lacks, each is a dead end.
    // Its uptake exchange for that metabolite is open, so this is a statement
    // about the medium, not about the bounds.
    const solo = new FluxProblem(m.model);
    const grown = solo.optimize();
    const openUptake = m.model.reactions.get(`EX_${m.id === 'A' ? 'y' : 'x'}_e`).lowerBound;
    assert.ok(openUptake < 0, 'the uptake this test relies on should be open');
    assert.ok(grown.objectiveValue > 0,
      'with uptake open and unlimited, a lone member does grow — that is the baseline');
  }
});

test('a metabolite the medium does not supply can only come from a partner', (t) => {
  if (!glpkAvailable()) return t.skip('GLPK not installed');
  // The regression this guards: the community exchange space needs a row for
  // every metabolite any member trades, not only the ones the medium supplies. A
  // metabolite with no row is one each member draws out of nowhere, and the
  // community then grows without any cross-feeding at all. Blocking A's only
  // route to x has to stop B, and so the whole community.
  const crippled = steadyCom(mutualists({ blockA: true }), { upperGuess: 20 });
  const healthy = steadyCom(mutualists(), { upperGuess: 20 });
  // Not exactly zero, and it should not be asserted to be. At a growth rate of
  // 1e-4 the x flux the LP has to find is around 1e-5, which is the floor of
  // GLPK's own feasibility tolerance, so it accepts a solution that is violating
  // the constraint by less than it can see. What the test can say is that the
  // community has collapsed: three orders of magnitude below the healthy rate.
  assert.ok(crippled.growthRate < healthy.growthRate / 1000,
    `B cannot get x from anywhere, yet the community grew at ${crippled.growthRate}`);
});

test('cross-feeding balances: what one secretes is what the other takes up', (t) => {
  if (!glpkAvailable()) return t.skip('GLPK not installed');
  const result = steadyCom(mutualists(), { upperGuess: 20 });
  for (const [producer, consumer, base] of [['A', 'B', 'x'], ['B', 'A', 'y']]) {
    const out = result.fluxes.get(`${producer}|EX_${base}_e`);
    const into = result.fluxes.get(`${consumer}|EX_${base}_e`);
    assert.ok(out > 1e-6, `${producer} should secrete ${base}, flux was ${out}`);
    assert.ok(into < -1e-6, `${consumer} should take up ${base}, flux was ${into}`);
    // The medium supplies none of it, so the two must cancel — to the solver's
    // own tolerance, not exactly: these are fluxes of 2.5 from a simplex.
    assert.ok(Math.abs(out + into) < 1e-5,
      `${base}: ${producer} secretes ${out}, ${consumer} takes ${into}`);
  }
});

test('the solution satisfies mass balance for every member', (t) => {
  if (!glpkAvailable()) return t.skip('GLPK not installed');
  const spec = mutualists();
  const result = steadyCom(spec, { upperGuess: 20 });

  for (const m of spec.members) {
    const balance = new Map();
    for (const [rid, rxn] of m.model.reactions) {
      const v = result.fluxes.get(`${m.id}|${rid}`) ?? 0;
      for (const [mid, coef] of Object.entries(rxn.metabolites)) {
        balance.set(mid, (balance.get(mid) ?? 0) + coef * v);
      }
    }
    for (const [mid, net] of balance) {
      assert.ok(Math.abs(net) < 1e-6, `${m.id}: ${mid} is off balance by ${net}`);
    }
  }
});

test('every flux stays inside its bound scaled by the abundance', (t) => {
  if (!glpkAvailable()) return t.skip('GLPK not installed');
  // This is the constraint that makes SteadyCom a community model rather than a
  // merged one: a member's rate is capped by how much of that member there is.
  const spec = mutualists();
  const result = steadyCom(spec, { upperGuess: 20 });

  for (const m of spec.members) {
    const x = result.abundances.get(m.id);
    for (const [rid, rxn] of m.model.reactions) {
      const v = result.fluxes.get(`${m.id}|${rid}`) ?? 0;
      assert.ok(v <= rxn.upperBound * x + 1e-6, `${m.id}|${rid}: ${v} > ${rxn.upperBound} * ${x}`);
      assert.ok(v >= rxn.lowerBound * x - 1e-6, `${m.id}|${rid}: ${v} < ${rxn.lowerBound} * ${x}`);
    }
  }
});

test('the answer is reproducible across runs', (t) => {
  if (!glpkAvailable()) return t.skip('GLPK not installed');
  const first = steadyCom(mutualists(), { upperGuess: 20 });
  const second = steadyCom(mutualists(), { upperGuess: 20 });
  assert.equal(first.growthRate, second.growthRate);
  for (const [id, x] of first.abundances) {
    assert.ok(Math.abs(x - second.abundances.get(id)) < 1e-9, `${id} drifted between runs`);
  }
});

test('maximum total biomass falls away as the growth rate rises', (t) => {
  if (!glpkAvailable()) return t.skip('GLPK not installed');
  // The property the bisection rests on. If this were not monotone there would be
  // no single crossing to find, and a bracket could close on the wrong side.
  const spec = mutualists();
  let previous = Infinity;
  for (const mu of [0.5, 1, 2, 4, 5]) {
    const step = maxTotalBiomassAt({ ...spec, totalBiomass: 1 }, mu);
    assert.equal(step.status, 'optimal', `mu=${mu}: ${step.message}`);
    assert.ok(step.total <= previous + 1e-6, `total biomass rose from ${previous} to ${step.total}`);
    previous = step.total;
  }
});

test('a growth rate the community cannot reach is reported as infeasible, not as zero', (t) => {
  if (!glpkAvailable()) return t.skip('GLPK not installed');
  const beyond = maxTotalBiomassAt(mutualists(), EXPECTED_GROWTH * 2);
  assert.equal(beyond.feasible, false);
  assert.equal(beyond.status, 'infeasible',
    'a solver that gave up must not be recorded as a community that cannot grow');
});

test('capping the total biomass and fixing it give the same growth rate', (t) => {
  if (!glpkAvailable()) return t.skip('GLPK not installed');
  // Two ways to ask the same question at each bisection step. Fixing the total
  // asks whether it is feasible; capping it maximises the total and compares. The
  // second is what the paper's own algorithm does and is much cheaper on a large
  // community, because proving an LP infeasible costs far more than solving a
  // feasible one and half a bisection's steps are infeasible by construction.
  // They must agree, or the speed is worthless.
  const fixed = steadyCom(mutualists(), { upperGuess: 20 });
  const capped = steadyCom(mutualists(), { upperGuess: 20, biomassCap: 2 });

  assert.equal(capped.status, 'optimal', capped.message);
  assert.ok(Math.abs(capped.growthRate - fixed.growthRate) < 1e-4,
    `capped ${capped.growthRate} against fixed ${fixed.growthRate}`);
  assert.ok(Math.abs(capped.growthRate - EXPECTED_GROWTH) < 1e-4);
  for (const [id, x] of fixed.abundances) {
    assert.ok(Math.abs(x - capped.abundances.get(id)) < 1e-4, `${id} differs between modes`);
  }
});

test('a growth rate above the maximum is a verdict, not a solver failure', (t) => {
  if (!glpkAvailable()) return t.skip('GLPK not installed');
  // The bug this guards. With a cap the LP always solves — every member can be
  // absent — so a growth rate the community cannot reach comes back optimal with
  // a total below the one required. Read as a solver failure, that aborts the
  // whole search and reports zero after no iterations at all.
  const beyond = maxTotalBiomassAt(mutualists(), EXPECTED_GROWTH * 2, { biomassCap: 2 });
  assert.equal(beyond.status, 'optimal', 'with a cap the LP is always feasible');
  assert.ok(beyond.total < 1, `the reachable total should fall short, got ${beyond.total}`);
  assert.equal(beyond.feasible, false);

  // And the search still finds the maximum rather than giving up at that point.
  const capped = steadyCom(mutualists(), { upperGuess: EXPECTED_GROWTH * 4, biomassCap: 2 });
  assert.equal(capped.status, 'optimal', capped.message);
  assert.ok(capped.iterations > 1, 'the search should have taken steps');
  assert.ok(Math.abs(capped.growthRate - EXPECTED_GROWTH) < 1e-4,
    `got ${capped.growthRate}`);
});

test('the parsimonious solve drops a futile cycle and costs what the paper sum says', (t) => {
  if (!glpkAvailable()) return t.skip('GLPK not installed');
  // A gets a futile cycle, glc -> z -> glc, that any feasible solve may run.
  // Without it every reaction of a member is forced. A takes up glucose for its
  // own biomass and for the x that B needs, mu*X_A + mu*X_B = mu, and the flux
  // magnitudes of A are
  //   EX_glc mu, GLCt mu, BIOMASS mu*X_A, MK_x, T_x and EX_x mu*X_B each,
  //   T_y and EX_y mu*X_A each,
  // which sum to 2*mu + 3*mu*(X_A + X_B) = 5*mu. B is the mirror image, so the
  // least total flux is 10*mu whatever the composition, and the cycle carries none.
  const spec = mutualists();
  const doc = spec.members[0].model.toJSON();
  doc.metabolites.push({ id: 'z_c', name: 'z', compartment: 'c' });
  doc.reactions.push(
    { id: 'CYC1', metabolites: { glc_c: -1, z_c: 1 }, lower_bound: 0, upper_bound: 100 },
    { id: 'CYC2', metabolites: { z_c: -1, glc_c: 1 }, lower_bound: 0, upper_bound: 100 },
  );
  spec.members[0].model = new Model(doc);
  const mu = EXPECTED_GROWTH / 2;
  const lean = parsimoniousAt(spec, mu);
  assert.equal(lean.status, 'optimal', lean.message);
  assert.equal(lean.feasible, true);
  assert.ok(Math.abs(lean.totalFlux - 10 * mu) < 1e-6, `least total flux ${lean.totalFlux}, expected ${10 * mu}`);
  assert.ok(Math.abs(lean.fluxes.get('A|CYC1')) < 1e-9 && Math.abs(lean.fluxes.get('A|CYC2')) < 1e-9, 'the cycle should carry nothing');
  // Same growth, same total biomass as the plain fixed solve.
  const total = [...lean.abundances.values()].reduce((sum, x) => sum + x, 0);
  assert.ok(Math.abs(total - 1) < 1e-7, `total biomass ${total}`);
  for (const [id, x] of lean.abundances) {
    const bio = lean.fluxes.get(`${id}|BIOMASS`);
    assert.ok(Math.abs(bio - mu * x) < 1e-7, `${id} grows at ${bio / x}`);
  }
});
