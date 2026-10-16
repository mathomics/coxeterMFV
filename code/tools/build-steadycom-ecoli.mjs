#!/usr/bin/env node
/**
 * Reproduce the four-auxotroph E. coli community from the SteadyCom paper and
 * print the growth rate, abundances and realised cross-feeding.
 *
 * The community itself is defined in src/communities/steadycom-ecoli.js, which
 * carries the provenance of every constraint. This is only the runner.
 *
 * Usage: node tools/build-steadycom-ecoli.mjs [path/to/iAF1260.xml]
 *
 * Takes a few minutes: 21 LPs over four genome-scale models, each about 38 000
 * rows once the bound-coupling rows are in.
 */
import { readFile } from 'node:fs/promises';
import { initGLPK } from '../src/solver/glpk.js';
import { steadyCom } from '../src/core/steadycom.js';
import {
  buildCommunity, PROVENANCE, EXPECTED_GROWTH_RATE, AMINO_ACID_EXCHANGES,
} from '../src/communities/steadycom-ecoli.js';

const source = process.argv[2] ?? '/home/user/mmc/sbml/iAF1260.xml';

if (!await initGLPK()) {
  console.error('This needs the GLPK backend; run `npm install` first.');
  process.exit(1);
}

const { paper, model } = PROVENANCE;
console.log(`${paper.authors} (${paper.year}) — ${paper.framework}`);
console.log(`doi:${paper.doi}`);
console.log(`model: ${model.id} (${model.organism}) from ${source}\n`);

const spec = buildCommunity(await readFile(source, 'utf8'));
console.log(`${spec.members.length} strains built from ${spec.document.reactions.length} reactions`);
for (const m of spec.members) {
  console.log(`  ${m.id}: auxotroph for ${m.strain.auxotrophFor.join(' + ')},`
    + ` cannot export ${m.strain.cannotExport}, so it supplies ${m.strain.supplies}`);
}
console.log(`community uptake: ${Object.keys(spec.communityUptake).length} metabolites`
  + ` (glucose ${spec.communityUptake.glc})\n`);

const t0 = Date.now();
const result = steadyCom(spec, { upperGuess: 1.0 });
if (result.status !== 'optimal') {
  console.error(`failed: ${result.message}`);
  process.exit(1);
}

console.log(`solved in ${((Date.now() - t0) / 1000).toFixed(1)} s over ${result.iterations} LPs`);
console.log(`  maximum community growth rate = ${result.growthRate.toFixed(5)} /h`);
console.log(`  reported by the tutorial       = ${EXPECTED_GROWTH_RATE} /h\n`);

console.log('  relative abundance:');
for (const [id, x] of result.abundances) console.log(`    X_${id} = ${x.toFixed(6)}`);

console.log('\n  amino-acid exchange (+ secreted, - taken up, mmol/h per gDW of community):');
for (const member of spec.members) {
  const row = Object.entries(AMINO_ACID_EXCHANGES).map(([aa, rid]) => {
    const v = result.fluxes.get(`${member.id}|${rid}`) ?? 0;
    return `${aa} ${v >= 0 ? '+' : ''}${v.toFixed(4)}`;
  }).join('   ');
  console.log(`    ${member.id}  ${row}`);
}
