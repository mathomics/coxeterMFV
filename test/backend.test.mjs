import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Model } from '../src/core/model.js';
import { FluxProblem } from '../src/solver/fba.js';
import { CommunityModel } from '../src/core/community.js';
import { initGLPK, glpkAvailable } from '../src/solver/glpk.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const available = await initGLPK().catch(() => false);

async function loadModel(name) {
  return new Model(JSON.parse(await readFile(join(root, 'models', `${name}.json`), 'utf8')));
}

// Everything in models/ is a metabolic model, bar the catalogue and the
// scenarios. Anything else — a results file, say — belongs somewhere else, which
// is why the sweep output lives in results/ rather than here.
const modelNames = (await readdir(join(root, 'models')))
  .filter((f) => f.endsWith('.json') && !['index.json', 'scenarios.json'].includes(f))
  .map((f) => f.replace(/\.json$/, ''))
  .sort();

test('the project still runs when GLPK is absent', async () => {
  // Whatever the environment, forcing the bundled simplex must work: that is
  // the whole point of keeping it as a fallback.
  const model = await loadModel('ecoli_core_mc');
  const res = new FluxProblem(model, { backend: 'simplex' }).optimize();
  assert.equal(res.status, 'optimal');
  assert.ok(res.objectiveValue > 0.8);
});

test('auto uses GLPK when it is available and the simplex when it is not', async () => {
  const model = await loadModel('methanogen_core_mc');
  assert.equal(new FluxProblem(model).backend(), available ? 'glpk' : 'simplex');
  assert.equal(glpkAvailable(), available);
});

test('the backend can always be forced either way', async () => {
  const model = await loadModel('ecoli_core_mc');
  assert.equal(new FluxProblem(model, { backend: 'simplex' }).backend(), 'simplex');
  if (available) assert.equal(new FluxProblem(model, { backend: 'glpk' }).backend(), 'glpk');
});

for (const name of modelNames) {
  test(`${name}: both backends agree on the optimum`, { skip: !available }, async () => {
    const model = await loadModel(name);
    const dense = new FluxProblem(model, { backend: 'simplex' }).optimize();
    const sparse = new FluxProblem(model, { backend: 'glpk' }).optimize();
    assert.equal(dense.status, sparse.status);
    assert.ok(
      Math.abs(dense.objectiveValue - sparse.objectiveValue) < 1e-6,
      `${dense.objectiveValue} vs ${sparse.objectiveValue}`,
    );
  });

  test(`${name}: the GLPK solution is feasible`, { skip: !available }, async () => {
    const model = await loadModel(name);
    const res = new FluxProblem(model, { backend: 'glpk' }).optimize();
    assert.equal(res.status, 'optimal');

    const residual = new Map();
    for (const [rid, v] of res.fluxes) {
      if (!v) continue;
      const rxn = model.reactions.get(rid);
      assert.ok(
        v >= rxn.lowerBound - 1e-6 && v <= rxn.upperBound + 1e-6,
        `${rid} = ${v} outside [${rxn.lowerBound}, ${rxn.upperBound}]`,
      );
      for (const [mid, coef] of Object.entries(rxn.metabolites)) {
        residual.set(mid, (residual.get(mid) ?? 0) + coef * v);
      }
    }
    for (const [mid, value] of residual) {
      assert.ok(Math.abs(value) < 1e-6, `${mid} unbalanced by ${value}`);
    }
  });
}

test('both backends agree on a merged community', { skip: !available }, async () => {
  const species = [];
  for (const [id, file] of [['eco', 'ecoli_core_mc'], ['mba', 'methanogen_core_mc']]) {
    species.push({ id, model: await loadModel(file), abundance: 0.5 });
  }
  const community = new CommunityModel({
    species,
    medium: {
      glc__D: { lower: -10 }, nh4: { lower: -1000 }, pi: { lower: -1000 },
      h2o: { lower: -1000 }, h: { lower: -1000 }, co2: { lower: -1000 }, o2: { lower: 0 },
    },
  });
  const dense = new FluxProblem(community.merged, { backend: 'simplex' }).optimize();
  const sparse = new FluxProblem(community.merged, { backend: 'glpk' }).optimize();
  assert.equal(dense.status, sparse.status);
  assert.ok(
    Math.abs(dense.objectiveValue - sparse.objectiveValue) < 1e-6,
    `${dense.objectiveValue} vs ${sparse.objectiveValue}`,
  );
});

test('both backends agree on flux variability', { skip: !available }, async () => {
  const model = await loadModel('ecoli_core_mc');
  const targets = ['PGI', 'EX_o2_e', 'PFK'];
  const dense = new FluxProblem(model, { backend: 'simplex' }).variability(targets);
  const sparse = new FluxProblem(model, { backend: 'glpk' }).variability(targets);
  for (const rid of targets) {
    const a = dense.get(rid);
    const b = sparse.get(rid);
    assert.ok(Math.abs(a.min - b.min) < 1e-4, `${rid} min: ${a.min} vs ${b.min}`);
    assert.ok(Math.abs(a.max - b.max) < 1e-4, `${rid} max: ${a.max} vs ${b.max}`);
  }
});

test('both backends agree that an essential knockout stops growth', { skip: !available }, async () => {
  const model = await loadModel('ecoli_core_mc');
  for (const backend of ['simplex', 'glpk']) {
    const problem = new FluxProblem(model, { backend });
    const res = problem.knockout('ENO');
    assert.ok(
      res.status !== 'optimal' || res.objectiveValue < 1e-6,
      `${backend}: expected no growth, got ${res.objectiveValue}`,
    );
    assert.ok(problem.optimize().objectiveValue > 0.5, `${backend}: knockout leaked`);
  }
});
