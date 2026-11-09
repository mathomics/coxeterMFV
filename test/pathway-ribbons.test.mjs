import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import { EditableMap } from '../src/map/editable-map.js';
import { renderMap } from '../src/render/mapview.js';
import { FluxScale } from '../src/render/theme.js';
import { normalizePathwayRibbonMode } from '../src/render/reaction-colors.js';
import { Store } from '../src/ui/store.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

async function fixture() {
  const raw = JSON.parse(await readFile(join(root, 'models', 'ecoli_core_mc.json'), 'utf8'));
  const community = new CommunityModel({
    species: [{ id: 'eco', model: new Model(raw), abundance: 1 }],
    medium: { glc__D: { lower: -10 }, o2: { lower: -20 } },
  });
  const map = EditableMap.fromCommunity(community);
  const rid = map.segments.find((seg) => seg.kind !== 'currency')?.reactionId;
  assert.ok(rid);
  return { community, map, rid };
}

test('pathway ribbon mode validates supported values', () => {
  assert.equal(normalizePathwayRibbonMode('pathway'), 'pathway');
  assert.equal(normalizePathwayRibbonMode('subsystem'), 'subsystem');
  assert.equal(normalizePathwayRibbonMode('anything-else'), 'off');
});

test('ribbons render behind reaction segments and preserve flux colour', async () => {
  const { map, rid } = await fixture();
  const fluxes = new Map([[rid, 4]]);
  const scale = new FluxScale(fluxes.values());
  const ribbonColor = '#7c3aed';
  const svg = renderMap(map, {
    fluxes,
    scale,
    reactionColorMode: 'flux',
    pathwayRibbonMode: 'pathway',
    pathwayRibbonColors: new Map([[rid, ribbonColor]]),
    pathwayRibbonGroups: new Map([[rid, 'Glycolysis']]),
    pathwayRibbonWidth: 60,
    pathwayRibbonOpacity: 0.2,
    showCrossFeeding: false,
  });
  const ribbonAt = svg.indexOf('emc-layer-pathway-ribbons');
  const segmentsAt = svg.indexOf('emc-layer-segments');
  assert.ok(ribbonAt >= 0 && segmentsAt > ribbonAt);
  assert.ok(svg.includes(`stroke="${ribbonColor}"`));
  assert.ok(svg.includes(`stroke="${scale.color(4)}"`));
  assert.ok(svg.includes('stroke-width="60"'));
});

test('unannotated reactions do not receive a ribbon', async () => {
  const { map, rid } = await fixture();
  const svg = renderMap(map, {
    fluxes: new Map(),
    scale: new FluxScale([]),
    pathwayRibbonMode: 'pathway',
    pathwayRibbonColors: new Map([[rid, '#2563eb']]),
    pathwayRibbonGroups: new Map([[rid, 'Sin pathway']]),
    showCrossFeeding: false,
  });
  assert.ok(!svg.includes('data-ribbon-rxn='));
});

test('store clamps pathway ribbon width and opacity', () => {
  const store = new Store('.');
  store.setView({ pathwayRibbonMode: 'pathway', pathwayRibbonWidth: 500, pathwayRibbonOpacity: 1 });
  assert.equal(store.view.pathwayRibbonMode, 'pathway');
  assert.equal(store.view.pathwayRibbonWidth, 120);
  assert.equal(store.view.pathwayRibbonOpacity, 0.45);
  store.setView({ pathwayRibbonWidth: 1, pathwayRibbonOpacity: 0 });
  assert.equal(store.view.pathwayRibbonWidth, 20);
  assert.equal(store.view.pathwayRibbonOpacity, 0.04);
});

test('scenario state persists pathway ribbon settings', () => {
  const store = new Store('.');
  store.view.pathwayRibbonMode = 'subsystem';
  store.view.pathwayRibbonWidth = 72;
  store.view.pathwayRibbonOpacity = 0.21;
  const doc = JSON.parse(store.stateJSON());
  assert.equal(doc.pathwayRibbonMode, 'subsystem');
  assert.equal(doc.pathwayRibbonWidth, 72);
  assert.equal(doc.pathwayRibbonOpacity, 0.21);
});
