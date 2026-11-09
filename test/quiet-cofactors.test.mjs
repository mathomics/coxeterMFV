/**
 * Quiet cofactors: a map that sets cofactorStyle 'quiet' draws its secondary
 * metabolites smaller and lighter. The setting belongs to the map: it is saved
 * with it, undone like an edit, and maps that do not set it look as before.
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { renderMap, QUIET_COFACTORS } from '../src/render/mapview.js';
import { EditableMap } from '../src/map/editable-map.js';
import { Store } from '../src/ui/store.js';
import { Model } from '../src/core/model.js';
import { FluxScale } from '../src/render/theme.js';

const model = {
  id: 'quiet',
  compartments: { e: 'extracellular', c: 'cytoplasm' },
  metabolites: [
    { id: 'glc_e', compartment: 'e' }, { id: 'glc_c', compartment: 'c' }, { id: 'g6p_c', compartment: 'c' },
    { id: 'atp_c', compartment: 'c' }, { id: 'adp_c', compartment: 'c' },
  ],
  reactions: [
    { id: 'EX_glc_e', metabolites: { glc_e: -1 }, lower_bound: -10, upper_bound: 1000 },
    { id: 'GLCt', metabolites: { glc_e: -1, glc_c: 1 }, lower_bound: 0, upper_bound: 1000 },
    { id: 'HEX', metabolites: { glc_c: -1, atp_c: -1, g6p_c: 1, adp_c: 1 }, lower_bound: 0, upper_bound: 1000 },
    { id: 'ATPS', metabolites: { adp_c: -1, atp_c: 1 }, lower_bound: 0, upper_bound: 1000 },
    { id: 'BIO', metabolites: { g6p_c: -1 }, lower_bound: 0, upper_bound: 1000, objective_coefficient: 1 },
  ],
};

async function quietStore() {
  const store = new Store('.');
  store.models.set('quiet', new Model(model));
  store.scenarios = [{ id: 'quiet', name: 'Quiet', objective: 'equal-growth', species: [{ id: 'sp', model: 'quiet', abundance: 1 }], medium: { glc: 10 } }];
  await store.useScenario('quiet');
  return store;
}

const draw = (map) => renderMap(map, { fluxes: new Map(), scale: new FluxScale([]) });
const circleOf = (svg, nodeId) => svg.match(new RegExp(`<circle class="emc-node[^"]*" data-node="${nodeId.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}"[^>]*>`))?.[0];
const widthOf = (tag) => Number(/stroke-width="([\d.]+)"/.exec(tag)[1]);

test('a quiet map draws its cofactors smaller and their branches thinner, and nothing else differs', async () => {
  const store = await quietStore();
  const secondary = [...store.map.nodes.values()].find((n) => n.kind === 'currency');
  const primary = [...store.map.nodes.values()].find((n) => n.mergedId === 'sp@g6p_c');
  assert.ok(secondary && primary, 'the layout draws ATP and ADP as cofactors');

  const full = draw(store.map);
  store.map.setCofactorStyle('quiet');
  const quiet = draw(store.map);

  assert.doesNotMatch(full, /emc-quiet-cofactors/);
  assert.match(quiet, /class="emc-map emc-quiet-cofactors"/, 'the stylesheet lightens what the class marks');
  assert.match(circleOf(full, secondary.id), /r="7"/);
  assert.match(circleOf(quiet, secondary.id), new RegExp(`r="${QUIET_COFACTORS.radius}"`));
  assert.equal(circleOf(quiet, primary.id), circleOf(full, primary.id));

  const branches = (svg) => [...svg.matchAll(/<path class="emc-seg emc-seg-currency[^"]*"[^>]*>/g)].map((m) => m[0]);
  assert.ok(branches(quiet).length > 0);
  for (const tag of branches(quiet)) {
    assert.match(tag, /is-cofactor/);
    assert.equal(widthOf(tag), 7.5 * QUIET_COFACTORS.branchWidth);
  }
  for (const tag of branches(full)) assert.equal(widthOf(tag), 7.5);
});

test('quiet cofactors travel with the map, undo like an edit and stay out of maps that do not set them', async () => {
  const store = await quietStore();
  assert.equal(store.map.cofactorStyle, null);
  assert.equal('cofactorStyle' in store.map.toJSON(), false);

  store.setMapCofactorStyle('quiet');
  assert.equal(store.map.cofactorStyle, 'quiet');
  const saved = store.mapJSON();
  assert.equal(JSON.parse(saved).map.cofactorStyle, 'quiet');

  store.undoMap();
  assert.equal(store.map.cofactorStyle, null, 'View ▸ Quiet cofactors is undone like any map edit');
  store.loadMapJSON(saved);
  assert.equal(store.map.cofactorStyle, 'quiet', 'and comes back with the file');
  store.setMapCofactorStyle(null);
  assert.equal(store.map.cofactorStyle, null, 'switched back to full size');

  assert.equal(EditableMap.fromJSON({ ...store.map.toJSON(), cofactorStyle: 'loud' }).cofactorStyle, null, 'an unknown style is the default');
});
