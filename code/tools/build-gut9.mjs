#!/usr/bin/env node
/**
 * Import the nine-species gut microbiota model and check it against the two TXT
 * exports that came with it, then solve the community once.
 *
 *   node tools/build-gut9.mjs --dir=<folder with the five model9 files> [--solve]
 *
 * The workbook is the primary source for structure. The TXT files are an
 * independent export of the same model, so they are the check: model9_stoich.txt
 * against the imported stoichiometric matrix, entry by entry and in both
 * directions, and model9_rxns_params.txt against every bound and objective
 * coefficient.
 *
 * That check is also what reveals the one place the workbook is not the primary
 * source. See the note at the top of src/communities/gut9.js.
 */
import { readFile } from 'node:fs/promises';
import { join } from 'node:path';
import {
  importGut9, validateBounds, validateStoichiometry, reactionsBySpecies,
  communityReactionIds, diet, toSteadyComSpec, EXPECTED, PROVENANCE,
} from '../src/communities/gut9.js';

const args = Object.fromEntries(process.argv.slice(2).map((a) => {
  const [k, v = 'true'] = a.replace(/^--/, '').split('=');
  return [k, v];
}));
const dir = args.dir ?? '.';

const { paper, files } = PROVENANCE;
console.log(`${paper.authors} (${paper.year}) — ${paper.framework}`);
console.log(`doi:${paper.doi}`);
console.log(`source: ${Object.values(files).join(', ')} in ${dir}\n`);

const xlsx = await readFile(join(dir, files.model));
const boundsText = await readFile(join(dir, files.bounds), 'utf8');
const stoichText = await readFile(join(dir, files.stoichiometry), 'utf8');

let failures = 0;
const check = (name, ok, detail = '') => {
  if (!ok) failures += 1;
  console.log(`${ok ? 'OK  ' : 'FAIL'}  ${name}${detail ? `  — ${detail}` : ''}`);
};

// Import as the workbook has it, before any bound replacement
const raw = importGut9(xlsx);
console.log('--- imported from the workbook ---');
check('nine species', raw.species.length === EXPECTED.species,
  raw.species.map((s) => s.id).join(' '));
check('reaction count', raw.reactions.size === EXPECTED.reactions,
  `${raw.reactions.size} of ${EXPECTED.reactions}`);
check('metabolite count', raw.metabolites.size === EXPECTED.metabolites,
  `${raw.metabolites.size} of ${EXPECTED.metabolites}`);
check('stoichiometry entries', raw.stoichiometryEntries === EXPECTED.stoichiometryEntries,
  `${raw.stoichiometryEntries} of ${EXPECTED.stoichiometryEntries}`);

// Every species declares its own counts in comSp; they must match what was read.
const bySpecies = reactionsBySpecies(raw);
let countsAgree = true;
for (const sp of raw.species) {
  if (bySpecies.get(sp.id).length !== sp.declaredReactions) countsAgree = false;
}
check('per-species reaction counts match the comSp sheet', countsAgree,
  raw.species.map((s) => `${s.id}:${bySpecies.get(s.id).length}`).join(' '));

const metsBySpecies = new Map(raw.species.map((s) => [s.id, 0]));
for (const met of raw.metabolites.values()) {
  if (metsBySpecies.has(met.species)) metsBySpecies.set(met.species, metsBySpecies.get(met.species) + 1);
}
check('per-species metabolite counts match the comSp sheet',
  raw.species.every((s) => metsBySpecies.get(s.id) === s.declaredMetabolites),
  raw.species.map((s) => `${s.id}:${metsBySpecies.get(s.id)}`).join(' '));

// Community exchange space
console.log('\n--- community exchange space ---');
check('community metabolites', raw.communityMetabolites.length === EXPECTED.communityMetabolites,
  `${raw.communityMetabolites.length}`);
check('one uptake and one export reaction per community metabolite',
  raw.communityUptake.size === EXPECTED.communityMetabolites
    && raw.communityExport.size === EXPECTED.communityMetabolites,
  `UT ${raw.communityUptake.size}, EX ${raw.communityExport.size}`);
check('community exchange reactions belong to no species',
  communityReactionIds(raw).length === EXPECTED.communityExchangeReactions,
  `${communityReactionIds(raw).length}`);
check('species-community exchange reactions',
  raw.speciesExchange.size === EXPECTED.speciesExchangeReactions,
  `${raw.speciesExchange.size} of ${EXPECTED.speciesExchangeReactions}`);
check('EXsp column order is the paper\'s member order',
  raw.exspOrder.join(',') === 'Bt,Fp,Kp,St,Ba,Ec,Ef,Er,Lc', raw.exspOrder.join(','));

