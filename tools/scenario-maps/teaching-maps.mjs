#!/usr/bin/env node
/**
 * The curated maps of the seven teaching scenarios, models/maps/<scenario>.json.
 *
 * Each species has one cell template (tools/scenario-maps/teaching/*.mjs) that
 * draws every reaction of its core model; a scenario's map sets its species'
 * cells out in columns in the shared medium, the same distance apart
 * everywhere, on a canvas no larger than what it holds. A cell's exchanges
 * stand on its outer boundary with a medium node just outside, in the medium
 * between the cells, where the app's cross-feeding lines start. One of those
 * nodes per metabolite is the shared pool itself and carries the community
 * exchange (supply and washout): the one with the most open medium in front of
 * it, so the exchange points out of the community rather than at a
 * neighbouring cell. The others are drawings of the same pool beside each
 * other cell that trades it. A scenario with one species has no lines to
 * draw, so every medium node of its cell is a pool.
 *
 * The maps draw their cofactors quiet (cofactorStyle 'quiet'), which View ▸
 * Quiet cofactors switches off; stoichiometry is complete either way.
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
import { CommunityModel, sharedMetabolite } from '../../src/core/community.js';
import { EditableMap } from '../../src/map/editable-map.js';
import { MAP_DOCUMENT_FORMAT, MAP_DOCUMENT_VERSION } from '../../src/map/map-document.js';
import { routeGrid } from '../../src/render/crossfeed-routes.js';
import { instantiate, communityExchangeGeometry } from './teaching/lib.mjs';
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
// Columns of cells, left to right, each listed top to bottom, in the order
// the scenario's story runs.
const ARRANGEMENT = {
  'consorcio-anaerobio': [['lla', 'mba'], ['eco'], ['sce']],
  'aerobio-mixto': [['eco'], ['sce']],
  'fototrofo-heterotrofo': [['syn'], ['eco'], ['sce']],
  metanogenesis: [['eco'], ['mba']],
  todas: [['lla', 'mba'], ['eco', 'syn'], ['sce']],
  'solo-ecoli': [['eco']],
  'solo-levadura': [['sce']],
};
// Medium between two cells: room for both cells' medium nodes and a corridor
// for the cross-feeding lines. Their router works on squares of four grid
// steps (routeGrid, which grows with the canvas) and blocks every square a
// cell touches, so a gap of four squares always leaves one open.
const GAP = 400;
const CORRIDOR = 16;    // grid steps in the narrowest gap
const PAD = 200;     // medium between the outermost drawing and the edge of the canvas
const PLATE = 44;    // a cell's name plate stands this far above its boundary
const FRAME = 26;    // the system boundary, inside the canvas edge

const option = (name) => process.argv.slice(2).find((a) => a.startsWith(`--${name}=`))?.slice(name.length + 3) ?? null;

const scenarios = JSON.parse(await readFile(join(root, 'models', 'scenarios.json'), 'utf8')).scenarios;
const models = new Map();
async function loadModel(file) {
  if (!models.has(file)) models.set(file, new Model(JSON.parse(await readFile(join(root, 'models', `${file}.json`), 'utf8'))));
  return models.get(file);
}

/**
 * Where each cell's top left stands: columns side by side `gap` apart, the
 * cells of a column `gap` apart, each column centred on the tallest. Within a
 * column the cells line up on the side facing the next column, so two cells
 * facing each other across a gap are `gap` apart.
 */
function arrange(columns, templates, gap) {
  const widths = columns.map((col) => Math.max(...col.map((id) => templates.get(id).width)));
  const heights = columns.map((col) => col.reduce((sum, id) => sum + templates.get(id).height, 0) + gap * (col.length - 1));
  const tallest = Math.max(...heights);
  const placed = new Map();
  let x = 0;
  columns.forEach((col, i) => {
    let y = (tallest - heights[i]) / 2;
    for (const id of col) {
      const t = templates.get(id);
      const free = widths[i] - t.width;
      const align = columns.length === 1 ? 0.5 : i === 0 ? 1 : i === columns.length - 1 ? 0 : 0.5;
      placed.set(id, { x: Math.round(x + free * align), y: Math.round(y) });
      y += t.height + gap;
    }
    x += widths[i] + gap;
  });
  return placed;
}

