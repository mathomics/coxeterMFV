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
console.log(`reacciones ${report.counts.reactions} (${JSON.stringify(report.counts.bySpecies)}`
  + `, ${report.counts.community} de la pareja), metabolitos ${report.counts.metabolites}`);
console.log(`compartidos: ${report.counts.shared.join(', ')}`);
console.log(`medidos: ${report.counts.measured.join(' ')}`);
for (const line of report.problems) console.log(`  PROBLEMA  ${line}`);
console.log(report.ok ? 'validación: todo cuadra' : `validación: ${report.problems.length} problemas`);

console.log('\nquién puede dar y quién tomar cada metabolito compartido:');
for (const link of exchangeStructure(model)) {
  console.log(`  ${link.metabolite.padEnd(12)} producen ${link.producers.join('+') || '—'}`
    + `  consumen ${link.consumers.join('+') || '—'}`);
}

if (!args.solve) {
  console.log('\nPasa --solve para resolver el modelo conjunto.');
  process.exit(report.ok ? 0 : 1);
}

const joint = new Model(toJointCobraJSON(model));
const problem = new FluxProblem(joint);
const solution = problem.optimize();
console.log(`\nestado ${solution.status}, objetivo ${solution.objectiveValue?.toFixed(6)}`);

const flux = (id) => solution.fluxes?.get?.(id) ?? solution.fluxes?.[id] ?? 0;
const show = (label, id) => console.log(`  ${label.padEnd(34)} ${flux(id).toFixed(4)}`);
show('crecimiento D. vulgaris (mue)', 'mue');
show('crecimiento M. maripaludis (MC_BIO)', 'MC_BIO');
show('lactato suministrado (D86)', 'D86');
show('H2 que sale de la pareja (H2gen)', 'H2gen');
show('acetato que sale (Acegen)', 'Acegen');
show('metano (M12)', 'M12');
show('mantenimiento ATP (D_ATPase)', 'D_ATPase');
show('sulfato reducido (D01)', 'D01');

console.log('\ntransferencias entre especies, de los flujos:');
const { bySpecies } = reactionsBySpecies(model);
for (const met of [...model.shared].sort()) {
  const moved = [];
  for (const [spId, ids] of bySpecies) {
    let net = 0;
    for (const id of ids) net += (model.reactions.get(id).metabolites[met] ?? 0) * flux(id);
    if (Math.abs(net) > 1e-9) moved.push(`${spId} ${net > 0 ? 'libera' : 'toma'} ${Math.abs(net).toFixed(4)}`);
  }
  console.log(`  ${met.padEnd(12)} ${moved.join(', ') || 'sin flujo'}`);
}

process.exit(report.ok && solution.status === 'optimal' ? 0 : 1);
