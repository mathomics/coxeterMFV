import test from 'node:test';
import assert from 'node:assert/strict';
import { Store } from '../src/ui/store.js';
import { Model } from '../src/core/model.js';
import {
  classifyReaction, signLogPosition, signLogSteps, signLogValue, virtualWindow,
} from '../src/ui/reactions-explorer.js';
import { renderResults, renderWorkspace } from '../src/ui/panels.js';
import { renderMap } from '../src/render/mapview.js';

// One cell: two nutrients in, a transport, an internal step, biomass and a demand.
async function makeStore() {
  const store = new Store('.');
  store.models.set('explorer-demo', new Model({
    id: 'explorer-demo',
    compartments: { c: 'Cytosol', e: 'Extracellular' },
    metabolites: [
      { id: 'a_e', compartment: 'e' }, { id: 'a_c', compartment: 'c' },
      { id: 'z_e', compartment: 'e' }, { id: 'b_c', compartment: 'c' },
    ],
    reactions: [
      { id: 'EX_a_e', metabolites: { a_e: -1 }, lower_bound: -10, upper_bound: 1000 },
      { id: 'EX_z_e', metabolites: { z_e: -1 }, lower_bound: -5, upper_bound: 1000 },
      { id: 'At', metabolites: { a_e: -1, a_c: 1 }, lower_bound: 0, upper_bound: 1000 },
      { id: 'Zt', metabolites: { z_e: -1, b_c: 1 }, lower_bound: 0, upper_bound: 1000 },
      { id: 'R1', name: 'a to b', metabolites: { a_c: -1, b_c: 1 }, lower_bound: 0, upper_bound: 1000, gene_reaction_rule: 'g1' },
      { id: 'BIOMASS', metabolites: { b_c: -1 }, lower_bound: 0, upper_bound: 1000, objective_coefficient: 1 },
      { id: 'DM_b_c', metabolites: { b_c: -1 }, lower_bound: 0, upper_bound: 1000 },
    ],
  }));
  store.scenarios = [{
    id: 'explorer-demo', name: 'Explorer demo', objective: 'equal-growth',
    species: [{ id: 'demo', model: 'explorer-demo', abundance: 1 }],
    medium: { a: 10, z: 5 },
  }];
  await store.useScenario('explorer-demo');
  assert.equal(store.solution.status, 'optimal');
  return store;
}

test('reactions are classified from how the community was built', async () => {
  const store = await makeStore();
  const type = (id) => classifyReaction(store.community, id);
  assert.equal(type('EX_a_s'), 'exchange');
  assert.equal(type('demo@EX_a_e'), 'uptake');
  assert.equal(type('demo@At'), 'transport');
  assert.equal(type('demo@R1'), 'internal');
  assert.equal(type('demo@BIOMASS'), 'biomass');
  assert.equal(type('COMMUNITY_GROWTH'), 'biomass');
  assert.equal(type('demo@DM_b_c'), 'demand');
});

test('the sign-log slider spans ±0.001 to ±1000 with a snap to 0', () => {
  const steps = signLogSteps();
  assert.equal(signLogValue(0), 0);
  assert.equal(signLogValue(1), 0, 'the positions next to 0 snap to 0');
  assert.equal(signLogValue(2), 0.001);
  assert.equal(signLogValue(-2), -0.001);
  assert.equal(signLogValue(steps), 1000);
  assert.equal(signLogValue(-steps - 50), -1000);
  assert.equal(signLogPosition(0), 0);
  assert.equal(signLogPosition(1e6), steps, 'an unbounded 1e6 sits at the end');
  assert.equal(signLogPosition(-1e-6), -2, 'a tiny bound is not shown as 0');
  for (const v of [0.001, 0.0042, 0.37, 1, 12.5, -80, 999]) {
    const back = signLogValue(signLogPosition(v));
    assert.ok(Math.abs(back - v) / Math.abs(v) < 0.06, `${v} came back as ${back}`);
  }
  for (let p = -steps; p < steps; p++) assert.ok(signLogValue(p) <= signLogValue(p + 1));
});

