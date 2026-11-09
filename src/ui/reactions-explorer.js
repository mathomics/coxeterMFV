/**
 * Reactions and bounds explorer: the logic behind the Reactions page, kept
 * free of the DOM so it can be tested and ported as it is.
 *
 *   types      what kind of reaction a row is (exchange, transport, ...)
 *   sign-log   the slider scale every bound control uses
 *   rows       search, quick filters, sorting and grouping of table rows
 *   history    undo/redo over constraint snapshots, apart from the map's
 *   CSV        bounds files in and out
 */

import { LINK_COMPARTMENT, SHARED_COMPARTMENT, communityExchange, splitId } from '../core/community.js';

// Types

/** The kinds of reaction the table tells apart, in the order groups are listed. */
export const REACTION_TYPES = [
  { id: 'exchange', label: 'Exchange', title: 'Boundary of the community or of a model: supply and washout' },
  { id: 'uptake', label: 'Uptake/secretion', title: 'A species takes from or gives to the shared medium' },
  { id: 'transport', label: 'Transport', title: 'Moves metabolites between compartments of one cell' },
  { id: 'internal', label: 'Internal', title: 'Takes place inside one compartment' },
  { id: 'biomass', label: 'Biomass', title: 'Growth: a biomass or community growth reaction' },
  { id: 'demand', label: 'Demand/Sink', title: 'A demand or sink: drains or supplies one metabolite' },
];

const TYPE_ORDER = new Map(REACTION_TYPES.map((type, index) => [type.id, index]));

/** The table's name for a type id. */
export function reactionTypeLabel(type) {
  return REACTION_TYPES.find((item) => item.id === type)?.label ?? type;
}

/**
 * What kind of reaction `rid` is in a community.
 *
 * Read from how the community was put together, not from the id alone: a
 * species' exchange became a transfer to the shared medium (uptake/secretion),
 * the community's own boundary is an exchange, and the biomass reactions are
 * the ones the community grows by. DM_/SK_ prefixes mark demands and sinks.
 *
 * @param {import('../core/community.js').CommunityModel} community
 * @param {string} rid merged reaction id
 * @returns {'exchange'|'uptake'|'transport'|'internal'|'biomass'|'demand'}
 */
export function classifyReaction(community, rid) {
  const reaction = community?.merged?.reactions?.get(rid);
  const owner = community?.reactionOwner?.get(rid);
  const localId = String(owner?.originalId ?? rid);
  if (rid === 'COMMUNITY_GROWTH' || owner?.role === 'biomass' || reaction?.kind === 'biomass') return 'biomass';
  for (const id of community?.biomassIds?.values?.() ?? []) if (id === rid) return 'biomass';
  // A reaction the community did not place: only the model's own kind is known.
  if (!owner) {
    return { exchange: 'exchange', sink: 'demand', transport: 'transport' }[reaction?.kind] ?? 'internal';
  }
  if (!owner.speciesId) return 'exchange';
  if (owner.role === 'transfer') return 'uptake';
  if (owner.role === 'sink' || /^(DM|SK)_/i.test(localId)) return 'demand';
  if (owner.role === 'exchange') return 'exchange';
  const compartments = (reaction?.compartments ?? []).filter((id) => id !== LINK_COMPARTMENT);
  if (owner.role === 'transport' || compartments.length > 1) return 'transport';
  return 'internal';
}

// Sign-log slider

/*
 * Bounds span six orders of magnitude in practice (a trace uptake of 0.001, an
 * unbounded 1000), and a linear slider over ±1000 cannot set anything below a
 * few units. The sign-log scale gives every decade the same travel, both signs
 * mirrored, with the middle position (and its neighbours) exactly 0. The number
 * box beside it stays exact; the slider is for moving fast.
 */
export const SIGN_LOG = Object.freeze({ min: 1e-3, max: 1e3, stepsPerDecade: 40, deadZone: 1 });

/** The largest slider position: positions run from -steps to +steps. */
export function signLogSteps(scale = SIGN_LOG) {
  return scale.deadZone + 1 + Math.round(Math.log10(scale.max / scale.min) * scale.stepsPerDecade);
}

