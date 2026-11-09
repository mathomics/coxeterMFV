import test from 'node:test';
import assert from 'node:assert/strict';
import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import { EditableMap } from '../src/map/editable-map.js';
import { Store } from '../src/ui/store.js';
import { renderMap } from '../src/render/mapview.js';
import { renderInspector, renderViewControls } from '../src/ui/panels.js';
import { parseMapDocument } from '../src/map/map-document.js';
import { readableBase, readableName } from '../src/core/label-sources.js';
import { FluxScale } from '../src/render/theme.js';
import { readFile } from 'node:fs/promises';

function fixture() {
  const model = new Model({
    id: 'labels',
    compartments: { c: 'Cytosol' },
    metabolites: [
      { id: 'a_c', name: 'Original A', compartment: 'c', annotation: { 'bigg.metabolite': ['a'], 'kegg.compound': ['C00001'], 'metanetx.chemical': ['MNXM1'] } },
      { id: 'b_c', name: 'Original B', compartment: 'c', annotation: { 'bigg.metabolite': ['b'], 'metacyc.compound': ['B-META'], 'seed.compound': ['cpd00002'] } },
    ],
    metNames: ['Alternative A', 'Alternative B'],
    metModelSEEDID: ['cpd00001', 'cpd00002'],
    metFormulas: ['H2O', 'CO2'],
    reactions: [
      { id: 'R', name: 'Original reaction', metabolites: { a_c: -1, b_c: 1 }, lower_bound: 0, upper_bound: 1000, annotation: { 'bigg.reaction': ['RXN-A'], 'kegg.reaction': ['R00001'], 'kegg.pathway': ['path:map00010'] } },
    ],
    rxnNames: ['Alternative reaction'],
    rxnMetaNetXID: ['MNXR1'],
    rxnMetaCycID: ['RXN-META'],
    rxnModelSEEDID: ['rxn00001'],
  });
  const community = new CommunityModel({
    species: [{ id: 'sp', model, abundance: 1 }],
    medium: {},
  });
  const map = EditableMap.emptyFromCommunity(community);
  map.addReaction(community, 'sp@R');
  const store = new Store('.');
  store.community = community;
  store.map = map;
  store.species = [{ id: 'sp', file: 'labels' }];
  store.models.set('labels', model);
  const node = [...map.nodes.values()].find(item => item.mergedId === 'sp@a_c');
  const other = [...map.nodes.values()].find(item => item.mergedId === 'sp@b_c');
  return { model, community, map, store, node, other };
}

test('model vectors and database annotations survive model and community round trips', () => {
  const { model, community } = fixture();
  const restored = new Model(model.toJSON());
  assert.deepEqual(restored.metabolites.get('a_c').labelSources.name, ['Alternative A']);
  assert.deepEqual(restored.metabolites.get('a_c').labelSources.modelseed, ['cpd00001']);
  assert.deepEqual(restored.reactions.get('R').labelSources.name, ['Alternative reaction']);
  assert.deepEqual(community.merged.reactions.get('sp@R').labelSources.metacyc, ['RXN-META']);
  assert.deepEqual(community.merged.metabolites.get('sp@a_c').labelSources.kegg, ['C00001']);
  assert.equal(model.reactions.get('R').labelSources.kegg.includes('path:map00010'), false);
  assert.equal(Object.hasOwn(model.metabolites.get('a_c').labelSources, 'metFormulas'), false);
});

test('global label choices allow per-instance overrides and fall back for missing annotations', () => {
  const { store, node, other, model, community } = fixture();
  const biology = JSON.stringify([model.toJSON(), community.merged.toJSON()]);
  assert.equal(store.mapLabelText('metabolite', node.id), 'a');
  store.setMapLabelSource('metabolite', 'kegg');
  assert.equal(store.mapLabelText('metabolite', node.id), 'C00001');
  assert.equal(store.mapLabelText('metabolite', other.id), 'b');
  store.setMapLabelSource('metabolite', 'name', node.id);
  assert.equal(store.mapLabelText('metabolite', node.id), 'Alternative A');
  store.setMapLabelSource('metabolite', 'inherit', node.id);
  assert.equal(store.mapLabelText('metabolite', node.id), 'C00001');
  store.setMapLabelSource('reaction', 'metanetx');
  assert.equal(store.mapLabelText('reaction', 'sp@R'), 'MNXR1');
  store.setMapLabelSource('reaction', 'name', 'sp@R');
  assert.equal(store.mapLabelText('reaction', 'sp@R'), 'Alternative reaction');
  assert.equal(JSON.stringify([model.toJSON(), community.merged.toJSON()]), biology);
});

test('global and individual label choices persist in map JSON and undo independently', () => {
  const { store, node } = fixture();
  store.setMapLabelSource('metabolite', 'bigg');
  store.setMapLabelSource('metabolite', 'name', node.id);
  store.setMapLabelSource('reaction', 'modelseed', 'sp@R');
  const parsed = parseMapDocument(store.mapJSON());
  const restored = EditableMap.fromJSON(parsed.map);
  assert.equal(restored.labelSettings.metabolite, 'bigg');
  assert.equal(restored.nodes.get(node.id).labelSource, 'name');
  assert.equal(restored.reactions.get('sp@R').labelSource, 'modelseed');
  store.undoMap();
  assert.equal(store.map.reactions.get('sp@R').labelSource, undefined);
  assert.equal(store.mapLabelText('metabolite', node.id), 'Alternative A');
  store.redoMap();
  assert.equal(store.mapLabelText('reaction', 'sp@R'), 'rxn00001');
});

