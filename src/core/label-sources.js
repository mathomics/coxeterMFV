const SOURCE_NAMES = {
  base: 'Base label',
  id: 'Model ID',
  name: 'Model name',
  bigg: 'BiGG',
  metanetx: 'MetaNetX',
  kegg: 'KEGG',
  metacyc: 'MetaCyc',
  modelseed: 'ModelSEED',
  chebi: 'ChEBI',
  hmdb: 'HMDB',
  pubchem: 'PubChem',
  rhea: 'Rhea',
  ec: 'EC number',
  inchikey: 'InChIKey',
};

/** Display name of a label source key, or the key itself when it has none. */
export function labelSourceName(source) {
  return SOURCE_NAMES[source] ?? source;
}

function sourceKey(field, kind) {
  const key = field.toLowerCase().replace(/[^a-z0-9]/g, '');
  if (key.includes('pathway') || key.includes('subsystem')) {
    return null;
  }
  if (/^(met|rxn)?names?$/.test(key)) {
    return 'name';
  }
  if (kind === 'metabolite' && key.startsWith('rxn')) {
    return null;
  }
  if (kind === 'reaction' && key.startsWith('met') && !key.startsWith('metanetx') && !key.startsWith('metacyc')) {
    return null;
  }
  for (const source of ['bigg', 'metanetx', 'kegg', 'metacyc', 'modelseed', 'chebi', 'hmdb', 'pubchem', 'rhea', 'inchikey']) {
    if (key.includes(source)) {
      if (key.endsWith('names') || key.endsWith('name')) {
        return field;
      }
      return source;
    }
  }
  if (/^(met|rxn)?seed(compound|reaction|id)?$/.test(key)) {
    return 'modelseed';
  }
  if (key.includes('biocyc')) {
    return 'metacyc';
  }
  if (kind === 'reaction' && /^(rxn)?ec(numbers?|code)?$/.test(key)) {
    return 'ec';
  }
  if (/^(met|rxn).*(names?|aliases|ids?)$/.test(key)) {
    return field;
  }
  return null;
}

function textValues(value) {
  const values = Array.isArray(value) ? value.flat(Infinity) : [value];
  const result = [];
  for (const item of values) {
    if (typeof item !== 'string') {
      continue;
    }
    const text = item.trim();
    if (text && !result.includes(text)) {
      result.push(text);
    }
  }
  return result;
}

function addSource(sources, key, value) {
  const values = textValues(value);
  if (!key || values.length === 0) {
    return;
  }
  const existing = sources[key] ?? [];
  sources[key] = [...new Set([...existing, ...values])];
}

/**
 * Model-level label arrays (`metNames`, `rxnKEGGID`, ...) of a raw model, as
 * [{source, values}]. Only `met*` or `rxn*` fields (by `kind`) with one entry
 * per entity (`count`) and a recognised source are kept.
 */
export function modelLabelVectors(raw, kind, count) {
  const prefix = kind === 'metabolite' ? 'met' : 'rxn';
  const vectors = [];
  for (const [field, values] of Object.entries(raw)) {
    if (!field.toLowerCase().startsWith(prefix) || !Array.isArray(values) || values.length !== count) {
      continue;
    }
    const source = sourceKey(field, kind);
    if (source) {
      vectors.push({ source, values });
    }
  }
  return vectors;
}

/**
 * Alternative labels of one raw metabolite or reaction, as {source: [unique
 * values]}. They come from its `labelSources`, its own fields (except `name`),
 * its annotation and notes, and entry `index` of the model-level `vectors`.
 */
export function entityLabelSources(raw, kind, vectors = [], index = 0) {
  const sources = {};
  for (const [key, value] of Object.entries(raw.labelSources ?? {})) {
    addSource(sources, key, value);
  }
  for (const fields of [raw, raw.annotation, raw.annotations, raw.notes]) {
    for (const [field, value] of Object.entries(fields ?? {})) {
      if (fields === raw && field === 'name') {
        continue;
      }
      addSource(sources, sourceKey(field, kind), value);
    }
  }
  for (const vector of vectors) {
    addSource(sources, vector.source, vector.values[index]);
  }
  return sources;
}

/**
 * Every label a map object can show, keyed by source: `id` (without the cell
 * prefix), `name`, `base` for metabolites, and each alternative label joined
 * with '; '. `entity` is the model's metabolite or reaction, when known.
 */
