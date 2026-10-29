// A cofactor copy beside one reaction (CO₂ at RuBisCO) can be made primary:
// it becomes that reaction's own metabolite where it stands.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import { EditableMap } from '../src/map/editable-map.js';
import { renderMap } from '../src/render/mapview.js';
import { FluxScale } from '../src/render/theme.js';
import { renderInspector } from '../src/ui/panels.js';
import { Store } from '../src/ui/store.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const RID = 'syn@RBPC';
const CO2 = `cur:${RID}:syn@co2_c`;
const H2O = `cur:${RID}:syn@h2o_c`;

async function cyanoMap() {
  const raw = JSON.parse(await readFile(join(root, 'models', 'cyano_core_mc.json'), 'utf8'));
  const community = new CommunityModel({ species: [{ id: 'syn', model: new Model(raw), abundance: 1 }], medium: {} });
  return EditableMap.fromCommunity(community);
}

const branch = (map, nodeId) => map.segments.find((seg) => seg.reactionId === RID && seg.nodeId === nodeId);

test('CO₂ beside RuBisCO made primary becomes the reaction\'s own metabolite, in place', async () => {
  const map = await cyanoMap();
  const before = { ...map.nodes.get(CO2) };
  assert.equal(before.kind, 'currency');
  assert.equal(branch(map, CO2).kind, 'currency');

  const node = map.setNodePrimary(CO2, true);
  assert.equal(node.kind, 'metabolite');
  assert.equal(node.primary, true);
  assert.equal(node.visualRole, 'primary');
  assert.equal(node.r, 13);
  assert.equal(node.reactionLocalFor, RID);
  assert.equal(node.reactionId, undefined, 'no longer carried by the marker');
  assert.equal(node.escherSecondary, undefined);
  assert.deepEqual([node.x, node.y], [before.x, before.y], 'it stays where it was');
  assert.equal(branch(map, CO2).kind, 'primary');
  assert.equal(map.nodes.get(H2O).kind, 'currency', 'the other cofactors are untouched');

  const reloaded = EditableMap.fromJSON(JSON.parse(JSON.stringify(map.toJSON())));
  assert.equal(reloaded.nodes.get(CO2).kind, 'metabolite');
  assert.equal(reloaded.nodes.get(CO2).primary, true, 'a saved map keeps it primary');
});

test('making a cofactor copy secondary leaves it as it is', async () => {
  const map = await cyanoMap();
  const node = map.setNodePrimary(CO2, false);
  assert.equal(node.kind, 'currency');
  assert.equal(branch(map, CO2).kind, 'currency');
});

test('a promoted cofactor moves on its own, can carry the line, and goes with its reaction', async () => {
  const map = await cyanoMap();
  map.setNodePrimary(CO2, true);
  const rxn = map.reactions.get(RID);
  const co2 = { ...map.nodes.get(CO2) };
  const h2o = { ...map.nodes.get(H2O) };
  map.moveReaction(RID, rxn.x + 50, rxn.y);
  assert.deepEqual([map.nodes.get(CO2).x, map.nodes.get(CO2).y], [co2.x, co2.y]);
  assert.notEqual(map.nodes.get(H2O).x, h2o.x, 'cofactor copies still follow the marker');

  assert.ok(map.setReactionLineEnd(RID, CO2));
  assert.equal(map.reactionLineEnds(RID).from, CO2);

  map.removeReaction(RID);
  assert.equal(map.nodes.has(CO2), false);
});

test('a promoted cofactor stays drawn when cofactors are hidden', async () => {
  const map = await cyanoMap();
  map.setNodePrimary(CO2, true);
  const svg = renderMap(map, { fluxes: new Map(), scale: new FluxScale([]), showCurrency: false, showCrossFeeding: false });
  assert.match(svg, new RegExp(`data-node="${CO2}"`));
  assert.doesNotMatch(svg, new RegExp(`data-node="${H2O}"`));
});

test('the inspector offers Primary on a cofactor copy, and the store makes it one undoable step', async () => {
  const store = new Store('.');
  store.models.set('demo', new Model({
    id: 'demo', compartments: { c: 'Cytosol', e: 'Extracellular' },
    metabolites: [{ id: 'a_e', compartment: 'e' }, { id: 'a_c', compartment: 'c' }, { id: 'co2_c', compartment: 'c' }],
    reactions: [
      { id: 'EX_a_e', metabolites: { a_e: -1 }, lower_bound: -10, upper_bound: 1000 },
      { id: 'At', metabolites: { a_e: -1, a_c: 1 }, lower_bound: 0, upper_bound: 1000 },
      { id: 'FIX', metabolites: { a_c: -1, co2_c: -1 }, lower_bound: 0, upper_bound: 1000, objective_coefficient: 1 },
    ],
  }));
  store.scenarios = [{
    id: 'demo', name: 'Demo', objective: 'equal-growth',
    species: [{ id: 'demo', model: 'demo', abundance: 1 }], medium: { a: 10 },
  }];
  await store.useScenario('demo');
  const id = 'cur:demo@FIX:demo@co2_c';
  assert.equal(store.map.nodes.get(id)?.kind, 'currency');

  store.selectedEntity = { type: 'metabolite', id };
  const html = renderInspector(store);
  assert.match(html, /<button class="" data-action="metabolite-primary"/);
  assert.match(html, /<button class="is-active" data-action="metabolite-secondary"/);
  assert.doesNotMatch(html, /always secondary/);

  store.setMapNodePrimary(id, true);
  assert.equal(store.map.nodes.get(id).kind, 'metabolite');
  store.undoMap();
  assert.equal(store.map.nodes.get(id).kind, 'currency', 'Undo brings the cofactor copy back');
});
