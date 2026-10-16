// Reference outputs of src/render for the Rust port's parity tests.
//
//   node rust/tools/fixtures-render.mjs
//
// Writes rust/fixtures/render/ (small, committed) and
// rust/fixtures/large/render/ (every bundled scenario and a gut9 snapshot,
// git-ignored):
//
//   units.json        theme, reaction colours, reaction paths, raster sizes,
//                     cross-feeding routes
//   cases.json        render cases: { name, map, state, svg }
//   maps/<name>.json  the live map each case draws (EditableMap#toJSON())
//   svg/<case>.svg    what renderMap(map, state) returned
//
// A state is written as JSON (Maps as objects, Sets as arrays, the flux scale
// as the values it is built from) and rebuilt by toState() below, so the
// renderer here draws exactly what the Rust test rebuilds from the same file.
globalThis.crypto ??= (await import('node:crypto')).webcrypto;
import { readFile, writeFile, mkdir, rm } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '../..');
const small = join(root, 'rust/fixtures/render');
const large = join(root, 'rust/fixtures/large/render');
const imp = (p) => import(pathToFileURL(join(root, p)).href);

const { Model } = await imp('src/core/model.js');
const { CommunityModel } = await imp('src/core/community.js');
const { EditableMap } = await imp('src/map/editable-map.js');
const { layoutEngineOptions, normalizeLayoutSettings } = await imp('src/layout/layout-settings.js');
const { renderMap, frameLabelScale, rotationArm } = await imp('src/render/mapview.js');
const { FluxScale, normalizeFluxColorStops, sampleRamp, formatFlux, compartmentStyle } = await imp('src/render/theme.js');
const { buildReactionColoring, categoryColor, reactionCategory } = await imp('src/render/reaction-colors.js');
const { reactionEndpointGeometry } = await imp('src/render/reaction-path.js');
const { rasterScales } = await imp('src/render/raster-size.js');
const { routeCrossFeeding, mediumEndpoints, roundedPath, routeAnchor, routeGrid } = await imp('src/render/crossfeed-routes.js');
const { FluxProblem } = await imp('src/solver/fba.js');
const { buildCrossFeedingIndex } = await imp('src/analysis/cross-feeding-explorer.js');
const { Store } = await imp('src/ui/store.js');

for (const dir of [small, large]) {
  await rm(join(dir, 'svg'), { recursive: true, force: true });
  await rm(join(dir, 'maps'), { recursive: true, force: true });
  await mkdir(join(dir, 'svg'), { recursive: true });
  await mkdir(join(dir, 'maps'), { recursive: true });
}

// Models

const modelCache = new Map();
async function bundledModel(name) {
  if (!modelCache.has(name)) {
    modelCache.set(name, new Model(JSON.parse(await readFile(join(root, 'models', `${name}.json`), 'utf8'))));
  }
  return modelCache.get(name);
}

const { scenarios } = JSON.parse(await readFile(join(root, 'models/scenarios.json'), 'utf8'));
async function scenarioCommunity(s) {
  const medium = {};
  for (const [base, limit] of Object.entries(s.medium ?? {})) medium[base] = { lower: -Math.abs(Number(limit) || 0), upper: 1000 };
  const species = [];
  for (const sp of s.species) {
    species.push({ id: sp.id, model: await bundledModel(sp.model), abundance: sp.abundance ?? 1 / s.species.length, enabled: sp.enabled !== false });
  }
  return new CommunityModel({ name: s.name, objective: s.objective || 'equal-growth', productTarget: s.productTarget || null, medium, species });
}

const solve = (community) => new FluxProblem(community.merged).optimize();
const obj = (m) => Object.fromEntries(m);

// States

function toState(spec) {
  const st = {};
  const asMap = (o) => new Map(Object.entries(o));
  for (const [k, v] of Object.entries(spec)) {
    switch (k) {
      case 'fluxes': case 'solvedFluxes': case 'reactionColors': case 'pathwayRibbonColors':
      case 'pathwayRibbonGroups': case 'metaboliteLabels': case 'reactionLabels': case 'growth':
      case 'omicsColors': case 'fvaRanges':
        st[k] = asMap(v);
        break;
      case 'disabledReactionIds': case 'inactiveSpeciesIds': case 'emphasizedReactionIds':
        st[k] = new Set(v);
        break;
      case 'crossFeedingBases':
        st[k] = v === null ? null : new Set(v);
        break;
      case 'multiSelection':
        st[k] = v && { nodes: new Set(v.nodes ?? []), reactions: new Set(v.reactions ?? []), texts: new Set(v.texts ?? []) };
        break;
      case 'scale': {
        const values = v.values === 'fluxes' ? Object.values(spec.fluxes ?? {}) : v.values;
        st.scale = new FluxScale(values, { dark: v.dark, colorStops: v.colorStops ?? null });
        if (v.reference !== undefined) st.scale.reference = v.reference;
        break;
      }
      default:
        st[k] = v;
    }
  }
  return st;
}

// Cases

const written = { small: [], large: [] };
const mapsWritten = new Set();
let checked = 0;

async function addMap(where, name, map) {
  const dir = where === 'large' ? large : small;
  const key = `${where}/${name}`;
  if (!mapsWritten.has(key)) {
    await writeFile(join(dir, 'maps', `${name}.json`), JSON.stringify(map.toJSON()));
    mapsWritten.add(key);
  }
  return `maps/${name}.json`;
}

