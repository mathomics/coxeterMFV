import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import factory from '../assets/vendor/glpk.js';
import { initGLPK } from '../src/solver/glpk.js';
import { Model } from '../src/core/model.js';
import { Store } from '../src/ui/store.js';
import { EditableMap } from '../src/map/editable-map.js';
import { ZUNIGA_ID, createZunigaScenario } from '../src/communities/zuniga.js';
import { renderWorkspace, renderInspector } from '../src/ui/panels.js';

test.before(async () => {
  globalThis.__GLPK_FACTORY__ = factory;
  globalThis.__GLPK_WASM__ = await readFile(new URL('../assets/vendor/glpk.wasm', import.meta.url));
  assert.equal(await initGLPK(), true);
});

// Synthetic small LPs test integration only. They are not the published models.
function fixtureStudy() {
  const rxn = (id, metabolites, extra = {}) => ({ id, metabolites, lower_bound: 0, upper_bound: 100, ...extra });
  const metabolite = id => ({ id, name: id, compartment: id.endsWith('[smp]') ? 's' : 'c' });
  const bio = 'BIOMASS_Ec_iJO1366_core_53p95M';
  const mono = {
    id: 'synthetic-integration-fixture-Ec', compartments: { c: 'Cytosol' },
    metabolites: ['glc_c', 'phe__L_c'].map(metabolite),
    reactions: [
      rxn('SOURCE', { glc_c: 1 }, { upper_bound: 10 }),
      rxn('PPNDH', { glc_c: -1, phe__L_c: 1 }, { gene_reaction_rule: 'b2599' }),
      rxn(bio, { glc_c: -1, phe__L_c: -1 }, { objective_coefficient: 1 }),
    ], cell: { name: 'E. coli fixture', organism: 'E. coli' },
  };
  const se = {
    id: 'synthetic-integration-fixture-Se', compartments: { c: 'Cytosol' },
    metabolites: ['glc_c', 'phe__L_c'].map(metabolite),
    reactions: [rxn('SOURCE', { glc_c: 1 }), rxn('MAKEPHE', { glc_c: -1, phe__L_c: 1 }), rxn('BOF', { glc_c: -1 }, { objective_coefficient: 1 })],
    cell: { name: 'S. elongatus fixture', organism: 'S. elongatus' },
  };
  const joint = {
    id: 'synthetic-integration-fixture-community', compartments: { c: 'Cytosol', s: 'Shared medium' },
    metabolites: ['glc_c_D', 'phe__L_c_D', 'glc_c_A', 'phe__L_c_A', 'phe__L[smp]'].map(metabolite),
    reactions: [
      rxn('SOURCE_D', { glc_c_D: 1 }, { upper_bound: 10 }),
      rxn('SOURCE_A', { glc_c_A: 1 }, { upper_bound: 10 }),
      rxn('PPNDH_D', { glc_c_D: -1, phe__L_c_D: 1 }, { gene_reaction_rule: 'b2599' }),
      rxn('MAKEPHE_A', { glc_c_A: -1, phe__L_c_A: 1 }),
      rxn('PHELtex_A', { phe__L_c_A: 1, 'phe__L[smp]': -1 }, { lower_bound: -100 }),
      rxn('PHEtex_D', { 'phe__L[smp]': -1, phe__L_c_D: 1 }),
      rxn(`${bio}_D`, { glc_c_D: -1, phe__L_c_D: -1 }, { objective_coefficient: 1 }),
      rxn('BOF_A', { glc_c_A: -1 }, { objective_coefficient: 0.5 }),
    ],
  };
  return { manifest: { community: ZUNIGA_ID, runtime: 'zuniga-mat', map: 'map.json', fluxReference: 10, members: [{ id: 'Ec' }, { id: 'Se' }] }, sourceHashes: { fixture: 'synthetic-only' }, sourceModels: { mono, joint }, models: { Ec: new Model(mono), Se: new Model(se) } };
}

async function fixtureStore() {
  const study = fixtureStudy();
  const context = createZunigaScenario(study, 'mutant-coculture').community;
  const map = EditableMap.emptyFromCommunity(context, { width: 1800, height: 1200 });
  map.addReaction(context, 'Ec@PPNDH');
  map.addReaction(context, 'Se@PHELtex');
  globalThis.__ESCHER_MCMC_DATA__ = { [`models/published/${ZUNIGA_ID}/map.json`]: map.toJSON() };
  const store = new Store('.');
  store.publishedCommunities = [{ id: ZUNIGA_ID, name: 'Synthetic integration fixture', species: [{ id: 'Ec', name: 'Ec', organism: 'E. coli' }, { id: 'Se', name: 'Se', organism: 'S. elongatus' }], paper: { title: 'Fixture', year: 2020, authors: 'Fixture', journal: 'Fixture', doi: '10.0000/fixture', framework: 'FBA' }, reproduction: { status: 'reproduced', needs: [] } }];
  await store.useZunigaStudy({ study });
  return store;
}

