#!/usr/bin/env node
/**
 * The curated maps of the seven teaching scenarios, models/maps/<scenario>.json.
 *
 * Each species has one cell template (tools/scenario-maps/teaching/*.mjs) that
 * draws every reaction of its core model; a scenario's map puts its species'
 * cells side by side in the shared medium. A cell's exchanges stand on its
 * outer boundary with a medium node of its own just outside, where the app's
 * cross-feeding lines start; the medium pools themselves, with their community
 * exchanges (supply and washout), stand in a row along the bottom. A scenario
 * with one species has no lines to draw, so its cell's medium nodes are the
 * pools and carry the community exchanges themselves.
 *
 * Equations, bounds and names are read from the bundled models through the
 * app's own CommunityModel, built with each scenario's medium, so every map
 * reaction is the model's reaction. Output is the same on every run.
 *
 * Usage: node tools/scenario-maps/teaching-maps.mjs [--only=<scenario-id>]
 */
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Model } from '../../src/core/model.js';
import { CommunityModel, sharedMetabolite, communityExchange } from '../../src/core/community.js';
import { EditableMap } from '../../src/map/editable-map.js';
import { MAP_DOCUMENT_FORMAT, MAP_DOCUMENT_VERSION } from '../../src/map/map-document.js';
import { instantiate, communityExchangeGeometry, PRIMARY_R } from './teaching/lib.mjs';
import ecoli from './teaching/ecoli.mjs';
import yeast from './teaching/yeast.mjs';
import cyano from './teaching/cyano.mjs';
import methanogen from './teaching/methanogen.mjs';
import lactobacillus from './teaching/lactobacillus.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
const TEMPLATES = {
  ecoli_core_mc: ecoli, yeast_core_mc: yeast, cyano_core_mc: cyano,
  methanogen_core_mc: methanogen, lactobacillus_core_mc: lactobacillus,
};
// Rows of cells, left to right, in the order the scenario's story runs.
const ARRANGEMENT = {
  'consorcio-anaerobio': [['sce', 'lla'], ['eco', 'mba']],
  'aerobio-mixto': [['eco', 'sce']],
  'fototrofo-heterotrofo': [['syn', 'eco', 'sce']],
  metanogenesis: [['eco', 'mba']],
  todas: [['lla', 'eco', 'mba'], ['syn', 'sce']],
  'solo-ecoli': [['eco']],
  'solo-levadura': [['sce']],
};
const GAP = 440;         // medium between two cells: room for the cross-feeding lines
const MARGIN = 300;      // medium between the cells and the edge of the system
const POOL_STEP = 150;   // spacing of the medium pools in their row

const option = (name) => process.argv.slice(2).find((a) => a.startsWith(`--${name}=`))?.slice(name.length + 3) ?? null;

const scenarios = JSON.parse(await readFile(join(root, 'models', 'scenarios.json'), 'utf8')).scenarios;
const models = new Map();
async function loadModel(file) {
  if (!models.has(file)) models.set(file, new Model(JSON.parse(await readFile(join(root, 'models', `${file}.json`), 'utf8'))));
  return models.get(file);
}

