import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import { layoutCommunity } from '../src/layout/layout.js';
import {
  DEFAULT_LAYOUT_PRESET,
  LAYOUT_PRESETS,
  layoutEngineOptions,
  layoutPreset,
  normalizeLayoutSettings,
} from '../src/layout/layout-settings.js';
import { renderLayoutSettings } from '../src/ui/panels.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

async function model(name) {
  return new Model(JSON.parse(await readFile(join(root, 'models', `${name}.json`), 'utf8')));
}

async function fourCellCommunity() {
  const files = [
    ['a', 'ecoli_core_mc'],
    ['b', 'ecoli_core_mc'],
    ['c', 'ecoli_core_mc'],
    ['d', 'ecoli_core_mc'],
  ];
  const species = [];
  for (const [id, file] of files) {
    species.push({ id, model: await model(file), abundance: 0.25, enabled: true });
  }
  return new CommunityModel({ species, medium: {} });
}

test('layout presets expose compact, normal and spacious with spacious as the default', () => {
  assert.equal(DEFAULT_LAYOUT_PRESET, 'spacious');
  assert.deepEqual(Object.keys(LAYOUT_PRESETS), ['compact', 'normal', 'spacious']);
  assert.ok(LAYOUT_PRESETS.compact.cellSpacing < LAYOUT_PRESETS.spacious.cellSpacing);
  assert.ok(LAYOUT_PRESETS.compact.minimumCompartmentSize < LAYOUT_PRESETS.spacious.minimumCompartmentSize);
  assert.ok(LAYOUT_PRESETS.compact.nodeSpacing < LAYOUT_PRESETS.spacious.nodeSpacing);
  assert.equal(LAYOUT_PRESETS.spacious.cellAreaMultiplier, 6);
});

test('custom layout values are clamped and marked custom', () => {
  const settings = normalizeLayoutSettings({
    preset: 'normal',
    cellSpacing: 9999,
    minimumCompartmentSize: 5,
    nodeSpacing: 1.31,
    cellAreaMultiplier: 99,
  });
  assert.equal(settings.preset, 'custom');
  assert.equal(settings.cellSpacing, 420);
  assert.equal(settings.minimumCompartmentSize, 28);
  assert.equal(settings.nodeSpacing, 1.31);
  assert.equal(settings.cellAreaMultiplier, 8);
});

test('sixfold cell-area multiplier produces roughly six times the automatic cell area', async () => {
  const community = await fourCellCommunity();
  const common = {
    cellGap: 180,
    minShell: 48,
    nodeSpacing: 1.08,
  };
  const base = layoutCommunity(community, { ...common, cellAreaMultiplier: 1 });
  const expanded = layoutCommunity(community, { ...common, cellAreaMultiplier: 6 });

  const totalCellArea = (layout) =>
    layout.cells.reduce((sum, cell) => sum + cell.rect.w * cell.rect.h, 0);
  const ratio = totalCellArea(expanded) / totalCellArea(base);

  // Shared-medium margins and inter-cell gaps mean the realised rectangle area
  // is not mathematically exact, but should remain close to the requested 6×.
  assert.ok(ratio > 5.0 && ratio < 7.0, `expected ~6× cell area, got ${ratio.toFixed(2)}×`);
});

test('spacious automatic layout gives cells and nodes more map area than compact', async () => {
  const community = await fourCellCommunity();
  const compact = layoutCommunity(community, layoutEngineOptions(layoutPreset('compact')));
  const spacious = layoutCommunity(community, layoutEngineOptions(layoutPreset('spacious')));

  const area = (r) => r.w * r.h;
  const compactCellArea = compact.cells.reduce((sum, cell) => sum + area(cell.rect), 0);
  const spaciousCellArea = spacious.cells.reduce((sum, cell) => sum + area(cell.rect), 0);

  assert.ok(spacious.width * spacious.height > compact.width * compact.height);
  assert.ok(spaciousCellArea > compactCellArea);

  const nearestDistance = (layout) => {
    const values = [...layout.nodes.values()]
      .filter((node) => node.kind === 'metabolite' && node.cellId === 'a');
    const nearest = values.map((node, index) => {
      let best = Infinity;
      for (let i = 0; i < values.length; i += 1) {
        if (i === index) continue;
        best = Math.min(best, Math.hypot(node.x - values[i].x, node.y - values[i].y));
      }
      return best;
    }).filter(Number.isFinite).sort((a, b) => a - b);
    return nearest[Math.floor(nearest.length / 2)] ?? 0;
  };

  assert.ok(nearestDistance(spacious) > nearestDistance(compact));
});

test('layout settings panel renders the same controls shown in the canvas toolbar', () => {
  const store = {
    layoutSettings: layoutPreset('spacious'),
    map: { width: 2400, height: 1800 },
  };
  const html = renderLayoutSettings(store);
  assert.match(html, /Compact/);
  assert.match(html, /Normal/);
  assert.match(html, /Spacious/);
  assert.match(html, /Cell area/);
  assert.match(html, /6\.0×/);
  assert.match(html, /Cell spacing/);
  assert.match(html, /Minimum compartment size/);
  assert.match(html, /Node spacing/);
  assert.match(html, /Fit to screen/);
  assert.match(html, /2400/);
  assert.match(html, /1800/);
});