test('the case solves complete source LPs and excludes Se from monoculture growth', async () => {
  const store = await fixtureStore();
  assert.equal(store.models.get(`published/${ZUNIGA_ID}/model-Ec`).reactions.size, 3);
  assert.equal(store.problem.rxnIds.length, 8);
  assert.equal(store.map.reactions.size, 2, 'the visual excerpt is independent of the LP');
  assert.equal(store.zunigaSolutions.get('wt-monoculture').zuniga.growth.Ec, 5);
  assert.equal(store.zunigaSolutions.get('mutant-monoculture').zuniga.growth.Ec, 0);
  assert.ok(store.growth.get('Ec') > 0);
  assert.ok(store.crossFeeding.links.some(link => link.base === 'phe__L' && link.from === 'Se' && link.to === 'Ec' && link.flux > 0));
  store.useZunigaScenario('mutant-monoculture');
  assert.deepEqual([...store.growth.keys()], ['Ec']);
  assert.equal(store.currentReactionBounds('Ec@PPNDH').upper, 0);
});

test('switching scenarios preserves edited geometry/history while changing the exact model and knockout', async () => {
  const store = await fixtureStore();
  const node = [...store.map.nodes.values()].find(node => node.cellId === 'Ec');
  const edit = store.beginMapEdit('Move fixture node');
  store.moveNode(node.id, node.x + 15, node.y + 10);
  store.commitMapEdit(edit);
  const drawing = store.map.toJSON(), history = store.mapHistory;
  store.useZunigaScenario('wt-monoculture');
  assert.ok(store.currentReactionBounds('Ec@PPNDH').upper > 0);
  store.useZunigaScenario('mutant-coculture');
  assert.deepEqual(store.map.toJSON(), drawing);
  assert.equal(store.mapHistory, history);
  assert.equal(store.problem.rxnIds.length, 8);
});

test('comparison uses a fixed scale and leaves actual fluxes, bounds and growth unchanged', async () => {
  const store = await fixtureStore();
  const solution = store.solution, fluxes = new Map(solution.fluxes), bounds = store.problem.snapshotBounds(), growth = new Map(store.growth);
  const baseline = store.zunigaSolutions.get('wt-monoculture').fluxes;
  store.setZunigaComparison('difference');
  assert.equal(store.displayFluxes().get('Ec@PPNDH'), fluxes.get('Ec@PPNDH') - baseline.get('Ec@PPNDH'));
  assert.equal(store.scale.reference, 10);
  assert.equal(store.solution, solution);
  assert.deepEqual(store.solution.fluxes, fluxes);
  assert.deepEqual(store.problem.snapshotBounds(), bounds);
  assert.deepEqual(store.growth, growth);
  store.useZunigaScenario('mutant-monoculture');
  assert.equal(store.scale.reference, 10);
  assert.equal(store.zunigaComparison, 'difference');
});

test('generic biological edits cannot silently replace the original formulation', async () => {
  const store = await fixtureStore();
  const problem = store.problem, snapshot = problem.snapshotBounds();
  assert.throws(() => store.setMediumLimit('glc', 2), /authors’ objective/);
  assert.throws(() => store.setReactionBounds('Ec@PPNDH', 0, 20), /authors’ objective/);
  assert.throws(() => store.setParsimonious(true), /authors’ objective/);
  await assert.rejects(store.setAbundance('Ec', 0.1), /authors’ objective/);
  await assert.rejects(store.setObjective('equal-growth'), /authors’ objective/);
  await assert.rejects(store.setSpeciesEnabled('Se', false), /authors’ objective/);
  await assert.rejects(store.createModelMetabolite({ id: 'new_c', compartment: 'c' }), /authors’ objective/);
  assert.equal(store.problem, problem);
  assert.deepEqual(store.problem.snapshotBounds(), snapshot);
});

test('map and scenario JSON restore the selected case and comparison without removing the ghost cell', async () => {
  const store = await fixtureStore();
  store.useZunigaScenario('mutant-monoculture');
  store.setZunigaComparison('difference');
  const document = store.mapJSON(), drawing = store.map.toJSON(), state = store.stateJSON();
  store.useZunigaScenario('wt-monoculture');
  store.loadMapJSON(document);
  assert.equal(store.zunigaScenarioId, 'mutant-monoculture');
  assert.equal(store.zunigaComparison, 'difference');
  assert.deepEqual(store.map.toJSON(), drawing);
  assert.equal(store.map.cells.length, 2);
  store.useZunigaScenario('mutant-coculture');
  await store.loadState(state);
  assert.equal(store.zunigaScenarioId, 'mutant-monoculture');
  assert.equal(store.zunigaComparison, 'difference');
  assert.deepEqual(store.map.toJSON(), drawing);
});