/** Render one case, check that the saved map draws the same, write it. */
async function addCase(where, name, mapName, map, spec) {
  const dir = where === 'large' ? large : small;
  const mapFile = await addMap(where, mapName, map);
  const svg = renderMap(map, toState(spec));
  // The Rust test draws the map read back from its JSON. That has to be the
  // same drawing as the live map's, or the fixture would test the wrong thing.
  const again = renderMap(EditableMap.fromJSON(JSON.parse(JSON.stringify(map.toJSON()))), toState(spec));
  if (again !== svg) throw new Error(`${name}: the map read back from JSON draws differently`);
  checked++;
  await writeFile(join(dir, 'svg', `${name}.svg`), svg);
  written[where].push({ name, map: mapFile, state: spec, svg: `svg/${name}.svg` });
}

/** The fluxes of a solution as plain JSON. */
const fluxJSON = (solution) => obj(solution.fluxes);

// Small: hand-built maps

// escher-style-render.test.mjs: an E. coli shell with GAPD drawn.
const ecoliRaw = await bundledModel('ecoli_core_mc');
const ecoli = new CommunityModel({ species: [{ id: 'eco', model: ecoliRaw, abundance: 1 }], medium: {} });
const gapd = EditableMap.emptyFromCommunity(ecoli);
gapd.addReaction(ecoli, 'eco@GAPD');
// The test's own map, with GAPD alone.
await addCase('small', 'gapd-only', 'gapd-only', gapd, { fluxes: { 'eco@GAPD': 8 }, scale: { values: [8] } });
for (const rid of ['eco@PGK', 'eco@GLCptspp', 'eco@EX_glc__D_e', 'eco@ACt2rpp']) gapd.addReaction(ecoli, rid);
gapd.addText('Glycolysis <upper> & "lower"', 140, 120, { fontSize: 30, fontFamily: 'serif' });
gapd.addText('Plain note', 140, 180, {});
gapd.addText('Mono note', 400, 180, { fontFamily: 'mono' });
{
  const fluxes = { 'eco@GAPD': 8, 'eco@PGK': -8, 'eco@GLCptspp': 4, 'eco@EX_glc__D_e': -4, 'eco@ACt2rpp': 0 };
  const base = { fluxes, scale: { values: 'fluxes' }, showCurrency: true, currencyLabels: true };
  const nodes = [...gapd.nodes.keys()];
  const reactions = [...gapd.reactions.keys()];
  const texts = [...gapd.texts.keys()];
  await addCase('small', 'gapd-default', 'gapd', gapd, base);
  await addCase('small', 'gapd-dark', 'gapd', gapd, { ...base, dark: true, scale: { values: 'fluxes', dark: true } });
  await addCase('small', 'gapd-no-solution', 'gapd', gapd, { scale: { values: [] } });
  await addCase('small', 'gapd-negative', 'gapd', gapd, { ...base, fluxes: { 'eco@GAPD': -8, 'eco@PGK': 0.01 } });
  await addCase('small', 'gapd-selected', 'gapd', gapd, { ...base, selectedReaction: 'eco@GAPD' });
  await addCase('small', 'gapd-multi', 'gapd', gapd, {
    ...base, multiSelection: { nodes: nodes.slice(0, 3), reactions: reactions.slice(0, 2), texts: texts.slice(0, 2) },
  });
  await addCase('small', 'gapd-multi-one', 'gapd', gapd, { ...base, multiSelection: { nodes: nodes.slice(0, 1) } });
  for (const labels of ['auto', 'all', 'active', 'none']) {
    await addCase('small', `gapd-labels-${labels}`, 'gapd', gapd, { ...base, labels });
  }
  await addCase('small', 'gapd-labels-active-solved', 'gapd', gapd, { ...base, labels: 'active', solvedFluxes: { 'eco@GAPD': 1 } });
  await addCase('small', 'gapd-currency-hidden', 'gapd', gapd, { ...base, showCurrency: false });
  await addCase('small', 'gapd-currency-labels-off', 'gapd', gapd, { ...base, currencyLabels: false });
  await addCase('small', 'gapd-subsystem-colours', 'gapd', gapd, {
    ...base, reactionColorMode: 'subsystem', reactionColors: { 'eco@GAPD': '#234abc', 'eco@PGK': '#bc4a23' },
  });
  await addCase('small', 'gapd-crossfeed-highlight', 'gapd', gapd, {
    ...base, showCrossFeeding: true, crossFeedingExplorer: { visibleEvents: [] }, selectedReaction: 'eco@PGK',
  });
  await addCase('small', 'gapd-omics', 'gapd', gapd, {
    ...base, omicsEnabled: true, omicsColors: { 'eco@GAPD': '#ff0000' },
  });
  await addCase('small', 'gapd-omics-fallback', 'gapd', gapd, {
    ...base, dark: true, omicsEnabled: true, omicsColors: { 'eco@PGK': '#00ff00' }, omicsFallbackColor: '#123456',
  });
  await addCase('small', 'gapd-fva', 'gapd', gapd, {
    ...base, fvaEnabled: true, labels: 'none',
    fvaRanges: { 'eco@GAPD': { min: -1.23456, max: 20 }, 'eco@PGK': { min: null, max: 3 } },
  });
  await addCase('small', 'gapd-edit', 'gapd', gapd, { ...base, reactionEdit: true, minHitRadius: 12 });
  await addCase('small', 'gapd-edit-far', 'gapd', gapd, { ...base, reactionEdit: true, minHitRadius: 80, dark: true });
  await addCase('small', 'gapd-hit-radius', 'gapd', gapd, { ...base, minHitRadius: 40 });
  await addCase('small', 'gapd-export', 'gapd', gapd, {
    ...base, forExport: true, selectedReaction: 'eco@GAPD', reactionEdit: true, minHitRadius: 12,
    selectedGeometry: { cellId: 'eco', compartmentId: 'c' }, multiSelection: { nodes: nodes.slice(0, 2) },
  });
  await addCase('small', 'gapd-geometry-cell', 'gapd', gapd, {
    ...base, geometryEdit: true, selectedGeometry: { cellId: 'eco', compartmentId: 'c' }, minHitRadius: 20,
  });
  await addCase('small', 'gapd-geometry-env', 'gapd', gapd, {
    ...base, selectedGeometry: { cellId: null, compartmentId: 's' }, dark: true,
  });
  await addCase('small', 'gapd-geometry-edit-none', 'gapd', gapd, { ...base, geometryEdit: true });
  await addCase('small', 'gapd-width-flux', 'gapd', gapd, { ...base, reactionWidthMode: 'flux' });
  await addCase('small', 'gapd-custom-labels', 'gapd', gapd, {
    ...base, labels: 'all', metaboliteLabels: { [nodes[0]]: 'Custom <name>' }, reactionLabels: { 'eco@GAPD': 'G3P dehydrogenase & co' },
  });
  await addCase('small', 'gapd-colour-stops', 'gapd', gapd, {
    ...base, scale: { values: 'fluxes', colorStops: [{ value: -10, color: '#2166ac' }, { value: 0, color: '#bdbdbd' }, { value: 10, color: '#b2182b' }] },
  });
  await addCase('small', 'gapd-growth', 'gapd', gapd, { ...base, growth: { eco: 0.87654321 } });
}