/**
 * The value at a slider position: 0 within the dead zone, then ±min to ±max
 * logarithmically, rounded to three significant digits so a drag lands on
 * numbers worth reading.
 */
export function signLogValue(position, scale = SIGN_LOG) {
  const steps = signLogSteps(scale);
  const p = Math.round(Number(position) || 0);
  const a = Math.min(Math.abs(p), steps);
  if (a <= scale.deadZone) return 0;
  const t = (a - scale.deadZone - 1) / (steps - scale.deadZone - 1);
  const value = Number((scale.min * (scale.max / scale.min) ** t).toPrecision(3));
  return Math.sign(p) * value;
}

/**
 * The slider position for a value. Beyond ±max (1e6 for "unbounded") it sits
 * at the end; a non-zero value below min sits on the first step off zero, so
 * the slider never claims a non-zero bound is 0.
 */
export function signLogPosition(value, scale = SIGN_LOG) {
  const steps = signLogSteps(scale);
  const v = Number(value);
  if (Number.isNaN(v) || v === 0) return 0;
  const sign = Math.sign(v);
  const a = Math.abs(v);
  if (a >= scale.max) return sign * steps;
  if (a <= scale.min) return sign * (scale.deadZone + 1);
  const t = Math.log10(a / scale.min) / Math.log10(scale.max / scale.min);
  return sign * (scale.deadZone + 1 + Math.round(t * (steps - scale.deadZone - 1)));
}

// Bounds

const TOLERANCE = 1e-9;

/** Whether two {lower, upper} pairs are the same (both absent counts as the same). */
export function sameBounds(a, b, tolerance = TOLERANCE) {
  if (!a || !b) return !a && !b;
  return Math.abs(a.lower - b.lower) <= tolerance && Math.abs(a.upper - b.upper) <= tolerance;
}

/** Closed by the user: both bounds 0 where the model had it open. */
export function isKnockedOut(current, original, tolerance = TOLERANCE) {
  const closed = (b) => Math.abs(b.lower) <= tolerance && Math.abs(b.upper) <= tolerance;
  return Boolean(current && original) && closed(current) && !closed(original);
}

// Catalogue

/**
 * What does not change between solves for every reaction of a community: its
 * names, where it is, what it is, and the text search runs over.
 *
 * @param {object} community
 * @param {object} [names]
 * @param {(rid:string)=>string} [names.label] the label the map would show
 * @param {(speciesId:string|null, compartmentId:string)=>string} [names.compartment]
 * @param {(speciesId:string|null)=>string} [names.species] a species' full name
 * @returns {object[]} one entry per merged reaction, in model order
 */
export function buildReactionCatalog(community, names = {}) {
  const merged = community?.merged;
  if (!merged?.reactions) return [];
  const labelOf = names.label ?? ((rid) => community.reactionOwner?.get(rid)?.originalId ?? rid);
  const compartmentName = names.compartment ?? ((_, id) => id);
  const speciesName = names.species ?? ((id) => id ?? 'Community');
  const rows = [];
  let index = 0;
  for (const [id, reaction] of merged.reactions) {
    const owner = community.reactionOwner?.get(id) ?? {};
    const speciesId = owner.speciesId ?? null;
    const localId = String(owner.originalId ?? id);
    const label = String(labelOf(id) ?? localId);
    const metabolites = Object.keys(reaction.metabolites ?? {})
      .map((mid) => ({ mid, metabolite: merged.metabolites?.get(mid) }))
      .filter(({ metabolite }) => metabolite?.compartment !== LINK_COMPARTMENT);
    // A cell may call one of its own compartments `s` too, so the shared
    // medium is told apart by who owns the metabolite, not by the id.
    const places = new Map();
    for (const { mid, metabolite } of metabolites) {
      const owner = community.metaboliteOwner?.get(mid);
      const shared = owner ? !owner.speciesId : metabolite?.compartment === SHARED_COMPARTMENT;
      const cid = shared ? SHARED_COMPARTMENT : String(metabolite?.compartment ?? '');
      if (!places.has(cid)) places.set(cid, shared ? 'Shared medium' : compartmentName(owner?.speciesId ?? speciesId, cid));
    }
    const compartments = [...places.keys()].sort();
    const compartmentLabels = compartments.map((cid) => places.get(cid));
    const ec = [reaction.labelSources?.ec ?? []].flat().join(' ');
    const genes = reaction.geneReactionRule ?? '';
    const metaboliteText = metabolites.map(({ mid, metabolite }) => `${metaboliteLocalId(mid)} ${metabolite?.name ?? ''}`).join(' ');
    rows.push({
      id, index: index++, localId, label, name: reaction.name ?? '',
      speciesId, species: speciesId ?? 'Community', speciesName: speciesName(speciesId),
      compartments, compartmentLabel: compartmentLabels.join(' ↔ '),
      subsystem: reaction.subsystem ?? '', pathway: reaction.pathway ?? '',
      type: classifyReaction(community, id),
      equation: readableEquation(reaction, metabolites),
      genes, ec,
      search: normalizeSearch([id, localId, label, reaction.name, genes, ec, metaboliteText].join(' ')),
    });
  }
  return rows;
}

