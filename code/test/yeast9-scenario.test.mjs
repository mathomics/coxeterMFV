import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  YEAST9_SCENARIO_ID,
  YEAST9_PREVIEW_COMPARTMENTS,
  YEAST9_PREVIEW_GROUPS,
} from '../src/core/yeast9.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

test('Yeast9 9.1.1 is registered in the Scenario list', async () => {
  const raw = JSON.parse(await readFile(join(root, 'models', 'scenarios.json'), 'utf8'));
  const scenario = raw.scenarios.find((item) => item.id === YEAST9_SCENARIO_ID);
  assert.ok(scenario);
  assert.equal(scenario.loader, 'yeast9');
  assert.equal(scenario.objective, 'sum-biomass');
  assert.equal(scenario.species.length, 1);
  assert.equal(scenario.species[0].id, 'sce');
});

test('the curated Yeast9 preview covers all 14 model compartments', () => {
  const covered = new Set(YEAST9_PREVIEW_GROUPS.map((group) => group.compartment));
  assert.deepEqual([...covered].sort(), [...YEAST9_PREVIEW_COMPARTMENTS].sort());
  assert.equal(YEAST9_PREVIEW_COMPARTMENTS.length, 14);

  const ids = YEAST9_PREVIEW_GROUPS.flatMap((group) => group.reactions);
  assert.ok(ids.length >= 30);
  assert.equal(new Set(ids).size, ids.length, 'representative reaction ids should not be duplicated');
});