// A copy with hand edits: roles, hidden nodes, label angles, colours, label positions, escapes.
const edited = EditableMap.fromJSON(gapd.toJSON());
{
  const ids = [...edited.nodes.values()].filter((n) => n.kind === 'metabolite').map((n) => n.id);
  edited.setNodePrimary(ids[0], false);
  edited.setNodeHidden(ids[1], true);
  edited.setMetaboliteLabelAngle(ids[2], 0);
  edited.setMetaboliteLabelAngle(ids[3], Math.PI / 2);
  edited.setMetaboliteLabelAngle(ids[4], 2.4);
  edited.nodes.get(ids[5]).base = 'x<y & "z"';
  const cell = edited.cells[0];
  cell.name = 'E. coli "core" <K-12> & co';
  edited.setCellColors('eco', { fill: '#12ab34', stroke: '#aa0000' });
  const inner = cell.compartments.find((c) => c.id !== cell.outerCompartment);
  edited.setCompartmentColors('eco', inner.id, { fill: '#0000ff', stroke: '#00ff00' });
  edited.setCompartmentLabelPosition('eco', inner.id, inner.rect.x + inner.rect.w, inner.rect.y + inner.rect.h / 2);
  edited.setCompartmentLabelPosition(null, 's', edited.environment.rect.x + 40, edited.environment.rect.y + edited.environment.rect.h);
  const fluxes = { 'eco@GAPD': 8, 'eco@PGK': -8, 'eco@GLCptspp': 4 };
  await addCase('small', 'edited-default', 'edited', edited, { fluxes, scale: { values: 'fluxes' }, labels: 'all' });
  await addCase('small', 'edited-dark', 'edited', edited, { fluxes, scale: { values: 'fluxes', dark: true }, dark: true, labels: 'all' });
  await addCase('small', 'edited-no-cofactors', 'edited', edited, { fluxes, scale: { values: 'fluxes' }, showCurrency: false });
}
const hiddenEnv = EditableMap.fromJSON(gapd.toJSON());
hiddenEnv.environment.hidden = true;
await addCase('small', 'hidden-environment', 'hidden-environment', hiddenEnv, { scale: { values: [] } });

