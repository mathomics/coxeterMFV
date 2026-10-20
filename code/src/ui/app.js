/**
 * Application bootstrap: mounts the map, wires the sidebar, and keeps zoom and
 * pan on the SVG viewBox so the drawing stays crisp at any scale.
 */

import { Store } from './store.js';
import {
  renderTaskPanel,
  renderWorkspace,
  renderViewControls,
  getWorkspaceTitle,
  renderInspector,
  renderLayoutSettings,
  renderResults,
  renderReactionSearchResults,
  renderCrossFeedingEvents,
  renderFluxColorPreview,
  modelKindLabel,
} from './panels.js';
import { statusLine } from './status-line.js';
import {
  EXPORT_SUFFIX, exportFileName, timestampedFileName, memberModelFileName, communityDataFileName,
} from './file-names.js';
import { BoundaryMenuController } from './boundary-menu.js';
import { MetaboliteMenuController } from './metabolite-menu.js';
import { CompartmentMenuController } from './compartment-menu.js';
import { MetaboliteAddMenuController } from './metabolite-add-menu.js';
import { renderMap, rotationArm, frameLabelScale, geometryHandleSide } from '../render/mapview.js';
import { formatFlux } from '../render/theme.js';
import { rasterScales, PREFERRED_SCALE } from '../render/raster-size.js';
import { TEXT_FONTS, TEXT_SIZE, clampTextSize, textFontStack } from '../core/text-style.js';
import { stageToMap } from '../layout/drag.js';
import {
  nearestRectEdge, rectFromHandle as geometryRectFromHandle,
  SHAPE_HANDLES, shapeAmountAt, shapeHandlePoint,
} from '../map/geometry-edit.js';
import { COMPARTMENT_SHAPES, compartmentShape } from '../layout/geometry.js';
import { beginPinch, updatePinch, zoomCameraAt } from './camera-zoom.js';
import { metaboliteLabelAngle } from '../map/metabolite-label.js';
import { compartmentKey } from '../map/multi-selection.js';
import { createWheelNavigation, wheelPanDelta } from './wheel-navigation.js';
import { saveFile } from './save-file.js';
import { signLogPosition, signLogValue } from './reactions-explorer.js';
import { RX_ROW_HEIGHT, formatBound, reactionColumns, renderReactionRows, renderReactionsRegion } from './reactions-panel.js';
import { OBJECTIVES } from '../core/community.js';
import { isBinaryModelFile } from '../core/model-import.js';

const els = {
  app: document.getElementById('app-shell'),
  leftPanel: document.getElementById('left-panel'),
  sidebar: document.getElementById('sidebar'),
  taskPanel: document.getElementById('task-panel'),
  workspaceEditor: document.getElementById('workspace-editor'),
  workspaceTitle: document.getElementById('workspace-title'),
  viewControls: document.getElementById('view-controls'),
  resultsSummary: document.getElementById('results-summary'),
  themeLabel: document.getElementById('theme-label'),
  activeToolLabel: document.getElementById('active-tool-label'),
  inspectorPanel: document.getElementById('inspector-panel'),
  inspector: document.getElementById('inspector'),
  resultsPanel: document.getElementById('results-panel'),
  results: document.getElementById('results-content'),
  resultsResize: document.querySelector('.results-resize'),
  stage: document.getElementById('stage'),
  modelName: document.getElementById('model-name'),
  modelKind: document.getElementById('model-kind'),
  saveState: document.getElementById('save-state'),
  zoomReadout: document.getElementById('zoom-readout'),
  status: document.getElementById('status'),
  statusText: document.getElementById('status-text'),
  fileInput: document.getElementById('file-input'),
  mapFileInput: document.getElementById('map-file-input'),
  waitingMapItem: document.getElementById('waiting-map-item'),
  modelFileInput: document.getElementById('model-file-input'),
  omicsFileInput: document.getElementById('omics-file-input'),
  metatranscriptomicsFileInput: document.getElementById('metatranscriptomics-file-input'),
  reproducibilityFileInput: document.getElementById('reproducibility-file-input'),
  boundsFileInput: document.getElementById('bounds-file-input'),
};

const ui = {
  leftTab: 'build',
  resultsTab: 'analysis',
  analysisTab: 'flux',
  tool: 'select',
  workspace: null,
  lastWorkspace: 'library',
  returnFocus: null,
  drafts: new Map(),
  fluxColorDraft: null,
  fluxColorSource: undefined,
  fluxColorsCommitting: false,
  steadyComDraft: null,
  steadyComPending: false,
  steadyComChangingCondition: false,
  // Set once the user works the right panel by hand.
  inspectorPinned: false,
  rightTab: 'inspector',
  // The Reactions page and the dock's Table view: the table model of the last
  // store change, scroll positions, whether the map shows above the page, the
  // bulk bar's typed values and the row the pointer is on.
  rx: {
    model: null, revision: -1, pageScroll: 0, compactScroll: 0, split: true, showChanges: false,
    bulk: { lower: '', upper: '', factor: '0.5' }, hover: null, compactMode: 'reactions',
  },
};

// Bumped by every store change that can change a table row; the table model is
// rebuilt once per revision, not once per paint or scroll.
let storeRevision = 0;

const store = new Store('.');
store.view.dark = document.documentElement.dataset.theme === 'dark';
const camera = { x: 0, y: 0, scale: 1, fitted: false };
let pendingSolve = null;

const boundaryMenu = new BoundaryMenuController({
  stage: els.stage,
  store,
  screenToMap: (x, y) => screenToMap(x, y),
  onChange: () => paintMap(),
});

const metaboliteMenu = new MetaboliteMenuController({
  stage: els.stage,
  store,
  onChange: () => paintMap(),
});

const compartmentMenu = new CompartmentMenuController({
  stage: els.stage,
  store,
  screenToMap: (x, y) => screenToMap(x, y),
  onChange: () => paintMap(),
});

const metaboliteAddMenu = new MetaboliteAddMenuController({
  stage: els.stage, store,
  screenToMap: (x,y)=>screenToMap(x,y),
  onChange: ()=>paintMap(),
});

// Rendering

function paintMap() {
  if (!store.layout || !store.scale) {
    els.stage.innerHTML = '<p style="padding:30px;color:var(--ink-3)">No model to display.</p>';
    return;
  }
  const svg = renderMap(store.layout, mapRenderState());
  // The text editor lives in the stage about to be replaced: note its focus first.
  captureTextEditorFocus();
  // Every menu controller that can be open has to be rendered here, or it opens
  // in state and never on screen, and its button appears to do nothing.
  els.stage.innerHTML = svg + emptyMapOverlay() + selectionOverlay() + cellDrawOverlay()
    + compartmentParentOverlay() + compartmentDrawOverlay() + boundaryMenu.render()
    + metaboliteMenu.render() + compartmentMenu.render() + metaboliteAddMenu.render();
  const node = els.stage.querySelector('svg');
  if (!camera.fitted) fitToStage(node);
  hitScaleApplied = camera.scale;
  applyCamera();
  markDraggedNode();
  markDraggedMetaboliteLabel();
  renderTextEditor();
  if (ui.rx.hover) highlightReactionOnMap(ui.rx.hover);
}

/** Everything the renderer needs to draw the map as it stands. */
function mapRenderState() {
  const reactionColoring = store.reactionColoring();
  const ribbonColoring = store.pathwayRibbonColoring();
  const omicsOverlay = store.omicsOverlay();
  const communityOmicsEnabled = Boolean(store.omics?.enabled && store.omics?.dataset);
  // Both imports retain their own settings. Reaction overlays take precedence
  // for matching reactions; community expression fills the remaining colors.
  const omicsColors = new Map(communityOmicsEnabled ? store.omics.colors : []);
  if (omicsOverlay.enabled) {
    for (const [reactionId, color] of omicsOverlay.colors) omicsColors.set(reactionId, color);
  }
  // Bounds changed from the model's: knocked-out reactions take the disabled
  // style, the others a badge.
  const changes = store.constraintChanges?.() ?? [];
  return {
    fluxes: store.displayFluxes(),
    solvedFluxes: store.solution?.status === 'optimal' ? store.solution.fluxes : new Map(),
    reactionWidthMode: store.isZunigaStudy || store.view.widthByFlux ? 'flux' : 'constant',
    // Grey and dashed only against a solution: with none, nothing carries flux.
    offStyle: store.view.dashNoFlux !== false && store.solution?.status === 'optimal',
    lineHops: store.view.lineHops !== false,
    disabledReactionIds: new Set([
      ...(store.isZunigaStudy ? store.zunigaPrepared?.knockouts ?? [] : []),
      ...changes.filter((change) => change.knockedOut).map((change) => change.id),
    ]),
    modifiedReactionIds: new Set(changes.filter((change) => !change.knockedOut).map((change) => change.id)),
    // Zuñiga's context cell, and any species switched off on a hand-built map.
    inactiveSpeciesIds: new Set([
      ...(store.isZunigaStudy && store.zunigaScenarioId !== 'mutant-coculture' ? ['Se'] : []),
      ...(store.disabledDrawnSpeciesIds?.() ?? []),
    ]),
    inactiveSpeciesLabel: store.isZunigaStudy ? 'inactive context' : 'switched off',
    emphasizedReactionIds: new Set(store.isZunigaStudy ? ['Se@PHELtex', 'Ec@PHEtex', 'Ec@PHEt2rpp'] : []),
    scale: store.scale,
    dark: store.view.dark,
    reactionColorMode: store.view.reactionColorMode,
    reactionColors: reactionColoring.colors,
    omicsEnabled: omicsOverlay.enabled || communityOmicsEnabled,
    omicsColors,
    omicsFallbackColor: omicsOverlay.fallbackColor,
    pathwayRibbonMode: store.view.pathwayRibbonMode,
    pathwayRibbonColors: ribbonColoring.colors,
    pathwayRibbonGroups: ribbonColoring.groups,
    pathwayRibbonWidth: store.view.pathwayRibbonWidth,
    pathwayRibbonOpacity: store.view.pathwayRibbonOpacity,
    selectedReaction: store.selectedReaction,
    reactionEdit: ui.tool === 'edit',
    selectedGeometry: store.selectedGeometry,
    geometryEdit: store.view.geometryEdit,
    multiSelection: store.multiSelection,
    fvaEnabled: store.fvaEnabled,
    fvaRanges: store.fvaRanges,
    fvaProgress: store.fvaProgress,
    labels: store.view.labels,
    metaboliteLabels: store.mapLabels('metabolite'),
    reactionLabels: store.mapLabels('reaction'),
    showCurrency: store.view.showCurrency,
    currencyLabels: store.view.currencyLabels || store.mapMode === 'core-overview',
    showCrossFeeding: store.view.showCrossFeeding,
    crossFeeding: store.crossFeeding,
    crossFeedingBases: store.view.showCrossFeeding ? store.crossFeedingVisibleBases() : null,
    crossFeedingExplorer: { enabled:store.crossFeedingExplorer.enabled, visibleEvents:store.crossFeedingExplorerEvents(), selectedEvent:store.crossFeedingExplorer.index?.events.find(e=>e.id===store.crossFeedingExplorer.selectedId)??null, selectedAt:store.crossFeedingExplorer.selectedAt??null },
    growth: store.growth,
    // Pre-size the pointer halos so a repaint never has to walk the DOM for them.
    minHitRadius: MIN_HIT_PX / (camera.scale > 0 ? camera.scale : 1),
  };
}

/**
 * What to do with a blank canvas.
 *
 * "Empty map" is a legitimate starting point, not an error, and the user is
 * looking at the canvas, not at small print in the sidebar. This puts the first
 * three steps where they are looking, and says plainly that the model is
 * untouched, which is the thing people worry about after watching 271
 * reactions disappear.
 *
 * A drawing shell (cells and compartments with nothing inside them, which is
 * what a genome-scale upload starts from) is just as blank to look at, so the
 * card appears whenever nothing is drawn, and skips the steps the shell has
 * already done.
 */
function emptyMapOverlay() {
  const map = store.map;
  if (!map || typeof map.stats !== 'function') return '';
  const s = map.stats();
  if (s.reactions > 0 || s.nodes > 0) return '';
  const available = store.community?.merged?.reactions?.size ?? 0;
  const shell = s.cells > 0;
  // Laying out a genome-scale model in one cell takes a minute or more, so do
  // not offer it as the obvious thing to press without saying what it costs.
  const bigModel = available > 600;

  const steps = shell
    ? ['<li>Click a compartment or a membrane to add the reactions that belong there.</li>',
      '<li>Or use <strong>Add Reaction</strong> and <strong>Add Metabolite</strong> to place them one at a time.</li>']
    : ['<li>Pick <strong>Cell</strong> and drag to draw a cell.</li>',
      '<li>Pick <strong>Compartment</strong> and drag inside it.</li>',
      '<li>Click a membrane or a compartment to add the reactions that belong on it.</li>'];

  return `<div class="stage-empty">
    <div class="stage-empty-card">
      <h2>Nothing drawn yet</h2>
      <p>${shell
        ? `The cells and compartments are drawn but nothing is inside them; the model is untouched${available ? ` — <strong>${available} reactions</strong> are waiting` : ''}.`
        : `The map is empty; the model is untouched${available ? ` — <strong>${available} reactions</strong> are still there` : ''}.`}</p>
      <ol>
        ${steps.join('\n        ')}
      </ol>
      ${bigModel && store.coreMetabolismOverviewSupport?.()?.supported
        ? `<button class="primary" data-action="map-core-overview" type="button">Draw central metabolism</button>
           <button data-action="map-full" type="button">Draw all ${available} reactions (slow)</button>`
        : '<button class="primary" data-action="map-full" type="button">Or draw the full map</button>'}
    </div>
  </div>`;
}

/** Coalesce the repaints a drag asks for into one per animation frame. */
let pendingPaint = null;
function schedulePaint() {
  if (pendingPaint !== null) return;
  pendingPaint = requestAnimationFrame(() => { pendingPaint = null; paintMap(); });
}

function selectionOverlay() {
  if (!selectionGesture) return '';
  const box = els.stage.getBoundingClientRect();
  const x1 = selectionGesture.startClient.x - box.left;
  const y1 = selectionGesture.startClient.y - box.top;
  const x2 = selectionGesture.currentClient.x - box.left;
  const y2 = selectionGesture.currentClient.y - box.top;
  const left = Math.min(x1, x2);
  const top = Math.min(y1, y2);
  const width = Math.abs(x2 - x1);
  const height = Math.abs(y2 - y1);
  return `<div class="emc-selection-box" style="left:${left}px;top:${top}px;width:${width}px;height:${height}px"></div>`;
}

function cellDrawOverlay() {
  if (!cellDrawGesture) return '';
  const box = els.stage.getBoundingClientRect();
  const x1 = cellDrawGesture.startClient.x - box.left;
  const y1 = cellDrawGesture.startClient.y - box.top;
  const x2 = cellDrawGesture.currentClient.x - box.left;
  const y2 = cellDrawGesture.currentClient.y - box.top;
  const left = Math.min(x1, x2);
  const top = Math.min(y1, y2);
  const width = Math.abs(x2 - x1);
  const height = Math.abs(y2 - y1);
  return `<div class="emc-cell-draw-box" style="left:${left}px;top:${top}px;width:${width}px;height:${height}px"></div>`;
}


function compartmentParentOverlay() {
  if (ui.tool !== 'compartment' || !store.layout) return '';
  const spec = store.currentCompartmentPlacement?.();
  if (!spec) return '';
  const parent = store.compartmentParentRect?.(spec.speciesId, spec.compartmentId);
  if (!parent) return '';
  // data-map-rect lets placeMapOverlays keep it on its parent as the camera moves.
  const { left, top, width, height } = stageRect(parent);
  return `<div class="emc-compartment-parent-box"
    data-map-rect="${parent.x} ${parent.y} ${parent.w} ${parent.h}"
    style="left:${left};top:${top};width:${width};height:${height}"
    title="Valid parent: ${spec.parentId}"></div>`;
}

function compartmentDrawOverlay() {
  if (!compartmentDrawGesture) return '';
  const box = els.stage.getBoundingClientRect();
  const x1 = compartmentDrawGesture.startClient.x - box.left;
  const y1 = compartmentDrawGesture.startClient.y - box.top;
  const x2 = compartmentDrawGesture.currentClient.x - box.left;
  const y2 = compartmentDrawGesture.currentClient.y - box.top;
  const left = Math.min(x1, x2);
  const top = Math.min(y1, y2);
  const width = Math.abs(x2 - x1);
  const height = Math.abs(y2 - y1);
  return `<div class="emc-compartment-draw-box" style="left:${left}px;top:${top}px;width:${width}px;height:${height}px"></div>`;
}

let deferredPaint = false;

function paintSidebar({ force = false } = {}) {
  // Importing another map/scenario replaces the palette. Local commits keep
  // their editing order so a blur cannot move a Remove button before its click.
  if (ui.fluxColorSource !== store.view.fluxColorStops) {
    ui.fluxColorSource = store.view.fluxColorStops;
    if (!ui.fluxColorsCommitting) {
      ui.fluxColorDraft = null;
      force = true;
    }
  }
  // The Reactions page repaints by region and keeps what is being typed in; it
  // is only painted while open, since a closed one would still cost a table.
  if ((ui.workspace || ui.lastWorkspace) === 'reactions') {
    if (ui.workspace === 'reactions') paintReactionsPage();
    return;
  }
  // Re-rendering while a slider is being dragged destroys the element under the
  // pointer and the drag dies mid-gesture. While an input has focus, patch the
  // few values that change instead of rebuilding the panel, and catch up once
  // the gesture ends.
  if (!force && isAdjusting()) {
    patchSidebarReadouts();
    deferredPaint = true;
    return;
  }
  const scroll = els.sidebar.scrollTop;
  captureWorkspaceDraft();
  els.sidebar.innerHTML = renderWorkspace(store, ui.workspace || ui.lastWorkspace, {
    fluxColorDraft: ui.fluxColorDraft,
    steadyComDraft: ui.steadyComDraft,
    steadyComPending: ui.steadyComPending,
  });
  restoreWorkspaceDraft();
  els.sidebar.scrollTop = scroll;
  deferredPaint = false;
}

function draftKey() { return `${ui.workspace || ui.lastWorkspace}:${store.modelEditorMode || ''}`; }

function captureWorkspaceDraft() {
  const form = els.sidebar.querySelector('.model-editor-form');
  if (!form) return;
  const values = [...form.querySelectorAll('[name]')].map(input => ({ name: input.name, value: input.value, checked: input.checked }));
  ui.drafts.set(els.sidebar.dataset.draftKey || draftKey(), values);
}

function restoreWorkspaceDraft() {
  const key = draftKey();
  els.sidebar.dataset.draftKey = key;
  for (const saved of ui.drafts.get(key) || []) {
    const input = [...els.sidebar.querySelectorAll('.model-editor-form [name]')].find(field => field.name === saved.name);
    if (input) { input.value = saved.value; if (input.type === 'checkbox') input.checked = saved.checked; }
  }
}

function discardModelEditorDraft(mode = store.modelEditorMode) {
  const form = els.sidebar.querySelector('.model-editor-form');
  if (form?.dataset.editorKind === mode) {
    ui.drafts.delete(els.sidebar.dataset.draftKey);
    // Remove the discarded fields before a Store notification can capture them
    // again, including when a focused form delays the next sidebar paint.
    form.remove();
  }
  ui.drafts.delete(`model:${mode}`);
}

const WORKSPACES = new Set(['library','structure','map','scenario','community','medium','model','omics','analysis','crossfeeding','overview','reactions']);

let scenarioLoadQueue = Promise.resolve();
let pendingScenarioLoad = null;

function queueScenarioSelection(value) {
  const requested = String(value ?? '');
  const task = scenarioLoadQueue.then(async () => {
    camera.fitted = false;
    const [kind, ...rest] = requested.split(':');
    const id = rest.join(':');
    if (!id) return;
    try {
      if (kind === 'pub') await store.usePublishedCommunity(id);
      else await store.useScenario(id);
    } catch (error) {
      store.status = {
        ...store.status,
        state: 'error',
        message: `Could not load community: ${error.message}`,
      };
      store.emit('status');
    }
  });
  scenarioLoadQueue = task;
  pendingScenarioLoad = task;
  task.finally(() => {
    if (pendingScenarioLoad === task) pendingScenarioLoad = null;
  });
  return task;
}

function openWorkspace(name, trigger = document.activeElement) {
  if (!WORKSPACES.has(name)) return;
  captureWorkspaceDraft();
  ui.workspace = name;
  ui.lastWorkspace = name;
  ui.returnFocus = trigger?.closest?.('[data-action]')?.dataset ? { ...trigger.closest('[data-action]').dataset } : null;
  closeMapMenus();
  els.app.classList.remove('is-task-overlay', 'is-results-maximized');
  els.workspaceEditor.hidden = false;
  applyWorkspaceLayout();
  els.workspaceTitle.textContent = getWorkspaceTitle(name);
  paintSidebar({ force: true });
  paintShell();
  requestAnimationFrame(() => {
    if (name === 'reactions') paintReactionRows('page');
    const first = name === 'library' ? els.sidebar.querySelector('#reaction-search-input')
      : name === 'reactions' ? els.sidebar.querySelector('.rx-search-input')
        : els.workspaceEditor.querySelector('[data-action="close-workspace"]');
    first?.focus({ preventScroll: true });
  });
}

/*
 * The Reactions page can leave the top of the map in view, so hovering a row
 * shows the reaction and clicking one on the map finds its row. Every other
 * workspace covers the map, which is then out of reach of pointer and keyboard.
 */
function applyWorkspaceLayout() {
  const split = ui.workspace === 'reactions' && ui.rx.split && Boolean(store.layout);
  els.workspaceEditor.classList.toggle('is-split', split);
  els.workspaceEditor.dataset.workspace = ui.workspace ?? '';
  els.stage.inert = Boolean(ui.workspace) && !split;
  if (els.stage.inert) els.stage.setAttribute('aria-hidden', 'true');
  else els.stage.removeAttribute('aria-hidden');
}

function closeWorkspace({ restoreFocus = true } = {}) {
  captureWorkspaceDraft();
  ui.workspace = null;
  els.workspaceEditor.hidden = true;
  applyWorkspaceLayout();
  highlightReactionOnMap(null);
  paintShell();
  if (restoreFocus && ui.returnFocus) {
    const focus = [...els.app.querySelectorAll('button[data-action]')].find(button =>
      Object.entries(ui.returnFocus).every(([key,value]) => button.dataset[key] === value)
      && !button.closest('details:not([open])')
      && button.checkVisibility({ checkVisibilityCSS: true })
    );
    (focus || els.app.querySelector('.canvas-toolbar [data-action="toggle-left"]'))?.focus({ preventScroll: true });
  }
}

function paintInspector({ force = false } = {}) {
  if (!force && isAdjusting()) return;
  const scroll = els.inspector.scrollTop;
  els.inspector.innerHTML = ui.rightTab === 'layout'
    ? renderLayoutSettings(store)
    : renderInspector(store);
  els.inspector.scrollTop = scroll;
  syncInspectorVisibility();
}

/**
 * Give the inspector's column back to the map when there is no selection.
 *
 * It is ~290 px wide and spends most of its life saying "No selection", which
 * on a 1366-wide laptop is a fifth of the window spent on nothing. It opens by
 * itself the moment something is selected. Touching the collapse button is
 * taken as the user wanting it their way, and auto behaviour stops.
 */
function syncInspectorVisibility() {
  if (ui.rightTab === 'layout') {
    els.app.classList.remove('is-inspector-collapsed');
    return;
  }
  if (ui.inspectorPinned) return;
  const hasSelection = Boolean(store.selectedReaction || store.selectedEntity);
  els.app.classList.toggle('is-inspector-collapsed', !hasSelection);
}

function paintResults() {
  const scroll = els.results.querySelector('.table-scroll')?.scrollTop ?? 0;
  const compact = els.results.querySelector('[data-rx-scroll="compact"]');
  if (compact) ui.rx.compactScroll = compact.scrollTop;
  // The Table view lists every reaction; built only while it can be seen.
  const tableShown = ui.resultsTab === 'table' && !els.app.classList.contains('is-results-collapsed');
  els.results.innerHTML = renderResults(store, ui.analysisTab, {
    table: tableShown ? {
      model: reactionTableModel(), scrollTop: ui.rx.compactScroll,
      viewportHeight: Math.max(120, els.results.clientHeight - 60), mode: ui.rx.compactMode,
    } : null,
  });
  els.resultsPanel.dataset.tab = ui.resultsTab;
  const table = els.results.querySelector('.table-scroll');
  if (table) table.scrollTop = scroll;
  const scroller = els.results.querySelector('[data-rx-scroll="compact"]');
  if (scroller) scroller.scrollTop = ui.rx.compactScroll;
}

// Reactions page

/** The reactions table for the store as it is now: rebuilt once per store change, shared by the page and the dock. */
function reactionTableModel() {
  if (!ui.rx.model || ui.rx.revision !== storeRevision) {
    ui.rx.model = store.reactionTableModel();
    ui.rx.revision = storeRevision;
  }
  return ui.rx.model;
}

/** A control a repaint must not replace while it has focus: replacing it ends the typing or the drag. */
function isTextEntry(el) {
  return Boolean(el?.matches?.('input:not([type="checkbox"]):not([type="radio"]), textarea, select'));
}

// What each region and table body was last painted with, so an unchanged one
// is left alone: replacing a button between pointerdown and click loses the click.
const paintedHTML = new WeakMap();

/**
 * Paint the Reactions page: the whole page the first time (or when the FVA or
 * reduced-cost columns come or go), then only the regions whose content
 * changed, never the one holding the focused control.
 */
