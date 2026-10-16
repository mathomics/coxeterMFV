import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

import { Store } from '../src/ui/store.js';
import { renderInspector, renderResults } from '../src/ui/panels.js';
import { MetaboliteMenuController } from '../src/ui/metabolite-menu.js';
import { CompartmentMenuController } from '../src/ui/compartment-menu.js';

test('application shell exposes the mockup-aligned work areas and analysis menu', async () => {
  const html = await readFile(new URL('../index.html', import.meta.url), 'utf8');
  for (const id of ['left-panel', 'stage', 'results-panel', 'inspector-panel']) {
    assert.match(html, new RegExp(`id="${id}"`));
  }
  for (const action of ['run-fba', 'run-pfba', 'run-fva', 'run-loopless']) {
    assert.match(html, new RegExp(`data-action="${action}"`));
  }
  assert.match(html, /data-action="tool-edit" data-tool="edit"/);
  assert.match(html, /data-action="tool-reaction" data-tool="reaction"[^>]*>[\s\S]*Add Reaction/);
  // The menu offers only what the application can actually run. Listing
  // unbuilt analyses as disabled "Planned" rows would advertise capability the
  // user cannot reach, so the menu ends at the four working ones.
  assert.doesNotMatch(html, /Planned/);
  assert.doesNotMatch(html, /Knockout analysis/);
  assert.doesNotMatch(html, /Sampling \/ MEMOTE/);
});

