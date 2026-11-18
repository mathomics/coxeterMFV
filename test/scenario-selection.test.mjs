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
  await store.useScenario('single-ecoli');
  await store.importReproducibilityBundle(await store.reproducibilityBundle());
  const html = renderWorkspace(store, 'scenario');
  assert.match(html, /<optgroup label="Current model">/);
  assert.match(html, /<option value="sc:reproducibility-bundle" selected disabled>/);
});

test('the scenario list shows the case studies, then the drafts, each from one cell to the largest community', async () => {
  const raw = JSON.parse(await readFile(new URL('../models/ecoli_core_mc.json', import.meta.url), 'utf8'));
  const catalogue = JSON.parse(await readFile(new URL('../models/scenarios.json', import.meta.url), 'utf8'));
  const store = new Store('.');
  store.models.set('ecoli_core_mc', new Model(raw));
  store.scenarios = catalogue.scenarios;
  store.publishedCommunities = catalogue.publishedCommunities;
  await store.useScenario('single-ecoli');
  const list = renderWorkspace(store, 'scenario').match(/<select data-action="scenario"[^>]*>([\s\S]*?)<\/select>/)[1];
  const groups = [...list.matchAll(/<optgroup label="([^"]*)">/g)].map((match) => match[1]);
  assert.deepEqual(groups, ['Explore Coxeter’s capabilities', 'Example drafts']);
  const values = [...list.matchAll(/<option value="([^"]*)"/g)].map((match) => match[1]);
  assert.deepEqual(values, [
    'sc:yeast9-9-1-1',
    'pub:zuniga-2020-phea-rescue',
    'pub:steadycom-ecoli-auxotrophs',
    'pub:steadycom-gut-nine',
    'sc:single-ecoli',
    'sc:methanogen-digester',
    'sc:aerobic-coculture',
    'pub:stolyar-2007-syntrophy',
    'sc:phototroph-heterotroph',
    'sc:anaerobic-consortium',
    'sc:all-species',
  ]);
  const catalogued = [
    ...catalogue.scenarios.map((scenario) => `sc:${scenario.id}`),
    ...catalogue.publishedCommunities.map((community) => `pub:${community.id}`),
  ];
  assert.deepEqual([...values].sort(), catalogued.sort());
  assert.match(list, /<option value="sc:single-ecoli" selected>Example: a single E\. coli cell<\/option>/);
});
