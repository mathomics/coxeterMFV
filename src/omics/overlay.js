/**
 * Omics overlay utilities.
 *
 * CSV rows are mapped to gene/protein identifiers used by reaction GPR rules.
 * Reaction scores use an intentionally transparent rule:
 *   AND -> minimum subunit abundance
 *   OR  -> maximum isoenzyme abundance
 *
 * Missing values in an AND branch make that branch unknown. OR can still use
 * another measured branch. This module never changes model bounds or fluxes.
 */

const ID_HEADERS = new Set([
  'id', 'gene', 'gene_id', 'geneid', 'protein', 'protein_id', 'proteinid',
  'locus', 'locus_tag', 'identifier',
]);
const SPECIES_HEADERS = new Set(['species', 'organism', 'cell', 'mag', 'genome', 'bin']);

/**
 * Reads a wide omics CSV: an id column (the first recognised id header, else the
 * first column), an optional species column, and one condition per column with
 * any numeric value. Returns a dataset whose `values` map omicsKey(species, id)
 * to {id, species, values: {condition: number}}.
 */
export function parseOmicsCSV(text, { source = 'omics.csv', dataType = 'gene' } = {}) {
  const rows = parseCSV(text);
  if (rows.length < 2) throw new Error('El CSV debe incluir encabezado y al menos una fila de datos.');

  const header = rows[0].map((x) => String(x).trim());
  const normalized = header.map(normalizeHeader);
  let idIndex = normalized.findIndex((h) => ID_HEADERS.has(h));
  if (idIndex < 0) idIndex = 0;
  const speciesIndex = normalized.findIndex((h) => SPECIES_HEADERS.has(h));

  const conditionIndices = [];
  for (let i = 0; i < header.length; i += 1) {
    if (i === idIndex || i === speciesIndex) continue;
    const hasNumeric = rows.slice(1).some((row) => Number.isFinite(toNumber(row[i])));
    if (hasNumeric) conditionIndices.push(i);
  }
  if (!conditionIndices.length) {
    throw new Error('No numeric expression/abundance columns were found.');
  }

  const conditions = conditionIndices.map((i) => header[i] || `condition_${i + 1}`);
  const values = new Map();
  let validRows = 0;

  for (const row of rows.slice(1)) {
    const id = String(row[idIndex] ?? '').trim();
    if (!id) continue;
    const species = speciesIndex >= 0 ? String(row[speciesIndex] ?? '').trim() : '';
    const conditionValues = {};
    let any = false;
    conditionIndices.forEach((columnIndex, j) => {
      const value = toNumber(row[columnIndex]);
      if (Number.isFinite(value)) {
        conditionValues[conditions[j]] = value;
        any = true;
      }
    });
    if (!any) continue;
    values.set(omicsKey(species, id), {
      id,
      species,
      values: conditionValues,
    });
    validRows += 1;
  }

  if (!validRows) throw new Error('The CSV contains no usable numeric values.');

  return {
    source,
    dataType,
    idColumn: header[idIndex],
    speciesColumn: speciesIndex >= 0 ? header[speciesIndex] : null,
    conditions,
    values,
    rowCount: validRows,
  };
}

/** A dataset as plain JSON, with its value map written as a `rows` array. */
export function serializeOmicsDataset(dataset) {
  if (!dataset) return null;
  return {
    source: dataset.source,
    dataType: dataset.dataType,
    idColumn: dataset.idColumn,
    speciesColumn: dataset.speciesColumn,
    conditions: [...dataset.conditions],
    rowCount: dataset.rowCount,
    rows: [...dataset.values.values()].map((row) => ({
      id: row.id,
      species: row.species,
      values: { ...row.values },
    })),
  };
}

/** The inverse of serializeOmicsDataset. Null when there is no usable row. */
export function deserializeOmicsDataset(raw) {
  if (!raw || typeof raw !== 'object' || !Array.isArray(raw.conditions)) return null;
  const values = new Map();
  for (const row of raw.rows ?? []) {
    if (!row?.id) continue;
    values.set(omicsKey(row.species ?? '', row.id), {
      id: String(row.id),
      species: String(row.species ?? ''),
      values: { ...(row.values ?? {}) },
    });
  }
  if (!values.size) return null;
  return {
    source: raw.source ?? 'omics.csv',
    dataType: raw.dataType ?? 'gene',
    idColumn: raw.idColumn ?? 'id',
    speciesColumn: raw.speciesColumn ?? null,
    conditions: [...raw.conditions],
    values,
    rowCount: values.size,
  };
}

/**
 * One score per merged community reaction for a condition, from its species'
 * GPR rule. A gene value is looked up for that species first, then without a
 * species. Reactions with no rule or no usable value are left out.
 */
