#!/usr/bin/env node
/**
 * Reproduce the Figure 4 experiment of the SteadyCom paper on the nine-species
 * gut microbiota model: sweep the maximum specific fiber uptake rate of
 * B. thetaiotaomicron from 1 to 30 C-mmol/gDW/h, holding F. prausnitzii at 5,
 * and at each point take the maximum community growth rate and the flux
 * variability of the eleven targets simFigure4.m asks for.
 *
 *   node tools/run-gut9-figure4.mjs --dir=<folder with the five model9 files> \
 *       [--points=1,10,30] [--out=results.json]
 *
 * Each point needs a bisection on the growth rate plus 22 FVA solves over a
 * 28 000-row LP, so expect a couple of minutes per point. Use --points to check
 * a few before committing to all thirty.
 *
 * Results are written as JSON after every point, so an interrupted run keeps
 * what it has, and --points lets a later run fill in the rest.
 */
import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { initGLPK } from '../src/solver/glpk.js';
import { steadyComFVA } from '../src/core/steadycom.js';
import { importGut9, toSteadyComSpec, EXPECTED } from '../src/communities/gut9.js';
import {
  FIBER_SWEEP, SOLVER_OPTIONS, fiberConstraints, figureTargets, FIGURE_SPECIES,
} from '../src/communities/gut9-figure4.js';

const args = Object.fromEntries(process.argv.slice(2).map((a) => {
  const [k, v = 'true'] = a.replace(/^--/, '').split('=');
  return [k, v];
}));

const dir = args.dir ?? '.';
const out = args.out ?? 'gut9-figure4.json';
const points = args.points && args.points !== 'true'
  ? args.points.split(',').map(Number)
  : FIBER_SWEEP.bacteroides;

if (!await initGLPK()) {
  console.error('This needs the GLPK backend; run `npm install` first.');
  process.exit(1);
}

const model = importGut9(
  await readFile(join(dir, 'model9.xlsx')),
  { boundsText: await readFile(join(dir, 'model9_rxns_params.txt'), 'utf8') },
);
console.log(`model9: ${model.species.length} species, ${model.reactions.size} reactions,`
  + ` ${model.metabolites.size} metabolites, ${model.communityMetabolites.length} community metabolites`);
if (model.reactions.size !== EXPECTED.reactions) {
  throw new Error(`expected ${EXPECTED.reactions} reactions, imported ${model.reactions.size}`);
}
console.log(`bounds from ${model.boundsFrom}; the diet opens ${model.boundChanges.length} uptake reactions\n`);

const targets = figureTargets(model);
const results = [];

for (const bacteroides of points) {
  const spec = toSteadyComSpec(model, {
    constraints: fiberConstraints(model, {
      bacteroides,
      faecalibacterium: FIBER_SWEEP.faecalibacterium,
    }),
  });

  const t0 = Date.now();
  const fva = steadyComFVA(spec, {
    targets,
    upperGuess: 0.1,
    tolerance: SOLVER_OPTIONS.tolerance,
    // Maximise the total biomass at each step rather than test the required one
    // for feasibility. On this community proving an LP infeasible costs ten times
    // what solving a feasible one does, and half a bisection's steps are
    // infeasible by construction. With a cap every step is the cheap case.
    biomassCap: 2,
  });
  const seconds = (Date.now() - t0) / 1000;

  if (fva.status !== 'optimal') {
    console.log(`FUR ${String(bacteroides).padStart(2)}: failed — ${fva.message}`);
    results.push({ bacteroides, status: fva.status, message: fva.message });
    continue;
  }

  const byName = new Map(fva.ranges.map((r) => [r.name, r]));
  // A target that did not solve is recorded as such. Writing null and moving on
  // would leave a gap in the curve that looks like data.
  if (fva.unsolved.length) {
    console.log(`         unsolved: ${fva.unsolved.join(', ')}`);
  }
  // The figure plots the minimum biomass production of each member, which is
  // growth rate times the smallest abundance that member can have.
  const row = {
    bacteroides,
    growthRate: fva.growthRate,
    seconds,
    abundances: Object.fromEntries(fva.abundances),
    biomassMin: Object.fromEntries(model.species.map((sp) =>
      [sp.id, byName.get(`Bm_${sp.id}`)?.min ?? null])),
    biomassMax: Object.fromEntries(model.species.map((sp) =>
      [sp.id, byName.get(`Bm_${sp.id}`)?.max ?? null])),
    // Panel C: fiber uptake is -max(fiber exchange), FDS export is min(carbohydrate).
    fiberUptake: -(byName.get('fiber_Bt_Cmmol')?.max ?? NaN),
    fdsExport: byName.get('carbohydrate_Bt_Cmmol')?.min ?? NaN,
    unsolved: fva.unsolved,
    relaxations: fva.relaxations,
  };
  results.push(row);

  const stack = FIGURE_SPECIES.map((id) =>
    `${id} ${(row.biomassMin[id] ?? 0).toExponential(2)}`).join('  ');
  console.log(`FUR ${String(bacteroides).padStart(2)}: mu = ${row.growthRate.toFixed(5)} /h`
    + `  fiber ${row.fiberUptake.toFixed(3)}  FDS ${row.fdsExport.toFixed(3)}`
    + `  (${seconds.toFixed(0)} s)`);
  console.log(`         min biomass: ${stack}`);

  await writeFile(out, `${JSON.stringify({
    source: 'simFigure4.m on model9.xlsx + model9_rxns_params.txt',
    sweep: FIBER_SWEEP,
    results,
  }, null, 2)}\n`);
}

console.log(`\nwrote ${out} (${results.length} points)`);
