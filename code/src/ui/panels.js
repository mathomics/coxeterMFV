/**
 * Sidebar panels. Each one renders to an HTML string and is wired through event
 * delegation on the sidebar element, so a re-render never leaves stale
 * listeners behind.
 */

import { annotationLinks } from '../core/provenance.js';
import { readableName } from '../core/label-sources.js';
import {
  compartmentColors, cellCompartmentName, cellCompartmentVisualId,
} from '../core/compartments.js';
import { formatFlux } from '../render/theme.js';
import { CROSSFEED_WIDTH_RANGE } from '../render/crossfeed-routes.js';
import { crossFeedingColors } from '../render/crossfeed-colors.js';
import { COMPARTMENT_SHAPES, compartmentShape } from '../layout/geometry.js';
import { normalizeHexColor } from '../core/colors.js';
import { TEXT_FONTS, TEXT_SIZE, DEFAULT_TEXT_FONT } from '../core/text-style.js';
import { OBJECTIVES } from '../core/community.js';
import { ZUNIGA_ID, ZUNIGA_SCENARIOS } from '../communities/zuniga.js';
import { boundControl, renderCompactReactionsTable, renderReactionsPage } from './reactions-panel.js';
import { selectionMainAxis, selectionSize, spacingItems } from '../map/multi-selection.js';