function paintReactionsPage() {
  const model = reactionTableModel();
  const columns = reactionColumns(model, 'page').map((column) => column.id).join();
  let page = els.sidebar.querySelector('[data-rx-page]');
  // New columns need a new table, which waits until nothing in it is being typed in.
  const stale = Boolean(page) && page.dataset.columns !== columns;
  if (stale && !page.contains(document.activeElement)) page = null;
  const view = { model, split: ui.rx.split, showChanges: ui.rx.showChanges, bulk: ui.rx.bulk };
  if (!page) {
    els.sidebar.innerHTML = renderWorkspace(store, 'reactions', {
      reactions: { ...view, scrollTop: ui.rx.pageScroll, viewportHeight: Math.max(200, els.sidebar.clientHeight - 220) },
    });
    page = els.sidebar.querySelector('[data-rx-page]');
    if (page) page.dataset.columns = columns;
    for (const region of page?.querySelectorAll('[data-rx-region]') ?? []) {
      paintedHTML.set(region, regionSignature(region.dataset.rxRegion, renderReactionsRegion(region.dataset.rxRegion, store, view)));
    }
    const scroller = els.sidebar.querySelector('[data-rx-scroll="page"]');
    if (scroller) scroller.scrollTop = ui.rx.pageScroll;
    paintReactionRows('page');
    deferredPaint = false;
    return;
  }
  const active = document.activeElement;
  let held = stale;
  for (const region of page.querySelectorAll('[data-rx-region]')) {
    const name = region.dataset.rxRegion;
    if (name === 'changes') region.hidden = !ui.rx.showChanges;
    const html = renderReactionsRegion(name, store, view);
    const signature = regionSignature(name, html);
    if (paintedHTML.get(region) === signature) continue;
    if (region.contains(active) && isTextEntry(active)) { held = true; continue; }
    region.innerHTML = html;
    paintedHTML.set(region, signature);
  }
  if (!paintReactionRows('page')) held = true;
  deferredPaint = held;
}

/** What decides whether a region needs repainting: its HTML, except the bulk bar, whose fields hold what is being typed. */
function regionSignature(name, html) {
  return name === 'bulk' ? `${store.reactionTable.selection.size}|${store.isZunigaStudy}` : html;
}

/**
 * Paint the rows in view of the page ('page') or the dock ('compact'). With a
 * control in the table focused, only the other cells of the rows already
 * there are brought up to date. Returns false when it had to hold back.
 */
function paintReactionRows(mode) {
  const container = mode === 'page' ? els.sidebar : els.results;
  const body = container.querySelector(`[data-rx-rows="${mode}"]`);
  const scroller = container.querySelector(`[data-rx-scroll="${mode}"]`);
  if (!body || !scroller) return true;
  const header = scroller.querySelector('thead')?.offsetHeight ?? 0;
  const html = renderReactionRows(store, {
    model: reactionTableModel(),
    scrollTop: Math.max(0, scroller.scrollTop - header),
    viewportHeight: scroller.clientHeight,
  }, mode);
  if (paintedHTML.get(body) === html) return true;
  const active = document.activeElement;
  if (body.contains(active) && isTextEntry(active)) {
    patchReactionRows(body, html, active);
    // The rows now match neither the last full paint nor this one, so the next
    // paint must not be skipped because its HTML equals that earlier one (undo).
    paintedHTML.delete(body);
    return false;
  }
  body.innerHTML = html;
  paintedHTML.set(body, html);
  return true;
}

/** Bring the rows already in a table body up to date, cell by cell, leaving the cell with the focused control as it is. */
function patchReactionRows(body, html, active) {
  const template = document.createElement('template');
  template.innerHTML = `<table><tbody>${html}</tbody></table>`;
  const fresh = new Map([...template.content.querySelectorAll('tr[data-rid]')].map((row) => [row.dataset.rid, row]));
  for (const row of body.querySelectorAll('tr[data-rid]')) {
    const next = fresh.get(row.dataset.rid);
    if (!next) continue;
    row.className = next.className;
    [...row.children].forEach((cell, index) => {
      const replacement = next.children[index];
      if (!replacement || cell.contains(active) || cell.innerHTML === replacement.innerHTML) return;
      cell.replaceWith(replacement.cloneNode(true));
    });
  }
}

const pendingRows = new Set();
function scheduleReactionRows(mode) {
  if (pendingRows.has(mode)) return;
  pendingRows.add(mode);
  requestAnimationFrame(() => {
    pendingRows.delete(mode);
    paintReactionRows(mode);
  });
}

/** Repaint what lists reactions after the table's own settings changed: the page and the dock. */
function paintReactionTables() {
  // A new search, filter, order or grouping is a new list: read it from the top.
  const table = store.reactionTable;
  const listing = JSON.stringify([table.query, table.filters, table.sort, table.groupBy]);
  if (ui.rx.listing !== undefined && ui.rx.listing !== listing) {
    ui.rx.pageScroll = 0;
    ui.rx.compactScroll = 0;
    for (const scroller of document.querySelectorAll('[data-rx-scroll]')) scroller.scrollTop = 0;
  }
  ui.rx.listing = listing;
  if (ui.workspace === 'reactions') paintReactionsPage();
  if (ui.resultsTab === 'table') paintResults();
}

/** Scroll a table so a reaction's row is in view, when it is listed. */
function revealReactionRow(rid, mode = 'page') {
  const container = mode === 'page' ? els.sidebar : els.results;
  const scroller = container.querySelector(`[data-rx-scroll="${mode}"]`);
  if (!scroller || !rid) return false;
  const index = reactionTableModel().items.findIndex((item) => item.kind === 'row' && item.row.id === rid);
  if (index < 0) return false;
  const height = RX_ROW_HEIGHT[mode];
  const header = scroller.querySelector('thead')?.offsetHeight ?? 0;
  const top = header + index * height;
  if (top < scroller.scrollTop + header || top + height > scroller.scrollTop + scroller.clientHeight) {
    scroller.scrollTop = Math.max(0, top - scroller.clientHeight / 3);
  }
  paintReactionRows(mode);
  return true;
}

/** Open the Reactions page on one reaction, clearing a search or filter that hides it. */
function revealInReactions(rid, trigger = null) {
  if (!rid) return;
  if (ui.workspace !== 'reactions') openWorkspace('reactions', trigger);
  const model = reactionTableModel();
  if (!model.items.some((item) => item.kind === 'row' && item.row.id === rid)) {
    store.reactionTable.collapsed.clear();
    const filtered = !model.visible.some((row) => row.id === rid);
    store.setReactionTable(filtered ? { query: '', filters: [] } : {});
  }
  if (store.selectedReaction !== rid) store.select(rid);
  requestAnimationFrame(() => revealReactionRow(rid, 'page'));
}

/** Select a reaction and centre the map on it, bringing the map into view above the page if need be. */
function locateReaction(rid) {
  if (!rid) return;
  if (!store.layout?.reactions?.has(rid)) {
    // Still selected, so the inspector shows it: only the map has nothing to show.
    store.select(rid);
    store.status = { ...store.status, state: 'warn', message: `${store.reactionTitle(rid)} is not drawn on the map. Add it from the Reaction library.` };
    store.emit('status');
    return;
  }
  if (ui.workspace === 'reactions' && !ui.rx.split) {
    ui.rx.split = true;
    applyWorkspaceLayout();
    paintReactionsPage();
  } else if (ui.workspace && ui.workspace !== 'reactions') {
    closeWorkspace({ restoreFocus: false });
  }
  store.select(rid);
  requestAnimationFrame(() => centerOnReaction(rid));
}

/** Ring a reaction on the map while its table row is under the pointer; null takes the ring away. */
function highlightReactionOnMap(rid) {
  ui.rx.hover = rid ?? null;
  const svg = els.stage.querySelector('svg.emc-map');
  if (!svg) return;
  svg.querySelectorAll('.emc-table-hover').forEach((node) => node.remove());
  svg.querySelectorAll('.is-table-hover').forEach((node) => node.classList.remove('is-table-hover'));
  const rxn = rid ? store.layout?.reactions?.get(rid) : null;
  if (!rxn) return;
  const ring = document.createElementNS('http://www.w3.org/2000/svg', 'circle');
  ring.setAttribute('class', 'emc-table-hover');
  ring.setAttribute('cx', String(rxn.x));
  ring.setAttribute('cy', String(rxn.y));
  ring.setAttribute('r', String(Math.max(26, 18 / (camera.scale || 1))));
  ring.setAttribute('pointer-events', 'none');
  svg.appendChild(ring);
  for (const part of svg.querySelectorAll(`.emc-seg[data-rxn="${CSS.escape(rid)}"], .emc-marker[data-rxn="${CSS.escape(rid)}"]`)) {
    part.classList.add('is-table-hover');
  }
}

/**
 * Run a constraint edit from the page. A published case refuses edits by
 * throwing, and an undo or a preset change is asynchronous: either way what
 * went wrong is said in the status line.
 */
function runConstraintEdit(edit) {
  const fail = (error) => {
    store.status = { ...store.status, state: 'error', message: error.message };
    store.emit('status');
  };
  try {
    const result = edit();
    if (result && typeof result.catch === 'function') result.catch(fail);
  } catch (error) {
    fail(error);
  }
}

async function exportBoundsCSV() {
  if (!store.community) return;
  const filename = await download(store.fileNameFor(EXPORT_SUFFIX.bounds, 'csv'), () => store.boundsCSV(), 'text/csv');
  if (!filename) return;
  store.addLog('ok', `Bounds exported as ${filename}`);
  store.emit('status');
}

/**
 * Saved means "the same as the last file written or opened", per part: the map
 * is saved with Map JSON, the scenario settings with Scenario and biological
 * edits with Model JSON.
 */
function paintSaveState() {
  const work = store.unsavedWork();
  const unsaved = work.map || work.scenario || work.model;
  els.saveState.classList.toggle('is-unsaved', unsaved);
  els.saveState.innerHTML = unsaved ? '<span>●</span><strong>Unsaved</strong>' : '<span>✓</span><strong>Saved</strong>';
  els.saveState.title = unsaved
    ? [work.map ? 'Map edits not saved (File → Export → Map JSON)' : '',
      work.scenario ? 'Scenario changes not saved (File → Export → Scenario)' : '',
      work.model ? 'Model edits not saved (File → Export → Model JSON)' : ''].filter(Boolean).join('\n')
    : 'Matches the last saved or opened map, scenario and model';
}

function paintShell() {
  const modelLabel = store.scenario?.name
    ?? store.community?.name
    ?? store.models.get(store.species?.[0]?.file)?.name
    ?? 'Untitled model';
  els.modelName.textContent = modelLabel;
  if (els.modelKind) {
    const kind = modelKindLabel(store);
    els.modelKind.textContent = kind.text;
    els.modelKind.title = kind.title;
  }
  // Comparing the work with what was saved serialises the scenario and the
  // pathways; a slider drag repaints the shell every frame, so the badge is
  // brought up to date once the changes pause rather than on each of them.
  debounce('save-state', paintSaveState, 150);
  els.leftPanel.dataset.tab = ui.leftTab;
  els.leftPanel.querySelectorAll('[data-action="left-tab"]').forEach(tab => {
    const active = tab.dataset.tab === ui.leftTab;
    tab.classList.toggle('is-active', active);
    if (active) tab.setAttribute('aria-current', 'page'); else tab.removeAttribute('aria-current');
  });
  els.taskPanel.innerHTML = renderTaskPanel(store, ui);
  const activeViewAction = els.viewControls.contains(document.activeElement) ? document.activeElement.dataset.action : null;
  els.viewControls.innerHTML = renderViewControls(store);
  if (activeViewAction) els.viewControls.querySelector(`[data-action="${activeViewAction}"]`)?.focus({ preventScroll: true });
  els.workspaceTitle.textContent = getWorkspaceTitle(ui.workspace || ui.lastWorkspace);
  document.documentElement.dataset.theme = store.view.dark ? 'dark' : 'light';
  if (els.themeLabel) els.themeLabel.textContent = store.view.dark ? 'Light' : 'Dark';
  els.app.querySelectorAll('[data-action="toggle-theme"]').forEach(button => {
    const label = `Switch to ${store.view.dark ? 'light' : 'dark'} theme`;
    button.setAttribute('aria-label', label);
    button.title = label;
    button.setAttribute('aria-pressed', String(store.view.dark));
  });
  // A map opened before its model is offered for forgetting while it waits.
  if (els.waitingMapItem) {
    const waiting = store.waitingMap;
    els.waitingMapItem.hidden = !waiting;
    els.waitingMapItem.textContent = waiting?.name ? `Forget waiting map (${waiting.name})` : 'Forget waiting map';
    els.waitingMapItem.title = waiting ? `Will be drawn when ${waiting.needs} is loaded` : '';
  }
  if (els.activeToolLabel) els.activeToolLabel.textContent = {select:'Select',pan:'Pan',edit:'Edit',reaction:'Add Reaction',metabolite:'Metabolite',text:'Text',cell:'Cell',compartment:'Compartment'}[ui.tool];
  const optimal = store.solution?.status === 'optimal';
  if (els.resultsSummary) paintRunSummary(optimal);
  els.app.querySelectorAll('[data-action="run-fva"]').forEach((button) => {
    button.disabled = Boolean(store.solution?.steadyCom || store.isZunigaStudy);
    if (store.isZunigaStudy) button.title = 'The Zuñiga case preserves the original source-model FBA method';
    else if (button.disabled) button.title = 'Run FBA, pFBA or Loopless before FVA';
    else if (button.title === 'Run FBA, pFBA or Loopless before FVA') button.removeAttribute('title');
  });
  els.app.querySelectorAll('[data-action="run-pfba"], [data-action="run-loopless"]').forEach(button => {
    button.disabled = Boolean(store.isZunigaStudy);
    if (button.disabled) button.title = 'The Zuñiga case preserves the original source-model FBA method';
    else if (button.title === 'The Zuñiga case preserves the original source-model FBA method') button.removeAttribute('title');
  });
  els.app.querySelectorAll('[data-action="map-undo"]').forEach(button => button.disabled = !store.canUndoMap);
  els.app.querySelectorAll('[data-action="map-redo"]').forEach(button => button.disabled = !store.canRedoMap);
  const relayoutBlocked = store.relayoutBlocked?.() ?? null;
  els.app.querySelectorAll('[data-action="layout-preset"]').forEach(button => {
    const active = button.dataset.layoutPreset === store.layoutSettings?.preset;
    button.classList.toggle('is-active', active);
    button.setAttribute('aria-pressed', String(active));
    // Said on the button, where the pointer is, rather than after the press.
    button.disabled = Boolean(relayoutBlocked);
    if (relayoutBlocked) button.title = relayoutBlocked;
    else button.removeAttribute('title');
  });
  // The spacing sliders re-run the same layout, so they are held by the same guard.
  els.app.querySelectorAll('.layout-control input[type="range"][data-action^="layout-"]').forEach(input => {
    input.disabled = Boolean(relayoutBlocked);
    if (relayoutBlocked) input.title = relayoutBlocked;
    else input.removeAttribute('title');
  });
  els.inspectorPanel.querySelectorAll('[data-action="right-tab"]').forEach(button => {
    const active = button.dataset.tab === ui.rightTab;
    button.classList.toggle('is-active', active);
    button.setAttribute('aria-selected', String(active));
  });
  els.app.querySelectorAll('.canvas-toolbar [data-action="toggle-left"]').forEach(button => button.setAttribute('aria-expanded', String(els.app.classList.contains('is-task-overlay') || (!els.app.classList.contains('is-left-collapsed') && !els.app.classList.contains('is-focus-mode')))));
  els.app.querySelectorAll('[data-action="toggle-focus"]').forEach(button => {
    const focused = els.app.classList.contains('is-focus-mode');
    button.textContent = focused ? 'Exit focus' : 'Focus mode';
    button.setAttribute('aria-pressed', String(focused));
  });
  els.resultsPanel.dataset.tab = ui.resultsTab;
  els.stage.dataset.tool = ui.tool;
  document.querySelectorAll('[data-tool]').forEach((button) => {
    button.classList.toggle('is-active', button.dataset.tool === ui.tool);
    button.setAttribute('aria-pressed', String(button.dataset.tool === ui.tool));
  });
}

/** Once the drag is over, bring the panels that were only patched up to date. */
function flushDeferredPaint() {
  if (!deferredPaint) return;
  setTimeout(() => {
    if (deferredPaint && !isAdjusting()) {
      paintSidebar({ force: true });
      paintInspector({ force: true });
    }
  }, 0);
}
window.addEventListener('pointerup', flushDeferredPaint);
window.addEventListener('keyup', flushDeferredPaint);
els.sidebar.addEventListener('focusout', flushDeferredPaint);
els.inspector.addEventListener('focusout', flushDeferredPaint);

function isAdjusting() {
  if (ui.steadyComChangingCondition) return true;
  const active = document.activeElement;
  if (!active || (!els.sidebar.contains(active) && !els.inspector.contains(active))) return false;
  if (active.closest('.model-editor-form')) return true;
  if (active.closest('.flux-color-editor')) return true;
  if (active.closest('.sc-steadycom')) return true;
  return active.matches('input:not([type="checkbox"]), textarea');
}

/** Live values that must keep up with the solver during a drag. */
function patchSidebarReadouts() {
  patchFluxColorEditor();
  patchSteadyComControls();
  const rid = store.selectedReaction;
  if (rid) {
    const flux = store.solution?.status === 'optimal' ? store.solution.fluxes.get(rid) ?? 0 : NaN;
    const cell = els.inspector.querySelector('.readout dd');
    if (cell) cell.textContent = Number.isFinite(flux) ? formatFlux(flux, 4) : '—';
  }
  els.sidebar.querySelectorAll('[data-action="species-toggle"]').forEach((box) => {
    const mu = store.growth?.get(box.dataset.species);
    const label = box.closest('.species-head')?.querySelector('.species-mu');
    if (label) label.textContent = mu === undefined ? '—' : `µ ${formatFlux(mu, 4)}`;
  });
  const explorer = els.sidebar.querySelector('.crossfeed-explorer');
  if (explorer && store.crossFeedingExplorer.enabled) {
    const list = explorer.querySelector('.crossfeed-events');
    const count = explorer.querySelector('.crossfeed-event-count');
    if (list) list.innerHTML = renderCrossFeedingEvents(store);
    if (count) count.innerHTML = `<strong>${store.crossFeedingExplorerEvents().length}</strong> of ${store.crossFeedingExplorer.index?.events.length ?? 0} active events · flux solution evidence.`;
  }
}

function patchSteadyComControls() {
  const panel = els.sidebar.querySelector('.sc-steadycom');
  if (!panel) return;
  const state = store.steadyComState?.communityId === panel.dataset.steadycomCommunity ? store.steadyComState : null;
  const computed = state?.source === 'computed' && state.key != null && Number.isFinite(state.max);
  const busy = Boolean(ui.steadyComPending || state?.phase);
  panel.querySelectorAll('[data-action="steadycom-mu"], [data-action="steadycom-mu-number"], [data-action="steadycom-solve"]').forEach((control) => control.disabled = !computed || busy);
  const range = panel.querySelector('[data-steadycom-range-value]');
  if (range) range.textContent = computed ? `${state.min ?? 0} – ${state.max} h⁻¹` : 'Not calculated for the current inputs';
  const calculate = panel.querySelector('[data-action="steadycom-recompute"]');
  if (calculate) calculate.disabled = busy;
  const cancel = panel.querySelector('[data-action="steadycom-cancel"]');
  if (cancel) cancel.hidden = !busy;
  const progress = panel.querySelector('[data-steadycom-progress]');
  if (progress && busy) {
    const iteration = state?.progress?.iteration ?? state?.progress?.iterations ?? state?.iterations;
    progress.textContent = `${state?.phase === 'fixed' ? (state.parsimonious ? 'Solving at fixed µ for the least total flux' : 'Solving at fixed µ') : 'Calculating the growth-rate range'}${Number.isFinite(iteration) ? ` · iteration ${iteration}` : ''}…`;
  }
}

function readFluxColorRows() {
  return [...els.sidebar.querySelectorAll('[data-flux-color-stop]')].map((row) => {
    const input = row.querySelector('[data-action="flux-color-value"]');
    return {
      value: input.value,
      number: input.valueAsNumber,
      color: row.querySelector('[data-action="flux-color-color"]').value,
    };
  });
}

function fluxColorValues(rows) {
  if (!rows.length) throw new Error('Keep at least one color point in a custom palette.');
  const invalid = [];
  const byValue = new Map();
  const points = rows.map((row, index) => {
    const value = row.number ?? Number(row.value);
    if (String(row.value).trim() === '' || !Number.isFinite(value)) invalid.push(index);
    else {
      const matches = byValue.get(value) ?? [];
      matches.push(index);
      byValue.set(value, matches);
    }
    return { value, color: row.color };
  });
  if (invalid.length) {
    const error = new Error('Enter a finite flux value for every point. The map keeps the last valid palette.');
    error.indices = invalid;
    throw error;
  }
  const duplicates = [...byValue.values()].filter((matches) => matches.length > 1).flat();
  if (duplicates.length) {
    const error = new Error('Each color point needs a unique flux value. The map keeps the last valid palette.');
    error.indices = duplicates;
    throw error;
  }
  return points;
}

function commitFluxColorRows(rows) {
  ui.fluxColorDraft = {
    rows: rows.map(({ value, color }) => ({ value: String(value), color })),
    source: store.view.fluxColorStops,
    error: '',
    invalid: [],
  };
  try {
    const points = fluxColorValues(rows);
    ui.fluxColorsCommitting = true;
    store.setFluxColorStops(points);
    ui.fluxColorDraft.source = store.view.fluxColorStops;
    patchFluxColorEditor();
    return true;
  } catch (error) {
    ui.fluxColorDraft.error = error.message;
    ui.fluxColorDraft.invalid = error.indices ?? [];
    patchFluxColorEditor();
    return false;
  } finally {
    ui.fluxColorsCommitting = false;
  }
}

function patchFluxColorEditor() {
  const error = els.sidebar.querySelector('[data-flux-color-error]');
  if (error) {
    error.textContent = ui.fluxColorDraft?.error ?? '';
    error.hidden = !error.textContent;
  }
  const invalid = new Set(ui.fluxColorDraft?.invalid ?? []);
  els.sidebar.querySelectorAll('[data-action="flux-color-value"]').forEach((input) => {
    if (invalid.has(Number(input.dataset.fluxColorIndex))) input.setAttribute('aria-invalid', 'true');
    else input.removeAttribute('aria-invalid');
  });
  els.sidebar.querySelectorAll('[data-flux-color-preview]').forEach((preview) => {
    preview.innerHTML = renderFluxColorPreview(store);
  });
}

function newFluxColorValue(points) {
  const sorted = [...points].sort((a, b) => a.value - b.value);
  const first = sorted[0].value;
  const last = sorted[sorted.length - 1].value;
  const used = new Set(sorted.map((point) => point.value));
  const step = Math.max(1, Math.abs(last) * 0.1,
    sorted.length > 1 ? last - sorted[sorted.length - 2].value : 0);
  const candidates = [last + step, first - Math.max(1, Math.abs(first) * 0.1), 0];
  for (let index = 1; index < sorted.length; index++) {
    candidates.push(sorted[index - 1].value / 2 + sorted[index].value / 2);
  }
  const value = candidates.find((candidate) => Number.isFinite(candidate) && !used.has(candidate));
  if (value === undefined) throw new Error('Choose a different value before adding another color point.');
  return value;
}

function focusFluxColorPoint(index) {
  const input = els.sidebar.querySelector(`[data-action="flux-color-value"][data-flux-color-index="${index}"]`);
  input?.focus({ preventScroll: true });
  input?.scrollIntoView({ block: 'nearest' });
}

function paintReactionSearchResults() {
  const box = els.sidebar.querySelector('#reaction-search-results');
  if (!box) return;
  box.innerHTML = renderReactionSearchResults(store, store.reactionSearchQuery);
}

function centerOnReaction(reactionId) {
  const rxn = store.layout?.reactions.get(reactionId);
  if (!rxn) return;
  const box = els.stage.getBoundingClientRect();
  camera.x = box.width / 2 - rxn.x * camera.scale;
  camera.y = visibleStageHeight(box) / 2 - rxn.y * camera.scale;
  camera.fitted = true;
  applyCamera();
}

/** How much of the stage shows: all of it, or what the Reactions page leaves above itself. */
function visibleStageHeight(box) {
  if (els.workspaceEditor.hidden || !els.workspaceEditor.classList.contains('is-split')) return box.height;
  return Math.max(80, Math.min(box.height, els.workspaceEditor.getBoundingClientRect().top - box.top));
}

function centerOnPathway(pathwayKey) {
  const ids = store.pathwayReactionIds(pathwayKey);
  const points = ids
    .map((rid) => store.layout?.reactions.get(rid))
    .filter(Boolean)
    .map((rxn) => ({ x: rxn.x, y: rxn.y }));
  if (!points.length) return false;

  const minX = Math.min(...points.map((p) => p.x));
  const maxX = Math.max(...points.map((p) => p.x));
  const minY = Math.min(...points.map((p) => p.y));
  const maxY = Math.max(...points.map((p) => p.y));
  const box = els.stage.getBoundingClientRect();
  const pad = 90;
  const width = Math.max(120, maxX - minX + pad * 2);
  const height = Math.max(120, maxY - minY + pad * 2);
  const nextScale = Math.min(
    2.4,
    Math.max(0.08, Math.min(box.width / width, box.height / height)),
  );
  const cx = (minX + maxX) / 2;
  const cy = (minY + maxY) / 2;
  camera.scale = nextScale;
  camera.x = box.width / 2 - cx * camera.scale;
  camera.y = box.height / 2 - cy * camera.scale;
  camera.fitted = true;
  applyCamera();
  return true;
}

// Every status change is a new object, so the first paint of one starts its
// notice clock; repaints of the same status do not restart it.
const statusNotice = { status: null, since: 0, timer: null };

function paintStatus() {
  const s = store.status;
  if (s !== statusNotice.status) {
    statusNotice.status = s;
    statusNotice.since = Date.now();
  }
  const line = statusLine(store, Date.now() - statusNotice.since);
  els.status.className = `status${line.state === 'error' ? ' is-error' : ''}${line.state === 'busy' ? ' is-busy' : ''}`;
  els.statusText.textContent = line.text;
  if (line.title) els.statusText.title = line.title;
  else els.statusText.removeAttribute('title');
  clearTimeout(statusNotice.timer);
  // A notice gives way to the summary on its own, without waiting for an event.
  if (line.expiresIn > 0) statusNotice.timer = setTimeout(paintStatus, line.expiresIn + 20);
}

/*
 * The results bar says how the last analysis run from the Simulate panel went.
 *
 * It is the one line that stays on screen at every width: the status line in
 * the header is hidden below 1280 px, which is where a laptop with the side
 * panels open lands, so a "completed" said only there would go unseen. A run
 * that has just ended flashes once.
 */
const RUN_MODE = { fba: 'FBA', pfba: 'pFBA', fva: 'FVA', loopless: 'Loopless' };
const runFlash = { finishedAt: 0, timer: null };