// name-plates.test.mjs: a shell with many compartments.
function algaCommunity() {
  const met = (id, compartment) => ({ id, name: id, compartment, formula: '', charge: 0 });
  const model = new Model({
    id: 'alga',
    name: 'A cell with a long descriptive name',
    compartments: { e: 'e', c: 'c', h: 'h', u: 'u', i: 'i', m: 'm', g: 'g' },
    metabolites: [met('a_e', 'e'), met('a_c', 'c'), met('a_h', 'h'), met('a_u', 'u'), met('a_i', 'i'), met('a_m', 'm'), met('a_g', 'g')],
    reactions: [{ id: 'T', name: 'T', metabolites: { a_e: -1, a_c: 1 }, lower_bound: -1000, upper_bound: 1000 }],
  });
  return new CommunityModel({ species: [{ id: 'alga', model, abundance: 1 }], medium: {} });
}
const alga = EditableMap.emptyFromCommunity(algaCommunity());
{
  await addCase('small', 'alga-default', 'alga', alga, { scale: { values: [] } });
  await addCase('small', 'alga-dark', 'alga', alga, { scale: { values: [] }, dark: true });
  await addCase('small', 'alga-far', 'alga', alga, { scale: { values: [] }, minHitRadius: 240 });
  await addCase('small', 'alga-geometry', 'alga', alga, {
    scale: { values: [] }, geometryEdit: true, selectedGeometry: { cellId: 'alga', compartmentId: 'm' }, minHitRadius: 30,
  });
  await addCase('small', 'alga-inactive', 'alga', alga, { scale: { values: [] }, inactiveSpeciesIds: ['alga'], inactiveSpeciesLabel: 'switched off' });
  const moved = EditableMap.fromJSON(alga.toJSON());
  const comps = moved.cells[0].compartments;
  const cytosol = comps.find((c) => c.id === 'c');
  const chloroplast = comps.find((c) => c.id === 'h');
  moved.editGeometryRect('alga', 'h', { ...chloroplast.rect, y: cytosol.rect.y + 10 });
  await addCase('small', 'alga-h-moved', 'alga-h-moved', moved, { scale: { values: [] } });
}

// rounded-transport.test.mjs: a transporter placed on the mitochondrial membrane.
{
  const yeastModel = await bundledModel('yeast_core_mc');
  const yeast = new CommunityModel({ species: [{ id: 'yeast', model: yeastModel, abundance: 1 }], medium: {} });
  const map = EditableMap.emptyFromCommunity(yeast);
  const cell = map.cells[0];
  const m = cell.compartments.find((c) => c.id === 'm');
  map.addReaction(yeast, 'yeast@PYRt2m', { boundaryId: `${cell.id}/m`, anchor: { x: m.rect.x, y: m.rect.y } });
  map.addReaction(yeast, 'yeast@AKGMALtm', { boundaryId: `${cell.id}/m`, anchor: { x: m.rect.x + m.rect.w, y: m.rect.y + m.rect.h / 2 } });
  map.addReaction(yeast, 'yeast@PDHm');
  const lanes = [...map.reactions.values()].filter((r) => (r.transportLanes?.length ?? 0) >= 2).length;
  if (lanes < 1) throw new Error('the yeast fixture has no multi-lane transporter');
  const base = { scale: { values: [1, 2, 3] } };
  await addCase('small', 'yeast-transport-forward', 'yeast-transport', map, { ...base, fluxes: { 'yeast@PYRt2m': 2, 'yeast@AKGMALtm': 1, 'yeast@PDHm': 3 } });
  await addCase('small', 'yeast-transport-reverse', 'yeast-transport', map, {
    ...base, fluxes: { 'yeast@PYRt2m': -2, 'yeast@AKGMALtm': -1 }, labels: 'all', dark: true,
  });
  await addCase('small', 'yeast-transport-ko', 'yeast-transport', map, {
    ...base, fluxes: { 'yeast@PYRt2m': 2 }, disabledReactionIds: ['yeast@PYRt2m', 'yeast@PDHm'], labels: 'auto',
  });
  await addCase('small', 'yeast-transport-edit', 'yeast-transport', map, { ...base, reactionEdit: true, selectedReaction: 'yeast@AKGMALtm' });
}

// zuniga-render.test.mjs: the published Zuñiga map (straight reactions, texts, two cells).
{
  const doc = JSON.parse(await readFile(join(root, 'models/published/zuniga-2020-phea-rescue/map.json'), 'utf8'));
  const map = EditableMap.fromJSON(doc);
  const ids = [...map.texts.keys()];
  map.styleText(ids[0], { fontFamily: 'serif' });
  map.styleText(ids[1], { fontFamily: 'courier', fontSize: 40 });
  map.styleText(ids[2], { fontFamily: 'sans' });
  const rid = 'Ec@CHORM';
  const fluxes = {};
  for (const [i, id] of [...map.reactions.keys()].entries()) fluxes[id] = ((i % 7) - 3) * 0.37;
  const scale = { values: 'fluxes', reference: 0.1 };
  await addCase('small', 'zuniga-default', 'zuniga', map, { fluxes, scale, labels: 'all' });
  await addCase('small', 'zuniga-width-flux', 'zuniga', map, {
    fluxes: { [rid]: -4 }, solvedFluxes: { [rid]: 2 }, scale: { values: [1, 4] }, reactionWidthMode: 'flux', labels: 'all',
  });
  await addCase('small', 'zuniga-ko', 'zuniga', map, {
    fluxes: { 'Ec@PPNDH': 0 }, scale: { values: [] }, labels: 'auto', disabledReactionIds: ['Ec@PPNDH'],
  });
  await addCase('small', 'zuniga-emphasized', 'zuniga', map, {
    fluxes: { 'Ec@PHEtex': 0.014371756 }, scale: { values: [1], reference: 0.1 }, labels: 'auto', emphasizedReactionIds: ['Ec@PHEtex'],
  });
  await addCase('small', 'zuniga-inactive', 'zuniga', map, {
    fluxes: { 'Se@CHORM': 100 }, scale: { values: [100] }, labels: 'all', growth: { Se: 0.1234, Ec: 0.2 }, inactiveSpeciesIds: ['Se'],
  });
  await addCase('small', 'zuniga-multi-texts', 'zuniga', map, {
    fluxes, scale, multiSelection: { texts: ids.slice(0, 3), reactions: ['Ec@CHORM'] }, dark: true,
  });
  await addCase('small', 'zuniga-edit', 'zuniga', map, { fluxes, scale, reactionEdit: true, minHitRadius: 25 });
}

