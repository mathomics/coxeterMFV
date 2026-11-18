import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

import {
  BUILD_LICENSE_URL,
  CODE_LICENSE_URL,
  REPOSITORY_URL,
  SITE_URL,
  WELCOME_EXAMPLES,
  WELCOME_LINKS,
  WELCOME_STORAGE_KEY,
  rememberWelcomeAtStart,
  scenarioFromAddress,
  welcomeMarkup,
  welcomeOpensAtStart,
} from '../src/ui/welcome.js';

/** A Storage stand-in that keeps its items in a Map. */
function memoryStorage() {
  const items = new Map();
  return {
    getItem: (key) => (items.has(key) ? items.get(key) : null),
    setItem: (key, value) => items.set(key, String(value)),
    removeItem: (key) => items.delete(key),
  };
}

test('the welcome page offers the three case studies, and each is a scenario the app can load', async () => {
  const registry = JSON.parse(await readFile(new URL('../models/scenarios.json', import.meta.url), 'utf8'));
  const values = new Set();
  for (const scenario of registry.scenarios) {
    values.add(`sc:${scenario.id}`);
  }
  for (const community of registry.publishedCommunities) {
    values.add(`pub:${community.id}`);
  }
  assert.deepEqual(WELCOME_EXAMPLES.map((example) => example.value), [
    'sc:yeast9-9-1-1',
    'pub:zuniga-2020-phea-rescue',
    'pub:steadycom-ecoli-auxotrophs',
  ]);
  for (const example of WELCOME_EXAMPLES) {
    assert.ok(values.has(example.value), `${example.value} is not in models/scenarios.json`);
  }
});

test('the welcome page shows the license and links the user guide, the source code and the citation', () => {
  const html = welcomeMarkup(true);
  assert.deepEqual(WELCOME_LINKS.map((link) => link.label), ['User guide', 'Source code', 'How to cite']);
  for (const link of WELCOME_LINKS) {
    assert.match(html, new RegExp(`href="${link.href}"`));
  }
  assert.equal(WELCOME_LINKS[0].href, `${SITE_URL}help.html`);
  assert.ok(WELCOME_LINKS[1].href.startsWith(REPOSITORY_URL));
  assert.ok(WELCOME_LINKS[2].href.startsWith(REPOSITORY_URL));
  assert.match(html, new RegExp(`href="${CODE_LICENSE_URL}"[^>]*>MIT License<`));
  assert.match(html, new RegExp(`href="${BUILD_LICENSE_URL}"[^>]*>GPL-3.0<`));
  assert.match(html, /Free to use, no login/);
  assert.match(html, /data-action="welcome-import"/);
  assert.equal(html.match(/data-action="welcome-example"/g).length, 3);
});

test('the welcome page opens at start until the user turns it off, and not when the address names a scenario', () => {
  const storage = memoryStorage();
  assert.equal(welcomeOpensAtStart(storage, ''), true);
  assert.equal(welcomeOpensAtStart(storage, '?scenario=anaerobic-consortium'), false);

  rememberWelcomeAtStart(storage, false);
  assert.equal(storage.getItem(WELCOME_STORAGE_KEY), 'off');
  assert.equal(welcomeOpensAtStart(storage, ''), false);
  assert.doesNotMatch(welcomeMarkup(welcomeOpensAtStart(storage)), /data-action="welcome-at-start" checked/);

  rememberWelcomeAtStart(storage, true);
  assert.equal(welcomeOpensAtStart(storage, ''), true);
  assert.match(welcomeMarkup(welcomeOpensAtStart(storage)), /data-action="welcome-at-start" checked/);
});

test('storage the browser refuses leaves the welcome page opening at start', () => {
  const refusing = {
    getItem: () => { throw new Error('denied'); },
    setItem: () => { throw new Error('denied'); },
    removeItem: () => { throw new Error('denied'); },
  };
  assert.equal(welcomeOpensAtStart(refusing, ''), true);
  assert.equal(welcomeOpensAtStart(null, ''), true);
  assert.doesNotThrow(() => rememberWelcomeAtStart(refusing, false));
});

test('a link opens a teaching scenario by its id or sc: value, and a published community by its pub: value', () => {
  assert.deepEqual(scenarioFromAddress('?scenario=yeast9-9-1-1'), { kind: 'scenario', id: 'yeast9-9-1-1' });
  assert.deepEqual(scenarioFromAddress('?scenario=sc:yeast9-9-1-1'), { kind: 'scenario', id: 'yeast9-9-1-1' });
  assert.deepEqual(scenarioFromAddress('?scenario=pub:steadycom-ecoli-auxotrophs'),
    { kind: 'published', id: 'steadycom-ecoli-auxotrophs' });
  for (const example of WELCOME_EXAMPLES) {
    const opened = scenarioFromAddress(`?scenario=${encodeURIComponent(example.value)}`);
    assert.equal(`${opened.kind === 'published' ? 'pub' : 'sc'}:${opened.id}`, example.value);
  }
  assert.equal(scenarioFromAddress(''), null);
  assert.equal(scenarioFromAddress('?scenario='), null);
});
