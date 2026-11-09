import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import { EditableMap } from '../src/map/editable-map.js';
import { buildReactionColoring, reactionCategory } from '../src/render/reaction-colors.js';
import { renderMap } from '../src/render/mapview.js';
import { FluxScale } from '../src/render/theme.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

test('reaction pathway metadata is read from explicit and common COBRA metadata locations', () => {
  const model = new Model({
    id: 'groups',
    compartments: { c: 'cytosol' },
    metabolites: [{ id: 'a_c', compartment: 'c' }, { id: 'b_c', compartment: 'c' }],
    reactions: [
      { id: 'R1', metabolites: { a_c: -1, b_c: 1 }, subsystem: 'Glycolysis', pathway: 'Central carbon' },
      { id: 'R2', metabolites: { b_c: -1, a_c: 1 }, notes: { pathway: 'Redox' } },
      { id: 'R3', metabolites: { a_c: -1, b_c: 1 }, annotation: { pathway: ['Amino acid metabolism'] } },
    ],
  });

  assert.equal(reactionCategory(model.reactions.get('R1'), 'subsystem'), 'Glycolysis');
  assert.equal(reactionCategory(model.reactions.get('R1'), 'pathway'), 'Central carbon');
  assert.equal(model.reactions.get('R2').pathway, 'Redox');
  assert.equal(model.reactions.get('R3').pathway, 'Amino acid metabolism');

  const roundTrip = new Model(model.toJSON());
  assert.equal(roundTrip.reactions.get('R1').pathway, 'Central carbon');
  assert.equal(roundTrip.reactions.get('R2').pathway, 'Redox');
  assert.deepEqual(roundTrip.reactions.get('R3').annotation.pathway, ['Amino acid metabolism']);
});

test('categorical colours are deterministic and missing metadata has an explicit group', () => {
  const model = new Model({
    id: 'groups',
    compartments: { c: 'cytosol' },
    metabolites: [{ id: 'a_c', compartment: 'c' }, { id: 'b_c', compartment: 'c' }],
    reactions: [
      { id: 'R1', metabolites: { a_c: -1, b_c: 1 }, subsystem: 'TCA cycle' },
      { id: 'R2', metabolites: { b_c: -1, a_c: 1 }, subsystem: 'TCA cycle' },
      { id: 'R3', metabolites: { a_c: -1, b_c: 1 } },
    ],
  });

  const first = buildReactionColoring(model, 'subsystem');
  const second = buildReactionColoring(model, 'subsystem');
  assert.equal(first.colors.get('R1'), first.colors.get('R2'));
  assert.equal(first.colors.get('R1'), second.colors.get('R1'));
  assert.equal(first.groups.get('R3'), 'Sin subsystem');
  assert.equal(first.legend.find((x) => x.label === 'TCA cycle')?.count, 2);
});

test('map renderer can colour reactions by category while reaction width stays fixed', async () => {
  const raw = JSON.parse(await readFile(join(root, 'models', 'ecoli_core_mc.json'), 'utf8'));
  const community = new CommunityModel({
    species: [{ id: 'eco', model: new Model(raw), abundance: 1 }],
    medium: { glc__D: { lower: -10 }, o2: { lower: -20 } },
  });
  const map = EditableMap.fromCommunity(community);
  const rid = map.segments[0]?.reactionId;
  assert.ok(rid, 'fixture should draw at least one reaction segment');

  const fluxes = new Map([[rid, 3]]);
  const scale = new FluxScale(fluxes.values());
  const categoryColor = '#123456';
  const svg = renderMap(map, {
    fluxes,
    scale,
    reactionColorMode: 'subsystem',
    reactionColors: new Map([[rid, categoryColor]]),
    showCrossFeeding: false,
  });

  assert.ok(svg.includes(`stroke="${categoryColor}"`));
  assert.ok(svg.includes('stroke-width="7.5"'));
});

test('flux mode ignores categorical colours', async () => {
  const raw = JSON.parse(await readFile(join(root, 'models', 'ecoli_core_mc.json'), 'utf8'));
  const community = new CommunityModel({
    species: [{ id: 'eco', model: new Model(raw), abundance: 1 }],
    medium: { glc__D: { lower: -10 }, o2: { lower: -20 } },
  });
  const map = EditableMap.fromCommunity(community);
  const rid = map.segments[0].reactionId;
  const fluxes = new Map([[rid, 2]]);
  const scale = new FluxScale(fluxes.values());
  const svg = renderMap(map, {
    fluxes,
    scale,
    reactionColorMode: 'flux',
    reactionColors: new Map([[rid, '#123456']]),
    showCrossFeeding: false,
  });
  assert.ok(svg.includes(`stroke="${scale.color(2)}"`));
});
