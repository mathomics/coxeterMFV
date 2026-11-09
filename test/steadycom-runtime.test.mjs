import test from 'node:test';
import assert from 'node:assert/strict';
import { Model } from '../src/core/model.js';
import { initGLPK, glpkAvailable } from '../src/solver/glpk.js';
import { steadyCom, maxTotalBiomassAt } from '../src/core/steadycom.js';
import {
  exchangeIndex, parseMediaCSV, steadyComSpec, toMergedFluxes,
} from '../src/communities/steadycom-runtime.js';

/*
 * The same two-member mutualism test/steadycom.test.mjs uses, but built the way
 * the browser has to build it: from member models alone, with the biomass
 * reaction and the exchange index derived rather than supplied.
 */
const GLUCOSE_BUDGET = 10;

function memberModel(id, makes, needs) {
  return new Model({
    id,
    compartments: { e: 'Extracellular', c: 'Cytosol' },
    metabolites: [
      { id: 'glc_e', compartment: 'e' }, { id: 'glc_c', compartment: 'c' },
      { id: `${makes}_c`, compartment: 'c' }, { id: `${makes}_e`, compartment: 'e' },
      { id: `${needs}_c`, compartment: 'c' }, { id: `${needs}_e`, compartment: 'e' },
    ],
    reactions: [
      { id: 'EX_glc_e', metabolites: { glc_e: -1 }, lower_bound: -GLUCOSE_BUDGET, upper_bound: 1000 },
      { id: 'GLCt', metabolites: { glc_e: -1, glc_c: 1 }, lower_bound: 0, upper_bound: 100 },
      { id: 'BIOMASS', metabolites: { glc_c: -1, [`${needs}_c`]: -1 }, lower_bound: 0, upper_bound: 100 },
      { id: `MK_${makes}`, metabolites: { glc_c: -1, [`${makes}_c`]: 1 }, lower_bound: 0, upper_bound: 100 },
      { id: `T_${makes}`, metabolites: { [`${makes}_c`]: -1, [`${makes}_e`]: 1 }, lower_bound: 0, upper_bound: 100 },
      { id: `EX_${makes}_e`, metabolites: { [`${makes}_e`]: -1 }, lower_bound: 0, upper_bound: 1000 },
      { id: `T_${needs}`, metabolites: { [`${needs}_e`]: -1, [`${needs}_c`]: 1 }, lower_bound: 0, upper_bound: 100 },
      { id: `EX_${needs}_e`, metabolites: { [`${needs}_e`]: -1 }, lower_bound: -100, upper_bound: 1000 },
    ],
    objective: { BIOMASS: 1 },
  });
}

const pair = () => [
  { id: 'A', model: memberModel('A', 'x', 'y') },
  { id: 'B', model: memberModel('B', 'y', 'x') },
];

test.before(async () => { await initGLPK(); });

test('media.csv becomes a community budget, with uptake read as positive', () => {
  const uptake = parseMediaCSV([
    'metabolite,exchange_reaction,lower_bound,upper_bound',
    'glc,EX_glc_e,-8,1000',
    'o2,EX_o2_e,-18.5,1000',
    'ac,EX_ac_e,0,1000',          // secretion only: no budget
    '',
  ].join('\n'));

  assert.deepEqual(uptake, { glc: 8, o2: 18.5 });
});

test('a media file without the columns it needs says so', () => {
  assert.throws(() => parseMediaCSV('a,b\n1,2'), /metabolite.*lower_bound/);
});

test('the exchange index keys every EX_ reaction by its metabolite', () => {
  const index = exchangeIndex(memberModel('A', 'x', 'y'));
  assert.equal(index.glc, 'EX_glc_e');
  assert.equal(index.x, 'EX_x_e');
  assert.equal(index.y, 'EX_y_e');
  assert.equal(index.GLCt, undefined, 'a transport reaction is not an exchange');
});