function metaboliteLocalId(mid) {
  const { species, id } = splitId(mid);
  return species === 'M' ? `${id}[medium]` : id;
}

/** `a + 2 b --> c`, with the cell prefix taken off and medium metabolites marked. */
function readableEquation(reaction, metabolites) {
  const side = (sign) => metabolites
    .map(({ mid }) => [mid, reaction.metabolites[mid]])
    .filter(([, coef]) => Math.sign(coef) === sign)
    .map(([mid, coef]) => (Math.abs(coef) === 1 ? metaboliteLocalId(mid) : `${Number(Math.abs(coef).toPrecision(6))} ${metaboliteLocalId(mid)}`))
    .join(' + ') || '∅';
  const lower = Number(reaction.lowerBound);
  const upper = Number(reaction.upperBound);
  const arrow = lower < 0 && upper > 0 ? '<=>' : upper > 0 ? '-->' : '<--';
  return `${side(-1)} ${arrow} ${side(1)}`;
}

/** Lower case, accents taken off, trimmed: how both rows and queries are compared. */
export function normalizeSearch(value) {
  return String(value ?? '').normalize('NFD').replace(/[̀-ͯ]/g, '').toLowerCase().trim();
}

// Rows

/** The quick filters, each a test on a table row. Several together all have to hold. */
export const REACTION_FILTERS = [
  { id: 'exchanges', label: 'Exchanges only', title: 'Exchanges and uptake/secretion', test: (row) => row.type === 'exchange' || row.type === 'uptake' },
  { id: 'modified', label: 'Modified', title: 'Bounds differ from the model’s', test: (row) => row.modified },
  { id: 'active', label: 'Active', title: '|flux| > 1e-9 in the solution', test: (row) => Math.abs(row.flux) > 1e-9 },
  { id: 'blocked', label: 'Blocked', title: 'FVA range is 0 to 0', test: (row) => row.blocked },
  { id: 'on-map', label: 'On map', title: 'Drawn on the map', test: (row) => row.onMap },
  { id: 'off-map', label: 'Not on map', title: 'In the model, not drawn', test: (row) => !row.onMap },
  { id: 'knocked-out', label: 'Knocked out', title: 'Both bounds 0 where the model had it open', test: (row) => row.knockedOut },
];

/** Switch one quick filter on or off. On map and Not on map exclude each other. */
export function toggleReactionFilter(filters, id) {
  const next = new Set(filters ?? []);
  if (next.has(id)) next.delete(id);
  else {
    next.add(id);
    if (id === 'on-map') next.delete('off-map');
    if (id === 'off-map') next.delete('on-map');
  }
  return REACTION_FILTERS.map((filter) => filter.id).filter((key) => next.has(key));
}

/**
 * The rows a search and the quick filters leave. Every word of the query has
 * to appear in the row's id, name, genes, metabolites or EC numbers.
 */
