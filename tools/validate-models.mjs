#!/usr/bin/env node
/**
 * Validate every bundled model: elemental + charge balance, a growth run, and a
 * report of what could not be checked (carrier proteins and photons have no
 * elemental formula, so the reactions that use them are listed explicitly
 * rather than silently skipped).
 *
 * Usage: node tools/validate-models.mjs
 */
import { readFile, readdir } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Model } from '../src/core/model.js';
import { FluxProblem } from '../src/solver/fba.js';
import { initGLPK } from '../src/solver/glpk.js';

await initGLPK().catch(() => false);

const here = dirname(fileURLToPath(import.meta.url));
const modelsDir = join(here, '..', 'models');

const NOT_MODELS = new Set(['index.json', 'scenarios.json']);
const files = (await readdir(modelsDir)).filter((f) => f.endsWith('.json') && !NOT_MODELS.has(f));
let failures = 0;

for (const file of files.sort()) {
  const doc = JSON.parse(await readFile(join(modelsDir, file), 'utf8'));
  const model = new Model(doc);

  const unbalanced = model.checkMassBalance();
  const unchecked = [];
  for (const rxn of model.reactions.values()) {
    if (['exchange', 'biomass', 'sink'].includes(rxn.kind)) continue;
    const missing = Object.keys(rxn.metabolites)
      .filter((mid) => !model.metabolites.get(mid).elements)
      .map((mid) => mid);
    if (missing.length) unchecked.push(`${rxn.id} (${[...new Set(missing)].join(', ')})`);
  }

  const problem = new FluxProblem(model);
  const growth = problem.optimize();

  const ok = unbalanced.length === 0 && growth.status === 'optimal' && growth.objectiveValue > 1e-6;
  if (!ok) failures += 1;

  console.log(`${ok ? '  OK  ' : ' FAIL '} ${file}`);
  console.log(`        ${model.metabolites.size} metabolites, ${model.reactions.size} reactions, compartments: ${model.compartmentIds().join(' ')}`);
  console.log(`        growth: ${growth.status === 'optimal' ? growth.objectiveValue.toFixed(4) : growth.status} 1/h  (objective ${model.biomassReactionId()})`);
  if (unbalanced.length) {
    console.log(`        ${unbalanced.length} UNBALANCED reaction(s):`);
    for (const u of unbalanced) console.log(`          ${u.id}: ${JSON.stringify(u.imbalance)}`);
  } else {
    console.log('        mass and charge balance: all reactions balanced');
  }
  if (unchecked.length) {
    console.log(`        not checkable (metabolite without elemental formula): ${unchecked.join('; ')}`);
  }
}

console.log(failures ? `\n${failures} model(s) failed validation` : `\nAll ${files.length} models validated`);
// Let the WASM runtime and output streams finish closing on Windows.
process.exitCode = failures ? 1 : 0;
