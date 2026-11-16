import test from 'node:test';
import assert from 'node:assert/strict';

import { fluxDataJSON, matchFluxData, parseDelimitedRows, parseFluxData } from '../src/core/flux-data.js';
import { Model } from '../src/core/model.js';
import { Store } from '../src/ui/store.js';
import { reactionColumns } from '../src/ui/reactions-panel.js';

/** A one-cell store on a three-reaction model, solved by useScenario. */
async function makeStore() {
  const store = new Store('.');
  store.models.set('demo', new Model({
    id: 'demo', compartments: { c: 'Cytosol', e: 'Extracellular' },
    metabolites: [{ id: 'a_e', compartment: 'e' }, { id: 'a_c', compartment: 'c' }],
    reactions: [
      { id: 'EX_a_e', metabolites: { a_e: -1 }, lower_bound: -10, upper_bound: 1000 },
      { id: 'At', metabolites: { a_e: -1, a_c: 1 }, lower_bound: 0, upper_bound: 1000 },
      { id: 'BIOMASS', metabolites: { a_c: -1 }, lower_bound: 0, upper_bound: 1000, objective_coefficient: 1 },
    ],
  }));
  store.scenarios = [{
    id: 'demo', name: 'Demo', objective: 'equal-growth',
    species: [{ id: 'demo', model: 'demo', abundance: 1 }], medium: { a: 10 },
  }];
  await store.useScenario('demo');
  return store;
}

test('Escher reaction data JSON reads as a flux for each reaction, and values that are not numbers are skipped', () => {
  const data = parseFluxData('{"PGI": 4.86, "PFK": "7.5", "GAPD": null, "TPI": ""}');
  assert.deepEqual([...data], [['PGI', 4.86], ['PFK', 7.5]]);
  assert.deepEqual([...parseFluxData('[{"PGI": -1}]')], [['PGI', -1]]);
});

test('a CSV with reaction and flux columns reads, as does the Flux table export with quoted names and a two-column table without a header', () => {
  const exported = 'reaction,species,local_id,name,subsystem,flux,lower_bound,upper_bound\n'
    + 'eco@PGI,eco,PGI,"glucose-6-phosphate isomerase, cytosolic",Glycolysis,4.860000,-1000,1000\n';
  assert.deepEqual([...parseFluxData(exported)], [['eco@PGI', 4.86]]);
  assert.deepEqual([...parseFluxData('PGI\t4.86\r\nPFK\t-2\r\n')], [['PGI', 4.86], ['PFK', -2]]);
  assert.deepEqual(parseDelimitedRows('a;b\n"x;y";"say ""hi"""\n'), [['a', 'b'], ['x;y', 'say "hi"']]);
});

test('a comparison file with two data sets is refused, and a file with no fluxes says so', () => {
  assert.throws(() => parseFluxData('[{"PGI": 1}, {"PGI": 2}]'), /2 data sets/);
  assert.throws(() => parseFluxData('{"PGI": "fast"}'), /No reaction fluxes/);
  assert.throws(() => parseFluxData('name,comment\nPGI,fast\n'), /reaction column and a flux column/);
  assert.throws(() => parseFluxData('   '), /empty/);
});

test('ids match the model reaction or the one species copy of it, and ids shared by several species stay unmatched', () => {
  const data = new Map([['Ec1@PGI', 1], ['PFK', 2], ['ATPM', 3], ['MISSING', 4]]);
  const ids = ['Ec1@PGI', 'Ec2@PGI', 'Ec1@PFK', 'Ec1@ATPM', 'Ec2@ATPM'];
  const { fluxes, unmatched } = matchFluxData(data, ids);
  assert.deepEqual([...fluxes], [['Ec1@PGI', 1], ['Ec1@PFK', 2]]);
  assert.deepEqual(unmatched, ['ATPM', 'MISSING']);
});

test('exported flux data imports back onto the same model as the same fluxes, without reduced costs or a replayable bundle', async () => {
  const store = await makeStore();
  assert.equal(store.solution.status, 'optimal');
  const solved = new Map(store.solution.fluxes);
  const exported = store.fluxDataJSON();
  assert.deepEqual(JSON.parse(exported), Object.fromEntries(solved));
  assert.equal(exported, fluxDataJSON(solved));

  const report = store.importFluxData(exported, 'demo_fluxes.json');
  assert.equal(report.matched, solved.size);
  assert.deepEqual(report.unmatched, []);
  assert.deepEqual([...store.solution.fluxes], [...solved]);
  assert.equal(store.solution.imported.source, 'demo_fluxes.json');
  assert.equal(store.reducedCostsAvailable(), false);
  await assert.rejects(() => store.reproducibilityBundle(), /imported flux data has no analysis to replay/);

  // Escher data keyed by the model's own ids lands on the cell's copies.
  store.importFluxData('{"BIOMASS": 2.5, "ZZZ": 1}', 'escher.json');
  assert.equal(store.solution.fluxes.get('demo@BIOMASS'), 2.5);
  assert.equal(store.solution.fluxes.size, 1);
  assert.equal(store.status.state, 'warn');
  assert.match(store.status.message, /1 ids are not in this model \(ZZZ\)/);

  const before = store.solution;
  assert.throws(() => store.importFluxData('{"NOPE": 1}', 'other.json'), /None of the 1 reactions in other.json is in this model/);
  assert.equal(store.solution, before);
});

test('the reactions table shows the scaled reduced cost beside the reduced cost, and the flux table exports both', async () => {
  const store = await makeStore();
  const model = store.reactionTableModel();
  assert.equal(model.columns.rc, true);
  for (const mode of ['page', 'compact']) {
    const ids = reactionColumns(model, mode).map((column) => column.id);
    assert.equal(ids.indexOf('src'), ids.indexOf('rc') + 1);
  }
  // The uptake bound limits growth. The cell's exchange and the community's
  // carry it in series, so a solver may put the reduced cost on either.
  const limiting = model.rows.filter((entry) => Number.isFinite(entry.reducedCost) && entry.reducedCost !== 0);
  assert.equal(limiting.length, 1);
  assert.ok(['EX_a_s', 'demo@EX_a_e'].includes(limiting[0].id));
  assert.equal(limiting[0].reducedCost, -1);
  assert.equal(limiting[0].scaledReducedCost, -1);

  const [header, ...lines] = store.fluxCSV().split('\n');
  assert.match(header, /,reduced_cost,scaled_reduced_cost$/);
  const line = lines.find((entry) => entry.startsWith(`"${limiting[0].id}",`)).split(',');
  assert.deepEqual(line.slice(-2), ['-1', '-1']);
});

test('flux data JSON lists each reaction id with its flux and reads back unchanged', () => {
  const fluxes = new Map([['Ec1@PGI', 4.86], ['EX_glc__D_s', -10], ['ATPM', 8.39]]);
  assert.equal(fluxDataJSON(fluxes), '{\n  "Ec1@PGI": 4.86,\n  "EX_glc__D_s": -10,\n  "ATPM": 8.39\n}');
  assert.deepEqual(parseFluxData(fluxDataJSON(fluxes)), fluxes);
});