test('exchange bounds and the Medium card edit the same medium', async () => {
  const store = await makeStore();
  store.setReactionBounds('EX_a_s', -4, null);
  assert.equal(store.medium.a, 4, 'the uptake bound is the medium supply');
  assert.equal(store.overrides.has('EX_a_s'), false);
  store.setMediumLimit('a', 6);
  assert.deepEqual(store.currentReactionBounds('EX_a_s'), { lower: -6, upper: 1000 });
  store.setReactionBounds('EX_a_s', null, 50);
  assert.deepEqual(store.overrides.get('EX_a_s'), { lower: -6, upper: 50 }, 'a secretion cap stays an override');
  assert.equal(store.medium.a, 6);
  assert.deepEqual(store.originalReactionBounds('EX_a_s'), { lower: -10, upper: 1000 });
  store.resetReaction('EX_a_s');
  assert.equal(store.medium.a, 10);
  assert.equal(store.overrides.has('EX_a_s'), false);
  assert.deepEqual(store.constraintChanges(), []);
});

test('constraint undo/redo covers bounds, medium and objective and leaves the map history alone', async () => {
  const store = await makeStore();
  store.setReactionBounds('demo@R1', null, 5);
  store.setReactionBounds('demo@R1', null, 3);   // the same slider drag: one step
  store.knockout('demo@At');
  store.setMediumLimit('z', 1);
  store.setObjectiveCoefficient('demo@DM_b_c', 2);
  assert.equal(store.constraintHistory.undoStack.length, 4);
  assert.equal(store.canUndoMap, false);

  await store.undoConstraints();
  assert.equal(store.customObjective, null, 'the objective is back to the preset');
  await store.undoConstraints();
  assert.equal(store.medium.z, 5);
  await store.undoConstraints();
  assert.equal(store.reactionKnockedOut('demo@At'), false);
  await store.undoConstraints();
  assert.equal(store.currentReactionBounds('demo@R1').upper, 1000, 'the whole drag is undone at once');
  assert.equal(store.canUndoConstraints, false);

  await store.redoConstraints();
  assert.equal(store.currentReactionBounds('demo@R1').upper, 3);
  await store.redoConstraints();
  assert.equal(store.reactionKnockedOut('demo@At'), true);
  assert.equal(store.solution.status, 'optimal');
});

test('an edited objective is maximised or minimised and round-trips in the scenario file', async () => {
  const store = await makeStore();
  store.setObjectiveCoefficient('COMMUNITY_GROWTH', 0);
  store.setObjectiveCoefficient('demo@DM_b_c', 1);
  assert.deepEqual(store.objectiveCoefficients(), { 'demo@DM_b_c': 1 });
  assert.ok(Math.abs(store.solution.objectiveValue - 15) < 1e-6, 'both nutrients end in the demand');
  store.setObjectiveCoefficient('demo@DM_b_c', -1);
  store.setObjectiveSense('min');
  assert.ok(Math.abs(store.solution.objectiveValue + 15) < 1e-6, `a minimum is reported with its own sign, got ${store.solution.objectiveValue}`);

  const file = store.stateJSON();
  assert.deepEqual(JSON.parse(file).customObjective, { sense: 'min', coefficients: { 'demo@DM_b_c': -1 } });
  const other = await makeStore();
  await other.loadState(file);
  assert.equal(other.objectiveSense(), 'min');
  assert.deepEqual(other.objectiveCoefficients(), { 'demo@DM_b_c': -1 });
  assert.ok(Math.abs(other.solution.objectiveValue + 15) < 1e-6);
  other.resetObjective();
  assert.deepEqual(other.objectiveCoefficients(), { COMMUNITY_GROWTH: 1 });
});

test('a change that leaves the LP infeasible flags its reaction until it is reverted', async () => {
  const store = await makeStore();
  store.setReactionBounds('demo@R1', 20, null);   // more than the 10 of a on offer
  store.solve();
  assert.notEqual(store.solution.status, 'optimal');
  assert.deepEqual([...store.constraintCulprits.ids], ['demo@R1']);
  assert.equal(store.reactionTableRows().find((row) => row.id === 'demo@R1').status, 'culprit');
  await store.undoConstraints();
  assert.equal(store.solution.status, 'optimal');
  assert.equal(store.constraintCulprits, null);
});

