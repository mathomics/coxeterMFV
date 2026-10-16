/**
 * Publish calculated numerical facts only, never the full third-party models.
 * Writes models/published/zuniga-2020-phea-rescue/results.json from the output
 * of validate.mjs and records the coculture summary in models/scenarios.json.
 *
 *   node tools/zuniga/publish-results.mjs LOCAL_RESULTS/validation.json [PHE_FVA.json]
 */
import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const input = process.argv[2];
if (!input) throw new Error('Usage: node tools/zuniga/publish-results.mjs LOCAL_RESULTS/validation.json [PHE_FVA.json]');
const folder = path.join(root, 'models/published/zuniga-2020-phea-rescue');
const manifest = JSON.parse(await readFile(path.join(folder, 'manifest.json'), 'utf8'));
const calculated = JSON.parse(await readFile(input, 'utf8'));
if (!calculated.publishedPredictionsMatched || calculated.scenarios?.length !== 3) throw new Error('Run the scientific validation before publishing results.');
for (const source of manifest.sources) {
  if (calculated.sourceHashes[source.file] !== source.sha256) throw new Error('Calculated data uses another source model.');
}
const payload = {
  schemaVersion: 1, id: manifest.id, method: manifest.formulation.method,
  sourceHashes: calculated.sourceHashes, publishedPredictionsMatched: true,
  reference: manifest.reproduction.referenceValues, tolerance: manifest.reproduction.tolerance,
  scenarios: calculated.scenarios.map(scenario => ({
    id: scenario.id, source: scenario.source, status: scenario.status,
    objectiveValue: scenario.objectiveValue, growth: scenario.growth,
    knockouts: scenario.knockouts, fluxes: scenario.fluxes,
    validation: scenario.validation,
  })),
  interpretation: 'One stationary optimal distribution per scenario. Published mutant growth and phenylalanine exchange are reproduced; WT is a calculated reference, and other fluxes may have alternative optima.',
};
if (process.argv[3]) payload.phenylalanineFVAAtSourceOptimum = JSON.parse(await readFile(process.argv[3], 'utf8'));
await writeFile(path.join(folder, 'results.json'), JSON.stringify(payload, null, 2) + '\n');
const scenariosPath = path.join(root, 'models/scenarios.json');
const scenarios = JSON.parse(await readFile(scenariosPath, 'utf8'));
const entry = scenarios.publishedCommunities.find(item => item.id === manifest.id);
const co = payload.scenarios.find(item => item.id === 'mutant-coculture');
entry.computed = { growthRates: co.growth, phenylalanineTransfer: co.fluxes['Ec@PHEtex'],
  status: 'optimal', source: 'Calculated from complete pinned source matrices; Supplementary Data 9 matched within 1e-8',
  scenarios: payload.scenarios.map(({ id, growth, objectiveValue }) => ({ id, growth, objectiveValue })) };
delete entry.result;
await writeFile(scenariosPath, JSON.stringify(scenarios, null, 2) + '\n');
console.log('Published three numerical solutions and their reference checks; no full models copied.');
