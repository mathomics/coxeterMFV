import test from 'node:test';
import assert from 'node:assert/strict';
import { Store } from '../src/ui/store.js';
import { renderTaskPanel, renderWorkspace, renderViewControls } from '../src/ui/panels.js';

const actions = (html) => [...html.matchAll(/data-action="([^"]+)"/g)].map((match) => match[1]);
const routes = (html) => [...html.matchAll(/data-workspace="([^"]+)"/g)].map((match) => match[1]);

test('task navigation stays bounded as a community and its medium grow', () => {
  const small = new Store('.');
  small.species = [{ id: 'species_0', enabled: true }];
  const large = new Store('.');
  large.species = Array.from({ length: 1000 }, (_, i) => ({ id: `species_${i}`, enabled: i % 2 === 0 }));
  large.medium = Object.fromEntries(Array.from({ length: 500 }, (_, i) => [`nutrient_${i}`, i]));

  for (const leftTab of ['build', 'model', 'analysis']) {
    const before = renderTaskPanel(small, { leftTab });
    const after = renderTaskPanel(large, { leftTab });
    assert.deepEqual(actions(after), actions(before), `${leftTab} controls grow with model data`);
    assert.deepEqual(routes(after), routes(before), `${leftTab} navigation changes with model data`);
    assert.doesNotMatch(after, /data-action="(?:abundance|medium|medium-number|species-toggle)"/);
    assert.doesNotMatch(after, /nutrient_499|species_999/);
  }
  assert.match(renderTaskPanel(large, { leftTab: 'model' }), /500 active species/);
});

test('current analysis labels reflect actual method and biomass target without changing state', () => {
  const variants = [
    { state: {}, label: /<strong>FBA<\/strong>/ },
    { state: { parsimonious: true, pfbaBiomassPercent: 87.5 }, label: /Biomass target: 87\.5% of optimum/ },
    { state: { parsimonious: true, fvaEnabled: true }, label: /<strong>pFBA \+ FVA<\/strong>/ },
    { state: { loopless: true }, label: /<strong>Loopless FBA<\/strong>/ },
    { state: { omics: { gimmeEnabled: true, gimmeObjectiveFraction: 0.8 } }, label: /Objective target: 80%/ },
  ];
  for (const variant of variants) {
    const store = Object.assign(new Store('.'), variant.state);
    store.solution = { status: 'optimal', objectiveValue: 0.15785 };
    const before = structuredClone({ parsimonious: store.parsimonious, target: store.pfbaBiomassPercent, loopless: store.loopless, fva: store.fvaEnabled, objective: store.solution.objectiveValue, omics: store.omics });
    assert.match(renderTaskPanel(store, { leftTab: 'analysis' }), variant.label);
    assert.deepEqual({ parsimonious: store.parsimonious, target: store.pfbaBiomassPercent, loopless: store.loopless, fva: store.fvaEnabled, objective: store.solution.objectiveValue, omics: store.omics }, before);
  }
});

test('central medium editor retains every imported nutrient and its editable value', () => {
  const store = new Store('.');
  store.medium = Object.fromEntries(Array.from({ length: 80 }, (_, i) => [`imported_${i}`, i + 0.5]));
  const html = renderWorkspace(store, 'medium');
  const values = [...html.matchAll(/data-action="medium-number" data-base="([^"]+)"/g)].map((match) => match[1]);
  assert.equal(values.length, 80);
  assert.deepEqual(new Set(values), new Set(Object.keys(store.medium)));
  assert.match(html, /value="79\.5"[\s\S]*?data-action="medium-number" data-base="imported_79"/);
  assert.doesNotMatch(html, /data-action="objective"|data-action="new-reaction"|data-action="abundance"/);
});

test('central workspaces preserve scientific control event routes and stay focused on one task', () => {
  const store = new Store('.');
  store.scenarios = [{ id: 'test', name: 'Test scenario' }];
  const analysis = renderWorkspace(store, 'analysis');
  for (const action of ['scenario', 'objective', 'parsimonious', 'pfba-biomass-percent', 'loopless', 'fva']) {
    assert.ok(actions(analysis).includes(action), `${action} became unreachable after moving controls`);
  }
  assert.doesNotMatch(analysis, /data-action="medium-number"|data-action="new-reaction"/);
  const model = renderWorkspace(store, 'model');
  assert.ok(actions(model).includes('new-metabolite'));
  assert.ok(actions(model).includes('new-reaction'));
  assert.doesNotMatch(model, /data-action="objective"|data-action="medium-number"/);
});

test('View menu exposes presentation controls with their actual selected state', () => {
  const store = new Store('.');
  store.view.showCurrency = false;
  store.view.showCrossFeeding = true;
  store.view.labels = 'none';
  const html = renderViewControls(store);
  assert.match(html, /data-action="toggle-currency" aria-pressed="false"/);
  assert.match(html, /data-action="toggle-crossfeed" aria-pressed="true"/);
  assert.match(html, /value="none" selected/);
  assert.doesNotMatch(html, /data-action="(?:objective|medium|parsimonious|loopless|fva|toggle-dark)"/);
});