test('bulk actions act on the ticked rows as one undoable edit', async () => {
  const store = await makeStore();
  store.selectReactionRows(['demo@R1', 'demo@At']);
  store.bulkReactionAction('knockout');
  assert.ok(store.reactionKnockedOut('demo@R1') && store.reactionKnockedOut('demo@At'));
  store.bulkReactionAction('reset');
  assert.deepEqual(store.constraintChanges(), []);

  store.bulkReactionAction('scale', { factor: 0.5 }, ['EX_a_s']);
  assert.equal(store.medium.a, 5, 'the finite uptake is scaled, the 1000 left as it is');
  assert.equal(store.currentReactionBounds('EX_a_s').upper, 1000);
  // Ticking the species' own uptake row keeps its nutrient open as well.
  store.bulkReactionAction('close-uptakes-except', {}, ['demo@EX_z_e']);
  assert.equal(store.medium.a, 0);
  assert.equal(store.medium.z, 5);
  await store.undoConstraints();
  assert.equal(store.medium.a, 5);
});

test('bounds CSV export and import, with unknown ids reported', async () => {
  const store = await makeStore();
  const csv = store.boundsCSV();
  assert.match(csv, /^id,lower,upper,species,original_id,name\n/);
  assert.match(csv, /\ndemo@R1,0,1000,demo,R1,a to b\n/);
  const report = store.importBoundsCSV([
    'id,lower,upper,species,original_id',
    'demo@R1,0,7,,',
    ',,9,demo,At',
    'Zt,,4,,',
    'nope,0,1,,',
  ].join('\n'), { name: 'test.csv' });
  assert.equal(report.applied, 3);
  assert.deepEqual(report.unknown, ['nope']);
  assert.equal(store.currentReactionBounds('demo@R1').upper, 7);
  assert.equal(store.currentReactionBounds('demo@At').upper, 9);
  assert.equal(store.currentReactionBounds('demo@Zt').upper, 4, 'an original id only one species has');
  assert.equal(store.status.state, 'warn');
  await store.undoConstraints();
  assert.deepEqual(store.constraintChanges(), [], 'the whole import is one step');
});

test('the table filters, groups and windows its rows', async () => {
  const store = await makeStore();
  store.setReactionBounds('demo@R1', null, 3);
  store.setReactionTable({ toggleFilter: 'modified' });
  let model = store.reactionTableModel();
  assert.deepEqual(model.visible.map((row) => row.id), ['demo@R1']);
  assert.deepEqual(model.items.map((item) => item.kind), ['group', 'row']);
  assert.equal(model.items[0].label, 'Internal');

  store.setReactionTable({ toggleFilter: 'modified', query: 'g1' });
  assert.deepEqual(store.reactionTableModel().visible.map((row) => row.id), ['demo@R1'], 'genes are searched');
  store.setReactionTable({ query: '', groupBy: 'none', sortBy: 'upper' });
  model = store.reactionTableModel();
  assert.equal(model.items.length, model.total);
  assert.equal(model.visible.at(-1).id, 'demo@R1', 'descending by upper bound puts the capped one last');

  assert.deepEqual(virtualWindow({ count: 10000, rowHeight: 30, scrollTop: 3000, viewportHeight: 300, overscan: 5 }),
    { start: 95, end: 115, top: 2850, bottom: 296550 });
});

test('the Reactions page, the dock Table view and the map show the same edits', async () => {
  const store = await makeStore();
  store.knockout('demo@At');
  store.setReactionBounds('demo@R1', null, 3);
  store.solve();
  const page = renderWorkspace(store, 'reactions');
  assert.match(page, /data-action="rx-search"/);
  assert.match(page, /<tr class="rx-row is-knocked-out" data-rid="demo@At">/);
  assert.match(page, /data-action="bound-upper" data-scale="signlog" data-rxn="demo@R1"/);
  assert.match(page, /Changes \(2\)/);

  const dock = renderResults(store, 'flux', { table: { scrollTop: 0, viewportHeight: 400 } });
  assert.match(dock, /data-workspace="reactions">Open in Reactions/);
  assert.match(dock, /data-rid="demo@R1"/);
  assert.doesNotMatch(dock, /data-rxn=/, 'the dock edits no bounds');

  const changes = store.constraintChanges();
  const svg = renderMap(store.map, {
    fluxes: store.solution.fluxes, scale: store.scale,
    disabledReactionIds: new Set(changes.filter((c) => c.knockedOut).map((c) => c.id)),
    modifiedReactionIds: new Set(changes.filter((c) => !c.knockedOut).map((c) => c.id)),
  });
  assert.match(svg, /data-disabled-reaction="demo@At"/);
  assert.match(svg, /data-modified-reaction="demo@R1"/);
  assert.doesNotMatch(renderMap(store.map, { fluxes: new Map(), scale: store.scale, forExport: true, modifiedReactionIds: new Set(['demo@R1']) }), /emc-modified-badge/);
});
