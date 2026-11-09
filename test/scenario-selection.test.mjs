/** Keep the active imported context represented in the Scenario chooser. */
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { Model } from '../src/core/model.js';
import { Store } from '../src/ui/store.js';
import { renderWorkspace } from '../src/ui/panels.js';

test('the Scenario chooser selects an imported bundle that has no catalogue entry', async () => {
  const raw = JSON.parse(await readFile(new URL('../models/ecoli_core_mc.json', import.meta.url), 'utf8'));
  const store = new Store('.');
  store.models.set('ecoli_core_mc', new Model(raw));
  store.scenarios = JSON.parse(await readFile(new URL('../models/scenarios.json', import.meta.url), 'utf8')).scenarios;
  await store.useScenario('solo-ecoli');
  await store.importReproducibilityBundle(await store.reproducibilityBundle());
  const html = renderWorkspace(store, 'scenario');
  assert.match(html, /<optgroup label="Current model">/);
  assert.match(html, /<option value="sc:reproducibility-bundle" selected disabled>/);
});
