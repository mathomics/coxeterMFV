/**
 * Figure 4, solved by Coxeter-MFV itself.
 *
 * Opens the four auxotrophic E. coli strains of Chan et al. (2017) as the
 * Simulate tab does, calculates the permitted range of the community growth
 * rate, then solves SteadyCom at μmax and at half of it with the Parsimonious
 * option on (the composition and fluxes with the least total member flux). At
 * each rate the abundance range of every strain comes from steadyComFVA in
 * src/core/steadycom.js, the module the application solves with. That range is
 * not offered in the interface, so it is the one number here not produced by a
 * button.
 *
 *   node coxeter_steadycom.mjs
 */
import { writeFile, mkdir } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { ROOT, startGLPK, createStore, exportSVG } from '../lib/coxeter-headless.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const OUT = path.join(HERE, 'results');
const COMMUNITY = 'steadycom-ecoli-auxotrophs';
const PUBLISHED_MU_MAX = 0.73599;
const AMINO_ACIDS = ['arg__L', 'lys__L', 'met__L', 'phe__L'];

await startGLPK();
await mkdir(OUT, { recursive: true });
// A file URL, not a path: on Windows import() reads "C:" as a URL scheme.
const { steadyComFVA } = await import(pathToFileURL(path.join(ROOT, 'src', 'core', 'steadycom.js')));
const { FluxScale } = await import(pathToFileURL(path.join(ROOT, 'src', 'render', 'theme.js')));
const { crossFeedingColors } = await import(pathToFileURL(path.join(ROOT, 'src', 'render', 'crossfeed-colors.js')));

const store = await createStore({ init: true });
await store.usePublishedCommunity(COMMUNITY);
const started = performance.now();
const range = await store.steadyComRange();
const rangeSeconds = (performance.now() - started) / 1000;
const muMax = range.max;
const spec = await store.steadyComProblem();
const members = store.species.map((sp) => sp.id);

store.setSteadyComParsimonious(true);
const points = [];
for (const fraction of [1, 0.5]) {
  // μmax is the largest rate the search verified, so it is solved exactly.
  const mu = fraction === 1 ? muMax : muMax * fraction;
  const at = await store.solveSteadyComAt(mu);
  const fva = steadyComFVA(spec, {
    growthFraction: fraction,
    tolerance: 1e-5,
    targets: members.map((id) => ({ name: id, biomass: { [id]: 1 } })),
  });
  // Cross-feeding as the map draws it: who secretes and who takes up each
  // amino acid, from the member exchange fluxes of this solution.
  const crossFeeding = (store.crossFeeding?.links ?? [])
    .filter((link) => AMINO_ACIDS.some((aa) => String(link.base).startsWith(aa.split('__')[0])));
  points.push({
    fraction,
    mu,
    abundances: Object.fromEntries(members.map((id) => [id, at.abundances.get(id) ?? 0])),
    abundanceRanges: Object.fromEntries(fva.ranges.map((r) => [r.name, [r.min, r.max]])),
    fvaRelaxations: fva.relaxations ?? 0,
    totalFlux: at.totalFlux,
    crossFeeding,
    fluxes: new Map(store.solution.fluxes),
    drawnFluxes: [...store.layout.reactions.keys()].map((id) => store.solution.fluxes.get(id) ?? 0),
    growth: Object.fromEntries(store.growth ?? []),
  });
}

// The two rates share one scale, so a colour means the same flux in both
// panels. Each rate is solved again to put its solution back in the store
// before its map is exported.
const shared = new FluxScale(points.flatMap((p) => p.drawnFluxes));
// The maps draw each cross-feeding exchange as a route through the medium, as
// the Cross-feeding button does.
store.setView({ showCrossFeeding: true });
// The color of each route on those maps, so Figure 4 can name the routes.
const routeColors = Object.fromEntries(crossFeedingColors(store.crossFeedingVisibleBases()));
for (const point of points) {
  await store.solveSteadyComAt(point.mu);
  store.scale = shared;
  const label = point.fraction === 1 ? 'mumax' : 'half_mumax';
  await writeFile(path.join(OUT, `auxotrophs_map_${label}.svg`), await exportSVG(store));
  await writeFile(path.join(OUT, `auxotrophs_fluxes_${label}.csv`),
    'reaction,flux\n' + [...point.fluxes].map(([id, v]) => `${id},${v}`).join('\n') + '\n');
}

const summary = {
  community: COMMUNITY,
  publishedMuMax: PUBLISHED_MU_MAX,
  muMax,
  matchesPublishedToFiveDecimals: Math.abs(muMax - PUBLISHED_MU_MAX) < 5e-6,
  rangeIterations: range.iterations,
  rangeSeconds: Math.round(rangeSeconds),
  points: points.map(({ fraction, mu, abundances, abundanceRanges, fvaRelaxations, totalFlux, crossFeeding, growth }) =>
    ({ fraction, mu, abundances, abundanceRanges, fvaRelaxations, totalFlux, crossFeeding, growth })),
  sharedScale: { reference: shared.reference, max: shared.max, legend: shared.legendStops(6) },
  routeColors,
};
await writeFile(path.join(OUT, 'coxeter_steadycom_summary.json'), JSON.stringify(summary, null, 1));
console.log(JSON.stringify(summary, null, 1));