export function filterReactionRows(rows, { query = '', filters = [] } = {}) {
  const words = normalizeSearch(query).split(/\s+/).filter(Boolean);
  const tests = REACTION_FILTERS.filter((filter) => filters.includes(filter.id)).map((filter) => filter.test);
  if (!words.length && !tests.length) return rows;
  return rows.filter((row) => words.every((word) => row.search.includes(word)) && tests.every((test) => test(row)));
}

const STATUS_ORDER = new Map(['culprit', 'knocked-out', 'modified', 'blocked', 'original'].map((s, i) => [s, i]));

/** The columns the table sorts by, each with how its values compare. */
export const REACTION_SORT_KEYS = {
  reaction: { text: (row) => row.label },
  species: { text: (row) => row.species },
  compartment: { text: (row) => row.compartmentLabel },
  subsystem: { text: (row) => row.subsystem },
  type: { number: (row) => TYPE_ORDER.get(row.type) ?? 99 },
  lower: { number: (row) => row.lower },
  upper: { number: (row) => row.upper },
  flux: { number: (row) => row.flux },
  fva: { number: (row) => row.fvaMax - row.fvaMin },
  rc: { number: (row) => row.reducedCost },
  objective: { number: (row) => row.objective },
  status: { number: (row) => STATUS_ORDER.get(row.status) ?? 99 },
};

/** Rows sorted by a column (`dir` 1 ascending, -1 descending). Missing numbers go last either way; ties go by label. */
export function sortReactionRows(rows, { key = 'reaction', dir = 1 } = {}) {
  const spec = REACTION_SORT_KEYS[key] ?? REACTION_SORT_KEYS.reaction;
  const sign = dir < 0 ? -1 : 1;
  const tie = (a, b) => a.label.localeCompare(b.label) || a.id.localeCompare(b.id);
  if (spec.text) {
    return [...rows].sort((a, b) => sign * String(spec.text(a) ?? '').localeCompare(String(spec.text(b) ?? '')) || tie(a, b));
  }
  return [...rows].sort((a, b) => {
    const x = spec.number(a);
    const y = spec.number(b);
    const okX = Number.isFinite(x);
    const okY = Number.isFinite(y);
    if (okX !== okY) return okX ? -1 : 1;
    return (okX ? sign * (x - y) : 0) || tie(a, b);
  });
}

/** How the table can be grouped. */
export const REACTION_GROUPINGS = [
  { id: 'type', label: 'Type' },
  { id: 'compartment', label: 'Compartment' },
  { id: 'species', label: 'Species' },
  { id: 'subsystem', label: 'Subsystem' },
  { id: 'pathway', label: 'Pathway' },
  { id: 'none', label: 'None' },
];

/**
 * The rows as the table lists them: a header item per group, with its count,
 * how many of its rows are modified and selected, followed by its rows unless
 * the group is collapsed. Without grouping, just the rows.
 *
 * @param {object[]} rows already filtered and sorted
 * @param {string} groupBy one of REACTION_GROUPINGS
 * @param {object} [options]
 * @param {Set<string>} [options.collapsed] group keys folded away
 * @param {Set<string>} [options.selection] selected reaction ids
 * @param {(rid:string)=>string|null} [options.pathwayOf] the pathway manager's group of a reaction
 * @returns {object[]} items `{kind:'group', key, label, count, modified, selected, collapsed, ids}` and `{kind:'row', row}`
 */
