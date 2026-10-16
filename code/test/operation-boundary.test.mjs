import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import { EditableMap } from '../src/map/editable-map.js';
import {
  OPERATION_DOMAIN,
  EDIT_OPERATION_DOMAIN,
  operationDomain,
  isMapOperation,
  isModelOperation,
  assertOperationDomain,
} from '../src/ui/operation-boundary.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

async function fixture() {
  const raw = JSON.parse(await readFile(join(root, 'models', 'ecoli_core_mc.json'), 'utf8'));
  const model = new Model(raw);
  const community = new CommunityModel({
    species: [{ id: 'eco', model, abundance: 1 }],
    medium: {},
  });
  const map = EditableMap.emptyFromCommunity(community);
  return { model, community, map };
}

function biologicalSnapshot(model, community) {
  return JSON.stringify({
    model: model.toJSON(),
    reactions: [...community.merged.reactions.entries()],
    metabolites: [...community.merged.metabolites.entries()],
  });
}

test('editing operations have an explicit MAP or MODEL domain', () => {
  assert.equal(operationDomain('addMapReaction'), OPERATION_DOMAIN.MAP);
  assert.equal(operationDomain('moveNode'), OPERATION_DOMAIN.MAP);
  assert.equal(operationDomain('duplicateMapMetabolite'), OPERATION_DOMAIN.MAP);
  assert.equal(operationDomain('createModelMetabolite'), OPERATION_DOMAIN.MODEL);
  assert.equal(operationDomain('createModelReaction'), OPERATION_DOMAIN.MODEL);
  assert.equal(operationDomain('unknownOperation'), null);

  for (const domain of Object.values(EDIT_OPERATION_DOMAIN)) {
    assert.ok([OPERATION_DOMAIN.MAP, OPERATION_DOMAIN.MODEL].includes(domain));
  }
});

test('domain guards reject crossing the BUILD/MODEL boundary', () => {
  assert.equal(isMapOperation('removeMapNode'), true);
  assert.equal(isModelOperation('createModelReaction'), true);
  assert.equal(assertOperationDomain('addMapReaction', OPERATION_DOMAIN.MAP), true);
  assert.throws(
    () => assertOperationDomain('createModelReaction', OPERATION_DOMAIN.MAP),
    /belongs to MODEL, not MAP/,
  );
  assert.throws(
    () => assertOperationDomain('futureEdit', OPERATION_DOMAIN.MAP),
    /has no MAP\/MODEL domain/,
  );
});

test('representative MAP edits cannot mutate the biological model', async () => {
  const { model, community, map } = await fixture();
  const before = biologicalSnapshot(model, community);
  const rid = [...community.merged.reactions.keys()].find((id) => !map.hasReaction(id));
  assert.ok(rid);

  const added = map.addReaction(community, rid);
  assert.ok(added);

  const movable = [...map.nodes.values()].find((node) => map.canMoveNode(node.id));
  if (movable) map.moveNode(movable.id, movable.x + 20, movable.y + 10);

  const duplicateSource = [...map.nodes.values()].find((node) => node.mergedId);
  if (duplicateSource) map.duplicateMetaboliteNode(duplicateSource.id, { dx: 30, dy: 20 });

  map.removeReaction(rid);

  assert.equal(biologicalSnapshot(model, community), before);
});

test('MAP serialization contains drawing state, not a rewritten biological model', async () => {
  const { model, community, map } = await fixture();
  const beforeModel = JSON.stringify(model.toJSON());
  const rid = [...community.merged.reactions.keys()][0];

  map.addReaction(community, rid);
  const mapDocument = map.toJSON();

  assert.ok(mapDocument);
  assert.equal(JSON.stringify(model.toJSON()), beforeModel);
});
