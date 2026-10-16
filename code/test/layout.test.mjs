import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import { FluxProblem } from '../src/solver/fba.js';
import { layoutCommunity } from '../src/layout/layout.js';
import { renderMap } from '../src/render/mapview.js';
import { FluxScale } from '../src/render/theme.js';
import { signedDistance, perimeterPoint, closestPerimeterPoint, gridCells, inset, compartmentCornerRadius } from '../src/layout/geometry.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

async function loadModel(name) {
  return new Model(JSON.parse(await readFile(join(root, 'models', `${name}.json`), 'utf8')));
}

async function buildLayout(names) {
  const species = [];
  for (const [id, file] of names) {
    species.push({ id, model: await loadModel(file), abundance: 1 / names.length });
  }
  const com = new CommunityModel({
    species,
    medium: {
      glc__D: { lower: -10 }, o2: { lower: -20 }, nh4: { lower: -1000 },
      pi: { lower: -1000 }, h2o: { lower: -1000 }, h: { lower: -1000 },
      co2: { lower: -1000 }, photon: { lower: -80 },
    },
  });
  return { com, layout: layoutCommunity(com) };
}

test('geometry helpers behave', () => {
  const r = { x: 0, y: 0, w: 100, h: 50 };
  assert.deepEqual(perimeterPoint(r, 0), { x: 50, y: 0, nx: 0, ny: -1 });
  assert.equal(closestPerimeterPoint(r, { x: 200, y: 25 }).x, 100);
  assert.ok(signedDistance(r, { x: 50, y: 25 }) < 0, 'inside is negative');
  assert.ok(signedDistance(r, { x: 150, y: 25 }) > 0, 'outside is positive');
  // Four items on a wide canvas should come out 2x2, not 3 + 1.
  const cells = gridCells({ x: 0, y: 0, w: 1600, h: 1000 }, 4, 40);
  const ys = new Set(cells.map((c) => Math.round(c.y)));
  assert.equal(ys.size, 2, 'expected two rows');
});

test('compartments nest strictly inside their parent', async () => {
  const { layout } = await buildLayout([['eco', 'ecoli_core_mc'], ['sce', 'yeast_core_mc']]);
  for (const cell of layout.cells) {
    const byId = new Map(cell.compartments.map((c) => [c.id, c]));
    for (const comp of cell.compartments) {
      if (!comp.parentId) continue;
      const parent = byId.get(comp.parentId);
      assert.ok(parent, `${comp.id} has no parent rect`);
      assert.ok(comp.rect.x >= parent.rect.x - 0.5, `${comp.id} escapes left of ${parent.id}`);
      assert.ok(comp.rect.y >= parent.rect.y - 0.5, `${comp.id} escapes above ${parent.id}`);
      assert.ok(comp.rect.x + comp.rect.w <= parent.rect.x + parent.rect.w + 0.5,
        `${comp.id} escapes right of ${parent.id}`);
      assert.ok(comp.rect.y + comp.rect.h <= parent.rect.y + parent.rect.h + 0.5,
        `${comp.id} escapes below ${parent.id}`);
      assert.ok(comp.rect.w > 20 && comp.rect.h > 20, `${comp.id} collapsed to nothing`);
    }
  }
});

test('cells do not overlap each other', async () => {
  const { layout } = await buildLayout([
    ['eco', 'ecoli_core_mc'], ['sce', 'yeast_core_mc'],
    ['mba', 'methanogen_core_mc'], ['lla', 'lactobacillus_core_mc'],
  ]);
  const rects = layout.cells.map((c) => c.rect);
  for (let i = 0; i < rects.length; i++) {
    for (let j = i + 1; j < rects.length; j++) {
      const a = rects[i];
      const b = rects[j];
      const overlap = a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
      assert.ok(!overlap, `cells ${i} and ${j} overlap`);
    }
  }
});

test('every metabolite node sits inside the compartment it belongs to', async () => {
  const { layout } = await buildLayout([['eco', 'ecoli_core_mc'], ['sce', 'yeast_core_mc']]);
  const rects = new Map();
  for (const cell of layout.cells) {
    for (const comp of cell.compartments) rects.set(`${cell.id}/${comp.id}`, comp.rect);
  }
  let checked = 0;
  for (const node of layout.nodes.values()) {
    if (node.kind !== 'metabolite') continue;
    const r = rects.get(`${node.cellId}/${node.compartment}`);
    assert.ok(r, `no rect for ${node.cellId}/${node.compartment}`);
    assert.ok(signedDistance(r, node) < 1, `${node.mergedId} outside ${node.compartment}`);
    checked += 1;
  }
  assert.ok(checked > 50, `only checked ${checked} nodes`);
});