test('Edit Map Metabolite tool stays in Edit Map and does not open the biological Edit Model workspace', async () => {
  const source = await readFile(new URL('../src/ui/app.js', import.meta.url), 'utf8');
  const match = source.match(/case 'tool-metabolite':[\s\S]*?break;/);
  assert.ok(match, 'tool-metabolite handler is missing');
  const block = match[0];

  assert.match(block, /closeWorkspace\(\{ restoreFocus: false \}\)/);
  assert.match(block, /activateTool\('metabolite'\)/);
  assert.match(block, /ui\.leftTab = 'build'/);
  assert.doesNotMatch(block, /ui\.leftTab = 'model'/);
  assert.doesNotMatch(block, /openWorkspace\('model'/);
  assert.doesNotMatch(block, /setModelEditorMode\('metabolite'\)/);
});

test('Scenario return waits for the selected community rebuild and repaints the map', async () => {
  const source = await readFile(new URL('../src/ui/app.js', import.meta.url), 'utf8');

  const scenario = source.match(/case 'scenario':[\s\S]*?break;/);
  assert.ok(scenario, 'scenario handler is missing');
  assert.match(scenario[0], /event\.type === 'change'/);
  assert.match(scenario[0], /queueScenarioSelection\(el\.value\)/);
  assert.doesNotMatch(scenario[0], /store\.useScenario\(id\)/);
  assert.doesNotMatch(scenario[0], /store\.usePublishedCommunity\(id\)/);

  const closeStart = source.indexOf("case 'close-workspace':");
  const closeEnd = source.indexOf("case 'toggle-focus':", closeStart);
  assert.ok(closeStart >= 0 && closeEnd > closeStart, 'close-workspace handler is missing');
  const close = source.slice(closeStart, closeEnd);
  assert.match(close, /ui\.workspace === 'scenario'/);
  assert.match(close, /select\[data-action="scenario"\]/);
  assert.match(close, /selectedValue !== activeValue/);
  assert.match(close, /await queueScenarioSelection\(selectedValue\)/);
  assert.match(close, /await pendingScenarioLoad/);
  assert.match(close, /camera\.fitted = false/);
  assert.match(close, /paintMap\(\)/);

  assert.match(source, /let scenarioLoadQueue = Promise\.resolve\(\)/);
  assert.match(source, /scenarioLoadQueue\.then\(async \(\) =>/);
});

test('published community without bundled models clears the previous active map', async () => {
  const store = new Store('.');
  store.publishedCommunities = [{
    id: 'missing-models',
    name: 'Missing models community',
    description: 'No bundled model files',
  }];
  store.publishedManifest = async () => null;
  store.community = { name: 'Old community' };
  store.problem = { old: true };
  store.map = { old: true };
  store.solution = { status: 'optimal' };
  store.scale = { old: true };

  await store.usePublishedCommunity('missing-models');

  assert.equal(store.scenario.id, 'published:missing-models');
  assert.equal(store.community, null);
  assert.equal(store.problem, null);
  assert.equal(store.map, null);
  assert.equal(store.solution, null);
  assert.equal(store.scale, null);
  assert.match(store.status.message, /no model files are bundled/i);
});

test('Shared Medium boundary starts a geometry resize drag instead of panning', async () => {
  const source = await readFile(new URL('../src/ui/app.js', import.meta.url), 'utf8');
  assert.match(source, /function sharedMediumBoundaryHandleAt\(target, event\)/);
  assert.match(source, /nearestRectEdge\([\s\S]*?store\.layout\.environment\.rect/);
  assert.match(source, /!store\.view\.geometryEdit && ui\.tool === 'edit'/);
  assert.match(source, /const sharedMediumHandle = sharedMediumBoundaryHandleAt\(event\.target, event\)/);
  assert.match(source, /beginGeometryDrag\(sharedMediumHandle, event\)/);
  assert.match(source, /store\.selectGeometry\(sharedMediumHandle\.cellId, sharedMediumHandle\.compartmentId\)/);
});

test('Select moves and resizes compartments from their edge, not their body', async () => {
  const source = await readFile(new URL('../src/ui/app.js', import.meta.url), 'utf8');

  assert.match(source, /function compartmentBoundaryHandleAt\(target, event\)/);
  assert.match(source, /store\.layout\.geometryTarget\(boundary\.cellId, boundary\.compartmentId\)/);
  assert.match(source, /nearestRectEdge\([\s\S]*?rect,[\s\S]*?screenToMap\(event\.clientX, event\.clientY\)/);

  // A compartment is moved from its handles and resized from its membrane. Its
  // body is not a grab target: the space inside it belongs to what is drawn
  // there, which is what a press inside a compartment is usually aiming at.
  assert.doesNotMatch(source, /function compartmentMoveHandleAt\(/);

  const selectStart = source.indexOf("if (!store.view.geometryEdit && ui.tool === 'select'");
  const selectEnd = source.indexOf("// The normal Edit tool must also let the user grab the Shared Medium", selectStart);
  assert.ok(selectStart >= 0 && selectEnd > selectStart, 'Select compartment geometry handler is missing');
  const selectBlock = source.slice(selectStart, selectEnd);
  assert.match(selectBlock, /compartmentBoundaryHandleAt\(event\.target, event\)/);
  // Three ways in, in order of precision: a corner handle, the shared medium's
  // membrane, a compartment's membrane.
  assert.match(selectBlock, /geometryHandleAt\(event\.target\)/);
  assert.match(selectBlock, /sharedMediumBoundaryHandleAt\(event\.target, event\)/);
  assert.doesNotMatch(selectBlock, /compartmentMoveHandleAt/);
  assert.equal((selectBlock.match(/beginGeometryDrag\(/g) ?? []).length, 3);
  // What is drawn inside a compartment keeps its own behaviour: a compartment
  // is an ancestor of every metabolite in it, so without this the compartment
  // moved instead of the metabolite.
  assert.match(selectBlock, /!mapContentAt\(event\.target\)|mapContentAt/);

  // A click must remain a click; only a deliberate drag edits geometry.
  assert.match(source, /startClient: \{ x: event\.clientX, y: event\.clientY \}/);
  assert.match(source, /const dragDistance = Math\.hypot\(/);
  assert.match(source, /if \(!g\.moved && dragDistance < 3\) return;/);

  // A no-drag click selects the compartment, from its body or from its
  // membrane alike, and shows its handles. The membrane's list of reactions to
  // add belongs to Add Reaction — opening it from Select turned an ordinary
  // selection into a menu nobody asked for.
  assert.match(source, /geometryGesture\.source === 'boundary'/);
  assert.match(source, /store\.selectGeometry\(geometryGesture\.cellId \?\? null, geometryGesture\.compartmentId\)/);
  // selectGeometry records the compartment as the selected entity itself.
  // selectEntity clears selectedGeometry, so calling it afterwards took the
  // five handles away again the moment the click had earned them.
  assert.doesNotMatch(source, /store\.selectEntity\('compartment', geometryGesture\.compartmentId/);
  const boundaryMenuOpens = source.match(/boundaryMenu\.open\(/g) ?? [];
  assert.equal(boundaryMenuOpens.length, 1,
    'the boundary reaction menu should be opened from one place only: the Add Reaction tool');
  assert.match(source, /if \(ui\.tool === 'reaction'\) \{\r?\n      boundaryMenu\.open\(/);
});

test('contextual selection keeps scientific state separate from visual entity state', () => {
  const store = new Store('.');
  let reason = null;
  store.subscribe((_, nextReason) => { reason = nextReason; });

  store.selectEntity('metabolite', 'node-1');
  assert.deepEqual(store.selectedEntity, { type: 'metabolite', id: 'node-1' });
  assert.equal(store.selectedReaction, null);
  assert.equal(reason, 'selection');

  store.select('reaction-1');
  assert.deepEqual(store.selectedEntity, { type: 'reaction', id: 'reaction-1' });
  assert.equal(store.selectedReaction, 'reaction-1');

  store.select(null);
  assert.equal(store.selectedEntity, null);
});

test('metabolite inspector reports available model and map properties', () => {
  const node = {
    id: 'node-1', mergedId: 'eco@glc__D_c', base: 'glc__D', compartment: 'c',
    kind: 'metabolite', primary: true, r: 12,
  };
  const metabolite = { id: 'eco@glc__D_c', name: 'D-Glucose', formula: 'C6H12O6', charge: 0 };
  const store = {
    selectedReaction: null,
    selectedEntity: { type: 'metabolite', id: node.id },
    map: { nodes: new Map([[node.id, node]]), segmentsForNode: () => [{}, {}] },
    community: { merged: { metabolites: new Map([[node.mergedId, metabolite]]) } },
    species: [{ id: 'eco', file: 'ecoli' }],
    models: new Map([['ecoli', { cell: { name: 'E. coli' } }]]),
    canRemoveMapNode: () => false,
  };

  const html = renderInspector(store);
  assert.match(html, /D-Glucose/);
  assert.match(html, /C6H12O6/);
  assert.match(html, /E\. coli/);
  assert.match(html, /Connected paths[\s\S]*2/);
  assert.match(html, /data-action="metabolite-primary"/);
  assert.match(html, /data-action="metabolite-secondary"/);
  assert.match(html, /data-action="duplicate-metabolite"/);
  assert.match(html, /data-action="hide-metabolite"/);
});

test('Add Reaction panel shows reaction stoichiometry before eight in-panel directions and reuses the selected metabolite', () => {
  const node = {
    id: 'met:eco@PGI:eco@g6p_c',
    mergedId: 'eco@g6p_c',
    base: 'g6p_c',
    name: 'D-Glucose 6-phosphate',
    kind: 'metabolite',
    x: 100,
    y: 120,
  };
  const productId = 'eco@6pgl_c';
  const reactionId = 'eco@G6PDH2r';
  const reaction = {
    name: 'Glucose 6-phosphate dehydrogenase',
    subsystem: 'Pentose phosphate pathway',
    lowerBound: 0,
    upperBound: 1000,
    metabolites: { [node.mergedId]: -1, [productId]: 1 },
  };
  const stage = {
    clientWidth: 900,
    clientHeight: 700,
    addEventListener() {},
    getBoundingClientRect() { return { left: 0, top: 0 }; },
  };
  let added = null;
  const store = {
    map: { nodes: new Map([[node.id, node]]) },
    community: {
      merged: {
        reactions: new Map([[reactionId, reaction]]),
        metabolites: new Map([
          [node.mergedId, { id: node.mergedId, base: 'g6p_c' }],
          [productId, { id: productId, base: '6pgl_c' }],
        ]),
      },
      reactionOwner: new Map([[reactionId, { originalId: 'G6PDH2r' }]]),
      metaboliteOwner: new Map([
        [node.mergedId, { originalId: 'g6p_c' }],
        [productId, { originalId: '6pgl_c' }],
      ]),
    },
    reactionsAvailableFromMetabolite: () => [reactionId],
    addMapReaction: (id, opts) => { added = { id, opts }; return {}; },
  };

  const menu = new MetaboliteMenuController({ stage, store, onChange() {} });
  menu.open(node.id, { clientX: 200, clientY: 160 });

  const html = menu.render();
  assert.match(html, /G6PDH2r/);
  assert.match(html, /g6p_c → 6pgl_c/);
  assert.equal((html.match(/data-add-rxn-direction=/g) ?? []).length, 8);
  assert.match(html, /data-select-add-rxn="eco@G6PDH2r"/);

  const east = {
    dataset: { addRxnDirection: 'e' },
    closest(selector) { return selector === '[data-add-rxn-direction]' ? this : null; },
  };
  menu.handleClick({ target: east, preventDefault() {}, stopPropagation() {} });

  assert.equal(added.id, reactionId);
  assert.equal(added.opts.anchor.x, 196);
  assert.equal(added.opts.anchor.y, 120);
  assert.equal(added.opts.anchorNodeId, node.id);
  assert.deepEqual(added.opts.placementDirection, { x: 1, y: 0 });
});

test('the Add Reaction panel is never taller than the stage, so its direction wheel is always in reach', async () => {
  const { panelPlacement, renderAddReactionPanel } = await import('../src/ui/add-reaction-panel.js');
  // A laptop: the stage is shorter than the panel at its full height.
  const laptop = { clientWidth: 820, clientHeight: 430 };
  const place = panelPlacement(laptop, { left: 600, top: 380 });
  assert.ok(place.maxHeight <= laptop.clientHeight - 16, 'it is cut to the stage');
  assert.ok(place.top >= 8 && place.top + place.maxHeight <= laptop.clientHeight - 8, 'and wholly on it');
  assert.ok(place.left >= 8 && place.left + place.width <= laptop.clientWidth - 8);

  // A roomy stage changes nothing: it opens beside the click at its full height.
  assert.deepEqual(panelPlacement({ clientWidth: 1400, clientHeight: 900 }, { left: 212, top: 172 }),
    { width: 380, maxHeight: 520, left: 212, top: 172 });

  const store = {
    community: {
      merged: { reactions: new Map([['eco@R', { name: 'r', metabolites: { 'eco@a_c': -1, 'eco@b_c': 1 }, lowerBound: 0, upperBound: 10 }]]), metabolites: new Map() },
      reactionOwner: new Map([['eco@R', { originalId: 'R' }]]),
      metaboliteOwner: new Map(),
    },
  };
  const html = renderAddReactionPanel({ store, reactionIds: ['eco@R'], selectedReactionId: 'eco@R', ...place });
  assert.match(html, new RegExp(`max-height:${place.maxHeight}px`));
  // The wheel sits beside its caption, inside the one section that does not scroll.
  assert.match(html, /<div class="add-reaction-direction-section">\s*<div class="add-reaction-direction-text">[\s\S]*<\/div>\s*<div class="add-reaction-direction-wheel"/);
  assert.equal((html.match(/data-add-rxn-direction=/g) ?? []).length, 8);
  assert.ok(html.indexOf('add-reaction-list') < html.indexOf('add-reaction-direction-section'));
});

test('the Add Reaction panel can be dragged by its header and stays where it is put', async () => {
  const { beginPanelDrag, panelPlacement } = await import('../src/ui/add-reaction-panel.js');
  const stage = { clientWidth: 900, clientHeight: 500 };
  const controller = { stage, state: { left: 100, top: 60 } };
  const listeners = {};
  const menu = {
    offsetLeft: 100, offsetTop: 60, style: {},
    classList: { add() {}, remove() {} },
  };
  const head = {
    closest: (selector) => (selector === '.add-reaction-head' ? head : selector === '.add-reaction-menu' ? menu : null),
    setPointerCapture() {}, releasePointerCapture() {},
    addEventListener: (type, fn) => { listeners[type] = fn; },
    removeEventListener: (type) => { delete listeners[type]; },
  };
  const press = (target) => ({ target, pointerType: 'mouse', button: 0, pointerId: 1, clientX: 300, clientY: 200, preventDefault() {} });

  // A press on a control in the header is that control's, not a drag.
  const closeButton = { closest: (selector) => (selector === '.add-reaction-head' ? head : selector.startsWith('button') ? closeButton : null) };
  assert.equal(beginPanelDrag(controller, press(closeButton)), false);
  // Nor is a press anywhere when no panel of this controller is open.
  assert.equal(beginPanelDrag({ stage, state: null }, press(head)), false);

  assert.equal(beginPanelDrag(controller, press(head)), true);
  listeners.pointermove({ clientX: 420, clientY: 330 });
  assert.deepEqual([controller.state.left, controller.state.top, controller.state.moved], [220, 190, true]);
  assert.deepEqual([menu.style.left, menu.style.top], ['220px', '190px']);

  // It may be pushed part of the way off the stage, but never out of reach.
  listeners.pointermove({ clientX: 5000, clientY: 5000 });
  assert.ok(controller.state.left <= stage.clientWidth - 64 && controller.state.top <= stage.clientHeight - 40);
  listeners.pointermove({ clientX: -5000, clientY: -5000 });
  assert.ok(controller.state.left + 380 >= 64 && controller.state.top >= 0);

  listeners.pointermove({ clientX: 380, clientY: 260 });
  listeners.pointerup({ pointerId: 1 });
  assert.equal(listeners.pointermove, undefined, 'the drag ends with the release');
  // The next repaint draws it where it was left, not back beside the click.
  const place = panelPlacement(stage, controller.state);
  assert.deepEqual([place.left, place.top], [180, 120]);
});

test('compartment Add Reaction panel uses the same reaction preview and directions without linking an existing metabolite', () => {
  const reactionId = 'eco@PSERT';
  const reaction = {
    name: 'phosphoserine transaminase',
    lowerBound: 0,
    upperBound: 1000,
    metabolites: { 'eco@pser__L_c': -1, 'eco@akg_c': -1, 'eco@3php_c': 1, 'eco@glu__L_c': 1 },
  };
  const stage = {
    clientWidth: 900,
    clientHeight: 700,
    addEventListener() {},
    getBoundingClientRect() { return { left: 0, top: 0 }; },
  };
  let added = null;
  const store = {
    map: {
      cells: [{
        id: 'eco',
        compartments: [{ id: 'c', label: 'Cytosol' }],
      }],
    },
    community: {
      merged: {
        reactions: new Map([[reactionId, reaction]]),
        metabolites: new Map(),
      },
      reactionOwner: new Map([[reactionId, { originalId: 'PSERT' }]]),
      metaboliteOwner: new Map([
        ['eco@pser__L_c', { originalId: 'pser__L_c' }],
        ['eco@akg_c', { originalId: 'akg_c' }],
        ['eco@3php_c', { originalId: '3php_c' }],
        ['eco@glu__L_c', { originalId: 'glu__L_c' }],
      ]),
    },
    reactionsAvailableInCompartment: () => [reactionId],
    addMapReaction: (id, opts) => { added = { id, opts }; return {}; },
  };
  const menu = new CompartmentMenuController({
    stage,
    store,
    screenToMap: () => ({ x: 300, y: 200 }),
    onChange() {},
  });

  menu.open('eco', 'c', { clientX: 250, clientY: 220 });
  const html = menu.render();
  assert.match(html, /Add reaction/);
  assert.match(html, /PSERT/);
  assert.match(html, /pser__L_c \+ akg_c → 3php_c \+ glu__L_c/);
  assert.equal((html.match(/data-add-rxn-direction=/g) ?? []).length, 8);

  const east = {
    dataset: { addRxnDirection: 'e' },
    closest(selector) { return selector === '[data-add-rxn-direction]' ? this : null; },
  };
  menu.handleClick({ target: east, preventDefault() {}, stopPropagation() {} });

  assert.equal(added.id, reactionId);
  assert.equal(added.opts.anchor.x, 396);
  assert.equal(added.opts.anchor.y, 200);
  assert.deepEqual(added.opts.placementDirection, { x: 1, y: 0 });
  assert.equal(Object.prototype.hasOwnProperty.call(added.opts, 'anchorNodeId'), false);
});

test('results panel uses real solver output and leaves unsupported values disabled', () => {
  const reaction = { name: 'Phosphofructokinase' };
  const store = {
    solution: { status: 'optimal', objectiveValue: 0.87392, fluxes: new Map([['eco@PFK', 6.32]]) },
    loopless: false,
    parsimonious: false,
    fvaEnabled: false,
    fvaRanges: new Map(),
    fvaProgress: { running: false, done: 0, total: 0, ms: 0 },
    community: {
      reactionOwner: new Map([['eco@PFK', { originalId: 'PFK' }]]),
      merged: { reactions: new Map([['eco@PFK', reaction]]) },
    },
    status: { solveMs: 12, message: '' },
  };

  const html = renderResults(store);
  assert.match(html, /0\.87392/);
  assert.match(html, /PFK/);
  assert.match(html, /6\.32/);
  assert.match(html, /Reduced costs<\/button>/);
  assert.match(html, /disabled title="Not returned by the current solver">Shadow prices/);
  assert.doesNotMatch(html, /data-rxn=/);
});

test('Select moves a text annotation, as it moves everything else on the map', async () => {
  const source = await readFile(new URL('../src/ui/app.js', import.meta.url), 'utf8');
  // Only Edit moved one: Select went over to moving things, and annotations
  // were left behind, so a press on one under Select did nothing at all.
  assert.match(source, /const textId = mapTextAt\(event\.target\);[\s\S]{0,400}if \(movesThings\(\)\) beginTextDrag\(textId, event\);/);
  assert.doesNotMatch(source, /if \(ui\.tool === 'edit'\) beginTextDrag/);
});

test('Escape drops a selected compartment in Select as well as in Geometry Edit', async () => {
  const source = await readFile(new URL('../src/ui/app.js', import.meta.url), 'utf8');
  const start = source.indexOf("if (event.key === 'Escape') {\n    if (textEditor)".replace(/\n/g, source.includes('\r\n') ? '\r\n' : '\n'));
  assert.ok(start >= 0, 'the Escape handler is missing');
  const block = source.slice(start, source.indexOf('return;\n  }'.replace(/\n/g, source.includes('\r\n') ? '\r\n' : '\n'), start));
  // Its handles show under Select too, and stayed after Escape there.
  assert.match(block, /^\s*store\.clearGeometrySelection\(\);$/m);
  assert.doesNotMatch(block, /if \(store\.view\.geometryEdit\) store\.clearGeometrySelection\(\)/);
});

test('undo and redo wait for a drag in progress to finish', async () => {
  const source = await readFile(new URL('../src/ui/app.js', import.meta.url), 'utf8');
  // Undo replaces the map the drag is editing, and the drag then committed
  // against the snapshot it started from: undoing it brought back the edit
  // just undone. The guard comes before either shortcut acts.
  const guard = source.search(/if \(mod && \(event\.key\.toLowerCase\(\) === 'z' \|\| event\.key\.toLowerCase\(\) === 'y'\) && mapDragInProgress\(\)\) \{\s*event\.preventDefault\(\);\s*return;/);
  assert.ok(guard >= 0, 'no guard for a drag in progress');
  const undo = source.indexOf("if (mod && event.key.toLowerCase() === 'z') {");
  const redo = source.indexOf("if (mod && event.key.toLowerCase() === 'y') {");
  assert.ok(guard < undo && guard < redo);
  const body = source.slice(source.indexOf('function mapDragInProgress()'), source.indexOf('function mapDragInProgress()') + 600);
  for (const g of ['nodeDragGesture', 'reactionDragGesture', 'geometryDragGesture', 'groupDragGesture', 'textDragGesture', 'rotateDragGesture', 'controlDragGesture']) {
    assert.match(body, new RegExp(`\\b${g}\\b`), g);
  }
});

test('a drag on the map is never taken over by the browser\'s own drag-and-drop', async () => {
  const source = await readFile(new URL('../src/ui/app.js', import.meta.url), 'utf8');
  const css = await readFile(new URL('../assets/app.css', import.meta.url), 'utf8');
  // A text selection left on the map's labels made the next press inside it a
  // native drag, and its pointercancel ended the handle drag after one move.
  assert.match(css, /\.stage > svg \{ user-select: none; -webkit-user-select: none; \}/);
  assert.match(source, /els\.stage\.addEventListener\('dragstart', \(event\) => event\.preventDefault\(\)\);/);
});

test('opening one dropdown closes the others, so File and View do not stack', async () => {
  const source = await readFile(new URL('../src/ui/app.js', import.meta.url), 'utf8');
  const html = await readFile(new URL('../index.html', import.meta.url), 'utf8');
  for (const menu of ['top-menu file-menu', 'top-menu analysis-menu', 'canvas-view']) {
    assert.match(html, new RegExp(`<details class="${menu}">`), menu);
  }
  const at = source.indexOf("document.addEventListener('toggle'");
  assert.ok(at >= 0, 'nothing listens for a dropdown opening');
  const body = source.slice(at, at + 400);
  assert.match(body, /if \(!menu\.open \|\| !menu\.matches\?\.\('\.canvas-view, \.top-menu'\)\) return;/);
  assert.match(body, /if \(other !== menu\) other\.open = false;/);
  assert.match(body, /\}, true\);/, 'toggle does not bubble: it has to be caught on the way down');
});
