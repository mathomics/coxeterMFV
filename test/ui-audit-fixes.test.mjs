import test from 'node:test';
import assert from 'node:assert/strict';
import { Store } from '../src/ui/store.js';
import { Model } from '../src/core/model.js';
import { MapHistory } from '../src/map/history.js';
import { EditableMap } from '../src/map/editable-map.js';
import { renderMap } from '../src/render/mapview.js';
import {
  renderInspector,
  renderLayoutSettings,
  renderTaskPanel,
  renderViewControls,
  modelKindLabel,
} from '../src/ui/panels.js';
import { statusLine, NOTICE_MS } from '../src/ui/status-line.js';

async function makeStore() {
  const store = new Store('.');
  store.models.set('audit-demo', new Model({
    id: 'audit-demo',
    compartments: { c: 'Cytosol', e: 'Extracellular' },
    metabolites: [
      { id: 'a_e', compartment: 'e' },
      { id: 'a_c', compartment: 'c' },
    ],
    reactions: [
      { id: 'EX_a_e', metabolites: { a_e: -1 }, lower_bound: -10, upper_bound: 1000 },
      { id: 'At', metabolites: { a_e: -1, a_c: 1 }, lower_bound: 0, upper_bound: 1000 },
      { id: 'BIOMASS', metabolites: { a_c: -1 }, lower_bound: 0, upper_bound: 1000, objective_coefficient: 1 },
    ],
  }));
  store.scenarios = [{
    id: 'audit-demo', name: 'Audit demo', objective: 'equal-growth',
    species: [{ id: 'demo', model: 'audit-demo', abundance: 1 }],
    medium: { a: 10 },
  }];
  await store.useScenario('audit-demo');
  assert.equal(store.solution.status, 'optimal');
  return store;
}

// Cleared numeric fields

test('a cleared or half-typed bound is not a bound of 0', async () => {
  const store = await makeStore();
  for (const value of ['', '   ', '-', 'abc', NaN, Infinity]) {
    const result = store.setReactionBounds('demo@At', null, value);
    assert.equal(result.upper, 1000, `upper bound changed by ${JSON.stringify(String(value))}`);
    assert.equal(store.currentReactionBounds('demo@At').upper, 1000);
  }
  assert.equal(store.overrides.has('demo@At'), false, 'an ignored value must not create an override');

  store.setReactionBounds('demo@At', null, '5');
  assert.equal(store.currentReactionBounds('demo@At').upper, 5, 'a real number still applies');
  store.setReactionBounds('demo@At', '', null);
  assert.equal(store.currentReactionBounds('demo@At').lower, 0, 'clearing the other side keeps it');
  store.setReactionBounds('demo@At', null, '0');
  assert.equal(store.currentReactionBounds('demo@At').upper, 0, 'an explicit 0 is still a bound of 0');
});

test('a cleared medium limit or abundance changes nothing and does not solve', async () => {
  const store = await makeStore();
  let solves = 0;
  const solve = store.solve.bind(store);
  store.solve = (...args) => { solves += 1; return solve(...args); };

  assert.equal(store.setMediumLimit('a', ''), null);
  assert.equal(store.setMediumLimit('a', '-'), null);
  assert.equal(store.medium.a, 10);
  assert.equal(solves, 0);
  store.setMediumLimit('a', '4');
  assert.equal(store.medium.a, 4);
  assert.equal(solves, 1);

  await store.setAbundance('demo', '');
  assert.equal(store.species[0].abundance, 1);
});

// Full-model warning

test('drawing the full model asks first only when the model is too large to lay out', async () => {
  const store = await makeStore();
  assert.equal(store.fullMapWarning(), null, 'a core model draws instantly');

  const big = { size: 4106 };
  store.community = {
    activeSpecies: [{ model: { reactions: big, metabolites: { size: 2749 } } }],
    merged: { reactions: big },
  };
  store.map = { reactions: { size: 93 } };
  const warning = store.fullMapWarning();
  assert.match(warning, /all 4106 reactions/);
  assert.match(warning, /replaces the current drawing of 93 reactions/);
  assert.match(store.relayoutBlocked(), /hand-built drawing/, 'the preset guard is unchanged');

  store.map = { reactions: { size: 0 } };
  assert.doesNotMatch(store.fullMapWarning(), /replaces/, 'nothing to replace on an empty map');
});

// Saved / unsaved

test('map history versions follow edits, undo and redo back to the saved map', () => {
  const history = new MapHistory();
  assert.equal(history.dirty, false);
  history.record({ a: 1 }, { a: 2 });
  assert.equal(history.dirty, true);
  history.markSaved();
  assert.equal(history.dirty, false);
  history.undo({ a: 2 });
  assert.equal(history.dirty, true, 'undoing past the save is unsaved');
  history.redo({ a: 1 });
  assert.equal(history.dirty, false, 'redoing back to the saved map is saved again');
  history.record({ a: 2 }, { a: 2 });
  assert.equal(history.dirty, false, 'a no-op edit changes nothing');
  history.record({ a: 2 }, { a: 3 });
  history.clear();
  assert.equal(history.dirty, false, 'a new map has nothing to save');
});