export function reactionOmicsScores(community, speciesConfig, models, dataset, condition) {
  const out = new Map();
  if (!community?.merged || !dataset || !condition) return out;

  const speciesById = new Map((speciesConfig ?? []).map((sp) => [sp.id, sp]));
  for (const [rid] of community.merged.reactions) {
    const owner = community.reactionOwner.get(rid);
    if (!owner?.speciesId || !owner?.originalId) continue;
    const species = speciesById.get(owner.speciesId);
    const model = species ? models.get(species.file) : null;
    const reaction = model?.reactions.get(owner.originalId);
    const rule = reaction?.geneReactionRule?.trim();
    if (!rule) continue;

    const ast = parseGPR(rule);
    if (!ast) continue;
    const value = evaluateGPR(ast, (geneId) => lookupValue(dataset, owner.speciesId, geneId, condition));
    if (Number.isFinite(value)) out.set(rid, value);
  }
  return out;
}

/**
 * Change from condition A to B per reaction scored in both: log2((B + pc) /
 * (A + pc)) with `pseudocount` pc (default 1), or B - A with metric 'difference'.
 */
export function compareReactionScores(aScores, bScores, {
  metric = 'log2fc',
  pseudocount = 1,
} = {}) {
  const out = new Map();
  const pc = Math.max(0, Number(pseudocount) || 0);

  for (const rid of new Set([...aScores.keys(), ...bScores.keys()])) {
    const a = aScores.get(rid);
    const b = bScores.get(rid);
    if (!Number.isFinite(a) || !Number.isFinite(b)) continue;

    let value;
    if (metric === 'difference') {
      value = b - a;
    } else {
      const aa = a + pc;
      const bb = b + pc;
      if (!(aa > 0) || !(bb > 0)) continue;
      value = Math.log2(bb / aa);
    }
    if (Number.isFinite(value)) out.set(rid, value);
  }
  return out;
}

/**
 * A sequential colour per reaction score, stretched between the 5th and 95th
 * percentiles so a few outliers do not wash out the rest. Returns the colours
 * and the scale's range for the legend.
 */
export function sequentialOmicsColors(scores, { dark = false } = {}) {
  const finite = [...scores.values()].filter(Number.isFinite);
  if (!finite.length) return emptyScale('sequential');
  const lo = quantile(finite, 0.05);
  const hi0 = quantile(finite, 0.95);
  const hi = hi0 > lo ? hi0 : lo + 1;
  const colors = new Map();
  for (const [rid, value] of scores) {
    const t = clamp01((value - lo) / (hi - lo));
    colors.set(rid, interpolateHex(dark ? '#334155' : '#e2e8f0', dark ? '#fb7185' : '#b91c1c', t));
  }
  return {
    kind: 'sequential',
    colors,
    min: Math.min(...finite),
    max: Math.max(...finite),
    domainMin: lo,
    domainMax: hi,
    center: null,
  };
}

/**
 * A diverging colour per reaction change, centred on zero and saturating at the
 * 95th percentile of |change|. Returns the colours and the scale's range.
 */
export function divergingOmicsColors(scores, { dark = false } = {}) {
  const finite = [...scores.values()].filter(Number.isFinite);
  if (!finite.length) return emptyScale('diverging');
  const abs = finite.map(Math.abs);
  const maxAbs = Math.max(quantile(abs, 0.95), 1e-12);
  const colors = new Map();
  const low = dark ? '#60a5fa' : '#2563eb';
  const mid = dark ? '#e2e8f0' : '#f8fafc';
  const high = dark ? '#fb7185' : '#dc2626';
  for (const [rid, value] of scores) {
    const x = Math.max(-maxAbs, Math.min(maxAbs, value));
    colors.set(rid, x < 0
      ? interpolateHex(low, mid, (x + maxAbs) / maxAbs)
      : interpolateHex(mid, high, x / maxAbs));
  }
  return {
    kind: 'diverging',
    colors,
    min: Math.min(...finite),
    max: Math.max(...finite),
    domainMin: -maxAbs,
    domainMax: maxAbs,
    center: 0,
  };
}

/**
 * Parses a GPR rule into a binary tree of {type: 'gene', id} and
 * {type: 'and' | 'or', left, right}, with `and` binding tighter. Null for an
 * empty rule.
 */