export function objectLabelSources(object, entity, kind) {
  let localId = String(object.localId ?? entity?.id ?? object.id ?? '');
  const prefix = object.cellId ? `${object.cellId}@` : '';
  if (prefix && localId.startsWith(prefix)) {
    localId = localId.slice(prefix.length);
  }
  const sources = {
    id: localId,
    name: entity?.name || object.name || localId,
  };
  if (kind === 'metabolite') {
    const base = object.base || entity?.base || localId;
    // A label shows one name, not the model's whole list of synonyms.
    sources.name = readableName(sources.name, [localId, base]);
    sources.base = readableBase(base, entity?.name || object.name);
  }
  const alternatives = entity?.labelSources ?? object.labelSources ?? {};
  for (const [source, value] of Object.entries(alternatives)) {
    const values = textValues(value);
    if (values.length > 0) {
      sources[source] = values.join('; ');
    }
  }
  return sources;
}

/**
 * A metabolite's base label, made readable where the model's id is not.
 * A ModelSEED accession (`cpd00023[c_Er]`) says nothing to a reader, so its
 * model name stands in (`L-Glutamate`). An SBML-escaped id (`M_ival_c[c_Ef]`)
 * loses the prefix and the compartment letter the bracket repeats
 * (`ival[c_Ef]`). Any other base is kept as it is.
 */
export function readableBase(base, name = '') {
  const text = String(base ?? '');
  const [, core, tag = ''] = /^(.*?)(\[[^\]]*\])?$/.exec(text);
  if (/^cpd\d{5}$/.test(core) && name && !/^cpd\d{5}/.test(name)) return readableName(name, [core]);
  const escaped = /^M_(.+)_([a-z][a-z0-9]?)$/.exec(core);
  if (escaped && (!tag || tag.startsWith(`[${escaped[2]}`))) return escaped[1] + tag;
  return text;
}

const EXCHANGE_FOR = /^exchange for\s+/i;

/**
 * One readable name out of a model name that lists synonyms joined by `|`, as
 * some reconstructions write them (`CELB|Cellobiose|cellobiose`,
 * `Exchange for Serine|L-Serine`, `Fe3+|fe3`).
 *
 * Parts that are codes rather than names are dropped: the metabolite's own id
 * or base (`ids`, with or without a `[compartment]` tag), short all-capital
 * codes (`CELB`), escaped SBML ids (`M_Raffinose_C18H32O16`) and ModelSEED
 * accessions; so are an exchange reaction's "Exchange for ..." wording and
 * repeats that differ only in case. Of the rest, the first that starts with a
 * capital letter and carries no bracketed or comma-separated qualifier wins
 * (`Raffinose`, `L-Serine`, `K+`), then the first that is capitalised at all
 * (`1,5-Diaminopentane`), then the first. A name without `|` only loses an
 * "Exchange for" in front; an empty one gives the first id.
 */
export function readableName(name, ids = []) {
  const text = String(name ?? '').trim();
  if (!text) return String(ids.find((id) => id) ?? '');
  if (!text.includes('|')) return text.replace(EXCHANGE_FOR, '') || text;
  const own = new Set();
  for (const id of ids) {
    if (!id) continue;
    const lower = String(id).toLowerCase();
    own.add(lower);
    own.add(lower.replace(/\[[^\]]*\]$/, ''));
  }
  const parts = text.split('|').map((part) => part.trim()).filter(Boolean);
  const isCode = (part) => own.has(part.toLowerCase()) || /^[A-Z][A-Z0-9]{1,5}$/.test(part)
    || /^M_/.test(part) || /^cpd\d{5}/.test(part);
  const seen = new Set();
  const names = [];
  for (const part of parts) {
    const key = part.toLowerCase();
    if (EXCHANGE_FOR.test(part) || isCode(part) || seen.has(key)) continue;
    seen.add(key);
    names.push(part);
  }
  if (!names.length) return parts.length ? parts[0].replace(EXCHANGE_FOR, '') || parts[0] : text;
  const capitalised = (part) => {
    const letter = /[A-Za-z]/.exec(part)?.[0];
    return Boolean(letter) && letter === letter.toUpperCase();
  };
  return names.find((part) => /^[A-Z]/.test(part) && !/[(),]/.test(part))
    ?? names.find(capitalised)
    ?? names[0];
}

/** The label of a map object for the chosen source, or its base label (metabolites) or id when that source has none. */
export function objectLabel(object, entity, kind, source) {
  const sources = objectLabelSources(object, entity, kind);
  const fallback = kind === 'metabolite' ? sources.base : sources.id;
  return sources[source] || fallback;
}
