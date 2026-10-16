#!/usr/bin/env node
/**
 * Write the Published Communities registry into models/scenarios.json, which is
 * where the UI reads it from.
 *
 * The registry in src/communities/published.js is the source of truth and is
 * meant to be read by a person: it carries the provenance and the comments. This
 * copies it into the JSON the browser fetches, leaving the hand-written
 * `scenarios` array untouched.
 *
 *   node tools/build-published-communities.mjs
 *       metadata only, in a second
 *   node tools/build-published-communities.mjs --run [path/to/models] [--stolyar=<dir>]
 *       also solve every reproducible community and record what came out, so the
 *       UI can show computed beside reported without solving anything itself.
 *       Takes a few minutes per community.
 *
 * Results already in the file survive a run without --run, so the quick path
 * never silently drops them.
 */
import { readFile, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { PUBLISHED_COMMUNITIES } from '../src/communities/published.js';
import { initGLPK } from '../src/solver/glpk.js';
import { steadyCom } from '../src/core/steadycom.js';
import {
  buildCommunity, AMINO_ACID_EXCHANGES,
} from '../src/communities/steadycom-ecoli.js';
import {
  importStolyar, validateStolyar, toJointCobraJSON, reactionsBySpecies,
} from '../src/communities/stolyar.js';
import { Model } from '../src/core/model.js';
import { FluxProblem } from '../src/solver/fba.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const target = join(root, 'models', 'scenarios.json');

const args = process.argv.slice(2);
const run = args.includes('--run');
const modelDir = args.find((a) => !a.startsWith('--')) ?? '/home/user/mmc/sbml';

/** Solvers for the communities that can be reproduced, keyed by registry id. */
const RUNNERS = {
  /*
   * The two-species mutualism, swept over how much sulfate D. vulgaris may have.
   * Sixteen small LPs, so unlike the other two this runs here rather than in a
   * tool of its own. The supplement is not in this repository, so the path to it
   * is an argument: --stolyar=<dir>.
   */
  'stolyar-2007-syntrophy': async () => {
    const dir = args.find((a) => a.startsWith('--stolyar='))?.slice('--stolyar='.length);
    if (!dir) throw new Error('pass --stolyar=<folder with the Stolyar supplement>');
    const t0 = Date.now();
    const read = (name) => readFile(join(dir, name), 'utf8');
    const model = importStolyar({
      reactions: await read('reactions.txt'),
      metabolites: await read('metabolites.txt'),
      macromolecules: await read('macromolecules.txt'),
      synthesis: await read('macromolecule_synthesis.txt'),
    });
    const check = validateStolyar(model);
    if (!check.ok) throw new Error(`the Stolyar import does not validate: ${check.problems.join('; ')}`);

    const joint = new Model(toJointCobraJSON(model));
    const { bySpecies } = reactionsBySpecies(model);
    const netFlux = (fluxes, speciesId, metabolite) => bySpecies.get(speciesId).reduce(
      (total, id) => total + (model.reactions.get(id).metabolites[metabolite] ?? 0) * (fluxes.get(id) ?? 0), 0);

    const sweep = [];
    for (let sulfate = 0; sulfate <= 6; sulfate += 0.4) {
      const problem = new FluxProblem(joint);
      // EX_SO4 drains sulfate, so a negative flux is uptake: the lower bound is
      // how much the organism may take.
      problem.setBounds('EX_SO4', -sulfate, 1000);
      const solution = problem.optimize();
      if (solution.status !== 'optimal') continue;
      sweep.push({
        sulfate: round(sulfate, 2),
        dv: round(solution.fluxes.get('mue') ?? 0, 5),
        mm: round(solution.fluxes.get('MC_BIO') ?? 0, 5),
        h2: round(netFlux(solution.fluxes, 'Dv', 'H2(exch)'), 4),
        formate: round(netFlux(solution.fluxes, 'Dv', 'Form(exch)'), 4),
        ch4: round(solution.fluxes.get('M12') ?? 0, 4),
        sulfateReduced: round(solution.fluxes.get('D01') ?? 0, 4),
      });
    }
    if (!sweep.length) throw new Error('no point of the sulfate sweep solved');

    const syntrophic = sweep[0];
    return {
      growthRates: { Dv: syntrophic.dv, Mm: syntrophic.mm },
      methane: syntrophic.ch4,
      sweepPoints: sweep.length,
      sweep,
      lpCount: sweep.length,
      seconds: round((Date.now() - t0) / 1000, 1),
    };
  },

  // The nine-species sweep takes hours, so it is run separately by
  // tools/run-gut9-figure4.mjs and this reads what it recorded. The summary is
  // what the UI shows: the span of the growth rate across the sweep and the two
  // curves Figure 4 plots, not the eleven full FVA ranges at every point.
  'steadycom-gut-nine': async () => {
    const path = join(root, 'results', 'gut9-figure4.json');
    let recorded;
    try {
      recorded = JSON.parse(await readFile(path, 'utf8'));
    } catch {
      throw new Error(`${path} is missing; run tools/run-gut9-figure4.mjs first`);
    }
    const rows = recorded.results.filter((r) => !r.status);
    if (!rows.length) throw new Error('the recorded sweep has no solved points');

    const rates = rows.map((r) => r.growthRate);
    return {
      growthRateRange: [round(Math.min(...rates), 5), round(Math.max(...rates), 5)],
      sweepPoints: rows.length,
      sweep: rows.map((r) => ({
        fiberUptakeMax: r.bacteroides,
        growthRate: round(r.growthRate, 5),
        fiberUptake: round(r.fiberUptake, 4),
        fdsExport: round(r.fdsExport, 4),
      })),
      // Who is actually present, at the midpoint of the sweep.
      abundances: Object.fromEntries(Object.entries(
        rows[Math.floor(rows.length / 2)].abundances,
      ).map(([id, x]) => [id, round(x, 6)])),
      seconds: round(rows.reduce((sum, r) => sum + (r.seconds ?? 0), 0), 0),
    };
  },

  'steadycom-ecoli-auxotrophs': async () => {
    const spec = buildCommunity(await readFile(join(modelDir, 'iAF1260.xml'), 'utf8'));
    const t0 = Date.now();
    const result = steadyCom(spec, { upperGuess: 1.0 });
    if (result.status !== 'optimal') throw new Error(result.message);

    const abundances = {};
    for (const [id, x] of result.abundances) abundances[id] = round(x, 6);

    // Only the amino acids are worth recording: they are the cross-feeding the
    // paper is about, and the full flux vector is 9532 numbers.
    const exchanges = {};
    for (const member of spec.members) {
      exchanges[member.id] = Object.fromEntries(
        Object.entries(AMINO_ACID_EXCHANGES).map(([aa, rid]) =>
          [aa, round(result.fluxes.get(`${member.id}|${rid}`) ?? 0, 6)]),
      );
    }

    return {
      growthRate: round(result.growthRate, 5),
      abundances,
      exchanges,
      lpCount: result.iterations,
      seconds: round((Date.now() - t0) / 1000, 1),
    };
  },
};

const round = (v, places) => Number(v.toFixed(places));

const existing = JSON.parse(await readFile(target, 'utf8'));
const previousResults = new Map(
  (existing.publishedCommunities ?? []).map((c) => [c.id, c.computed]),
);

const out = [];
for (const community of PUBLISHED_COMMUNITIES) {
  const entry = { ...community };
  const runner = RUNNERS[community.id];

  if (run && runner) {
    if (!await initGLPK()) throw new Error('GLPK is needed to run a community; npm install first.');
    process.stderr.write(`collecting ${community.id} ...\n`);
    entry.computed = { ...await runner(), at: new Date().toISOString().slice(0, 10) };
    const rate = entry.computed.growthRate ?? entry.computed.growthRateRange?.join(' .. ');
    process.stderr.write(`  growth rate ${rate} /h in ${entry.computed.seconds} s\n`);
  } else if (previousResults.get(community.id)) {
    entry.computed = previousResults.get(community.id);
  }

  out.push(entry);
}

existing.publishedCommunities = out;
await writeFile(target, `${JSON.stringify(existing, null, 2)}\n`);

const withResults = out.filter((c) => c.computed).length;
console.log(`models/scenarios.json: ${out.length} published communities`
  + ` (${withResults} with recorded results, ${PUBLISHED_COMMUNITIES.filter((c) => c.reproduction.status === 'blocked').length} blocked)`);
if (!run) console.log('Pass --run to solve the reproducible ones and record the numbers.');
