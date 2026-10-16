/**
 * The menus that offer metabolites and reactions to add name them the way the
 * map does. A yeast-GEM model calls ATP `s_0434`; a map labelled by name or by
 * BiGG id offered its next reaction as `r_0534: s_0563 + s_0434 → …`, which
 * nobody reading the map could connect to what is on it.
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { Store } from '../src/ui/store.js';
import { Model } from '../src/core/model.js';
import { renderAddReactionPanel, reactionListing } from '../src/ui/add-reaction-panel.js';
import { BoundaryMenuController } from '../src/ui/boundary-menu.js';

/* A yeast-GEM in miniature: opaque ids, readable names, BiGG annotations. */
const yeastLike = {
  id: 'ymini',
  compartments: { e: 'extracellular', c: 'cytoplasm' },
  metabolites: [
    { id: 's_0563', name: 'D-glucose', compartment: 'e', annotation: { 'bigg.metabolite': 'glc__D' } },
    { id: 's_0565', name: 'D-glucose', compartment: 'c', annotation: { 'bigg.metabolite': 'glc__D' } },
    { id: 's_0434', name: 'ATP', compartment: 'c', annotation: { 'bigg.metabolite': 'atp' } },
    { id: 's_0394', name: 'ADP', compartment: 'c', annotation: { 'bigg.metabolite': 'adp' } },
    { id: 's_0568', name: 'D-glucose 6-phosphate', compartment: 'c', annotation: { 'bigg.metabolite': 'g6p' } },
  ],
  reactions: [
    { id: 'r_1714', name: 'D-glucose exchange', metabolites: { s_0563: -1 }, lower_bound: -10, upper_bound: 1000 },
    { id: 'r_1166', name: 'glucose transport', metabolites: { s_0563: -1, s_0565: 1 }, lower_bound: 0, upper_bound: 1000, annotation: { 'bigg.reaction': 'GLCt1' } },
    { id: 'r_0534', name: 'hexokinase', metabolites: { s_0565: -1, s_0434: -1, s_0568: 1, s_0394: 1 }, lower_bound: 0, upper_bound: 1000, annotation: { 'bigg.reaction': 'HEX1' } },
    { id: 'r_atp', name: 'ATP source', metabolites: { s_0394: -1, s_0434: 1 }, lower_bound: 0, upper_bound: 1000 },
    { id: 'r_bio', name: 'growth', metabolites: { s_0568: -1 }, lower_bound: 0, upper_bound: 1000, objective_coefficient: 1 },
  ],
};

async function yeastStore() {
  const store = new Store('.');
  store.models.set('ymini', new Model(yeastLike));
  store.scenarios = [{
    id: 'ymini', name: 'Yeast in miniature', objective: 'equal-growth',
    species: [{ id: 'sce', model: 'ymini', abundance: 1 }], medium: { s_0563: 10 },
  }];
  await store.useScenario('ymini');
  return store;
}

const merged = (store, kind, localId) => {
  const owners = kind === 'metabolite' ? store.community.metaboliteOwner : store.community.reactionOwner;
  return [...owners].find(([, owner]) => owner.speciesId === 'sce' && owner.originalId === localId)[0];
};

test('a metabolite or reaction not yet drawn is named as the map would name it', async () => {
  const store = await yeastStore();
  for (const source of ['name', 'bigg', 'id']) {
    store.setMapLabelSource('metabolite', source);
    store.setMapLabelSource('reaction', source);
    for (const node of store.map.nodes.values()) {
      if (!node.mergedId || node.labelSource) continue;
      assert.equal(store.entityLabel('metabolite', node.mergedId).label, store.mapLabelText('metabolite', node.id), `${source}: ${node.id}`);
    }
    for (const id of store.map.reactions.keys()) {
      assert.equal(store.entityLabel('reaction', id).label, store.mapLabelText('reaction', id), `${source}: ${id}`);
    }
  }
});

test('the Add Reaction panel lists reactions and their equations in the map’s labels', async () => {
  const store = await yeastStore();
  const hex = merged(store, 'reaction', 'r_0534');
  store.removeMapReaction(hex);

  store.setMapLabelSource('metabolite', 'name');
  let html = renderAddReactionPanel({ store, reactionIds: [hex], selectedReactionId: hex });
  assert.match(html, /<strong>r_0534<\/strong>\s*<span>hexokinase<\/span>/, 'reactions keep their own label setting');
  assert.match(html, /D-glucose \+ ATP → D-glucose 6-phosphate \+ ADP/);
  assert.doesNotMatch(html, /s_0434|s_0565/);
  assert.match(html, /<code>D-glucose 6-phosphate<\/code>/, 'the terminal metabolite too');

  store.setMapLabelSource('reaction', 'name');
  html = renderAddReactionPanel({ store, reactionIds: [hex] });
  assert.match(html, /<strong>hexokinase<\/strong>\s*<span>r_0534<\/span>/, 'labelled by name, the id is what goes beside it');

  store.setMapLabelSource('metabolite', 'bigg');
  store.setMapLabelSource('reaction', 'bigg');
  html = renderAddReactionPanel({ store, reactionIds: [hex] });
  assert.match(html, /<strong>HEX1<\/strong>\s*<span>hexokinase<\/span>/);
  assert.match(html, /glc__D \+ atp → g6p \+ adp/);
});

test('by default the panel still lists the model id with the name beside it', async () => {
  const store = await yeastStore();
  const hex = merged(store, 'reaction', 'r_0534');
  assert.deepEqual(reactionListing(store, hex), { label: 'r_0534', detail: 'hexokinase' });
});

test('Add Metabolite lists the compartment’s metabolites in the map’s labels, searchable by id too', async () => {
  const store = await yeastStore();
  store.setMapLabelSource('metabolite', 'name');
  const listed = store.metabolitesAvailableInCompartment('sce', 'c');
  const atp = listed.find((m) => m.modelId === 's_0434');
  assert.equal(atp.label, 'ATP');
  assert.equal(atp.name, 's_0434', 'beside a name, the model id');

  store.setMapLabelSource('metabolite', 'bigg');
  const g6p = store.metabolitesAvailableInCompartment('sce', 'c').find((m) => m.modelId === 's_0568');
  assert.deepEqual([g6p.label, g6p.name], ['g6p', 'D-glucose 6-phosphate']);
});

test('the reaction library finds a reaction by the label the map shows, and lists it by that label', async () => {
  const store = await yeastStore();
  store.setMapLabelSource('reaction', 'bigg');
  const [hit] = store.searchReactions('HEX1');
  assert.equal(hit.reactionId, merged(store, 'reaction', 'r_0534'));
  assert.equal(hit.label, 'HEX1');
  assert.equal(hit.detail, 'hexokinase');
  assert.equal(store.searchReactions('r_0534')[0].label, 'HEX1', 'the model id still finds it');
});

test('the membrane menu offers transport reactions in the map’s labels', async () => {
  const store = await yeastStore();
  const transport = merged(store, 'reaction', 'r_1166');
  store.removeMapReaction(transport);
  store.setMapLabelSource('reaction', 'name');
  const menu = new BoundaryMenuController({
    stage: { clientWidth: 1200, clientHeight: 800, addEventListener() {} }, store,
  });
  const boundaryId = [...store.map.boundaryIndex.keys()].find((id) => menu.availableFor(id).includes(transport));
  assert.ok(boundaryId, 'the transport reaction is offered on a membrane');
  menu.state = { boundaryId, left: 10, top: 10 };
  assert.match(menu.render(), /<strong>glucose transport<\/strong>\s*<span>r_1166<\/span>/);
});
