// Reference outputs of the JS Store (src/ui/store.js) for the parity tests of
// rust/crates/coxeter-state.
//
//   node rust/tools/fixtures-app.mjs          small fixtures (committed)
//   node rust/tools/fixtures-app.mjs --full   also every map and render state
//                                             in full, for debugging a digest
//                                             mismatch (rust/fixtures/large/app)
//
// Each case opens a scenario (or nothing) and runs a list of steps: store
// actions named as the JS methods, with JSON arguments. After every step a
// snapshot of the store is written: the Scenario file (stateJSON), digests of
// the map and of the deterministic render state, the selection, the undo
// labels, the status line, unsaved work and the objective value. The steps are
// written into the fixture, so the Rust test replays exactly these.
//
// Maps and labels are compared through the SHA-256 of their canonical JSON
// (src/core/reproducibility.js), which coxeter-io reproduces bit for bit. This
// keeps the fixtures small. A map is digested in the form every reader of a
// map sees (the rule of coxeter-map's parity tests): a key holding null is
// the same as an absent one, and `backboneAnchor` / `transportLane` are only
// ever written as true. Fluxes are not compared: GLPK and HiGHS may return
// different vertices of the same optimal face (see coxeter-solver's parity
// tests). The objective value is unique and is compared to 1e-6.
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

globalThis.crypto ??= (await import('node:crypto')).webcrypto;

const root = join(dirname(fileURLToPath(import.meta.url)), '../..');
const out = join(root, 'rust/fixtures/app');
const outLarge = join(root, 'rust/fixtures/large/app');
const full = process.argv.includes('--full');
await mkdir(out, { recursive: true });
if (full) await mkdir(outLarge, { recursive: true });

// Windows needs file:// URLs for dynamic imports of absolute paths.
const load = (path) => import(pathToFileURL(join(root, path)).href);
const factory = (await load('assets/vendor/glpk.js')).default;
globalThis.__GLPK_FACTORY__ = factory;
globalThis.__GLPK_WASM__ = await readFile(join(root, 'assets/vendor/glpk.wasm'));