// ui-audit-fixes.test.mjs: the store's own map of a one-cell demo, and of a pair.
async function auditStore(pair = false) {
  const store = new Store('.');
  store.models.set('audit-demo', new Model({
    id: 'audit-demo',
    compartments: { c: 'Cytosol', e: 'Extracellular' },
    metabolites: [{ id: 'a_e', compartment: 'e' }, { id: 'a_c', compartment: 'c' }],
    reactions: [
      { id: 'EX_a_e', metabolites: { a_e: -1 }, lower_bound: -10, upper_bound: 1000 },
      { id: 'At', metabolites: { a_e: -1, a_c: 1 }, lower_bound: 0, upper_bound: 1000 },
      { id: 'BIOMASS', metabolites: { a_c: -1 }, lower_bound: 0, upper_bound: 1000, objective_coefficient: 1 },
    ],
  }));
  store.scenarios = [{
    id: 'audit-demo', name: 'Audit demo', objective: 'equal-growth',
    species: [{ id: 'demo', model: 'audit-demo', abundance: 1 }], medium: { a: 10 },
  }, {
    id: 'audit-pair', name: 'Audit pair', objective: 'equal-growth',
    species: [{ id: 'demo', model: 'audit-demo', abundance: 0.5 }, { id: 'other', model: 'audit-demo', abundance: 0.5 }],
    medium: { a: 10 },
  }];
  await store.useScenario(pair ? 'audit-pair' : 'audit-demo');
  return store;
}
{
  const store = await auditStore();
  const fluxes = obj(store.displayFluxes());
  const solved = obj(store.solution.fluxes);
  const base = { fluxes, scale: { values: 'fluxes' } };
  const map = store.layout;
  for (const labels of ['auto', 'none']) await addCase('small', `audit-labels-${labels}`, 'audit', map, { ...base, labels });
  await addCase('small', 'audit-active-solved', 'audit', map, { ...base, labels: 'active', solvedFluxes: solved });
  await addCase('small', 'audit-active-idle', 'audit', map, { ...base, labels: 'active', solvedFluxes: {} });
  await addCase('small', 'audit-active-fva', 'audit', map, { ...base, labels: 'active', solvedFluxes: {}, fvaEnabled: true });
  const editor = {
    ...base, selectedReaction: 'demo@At', reactionEdit: true, selectedGeometry: { cellId: 'demo', compartmentId: 'c' },
    multiSelection: { nodes: ['met:demo@a_c', 'met:demo@a_e'], reactions: [], texts: [] }, minHitRadius: 12,
  };
  await addCase('small', 'audit-editor', 'audit', map, editor);
  await addCase('small', 'audit-exported', 'audit', map, { ...editor, forExport: true });
  const coloured = EditableMap.fromJSON(map.toJSON());
  const cell = coloured.cells.find((c) => c.id === 'demo');
  const inner = cell.compartments.find((c) => c.id !== cell.outerCompartment);
  coloured.setCellColors('demo', { fill: '#12ab34', stroke: '#aa0000' });
  coloured.setCompartmentColors('demo', inner.id, { fill: '#0000ff', stroke: '#00ff00' });
  await addCase('small', 'audit-colours', 'audit-colours', coloured, base);
  const secondary = EditableMap.fromJSON(map.toJSON());
  secondary.setNodePrimary('met:demo@a_c', false);
  for (const [name, view] of [['shown', { showCurrency: true, currencyLabels: true }], ['hidden', { showCurrency: false, currencyLabels: true }], ['unnamed', { showCurrency: true, currencyLabels: false }]]) {
    await addCase('small', `audit-cofactors-${name}`, 'audit-secondary', secondary, { ...base, ...view });
  }
}
{
  const store = await auditStore(true);
  const link = { from: 'demo', to: 'other', base: 'a', flux: 1 };
  const state = {
    fluxes: obj(store.displayFluxes()), scale: { values: 'fluxes' }, showCrossFeeding: true,
    crossFeeding: { links: [link], perMetabolite: [] },
    crossFeedingExplorer: { enabled: true, visibleEvents: [link], selectedEvent: link },
  };
  await addCase('small', 'pair-crossfeed-picked', 'pair', store.layout, state);
  await addCase('small', 'pair-crossfeed-exported', 'pair', store.layout, { ...state, forExport: true });
  await addCase('small', 'pair-crossfeed-at', 'pair', store.layout, {
    ...state, dark: true, crossFeedingExplorer: { ...state.crossFeedingExplorer, selectedAt: { x: 100, y: 100 } },
  });
  await addCase('small', 'pair-inactive', 'pair', store.layout, {
    fluxes: obj(store.displayFluxes()), scale: { values: 'fluxes' }, inactiveSpeciesIds: ['other'], inactiveSpeciesLabel: 'switched off',
    growth: { demo: 0.5, other: 0.25 },
  });
}