// A UT reaction produces its community metabolite; an EX reaction consumes it.
let directionsOk = true;
for (const [met, rxnId] of raw.communityUptake) {
  if (raw.reactions.get(rxnId)?.metabolites[met] !== 1) directionsOk = false;
}
for (const [met, rxnId] of raw.communityExport) {
  if (raw.reactions.get(rxnId)?.metabolites[met] !== -1) directionsOk = false;
}
check('uptake produces and export consumes the community metabolite', directionsOk);

// An EXsp reaction moves one unit from the species' extracellular space into the
// community medium, so a positive flux is secretion.
let exspOk = true;
for (const [key, rxnId] of raw.speciesExchange) {
  const met = key.slice(0, key.indexOf('\u0000'));
  const rxn = raw.reactions.get(rxnId);
  const entries = Object.entries(rxn?.metabolites ?? {});
  if (entries.length !== 2 || rxn.metabolites[met] !== 1) exspOk = false;
  if (!entries.some(([m, c]) => m !== met && c === -1)) exspOk = false;
}
check('each species-community exchange is one species metabolite into the medium', exspOk);

// Biomass and maintenance
console.log('\n--- biomass and ATP maintenance ---');
const objectives = [...raw.reactions.values()].filter((r) => r.objective !== 0);
check('exactly nine objective reactions, the nine biomass reactions',
  objectives.length === EXPECTED.species
    && objectives.every((r) => raw.species.some((s) => s.biomassReaction === r.id)),
  objectives.map((r) => r.id).join(' '));
for (const sp of raw.species) {
  const bm = raw.reactions.get(sp.biomassReaction);
  const atpm = raw.reactions.get(sp.atpMaintenance);
  check(`${sp.id}: ${sp.biomassReaction} / ${sp.atpMaintenance}`,
    Boolean(bm) && Boolean(atpm) && bm.species === sp.id && atpm.species === sp.id,
    atpm ? `ATPM [${atpm.lowerBound}, ${atpm.upperBound}]` : 'missing');
}

// The TXT exports, as independent validation
console.log('\n--- checked against the TXT exports ---');
const stoich = validateStoichiometry(raw, stoichText);
check('stoichiometry matches model9_stoich.txt entry for entry',
  stoich.differing.length === 0 && stoich.onlyInText.length === 0 && stoich.onlyInModel.length === 0,
  `${stoich.identical} identical of ${stoich.entries}`
  + `; ${stoich.differing.length} differing, ${stoich.onlyInText.length} only in the TXT`
  + `, ${stoich.onlyInModel.length} only in the import`);

const before = validateBounds(raw, boundsText);
console.log(`      workbook bounds vs TXT: ${before.identical} identical,`
  + ` ${before.differing.length} differing`);
check('the only bounds the workbook and the TXT disagree on are community uptake',
  before.differing.every((d) => d.id.startsWith('UT_')),
  `${before.differing.length} reactions, all UT_: `
  + `${before.differing.every((d) => d.id.startsWith('UT_'))}`);
check('the workbook carries no diet at all',
  before.differing.length === EXPECTED.dietReactions
    && diet(raw).size === 0,
  `${before.differing.length} uptake reactions closed in the workbook`);

// Import again, with the diet
const model = importGut9(xlsx, { boundsText });
const after = validateBounds(model, boundsText);
console.log('\n--- imported with the diet from model9_rxns_params.txt ---');
check('every bound now matches the TXT', after.differing.length === 0,
  `${after.identical} of ${model.reactions.size}`);
check('the diet opens the expected number of uptake reactions',
  diet(model).size === EXPECTED.dietReactions, `${diet(model).size}`);
const sample = [...diet(model)].slice(0, 4).map(([m, v]) => `${m} ${v}`).join(', ');
console.log(`      diet, first four: ${sample}`);

// Does it grow?
if (args.solve) {
  console.log('\n--- community steady state ---');
  const { initGLPK } = await import('../src/solver/glpk.js');
  const { steadyCom } = await import('../src/core/steadycom.js');
  if (!await initGLPK()) throw new Error('GLPK is required');
  const t0 = Date.now();
  const result = steadyCom(toSteadyComSpec(model), { upperGuess: 0.1 });
  check('the community grows', result.status === 'optimal' && result.growthRate > 0,
    `mu = ${result.growthRate.toFixed(5)} /h in ${((Date.now() - t0) / 1000).toFixed(0)} s`);
  if (result.status === 'optimal') {
    for (const [id, x] of result.abundances) {
      if (x > 1e-6) console.log(`      X_${id} = ${x.toFixed(6)}`);
    }
  }
}

console.log(failures ? `\n${failures} check(s) failed` : '\nAll checks passed');
process.exit(failures ? 1 : 0);
