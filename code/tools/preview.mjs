#!/usr/bin/env node
/**
 * Render a community map straight to an HTML file, for eyeballing the layout
 * without starting the app. Usage: node tools/preview.mjs [out.html] [species...]
 */
import { readFile, writeFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Model } from '../src/core/model.js';
import { CommunityModel, OBJECTIVES } from '../src/core/community.js';
import { FluxProblem } from '../src/solver/fba.js';
import { initGLPK } from '../src/solver/glpk.js';
import { layoutCommunity } from '../src/layout/layout.js';
import { renderMap } from '../src/render/mapview.js';
import { FluxScale } from '../src/render/theme.js';

await initGLPK().catch(() => false);

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const out = process.argv[2] || join(root, 'preview.html');
const wanted = process.argv.slice(3);

const DEFAULT = [
  ['lla', 'lactobacillus_core_mc'],
  ['eco', 'ecoli_core_mc'],
  ['sce', 'yeast_core_mc'],
  ['mba', 'methanogen_core_mc'],
];

const chosen = wanted.length
  ? wanted.map((w, i) => [w.slice(0, 3), w])
  : DEFAULT;

const species = [];
for (const [id, file] of chosen) {
  const doc = JSON.parse(await readFile(join(root, 'models', `${file}.json`), 'utf8'));
  species.push({ id, model: new Model(doc), abundance: 1 / chosen.length });
}

const community = new CommunityModel({
  name: 'Vista previa',
  objective: OBJECTIVES.EQUAL_GROWTH,
  species,
  medium: {
    glc__D: { lower: -10 }, nh4: { lower: -1000 }, pi: { lower: -1000 },
    h2o: { lower: -1000 }, h: { lower: -1000 }, co2: { lower: -1000 },
    o2: { lower: 0 }, photon: { lower: -60 },
  },
});

const t0 = Date.now();
const solution = new FluxProblem(community.merged).parsimonious();
const tSolve = Date.now() - t0;

const t1 = Date.now();
const layout = layoutCommunity(community);
const tLayout = Date.now() - t1;

const scale = new FluxScale(solution.fluxes.values());
const t2 = Date.now();
const svg = renderMap(layout, {
  fluxes: solution.fluxes,
  scale,
  labels: 'auto',
  growth: community.growthRates(solution.fluxes),
  crossFeeding: community.crossFeeding(solution.fluxes),
});
const tRender = Date.now() - t2;

const css = await readFile(join(root, 'assets', 'map.css'), 'utf8');
await writeFile(out, `<!doctype html><html lang="es"><head><meta charset="utf-8">
<title>Vista previa del mapa</title>
<style>
  body { margin:0; background:#eef2f7; font-family: system-ui, sans-serif; }
  .wrap { padding: 12px; }
  svg { width: 100%; height: auto; display:block; background:#fff; border-radius: 14px; }
${css}
</style></head><body><div class="wrap">${svg}</div></body></html>`, 'utf8');

console.log(`status ${solution.status}  community mu = ${solution.objectiveValue.toFixed(5)}`);
console.log(`solve ${tSolve} ms · layout ${tLayout} ms · render ${tRender} ms`);
console.log(`canvas ${layout.width}x${layout.height} · ${layout.nodes.size} nodes · ${layout.reactions.size} reactions · ${layout.segments.length} segments`);
console.log(`wrote ${out}`);
