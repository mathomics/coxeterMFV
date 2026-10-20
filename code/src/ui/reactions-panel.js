/**
 * The Reactions page and its compact twin in the results dock, as HTML.
 *
 * The page is painted in regions (data-rx-region) so the app can repaint the
 * parts that changed and leave alone the one being typed in. The table body is
 * a window over the rows (see virtualWindow): a genome-scale community has
 * eleven thousand of them, and only the visible few are ever in the DOM.
 */

import { formatFlux } from '../render/theme.js';
import { OBJECTIVES } from '../core/community.js';
import {
  REACTION_FILTERS, REACTION_GROUPINGS, REACTION_TYPES, reactionTypeLabel,
  signLogPosition, signLogSteps, virtualWindow,
} from './reactions-explorer.js';

const esc = (s) => String(s ?? '')
  .replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
  .replace(/"/g, '&quot;');

/** Row heights in px, fixed so the window arithmetic is exact. */
export const RX_ROW_HEIGHT = { page: 38, compact: 28 };

const STATUS_LABELS = {
  culprit: 'Infeasible culprit', 'knocked-out': 'Knocked out', modified: 'Modified',
  blocked: 'Blocked', original: 'Original',
};

// Widths are minimums: a wider page shares the extra out. The tick box and the
// name stay put at the left and the actions at the right while the rest scrolls.
const PAGE_COLUMNS = [
  { id: 'select', width: 32 },
  { id: 'reaction', width: 190, label: 'Reaction', sort: 'reaction' },
  { id: 'species', width: 82, label: 'Species', sort: 'species' },
  { id: 'compartment', width: 100, label: 'Compartment', sort: 'compartment' },
  { id: 'subsystem', width: 112, label: 'Subsystem', sort: 'subsystem' },
  { id: 'type', width: 108, label: 'Type', sort: 'type' },
  { id: 'lower', width: 150, label: 'Lower bound', sort: 'lower' },
  { id: 'upper', width: 150, label: 'Upper bound', sort: 'upper' },
  { id: 'flux', width: 76, label: 'Flux', sort: 'flux' },
  { id: 'fva', width: 132, label: 'FVA min–max', sort: 'fva', when: 'fva' },
  { id: 'rc', width: 88, label: 'Reduced cost', sort: 'rc', when: 'rc' },
  { id: 'objective', width: 70, label: 'Objective', sort: 'objective' },
  { id: 'status', width: 110, label: 'Status', sort: 'status' },
  { id: 'actions', width: 114 },
];

const COMPACT_COLUMNS = [
  { id: 'reaction', width: 200, label: 'Reaction', sort: 'reaction' },
  { id: 'species', width: 80, label: 'Species', sort: 'species' },
  { id: 'type', width: 112, label: 'Type', sort: 'type' },
  { id: 'lower', width: 84, label: 'Lower', sort: 'lower' },
  { id: 'upper', width: 84, label: 'Upper', sort: 'upper' },
  { id: 'flux', width: 84, label: 'Flux', sort: 'flux' },
  { id: 'fva', width: 136, label: 'FVA min–max', sort: 'fva', when: 'fva' },
  { id: 'rc', width: 92, label: 'Reduced cost', sort: 'rc', when: 'rc' },
  { id: 'status', width: 112, label: 'Status', sort: 'status' },
  { id: 'actions', width: 80 },
];

/** The columns a table shows for a model: FVA and reduced costs only when there is something in them. */
export function reactionColumns(model, mode = 'page') {
  return (mode === 'compact' ? COMPACT_COLUMNS : PAGE_COLUMNS).filter((column) => !column.when || model?.columns?.[column.when]);
}

// Shared bound control

/** A bound as the number box shows it: exact, without float noise. */
export function formatBound(value) {
  if (!Number.isFinite(value)) return '';
  return String(Number(value.toPrecision(10)));
}

/**
 * One bound as a sign-log slider and an exact number box (see SIGN_LOG). The
 * Inspector and the Reactions page share it, so both edit bounds the same way.
 *
 * @param {object} spec
 * @param {string} spec.rid reaction id (data-rxn)
 * @param {'lower'|'upper'} spec.side
 * @param {number} spec.value the bound in force
 * @param {boolean} [spec.disabled]
 * @param {string} [spec.label] what the bound belongs to, for screen readers
 */
export function boundControl({ rid, side, value, disabled = false, label = '' }) {
  const steps = signLogSteps();
  const action = side === 'lower' ? 'bound-lower' : 'bound-upper';
  const name = `${side === 'lower' ? 'Lower' : 'Upper'} bound${label ? ` of ${label}` : ''}`;
  const off = disabled ? ' disabled' : '';
  return `<span class="bound-control">
    <input type="range" min="${-steps}" max="${steps}" step="1" value="${signLogPosition(value)}"
      data-action="${action}" data-scale="signlog" data-rxn="${esc(rid)}"
      aria-label="${esc(name)}, logarithmic slider" title="${esc(formatBound(value))} (log scale; the middle is 0)"${off}>
    <input type="number" step="any" value="${esc(formatBound(value))}"
      data-action="${action}-number" data-rxn="${esc(rid)}" aria-label="${esc(name)}"${off}>
  </span>`;
}

// Page

/**
 * The whole Reactions page.
 *
 * @param {object} store
 * @param {object} [view] what the app keeps between paints: `model`
 *   (store.reactionTableModel(), computed when absent), `scrollTop`,
 *   `viewportHeight`, `showChanges`, `split` and the bulk bar's draft `bulk`
 */
export function renderReactionsPage(store, view = {}) {
  const model = view.model ?? store.reactionTableModel();
  const state = { ...view, model };
  const region = (name, tag = 'div', extra = '') =>
    `<${tag} class="rx-region rx-${name}" data-rx-region="${name}"${extra}>${renderReactionsRegion(name, store, state)}</${tag}>`;
  const columns = reactionColumns(model, 'page');
  return `<div class="rx-page" data-rx-page>
    <div class="rx-toolbar">
      ${region('search')}
      ${region('filters')}
      ${region('count')}
      ${region('actions')}
    </div>
    ${region('objective')}
    <div class="rx-gene-line">${region('gene-search')}${region('genes')}</div>
    ${region('alert')}
    ${region('bulk')}
    <div class="rx-main">
      <div class="rx-scroll" data-rx-scroll="page" tabindex="-1">
        <table class="rx-table" style="min-width:${columns.reduce((sum, column) => sum + column.width, 0)}px">
          ${colgroup(columns)}
          <thead data-rx-region="head">${renderReactionsRegion('head', store, state)}</thead>
          <tbody data-rx-rows="page">${renderReactionRows(store, state, 'page')}</tbody>
        </table>
      </div>
      <aside class="rx-region rx-changes" data-rx-region="changes"${view.showChanges ? '' : ' hidden'} aria-label="Changed reactions">${renderReactionsRegion('changes', store, state)}</aside>
    </div>
  </div>`;
}

function colgroup(columns) {
  return `<colgroup>${columns.map((column) => `<col class="rx-col-${column.id}" style="width:${column.width}px">`).join('')}</colgroup>`;
}

/** The inner HTML of one page region: search, filters, count, actions, objective, gene-search, genes, alert, bulk, head or changes. */
export function renderReactionsRegion(name, store, view) {
  const model = view.model;
  const table = store.reactionTable;
  const locked = Boolean(store.isZunigaStudy);
  switch (name) {
    case 'search': return searchRegion(table);
    case 'filters': return filtersRegion(table);
    case 'count': return `<strong>${model.shown}</strong> of ${model.total} reactions${model.modified ? ` · <span class="rx-count-modified">${model.modified} modified</span>` : ''}`;
    case 'actions': return actionsRegion(store, view, locked);
    case 'objective': return objectiveRegion(store, locked);
    case 'gene-search': return geneSearchRegion(store, locked);
    case 'genes': return genesRegion(store, locked);
    case 'alert': return alertRegion(store, locked);
    case 'bulk': return locked || !table.selection.size ? '' : bulkRegion(store, view);
    case 'head': return headRow(store, model, 'page');
    case 'changes': return changesRegion(store, locked);
    default: return '';
  }
}

// The search box has a region of its own: repainting the filter buttons after
// typing must not take the box away, nor the box keep a button from repainting.
function searchRegion(table) {
  const groups = REACTION_GROUPINGS.map((group) =>
    `<option value="${group.id}"${table.groupBy === group.id ? ' selected' : ''}>${esc(group.label)}</option>`).join('');
  return `<input type="search" class="rx-search-input" data-action="rx-search" value="${esc(table.query)}"
      placeholder="Search id, name, gene, metabolite, EC…" aria-label="Search reactions" autocomplete="off" spellcheck="false">
    <label class="rx-group-by">Group by <select data-action="rx-group-by" aria-label="Group reactions by">${groups}</select></label>`;
}

function filtersRegion(table) {
  const chips = REACTION_FILTERS.map((filter) => {
    const on = table.filters.includes(filter.id);
    return `<button type="button" class="rx-chip" data-action="rx-filter" data-filter="${filter.id}" aria-pressed="${on}" title="${esc(filter.title)}">${esc(filter.label)}</button>`;
  }).join('');
  return `<div class="rx-chips" role="group" aria-label="Quick filters">${chips}</div>`;
}

function actionsRegion(store, view, locked) {
  const history = store.constraintHistory;
  const changes = view.changes ?? store.constraintChanges();
  const listed = changes.filter((change) => !change.genes?.length).length + (store.geneKnockoutChanges?.().length ?? 0);
  const off = locked ? ' disabled' : '';
  return `<button type="button" data-action="rx-undo"${history.canUndo && !locked ? ` title="Undo: ${esc(history.undoLabel)}"` : ' disabled title="No constraint edit to undo"'}>↶ Undo</button>
    <button type="button" data-action="rx-redo"${history.canRedo && !locked ? ` title="Redo: ${esc(history.redoLabel)}"` : ' disabled title="No constraint edit to redo"'}>↷ Redo</button>
    <button type="button" data-action="rx-changes-toggle" aria-pressed="${Boolean(view.showChanges)}" title="Every reaction whose bounds differ from the model's, and every gene knocked out">Changes (${listed + (store.customObjective ? 1 : 0)})</button>
    <button type="button" data-action="rx-import-csv" title="Apply a CSV of id, lower, upper"${off}>Import CSV</button>
    <button type="button" data-action="rx-export-csv" title="Every reaction's bounds in force, as CSV">Export CSV</button>
    <button type="button" data-action="rx-split-toggle" aria-pressed="${Boolean(view.split)}" title="Show the map above the table">Map</button>`;
}

function objectiveRegion(store, locked) {
  const coefficients = store.objectiveCoefficients?.() ?? {};
  const entries = Object.entries(coefficients);
  const sense = store.objectiveSense?.() ?? 'max';
  const custom = Boolean(store.customObjective);
  const shown = entries.slice(0, 6).map(([id, coefficient]) => `<span class="rx-term">
      <b>${esc(formatBound(coefficient))}</b> ×
      <button type="button" class="rx-term-name" data-action="rx-open" data-rid="${esc(id)}">${esc(store.reactionTitle?.(id) ?? id)}</button>
      ${locked ? '' : `<button type="button" class="rx-term-remove" data-action="rx-objective-remove" data-rid="${esc(id)}" aria-label="Take ${esc(id)} out of the objective">×</button>`}
    </span>`).join('');
  const more = entries.length > 6 ? `<span class="rx-term-more">+ ${entries.length - 6} more</span>` : '';
  if (locked) {
    return `<div class="rx-objective-line"><span class="rx-objective-title">Objective</span>
      <span class="rx-terms">${shown || '—'}${more}</span>
      <small>The authors’ objective, medium and bounds are fixed for this case.</small></div>`;
  }
  const bases = [...(store.mediumReactions?.().byBase?.keys?.() ?? [])].sort((a, b) => a.localeCompare(b));
  const product = !custom && store.objective === OBJECTIVES.PRODUCT ? store.productTarget : '';
  const preset = (id, label, title) => `<button type="button" data-action="rx-objective-preset" data-preset="${id}"
      aria-pressed="${!custom && store.objective === id}" title="${esc(title)}">${label}</button>`;
  return `<div class="rx-objective-line">
    <span class="rx-objective-title" title="Type a coefficient in any row’s Objective column to add that reaction; 0 takes it out">Objective</span>
    <select data-action="rx-objective-sense" aria-label="Maximise or minimise the objective">
      <option value="max"${sense === 'max' ? ' selected' : ''}>Maximise</option>
      <option value="min"${sense === 'min' ? ' selected' : ''}>Minimise</option>
    </select>
    <span class="rx-terms">${shown || '<em>no reaction: any feasible flux</em>'}${more}<span class="rx-term-help">add any reaction from its Objective column</span></span>
    ${custom ? '<span class="rx-custom-tag" title="Edited here, on top of the preset">Edited</span>' : ''}
    <span class="rx-presets"><span>Presets</span>
      ${preset(OBJECTIVES.EQUAL_GROWTH, 'Equal growth', 'Every species grows at one shared rate')}
      ${preset(OBJECTIVES.SUM_BIOMASS, 'Biomass sum', 'The abundance-weighted sum of the biomass reactions')}
      <select data-action="rx-objective-product" aria-label="Maximise the export of a product">
        <option value="">Product…</option>
        ${bases.map((base) => `<option value="${esc(base)}"${product === base ? ' selected' : ''}>${esc(base)}</option>`).join('')}
      </select>
    </span>
    ${custom ? '<button type="button" data-action="rx-objective-reset">Back to preset</button>' : ''}
  </div>`;
}

// The gene search has a region of its own for the same reason as the reaction search.
function geneSearchRegion(store, locked) {
  const any = store.geneIndex().genes.size > 0;
  const placeholder = locked ? 'Fixed for this case' : any ? 'Search a gene to knock out…' : 'No gene rules in this model';
  return `<span class="rx-objective-title" title="A knocked-out gene closes every reaction whose gene rule it makes false; an isozyme left keeps a reaction open">Genes</span>
    <input type="search" class="rx-gene-input" data-action="rx-gene-search" value="${esc(store.reactionTable.geneQuery ?? '')}"
      placeholder="${placeholder}" aria-label="Search genes to knock out" autocomplete="off" spellcheck="false"${locked || !any ? ' disabled' : ''}>`;
}

/** A few reaction titles, then how many more. */
function reactionList(store, ids, shown = 4) {
  const names = ids.slice(0, shown).map((id) => store.reactionTitle(id)).join(', ');
  return ids.length > shown ? `${names} +${ids.length - shown} more` : names;
}

function genesRegion(store, locked) {
  if (locked) return '';
  const knocked = store.geneKnockoutChanges().map((gene) => `<button type="button" class="rx-term rx-gene-chip" data-action="gene-ko" data-gene="${esc(gene.key)}"
      title="Restore ${esc(gene.title)}: reopens ${esc(reactionList(store, gene.reactions, 8) || 'nothing')}">${esc(gene.title)} <span aria-hidden="true">×</span></button>`).join('');
  const query = String(store.reactionTable.geneQuery ?? '').trim();
  const results = query ? store.geneSearch(query) : [];
  const items = results.map((gene) => {
    const several = gene.speciesId && store.species.length > 1;
    const effect = gene.closes.length
      ? `${gene.knockedOut ? 'closes' : 'would close'} ${reactionList(store, gene.closes)}`
      : `${gene.knockedOut ? 'closes' : 'would close'} nothing: other genes keep its ${gene.reactions.length} reaction${gene.reactions.length === 1 ? '' : 's'} open`;
    return `<li class="rx-gene-result${gene.knockedOut ? ' is-knocked-out' : ''}">
      <code>${esc(gene.gene)}</code>${several ? `<small>${esc(gene.speciesId)}</small>` : ''}${gene.name && gene.name !== gene.gene ? `<span class="rx-sub">${esc(gene.name)}</span>` : ''}
      <span class="rx-gene-effect">${esc(effect)}</span>
      <button type="button" class="${gene.knockedOut ? '' : 'danger'}" data-action="gene-ko" data-gene="${esc(gene.key)}" aria-pressed="${gene.knockedOut}">${gene.knockedOut ? 'Restore' : 'Knock out'}</button>
    </li>`;
  }).join('');
  const list = query ? `<ul class="rx-gene-results">${items || '<li class="rx-muted">No gene matches.</li>'}</ul>` : '';
  return `${knocked ? `<span class="rx-terms">${knocked}</span>` : ''}${list}`;
}

function alertRegion(store, locked) {
  if (locked) return '<p class="rx-note">Zuñiga case: the source bounds are fixed. Choose a documented case scenario to apply its perturbation.</p>';
  const culprits = store.constraintCulprits;
  const solution = store.solution;
  if (culprits) {
    const names = [...culprits.ids].slice(0, 6).map((id) => `<button type="button" class="rx-term-name" data-action="rx-reveal" data-rid="${esc(id)}">${esc(store.reactionTitle(id))}</button>`).join(', ');
    return `<div class="rx-alert-box" role="alert">
      <span><strong>No optimum (${esc(culprits.status)})</strong> after “${esc(culprits.label)}”.${names ? ` Flagged: ${names}${culprits.ids.size > 6 ? ', …' : ''}.` : ''}</span>
      <button type="button" class="primary" data-action="rx-undo">Revert last change</button>
    </div>`;
  }
  if (solution && solution.status !== 'optimal') {
    return `<div class="rx-alert-box"><span><strong>No optimum (${esc(solution.status)}).</strong> ${esc(solution.message || 'Check the bounds of the flagged or modified reactions.')}</span></div>`;
  }
  if (!solution && store.community) return '<p class="rx-note">Not solved yet: run an analysis to see fluxes. Bounds can be edited now.</p>';
  return '';
}

function bulkRegion(store, view) {
  const bulk = view.bulk ?? {};
  const count = store.reactionTable.selection.size;
  const field = (name, label, placeholder) => `<input type="number" step="any" class="rx-bulk-field" data-action="rx-bulk-field" data-field="${name}"
      value="${esc(bulk[name] ?? '')}" placeholder="${placeholder}" aria-label="${esc(label)}">`;
  return `<div class="rx-bulk-bar" role="group" aria-label="Actions on the ticked reactions">
    <strong>${count} ticked</strong>
    <span class="rx-bulk-group">${field('lower', 'Lower bound for the ticked reactions', 'lower')}${field('upper', 'Upper bound for the ticked reactions', 'upper')}
      <button type="button" data-action="rx-bulk" data-bulk="set-bounds" title="An empty side is left as it is">Set bounds</button></span>
    <span class="rx-bulk-group"><span aria-hidden="true">×</span>${field('factor', 'Scale factor', '0.5')}
      <button type="button" data-action="rx-bulk" data-bulk="scale" title="Multiply the finite bounds; ±1000 (unbounded) stays">Scale</button></span>
    <button type="button" class="danger" data-action="rx-bulk" data-bulk="knockout">Knock out</button>
    <button type="button" data-action="rx-bulk" data-bulk="reset">Reset</button>
    <button type="button" data-action="rx-bulk" data-bulk="close-uptakes-except" title="Close the uptake of every exchange except the ticked ones">Close all uptakes except ticked</button>
    <button type="button" data-action="rx-select-none">Untick all</button>
  </div>`;
}

function changesRegion(store, locked) {
  // A reaction a gene knockout closes is listed under its gene.
  const changes = store.constraintChanges().filter((change) => !change.genes?.length);
  const genes = store.geneKnockoutChanges().map((gene) => `<li class="rx-change is-knocked-out is-gene">
      <span class="rx-change-name"><strong>Gene ${esc(gene.title)}</strong> <small>knocked out</small></span>
      <span class="rx-change-bounds">${gene.reactions.length ? `closes ${gene.reactions.map((id) => `<button type="button" class="rx-term-name" data-action="rx-reveal" data-rid="${esc(id)}">${esc(store.reactionTitle(id))}</button>`).join(', ')}` : 'closes no reaction on its own'}</span>
      ${locked ? '' : `<button type="button" class="rx-icon" data-action="gene-ko" data-gene="${esc(gene.key)}" title="Restore the gene">↺</button>`}
    </li>`).join('');
  const bound = (b) => `[${formatBoundText(b.lower)}, ${formatBoundText(b.upper)}]`;
  const objective = store.customObjective
    ? `<li class="rx-change is-objective"><span class="rx-change-name"><strong>Objective</strong></span>
        <span class="rx-change-bounds">${esc(store.objectiveDescription())}</span>
        ${locked ? '' : '<button type="button" class="rx-icon" data-action="rx-objective-reset" title="Back to the preset objective">↺</button>'}</li>`
    : '';
  const rows = changes.map((change) => `<li class="rx-change${change.knockedOut ? ' is-knocked-out' : ''}">
      <span class="rx-change-name"><button type="button" class="rx-term-name" data-action="rx-reveal" data-rid="${esc(change.id)}">${esc(change.label)}</button>
        ${change.speciesId ? `<small>${esc(change.speciesId)}</small>` : ''}${change.medium ? '<small title="Its uptake is the medium supply">medium</small>' : ''}</span>
      <span class="rx-change-bounds"><s title="As opened">${bound(change.original)}</s> → <b>${bound(change.current)}</b></span>
      ${locked ? '' : `<button type="button" class="rx-icon" data-action="rx-reset" data-rid="${esc(change.id)}" title="Reset to the bounds as opened">↺</button>`}
    </li>`).join('');
  return `<div class="rx-changes-head"><h3>Changes</h3>
      <button type="button" data-action="reset-all"${(changes.length || genes) && !locked ? '' : ' disabled'} title="Every reaction back to the bounds it was opened with, and every gene restored">Reset all</button></div>
    ${rows || objective || genes ? `<ol class="rx-change-list">${objective}${genes}${rows}</ol>` : '<p class="empty">No reaction differs from the bounds it was opened with.</p>'}`;
}

function headRow(store, model, mode) {
  const sort = store.reactionTable.sort;
  const visible = model.visible.map((row) => row.id);
  const allTicked = visible.length > 0 && visible.every((id) => store.reactionTable.selection.has(id));
  const cells = reactionColumns(model, mode).map((column) => {
    if (column.id === 'select') {
      return `<th scope="col" class="rx-c-select"><input type="checkbox" data-action="rx-select-visible"${allTicked ? ' checked' : ''}
        aria-label="Tick every listed reaction"${store.isZunigaStudy ? ' disabled' : ''}></th>`;
    }
    if (!column.sort) return `<th scope="col" class="rx-c-${column.id}"><span class="sr-only">Actions</span></th>`;
    const active = sort.key === column.sort;
    const order = active ? (sort.dir < 0 ? 'descending' : 'ascending') : 'none';
    return `<th scope="col" class="rx-c-${column.id}" aria-sort="${order}"><button type="button" class="rx-sort" data-action="rx-sort"
      data-sort="${column.sort}">${esc(column.label)}<span aria-hidden="true">${active ? (sort.dir < 0 ? ' ▾' : ' ▴') : ''}</span></button></th>`;
  }).join('');
  return `<tr>${cells}</tr>`;
}

// Rows

/**
 * The table body for the rows in view: a spacer, the rows of the window and a
 * spacer, so the scrollbar spans every row while few are drawn.
 *
 * @param {object} store
 * @param {object} view `model`, `scrollTop`, `viewportHeight`
 * @param {'page'|'compact'} mode
 */
export function renderReactionRows(store, view, mode = 'page') {
  const model = view.model;
  const columns = reactionColumns(model, mode);
  const span = columns.length;
  if (!model.items.length) {
    const why = model.total ? 'No reaction matches the search and filters.' : 'No model is loaded.';
    return `<tr class="rx-empty"><td colspan="${span}">${why}</td></tr>`;
  }
  const height = RX_ROW_HEIGHT[mode];
  const window = virtualWindow({
    count: model.items.length, rowHeight: height,
    scrollTop: view.scrollTop ?? 0, viewportHeight: view.viewportHeight ?? 480,
  });
  const spacer = (px) => (px > 0 ? `<tr class="rx-spacer" aria-hidden="true" style="height:${px}px"><td colspan="${span}"></td></tr>` : '');
  const context = {
    store, columns, mode, locked: Boolean(store.isZunigaStudy),
    selection: store.reactionTable.selection, selected: store.selectedReaction,
  };
  const rows = model.items.slice(window.start, window.end).map((item) => (item.kind === 'group'
    ? groupRow(item, context) : reactionRow(item.row, context))).join('');
  return spacer(window.top) + rows + spacer(window.bottom);
}

function groupRow(group, { columns, locked }) {
  const all = group.count > 0 && group.selected === group.count;
  const hasSelect = columns[0].id === 'select';
  const label = `<button type="button" class="rx-group-toggle" data-action="rx-group-toggle" data-group="${esc(group.key)}"
      aria-expanded="${!group.collapsed}"><span aria-hidden="true">${group.collapsed ? '▸' : '▾'}</span> ${esc(group.label)}</button>
    <span class="rx-group-meta">${group.count} reaction${group.count === 1 ? '' : 's'}${group.modified ? ` · <b>${group.modified} modified</b>` : ''}${group.selected ? ` · ${group.selected} ticked` : ''}</span>`;
  return `<tr class="rx-group-row" data-group="${esc(group.key)}">
    ${hasSelect ? `<td class="rx-c-select"><input type="checkbox" data-action="rx-group-select" data-group="${esc(group.key)}"${all ? ' checked' : ''}
      aria-label="Tick every reaction in ${esc(group.label)}"${locked ? ' disabled' : ''}></td>` : ''}
    <td colspan="${columns.length - (hasSelect ? 1 : 0)}">${label}</td>
  </tr>`;
}

function reactionRow(row, context) {
  const { columns, selection, selected } = context;
  const classes = ['rx-row', `is-${row.status}`];
  if (selection.has(row.id)) classes.push('is-ticked');
  if (selected === row.id) classes.push('is-current');
  const cells = columns.map((column) => reactionCell(column.id, row, context)).join('');
  return `<tr class="${classes.join(' ')}" data-rid="${esc(row.id)}">${cells}</tr>`;
}

function reactionCell(column, row, { mode, locked, selection, store }) {
  const id = esc(row.id);
  const geneClosed = Boolean(row.geneKnockouts?.length);
  const editable = !locked && !row.heldOut && !geneClosed;
  const geneNote = geneClosed ? `${geneKnockoutText(row, store)}; restore ${row.geneKnockouts.length === 1 ? 'the gene' : 'the genes'} to reopen it` : '';
  switch (column) {
    case 'select':
      return `<td class="rx-c-select"><input type="checkbox" data-action="rx-select" data-rid="${id}"${selection.has(row.id) ? ' checked' : ''}
        aria-label="Tick ${esc(row.label)}"${locked ? ' disabled' : ''}></td>`;
    case 'reaction': {
      const title = [row.label !== row.localId ? `${row.localId} — ${row.label}` : row.localId, row.name, row.equation].filter(Boolean).join('\n');
      const name = row.name && row.name !== row.label ? `<span class="rx-sub">${esc(row.name)}</span>` : '';
      const action = mode === 'compact' ? 'rx-locate' : 'rx-open';
      return `<td class="rx-c-reaction" title="${esc(title)}"><button type="button" class="rx-name" data-action="${action}" data-rid="${id}">${esc(row.label)}</button>${name}</td>`;
    }
    case 'species':
      return `<td class="rx-c-species" title="${esc(row.speciesName)}">${esc(row.species)}</td>`;
    case 'compartment':
      return `<td class="rx-c-compartment" title="${esc(row.compartmentLabel)}">${esc(row.compartmentLabel || '—')}</td>`;
    case 'subsystem':
      return `<td class="rx-c-subsystem" title="${esc(row.subsystem)}">${esc(row.subsystem || '—')}</td>`;
    case 'type':
      return `<td class="rx-c-type"><span class="rx-type is-${row.type}" title="${esc(REACTION_TYPES.find((t) => t.id === row.type)?.title ?? '')}">${esc(reactionTypeLabel(row.type))}</span></td>`;
    case 'lower':
    case 'upper': {
      const value = row[column];
      if (mode === 'compact') return `<td class="rx-c-${column} number">${esc(formatBoundText(value))}</td>`;
      const title = row.heldOut ? ' title="Its species is held out of the simulation"' : geneClosed ? ` title="${esc(geneNote)}"` : '';
      return `<td class="rx-c-${column}"${title}>${boundControl({ rid: row.id, side: column, value, disabled: !editable, label: row.label })}</td>`;
    }
    case 'flux':
      return `<td class="rx-c-flux number">${Number.isFinite(row.flux) ? esc(formatFlux(row.flux, 4)) : '—'}</td>`;
    case 'fva':
      return `<td class="rx-c-fva">${fvaCell(row)}</td>`;
    case 'rc':
      return `<td class="rx-c-rc number">${Number.isFinite(row.reducedCost) ? esc(formatFlux(row.reducedCost, 5)) : '—'}</td>`;
    case 'objective':
      return `<td class="rx-c-objective"><input type="number" step="any" class="rx-objective-input" data-action="rx-objective" data-rid="${id}"
        value="${row.objective ? esc(formatBound(row.objective)) : ''}" placeholder="0" aria-label="Objective coefficient of ${esc(row.label)}"${locked ? ' disabled' : ''}></td>`;
    case 'status':
      return `<td class="rx-c-status"><span class="rx-status is-${row.status}" title="${esc(statusTitle(row, store))}">${STATUS_LABELS[row.status]}</span></td>`;
    case 'actions': {
      const ko = `<button type="button" class="rx-icon rx-ko${row.knockedOut ? ' is-on' : ''}" data-action="rx-ko" data-rid="${id}" aria-pressed="${row.knockedOut}"
        title="${geneClosed ? esc(geneNote) : row.knockedOut ? 'Restore: undo the knockout' : 'Knock out: both bounds 0'}"${editable ? '' : ' disabled'}>KO</button>`;
      const locate = `<button type="button" class="rx-icon" data-action="rx-locate" data-rid="${id}" title="${row.onMap ? 'Locate on the map' : 'Not drawn on the map'}"
        aria-label="Locate ${esc(row.label)} on the map"${row.onMap ? '' : ' disabled'}>⌖</button>`;
      if (mode === 'compact') return `<td class="rx-c-actions">${ko}${locate}</td>`;
      const reset = `<button type="button" class="rx-icon" data-action="rx-reset" data-rid="${id}" title="Reset to the bounds as opened"
        aria-label="Reset ${esc(row.label)}"${row.modified && editable ? '' : ' disabled'}>↺</button>`;
      return `<td class="rx-c-actions">${ko}${reset}${locate}</td>`;
    }
    default:
      return '<td></td>';
  }
}

function formatBoundText(value) {
  if (!Number.isFinite(value)) return '—';
  return Math.abs(value) >= 1e5 ? value.toExponential(0) : formatBound(value);
}

/** The FVA range drawn on the same sign-log scale as the sliders, with the flux marked on it. */
function fvaCell(row) {
  if (!Number.isFinite(row.fvaMin) || !Number.isFinite(row.fvaMax)) return '<span class="rx-muted">—</span>';
  const steps = signLogSteps();
  const at = (v) => ((signLogPosition(v) + steps) / (2 * steps)) * 100;
  const left = at(row.fvaMin);
  const width = Math.max(1.5, at(row.fvaMax) - left);
  const flux = Number.isFinite(row.flux) ? `<b style="left:${at(row.flux).toFixed(2)}%"></b>` : '';
  return `<span class="rx-fva-bar" aria-hidden="true"><i style="left:${left.toFixed(2)}%;width:${width.toFixed(2)}%"></i>${flux}</span>
    <span class="rx-fva-text">${esc(formatFlux(row.fvaMin, 3))} – ${esc(formatFlux(row.fvaMax, 3))}</span>`;
}

/** "Closed by the knockout of gene g1" (or of genes g1, g2). */
function geneKnockoutText(row, store) {
  const genes = row.geneKnockouts.map((key) => store?.geneTitle?.(key) ?? key);
  return `Closed by the knockout of gene${genes.length === 1 ? '' : 's'} ${genes.join(', ')}`;
}

function statusTitle(row, store) {
  const original = `As opened: [${formatFlux(row.originalLower, 4)}, ${formatFlux(row.originalUpper, 4)}]`;
  if (row.status === 'culprit') return `This edit left the LP without an optimum (${store?.constraintCulprits?.status ?? 'infeasible'}). ${original}`;
  if (row.geneKnockouts?.length) return `${geneKnockoutText(row, store)}. ${original}`;
  if (row.status === 'knocked-out' || row.status === 'modified') return original;
  if (row.status === 'blocked') return 'FVA: carries no flux at the optimum';
  return 'The model’s bounds';
}

// Results dock

/**
 * The results dock's Table view: the same rows, filters and sorting as the
 * Reactions page, read-mostly (knock out and locate only), with the way to the
 * page itself.
 *
 * @param {object} store
 * @param {object} view `model`, `scrollTop`, `viewportHeight`
 */
export function renderCompactReactionsTable(store, view = {}) {
  const model = view.model ?? store.reactionTableModel();
  const state = { ...view, model };
  const columns = reactionColumns(model, 'compact');
  const filters = store.reactionTable.filters.length || store.reactionTable.query
    ? ' · filtered as on the Reactions page' : '';
  return `<div class="rx-compact-body">
    <div class="rx-compact-scroll" data-rx-scroll="compact">
      <table class="rx-table is-compact" style="min-width:${columns.reduce((sum, column) => sum + column.width, 0)}px">
        ${colgroup(columns)}
        <thead>${headRow(store, model, 'compact')}</thead>
        <tbody data-rx-rows="compact">${renderReactionRows(store, state, 'compact')}</tbody>
      </table>
    </div>
    <p class="rx-compact-count"><strong>${model.shown}</strong> of ${model.total} reactions${model.modified ? ` · ${model.modified} modified` : ''}${filters}</p>
  </div>`;
}