export function groupReactionRows(rows, groupBy = 'type', { collapsed = new Set(), selection = new Set(), pathwayOf = null } = {}) {
  if (!groupBy || groupBy === 'none') return rows.map((row) => ({ kind: 'row', row }));
  const keyOf = {
    type: (row) => [row.type, reactionTypeLabel(row.type)],
    compartment: (row) => [row.compartmentLabel || '—', row.compartmentLabel || 'No compartment'],
    species: (row) => [row.speciesId ?? '', row.speciesId ? row.species : 'Community'],
    subsystem: (row) => [row.subsystem || '', row.subsystem || 'No subsystem'],
    pathway: (row) => {
      const name = pathwayOf?.(row.id) || row.pathway || '';
      return [name, name || 'No pathway'];
    },
  }[groupBy] ?? ((row) => [row.type, reactionTypeLabel(row.type)]);

  const groups = new Map();
  for (const row of rows) {
    const [key, label] = keyOf(row);
    if (!groups.has(key)) groups.set(key, { key, label, rows: [] });
    groups.get(key).rows.push(row);
  }
  const ordered = [...groups.values()].sort((a, b) => {
    if (groupBy === 'type') return (TYPE_ORDER.get(a.key) ?? 99) - (TYPE_ORDER.get(b.key) ?? 99);
    // The group of rows with nothing to group by goes last.
    if (!a.key !== !b.key) return a.key ? -1 : 1;
    return a.label.localeCompare(b.label);
  });
  const items = [];
  for (const group of ordered) {
    const ids = group.rows.map((row) => row.id);
    const folded = collapsed.has(group.key);
    items.push({
      kind: 'group', key: group.key, label: group.label, count: group.rows.length,
      modified: group.rows.filter((row) => row.modified).length,
      selected: ids.filter((id) => selection.has(id)).length,
      collapsed: folded, ids,
    });
    if (!folded) for (const row of group.rows) items.push({ kind: 'row', row });
  }
  return items;
}

/**
 * Which items of a long list to render: those in the viewport plus `overscan`
 * on each side, and the empty height to leave above and below them so the
 * scrollbar still stands for the whole list.
 */
export function virtualWindow({ count, rowHeight, scrollTop = 0, viewportHeight = 600, overscan = 6 }) {
  const total = Math.max(0, count | 0);
  const height = Math.max(1, rowHeight);
  const first = Math.max(0, Math.floor(Math.max(0, scrollTop) / height) - overscan);
  const shown = Math.ceil(Math.max(height, viewportHeight) / height) + overscan * 2;
  // Scrolled past a list that has just shrunk: show its end, where the browser will clamp to.
  const start = Math.min(first, Math.max(0, total - shown));
  const end = Math.min(total, start + shown);
  return { start: total ? start : 0, end, top: (total ? start : 0) * height, bottom: Math.max(0, total - end) * height };
}

// Constraint history

/**
 * Undo and redo for what the LP is asked: reaction bounds, the medium and the
 * objective. The map keeps its own history (MapHistory); the two never mix, so
 * the map's Ctrl+Z cannot take back a knockout, nor this a drag.
 *
 * Entries hold the snapshot taken before a change. A run of changes to the
 * same thing within `coalesceMs` (one slider drag, typing in a box) is one
 * entry, so Undo goes back to before the drag rather than one tick of it.
 */
export class ConstraintHistory {
  constructor(limit = 100, { coalesceMs = 1500 } = {}) {
    this.limit = Math.max(1, Number(limit) || 100);
    this.coalesceMs = coalesceMs;
    this.undoStack = [];
    this.redoStack = [];
  }

  get canUndo() { return this.undoStack.length > 0; }
  get canRedo() { return this.redoStack.length > 0; }
  get undoLabel() { return this.undoStack.at(-1)?.label ?? ''; }
  get redoLabel() { return this.redoStack.at(-1)?.label ?? ''; }

  clear() {
    this.undoStack.length = 0;
    this.redoStack.length = 0;
  }

  /**
   * Record the snapshot from before a change. Returns the entry the change
   * belongs to: a new one, or the last one when it continues the same edit.
   */
  record(before, { label = 'Change constraints', key = null, at = Date.now() } = {}) {
    const last = this.undoStack.at(-1);
    if (key && last?.key === key && !this.redoStack.length && at - last.at <= this.coalesceMs) {
      last.at = at;
      last.label = label;
      return last;
    }
    const entry = { label, key, at, snapshot: copy(before) };
    this.undoStack.push(entry);
    if (this.undoStack.length > this.limit) this.undoStack.shift();
    this.redoStack.length = 0;
    return entry;
  }