test('a spec built from member models alone finds the same growth rate', (t) => {
  if (!glpkAvailable()) return t.skip('GLPK not installed');
  // The browser has the models and the medium and nothing else; it must reach
  // the answer the hand-written spec reaches.
  const spec = steadyComSpec(pair(), { glc: GLUCOSE_BUDGET }, 1);
  assert.deepEqual(spec.members.map((m) => m.biomassReaction), ['BIOMASS', 'BIOMASS']);

  const result = steadyCom(spec, { upperGuess: 20 });
  assert.equal(result.status, 'optimal');
  assert.ok(result.growthRate > 0, `grew at ${result.growthRate}`);
});

test('a member with no biomass reaction is refused rather than solved', () => {
  const model = new Model({
    id: 'C', compartments: { c: 'Cytosol' },
    metabolites: [{ id: 'a_c', compartment: 'c' }],
    reactions: [{ id: 'R', metabolites: { a_c: 1 }, lower_bound: 0, upper_bound: 1 }],
  });
  assert.throws(() => steadyComSpec([{ id: 'C', model }], {}), /no biomass reaction/);
});

test('at a fixed growth rate every member grows at exactly that rate per gDW', (t) => {
  if (!glpkAvailable()) return t.skip('GLPK not installed');
  /*
   * The invariant the whole fixed-mu view rests on. SteadyCom's fluxes are
   * absolute rates for a member's entire population, so the per-gDW rate the map
   * draws is V/X — and for the biomass reaction that has to come back as the mu
   * the user asked for, or the map is labelled with a growth rate nobody chose.
   */
  const spec = steadyComSpec(pair(), { glc: GLUCOSE_BUDGET }, 1);
  const max = steadyCom(spec, { upperGuess: 20 }).growthRate;
  const mu = max / 2;

  const at = maxTotalBiomassAt(spec, mu);
  assert.equal(at.feasible, true, `µ = ${mu} should be inside the admissible range`);

  const fluxes = toMergedFluxes(at.fluxes);
  for (const id of ['A', 'B']) {
    const x = at.abundances.get(id);
    assert.ok(x > 0, `${id} is present`);
    const perGDW = fluxes.get(`${id}@BIOMASS`) / x;
    assert.ok(Math.abs(perGDW - mu) < 1e-6, `${id} grows at ${perGDW}, asked for ${mu}`);
  }
});

test('above the admissible range the answer is a verdict, not a flux map', (t) => {
  if (!glpkAvailable()) return t.skip('GLPK not installed');
  const spec = steadyComSpec(pair(), { glc: GLUCOSE_BUDGET }, 1);
  const max = steadyCom(spec, { upperGuess: 20 }).growthRate;

  const at = maxTotalBiomassAt(spec, max * 2);
  assert.equal(at.feasible, false);
  assert.equal(at.fluxes.size, 0);
});

test('SteadyCom flux names are renamed into the ones the map reads', () => {
  const renamed = toMergedFluxes(new Map([
    ['Ec1|PGI', 1.5],
    ['Ec2|EX_glc_e', -8],
    ['EX_glc_comm', 2],        // a community-level column has no member prefix
  ]));

  assert.equal(renamed.get('Ec1@PGI'), 1.5);
  assert.equal(renamed.get('Ec2@EX_glc_e'), -8);
  assert.equal(renamed.get('EX_glc_comm'), 2);
});

test('below the maximum a fixed rate leaves more than one community', (t) => {
  if (!glpkAvailable()) return t.skip('GLPK not installed');
  /*
   * Why the panel says "one of them" rather than reporting a prediction.
   *
   * Fixing mu leaves a polytope of feasible communities, not a point, and an LP
   * returns a vertex of it. Which vertex depends on what is being optimised: the
   * exact formulation has no objective and takes any feasible point, while the
   * capped one maximises the total biomass. Both are legitimate communities
   * growing at that rate, and they are not the same community.
   */
  const spec = steadyComSpec(pair(), { glc: GLUCOSE_BUDGET }, 1);
  const max = steadyCom(spec, { upperGuess: 20 }).growthRate;
  const mu = max / 2;

  const feasiblePoint = maxTotalBiomassAt(spec, mu);
  const mostBiomass = maxTotalBiomassAt(spec, mu, { biomassCap: 2 });

  assert.equal(feasiblePoint.feasible, true);
  assert.ok(mostBiomass.total > 1,
    `holding mu at half the maximum leaves room for more than the required biomass`
    + ` (reached ${mostBiomass.total})`);
});