test('a private bundle replays the complete source models without the generic community merger', async () => {
  const store = await fixtureStore();
  store.useZunigaScenario('mutant-monoculture');
  store.setZunigaComparison('difference');
  const bundle = await store.reproducibilityBundle();
  assert.equal(bundle.study.sourceModels.joint.reactions.length, 8);
  assert.equal(bundle.study.activeModel.reactions.length, 3);
  const restored = new Store('.');
  const result = await restored.importReproducibilityBundle(bundle);
  assert.equal(result.valid, true);
  assert.equal(result.replay.pass, true);
  assert.equal(restored.zunigaScenarioId, 'mutant-monoculture');
  assert.deepEqual(restored.map.toJSON(), store.map.toJSON());
  assert.equal(restored.map.cells.length, 2);
  restored.useZunigaScenario('mutant-coculture');
  assert.ok(restored.growth.get('Ec') > 0);
});

test('the case controls expose honest source constraints, growth, comparison and editable map tools', async () => {
  const store = await fixtureStore();
  const html = renderWorkspace(store, 'scenario');
  assert.match(html, /data-action="zuniga-scenario"/);
  assert.match(html, /data-action="zuniga-comparison"/);
  assert.match(html, /Ec PPNDH \(pheA\)/);
  assert.match(html, /Calculated phenylalanine transfer/);
  assert.match(html, /not a comparison of experimental/);
  assert.doesNotMatch(html, /id="objective-select"/);
  assert.match(html, /data-action="export-png"/);
  store.selectEntity('reaction', 'Ec@PPNDH');
  const inspector = renderInspector(store);
  assert.match(inspector, /<fieldset disabled/);
  assert.match(inspector, /data-action="remove-map-reaction"/);
  assert.doesNotMatch(renderWorkspace(store, 'medium'), /data-action="medium"/);
  assert.doesNotMatch(renderWorkspace(store, 'community'), /data-action="abundance"/);
});

test('invalid saved case settings are rejected before replacing the active scenario', async () => {
  const store = await fixtureStore();
  const solution = store.solution, drawing = store.map.toJSON();
  const document = store.mapDocument();
  document.study.scenarioId = 'invented-case';
  assert.throws(() => store.loadMapJSON(JSON.stringify(document)), /scenario is invalid/);
  assert.equal(store.solution, solution);
  assert.deepEqual(store.map.toJSON(), drawing);
  const state = JSON.parse(store.stateJSON());
  state.study.fluxReference = 999;
  await assert.rejects(store.loadState(JSON.stringify(state)), /different flux scale/);
  assert.equal(store.solution, solution);
});

test('another scenario supersedes a late model download without installing its stale biology', async () => {
  const store = await fixtureStore();
  const study = fixtureStudy();
  let resolveDownload, reportProgress;
  let signalCalled;
  const called = new Promise(resolve => { signalCalled = resolve; });
  store.loadZunigaStudy = (_manifest, options) => {
    reportProgress = options.onProgress;
    signalCalled();
    return new Promise(resolve => { resolveDownload = resolve; });
  };
  const pending = store.useZunigaStudy();
  await called;
  store.models.set('ordinary-fixture', study.models.Ec);
  store.scenarios = [{ id: 'ordinary', name: 'Ordinary fixture', species: [{ id: 'Ec', model: 'ordinary-fixture' }], medium: {} }];
  await store.useScenario('ordinary');
  const ordinaryProblem = store.problem, status = store.status;
  reportProgress('Stale source download status');
  resolveDownload(study);
  await assert.rejects(pending, { name: 'AbortError' });
  assert.equal(store.problem, ordinaryProblem);
  assert.equal(store.status, status);
  assert.equal(store.scenario.id, 'ordinary');
  assert.equal(store.isZunigaStudy, false);
  assert.equal(store.zunigaComparison, 'absolute');
});

test('a re-solve leaves the header to its summary and records itself in the log', async () => {
  const store = await fixtureStore();
  store.log = [];
  store.solve();
  store.solve();
  assert.equal(store.status.state, 'ok');
  assert.equal(store.status.message, '', 'a notice on every re-solve hid the objective for six seconds');
  assert.equal(store.log.length, 1);
  assert.match(store.log[0].message, /exact source constraints solved: optimal/);
  assert.equal(store.log[0].count, 2);
});

test('opening the case reads Saved', async () => {
  const store = await fixtureStore();
  assert.deepEqual(store.unsavedWork(), { map: false, scenario: false, model: false });
});