const esc = (s) => String(s ?? '')
  .replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
  .replace(/"/g, '&quot;');

/** Medium components worth putting a slider on, in the order they matter. */
const MEDIUM_ORDER = ['glc__D', 'o2', 'photon', 'co2', 'hco3', 'ac', 'etoh', 'lac__L', 'lac__D', 'for', 'h2', 'succ', 'pyr', 'glyc', 'nh4', 'pi'];
const MEDIUM_LABELS = {
  glc__D: 'Glucose', o2: 'Oxygen', photon: 'Light (photons)', co2: 'CO₂',
  hco3: 'Bicarbonate', ac: 'Acetate', etoh: 'Ethanol', lac__L: 'L-lactate',
  lac__D: 'D-lactate', for: 'Formate', h2: 'H₂', succ: 'Succinate',
  pyr: 'Pyruvate', glyc: 'Glycerol', nh4: 'Ammonium', pi: 'Phosphate',
};

/**
 * The sidebar's two tabs as HTML, with the cards grouped by concern: how the
 * map is drawn goes under Edit Map, what the community is made of goes under
 * Edit Model. The inspector has its own panel and is not here.
 */
export function renderSidebar(store) {
  return `<div class="left-section" data-left-section="build">
    ${displayCard(store)}
    ${reactionSearchCard(store)}
    ${pathwayManagerCard(store)}
    ${omicsOverlayCard(store)}
    ${communityOmicsCard(store)}
    ${mapStructureCard(store)}
    ${legendCard(store)}
    ${modelOverviewCard(store)}
  </div>
  <div class="left-section" data-left-section="model">
    ${scenarioCard(store)}
    ${speciesCard(store)}
    ${modelEditorCard(store)}
    ${mediumCard(store)}
    ${crossFeedingCard(store)}
    ${modelOverviewCard(store)}
  </div>`;
}

/**
 * The task hub of the left panel for `ui.leftTab` ('build', 'model' or
 * 'analysis'), as HTML.
 *
 * The fixed task hub contains only a bounded number of controls. Data-dependent
 * lists and forms belong in the workspace, so a large model never makes this
 * navigation area grow or introduces a second scroll beside the map.
 */
export function renderTaskPanel(store, ui = {}) {
  const tab = ui.leftTab ?? 'build';
  const link = (workspace, icon, title) =>
    `<button class="task-link" data-action="open-workspace" data-workspace="${workspace}" type="button">
      <span class="task-link-icon" aria-hidden="true">${icon}</span>
      <span class="task-link-copy"><strong>${title}</strong></span>
      <span class="task-link-arrow" aria-hidden="true">›</span>
    </button>`;

  if (tab === 'model') {
    const count = store.species.filter((species) => species.enabled).length;
    return `<section class="task-deck task-hub" data-task-hub="model">
      <div class="tool-context task-context"><span class="section-label">Community model</span>
        <strong>${count} active species</strong>
      </div>
      <nav class="task-links" aria-label="Model editors">
        ${link('scenario', '◇', 'Scenario & objective')}
        ${link('community', '⬡', 'Species & abundance')}
        ${link('medium', '◌', 'Shared medium')}
        ${link('reactions', '☰', 'Reactions & bounds')}
        ${link('model', '⌁', 'Biological model')}
        ${link('omics', '≋', 'Omics & expression')}
      </nav>
    </section>`;
  }

  if (tab === 'analysis') {
    const fixedMu = Boolean(store.solution?.steadyCom);
    const method = currentAnalysisMethod(store);
    const objective = fixedMu ? `µ = ${store.solution.steadyCom.mu} h⁻¹`
      : store.customObjective ? `Edited objective (${store.customObjective.sense === 'min' ? 'minimised' : 'maximised'})`
        : store.objective === OBJECTIVES.SUM_BIOMASS ? 'Weighted biomass sum' : 'Equal growth';
    const status = store.status?.state === 'busy' ? 'Computing…'
      : store.solution?.status === 'optimal' ? 'Optimal solution'
        : store.solution?.status ?? 'Ready to analyse';
    const parameters = fixedMu ? 'Abundances are outputs of this solve' : store.omics?.gimmeEnabled
      ? `Objective target: ${Math.round(100 * (store.omics.gimmeObjectiveFraction ?? 0.9))}%`
      : store.loopless ? 'Internal cycles excluded'
        : store.cycleFree ? 'Internal cycles removed'
        : store.parsimonious ? `Biomass target: ${formatPercent(store.pfbaBiomassPercent)}% of optimum`
          : 'Community objective maximisation';
    return `<section class="task-deck task-hub" data-task-hub="analysis">
      <div class="tool-context task-context"><span class="section-label">Current simulation</span>
        <strong>${esc(method)}</strong><p>${esc(objective)} · ${esc(status)}</p>
        <small class="task-parameters">${esc(parameters)}</small>
      </div>
      <div class="analysis-methods" role="group" aria-label="Run simulation">
        <button data-action="run-fba" type="button"><strong>FBA</strong></button>
        <button data-action="run-pfba" type="button"><strong>pFBA</strong></button>
        <button data-action="run-fva" type="button"${fixedMu ? ' disabled title="Run an ordinary analysis before FVA"' : ''}><strong>FVA</strong></button>
        <button data-action="run-loopless" type="button"><strong>Loopless</strong></button>
        <button data-action="run-cyclefree" type="button"><strong>CycleFreeFlux</strong></button>
      </div>
      <nav class="task-links" aria-label="Analysis tools">
        ${link('analysis', '⚙', 'Simulation settings')}
        ${link('reactions', '☰', 'Reactions & bounds')}
        ${link('crossfeeding', '⇄', 'Cross-feeding explorer')}
      </nav>
      <button class="task-results-button" data-action="show-results" type="button">View simulation results <span aria-hidden="true">↗</span></button>
    </section>`;
  }

  const tool = ui.tool ?? 'select';
  const help = {
    select: ['Select & inspect', 'Click an object to inspect it, or drag it to move it.'],
    edit: ['Edit Control Points', 'Drag control points and Bézier handles to reshape reaction paths.'],
    reaction: ['Add Reaction mode', 'Click a metabolite to add a reaction from it.'],
    metabolite: ['Add Metabolite mode', 'Click a compartment, then pick a metabolite.'],
    text: ['Add map text', 'Click the map, type, and press Enter.'],
    compartment: ['Add Compartment', 'Pick a compartment and a shape, then drag inside its highlighted parent.'],
    cell: ['Add Cell', cellToolHelp(store)],
  }[tool] ?? ['Edit your map', 'Choose an editing tool.'];

  return `<section class="task-deck task-hub" data-task-hub="build">
    <div class="tool-context task-context"><span class="section-label">${esc(help[0])}</span><p>${esc(help[1])}</p>${tool === 'compartment' ? compartmentToolControls(store) : ''}</div>
    <nav class="task-links" aria-label="Map editors">
      ${link('library', '⌕', 'Reaction library')}
      ${link('structure', '⬡', 'Structure')}
      ${link('map', '⚙', 'Map options')}
    </nav>
  </section>`;
}

/**
 * What a drag with the Cell tool draws, picked as addMapCell picks it (the
 * species chosen in Structure, else the first not on the map), or that there
 * is nothing left to draw, where a drag would otherwise do nothing at all.
 */
function cellToolHelp(store) {
  const available = store.availableMapCells?.() ?? [];
  const next = available.find((cell) => cell.speciesId === store.cellPlacementSpeciesId) ?? available[0];
  return next ? `Drag on the map to draw ${next.speciesName}.` : 'Every species is on the map.';
}

function currentAnalysisMethod(store) {
  if (store.solution?.steadyCom) return 'SteadyCom fixed µ';
  const method = store.solution?.gimme || store.omics?.gimmeEnabled ? 'GIMME'
    : store.loopless ? 'Loopless FBA'
      : store.cycleFree ? 'CycleFreeFlux'
        : store.parsimonious ? 'pFBA' : 'FBA';
  return store.fvaEnabled && !store.loopless ? `${method} + FVA` : method;
}

const WORKSPACE_TITLES = {
  library: 'Reaction library', structure: 'Map structure', map: 'Map options',
  scenario: 'Scenario & objective', community: 'Species & abundance', medium: 'Shared medium',
  model: 'Biological model', omics: 'Omics & expression', analysis: 'Analysis settings',
  settings: 'Analysis settings', crossfeeding: 'Cross-feeding explorer', overview: 'Model overview',
  reactions: 'Reactions & bounds',
};

/** The heading shown for a workspace id. */
export function getWorkspaceTitle(workspace) {
  return WORKSPACE_TITLES[workspace] ?? 'Workspace';
}

/**
 * The HTML of one workspace (`library`, `scenario`, `medium`, `reactions`,
 * ...), one full editor at a time. Its cards keep the data-actions and form
 * ids they have in the sidebar, so the same delegated handlers drive them.
 * `reactions` is the Reactions page's view state (see renderReactionsPage).
 */
export function renderWorkspace(store, workspace, { fluxColorDraft = null, steadyComDraft = null, steadyComPending = false, reactions = null } = {}) {
  const content = {
    library: () => reactionSearchCard(store),
    structure: () => mapStructureCard(store) + mapCanvasCard(store),
    map: () => displayCard(store, { workspace: true }) + mapAppearanceCard(store, fluxColorDraft) + pathwayManagerCard(store) + mapCanvasCard(store) + legendCard(store),
    scenario: () => scenarioCard(store, { steadyComDraft, steadyComPending }),
    community: () => speciesCard(store),
    medium: () => mediumCard(store),
    model: () => modelEditorCard(store) + modelOverviewCard(store),
    omics: () => omicsOverlayCard(store) + communityOmicsCard(store),
    analysis: () => scenarioCard(store, { steadyComDraft, steadyComPending }),
    settings: () => scenarioCard(store, { steadyComDraft, steadyComPending }),
    crossfeeding: () => crossFeedingCard(store),
    overview: () => modelOverviewCard(store),
    reactions: () => renderReactionsPage(store, reactions ?? {}),
  }[workspace];
  return `<div class="workspace-cards" data-workspace-content="${esc(workspace)}">${content ? content() : '<p class="empty">Choose a workspace from the task panel.</p>'}</div>`;
}

/**
 * The display options used most, for the map toolbar. Two groups, so that a
 * narrow toolbar can hand the second (`quick-b`) and then both over to the View
 * popover, which carries the same controls for that (see renderViewControls).
 */
export function renderMapQuickControls(store) {
  const v = store.view;
  const toggle = (action, on, label, title) =>
    `<button class="quick-toggle" data-action="${action}" aria-pressed="${on}" title="${esc(title)}" type="button">${label}</button>`;
  const option = (value, current, label) => `<option value="${value}"${current === value ? ' selected' : ''}>${label}</option>`;
  const color = v.reactionColorMode ?? 'flux';
  const multicell = (store.community?.activeSpecies?.length ?? 0) > 1;
  return `<span class="quick quick-a">
      <select data-action="labels" aria-label="Map labels" title="Which labels are shown">
        ${option('auto', v.labels, 'Labels: major fluxes')}
        ${option('active', v.labels, 'Labels: all active')}
        ${option('all', v.labels, 'Labels: all')}
        ${option('none', v.labels, 'Labels: none')}
      </select>
      ${toggle('toggle-currency', v.showCurrency, 'Cofactors', 'Show cofactors such as ATP, NADH and H⁺')}
      ${multicell ? toggle('toggle-crossfeed', v.showCrossFeeding, '⇄ Cross-feeding', 'Highlight transfers between cells') : ''}
    </span>
    <span class="quick quick-b">
      <select data-action="reaction-color-mode" aria-label="Line color" title="What colors the reaction lines">
        ${option('flux', color, 'Color: flux')}
        ${option('pathway', color, 'Color: pathway')}
        ${option('subsystem', color, 'Color: subsystem')}
      </select>
      ${toggle('toggle-width-by-flux', Boolean(v.widthByFlux), 'Width by flux', 'Line width follows flux')}
    </span>`;
}

/** View options affect presentation only; the theme control lives in the app. */
export function renderViewControls(store) {
  const v = store.view;
  const toggle = (action, on, label, title) =>
    `<button data-action="${action}" aria-pressed="${on}" title="${esc(title)}" type="button">${label}</button>`;
  const overview = store.coreMetabolismOverviewSupport?.() ?? { supported:false };
  return `<div class="view-controls">
    <div class="view-folded">${renderMapQuickControls(store)}</div>
    <span class="section-label">Map view</span>
    <div class="toggle-list map-mode-toggle">
      <button data-action="map-core-overview" aria-pressed="${store.mapMode === 'core-overview'}"
        ${overview.supported ? '' : 'disabled'}
        title="Glycolysis, PPP, TCA and amino-acid synthesis">Core metabolism</button>
      <button data-action="map-full" aria-pressed="${store.mapMode === 'full'}"
        title="Every reaction in the map's model">Full network</button>
    </div>
    <span class="section-label">Cofactors</span>
    <div class="toggle-list">
      ${toggle('toggle-currency-labels', v.currencyLabels, 'Cofactor labels', 'Shown past 34% zoom')}
      ${toggle('toggle-quiet-cofactors', store.map?.cofactorStyle === 'quiet', 'Quiet cofactors', 'Small and light; saved with the map')}
    </div>
    ${crossFeedingTopControl(store)}
    ${crossFeedingWidthControl(store)}
    ${mapLabelControl(store, 'metabolite')}
    ${mapLabelControl(store, 'reaction')}
    <label class="view-label-control">Scrolling
      <select data-action="scroll-input" aria-label="Scrolling input">
        <option value="auto"${(v.scrollInput ?? 'auto') === 'auto' ? ' selected' : ''}>Automatic</option>
        <option value="mouse"${v.scrollInput === 'mouse' ? ' selected' : ''}>Mouse wheel: zoom</option>
        <option value="touchpad"${v.scrollInput === 'touchpad' ? ' selected' : ''}>Touchpad: pan</option>
      </select>
    </label>
    <button data-action="open-workspace" data-workspace="map" type="button">Colors & pathways <span aria-hidden="true">↗</span></button>
    ${store.hiddenMetaboliteCount > 0 ? `<button data-action="show-hidden-metabolites" type="button">Show hidden metabolites (${store.hiddenMetaboliteCount})</button>` : ''}
  </div>`;
}

/**
 * The header badge, read from what is loaded rather than written once in the
 * page: a single-species upload is not a multicell community.
 */
export function modelKindLabel(store) {
  const species = store.community?.activeSpecies ?? [];
  if (!species.length) return { text: '', title: '' };
  // Read from the community, which works these out once when it is built;
  // this label is repainted on every change, solves included.
  const perSpecies = store.community.speciesCompartments;
  const compartments = Math.max(...species.map((sp) => perSpecies?.get(sp.id)?.length ?? 1));
  const text = `${compartments > 1 ? 'Multi-compartment' : 'Single compartment'} · `
    + `${species.length > 1 ? 'Multi-cell' : 'Single cell'}`;
  const title = `${species.length} active species; up to ${compartments} compartment${compartments === 1 ? '' : 's'} per cell`;
  return { text, title };
}

/**
 * The right-hand inspector as HTML, for the selected reaction or the selected
 * entity (metabolite, gene, text, cell, compartment), or else for the
 * multi-selection (showsSelectionInspector).
 */
export function renderInspector(store) {
  if (store.selectedReaction) return inspectorCard(store);
  if (showsSelectionInspector(store)) return selectionInspector(store);

  const selected = store.selectedEntity;
  if (!selected) {
    return `<section class="inspector-empty">
      <div class="inspector-empty-icon">⌁</div>
      <h2>No selection</h2>
      <p>Select something on the map to inspect it.</p>
    </section>`;
  }

  if (selected.type === 'metabolite') return metaboliteInspector(store, selected.id);
  if (selected.type === 'gene') return geneInspector(store, selected.speciesId, selected.id);
  if (selected.type === 'text') return textInspector(store, selected.id);
  if (selected.type === 'cell') return cellInspector(store, selected.id);
  if (selected.type === 'compartment') {
    return compartmentInspector(store, selected.cellId, selected.id);
  }
  return `<section class="inspector-empty"><h2>Selection</h2><p>No properties are available.</p></section>`;
}

/**
 * The results dock as HTML: the analysis summary with its subtab (`flux`,
 * `fva`, `reduced`, `scaled`, `shadow`, `repro`), the Table view and the log.
 * The Table view lists every reaction (see renderCompactReactionsTable) when
 * `table` gives its view state, `{model, scrollTop, viewportHeight, mode}`;
 * `mode: 'shadow'` lists the metabolites' shadow prices instead. Without it,
 * which is while the view is hidden, it is left empty.
 */
export function renderResults(store, analysisTab = 'flux', { table = null } = {}) {
  const solution = store.solution;
  const optimal = solution?.status === 'optimal';
  const mode = solution?.steadyCom ? 'SteadyCom fixed µ' : (solution?.gimme ? 'GIMME' : (store.loopless ? 'Loopless FBA' : (store.cycleFree ? 'CycleFreeFlux' : (store.parsimonious ? 'pFBA' : (store.fvaEnabled ? 'FVA' : 'FBA')))));
  const statusLabel = optimal ? 'Optimal' : (solution?.status ?? 'Not run');
  const progress = store.fvaProgress ?? {};
  const replayAvailable = Boolean(store.reproducibilityReplay?.replay);
  const tabs = ['flux','fva','reduced','scaled','shadow','repro'];
  const active = tabs.includes(analysisTab) ? analysisTab : 'flux';
  const dualAvailable = optimal && solution?.dualAnalysis?.available === true && !store.loopless;
  const fvaAvailable = !solution?.steadyCom && (store.fvaEnabled || (store.fvaRanges?.size ?? 0) > 0 || progress.running);
  const tabButton = (id, label, enabled = true, reason = 'Not returned by the current solver') =>
    `<button data-action="analysis-subtab" data-analysis-tab="${id}" class="${active === id ? 'is-active' : ''}" `
    + `aria-pressed="${active === id}"${enabled ? '' : ` disabled title="${esc(reason)}"`}>${label}</button>`;

  return `<div class="result-pane" data-result-pane="analysis">
    <div class="analysis-summary">
      <div class="analysis-message ${optimal ? 'is-success' : 'is-error'}">
        <span>${optimal ? '✓' : '!'}</span><div><strong>${mode} · ${esc(statusLabel)}</strong>
        ${optimal ? '' : `<small>${esc(solution?.message ?? store.status?.message ?? 'No solution available.')}</small>`}</div>
      </div>
      <dl class="analysis-metrics">
        <div><dt>Objective value</dt><dd>${optimal ? formatFlux(solution.objectiveValue, 5) : '—'}</dd></div>
        <div><dt>Solver</dt><dd>${esc(solverLabel(store))}</dd></div>
        <div><dt>Time</dt><dd>${store.status?.solveMs ?? 0} ms</dd></div>
        <div><dt>Status</dt><dd><span class="status-pill ${optimal ? 'is-success' : ''}">${esc(statusLabel)}</span></dd></div>
      </dl>
    </div>
    <div class="analysis-table-card">
      <div class="subtabs" role="group" aria-label="Analysis result">
        ${tabButton('flux','Flux')}
        ${tabButton('fva', 'FVA', fvaAvailable, 'Run FVA to fill this tab')}
        ${tabButton('reduced', 'Reduced costs', dualAvailable)}
        ${tabButton('scaled', 'Scaled reduced costs', dualAvailable)}
        ${tabButton('shadow', 'Shadow prices', dualAvailable)}
        ${tabButton('repro', 'Reproducibility', replayAvailable, 'Import or run a reproducibility replay first')}
      </div>
      ${analysisTabContent(store, active, { optimal, dualAvailable, fvaAvailable, progress })}
    </div>
  </div>
  <div class="result-pane" data-result-pane="table">
    ${tableViewPane(store, table, { optimal, dualAvailable, fvaAvailable, progress })}
  </div>
  <div class="result-pane log-pane" data-result-pane="logs">
    <p><span>Now</span> ${esc(mode)} · ${esc(statusLabel)} · ${store.status?.solveMs ?? 0} ms</p>
    ${store.fvaEnabled ? `<p><span>FVA</span> ${progress.running ? `Computing ${progress.done ?? 0}/${progress.total ?? 0}` : `Available for ${store.fvaRanges.size} reactions`}</p>` : ''}
    ${solution?.dualAnalysis?.available ? `<p><span>Duals</span> ${esc(solution.dualAnalysis.context ?? solution.dualAnalysis.interpretation ?? 'optimal LP basis')}</p>` : ''}
    ${renderLogEntries(store.log)}
  </div>`;
}

/** The session's messages, newest first, each with the time it last happened. */
export function renderLogEntries(log = []) {
  if (!log.length) return '<p class="muted">No messages yet in this session.</p>';
  const rows = [...log].reverse().map((entry) => {
    const time = new Date(entry.time).toLocaleTimeString();
    const count = entry.count > 1 ? ` <small class="log-count">×${entry.count}</small>` : '';
    return `<li class="log-entry is-${esc(entry.level)}"><span>${esc(time)}</span> <b class="log-level">${esc(entry.level)}</b> ${esc(entry.message)}${count}</li>`;
  }).join('');
  return `<div class="log-history">
    <div class="log-history-head"><span>History · ${log.length} message${log.length === 1 ? '' : 's'}</span>
      <button class="secondary" data-action="clear-log" type="button">Clear</button></div>
    <ol class="log-list">${rows}</ol>
  </div>`;
}

function tableViewPane(store, table, state) {
  if (!table) return '<div class="analysis-empty">Open the Table view to list every reaction.</div>';
  const shadow = table.mode === 'shadow';
  const tab = (mode, label) => `<button type="button" data-action="rx-compact-mode" data-mode="${mode}" class="${(table.mode ?? 'reactions') === mode ? 'is-active' : ''}"
    aria-pressed="${(table.mode ?? 'reactions') === mode}">${label}</button>`;
  return `<div class="rx-compact">
    <div class="rx-compact-bar">
      <div class="subtabs" role="group" aria-label="Table">${tab('reactions', 'Reactions')}${tab('shadow', 'Shadow prices')}</div>
      <span class="grow"></span>
      <button type="button" class="rx-open-page" data-action="open-workspace" data-workspace="reactions">Open in Reactions ↗</button>
    </div>
    ${shadow
      ? analysisTabContent(store, 'shadow', { ...state, full: true })
      : renderCompactReactionsTable(store, table)}
  </div>`;
}

function solverLabel(store) {
  const reported = store.solution?.backend ?? store.solution?.solver;
  if (reported) return reported === 'glpk' ? 'GLPK (WebAssembly)' : reported === 'simplex' ? 'Local simplex.js' : String(reported);
  if (store.loopless) return store.problem?.backend?.() === 'glpk' ? 'GLPK MILP (WebAssembly)' : 'Local MILP';
  const backend = store.problem?.backend?.();
  return backend === 'glpk' ? 'GLPK (WebAssembly)' : backend === 'simplex' ? 'Local simplex.js' : '—';
}

function analysisTabContent(store, tab, state) {
  const { optimal, dualAvailable, fvaAvailable, progress, full=false } = state;
  const wrap = (html) => `<div class="analysis-tab-content ${full?'full-table':''}" data-analysis-content="${tab}">${html}</div>`;
  if (tab === 'repro') return wrap(reproducibilityReport(store));
  if (!optimal) return wrap('<div class="analysis-empty">Run an analysis to see results.</div>');

  if (tab === 'flux') {
    const rows=[...store.solution.fluxes].sort((a,b)=>Math.abs(b[1])-Math.abs(a[1])||a[0].localeCompare(b[0]));
    return wrap(resultReactionTable(store, rows, 'Flux', (rid,v)=>formatFlux(v,6), full?Infinity:20));
  }
  if (tab === 'fva') {
    if (!fvaAvailable) return wrap('<div class="analysis-empty">FVA is not enabled for this solution.</div>');
    const rows=[...(store.fvaRanges??new Map())].sort((a,b)=>(b[1].max-b[1].min)-(a[1].max-a[1].min));
    const body=rows.slice(0,full?rows.length:20).map(([rid,r])=>reactionRow(store,rid,`[${formatFlux(r.min,5)}, ${formatFlux(r.max,5)}]`,formatFlux(r.max-r.min,5))).join('');
    const note=progress.running?`<p class="analysis-progress">Computing FVA: ${progress.done??0}/${progress.total??0}</p>`:'';
    return wrap(note+`<div class="table-scroll"><table class="results-table"><thead><tr><th>Reaction</th><th>Name</th><th>FVA range</th><th>Width</th></tr></thead><tbody>${body||'<tr><td colspan="4">No FVA ranges available yet.</td></tr>'}</tbody></table></div>`);
  }
  if (tab === 'shadow') {
    if (!dualAvailable) return wrap(dualUnavailable(store));
    const rows=[...(store.solution.shadowPrices??new Map())].sort((a,b)=>Math.abs(b[1])-Math.abs(a[1]));
    const body=rows.slice(0,full?rows.length:20).map(([id,v])=>`<tr><td><code>${esc(id)}</code></td><td class="number">${formatFlux(v,7)}</td></tr>`).join('');
    return wrap(dualContext(store)+`<div class="table-scroll"><table class="results-table"><thead><tr><th>Metabolite</th><th>Shadow price</th></tr></thead><tbody>${body||'<tr><td colspan="2">No shadow prices available.</td></tr>'}</tbody></table></div>`);
  }
  const scaled=tab==='scaled';
  if (!dualAvailable) return wrap(dualUnavailable(store));
  const source=scaled?(store.solution.scaledReducedCosts??new Map()):(store.solution.reducedCosts??new Map());
  const rows=[...source].sort((a,b)=>Math.abs(b[1])-Math.abs(a[1])||a[0].localeCompare(b[0]));
  return wrap(dualContext(store)+resultReactionTable(store,rows,scaled?'Scaled RC':'Reduced cost',(rid,v)=>formatFlux(v,scaled?6:7),full?Infinity:20));
}

function reproducibilityReport(store) {
 const state=store.reproducibilityReplay;
 if(!state?.replay)return '<div class="analysis-empty">Import a reproducibility bundle to replay it.</div>';
 const r=state.replay, integrity=state.integrity??{};
 const icon=(ok)=>ok?'✓':'✕', cls=(ok)=>ok?'is-pass':'is-fail';
 const metric=(label,key,detail='')=>`<div class="repro-check ${cls(Boolean(r.checks?.[key]))}"><span class="repro-icon">${icon(Boolean(r.checks?.[key]))}</span><div><strong>${label}</strong><small>${detail}</small></div></div>`;
 const err=(x)=>x?(`max |Δ| ${Number(x.maxAbsError??0).toExponential(2)} · max rel ${Number(x.maxRelativeError??0).toExponential(2)}${x.worstKey?` · worst: ${esc(x.worstKey)}`:''}${x.missing?` · missing: ${x.missing}`:''}`):'';
 const models=Object.values(integrity.modelChecks??{}); const modelsOK=models.length?models.every(Boolean):true;
 const mapOK=Boolean(state.restored?.map); const omicsExpected=Boolean(state.restored?.omicsExpected); const omicsOK=!omicsExpected||Boolean(state.restored?.omics);
 return `<div class="repro-report">
   <div class="repro-verdict ${cls(r.pass&&integrity.valid)}"><span>${icon(r.pass&&integrity.valid)}</span><div><strong>${r.pass&&integrity.valid?'Reproducibility verified':'Reproducibility warning'}</strong></div></div>
   <div class="repro-grid">
    ${metric('Bundle integrity','integrity',integrity.payloadValid?'SHA-256 payload verified':'SHA-256 payload mismatch')}
    ${metric('Models','models',`${models.length} embedded model hash(es) verified`)}
    ${metric('Map restored','map',mapOK?'Exact archived EditableMap restored':'No archived map restored')}
    ${metric('Omics restored','omics',omicsExpected?(omicsOK?'Archived omics dataset restored':'Expected omics dataset missing'):'No omics dataset archived')}
    ${metric('Objective','objective',r.objective?`|Δ| ${Number(r.objective.absError??0).toExponential(2)}`:'')}
    ${metric('Fluxes','fluxes',err(r.fluxes))}
    ${metric('Reduced costs','reducedCosts',err(r.reducedCosts))}
    ${metric('Scaled RC','scaledReducedCosts',err(r.scaledReducedCosts))}
    ${metric('Shadow prices','shadowPrices',err(r.shadowPrices))}
   </div>
   <p class="repro-tolerance">Tolerance: absolute ≤ ${Number(r.tolerances?.absolute??0).toExponential(1)} or relative ≤ ${Number(r.tolerances?.relative??0).toExponential(1)}.</p>
 </div>`.replace(
   metric('Bundle integrity','integrity',integrity.payloadValid?'SHA-256 payload verified':'SHA-256 payload mismatch'),
   `<div class="repro-check ${cls(Boolean(integrity.payloadValid))}"><span class="repro-icon">${icon(Boolean(integrity.payloadValid))}</span><div><strong>Bundle integrity</strong><small>${integrity.payloadValid?'SHA-256 payload verified':'SHA-256 payload mismatch'}</small></div></div>`
 ).replace(
   metric('Models','models',`${models.length} embedded model hash(es) verified`),
   `<div class="repro-check ${cls(modelsOK)}"><span class="repro-icon">${icon(modelsOK)}</span><div><strong>Models</strong><small>${models.length} embedded model hash(es) verified</small></div></div>`
 ).replace(
   metric('Map restored','map',mapOK?'Exact archived EditableMap restored':'No archived map restored'),
   `<div class="repro-check ${cls(mapOK)}"><span class="repro-icon">${icon(mapOK)}</span><div><strong>Map restored</strong><small>${mapOK?'Exact archived EditableMap restored':'No archived map restored'}</small></div></div>`
 ).replace(
   metric('Omics restored','omics',omicsExpected?(omicsOK?'Archived omics dataset restored':'Expected omics dataset missing'):'No omics dataset archived'),
   `<div class="repro-check ${cls(omicsOK)}"><span class="repro-icon">${icon(omicsOK)}</span><div><strong>Omics restored</strong><small>${omicsExpected?(omicsOK?'Archived omics dataset restored':'Expected omics dataset missing'):'No omics dataset archived'}</small></div></div>`
 );
}

function resultReactionTable(store, rows, valueLabel, formatter, limit=20) {
  const selected=Number.isFinite(limit)?rows.slice(0,limit):rows;
  const body=selected.map(([rid,v])=>reactionRow(store,rid,formatter(rid,v))).join('');
  return `<div class="table-scroll"><table class="results-table"><thead><tr><th>Reaction</th><th>Name</th><th>${esc(valueLabel)}</th></tr></thead><tbody>${body||'<tr><td colspan="3">No results available.</td></tr>'}</tbody></table></div>`;
}

function reactionRow(store,rid,value,extra=null) {
 const owner=store.community?.reactionOwner.get(rid)??{};
 const reaction=store.community?.merged.reactions.get(rid);
 return `<tr><td><button class="table-link" data-action="select-result-reaction" data-result-rxn="${esc(rid)}">${esc(owner.originalId??rid)}</button></td><td>${esc(reaction?.name??'')}</td><td class="number">${value}</td>${extra===null?'':`<td class="number">${extra}</td>`}</tr>`;
}
function dualContext(store){
 const d=store.solution?.dualAnalysis;
 return `<p class="analysis-context">${esc(d?.context??d?.interpretation??'Optimal LP basis')}${d?.interpretation&&d?.context?` · ${esc(d.interpretation)}`:''}</p>`;
}
function dualUnavailable(store){
 return `<div class="analysis-empty">${store.loopless?'Dual analysis is not reported for the Loopless MILP solution.':'Dual values are not available for this solution.'}</div>`;
}

function modelOverviewCard(store) {
  const counts = store.map?.stats?.() ?? {};
  const merged = store.community?.merged;
  const geneCount = Array.isArray(merged?.genes) ? merged.genes.length : Object.keys(merged?.genes ?? {}).length;
  return `<section class="card model-overview-card"><h2>Model overview</h2>
    <dl class="overview-list">
      <div><dt>Cells</dt><dd>${store.map?.cells?.length ?? 0}</dd></div>
      <div><dt>Compartments</dt><dd>${counts.compartments ?? 0}</dd></div>
      <div><dt>Reactions</dt><dd>${merged?.reactions?.size ?? 0}</dd></div>
      <div><dt>Metabolites</dt><dd>${merged?.metabolites?.size ?? 0}</dd></div>
      <div><dt>Genes (GPR)</dt><dd>${geneCount}</dd></div>
    </dl>
  </section>`;
}


/** The automatic layout panel as HTML: presets, spacing sliders and the canvas size. */
export function renderLayoutSettings(store) {
  const settings = store.layoutSettings ?? {
    preset: 'spacious',
    cellSpacing: 240,
    minimumCompartmentSize: 60,
    nodeSpacing: 1.42,
    cellAreaMultiplier: 6,
  };
  // The spacing sliders re-run the same automatic layout as the presets, so they
  // are refused for the same reason, and say so where the pointer is.
  const blocked = store.relayoutBlocked?.() ?? null;
  const lock = blocked ? ` disabled title="${esc(blocked)}"` : '';
  const preset = (name, label) =>
    `<button data-action="layout-preset" data-layout-preset="${name}" type="button"${lock}
      aria-pressed="${settings.preset === name}" class="${settings.preset === name ? 'is-active' : ''}">${label}</button>`;
  const width = Math.round(store.map?.width ?? 1200);
  const height = Math.round(store.map?.height ?? 800);

  return `<div class="layout-settings-panel">
    <section class="layout-settings-hero">
      <div>
        <span class="section-label">Automatic layout</span>
        <h2>Layout settings</h2>
      </div>
    </section>

    <section class="layout-settings-section">
      <h3>Preset</h3>
      <div class="layout-preset-group" role="group" aria-label="Automatic layout preset">
        ${preset('compact', 'Compact')}
        ${preset('normal', 'Normal')}
        ${preset('spacious', 'Spacious')}
      </div>
      ${settings.preset === 'custom' ? '<p class="layout-custom-note">Custom spacing</p>' : ''}
    </section>

    <section class="layout-settings-section">
      <div class="layout-control">
        <div class="layout-control-head"><label for="layout-cell-area">Cell area</label>
          <output data-layout-output="cellAreaMultiplier">${Number(settings.cellAreaMultiplier ?? 6).toFixed(1)}×</output></div>
        <input id="layout-cell-area" data-action="layout-cell-area" type="range"${lock}
          min="1" max="8" step="0.5" value="${settings.cellAreaMultiplier ?? 6}">
      </div>

      <div class="layout-control">
        <div class="layout-control-head"><label for="layout-cell-spacing">Cell spacing</label>
          <output data-layout-output="cellSpacing">${Math.round(settings.cellSpacing)} px</output></div>
        <input id="layout-cell-spacing" data-action="layout-cell-spacing" type="range"${lock}
          min="80" max="420" step="10" value="${settings.cellSpacing}">
      </div>

      <div class="layout-control">
        <div class="layout-control-head"><label for="layout-min-compartment">Minimum compartment size</label>
          <output data-layout-output="minimumCompartmentSize">${Math.round(settings.minimumCompartmentSize)} px</output></div>
        <input id="layout-min-compartment" data-action="layout-min-compartment" type="range"${lock}
          min="28" max="110" step="2" value="${settings.minimumCompartmentSize}">
      </div>

      <div class="layout-control">
        <div class="layout-control-head"><label for="layout-node-spacing">Node spacing</label>
          <output data-layout-output="nodeSpacing">${Number(settings.nodeSpacing).toFixed(2)}×</output></div>
        <input id="layout-node-spacing" data-action="layout-node-spacing" type="range"${lock}
          min="0.65" max="1.90" step="0.05" value="${settings.nodeSpacing}">
      </div>
      ${blocked ? `<p class="hint layout-blocked-note">${esc(blocked)}</p>` : ''}
      <p class="hint">Undo restores the previous layout.</p>
    </section>

    <section class="layout-settings-section">
      <h3>Canvas size</h3>
      <div class="layout-size-grid">
        <label>Width
          <div class="layout-number"><input data-action="layout-canvas-width" type="number" min="320" step="50" value="${width}"><span>px</span></div>
        </label>
        <label>Height
          <div class="layout-number"><input data-action="layout-canvas-height" type="number" min="240" step="50" value="${height}"><span>px</span></div>
        </label>
      </div>
      <button class="layout-fit-button" data-action="zoom-fit" type="button">⤢ Fit to screen</button>
    </section>
  </div>`;
}

// Scenario

/**
 * Scenario on the left, objective on the right, in one card.
 *
 * One list holds every community the app can load (the published benchmarks
 * first, then the teaching scenarios), because from the reader's side they are
 * the same choice: which community am I looking at. What changes is how much
 * there is to say about it, and that is what the detail below the list carries.
 */
function scenarioCard(store, steadyComUI = {}) {
  const published = store.publishedCommunities ?? [];
  const selectedPublished = store.scenario?.published
    ? published.find((c) => c.id === store.scenario.published)
    : null;

  const publishedOptions = published.map((c) => {
    const mark = c.reproduction.status === 'reproduced' ? '✓' : '—';
    const value = `pub:${c.id}`;
    const on = selectedPublished?.id === c.id;
    return `<option value="${esc(value)}"${on ? ' selected' : ''}>${mark} ${esc(c.name)}</option>`;
  }).join('');

  const teachingOptions = store.scenarios.map((sc) => {
    const on = !selectedPublished && sc.id === store.scenario?.id;
    return `<option value="${esc(`sc:${sc.id}`)}"${on ? ' selected' : ''}>${esc(sc.name)}</option>`;
  }).join('');

  const detail = selectedPublished
    ? publishedCommunityDetail(selectedPublished, store, steadyComUI)
    : `<p class="hint">${esc(store.scenario?.description ?? '')}</p>${steadyComBlock(null, store, steadyComUI)}`;

  return `<section class="card sc-card">
    <div class="sc-grid">
      <div class="sc-col sc-scenario">
        <h2>Scenario</h2>
        <select data-action="scenario" aria-label="Community">
          ${publishedOptions ? `<optgroup label="Published communities">${publishedOptions}</optgroup>` : ''}
          <optgroup label="Teaching scenarios">${teachingOptions}</optgroup>
        </select>
        ${detail}
      </div>
      <div class="sc-col sc-objective">
        <h2>Objective</h2>
        ${objectiveControls(store)}
        ${solverNote(store)}
      </div>
    </div>
  </section>`;
}

/**
 * The solver, read only: a browser cannot use a licence installed on the
 * computer. The desktop app offers a choice here between HiGHS and the
 * Gurobi or CPLEX the computer has.
 */
function solverNote(store) {
  return `<h2 class="sc-solver-title">Solver</h2>
    <p class="hint"><strong>${esc(solverLabel(store))}</strong> · Gurobi/CPLEX: desktop app only.</p>`;
}

function objectiveControls(store) {
  if (store.isZunigaStudy) {
    const coefficients = Object.entries(store.problem?.objective ?? {}).map(([id, value]) => `${value} × ${id}`).join(' + ');
    return `<p><strong>Authors’ original objective</strong></p><p class="hint"><code>${esc(coefficients || 'Source model objective')}</code></p>
      <p class="hint">Medium, bounds and population coefficients are fixed for this case.</p>
      <p class="hint">Stationary FBA · GLPK.</p>`;
  }
  const fixedMu = Boolean(store.solution?.steadyCom);
  const objectives = [
    [OBJECTIVES.EQUAL_GROWTH, 'Equal growth (shared µ)'],
    [OBJECTIVES.SUM_BIOMASS, 'Weighted biomass sum'],
  ].map(([v, label]) => `<option value="${v}"${store.objective === v ? ' selected' : ''}>${esc(label)}</option>`).join('');

  const edited = store.customObjective
    ? `<option value="" selected disabled>Edited: ${esc(store.objectiveDescription?.() ?? 'custom')}</option>` : '';
  return `<label class="sc-label" for="objective-select">Community objective</label>
    <select id="objective-select" data-action="objective">${edited}${objectives}</select>
    <button type="button" class="sc-objective-edit" data-action="open-workspace" data-workspace="reactions"
      title="Any reaction, any coefficient, maximised or minimised">Edit objective in Reactions ↗</button>
    <label class="row" style="margin-top:10px;cursor:pointer" title="Parsimonious FBA: least total flux at the target biomass">
      <input type="checkbox" data-action="parsimonious" ${store.parsimonious ? 'checked' : ''}
        aria-label="Use Parsimonious Flux Balance Analysis">
      <span class="grow"><strong>pFBA</strong></span>
    </label>
    <div class="row pfba-biomass-row">
      <label class="grow" for="pfba-biomass-percent">Target biomass (% of optimum)</label>
      <input id="pfba-biomass-percent" type="number" min="0" max="100" step="0.1"
        data-action="pfba-biomass-percent"
        value="${esc(formatPercent(store.pfbaBiomassPercent))}"
        aria-label="Percentage of the optimal objective fixed for pFBA">
    </div>
    <label class="row" style="margin-top:10px;cursor:pointer" title="Excludes internal cycles with a MILP; can be much slower">
      <input type="checkbox" data-action="loopless" ${store.loopless ? 'checked' : ''}
        aria-label="Enable Loopless Flux Balance Analysis">
      <span class="grow"><strong>Loopless</strong></span>
    </label>
    ${looplessProgressLine(store)}
    <label class="row" style="margin-top:10px;cursor:pointer"${fixedMu ? '' : ' title="Flux Variability Analysis: each reaction’s range at the optimum"'}>
      <input type="checkbox" data-action="fva" ${store.fvaEnabled ? 'checked' : ''}
        aria-label="Enable Flux Variability Analysis"${fixedMu ? ' disabled title="Run an ordinary analysis before enabling FVA"' : ''}>
      <span class="grow"><strong>FVA</strong></span>
    </label>
    ${fvaProgressLine(store)}`;
}

function formatPercent(value) {
  const n = Number(value);
  if (!Number.isFinite(n)) return '100';
  return Number.isInteger(n) ? String(n) : String(Number(n.toFixed(2)));
}

function looplessProgressLine(store) {
  if (!store.loopless) return '';

  const solution = store.solution;
  if (!solution) return '<p class="hint">Loopless enabled.</p>';

  if (solution.status === 'optimal') {
    const nodes = Number.isFinite(solution.nodes)
      ? ` · <strong>${solution.nodes}</strong> branch-and-bound nodes` : '';
    return `<p class="hint">Loopless MILP optimal · <strong>${solution.binaryVariables ?? 0}</strong> binaries${nodes}.</p>`;
  }

  return `<p class="hint">Loopless: ${esc(solution.message || solution.status)}.</p>`;
}

function fvaProgressLine(store) {
  if (!store.fvaEnabled) return '';
  const progress = store.fvaProgress ?? {};
  if (progress.running) {
    return `<p class="hint">FVA running… <strong>${progress.done ?? 0}/${progress.total ?? 0}</strong> reactions.</p>`;
  }
  if ((progress.total ?? 0) > 0) {
    return `<p class="hint">FVA done: <strong>${progress.done}/${progress.total}</strong> reactions · ${progress.ms ?? 0} ms.</p>`;
  }
  return '<p class="hint">FVA runs on the next solve.</p>';
}

// Published communities

/**
 * What a published community shows once it is selected.
 *
 * Two different things are on this card and they are kept apart on purpose: what
 * the paper reported, and what this app computed. A community whose sources could
 * not be retrieved says so rather than showing a plausible number, and anything
 * downloadable is a real file generated from the authors' own distribution by
 * tools/build-published-assets.mjs, never a button that leads nowhere.
 */
function publishedCommunityDetail(c, store, steadyComUI = {}) {
  const { paper, reproduction: rep } = c;
  const reproduced = rep.status === 'reproduced';
  const assets = store.publishedManifests?.get(c.id) ?? null;
  const base = `${store.base ?? '.'}/models/published/${c.id}`;

  const doi = paper.doi
    ? `<a href="https://doi.org/${esc(paper.doi)}" target="_blank" rel="noopener">doi:${esc(paper.doi)}</a>`
    : '';
  // A citation nobody could check offline is marked as such rather than left to
  // look as solid as the ones verified against a retrieved file.
  const doiNote = paper.doiVerifiedFrom
    ? `<span class="pc-verified" title="Verified against ${esc(paper.doiVerifiedFrom)}">verified</span>`
    : '<span class="pc-unverified" title="Could not verify against a retrieved source">unverified</span>';

  return `<div class="pc-detail">
    <div class="pc-head">
      <span class="chip">${esc(paper.framework)}</span>
      <span class="chip ${reproduced ? 'pc-ok' : 'pc-blocked'}">${reproduced ? 'Reproduced' : 'Blocked'}</span>
    </div>
    <p class="pc-paper"><b>${esc(paper.title)}</b><br>
      ${esc(paper.authors)} (${paper.year}), ${esc(paper.citation ?? paper.journal)}<br>
      ${doi} ${doiNote}</p>
    <p class="hint">${esc(c.description)}</p>
    ${c.id === ZUNIGA_ID ? zunigaCaseBlock(store) : ''}
    ${speciesBlock(c, assets, base, store)}
    ${conditionsBlock(c, assets, base)}
    ${resultsBlock(c)}
    ${steadyComBlock(c, store, steadyComUI)}
    ${communityProvenanceBlock(c)}
  </div>`;
}

/**
 * One row per member, with the model itself behind a download.
 *
 * The link hands the reader the model itself, something they can act on,
 * rather than naming the workbook the member came from.
 */
function speciesBlock(c, assets, base, store) {
  if (!c.species?.length) {
    return '<p class="empty">The species list could not be retrieved from the paper.</p>';
  }
  const byId = new Map((assets?.members ?? []).map((m) => [m.id, m]));

  // The row is the organism's full name, once, not an abbreviation
  // ("B. thetaiotaomicron") with the full name repeated under it.
  //
  // The role line survives only where the organism names repeat inside a
  // community: the four auxotrophs are all "Escherichia coli K-12 MG1655", and
  // what tells them apart is precisely the role ("cannot export Phe, so it
  // supplies Arg"). Dropping it there would leave four identical rows.
  const organisms = c.species.map((sp) => sp.organism || sp.name);
  const organismsDistinct = new Set(organisms).size === organisms.length;

  const rows = c.species.map((sp) => {
    const asset = byId.get(sp.id);
    const abundance = sp.abundance === null || sp.abundance === undefined
      ? '' : ` · ${(sp.abundance * 100).toFixed(0)} %`;
    const title = asset
      ? `${asset.reactions} reactions, ${asset.metabolites} metabolites`
      : '';
    // Served builds can use a plain link. The standalone creates a download
    // from its embedded model instead of requiring a second file beside it.
    const embeddedModel = asset && globalThis.__ESCHER_MCMC_DATA__?.[
      `models/published/${c.id}/${asset.model}`
    ];
    const download = c.id === ZUNIGA_ID
      ? `<button class="pc-dl" type="button" data-action="download-member-json" data-community="${esc(c.id)}" data-member="${esc(sp.id)}"${store?.isZunigaStudy ? '' : ' disabled'} title="Personal export of the privately loaded complete model">JSON</button>
         <button class="pc-dl" type="button" data-action="download-member-sbml" data-community="${esc(c.id)}" data-member="${esc(sp.id)}"${store?.isZunigaStudy ? '' : ' disabled'}>SBML</button>`
      : asset
      ? (embeddedModel
        ? `<button class="pc-dl" type="button" data-action="download-member-json"
             data-community="${esc(c.id)}" data-member="${esc(sp.id)}"
             title="${esc(`${title}, COBRA JSON`)}">JSON</button>`
        : `<a class="pc-dl" download href="${esc(`${base}/${asset.model}`)}"
             title="${esc(`${title}, COBRA JSON`)}">JSON</a>`)
        + `<button class="pc-dl" type="button" data-action="download-member-sbml"
           data-community="${esc(c.id)}" data-member="${esc(sp.id)}"
           title="${esc(`${title}, SBML L3V1 fbc-v2`)}">SBML</button>`
      : '<span class="pc-dl is-off" title="No model file is bundled for this member">—</span>';
    // The full organism name, once (see above).
    const label = organismsDistinct
      ? (sp.organism || sp.name)
      : `${sp.organism || sp.name} · ${sp.name}`;
    const role = organismsDistinct || !sp.role
      ? '' : `<div class="pc-role">${esc(sp.role)}</div>`;
    return `<div class="pc-species">
      <div class="pc-species-head"><b>${esc(label)}</b>${abundance}${download}</div>
      ${role}
    </div>`;
  }).join('');

  return `<h3>Species</h3><div class="pc-species-list">${rows}</div>`;
}

function zunigaCaseBlock(store) {
  if (!store.isZunigaStudy) return `<p class="hint">Loading needs an internet connection the first time.</p>`;
  const co = store.zunigaSolutions.get('mutant-coculture');
  const monoGrowth = createGrowthForCase(store, 'mutant-monoculture', 'Ec');
  const coGrowth = co?.zuniga?.growth?.Ec ?? createGrowthForCase(store, 'mutant-coculture', 'Ec');
  const pheTransfer = co?.zuniga?.crossFeeding?.links?.find(link => link.base === 'phe__L' && link.from === 'Se' && link.to === 'Ec')?.flux;
  const rescued = Number.isFinite(coGrowth) && coGrowth > 1e-8 && Number.isFinite(monoGrowth) && Math.abs(monoGrowth) < 1e-8 && pheTransfer > 1e-8;
  const rows = ZUNIGA_SCENARIOS.map(scenario => `<tr><td>${esc(scenario.name)}</td><td>${formatFlux(createGrowthForCase(store, scenario.id, 'Ec'), 6)}</td><td>${scenario.coculture ? formatFlux(createGrowthForCase(store, scenario.id, 'Se'), 6) : 'Absent'}</td></tr>`).join('');
  return `<h3>Comparable case scenarios</h3>
    <label class="sc-label" for="zuniga-scenario">Case scenario</label>
    <select id="zuniga-scenario" data-action="zuniga-scenario" aria-label="Zuñiga case scenario">${ZUNIGA_SCENARIOS.map(s => `<option value="${esc(s.id)}"${s.id === store.zunigaScenarioId ? ' selected' : ''}>${esc(s.name)}</option>`).join('')}</select>
    <label class="sc-label" for="zuniga-comparison">Flux display</label>
    <select id="zuniga-comparison" data-action="zuniga-comparison" aria-label="Zuñiga flux comparison"><option value="absolute"${store.zunigaComparison === 'absolute' ? ' selected' : ''}>Solved fluxes</option><option value="difference"${store.zunigaComparison === 'difference' ? ' selected' : ''}>Δ flux vs WT monoculture</option></select>
    <p class="hint">${store.zunigaComparison === 'difference' ? 'Blue is a decrease and red an increase relative to WT' : 'Color and width show the solved flux'} (reference magnitude ${formatFlux(store.zunigaState()?.fluxReference, 4)}).</p>
    <table class="pc-sweep"><thead><tr><th>Scenario</th><th>E. coli (h⁻¹)</th><th>S. elongatus (h⁻¹)</th></tr></thead><tbody>${rows}</tbody></table>
    <p class="hint"><strong>Perturbation:</strong> Ec PPNDH (pheA) is disabled in both mutant scenarios.</p>
    <p class="hint"><strong>Calculated phenylalanine transfer:</strong> S. elongatus → E. coli ${formatFlux(pheTransfer, 6)} in source-model shared-pool flux units.</p>
    <p class="hint">${rescued ? 'The calculated phenylalanine transfer restores growth in the mutant coculture.' : 'Rescue applies only where the mutant coculture grows and the transfer is positive.'}</p>
    <p class="hint">A stationary FBA adaptation, not a comparison of experimental cultures over time.</p>
    <div class="row"><button data-action="export-svg">Export SVG</button><button data-action="export-png">Export PNG</button><button data-action="export-reproducibility">Private reproducibility bundle</button></div>`;
}

function createGrowthForCase(store, scenarioId, speciesId) {
  const result = store.zunigaSolutions.get(scenarioId);
  const metadata = result?.zuniga?.growth?.[speciesId];
  if (Number.isFinite(metadata)) return metadata;
  const biomass = store.zunigaPrepared?.community?.biomassIds?.get(speciesId);
  return biomass ? result?.fluxes.get(biomass) : undefined;
}

/**
 * The medium and the bounds, as files rather than prose.
 *
 * Maintenance, secretion caps and the total biomass are all bounds, and the
 * bounds file states them exactly, so listing them here in words would be a
 * lossy second copy of a download.
 */
function conditionsBlock(c, assets, base) {
  if (c.id === ZUNIGA_ID) return `<h3>Source conditions</h3><p class="hint">The author models are downloaded from the pinned source, verified by SHA-256, and not redistributed.</p>
    <p class="hint">Monoculture and coculture use different source models.</p>`;
  if (!assets) {
    return c.conditions
      ? `<h3>Conditions</h3><p class="hint">${esc(c.conditions.medium ?? '')}</p>`
      : '';
  }
  const link = (file, label) => typeof globalThis.__ESCHER_MCMC_TEXT__?.[
    `models/published/${c.id}/${file}`
  ] === 'string'
    ? `<button class="pc-dl" type="button" data-action="download-community-csv"
         data-community="${esc(c.id)}" data-file="${esc(file)}">${label}</button>`
    : `<a class="pc-dl" download href="${esc(`${base}/${file}`)}">${label}</a>`;
  return `<h3>Conditions</h3>
    <ul class="pc-list pc-downloads">
      <li><b>Medium</b> · open exchanges
        ${link(assets.media, 'CSV')}</li>
      <li><b>Bounds</b> · every reaction
        ${link(assets.bounds, 'CSV')}</li>
    </ul>`;
}

function resultsBlock(c) {
  const expected = c.reproduction.expected;
  const computed = c.computed;
  if (!expected && !computed) return '';

  const lines = [];
  if (expected?.growthRate !== undefined) {
    lines.push(`<li><b>Paper µ:</b> ${expected.growthRate} h⁻¹</li>`);
  }
  if (computed?.growthRate !== undefined) {
    lines.push(`<li><b>Recorded µ (offline):</b> ${computed.growthRate} h⁻¹`
      + ` <span class="hint">(${computed.lpCount} LP, ${computed.seconds} s)</span></li>`);
  } else if (computed?.growthRateRange) {
    lines.push(`<li><b>Recorded sweep µ:</b> ${computed.growthRateRange.join(' – ')} h⁻¹`
      + ` <span class="hint">(${computed.sweepPoints} sweep points)</span></li>`);
  } else if (computed?.growthRates) {
    // A joint model whose members grow at different rates: one figure each,
    // and no community rate, because there is not one.
    const byMember = Object.entries(computed.growthRates)
      .map(([id, rate]) => `${esc(id)} ${rate}`).join(' · ');
    const sampling = computed.scenarios?.length
      ? `${computed.scenarios.length} source scenarios`
      : `${computed.sweepPoints} sweep points`;
    lines.push(`<li><b>Recorded growth:</b> ${byMember} h⁻¹`
      + ` <span class="hint">(${sampling})</span></li>`);
  }
  if (computed?.methane !== undefined) {
    lines.push(`<li><b>Methane:</b> ${computed.methane} mmol gDW⁻¹ h⁻¹</li>`);
  }
  if (expected?.note) lines.push(`<li class="hint">${esc(expected.note)}</li>`);
  const abundances = computed?.abundances ?? expected?.abundances;
  if (abundances) {
    // A nine-species community leaves most members at essentially zero; listing
    // them all buries the ones that are actually there.
    const present = Object.entries(abundances).filter(([, x]) => Number(x) >= 1e-4);
    const parts = present.sort((a, b) => Number(b[1]) - Number(a[1]))
      .map(([id, x]) => `${esc(id)} ${(Number(x) * 100).toFixed(1)} %`).join(' · ');
    const hidden = Object.keys(abundances).length - present.length;
    lines.push(`<li><b>Relative abundances:</b> ${parts}`
      + `${hidden > 0 ? ` <span class="hint">(${hidden} below 0.01%)</span>` : ''}</li>`);
  }

  // Both reproduced communities record a sweep, and they sweep different things,
  // so what to plot is part of the registry entry rather than built in here.
  const chart = computed?.sweep?.length && c.figure ? sweepFigure(computed.sweep, c.figure) : '';
  return `<h3>Recorded published results</h3>${lines.length ? `<ul class="pc-list">${lines.join('')}</ul>` : ''}${chart}`;
}

/** Current-input analysis is kept separate from recorded published benchmarks. */
function steadyComBlock(c, store, { steadyComDraft = null, steadyComPending = false } = {}) {
  const support = store.steadyComSupport?.(c?.id);
  if (!support) return '';
  if (!support.supported) {
    if (c && c.paper?.framework !== 'SteadyCom') return '';
    return `<h3>Fix a growth rate</h3><p class="hint">${esc(support.reason)}</p>`;
  }
  const communityId = support.communityId ?? c?.id ?? store.scenario?.id;
  const state = store.steadyComState?.communityId === communityId ? store.steadyComState : null;
  const computed = state?.source === 'computed' && state.key != null && Number.isFinite(state.max);
  const top = computed ? state.max : null;
  const minimum = Number.isFinite(state?.min) ? state.min : 0;
  const busy = Boolean(state?.phase || steadyComPending);
  const draft = steadyComDraft?.communityId === communityId && steadyComDraft.key === state?.key ? steadyComDraft : null;
  const mu = draft ? draft.value : String(Number.isFinite(state?.mu) ? state.mu : (top ?? minimum));
  const iteration = state?.progress?.iteration ?? state?.progress?.iterations ?? state?.iterations;
  const progress = busy ? `<p class="sc-steadycom-progress" data-steadycom-progress role="status" aria-live="polite">${state?.phase === 'fixed' ? (state.parsimonious ? 'Solving at fixed µ for the least total flux' : 'Solving at fixed µ') : 'Calculating the growth-rate range'}${Number.isFinite(iteration) ? ` · iteration ${iteration}` : ''}…</p>` : '';
  const composition = state?.feasible && state.abundances?.size && Number.isFinite(state.mu)
    ? `<li><b>Composition at µ = ${esc(String(state.mu))} h⁻¹:</b> ${[...state.abundances]
        .sort((a, b) => b[1] - a[1])
        .map(([id, x]) => x > 1e-9 ? `${esc(id)} ${(100 * x).toFixed(1)} %` : `${esc(id)} inactive (X = 0)`).join(' · ')}
        <span class="hint">(${state.solveMs ?? 0} ms)</span></li>`
    : '';
  const refused = state?.feasible === false && Number.isFinite(state.mu)
    ? `<li class="pc-blocked">No feasible community at µ = ${esc(String(state.mu))} h⁻¹.</li>`
    : '';
  const nine = communityId === 'steadycom-gut-nine';
  const fiber = store.steadyComConfig?.fiber;
  const fiberValue = Number.isFinite(fiber?.bacteroides) ? fiber.bacteroides : 30;
  const condition = nine ? `<div class="sc-steadycom-condition">
      <label>Diet / fiber condition<select data-action="steadycom-fiber-condition" aria-label="SteadyCom fiber condition"${busy ? ' disabled' : ''}>
        <option value="baseline"${!fiber ? ' selected' : ''} title="No additional fiber limits">Baseline diet</option>
        <option value="figure4"${fiber ? ' selected' : ''}>Figure 4 fiber limits</option>
      </select></label>
      ${fiber ? `<label class="sc-steadycom-fiber">B. thetaiotaomicron<input type="number" step="any" min="1" max="30" value="${esc(draft?.fiberValue ?? String(fiberValue))}" data-action="steadycom-fiber" aria-label="Bacteroides maximum specific fiber uptake"${busy ? ' disabled' : ''}${draft?.invalidField === 'fiber' ? ' aria-invalid="true"' : ''}><span>C-mmol/gDW/h</span></label>
        <p class="hint">F. prausnitzii is held at 5 C-mmol/gDW/h; changing a limit invalidates the calculated range.</p>` : ''}
    </div>` : '';

  return `<section class="sc-steadycom" data-steadycom-community="${esc(communityId)}">
    <h3>Fix a growth rate · SteadyCom</h3>
    ${condition}
    <ul class="pc-list">
      <li><b>Calculated µ range:</b> <span data-steadycom-range-value>${top === null ? 'Not calculated for the current inputs' : `${esc(String(+Number(minimum).toFixed(6)))} – ${esc(String(+top.toFixed(6)))} h⁻¹`}</span></li>
      ${computed && Number.isFinite(state.resolution) ? `<li class="hint">Search resolution: ${esc(String(state.resolution))} h⁻¹${Number.isFinite(state.upperBound) ? `; upper search bound: ${esc(String(state.upperBound))} h⁻¹` : ''}.</li>` : ''}
      ${composition}
      ${refused}
    </ul>
    ${progress}
    <div class="sc-mu">
      <input type="range" data-action="steadycom-mu" aria-label="Growth rate"
        min="${minimum}" max="${top ?? 1}" step="any" value="${esc(mu)}" ${top === null || busy ? 'disabled' : ''}>
      <input type="number" data-action="steadycom-mu-number" aria-label="Growth rate, h⁻¹"
        min="${minimum}"${top === null ? '' : ` max="${top}"`} step="any" value="${esc(mu)}" ${top === null || busy ? 'disabled' : ''}${draft?.invalidField === 'mu' ? ' aria-invalid="true"' : ''}>
      <span class="hint">h⁻¹</span>
    </div>
    <p class="sc-steadycom-error" data-steadycom-error role="alert"${draft?.error ? '' : ' hidden'}>${esc(draft?.error ?? '')}</p>
    <label class="sc-steadycom-parsimonious" title="Least summed member flux at this µ; slower"><input type="checkbox" data-action="steadycom-parsimonious"${store.steadyComConfig?.parsimonious ? ' checked' : ''}${busy ? ' disabled' : ''}>
      Parsimonious</label>
    <div class="sc-mu-actions">
      <button class="primary" data-action="steadycom-recompute"${busy ? ' disabled' : ''}>${computed ? 'Recalculate range' : 'Calculate range'}</button>
      <button data-action="steadycom-solve" ${top === null || busy ? 'disabled' : ''}>Solve at this µ</button>
      <button data-action="steadycom-cancel"${busy ? '' : ' hidden'}>Cancel</button>
    </div>
    <p class="hint">Composition may be non-unique.</p>
  </section>`;
}

// The sweep figure

/**
 * The sweep, as the paper's Figure 4 draws it: two panels sharing an x axis.
 *
 * Two panels and not one chart with two y axes. The growth rate runs to about
 * 0.08 h⁻¹ and the aggregate exchange to about 6 C-mmol/h, and putting two
 * scales on one frame lets the reader infer a crossing that is an artefact of
 * where the axes were placed. The paper separates them for the same reason.
 *
 * The table below the panels is the same numbers, for reading exactly and for
 * anyone the colours do not reach.
 */
function sweepFigure(sweep, figure) {
  const xs = sweep.map((p) => Number(p[figure.x.key]));
  // Series colours follow the series' place in the panel, not its value, so a
  // curve keeps its colour whatever the numbers do.
  const colours = ['var(--series-1)', 'var(--series-2)'];

  const panels = figure.panels.map((panel) => {
    const series = panel.series.map((spec, i) => ({
      values: sweep.map((p) => Number(p[spec.key])),
      color: colours[i % colours.length],
      label: spec.label,
    }));
    return sweepPanel({
      xs,
      series,
      yLabel: panel.yLabel,
      title: panel.title,
      legend: series.length > 1,
      tip: (i) => `${figure.x.label.split('(')[0].trim()} ${xs[i]} · `
        + series.map((sr) => `${sr.label} ${format(sr.values[i])}`).join(' · '),
    });
  }).join('\n    ');

  return `<figure class="pc-figure">
    ${panels}
    <figcaption>${esc(figure.caption)}</figcaption>
    <details class="pc-figure-table"><summary>Table view</summary>
      ${sweepTable(sweep, figure)}</details>
  </figure>`;
}

/** Enough digits to tell two points apart, without a column of noise. */
function format(value) {
  if (!Number.isFinite(value)) return '—';
  const magnitude = Math.abs(value);
  if (magnitude === 0) return '0';
  return magnitude < 0.01 ? value.toExponential(2) : value.toFixed(magnitude < 1 ? 4 : 3);
}

function sweepPanel({ xs, series, yLabel, title, legend = false, tip }) {
  const W = 260;
  const H = 96;
  const padL = 40;
  const padR = 10;
  const padT = 16;
  const padB = 20;

  const all = series.flatMap((s) => s.values);
  const xMin = Math.min(...xs);
  const xMax = Math.max(...xs);
  const yMin = Math.min(0, ...all);
  const yMax = Math.max(...all);
  const span = yMax - yMin || 1;

  const X = (v) => padL + ((v - xMin) / ((xMax - xMin) || 1)) * (W - padL - padR);
  const Y = (v) => H - padB - ((v - yMin) / span) * (H - padT - padB);

  const paths = series.map((s) => {
    const d = s.values.map((v, i) => `${i ? 'L' : 'M'}${X(xs[i]).toFixed(1)} ${Y(v).toFixed(1)}`).join(' ');
    return `<path d="${d}" fill="none" stroke="${s.color}" stroke-width="2"
      stroke-linejoin="round" stroke-linecap="round"/>`;
  }).join('');

  // Direct labels at the end of each line, so identity never rests on colour.
  const endLabels = series.map((s) => {
    const last = s.values.length - 1;
    return `<text class="pc-end-label" x="${(X(xs[last]) - 2).toFixed(1)}"
      y="${(Y(s.values[last]) - 5).toFixed(1)}" text-anchor="end"
      fill="${s.color}">${esc(s.label)}</text>`;
  }).join('');

  // One wide invisible target per x, carrying a native tooltip: the hover layer
  // a line chart owes the reader, without a millisecond of JavaScript.
  const step = (W - padL - padR) / Math.max(1, xs.length - 1);
  const hits = xs.map((x, i) => `<rect x="${(X(x) - step / 2).toFixed(1)}" y="${padT}"
    width="${step.toFixed(1)}" height="${H - padT - padB}" fill="transparent">
    <title>${esc(tip(i))}</title></rect>`).join('');

  const ticks = [yMin, yMax].map((v) => `<g>
    <line x1="${padL}" y1="${Y(v).toFixed(1)}" x2="${W - padR}" y2="${Y(v).toFixed(1)}" class="pc-grid"/>
    <text class="pc-tick" x="${padL - 5}" y="${(Y(v) + 3).toFixed(1)}" text-anchor="end">${fmtTick(v)}</text>
  </g>`).join('');

  const xTicks = [xMin, xMax].map((v) => `<text class="pc-tick" x="${X(v).toFixed(1)}"
    y="${H - 6}" text-anchor="middle">${v}</text>`).join('');

  const key = legend ? `<div class="pc-legend">${series.map((s) =>
    `<span><i style="background:${s.color}"></i>${esc(s.label)}</span>`).join('')}</div>` : '';

  return `<div class="pc-panel">
    <div class="pc-panel-title">${esc(title)}${key}</div>
    <svg viewBox="0 0 ${W} ${H}" role="img" aria-label="${esc(`${title}: ${yLabel}`)}"
      preserveAspectRatio="xMidYMid meet">
      ${ticks}${xTicks}${paths}${endLabels}${hits}
      <text class="pc-axis" transform="rotate(-90)" x="${-(H / 2)}" y="10"
        text-anchor="middle">${esc(yLabel)}</text>
    </svg>
  </div>`;
}

const fmtTick = (v) => (Math.abs(v) >= 1 || v === 0
  ? String(Number(v.toFixed(2)))
  : String(Number(v.toPrecision(2))));

/** The same numbers as the figure, for reading exactly. */
function sweepTable(sweep, figure) {
  const columns = [
    { key: figure.x.key, label: figure.x.label.replace(/\s*\(.*\)$/, '') },
    ...figure.panels.flatMap((panel) => panel.series),
  ];
  const head = columns.map((col) => `<th>${esc(col.label)}</th>`).join('');
  const rows = sweep.map((point) => `<tr>${columns
    .map((col) => `<td>${esc(format(Number(point[col.key])))}</td>`).join('')}</tr>`).join('');
  return `<table class="pc-sweep">
    <thead><tr>${head}</tr></thead>
    <tbody>${rows}</tbody></table>`;
}

/**
 * What was reproduced, what was adapted, and where the models came from.
 *
 * The findings and the adaptations download together, because they only mean
 * anything together: a list of matched numbers without the departures that
 * produced them is the half of the story that flatters the result.
 */
function communityProvenanceBlock(c) {
  const { reproduction: rep } = c;
  const list = (items) => items.map((t) => `<li>${esc(t)}</li>`).join('');
  const sections = [];

  if (rep.reproduces?.length || rep.adaptations?.length) {
    sections.push(`<details><summary>Reproduced paper findings
        <button class="pc-dl" data-action="download-findings" data-community="${esc(c.id)}"
          title="Findings and adaptations as Markdown">Download</button></summary>
      ${rep.reproduces?.length ? `<p class="pc-sub">Reproduced</p><ul class="pc-list">${list(rep.reproduces)}</ul>` : ''}
      ${rep.adaptations?.length ? `<p class="pc-sub">Adaptations and assumptions</p><ul class="pc-list">${list(rep.adaptations)}</ul>` : ''}
      </details>`);
  }
  if (rep.needs?.length) {
    sections.push(`<details open><summary>Requirements for reproduction (${rep.needs.length})</summary>
      <ul class="pc-list">${list(rep.needs)}</ul></details>`);
  }
  const models = c.models ?? c.species.map((sp) => sp.model).filter((m) => m && m.id);
  if (models.length) {
    sections.push(`<details><summary>Model provenance (${models.length})</summary>
      <ul class="pc-list">${models.map((m) => `<li><b>${esc(m.id)}</b> — ${esc(m.reference)}<br>
        <span class="hint">${esc(m.source)}${m.note ? ` · ${esc(m.note)}` : ''}</span></li>`).join('')}</ul>
      </details>`);
  }
  return sections.join('');
}

// Species

function speciesCard(store) {
  if (store.isZunigaStudy) return `<section class="card"><h2>Case organisms</h2>
    ${['Ec', 'Se'].map(id => {
      const sp = store.species.find(item => item.id === id), model = store.models.get(sp?.file);
      const present = id === 'Ec' || store.zunigaScenarioId === 'mutant-coculture';
      return `<div class="species-item${present ? '' : ' is-off'}"><b>${esc(model?.cell?.organism ?? (id === 'Ec' ? 'Escherichia coli' : 'Synechococcus elongatus'))}</b><p class="hint">${present ? `Growth: ${formatFlux(store.growth.get(id), 6)} h⁻¹` : 'Absent from this monoculture; drawn faintly.'}</p></div>`;
    }).join('')}<p class="hint">Abundances are fixed by the source model’s population coefficients.</p></section>`;
  const items = store.species.map((sp) => {
    const model = store.models.get(sp.file);
    const mu = store.growth?.get(sp.id);
    const compartments = model ? model.compartmentIds() : [];
    const chips = compartments.map((cid) => {
      const c = compartmentColors(cellCompartmentVisualId(cid, model.compartmentNames), store.view.dark);
      return `<span class="chip" style="background:${c.fill};color:${c.text};border-color:${c.stroke}">`
        + `${esc(cellCompartmentName(cid, model.compartmentNames))}</span>`;
    }).join('');

    return `<div class="species-item${sp.enabled ? '' : ' is-off'}">
      <div class="species-head">
        <input type="checkbox" data-action="species-toggle" data-species="${esc(sp.id)}" ${sp.enabled ? 'checked' : ''}
          aria-label="Include ${esc(model?.cell.name ?? sp.id)} in the analysis">
        <span class="species-swatch" style="background:${esc(model?.cell.color ?? '#64748b')}"></span>
        <span class="species-name">${esc(model?.cell.name ?? sp.id)}</span>
        <span class="species-mu">${mu === undefined ? '—' : `µ ${formatFlux(mu, 4)}`}</span>
      </div>
      <div class="species-meta">${esc(model?.cell.organism ?? '')}</div>
      <div class="row" style="margin-top:7px">
        <label style="width:74px">Abundance</label>
        <input class="grow" type="range" min="0" max="1" step="0.05" value="${sp.abundance}"
          data-action="abundance" data-species="${esc(sp.id)}" aria-label="Abundance of ${esc(model?.cell.name ?? sp.id)}" ${sp.enabled ? '' : 'disabled'}>
        <span class="species-mu" style="width:34px;text-align:right">${(sp.abundance * 100).toFixed(0)}%</span>
      </div>
      <div class="species-compartments">${chips}</div>
    </div>`;
  }).join('');

  return `<section class="card">
    <h2>Species <span>${store.species.filter((s) => s.enabled).length} active</span></h2>
    <div class="species">${items}</div>
    <div class="row" style="margin-top:10px;gap:8px">
      <button class="primary grow" data-action="import-models">Load model(s)</button>
      <button class="grow" data-action="load-yeast9" title="Load the official yeast-GEM 9.1.1 SBML model">Yeast9 9.1.1</button>
    </div>
    <p class="hint">JSON, SBML, MAT or XLSX; one species per file.</p>
    <p class="hint">Abundances sum to 1.</p>
  </section>`;
}

// Map structure

const SHAPE_LABELS = {
  auto: 'Auto',
  rectangle: 'Rectangle',
  rounded: 'Rounded corners',
  capsule: 'Capsule',
  oval: 'Oval',
};

/**
 * What Add Compartment draws next, and in what shape. Every compartment of the
 * model is listed, and the ones that cannot be drawn yet are greyed out with
 * the reason: the model's nesting decides what goes where, and a list of only
 * the drawable ones would give no clue why most organelles are missing.
 */
function compartmentPlacementControls(store, idPrefix) {
  const choices = store.mapCompartmentChoices?.() ?? [];
  const current = store.currentCompartmentPlacement?.() ?? null;
  const currentKey = current ? `${current.speciesId}|${current.compartmentId}` : null;
  const manySpecies = new Set(choices.map((item) => item.speciesId)).size > 1;
  const option = (item) => {
    const key = `${item.speciesId}|${item.compartmentId}`;
    const who = manySpecies ? `${item.speciesName} · ` : '';
    return `<option value="${esc(key)}"${key === currentKey ? ' selected' : ''}${item.status === 'ready' ? '' : ' disabled'}${item.reason ? ` title="${esc(item.reason)}"` : ''}>${esc(who)}${esc(item.compartmentName)} (${esc(item.compartmentId)})</option>`;
  };
  const groups = [
    ['ready', 'Ready to draw'],
    ['waiting', 'Not yet'],
    ['cell', 'Drawn with Add Cell'],
    ['drawn', 'On the map'],
  ].map(([status, label]) => {
    const items = choices.filter((item) => item.status === status);
    return items.length ? `<optgroup label="${label}">${items.map(option).join('')}</optgroup>` : '';
  }).join('');
  const shapes = ['auto', ...Object.keys(COMPARTMENT_SHAPES)].map((name) =>
    `<option value="${name}"${(store.compartmentShapePreset ?? 'auto') === name ? ' selected' : ''}${name === 'auto' ? ' title="Its usual shape; organelles are round"' : ''}>${SHAPE_LABELS[name]}</option>`
  ).join('');
  return `<label for="${idPrefix}-compartment-select">Compartment to draw</label>
    <select id="${idPrefix}-compartment-select" data-action="compartment-placement"
      aria-label="Compartment to add"${choices.length ? '' : ' disabled'}>${groups}</select>
    <label for="${idPrefix}-compartment-shape">Shape</label>
    <select id="${idPrefix}-compartment-shape" data-action="compartment-shape-preset"
      aria-label="Shape of the new compartment">${shapes}</select>`;
}

/* The left panel while Add Compartment is the tool: choose, then drag. */
function compartmentToolControls(store) {
  const current = store.currentCompartmentPlacement?.() ?? null;
  const parent = current
    ? store.mapCompartmentChoices?.().find((item) =>
      item.speciesId === current.speciesId && item.compartmentId === current.compartmentId)?.parentName
    : null;
  return `<div class="compartment-tool-controls">
    ${compartmentPlacementControls(store, 'tool')}
    <p class="hint">${current
      ? `Drag inside <strong>${esc(parent ?? current.parentId)}</strong>, highlighted on the map.`
      : 'Nothing can be drawn yet.'}</p>
  </div>`;
}

function mapStructureCard(store) {
  const availableCells = store.availableMapCells?.() ?? [];
  const currentCellSpecies = availableCells.some((item) => item.speciesId === store.cellPlacementSpeciesId)
    ? store.cellPlacementSpeciesId
    : availableCells[0]?.speciesId ?? null;
  const cellOptions = availableCells.map((item) =>
    `<option value="${esc(item.speciesId)}"${item.speciesId === currentCellSpecies ? ' selected' : ''}>
      ${esc(item.speciesName)} · boundary ${esc(item.outerCompartmentId)} (${esc(item.outerCompartmentName)})
    </option>`
  ).join('');

  const available = store.availableMapCompartments?.() ?? [];

  const cells = store.map?.cells?.length ?? 0;
  const compartments = store.map?.cells?.reduce((n, cell) => n + cell.compartments.length, 0) ?? 0;

  return `<section class="card">
    <h2>Map structure <span>${cells} cells · ${compartments} compartments</span></h2>

    ${availableCells.length ? `
      <div class="map-structure-block">
        <strong>Cell boundary</strong>
        <label for="cell-species-select">Species to draw</label>
        <select id="cell-species-select" data-action="cell-placement-species"
          aria-label="Species to draw a cell for">${cellOptions}</select>
        <p class="hint">Pick the <strong>Cell</strong> tool and drag on the map.</p>
      </div>`
      : '<p class="hint">Every active species already has a cell boundary drawn.</p>'}

    <div class="map-structure-block">
      <strong>Nested compartments</strong>
      ${compartmentPlacementControls(store, 'map')}
      ${available.length
        ? `<div class="row" style="margin-top:8px">
            <button class="primary grow" data-action="tool-compartment" type="button">Draw compartment</button>
            <button data-action="add-map-compartment" type="button" title="Skip drawing and fit it inside the parent automatically">Place automatically</button>
          </div>`
        : '<p class="empty">Nothing to draw: draw a cell first, or all compartments are on the map.</p>'}
      <p class="hint">Drag inside the highlighted parent.</p>
    </div>
  </section>`;
}

// Model editor

function modelEditorCard(store) {
  if (store.isZunigaStudy) return `<section class="card"><h2>Complete source models</h2>
    <label class="row">Model <select class="grow" data-action="model-editor-species">${store.species.map(sp => `<option value="${esc(sp.id)}"${store.modelEditorSpeciesId === sp.id ? ' selected' : ''}>${esc(sp.id)}</option>`).join('')}</select></label>
    <p class="hint">The map shows only the complementing pathways.</p>
    <p class="hint">Source models are read-only in this case.</p></section>`;
  const speciesOptions = store.species.map((sp) => {
    const model = store.models.get(sp.file);
    return `<option value="${esc(sp.id)}"${sp.id === store.modelEditorSpeciesId ? ' selected' : ''}>
      ${esc(model?.name ?? sp.id)} [${esc(sp.id)}]
    </option>`;
  }).join('');

  const sp = store.modelEditorSpecies();
  const model = store.modelEditorModel();
  const compartments = model?.compartmentIds() ?? [];
  const mode = store.modelEditorMode;

  const summary = model
    ? `<p class="hint"><strong>${model.metabolites.size}</strong> metabolites ·
      <strong>${model.reactions.size}</strong> reactions ·
      <strong>${compartments.length}</strong> compartments.</p>`
    : '<p class="empty">No editable model.</p>';

  let form = '';
  if (mode === 'metabolite' && model) {
    const compartmentOptions = compartments.map((cid) =>
      `<option value="${esc(cid)}">${esc(cid)} · ${esc(cellCompartmentName(cid, model.compartmentNames))}</option>`
    ).join('');

    form = `<div class="model-editor-form" data-editor-kind="metabolite">
      <h3>New metabolite</h3>
      <label>ID *</label>
      <input type="text" name="id" placeholder="lac__L_c" autocomplete="off">
      <label>Name *</label>
      <input type="text" name="name" placeholder="L-lactate" autocomplete="off">
      <label>Compartment *</label>
      <select name="compartment">${compartmentOptions}</select>
      <div class="model-editor-two">
        <div><label>Formula <span>(optional)</span></label><input type="text" name="formula" placeholder="C3H5O3"></div>
        <div><label>Charge <span>(optional)</span></label><input type="number" name="charge" value="0" step="1"></div>
      </div>
      <div class="row model-editor-actions">
        <button data-action="cancel-model-editor" type="button">Cancel</button>
        <button class="primary grow" data-action="create-metabolite" type="button">Create metabolite</button>
      </div>
    </div>`;
  }

  if (mode === 'reaction' && model) {
    form = `<div class="model-editor-form" data-editor-kind="reaction">
      <h3>New reaction</h3>
      <label>ID *</label>
      <input type="text" name="id" placeholder="LDH_NEW" autocomplete="off">
      <label>Name</label>
      <input type="text" name="name" placeholder="Lactate dehydrogenase" autocomplete="off">
      <label>Stoichiometry *</label>
      <textarea name="stoichiometry" rows="5"
        placeholder="pyr_c:-1, nadh_c:-1, h_c:-1, lac__L_c:1, nad_c:1"></textarea>
      <p class="hint">Negative = substrate, positive = product.</p>
      <div class="model-editor-two">
        <div><label>Lower bound *</label><input type="number" name="lower" value="0" step="any"></div>
        <div><label>Upper bound *</label><input type="number" name="upper" value="1000" step="any"></div>
      </div>
      <label>GPR</label>
      <input type="text" name="gpr" placeholder="geneA and (geneB or geneC)" autocomplete="off">
      <label>Subsystem <span>(optional)</span></label>
      <input type="text" name="subsystem" placeholder="Glycolysis" autocomplete="off">
      <label class="row model-editor-check">
        <input type="checkbox" name="addToMap" checked>
        <span>Add to the map after creating</span>
      </label>
      <div class="row model-editor-actions">
        <button data-action="cancel-model-editor" type="button">Cancel</button>
        <button class="primary grow" data-action="create-reaction" type="button">Create reaction</button>
      </div>
    </div>`;
  }

  return `<section class="card model-editor-card">
    <h2>Model editor</h2>
    <label>Model / species</label>
    <select data-action="model-editor-species" aria-label="Model to edit">${speciesOptions}</select>
    ${summary}
    <div class="row" style="margin-top:9px">
      <button class="grow" data-action="new-metabolite" type="button">+ Metabolite</button>
      <button class="grow" data-action="new-reaction" type="button">+ Reaction</button>
    </div>
    <div class="row" style="margin-top:7px">
      <button class="grow" data-action="export-model-json" type="button">Export COBRA JSON</button>
      <button class="grow" data-action="export-model-sbml" type="button">Export SBML</button>
    </div>
    <p class="hint">SBML L3V1 + fbc v2.</p>
    ${form}
  </section>`;
}

// Medium

function mediumCard(store) {
  if (store.isZunigaStudy) return `<section class="card"><h2>Source medium</h2><p class="hint">The source model fixes the medium for each case scenario.</p></section>`;
  const bases = new Set(Object.keys(store.medium));
  if (store.community) for (const b of store.community.transfersByBase.keys()) bases.add(b);

  // Known components lead, in the order they usually matter; anything else a
  // loaded model brings with it still gets a slider rather than disappearing.
  const ordered = [...bases].sort((a, b) => {
    const ia = MEDIUM_ORDER.indexOf(a);
    const ib = MEDIUM_ORDER.indexOf(b);
    return (ia < 0 ? 99 : ia) - (ib < 0 ? 99 : ib) || a.localeCompare(b);
  });

  const rows = ordered.map((base) => {
    const value = Math.abs(Number(store.medium[base] ?? 0));
    const max = value > 100 ? 1000 : 40;
    const step = max > 100 ? 10 : 0.5;
    return `<div class="row">
      <label style="width:96px">${esc(MEDIUM_LABELS[base] ?? base)}</label>
      <input class="grow" type="range" min="0" max="${max}" step="${step}" value="${Math.min(value, max)}"
        data-action="medium" data-base="${esc(base)}" aria-label="Supply of ${esc(MEDIUM_LABELS[base] ?? base)}">
      <input type="number" style="width:72px" min="0" step="${step}" value="${value}"
        data-action="medium-number" data-base="${esc(base)}" aria-label="Maximum supply of ${esc(MEDIUM_LABELS[base] ?? base)}">
    </div>`;
  }).join('');

  return `<section class="card">
    <h2>Shared medium</h2>
    ${rows || '<p class="empty">No exchangeable medium components.</p>'}
    <p class="hint">mmol·gDW⁻¹·h⁻¹; 0 = only from secretion.</p>
  </section>`;
}

// Reaction search

function communityOmicsCard(store) {
  const o=store.omics??{}, d=o.dataset, r=o.report;
  const opts=(selected)=>(d?.conditions??[]).map(c=>`<option value="${esc(c)}"${c===selected?' selected':''}>${esc(c)}</option>`).join('');
  const speciesRows=(r?.species??[]).slice(0,12).map(x=>`<div class="crossfeed-row"><div><b>${esc(x.species)}</b> · genes ${x.measuredGenes}/${x.genes}</div><span>${x.gprReactions?Math.round(100*x.mappedReactions/x.gprReactions):0}% rxn</span></div>`).join('');
  return `<section class="card">
    <h2>Metatranscriptomics</h2>
    <div class="row"><button class="grow" data-action="import-metatranscriptomics">${d?'Replace':'Import'} CSV/TSV</button>${d?`<button data-action="omics-toggle">${o.enabled?'Hide':'Show'}</button>`:''}</div>
    ${!d?'<p class="hint">Columns: species/MAG, gene, condition, expression.</p>':`
      <p class="hint"><strong>${esc(d.source)}</strong> · ${d.rowCount} genes/MAG · ${d.conditions.length} condition(s)</p>
      <div class="row"><button data-action="omics-single" aria-pressed="${o.mode==='single'}">Condition</button><button data-action="omics-compare" aria-pressed="${o.mode==='compare'}">A vs B</button></div>
      <div class="row"><label>A</label><select class="grow" data-action="community-omics-condition-a" aria-label="Community expression condition A">${opts(o.conditionA)}</select></div>
      ${o.mode==='compare'?`<div class="row"><label>B</label><select class="grow" data-action="community-omics-condition-b" aria-label="Community expression condition B">${opts(o.conditionB)}</select></div>
      <div class="row"><select class="grow" data-action="community-omics-metric" aria-label="Community expression comparison"><option value="log2fc"${o.metric==='log2fc'?' selected':''}>log₂(B/A)</option><option value="difference"${o.metric==='difference'?' selected':''}>B − A</option></select><input style="width:70px" type="number" min="0" step="0.1" value="${o.pseudocount}" data-action="community-omics-pseudocount" title="Pseudocount" aria-label="Community expression pseudocount"></div>`:''}
      <h3 class="inspector-section-title" title="Penalises flux through reactions below the expression threshold">GIMME context-specific model</h3>
      <div class="row"><label>Objective</label><input style="width:75px" type="number" min="0" max="100" step="1" value="${Math.round(100*(o.gimmeObjectiveFraction??0.9))}" data-action="gimme-objective-fraction"><span>%</span></div>
      <div class="row"><label>Threshold</label><input class="grow" type="number" step="any" placeholder="auto: P25" value="${o.gimmeThreshold??''}" data-action="gimme-threshold"></div>
      <div class="row"><button class="grow" data-action="run-gimme"${store.solution?.steadyCom ? ' disabled title="Run an ordinary analysis before GIMME"' : ''}>Run GIMME · ${esc(o.conditionA??'')}</button>${o.gimmeEnabled?'<button data-action="disable-gimme">Return FBA</button>':''}</div>
      ${store.solution?.steadyCom ? '<p class="hint">Run FBA, pFBA or Loopless before GIMME.</p>' : ''}
      ${o.gimmeResult?.status==='optimal'?`<p class="hint">Threshold <strong>${Number(o.gimmeResult.threshold).toPrecision(4)}</strong> · inconsistency <strong>${Number(o.gimmeResult.inconsistencyScore).toPrecision(4)}</strong> · inactive penalized <strong>${o.gimmeResult.inactive?.size??0}</strong></p>`:''}
      <h3 class="inspector-section-title">Mapping report</h3>
      <p class="hint">Genes: <strong>${r?.measuredGenes??0}/${r?.genesInModels??0}</strong> (${Math.round(100*(r?.geneCoverage??0))}%) · GPR reactions: <strong>${r?.mappedReactions??0}/${r?.gprReactions??0}</strong> (${Math.round(100*(r?.reactionCoverage??0))}%)</p>
      ${r?.unmatchedSpecies?.length?`<p class="hint">MAG/species not found: ${r.unmatchedSpecies.map(esc).join(', ')}</p>`:''}
      <div class="crossfeed-list">${speciesRows}</div>
    `}
  </section>`;
}

function reactionSearchCard(store) {
  return `<section class="card reaction-search-card">
    <h2>Find reaction</h2>
    <input type="text" id="reaction-search-input" data-action="reaction-search"
      value="${esc(store.reactionSearchQuery ?? '')}"
      placeholder="ID, name or subsystem…" autocomplete="off" spellcheck="false">
    <div id="reaction-search-results" class="reaction-search-results">
      ${renderReactionSearchResults(store, store.reactionSearchQuery)}
    </div>
  </section>`;
}

/** Search results for the reaction library as HTML, at most 30, each offering Select (drawn) or Add to map. */
export function renderReactionSearchResults(store, query) {
  const q = String(query ?? '').trim();
  if (!q) {
    return '<p class="empty">Search every reaction in the model.</p>';
  }

  const results = store.searchReactions(q, { limit: 30 });
  if (!results.length) {
    return `<p class="empty">No reactions found for “${esc(q)}”.</p>`;
  }

  return results.map((item) => {
    const status = item.drawn ? 'On map' : 'Not drawn';
    const action = item.drawn ? 'search-select-reaction' : 'search-add-reaction';
    const actionLabel = item.drawn ? 'Select' : 'Add to map';
    const meta = [item.detail ?? item.name, item.subsystem, item.speciesId].filter(Boolean).join(' · ');
    return `<div class="reaction-search-result">
      <div class="reaction-search-copy">
        <strong>${esc(item.label ?? item.localId)}</strong>
        <span>${esc(meta)}</span>
        <small>${status}</small>
      </div>
      <button data-action="${action}" data-rxn="${esc(item.reactionId)}"
        class="${item.drawn ? '' : 'primary'}">${actionLabel}</button>
    </div>`;
  }).join('');
}

// Pathway manager

function pathwayManagerCard(store) {
  const entries = store.pathwayManagerEntries?.() ?? [];
  const selectedReactionCount = store.multiSelection?.reactions?.size ?? 0;

  const items = entries.map((item) => {
    const manual = item.source === 'manual';
    const name = manual
      ? `<input class="pathway-manager-name" type="text" data-action="pathway-name"
          data-pathway-key="${esc(item.key)}" value="${esc(item.name)}" aria-label="Pathway name">`
      : `<strong title="${esc(item.name)}">${esc(item.name)}</strong>`;

    return `<div class="pathway-manager-item${item.visible ? '' : ' is-hidden'}">
      <input type="checkbox" data-action="pathway-visible" data-pathway-key="${esc(item.key)}"
        ${item.visible ? 'checked' : ''} title="Show or hide ribbon" aria-label="Show ribbon for ${esc(item.name)}">
      <input class="pathway-manager-color" type="color" value="${esc(item.color)}"
        data-action="pathway-color" data-pathway-key="${esc(item.key)}" title="Pathway color" aria-label="Color of ${esc(item.name)}">
      <div class="pathway-manager-copy">
        ${name}
        <small>${manual ? 'Manual' : 'Model'} · ${item.count} drawn reaction${item.count === 1 ? '' : 's'}</small>
      </div>
      <button data-action="pathway-select" data-pathway-key="${esc(item.key)}" title="Select all pathway reactions">Select</button>
      <button data-action="pathway-focus" data-pathway-key="${esc(item.key)}" title="Center this pathway on the map" aria-label="Focus ${esc(item.name)}">⌖</button>
      ${manual ? `<button class="danger" data-action="pathway-delete" data-pathway-key="${esc(item.key)}" title="Delete manual pathway" aria-label="Delete ${esc(item.name)}">×</button>` : ''}
    </div>`;
  }).join('');

  const createBlock = selectedReactionCount > 0
    ? `<div class="pathway-create">
        <div class="pathway-create-head">
          <strong>Create pathway from selection</strong>
          <span>${selectedReactionCount} reaction${selectedReactionCount === 1 ? '' : 's'}</span>
        </div>
        <div class="pathway-create-row">
          <input type="text" name="pathway-name" placeholder="Pathway name…" autocomplete="off" aria-label="New pathway name">
          <input type="color" name="pathway-color" value="#2563eb" title="Color" aria-label="New pathway color">
          <button class="primary" data-action="pathway-create">Create</button>
        </div>
      </div>`
    : `<div class="pathway-create-empty">
        <p class="hint">Select one or more reactions to create a manual pathway.</p>
        <button data-action="pathway-enable-selection">Enable multi-select</button>
      </div>`;

  return `<section class="card pathway-manager-card">
    <h2>Pathway Manager <span>${entries.length}</span></h2>
    ${createBlock}
    <div class="pathway-manager-list">
      ${items || '<p class="empty">No annotated pathways among the drawn reactions.</p>'}
    </div>
  </section>`;
}

// Omics overlay

function omicsOverlayCard(store) {
  const dataset = store.omicsDataset;
  const settings = store.omicsSettings ?? {};
  const overlay = store.omicsOverlay?.() ?? { enabled: false, matchedReactions: 0, totalDrawn: 0, scale: null };
  const conditions = dataset?.conditions ?? [];
  const options = (selected) => conditions
    .map((name) => `<option value="${esc(name)}"${name === selected ? ' selected' : ''}>${esc(name)}</option>`)
    .join('');

  const typeControl = `<div class="row">
    <label class="grow">Data type</label>
    <select data-action="omics-type" style="width:130px" aria-label="Omics overlay data type">
      <option value="gene"${settings.dataType === 'gene' ? ' selected' : ''}>Gene expression</option>
      <option value="protein"${settings.dataType === 'protein' ? ' selected' : ''}>Proteomics</option>
    </select>
  </div>`;

  if (!dataset) {
    return `<section class="card omics-card">
      <h2>Omics Overlay</h2>
      ${typeControl}
      <div class="row" style="margin-top:9px">
        <button class="primary grow" data-action="import-omics">Load CSV</button>
      </div>
      <p class="hint">Columns: <code>id,Condition_A,Condition_B</code>, optional <code>species</code>.</p>
      <p class="hint">IDs map through each reaction's GPR (AND = minimum, OR = maximum).</p>
    </section>`;
  }

  const scale = overlay.scale;
  const min = Number.isFinite(scale?.min) ? formatFlux(scale.min, 3) : '—';
  const max = Number.isFinite(scale?.max) ? formatFlux(scale.max, 3) : '—';
  const comparison = settings.mode === 'compare' && conditions.length >= 2;
  const gradient = comparison
    ? 'linear-gradient(90deg,#2563eb 0%,#f8fafc 50%,#dc2626 100%)'
    : 'linear-gradient(90deg,#e2e8f0 0%,#fca5a5 55%,#b91c1c 100%)';

  return `<section class="card omics-card">
    <h2>Omics Overlay <span>${esc(dataset.dataType === 'protein' ? 'proteomics' : 'genes')}</span></h2>
    ${typeControl}
    <div class="omics-file-meta">
      <strong title="${esc(dataset.source)}">${esc(dataset.source)}</strong>
      <span>${dataset.rowCount} IDs · ${conditions.length} condition${conditions.length === 1 ? '' : 's'}</span>
    </div>

    <label class="row" style="margin-top:9px;cursor:pointer">
      <input type="checkbox" data-action="omics-enabled" ${settings.enabled ? 'checked' : ''}>
      <span class="grow"><strong>Show overlay</strong></span>
    </label>

    <div class="row" style="margin-top:8px">
      <label class="grow">View</label>
      <select data-action="omics-mode" style="width:150px" aria-label="Omics overlay view" ${conditions.length < 2 ? 'disabled' : ''}>
        <option value="single"${settings.mode === 'single' ? ' selected' : ''}>One condition</option>
        <option value="compare"${settings.mode === 'compare' ? ' selected' : ''}>Compare A vs B</option>
      </select>
    </div>

    <div class="omics-condition-grid">
      <label>${comparison ? 'Condition A' : 'Condition'}</label>
      <select data-action="omics-condition-a" aria-label="Omics overlay condition A">${options(settings.conditionA)}</select>
      ${comparison ? `
        <label>Condition B</label>
        <select data-action="omics-condition-b" aria-label="Omics overlay condition B">${options(settings.conditionB)}</select>
      ` : ''}
    </div>

    ${comparison ? `
      <div class="row" style="margin-top:8px">
        <label class="grow">Comparison</label>
        <select data-action="omics-metric" style="width:150px" aria-label="Omics overlay comparison metric">
          <option value="log2fc"${settings.metric === 'log2fc' ? ' selected' : ''}>log₂(B/A)</option>
          <option value="difference"${settings.metric === 'difference' ? ' selected' : ''}>B − A</option>
        </select>
      </div>
      ${settings.metric === 'log2fc' ? `
        <div class="row" style="margin-top:6px">
          <label class="grow">Pseudocount</label>
          <input type="number" min="0" step="0.01" value="${esc(settings.pseudocount)}"
            data-action="omics-pseudocount" style="width:90px" aria-label="Omics overlay pseudocount">
        </div>` : ''}
    ` : ''}

    <div class="omics-scale" style="background:${gradient}"></div>
    <div class="omics-scale-labels">
      <span>${comparison ? `${esc(settings.conditionA)} ↑` : min}</span>
      <span>${comparison ? (settings.metric === 'log2fc' ? '0 log₂FC' : '0 Δ') : 'expression / abundance'}</span>
      <span>${comparison ? `${esc(settings.conditionB)} ↑` : max}</span>
    </div>

    <p class="hint">${overlay.enabled
      ? `<strong>${overlay.matchedReactions}/${overlay.totalDrawn}</strong> drawn reactions have a GPR-derived omics value.`
      : 'Overlay is hidden.'}</p>
    <div class="row" style="margin-top:8px">
      <button data-action="import-omics" class="grow">Replace CSV</button>
      <button data-action="clear-omics">Remove</button>
    </div>
  </section>`;
}

// Inspector

function omicsReactionReadout(store, reactionId) {
  if (!store.omicsDataset || !store.omicsSettings.enabled) return '';
  const overlay = store.omicsOverlay();
  const a = overlay.scoresA.get(reactionId);
  const b = overlay.scoresB.get(reactionId);
  const value = overlay.values.get(reactionId);

  if (overlay.mode === 'compare') {
    if (!Number.isFinite(a) || !Number.isFinite(b) || !Number.isFinite(value)) {
      return '<p class="hint omics-inspector">Omics: no value for both conditions.</p>';
    }
    const metric = overlay.metric === 'log2fc' ? 'log₂FC' : 'Δ';
    return `<div class="omics-inspector">
      <div class="omics-inspector-head"><strong>Omics</strong><span>${esc(metric)}</span></div>
      <dl class="readout">
        <div><dt>${esc(overlay.conditionA)}</dt><dd>${formatFlux(a, 4)}</dd></div>
        <div><dt>${esc(overlay.conditionB)}</dt><dd>${formatFlux(b, 4)}</dd></div>
      </dl>
      <p class="hint">${esc(metric)} = <strong>${formatFlux(value, 4)}</strong></p>
    </div>`;
  }

  if (!Number.isFinite(value)) {
    return '<p class="hint omics-inspector">Omics: no value for this reaction.</p>';
  }
  return `<div class="omics-inspector">
    <div class="omics-inspector-head"><strong>Omics</strong><span>${esc(overlay.conditionA)}</span></div>
    <p class="hint">Reaction expression/abundance = <strong>${formatFlux(value, 4)}</strong></p>
  </div>`;
}

function inspectorHeader(kind, title, subtitle = '', closeAction = 'clear-selection') {
  return `<header class="inspector-heading"><span>${esc(kind)}</span><button class="icon" data-action="${closeAction}" title="Close">×</button>
    <h2>${esc(title)}</h2>${subtitle ? `<p>${esc(subtitle)}</p>` : ''}</header>`;
}

/**
 * Whether the inspector is about the multi-selection: nothing is selected on
 * its own (a selected reaction keeps its inspector, which takes the line ends
 * from shift-clicked metabolites), and there is something to arrange, two
 * objects or one reaction to lay on a line.
 */
export function showsSelectionInspector(store) {
  if (store.selectedReaction || store.selectedEntity || !store.map) return false;
  return selectionSize(store.multiSelection) >= 2 || Boolean(store.canArrangeSelection?.());
}

/** "3 reactions · 4 metabolites": what a selection holds, kinds with none left out. */
function selectionCounts(selection) {
  return [
    [selection?.reactions?.size ?? 0, 'reaction'],
    [selection?.nodes?.size ?? 0, 'metabolite'],
    [selection?.texts?.size ?? 0, 'text'],
    [selection?.compartments?.size ?? 0, 'compartment'],
  ].filter(([count]) => count > 0)
    .map(([count, noun]) => `${count} ${noun}${count === 1 ? '' : 's'}`)
    .join(' · ');
}

/**
 * Arrange the multi-selection as a whole: align and distribute it, space it
 * evenly along its own line, and turn it, rigidly, to a compass direction or
 * by 45°. All map-only.
 */
function selectionInspector(store) {
  const selection = store.multiSelection;
  const spaced = spacingItems(store.map, selection).length;
  const hasLine = Boolean(selectionMainAxis(store.map, selection));
  // The way the selection runs, as on a compass with north at the top of the map.
  const compass = [
    ['NW', '↖', 'up-left'], ['N', '↑', 'up'], ['NE', '↗', 'up-right'],
    ['W', '←', 'left'], null, ['E', '→', 'right'],
    ['SW', '↙', 'down-left'], ['S', '↓', 'down'], ['SE', '↘', 'down-right'],
  ].map((cell) => (cell
    ? `<button data-action="selection-point" data-direction="${cell[0]}" type="button"${hasLine ? '' : ' disabled'}
        title="Point ${cell[2]}" aria-label="Point ${cell[0]}">${cell[1]}</button>`
    : '<span class="reaction-compass-centre" aria-hidden="true">●</span>')).join('');

  return `<section class="inspector-detail selection-inspector">
    ${inspectorHeader('Map', 'Selection', selectionCounts(selection), 'clear-multi-selection')}
    <h3 class="inspector-section-title">Align &amp; distribute</h3>
    <div class="metabolite-inspector-controls">${arrangeSelectionBlock(store)}</div>
    <h3 class="inspector-section-title">Spacing</h3>
    <div class="metabolite-inspector-controls">
      <button data-action="selection-space-evenly" type="button"${spaced < 3 ? ' disabled' : ''}
        title="Equal steps along the selection's own line">Even spacing</button>
    </div>
    <h3 class="inspector-section-title">Rotate</h3>
    <div class="metabolite-inspector-controls selection-turn-controls">
      <div class="reaction-align-row">
        <span class="metabolite-role-label">Point</span>
        <div class="reaction-compass" role="group" aria-label="Point the selection">${compass}</div>
      </div>
      <div class="row">
        <button class="grow" data-action="selection-rotate" data-angle="-45" type="button" title="Rotate 45° anticlockwise">⟲ 45°</button>
        <button class="grow" data-action="selection-rotate" data-angle="45" type="button" title="Rotate 45° clockwise">⟳ 45°</button>
      </div>
    </div>
    <p class="inspector-note">Turns as one piece; cofactors follow.</p>
  </section>`;
}

function propertyRow(label, value, className = '') {
  return `<div class="property-row ${className}"><dt>${esc(label)}</dt><dd>${value}</dd></div>`;
}

function mapLabelControl(store, kind, id = null) {
  const defaultSource = kind === 'metabolite' ? 'base' : 'id';
  const globalSource = store.mapLabelSource?.(kind) ?? defaultSource;
  const object = id ? store.mapLabelObject?.(kind, id) : null;
  const selected = id ? (object?.labelSource ?? 'inherit') : globalSource;
  const options = store.mapLabelOptions?.(kind, id) ?? [
    { value: defaultSource, label: kind === 'metabolite' ? 'Base label' : 'Model ID' },
    { value: 'name', label: 'Model name' },
  ];
  const rows = [];
  if (id) {
    rows.push(`<option value="inherit"${selected === 'inherit' ? ' selected' : ''}>Follow View setting</option>`);
  }
  for (const option of options) {
    const preview = id ? store.mapLabelText?.(kind, id, option.value) : '';
    const text = preview ? `${option.label}: ${preview}` : option.label;
    rows.push(`<option value="${esc(option.value)}"${selected === option.value ? ' selected' : ''}>${esc(text)}</option>`);
  }
  const label = id ? 'Map label' : (kind === 'metabolite' ? 'Metabolite labels' : 'Reaction labels');
  const target = id ? ` data-${kind === 'metabolite' ? 'node' : 'rxn'}="${esc(id)}"` : '';
  return `<label class="view-label-control map-label-control">${label}
    <select data-action="${kind}-label-source"${target} aria-label="${kind === 'metabolite' ? 'Metabolite' : 'Reaction'} label source">
      ${rows.join('')}
    </select>
  </label>`;
}

function metaboliteInspector(store, nodeId) {
  const node = store.map?.nodes.get(nodeId);
  if (!node) return `<section class="inspector-empty"><h2>Metabolite</h2><p>The selected metabolite is no longer present.</p></section>`;
  const metabolite = store.community?.merged.metabolites.get(node.mergedId);
  const owner = String(node.mergedId ?? '').split('@')[0];
  const species = store.species.find((item) => item.id === owner);
  const model = store.models.get(species?.file);
  // Shared-medium ids start with the medium's own prefix (`M@ac`), which is not
  // the id of any cell.
  const sharedMedium = node.kind === 'medium' || (!species && !node.cellId && owner === 'M');
  const cellName = sharedMedium ? 'Shared medium' : (model?.cell?.name ?? owner) || '—';
  const connected = store.map?.segmentsForNode?.(nodeId)?.length ?? 0;
  const shadowPrice = store.solution?.shadowPrices?.get(node.mergedId);
  const copies = store.metaboliteCopies?.(nodeId)
    ?? [...(store.map?.nodes?.values?.() ?? [])].filter((item) => item.mergedId === node.mergedId);
  const removable = Boolean(store.canRemoveMapNode?.(nodeId));
  // The heading gives one readable name; a model name that lists synonyms is shown whole below it.
  const modelName = metabolite?.name ?? '';
  const name = modelName ? readableName(modelName, [metabolite?.id, node.base]) : '';

  return `<section class="inspector-detail">
    ${inspectorHeader('Metabolite', node.base ?? metabolite?.id ?? node.id, name)}
    <dl class="property-list">
      ${propertyRow('Map ID', `<code>${esc(node.id)}</code>`)}
      ${propertyRow('Model ID', `<code>${esc(metabolite?.id ?? node.mergedId ?? '')}</code>`)}
      ${name !== modelName ? propertyRow('Model name', esc(modelName)) : ''}
      ${propertyRow('Formula', esc(metabolite?.formula ?? '—'))}
      ${propertyRow('Shadow price', Number.isFinite(shadowPrice) ? formatFlux(shadowPrice, 6) : '—')}
      ${propertyRow('Charge', Number.isFinite(metabolite?.charge) ? esc(metabolite.charge) : '—')}
      ${propertyRow('Compartment', esc(node.compartment ?? '—'))}
      ${propertyRow('Cell', esc(cellName))}
      ${propertyRow('Visual role', node.primary === false ? 'Secondary' : 'Primary')}
      ${propertyRow('Visual copies', String(copies.length || 1))}
      ${propertyRow('Connected paths', String(connected))}
      ${propertyRow('Radius', `${esc(node.r ?? '—')} px`)}
      ${propertyRow('Reactions', metaboliteReactionLinks(store, metabolite, owner), 'is-stacked')}
      ${propertyRow('Annotations', provenanceBlock(localMetabolite(store, node.mergedId) ?? metabolite, 'metabolite'), 'is-stacked')}
    </dl>

    <h3 class="inspector-section-title">Visual map instance</h3>
    ${mapLabelControl(store, 'metabolite', nodeId)}
    <div class="metabolite-inspector-controls">
      ${node.kind === 'currency'
        // A cofactor the layout placed beside one reaction is secondary by
        // construction; the map refuses to make it primary, so offering the
        // button only made it look broken.
        ? '<p class="inspector-note cofactor-role-note">Cofactor beside its reaction: always secondary.</p>'
        : `<div class="metabolite-role-row">
        <span class="metabolite-role-label">Visual role</span>
        <div class="metabolite-role-buttons">
          <button class="${node.primary !== false ? 'is-active' : ''}" data-action="metabolite-primary" data-node="${esc(nodeId)}" type="button">Primary</button>
          <button class="${node.primary === false ? 'is-active' : ''}" data-action="metabolite-secondary" data-node="${esc(nodeId)}" type="button">Secondary</button>
        </div>
      </div>
      <p class="metabolite-size-note">Primary = 13 px · Secondary = 7 px</p>`}
      ${nodeColorRow(store, [nodeId], node)}
      <button class="grow" data-action="duplicate-metabolite" data-node="${esc(nodeId)}" type="button">Duplicate metabolite</button>
      <button class="grow danger" data-action="hide-metabolite" data-node="${esc(nodeId)}" type="button">Hide metabolite</button>
      ${removable
        ? `<button class="danger inspector-delete" data-action="remove-map-node" data-node="${esc(nodeId)}" type="button">Delete metabolite from map</button>`
        : '<p class="inspector-note">Remove the drawn reactions that use it before deleting it.</p>'}
    </div>
  </section>`;
}

/**
 * Fill and outline for one metabolite circle or several. Shows what is drawn
 * now: the node's own colour, else the map's, else the theme's.
 */
function nodeColorRow(store, nodeIds, node = null) {
  const style = store.map?.nodeStyle ?? {};
  const theme = defaultNodeColors(store.view?.dark);
  const own = Boolean(node ? node.fill || node.stroke : nodeIds.some((id) => {
    const item = store.map?.nodes?.get?.(id);
    return item?.fill || item?.stroke;
  }));
  const ids = esc(JSON.stringify(nodeIds));
  return `<div class="map-color-row" data-node-ids="${ids}">
      <span class="map-color-row-title">Circle color</span>
      <label>Fill <input type="color" data-action="node-fill" data-node-ids="${ids}" value="${esc(node?.fill ?? style.fill ?? theme.fill)}" aria-label="Circle fill"></label>
      <label>Outline <input type="color" data-action="node-stroke" data-node-ids="${ids}" value="${esc(node?.stroke ?? style.stroke ?? theme.stroke)}" aria-label="Circle outline"></label>
      <button data-action="node-colors-reset" data-node-ids="${ids}" type="button"${own ? '' : ' disabled'}>Map colors</button>
    </div>`;
}

/**
 * The colour of one reaction's lines or several, used when lines are not
 * coloured by flux.
 */
function reactionColorRow(store, reactionIds) {
  const reactions = reactionIds.map((id) => store.map?.reactions?.get?.(id)).filter(Boolean);
  if (!reactions.length) return '';
  const own = reactions.find((rxn) => rxn.color)?.color;
  const byFlux = (store.view?.reactionColorMode ?? 'flux') === 'flux';
  const ids = esc(JSON.stringify(reactions.map((rxn) => rxn.id)));
  return `<div class="map-color-row">
      <span class="map-color-row-title">Line color</span>
      <label>Color <input type="color" data-action="reaction-line-color" data-rxn-ids="${ids}" value="${esc(own ?? '#64748b')}" aria-label="Reaction line color"></label>
      <button data-action="reaction-color-reset" data-rxn-ids="${ids}" type="button"${own ? '' : ' disabled'}>Pathway color</button>
    </div>
    ${byFlux ? '<p class="inspector-note">Used when lines are not colored by flux.</p>' : ''}`;
}

function geneInspector(store, speciesId, geneId) {
  const sp=store.species.find(x=>x.id===speciesId), model=store.models.get(sp?.file);
  const gene=(model?.genes??[]).find(g=>String(g.id)===String(geneId));
  if(!gene)return `<section class="inspector-empty"><h2>Gene</h2><p>Gene ${esc(geneId)} not found.</p></section>`;
  const reactions=[...model.reactions.values()].filter(r=>genesFromRule(r.geneReactionRule).includes(geneId));
  const rxns=reactions.map(r=>`<button class="chip" data-action="select-model-reaction" data-rxn="${esc(speciesId+'@'+r.id)}">${esc(r.id)}</button>`).join(' ')||'—';
  const fields=Object.entries(gene).filter(([k])=>!['annotation','annotations','notes'].includes(k)).map(([k,v])=>propertyRow(k,esc(formatField(v)))).join('');
  const key=speciesId?`${speciesId}@${gene.id}`:String(gene.id);
  const knockout=!store.isZunigaStudy&&store.geneIndex?.().genes.has(key)
    ?`<div class="row" style="margin-top:10px">${geneKnockoutButton(store,key,{wide:true})}</div><p class="hint">${esc(geneEffectText(store,key))}</p>`:'';
  return `<section class="inspector-detail">
    ${inspectorHeader('Gene',gene.id,gene.name??'')}
    <dl class="property-list">${fields}${propertyRow('Cell / species',esc(model?.cell?.name??speciesId))}${propertyRow('Reactions',rxns,'is-stacked')}${propertyRow('Annotations',provenanceBlock(gene,'gene'),'is-stacked')}${propertyRow('Notes',notesBlock(gene.notes),'is-stacked')}</dl>
    ${knockout}
  </section>`;
}

/** What knocking out a gene does, or does now that it is out, in words. */
function geneEffectText(store, key) {
  const on = store.geneKnockouts.has(key);
  const closes = store.geneKnockoutEffect(key).map((rid) => store.reactionTitle(rid));
  if (!closes.length) return on ? 'Knocked out; other genes keep its reactions open.' : 'Knocking it out closes nothing: other genes keep its reactions open.';
  const list = closes.length > 6 ? `${closes.slice(0, 6).join(', ')} +${closes.length - 6} more` : closes.join(', ');
  return on ? `Knocked out: closes ${list}.` : `Knocking it out closes ${list}.`;
}

/** The KO toggle of one gene (`species@gene`), titled with what it does. */
function geneKnockoutButton(store, key, { wide = false } = {}) {
  const on = store.geneKnockouts.has(key);
  const label = wide ? (on ? 'Restore gene' : 'Knock out gene') : 'KO';
  return `<button type="button" class="${wide ? `grow${on ? '' : ' danger'}` : `chip gene-ko${on ? ' is-on' : ''}`}" data-action="gene-ko" data-gene="${esc(key)}"
    aria-pressed="${on}" title="${esc(geneEffectText(store, key))}">${label}</button>`;
}
function provenanceBlock(entity,type){
 const links=annotationLinks(entity?.annotation??{},entity?.notes??{},type);
 if(!links.length)return '—';
 return `<div class="provenance-links">${links.map(x=>x.url?`<a href="${esc(x.url)}" target="_blank" rel="noopener noreferrer"><b>${esc(x.label)}</b>: ${esc(x.value)} ↗</a>`:`<span><b>${esc(x.label)}</b>: ${esc(x.value)}</span>`).join('')}</div>`;
}
function notesBlock(notes){if(!notes||!Object.keys(notes).length)return '—';return Object.entries(notes).map(([k,v])=>`<div><b>${esc(k)}</b>: ${esc(formatField(v))}</div>`).join('');}
function formatField(v){return Array.isArray(v)?v.join(', '):typeof v==='object'?JSON.stringify(v):String(v??'');}
function genesFromRule(rule){return [...new Set(String(rule??'').replace(/[()]/g,' ').split(/\s+/).filter(x=>x&&!/^(and|or)$/i.test(x)))];}
function localMetabolite(store,mergedId){const owner=store.community?.metaboliteOwner?.get(mergedId);if(!owner?.speciesId)return null;const sp=store.species.find(x=>x.id===owner.speciesId);return store.models.get(sp?.file)?.metabolites.get(owner.originalId)??null;}
function metaboliteReactionLinks(store,met,owner){
 if(!met)return '—'; return (met.reactions??[]).map(rid=>{const merged=owner&&owner!=='M'?owner+'@'+rid:rid;return `<button class="chip" data-action="select-model-reaction" data-rxn="${esc(merged)}">${esc(rid)}</button>`;}).join(' ')||'—';
}

function textInspector(store, textId) {
  const item = store.map?.texts.get(textId);
  if (!item) return `<section class="inspector-empty"><h2>Text</h2><p>The selected text is no longer present.</p></section>`;
  return `<section class="inspector-detail">
    ${inspectorHeader('Text', item.text, item.id)}
    <dl class="property-list">
      ${propertyRow('ID', `<code>${esc(item.id)}</code>`)}
      ${propertyRow('Content', esc(item.text))}
      ${propertyRow('X', formatFlux(item.x, 2))}
      ${propertyRow('Y', formatFlux(item.y, 2))}
    </dl>
    <div class="inspector-section-label">STYLE</div>
    ${textStyleFields({ fontSize: item.fontSize, fontFamily: item.fontFamily }, `data-text="${esc(item.id)}"`)}
    <p class="inspector-note">Also used for the next text you write.</p>
    <button class="inspector-delete danger" data-action="remove-map-text" data-text="${esc(item.id)}">Delete text</button>
  </section>`;
}

/** Size and font controls for a map text; `ids` names the text they act on. */
export function textStyleFields({ fontSize, fontFamily } = {}, ids = '') {
  const size = Number.isFinite(Number(fontSize)) ? Number(fontSize) : TEXT_SIZE.default;
  const family = fontFamily ?? DEFAULT_TEXT_FONT;
  const options = TEXT_FONTS.map((font) =>
    `<option value="${esc(font.id)}"${font.id === family ? ' selected' : ''}>${esc(font.label)}</option>`).join('');
  return `<label class="field"><span>Size</span><input type="number" data-action="text-font-size" ${ids}
      min="${TEXT_SIZE.min}" max="${TEXT_SIZE.max}" step="1" value="${esc(size)}" aria-label="Text size in pixels"></label>
    <label class="field"><span>Font</span><select data-action="text-font-family" ${ids} aria-label="Text font">${options}</select></label>`;
}

/*
 * Colour pickers for a cell or a compartment. Each shows the colour actually
 * drawn, theme or chosen, and is applied on `change`, once the picker closes,
 * so choosing a colour is one undo step rather than one per pointer move.
 */
const pickerValue = (...candidates) => candidates.map(normalizeHexColor).find(Boolean) ?? '#64748b';

function colorField(label, action, ids, value) {
  return `<label class="field"><span>${label}</span><input type="color" data-action="${action}" ${ids} value="${esc(value)}"></label>`;
}

function cellColorFields(store, cell) {
  const body = cell.compartments.find((item) => item.id === cell.outerCompartment)
    ?? cell.compartments.find((item) => item.parentId === null);
  const theme = body ? compartmentColors(body.visualId ?? body.id, Boolean(store.view?.dark)) : {};
  const ids = `data-cell-id="${esc(cell.id)}"`;
  const custom = Boolean(body?.fill || cell.stroke);
  return colorField('Fill color', 'cell-fill-color', ids, pickerValue(body?.fill, theme.fill))
    + colorField('Border color', 'cell-border-color', ids, pickerValue(cell.stroke, cell.color))
    + (custom ? `<button class="secondary wide" data-action="cell-reset-colors" ${ids} type="button">Use theme colors</button>` : '');
}

function compartmentColorFields(store, cellId, comp) {
  const theme = compartmentColors(comp.visualId ?? comp.id, Boolean(store.view?.dark));
  const ids = `data-cell-id="${esc(cellId)}" data-compartment-id="${esc(comp.id)}"`;
  return colorField('Fill color', 'compartment-fill-color', ids, pickerValue(comp.fill, theme.fill))
    + colorField('Border color', 'compartment-border-color', ids, pickerValue(comp.stroke, theme.stroke))
    + (comp.fill || comp.stroke
      ? `<button class="secondary wide" data-action="compartment-reset-colors" ${ids} type="button">Use theme colors</button>`
      : '');
}

function cellInspector(store, cellId) {
  const cell = store.map?.cells.find((item) => item.id === cellId);
  if (!cell) return `<section class="inspector-empty"><h2>Cell</h2><p>The selected cell is no longer present.</p></section>`;
  const species = store.species.find((item) => item.id === cell.id);
  const model = store.models.get(species?.file);
  return `<section class="inspector-detail">
    ${inspectorHeader('Cell', cell.name ?? cell.id, model?.cell?.organism ?? '')}
    <dl class="property-list">
      ${propertyRow('ID', `<code>${esc(cell.id)}</code>`)}
      ${propertyRow('Compartments', String(cell.compartments?.length ?? 0))}
      ${propertyRow('Outer compartment', esc(cell.outerCompartment ?? '—'))}
      ${propertyRow('Abundance', species ? `${Math.round(species.abundance * 100)}%` : '—')}
      ${propertyRow('Growth rate', store.growth?.has(cell.id) ? `${formatFlux(store.growth.get(cell.id), 4)} h⁻¹` : '—')}
      ${propertyRow('Canvas bounds', `${Math.round(cell.rect.w)} × ${Math.round(cell.rect.h)}`)}
    </dl>
    <div class="inspector-section-label">MAP</div>
    ${cellColorFields(store, cell)}
    <button class="secondary wide" data-action="edit-selected-cell-boundary" data-cell-id="${esc(cellId)}" type="button">Edit Boundary</button>
    <button class="danger wide" data-action="remove-map-compartment" data-cell-id="${esc(cellId)}" type="button">Delete cell</button>
    <p class="inspector-note">Delete key also works; Ctrl+Z undoes.</p>
  </section>`;
}

/* Ready-made shapes for a compartment, and how to get any shape in between. */
function compartmentShapeFields(cellId, comp) {
  const now = compartmentShape(comp);
  const matches = (s) => Math.abs(s.corner - now.corner) < 0.01 && Math.abs(s.oval - now.oval) < 0.01;
  const buttons = Object.entries(COMPARTMENT_SHAPES).map(([name, s]) =>
    `<button type="button" data-action="compartment-shape" data-shape="${name}"
      data-cell-id="${esc(cellId)}" data-compartment-id="${esc(comp.id)}"
      aria-pressed="${matches(s) ? 'true' : 'false'}">${SHAPE_LABELS[name]}</button>`
  ).join('');
  return `<div class="inspector-section-label">SHAPE</div>
    <div class="compartment-shape-presets" role="group" aria-label="Compartment shape">${buttons}</div>
    <p class="inspector-note">Or drag the <b class="shape-key shape-key-corner">orange diamond</b> to round the corners and the <b class="shape-key shape-key-oval">yellow diamond</b> to make it oval.</p>`;
}

function compartmentInspector(store, cellId, compartmentId) {
  if ((cellId === null || cellId === undefined) && compartmentId === store.map?.environment?.id) {
    const env = store.map.environment;
    return `<section class="inspector-detail">
      ${inspectorHeader('Shared medium', env.label ?? 'Shared medium', 'System boundary')}
      <dl class="property-list">
        ${propertyRow('ID', `<code>${esc(env.id)}</code>`)}
        ${propertyRow('Role', 'Shared extracellular medium')}
        ${propertyRow('Bounds', `${Math.round(env.rect.w)} × ${Math.round(env.rect.h)}`)}
        ${propertyRow('Position', `x ${Math.round(env.rect.x)}, y ${Math.round(env.rect.y)}`)}
      </dl>
      <div class="inspector-section-label">MAP</div><button class="secondary wide" data-action="edit-selected-compartment-boundary" type="button">Edit Boundary</button>
    </section>`;
  }

  const cell = store.map?.cells.find((item) => item.id === cellId);
  const comp = cell?.compartments.find((item) => item.id === compartmentId);
  if (!comp) return `<section class="inspector-empty"><h2>Compartment</h2><p>The selected compartment is no longer present.</p></section>`;
  return `<section class="inspector-detail">
    ${inspectorHeader('Compartment', comp.label ?? comp.id, cell?.name ?? cellId)}
    <dl class="property-list">
      ${propertyRow('ID', `<code>${esc(comp.id)}</code>`)}
      ${propertyRow('Cell', esc(cell?.name ?? cellId))}
      ${propertyRow('Parent', esc(comp.parentId ?? 'Cell root'))}
      ${propertyRow('Depth', String(comp.depth ?? 0))}
      ${propertyRow('Membrane', esc(comp.membrane ?? '—'))}
      ${propertyRow('Bounds', `${Math.round(comp.rect.w)} × ${Math.round(comp.rect.h)}`)}
    </dl>
    <p class="inspector-note compartment-move-hint">Drag the <b>centre</b> to move it, a <b>corner</b> or <b>side</b> to resize it.</p>
    ${compartmentShapeFields(cellId, comp)}
    <div class="inspector-section-label">MAP</div>${compartmentColorFields(store, cellId, comp)}<button class="secondary wide" data-action="edit-selected-compartment-boundary" type="button">Edit Boundary</button>
    <button class="danger wide" data-action="remove-map-compartment" data-cell-id="${esc(cellId)}" data-compartment-id="${esc(comp.id)}" type="button">${comp.parentId ? 'Delete compartment' : 'Delete cell'}</button>
    <p class="inspector-note">Delete key also works; Ctrl+Z undoes.</p>
  </section>`;
}

function inspectorCard(store) {
  const rid = store.selectedReaction;
  if (!rid || !store.layout) {
    return `<section class="inspector-empty"><div class="inspector-empty-icon">⌁</div>
      <h2>No reaction selected</h2><p>Select a reaction on the map.</p></section>`;
  }

  const info = store.layout.reactions.get(rid);
  const merged = store.community?.merged.reactions.get(rid);
  // The reaction of a species held out of the simulation: switched off (kept
  // dimmed on a hand-built map, and gone from the community) or, in a joint
  // community, at zero abundance (still in the problem, held at zero). It has
  // no flux or bounds to show; offering them would edit what is not applied.
  const speciesId = store.community?.reactionOwner?.get(rid)?.speciesId ?? String(rid).split('@')[0];
  const heldOut = Boolean(store.speciesHeldOut?.(speciesId));
  if (!merged || heldOut) {
    const sp = store.species.find((item) => item.id === speciesId);
    const name = store.models.get(sp?.file)?.cell?.name ?? speciesId;
    const why = !sp ? 'Drawn, but not in the active model.'
      : sp.enabled === false
        ? `<b>${esc(name)}</b> is switched off, so this reaction is not simulated.`
        : heldOut
          ? `<b>${esc(name)}</b> has zero abundance, so this reaction is held at zero.`
          : 'Drawn, but not in the active model.';
    return `<section class="inspector-detail">
      ${inspectorHeader('Reaction', info?.localId ?? rid, sp ? name : '')}
      <p class="inspector-note">${why}</p>
    </section>`;
  }
  const owner = store.community.reactionOwner.get(rid) || {};
  const flux = store.solution?.fluxes.get(rid) ?? 0;
  const bounds = store.currentReactionBounds(rid);
  const defaults = store.originalReactionBounds?.(rid) ?? store.defaultReactionBounds(rid);
  const knockedOut = Boolean(store.reactionKnockedOut?.(rid));
  const modified = store.originalReactionBounds
    ? Math.abs(bounds.lower - defaults.lower) > 1e-9 || Math.abs(bounds.upper - defaults.upper) > 1e-9
    : store.overrides.has(rid);
  const fva = store.fvaEnabled ? store.fvaRanges.get(rid) ?? null : null;
  const reducedCost = store.solution?.reducedCosts?.get(rid);
  const scaledReducedCost = store.solution?.scaledReducedCosts?.get(rid);
  const objectiveCoefficient = store.objectiveCoefficients?.()[rid] ?? merged?.objectiveCoefficient ?? 0;

  const span = Math.max(Math.abs(defaults.lower), Math.abs(defaults.upper), 10);
  const lo = defaults.lower < 0 ? -span : 0;
  const hi = span;

  const cellName = owner.speciesId
    ? store.models.get(store.species.find((s) => s.id === owner.speciesId)?.file)?.cell.name
    : 'Shared medium';

  const compartments = (info?.compartments ?? []).map((cid) => {
    const c = compartmentColors(cid, store.view.dark);
    return `<span class="chip" style="background:${c.fill};color:${c.text};border-color:${c.stroke}">${esc(cid)}</span>`;
  }).join('');
  const localModel = owner.speciesId ? store.models.get(store.species.find((x)=>x.id===owner.speciesId)?.file) : null;
  const localReaction = owner.originalId ? localModel?.reactions.get(owner.originalId) : null;
  const annotations = provenanceBlock(localReaction ?? merged, 'reaction');
  const geneIndex = store.geneIndex?.().genes ?? new Map();
  const geneClosedBy = store.geneClosedReactions?.().get(rid) ?? [];
  const geneButtons = genesFromRule(localReaction?.geneReactionRule ?? merged?.geneReactionRule).map((gid) => {
    const select = `<button class="chip" data-action="select-gene" data-gene="${esc(gid)}" data-species="${esc(owner.speciesId??'')}">${esc(gid)}</button>`;
    const key = owner.speciesId ? `${owner.speciesId}@${gid}` : gid;
    if (store.isZunigaStudy || !geneIndex.has(key)) return select;
    return `<span class="gene-ko-pair">${select}${geneKnockoutButton(store, key)}</span>`;
  }).join(' ') || '—';
  const metaboliteButtons = Object.keys(localReaction?.metabolites ?? {}).map((mid) => {
    const mergedId=owner.speciesId ? `${owner.speciesId}@${mid}` : mid;
    return `<button class="chip" data-action="select-model-metabolite" data-metabolite="${esc(mergedId)}">${esc(mid)}</button>`;
  }).join(' ') || '—';

  let fvaBlock = store.fvaEnabled
    ? '<p class="hint">Computing the FVA range…</p>'
    : '';
  if (store.fvaEnabled && fva && Number.isFinite(fva.min) && Number.isFinite(fva.max)) {
    const range = Math.max(hi - lo, 1e-9);
    const left = ((fva.min - lo) / range) * 100;
    const width = ((fva.max - fva.min) / range) * 100;
    const mark = ((flux - lo) / range) * 100;
    fvaBlock = `<div class="range-bar">
        <i style="left:${clamp(left)}%;width:${clamp(width)}%"></i>
        <b style="left:${clamp(mark)}%"></b>
      </div>
      <div class="flux-ticks"><span>${formatFlux(fva.min, 3)}</span>
      <span>FVA at the optimal objective</span><span>${formatFlux(fva.max, 3)}</span></div>`;
  }

  return `<section class="inspector-detail">
    ${inspectorHeader('Reaction', owner.originalId ?? rid, merged?.name ?? '')}
    <div class="property-chips">${compartments}</div>
    <dl class="property-list">
      ${propertyRow('ID', `<code>${esc(owner.originalId ?? rid)}</code>`)}
      ${propertyRow('Name', esc(merged?.name ?? '—'))}
      ${propertyRow('Equation', `<code class="equation-inline">${esc(merged?.equation() ?? '')}</code>`, 'is-stacked')}
      ${propertyRow('Objective coefficient', `${formatFlux(objectiveCoefficient, 4)}${store.objectiveSense?.() === 'min' ? ' (minimised)' : ''}`)}
      ${propertyRow('Reduced cost', Number.isFinite(reducedCost) ? formatFlux(reducedCost, 6) : '—')}
      ${propertyRow('Scaled reduced cost', Number.isFinite(scaledReducedCost) ? formatFlux(scaledReducedCost, 6) : '—')}
      ${propertyRow('GPR', esc(merged?.geneReactionRule || '—'))}
      ${propertyRow('Genes', geneButtons, 'is-stacked')}
      ${propertyRow('Metabolites', metaboliteButtons, 'is-stacked')}
      ${propertyRow('Compartment', compartments || '—')}
      ${propertyRow('Cell', esc(cellName ?? '—'))}
      ${propertyRow('Subsystem', esc(info?.subsystem ?? merged?.subsystem ?? '—'))}
      ${propertyRow('Annotations', annotations, 'is-stacked')}
    </dl>

    <dl class="readout">
      <div><dt>Flux</dt><dd>${formatFlux(flux, 4)}</dd></div>
      <div><dt>State</dt><dd style="font-size:13px">${geneClosedBy.length ? 'Gene knockout' : knockedOut ? 'Knocked out' : modified ? 'Modified' : 'Original'}</dd></div>
    </dl>

    ${omicsReactionReadout(store, rid)}
    ${fvaBlock}

    ${store.map?.reactions.has(rid) ? mapLabelControl(store, 'reaction', rid) + reactionColorRow(store, [rid]) : ''}
    <h3 class="inspector-section-title">Bounds</h3>
    ${store.isZunigaStudy ? '<p class="hint">Source bounds are fixed for this case.</p><fieldset disabled style="border:0;padding:0;margin:0">' : ''}
    ${geneClosedBy.length ? `<p class="hint">Closed by the knockout of gene${geneClosedBy.length === 1 ? '' : 's'} ${esc(geneClosedBy.map((key) => store.geneTitle(key)).join(', '))}.</p>` : ''}
    <div class="bound-row">
      <label>Lower</label>
      ${boundControl({ rid, side: 'lower', value: bounds.lower, label: owner.originalId ?? rid, disabled: geneClosedBy.length > 0 })}
    </div>
    <div class="bound-row">
      <label>Upper</label>
      ${boundControl({ rid, side: 'upper', value: bounds.upper, label: owner.originalId ?? rid, disabled: geneClosedBy.length > 0 })}
    </div>
    <p class="hint bound-scale-note">Log sliders, 0 in the middle.</p>

    <div class="row" style="margin-top:10px">
      ${knockedOut
        ? `<button data-action="reset-reaction" data-rxn="${esc(rid)}" title="Back to the bounds as opened">Restore</button>`
        : `<button class="danger" data-action="knockout" data-rxn="${esc(rid)}">Knockout</button>`}
      <button data-action="reset-reaction" data-rxn="${esc(rid)}">Reset</button>
      <span class="grow"></span>
      <button data-action="reset-all" title="Reset all modified reactions">All</button>
    </div>
    ${store.isZunigaStudy ? '</fieldset>' : ''}
    <button class="inspector-reactions-link" data-action="rx-reveal" data-rid="${esc(rid)}" type="button">Show in Reactions ↗</button>
    ${reactionDrawingControls(store, rid)}
    <div class="row" style="margin-top:8px">
      <button class="danger grow inspector-delete" data-action="remove-map-reaction" data-rxn="${esc(rid)}">Delete reaction from map</button>
    </div>
    <p class="hint">Original bounds: [${formatFlux(defaults.lower, 3)}, ${formatFlux(defaults.upper, 3)}]</p>
  </section>`;
}

/**
 * How one reaction is drawn: straight or free, which metabolite stands where,
 * and which way round it is. The counterpart of the metabolite inspector's
 * "Visual map instance", and like it none of this reaches the model.
 */
function reactionDrawingControls(store, rid) {
  const map = store.map;
  const rxn = map?.reactions?.get(rid);
  if (!rxn || typeof map.reactionSides !== 'function') return '';

  const sides = map.reactionSides(rid);
  const linear = Boolean(rxn.linear);
  const canLinear = linear || map.canLinearizeReaction(rid);
  const ends = linear
    ? [map.nodes.get(rxn.linear.from)?.base, map.nodes.get(rxn.linear.to)?.base].filter(Boolean)
    : [];

  const membrane = Boolean(rxn.onMembrane);
  const line = membrane ? { from: null, to: null } : map.reactionLineEnds(rid);
  // What a press on Linear or on the compass will use: two metabolites picked
  // out on the map win over the ones marked here.
  const picked = store.selectedLineEnds?.(rid) ?? {};
  const lineEnd = (entry, side) => (picked[side] ?? line[side]) === entry.nodeId;

  // Each metabolite is a button that runs the reaction's line through it, and
  // between each pair of neighbours a ⇄ exchanges their places: enough to reach
  // any order, and it says which two a press will exchange.
  const sideRow = (label, entries, side) => {
    if (!entries.length) return '';
    const parts = entries.map((entry, i) => {
      const on = lineEnd(entry, side);
      const chip = `<button class="chip reaction-line-end${on ? ' is-active' : ''}" data-action="reaction-line-end"
        data-rxn="${esc(rid)}" data-node="${esc(entry.nodeId)}" type="button" aria-pressed="${on}"${membrane ? ' disabled' : ''}
        title="${membrane ? esc(entry.label) : `Run the line through ${esc(entry.label)}`}">${esc(entry.label)}</button>`;
      const next = entries[i + 1];
      if (!next) return chip;
      const allowed = entry.group === next.group;
      return `${chip}<button class="reaction-swap" data-action="reaction-swap" data-rxn="${esc(rid)}"
        data-a="${esc(entry.nodeId)}" data-b="${esc(next.nodeId)}" type="button"${allowed ? '' : ' disabled'}
        title="${allowed
          ? `Swap ${esc(entry.label)} and ${esc(next.label)}`
          : 'In different compartments; cannot swap'}"
        aria-label="Swap ${esc(entry.label)} and ${esc(next.label)}">⇄</button>`;
    }).join('');
    return `<div class="reaction-swap-row"><span class="metabolite-role-label">${label}</span><div class="reaction-swap-list">${parts}</div></div>`;
  };

  // Substrate to product, as on a compass with north at the top of the map.
  const compass = [
    ['NW', '↖', 'up and to the left'], ['N', '↑', 'upwards'], ['NE', '↗', 'up and to the right'],
    ['W', '←', 'to the left'], null, ['E', '→', 'to the right'],
    ['SW', '↙', 'down and to the left'], ['S', '↓', 'downwards'], ['SE', '↘', 'down and to the right'],
  ].map((cell) => (cell
    ? `<button data-action="reaction-align" data-rxn="${esc(rid)}" data-direction="${cell[0]}" type="button"${canLinear ? '' : ' disabled'}
        title="Lay the reaction out ${cell[2]}"
        aria-label="Align ${cell[0]}">${cell[1]}</button>`
    : '<span class="reaction-compass-centre" aria-hidden="true">●</span>')).join('');

  return `<h3 class="inspector-section-title">Map drawing</h3>
    <div class="metabolite-inspector-controls reaction-drawing-controls">
      <div class="metabolite-role-row">
        <span class="metabolite-role-label">Shape</span>
        <div class="metabolite-role-buttons">
          <button class="${linear ? '' : 'is-active'}" data-action="reaction-free" data-rxn="${esc(rid)}" type="button">Free</button>
          <button class="${linear ? 'is-active' : ''}" data-action="reaction-linear" data-rxn="${esc(rid)}" type="button"${canLinear ? '' : ' disabled'}
            title="${membrane
              ? 'A transporter stays on its membrane'
              : 'Straight line between the two marked metabolites'}">Linear</button>
        </div>
      </div>
      ${linear ? `<p class="metabolite-size-note">Kept straight between ${esc(ends.join(' and '))}.</p>` : ''}
      ${sideRow('Substrates', sides.substrates, 'from')}
      ${sideRow('Products', sides.products, 'to')}
      <div class="reaction-align-row">
        <span class="metabolite-role-label">Align</span>
        <div class="reaction-compass" role="group" aria-label="Lay the reaction out in a direction">${compass}</div>
      </div>
      <div class="row reaction-axis-row">
        <button class="grow" data-action="reaction-align-axis" data-rxn="${esc(rid)}" data-axis="vertical" type="button"${canLinear ? '' : ' disabled'}
          title="Reaction and marked metabolites on one vertical line">Vertical</button>
        <button class="grow" data-action="reaction-align-axis" data-rxn="${esc(rid)}" data-axis="horizontal" type="button"${canLinear ? '' : ' disabled'}
          title="Reaction and marked metabolites on one horizontal line">Horizontal</button>
      </div>
      <button data-action="reaction-flip" data-rxn="${esc(rid)}" type="button"${membrane ? ' disabled' : ''}
        title="Swap the places of substrates and products">Swap substrates ⇄ products</button>
      <div class="row">
        <button class="grow" data-action="reaction-mirror" data-rxn="${esc(rid)}" type="button"
          title="Mirror the reaction across its own line">Mirror</button>
        <button class="grow" data-action="reaction-rotate" data-rxn="${esc(rid)}" data-angle="-90" type="button"${map.canRotateReaction(rid) ? '' : ' disabled'}
          title="Rotate 90° anticlockwise">⟲ 90°</button>
        <button class="grow" data-action="reaction-rotate" data-rxn="${esc(rid)}" data-angle="90" type="button"${map.canRotateReaction(rid) ? '' : ' disabled'}
          title="Rotate 90° clockwise">⟳ 90°</button>
      </div>
      <button data-action="reaction-tidy" data-rxn="${esc(rid)}" type="button"
        title="Reset cofactor places and curve shapes">Tidy curves</button>
      ${rxn.labelOffset ? `<button data-action="reaction-label-reset" data-rxn="${esc(rid)}" type="button"
        title="Put the name back beside the marker">Reset label</button>` : ''}
    </div>
    <p class="inspector-note">Shift-click two metabolites to pick them.</p>`;
}

// Cross-feeding

/** Names of the exchanged metabolites, from the medium nodes of the map: one readable name each. */
function exchangeNames(store) {
  const names = new Map();
  for (const node of store.layout?.nodes?.values?.() ?? []) {
    if (node.kind !== 'medium') continue;
    const m = /^M@(.+)_s$/.exec(node.mergedId ?? '');
    const base = m ? m[1] : node.base;
    if (base && node.name && !names.has(base)) names.set(base, readableName(node.name, [base, node.localId]));
  }
  return names;
}

/** An exchanged metabolite as the cross-feeding panels name it: `Acetate (ac)`, or its id when it has no other name. */
function crossFeedingLabel(names, base) {
  const name = names.get(base);
  return name && name !== base ? `${name} (${base})` : base;
}

/** How many exchanged metabolites the map shows: a number in the View menu. */
function crossFeedingTopControl(store) {
  const all = store.crossFeedingMetabolites();
  if (!all.length) return '';
  const custom = Array.isArray(store.view.crossFeedingShown);
  const shown = store.crossFeedingVisibleBases().size;
  return `<div class="crossfeed-top-control">
      <label title="SCFAs and lactate first, then carbon sources; 0 shows all">Exchanged metabolites shown
        <input type="number" min="0" max="${all.length}" step="1" data-action="crossfeed-top" value="${custom ? shown : store.view.crossFeedingTop}" aria-label="Number of exchanged metabolites shown"></label>
      <span class="hint">of ${all.length}${custom ? ' · picked by hand' : ''}</span>
      <button type="button" data-action="crossfeed-choose" title="Pick them one by one">Choose…</button>
    </div>`;
}

/** The cross-feeding line width slider, as a share of the default width. */
function crossFeedingWidthControl(store) {
  if (!store.crossFeedingMetabolites().length) return '';
  const { min, max, step } = CROSSFEED_WIDTH_RANGE;
  const value = store.view.crossFeedingWidth ?? CROSSFEED_WIDTH_RANGE.default;
  return `<label class="view-label-control crossfeed-width-control" title="100% is about a cell boundary's width">Cross-feeding line width
      <span class="crossfeed-width-row">
        <input class="grow" type="range" min="${min}" max="${max}" step="${step}" data-action="crossfeed-width" value="${esc(value)}" aria-label="Cross-feeding line width">
        <span data-crossfeed-width-output>${Math.round(value * 100)}%</span>
      </span>
    </label>`;
}

/** The colour of each metabolite whose cross-feeding lines are drawn, as the map gives them. */
function drawnCrossFeedingColors(store) {
  const shown = store.crossFeedingVisibleBases();
  const bases = store.crossFeedingMetabolites().map((m) => m.base).filter((base) => shown.has(base));
  return crossFeedingColors(bases, store.view.dark);
}

/** A short bar in a cross-feeding line's colour; hollow for a metabolite not drawn. */
function crossFeedingSwatch(color) {
  return `<span class="crossfeed-swatch"${color ? ` style="background:${esc(color)}"` : ''} aria-hidden="true"></span>`;
}

/** One checkbox per exchanged metabolite, grouped, with its line colour, lines and flux. */
function crossFeedingMetabolitePicker(store) {
  const all = store.crossFeedingMetabolites();
  if (!all.length) return '';
  const shown = store.crossFeedingVisibleBases();
  const colors = drawnCrossFeedingColors(store);
  const names = exchangeNames(store);
  let group = '';
  const rows = all.map((m) => {
    const head = m.group !== group ? `<p class="crossfeed-group">${esc(m.group)}</p>` : '';
    group = m.group;
    return `${head}<label class="crossfeed-pick">
        <input type="checkbox" data-action="crossfeed-metabolite" data-base="${esc(m.base)}"${shown.has(m.base) ? ' checked' : ''}>
        ${crossFeedingSwatch(colors.get(m.base))}
        <span class="grow">${esc(crossFeedingLabel(names, m.base))}</span>
        <small>${m.lines} line${m.lines === 1 ? '' : 's'} · ${formatFlux(m.flux, 3)}</small>
      </label>`;
  }).join('');
  return `<div class="crossfeed-picker">
      <div class="row"><h3 class="grow">Metabolites on the map</h3>
        <span class="hint">${shown.size} of ${all.length}</span></div>
      <div class="row">
        <button type="button" data-action="crossfeed-show-top" data-top="5">Top 5</button>
        <button type="button" data-action="crossfeed-show-all">All</button>
        <button type="button" data-action="crossfeed-show-none">None</button>
      </div>
      <div class="crossfeed-pick-list">${rows}</div>
      <p class="hint">FBA does not say which cell feeds which; lines split each metabolite by production and uptake.</p>
    </div>`;
}

/** The key to the cross-feeding lines on the map: each metabolite drawn, its colour and how many lines it has. */
function crossFeedingLegend(store) {
  const colors = drawnCrossFeedingColors(store);
  if (!colors.size) return '';
  const names = exchangeNames(store);
  const items = store.crossFeedingMetabolites().filter((m) => colors.has(m.base)).map((m) => {
    const label = crossFeedingLabel(names, m.base);
    return `<div class="reaction-category-item">${crossFeedingSwatch(colors.get(m.base))}<span title="${esc(label)}">${esc(label)}</span><b>${m.lines}</b></div>`;
  }).join('');
  return `<div class="reaction-category-legend crossfeed-legend">
      <div class="reaction-category-head"><strong>Cross-feeding lines</strong><span>${colors.size}</span></div>
      ${items}
    </div>`;
}

function crossFeedingCard(store) {
  const x=store.crossFeedingExplorer, idx=x.index, visible=store.crossFeedingExplorerEvents();
  const nameOf=(id)=>{const sp=store.species.find(s=>s.id===id);return store.models.get(sp?.file)?.cell.name??id;};
  if(!x.enabled){
    const n=(store.crossFeeding?.links??[]).filter(l=>l.from&&l.to).length;
    return `<section class="card"><h2>Cross-Feeding Explorer</h2><p class="hint">${n} active producer→consumer event(s).</p><button class="grow" data-action="open-crossfeeding-explorer">Explore cross-feeding</button>${crossFeedingMetabolitePicker(store)}</section>`;
  }
  const option=(v,label,selected)=>`<option value="${esc(v)}"${v===selected?' selected':''}>${esc(label)}</option>`;
  const names=exchangeNames(store);
  const mets=(idx?.metabolites??[]).map(v=>option(v,crossFeedingLabel(names,v),x.metabolite)).join('');
  const species=(idx?.species??[]).map(v=>option(v,nameOf(v),x.producer)).join('');
  const selected=idx?.events.find(e=>e.id===x.selectedId);
  return `<section class="card crossfeed-explorer"><div class="row"><h2 class="grow">Cross-Feeding Explorer</h2><button data-action="close-crossfeeding-explorer" title="Close" aria-label="Close explorer">×</button></div>
    <p class="hint crossfeed-event-count"><strong>${visible.length}</strong> of ${idx?.events.length??0} active events</p>
    <input class="grow" data-action="crossfeeding-query" value="${esc(x.query)}" placeholder="Search metabolite or species">
    <div class="crossfeed-filters">
      <select data-action="crossfeeding-metabolite"><option value="">All metabolites</option>${mets}</select>
      <select data-action="crossfeeding-producer"><option value="">All producers</option>${species}</select>
      <select data-action="crossfeeding-consumer"><option value="">All consumers</option>${(idx?.species??[]).map(v=>option(v,nameOf(v),x.consumer)).join('')}</select>
      <label>Min flux <input type="number" min="0" step="0.001" data-action="crossfeeding-min-flux" value="${esc(x.minFlux)}"></label>
    </div>
    <div class="row"><button data-action="clear-crossfeeding-filters">Clear filters</button><span class="grow"></span><span class="hint">mmol·gDW community⁻¹·h⁻¹</span></div>
    ${selected?`<div class="crossfeed-selected"><b>${esc(nameOf(selected.from))} → ${esc(crossFeedingLabel(names, selected.base))} → ${esc(nameOf(selected.to))}</b><small>Producer transfer: ${esc(selected.producerTransfer??'—')} · Consumer transfer: ${esc(selected.consumerTransfer??'—')}</small></div>`:''}
    <div class="crossfeed-events">${renderCrossFeedingEvents(store)}</div>
    ${crossFeedingMetabolitePicker(store)}
  </section>`;
}

/** Update filtered events without replacing the search field while it is edited. */
export function renderCrossFeedingEvents(store) {
  const explorer = store.crossFeedingExplorer;
  const events = store.crossFeedingExplorerEvents();
  const nameOf = (id) => {
    const species = store.species.find((item) => item.id === id);
    return store.models.get(species?.file)?.cell.name ?? id;
  };
  const names = exchangeNames(store);
  const rows = events.slice(0, 50).map((event) =>
    `<button class="crossfeed-event${event.id === explorer.selectedId ? ' is-selected' : ''}" data-action="select-crossfeeding-event" data-crossfeed-id="${esc(event.id)}">
      <span><b>${esc(nameOf(event.from))}</b> → <b>${esc(nameOf(event.to))}</b></span>
      <span class="crossfeed-met">${esc(crossFeedingLabel(names, event.base))}</span><strong>${formatFlux(event.flux, 3)}</strong>
    </button>`
  ).join('');
  return (rows || '<p class="empty">No events match these filters.</p>')
    + (events.length > 50 ? '<p class="hint">Showing first 50 events.</p>' : '');
}

// Legend

function legendCard(store) {
  const seen = new Map();
  for (const cell of store.layout?.cells ?? []) {
    for (const comp of cell.compartments) {
      if (!seen.has(comp.id)) seen.set(comp.id, comp.label);
    }
  }
  seen.set('s', 'Shared medium');

  const items = [...seen].map(([id, label]) => {
    const c = compartmentColors(id, store.view.dark);
    return `<div class="legend-item">
      <span class="legend-swatch" style="background:${c.fill};border-color:${c.stroke}"></span>
      ${esc(label)}</div>`;
  }).join('');

  const reactionMode = store.view.reactionColorMode ?? 'flux';
  const ribbonMode = store.view.pathwayRibbonMode ?? 'off';
  const mode = ribbonMode !== 'off' ? ribbonMode : reactionMode;
  const coloring = ribbonMode !== 'off'
    ? (store.pathwayRibbonColoring?.({ drawnOnly: true }) ?? { legend: [] })
    : (store.reactionColoring?.({ drawnOnly: true }) ?? { legend: [] });
  const categorical = mode !== 'flux';
  const groupTitle = mode === 'pathway' ? 'Pathways' : 'Subsystems';
  const shownGroups = coloring.legend.slice(0, 18);
  const extraGroups = Math.max(0, coloring.legend.length - shownGroups.length);
  const categoryItems = shownGroups.map((item) => `<div class="reaction-category-item">
      <span class="reaction-category-swatch" style="background:${item.color}"></span>
      <span title="${esc(item.label)}">${esc(item.label)}</span>
      <b>${item.count}</b>
    </div>`).join('');

  const customFluxColors = reactionMode === 'flux' && Array.isArray(store.view.fluxColorStops) && store.view.fluxColorStops.length > 0;
  // Categorical arrow colors do not use the saved signed palette. Their
  // neutral magnitude guide must continue to start at zero.
  const stops = !store.scale ? [] : (categorical ? Array.from({ length: 7 }, (_, index) => {
    const t = index / 6;
    return { t, value: t === 0 ? 0 : Math.expm1(t * Math.log(10)) / 9 * store.scale.reference };
  }) : store.scale.legendStops(7));
  const bars = stops.map((s) => `<div style="background:${categorical ? (store.view.dark ? '#94a3b8' : '#64748b') : s.color};height:${6 + s.t * 26}px"></div>`).join('');
  // SteadyCom solves for abundance × flux; the map shows each species per gram of
  // itself, and the cross-feeding lines per gram of the whole community.
  const steadyCom = Boolean(store.solution?.steadyCom);
  const ticks = stops.length
    ? `<div class="flux-ticks"><span>0</span><span>|flux| mmol·gDW⁻¹·h⁻¹${steadyCom ? ' of each species' : ''}</span><span>${formatFlux(stops[stops.length - 1].value, 2)}</span></div>`
    : '';
  const units = steadyCom ? `<div class="legend-units">
      <strong>Units</strong>
      <p><b>Reaction arrows</b>: mmol·gDW⁻¹·h⁻¹ <i>of that species</i>.</p>
      ${store.crossFeedingHighlighted() ? `<p><b>Cross-feeding lines</b>: mmol·gDW⁻¹·h⁻¹ <i>of the whole community</i> (abundance × flux).</p>` : ''}
    </div>` : '';

  return `<section class="card">
    <h2>Legend</h2>
    <div class="legend-grid">${items}</div>
    ${categorical ? `<div class="reaction-category-legend">
      <div class="reaction-category-head"><strong>${groupTitle}</strong><span>${coloring.legend.length}</span></div>
      ${categoryItems || '<p class="empty">No drawn reactions.</p>'}
      ${extraGroups ? `<p class="hint">+${extraGroups} more</p>` : ''}
    </div>` : ''}
    ${customFluxColors ? `<div class="flux-color-legend" data-flux-color-preview>${renderFluxColorPreview(store)}</div>` : `<div class="flux-scale" style="margin-top:12px">${bars}</div>${ticks}`}
    ${store.crossFeedingHighlighted() ? crossFeedingLegend(store) : ''}
    ${units}
  </section>`;
}

// Display

/** Preview the palette used by the map, never the editor's uncommitted fields. */
export function renderFluxColorPreview(store) {
  const custom = Array.isArray(store.view.fluxColorStops) && store.view.fluxColorStops.length > 0;
  const stops = custom
    ? [...store.view.fluxColorStops].sort((a, b) => a.value - b.value)
    : (store.scale?.legendStops(7) ?? []);
  if (!stops.length) return '<p class="hint">The scale appears after an analysis.</p>';
  const first = stops[0].value;
  const last = stops[stops.length - 1].value;
  const position = (stop) => {
    if (!custom) return Math.max(0, Math.min(100, Number(stop.t ?? 0) * 100));
    if (first === last) return 50;
    if (stop.value === first) return 0;
    if (stop.value === last) return 100;
    const range = last - first;
    const fraction = Number.isFinite(range)
      ? (stop.value - first) / range
      : (stop.value / 2 - first / 2) / (last / 2 - first / 2);
    return Math.max(0, Math.min(100, fraction * 100));
  };
  const gradient = stops.length === 1
    ? `${stops[0].color}, ${stops[0].color}`
    : stops.map((stop) => `${stop.color} ${position(stop)}%`).join(', ');
  const marks = stops.map((stop) => `<span class="flux-color-mark" style="left:${position(stop)}%" title="${esc(formatFlux(stop.value, 6))}" aria-hidden="true"></span>`).join('');
  const label = custom ? 'Flux' : '|flux|';
  return `<div class="flux-color-gradient" role="img" aria-label="${esc(`${custom ? 'Signed flux' : 'Automatic flux magnitude'} color gradient, ${formatFlux(first, 6)} to ${formatFlux(last, 6)}`)}" style="background:linear-gradient(to right in srgb, ${esc(gradient)})">${marks}</div>
    <div class="flux-color-range"><span>${esc(formatFlux(first, 6))}</span><span>${label} mmol·gDW⁻¹·h⁻¹</span>${stops.length > 1 ? `<span>${esc(formatFlux(last, 6))}</span>` : ''}</div>`;
}

function fluxColorEditor(store, draft) {
  const custom = Array.isArray(store.view.fluxColorStops) && store.view.fluxColorStops.length > 0;
  const omicsActive = Boolean(
    (store.omicsDataset && store.omicsSettings?.enabled && store.omicsSettings?.conditionA && store.community)
    || (store.omics?.enabled && store.omics?.dataset)
  );
  const rows = draft?.rows ?? (store.view.fluxColorStops ?? []).map((stop) => ({ value: String(stop.value), color: stop.color }));
  const invalid = new Set(draft?.invalid ?? []);
  return `<div class="flux-color-editor">
    <div class="flux-color-editor-head"><strong>Flux palette</strong><span>${custom ? `${rows.length} point${rows.length === 1 ? '' : 's'}` : 'Automatic'}</span></div>
    <div class="flux-color-mode-buttons" role="group" aria-label="Flux color palette">
      <button data-action="flux-colors-reset" type="button" aria-pressed="${!custom}">Automatic</button>
      <button data-action="flux-colors-customize" type="button" aria-pressed="${custom}">Custom colors</button>
    </div>
    ${omicsActive ? '<p class="hint flux-color-omics-hint">An omics overlay is active and hides flux colors.</p>' : ''}
    <div data-flux-color-preview>${renderFluxColorPreview(store)}</div>
    ${custom ? `<div class="flux-color-columns" aria-hidden="true"><span>Color</span><span>Flux value</span><span></span></div>
      <div class="flux-color-stops">${rows.map((stop, index) => `<div class="flux-color-stop" data-flux-color-stop="${index}">
        <input type="color" value="${esc(stop.color)}" data-action="flux-color-color" data-flux-color-index="${index}" aria-label="Color at flux point ${index + 1}">
        <input type="number" step="any" value="${esc(stop.value)}" data-action="flux-color-value" data-flux-color-index="${index}" aria-label="Flux value at point ${index + 1}"${invalid.has(index) ? ' aria-invalid="true"' : ''}>
        <button data-action="flux-color-remove" data-flux-color-index="${index}" type="button" aria-label="Remove flux point ${index + 1}" title="Remove point"${rows.length <= 1 ? ' disabled' : ''}>×</button>
      </div>`).join('')}</div>
      <p class="flux-color-error" data-flux-color-error role="alert"${draft?.error ? '' : ' hidden'}>${esc(draft?.error ?? '')}</p>
      <button class="flux-color-add" data-action="flux-color-add" type="button">+ Add color point</button>
      <p class="hint">Each point needs a unique value.</p>`
      : ''}
  </div>`;
}

/** A switch in the map appearance card: one way of drawing the lines, on or off. */
function lineToggle(action, checked, title) {
  return `<label class="row map-line-toggle">
      <input type="checkbox" data-action="${action}"${checked ? ' checked' : ''}>
      <span class="grow"><strong>${esc(title)}</strong></span>
    </label>`;
}

/** The colours metabolite circles take on this map unless one has its own. */
function nodeStyleControls(store) {
  const style = store.map?.nodeStyle ?? {};
  const theme = defaultNodeColors(store.view?.dark);
  return `<h3 class="map-appearance-subtitle">Metabolite circles</h3>
    <div class="map-color-row">
      <label>Fill <input type="color" data-action="node-style-fill" value="${esc(style.fill ?? theme.fill)}" aria-label="Fill of metabolite circles"></label>
      <label>Outline <input type="color" data-action="node-style-stroke" value="${esc(style.stroke ?? theme.stroke)}" aria-label="Outline of metabolite circles"></label>
      <button data-action="node-style-reset" type="button"${style.fill || style.stroke ? '' : ' disabled'}>Theme colors</button>
    </div>`;
}

/** The circle colours of the theme, as the renderer draws them. */
export function defaultNodeColors(dark) {
  return dark ? { fill: '#f59e0b', stroke: '#7c2d12' } : { fill: '#e67e4c', stroke: '#a64613' };
}

function mapAppearanceCard(store, fluxColorDraft = null) {
  const view = store.view;
  const byFlux = (view.reactionColorMode ?? 'flux') === 'flux';
  return `<section class="card map-appearance-card">
    <h2>Lines, circles & ribbons</h2>
    ${lineToggle('color-by-flux', byFlux, 'Color lines by flux')}
    ${byFlux ? fluxColorEditor(store, fluxColorDraft) : `<label class="view-label-control">Color lines by
      <select data-action="reaction-color-mode" aria-label="Reaction color mode">
        <option value="pathway"${view.reactionColorMode === 'pathway' ? ' selected' : ''}>Pathway</option>
        <option value="subsystem"${view.reactionColorMode === 'subsystem' ? ' selected' : ''}>Subsystem</option>
      </select>
    </label>`}
    ${lineToggle('width-by-flux', view.widthByFlux, 'Line width follows flux')}
    ${lineToggle('dash-no-flux', view.dashNoFlux !== false, 'Dash lines with no flux')}
    ${lineToggle('line-hops', view.lineHops !== false, 'Gap where lines cross')}
    ${nodeStyleControls(store)}
    <label class="view-label-control">Pathway ribbons
      <select data-action="pathway-ribbon-mode" aria-label="Pathway ribbon mode">
        <option value="off"${view.pathwayRibbonMode === 'off' ? ' selected' : ''}>Off</option>
        <option value="subsystem"${view.pathwayRibbonMode === 'subsystem' ? ' selected' : ''}>Subsystem</option>
        <option value="pathway"${view.pathwayRibbonMode === 'pathway' ? ' selected' : ''}>Pathway</option>
      </select>
    </label>
    ${view.pathwayRibbonMode !== 'off' ? `<div class="pathway-ribbon-controls">
      <label class="pathway-ribbon-slider">Width
        <input class="grow" type="range" min="20" max="120" step="2" data-action="pathway-ribbon-width" value="${esc(view.pathwayRibbonWidth ?? 56)}" aria-label="Pathway ribbon width">
        <span>${esc(Math.round(view.pathwayRibbonWidth ?? 56))} px</span>
      </label>
      <label class="pathway-ribbon-slider">Opacity
        <input class="grow" type="range" min="0.04" max="0.45" step="0.01" data-action="pathway-ribbon-opacity" value="${esc(view.pathwayRibbonOpacity ?? 0.18)}" aria-label="Pathway ribbon opacity">
        <span>${esc(Math.round((view.pathwayRibbonOpacity ?? 0.18) * 100))}%</span>
      </label>
    </div>` : ''}
  </section>`;
}

function displayCard(store, { workspace = false } = {}) {
  const v = store.view;
  const toggle = (action, on, label, title) =>
    `<button data-action="${action}" aria-pressed="${on}" title="${esc(title)}">${esc(label)}</button>`;

  if (workspace) return `<section class="card selection-options-card">
    <h2>Selection & geometry</h2>
    <div class="toggle-list">
      ${toggle('toggle-geometry-edit', v.geometryEdit, 'Edit geometry', 'Move and resize cells and compartments')}
      ${toggle('toggle-multi-select', v.multiSelect, 'Multi-select', 'Select and move several metabolites and reactions')}
    </div>
    ${selectionColorBlock(store)}
  </section>`;

  return `<section class="card">
    <h2>View</h2>
    ${v.geometryEdit ? '<p class="hint geometry-edit-hint"><strong>Geometry editor on.</strong> Click a membrane or compartment.</p>' : ''}
    ${v.multiSelect ? `<p class="hint"><strong>Multi-select on.</strong> Selected: ${store.multiSelectionSize}.</p>` : ''}
    ${v.textTool ? '<p class="hint"><strong>Text tool on.</strong> Click the map, type, and press Enter.</p>' : ''}
    <div class="toggle-list">
      ${toggle('toggle-currency', v.showCurrency, 'Cofactors', 'Show secondary metabolites (ATP, NADH, H⁺…)')}
      ${toggle('toggle-quiet-cofactors', store.map?.cofactorStyle === 'quiet', 'Quiet cofactors', 'Draw secondary metabolites small and light')}
      ${toggle('toggle-crossfeed', v.showCrossFeeding, 'Highlight cross-feeding', 'Dim the network and highlight transfers between cells')}
      ${toggle('toggle-dark', v.dark, 'Dark', 'Switch the colour scheme')}
      ${toggle('toggle-geometry-edit', v.geometryEdit, 'Edit geometry', 'Move and resize cells and compartments')}
      ${toggle('toggle-multi-select', v.multiSelect, 'Multi-select', 'Select and move several metabolites and reactions')}
      ${toggle('toggle-text-tool', v.textTool, 'Text', 'Add free text to the map')}
    </div>

    ${selectionColorBlock(store)}
    ${store.hiddenMetaboliteCount > 0 ? `<div class="row" style="margin-top:10px">
      <button class="grow" data-action="show-hidden-metabolites">Show hidden metabolites (${store.hiddenMetaboliteCount})</button>
    </div>` : ''}
    <div class="row reaction-color-mode-row" style="margin-top:10px">
      <label style="width:110px">Reaction color</label>
      <select class="grow" data-action="reaction-color-mode">
        <option value="flux"${v.reactionColorMode === 'flux' ? ' selected' : ''}>Flux</option>
        <option value="subsystem"${v.reactionColorMode === 'subsystem' ? ' selected' : ''}>Subsystem</option>
        <option value="pathway"${v.reactionColorMode === 'pathway' ? ' selected' : ''}>Pathway</option>
      </select>
    </div>

    <div class="pathway-ribbon-controls">
      <div class="row" style="margin-top:10px">
        <label style="width:110px">Pathway ribbons</label>
        <select class="grow" data-action="pathway-ribbon-mode">
          <option value="off"${v.pathwayRibbonMode === 'off' ? ' selected' : ''}>Off</option>
          <option value="subsystem"${v.pathwayRibbonMode === 'subsystem' ? ' selected' : ''}>Subsystem</option>
          <option value="pathway"${v.pathwayRibbonMode === 'pathway' ? ' selected' : ''}>Pathway</option>
        </select>
      </div>
      ${v.pathwayRibbonMode !== 'off' ? `
      <div class="pathway-ribbon-slider">
        <label>Width</label>
        <input class="grow" type="range" min="20" max="120" step="2" data-action="pathway-ribbon-width" value="${esc(v.pathwayRibbonWidth ?? 56)}">
        <span>${esc(Math.round(v.pathwayRibbonWidth ?? 56))} px</span>
      </div>
      <div class="pathway-ribbon-slider">
        <label>Opacity</label>
        <input class="grow" type="range" min="0.04" max="0.45" step="0.01" data-action="pathway-ribbon-opacity" value="${esc(v.pathwayRibbonOpacity ?? 0.18)}">
        <span>${esc(Math.round((v.pathwayRibbonOpacity ?? 0.18) * 100))}%</span>
      </div>` : ''}
    </div>

    <div class="row" style="margin-top:10px">
      <label style="width:74px">Labels</label>
      <select class="grow" data-action="labels">
        <option value="auto"${v.labels === 'auto' ? ' selected' : ''}>Major fluxes</option>
        <option value="active"${v.labels === 'active' ? ' selected' : ''}>All active</option>
        <option value="all"${v.labels === 'all' ? ' selected' : ''}>All</option>
        <option value="none"${v.labels === 'none' ? ' selected' : ''}>None</option>
      </select>
    </div>
    <div class="row" style="margin-top:10px">
      <button class="grow" data-action="map-undo"${store.canUndoMap ? '' : ' disabled'} title="Ctrl+Z">↶ Undo</button>
      <button class="grow" data-action="map-redo"${store.canRedoMap ? '' : ' disabled'} title="Ctrl+Y / Ctrl+Shift+Z">↷ Redo</button>
    </div>
    <div class="row">
      <button class="grow" data-action="map-empty">Empty map</button>
      <button class="grow" data-action="map-full">Full map</button>
    </div>
  </section>`;
}

/**
 * Align and distribute for a selection. With reactions in it, Horizontal and
 * Vertical work as in Escher: each reaction comes with its metabolites, all on
 * one line; one selected reaction is enough for that.
 */
function arrangeSelectionBlock(store) {
  const size = store.multiSelectionSize;
  const withReactions = Boolean(store.map?.selectionHasLineReactions?.(store.multiSelection));
  const few = size < 2 ? ' disabled' : '';
  const lineTitle = (axis, same) => (withReactions
    ? `Put the selected reactions and their metabolites on one ${axis} line`
    : `Same ${same}: a ${axis} line`);
  return `<div class="selection-arrange-grid">
    <button data-action="align-horizontal" title="${lineTitle('horizontal', 'y')}">Horizontal</button>
    <button data-action="align-vertical" title="${lineTitle('vertical', 'x')}">Vertical</button>
    <button data-action="align-left"${few} title="Align left edges">Left</button>
    <button data-action="align-right"${few} title="Align right edges">Right</button>
    <button data-action="align-top"${few} title="Align top edges">Top</button>
    <button data-action="align-bottom"${few} title="Align bottom edges">Bottom</button>
    <button data-action="distribute-horizontal" ${size < 3 ? 'disabled' : ''} title="Space centres evenly, horizontally">Distribute ↔</button>
    <button data-action="distribute-vertical" ${size < 3 ? 'disabled' : ''} title="Space centres evenly, vertically">Distribute ↕</button>
  </div>`;
}

/** Colour what is selected: the circles of its metabolites, the lines of its reactions. */
function selectionColorBlock(store) {
  const nodes = [...(store.multiSelection?.nodes ?? [])].filter((id) =>
    ['metabolite', 'medium', 'currency'].includes(store.map?.nodes?.get?.(id)?.kind));
  const reactions = [...(store.multiSelection?.reactions ?? [])].filter((id) => store.map?.reactions?.has?.(id));
  if (!nodes.length && !reactions.length) return '';
  return `<div class="selection-colors">
    <div class="selection-arrange-head"><strong>Color selection</strong><span>${nodes.length} metabolite${nodes.length === 1 ? '' : 's'} · ${reactions.length} reaction${reactions.length === 1 ? '' : 's'}</span></div>
    ${nodes.length ? nodeColorRow(store, nodes) : ''}
    ${reactions.length ? reactionColorRow(store, reactions) : ''}
  </div>`;
}

function mapCanvasCard(store) {
  const width = Math.round(store.map?.width ?? 0);
  const height = Math.round(store.map?.height ?? 0);
  return `<section class="card map-canvas-card">
    <h2>Canvas</h2>
    <dl class="overview-list"><div><dt>Dimensions</dt><dd>${width} × ${height}</dd></div></dl>
    <p class="hint">Drag the canvas edge or corner to resize it.</p>
    <div class="row">
      <button class="grow" data-action="map-empty" type="button">Clear drawing</button>
      <button class="grow" data-action="map-full" type="button">Draw full model</button>
    </div>
    <p class="hint">Replaces the drawing; Undo restores it.</p>
  </section>`;
}

const clamp = (v) => Math.min(100, Math.max(0, v));

export { esc };