export function parseGPR(rule) {
  const tokens = tokenizeGPR(rule);
  if (!tokens.length) return null;
  let pos = 0;

  function primary() {
    const token = tokens[pos];
    if (token === '(') {
      pos += 1;
      const node = orExpr();
      if (tokens[pos] === ')') pos += 1;
      return node;
    }
    if (!token || token === ')' || token === 'and' || token === 'or') return null;
    pos += 1;
    return { type: 'gene', id: token };
  }

  function andExpr() {
    let node = primary();
    while (tokens[pos] === 'and') {
      pos += 1;
      const right = primary();
      if (!right) break;
      node = { type: 'and', left: node, right };
    }
    return node;
  }

  function orExpr() {
    let node = andExpr();
    while (tokens[pos] === 'or') {
      pos += 1;
      const right = andExpr();
      if (!right) break;
      node = { type: 'or', left: node, right };
    }
    return node;
  }

  return orExpr();
}

/** Scores a GPR tree with the AND/OR rule above. `getValue(geneId)` gives a gene's value. Null when unknown. */
export function evaluateGPR(node, getValue) {
  if (!node) return null;
  if (node.type === 'gene') {
    const v = getValue(node.id);
    return Number.isFinite(v) ? v : null;
  }
  const left = evaluateGPR(node.left, getValue);
  const right = evaluateGPR(node.right, getValue);
  if (node.type === 'and') {
    if (!Number.isFinite(left) || !Number.isFinite(right)) return null;
    return Math.min(left, right);
  }
  if (node.type === 'or') {
    if (Number.isFinite(left) && Number.isFinite(right)) return Math.max(left, right);
    if (Number.isFinite(left)) return left;
    if (Number.isFinite(right)) return right;
    return null;
  }
  return null;
}

function lookupValue(dataset, speciesId, geneId, condition) {
  const exact = dataset.values.get(omicsKey(speciesId, geneId));
  const global = dataset.values.get(omicsKey('', geneId));
  const row = exact ?? global;
  const value = row?.values?.[condition];
  return Number.isFinite(value) ? value : null;
}

/** Dataset key of a gene or protein id within a species ('' for no species). */
export function omicsKey(species, id) {
  return `${String(species ?? '').trim()}\u0000${String(id ?? '').trim()}`;
}

function tokenizeGPR(rule) {
  return String(rule ?? '')
    .replace(/\(/g, ' ( ')
    .replace(/\)/g, ' ) ')
    .trim()
    .split(/\s+/)
    .filter(Boolean)
    .map((token) => {
      const lower = token.toLowerCase();
      return lower === 'and' || lower === 'or' ? lower : token;
    });
}

function parseCSV(text) {
  const rows = [];
  let row = [];
  let field = '';
  let quoted = false;
  const input = String(text ?? '').replace(/^\uFEFF/, '');

  for (let i = 0; i < input.length; i += 1) {
    const ch = input[i];
    if (quoted) {
      if (ch === '"' && input[i + 1] === '"') {
        field += '"';
        i += 1;
      } else if (ch === '"') {
        quoted = false;
      } else {
        field += ch;
      }
    } else if (ch === '"') {
      quoted = true;
    } else if (ch === ',') {
      row.push(field);
      field = '';
    } else if (ch === '\n') {
      row.push(field.replace(/\r$/, ''));
      if (row.some((x) => String(x).trim() !== '')) rows.push(row);
      row = [];
      field = '';
    } else {
      field += ch;
    }
  }
  row.push(field.replace(/\r$/, ''));
  if (row.some((x) => String(x).trim() !== '')) rows.push(row);
  return rows;
}

function toNumber(value) {
  const text = String(value ?? '').trim();
  if (!text) return NaN;
  const n = Number(text);
  return Number.isFinite(n) ? n : NaN;
}

function normalizeHeader(value) {
  return String(value ?? '').trim().toLowerCase().replace(/[\s.-]+/g, '_');
}

function emptyScale(kind) {
  return {
    kind,
    colors: new Map(),
    min: null,
    max: null,
    domainMin: null,
    domainMax: null,
    center: kind === 'diverging' ? 0 : null,
  };
}

function quantile(values, q) {
  const sorted = [...values].sort((a, b) => a - b);
  if (!sorted.length) return NaN;
  const index = (sorted.length - 1) * q;
  const lo = Math.floor(index);
  const hi = Math.ceil(index);
  if (lo === hi) return sorted[lo];
  const t = index - lo;
  return sorted[lo] * (1 - t) + sorted[hi] * t;
}

function interpolateHex(a, b, t) {
  const aa = hexToRgb(a);
  const bb = hexToRgb(b);
  const mix = [0, 1, 2].map((i) => Math.round(aa[i] + (bb[i] - aa[i]) * clamp01(t)));
  return `#${mix.map((v) => v.toString(16).padStart(2, '0')).join('')}`;
}

function hexToRgb(hex) {
  const clean = hex.replace('#', '');
  return [0, 2, 4].map((i) => parseInt(clean.slice(i, i + 2), 16));
}

function clamp01(x) {
  return Math.min(1, Math.max(0, x));
}