function paintRunSummary(optimal) {
  const el = els.resultsSummary;
  const mode = store.solution?.steadyCom ? 'SteadyCom fixed µ' : store.loopless ? 'Loopless' : store.parsimonious ? 'pFBA' : store.fvaEnabled ? 'FVA' : 'FBA';
  const run = store.lastRun;
  let text = `${mode} · ${store.solution?.status || 'Not run'}${optimal ? ` · ${formatFlux(store.solution.objectiveValue, 5)}` : ''}`;
  let state = '';
  if (run?.phase === 'running') {
    const fva = run.method === 'fva' && store.fvaProgress?.running
      ? ` ${store.fvaProgress.done} / ${store.fvaProgress.total} reactions` : '';
    text = `Running ${run.label}…${fva}`;
    state = 'is-busy';
  } else if (run && RUN_MODE[run.method] === mode) {
    const at = new Date(run.finishedAt).toLocaleTimeString();
    if (run.phase === 'done') {
      const detail = run.method === 'fva' ? `${store.fvaRanges.size} flux ranges` : `${run.outcome} · ${formatFlux(run.objective, 5)}`;
      text = `✓ ${run.label} completed ${at} · ${detail} · ${run.took}`;
      state = 'is-success';
    } else if (run.phase === 'failed') {
      text = `✕ ${run.label} did not finish ${at} · ${run.outcome}`;
      state = 'is-error';
    } else if (run.phase === 'cancelled') {
      text = `${run.label} cancelled ${at}`;
    }
  }
  el.textContent = text;
  el.title = text;
  el.classList.toggle('is-busy', state === 'is-busy');
  el.classList.toggle('is-success', state === 'is-success');
  el.classList.toggle('is-error', state === 'is-error');
  if (run?.finishedAt && run.finishedAt !== runFlash.finishedAt) {
    runFlash.finishedAt = run.finishedAt;
    el.classList.remove('is-just-finished');
    void el.offsetWidth;   // restart the animation
    el.classList.add('is-just-finished');
    clearTimeout(runFlash.timer);
    runFlash.timer = setTimeout(() => el.classList.remove('is-just-finished'), 2600);
  }
}

// Camera

function fitToStage(svgNode) {
  if (!svgNode || !store.layout) return;
  const box = els.stage.getBoundingClientRect();
  const pad = 20;
  const scale = Math.min(
    (box.width - pad * 2) / store.layout.width,
    (box.height - pad * 2) / store.layout.height,
  );
  camera.scale = Number.isFinite(scale) && scale > 0 ? scale : 1;
  camera.x = (box.width - store.layout.width * camera.scale) / 2;
  camera.y = (box.height - store.layout.height * camera.scale) / 2;
  camera.fitted = true;
}

function applyCamera() {
  const svg = els.stage.querySelector('svg');
  if (!svg) return;
  svg.style.transformOrigin = '0 0';
  svg.style.transform = `translate(${camera.x}px, ${camera.y}px) scale(${camera.scale})`;
  svg.style.width = `${store.layout.width}px`;
  svg.style.height = `${store.layout.height}px`;
  // Labels are sized for the full-resolution canvas; scale them back up as the
  // map zooms out so they stay legible instead of turning into specks. The
  // stylesheet multiplies every label size by --label-scale.
  const inverse = Math.max(1, Math.min(2.6, 0.95 / camera.scale));
  svg.style.setProperty('--label-scale', inverse.toFixed(3));
  // Compartment, cell and medium names keep one size on screen at any zoom;
  // each is held back by its own compartment where that is too small for it.
  svg.style.setProperty('--frame-label-scale', frameLabelScale(MIN_HIT_PX / camera.scale).toFixed(3));
  svg.classList.toggle('is-far', camera.scale < 0.34);
  updateHitTargets(svg);
  updateEditHandles(svg);
  placeMapOverlays();
  if (els.zoomReadout) els.zoomReadout.textContent = `${Math.round(camera.scale * 100)}%`;
}

/**
 * Move the HTML laid over the map to where the camera now puts its spot.
 *
 * The SVG follows the camera through its transform; these are divs placed in
 * stage pixels when the map was painted. A pan, a zoom or the stage changing
 * size (the inspector opening as a new compartment is selected) moves the
 * camera without a repaint, which would leave the parent highlight of Add
 * Compartment standing over the wrong part of the map.
 */
function placeMapOverlays() {
  for (const el of els.stage.querySelectorAll('[data-map-rect]')) {
    const [x, y, w, h] = el.dataset.mapRect.split(' ').map(Number);
    Object.assign(el.style, stageRect({ x, y, w, h }));
  }
  const editor = els.stage.querySelector('.map-text-editor');
  if (editor && textEditor) Object.assign(editor.style, textEditorPlace());
}

/** Where a rectangle in map units falls on the stage, as CSS. */
function stageRect(r) {
  return {
    left: `${camera.x + r.x * camera.scale}px`,
    top: `${camera.y + r.y * camera.scale}px`,
    width: `${r.w * camera.scale}px`,
    height: `${r.h * camera.scale}px`,
  };
}

/**
 * Keep the invisible pointer targets a constant size on screen.
 *
 * The map is laid out at canvas resolution and scaled by the camera, so a glyph
 * that is comfortable at 100% is a few pixels wide at the fitted zoom (3 px on
 * a laptop), which no pointer can hit. Each halo's radius is recomputed as
 * MIN_HIT_PX / scale and floored at the glyph's own radius, so a target is never
 * smaller than a usable one and never grows past the artwork once zoomed in.
 *
 * Panning calls applyCamera on every frame, so this exits early unless the zoom
 * actually changed.
 */
const MIN_HIT_PX = 12;        // radius: a 24 px target, the WCAG 2.2 AA minimum
let hitScaleApplied = null;

function updateHitTargets(svg) {
  const scale = Number.isFinite(camera.scale) && camera.scale > 0 ? camera.scale : 1;
  if (hitScaleApplied === scale) return;
  hitScaleApplied = scale;
  const wanted = MIN_HIT_PX / scale;
  for (const border of svg.querySelectorAll('.emc-multi-drag-hit')) {
    border.setAttribute('stroke-width', String(Math.max(12, wanted)));
  }
  for (const hit of svg.querySelectorAll('.emc-hit')) {
    const base = Number(hit.dataset.hitBase) || 0;
    hit.setAttribute('r', String(Math.max(base, wanted)));
  }
}

/**
 * Keep reaction editing handles visible at fitted/mobile zooms.
 * Geometry lives in map units, but handles should remain roughly finger-sized
 * on screen instead of shrinking below one pixel with the whole SVG.
 */
function updateEditHandles(svg) {
  const scale = Number.isFinite(camera.scale) && camera.scale > 0 ? camera.scale : 1;
  const visualRadius = 7 / scale;
  const maxRadius = 28;
  for (const handle of svg.querySelectorAll('[data-edit-handle-base]')) {
    const base = Number(handle.dataset.editHandleBase) || 5.5;
    handle.setAttribute('r', String(Math.min(maxRadius, Math.max(base, visualRadius))));
    handle.style.strokeWidth = String(Math.min(6, Math.max(1.8, 1.8 / scale)));
  }
  // The rotation lever keeps its length on screen too, or at a fitted zoom its
  // handle would sit inside the centre handle it turns about.
  const arm = rotationArm(MIN_HIT_PX / scale);
  for (const part of svg.querySelectorAll('[data-rotate-handle], [data-rotate-guide]')) {
    const x = Number(part.dataset.rotateOx) + Number(part.dataset.rotateUx) * arm;
    const y = Number(part.dataset.rotateOy) + Number(part.dataset.rotateUy) * arm;
    if (!Number.isFinite(x) || !Number.isFinite(y)) continue;
    if (part.dataset.rotateHandle) {
      part.setAttribute('cx', String(x));
      part.setAttribute('cy', String(y));
    } else {
      part.setAttribute('x2', String(x));
      part.setAttribute('y2', String(y));
    }
  }
}

function zoomBy(factor, origin) {
  zoomCameraAt(camera, factor, els.stage.getBoundingClientRect(), origin);
  applyCamera();
}

// Events

const wheelNavigation = createWheelNavigation();
els.stage.addEventListener('wheel', (event) => {
  event.preventDefault();
  const action = wheelNavigation.classify(event, store.view.scrollInput);
  if (action === 'pan') {
    const delta = wheelPanDelta(event, { pageWidth: els.stage.clientWidth, pageHeight: els.stage.clientHeight });
    camera.x -= delta.x;
    camera.y -= delta.y;
    camera.fitted = true;
    applyCamera();
  } else if (action === 'zoom') {
    zoomBy(event.deltaY < 0 ? 1.12 : 1 / 1.12, { x: event.clientX, y: event.clientY });
  }
}, { passive: false });

els.stage.addEventListener('dblclick', (event) => {
  const reaction = event.target.closest?.('[data-rxn]')?.dataset?.rxn;
  if (reaction) {
    store.select(reaction);
    setTimeout(() => els.inspector.querySelector('[data-action="bound-lower-number"]')?.focus(), 0);
  }
});

/**
 * Two gestures, kept apart on purpose.
 *
 *   panGesture       dragging the background moves the camera
 *   nodeDragGesture  dragging a metabolite moves that node in the layout
 *
 * A single ambiguous "dragging" object cannot express "the pointer went down on
 * a node, so the camera must not move at all". A node drag also records where it
 * started, which is what a later undo would need.
 */
let panGesture = null;
let nodeDragGesture = null;
let metaboliteLabelDragGesture = null;
let compartmentLabelDragGesture = null;
let reactionPickGesture = null;
let reactionDragGesture = null;
let rotateDragGesture = null;
let controlDragGesture = null;
let geometryDragGesture = null;
let canvasResizeGesture = null;
let cellDrawGesture = null;
let compartmentDrawGesture = null;
let selectionGesture = null;
let groupDragGesture = null;
let textDragGesture = null;
let textEditor = null;
let multiTogglePointerId = null;
// The cross-feeding line a press began on: a click on it (a press that never
// moved) names what it carries. Dragging from it still sweeps or pans.
let crossFeedPress = null;
let pinchGesture = null;
let suppressTouchUntilClear = false;
const touchPointers = new Map();

/** Screen coordinates to layout coordinates, through the current camera. */
function screenToMap(clientX, clientY) {
  return stageToMap({ x: clientX, y: clientY }, camera, els.stage.getBoundingClientRect());
}

/**
 * Whatever is drawn *inside* a compartment and has an editing behaviour of its
 * own: metabolites, reaction geometry and its handles, text, and the geometry
 * handles themselves. The compartment's own membrane is deliberately absent,
 * since grabbing that is how a compartment is resized.
 */
function mapContentAt(target) {
  return target?.closest?.(
    '.emc-node[data-node], .emc-marker[data-rxn], .emc-seg[data-rxn],'
    + ' .emc-hit-reaction[data-rxn], .emc-hit-node[data-node],'
    + ' [data-reaction-handle], [data-rotate-handle], [data-control-handle], [data-map-text],'
    + ' [data-geometry-handle], [data-cell-plate], [data-metabolite-label]',
  ) ?? null;
}

/**
 * Whether the active tool moves what is already on the map.
 *
 * Select and Edit both do. They differ in what else they show: Edit reveals a
 * reaction's control points, Select leaves the curve alone. Moving a
 * metabolite, a reaction or a selected group is ordinary direct manipulation
 * and belongs to the tool people reach for first.
 */
function movesThings() {
  return ui.tool === 'select' || ui.tool === 'edit';
}

/**
 * The metabolite circle under the pointer, if it is one the user may move.
 *
 * Only the visible circle and the invisible hit circle around it qualify.
 * Reaction segments carry data-node as well, so a curve can be traced back to
 * the metabolite it ends at, but a press on a curve must not drag that node.
 */
function draggableNodeAt(target) {
  const id = target?.closest?.('.emc-node[data-node], .emc-hit-node[data-node]')?.dataset?.node;
  return id && store.canMoveNode(id) ? id : null;
}

function reactionAt(target) {
  // A control point or the rotation lever is part of the reaction it shapes. A
  // click that ends on one is still a click on that reaction, not on nothing.
  const explicit = target?.closest?.('[data-reaction-handle]')?.dataset?.reactionHandle
    ?? target?.closest?.('[data-rotate-handle]')?.dataset?.rotateHandle
    ?? target?.closest?.('[data-control-rxn]')?.dataset?.controlRxn;
  // The hit layer is the generous target the map draws around every reaction
  // (eleven units of radius, fifteen on a membrane, and a pointer cursor to say
  // so). It resolves to its reaction too, or only a press on the thin visible
  // geometry would count and everything around it would fall through to
  // whatever is underneath.
  const visual = target?.closest?.(
    '.emc-marker[data-rxn], .emc-seg[data-rxn], .emc-hit-reaction[data-rxn]',
  )?.dataset?.rxn;
  return explicit ?? visual ?? null;
}

function draggableReactionAt(target) {
  const id = reactionAt(target);
  return id && store.canMoveReaction(id) ? id : null;
}

/**
 * Was the press on the reaction's marker, rather than on one of its curves?
 *
 * The marker is the thing a reaction is picked up by. Its curves run the width
 * of a pathway, and a press on one of those is far more often the start of a
 * sweep across the map than an attempt to move the reaction they belong to.
 */
function reactionMarkerAt(target) {
  return Boolean(target?.closest?.(
    '.emc-marker[data-rxn], .emc-hit-reaction[data-rxn], [data-reaction-handle]',
  ));
}

function rotateHandleAt(target) {
  const id = target?.closest?.('[data-rotate-handle]')?.dataset?.rotateHandle;
  return id && store.canRotateReaction(id) ? id : null;
}

function multiSelectableAt(target) {
  const nodeId = target?.closest?.('.emc-node[data-node]')?.dataset?.node ?? metaboliteLabelAt(target);
  if (nodeId && store.canMoveNode(nodeId)) return { kind: 'node', id: nodeId };

  const reactionId = target?.closest?.('.emc-marker[data-rxn], .emc-seg[data-rxn]')?.dataset?.rxn;
  if (reactionId && store.canMoveReaction(reactionId)) return { kind: 'reaction', id: reactionId };

  const textId = mapTextAt(target);
  if (textId) return { kind: 'text', id: textId };
  return null;
}

function mapTextAt(target) {
  return target?.closest?.('[data-map-text]')?.dataset?.mapText ?? null;
}

function metaboliteLabelAt(target) {
  const id = target?.closest?.('[data-metabolite-label]')?.dataset?.metaboliteLabel;
  const node = id ? store.layout?.nodes.get(id) : null;
  return node && ['metabolite', 'medium', 'currency'].includes(node.kind) ? id : null;
}

/**
 * Note which field of the open text editor has the keyboard, caret included,
 * before a repaint replaces the stage it lives in, so the rebuilt editor can
 * give it back: a background repaint must never move typing elsewhere.
 */
function captureTextEditorFocus() {
  const old = els.stage.querySelector('.map-text-editor');
  const focused = textEditor && old?.contains(document.activeElement) ? document.activeElement : null;
  if (!focused) return;
  const keep = { role: focused.dataset.role, start: null, end: null };
  // Number fields and selects have no caret; some browsers throw for asking.
  try { keep.start = focused.selectionStart ?? null; keep.end = focused.selectionEnd ?? null; } catch { /* no caret */ }
  textEditor.keep = keep;
}

/** The text editor's spot on the stage: just above where the text sits. */
function textEditorPlace() {
  const annotation = textEditor.id ? store.layout?.texts.get(textEditor.id) : null;
  const fontSize = annotation?.fontSize ?? textEditor.fontSize ?? 22;
  return {
    left: `${camera.x + textEditor.x * camera.scale}px`,
    top: `${camera.y + textEditor.y * camera.scale - Math.max(20, fontSize * camera.scale * 1.25)}px`,
  };
}

function renderTextEditor() {
  captureTextEditorFocus();
  els.stage.querySelector('.map-text-editor')?.remove();
  if (!textEditor) return;
  const keep = textEditor.keep ?? null;
  const firstShown = !textEditor.shown;
  textEditor.keep = null;
  textEditor.shown = true;

  const annotation = textEditor.id ? store.layout?.texts.get(textEditor.id) : null;

  const wrap = document.createElement('div');
  wrap.className = 'map-text-editor';
  Object.assign(wrap.style, textEditorPlace());

  const input = document.createElement('input');
  input.type = 'text';
  input.value = textEditor.value ?? annotation?.text ?? '';
  input.placeholder = 'Type text…';
  input.dataset.role = 'label';
  input.setAttribute('aria-label', textEditor.id ? 'Edit map text' : 'Add text to the map');

  input.addEventListener('input', () => {
    if (textEditor) textEditor.value = input.value;
  });
  // Enter writes the text and Escape drops it from any of the editor's fields;
  // Enter on one of its buttons is that button's own.
  wrap.addEventListener('keydown', (event) => {
    if (event.key === 'Enter' && event.target.closest?.('button')) return;
    if (event.key === 'Enter') {
      event.preventDefault();
      event.stopPropagation();
      commitTextEditor();
    } else if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      cancelTextEditor();
    }
  });

  // Size and font, chosen while writing; the field shows the font chosen.
  const preview = () => { input.style.fontFamily = textFontStack(textEditor?.fontFamily); };
  const size = document.createElement('input');
  size.type = 'number';
  size.className = 'map-text-editor-size';
  size.dataset.role = 'size';
  size.min = String(TEXT_SIZE.min);
  size.max = String(TEXT_SIZE.max);
  size.step = '1';
  size.value = String(textEditor.fontSize ?? TEXT_SIZE.default);
  size.title = 'Text size (px)';
  size.setAttribute('aria-label', 'Text size in pixels');
  // Kept up to date as it is typed: Enter writes the text before `change` fires.
  size.addEventListener('input', () => {
    if (textEditor && Number.isFinite(size.valueAsNumber)) textEditor.fontSize = clampTextSize(size.valueAsNumber);
  });
  size.addEventListener('change', () => {
    if (textEditor) size.value = String(textEditor.fontSize);
  });
  const font = document.createElement('select');
  font.className = 'map-text-editor-font';
  font.dataset.role = 'font';
  font.title = 'Font';
  font.setAttribute('aria-label', 'Text font');
  for (const option of TEXT_FONTS) {
    const item = document.createElement('option');
    item.value = option.id;
    item.textContent = option.label;
    item.selected = option.id === textEditor.fontFamily;
    font.appendChild(item);
  }
  font.addEventListener('change', () => {
    if (!textEditor) return;
    textEditor.fontFamily = font.value;
    preview();
    input.focus();
  });
  preview();

  wrap.appendChild(input);
  wrap.appendChild(size);
  wrap.appendChild(font);

  if (textEditor.id) {
    const remove = document.createElement('button');
    remove.type = 'button';
    remove.className = 'map-text-editor-delete';
    remove.dataset.role = 'delete';
    remove.textContent = '×';
    remove.title = 'Delete text';
    remove.setAttribute('aria-label', 'Delete text');
    remove.addEventListener('pointerdown', (event) => event.preventDefault());
    remove.addEventListener('click', (event) => {
      event.preventDefault();
      event.stopPropagation();
      const id = textEditor?.id;
      textEditor = null;
      if (id) store.removeMapText(id);
    });
    wrap.appendChild(remove);
  }

  els.stage.appendChild(wrap);
  const restore = keep ? wrap.querySelector(`[data-role="${keep.role}"]`) : null;
  if (restore) {
    restore.focus({ preventScroll: true });
    if (keep.start !== null && typeof restore.setSelectionRange === 'function') {
      try { restore.setSelectionRange(keep.start, keep.end ?? keep.start); } catch { /* not a text field */ }
    }
    return;
  }
  // Opened just now: ready to type over. A later repaint, with the keyboard
  // somewhere else, leaves it there.
  if (!firstShown) return;
  requestAnimationFrame(() => {
    if (!input.isConnected) return;
    input.focus();
    input.select();
  });
}

function openTextEditor({ id = null, x, y, value = '', fontSize, fontFamily }) {
  closeMapMenus();
  // A new text starts in the last size and font chosen; an existing one in its own.
  const style = store.view.textStyle ?? {};
  textEditor = {
    id, x, y, value,
    fontSize: fontSize ?? style.fontSize ?? 22,
    fontFamily: fontFamily ?? style.fontFamily ?? 'sans',
  };
  renderTextEditor();
}

function commitTextEditor() {
  if (!textEditor) return;
  const editor = textEditor;
  const value = String(editor.value ?? '').trim();
  textEditor = null;

  if (!value) {
    paintMap();
    return;
  }

  const style = { fontSize: editor.fontSize, fontFamily: editor.fontFamily };
  store.setTextStyle(style);
  if (editor.id) store.updateMapText(editor.id, value, style);
  else store.addMapText(value, editor.x, editor.y, style);
}

function cancelTextEditor() {
  if (!textEditor) return;
  textEditor = null;
  paintMap();
}

function controlHandleAt(target) {
  const hit = target?.closest?.('[data-control-rxn][data-control-node][data-control-handle]');
  return hit ? {
    reactionId: hit.dataset.controlRxn,
    nodeId: hit.dataset.controlNode,
    handle: hit.dataset.controlHandle,
  } : null;
}

/**
 * Synthetic pointer events (tests, automation) have no active pointer and
 * capturing one throws. Both gestures fall back to uncaptured events, which the
 * stage still receives because it covers the whole map.
 */
function capturePointer(event) {
  try { els.stage.setPointerCapture?.(event.pointerId); } catch { /* not capturable */ }
}

function releasePointer(event) {
  try {
    if (els.stage.hasPointerCapture?.(event.pointerId)) els.stage.releasePointerCapture(event.pointerId);
  } catch { /* never captured */ }
}

// Add reaction pick

function beginReactionPick(nodeId, event) {
  reactionPickGesture = {
    nodeId,
    pointerId: event.pointerId,
    x: event.clientX,
    y: event.clientY,
    moved: false,
  };
  capturePointer(event);
}

function updateReactionPick(event) {
  const gesture = reactionPickGesture;
  if (!gesture || event.pointerId !== gesture.pointerId) return;
  if (Math.abs(event.clientX - gesture.x) + Math.abs(event.clientY - gesture.y) > 4) {
    gesture.moved = true;
  }
}

function finishReactionPick(event) {
  const gesture = reactionPickGesture;
  if (!gesture) return null;
  reactionPickGesture = null;
  releasePointer(event);
  if (gesture.moved) return gesture;

  boundaryMenu.close();
  compartmentMenu.close();
  metaboliteMenu.open(gesture.nodeId, event);
  store.selectEntity('metabolite', gesture.nodeId);
  return gesture;
}

// Node drag

function beginNodeDrag(nodeId, event) {
  const node = store.layout.nodes.get(nodeId);
  const at = screenToMap(event.clientX, event.clientY);
  const history = store.beginMapEdit('Move metabolite');
  // The drag mutates nodes and branches in place; keep Undo's snapshot detached.
  if (history) history.before = structuredClone(history.before);
  nodeDragGesture = {
    nodeId,
    pointerId: event.pointerId,
    // Where on the circle the pointer grabbed it, so the node does not snap its
    // centre to the cursor on the first move.
    grabX: node.x - at.x,
    grabY: node.y - at.y,
    startX: node.x,
    startY: node.y,
    moved: false,
    history,
  };
  capturePointer(event);
  els.stage.classList.add('is-dragging-node');
  markDraggedNode();
}

function updateNodeDrag(event) {
  const gesture = nodeDragGesture;
  if (!gesture) return;
  const at = screenToMap(event.clientX, event.clientY);
  // Geometry only: no solve, no rebuild, no relayout. Repainting the SVG from
  // the updated coordinates is what makes the labels and the curves follow.
  const node = store.moveNode(gesture.nodeId, at.x + gesture.grabX, at.y + gesture.grabY);
  if (!node) return;
  if (Math.abs(node.x - gesture.startX) + Math.abs(node.y - gesture.startY) > 0.5) {
    gesture.moved = true;
  }
  schedulePaint();
}

function finishNodeDrag(event) {
  if (!nodeDragGesture) return null;
  const gesture = nodeDragGesture;
  releasePointer(event);
  nodeDragGesture = null;
  els.stage.classList.remove('is-dragging-node');
  markDraggedNode();
  if (gesture.moved) {
    const merged = store.mergeOverlappingMetabolite(gesture.nodeId);
    if (merged) gesture.mergedInto = merged.target.id;
    store.commitMapEdit(gesture.history);
  }
  // A final paint, in case the pointer went up before the pending frame ran.
  schedulePaint();
  return gesture;
}