  /** Step back: returns `{label, snapshot}` to restore, with `current` kept for redo; null when there is nothing. */
  undo(current) {
    const entry = this.undoStack.pop();
    if (!entry) return null;
    this.redoStack.push({ label: entry.label, key: null, at: 0, snapshot: copy(current) });
    return { label: entry.label, snapshot: copy(entry.snapshot) };
  }

  /** Step forward again: returns `{label, snapshot}`, with `current` back on the undo stack; null when there is nothing. */
  redo(current) {
    const entry = this.redoStack.pop();
    if (!entry) return null;
    this.undoStack.push({ label: entry.label, key: null, at: 0, snapshot: copy(current) });
    return { label: entry.label, snapshot: copy(entry.snapshot) };
  }
}

function copy(value) {
  return JSON.parse(JSON.stringify(value));
}

/** Whether two constraint snapshots ask the LP the same thing. */
export function sameConstraints(a, b) {
  return JSON.stringify(a) === JSON.stringify(b);
}

/**
 * The reactions whose constraints differ between two snapshots: overridden
 * bounds, medium limits (through `mediumReactionId`, the exchange a medium
 * metabolite opens) and objective coefficients. These are the suspects when a
 * change leaves the LP without a solution.
 */
export function changedReactionIds(before, after, mediumReactionId = (base) => communityExchange(base)) {
  const ids = new Set();
  const a = new Map(before?.overrides ?? []);
  const b = new Map(after?.overrides ?? []);
  for (const id of new Set([...a.keys(), ...b.keys()])) {
    if (!sameBounds(a.get(id), b.get(id))) ids.add(id);
  }
  const ma = before?.medium ?? {};
  const mb = after?.medium ?? {};
  for (const base of new Set([...Object.keys(ma), ...Object.keys(mb)])) {
    if (Number(ma[base] ?? NaN) !== Number(mb[base] ?? NaN)) {
      const id = mediumReactionId(base);
      if (id) ids.add(id);
    }
  }
  const ca = before?.objective?.custom?.coefficients ?? {};
  const cb = after?.objective?.custom?.coefficients ?? {};
  for (const id of new Set([...Object.keys(ca), ...Object.keys(cb)])) {
    if ((ca[id] ?? 0) !== (cb[id] ?? 0)) ids.add(id);
  }
  return [...ids];
}

// Bounds CSV

const CSV_COLUMNS = {
  id: ['id', 'reaction', 'reaction_id', 'rxn', 'reactionid'],
  lower: ['lower', 'lower_bound', 'lb', 'lowerbound', 'min'],
  upper: ['upper', 'upper_bound', 'ub', 'upperbound', 'max'],
  species: ['species', 'species_id', 'organism', 'cell'],
  originalId: ['original_id', 'local_id', 'model_id', 'originalid'],
};

/**
 * A bounds table as CSV text: `id, lower, upper, species, original_id, name`,
 * one line per entry `{id, lower, upper, species, originalId, name}`. Only the
 * first three are read back; the rest say what each line is.
 */
export function formatBoundsCSV(entries) {
  const lines = ['id,lower,upper,species,original_id,name'];
  for (const entry of entries) {
    lines.push([
      csvField(entry.id), csvNumber(entry.lower), csvNumber(entry.upper),
      csvField(entry.species ?? ''), csvField(entry.originalId ?? ''), csvField(entry.name ?? ''),
    ].join(','));
  }
  return `${lines.join('\n')}\n`;
}