// pathway-ribbons.test.mjs / reaction-coloring.test.mjs: a stretch of the E. coli core map.
{
  const community = new CommunityModel({
    species: [{ id: 'eco', model: ecoliRaw, abundance: 1 }],
    medium: { glc__D: { lower: -10 }, o2: { lower: -20 } },
  });
  const full = EditableMap.fromCommunity(community);
  const keep = ['eco@PGI', 'eco@PFK', 'eco@FBA', 'eco@TPI', 'eco@GAPD', 'eco@PGK', 'eco@PGM', 'eco@ENO', 'eco@PYK',
    'eco@CS', 'eco@ACONTa', 'eco@ACONTb', 'eco@ICDHyr', 'eco@GLCpts', 'eco@EX_glc__D_e', 'eco@O2t', 'eco@EX_o2_e', 'eco@ACt2r'];
  const map = full.subsetByReactions(keep.filter((id) => full.reactions.has(id)));
  const solution = solve(community);
  const fluxes = fluxJSON(solution);
  for (const mode of ['subsystem', 'pathway']) {
    for (const dark of [false, true]) {
      const coloring = buildReactionColoring(community.merged, mode, { dark });
      const spec = {
        fluxes, scale: { values: 'fluxes', dark }, dark, pathwayRibbonMode: mode,
        pathwayRibbonColors: obj(coloring.colors), pathwayRibbonGroups: obj(coloring.groups),
        reactionColorMode: mode, reactionColors: obj(coloring.colors),
      };
      await addCase('small', `ribbons-${mode}${dark ? '-dark' : ''}`, 'ecoli-stretch', map, spec);
    }
  }
  const coloring = buildReactionColoring(community.merged, 'subsystem', {});
  await addCase('small', 'ribbons-wide', 'ecoli-stretch', map, {
    fluxes, scale: { values: 'fluxes' }, pathwayRibbonMode: 'subsystem', pathwayRibbonColors: obj(coloring.colors),
    pathwayRibbonGroups: obj(coloring.groups), pathwayRibbonWidth: 500, pathwayRibbonOpacity: 0, showCurrency: false,
  });
  await addCase('small', 'ribbons-off', 'ecoli-stretch', map, {
    fluxes, scale: { values: 'fluxes' }, pathwayRibbonMode: 'off', pathwayRibbonColors: obj(coloring.colors), pathwayRibbonGroups: obj(coloring.groups),
  });
  await addCase('small', 'ribbons-unannotated', 'ecoli-stretch', map, {
    fluxes, scale: { values: 'fluxes' }, pathwayRibbonMode: 'pathway',
    pathwayRibbonColors: Object.fromEntries(Object.keys(fluxes).map((id) => [id, '#2563eb'])),
    pathwayRibbonGroups: Object.fromEntries(Object.keys(fluxes).map((id) => [id, 'Sin pathway'])),
  });
}

// Large: bundled scenarios

const scenarioCases = [];
for (const s of scenarios) {
  if (s.loader) continue;
  const community = await scenarioCommunity(s);
  const map = EditableMap.fromCommunity(community, layoutEngineOptions(normalizeLayoutSettings()));
  const solution = solve(community);
  const fluxes = solution.status === 'optimal' ? fluxJSON(solution) : {};
  const growth = obj(community.growthRates(solution.fluxes));
  const base = { fluxes, solvedFluxes: fluxes, scale: { values: 'fluxes' }, growth };
  await addCase('large', `${s.id}-default`, s.id, map, base);
  await addCase('large', `${s.id}-dark-all`, s.id, map, { ...base, scale: { values: 'fluxes', dark: true }, dark: true, labels: 'all', minHitRadius: 30 });
  const coloring = buildReactionColoring(community.merged, 'subsystem', {});
  await addCase('large', `${s.id}-ribbons`, s.id, map, {
    ...base, pathwayRibbonMode: 'subsystem', pathwayRibbonColors: obj(coloring.colors), pathwayRibbonGroups: obj(coloring.groups),
  });
  const crossFeeding = community.crossFeeding(solution.fluxes, 1e-6);
  const index = buildCrossFeedingIndex(community, solution.fluxes);
  const explorer = { enabled: true, visibleEvents: index.events, selectedEvent: index.events[0] ?? null, selectedAt: null };
  await addCase('large', `${s.id}-crossfeed`, s.id, map, {
    ...base, showCrossFeeding: true, crossFeeding: { links: crossFeeding.links, perMetabolite: [] }, crossFeedingExplorer: explorer,
  });
  await addCase('large', `${s.id}-export`, s.id, map, {
    ...base, forExport: true, showCrossFeeding: true, crossFeeding: { links: crossFeeding.links, perMetabolite: [] },
    crossFeedingExplorer: { ...explorer, enabled: false }, selectedReaction: [...map.reactions.keys()][3], reactionEdit: true,
  });
  scenarioCases.push(s.id);
}

// Large: a gut9 snapshot