/** How far the medium is open in front of a node: to the nearest other cell along its normal, or Infinity. */
function openAhead(m, rects, own) {
  let best = Infinity;
  const [nx, ny] = m.normal;
  for (const [id, r] of rects) {
    if (id === own) continue;
    const across = nx ? [r.y, r.y + r.h] : [r.x, r.x + r.w];
    const at = nx ? m.at[1] : m.at[0];
    if (at < across[0] - 40 || at > across[1] + 40) continue;
    const d = nx > 0 ? r.x - m.at[0] : nx < 0 ? m.at[0] - (r.x + r.w)
      : ny > 0 ? r.y - m.at[1] : m.at[1] - (r.y + r.h);
    if (d > 0) best = Math.min(best, d);
  }
  return best;
}

/** Every cell of a scenario at its place, offset by (ox, oy), with the shared pools. */
function draw(species, community, templates, placed, ox, oy) {
  const single = species.length === 1;
  const nodes = new Map();
  const reactions = new Map();
  const segments = [];
  const texts = [];
  const cells = [];
  const medium = [];
  const missing = [];
  for (const sp of species) {
    const { x, y } = placed.get(sp.id);
    const out = instantiate(templates.get(sp.id), {
      sp: sp.id, ox: x + ox, oy: y + oy, community, abundance: sp.abundance, mediumMode: single ? 'own' : 'copy',
    });
    cells.push(out.cell);
    for (const [k, v] of out.nodes) { if (nodes.has(k)) throw new Error(`duplicate node ${k}`); nodes.set(k, v); }
    for (const [k, v] of out.reactions) reactions.set(k, v);
    segments.push(...out.segments);
    texts.push(...out.texts);
    missing.push(...out.missing.map((r) => `${sp.id}@${r}`));
    medium.push(...out.medium.map((m) => ({ ...m, cellId: sp.id })));
  }

  // One node per metabolite becomes the pool, the others stay drawings of it.
  const rects = new Map(cells.map((c) => [c.id, c.rect]));
  const bases = [...new Set(medium.map((m) => m.base))];
  const exchanges = [];
  for (const base of bases) {
    const list = medium.filter((m) => m.base === base);
    const host = single ? list[0] : [...list].sort((a, b) => openAhead(b, rects, b.cellId) - openAhead(a, rects, a.cellId)
      || a.cellId.localeCompare(b.cellId))[0];
    const pool = `met:${sharedMetabolite(base)}`;
    if (host.nodeId !== pool) {
      const node = nodes.get(host.nodeId);
      nodes.delete(host.nodeId);
      const { reactionLocalFor, reactionLocal, ...rest } = node;
      nodes.set(pool, { ...rest, id: pool });
      for (const seg of segments) if (seg.nodeId === host.nodeId) seg.nodeId = pool;
      host.nodeId = pool;
    }
    const geo = communityExchangeGeometry(community, base, nodes.get(pool), host.normal);
    if (geo) { reactions.set(geo.reaction.id, geo.reaction); segments.push(...geo.segments); exchanges.push(geo.reaction.id); }
  }
  const mediumNodes = medium.map((m) => m.nodeId);
  return { nodes, reactions, segments, texts, cells, mediumNodes, exchanges, missing };
}

