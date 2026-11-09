import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Model, parseFormula } from '../src/core/model.js';
import { FluxProblem } from '../src/solver/fba.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const modelsDir = join(root, 'models');

async function loadModel(name) {
  return new Model(JSON.parse(await readFile(join(modelsDir, `${name}.json`), 'utf8')));
}

// Everything in models/ is a metabolic model, bar the catalogue and the
// scenarios. Anything else — a results file, say — belongs somewhere else, which
// is why the sweep output lives in results/ rather than here.
const files = (await readdir(modelsDir))
  .filter((f) => f.endsWith('.json') && !['index.json', 'scenarios.json'].includes(f))
  .map((f) => f.replace(/\.json$/, ''))
  .sort();

test('the catalogue is not empty', () => {
  assert.ok(files.length >= 5, `found ${files.length} models`);
});

for (const name of files) {
  test(`${name}: every reaction balances mass and charge`, async () => {
    const model = await loadModel(name);
    const problems = model.checkMassBalance();
    assert.deepEqual(problems, [], `unbalanced: ${JSON.stringify(problems)}`);
  });

  test(`${name}: grows on its default medium`, async () => {
    const model = await loadModel(name);
    const res = new FluxProblem(model).optimize();
    assert.equal(res.status, 'optimal');
    assert.ok(res.objectiveValue > 1e-3, `growth ${res.objectiveValue}`);
  });

  test(`${name}: the solution satisfies S·v = 0 and every bound`, async () => {
    const model = await loadModel(name);
    const res = new FluxProblem(model).optimize();
    const residual = new Map();
    for (const [rid, v] of res.fluxes) {
      if (!v) continue;
      for (const [mid, coef] of Object.entries(model.reactions.get(rid).metabolites)) {
        residual.set(mid, (residual.get(mid) ?? 0) + coef * v);
      }
      const rxn = model.reactions.get(rid);
      assert.ok(v >= rxn.lowerBound - 1e-6 && v <= rxn.upperBound + 1e-6,
        `${rid} = ${v} outside [${rxn.lowerBound}, ${rxn.upperBound}]`);
    }
    for (const [mid, value] of residual) {
      assert.ok(Math.abs(value) < 1e-6, `${mid} unbalanced by ${value}`);
    }
  });

  test(`${name}: compartments nest without cycles`, async () => {
    const model = await loadModel(name);
    const { roots, byId } = model.compartmentTree();
    assert.ok(roots.length >= 1);
    for (const node of byId.values()) {
      const seen = new Set([node.id]);
      let parent = node.parent;
      while (parent) {
        assert.ok(!seen.has(parent), `cycle through ${parent}`);
        seen.add(parent);
        parent = byId.get(parent)?.parent ?? null;
      }
    }
    // Every metabolite lives in a declared compartment.
    for (const met of model.metabolites.values()) {
      assert.ok(byId.has(met.compartment), `${met.id} in undeclared ${met.compartment}`);
    }
  });
}

test('E. coli reproduces the published e_coli_core aerobic growth rate', async () => {
  const model = await loadModel('ecoli_core_mc');
  const res = new FluxProblem(model).optimize();
  assert.ok(Math.abs(res.objectiveValue - 0.8739) < 1e-3,
    `expected 0.8739, got ${res.objectiveValue}`);
});

test('E. coli switches to fermentation without oxygen', async () => {
  const model = await loadModel('ecoli_core_mc');
  const problem = new FluxProblem(model);
  problem.setBounds('EX_o2_e', 0, 1000);
  const res = problem.optimize();
  assert.equal(res.status, 'optimal');
  assert.ok(res.objectiveValue < 0.3, 'anaerobic growth should be much slower');
  const products = ['EX_ac_e', 'EX_etoh_e', 'EX_for_e']
    .map((id) => res.fluxes.get(id));
  assert.ok(products.some((v) => v > 1), 'expected fermentation products');
});

test('yeast uses glycerol as a redox sink when anaerobic', async () => {
  const model = await loadModel('yeast_core_mc');
  const problem = new FluxProblem(model);
  problem.setBounds('EX_o2_e', 0, 1000);
  const res = problem.optimize();
  assert.equal(res.status, 'optimal');
  assert.ok(res.fluxes.get('EX_etoh_e') > 5, 'expected ethanol');
  assert.ok(res.fluxes.get('EX_glyc_e') > 0.1, 'expected glycerol');
});

test('the cyanobacterium cannot grow in the dark', async () => {
  const model = await loadModel('cyano_core_mc');
  const problem = new FluxProblem(model);
  problem.setBounds('EX_photon_e', 0, 0);
  const res = problem.optimize();
  assert.ok(res.status !== 'optimal' || res.objectiveValue < 1e-6,
    `dark growth should be impossible, got ${res.status} ${res.objectiveValue}`);
});

test('growth scales with the light supply', async () => {
  const model = await loadModel('cyano_core_mc');
  const rates = [30, 60, 100].map((light) => {
    const problem = new FluxProblem(model);
    problem.setBounds('EX_photon_e', -light, 0);
    return problem.optimize().objectiveValue;
  });
  assert.ok(rates[0] < rates[1] && rates[1] < rates[2], `not monotonic: ${rates}`);
});

test('parsimonious FBA keeps the objective but lowers total flux', async () => {
  const model = await loadModel('ecoli_core_mc');
  const problem = new FluxProblem(model);
  const plain = problem.optimize();
  const parsimonious = problem.parsimonious();
  assert.ok(Math.abs(plain.objectiveValue - parsimonious.objectiveValue) < 1e-6);
  const totalPlain = [...plain.fluxes.values()].reduce((s, v) => s + Math.abs(v), 0);
  assert.ok(parsimonious.totalFlux <= totalPlain + 1e-6,
    `pFBA total ${parsimonious.totalFlux} exceeds FBA total ${totalPlain}`);
});

test('a knockout of an essential reaction stops growth', async () => {
  const model = await loadModel('ecoli_core_mc');
  const problem = new FluxProblem(model);
  const res = problem.knockout('ENO');   // enolase has no bypass in the core model
  assert.ok(res.status !== 'optimal' || res.objectiveValue < 1e-6,
    `expected no growth without enolase, got ${res.objectiveValue}`);
  // The knockout must not leak into later solves.
  assert.ok(problem.optimize().objectiveValue > 0.5);
});

test('flux variability brackets the reported flux', async () => {
  const model = await loadModel('ecoli_core_mc');
  const problem = new FluxProblem(model);
  const res = problem.optimize();
  const ranges = problem.variability(['PGI', 'EX_o2_e'], { fraction: 0.999 });
  for (const [rid, range] of ranges) {
    const v = res.fluxes.get(rid);
    assert.ok(range.min - 1e-4 <= v && v <= range.max + 1e-4,
      `${rid}: ${v} outside [${range.min}, ${range.max}]`);
  }
});

test('formula parsing rejects what it cannot understand', () => {
  assert.deepEqual(parseFormula('C6H12O6'), { C: 6, H: 12, O: 6 });
  assert.deepEqual(parseFormula('HO4P'), { H: 1, O: 4, P: 1 });
  assert.equal(parseFormula('X(R)'), null);
  assert.equal(parseFormula(''), null);
  assert.equal(parseFormula(null), null);
});