const gutLinks = (map, limit) => {
  const ends = mediumEndpoints(map);
  const byBase = new Map();
  for (const key of ends.keys()) {
    const [cell, base] = key.split('|');
    if (!byBase.has(base)) byBase.set(base, []);
    byBase.get(base).push(cell);
  }
  const links = [];
  let k = 0;
  for (const [base, cells] of byBase) {
    if (cells.length < 2) continue;
    links.push({ from: cells[0], to: cells[1], base, flux: 0.05 + (k++ % 9) * 0.37 });
    if (cells.length > 2) links.push({ from: cells[2], to: cells[0], base, flux: 0.02 + (k++ % 5) * 0.11 });
    if (links.length >= limit) break;
  }
  return links;
};
{
  const doc = JSON.parse(await readFile(join(root, 'docs/gut9-map/maps/12-phase7-Ba-Bt-envelope.json'), 'utf8'));
  const map = EditableMap.fromJSON(doc.map);
  const ids = [...map.reactions.keys()];
  const fluxes = {};
  ids.forEach((id, i) => { if (i % 3) fluxes[id] = Math.sin(i) * 4; });
  const links = gutLinks(map, 40);
  const base = { fluxes, scale: { values: 'fluxes', colorStops: doc.fluxColorStops ?? null } };
  await addCase('large', 'gut9-default', 'gut9', map, base);
  await addCase('large', 'gut9-dark-crossfeed', 'gut9', map, {
    ...base, dark: true, labels: 'all', showCrossFeeding: true, crossFeeding: { links, perMetabolite: [] },
    crossFeedingBases: [...new Set(links.map((l) => l.base))].slice(0, 25),
    crossFeedingExplorer: { enabled: false, visibleEvents: [], selectedEvent: links[0], selectedAt: null },
  });
  await addCase('large', 'gut9-edit', 'gut9', map, { ...base, reactionEdit: true, minHitRadius: 60, labels: 'active', solvedFluxes: fluxes });
}

// Units

const scaleSamples = [0, 1e-12, 1e-9, 2e-9, 0.001, 0.0123, -0.5, 1, -3.7, 10, 42.42, 100, -999.9, 1e4, Infinity, -Infinity];
const valueSets = {
  empty: [],
  one: [5],
  mixed: [-20, 0, 5, 100, 0.0001, -0.003, 12.5, 7, 7, 1e-10, 3.3333],
  ramp: Array.from({ length: 60 }, (_, i) => (i % 2 ? -1 : 1) * Math.pow(1.31, i) * 0.01),
};
const stopSets = {
  none: null,
  signed: [{ value: -20, color: '#ff0000' }, { value: 0, color: '#ffffff' }, { value: 5, color: '#0000ff' }],
  single: [{ value: -3, color: '#123456' }],
  wide: [{ value: -1e308, color: '#000000' }, { value: 1e308, color: '#ffffff' }],
  unsorted: [{ value: 10, color: '#FFFFFF' }, { value: -100, color: ' #000000 ' }, { value: 0, color: '#ff0000' }],
};
const scales = [];
for (const [vname, values] of Object.entries(valueSets)) {
  for (const [sname, colorStops] of Object.entries(stopSets)) {
    for (const dark of [false, true]) {
      const s = new FluxScale(values, { dark, colorStops });
      scales.push({
        values: vname, stops: sname, dark,
        count: s.count, min: s.min, max: s.max, reference: s.reference,
        samples: scaleSamples.map((v) => ({ v: String(v), position: s.position(v), width: s.width(v), color: s.color(v) })),
        legend: s.legendStops().map((x) => ({ ...x, value: x.value })),
      });
    }
  }
}
const stopErrors = [
  [], { value: 0, color: '#ffffff' }, [{ value: 0, color: '#ffffff' }, { value: 0, color: '#000000' }],
  [{ value: 'not a number', color: '#ffffff' }], [{ value: 0, color: '#fff' }], [{ value: 0, color: '#gg0000' }],
  [{ value: 0, color: 'red' }], [{ value: 0 }], [null], [{ value: -0, color: '#000000' }, { value: 0, color: '#ffffff' }],
].map((input) => {
  try { return { input, ok: normalizeFluxColorStops(input) }; } catch (e) { return { input, error: e.message }; }
});
const ramps = { light: [], dark: [] };
for (let i = -5; i <= 105; i++) {
  const t = i / 100;
  ramps.light.push(sampleRamp([[0, '#cbd5e1'], [0.12, '#38bdf8'], [0.35, '#2563eb'], [0.65, '#7c3aed'], [1, '#be123c']], t));
  ramps.dark.push(sampleRamp([[0, '#475569'], [0.12, '#38bdf8'], [0.35, '#60a5fa'], [0.65, '#a78bfa'], [1, '#fb7185']], t));
}
const fluxFormats = [];
for (const v of [0, 1e-10, -1e-10, 0.0004, -0.0004, 0.0005, 1.0005, 2.5, 1.23456789, -7.77777, 999.9999, 999.99951, 1000, 1125, -1234.5,
  12345.678, 9.995e5, 1e21, 0.1 + 0.2, null, NaN]) {
  for (const digits of [3, 4]) fluxFormats.push({ v: v === null ? null : String(v), digits, text: formatFlux(v, digits) });
}
const compartmentStyles = [];
for (const id of ['s', 'c', 'e', 'm', 'im', 'es', 'x', 'p', 'zz', 'unknown-compartment']) {
  for (const dark of [false, true]) compartmentStyles.push({ id, dark, style: compartmentStyle(id, dark) });
}
const categoryLabels = ['Glycolysis', 'TCA cycle', 'Sin pathway', 'sin SUBSYSTEM', '  ', '', 'Citric Acid Cycle', 'Transport, extracellular',
  'Ácido graso β-oxidación', '𝔸 astral', 'Oxidative Phosphorylation', 'Pyruvate Metabolism'];