/** The map document of one scenario. */
export async function buildScenarioMap(scenario) {
  const rows = ARRANGEMENT[scenario.id];
  if (!rows) throw new Error(`No arrangement for ${scenario.id}`);
  const species = [];
  for (const s of scenario.species) species.push({ id: s.id, file: s.model, model: await loadModel(s.model), abundance: s.abundance ?? 1 / scenario.species.length });
  const listed = rows.flat().sort().join();
  if (listed !== species.map((s) => s.id).sort().join()) throw new Error(`${scenario.id}: arrangement ${listed} does not match its species`);
  // The medium the store builds the community with (Store#mediumSpec).
  const medium = Object.fromEntries(Object.entries(scenario.medium ?? {}).map(([b, v]) => [b, { lower: -Math.abs(Number(v) || 0), upper: 1000 }]));
  const community = new CommunityModel({
    name: scenario.name, objective: scenario.objective, medium,
    species: species.map((s) => ({ id: s.id, model: s.model, abundance: s.abundance, enabled: true })),
  });
  const single = species.length === 1;
  const templates = new Map(species.map((s) => [s.id, TEMPLATES[s.file](s.model)]));

  // Cells in rows, each row centred, the cells of a row centred on its middle line
  const rowSizes = rows.map((row) => ({
    w: row.reduce((sum, id) => sum + templates.get(id).width, 0) + GAP * (row.length - 1),
    h: Math.max(...row.map((id) => templates.get(id).height)),
  }));
  const width = Math.max(...rowSizes.map((r) => r.w)) + 2 * MARGIN;
  let y = MARGIN;
  const placed = [];
  rows.forEach((row, i) => {
    let x = (width - rowSizes[i].w) / 2;
    for (const id of row) {
      const tpl = templates.get(id);
      placed.push({ id, x: Math.round(x), y: Math.round(y + (rowSizes[i].h - tpl.height) / 2) });
      x += tpl.width + GAP;
    }
    y += rowSizes[i].h + GAP;
  });
  const cellsBottom = y - GAP;

  const nodes = new Map();
  const reactions = new Map();
  const segments = [];
  const texts = [];
  const cells = [];
  const mediumNodes = [];
  const exchanges = [];
  const missing = [];
  for (const { id, x, y: cy } of placed) {
    const sp = species.find((s) => s.id === id);
    const out = instantiate(templates.get(id), {
      sp: id, ox: x, oy: cy, community, abundance: sp.abundance, mediumMode: single ? 'own' : 'copy',
    });
    cells.push(out.cell);
    for (const [k, v] of out.nodes) { if (nodes.has(k)) throw new Error(`duplicate node ${k}`); nodes.set(k, v); }
    for (const [k, v] of out.reactions) reactions.set(k, v);
    segments.push(...out.segments);
    texts.push(...out.texts);
    missing.push(...out.missing.map((r) => `${id}@${r}`));
    for (const m of out.medium) {
      mediumNodes.push(m.nodeId);
      if (single) {
        const geo = communityExchangeGeometry(community, m.base, nodes.get(m.nodeId), m.normal);
        if (geo) { reactions.set(geo.reaction.id, geo.reaction); segments.push(...geo.segments); exchanges.push(geo.reaction.id); }
      }
    }
  }

  let height = cellsBottom + MARGIN;
  if (!single) {
    // The pools: what the medium supplies first, in the order the scenario lists it.
    const bases = [...community.transfersByBase.keys()];
    const supplied = Object.keys(scenario.medium ?? {}).filter((b) => bases.includes(b) && Number(scenario.medium[b]) > 0);
    const order = [...supplied, ...bases.filter((b) => !supplied.includes(b)).sort()];
    // Low enough that the cross-feeding lines, which keep to the middle of the
    // medium below the cells, run above the pools rather than through them.
    const poolY = cellsBottom + 520;
    const x0 = (width - POOL_STEP * (order.length - 1)) / 2;
    order.forEach((base, i) => {
      const mid = sharedMetabolite(base);
      const info = community.merged.metabolites.get(mid);
      const id = `met:${mid}`;
      const node = {
        id, kind: 'medium', mergedId: mid, localId: base, base, name: info.name, formula: info.formula, compartment: 's',
        cellId: null, x: Math.round(x0 + i * POOL_STEP), y: poolY, r: PRIMARY_R, primary: true, visualRole: 'primary',
        labelAngle: -Math.PI / 2,
      };
      nodes.set(id, node);
      mediumNodes.push(id);
      const geo = communityExchangeGeometry(community, base, node, [0, 1]);
      if (geo) { reactions.set(geo.reaction.id, geo.reaction); segments.push(...geo.segments); exchanges.push(geo.reaction.id); }
    });
    // The row's title stands at its left end, out of the lines' way.
    const purpose = 'One pool per metabolite, supplied or washed out; every cell trades with these pools';
    const tx = Math.round(x0 - 90 - purpose.length * 16 * 0.52);
    texts.push({ id: 'pathway:medium', text: 'Shared medium', x: tx, y: poolY - 8, fontSize: 24 });
    texts.push({ id: 'purpose:medium', text: purpose, x: tx, y: poolY + 18, fontSize: 16 });
    height = poolY + 96 + 130;
  }

  // Everything the community has, and nothing it does not
  for (const id of reactions.keys()) if (!community.merged.reactions.has(id)) throw new Error(`${scenario.id}: ${id} is not in the model`);
  for (const id of community.merged.reactions.keys()) {
    if (id === 'COMMUNITY_GROWTH') continue;
    if (!reactions.has(id) && !missing.includes(id)) missing.push(id);
  }

  const map = new EditableMap({
    canvas: { width, height },
    environment: { rect: { x: 26, y: 26, w: width - 52, h: height - 52 }, id: 's', label: 'Shared medium' },
    cells, medium: { nodes: mediumNodes, exchanges }, nodes, reactions, segments,
    texts: new Map(texts.map((t) => [t.id, t])),
  });
  const json = map.toJSON();
  const doc = {
    format: MAP_DOCUMENT_FORMAT,
    version: MAP_DOCUMENT_VERSION,
    metadata: {
      createdAt: null,
      scenario: scenario.id,
      species: species.map((s) => ({ id: s.id, model: s.file })).sort((a, b) => a.id.localeCompare(b.id)),
      reactionCount: json.reactions.length,
      nodeCount: json.nodes.length,
      segmentCount: json.segments.length,
      textCount: json.texts.length,
    },
    pathwayManager: null,
    fluxColorStops: null,
    map: json,
  };
  return { doc, missing, community };
}

async function main() {
  const only = option('only');
  for (const id of Object.keys(ARRANGEMENT)) {
    if (only && only !== id) continue;
    const scenario = scenarios.find((s) => s.id === id);
    if (!scenario) throw new Error(`Scenario ${id} is not in models/scenarios.json`);
    const { doc, missing } = await buildScenarioMap(scenario);
    const out = join(root, 'models', 'maps', `${id}.json`);
    await mkdir(dirname(out), { recursive: true });
    await writeFile(out, `${JSON.stringify(doc)}\n`);
    const m = doc.metadata;
    console.log(`${id}: ${m.reactionCount} reactions, ${m.nodeCount} nodes, ${m.segmentCount} segments, ${m.textCount} texts${missing.length ? `; not drawn: ${missing.join(', ')}` : ''}`);
  }
}
if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) await main();
