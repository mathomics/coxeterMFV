#!/usr/bin/env node
/**
 * Import, validate and solve the Stolyar 2007 two-species model.
 *
 *   node tools/build-stolyar.mjs --dir=<folder with the four supplement files>
 *   node tools/build-stolyar.mjs --dir=… --solve
 *
 * The files are the paper's supplementary material and are not in this
 * repository. src/communities/published.js records where they come from.
 */
import { readFile } from 'node:fs/promises';
import { join } from 'node:path';

import { importStolyar, validateStolyar, toJointCobraJSON, exchangeStructure, reactionsBySpecies }
  from '../src/communities/stolyar.js';
import { Model } from '../src/core/model.js';
import { FluxProblem } from '../src/solver/fba.js';

const args = Object.fromEntries(process.argv.slice(2).map((a) => {
  const [k, v = 'true'] = a.replace(/^--/, '').split('=');
  return [k, v];
}));

if (!args.dir) {
  console.error('Pass --dir=<folder with reactions.txt, metabolites.txt, macromolecules.txt and macromolecule_synthesis.txt>');
  process.exit(2);
}

const read = (name) => readFile(join(args.dir, name), 'utf8');
const model = importStolyar({
  reactions: await read('reactions.txt'),
  metabolites: await read('metabolites.txt'),
  macromolecules: await read('macromolecules.txt'),
  synthesis: await read('macromolecule_synthesis.txt'),
});

const report = validateStolyar(model);
console.log(`reactions ${report.counts.reactions} (${JSON.stringify(report.counts.bySpecies)}`
  + `, ${report.counts.community} of the pair), metabolites ${report.counts.metabolites}`);
console.log(`shared: ${report.counts.shared.join(', ')}`);
console.log(`measured: ${report.counts.measured.join(' ')}`);
for (const line of report.problems) console.log(`  PROBLEM  ${line}`);
console.log(report.ok ? 'validation: everything checks out' : `validation: ${report.problems.length} problems`);

console.log('\nwho can give and who can take each shared metabolite:');
for (const link of exchangeStructure(model)) {
  console.log(`  ${link.metabolite.padEnd(12)} produced by ${link.producers.join('+') || '—'}`
    + `  consumed by ${link.consumers.join('+') || '—'}`);
}

if (!args.solve) {
  console.log('\nPass --solve to solve the joint model.');
  process.exit(report.ok ? 0 : 1);
}

const joint = new Model(toJointCobraJSON(model));
const problem = new FluxProblem(joint);
const solution = problem.optimize();
console.log(`\nstatus ${solution.status}, objective ${solution.objectiveValue?.toFixed(6)}`);

const flux = (id) => solution.fluxes?.get?.(id) ?? solution.fluxes?.[id] ?? 0;
const show = (label, id) => console.log(`  ${label.padEnd(34)} ${flux(id).toFixed(4)}`);
show('growth D. vulgaris (mue)', 'mue');
show('growth M. maripaludis (MC_BIO)', 'MC_BIO');
show('lactate supplied (D86)', 'D86');
show('H2 leaving the pair (H2gen)', 'H2gen');
show('acetate leaving (Acegen)', 'Acegen');
show('methane (M12)', 'M12');
show('ATP maintenance (D_ATPase)', 'D_ATPase');
show('sulfate reduced (D01)', 'D01');

console.log('\ntransfers between species, from the fluxes:');
const { bySpecies } = reactionsBySpecies(model);
for (const met of [...model.shared].sort()) {
  const moved = [];
  for (const [spId, ids] of bySpecies) {
    let net = 0;
    for (const id of ids) net += (model.reactions.get(id).metabolites[met] ?? 0) * flux(id);
    if (Math.abs(net) > 1e-9) moved.push(`${spId} ${net > 0 ? 'releases' : 'takes'} ${Math.abs(net).toFixed(4)}`);
  }
  console.log(`  ${met.padEnd(12)} ${moved.join(', ') || 'no flux'}`);
}

process.exit(report.ok && solution.status === 'optimal' ? 0 : 1);