test('Saved reflects map, scenario and model files, not undo history or uploads', async () => {
  const store = await makeStore();
  assert.deepEqual(store.unsavedWork(), { map: false, scenario: false, model: false }, 'a fresh scenario is saved');

  store.setMediumLimit('a', '4');
  assert.equal(store.unsavedWork().scenario, true, 'a medium change is unsaved');
  const scenarioFile = store.stateJSON();
  store.markScenarioSaved();
  assert.equal(store.unsavedWork().scenario, false, 'Scenario export saves it');

  store.setReactionBounds('demo@At', null, '7');
  assert.equal(store.unsavedWork().scenario, true, 'a bound change is unsaved');
  await store.loadState(scenarioFile);
  assert.equal(store.unsavedWork().scenario, false, 'opening a scenario file is saved');

  const tx = store.beginMapEdit('Make secondary');
  store.map.setNodePrimary('met:demo@a_c', false);
  store.commitMapEdit(tx);
  assert.equal(store.unsavedWork().map, true, 'a map edit is unsaved');
  const mapFile = store.mapJSON();
  store.markMapSaved();
  assert.equal(store.unsavedWork().map, false, 'Map JSON export saves it');
  store.undoMap();
  assert.equal(store.unsavedWork().map, true);
  store.loadMapJSON(mapFile);
  assert.equal(store.unsavedWork().map, false, 'opening a map file is saved');

  store.modelEditorSpeciesId = 'demo';
  await store.createModelMetabolite({ id: 'b_c', name: 'B', compartment: 'c' });
  assert.equal(store.unsavedWork().model, true, 'a model edit is unsaved');
  store.markModelSaved();
  assert.equal(store.unsavedWork().model, false, 'Model JSON export saves it');

  store.uploadedModelKeys.add('audit-demo');
  assert.deepEqual(store.unsavedWork(), { map: false, scenario: false, model: false },
    'an uploaded model alone is not unsaved work');
});

// Rendering

test('Labels: None hides metabolite labels as well as reaction labels', async () => {
  const store = await makeStore();
  const draw = (labels) => renderMap(store.layout, {
    fluxes: store.displayFluxes(), scale: store.scale, labels,
  });
  assert.match(draw('auto'), /emc-label-met/);
  assert.match(draw('auto'), /emc-label-medium/);
  const none = draw('none');
  assert.doesNotMatch(none, /emc-label-met|emc-label-medium|emc-label-currency|emc-label-rxn/);
});

test('an exported map carries no selection, handles, pointer targets or resize grips', async () => {
  const store = await makeStore();
  const state = {
    fluxes: store.displayFluxes(),
    scale: store.scale,
    selectedReaction: 'demo@At',
    reactionEdit: true,
    selectedGeometry: { cellId: 'demo', compartmentId: 'c' },
    multiSelection: { nodes: new Set(['met:demo@a_c', 'met:demo@a_e']), reactions: new Set(), texts: new Set() },
    minHitRadius: 12,
  };
  const editor = renderMap(store.layout, state);
  for (const marker of ['emc-hit', 'data-canvas-resize', 'emc-multi-area', 'emc-multi-drag', 'is-selected', 'is-multi-selected', 'emc-geometry-selection']) {
    assert.ok(editor.includes(marker), `the editor view should contain ${marker}`);
  }
  const exported = renderMap(store.layout, { ...state, forExport: true });
  for (const marker of ['emc-hit', 'data-canvas-resize', 'emc-multi-area', 'emc-multi-drag', 'is-selected', 'is-multi-selected', 'emc-geometry-selection', 'emc-layer-edit-handles']) {
    assert.ok(!exported.includes(marker), `the export must not contain ${marker}`);
  }
  assert.match(exported, /emc-node/, 'the map itself is still drawn');
  assert.match(exported, /emc-canvas-frame/, 'and so is its white canvas');
});

// Status

test('an ok message is shown for a while before the summary returns', () => {
  const store = { status: { state: 'ok', message: 'The drawing was kept.', solveMs: 3 }, community: null };
  const fresh = statusLine(store, 0);
  assert.equal(fresh.text, 'The drawing was kept.');
  assert.equal(fresh.expiresIn, NOTICE_MS);
  const stale = statusLine(store, NOTICE_MS + 1);
  assert.match(stale.text, /species · 0 reactions · objective — · LP 3 ms/);
  assert.equal(stale.expiresIn, 0);

  assert.equal(statusLine({ status: { state: 'ok', message: '' } }, 0).state, 'summary');
  assert.equal(statusLine({ status: { state: 'warn', message: 'w' } }, 1e9).text, 'w');
  assert.equal(statusLine({ status: { state: 'error', message: 'e' } }, 1e9).text, 'e');
  assert.equal(statusLine({ status: { state: 'busy', message: '' } }, 1e9).text, 'Working…');
});