function csvField(value) {
  const text = String(value ?? '');
  return /[",\n\r]/.test(text) ? `"${text.replace(/"/g, '""')}"` : text;
}

function csvNumber(value) {
  return Number.isFinite(value) ? String(Number(value.toPrecision(12))) : '';
}

/**
 * Read a bounds CSV (comma, semicolon or tab separated; quoted fields allowed).
 * The first line names the columns: an id column and a lower and/or upper
 * column, optionally species and original_id. An empty bound leaves that side
 * as it is.
 *
 * @returns {{rows:{line:number,id:string,species:string,originalId:string,lower:number|null,upper:number|null}[], errors:{line:number,message:string}[]}}
 */
export function parseBoundsCSV(text) {
  const lines = String(text ?? '').replace(/^﻿/, '').split(/\r\n|\n|\r/);
  const rows = [];
  const errors = [];
  let headerAt = lines.findIndex((line) => line.trim() && !line.trim().startsWith('#'));
  if (headerAt < 0) return { rows, errors: [{ line: 0, message: 'The file is empty.' }] };
  const headerLine = lines[headerAt];
  const delimiter = headerLine.includes('\t') ? '\t' : (headerLine.includes(';') && !headerLine.includes(',') ? ';' : ',');
  const header = splitCSVLine(headerLine, delimiter).map((name) => name.trim().toLowerCase().replace(/\s+/g, '_'));
  const column = {};
  for (const [key, aliases] of Object.entries(CSV_COLUMNS)) {
    const at = header.findIndex((name) => aliases.includes(name));
    if (at >= 0) column[key] = at;
  }
  if (column.id === undefined || (column.lower === undefined && column.upper === undefined)) {
    return { rows, errors: [{ line: headerAt + 1, message: 'The first line must name the columns: id, lower, upper.' }] };
  }
  for (let i = headerAt + 1; i < lines.length; i++) {
    const raw = lines[i];
    if (!raw.trim() || raw.trim().startsWith('#')) continue;
    const fields = splitCSVLine(raw, delimiter);
    const field = (key) => (column[key] === undefined ? '' : String(fields[column[key]] ?? '').trim());
    const id = field('id');
    const originalId = field('originalId');
    if (!id && !originalId) { errors.push({ line: i + 1, message: 'No reaction id.' }); continue; }
    const lower = boundValue(field('lower'));
    const upper = boundValue(field('upper'));
    if (Number.isNaN(lower) || Number.isNaN(upper)) {
      errors.push({ line: i + 1, message: `${id || originalId}: a bound is not a number.` });
      continue;
    }
    if (lower === null && upper === null) { errors.push({ line: i + 1, message: `${id || originalId}: no bound given.` }); continue; }
    if (lower !== null && upper !== null && lower > upper) {
      errors.push({ line: i + 1, message: `${id || originalId}: the lower bound is above the upper bound.` });
      continue;
    }
    rows.push({ line: i + 1, id, species: field('species'), originalId, lower, upper });
  }
  return { rows, errors };
}

/** A bound cell: null when empty, NaN when it is not a finite number. */
function boundValue(text) {
  if (text === '') return null;
  const number = Number(text);
  return Number.isFinite(number) ? number : NaN;
}

function splitCSVLine(line, delimiter) {
  const out = [];
  let field = '';
  let quoted = false;
  for (let i = 0; i < line.length; i++) {
    const ch = line[i];
    if (quoted) {
      if (ch === '"' && line[i + 1] === '"') { field += '"'; i++; }
      else if (ch === '"') quoted = false;
      else field += ch;
    } else if (ch === '"') quoted = true;
    else if (ch === delimiter) { out.push(field); field = ''; }
    else field += ch;
  }
  out.push(field);
  return out;
}

/**
 * The merged reaction a CSV line means: the merged id itself, or the original
 * id with its species, or an original id that only one species (or the
 * community) has.
 *
 * @returns {{id:string}|{error:'unknown'|'ambiguous', candidates?:string[]}}
 */
export function resolveBoundsReaction(row, community) {
  const reactions = community?.merged?.reactions;
  if (!reactions) return { error: 'unknown' };
  if (row.id && reactions.has(row.id)) return { id: row.id };
  const local = row.originalId || row.id;
  if (row.species) {
    const id = `${row.species}@${local}`;
    return reactions.has(id) ? { id } : { error: 'unknown' };
  }
  if (!local || local.includes('@')) return { error: 'unknown' };
  const candidates = [];
  if (reactions.has(local)) candidates.push(local);
  for (const sp of community.activeSpecies ?? community.species ?? []) {
    const id = `${sp.id}@${local}`;
    if (reactions.has(id)) candidates.push(id);
  }
  if (candidates.length === 1) return { id: candidates[0] };
  return candidates.length ? { error: 'ambiguous', candidates } : { error: 'unknown' };
}