/** The box round everything drawn: cells with their name plates, nodes, stubs and titles. */
function extent({ cells, nodes, segments, texts }) {
  const box = { x0: Infinity, y0: Infinity, x1: -Infinity, y1: -Infinity };
  const add = (x, y) => { box.x0 = Math.min(box.x0, x); box.y0 = Math.min(box.y0, y); box.x1 = Math.max(box.x1, x); box.y1 = Math.max(box.y1, y); };
  for (const c of cells) { add(c.rect.x, c.rect.y - PLATE); add(c.rect.x + c.rect.w, c.rect.y + c.rect.h); }
  for (const n of nodes.values()) { add(n.x - 40, n.y - 30); add(n.x + 40, n.y + 30); }
  for (const s of segments) if (s.kind === 'boundary') add(s.to.x, s.to.y);
  for (const t of texts) { add(t.x, t.y - t.fontSize); add(t.x + t.text.length * t.fontSize * 0.55, t.y); }
  return box;
}

/** The map document of one scenario. */
export async function buildScenarioMap(scenario) {
  const columns = ARRANGEMENT[scenario.id];
  if (!columns) throw new Error(`No arrangement for ${scenario.id}`);
  const species = [];
  for (const s of scenario.species) species.push({ id: s.id, file: s.model, model: await loadModel(s.model), abundance: s.abundance ?? 1 / scenario.species.length });
  const listed = columns.flat().sort().join();
  if (listed !== species.map((s) => s.id).sort().join()) throw new Error(`${scenario.id}: arrangement ${listed} does not match its species`);
  // The medium the store builds the community with (Store#mediumSpec).
  const medium = Object.fromEntries(Object.entries(scenario.medium ?? {}).map(([b, v]) => [b, { lower: -Math.abs(Number(v) || 0), upper: 1000 }]));
  const community = new CommunityModel({
    name: scenario.name, objective: scenario.objective, medium,
    species: species.map((s) => ({ id: s.id, model: s.model, abundance: s.abundance, enabled: true })),
  });
  const templates = new Map(species.map((s) => [s.id, TEMPLATES[s.file](s.model)]));

  // Drawn once to find its extent, then again where that extent sits PAD in;
  // the gap widens until it holds a corridor at the grid of the canvas it makes.
  let gap = GAP, placed, box, width, height;
  for (;;) {
    placed = arrange(columns, templates, gap);
    box = extent(draw(species, community, templates, placed, 0, 0));
    width = Math.round(box.x1 - box.x0 + 2 * PAD);
    height = Math.round(box.y1 - box.y0 + 2 * PAD);
    const wanted = CORRIDOR * routeGrid({ width, height });
    if (columns.flat().length === 1 || gap >= wanted) break;
    gap = wanted;
  }
  const ox = Math.round(PAD - box.x0), oy = Math.round(PAD - box.y0);
  const drawn = draw(species, community, templates, placed, ox, oy);
  const { nodes, reactions, segments, texts, cells, mediumNodes, exchanges, missing } = drawn;

  // Everything the community has, and nothing it does not
  for (const id of reactions.keys()) if (!community.merged.reactions.has(id)) throw new Error(`${scenario.id}: ${id} is not in the model`);
  for (const id of community.merged.reactions.keys()) {
    if (id === 'COMMUNITY_GROWTH') continue;
    if (!reactions.has(id) && !missing.includes(id)) missing.push(id);
  }

  const map = new EditableMap({
    canvas: { width, height },
    environment: { rect: { x: FRAME, y: FRAME, w: width - 2 * FRAME, h: height - 2 * FRAME }, id: 's', label: 'Shared medium' },
    cells, medium: { nodes: mediumNodes, exchanges }, nodes, reactions, segments,
    texts: new Map(texts.map((t) => [t.id, t])),
    cofactorStyle: 'quiet',
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
    console.log(`${id}: ${doc.map.canvas.width}x${doc.map.canvas.height}, ${m.reactionCount} reactions, ${m.nodeCount} nodes, ${m.segmentCount} segments, ${m.textCount} texts${missing.length ? `; not drawn: ${missing.join(', ')}` : ''}`);
  }
}
if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) await main();