/** Re-apply drag and merge feedback after a repaint replaced the SVG. */
function markDraggedNode() {
  els.stage.querySelectorAll('.emc-node.is-dragging, .emc-node.is-merge-target')
    .forEach((circle) => circle.classList.remove('is-dragging', 'is-merge-target'));
  const gesture = nodeDragGesture;
  if (!gesture) return;
  const circleFor = (id) => {
    const escaped = String(id).replace(/["\\]/g, '\\$&');
    return els.stage.querySelector('.emc-node[data-node="' + escaped + '"]');
  };
  circleFor(gesture.nodeId)?.classList.add('is-dragging');
  // Match the merge on release, using the clamped position and nearest copy.
  const target = gesture.moved
    ? store.layout?.overlappingMetaboliteTarget(gesture.nodeId)
    : null;
  if (target) circleFor(target.id)?.classList.add('is-merge-target');
}

// Metabolite labels

function beginMetaboliteLabelDrag(nodeId, event) {
  const node = store.layout?.nodes.get(nodeId);
  if (!node) return;
  closeMapMenus();
  const at = screenToMap(event.clientX, event.clientY);
  const history = store.beginMapEdit('Move metabolite label');
  if (history) history.before = structuredClone(history.before);
  metaboliteLabelDragGesture = {
    nodeId, pointerId: event.pointerId, startAngle: metaboliteLabelAngle(node),
    labelAngleBefore: node.labelAngle,
    pointerAngle: Math.atan2(at.y - node.y, at.x - node.x),
    moved: false, history,
  };
  event.preventDefault();
  capturePointer(event);
  els.stage.classList.add('is-dragging-metabolite-label');
  markDraggedMetaboliteLabel();
}

function updateMetaboliteLabelDrag(event) {
  const g = metaboliteLabelDragGesture;
  if (!g || event.pointerId !== g.pointerId) return;
  const node = store.layout?.nodes.get(g.nodeId);
  if (!node) return;
  const at = screenToMap(event.clientX, event.clientY);
  if (Math.hypot(at.x - node.x, at.y - node.y) < 1e-6) return;
  const pointerAngle = Math.atan2(at.y - node.y, at.x - node.x);
  const angle = g.startAngle + pointerAngle - g.pointerAngle;
  const change = Math.atan2(Math.sin(angle - g.startAngle), Math.cos(angle - g.startAngle));
  if (Math.abs(change) < 1e-4 && !g.moved) return;
  if (!store.setMetaboliteLabelAngle(g.nodeId, angle)) return;
  g.moved = true;
  schedulePaint();
}

function finishMetaboliteLabelDrag(event, { cancel = false } = {}) {
  const g = metaboliteLabelDragGesture;
  if (!g || event.pointerId !== g.pointerId) return null;
  metaboliteLabelDragGesture = null;
  releasePointer(event);
  els.stage.classList.remove('is-dragging-metabolite-label');
  if (cancel) {
    const node = store.layout?.nodes.get(g.nodeId);
    if (node) {
      if (g.labelAngleBefore === undefined) delete node.labelAngle;
      else node.labelAngle = g.labelAngleBefore;
    }
  } else if (g.moved) store.commitMapEdit(g.history);
  markDraggedMetaboliteLabel();
  schedulePaint();
  return g;
}

function markDraggedMetaboliteLabel() {
  els.stage.querySelectorAll('.emc-label-draggable.is-dragging')
    .forEach(label => label.classList.remove('is-dragging'));
  const id = metaboliteLabelDragGesture?.nodeId;
  if (!id) return;
  const escaped = String(id).replace(/["\\]/g, '\\$&');
  els.stage.querySelector('[data-metabolite-label="' + escaped + '"]')?.classList.add('is-dragging');
}

function finishMetaboliteLabelDragForHistory() {
  if (metaboliteLabelDragGesture) {
    finishMetaboliteLabelDrag({ pointerId: metaboliteLabelDragGesture.pointerId });
  }
}

/**
 * Is a drag changing the map right now? Undo swaps the whole map out from
 * under it: the drag would go on editing the restored map and then commit
 * against the snapshot it started from, so undoing the drag would bring back
 * the edit just undone, and lose the one before.
 */
function mapDragInProgress() {
  return Boolean(nodeDragGesture || compartmentLabelDragGesture || reactionDragGesture
    || rotateDragGesture || controlDragGesture || geometryDragGesture || canvasResizeGesture
    || cellDrawGesture || compartmentDrawGesture || groupDragGesture || textDragGesture);
}

// Text drag

function beginTextDrag(textId, event) {
  const annotation = store.layout?.texts.get(textId);
  if (!annotation) return;
  const at = screenToMap(event.clientX, event.clientY);
  textDragGesture = {
    textId,
    grabX: annotation.x - at.x,
    grabY: annotation.y - at.y,
    startX: annotation.x,
    startY: annotation.y,
    moved: false,
    history: store.beginMapEdit('Move text'),
  };
  capturePointer(event);
  els.stage.classList.add('is-dragging-text');
}

function updateTextDrag(event) {
  const g = textDragGesture;
  if (!g) return;
  const at = screenToMap(event.clientX, event.clientY);
  const item = store.moveMapText(g.textId, at.x + g.grabX, at.y + g.grabY);
  if (!item) return;
  if (Math.abs(item.x - g.startX) + Math.abs(item.y - g.startY) > 0.5) g.moved = true;
  schedulePaint();
}

function finishTextDrag(event) {
  if (!textDragGesture) return null;
  const g = textDragGesture;
  releasePointer(event);
  textDragGesture = null;
  els.stage.classList.remove('is-dragging-text');
  if (g.moved) {
    store.commitMapEdit(g.history);
    schedulePaint();
  } else {
    const annotation = store.layout?.texts.get(g.textId);
    if (annotation) {
      store.selectEntity('text', annotation.id);
      openTextEditor({
        id: annotation.id,
        x: annotation.x,
        y: annotation.y,
        value: annotation.text,
        fontSize: annotation.fontSize,
        fontFamily: annotation.fontFamily,
      });
    }
  }
  return g;
}

// Reaction drag

/**
 * A press on a reaction: a drag if the pointer travels, a selection if it does
 * not, decided when the pointer comes up.
 *
 * One press does both, so a reaction can be moved on its own and not only as
 * part of a group. Selecting on the press would repaint and put the reaction's
 * control points under the pointer. The release would then land on one of
 * those, be read as a click on empty canvas and deselect it again, and the
 * inspector would flash open and shut.
 *
 * @param {object} [opts]
 * @param {boolean} [opts.movable] false for a press that may only select
 */
function beginReactionDrag(reactionId, event, { movable = true } = {}) {
  const rxn = store.layout.reactions.get(reactionId);
  const at = screenToMap(event.clientX, event.clientY);
  reactionDragGesture = {
    reactionId,
    movable,
    clientX: event.clientX,
    clientY: event.clientY,
    dragging: false,
    grabX: rxn.x - at.x,
    grabY: rxn.y - at.y,
    startX: rxn.x,
    startY: rxn.y,
    moved: false,
    history: movable
      ? store.beginMapEdit(rxn.onMembrane ? 'Move transporter' : 'Move reaction')
      : null,
  };
  capturePointer(event);
}

function updateReactionDrag(event) {
  const g = reactionDragGesture;
  if (!g || !g.movable) return;
  // A few pixels of slack, so the tremor of a click or a tap does not nudge
  // the reaction and get taken for a drag.
  if (!g.dragging) {
    if (Math.abs(event.clientX - g.clientX) + Math.abs(event.clientY - g.clientY) <= 4) return;
    g.dragging = true;
  }
  const at = screenToMap(event.clientX, event.clientY);
  const rxn = store.moveReaction(g.reactionId, at.x + g.grabX, at.y + g.grabY);
  if (!rxn) return;
  if (Math.abs(rxn.x - g.startX) + Math.abs(rxn.y - g.startY) > 0.5) g.moved = true;
  schedulePaint();
}

function finishReactionDrag(event) {
  if (!reactionDragGesture) return null;
  const g = reactionDragGesture;
  releasePointer(event);
  reactionDragGesture = null;
  if (g.moved) store.commitMapEdit(g.history);
  schedulePaint();
  return g;
}

// Rotation

/** Angles a turned backbone settles on when it is brought close to one. */
const ROTATE_SNAP_STEP = Math.PI / 4;
const ROTATE_SNAP_WITHIN = (4 * Math.PI) / 180;

function beginRotateDrag(reactionId, event) {
  const rxn = store.layout.reactions.get(reactionId);
  if (!rxn) return;
  rotateDragGesture = {
    reactionId,
    startAngle: store.layout.reactionAngle(reactionId),
    moved: false,
    history: store.beginMapEdit('Rotate reaction'),
  };
  capturePointer(event);
}

function updateRotateDrag(event) {
  const g = rotateDragGesture;
  if (!g) return;
  const rxn = store.layout.reactions.get(g.reactionId);
  if (!rxn) return;
  const at = screenToMap(event.clientX, event.clientY);
  if (Math.hypot(at.x - rxn.x, at.y - rxn.y) < 1e-6) return;
  // The lever stands a quarter turn on from the backbone it turns.
  let angle = Math.atan2(at.y - rxn.y, at.x - rxn.x) - Math.PI / 2;
  const nearest = Math.round(angle / ROTATE_SNAP_STEP) * ROTATE_SNAP_STEP;
  if (Math.abs(angle - nearest) < ROTATE_SNAP_WITHIN) angle = nearest;
  if (!store.setReactionAngle(g.reactionId, angle)) return;
  const turned = store.layout.reactionAngle(g.reactionId) - g.startAngle;
  if (Math.abs(Math.atan2(Math.sin(turned), Math.cos(turned))) > 1e-4) g.moved = true;
  schedulePaint();
}

function finishRotateDrag(event) {
  if (!rotateDragGesture) return null;
  const g = rotateDragGesture;
  releasePointer(event);
  rotateDragGesture = null;
  if (g.moved) store.commitMapEdit(g.history);
  schedulePaint();
  return g;
}

// Curve drag

function beginControlDrag(handle, event) {
  const current = store.layout.reactionControlPoints(handle.reactionId)
    .find((p) => p.nodeId === handle.nodeId && p.handle === handle.handle);
  if (!current) return;
  controlDragGesture = {
    ...handle,
    startX: current.x,
    startY: current.y,
    moved: false,
    history: store.beginMapEdit(
      handle.handle === 'b1' ? 'Move metabolite control point'
        : handle.handle === 'b2' ? 'Move reaction control point'
          : 'Move reaction backbone control point'
    ),
  };
  capturePointer(event);
}

function updateControlDrag(event) {
  const g = controlDragGesture;
  if (!g) return;
  const at = screenToMap(event.clientX, event.clientY);
  const point = store.moveBranchControl(g.reactionId, g.nodeId, g.handle, at.x, at.y);
  if (!point) return;
  if (Math.abs(point.x - g.startX) + Math.abs(point.y - g.startY) > 0.5) g.moved = true;
  schedulePaint();
}

function finishControlDrag(event) {
  if (!controlDragGesture) return null;
  const g = controlDragGesture;
  releasePointer(event);
  controlDragGesture = null;
  if (g.moved) store.commitMapEdit(g.history);
  schedulePaint();
  return g;
}

// Draw Cell

function cellPlacementSpeciesId() {
  const select = els.sidebar.querySelector('#cell-species-select');
  const available = store.availableMapCells?.() ?? [];
  return String(select?.value || store.cellPlacementSpeciesId || available[0]?.speciesId || '');
}

function beginCellDraw(event) {
  const speciesId = cellPlacementSpeciesId();
  if (!speciesId || !store.layout) return false;
  const mapPoint = screenToMap(event.clientX, event.clientY);
  cellDrawGesture = {
    speciesId,
    pointerId: event.pointerId,
    startMap: mapPoint,
    currentMap: mapPoint,
    startClient: { x: event.clientX, y: event.clientY },
    currentClient: { x: event.clientX, y: event.clientY },
    moved: false,
  };
  closeMapMenus();
  capturePointer(event);
  els.stage.classList.add('is-drawing-cell');
  schedulePaint();
  return true;
}

function updateCellDraw(event) {
  const g = cellDrawGesture;
  if (!g) return;
  g.currentMap = screenToMap(event.clientX, event.clientY);
  g.currentClient = { x: event.clientX, y: event.clientY };
  if (Math.abs(g.currentClient.x - g.startClient.x) + Math.abs(g.currentClient.y - g.startClient.y) > 5) {
    g.moved = true;
  }
  schedulePaint();
}

function finishCellDraw(event) {
  if (!cellDrawGesture) return null;
  const g = cellDrawGesture;
  releasePointer(event);
  cellDrawGesture = null;
  els.stage.classList.remove('is-drawing-cell');

  let rect;
  if (g.moved) {
    rect = {
      x: Math.min(g.startMap.x, g.currentMap.x),
      y: Math.min(g.startMap.y, g.currentMap.y),
      w: Math.abs(g.currentMap.x - g.startMap.x),
      h: Math.abs(g.currentMap.y - g.startMap.y),
    };
  } else {
    rect = { x: g.startMap.x - 180, y: g.startMap.y - 120, w: 360, h: 240 };
  }

  const result = store.addMapCell(g.speciesId, rect);
  if (result) {
    store.selectCellGeometry(result.cell.id);
    ui.tool = 'cell';
    paintShell();
  } else {
    schedulePaint();
  }
  return result;
}

// Draw Compartment

/** Smallest rectangle that counts as a deliberate drawing, in map units. */
const MIN_DRAWN_SIZE = 24;

/**
 * What the drawing tools are really aiming at.
 *
 * `event.target` is whatever happens to be painted at that pixel (a reaction
 * curve, a metabolite, a pointer halo), so on a map with any content the press
 * lands on artwork and the draw gesture never starts. The compartment and cell
 * spaces are underneath all of it, so the whole stack at that point has to be
 * consulted, not just the topmost element.
 */
function drawTargetAt(event, selector) {
  const direct = event.target?.closest?.(selector);
  if (direct) return direct;
  const stack = document.elementsFromPoint?.(event.clientX, event.clientY) ?? [];
  for (const el of stack) {
    const found = el.closest?.(selector);
    if (found) return found;
  }
  return null;
}

function selectedCompartmentPlacement() {
  const select = els.sidebar.querySelector('#map-compartment-select');
  const key = String(select?.value || store.compartmentPlacementKey || '');
  if (key) store.setCompartmentPlacement?.(key);
  return store.currentCompartmentPlacement?.() ?? null;
}

function pointInRect(point, rect, pad = 0) {
  return Boolean(rect)
    && point.x >= rect.x + pad
    && point.y >= rect.y + pad
    && point.x <= rect.x + rect.w - pad
    && point.y <= rect.y + rect.h - pad;
}

function clampPointToRect(point, rect, pad = 12) {
  return {
    x: Math.min(Math.max(point.x, rect.x + pad), rect.x + rect.w - pad),
    y: Math.min(Math.max(point.y, rect.y + pad), rect.y + rect.h - pad),
  };
}

function beginCompartmentDraw(event) {
  const spec = selectedCompartmentPlacement();
  if (!spec || !store.layout) return false;
  const parentRect = store.compartmentParentRect?.(spec.speciesId, spec.compartmentId);
  if (!parentRect) return false;

  const at = screenToMap(event.clientX, event.clientY);
  if (!pointInRect(at, parentRect, 10)) return false;

  compartmentDrawGesture = {
    speciesId: spec.speciesId,
    compartmentId: spec.compartmentId,
    parentId: spec.parentId,
    parentRect,
    pointerId: event.pointerId,
    startMap: clampPointToRect(at, parentRect),
    currentMap: clampPointToRect(at, parentRect),
    startClient: { x: event.clientX, y: event.clientY },
    currentClient: { x: event.clientX, y: event.clientY },
    moved: false,
  };
  closeMapMenus();
  capturePointer(event);
  els.stage.classList.add('is-drawing-compartment');
  schedulePaint();
  return true;
}

function updateCompartmentDraw(event) {
  const g = compartmentDrawGesture;
  if (!g) return;
  const raw = screenToMap(event.clientX, event.clientY);
  g.currentMap = clampPointToRect(raw, g.parentRect);
  g.currentClient = {
    x: camera.x + g.currentMap.x * camera.scale + els.stage.getBoundingClientRect().left,
    y: camera.y + g.currentMap.y * camera.scale + els.stage.getBoundingClientRect().top,
  };
  if (Math.abs(g.currentClient.x - g.startClient.x) + Math.abs(g.currentClient.y - g.startClient.y) > 5) {
    g.moved = true;
  }
  schedulePaint();
}

function finishCompartmentDraw(event) {
  if (!compartmentDrawGesture) return null;
  const g = compartmentDrawGesture;
  releasePointer(event);
  compartmentDrawGesture = null;
  els.stage.classList.remove('is-drawing-compartment');

  // A tool meant for drawing must not place a default-sized box on a bare
  // click. Nothing is created unless a rectangle was actually pulled out, and
  // the status line says so rather than leaving a silent no-op.
  const rect = {
    x: Math.min(g.startMap.x, g.currentMap.x),
    y: Math.min(g.startMap.y, g.currentMap.y),
    w: Math.abs(g.currentMap.x - g.startMap.x),
    h: Math.abs(g.currentMap.y - g.startMap.y),
  };
  if (!g.moved || rect.w < MIN_DRAWN_SIZE || rect.h < MIN_DRAWN_SIZE) {
    store.status = {
      ...store.status,
      state: 'ok',
      message: 'Drag to draw the compartment: press inside the highlighted parent and pull out a rectangle.',
    };
    store.emit('status');
    schedulePaint();
    return null;
  }

  const result = store.addMapCompartment(g.speciesId, g.compartmentId, rect);
  if (result) {
    store.selectGeometry(g.speciesId, g.compartmentId);
    ui.tool = 'compartment';
    paintShell();
  } else {
    schedulePaint();
  }
  return result;
}

// Canvas resize

function canvasResizeAt(target) {
  return target?.closest?.('[data-canvas-resize]')?.dataset?.canvasResize ?? null;
}

const CANVAS_EDGES = ['right', 'bottom', 'left', 'top', 'corner'];

function beginCanvasResize(edge, event) {
  if (!store.layout || !CANVAS_EDGES.includes(edge)) return;
  canvasResizeGesture = {
    edge,
    pointerId: event.pointerId,
    startClient: { x: event.clientX, y: event.clientY },
    startWidth: store.layout.width,
    startHeight: store.layout.height,
    // How much of the drawing's shift is already applied. Each move recomputes
    // the target from the start of the gesture, so only the difference may be
    // applied or the drawing would travel once per pointer event.
    shifted: { x: 0, y: 0 },
    moved: false,
    history: store.beginMapEdit('Resize canvas'),
  };
  capturePointer(event);
  els.stage.classList.add('is-resizing-canvas');
}

function updateCanvasResize(event) {
  const g = canvasResizeGesture;
  if (!g) return;
  const dx = (event.clientX - g.startClient.x) / (camera.scale || 1);
  const dy = (event.clientY - g.startClient.y) / (camera.scale || 1);

  // The edge being dragged is the one that moves; the opposite one stays put.
  // Pulling the left edge leftwards is a negative dx and a wider canvas.
  const horizontal = { right: dx, corner: dx, left: -dx }[g.edge] ?? 0;
  const vertical = { bottom: dy, corner: dy, top: -dy }[g.edge] ?? 0;
  const width = g.startWidth + horizontal;
  const height = g.startHeight + vertical;

  // Clamping decides the real size, so the shift is derived from what the
  // canvas actually became rather than from what the pointer asked for.
  // Otherwise the drawing keeps sliding after the canvas has stopped growing.
  const target = store.previewCanvasSize(width, height);
  if (!target) return;
  const wantX = g.edge === 'left' ? target.width - g.startWidth : 0;
  const wantY = g.edge === 'top' ? target.height - g.startHeight : 0;

  const result = store.setMapCanvasSize(width, height, {
    shiftX: wantX - g.shifted.x,
    shiftY: wantY - g.shifted.y,
  });
  if (!result) return;
  g.shifted = { x: wantX, y: wantY };
  if (Math.abs(result.width - g.startWidth) + Math.abs(result.height - g.startHeight) > 0.5) {
    g.moved = true;
  }
  schedulePaint();
}

function finishCanvasResize(event) {
  if (!canvasResizeGesture) return null;
  const g = canvasResizeGesture;
  releasePointer(event);
  canvasResizeGesture = null;
  els.stage.classList.remove('is-resizing-canvas');
  if (g.moved) store.commitMapEdit(g.history);
  schedulePaint();
  return g;
}

function beginCompartmentLabelDrag(hit, event) {
  closeMapMenus();
  const cellId = hit.closest('[data-cell]')?.dataset.cell ?? null;
  const compartmentId = hit.dataset.compartmentLabel;
  const matrix = hit.transform.baseVal.consolidate()?.matrix;
  if (!matrix || !store.map.geometryTarget(cellId, compartmentId)) {
    return;
  }
  compartmentLabelDragGesture = {
    cellId,
    compartmentId,
    pointerId: event.pointerId,
    startMap: screenToMap(event.clientX, event.clientY),
    startClient: { x: event.clientX, y: event.clientY },
    anchor: { x: matrix.e, y: matrix.f },
    moved: false,
    history: store.beginMapEdit('Move compartment name'),
  };
  capturePointer(event);
  els.stage.classList.add('is-dragging-compartment-label');
}

function updateCompartmentLabelDrag(event) {
  const gesture = compartmentLabelDragGesture;
  const distance = Math.hypot(event.clientX - gesture.startClient.x, event.clientY - gesture.startClient.y);
  if (!gesture.moved && distance < 4) {
    return;
  }
  const point = screenToMap(event.clientX, event.clientY);
  const position = store.moveCompartmentLabel(gesture.cellId, gesture.compartmentId,
    gesture.anchor.x + point.x - gesture.startMap.x,
    gesture.anchor.y + point.y - gesture.startMap.y);
  if (position) {
    gesture.moved = true;
  }
  schedulePaint();
}

function finishCompartmentLabelDrag(event) {
  const gesture = compartmentLabelDragGesture;
  releasePointer(event);
  compartmentLabelDragGesture = null;
  els.stage.classList.remove('is-dragging-compartment-label');
  if (gesture.moved) {
    store.commitMapEdit(gesture.history);
  } else {
    store.selectEntity('compartment', gesture.compartmentId, { cellId: gesture.cellId });
  }
  schedulePaint();
}

// Geometry drag

/* A cell's outermost compartment: deleting it deletes the cell. */
function cellRootOf(cellId) {
  const cell = store.map?.cells.find((item) => item.id === cellId);
  const root = cell?.compartments.find((item) => item.id === cell.outerCompartment)
    ?? cell?.compartments.find((item) => !item.parentId);
  return root ? { cellId, compartmentId: root.id } : null;
}

function geometryHandleAt(target) {
  const hit = target?.closest?.('[data-geometry-handle][data-geometry-cell][data-geometry-comp]');
  if (!hit) return null;
  return {
    cellId: hit.dataset.geometryCell || null,
    compartmentId: hit.dataset.geometryComp,
    handle: hit.dataset.geometryHandle,
  };
}

function sharedMediumBoundaryHandleAt(target, event) {
  const hit = target?.closest?.('[data-boundary]');
  if (!hit || !store.layout?.environment?.rect) return null;
  const boundary = store.layout.boundary(hit.dataset.boundary);
  const isSharedMedium = boundary
    && (boundary.cellId === null || boundary.cellId === undefined)
    && boundary.compartmentId === store.layout.environment.id;
  if (!isSharedMedium) return null;

  const handle = nearestRectEdge(
    store.layout.environment.rect,
    screenToMap(event.clientX, event.clientY),
  );
  return handle ? {
    cellId: null,
    compartmentId: store.layout.environment.id,
    handle,
    source: 'boundary',
    boundaryId: hit.dataset.boundary,
  } : null;
}

function compartmentBoundaryHandleAt(target, event) {
  const hit = target?.closest?.('[data-boundary]');
  if (!hit || !store.layout) return null;
  const boundary = store.layout.boundary(hit.dataset.boundary);
  if (!boundary?.cellId || !boundary.compartmentId) return null;

  const geometry = store.layout.geometryTarget(boundary.cellId, boundary.compartmentId);
  const rect = geometry?.compartment?.rect;
  if (!rect) return null;

  const handle = nearestRectEdge(
    rect,
    screenToMap(event.clientX, event.clientY),
  );
  return handle ? {
    cellId: boundary.cellId,
    compartmentId: boundary.compartmentId,
    handle,
    source: 'boundary',
    boundaryId: hit.dataset.boundary,
  } : null;
}

function beginGeometryDrag(info, event) {
  const target = store.layout.geometryTarget(info.cellId, info.compartmentId);
  if (!target) return;
  const startPoint = screenToMap(event.clientX, event.clientY);
  const startRect = { ...target.compartment.rect };
  const shaping = SHAPE_HANDLES.includes(info.handle) && !target.environment;
  let shapeGrab = null;
  if (shaping) {
    // Spaced as drawn, at the zoom the drag starts at; the offset keeps the
    // diamond where it was taken hold of instead of jumping under the pointer.
    const side = geometryHandleSide(MIN_HIT_PX / (camera.scale > 0 ? camera.scale : 1));
    const at = shapeHandlePoint(startRect, info.handle, compartmentShape(target.compartment)[info.handle], side);
    shapeGrab = { side, offset: { x: startPoint.x - at.x, y: startPoint.y - at.y } };
  }
  geometryDragGesture = {
    ...info,
    pointerId: event.pointerId,
    startPoint,
    startClient: { x: event.clientX, y: event.clientY },
    startRect,
    shapeGrab,
    moved: false,
    history: store.beginMapEdit(
      target.environment
        ? (info.handle === 'move' ? 'Move shared medium' : 'Resize shared medium')
        : shaping
          ? (info.handle === 'oval' ? 'Make compartment oval' : 'Soften compartment corners')
          : (info.handle === 'move' ? 'Move compartment' : 'Resize compartment'),
    ),
  };
  capturePointer(event);
}

function updateGeometryDrag(event) {
  const g = geometryDragGesture;
  if (!g) return;

  // Do not mutate geometry for tiny pointer jitter. Besides feeling more
  // deliberate, this preserves ordinary click semantics on compartments and
  // boundaries when the user only intended to select/open them.
  const dragDistance = Math.hypot(
    event.clientX - g.startClient.x,
    event.clientY - g.startClient.y,
  );
  if (!g.moved && dragDistance < 3) return;

  const at = screenToMap(event.clientX, event.clientY);
  if (g.shapeGrab) {
    const { side, offset } = g.shapeGrab;
    const amount = shapeAmountAt(g.startRect, g.handle, { x: at.x - offset.x, y: at.y - offset.y }, side);
    if (!store.editCompartmentShape(g.cellId, g.compartmentId, { [g.handle]: amount })) return;
    g.moved = true;
    schedulePaint();
    return;
  }
  const requested = geometryRectFromHandle(g.startRect, g.handle, g.startPoint, at);
  if (!requested) return;
  const edited = store.editGeometryRect(g.cellId, g.compartmentId, requested);
  if (!edited) return;
  g.moved = true;
  schedulePaint();
}

function finishGeometryDrag(event) {
  if (!geometryDragGesture) return null;
  const g = geometryDragGesture;
  releasePointer(event);
  geometryDragGesture = null;
  if (g.moved) store.commitMapEdit(g.history);
  schedulePaint();
  return g;
}

// Multi-selection

function beginSelectionRect(event) {
  closeMapMenus();
  const at = screenToMap(event.clientX, event.clientY);
  selectionGesture = {
    pointerId: event.pointerId,
    startMap: at,
    currentMap: at,
    startClient: { x: event.clientX, y: event.clientY },
    currentClient: { x: event.clientX, y: event.clientY },
    moved: false,
  };
  capturePointer(event);
  schedulePaint();
}

function updateSelectionRect(event) {
  const g = selectionGesture;
  if (!g) return;
  g.currentMap = screenToMap(event.clientX, event.clientY);
  g.currentClient = { x: event.clientX, y: event.clientY };
  if (Math.abs(g.currentClient.x - g.startClient.x) + Math.abs(g.currentClient.y - g.startClient.y) > 4) {
    g.moved = true;
  }
  schedulePaint();
}

function finishSelectionRect(event) {
  if (!selectionGesture) return null;
  const g = selectionGesture;
  releasePointer(event);
  selectionGesture = null;
  if (g.moved) {
    store.selectInRect({
      x1: g.startMap.x,
      y1: g.startMap.y,
      x2: g.currentMap.x,
      y2: g.currentMap.y,
    }, { replace: true });
  } else {
    store.clearMultiSelection();
  }
  schedulePaint();
  return g;
}

function beginGroupDrag(hit, event, { toggleOnClick = false } = {}) {
  closeMapMenus();
  const at = screenToMap(event.clientX, event.clientY);
  groupDragGesture = {
    ...hit,
    pointerId: event.pointerId,
    startMap: at,
    // How far the selection has really gone. A wall can hold it back, so each
    // step asks for what is still owed to the pointer, and the group catches up
    // once the pointer comes back past where it stopped.
    applied: { x: 0, y: 0 },
    moved: false,
    toggleOnClick,
    history: store.beginMapEdit('Move selection'),
  };
  capturePointer(event);
  els.stage.classList.add('is-dragging-group');
}

function updateGroupDrag(event) {
  const g = groupDragGesture;
  if (!g) return;
  const at = screenToMap(event.clientX, event.clientY);
  const dx = (at.x - g.startMap.x) - g.applied.x;
  const dy = (at.y - g.startMap.y) - g.applied.y;
  const moved = store.moveMultiSelection(dx, dy);
  g.applied.x += moved?.dx ?? 0;
  g.applied.y += moved?.dy ?? 0;
  if ((moved?.nodes?.length ?? 0) + (moved?.reactions?.length ?? 0)
    + (moved?.texts?.length ?? 0) + (moved?.compartments?.length ?? 0) > 0) g.moved = true;
  schedulePaint();
}

/*
 * A press on the empty inside of a compartment the multi-selection carries (it,
 * or one round it, is selected) grabs the group, as pressing inside a selected
 * shape does in any drawing program.
 */
function inSelectedCompartment(target) {
  const chosen = store.multiSelection?.compartments;
  if (!chosen?.size) return false;
  const space = target?.closest?.('[data-compartment-space]');
  if (!space) return false;
  const cellId = space.dataset.cellSpace;
  const cell = store.map?.cells?.find((c) => c.id === cellId);
  if (!cell) return false;
  let id = space.dataset.compartmentSpace;
  for (let i = 0; id && i <= cell.compartments.length; i += 1) {
    if (chosen.has(compartmentKey(cellId, id))) return true;
    id = cell.compartments.find((c) => c.id === id)?.parentId;
  }
  return false;
}

function finishGroupDrag(event) {
  if (!groupDragGesture) return null;
  const g = groupDragGesture;
  releasePointer(event);
  groupDragGesture = null;
  els.stage.classList.remove('is-dragging-group');
  if (g.moved) store.commitMapEdit(g.history);
  else if (g.toggleOnClick) store.toggleMultiSelection(g.kind, g.id);
  schedulePaint();
  return g;
}

// Pinch zoom

function touchPoint(event) {
  return { x: event.clientX, y: event.clientY };
}

function beginPinchIfReady(event) {
  if (event.pointerType !== 'touch') return false;
  touchPointers.set(event.pointerId, touchPoint(event));
  if (touchPointers.size < 2) return false;

  event.preventDefault();
  closeMapMenus();
  suspendSinglePointerGestureForPinch();

  const [first, second] = [...touchPointers.values()].slice(0, 2);
  pinchGesture = beginPinch(camera, first, second, els.stage.getBoundingClientRect());
  suppressTouchUntilClear = true;
  capturePointer(event);
  return Boolean(pinchGesture);
}

function updateTouchPointer(event) {
  if (event.pointerType !== 'touch' || !touchPointers.has(event.pointerId)) return false;
  touchPointers.set(event.pointerId, touchPoint(event));

  if (!pinchGesture) return suppressTouchUntilClear;
  event.preventDefault();

  const points = [...touchPointers.values()];
  if (points.length >= 2) {
    updatePinch(camera, pinchGesture, points[0], points[1], els.stage.getBoundingClientRect());
    applyCamera();
  }
  return true;
}

function finishTouchPointer(event) {
  if (event.pointerType !== 'touch' || !touchPointers.has(event.pointerId)) return false;

  const wasPinching = Boolean(pinchGesture) || suppressTouchUntilClear;
  touchPointers.delete(event.pointerId);
  releasePointer(event);

  if (pinchGesture && touchPointers.size < 2) pinchGesture = null;
  if (touchPointers.size === 0) suppressTouchUntilClear = false;

  if (wasPinching) event.preventDefault();
  return wasPinching;
}

function suspendSinglePointerGestureForPinch() {
  multiTogglePointerId = null;
  if (compartmentLabelDragGesture) {
    finishCompartmentLabelDrag({ pointerId: compartmentLabelDragGesture.pointerId });
  }
  if (metaboliteLabelDragGesture) {
    finishMetaboliteLabelDrag({ pointerId: metaboliteLabelDragGesture.pointerId });
  }
  if (textDragGesture) {
    const g = textDragGesture;
    textDragGesture = null;
    els.stage.classList.remove('is-dragging-text');
    if (g.moved) store.commitMapEdit(g.history);
  }
  if (selectionGesture) selectionGesture = null;
  if (groupDragGesture) {
    const g = groupDragGesture;
    groupDragGesture = null;
    els.stage.classList.remove('is-dragging-group');
    if (g.moved) store.commitMapEdit(g.history);
  }
  if (nodeDragGesture) {
    const g = nodeDragGesture;
    nodeDragGesture = null;
    els.stage.classList.remove('is-dragging-node');
    markDraggedNode();
    if (g.moved) store.commitMapEdit(g.history);
  }
  if (reactionDragGesture) {
    const g = reactionDragGesture;
    reactionDragGesture = null;
    if (g.moved) store.commitMapEdit(g.history);
  }
  if (rotateDragGesture) {
    const g = rotateDragGesture;
    rotateDragGesture = null;
    if (g.moved) store.commitMapEdit(g.history);
  }
  if (controlDragGesture) {
    const g = controlDragGesture;
    controlDragGesture = null;
    if (g.moved) store.commitMapEdit(g.history);
  }
  if (cellDrawGesture) {
    cellDrawGesture = null;
    els.stage.classList.remove('is-drawing-cell');
  }
  if (compartmentDrawGesture) {
    compartmentDrawGesture = null;
    els.stage.classList.remove('is-drawing-compartment');
  }
  if (canvasResizeGesture) {
    const g = canvasResizeGesture;
    canvasResizeGesture = null;
    els.stage.classList.remove('is-resizing-canvas');
    if (g.moved) store.commitMapEdit(g.history);
  }
  if (geometryDragGesture) {
    const g = geometryDragGesture;
    geometryDragGesture = null;
    if (g.moved) store.commitMapEdit(g.history);
  }
  if (panGesture) {
    panGesture = null;
    els.stage.classList.remove('is-panning');
  }
}

// Pan

function beginPan(event) {
  // Remember what was under the pointer now: capturing the pointer retargets
  // every later event to the stage, so by pointerup the reaction that was
  // clicked is no longer the event target.
  panGesture = {
    x: event.clientX, y: event.clientY,
    cx: camera.x, cy: camera.y,
    moved: false, target: event.target,
  };
  capturePointer(event);
  els.stage.classList.add('is-panning');
}

function endPan(event) {
  const gesture = panGesture;
  panGesture = null;
  els.stage.classList.remove('is-panning');
  releasePointer(event);
  return gesture;
}

// Pointer plumbing

// Menus are deliberately click-only. Hover may still highlight SVG geometry
// through CSS, but it never opens, closes or switches reaction menus.
els.stage.addEventListener('pointerdown', (event) => {
  if (textEditor && !event.target.closest('.map-text-editor')) {
    commitTextEditor();
    return;
  }
  if (event.target.closest('.map-text-editor')) return;
  crossFeedPress = event.target.closest?.('[data-crossfeed-route]') ?? null;
  if (beginPinchIfReady(event)) return;
  if (event.pointerType === 'touch' && suppressTouchUntilClear) return;
  if (event.target.closest('.zoombar') || boundaryMenu.isMenuTarget(event.target) || metaboliteMenu.isMenuTarget(event.target) || compartmentMenu.isMenuTarget(event.target)) return;

  // Desktop navigation is deliberately separated from editing:
  // middle-button drag always pans the canvas; left-button is reserved for
  // selection and the primary action of the active tool.
  if (event.pointerType === 'mouse' && event.button === 1) {
    event.preventDefault();
    beginPan(event);
    return;
  }
  if (event.pointerType === 'mouse' && event.button !== 0) return;

  /*
   * Pan Mode: the drag that middle-button does everywhere, on the primary
   * button, and nothing else.
   *
   * Returning here is the whole of it. Every gesture below (the selection
   * rectangle, dragging a node, drawing a cell, resizing the canvas) starts
   * from this handler, so not reaching them is what makes the mode unable to
   * change anything. The map's own pointer events are off as well, so there is
   * no hover highlight inviting a click that would do nothing.
   */
  if (ui.tool === 'pan') {
    beginPan(event);
    return;
  }

  const compartmentLabel = event.target.closest('[data-compartment-label]');
  if (compartmentLabel && movesThings()) {
    event.preventDefault();
    beginCompartmentLabelDrag(compartmentLabel, event);
    return;
  }

  const canvasEdge = canvasResizeAt(event.target);
  if (canvasEdge) {
    beginCanvasResize(canvasEdge, event);
    return;
  }

  if (ui.tool === 'cell') {
    const existingCellGeometry = event.target.closest?.(
      '[data-cell-space], [data-cell-boundary], [data-cell-plate], [data-geometry-handle], [data-boundary]'
    );
    if (!existingCellGeometry && beginCellDraw(event)) return;
  }

  if (ui.tool === 'compartment') {
    const handle = event.target.closest?.('[data-geometry-handle]');
    const placement = store.currentCompartmentPlacement?.();
    const space = drawTargetAt(event, '[data-compartment-space]');
    const onExpectedParent = placement && space
      && space.dataset.cellSpace === placement.speciesId
      && space.dataset.compartmentSpace === placement.parentId;
    if (!handle && onExpectedParent && beginCompartmentDraw(event)) return;
  }

  const multiHit = multiSelectableAt(event.target);
  const selecting = event.shiftKey || store.view.multiSelect;

  const selectionBorder = event.target.closest('[data-selection-drag]');
  if (selectionBorder && !store.view.geometryEdit
    && store.multiSelectionSize >= 2 && movesThings()) {
    event.preventDefault();
    beginGroupDrag({}, event);
    return;
  }

  // The empty inside of a selected compartment moves the group it is part of.
  // Its membrane still resizes it, and what is drawn inside answers for itself.
  if (!store.view.geometryEdit && !selecting && store.multiSelectionSize >= 2
    && movesThings() && !mapContentAt(event.target) && inSelectedCompartment(event.target)) {
    event.preventDefault();
    beginGroupDrag({}, event);
    return;
  }

  // An annotation is dragged on its own by default, but it joins a group like
  // anything else: shift, the multi-select tool, or already being in the group
  // all have to reach the branches below rather than stopping here.
  const textId = mapTextAt(event.target);
  if (textId && !store.view.geometryEdit && !selecting
    && !(store.isMultiSelected('text', textId) && store.multiSelectionSize >= 2 && movesThings())) {
    if (movesThings()) beginTextDrag(textId, event);
    return;
  }

  if (store.view.textTool) {
    beginPan(event);
    return;
  }

  const labelNodeId = metaboliteLabelAt(event.target);
  if (labelNodeId && !store.view.geometryEdit && !selecting) {
    if (movesThings()) beginMetaboliteLabelDrag(labelNodeId, event);
    else if (ui.tool === 'reaction') beginReactionPick(labelNodeId, event);
    else if (ui.tool !== 'metabolite') store.selectEntity('metabolite', labelNodeId);
    return;
  }

  if (selecting && !store.view.geometryEdit) {
    if (multiHit) {
      if (store.isMultiSelected(multiHit.kind, multiHit.id) && store.multiSelectionSize >= 2 && movesThings()) {
        beginGroupDrag(multiHit, event, { toggleOnClick: true });
      } else {
        multiTogglePointerId = event.pointerId;
        capturePointer(event);
        store.toggleMultiSelection(multiHit.kind, multiHit.id);
      }
      return;
    }
    beginSelectionRect(event);
    return;
  }

  if (!store.view.geometryEdit && multiHit
    && store.isMultiSelected(multiHit.kind, multiHit.id)
    && store.multiSelectionSize >= 2 && movesThings()) {
    beginGroupDrag(multiHit, event);
    return;
  }

  /*
   * Select edits compartment geometry from its edge, never from its middle.
   *
   * A compartment's membrane resizes it and its handles move it; the space
   * inside belongs to what is drawn there, and to the selection rectangle. A
   * draggable body would make the inside of a compartment compete with its own
   * contents for every press, in the one place a user is most likely to be
   * aiming at something else.
   *
   * mapContentAt keeps the layer above this one intact: a compartment is an
   * ancestor of everything inside it, so a press on a metabolite matches
   * [data-compartment-space] on the way up as well, and without that guard the
   * compartment would answer instead of the metabolite.
   */
  if (!store.view.geometryEdit && ui.tool === 'select') {
    // The handles come first and are not subject to the guard below: a handle
    // is drawn over the thing it belongs to, and mapContentAt counts it as map
    // content, which would make it shield itself from its own gesture.
    const handle = geometryHandleAt(event.target);
    if (handle) { beginGeometryDrag(handle, event); return; }
  }

  if (!store.view.geometryEdit && ui.tool === 'select' && !mapContentAt(event.target)) {
    // The shared medium is a compartment like any other here: its membrane
    // resizes it, its handles move it. Otherwise it would be the one boundary
    // Select could not touch.
    const sharedMediumHandle = sharedMediumBoundaryHandleAt(event.target, event);
    if (sharedMediumHandle) {
      store.selectGeometry(sharedMediumHandle.cellId, sharedMediumHandle.compartmentId);
      beginGeometryDrag(sharedMediumHandle, event);
      return;
    }
    const boundaryHandle = compartmentBoundaryHandleAt(event.target, event);
    if (boundaryHandle) {
      beginGeometryDrag(boundaryHandle, event);
      return;
    }
  }

  // The normal Edit tool must also let the user grab the Shared Medium
  // boundary directly. Geometry Edit adds the full set of compartment handles,
  // but moving the system boundary should not require discovering a second mode.
  if (!store.view.geometryEdit && ui.tool === 'edit') {
    const sharedMediumHandle = sharedMediumBoundaryHandleAt(event.target, event);
    if (sharedMediumHandle) {
      beginGeometryDrag(sharedMediumHandle, event);
      return;
    }
  }

  if (store.view.geometryEdit) {
    const geometryHandle = geometryHandleAt(event.target);
    if (geometryHandle) { beginGeometryDrag(geometryHandle, event); return; }

    // The Shared Medium boundary itself is a resize handle. This is important
    // for large maps where its corner handles can be off-screen or hard to hit.
    const sharedMediumHandle = sharedMediumBoundaryHandleAt(event.target, event);
    if (sharedMediumHandle) {
      store.selectGeometry(sharedMediumHandle.cellId, sharedMediumHandle.compartmentId);
      beginGeometryDrag(sharedMediumHandle, event);
      return;
    }

    beginPan(event);
    return;
  }

  const rotating = rotateHandleAt(event.target);
  if (rotating) { beginRotateDrag(rotating, event); return; }

  const control = controlHandleAt(event.target);
  if (control) { beginControlDrag(control, event); return; }

  const reactionId = draggableReactionAt(event.target);
  if (reactionId) {
    // Add Metabolite acts on the compartment under the pointer, so what is
    // drawn there must not answer first. Handled on pointerup.
    if (ui.tool === 'metabolite') return;
    // One press does both: it drags the reaction if the pointer travels and
    // selects it if it does not. The marker moves a reaction at once; a curve
    // moves it only once the reaction is selected, so that a sweep begun on top
    // of a long branch does not carry a reaction off with it.
    const movable = movesThings()
      && (reactionMarkerAt(event.target) || store.selectedReaction === reactionId);
    beginReactionDrag(reactionId, event, { movable });
    return;
  }

  const nodeId = draggableNodeAt(event.target);
  if (nodeId) {
    if (ui.tool === 'reaction') { beginReactionPick(nodeId, event); return; }
    if (ui.tool === 'metabolite') return;
    // A press that moves drags the metabolite; one that does not still selects
    // it, which the pointerup handler does from the finished gesture.
    if (movesThings()) beginNodeDrag(nodeId, event);
    else store.selectEntity('metabolite', nodeId);
    return;
  }

  /*
   * Dragging the background in Select sweeps out a selection rectangle.
   *
   * That is what group selection needs and what every graphics editor does,
   * with no Shift key or Multi-select mode to discover before the obvious
   * gesture works. Panning is not lost by taking the gesture: it is still the
   * middle button, and it has a tool of its own.
   */
  if (ui.tool === 'select' && event.pointerType === 'mouse') {
    beginSelectionRect(event);
    return;
  }
  if (event.pointerType !== 'mouse') beginPan(event);
});

els.stage.addEventListener('pointermove', (event) => {
  if (updateTouchPointer(event)) return;
  if (compartmentLabelDragGesture) {
    updateCompartmentLabelDrag(event);
    return;
  }
  if (metaboliteLabelDragGesture) { updateMetaboliteLabelDrag(event); return; }
  if (textDragGesture) { updateTextDrag(event); return; }
  if (selectionGesture) { updateSelectionRect(event); return; }
  if (groupDragGesture) { updateGroupDrag(event); return; }
  if (cellDrawGesture) { updateCellDraw(event); return; }
  if (compartmentDrawGesture) { updateCompartmentDraw(event); return; }
  if (canvasResizeGesture) { updateCanvasResize(event); return; }
  if (geometryDragGesture) { updateGeometryDrag(event); return; }
  if (rotateDragGesture) { updateRotateDrag(event); return; }
  if (controlDragGesture) { updateControlDrag(event); return; }
  if (reactionDragGesture) { updateReactionDrag(event); return; }
  if (reactionPickGesture) { updateReactionPick(event); return; }
  if (nodeDragGesture) { updateNodeDrag(event); return; }
  if (!panGesture) return;
  const dx = event.clientX - panGesture.x;
  const dy = event.clientY - panGesture.y;
  if (Math.abs(dx) + Math.abs(dy) > 3) panGesture.moved = true;
  camera.x = panGesture.cx + dx;
  camera.y = panGesture.cy + dy;
  applyCamera();
});

els.stage.addEventListener('pointerup', (event) => {
  if (finishTouchPointer(event)) return;
  if (compartmentLabelDragGesture) {
    finishCompartmentLabelDrag(event);
    return;
  }
  if (metaboliteLabelDragGesture) {
    const gesture = finishMetaboliteLabelDrag(event);
    if (gesture && !gesture.moved) store.selectEntity('metabolite', gesture.nodeId);
    return;
  }
  if (textDragGesture) { finishTextDrag(event); return; }
  if (reactionPickGesture) { finishReactionPick(event); return; }
  if (multiTogglePointerId === event.pointerId) {
    multiTogglePointerId = null;
    releasePointer(event);
    return;
  }
  if (selectionGesture) {
    const rect = finishSelectionRect(event);
    // A press that never moved is a click on the background, which should
    // deselect and close menus, so it falls through.
    if (rect?.moved) return;
  }
  if (groupDragGesture) { finishGroupDrag(event); return; }
  if (cellDrawGesture) { finishCellDraw(event); return; }
  if (compartmentDrawGesture) { finishCompartmentDraw(event); return; }
  if (canvasResizeGesture) { finishCanvasResize(event); return; }
  if (boundaryMenu.isMenuTarget(event.target) || metaboliteMenu.isMenuTarget(event.target) || compartmentMenu.isMenuTarget(event.target) || metaboliteAddMenu.isMenuTarget(event.target)) return;

  if (geometryDragGesture) {
    const geometryGesture = finishGeometryDrag(event);
    if (geometryGesture && !geometryGesture.moved && !store.view.geometryEdit && ui.tool === 'select') {
      if (geometryGesture.source === 'boundary' && geometryGesture.boundaryId) {
        // Selecting a membrane selects the compartment it belongs to, so its
        // handles appear and it can be resized. The list of reactions that can
        // be added to a membrane belongs to Add Reaction, which is the tool that
        // adds reactions. Offering it here would make a plain selection pop a menu.
        closeMapMenus();
        // selectGeometry already records the compartment as the selected
        // entity. Calling selectEntity after it would clear selectedGeometry and
        // take the five handles away again the moment they appear.
        if (geometryGesture.compartmentId) {
          store.selectGeometry(geometryGesture.cellId ?? null, geometryGesture.compartmentId);
        }
      } else if (geometryGesture.compartmentId) {
        closeMapMenus();
        store.selectGeometry(geometryGesture.cellId ?? null, geometryGesture.compartmentId);
      }
    }
    return;
  }

  if (store.view.geometryEdit) {
    const gesture = endPan(event);
    const origin = gesture?.target ?? event.target;
    if (gesture?.moved) return;

    if (ui.tool === 'cell' || ui.tool === 'cell-edit') {
      const cellPlate = origin.closest?.('[data-cell-plate]');
      const boundaryHit = origin.closest?.('[data-boundary]');
      const compartmentHit = origin.closest?.('[data-compartment-space]');
      const boundary = boundaryHit ? store.layout.boundary(boundaryHit.dataset.boundary) : null;
      const cellId = cellPlate?.dataset.cellPlate
        ?? boundary?.cellId
        ?? compartmentHit?.dataset.cellSpace
        ?? null;
      if (cellId) {
        store.selectCellGeometry(cellId);
        return;
      }
      store.clearGeometrySelection();
      return;
    }

    const boundaryHit = origin.closest?.('[data-boundary]');
    if (boundaryHit) {
      const boundary = store.layout.boundary(boundaryHit.dataset.boundary);
      if (boundary?.compartmentId) {
        store.selectGeometry(boundary.cellId ?? null, boundary.compartmentId);
      }
      return;
    }

    const compartmentHit = origin.closest?.('[data-compartment-space]');
    if (compartmentHit) {
      store.selectGeometry(
        compartmentHit.dataset.cellSpace,
        compartmentHit.dataset.compartmentSpace,
      );
      return;
    }

    store.clearGeometrySelection();
    return;
  }

  if (rotateDragGesture) { finishRotateDrag(event); return; }
  if (controlDragGesture) { finishControlDrag(event); return; }

  if (reactionDragGesture) {
    const gesture = finishReactionDrag(event);
    if (gesture && !gesture.moved) {
      closeMapMenus();
      store.select(gesture.reactionId);
    }
    return;
  }

  if (nodeDragGesture) {
    const gesture = finishNodeDrag(event);
    if (gesture && !gesture.moved) {
      closeMapMenus();
      store.selectEntity('metabolite', gesture.nodeId);
    }
    return;
  }

  const gesture = endPan(event);
  const origin = gesture?.target ?? event.target;
  if (gesture?.moved) return;

  const zoomButton = origin.closest?.('.zoombar button') ?? event.target.closest('.zoombar button');
  if (zoomButton) {
    const action = zoomButton.dataset.action;
    if (action === 'zoom-in') zoomBy(1.3);
    else if (action === 'zoom-out') zoomBy(1 / 1.3);
    else { camera.fitted = false; fitToStage(els.stage.querySelector('svg')); applyCamera(); }
    return;
  }

  // A press that never moved is a click, and in Pan Mode a click selects
  // nothing. The mode is left by choosing another tool, not by clicking.
  if (ui.tool === 'pan') return;

  if (store.view.textTool) {
    const at = screenToMap(event.clientX, event.clientY);
    openTextEditor({ x: at.x, y: at.y, value: '' });
    return;
  }

  /*
   * Add Metabolite asks the compartment, not what happens to be drawn in it.
   *
   * So it comes before the reaction and metabolite branches. Below them, a
   * press anywhere near a mark would be answered with a selection and the menu
   * would never open, and in a cytosol with any content at all the tool would
   * look like it did nothing. Add Reaction has the same shape and reaches its
   * menu the same way.
   */
  if (ui.tool === 'metabolite') {
    const space = origin.closest?.('[data-compartment-space]');
    if (space) {
      closeMapMenus();
      metaboliteAddMenu.open(space.dataset.cellSpace, space.dataset.compartmentSpace, event);
      return;
    }
  }

  // A click on a cross-feeding line pins the name of what it carries to it;
  // a second click on the same line takes the name away.
  const crossFeedLine = crossFeedPress ?? origin.closest?.('[data-crossfeed-route]');
  crossFeedPress = null;
  if (crossFeedLine) {
    closeMapMenus();
    store.toggleCrossFeedingLink({
      from: crossFeedLine.dataset.crossfeedFrom,
      base: crossFeedLine.dataset.crossfeed,
      to: crossFeedLine.dataset.crossfeedTo,
    }, screenToMap(event.clientX, event.clientY));
    return;
  }

  // Reaction geometry has priority over the membrane/boundary beneath it.
  // This is especially important for exchange and transport reactions, whose
  // visible segments intentionally overlap a boundary.
  const reactionId = reactionAt(origin);
  if (reactionId) {
    boundaryMenu.close();
    metaboliteMenu.close();
    compartmentMenu.close();
    store.select(reactionId);
    return;
  }
  if (origin.closest?.('[data-node]')) return;

  const boundaryHit = origin.closest?.('[data-boundary]');
  if (boundaryHit) {
    // Same rule as above: the membrane's reaction menu is Add Reaction's.
    if (ui.tool === 'reaction') {
      boundaryMenu.open(boundaryHit.dataset.boundary, event);
      return;
    }
    const boundary = store.layout?.boundary?.(boundaryHit.dataset.boundary);
    closeMapMenus();
    if (boundary?.compartmentId) {
      store.selectGeometry(boundary.cellId ?? null, boundary.compartmentId);
    }
    return;
  }

  const cellHit = origin.closest?.('[data-cell-plate]');
  if (cellHit) {
    closeMapMenus();
    store.selectEntity('cell', cellHit.dataset.cellPlate);
    return;
  }

  const compartmentHit = origin.closest?.('[data-compartment-space]');
  if (compartmentHit) {
    boundaryMenu.close();
    metaboliteMenu.close();
    if (ui.tool === 'metabolite') {
      compartmentMenu.close();
      metaboliteAddMenu.open(compartmentHit.dataset.cellSpace, compartmentHit.dataset.compartmentSpace, event);
      return;
    }
    metaboliteAddMenu.close();
    if (ui.tool === 'reaction') {
      compartmentMenu.open(
        compartmentHit.dataset.cellSpace,
        compartmentHit.dataset.compartmentSpace,
        event,
      );
    } else {
      compartmentMenu.close();
    }
    store.selectEntity('compartment', compartmentHit.dataset.compartmentSpace, {
      cellId: compartmentHit.dataset.cellSpace,
    });
    return;
  }

  boundaryMenu.close();
  metaboliteMenu.close();
  compartmentMenu.close();
  metaboliteAddMenu.close();
  store.clearMultiSelection();
  store.clearCrossFeedingPick();
  store.select(null);
});

// Nothing on the map is dragged and dropped by the browser: a native drag
// (of a text selection reaching into the map, say) cancels the pointer, and
// with it the handle or metabolite being dragged.
els.stage.addEventListener('dragstart', (event) => event.preventDefault());

// A cancelled pointer (the browser took it over, or the window lost focus)
// ends the gesture instead of leaving it live for the next unrelated move.
els.stage.addEventListener('pointercancel', (event) => {
  if (finishTouchPointer(event)) return;
  if (compartmentLabelDragGesture) {
    finishCompartmentLabelDrag(event);
    return;
  }
  if (metaboliteLabelDragGesture) { finishMetaboliteLabelDrag(event, { cancel: true }); return; }
  if (textDragGesture) { finishTextDrag(event); return; }
  if (reactionPickGesture) {
    reactionPickGesture = null;
    releasePointer(event);
    return;
  }
  if (multiTogglePointerId === event.pointerId) {
    multiTogglePointerId = null;
    releasePointer(event);
    return;
  }
  if (selectionGesture) { finishSelectionRect(event); return; }
  if (groupDragGesture) { finishGroupDrag(event); return; }
  if (cellDrawGesture) {
    releasePointer(event);
    cellDrawGesture = null;
    els.stage.classList.remove('is-drawing-cell');
    schedulePaint();
    return;
  }
  if (compartmentDrawGesture) {
    releasePointer(event);
    compartmentDrawGesture = null;
    els.stage.classList.remove('is-drawing-compartment');
    schedulePaint();
    return;
  }
  if (canvasResizeGesture) { finishCanvasResize(event); return; }
  if (geometryDragGesture) { finishGeometryDrag(event); return; }
  if (rotateDragGesture) { finishRotateDrag(event); return; }
  if (controlDragGesture) { finishControlDrag(event); return; }
  if (reactionDragGesture) { finishReactionDrag(event); return; }
  if (nodeDragGesture) { finishNodeDrag(event); return; }
  if (panGesture) endPan(event);
});

els.app.addEventListener('input', onSidebarInput);
els.app.addEventListener('change', onSidebarInput);
els.app.addEventListener('click', onSidebarClick);

let resultsResizeGesture = null;
els.resultsResize?.addEventListener('pointerdown', (event) => {
  const height = els.resultsPanel.getBoundingClientRect().height;
  resultsResizeGesture = { y: event.clientY, height };
  els.resultsResize.setPointerCapture?.(event.pointerId);
  event.preventDefault();
});
window.addEventListener('pointermove', (event) => {
  if (!resultsResizeGesture) return;
  const available = Math.max(140, els.app.querySelector('.workspace').getBoundingClientRect().height - 48 - 320);
  const next = Math.max(140, Math.min(available, resultsResizeGesture.height + resultsResizeGesture.y - event.clientY));
  document.documentElement.style.setProperty('--results-h', `${Math.round(next)}px`);
});
window.addEventListener('pointerup', () => { resultsResizeGesture = null; });

/** The ids a control lists in a data- attribute, as JSON. */
function listedIds(value) {
  try {
    const ids = JSON.parse(value ?? '[]');
    return Array.isArray(ids) ? ids.map(String) : [];
  } catch {
    return [];
  }
}

function onSidebarInput(event) {
  const el = event.target;
  const action = el.dataset.action;
  if (!action) return;
  // A select, checkbox or radio fires both `input` and `change` for one choice,
  // and both reach this handler: act on `change` only, or a new objective
  // rebuilds and solves the community twice. Sliders and number fields keep
  // `input`, which is what makes them live.
  if (event.type === 'input' && (el.tagName === 'SELECT' || el.type === 'checkbox' || el.type === 'radio')) return;

  switch (action) {
    case 'steadycom-mu':
    case 'steadycom-mu-number': {
      // Repainting the sidebar on every tick would take the focus out of the
      // control being dragged, so the pair is synced in place and the store is
      // only told when Solve is pressed.
      const panel = el.closest('.sc-steadycom');
      const value = el.valueAsNumber;
      ui.steadyComDraft = {
        communityId: panel?.dataset.steadycomCommunity,
        key: store.steadyComState?.key,
        value: el.value,
        error: '',
      };
      if (!Number.isFinite(value)) {
        if (event.type === 'change') showSteadyComError('Enter a finite growth rate; an empty field is not µ = 0.', 'mu');
        break;
      }
      const slider = panel?.querySelector('[data-action="steadycom-mu"]');
      const box = panel?.querySelector('[data-action="steadycom-mu-number"]');
      const top = slider ? Number(slider.max) : value;
      const minimum = slider ? Number(slider.min) : 0;
      const clamped = Math.min(Math.max(value, minimum), Number.isFinite(top) ? top : value);
      if (slider && el !== slider) slider.value = String(clamped);
      if (box && el !== box) box.value = el.value;
      clearSteadyComError(panel);
      break;
    }
    case 'steadycom-parsimonious':
      if (event.type === 'change') {
        store.setSteadyComParsimonious(el.checked);
        paintSidebar({ force: true });
      }
      break;
    case 'steadycom-fiber-condition':
      if (event.type === 'change') {
        ui.steadyComDraft = null;
        store.setSteadyComFiber(el.value === 'figure4' ? 30 : null);
        paintSidebar({ force: true });
      }
      break;
    case 'steadycom-fiber':
      if (event.type === 'change') {
        const value = el.valueAsNumber;
        if (!Number.isFinite(value) || value < 1 || value > 30) {
          showSteadyComError('Enter a Bacteroides fiber limit between 1 and 30 C-mmol/gDW/h.', 'fiber');
        } else {
          ui.steadyComDraft = null;
          // A numeric blur may be the pointerdown of Calculate. Commit the
          // condition without replacing that button before its click arrives.
          ui.steadyComChangingCondition = true;
          try { store.setSteadyComFiber(value); }
          finally { ui.steadyComChangingCondition = false; }
          patchSteadyComControls();
        }
      }
      break;
    case 'gimme-threshold': store.omics.gimmeThreshold=el.value===''?null:Number(el.value); break;
    case 'gimme-objective-fraction': store.omics.gimmeObjectiveFraction=Math.max(0,Math.min(1,Number(el.value)/100)); break;
    case 'crossfeeding-query':
      if (store.crossFeedingExplorer.query !== el.value) store.setCrossFeedingExplorer({query:el.value});
      break;
    case 'crossfeeding-metabolite': store.setCrossFeedingExplorer({metabolite:el.value}); break;
    case 'crossfeeding-producer': store.setCrossFeedingExplorer({producer:el.value}); break;
    case 'crossfeeding-consumer': store.setCrossFeedingExplorer({consumer:el.value}); break;
    case 'crossfeeding-min-flux': store.setCrossFeedingExplorer({minFlux:Number(el.value)||0}); break;
    case 'crossfeed-top':
      // Only once the number is settled: a repaint on every keystroke would take the field away.
      if (event.type === 'change') store.setCrossFeedingTop(el.value);
      break;
    case 'crossfeed-metabolite': store.toggleCrossFeedingMetabolite(el.dataset.base, el.checked); break;
    case 'community-omics-condition-a': store.setCommunityOmics({conditionA:el.value}); break;
    case 'community-omics-condition-b': store.setCommunityOmics({conditionB:el.value}); break;
    case 'community-omics-metric': store.setCommunityOmics({metric:el.value}); break;
    case 'community-omics-pseudocount': store.setCommunityOmics({pseudocount:Number(el.value)}); break;
    case 'layout-cell-area':
      if (event.type === 'input') {
        const output = els.inspector.querySelector('[data-layout-output="cellAreaMultiplier"]');
        if (output) output.textContent = `${Number(el.value).toFixed(1)}×`;
      } else {
        camera.fitted = false;
        store.setLayoutSettings({ cellAreaMultiplier: Number(el.value) });
      }
      break;
    case 'layout-cell-spacing':
      if (event.type === 'input') {
        const output = els.inspector.querySelector('[data-layout-output="cellSpacing"]');
        if (output) output.textContent = `${Math.round(Number(el.value))} px`;
      } else {
        camera.fitted = false;
        store.setLayoutSettings({ cellSpacing: Number(el.value) });
      }
      break;
    case 'layout-min-compartment':
      if (event.type === 'input') {
        const output = els.inspector.querySelector('[data-layout-output="minimumCompartmentSize"]');
        if (output) output.textContent = `${Math.round(Number(el.value))} px`;
      } else {
        camera.fitted = false;
        store.setLayoutSettings({ minimumCompartmentSize: Number(el.value) });
      }
      break;
    case 'layout-node-spacing':
      if (event.type === 'input') {
        const output = els.inspector.querySelector('[data-layout-output="nodeSpacing"]');
        if (output) output.textContent = `${Number(el.value).toFixed(2)}×`;
      } else {
        camera.fitted = false;
        store.setLayoutSettings({ nodeSpacing: Number(el.value) });
      }
      break;
    case 'layout-canvas-width':
    case 'layout-canvas-height':
      if (event.type === 'change' && store.map) {
        const tx = store.beginMapEdit('Resize canvas from layout settings');
        const width = action === 'layout-canvas-width' ? Number(el.value) : store.map.width;
        const height = action === 'layout-canvas-height' ? Number(el.value) : store.map.height;
        store.setMapCanvasSize(width, height);
        store.commitMapEdit(tx);
        camera.fitted = false;
        paintMap();
      }
      break;
    case 'reaction-search':
      // Blur also emits change. Replacing a result button between pointerdown
      // and click would swallow the first attempt to select the reaction.
      if (store.reactionSearchQuery === el.value) break;
      store.reactionSearchQuery = el.value;
      paintReactionSearchResults();
      break;
    case 'published-community':
      store.publishedCommunityId = el.value;
      paintSidebar();
      break;
    case 'scenario':
      // The app listens to both input and change globally. A <select> emits both,
      // so only commit the expensive asynchronous community rebuild once.
      if (event.type === 'change') {
        store.cancelSteadyCom?.();
        ui.steadyComDraft = null;
        queueScenarioSelection(el.value);
      }
      break;
    case 'zuniga-scenario':
      if (event.type === 'change') store.useZunigaScenario(el.value);
      break;
    case 'zuniga-comparison':
      if (event.type === 'change') store.setZunigaComparison(el.value);
      break;
    case 'model-editor-species':
      store.setModelEditorSpecies(el.value);
      break;
    case 'cell-placement-species':
      store.setCellPlacementSpecies(el.value);
      break;
    case 'compartment-placement':
      store.setCompartmentPlacement(el.value);
      break;
    case 'compartment-shape-preset':
      store.setCompartmentShapePreset(el.value);
      break;
    case 'objective':
      store.setObjective(el.value);
      break;
    case 'parsimonious':
      store.setParsimonious(el.checked);
      break;
    case 'pfba-biomass-percent':
      debounce('pfba-biomass-percent', () => store.setPFBABiomassPercent(el.value), 180);
      break;
    case 'loopless':
      store.setLoopless(el.checked);
      break;
    case 'fva':
      if (store.solution?.steadyCom && el.checked) { el.checked = false; break; }
      store.setFVA(el.checked);
      break;
    case 'labels':
      store.setView({ labels: el.value });
      break;
    case 'text-font-size':
      // Applied when the field is left or Enter pressed; a cleared field keeps the size.
      if (event.type === 'change') {
        if (Number.isFinite(el.valueAsNumber)) store.styleMapText(el.dataset.text, { fontSize: el.valueAsNumber });
        else el.value = String(store.map?.texts.get(el.dataset.text)?.fontSize ?? TEXT_SIZE.default);
      }
      break;
    case 'text-font-family':
      store.styleMapText(el.dataset.text, { fontFamily: el.value });
      break;
    case 'cell-fill-color':
    case 'cell-border-color':
    case 'compartment-fill-color':
    case 'compartment-border-color':
      // Applied when the picker closes, so one colour is one undo step rather
      // than one per pointer move across the palette.
      if (event.type === 'change') {
        const part = action.endsWith('-fill-color') ? 'fill' : 'stroke';
        store.setMapGeometryColors(el.dataset.cellId, el.dataset.compartmentId ?? null, { [part]: el.value });
      }
      break;
    case 'metabolite-label-source':
      if (event.type === 'change') {
        store.setMapLabelSource('metabolite', el.value, el.dataset.node ?? null);
      }
      break;
    case 'reaction-label-source':
      if (event.type === 'change') {
        store.setMapLabelSource('reaction', el.value, el.dataset.rxn ?? null);
      }
      break;
    case 'scroll-input':
      store.setView({ scrollInput: el.value });
      break;
    case 'reaction-color-mode':
      store.setView({ reactionColorMode: el.value, ...(el.value !== 'flux' ? { lineColorGroup: el.value } : {}) });
      break;
    case 'color-by-flux':
      store.setView({ reactionColorMode: el.checked ? 'flux' : (store.view.lineColorGroup ?? 'pathway') });
      break;
    case 'width-by-flux':
      store.setView({ widthByFlux: el.checked });
      break;
    case 'dash-no-flux':
      store.setView({ dashNoFlux: el.checked });
      break;
    case 'line-hops':
      store.setView({ lineHops: el.checked });
      break;
    // A colour is taken when the picker closes, not on every shade it passes
    // through, so one choice is one step in Undo.
    case 'node-style-fill':
      if (event.type === 'change') store.setMapNodeStyle({ fill: el.value });
      break;
    case 'node-style-stroke':
      if (event.type === 'change') store.setMapNodeStyle({ stroke: el.value });
      break;
    case 'node-fill':
      if (event.type === 'change') store.setMapNodeColors(listedIds(el.dataset.nodeIds), { fill: el.value });
      break;
    case 'node-stroke':
      if (event.type === 'change') store.setMapNodeColors(listedIds(el.dataset.nodeIds), { stroke: el.value });
      break;
    case 'reaction-line-color':
      if (event.type === 'change') store.setMapReactionColor(listedIds(el.dataset.rxnIds), el.value);
      break;
    case 'flux-color-value':
    case 'flux-color-color':
      if (event.type === 'change') commitFluxColorRows(readFluxColorRows());
      break;
    case 'pathway-ribbon-mode':
      store.setView({
        pathwayRibbonMode: el.value,
        ...(el.value !== 'off' ? { reactionColorMode: 'flux' } : {}),
      });
      break;
    case 'pathway-ribbon-width':
      store.setView({ pathwayRibbonWidth: el.value });
      break;
    case 'pathway-ribbon-opacity':
      store.setView({ pathwayRibbonOpacity: el.value });
      break;
    case 'pathway-visible':
      if (event.type === 'change') store.setPathwayVisible(el.dataset.pathwayKey, el.checked);
      break;
    case 'pathway-color':
      if (event.type === 'change') store.setPathwayColor(el.dataset.pathwayKey, el.value);
      break;
    case 'pathway-name':
      if (event.type === 'change') {
        const key = el.dataset.pathwayKey ?? '';
        if (key.startsWith('manual:')) {
          try {
            store.updateManualPathway(key.slice(7), { name: el.value });
          } catch (error) {
            store.status = { ...store.status, state: 'error', message: error.message };
            store.emit('status');
          }
        }
      }
      break;
    case 'omics-enabled':
      store.setOmicsSettings({ enabled: el.checked });
      break;
    case 'omics-type':
      store.setOmicsSettings({ dataType: el.value });
      break;
    case 'omics-mode':
      store.setOmicsSettings({ mode: el.value });
      break;
    case 'omics-condition-a':
      store.setOmicsSettings({ conditionA: el.value });
      break;
    case 'omics-condition-b':
      store.setOmicsSettings({ conditionB: el.value });
      break;
    case 'omics-metric':
      store.setOmicsSettings({ metric: el.value });
      break;
    case 'omics-pseudocount':
      store.setOmicsSettings({ pseudocount: el.value });
      break;
    case 'abundance':
      debounce('abundance', () => store.setAbundance(el.dataset.species, el.value), 140);
      break;
    case 'medium':
    case 'medium-number':
      if (!acceptNumber(el, event, () => Math.abs(Number(store.medium[el.dataset.base] ?? 0)))) break;
      debounce('medium', () => store.setMediumLimit(el.dataset.base, el.value), 120);
      break;
    case 'bound-lower':
    case 'bound-upper': {
      // A sign-log slider: its position is not the bound, signLogValue is.
      const lower = action === 'bound-lower';
      const value = el.dataset.scale === 'signlog' ? signLogValue(el.valueAsNumber) : el.valueAsNumber;
      if (!Number.isFinite(value)) break;
      const box = el.closest('.bound-control')?.querySelector('input[type="number"]');
      if (box) {
        box.value = formatBound(value);
        box.removeAttribute('aria-invalid');
      }
      el.title = `${formatBound(value)} (log scale; the middle is 0)`;
      runConstraintEdit(() => store.setReactionBounds(el.dataset.rxn, lower ? value : null, lower ? null : value));
      scheduleSolve();
      break;
    }
    case 'bound-lower-number':
    case 'bound-upper-number': {
      // The box applies on Enter or when it is left; while typing it is only checked.
      const lower = action === 'bound-lower-number';
      const current = () => formatBound(store.currentReactionBounds(el.dataset.rxn)[lower ? 'lower' : 'upper']);
      if (!acceptNumber(el, event, current) || event.type !== 'change') break;
      runConstraintEdit(() => store.setReactionBounds(el.dataset.rxn, lower ? el.value : null, lower ? null : el.value));
      const slider = el.closest('.bound-control')?.querySelector('input[type="range"]');
      if (slider?.dataset.scale === 'signlog') slider.value = String(signLogPosition(el.valueAsNumber));
      scheduleSolve();
      break;
    }
    case 'rx-search':
      if (store.reactionTable.query !== el.value) debounce('rx-search', () => store.setReactionTable({ query: el.value }), 140);
      break;
    case 'rx-gene-search':
      if (store.reactionTable.geneQuery !== el.value) debounce('rx-gene-search', () => store.setReactionTable({ geneQuery: el.value }), 140);
      break;
    case 'rx-group-by':
      if (event.type === 'change') store.setReactionTable({ groupBy: el.value });
      break;
    case 'rx-select':
      if (event.type === 'change') store.selectReactionRows([el.dataset.rid], el.checked);
      break;
    case 'rx-group-select':
      if (event.type === 'change') {
        const group = reactionTableModel().items.find((item) => item.kind === 'group' && item.key === el.dataset.group);
        if (group) store.selectReactionRows(group.ids, el.checked);
      }
      break;
    case 'rx-select-visible':
      if (event.type === 'change') store.selectReactionRows(reactionTableModel().visible.map((row) => row.id), el.checked);
      break;
    case 'rx-objective':
      if (event.type === 'change') {
        // An emptied box takes the reaction out of the objective.
        if (el.value.trim() !== '' && !Number.isFinite(el.valueAsNumber)) {
          acceptNumber(el, event, () => formatBound(store.objectiveCoefficients()[el.dataset.rid] ?? 0));
          break;
        }
        runConstraintEdit(() => store.setObjectiveCoefficient(el.dataset.rid, el.value.trim() === '' ? 0 : el.value));
      }
      break;
    case 'rx-objective-sense':
      if (event.type === 'change') runConstraintEdit(() => store.setObjectiveSense(el.value));
      break;
    case 'rx-objective-product':
      if (event.type === 'change' && el.value) runConstraintEdit(() => store.setObjective(OBJECTIVES.PRODUCT, el.value));
      break;
    case 'rx-bulk-field':
      ui.rx.bulk[el.dataset.field] = el.value;
      break;
    case 'species-toggle':
      // A kept drawing keeps its view too; only a fresh layout is refitted.
      if (!store.keepsDrawingOnSpeciesToggle()) camera.fitted = false;
      store.setSpeciesEnabled(el.dataset.species, el.checked);
      break;
    default:
      break;
  }
}

/**
 * Run one SteadyCom request and repaint around it.
 *
 * The Store publishes worker progress; paint the busy controls immediately
 * and preserve ordinary editing drafts while waiting for completion.
 */
async function runSteadyCom(request) {
  if (ui.steadyComPending || store.steadyComState?.phase) return;
  ui.steadyComPending = true;
  try {
    const pending = request();
    paintSidebar({ force: true });
    await pending;
  } catch (error) {
    if (error.name !== 'AbortError' && !/cancel|superseded/i.test(error.message)) {
      showSteadyComError(error.message);
      store.status = { ...store.status, state: 'error', message: `SteadyCom: ${error.message}` };
      store.emit('status');
    }
  } finally {
    ui.steadyComPending = false;
    paintSidebar({ force: true });
  }
}

function clearSteadyComError(panel) {
  const alert = panel?.querySelector('[data-steadycom-error]');
  if (alert) { alert.hidden = true; alert.textContent = ''; }
  panel?.querySelectorAll('[aria-invalid="true"]').forEach((input) => input.removeAttribute('aria-invalid'));
}

function showSteadyComError(message, invalidField = null) {
  const panel = els.sidebar.querySelector('.sc-steadycom');
  const input = panel?.querySelector('[data-action="steadycom-mu-number"]');
  ui.steadyComDraft = {
    communityId: panel?.dataset.steadycomCommunity,
    key: store.steadyComState?.key,
    value: input?.value ?? '',
    error: message,
    invalidField,
    fiberValue: panel?.querySelector('[data-action="steadycom-fiber"]')?.value,
  };
  const alert = panel?.querySelector('[data-steadycom-error]');
  if (alert) { alert.textContent = message; alert.hidden = false; }
  if (invalidField) panel?.querySelector(`[data-action="steadycom-${invalidField === 'mu' ? 'mu-number' : 'fiber'}"]`)?.setAttribute('aria-invalid', 'true');
}

function validateSteadyComFiber(panel) {
  const input = panel?.querySelector('[data-action="steadycom-fiber"]');
  if (!input) return true;
  const value = input.valueAsNumber;
  if (!Number.isFinite(value) || value < 1 || value > 30) {
    showSteadyComError('Enter a Bacteroides fiber limit between 1 and 30 C-mmol/gDW/h.', 'fiber');
    return false;
  }
  return true;
}

async function onSidebarClick(event) {
  const button = event.target.closest('button[data-action]');
  if (!button) return;
  if (button.closest('.zoombar')) return;
  const { action, rxn } = button.dataset;

  switch (action) {
    case 'download-findings': downloadFindings(button.dataset.community); break;
    case 'download-member-json':
      await downloadMemberJSON(button.dataset.community, button.dataset.member);
      break;
    case 'download-member-sbml':
      await downloadMemberSBML(button.dataset.community, button.dataset.member);
      break;
    case 'download-community-csv':
      await downloadCommunityCSV(button.dataset.community, button.dataset.file);
      break;
    case 'flux-colors-customize': {
      if (!store.view.fluxColorStops) {
        const reference = Number(store.scale?.reference ?? 1);
        const limit = Number.isFinite(reference) && reference > 0 ? reference : 1;
        commitFluxColorRows([
          { value: String(-limit), number: -limit, color: '#2563eb' },
          { value: '0', number: 0, color: '#cbd5e1' },
          { value: String(limit), number: limit, color: '#dc2626' },
        ]);
      }
      paintSidebar({ force: true });
      focusFluxColorPoint(0);
      break;
    }
    case 'flux-colors-reset':
      ui.fluxColorDraft = null;
      store.setFluxColorStops(null);
      paintSidebar({ force: true });
      els.sidebar.querySelector('[data-action="flux-colors-reset"]')?.focus({ preventScroll: true });
      break;
    case 'flux-color-add': {
      const rows = readFluxColorRows();
      try {
        const points = fluxColorValues(rows);
        const value = newFluxColorValue(points);
        const last = [...points].sort((a, b) => a.value - b.value).at(-1);
        rows.push({ value: String(value), number: value, color: last.color });
        if (commitFluxColorRows(rows)) {
          paintSidebar({ force: true });
          focusFluxColorPoint(rows.length - 1);
        }
      } catch (error) {
        commitFluxColorRows(rows);
      }
      break;
    }
    case 'flux-color-remove': {
      const rows = readFluxColorRows();
      const index = Number(button.dataset.fluxColorIndex);
      if (rows.length <= 1 || !Number.isInteger(index) || index < 0 || index >= rows.length) break;
      rows.splice(index, 1);
      commitFluxColorRows(rows);
      paintSidebar({ force: true });
      focusFluxColorPoint(Math.min(index, rows.length - 1));
      break;
    }
    case 'open-workspace':
      openWorkspace(button.dataset.workspace, button);
      break;
    case 'steadycom-solve': {
      const panel = button.closest('.sc-steadycom');
      if (!validateSteadyComFiber(panel)) break;
      const mu = panel?.querySelector('[data-action="steadycom-mu-number"]')?.valueAsNumber;
      if (!Number.isFinite(mu) || mu < 0) {
        showSteadyComError('Enter a finite growth rate of zero or more.', 'mu');
        break;
      }
      clearSteadyComError(panel);
      await runSteadyCom(() => store.solveSteadyComAt(mu));
      break;
    }
    case 'steadycom-recompute':
      if (!validateSteadyComFiber(button.closest('.sc-steadycom'))) break;
      ui.steadyComDraft = null;
      await runSteadyCom(() => store.steadyComRange({ recompute: true }));
      break;
    case 'steadycom-cancel':
      store.cancelSteadyCom();
      paintSidebar({ force: true });
      break;
    case 'close-workspace': {
      const returningFromScenario = ui.workspace === 'scenario';
      if (returningFromScenario) {
        // Read what the user can actually see selected right now. This makes
        // Return to map the definitive commit point even if a browser delays
        // the select's change event or the user clicks Return immediately.
        const scenarioSelect = els.sidebar.querySelector('select[data-action="scenario"]');
        const selectedValue = scenarioSelect?.value ?? '';
        const activeValue = store.scenario?.published
          ? `pub:${store.scenario.published}`
          : (store.scenario?.id ? `sc:${store.scenario.id}` : '');

        if (selectedValue && selectedValue !== activeValue) {
          button.disabled = true;
          button.textContent = 'Loading community…';
          await queueScenarioSelection(selectedValue);
        } else if (pendingScenarioLoad) {
          button.disabled = true;
          button.textContent = 'Loading community…';
          await pendingScenarioLoad;
        }
      }
      closeWorkspace();
      if (returningFromScenario) {
        camera.fitted = false;
        paintMap();
      }
      break;
    }
    case 'toggle-focus':
      els.app.classList.toggle('is-focus-mode');
      els.app.classList.remove('is-task-overlay');
      paintShell();
      break;
    case 'show-results':
      closeWorkspace({ restoreFocus: false });
      els.app.classList.remove('is-results-collapsed', 'is-task-overlay', 'is-focus-mode');
      if (els.stage.getBoundingClientRect().height < 320) els.app.classList.add('is-results-maximized');
      paintShell();
      break;
    case 'maximize-results':
      closeWorkspace({ restoreFocus: false });
      els.app.classList.remove('is-results-collapsed', 'is-focus-mode');
      els.app.classList.toggle('is-results-maximized');
      button.title = els.app.classList.contains('is-results-maximized') ? 'Restore results dock' : 'Maximize results';
      button.setAttribute('aria-label', button.title);
      break;
    case 'left-tab': {
      const nextTab = ['build', 'model', 'analysis'].includes(button.dataset.tab) ? button.dataset.tab : 'build';
      // Map-editing tools are scoped to Edit Map. Leaving that workspace must
      // cancel any pending draw/edit mode so clicks in Edit Model or Simulate
      // cannot accidentally mutate the map when the user comes back.
      if (nextTab !== 'build') activateTool('select');
      ui.leftTab = nextTab;
      paintShell();
      break;
    }
    case 'right-tab':
      ui.rightTab = ['inspector', 'layout'].includes(button.dataset.tab) ? button.dataset.tab : 'inspector';
      ui.inspectorPinned = true;
      els.app.classList.remove('is-inspector-collapsed', 'is-focus-mode');
      paintInspector({ force: true });
      paintShell();
      break;
    case 'layout-preset':
      camera.fitted = false;
      ui.rightTab = 'layout';
      ui.inspectorPinned = true;
      els.app.classList.remove('is-inspector-collapsed', 'is-focus-mode');
      store.setLayoutPreset(button.dataset.layoutPreset);
      break;
    case 'results-tab':
      closeWorkspace({ restoreFocus: false });
      els.app.classList.remove('is-results-collapsed', 'is-focus-mode');
      ui.resultsTab = ['analysis', 'table', 'logs'].includes(button.dataset.tab) ? button.dataset.tab : 'analysis';
      els.resultsPanel.dataset.tab = ui.resultsTab;
      els.resultsPanel.querySelectorAll('.results-tabs [data-tab]').forEach((tab) => {
        const active = tab.dataset.tab === ui.resultsTab;
        tab.classList.toggle('is-active', active);
        tab.setAttribute('aria-pressed', String(active));
      });
      if (ui.resultsTab === 'table') {
        paintResults();
        requestAnimationFrame(() => paintReactionRows('compact'));
      }
      break;
    case 'analysis-subtab':
      ui.analysisTab = ['flux','fva','reduced','scaled','shadow','repro'].includes(button.dataset.analysisTab) ? button.dataset.analysisTab : 'flux';
      paintResults();
      break;
    case 'toggle-left':
      if (matchMedia('(max-width: 1000px), (max-height: 620px)').matches || els.app.classList.contains('is-focus-mode')) {
        els.app.classList.toggle('is-task-overlay');
        els.app.classList.remove('is-left-collapsed');
      } else els.app.classList.toggle('is-left-collapsed');
      paintShell();
      break;
    case 'toggle-inspector':
      // Explicit beats automatic: once asked for, the panel stays as asked.
      ui.inspectorPinned = true;
      els.app.classList.remove('is-focus-mode');
      els.app.classList.toggle('is-inspector-collapsed');
      paintShell();
      break;
    case 'toggle-results':
      els.app.classList.remove('is-results-maximized');
      els.app.classList.toggle('is-results-collapsed');
      paintShell();
      if (ui.resultsTab === 'table') paintResults();
      break;
    case 'tool-select':
      closeWorkspace({ restoreFocus: false });
      activateTool('select');
      els.app.classList.remove('is-task-overlay');
      break;
    case 'tool-pan':
      closeWorkspace({ restoreFocus: false });
      activateTool('pan');
      els.app.classList.remove('is-task-overlay');
      break;
    case 'tool-edit':
      closeWorkspace({ restoreFocus: false });
      activateTool('edit');
      els.app.classList.remove('is-task-overlay');
      break;
    case 'tool-reaction':
      closeWorkspace({ restoreFocus: false });
      activateTool('reaction');
      ui.leftTab = 'build';
      els.app.classList.remove('is-task-overlay');
      break;
    case 'tool-metabolite':
      closeWorkspace({ restoreFocus: false });
      activateTool('metabolite');
      ui.leftTab = 'build';
      els.app.classList.remove('is-task-overlay');
      paintShell();
      break;
    case 'tool-text':
      closeWorkspace({ restoreFocus: false });
      activateTool('text');
      els.app.classList.remove('is-task-overlay');
      break;
    case 'remove-map-compartment': {
      const target = button.dataset.compartmentId
        ? { cellId: button.dataset.cellId, compartmentId: button.dataset.compartmentId }
        : cellRootOf(button.dataset.cellId);
      if (target) { closeMapMenus(); store.removeMapCompartment(target.cellId, target.compartmentId); }
      break;
    }
    case 'edit-selected-cell-boundary':
      closeWorkspace({ restoreFocus: false });
      activateTool('cell-edit');
      store.setView({ textTool: false, geometryEdit: true, multiSelect: false });
      if (button.dataset.cellId) store.selectCellGeometry(button.dataset.cellId);
      paintShell();
      break;
    case 'edit-selected-compartment-boundary':
      closeWorkspace({ restoreFocus: false });
      activateTool('compartment-edit');
      store.setView({ textTool: false, geometryEdit: true, multiSelect: false });
      paintShell();
      break;
    case 'compartment-shape': {
      const shape = COMPARTMENT_SHAPES[button.dataset.shape];
      if (shape) store.setCompartmentShape(button.dataset.cellId, button.dataset.compartmentId, shape);
      break;
    }
    case 'tool-compartment':
    case 'tool-cell':
      closeWorkspace({ restoreFocus: false });
      activateTool(action === 'tool-cell' ? 'cell' : 'compartment');
      ui.leftTab = 'build';
      els.app.classList.remove('is-task-overlay');
      paintShell();
      break;
    case 'open-crossfeeding-explorer': openWorkspace('crossfeeding', button); store.setCrossFeedingExplorer({enabled:true}); break;
    case 'close-crossfeeding-explorer': store.setCrossFeedingExplorer({enabled:false,selectedId:null}); break;
    case 'select-crossfeeding-event': store.crossFeedingExplorer.selectedAt = null; store.selectCrossFeedingEvent(button.dataset.crossfeedId); break;
    case 'clear-crossfeeding-filters': store.setCrossFeedingExplorer({query:'',metabolite:'',producer:'',consumer:'',minFlux:0,selectedId:null}); break;
    case 'run-fba':
    case 'run-pfba':
    case 'run-fva':
    case 'run-loopless':
      store.runAnalysis(action.slice('run-'.length)).catch((error) => {
        store.status = { ...store.status, state: 'error', message: error.message };
        store.emit('status');
      });
      break;
    case 'zoom-in': zoomBy(1.3); break;
    case 'zoom-out': zoomBy(1 / 1.3); break;
    case 'zoom-fit': camera.fitted = false; fitToStage(els.stage.querySelector('svg')); applyCamera(); break;
    case 'select-result-reaction':
      store.select(button.dataset.resultRxn);
      centerOnReaction(button.dataset.resultRxn);
      break;
    case 'metabolite-primary':
      store.setMapNodePrimary(button.dataset.node, true);
      break;
    case 'metabolite-secondary':
      store.setMapNodePrimary(button.dataset.node, false);
      break;
    case 'cell-reset-colors':
      store.setMapGeometryColors(button.dataset.cellId, null, { fill: null, stroke: null });
      break;
    case 'compartment-reset-colors':
      store.setMapGeometryColors(button.dataset.cellId, button.dataset.compartmentId, { fill: null, stroke: null });
      break;
    case 'duplicate-metabolite': {
      const copy = store.duplicateMapMetabolite(button.dataset.node);
      if (copy) store.selectEntity('metabolite', copy.id);
      break;
    }
    case 'hide-metabolite':
      if (store.setMapNodeHidden(button.dataset.node, true)) store.select(null);
      break;
    case 'remove-map-node':
      closeMapMenus();
      store.removeMapNode(button.dataset.node);
      break;
    case 'remove-map-text':
      cancelTextEditor();
      store.removeMapText(button.dataset.text);
      break;
    case 'reaction-linear': store.setMapReactionLinear(rxn, true); break;
    case 'reaction-free': store.setMapReactionLinear(rxn, false); break;
    case 'reaction-swap': store.swapMapReactionMetabolites(rxn, button.dataset.a, button.dataset.b); break;
    case 'reaction-line-end': store.setMapReactionLineEnd(rxn, button.dataset.node); break;
    case 'reaction-align': store.alignMapReaction(rxn, button.dataset.direction); break;
    case 'reaction-align-axis': store.alignMapReactionAxis(rxn, button.dataset.axis); break;
    case 'reaction-tidy': store.tidyMapReaction(rxn); break;
    case 'reaction-flip': store.flipMapReaction(rxn); break;
    case 'reaction-mirror': store.mirrorMapReaction(rxn); break;
    case 'reaction-rotate':
      store.rotateMapReaction(rxn, (Number(button.dataset.angle) * Math.PI) / 180);
      break;
    case 'knockout': runConstraintEdit(() => store.knockout(rxn)); break;
    case 'remove-map-reaction': boundaryMenu.close(); metaboliteMenu.close(); compartmentMenu.close(); store.removeMapReaction(rxn); break;
    case 'map-undo': finishMetaboliteLabelDragForHistory(); closeMapMenus(); store.undoMap(); break;
    case 'map-redo': finishMetaboliteLabelDragForHistory(); closeMapMenus(); store.redoMap(); break;
    case 'map-save': saveMap(); break;
    case 'clear-log':
      store.clearLog();
      break;
    case 'map-load': els.mapFileInput.click(); break;
    case 'node-style-reset': store.setMapNodeStyle({ fill: null, stroke: null }); break;
    case 'node-colors-reset': store.setMapNodeColors(listedIds(button.dataset.nodeIds), { fill: null, stroke: null }); break;
    case 'reaction-color-reset': store.setMapReactionColor(listedIds(button.dataset.rxnIds), null); break;
    case 'discard-waiting-map':
      store.discardWaitingMap();
      store.status = { ...store.status, state: 'ok', message: 'The waiting map was forgotten.' };
      store.emit('status');
      break;
    case 'import-models': els.modelFileInput.click(); break;
    case 'load-yeast9': loadYeast9Model(); break;
    case 'import-omics': els.omicsFileInput.click(); break;
    case 'clear-omics': store.clearOmics(); break;
    case 'select-gene': store.selectedReaction=null; store.selectedEntity={type:'gene',id:button.dataset.gene,speciesId:button.dataset.species}; store.emit('selection'); break;
    case 'select-model-reaction': store.select(button.dataset.rxn); break;
    case 'select-model-metabolite': selectModelMetabolite(button.dataset.metabolite); break;
    case 'import-metatranscriptomics': els.metatranscriptomicsFileInput.click(); break;
    case 'omics-toggle': store.setCommunityOmics({enabled: !store.omics.enabled}); break;
    case 'run-gimme': if (!store.solution?.steadyCom) store.runCommunityGIMME(); break;
    case 'disable-gimme': store.disableGIMME(); break;
    case 'omics-single': store.setCommunityOmics({mode:'single'}); break;
    case 'omics-compare': store.setCommunityOmics({mode:'compare'}); break;
    case 'add-map-compartment': {
      const select = els.sidebar.querySelector('#map-compartment-select');
      const key = String(select?.value ?? '');
      const [speciesId, compartmentId] = key.split('|');
      if (speciesId && compartmentId) {
        closeMapMenus();
        store.setCompartmentPlacement(key);
        store.addMapCompartment(speciesId, compartmentId);
      }
      break;
    }
    case 'new-metabolite':
      if (ui.workspace !== 'model') openWorkspace('model', button);
      store.setModelEditorMode(store.modelEditorMode === 'metabolite' ? null : 'metabolite');
      break;
    case 'new-reaction':
      if (ui.workspace !== 'model') openWorkspace('model', button);
      store.setModelEditorMode(store.modelEditorMode === 'reaction' ? null : 'reaction');
      break;
    case 'cancel-model-editor':
      discardModelEditorDraft();
      store.setModelEditorMode(null);
      break;
    case 'create-metabolite':
      createMetaboliteFromEditor(button);
      break;
    case 'create-reaction':
      createReactionFromEditor(button);
      break;
    case 'export-model-json': saveEditedModel('json'); break;
    case 'export-model-sbml': saveEditedModel('xml'); break;
    case 'map-empty': {
      closeWorkspace({ restoreFocus: false });
      closeMapMenus();
      camera.fitted = false;
      const cleared = store.map?.reactions?.size ?? 0;
      store.showEmptyMap();
      store.status = {
        ...store.status,
        state: 'ok',
        message: cleared
          ? `Map cleared — ${cleared} drawn reactions removed. The model is unchanged; Ctrl+Z undoes this.`
          : 'Map cleared. The model is unchanged.',
      };
      store.emit('status');
      break;
    }
    case 'map-core-overview':
      closeWorkspace({ restoreFocus: false });
      closeMapMenus();
      camera.fitted = false;
      store.showCoreMetabolismOverview();
      break;
    case 'map-full': {
      // On a genome-scale model this freezes the page and replaces the
      // drawing; ask first, where the layout presets simply refuse.
      const warning = store.fullMapWarning?.();
      if (warning && !window.confirm(warning)) break;
      closeWorkspace({ restoreFocus: false });
      closeMapMenus();
      camera.fitted = false;
      store.showFullMap();
      break;
    }
    case 'reset-reaction': runConstraintEdit(() => store.resetReaction(rxn)); break;
    case 'reset-all': runConstraintEdit(() => store.resetAllReactions()); break;
    case 'rx-filter': store.setReactionTable({ toggleFilter: button.dataset.filter }); break;
    case 'rx-sort': store.setReactionTable({ sortBy: button.dataset.sort }); break;
    case 'rx-group-toggle': store.setReactionTable({ toggleGroup: button.dataset.group }); break;
    case 'rx-open': store.select(button.dataset.rid); break;
    case 'rx-locate': locateReaction(button.dataset.rid); break;
    case 'rx-reveal': revealInReactions(button.dataset.rid, button); break;
    case 'rx-ko': {
      const id = button.dataset.rid;
      runConstraintEdit(() => (store.reactionKnockedOut(id) ? store.resetReaction(id) : store.knockout(id)));
      break;
    }
    case 'rx-reset': runConstraintEdit(() => store.resetReaction(button.dataset.rid)); break;
    case 'gene-ko': runConstraintEdit(() => store.toggleGeneKnockout(button.dataset.gene)); break;
    case 'rx-undo': runConstraintEdit(() => store.undoConstraints()); break;
    case 'rx-redo': runConstraintEdit(() => store.redoConstraints()); break;
    case 'rx-changes-toggle':
      ui.rx.showChanges = !ui.rx.showChanges;
      paintReactionsPage();
      break;
    case 'rx-split-toggle':
      ui.rx.split = !ui.rx.split;
      applyWorkspaceLayout();
      paintReactionsPage();
      break;
    case 'rx-export-csv': exportBoundsCSV(); break;
    case 'rx-import-csv': els.boundsFileInput?.click(); break;
    case 'rx-bulk': {
      // Read where the user typed: the draft follows the fields, the fields are the truth.
      const bar = button.closest('.rx-bulk-bar');
      for (const field of bar?.querySelectorAll('[data-field]') ?? []) ui.rx.bulk[field.dataset.field] = field.value;
      runConstraintEdit(() => store.bulkReactionAction(button.dataset.bulk, { ...ui.rx.bulk }));
      break;
    }
    case 'rx-select-none': store.clearReactionRowSelection(); break;
    case 'rx-objective-preset': runConstraintEdit(() => store.setObjective(button.dataset.preset)); break;
    case 'rx-objective-remove': runConstraintEdit(() => store.setObjectiveCoefficient(button.dataset.rid, 0)); break;
    case 'rx-objective-reset': runConstraintEdit(() => store.resetObjective()); break;
    case 'rx-compact-mode':
      ui.rx.compactMode = button.dataset.mode === 'shadow' ? 'shadow' : 'reactions';
      paintResults();
      break;
    case 'clear-selection': store.select(null); break;
    case 'search-add-reaction': {
      closeWorkspace({ restoreFocus: false });
      closeMapMenus();
      const result = store.addMapReaction(rxn);
      if (result) {
        store.select(rxn);
        centerOnReaction(rxn);
      }
      break;
    }
    case 'search-select-reaction':
      closeWorkspace({ restoreFocus: false });
      closeMapMenus();
      store.select(rxn);
      centerOnReaction(rxn);
      break;
    case 'pathway-enable-selection':
      closeWorkspace({ restoreFocus: false });
      closeMapMenus();
      cancelTextEditor();
      store.clearGeometrySelection();
      store.setView({ multiSelect: true, geometryEdit: false, textTool: false });
      break;
    case 'pathway-create': {
      const box = button.closest('.pathway-create');
      const name = box?.querySelector('[name="pathway-name"]')?.value ?? '';
      const color = box?.querySelector('[name="pathway-color"]')?.value ?? '#2563eb';
      try {
        const pathway = store.createManualPathway({ name, color });
        if (pathway) centerOnPathway(`manual:${pathway.id}`);
      } catch (error) {
        store.status = { ...store.status, state: 'error', message: error.message };
        store.emit('status');
      }
      break;
    }
    case 'pathway-select':
      closeWorkspace({ restoreFocus: false });
      closeMapMenus();
      store.selectPathway(button.dataset.pathwayKey);
      break;
    case 'pathway-focus':
      closeWorkspace({ restoreFocus: false });
      closeMapMenus();
      store.selectPathway(button.dataset.pathwayKey);
      centerOnPathway(button.dataset.pathwayKey);
      break;
    case 'pathway-delete': {
      const key = button.dataset.pathwayKey ?? '';
      if (key.startsWith('manual:')) store.deleteManualPathway(key.slice(7));
      break;
    }
    case 'toggle-currency': store.setView({ showCurrency: !store.view.showCurrency }); break;
    case 'toggle-currency-labels': store.setView({ currencyLabels: !store.view.currencyLabels }); break;
    case 'toggle-crossfeed': store.setView({ showCrossFeeding: !store.view.showCrossFeeding }); break;
    case 'crossfeed-show-all': store.setCrossFeedingTop(0); break;
    case 'crossfeed-show-none': store.setView({ crossFeedingShown: [] }); break;
    case 'crossfeed-show-top': store.setCrossFeedingTop(Number(button.dataset.top) || 5); break;
    case 'crossfeed-choose': openWorkspace('crossfeeding', button); break;
    case 'toggle-dark':
    case 'toggle-theme': setTheme(!store.view.dark); break;
    case 'toggle-geometry-edit':
      closeMapMenus();
      cancelTextEditor();
      store.setView({ geometryEdit: !store.view.geometryEdit, multiSelect: false, textTool: false });
      break;
    case 'toggle-multi-select':
      closeMapMenus();
      cancelTextEditor();
      store.clearGeometrySelection();
      store.setView({ multiSelect: !store.view.multiSelect, geometryEdit: false, textTool: false });
      break;
    case 'toggle-text-tool':
      closeMapMenus();
      cancelTextEditor();
      store.clearGeometrySelection();
      store.clearMultiSelection();
      store.setView({ textTool: !store.view.textTool, geometryEdit: false, multiSelect: false });
      break;
    case 'align-horizontal':
    case 'align-vertical':
    case 'align-left':
    case 'align-right':
    case 'align-top':
    case 'align-bottom':
    case 'distribute-horizontal':
    case 'distribute-vertical':
      closeMapMenus();
      store.arrangeMultiSelection(action);
      break;
    case 'show-hidden-metabolites':
      closeMapMenus();
      store.showAllHiddenMetabolites();
      break;
    case 'export-svg': exportSVG(); break;
    case 'export-png': exportPNG(); break;
    case 'export-reproducibility': exportReproducibility(); break;
    case 'export-methods-provenance': exportMethodsProvenance(); break;
    case 'import-reproducibility': els.reproducibilityFileInput.click(); break;
    case 'export-csv': exportFluxCSV(); break;
    case 'export-state': saveScenario(); break;
    case 'import-state': els.fileInput.click(); break;
    default: break;
  }

  if (button.closest('.top-menu')) button.closest('.top-menu').removeAttribute('open');
  if (!button.isConnected && !ui.workspace) {
    const restored = [...els.app.querySelectorAll('button[data-action]')].find(candidate =>
      Object.entries(button.dataset).every(([key,value]) => candidate.dataset[key] === value)
      && !candidate.closest('details:not([open])')
      && candidate.checkVisibility({ checkVisibilityCSS: true })
    );
    restored?.focus({ preventScroll: true });
  }
  // Settle layout changes before the next pointer gesture can snapshot the
  // camera. ResizeObserver alone runs after synchronous click handlers.
  syncStageViewport();
}

els.fileInput.addEventListener('change', async () => {
  const file = els.fileInput.files?.[0];
  if (!file) return;
  try {
    await store.loadState(await file.text());
    store.addLog('ok', `Scenario loaded from ${file.name}`);
    store.emit('status');
  } catch (error) {
    store.status = { ...store.status, state: 'error', message: `Could not load: ${error.message}` };
    store.emit('status');
  }
  els.fileInput.value = '';
});

els.mapFileInput.addEventListener('change', async () => {
  const file = els.mapFileInput.files?.[0];
  if (!file) return;
  try {
    closeMapMenus();
    camera.fitted = false;
    const was = store.scenario?.id ?? null;
    await store.loadMapDocument(await file.text(), { name: file.name });
    if (store.waitingMap) {
      store.status = {
        ...store.status,
        state: 'warn',
        message: `“${file.name}” does not fit what is open: ${store.waitingMap.reason} The map is kept and will be drawn as soon as ${store.waitingMap.needs} is loaded (File ▸ Model). The current drawing is unchanged.`,
      };
      store.emit('status');
      els.mapFileInput.value = '';
      return;
    }
    const moved = (store.scenario?.id ?? null) !== was;
    store.status = {
      ...store.status,
      // Anything the load had to change to make the file fit is said where it
      // will be read, not only kept for the results panel.
      state: store.mapLoadNote ? 'warn' : 'ok',
      message: `Map loaded: ${store.map.reactions.size} reactions, ${store.map.nodes.size} metabolites.`
        + (moved ? ` Switched to “${store.scenario?.name ?? store.scenario?.id}”, which this map was drawn over.` : '')
        + (store.mapLoadNote ?? ''),
    };
    store.emit('status');
  } catch (error) {
    store.status = { ...store.status, state: 'error', message: `Could not load the map: ${error.message}` };
    store.emit('status');
  }
  els.mapFileInput.value = '';
});

async function loadYeast9Model() {
  try {
    closeMapMenus();
    camera.fitted = false;
    store.status = {
      ...store.status,
      state: 'busy',
      message: 'Downloading the official Yeast9 9.1.1 model…',
    };
    store.emit('status');
    await store.importYeast9();
  } catch (error) {
    store.status = {
      ...store.status,
      state: 'error',
      message: `Could not load Yeast9: ${error.message}`,
    };
    store.emit('status');
  }
}

els.modelFileInput.addEventListener('change', async () => {
  const selected = [...(els.modelFileInput.files ?? [])];
  if (!selected.length) return;

  try {
    closeMapMenus();
    camera.fitted = false;
    store.status = {
      ...store.status,
      state: 'busy',
      message: `Reading ${selected.length} model file(s)…`,
    };
    store.emit('status');

    // MATLAB and Excel files are binary; SBML and COBRA JSON are text.
    const files = await Promise.all(selected.map(async (file) => (isBinaryModelFile(file.name)
      ? { name: file.name, bytes: new Uint8Array(await file.arrayBuffer()) }
      : { name: file.name, text: await file.text() })));

    await store.importModelFiles(files);
  } catch (error) {
    store.status = {
      ...store.status,
      state: 'error',
      message: `Could not load the model: ${error.message}`,
    };
    store.emit('status');
  }

  els.modelFileInput.value = '';
});

els.omicsFileInput.addEventListener('change', async () => {
  const file = els.omicsFileInput.files?.[0];
  if (!file) return;
  try {
    const text = await file.text();
    store.importOmicsCSV(text, {
      filename: file.name,
      dataType: store.omicsSettings.dataType,
    });
    store.status = {
      ...store.status,
      state: 'ok',
      message: `Omics loaded: ${file.name} · ${store.omicsDataset?.rowCount ?? 0} IDs · ${store.omicsDataset?.conditions?.length ?? 0} condition(s).`,
    };
    store.emit('status');
  } catch (error) {
    store.status = {
      ...store.status,
      state: 'error',
      message: `Could not load omics: ${error.message}`,
    };
    store.emit('status');
  }
  els.omicsFileInput.value = '';
});

els.boundsFileInput?.addEventListener('change', async () => {
  const file = els.boundsFileInput.files?.[0];
  if (!file) return;
  try {
    store.importBoundsCSV(await file.text(), { name: file.name });
  } catch (error) {
    store.status = { ...store.status, state: 'error', message: `Could not import bounds: ${error.message}` };
    store.emit('status');
  }
  els.boundsFileInput.value = '';
});

// The tables draw the rows in view only: scrolling draws the next ones. A box
// being typed in is committed first, as leaving it would.
for (const [container, mode] of [[els.sidebar, 'page'], [els.results, 'compact']]) {
  container.addEventListener('scroll', (event) => {
    if (event.target?.dataset?.rxScroll !== mode) return;
    const active = document.activeElement;
    if (event.target.contains(active) && isTextEntry(active) && active.closest('tbody')) active.blur();
    if (mode === 'page') ui.rx.pageScroll = event.target.scrollTop;
    else ui.rx.compactScroll = event.target.scrollTop;
    scheduleReactionRows(mode);
  }, true);
  container.addEventListener('mouseover', (event) => {
    const rid = event.target.closest?.('.rx-table tr[data-rid]')?.dataset.rid ?? null;
    if (rid !== ui.rx.hover) highlightReactionOnMap(rid);
  });
  container.addEventListener('mouseleave', () => highlightReactionOnMap(null));
}
// A resized page or dock shows a different number of rows.
const tableSizeObserver = new ResizeObserver(() => {
  if (ui.workspace === 'reactions') scheduleReactionRows('page');
  if (ui.resultsTab === 'table') scheduleReactionRows('compact');
});
tableSizeObserver.observe(els.sidebar);
tableSizeObserver.observe(els.results);

els.reproducibilityFileInput?.addEventListener('change', async () => {
  const file=els.reproducibilityFileInput.files?.[0]; if(!file)return;
  try { await store.importReproducibilityBundle(JSON.parse(await file.text())); }
  catch(error){store.status={...store.status,state:'error',message:`Could not replay the bundle: ${error.message}`};store.emit('status');}
  els.reproducibilityFileInput.value='';
});

async function exportMethodsProvenance(){
 try{
  let bundle=null;
  const saved=await saveFile(timestampedFileName(store.exportBaseName(),EXPORT_SUFFIX.methods,'md'),'text/markdown',async()=>{const report=await store.methodsProvenanceReport();bundle=report.bundle;return report.text;});
  if(!saved)return;
  store.status={...store.status,state:'ok',message:`Methods / Provenance report exported · ${bundle.metadata.software.gitCommit?'commit '+bundle.metadata.software.gitCommit.slice(0,12):'commit not injected'}`};store.emit('status');
 }catch(error){store.status={...store.status,state:'error',message:`Could not export the Methods report: ${error.message}`};store.emit('status');}
}

async function exportReproducibility(){
 try{
  let bundle=null;
  const saved=await saveFile(timestampedFileName(store.exportBaseName(),EXPORT_SUFFIX.repro,'json'),'application/json',async()=>{bundle=await store.reproducibilityBundle();return JSON.stringify(bundle,null,2);});
  if(!saved)return;
  store.status={...store.status,state:'ok',message:`Reproducibility bundle exported · SHA-256 ${bundle.integrity.payloadHash.slice(0,12)}…`};store.emit('status');
 }catch(error){store.status={...store.status,state:'error',message:`Could not export provenance: ${error.message}`};store.emit('status');}
}

els.metatranscriptomicsFileInput?.addEventListener('change', async () => {
  const file=els.metatranscriptomicsFileInput.files?.[0]; if(!file)return;
  try { store.importMetatranscriptomics(await file.text(), file.name); }
  catch(error){ store.status={...store.status,state:'error',message:`Could not load metatranscriptomics: ${error.message}`}; store.emit('status'); }
  els.metatranscriptomicsFileInput.value='';
});

// One dropdown open at a time: File sits right above View, and two open ones
// stack on top of each other. `toggle` does not bubble, hence the capture.
document.addEventListener('toggle', (event) => {
  const menu = event.target;
  if (!menu.open || !menu.matches?.('.canvas-view, .top-menu')) return;
  document.querySelectorAll('.canvas-view[open], .top-menu[open]').forEach((other) => {
    if (other !== menu) other.open = false;
  });
}, true);

window.addEventListener('keydown', (event) => {
  if (event.key === 'Escape' && metaboliteLabelDragGesture) {
    event.preventDefault();
    finishMetaboliteLabelDrag({ pointerId: metaboliteLabelDragGesture.pointerId }, { cancel: true });
    return;
  }
  if (event.key === 'Escape' && ui.workspace) { event.preventDefault(); closeWorkspace(); return; }
  if (event.key === 'Escape' && els.app.classList.contains('is-task-overlay')) {
    els.app.classList.remove('is-task-overlay');
    els.app.querySelector('.canvas-toolbar [data-action="toggle-left"]')?.focus();
    return;
  }
  if (event.key === 'Escape' && document.querySelector('.canvas-view[open], .top-menu[open]')) {
    document.querySelectorAll('.canvas-view[open], .top-menu[open]').forEach(menu => menu.open = false);
    return;
  }
  if (event.key === 'Escape' && els.app.classList.contains('is-results-maximized')) {
    els.app.classList.remove('is-results-maximized');
    return;
  }
  if (event.target.matches('input, select, textarea')) return;

  const mod = event.ctrlKey || event.metaKey;
  if (mod && event.key.toLowerCase() === 'f') {
    event.preventDefault();
    openWorkspace('library');
    const input = els.sidebar.querySelector('#reaction-search-input');
    input?.focus();
    input?.select();
    return;
  }
  if (mod && (event.key.toLowerCase() === 'z' || event.key.toLowerCase() === 'y') && mapDragInProgress()) {
    event.preventDefault();
    return;
  }
  if (mod && event.key.toLowerCase() === 'z') {
    event.preventDefault();
    finishMetaboliteLabelDragForHistory();
    closeMapMenus();
    if (event.shiftKey) store.redoMap();
    else store.undoMap();
    return;
  }
  if (mod && event.key.toLowerCase() === 'y') {
    event.preventDefault();
    finishMetaboliteLabelDragForHistory();
    closeMapMenus();
    store.redoMap();
    return;
  }

  if (event.key === 'Escape') {
    if (textEditor) { cancelTextEditor(); return; }
    closeMapMenus();
    store.clearMultiSelection();
    if (store.view.textTool) store.setView({ textTool: false });
    // A selected compartment shows its handles in Select too, not only in
    // Geometry Edit, so Escape drops it either way.
    store.clearGeometrySelection();
    if (!store.view.geometryEdit) store.select(null);
    ui.tool = 'select';
    paintShell();
    return;
  }
  if (event.key === 'Delete' || event.key === 'Backspace') {
    if (store.multiSelectionSize) {
      event.preventDefault();
      closeMapMenus();
      store.removeMultiSelection();
      return;
    }
    const selected = store.selectedEntity;
    // A selected compartment or cell goes with what is drawn in it. Read from
    // what is selected, not from the handles: they can outlive a click on a
    // reaction, which must delete the reaction. The shared medium (no cell)
    // is the whole map and is not deleted.
    const compartment = selected?.type === 'compartment' && selected.cellId
      ? { cellId: selected.cellId, compartmentId: selected.id }
      : selected?.type === 'cell' ? cellRootOf(selected.id) : null;
    if (compartment) {
      event.preventDefault();
      closeMapMenus();
      store.removeMapCompartment(compartment.cellId, compartment.compartmentId);
      return;
    }
    if (selected?.type === 'reaction') store.removeMapReaction(selected.id);
    else if (selected?.type === 'metabolite' && store.canRemoveMapNode(selected.id)) store.removeMapNode(selected.id);
    else if (selected?.type === 'text') store.removeMapText(selected.id);
    return;
  }
  if (event.key === '0') { camera.fitted = false; fitToStage(els.stage.querySelector('svg')); applyCamera(); }
  if (event.key === '+' || event.key === '=') zoomBy(1.3);
  if (event.key === '-') zoomBy(1 / 1.3);
});

let previousStageSize = null;
function syncStageViewport() {
  const { width, height } = els.stage.getBoundingClientRect();
  // Maximized results temporarily give the map a zero-height grid track. Keep
  // its last usable viewport so restoring the dock does not recenter twice.
  if (width <= 0 || height <= 0) return;
  if (previousStageSize && camera.fitted && width > 0 && height > 0) {
    camera.x += (width - previousStageSize.width) / 2;
    camera.y += (height - previousStageSize.height) / 2;
    applyCamera();
  }
  previousStageSize = { width, height };
  els.app.classList.toggle('is-short-workspace', els.app.querySelector('.workspace').getBoundingClientRect().height < 500);
}
const stageSizeObserver = new ResizeObserver(syncStageViewport);
stageSizeObserver.observe(els.stage);

// Helpers

const timers = new Map();
/**
 * Whether a numeric field holds a number to apply.
 *
 * A cleared or half-typed field ("", "-", "1e") must not reach the model as 0,
 * or emptying an upper bound of 1000 would close the reaction. While the user types
 * such a field is marked invalid and nothing is applied; when they leave it, it
 * goes back to the value still in force and the header says why.
 */
function acceptNumber(el, event, current) {
  if (Number.isFinite(el.valueAsNumber)) {
    el.removeAttribute('aria-invalid');
    return true;
  }
  if (event.type === 'change') {
    el.value = String(current());
    el.removeAttribute('aria-invalid');
    store.status = {
      ...store.status,
      state: 'warn',
      message: 'That field was empty or not a number, so nothing changed; it shows the value still in force.',
    };
    store.emit('status');
  } else {
    el.setAttribute('aria-invalid', 'true');
  }
  return false;
}

function debounce(key, fn, ms) {
  clearTimeout(timers.get(key));
  timers.set(key, setTimeout(fn, ms));
}

/** Coalesce slider drags into one solve per animation frame. */
function scheduleSolve() {
  if (pendingSolve) return;
  pendingSolve = requestAnimationFrame(() => {
    pendingSolve = null;
    store.solve();
  });
}

async function createMetaboliteFromEditor(button) {
  const form = button.closest('.model-editor-form');
  if (!form) return;
  const value = (name) => form.querySelector(`[name="${name}"]`)?.value ?? '';

  try {
    button.disabled = true;
    await store.createModelMetabolite({
      id: value('id'),
      name: value('name'),
      compartment: value('compartment'),
      formula: value('formula'),
      charge: value('charge'),
    });
    discardModelEditorDraft('metabolite');
    store.setModelEditorMode(null);
  } catch (error) {
    button.disabled = false;
    showModelEditError(error);
  }
}

async function createReactionFromEditor(button) {
  const form = button.closest('.model-editor-form');
  if (!form) return;
  const value = (name) => form.querySelector(`[name="${name}"]`)?.value ?? '';
  const checked = (name) => Boolean(form.querySelector(`[name="${name}"]`)?.checked);

  try {
    button.disabled = true;
    const result = await store.createModelReaction({
      id: value('id'),
      name: value('name'),
      metabolites: value('stoichiometry'),
      lowerBound: value('lower'),
      upperBound: value('upper'),
      geneReactionRule: value('gpr'),
      subsystem: value('subsystem'),
    }, { addToMap: checked('addToMap') });

    discardModelEditorDraft('reaction');
    store.setModelEditorMode(null);
    if (result?.mapResult) centerOnReaction(result.mergedId);
  } catch (error) {
    button.disabled = false;
    showModelEditError(error);
  }
}

function showModelEditError(error) {
  store.status = {
    ...store.status,
    state: 'error',
    message: `Could not edit the model: ${error.message}`,
  };
  store.emit('status');
  const card = els.sidebar.querySelector('.model-editor-card');
  if (card) {
    let box = card.querySelector('.model-editor-error');
    if (!box) {
      box = document.createElement('p');
      box.className = 'model-editor-error';
      card.appendChild(box);
    }
    box.textContent = error.message;
  }
}

function selectModelMetabolite(mergedId) {
  const node=[...(store.map?.nodes?.values?.()??[])].find(n=>n.mergedId===mergedId);
  if(node){store.selectedReaction=null;store.selectedEntity={type:'metabolite',id:node.id};store.emit('selection');return;}
  store.status={...store.status,state:'ok',message:`Metabolite ${mergedId} exists in the model but is not currently drawn on the map.`};store.emit('status');
}

function closeMapMenus() {
  boundaryMenu.close();
  metaboliteMenu.close();
  compartmentMenu.close();
}

function activateTool(tool) {
  ui.tool = tool;
  // The pointer halos must not swallow presses meant for the canvas underneath.
  els.stage.classList.toggle('is-drawing-tool', tool === 'cell' || tool === 'compartment' || tool === 'compartment-edit' || tool === 'cell-edit');
  closeMapMenus();
  cancelTextEditor();
  store.clearMultiSelection();
  if (tool === 'text') {
    store.setView({ textTool: true, geometryEdit: false, multiSelect: false });
  } else if (tool === 'cell' || tool === 'compartment' || tool === 'compartment-edit' || tool === 'cell-edit') {
    store.setView({ textTool: false, geometryEdit: true, multiSelect: false });
  } else {
    store.setView({ textTool: false, geometryEdit: false, multiSelect: false });
  }
  paintShell();
}

function setTheme(dark) {
  document.documentElement.dataset.theme = dark ? 'dark' : 'light';
  try { localStorage.setItem('coxeter-theme', dark ? 'dark' : 'light'); } catch (_) {}
  store.setView({ dark });
}

async function saveMap() {
  // Before a model has loaded there is no map; say so rather than throw.
  if (!store.map) {
    store.status = { ...store.status, state: 'warn', message: 'There is no map to save yet.' };
    store.emit('status');
    return;
  }
  const filename = await download(store.fileNameFor(EXPORT_SUFFIX.map, 'json'), () => store.mapJSON(), 'application/json');
  if (!filename) return;
  store.markMapSaved();
  store.addLog('ok', `Map saved as ${filename}`);
  store.emit('status');
}

async function saveEditedModel(ext) {
  const sp = store.modelEditorSpecies();
  const model = store.modelEditorModel();
  if (!sp || !model) return;
  const filename = ext === 'xml'
    ? await download(exportFileName(model.id || sp.id, EXPORT_SUFFIX.model, 'xml'), () => store.modelSBML(sp.id), 'application/sbml+xml')
    : await download(exportFileName(model.id || sp.id, EXPORT_SUFFIX.model, 'json'), () => store.modelJSON(sp.id), 'application/json');
  if (!filename) return;
  store.markModelSaved(model);
  store.addLog('ok', `Model ${sp.id} saved as ${filename}`);
  store.emit('status');
}

async function exportFluxCSV() {
  const filename = await download(store.fileNameFor(EXPORT_SUFFIX.fluxes, 'csv'), () => store.fluxCSV(), 'text/csv');
  if (!filename) return;
  store.addLog('ok', `Flux table exported as ${filename}`);
  store.emit('status');
}

async function saveScenario() {
  const filename = await download(store.fileNameFor(EXPORT_SUFFIX.scenario, 'json'), () => store.stateJSON(), 'application/json');
  if (!filename) return;
  store.markScenarioSaved();
  store.addLog('ok', `Scenario saved as ${filename}`);
  store.emit('status');
}

async function exportSVG() {
  if (!store.layout || !store.scale) return;
  const filename = await download(store.fileNameFor(EXPORT_SUFFIX.map, 'svg'), () => exportableSVG(), 'image/svg+xml');
  if (!filename) return;
  store.addLog('ok', `Map exported as ${filename}`);
  store.emit('status');
}

function exportableSVG() {
  if (!store.layout || !store.scale) return null;
  // Drawn afresh rather than copied from the screen, so the file holds the map
  // and not the editor: no selection, handles, pointer targets or resize grips,
  // and no zoom-dependent hiding of labels. Parsed as HTML, like the stage,
  // which accepts the renderer's valueless attributes.
  const template = document.createElement('template');
  // Hover text goes into the file: a reaction or metabolite says what it is
  // and what flux it carries wherever the SVG is opened.
  template.innerHTML = renderMap(store.layout, { ...mapRenderState(), forExport: true, tooltips: store.exportTooltips() });
  const clone = template.content.querySelector('svg');
  if (!clone) return null;
  const svgNS = 'http://www.w3.org/2000/svg';
  const about = store.exportMetadata();
  const title = document.createElementNS(svgNS, 'title');
  title.textContent = about.scenario.name ?? store.exportBaseName();
  const metadata = document.createElementNS(svgNS, 'metadata');
  metadata.textContent = JSON.stringify(about);
  const css = [...document.styleSheets]
    .flatMap((sheet) => { try { return [...sheet.cssRules].map((r) => r.cssText); } catch { return []; } })
    .filter((rule) => rule.includes('.emc-'))
    .join('\n');
  const style = document.createElementNS('http://www.w3.org/2000/svg', 'style');
  style.textContent = css;
  clone.insertBefore(style, clone.firstChild);
  if (store.isZunigaStudy) {
    const study = document.createElementNS(svgNS, 'metadata');
    study.textContent = JSON.stringify({ ...store.zunigaState(), method: 'Stationary source-model FBA', units: 'Source-model flux units; growth h^-1', solutionStatus: store.solution?.status, growth: Object.fromEntries(store.growth), adaptation: 'Not a dynamic or experimental trajectory reproduction.' });
    clone.insertBefore(study, clone.firstChild);
  }
  clone.insertBefore(metadata, clone.firstChild);
  clone.insertBefore(title, clone.firstChild);
  return new XMLSerializer().serializeToString(clone);
}

async function exportPNG() {
  if (!store.layout || !store.scale) return;
  let url = null;
  try {
    let width = 0;
    let height = 0;
    let used = 0;
    // Drawn once the file has a place to go: the Save As window has to open
    // while the click still counts, and a large map takes a while to draw.
    const filename = await saveFile(store.fileNameFor(EXPORT_SUFFIX.map, 'png'), 'image/png', async () => {
      const svg = exportableSVG();
      if (!svg) throw new Error('the map could not be drawn.');
      url = URL.createObjectURL(new Blob([svg], { type: 'image/svg+xml;charset=utf-8' }));
      const image = new Image();
      await new Promise((resolve, reject) => { image.onload = resolve; image.onerror = () => reject(new Error('The SVG could not be rasterized.')); image.src = url; });
      width = image.naturalWidth || store.map.width;
      height = image.naturalHeight || store.map.height;
      // The largest scale this browser's canvas accepts, 2× when the map allows.
      let refusal = null;
      for (const scale of rasterScales(width, height)) {
        const attempt = await rasterize(image, width, height, scale);
        if (attempt.png) { used = scale; return attempt.png; }
        refusal = attempt.error ?? refusal;
      }
      const detail = refusal?.message ? ` (${refusal.message})` : '';
      throw new Error(`the map is too large for this browser to rasterize${detail}. Export the SVG instead; it keeps full resolution.`);
    });
    if (!filename) return;
    if (used >= PREFERRED_SCALE) {
      store.addLog('ok', `Map exported as ${filename} (${Math.round(width * used)} × ${Math.round(height * used)} px)`);
      store.emit('status');
    } else {
      store.status = {
        ...store.status,
        state: 'warn',
        message: `PNG exported at ${used}× (${Math.round(width * used)} × ${Math.round(height * used)} px): a ${PREFERRED_SCALE}× image of this map exceeds the browser's canvas limit. The SVG keeps full resolution.`,
      };
      store.emit('status');
    }
  } catch (error) {
    store.status = { ...store.status, state: 'error', message: `Could not export PNG: ${error.message}` };
    store.emit('status');
  } finally { if (url) URL.revokeObjectURL(url); }
}

/**
 * One attempt at a PNG: `{ png }`, or `{ png: null, error }` when the browser
 * cannot make a canvas this large.
 */
async function rasterize(image, width, height, scale) {
  const canvas = document.createElement('canvas');
  try {
    // Some browsers refuse an oversized canvas by returning null, others by
    // throwing; either way the next, smaller scale is tried.
    canvas.width = Math.max(1, Math.round(width * scale));
    canvas.height = Math.max(1, Math.round(height * scale));
    const context = canvas.getContext('2d');
    if (!context) return { png: null, error: null };
    context.fillStyle = store.view.dark ? '#0b3046' : '#ffffff';
    context.fillRect(0, 0, canvas.width, canvas.height);
    context.drawImage(image, 0, 0, canvas.width, canvas.height);
    return { png: await new Promise(resolve => canvas.toBlob(resolve, 'image/png')), error: null };
  } catch (error) {
    // A canvas the browser will not export (tainted) is not a size problem: no
    // smaller scale would help, and calling it "too large" would mislead.
    if (error?.name === 'SecurityError') throw error;
    return { png: null, error };
  } finally {
    // Give an oversized backing store back before the next, smaller attempt.
    canvas.width = 0;
    canvas.height = 0;
  }
}

/**
 * The findings and the adaptations of a published community, as one Markdown
 * file. They go together because they only mean anything together: a list of
 * matched numbers without the departures that produced them is the half of the
 * story that flatters the result.
 */
function downloadFindings(communityId) {
  const c = store.publishedCommunities?.find((x) => x.id === communityId);
  if (!c) return;
  const rep = c.reproduction;
  const bullets = (items) => (items?.length ? items.map((t) => `- ${t}`).join('\n') : '_none recorded_');

  const text = [
    `# ${c.name}`,
    '',
    `**${c.paper.title}**`,
    `${c.paper.authors} (${c.paper.year}), ${c.paper.citation ?? c.paper.journal}`,
    `doi:${c.paper.doi}${c.paper.doiVerifiedFrom ? ` — verified against ${c.paper.doiVerifiedFrom}` : ' — citation unverified'}`,
    '',
    `Framework: ${c.paper.framework}  ·  Status: ${rep.status}`,
    '',
    c.description,
    '',
    '## Reproduced paper findings',
    '',
    bullets(rep.reproduces),
    '',
    '## Adaptations and assumptions',
    '',
    bullets(rep.adaptations),
    ...(rep.needs?.length ? ['', '## Still required to reproduce', '', bullets(rep.needs)] : []),
    ...(c.computed ? ['', '## Recorded result', '', '```json',
      JSON.stringify(c.computed, null, 2), '```'] : []),
    '',
  ].join('\n');

  download(exportFileName(communityId, EXPORT_SUFFIX.findings, 'md'), text, 'text/markdown');
}

/** Download the full member model from the standalone's embedded data. */
async function downloadMemberJSON(communityId, memberId) {
  if (!store.publishedCommunities?.some((c) => c.id === communityId)) return;
  try {
    await saveFile(memberModelFileName(communityId, memberId, 'json'), 'application/json', async () => {
      if (store.isZunigaStudy && communityId === store.scenario.published) {
        const model = store.models.get(`published/${communityId}/model-${memberId}`);
        if (!model) throw new Error('The privately loaded model is unavailable.');
        return JSON.stringify(model.toJSON(), null, 2);
      }
      const manifest = await store.publishedManifest(communityId);
      const member = manifest?.members?.find((m) => m.id === memberId);
      if (!member) throw new Error(`${memberId} has no model file in this build.`);
      const model = await store.loadModel(`published/${communityId}/${member.model.replace(/\.json$/, '')}`);
      return JSON.stringify(model.toJSON(), null, 2);
    });
  } catch (error) {
    store.status = { ...store.status, state: 'error', message: `Could not export JSON: ${error.message}` };
    store.emit('status');
  }
}

/** Only the manifest's medium and bounds files can be downloaded as CSV. */
async function downloadCommunityCSV(communityId, file) {
  if (!store.publishedCommunities?.some((c) => c.id === communityId)) return;
  try {
    // `media.csv` → `steadycom-gut-nine_media.csv`.
    await saveFile(communityDataFileName(communityId, file), 'text/csv', async () => {
      const manifest = await store.publishedManifest(communityId);
      if (!file || ![manifest?.media, manifest?.bounds].includes(file)) throw new Error('This community has no such CSV file.');
      const text = globalThis.__ESCHER_MCMC_TEXT__?.[`models/published/${communityId}/${file}`];
      if (typeof text !== 'string') throw new Error('This CSV file is not embedded in the standalone.');
      return text;
    });
  } catch (error) {
    store.status = { ...store.status, state: 'error', message: `Could not export CSV: ${error.message}` };
    store.emit('status');
  }
}

/** Convert a full member model to SBML, using embedded data when available. */
async function downloadMemberSBML(communityId, memberId) {
  if (!communityId || !memberId) return;
  try {
    await saveFile(memberModelFileName(communityId, memberId, 'xml'), 'application/sbml+xml',
      async () => (await store.publishedMemberSBML(communityId, memberId)).xml);
  } catch (error) {
    store.status = {
      ...store.status,
      state: 'error',
      message: `Could not export SBML: ${error.message}`,
    };
    store.emit('status');
  }
}

/**
 * Save through a Save As window, where the user picks the folder and may
 * rename the file. `content` is the data or a function that makes it. Resolves
 * to the name it was saved under, or null when the user cancelled or the save
 * failed, which is reported.
 */
async function download(name, content, type) {
  try {
    return await saveFile(name, type, content);
  } catch (error) {
    store.status = { ...store.status, state: 'error', message: `Could not save ${name}: ${error.message}` };
    store.emit('status');
    return null;
  }
}

// Boot

store.subscribe((_, reason) => {
  if (!['status', 'selection', 'variability'].includes(reason)) storeRevision += 1;
  paintStatus();
  paintShell();
  if (reason === 'status') {
    paintResults();
    return;
  }
  // How the reactions table lists rows: nothing else changed.
  if (reason === 'reaction-table') {
    paintReactionTables();
    return;
  }
  if (reason === 'variability' || reason === 'selection') {
    paintInspector();
    paintResults();
    if (reason === 'selection') {
      paintMap();
      // A reaction picked on the map is found in the table.
      if (ui.workspace === 'reactions' && !revealReactionRow(store.selectedReaction, 'page')) paintReactionRows('page');
      if (ui.resultsTab === 'table') revealReactionRow(store.selectedReaction, 'compact');
    }
    return;
  }
  paintSidebar();
  paintInspector();
  paintResults();
  paintMap();
});

(async function boot() {
  try {
    await store.init();
    const requested = new URLSearchParams(location.search).get('scenario');
    await store.useScenario(requested || store.scenarios[0].id);
  } catch (error) {
    els.stage.innerHTML = `<p style="padding:30px;color:var(--danger)">${error.message}</p>`;
    els.statusText.textContent = error.message;
    els.status.classList.add('is-error');
  }
})();

// Small surface for the browser checks in tools/: the camera, the coordinate
// conversion and which gesture is live are exactly what a drag test needs.
window.escherMCMC = {
  store,
  camera,
  screenToMap,
  openWorkspace,
  closeWorkspace,
  setTheme,
  gestures: () => ({
    pan: panGesture,
    node: nodeDragGesture,
    metaboliteLabel: metaboliteLabelDragGesture,
    compartmentLabel: compartmentLabelDragGesture,
    reactionPick: reactionPickGesture,
    reaction: reactionDragGesture,
    rotate: rotateDragGesture,
    control: controlDragGesture,
    geometry: geometryDragGesture,
    cellDraw: cellDrawGesture,
    compartmentDraw: compartmentDrawGesture,
    selection: selectionGesture,
    group: groupDragGesture,
    pinch: pinchGesture,
    touchPointers: touchPointers.size,
  }),
};
