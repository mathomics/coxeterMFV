import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Model } from '../src/core/model.js';
import { FluxProblem } from '../src/solver/fba.js';
import { EditableMap } from '../src/map/editable-map.js';
import { Store } from '../src/ui/store.js';
import { renderMap } from '../src/render/mapview.js';
import { FluxScale } from '../src/render/theme.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

async function ecoli() {
  return new Model(JSON.parse(
    await readFile(join(root, 'models', 'ecoli_core_mc.json'), 'utf8')
  ));
}

test('progressive FVA matches synchronous FVA for the same reactions', async () => {
  const model = await ecoli();
  const problem = new FluxProblem(model);
  const ids = ['PGI', 'TALA', 'EX_o2_e'];

  const expected = problem.variability(ids, { fraction: 1.0 });
  const progress = [];
  const actual = await problem.variabilityAsync(ids, {
    fraction: 1.0,
    chunkSize: 1,
    onProgress: (_partial, done, total) => progress.push([done, total]),
  });

  assert.equal(actual.size, ids.length);
  assert.deepEqual(progress.at(-1), [ids.length, ids.length]);

  for (const rid of ids) {
    const a = actual.get(rid);
    const b = expected.get(rid);
    assert.ok(Math.abs(a.min - b.min) < 1e-7, `${rid} min differs`);
    assert.ok(Math.abs(a.max - b.max) < 1e-7, `${rid} max differs`);
  }
});

test('progressive FVA can be cancelled between reactions', async () => {
  const model = await ecoli();
  const problem = new FluxProblem(model);
  const ids = ['PGI', 'TALA', 'PFK', 'PYK'];
  let allowed = true;

  const result = await problem.variabilityAsync(ids, {
    chunkSize: 1,
    shouldContinue: () => allowed,
    onProgress: (_partial, done) => {
      if (done === 1) allowed = false;
    },
  });

  assert.equal(result.size, 1);
});

test('Store FVA mode computes and exposes global ranges', async () => {
  const store = new Store('.');
  store.problem = {
    rxnIds: ['R1', 'R2'],
    async variabilityAsync(ids, opts) {
      const ranges = new Map([
        ['R1', { min: -1, max: 3 }],
        ['R2', { min: 0, max: 5 }],
      ]);
      opts.onProgress?.(ranges, ids.length, ids.length);
      return ranges;
    },
  };
  store.solution = {
    status: 'optimal',
    objectiveValue: 1,
    fluxes: new Map([['R1', 1], ['R2', 2]]),
  };

  store.setFVA(true);
  await new Promise((resolve) => setTimeout(resolve, 15));

  assert.equal(store.fvaEnabled, true);
  assert.deepEqual(store.fvaRanges.get('R1'), { min: -1, max: 3 });
  assert.equal(store.fvaProgress.running, false);
  assert.equal(store.fvaProgress.done, 2);

  store.select('R1');
  assert.deepEqual(store.variability, { min: -1, max: 3 });

  store.setFVA(false);
  assert.equal(store.fvaEnabled, false);
  assert.equal(store.fvaRanges.size, 0);
  assert.equal(store.variability, null);
});

test('scenario JSON persists FVA checkbox state', () => {
  const store = new Store('.');
  store.fvaEnabled = true;
  const doc = JSON.parse(store.stateJSON());
  assert.equal(doc.fvaEnabled, true);
});

test('FVA mode renders min and max beside every drawn reaction', () => {
  const map = new EditableMap({
    canvas: { width: 400, height: 240 },
    environment: {
      id: 'u',
      label: 'Medium',
      rect: { x: 0, y: 0, w: 400, h: 240 },
    },
    cells: [],
    medium: { nodes: [], exchanges: [] },
    nodes: new Map(),
    reactions: new Map([
      ['R1', {
        id: 'R1',
        localId: 'R1',
        x: 200,
        y: 120,
        onMembrane: false,
        compartments: [],
      }],
    ]),
    segments: [],
  });

  const fluxes = new Map([['R1', 2]]);
  const svg = renderMap(map, {
    fluxes,
    scale: new FluxScale(fluxes.values()),
    labels: 'none',
    showCrossFeeding: false,
    fvaEnabled: true,
    fvaRanges: new Map([['R1', { min: -1.25, max: 4.5 }]]),
  });

  assert.ok(svg.includes('R1  [-1.25, 4.5]'));
  assert.ok(svg.includes('emc-label-fva'));
});

test('FVA mode marks reactions whose range is still pending', () => {
  const map = new EditableMap({
    canvas: { width: 400, height: 240 },
    environment: {
      id: 'u',
      label: 'Medium',
      rect: { x: 0, y: 0, w: 400, h: 240 },
    },
    cells: [],
    medium: { nodes: [], exchanges: [] },
    nodes: new Map(),
    reactions: new Map([
      ['R1', { id: 'R1', localId: 'R1', x: 200, y: 120, onMembrane: false }],
    ]),
    segments: [],
  });
  const fluxes = new Map([['R1', 2]]);
  const svg = renderMap(map, {
    fluxes,
    scale: new FluxScale(fluxes.values()),
    fvaEnabled: true,
    fvaRanges: new Map(),
    showCrossFeeding: false,
  });

  assert.ok(svg.includes('R1  [calculando…]'));
});