// Panels

test('the model badge is read from the loaded model', async () => {
  const store = await makeStore();
  assert.deepEqual(modelKindLabel(store).text, 'Multi-compartment · Single cell');
  assert.match(modelKindLabel(store).title, /1 active species; up to 2 compartments/);
  assert.equal(modelKindLabel(new Store('.')).text, '');
});

test('a shared-medium metabolite belongs to the shared medium, not to a cell called M', async () => {
  const store = await makeStore();
  store.selectedEntity = { type: 'metabolite', id: 'met:M@a' };
  const html = renderInspector(store);
  assert.match(html, /<dt>Cell<\/dt>\s*<dd[^>]*>Shared medium<\/dd>/);

  store.selectedEntity = { type: 'metabolite', id: 'met:demo@a_c' };
  assert.doesNotMatch(renderInspector(store), />Shared medium</);
});

test('label options, Add Metabolite help and blocked layout sliders say what they do', async () => {
  const store = await makeStore();
  const view = renderViewControls(store);
  assert.match(view, />Labels: major fluxes</);
  assert.doesNotMatch(view, /Primary only/);

  const build = renderTaskPanel(store, { leftTab: 'build', tool: 'metabolite' });
  assert.match(build, /Click a compartment, then pick a metabolite\./);
  assert.doesNotMatch(build, /biological model form/);

  const open = renderLayoutSettings(store);
  assert.doesNotMatch(open, /type="range" disabled/);
  store.relayoutBlocked = () => 'Layout is blocked here.';
  const blocked = renderLayoutSettings(store);
  assert.equal((blocked.match(/type="range" disabled title="Layout is blocked here\."/g) ?? []).length, 4);
  assert.match(blocked, /layout-blocked-note">Layout is blocked here\./);
});

// Map colour pickers

test('cell and compartment colours are drawn, undoable and saved with the map', async () => {
  const store = await makeStore();
  const cell = store.map.cells.find((item) => item.id === 'demo');
  const body = cell.compartments.find((item) => item.id === cell.outerCompartment);
  const inner = cell.compartments.find((item) => item.id !== body.id);
  const draw = () => renderMap(store.layout, { fluxes: store.displayFluxes(), scale: store.scale });

  store.setMapGeometryColors('demo', null, { fill: '#12AB34' });
  assert.equal(body.fill, '#12ab34', 'the cell fill is the fill of its body');
  store.setMapGeometryColors('demo', null, { stroke: '#aa0000' });
  assert.equal(cell.stroke, '#aa0000');
  store.setMapGeometryColors('demo', inner.id, { fill: '#0000ff', stroke: '#00ff00' });
  let svg = draw();
  assert.match(svg, new RegExp(`data-compartment-space="${body.id}"[^>]*fill="#12ab34"`));
  assert.match(svg, /class="emc-cell-boundary"[^>]*stroke="#aa0000"/);
  assert.match(svg, new RegExp(`data-compartment-space="${inner.id}"[^>]*fill="#0000ff"`));
  assert.match(svg, /stroke="#00ff00"/);

  store.setMapGeometryColors('demo', inner.id, { fill: 'red' });
  assert.equal(inner.fill, '#0000ff', 'only #rrggbb colours are taken');

  const saved = store.mapJSON();
  store.undoMap();
  const undone = store.map.cells.find((item) => item.id === 'demo').compartments.find((item) => item.id === inner.id);
  assert.equal(undone.fill, undefined, 'undo takes the last colour back');
  assert.equal(store.map.cells.find((item) => item.id === 'demo').stroke, '#aa0000', 'and only the last one');
  store.loadMapJSON(saved);
  const reopened = store.map.cells.find((item) => item.id === 'demo');
  assert.equal(reopened.stroke, '#aa0000', 'colours survive Map JSON');
  assert.equal(reopened.compartments.find((item) => item.id === inner.id).fill, '#0000ff');

  store.setMapGeometryColors('demo', null, { fill: null, stroke: null });
  store.setMapGeometryColors('demo', inner.id, { fill: null, stroke: null });
  svg = draw();
  assert.doesNotMatch(svg, /#12ab34|#aa0000|#0000ff|#00ff00/, 'reset goes back to the theme');
});

test('colour pickers show the colour drawn and offer a reset only once customised', async () => {
  const store = await makeStore();
  const cell = store.map.cells.find((item) => item.id === 'demo');
  store.selectedEntity = { type: 'cell', id: 'demo' };
  let html = renderInspector(store);
  assert.match(html, /data-action="cell-fill-color"[^>]*value="#[0-9a-f]{6}"/);
  assert.match(html, new RegExp(`data-action="cell-border-color"[^>]*value="${cell.color}"`), 'border shows the species colour');
  assert.doesNotMatch(html, /cell-reset-colors/);
  store.setMapGeometryColors('demo', null, { stroke: '#aa0000' });
  html = renderInspector(store);
  assert.match(html, /data-action="cell-border-color"[^>]*value="#aa0000"/);
  assert.match(html, /data-action="cell-reset-colors"/);

  const comp = cell.compartments[0];
  store.selectedEntity = { type: 'compartment', cellId: 'demo', id: comp.id };
  html = renderInspector(store);
  assert.match(html, /data-action="compartment-fill-color"[^>]*value="#[0-9a-f]{6}"/);
  assert.doesNotMatch(html, /value="#dbeafe"/, 'no stale hard-coded default');
});

// Cofactors and role buttons

test('the Cofactors toggles hide every secondary node, its lines and its label', async () => {
  const store = await makeStore();
  store.map.setNodePrimary('met:demo@a_c', false);
  const draw = (view) => renderMap(store.layout, { fluxes: store.displayFluxes(), scale: store.scale, ...view });

  const shown = draw({ showCurrency: true, currencyLabels: true });
  assert.match(shown, /class="emc-node[^"]*"[^>]*data-node="met:demo@a_c"/);
  assert.match(shown, /data-metabolite-label="met:demo@a_c"/);

  const hidden = draw({ showCurrency: false, currencyLabels: true });
  assert.doesNotMatch(hidden, /data-node="met:demo@a_c"/, 'node, hit target and lines go');
  assert.doesNotMatch(hidden, /data-metabolite-label="met:demo@a_c"/);
  assert.match(hidden, /data-node="met:demo@a_e"/, 'primary metabolites stay');

  const unnamed = draw({ showCurrency: true, currencyLabels: false });
  assert.match(unnamed, /class="emc-node[^"]*"[^>]*data-node="met:demo@a_c"/);
  assert.doesNotMatch(unnamed, /data-metabolite-label="met:demo@a_c"/, 'Cofactor labels names secondary nodes too');
  assert.match(unnamed, /data-metabolite-label="met:demo@a_e"/);
});

// Labels and PNG size

test('All active names only metabolites drawn on a reaction that carries flux', async () => {
  const store = await makeStore();
  const draw = (labels, solvedFluxes) => renderMap(store.layout, {
    fluxes: store.displayFluxes(), scale: store.scale, labels, solvedFluxes,
  });
  const solved = store.solution.fluxes;
  const metLabels = (svg) => (svg.match(/data-metabolite-label="/g) ?? []).length;

  const active = draw('active', solved);
  assert.match(active, /data-metabolite-label="met:demo@a_c"/, 'a_c is on the active transporter');
  const idle = draw('active', new Map());
  assert.equal(metLabels(idle), 0, 'nothing carries flux, so nothing is named');
  assert.equal(metLabels(draw('all', new Map())), metLabels(draw('all', solved)), 'All ignores activity');

  const fva = renderMap(store.layout, {
    fluxes: store.displayFluxes(), scale: store.scale, labels: 'active', solvedFluxes: new Map(), fvaEnabled: true,
  });
  assert.ok(metLabels(fva) > 0, 'FVA mode keeps every name, as it does for reactions');
});

test('PNG scales fit each browser canvas ceiling, largest first', async () => {
  const { rasterScales, PREFERRED_SCALE } = await import('../src/render/raster-size.js');
  assert.equal(PREFERRED_SCALE, 2);
  assert.deepEqual(rasterScales(1200, 800), [2], 'an ordinary map gets the full 2x');

  // The hand-built Yeast9 map this was found on.
  const scales = rasterScales(10000, 10200);
  assert.deepEqual(scales, [0.99, 0.4], 'at most 100 million pixels, then Safari\'s ceiling');
  for (const [scale, area, side] of [[0.99, 100_000_000, 32_767], [0.4, 16_777_216, 16_384]]) {
    const w = Math.round(10000 * scale);
    const h = Math.round(10200 * scale);
    assert.ok(w * h <= area && w <= side && h <= side, `${scale}x must fit ${area} px`);
  }
  assert.deepEqual(rasterScales(40000, 200), [0.81, 0.4], 'a very wide map is held by the side limit');
  assert.deepEqual(rasterScales(1e7, 1e7), [], 'nothing fits: the export says so and points to SVG');
});

// Species toggle and the log

async function makePairStore() {
  const store = await makeStore();
  store.scenarios = [{
    id: 'audit-pair', name: 'Audit pair', objective: 'equal-growth',
    species: [
      { id: 'demo', model: 'audit-demo', abundance: 0.5 },
      { id: 'other', model: 'audit-demo', abundance: 0.5 },
    ],
    medium: { a: 10 },
  }];
  await store.useScenario('audit-pair');
  assert.equal(store.map.cells.length, 2);
  return store;
}

test('switching a species off keeps a hand-built map, its undo history and every bound edit', async () => {
  const store = await makePairStore();
  const tx = store.beginMapEdit('hand edit');
  store.map.setNodePrimary('met:demo@a_c', false);
  store.commitMapEdit(tx);
  store.setReactionBounds('demo@At', null, '7');
  store.setReactionBounds('other@At', null, '3');

  assert.equal(store.keepsDrawingOnSpeciesToggle(), true);
  await store.setSpeciesEnabled('other', false);
  assert.equal(store.map.cells.length, 2, 'the switched-off cell stays drawn');
  assert.ok(store.map.reactions.has('other@At'), 'and so do its reactions');
  assert.equal(store.map.nodes.get('met:demo@a_c').primary, false, 'the hand edit survives');
  assert.equal(store.canUndoMap, true, 'so does the undo history');
  assert.deepEqual(store.disabledDrawnSpeciesIds(), ['other']);
  assert.equal(store.community.merged.reactions.has('other@At'), false, 'it is not simulated');
  assert.equal(store.currentReactionBounds('demo@At').upper, 7, 'edits on the species that stayed are kept');

  const svg = renderMap(store.layout, {
    fluxes: store.displayFluxes(), scale: store.scale,
    inactiveSpeciesIds: new Set(store.disabledDrawnSpeciesIds()), inactiveSpeciesLabel: 'switched off',
  });
  assert.match(svg, /· switched off/);

  store.selectedReaction = 'other@At';
  const inspector = renderInspector(store);
  assert.match(inspector, /is switched off, so this reaction is not simulated/);
  assert.doesNotMatch(inspector, /data-action="bound-upper/, 'no bounds to edit for a reaction the analysis cannot see');
  store.selectedReaction = null;

  await store.setSpeciesEnabled('other', true);
  assert.equal(store.map.cells.length, 2);
  assert.deepEqual(store.disabledDrawnSpeciesIds(), []);
  assert.equal(store.problem.getBounds('other@At').upper, 3, 'its own bound edit applies again');
});

test('an untouched automatic map is laid out again, as before', async () => {
  const store = await makePairStore();
  assert.equal(store.keepsDrawingOnSpeciesToggle(), false);
  await store.setSpeciesEnabled('other', false);
  assert.equal(store.map.cells.length, 1);
  assert.equal(store.map.cells[0].id, 'demo');
});

test('the last species cannot be switched off, which used to delete the map', async () => {
  const store = await makeStore();
  const map = store.map;
  await store.setSpeciesEnabled('demo', false);
  assert.equal(store.species[0].enabled, true);
  assert.equal(store.map, map);
  assert.equal(store.status.state, 'warn');
});

test('a map saved with a switched-off species opens again with that species still drawn', async () => {
  const store = await makePairStore();
  const tx = store.beginMapEdit('hand edit');
  store.map.setNodePrimary('met:other@a_c', false);
  store.commitMapEdit(tx);
  await store.setSpeciesEnabled('other', false);
  const file = store.mapJSON();
  store.showEmptyMap();
  assert.equal(store.map.cells.length, 0);

  store.loadMapJSON(file);
  assert.equal(store.map.cells.length, 2);
  assert.ok(store.map.reactions.has('other@At'), 'the switched-off species is not pruned as missing');
  assert.equal(store.map.nodes.get('met:other@a_c')?.primary, false);
});

test('the log keeps every message, counts repeats and folds back-to-back solves', async () => {
  const store = await makeStore();
  store.log = [];
  store.status = { ...store.status, state: 'warn', message: 'Layout kept.' };
  store.status = { ...store.status, state: 'warn', message: 'Layout kept.' };
  store.status = { ...store.status, state: 'busy', message: 'Working on it' };
  store.status = { ...store.status, state: 'error', message: 'It broke.' };
  store.solve();
  store.solve();
  assert.deepEqual(store.log.map((e) => [e.level, e.count]), [['warn', 2], ['error', 1], ['ok', 2]]);
  assert.match(store.log.at(-1).message, /^FBA solved: optimal, objective [\d.]+, \d+ ms$/);

  for (let i = 0; i < 260; i += 1) store.addLog('ok', `message ${i}`);
  assert.equal(store.log.length, 200, 'the log is bounded');
  assert.equal(store.log.at(-1).message, 'message 259');

  const { renderLogEntries } = await import('../src/ui/panels.js');
  const html = renderLogEntries([
    { time: 0, level: 'ok', message: 'first', count: 1 },
    { time: 1, level: 'warn', message: 'second <b>', count: 3 },
  ]);
  assert.ok(html.indexOf('second') < html.indexOf('first'), 'newest first');
  assert.match(html, /second &lt;b&gt;.*×3/s, 'escaped, with its count');
  assert.match(html, /data-action="clear-log"/);
  store.clearLog();
  assert.equal(store.log.length, 0);
  assert.match(renderLogEntries(store.log), /No messages yet/);
});

// Saved state and species toggles

test('a rectangle does not catch secondary nodes the Cofactors toggle has hidden', async () => {
  const store = await makeStore();
  store.map.setNodePrimary('met:demo@a_c', false);
  const everything = { x: -1e6, y: -1e6, w: 2e6, h: 2e6 };
  store.setView({ showCurrency: true });
  assert.ok(store.selectInRect(everything).nodes.has('met:demo@a_c'));
  store.setView({ showCurrency: false });
  const hidden = store.selectInRect(everything);
  assert.equal(hidden.nodes.has('met:demo@a_c'), false, 'Delete must not remove what nobody could see');
  assert.ok(hidden.nodes.has('met:demo@a_e'), 'primary metabolites are still caught');
});

test('Saved also follows pathways, flux colours, layout, colour mode and ribbons', async () => {
  const store = await makeStore();
  assert.deepEqual(store.unsavedWork(), { map: false, scenario: false, model: false });

  store.setView({ fluxColorStops: [{ value: 0, color: '#000000' }, { value: 5, color: '#ff0000' }] });
  assert.equal(store.unsavedWork().map, true, 'flux colours are in the map file');
  store.markMapSaved();
  assert.equal(store.unsavedWork().map, false);

  store.setView({ reactionColorMode: 'subsystem' });
  assert.equal(store.unsavedWork().scenario, true, 'the colour mode is in the scenario file');
  store.setView({ pathwayRibbonOpacity: 0.3 });
  store.markScenarioSaved();
  assert.deepEqual(store.unsavedWork(), { map: false, scenario: false, model: false });

  store.setView({ fluxColorStops: [{ value: 0, color: '#000000' }, { value: 9, color: '#00ff00' }] });
  store.markScenarioSaved();
  assert.equal(store.unsavedWork().map, false, 'the scenario file carries the flux colours too');
});

test('asking whether the work is saved changes nothing', async () => {
  const store = await makeStore();
  const before = structuredClone(store.savedWork);
  store.medium.a = 3;   // changed without any announcement
  assert.equal(store.unsavedWork().scenario, true);
  store.unsavedWork();
  assert.deepEqual(store.savedWork, before, 'the baseline is not moved by a query');
});

test('switching a genome-scale species back on keeps an untouched map too', async () => {
  const store = await makePairStore();
  const big = store.models.get('audit-demo');
  const real = big.reactions;
  big.reactions = { size: 5000 };
  try {
    store.species.find((sp) => sp.id === 'other').enabled = false;
    assert.equal(store.tooLargeToAutoDraw(), true);
    assert.equal(store.anySpeciesTooLargeToAutoDraw(), true);
    assert.equal(store.keepsDrawingOnSpeciesToggle(), true, 'no full automatic layout on the way back');
  } finally {
    big.reactions = real;
  }
});

test('reopening keeps the shared-medium parts only a switched-off species used', async () => {
  const { pruneMapToModel } = await import('../src/map/reconcile-map.js');
  const store = await makePairStore();
  const map = EditableMap.fromJSON(store.map.toJSON());
  assert.ok(map.reactions.has('EX_a_s') && map.nodes.has('met:M@a'));
  const keepOut = (id) => id.startsWith('other@') || id === 'EX_a_s' || id === 'demo@EX_a_e' || id === 'M@a';
  const community = {
    merged: {
      reactions: new Map([...store.community.merged.reactions].filter(([id]) => !keepOut(id))),
      metabolites: new Map([...store.community.merged.metabolites].filter(([id]) => !keepOut(id))),
    },
  };
  const report = pruneMapToModel(map, community, { keepSpecies: new Set(['other']) });
  assert.ok(map.reactions.has('other@At'), 'its own reactions stay');
  assert.ok(map.nodes.has('met:M@a'), 'the medium node its transfer is drawn to stays');
  assert.ok(map.reactions.has('EX_a_s'), 'and the community exchange on that node');
  assert.ok(report.dropped.includes('demo@EX_a_e'), 'a missing reaction of a species that is on is still gone');
});

test('the model badge reads the compartments the community already counted', async () => {
  const store = await makeStore();
  const model = store.models.get('audit-demo');
  const original = model.compartmentIds;
  model.compartmentIds = () => { throw new Error('recomputed on a repaint'); };
  try {
    assert.equal(modelKindLabel(store).text, 'Multi-compartment · Single cell');
  } finally {
    model.compartmentIds = original;
  }
});

test('every bundled scenario and published community opens as Saved', async (t) => {
  const { readFile } = await import('node:fs/promises');
  const previousFetch = globalThis.fetch;
  globalThis.fetch = async (url) => {
    const path = new URL(url, 'http://saved-fixture.invalid/').pathname.replace(/^\//, '');
    const text = await readFile(new URL(`../${path}`, import.meta.url), 'utf8');
    return { ok: true, status: 200, json: async () => JSON.parse(text), text: async () => text };
  };
  t.after(() => { globalThis.fetch = previousFetch; });
  const catalog = JSON.parse(await readFile(new URL('../models/scenarios.json', import.meta.url), 'utf8'));
  const open = async (label, how) => {
    const store = new Store('http://saved-fixture.invalid');
    store.scenarios = catalog.scenarios;
    store.publishedCommunities = catalog.publishedCommunities;
    // Opening is what is tested; the genome-scale LPs would only add minutes.
    store.solve = () => null;
    store.runSteadyComTask = () => new Promise(() => {});
    await how(store);
    store.emit('probe');
    assert.deepEqual(store.unsavedWork(), { map: false, scenario: false, model: false }, `${label} opens as Unsaved`);
  };
  // Yeast9 and the Zuñiga case download their models; they are covered elsewhere.
  for (const scenario of catalog.scenarios.filter((item) => item.loader !== 'yeast9')) {
    await open(scenario.id, (store) => store.useScenario(scenario.id));
  }
  for (const community of catalog.publishedCommunities.filter((item) => item.id !== 'zuniga-2020-phea-rescue')) {
    await open(`published:${community.id}`, (store) => store.usePublishedCommunity(community.id));
  }
});

test('a zero-abundance joint member is held out like a switched-off one', async () => {
  const store = await makePairStore();
  store._jointProjectionSpec = {};   // what installJointProjection leaves behind
  store.community.abundances = new Map([['demo', 1], ['other', 0]]);
  assert.equal(store.speciesHeldOut('other'), true);
  assert.equal(store.speciesHeldOut('demo'), false);
  store.selectedReaction = 'other@At';
  const html = renderInspector(store);
  assert.match(html, /has zero abundance, so this reaction is held at zero/);
  assert.doesNotMatch(html, /data-action="bound-upper/);
});

test('hand edits waiting in Redo keep the drawing when a species is toggled', async () => {
  const store = await makePairStore();
  const tx = store.beginMapEdit('hand edit');
  store.map.setNodePrimary('met:demo@a_c', false);
  store.commitMapEdit(tx);
  store.undoMap();
  assert.equal(store.canUndoMap, false);
  assert.equal(store.canRedoMap, true);
  assert.equal(store.keepsDrawingOnSpeciesToggle(), true, 'a relayout would clear Redo');
});

// Export file names

test('exported files are named after the model with a short suffix', async () => {
  const { exportFileName, fileStem, EXPORT_SUFFIX } = await import('../src/ui/file-names.js');
  assert.equal(exportFileName('yeastGEM_v9.1.1', EXPORT_SUFFIX.map, 'json'), 'yeastGEM_v9.1.1_map.json');
  assert.equal(exportFileName('yeastGEM_v9.1.1', EXPORT_SUFFIX.model, 'xml'), 'yeastGEM_v9.1.1_model.xml');
  assert.equal(fileStem('Zuñiga (2020): S. elongatus + E. coli'), 'Zuniga_2020_S._elongatus_+_E._coli');
  assert.equal(fileStem('  ../  '), 'coxeter', 'nothing usable falls back to a name');
  assert.ok(fileStem('x'.repeat(200)).length <= 80);

  const single = await makeStore();
  assert.equal(single.exportBaseName(), 'audit-demo', 'one species: the model id');
  assert.equal(single.fileNameFor(EXPORT_SUFFIX.scenario, 'json'), 'audit-demo_scenario.json');

  const pair = await makePairStore();
  assert.equal(pair.exportBaseName(), 'audit-pair', 'a community: its scenario id');
  assert.equal(pair.fileNameFor(EXPORT_SUFFIX.fluxes, 'csv'), 'audit-pair_fluxes.csv');
  pair.scenario = { id: 'published:steadycom-gut-nine' };
  assert.equal(pair.exportBaseName(), 'steadycom-gut-nine', 'published communities drop the prefix');
  pair.scenario = { id: 'uploaded-models' };
  assert.equal(pair.exportBaseName(), 'audit-demo+audit-demo', 'several uploads: their model ids');
});

test('member, community-data and dated exports keep their context in the name', async () => {
  const { memberModelFileName, communityDataFileName, timestampedFileName, exportFileName } = await import('../src/ui/file-names.js');
  assert.equal(memberModelFileName('steadycom-gut-nine', 'Ec', 'json'), 'steadycom-gut-nine_Ec_model.json');
  assert.notEqual(memberModelFileName('steadycom-gut-nine', 'Ec', 'json'), memberModelFileName('zuniga-2020-phea-rescue', 'Ec', 'json'),
    'the same member id in two communities gives two names');
  assert.equal(memberModelFileName('stolyar-2007-syntrophy', 'Dv', 'xml'), 'stolyar-2007-syntrophy_Dv_model.xml');
  assert.equal(communityDataFileName('steadycom-gut-nine', 'media.csv'), 'steadycom-gut-nine_media.csv');
  assert.equal(communityDataFileName('x', 'bounds'), 'x_bounds.csv', 'a file without extension stays a CSV');
  const when = new Date(2026, 9, 10, 15, 7);
  assert.equal(timestampedFileName('consorcio-anaerobio', 'repro', 'json', when), 'consorcio-anaerobio_repro_20261010-1507.json');
  assert.equal(timestampedFileName('yeastGEM_v9.1.1', 'methods', 'md', when), 'yeastGEM_v9.1.1_methods_20261010-1507.md');
  assert.equal(exportFileName('iML1515+yeastGEM', 'map', 'json'), 'iML1515+yeastGEM_map.json', 'the + joining uploads survives');
});

async function makeTwoModelStore() {
  const store = await makeStore();
  const other = new Model({
    id: 'audit-other',
    compartments: { c: 'Cytosol', e: 'Extracellular' },
    metabolites: [{ id: 'a_e', compartment: 'e' }, { id: 'a_c', compartment: 'c' }],
    reactions: [
      { id: 'EX_a_e', metabolites: { a_e: -1 }, lower_bound: -10, upper_bound: 1000 },
      { id: 'At', metabolites: { a_e: -1, a_c: 1 }, lower_bound: 0, upper_bound: 1000 },
      { id: 'BIOMASS', metabolites: { a_c: -1 }, lower_bound: 0, upper_bound: 1000, objective_coefficient: 1 },
    ],
  });
  store.models.set('audit-other', other);
  store.scenarios.push({
    id: 'audit-two', name: 'Two models', objective: 'equal-growth',
    species: [{ id: 'demo', model: 'audit-demo', abundance: 0.5 }, { id: 'oth', model: 'audit-other', abundance: 0.5 }],
    medium: { a: 10 },
  });
  await store.useScenario('audit-two');
  return store;
}

test('exporting one model leaves another model\'s edits unsaved', async () => {
  const store = await makeTwoModelStore();
  store.setModelEditorSpecies('demo');
  await store.createModelMetabolite({ id: 'b_c', name: 'B', compartment: 'c' });
  store.setModelEditorSpecies('oth');
  await store.createModelMetabolite({ id: 'b_c', name: 'B', compartment: 'c' });
  store.markModelSaved(store.models.get('audit-demo'));
  assert.equal(store.unsavedWork().model, true, 'the other model was not written out');
  store.markModelSaved(store.models.get('audit-other'));
  assert.equal(store.unsavedWork().model, false);
});

test('reopening a scenario does not hide edits to a model it reuses', async () => {
  const store = await makeStore();
  store.modelEditorSpeciesId = 'demo';
  await store.createModelMetabolite({ id: 'b_c', name: 'B', compartment: 'c' });
  assert.equal(store.unsavedWork().model, true);
  await store.useScenario('audit-demo');   // the cached, edited model is used again
  assert.equal(store.unsavedWork().model, true, 'the edit is still in memory and still unsaved');
});

test('an exported map does not keep the cross-feeding event picked in the explorer', async () => {
  const store = await makePairStore();
  const link = { from: 'demo', to: 'other', base: 'a', flux: 1 };
  const state = {
    fluxes: store.displayFluxes(), scale: store.scale, showCrossFeeding: true,
    crossFeeding: { links: [link], perMetabolite: [] },
    crossFeedingExplorer: { enabled: true, visibleEvents: [link], selectedEvent: link },
  };
  // a routed line carries its opacity on its group, an arc on the path itself;
  // on screen the picked one also carries is-picked
  const opacity = (svg) => Number(/class="emc-crossfeed(?:-route)?(?: [^"]*)?"[^>]*opacity="([\d.]+)"/.exec(svg)?.[1]);
  const onScreen = opacity(renderMap(store.layout, state));
  const inFigure = opacity(renderMap(store.layout, { ...state, forExport: true }));
  assert.ok(onScreen > inFigure, 'on screen the picked event stands out; in the figure it is one event among the others');
});