const categories = categoryLabels.flatMap((label) => [false, true].map((dark) => ({ label, dark, color: categoryColor(label, dark) })));
const colorings = [];
for (const name of ['ecoli_core_mc', 'yeast_core_mc', 'cyano_core_mc']) {
  const model = await bundledModel(name);
  for (const mode of ['flux', 'subsystem', 'pathway']) {
    for (const dark of [false, true]) {
      const ids = [...model.reactions.keys()].filter((_, i) => i % 3 === 0);
      const coloring = buildReactionColoring(model, mode, { dark, reactionIds: dark ? ids : undefined });
      colorings.push({
        model: name, mode, dark, limited: dark, colors: obj(coloring.colors), groups: obj(coloring.groups), legend: coloring.legend,
        categories: Object.fromEntries([...model.reactions].slice(0, 20).map(([id, r]) => [id, reactionCategory(r, mode)])),
      });
    }
  }
}
// reaction-path: lines and curves, with and without a node at the end.
let seed = 7;
const rand = () => { seed = (seed * 16807) % 2147483647; return seed / 2147483647; };
const nodeKinds = [null, { kind: 'metabolite', primary: true }, { kind: 'currency' }, { kind: 'metabolite', primary: false },
  { kind: 'medium' }, { kind: 'other', r: 9 }];
const paths = [];
for (let i = 0; i < 160; i++) {
  const cubic = i % 2 === 1;
  const pts = Array.from({ length: cubic ? 4 : 2 }, () => ({ x: Math.round(rand() * 4000) / 10 - 50, y: Math.round(rand() * 4000) / 10 - 50 }));
  if (i % 17 === 0) pts[pts.length - 1] = { x: pts[0].x + 3, y: pts[0].y };
  const node = nodeKinds[i % nodeKinds.length];
  const arrow = i % 3 !== 0;
  const width = [7.5, 1.1, 15, 2][i % 4];
  const g = reactionEndpointGeometry(pts, { node, arrow, width });
  paths.push({ points: pts, node, arrow, width, d: g.d, arrow_head: g.arrow });
}
const rasters = [[1200, 800], [10000, 10200], [40000, 200], [1e7, 1e7], [0, 0], [27400, 25600], [16384, 16384], [1, 1], [8192, 9000], [33000, 10]]
  .map(([w, h]) => ({ w, h, scales: rasterScales(w, h) }));
const misc = {
  frameLabelScale: [0, 6, 12, 24, 100, 240, 1e6, -5].map((m) => ({ m, v: frameLabelScale(m) })),
  rotationArm: [0, 5, 12, 40, 100, -3].map((m) => ({ m, v: rotationArm(m) })),
};
// crossfeed routes on the published gut9 map, as crossfeed-routes.test.mjs draws it.
const gutDoc = JSON.parse(await readFile(join(root, 'models/published/steadycom-gut-nine/map.json'), 'utf8'));
const gutMap = EditableMap.fromJSON(gutDoc.map);
await writeFile(join(large, 'maps', 'gut9-published.json'), JSON.stringify(gutMap.toJSON()));
const routeSets = [];
{
  const ends = mediumEndpoints(gutMap);
  const named = [['Bt', 'ac', 'Fp'], ['St', 'lac-L', 'Bt'], ['Kp', 'nh4', 'St'], ['Ef', 'gln-L', 'Lc'], ['Ec', 'asp-L', 'Ef'], ['Bt', 'h', 'Fp']]
    .filter(([from, base, to]) => ends.has(`${from}|${base}`) && ends.has(`${to}|${base}`))
    .map(([from, base, to]) => ({ from, base, to, flux: 1 }));
  for (const links of [named, gutLinks(gutMap, 30), [...gutLinks(gutMap, 12), { from: 'Bt', to: 'Fp', base: 'not-a-medium-base', flux: 1 }]]) {
    const routes = routeCrossFeeding(gutMap, links);
    routeSets.push({ links, routes: Object.fromEntries(routes), grid: routeGrid(gutMap) });
  }
}
const endpoints = Object.fromEntries(mediumEndpoints(gutMap));
const rounded = [];
for (let i = 0; i < 40; i++) {
  const pts = Array.from({ length: 2 + (i % 5) }, () => ({ x: Math.round(rand() * 2000) - 1000, y: Math.round(rand() * 2000) - 1000 }));
  if (i % 7 === 0) pts.splice(1, 0, { ...pts[0] });
  const at = i % 3 ? { x: rand() * 1000, y: rand() * 1000 } : null;
  rounded.push({ points: pts, radius: [0, 12, 50, 300][i % 4], d: roundedPath(pts, [0, 12, 50, 300][i % 4]), at, anchor: routeAnchor(pts, at) });
}

await writeFile(join(large, 'units-routes.json'), JSON.stringify({ map: 'maps/gut9-published.json', routeSets, endpoints }));
await writeFile(join(small, 'units.json'), JSON.stringify({
  scales, stopErrors, ramps, fluxFormats, compartmentStyles, categories, colorings, paths, rasters, misc, rounded,
}));
await writeFile(join(small, 'cases.json'), JSON.stringify(written.small, null, 1));
await writeFile(join(large, 'cases.json'), JSON.stringify(written.large, null, 1));
console.log(`wrote ${written.small.length} small and ${written.large.length} large render cases (${checked} checked against their saved maps), scenarios: ${scenarioCases.join(', ')}`);
