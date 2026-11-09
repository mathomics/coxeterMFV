/**
 * Figure 2 fluxes, solved by Coxeter-MFV itself.
 *
 * Opens each Yeast9 model with the application's importer, opens the
 * cytoplasm, mitochondrion and peroxisome map over it, runs pFBA as the
 * Simulate panel does (100 % of the optimal growth) and writes the fluxes and
 * an exported SVG of the map. Each model is also solved directly with the
 * FluxProblem the store builds on, as a check that the community wrapper the
 * importer puts around a single model does not change the answer.
 *
 *   node coxeter_pfba.mjs
 */
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { ROOT, startGLPK, createStore, exportSVG } from '../lib/coxeter-headless.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const OUT = path.join(HERE, 'results');
const MAP = path.join(HERE, 'models', 'yeast9_cytosol_mitochondria_peroxisome.json');
const CONDITIONS = [
  { id: 'aerobic', file: 'yeastgem_v9.1.1-edited.json' },
  { id: 'anaerobic', file: 'yeastgem_v9.1.1_anaerobic.json' },
];

await startGLPK();
const { Model } = await import(path.join(ROOT, 'src', 'core', 'model.js'));
const { FluxProblem } = await import(path.join(ROOT, 'src', 'solver', 'fba.js'));
await mkdir(OUT, { recursive: true });

const { FluxScale } = await import(path.join(ROOT, 'src', 'render', 'theme.js'));
const mapText = await readFile(MAP, 'utf8');
const summary = {};
const opened = [];

for (const condition of CONDITIONS) {
  const text = await readFile(path.join(HERE, 'models', condition.file), 'utf8');

  const store = await createStore();
  await store.importModelFiles([{ name: condition.file, text }]);
  await store.loadMapDocument(mapText, { name: path.basename(MAP) });
  store.parsimonious = true;
  store.pfbaBiomassPercent = 100;
  store.view.widthByFlux = true;
  const solution = store.solve({ silent: true });
  store.refreshFluxScale();
  if (solution?.status !== 'optimal') throw new Error(`${condition.id}: ${solution?.status}`);

  // The importer namespaces every reaction by its species, which is the one
  // model here, so the original id is what follows the separator.
  const fluxes = new Map();
  for (const [id, value] of solution.fluxes) {
    const original = store.community.reactionOwner.get(id)?.originalId;
    if (original) fluxes.set(original, value);
  }
  const drawn = [...store.layout.reactions?.keys?.() ?? []]
    .map((id) => store.community.reactionOwner.get(id)?.originalId ?? id);

  const direct = new FluxProblem(new Model(JSON.parse(text))).parsimonious({ fraction: 1 });

  await writeFile(path.join(OUT, `coxeter_pfba_${condition.id}.csv`),
    'reaction,flux\n' + [...fluxes].map(([id, v]) => `${id},${v}`).join('\n') + '\n');
  await writeFile(path.join(OUT, `coxeter_direct_pfba_${condition.id}.csv`),
    'reaction,flux\n' + [...direct.fluxes].map(([id, v]) => `${id},${v}`).join('\n') + '\n');
  opened.push({ condition, store, drawnFluxes: [...store.layout.reactions.keys()].map((id) => store.solution.fluxes.get(id) ?? 0) });

  summary[condition.id] = {
    status: solution.status,
    growth: fluxes.get('r_2111'),
    objective: solution.objectiveValue,
    totalAbsoluteFlux: [...fluxes].filter(([id]) => id !== 'COMMUNITY_GROWTH').reduce((s, [, v]) => s + Math.abs(v), 0),
    directStatus: direct.status,
    directGrowth: direct.fluxes.get('r_2111'),
    directTotalAbsoluteFlux: [...direct.fluxes.values()].reduce((s, v) => s + Math.abs(v), 0),
    drawnReactions: drawn.length,
    mapLoadNote: store.mapLoadNote || null,
  };
}

// One colour and width scale for both conditions, built from every drawn flux
// of both, so the same colour means the same flux in either panel.
const shared = new FluxScale(opened.flatMap((o) => o.drawnFluxes));
for (const { condition, store } of opened) {
  store.scale = shared;
  await writeFile(path.join(OUT, `yeast9_map_${condition.id}.svg`), await exportSVG(store));
}
summary.sharedScale = { reference: shared.reference, max: shared.max, legend: shared.legendStops(6) };

await writeFile(path.join(OUT, 'coxeter_summary.json'), JSON.stringify(summary, null, 1));
console.log(JSON.stringify(summary, null, 1));
