/**
 * Shared Add Reaction panel helpers.
 *
 * The panel deliberately separates reaction choice from placement direction:
 * users inspect the reaction and compact stoichiometry first, then choose one
 * of eight equidistant directions to commit the insertion.
 */

import { formatFlux } from '../render/theme.js';

const esc = (value) => String(value ?? '')
  .replace(/&/g, '&amp;')
  .replace(/</g, '&lt;')
  .replace(/>/g, '&gt;')
  .replace(/"/g, '&quot;');

export const REACTION_PLACEMENT_DISTANCE = 96;
const ROOT_HALF = Math.SQRT1_2;

export const REACTION_DIRECTIONS = Object.freeze([
  { id: 'n',  label: 'North',      glyph: '↑', dx: 0,          dy: -1 },
  { id: 'ne', label: 'North-east', glyph: '↗', dx: ROOT_HALF,  dy: -ROOT_HALF },
  { id: 'e',  label: 'East',       glyph: '→', dx: 1,          dy: 0 },
  { id: 'se', label: 'South-east', glyph: '↘', dx: ROOT_HALF,  dy: ROOT_HALF },
  { id: 's',  label: 'South',      glyph: '↓', dx: 0,          dy: 1 },
  { id: 'sw', label: 'South-west', glyph: '↙', dx: -ROOT_HALF, dy: ROOT_HALF },
  { id: 'w',  label: 'West',       glyph: '←', dx: -1,         dy: 0 },
  { id: 'nw', label: 'North-west', glyph: '↖', dx: -ROOT_HALF, dy: -ROOT_HALF },
]);

export function directionById(id) {
  return REACTION_DIRECTIONS.find((item) => item.id === id) ?? null;
}

// Where the panel sits

export const ADD_REACTION_PANEL_WIDTH = 380;
const PANEL_HEIGHT = 520;
const PANEL_MARGIN = 8;

const clampTo = (value, low, high) => Math.max(low, Math.min(value, Math.max(low, high)));

/**
 * Where an Add Reaction panel goes on the stage, and how tall it may be.
 *
 * It opens beside the click and is kept wholly on the stage, height included.
 * On a laptop the stage can be shorter than 520 px, and the lower part of the
 * panel holds the direction wheel, the one control that actually adds the
 * reaction. So the panel is never taller than the stage, and gives up list rows
 * rather than the wheel when it is short of room.
 *
 * Once it has been dragged it stays where it was put. Only its header is kept
 * within reach, so it can be moved aside to see the map behind it.
 *
 * @param {{clientWidth:number, clientHeight:number}} stage
 * @param {{left:number, top:number, moved?:boolean}} state
 * @returns {{left:number, top:number, width:number, maxHeight:number}} stage pixels
 */
export function panelPlacement(stage, state) {
  const stageWidth = Number(stage?.clientWidth) || 0;
  const stageHeight = Number(stage?.clientHeight) || 0;
  const width = stageWidth
    ? Math.min(ADD_REACTION_PANEL_WIDTH, Math.max(240, stageWidth - 2 * PANEL_MARGIN))
    : ADD_REACTION_PANEL_WIDTH;
  const maxHeight = stageHeight
    ? Math.min(PANEL_HEIGHT, Math.max(220, stageHeight - 2 * PANEL_MARGIN))
    : PANEL_HEIGHT;
  if (state?.moved) {
    return {
      width,
      maxHeight,
      left: clampTo(state.left, 64 - width, stageWidth - 64),
      top: clampTo(state.top, 0, stageHeight - 40),
    };
  }
  return {
    width,
    maxHeight,
    left: clampTo(state?.left ?? PANEL_MARGIN, PANEL_MARGIN, stageWidth - width - PANEL_MARGIN),
    top: clampTo(state?.top ?? PANEL_MARGIN, PANEL_MARGIN, stageHeight - maxHeight - PANEL_MARGIN),
  };
}

/**
 * Let the panel be moved by its header.
 *
 * Called from a controller's pointerdown listener; returns whether the press
 * started a drag. The pointer is captured by the header for the length of the
 * drag, so the release still counts as a press on the panel wherever it ends:
 * left to the map underneath, it would be read as a click on empty canvas and
 * close the very panel that was being moved.
 */
export function beginPanelDrag(controller, event) {
  if (!controller?.state) return false;
  const head = event.target?.closest?.('.add-reaction-head');
  if (!head || event.target.closest('button, input, select, a')) return false;
  if (event.pointerType === 'mouse' && event.button !== 0) return false;
  const menu = head.closest('.add-reaction-menu');
  if (!menu) return false;

  event.preventDefault();
  const from = { x: event.clientX, y: event.clientY, left: menu.offsetLeft, top: menu.offsetTop };
  try { head.setPointerCapture?.(event.pointerId); } catch { /* not capturable */ }
  menu.classList.add('is-dragging');

  const move = (e) => {
    if (!controller.state) return;
    const place = panelPlacement(controller.stage, {
      left: from.left + e.clientX - from.x,
      top: from.top + e.clientY - from.y,
      moved: true,
    });
    controller.state.left = place.left;
    controller.state.top = place.top;
    controller.state.moved = true;
    menu.style.left = `${place.left}px`;
    menu.style.top = `${place.top}px`;
  };
  const end = (e) => {
    head.removeEventListener('pointermove', move);
    head.removeEventListener('pointerup', end);
    head.removeEventListener('pointercancel', end);
    try { head.releasePointerCapture?.(e.pointerId); } catch { /* never captured */ }
    menu.classList.remove('is-dragging');
  };
  head.addEventListener('pointermove', move);
  head.addEventListener('pointerup', end);
  head.addEventListener('pointercancel', end);
  return true;
}

/**
 * A compact equation for a merged reaction id, such as `2 A + B → C`, with each
 * side cut to `maxTerms` terms (default 3) and `↔` for reversible bounds.
 * Returns '—' for an unknown reaction.
 */
export function reactionStoichiometry(store, reactionId, { maxTerms = 3 } = {}) {
  const reaction = store.community?.merged?.reactions?.get?.(reactionId);
  if (!reaction) return '—';

  const substrates = [];
  const products = [];
  for (const [mergedId, coefficientRaw] of Object.entries(reaction.metabolites ?? {})) {
    const coefficient = Number(coefficientRaw);
    if (!Number.isFinite(coefficient) || coefficient === 0) continue;
    const label = metaboliteLabel(store, mergedId);
    const magnitude = Math.abs(coefficient);
    const term = magnitude === 1
      ? label
      : `${formatCoefficient(magnitude)} ${label}`;
    (coefficient < 0 ? substrates : products).push(term);
  }

  const reversible = Number(reaction.lowerBound) < 0 && Number(reaction.upperBound) > 0;
  const arrow = reversible ? '↔' : '→';
  return `${summarizeSide(substrates, maxTerms)} ${arrow} ${summarizeSide(products, maxTerms)}`;
}

/** Accent-free, lower-case text a reaction filter matches against: label, id, name, subsystem and full equation. */
export function reactionSearchText(store, reactionId) {
  const reaction = store.community?.merged?.reactions?.get?.(reactionId);
  const named = entityLabel(store, 'reaction', reactionId);
  return normalizeSearch([
    named.label,
    named.id,
    named.name,
    reaction?.subsystem ?? '',
    reactionStoichiometry(store, reactionId, { maxTerms: 99 }),
  ].join(' '));
}

/**
 * How a reaction is listed in a menu: by the map's label for it, and beside
 * that the one thing the label is not, which is its name, or its model id when
 * the map is labelled by name.
 */
export function reactionListing(store, reactionId) {
  const { label, id, name } = entityLabel(store, 'reaction', reactionId);
  return { label, detail: label === name ? (id === label ? '' : id) : name };
}

/**
 * A metabolite or reaction named as the map names it (see Store#entityLabel),
 * or by its model id where the store cannot say.
 */
function entityLabel(store, kind, mergedId) {
  const named = store.entityLabel?.(kind, mergedId);
  if (named) return named;
  const community = store.community;
  const owner = (kind === 'metabolite' ? community?.metaboliteOwner : community?.reactionOwner)?.get?.(mergedId);
  const entity = (kind === 'metabolite' ? community?.merged?.metabolites : community?.merged?.reactions)?.get?.(mergedId);
  const id = owner?.originalId ?? (kind === 'metabolite' ? entity?.base : null) ?? entity?.id ?? mergedId;
  return { label: id, id, name: entity?.name ?? '' };
}


/**
 * The metabolites on one side ('product' or 'substrate') of a merged reaction,
 * as {id, label}, leaving out cofactors and currency metabolites (NAD(P)H,
 * CoA, water, CO2, protons, ATP/ADP/AMP, phosphate).
 */
export function primaryMetabolitesForSide(store, reactionId, side = 'product') {
  const reaction = store.community?.merged?.reactions?.get?.(reactionId);
  if (!reaction) return [];
  const wantProduct = side === 'product';
  const out = [];
  for (const [mergedId, raw] of Object.entries(reaction.metabolites ?? {})) {
    const coefficient = Number(raw);
    if (!Number.isFinite(coefficient) || coefficient === 0 || (coefficient > 0) !== wantProduct) continue;
    const metabolite = store.community?.merged?.metabolites?.get?.(mergedId);
    if (!metabolite) continue;
    const base = String(metabolite.base ?? metabolite.id ?? '').toLowerCase();
    if (['nad','nadh','nadp','nadph','coa','h2o','co2','h','atp','adp','amp','pi'].includes(base)) continue;
    out.push({ id: mergedId, label: metaboliteLabel(store, mergedId) });
  }
  return out;
}

/**
 * HTML of the Add Reaction panel: a filterable list of `reactionIds` (merged
 * ids), the terminal-metabolite chooser for the selected reaction and the
 * eight-way direction wheel. Position and size are stage pixels, usually from
 * panelPlacement. With `fluxes` (reaction id -> flux of the current solution)
 * every row also shows its reaction's flux.
 */
export function renderAddReactionPanel({
  store,
  reactionIds,
  selectedReactionId = null,
  title = 'Add reaction',
  subtitle = '',
  left = 8,
  top = 8,
  width = ADD_REACTION_PANEL_WIDTH,
  maxHeight = PANEL_HEIGHT,
  terminalMetaboliteId = null,
  fluxes = null,
}) {
  const selected = reactionIds.includes(selectedReactionId) ? selectedReactionId : null;

  const rows = reactionIds.length
    ? reactionIds.map((reactionId) => {
        const { label, detail } = reactionListing(store, reactionId);
        const selectedClass = reactionId === selected ? ' is-selected' : '';
        const equation = reactionStoichiometry(store, reactionId);
        const flux = fluxes
          ? `<span class="add-reaction-flux" title="Flux in the current solution">${esc(formatFlux(Number(fluxes.get(reactionId) ?? 0), 4))}</span>`
          : '';
        return `
          <button class="add-reaction-choice${selectedClass}"
            data-select-add-rxn="${esc(reactionId)}"
            data-search="${esc(reactionSearchText(store, reactionId))}"
            type="button"
            aria-pressed="${reactionId === selected ? 'true' : 'false'}">
            <span class="add-reaction-choice-head${fluxes ? ' has-flux' : ''}">
              <strong>${esc(label)}</strong>
              <span>${esc(detail)}</span>
              ${flux}
            </span>
            <code class="add-reaction-stoich">${esc(equation)}</code>
          </button>`;
      }).join('')
    : '<p class="add-reaction-empty">No reactions are available to add here.</p>';

  const terminals = selected ? primaryMetabolitesForSide(store, selected, 'product') : [];
  const terminalIndex = Math.max(0, terminals.findIndex((item) => item.id === terminalMetaboliteId));
  const terminal = terminals[terminalIndex] ?? terminals[0] ?? null;
  const terminalControl = terminals.length > 1 ? `
    <div class="add-reaction-terminal">
      <div><strong>Terminal metabolite</strong><span>Product on the reaction axis.</span></div>
      <div class="add-reaction-terminal-controls">
        <button type="button" data-rotate-terminal="-1" aria-label="Previous terminal metabolite">‹</button>
        <code>${esc(terminal?.label ?? '—')}</code>
        <button type="button" data-rotate-terminal="1" aria-label="Next terminal metabolite">›</button>
      </div>
    </div>` : (terminal ? `
    <div class="add-reaction-terminal">
      <div><strong>Terminal metabolite</strong><span>Product on the reaction axis.</span></div>
      <code>${esc(terminal.label)}</code>
    </div>` : '');

  const directions = REACTION_DIRECTIONS.map((direction) => {
    const disabled = selected ? '' : ' disabled';
    const x = 50 + direction.dx * 35.82;
    const y = 50 + direction.dy * 35.82;
    return `<button class="add-reaction-direction-point" type="button"
      data-add-rxn-direction="${direction.id}"
      aria-label="Add selected reaction to the ${esc(direction.label)}"
      title="${esc(direction.label)}"${disabled}
      style="left:${x}%;top:${y}%">${direction.glyph}</button>`;
  }).join('');

  // The wheel stands beside its caption rather than under it, and the panel is
  // cut to the height it is given: what gives way is the list, which scrolls.
  const compact = maxHeight < 380 ? ' is-compact' : '';
  return `<div class="add-reaction-menu${compact}" style="left:${left}px;top:${top}px;width:${width}px;max-height:${maxHeight}px">
    <div class="add-reaction-head" title="Drag to move this panel">
      <div>
        <strong>${esc(title)}</strong>
        <span>${esc(subtitle)}</span>
      </div>
      <button class="add-reaction-close" data-action="close-add-reaction-menu" type="button" aria-label="Close Add Reaction panel">×</button>
    </div>
    <div class="add-reaction-summary">
      ${reactionIds.length} ${reactionIds.length === 1 ? 'reaction available' : 'reactions available'}
    </div>
    ${reactionIds.length ? '<div class="add-reaction-search"><input type="text" data-action="filter-add-reactions" placeholder="Filter reactions…" autocomplete="off" spellcheck="false"></div>' : ''}
    <div class="add-reaction-list">${rows}</div>
    <div class="add-reaction-direction-section">
      <div class="add-reaction-direction-text">
        <div class="add-reaction-direction-title">
          <strong>Direction</strong>
          <span>${selected ? 'Where to place the reaction.' : 'Select a reaction first.'}</span>
        </div>
        ${terminalControl}
      </div>
      <div class="add-reaction-direction-wheel" aria-label="Reaction placement direction">
        <span class="add-reaction-direction-center" aria-hidden="true"></span>
        ${directions}
      </div>
    </div>
  </div>`;
}

/** Hide the panel's reaction rows that do not match the filter input, and update the count. */
export function filterReactionButtons(input) {
  const q = normalizeSearch(input.value);
  const menu = input.closest('.add-reaction-menu');
  const buttons = [...(menu?.querySelectorAll('.add-reaction-choice') ?? [])];
  let visible = 0;
  for (const button of buttons) {
    const match = !q || normalizeSearch(button.dataset.search).includes(q);
    button.hidden = !match;
    if (match) visible += 1;
  }
  const summary = menu?.querySelector('.add-reaction-summary');
  if (summary) {
    summary.textContent = q
      ? `${visible} of ${buttons.length} reactions`
      : `${buttons.length} ${buttons.length === 1 ? 'reaction available' : 'reactions available'}`;
  }
}

function metaboliteLabel(store, mergedId) {
  return entityLabel(store, 'metabolite', mergedId).label;
}

function summarizeSide(terms, maxTerms) {
  if (!terms.length) return '∅';
  if (terms.length <= maxTerms) return terms.join(' + ');
  return `${terms.slice(0, maxTerms).join(' + ')} + …`;
}

function formatCoefficient(value) {
  if (Number.isInteger(value)) return String(value);
  return String(Number(value.toFixed(3)));
}

function normalizeSearch(value) {
  return String(value ?? '')
    .normalize('NFD')
    .replace(/[\u0300-\u036f]/g, '')
    .trim()
    .toLowerCase();
}
