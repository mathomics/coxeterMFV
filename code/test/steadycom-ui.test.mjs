import test from 'node:test';
import assert from 'node:assert/strict';
import { Store } from '../src/ui/store.js';
import { renderResults, renderTaskPanel, renderWorkspace } from '../src/ui/panels.js';

function fixture(id = 'steadycom-ecoli-auxotrophs', { published = true } = {}) {
  const store = new Store('.');
  store.scenarios = [{ id: 'teaching-mutualism', name: 'Teaching mutualism', description: 'Two-member teaching model.' }];
  if (published) {
    const entry = {
      id, name: id, description: 'Published benchmark.', species: [],
      paper: { framework: 'SteadyCom', title: 'A recorded community', authors: 'Authors', year: 2026 },
      reproduction: { status: 'reproduced', reproduces: [], adaptations: [], needs: [] },
      computed: { growthRate: 0.73599, lpCount: 23, seconds: 42 },
    };
    store.publishedCommunities = [entry];
    store.scenario = { id: `published:${id}`, published: id, name: id };
  } else {
    store.scenario = store.scenarios[0];
  }
  const communityId = published ? id : store.scenario.id;
  store.steadyComSupport = () => ({ supported: true, communityId, kind: id === 'steadycom-gut-nine' ? 'joint' : 'separate', reason: '' });
  store.steadyComConfig = { fiber: null };
  return store;
}

function tag(html, action, element = 'input') {
  const match = html.match(new RegExp(`<${element}\\b[^>]*data-action="${action}"[^>]*>`));
  assert.ok(match, `Missing ${action}`);
  return match[0];
}

test('recorded benchmarks never enable a growth-rate solve before calculating the current range', () => {
  const store = fixture();
  const html = renderWorkspace(store, 'scenario');
  assert.match(html, /Recorded µ \(offline\):<\/b> 0\.73599/);
  assert.match(html, /Not calculated for the current inputs/);
  assert.match(tag(html, 'steadycom-mu'), / disabled/);
  assert.match(tag(html, 'steadycom-mu-number'), / disabled/);
  assert.match(tag(html, 'steadycom-solve', 'button'), / disabled/);
  assert.doesNotMatch(tag(html, 'steadycom-recompute', 'button'), / disabled/);
});

test('calculated bounds and target preserve their exact numeric values', () => {
  const store = fixture();
  const max = 0.7359843211234567;
  store.steadyComState = { communityId: 'steadycom-ecoli-auxotrophs', key: 'inputs-1', source: 'computed', min: 0, max, resolution: 1e-7, upperBound: max + 1e-7, phase: null, mu: max };
  const html = renderWorkspace(store, 'scenario');
  assert.ok(tag(html, 'steadycom-mu').includes(`max="${max}"`));
  assert.ok(tag(html, 'steadycom-mu-number').includes(`value="${max}"`));
  assert.match(tag(html, 'steadycom-mu-number'), /step="any"/);
  assert.doesNotMatch(tag(html, 'steadycom-solve', 'button'), / disabled/);
  assert.match(html, /Search resolution: 1e-7 h⁻¹/);
});

test('worker progress disables duplicate requests and exposes cancellation', () => {
  const store = fixture();
  store.steadyComState = { communityId: 'steadycom-ecoli-auxotrophs', key: 'inputs-1', source: 'computed', min: 0, max: null, phase: 'range', progress: { iteration: 9 } };
  const html = renderWorkspace(store, 'scenario');
  assert.match(tag(html, 'steadycom-recompute', 'button'), / disabled/);
  assert.match(tag(html, 'steadycom-solve', 'button'), / disabled/);
  assert.doesNotMatch(tag(html, 'steadycom-cancel', 'button'), / hidden/);
  assert.match(html, /data-steadycom-progress[^>]*role="status"[^>]*>Calculating[\s\S]*iteration 9/);
});

test('zero growth and inactive species are described without claiming a unique composition', () => {
  const store = fixture();
  store.steadyComState = { communityId: 'steadycom-ecoli-auxotrophs', key: 'inputs-1', source: 'computed', min: 0, max: 0.7, phase: null, mu: 0, feasible: true, abundances: new Map([['A', 1], ['B', 0]]), solveMs: 12 };
  const html = renderWorkspace(store, 'scenario');
  assert.match(html, /Composition at µ = 0 h⁻¹/);
  assert.match(html, /B inactive \(X = 0\)/);
  assert.match(html, /Composition may be non-unique/);
  assert.doesNotMatch(html, /answer unique|community is pinned down/);
});

test('nine-species conditions distinguish baseline from optional Figure 4 fiber limits', () => {
  const store = fixture('steadycom-gut-nine');
  let html = renderWorkspace(store, 'scenario');
  assert.match(html, /value="baseline" selected/);
  assert.doesNotMatch(html, /data-action="steadycom-fiber"/);
  store.steadyComConfig.fiber = { bacteroides: 12.5, faecalibacterium: 5 };
  html = renderWorkspace(store, 'scenario');
  assert.match(html, /value="figure4" selected/);
  assert.match(tag(html, 'steadycom-fiber'), /min="1" max="30" value="12.5"/);
  assert.match(html, /F\. prausnitzii is held at 5/);
  assert.match(html, /invalidates the calculated range/);
});

test('supported teaching communities expose the same fixed-growth workflow', () => {
  const store = fixture('teaching-mutualism', { published: false });
  const html = renderWorkspace(store, 'scenario');
  assert.match(html, /class="sc-steadycom" data-steadycom-community="teaching-mutualism"/);
  assert.ok(tag(html, 'steadycom-recompute', 'button'));
  assert.doesNotMatch(html, /Recorded published results/);
});

test('growth-rate drafts survive rerenders only for the same input key', () => {
  const store = fixture();
  store.steadyComState = { communityId: 'steadycom-ecoli-auxotrophs', key: 'inputs-1', source: 'computed', min: 0, max: 0.7, phase: null, mu: 0.3 };
  const draft = { communityId: 'steadycom-ecoli-auxotrophs', key: 'inputs-1', value: '', invalidField: 'mu', error: 'Enter a finite rate.' };
  let html = renderWorkspace(store, 'scenario', { steadyComDraft: draft });
  assert.match(tag(html, 'steadycom-mu-number'), /value=""[^>]*aria-invalid="true"/);
  assert.match(html, /data-steadycom-error role="alert">Enter a finite rate/);
  store.steadyComState.key = 'inputs-2';
  html = renderWorkspace(store, 'scenario', { steadyComDraft: draft });
  assert.match(tag(html, 'steadycom-mu-number'), /value="0\.3"/);
  assert.doesNotMatch(html, /Enter a finite rate/);
});

test('fixed-growth results take precedence over ordinary mode flags and disable incompatible FVA', () => {
  const store = fixture();
  store.solution = { status: 'optimal', objectiveValue: 0.2, fluxes: new Map(), backend: 'glpk', steadyCom: { mu: 0.2 } };
  store.parsimonious = true;
  store.loopless = true;
  store.fvaEnabled = true;
  const task = renderTaskPanel(store, { leftTab: 'analysis' });
  assert.match(task, /<strong>SteadyCom fixed µ<\/strong>/);
  assert.match(task, /Abundances are outputs of this solve/);
  assert.match(tag(task, 'run-fva', 'button'), / disabled/);
  assert.match(renderResults(store), /SteadyCom fixed µ · Optimal/);
  assert.match(tag(renderWorkspace(store, 'scenario'), 'fva'), / disabled/);
});