test('rendered labels use the selected sources and retain flux and FVA values', () => {
  const { store, map, node } = fixture();
  store.setMapLabelSource('metabolite', 'name');
  store.setMapLabelSource('reaction', 'kegg');
  const state = {
    scale: { reference: 1, color: () => '#94a3b8', width: () => 1 },
    labels: 'all',
    fluxes: new Map([['sp@R', 2]]),
    metaboliteLabels: store.mapLabels('metabolite'),
    reactionLabels: store.mapLabels('reaction'),
  };
  assert.equal(state.metaboliteLabels.get(node.id), 'Alternative A');
  assert.match(renderMap(map, state), />Alternative A<\/text>/);
  assert.match(renderMap(map, state), />R00001  2<\/text>/);
  const fva = renderMap(map, {
    ...state,
    fvaEnabled: true,
    fvaRanges: new Map([['sp@R', { min: 1, max: 3 }]]),
  });
  assert.match(fva, />R00001  \[1, 3\]<\/text>/);
});

test('View and both inspectors offer the available sources with an inherit option', () => {
  const { store, node } = fixture();
  const view = renderViewControls(store);
  assert.match(view, /data-action="metabolite-label-source"/);
  assert.match(view, /data-action="reaction-label-source"/);
  assert.match(view, /value="modelseed">ModelSEED/);
  store.selectEntity('metabolite', node.id);
  const metabolite = renderInspector(store);
  assert.match(metabolite, /Follow View setting/);
  assert.match(metabolite, /Model name: Alternative A/);
  store.select('sp@R');
  const reaction = renderInspector(store);
  assert.match(reaction, /data-action="reaction-label-source" data-rxn="sp@R"/);
  assert.match(reaction, /MetaNetX: MNXR1/);
});

test('cofactors and their labels start enabled while cross-feeding starts disabled', () => {
  const store = new Store('.');
  assert.equal(store.view.showCurrency, true);
  assert.equal(store.view.currencyLabels, true);
  assert.equal(store.view.showCrossFeeding, false);
});

test('a base label reads as a name where the model id does not', () => {
  // ModelSEED accessions give way to the model's name
  assert.equal(readableBase('cpd00023[c_Er]', 'L-Glutamate'), 'L-Glutamate');
  assert.equal(readableBase('cpd00007[e_Lc]', 'O2'), 'O2');
  // ... unless the name is the accession again
  assert.equal(readableBase('cpd00023[c_Er]', 'cpd00023'), 'cpd00023[c_Er]');
  // SBML-escaped ids lose the prefix and the compartment letter the bracket repeats
  assert.equal(readableBase('M_ival_c[c_Ef]', 'ival_c[c]'), 'ival[c_Ef]');
  assert.equal(readableBase('M_glc__D_e[e_Ef]'), 'glc__D[e_Ef]');
  // plain BiGG bases are left alone
  assert.equal(readableBase('pro-L[c_St]', 'L-Proline'), 'pro-L[c_St]');
  assert.equal(readableBase('ac[e_Ef]', 'Acetate'), 'ac[e_Ef]');
});

test('a name that lists synonyms reads as one of them', () => {
  assert.equal(readableName('CELB|Cellobiose|cellobiose', ['cellb[u]']), 'Cellobiose');
  assert.equal(readableName('6G-alpha-D-galactosylsucrose|M_Raffinose_C18H32O16|Raffinose', ['raffin[u]']), 'Raffinose');
  assert.equal(readableName('Exchange for Serine|L-Serine', ['ser-L[u]']), 'L-Serine');
  assert.equal(readableName('Hydrogen sulfide|hydrogen sulfide', ['h2s']), 'Hydrogen sulfide');
  assert.equal(readableName('Fe3+|fe3', ['fe3[u]']), 'Fe3+');
  assert.equal(readableName('K+|potassium', ['k']), 'K+');
  assert.equal(readableName('1,5-Diaminopentane|cadaverine (penta-1,5-diamine)'), '1,5-Diaminopentane');
  // one name stays as it is, an empty one falls back to the id
  assert.equal(readableName('D-Glucose', ['glc__D']), 'D-Glucose');
  assert.equal(readableName('', ['glc__D']), 'glc__D');
  // a ModelSEED base takes the readable name too
  assert.equal(readableBase('cpd00222[c_Lc]', 'GLCN|Gluconic Acid'), 'Gluconic Acid');
});

test('cross-feeding lines and the picked line name the metabolite once', async () => {
  const doc = JSON.parse(await readFile(new URL('../models/published/steadycom-gut-nine/map.json', import.meta.url), 'utf8'));
  const layout = EditableMap.fromJSON(doc.map);
  const link = { from: 'Bt', base: 'ser-L', to: 'Lc', flux: 0.5 };
  const svg = renderMap(layout, {
    fluxes: new Map(), scale: new FluxScale([]), showCrossFeeding: true,
    crossFeeding: { links: [link], perMetabolite: [] },
    crossFeedingExplorer: { enabled: true, visibleEvents: [link], selectedEvent: link, selectedAt: null },
  });
  const title = /<title>([^<]*ser-L[^<]*)<\/title>/.exec(svg)?.[1] ?? '';
  assert.match(title, /^L-Serine \(ser-L\) · Bt → Lc/);
  assert.match(svg, /emc-crossfeed-tag[\s\S]*L-Serine \(ser-L\) · Bt → Lc/);
  assert.doesNotMatch(svg, /Exchange for Serine/);
});
