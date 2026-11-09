/**
 * Figure 3, from the Zuñiga et al. (2020) case as Coxeter-MFV opens it.
 *
 * Opens the case from the scenario list (which downloads and hash-checks the
 * authors' two MAT files), solves the three scenarios on the complete source
 * matrices, and exports the two-cell map of the ΔpheA coculture twice, with the
 * solved fluxes and as the difference from the WT monoculture.
 *
 *   node coxeter_zuniga.mjs
 */
import { writeFile, mkdir } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { startGLPK, createStore, exportSVG } from '../lib/coxeter-headless.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const OUT = path.join(HERE, 'results');

await startGLPK();
await mkdir(OUT, { recursive: true });
const store = await createStore({ init: true });
await store.useZunigaStudy({ scenarioId: 'mutant-coculture' });

const scenarios = {};
for (const id of ['wt-monoculture', 'mutant-monoculture', 'mutant-coculture']) {
  const result = store.useZunigaScenario(id, { emit: false });
  scenarios[id] = {
    status: result?.status,
    growth: result?.zuniga?.growth ?? null,
    pheSecretedBySe: result?.fluxes?.get('Se@PHELtex') ?? null,
    pheTakenUpByEc: result?.fluxes?.get('Ec@PHEtex') ?? null,
  };
}

store.useZunigaScenario('mutant-coculture', { emit: false });
store.setZunigaComparison('absolute');
store.refreshFluxScale();
await writeFile(path.join(OUT, 'zuniga_map_coculture_solved.svg'), await exportSVG(store));
// The color scale of that map, for the legend Figure 3 draws beside it.
const scale = { reference: store.scale.reference, legend: store.scale.legendStops(6) };
store.setZunigaComparison('difference');
store.refreshFluxScale();
await writeFile(path.join(OUT, 'zuniga_map_coculture_difference.svg'), await exportSVG(store));

const summary = {
  scenarios,
  knockouts: [...(store.zunigaPrepared?.knockouts ?? [])],
  drawnReactions: store.layout?.reactions?.size ?? null,
  fluxReference: store.zunigaState()?.fluxReference ?? null,
  scale,
};
await writeFile(path.join(OUT, 'coxeter_zuniga_summary.json'), JSON.stringify(summary, null, 1));
console.log(JSON.stringify(summary, null, 1));