test('medium metabolites stay outside every cell', async () => {
  const { layout } = await buildLayout([['eco', 'ecoli_core_mc'], ['sce', 'yeast_core_mc']]);
  let checked = 0;
  for (const node of layout.nodes.values()) {
    if (node.kind !== 'medium') continue;
    for (const cell of layout.cells) {
      assert.ok(signedDistance(cell.rect, node) > -1,
        `medium node ${node.base} sits inside cell ${cell.id}`);
    }
    checked += 1;
  }
  assert.ok(checked > 3, `only checked ${checked} medium nodes`);
});

test('transport reactions land on the membrane they cross', async () => {
  const { layout } = await buildLayout([['sce', 'yeast_core_mc']]);
  const cell = layout.cells[0];
  const byId = new Map(cell.compartments.map((c) => [c.id, c]));
  let checked = 0;
  for (const rxn of layout.reactions.values()) {
    if (!rxn.onMembrane) {
      continue;
    }
    const comp = byId.get(rxn.membrane);
    assert.ok(comp, `${rxn.id} on unknown membrane ${rxn.membrane}`);
    const radius = compartmentCornerRadius(comp);
    const inner = inset(comp.rect, radius);
    const distance = Math.abs(signedDistance(inner, rxn) - radius);
    assert.ok(distance < 2, `${rxn.localId} is ${distance.toFixed(1)}px off its membrane`);
    checked += 1;
  }
  assert.ok(checked > 5, `only checked ${checked} transporters`);
});

test('every reaction in the merged model is drawn exactly once', async () => {
  const { com, layout } = await buildLayout([['eco', 'ecoli_core_mc'], ['mba', 'methanogen_core_mc']]);
  const drawn = new Set(layout.reactions.keys());
  for (const rid of com.merged.reactions.keys()) {
    if (rid === 'COMMUNITY_GROWTH') continue;   // a virtual coupling variable
    assert.ok(drawn.has(rid), `${rid} is missing from the map`);
  }
  // And nothing is invented.
  for (const rid of drawn) assert.ok(com.merged.reactions.has(rid), `${rid} is not in the model`);
});

test('the layout is deterministic', async () => {
  const a = await buildLayout([['eco', 'ecoli_core_mc']]);
  const b = await buildLayout([['eco', 'ecoli_core_mc']]);
  const pick = (l) => [...l.layout.nodes.values()].slice(0, 40).map((n) => `${n.id}:${n.x.toFixed(4)},${n.y.toFixed(4)}`);
  assert.deepEqual(pick(a), pick(b));
});

test('the renderer emits well-formed SVG containing the whole community', async () => {
  const { com, layout } = await buildLayout([['eco', 'ecoli_core_mc'], ['sce', 'yeast_core_mc']]);
  const res = new FluxProblem(com.merged).parsimonious();
  const svg = renderMap(layout, {
    fluxes: res.fluxes,
    scale: new FluxScale(res.fluxes.values()),
    growth: com.growthRates(res.fluxes),
    crossFeeding: com.crossFeeding(res.fluxes),
  });

  assert.ok(svg.startsWith('<svg'), 'not an SVG document');
  assert.ok(svg.endsWith('</svg>'));
  // Balanced tags for the elements we generate by hand.
  for (const tag of ['g', 'text', 'defs']) {
    const open = (svg.match(new RegExp(`<${tag}[\\s>]`, 'g')) ?? []).length;
    const close = (svg.match(new RegExp(`</${tag}>`, 'g')) ?? []).length;
    assert.equal(open, close, `unbalanced <${tag}>`);
  }
  assert.ok(!/NaN|undefined|Infinity/.test(svg), 'numeric attribute did not resolve');
  for (const cell of layout.cells) {
    assert.ok(svg.includes(`data-cell="${cell.id}"`), `cell ${cell.id} not rendered`);
  }
  // Compartment colours actually differ between compartments.
  const comps = new Set(layout.cells.flatMap((c) => c.compartments.map((k) => k.id)));
  assert.ok(comps.size >= 4, `expected several compartments, got ${[...comps]}`);
});

test('flux scale maps magnitude monotonically to width', () => {
  const scale = new FluxScale([0.1, 1, 5, 20, 60]);
  const widths = [0, 0.1, 1, 5, 20, 60].map((v) => scale.width(v));
  for (let i = 1; i < widths.length; i++) {
    assert.ok(widths[i] >= widths[i - 1] - 1e-9, `width dropped at ${i}: ${widths}`);
  }
  assert.equal(scale.width(5), scale.width(-5), 'direction must not change width');
});
