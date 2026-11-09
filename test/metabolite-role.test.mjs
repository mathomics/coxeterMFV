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

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

async function build() {
  const raw = JSON.parse(await readFile(join(root, 'models', 'ecoli_core_mc.json'), 'utf8'));
  const community = new CommunityModel({
    species: [{ id: 'eco', model: new Model(raw), abundance: 1 }],
    medium: { glc__D: { lower: -10 }, o2: { lower: -20 } },
  });
  return { community, map: EditableMap.fromCommunity(community) };
}

function metaboliteNode(map) {
  return [...map.nodes.values()].find((node) => node.kind === 'metabolite');
}

test('existing metabolite nodes default to primary', async () => {
  const { map } = await build();
  const node = metaboliteNode(map);
  assert.ok(node);
  assert.equal(node.primary, true);
  assert.equal(node.visualRole, 'primary');
});

test('a metabolite can be changed from primary to secondary and back', async () => {
  const { map } = await build();
  const node = metaboliteNode(map);

  map.setNodePrimary(node.id, false);
  assert.equal(node.primary, false);
  assert.equal(node.visualRole, 'secondary');
  assert.equal(node.r, 7);

  map.setNodePrimary(node.id, true);
  assert.equal(node.primary, true);
  assert.equal(node.visualRole, 'primary');
  assert.equal(node.r, 13);
});

test('radius requests snap to the two supported visual roles', async () => {
  const { map } = await build();
  const node = metaboliteNode(map);

  map.setNodeRadius(node.id, 23);
  assert.equal(node.primary, true);
  assert.equal(node.r, 13);

  map.setNodeRadius(node.id, 5);
  assert.equal(node.primary, false);
  assert.equal(node.r, 7);
});

test('duplicated metabolite instances can have independent visual roles', async () => {
  const { map } = await build();
  const original = metaboliteNode(map);
  const copy = map.duplicateMetaboliteNode(original.id);
  assert.ok(copy);

  map.setNodePrimary(copy.id, false);

  assert.equal(original.primary, true);
  assert.equal(original.visualRole, 'primary');
  assert.equal(copy.primary, false);
  assert.equal(copy.visualRole, 'secondary');
  assert.equal(copy.mergedId, original.mergedId);
});

test('primary/secondary role survives map JSON round trip', async () => {
  const { map } = await build();
  const node = metaboliteNode(map);
  map.setNodePrimary(node.id, false);

  const restored = EditableMap.fromJSON(JSON.parse(JSON.stringify(map.toJSON())));
  const back = restored.nodes.get(node.id);
  assert.equal(back.primary, false);
  assert.equal(back.visualRole, 'secondary');
  assert.equal(back.r, 7);
});

test('legacy map nodes without role metadata load as primary', async () => {
  const { map } = await build();
  const doc = JSON.parse(JSON.stringify(map.toJSON()));
  const node = doc.nodes.find((item) => item.kind === 'metabolite');
  delete node.primary;
  delete node.visualRole;

  const restored = EditableMap.fromJSON(doc);
  const back = restored.nodes.get(node.id);
  assert.equal(back.primary, true);
  assert.equal(back.visualRole, 'primary');
});

test('renderer distinguishes primary and secondary nodes and labels', async () => {
  const { map } = await build();
  const original = metaboliteNode(map);
  const copy = map.duplicateMetaboliteNode(original.id, {
    x: original.x + 40,
    y: original.y + 30,
  });
  map.setNodePrimary(copy.id, false);

  const svg = renderMap(map, {
    fluxes: new Map(),
    scale: new FluxScale([]),
    showCrossFeeding: false,
  });

  assert.ok(svg.includes('emc-node-primary'));
  assert.ok(svg.includes('emc-node-secondary'));
  assert.ok(svg.includes('emc-label-primary'));
  assert.ok(svg.includes('emc-label-secondary'));
});

test('changing visual role does not change biological model membership', async () => {
  const { community, map } = await build();
  const node = metaboliteNode(map);
  const biologicalCount = community.merged.metabolites.size;
  const reactionCount = community.merged.reactions.size;

  map.setNodePrimary(node.id, false);

  assert.equal(community.merged.metabolites.size, biologicalCount);
  assert.equal(community.merged.reactions.size, reactionCount);
  assert.ok(community.merged.metabolites.has(node.mergedId));
});