// The Store loads its bundled files with fetch, so the stub answers from the repository.
const BASE = 'http://fixture.invalid';
globalThis.fetch = async (url) => {
  const path = new URL(url, `${BASE}/`).pathname.replace(/^\//, '');
  try {
    const text = await readFile(join(root, path), 'utf8');
    return { ok: true, status: 200, json: async () => JSON.parse(text), text: async () => text };
  } catch {
    return { ok: false, status: 404, json: async () => null, text: async () => '' };
  }
};

const { Store } = await load('src/ui/store.js');
const { sha256 } = await load('src/core/reproducibility.js');

const obj = (m) => Object.fromEntries(m ?? []);

/** A map's JSON with null (and the two false-only flags) left out. */
function canonicalMap(v) {
  if (Array.isArray(v)) return v.map(canonicalMap);
  if (v && typeof v === 'object') {
    const out = {};
    for (const [k, x] of Object.entries(v)) {
      if (x === null || x === undefined) continue;
      if ((k === 'backboneAnchor' || k === 'transportLane') && x === false) continue;
      out[k] = canonicalMap(x);
    }
    return out;
  }
  return v;
}

/** The parts of mapRenderState() that do not depend on the fluxes. */
function renderProjection(store) {
  const reactionColoring = store.reactionColoring();
  const ribbon = store.pathwayRibbonColoring();
  return {
    reactionColors: obj(reactionColoring.colors),
    pathwayRibbonColors: obj(ribbon.colors),
    pathwayRibbonGroups: obj(ribbon.groups),
    metaboliteLabels: obj(store.mapLabels('metabolite')),
    reactionLabels: obj(store.mapLabels('reaction')),
  };
}

async function snapshot(store, harness) {
  const state = JSON.parse(store.stateJSON());
  const map = store.map ? store.map.toJSON() : null;
  const render = renderProjection(store);
  const sel = store.multiSelection;
  const snap = {
    state,
    map: map ? {
      digest: await sha256(canonicalMap(map)),
      cells: map.cells?.length ?? 0,
      nodes: map.nodes.length,
      reactions: map.reactions.length,
      segments: map.segments.length,
      texts: (map.texts ?? []).length,
      width: map.canvas?.width ?? null,
      height: map.canvas?.height ?? null,
    } : null,
    render: {
      digest: await sha256(render),
      reactionColors: Object.keys(render.reactionColors).length,
      ribbonGroups: Object.keys(render.pathwayRibbonGroups).length,
      metaboliteLabels: Object.keys(render.metaboliteLabels).length,
      inactiveSpeciesIds: store.disabledDrawnSpeciesIds(),
      currencyLabels: store.view.currencyLabels || store.mapMode === 'core-overview',
      showCurrency: store.view.showCurrency,
      showCrossFeeding: store.view.showCrossFeeding,
    },
    mapMode: store.mapMode,
    mapLoadNote: store.mapLoadNote,
    status: { state: store.status.state, message: store.status.message },
    unsaved: store.unsavedWork(),
    undoLabels: store.mapHistory.undoStack.map((e) => e.label),
    redoCount: store.mapHistory.redoStack.length,
    selectedReaction: store.selectedReaction ?? null,
    selectedEntity: store.selectedEntity ?? null,
    selectedGeometry: store.selectedGeometry ?? null,
    multiSelection: { nodes: [...sel.nodes], reactions: [...sel.reactions], texts: [...sel.texts] },
    community: store.community?.merged ? {
      reactions: store.community.merged.reactions.size,
      metabolites: store.community.merged.metabolites.size,
      active: store.community.activeSpecies.map((sp) => sp.id),
    } : null,
    solution: store.solution ? { status: store.solution.status, objectiveValue: store.solution.objectiveValue } : null,
    availableCells: store.availableMapCells().map((c) => c.speciesId),
    availableCompartments: store.availableMapCompartments().map((c) => `${c.speciesId}|${c.compartmentId}|${c.parentId}`),
    pathwayEntries: store.pathwayManagerEntries().map((e) => ({ key: e.key, name: e.name, color: e.color, visible: e.visible, count: e.count })),
    exportBaseName: store.exportBaseName(),
  };
  if (harness.search) snap.search = store.searchReactions(harness.search).map((r) => [r.reactionId, r.score, r.drawn]);
  if (harness.analysis) snap.analysis = analysisProjection(store);
  if (full) snap.full = { map, render };
  return snap;
}

/** What the omics overlay, community omics and the explorer hold. */
function analysisProjection(store) {
  const overlay = store.omicsOverlay();
  const o = store.omics;
  const report = o.report;
  return {
    overlay: {
      enabled: overlay.enabled, matchedReactions: overlay.matchedReactions, totalDrawn: overlay.totalDrawn,
      mode: overlay.mode, metric: overlay.metric, conditionA: overlay.conditionA, conditionB: overlay.conditionB,
      values: obj(overlay.values), colors: obj(overlay.colors),
    },
    communityOmics: {
      enabled: o.enabled, mode: o.mode, conditionA: o.conditionA, conditionB: o.conditionB, metric: o.metric,
      scores: obj(o.scores), colors: obj(o.colors), gimmeEnabled: o.gimmeEnabled,
      report: report ? {
        inputRows: report.inputRows, inputSpecies: report.inputSpecies, modelSpecies: report.modelSpecies,
        genesInModels: report.genesInModels, measuredGenes: report.measuredGenes, gprReactions: report.gprReactions,
        mappedReactions: report.mappedReactions, unmatchedSpecies: report.unmatchedSpecies,
      } : null,
    },
    explorer: { enabled: store.crossFeedingExplorer.enabled, indexed: Boolean(store.crossFeedingExplorer.index) },
  };
}

/**
 * A bundled model with synthetic gene rules (the bundled core models have
 * none), so the omics functions have something to read. The Rust test makes
 * the same rules: reaction i (exchanges excepted) gets G(i mod 23), joined
 * with a second gene by `and` when i is a multiple of 5 and by `or` when it is
 * a multiple of 7.
 */
async function modelWithGeneRules(path, id) {
  const doc = JSON.parse(await readFile(join(root, path), 'utf8'));
  doc.id = id;
  doc.reactions.forEach((r, i) => {
    if (r.id.startsWith('EX_')) return;
    const g = (k) => `G${k % 23}`;
    r.gene_reaction_rule = i % 5 === 0 ? `(${g(i)} and ${g(i + 7)})` : i % 7 === 0 ? `${g(i)} or ${g(i + 3)}` : g(i);
  });
  return JSON.stringify(doc);
}

/** Run one step: [method, ...args]. */
async function step(store, harness, [method, ...args]) {
  switch (method) {
    case 'moveNodeTx': {
      const tx = store.beginMapEdit('Move metabolite');
      store.moveNode(args[0], args[1], args[2]);
      return store.commitMapEdit(tx);
    }
    case 'moveReactionTx': {
      const tx = store.beginMapEdit('Move reaction');
      store.moveReaction(args[0], args[1], args[2]);
      return store.commitMapEdit(tx);
    }
    case 'rotateReactionTx': {
      const tx = store.beginMapEdit('Rotate reaction');
      store.setReactionAngle(args[0], args[1]);
      return store.commitMapEdit(tx);
    }
    case 'moveTextTx': {
      const tx = store.beginMapEdit('Move text');
      store.moveMapText(args[0], args[1], args[2]);
      return store.commitMapEdit(tx);
    }
    case 'saveState': harness.state = store.stateJSON(); return null;
    case 'loadSavedState': return store.loadState(harness.state);
    case 'saveMap': harness.map = store.mapJSON(); return null;
    case 'loadSavedMap': return store.loadMapJSON(harness.map);
    case 'loadSavedMapDocument': return store.loadMapDocument(harness.map);
    case 'importModelFiles': {
      const files = [];
      for (const f of args[0]) files.push({ name: f.name, text: await readFile(join(root, f.path), 'utf8') });
      return store.importModelFiles(files);
    }
    case 'importModelsWithGeneRules': {
      const files = [];
      for (const f of args[0]) files.push({ name: f.name, text: await modelWithGeneRules(f.path, f.id) });
      return store.importModelFiles(files);
    }
    case 'importOmicsCSV': return store.importOmicsCSV(args[0], args[1] ?? {});
    case 'importMetatranscriptomics': return store.importMetatranscriptomics(args[0], args[1]);
    case 'selectInRect': return store.selectInRect(args[0]);
    case 'createManualPathway': return store.createManualPathway(args[0]);
    case 'createModelReaction': return store.createModelReaction(args[0], { addToMap: args[1] });
    case 'addMapReaction': return store.addMapReaction(args[0], args[1] ?? {});
    case 'duplicateMapMetabolite': return store.duplicateMapMetabolite(args[0], args[1] ?? {});
    case 'setMapCanvasSize': return store.setMapCanvasSize(args[0], args[1], {});
    case 'showCoreMetabolismOverview': return store.showCoreMetabolismOverview({ recordHistory: true });
    case 'select': return store.select(args[0]);
    default:
      if (typeof store[method] !== 'function') throw new Error(`unknown step ${method}`);
      return store[method](...args);
  }
}

async function runCase(spec) {
  const store = new Store(BASE);
  await store.init();
  const harness = { search: spec.search ?? null, analysis: spec.analysis ?? false };
  const snapshots = [];
  if (spec.scenario) {
    await store.useScenario(spec.scenario);
    snapshots.push(await snapshot(store, harness));
  }
  const steps = typeof spec.steps === 'function' ? spec.steps(store) : spec.steps;
  for (const s of steps) {
    try {
      await step(store, harness, s);
      snapshots.push(await snapshot(store, harness));
    } catch (error) {
      snapshots.push({ error: error.message });
    }
  }
  return { name: spec.name, scenario: spec.scenario ?? null, search: spec.search ?? null, analysis: spec.analysis ?? false, steps, snapshots };
}

// Cases

const firstNode = (store, cell, test = () => true) =>
  [...store.map.nodes.values()].find((n) => n.cellId === cell && n.kind === 'metabolite' && test(n))?.id;

const cases = [];

// Every bundled scenario, opened.
for (const id of ['consorcio-anaerobio', 'aerobio-mixto', 'fototrofo-heterotrofo', 'metanogenesis', 'todas', 'solo-ecoli', 'solo-levadura']) {
  cases.push({ name: `open-${id}`, scenario: id, steps: [], search: id === 'aerobio-mixto' ? 'pgi' : null });
}

cases.push({
  name: 'simulation',
  scenario: 'aerobio-mixto',
  steps: [
    ['setMediumLimit', 'glc__D', 5],
    ['setMediumLimit', 'o2', ''],
    ['setMediumLimit', 'o2', '-'],
    ['setReactionBounds', 'eco@PGI', 0, 0],
    ['solve'],
    ['setReactionBounds', 'eco@PGI', null, '5'],
    ['setReactionBounds', 'eco@PFK', '', null],
    ['resetReaction', 'eco@PGI'],
    ['knockout', 'sce@HEX1'],
    ['setParsimonious', true],
    ['setPFBABiomassPercent', 90],
    ['setPFBABiomassPercent', ''],
    ['setLoopless', true],
    ['setParsimonious', true],
    ['setLoopless', false],
    ['setObjective', 'sum-biomass', null],
    ['setAbundance', 'eco', 0.7],
    ['setAbundance', 'eco', ''],
    ['setSpeciesEnabled', 'sce', false],
    ['setSpeciesEnabled', 'sce', true],
    ['resetAllReactions'],
    ['saveState'],
    ['useScenario', 'solo-ecoli'],
    ['loadSavedState'],
    ['markScenarioSaved'],
    ['setView', { reactionColorMode: 'subsystem' }],
    ['setView', { pathwayRibbonMode: 'subsystem', pathwayRibbonWidth: 500, pathwayRibbonOpacity: 0 }],
    ['setFluxColorStops', [{ value: 5, color: '#0000FF' }, { value: -20, color: '#ff0000' }]],
    ['setCrossFeedingTop', 2.6],
  ],
});

cases.push({
  name: 'map-edits',
  scenario: 'solo-ecoli',
  steps: (store) => {
    const g6p = firstNode(store, 'eco', (n) => n.mergedId === 'eco@g6p_c') ?? firstNode(store, 'eco');
    const pyr = firstNode(store, 'eco', (n) => n.mergedId === 'eco@pyr_c') ?? firstNode(store, 'eco');
    const node = store.map.nodes.get(g6p);
    const rxn = store.map.reactions.get('eco@PGI');
    return [
      ['addMapText', 'Glycolysis', 120, 90],
      ['moveTextTx', 'text-1', 160, 110],
      ['moveNodeTx', g6p, node.x + 30, node.y - 20],
      ['moveReactionTx', 'eco@PGI', rxn.x + 15, rxn.y + 10],
      ['rotateReactionTx', 'eco@PFK', 0.6],
      ['setMapNodePrimary', pyr, false],
      ['setMapGeometryColors', 'eco', null, { fill: '#12AB34', stroke: '#aa0000' }],
      ['setMapGeometryColors', 'eco', 'c', { fill: 'red' }],
      ['removeMapReaction', 'eco@TALA'],
      ['undoMap'],
      ['redoMap'],
      ['setMapLabelSource', 'metabolite', 'name', null],
      ['setMapLabelSource', 'reaction', 'name', 'eco@PGI'],
      ['setMapLabelSource', 'reaction', 'inherit', 'eco@PGI'],
      ['select', 'eco@PGI'],
      ['createManualPathway', { name: 'Upper glycolysis', color: '#123456', reactionIds: ['eco@PGI', 'eco@PFK', 'eco@FBA'] }],
      ['createManualPathway', { name: 'Upper glycolysis', reactionIds: ['eco@FBA', 'eco@TPI'] }],
      ['setPathwayColor', 'model:Glycolysis/Gluconeogenesis', '#112233'],
      ['setPathwayVisible', 'model:Pentose Phosphate Pathway', false],
      ['setView', { reactionColorMode: 'pathway' }],
      ['selectInRect', { x1: node.x - 150, y1: node.y - 150, x2: node.x + 150, y2: node.y + 150 }],
      ['arrangeMultiSelection', 'align-vertical'],
      ['arrangeMultiSelection', 'distribute-horizontal'],
      ['toggleMultiSelection', 'text', 'text-1'],
      ['removeMultiSelection'],
      ['undoMap'],
      ['clearMultiSelection'],
      ['rotateMapReaction', 'eco@ENO', 0.5],
      ['setMapReactionLinear', 'eco@GAPD', true],
      ['flipMapReaction', 'eco@PGK'],
      ['mirrorMapReaction', 'eco@PGM'],
      ['tidyMapReaction', 'eco@PGM'],
      ['alignMapReaction', 'eco@PYK', 'E'],
      ['alignMapReactionAxis', 'eco@ENO', 'vertical'],
      ['duplicateMapMetabolite', g6p],
      ['setMapNodeHidden', pyr, true],
      ['showAllHiddenMetabolites'],
      ['saveMap'],
      ['markMapSaved'],
      ['showEmptyMap'],
      ['loadSavedMap'],
      ['undoMap'],
      ['loadSavedMapDocument'],
    ];
  },
});

cases.push({
  name: 'layout',
  scenario: 'metanogenesis',
  steps: [
    ['setLayoutPreset', 'compact'],
    ['setLayoutSettings', { nodeSpacing: 1.2 }],
    ['removeMapReaction', 'eco@PGI'],
    ['relayoutMap'],
    ['showEmptyMap'],
    ['showFullMap'],
    ['showCoreMetabolismOverview'],
    ['setLayoutPreset', 'normal'],
    ['undoMap'],
    ['undoMap'],
  ],
});

cases.push({
  name: 'cell-tool',
  scenario: 'solo-levadura',
  steps: [
    ['showEmptyMap'],
    ['addMapCell', 'sce', { x: 80, y: 70, w: 900, h: 650 }],
    ['addMapCompartment', 'sce', 'c', { x: 130, y: 120, w: 800, h: 540 }],
    ['addMapCompartment', 'sce', 'im', { x: 250, y: 220, w: 360, h: 260 }],
    ['addMapCompartment', 'sce', 'm', { x: 285, y: 255, w: 285, h: 190 }],
    ['addMapCompartment', 'sce', 'm', { x: 285, y: 255, w: 285, h: 190 }],
    ['addMapReaction', 'sce@HEX1', { anchor: { x: 400, y: 600 } }],
    ['addMapReaction', 'sce@PYRt2', {}],
    ['setMapCanvasSize', 1800, 1200],
    ['undoMap'],
  ],
});

cases.push({
  name: 'species-toggle',
  scenario: 'consorcio-anaerobio',
  steps: (store) => {
    const node = firstNode(store, 'eco');
    return [
      ['setReactionBounds', 'eco@PGI', null, '7'],
      ['setSpeciesEnabled', 'mba', false],
      ['setSpeciesEnabled', 'mba', true],
      ['setMapNodePrimary', node, false],
      ['setSpeciesEnabled', 'lla', false],
      ['saveMap'],
      ['showEmptyMap'],
      ['loadSavedMap'],
      ['setSpeciesEnabled', 'lla', true],
      ['setSpeciesEnabled', 'eco', false],
      ['setSpeciesEnabled', 'sce', false],
      ['setSpeciesEnabled', 'mba', false],
    ];
  },
});

cases.push({
  name: 'import',
  steps: [
    ['importModelFiles', [{ name: 'my-ecoli.json', path: 'models/ecoli_core_mc.json' }]],
    ['saveMap'],
    ['importModelFiles', [{ name: 'my-yeast.json', path: 'models/yeast_core_mc.json' }, { name: 'my-ecoli.json', path: 'models/ecoli_core_mc.json' }]],
    ['useScenario', 'solo-ecoli'],
    ['loadSavedMapDocument'],
  ],
});

cases.push({
  name: 'model-edit',
  scenario: 'solo-ecoli',
  steps: [
    ['createModelMetabolite', { id: 'xyl_c', name: 'Xylose', compartment: 'c', formula: 'C5H10O5' }],
    ['createModelReaction', { id: 'XYLt', name: 'Xylose sink', metabolites: 'xyl_c:-1', lowerBound: 0, upperBound: 10 }, true],
    ['markModelSaved'],
    ['setModelEditorSpecies', 'eco'],
  ],
});

cases.push({
  name: 'published-stolyar',
  steps: [
    ['usePublishedCommunity', 'stolyar-2007-syntrophy'],
    ['setMediumLimit', 'lac__L', 5],
    ['saveMap'],
    ['useScenario', 'solo-ecoli'],
    ['loadSavedMapDocument'],
  ],
});

// Omics overlay, community omics, GIMME and the explorer (coxeter-analysis).
const genes = Array.from({ length: 23 }, (_, i) => `G${i}`);
const omicsCSV = ['gene,A,B', ...genes.map((g, i) => `${g},${(i * 7) % 13 + 1},${(i * 5) % 11 + 0.5}`)].join('\n');
const metaCSV = ['MAG,gene,condition,TPM',
  ...['EcoG', 'SceG'].flatMap((sp, s) => genes.flatMap((g, i) => [
    `${sp},${g},A,${(i * 3 + s) % 17 + 2}`, `${sp},${g},B,${(i * 11 + 2 * s) % 19 + 1}`,
  ])),
  'Other,G1,A,4'].join('\n');
cases.push({
  name: 'analysis',
  analysis: true,
  steps: [
    ['importModelsWithGeneRules', [
      { name: 'eco-gpr.json', path: 'models/ecoli_core_mc.json', id: 'EcoG' },
      { name: 'sce-gpr.json', path: 'models/yeast_core_mc.json', id: 'SceG' },
    ]],
    ['importOmicsCSV', omicsCSV, { filename: 'omics.csv', dataType: 'gene' }],
    ['setOmicsSettings', { metric: 'difference' }],
    ['setOmicsSettings', { mode: 'single', conditionA: 'B' }],
    ['importMetatranscriptomics', metaCSV, 'meta.csv'],
    ['setCommunityOmics', { mode: 'compare', conditionB: 'B' }],
    ['setCommunityOmics', { mode: 'compare', metric: 'difference' }],
    ['setCrossFeedingExplorer', { enabled: true }],
    ['runCommunityGIMME', {}],
    ['saveState'],
    ['disableGIMME'],
    ['loadSavedState'],
  ],
});

const index = [];
for (const spec of cases) {
  const result = await runCase(spec);
  const name = `case-${spec.name}.json`;
  if (full) {
    await writeFile(join(outLarge, name), JSON.stringify(result));
    for (const s of result.snapshots) delete s.full;
  }
  await writeFile(join(out, name), JSON.stringify(result));
  index.push(name);
  console.log(`${name}: ${result.snapshots.length} snapshots${result.snapshots.some((s) => s.error) ? ' (some steps threw)' : ''}`);
}
await writeFile(join(out, 'index.json'), JSON.stringify({ cases: index }));
