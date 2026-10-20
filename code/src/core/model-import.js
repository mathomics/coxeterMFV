import { extractIdentifiersOrg } from './provenance.js';

/**
 * Import the models a user supplies: COBRA JSON, SBML Level 3/FBC, a COBRA
 * Toolbox MATLAB file (.mat, v5/v7) or a COBRA Toolbox Excel workbook (.xlsx).
 *
 * The browser-facing code hands this module {name, text} records, or
 * {name, bytes} for the binary formats. Everything here is pure and
 * independently testable: parsing, structural validation, Model construction,
 * stable species/model identifiers and medium inference.
 */

import { Model, inferCompartment } from './model.js';
import { unmangleId } from './sbml.js';
import { readMat5 } from '../import/mat5.js';
import { readWorkbookAsync } from './xlsx-core.js';
import { evaluateGeneRule, geneRuleGenes } from './gene-rules.js';

const MODEL_EXTENSION = /\.(json|xml|sbml|mat|xlsx|xlsm)$/i;

/**
 * Parses and validates a COBRA JSON model. Returns a Model, and throws an Error
 * prefixed with `filename` when the file is not a usable model.
 */
export function parseModelJSON(text, filename = 'model.json') {
  let raw;
  try {
    raw = JSON.parse(text);
  } catch (error) {
    throw new Error(`${filename}: invalid JSON (${error.message})`);
  }

  validateRawModel(raw, filename);

  let model;
  try {
    model = new Model(raw);
  } catch (error) {
    throw new Error(`${filename}: ${error.message}`);
  }

  if (!model.reactions.size) {
    throw new Error(`${filename}: the model contains no reactions.`);
  }
  if (!model.metabolites.size) {
    throw new Error(`${filename}: the model contains no metabolites.`);
  }

  return model;
}

/** Parses and validates an SBML Level 3/FBC model into a Model. Throws like parseModelJSON. */
export function parseModelSBML(text, filename = 'model.xml') {
  const raw = sbmlToCobra(text, filename);
  validateRawModel(raw, filename);
  try { return new Model(raw); } catch (error) { throw new Error(`${filename}: ${error.message}`); }
}

/** Parses a model as SBML (by .xml/.sbml extension or a leading `<`) or else as COBRA JSON. */
export function parseModelFile(text, filename = 'model.json') {
  const trimmed = String(text ?? '').trim();
  const lower = filename.toLowerCase();
  if (lower.endsWith('.xml') || lower.endsWith('.sbml') || trimmed.startsWith('<')) return parseModelSBML(text, filename);
  return parseModelJSON(text, filename);
}

/**
 * What an uploaded file is, by its extension and else by its first bytes:
 * 'mat', 'xlsx', or 'text' (SBML or COBRA JSON). Throws for the old binary
 * Excel format.
 */
export function modelFileKind(filename = '', bytes = null) {
  const lower = String(filename).toLowerCase();
  if (lower.endsWith('.mat')) return 'mat';
  if (lower.endsWith('.xlsx') || lower.endsWith('.xlsm')) return 'xlsx';
  if (lower.endsWith('.xls')) {
    throw new Error(`${filename}: Excel 97-2003 workbooks (.xls) cannot be read; save it as .xlsx.`);
  }
  if (bytes?.length >= 4) {
    if (String.fromCharCode(...bytes.subarray(0, 6)) === 'MATLAB') return 'mat';
    if (bytes[0] === 0x50 && bytes[1] === 0x4b && bytes[2] === 3 && bytes[3] === 4) return 'xlsx';
  }
  return 'text';
}

/** Whether a picked file has to be read as bytes rather than text. */
export function isBinaryModelFile(filename = '') {
  return /\.(mat|xlsx|xlsm|xls)$/i.test(String(filename));
}

/**
 * Parses one uploaded file, {name, text} or {name, bytes}, in whichever format
 * it is (see modelFileKind). Resolves to a Model; rejects like parseModelJSON.
 */
export async function parseModelUpload(file, index = 0) {
  const filename = file?.name || `model-${index + 1}.json`;
  const bytes = file?.bytes ? asBytes(file.bytes) : null;
  const kind = modelFileKind(filename, bytes);
  if (kind === 'mat') return parseModelMAT(bytes ?? new Uint8Array(), filename);
  if (kind === 'xlsx') return parseModelXLSX(bytes ?? new Uint8Array(), filename);
  return parseModelFile(file?.text ?? new TextDecoder().decode(bytes ?? new Uint8Array()), filename);
}

/** prepareImportedModels for uploads in any format: parses each file (see parseModelUpload) first. */
export async function prepareModelUploads(files) {
  if (!Array.isArray(files) || files.length === 0) return prepareImportedModels(files);
  const parsed = await Promise.all(files.map(async (file, index) => ({
    name: file.name || `model-${index + 1}.json`,
    model: await parseModelUpload(file, index),
  })));
  return prepareImportedModels(parsed);
}

/**
 * Turns uploaded {name, text} files (or {name, model} already parsed) into
 * community entries: the parsed model, a unique species id and model key, an
 * equal starting abundance and a short summary. Returns {entries, medium},
 * with the medium inferred from all models.
 */
export function prepareImportedModels(files) {
  if (!Array.isArray(files) || files.length === 0) {
    throw new Error('Select at least one model: COBRA JSON, SBML, MATLAB (.mat) or Excel (.xlsx).');
  }

  const takenSpecies = new Set();
  const takenKeys = new Set();

  const entries = files.map((file, index) => {
    const filename = file.name || `model-${index + 1}.json`;
    const model = file.model instanceof Model ? file.model : parseModelFile(file.text, filename);
    const stem = filename.replace(MODEL_EXTENSION, '');
    const baseSpecies = safeId(model.id || stem || `sp${index + 1}`);
    const speciesId = uniqueId(baseSpecies || `sp${index + 1}`, takenSpecies);
    const baseKey = `upload:${safeId(stem || model.id || speciesId)}`;
    const modelKey = uniqueId(baseKey, takenKeys);

    return {
      filename,
      model,
      modelKey,
      speciesId,
      abundance: 1 / files.length,
      enabled: true,
      summary: {
        reactions: model.reactions.size,
        metabolites: model.metabolites.size,
        compartments: model.compartmentIds().length,
        biomass: model.biomassReactionId(),
      },
    };
  });

  return {
    entries,
    medium: inferCommunityMedium(entries.map((x) => x.model)),
  };
}

/**
 * Medium the models are open to: {base metabolite id: supply rate}, the largest
 * uptake any single-metabolite exchange allows (its lower bound, made positive).
 */
export function inferCommunityMedium(models) {
  const medium = {};

  for (const model of models) {
    for (const reaction of model.exchangeReactions()) {
      if (!(reaction.lowerBound < 0)) continue;

      const entries = Object.keys(reaction.metabolites);
      if (entries.length !== 1) continue;

      const met = model.metabolites.get(entries[0]);
      if (!met) continue;

      medium[met.base] = Math.max(
        Number(medium[met.base] ?? 0),
        Math.abs(reaction.lowerBound),
      );
    }
  }

  return medium;
}

function validateRawModel(raw, filename) {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) {
    throw new Error(`${filename}: the root of the file must be a JSON object.`);
  }
  // Both files are "the JSON" the app offers to download, so they get swapped.
  // Saying "missing the metabolites array" about a saved map helps nobody.
  if (raw.format === 'EscherMCMCMap') {
    throw new Error(`${filename}: this is a saved map, not a model. Load it from File ▸ Load map.`);
  }
  if (!Array.isArray(raw.metabolites)) {
    throw new Error(`${filename}: missing the "metabolites" array.`);
  }
  if (!Array.isArray(raw.reactions)) {
    throw new Error(`${filename}: missing the "reactions" array.`);
  }

  assertUniqueIds(raw.metabolites, 'metabolite', filename);
  assertUniqueIds(raw.reactions, 'reaction', filename);

  for (const reaction of raw.reactions) {
    if (!reaction || typeof reaction !== 'object' || !reaction.id) {
      throw new Error(`${filename}: a reaction has no valid id.`);
    }
    if (!reaction.metabolites || typeof reaction.metabolites !== 'object' || Array.isArray(reaction.metabolites)) {
      throw new Error(`${filename}: reaction ${reaction.id} has no valid "metabolites" object.`);
    }
    for (const [mid, coefficient] of Object.entries(reaction.metabolites)) {
      if (!Number.isFinite(Number(coefficient))) {
        throw new Error(`${filename}: non-numeric coefficient for ${mid} in ${reaction.id}.`);
      }
    }
  }
}

function assertUniqueIds(items, kind, filename) {
  const seen = new Set();
  for (const item of items) {
    const id = item?.id;
    if (!id || typeof id !== 'string') {
      throw new Error(`${filename}: a ${kind} has no valid id.`);
    }
    if (seen.has(id)) {
      throw new Error(`${filename}: duplicate ${kind} id: ${id}.`);
    }
    seen.add(id);
  }
}

function safeId(value) {
  const id = String(value ?? '')
    .trim()
    .replace(MODEL_EXTENSION, '')
    .replace(/[^A-Za-z0-9_.-]+/g, '_')
    .replace(/^_+|_+$/g, '');
  return id || 'model';
}

function uniqueId(base, taken) {
  let id = base;
  let i = 2;
  while (taken.has(id)) id = `${base}_${i++}`;
  taken.add(id);
  return id;
}


// SBML Level 3/FBC

/**
 * Convert the subset of SBML Level 3 + FBC used by constraint-based metabolic
 * models into Coxeter's COBRA-compatible document. Namespace prefixes are
 * intentionally accepted rather than hard-coded (fbc:, fbc2:, etc.).
 */
function sbmlToCobra(text, filename) {
  const xml = String(text ?? '').replace(/^\uFEFF/, '');
  if (!/<(?:\w+:)?sbml\b/i.test(xml) || !/<(?:\w+:)?model\b/i.test(xml)) {
    throw new Error(`${filename}: does not look like a valid SBML document.`);
  }

  const modelTag = firstOpenTag(xml, 'model');
  const modelAttrs = attrs(modelTag);
  const parameters = new Map();
  for (const tag of openTags(xml, 'parameter')) {
    const a = attrs(tag);
    if (a.id && finiteNumber(a.value)) parameters.set(a.id, Number(a.value));
  }

  const compartments = {};
  for (const tag of openTags(xml, 'compartment')) {
    const a = attrs(tag);
    if (a.id) compartments[unmangleId(a.id, 'C_')] = a.name || a.id;
  }

  const metabolites = [];
  // Genome-scale SBML files such as Yeast9 contain thousands of species, and
  // rescanning the whole XML document once per species for its notes and
  // annotations would make large imports O(n²). The paired element bodies are
  // indexed once, and the open-tag pass still reads self-closing species.
  const speciesBlocks = indexBlocksByAttr(xml, 'species', 'id');
  for (const tag of openTags(xml, 'species')) {
    const a = attrs(tag);
    if (!a.id) continue;
    // The block lookup needs the id as written. Everything stored uses the id
    // the papers use. SBML forbids most of BiGG's punctuation, so an exporter
    // writes `M_12ppd__45__R_e` and this is where that is undone. Without it a
    // perfectly good upload arrives with every identifier renamed.
    const block = speciesBlocks.get(a.id) ?? '';
    const notes = extractNotes(block);
    metabolites.push({
      id: unmangleId(a.id, 'M_'),
      name: a.name || a.id,
      compartment: unmangleId(a.compartment || 'c', 'C_'),
      // A formula fbc cannot hold (fractional counts, an underscore) travels in
      // the notes under COBRA's own key, which is also where Level 2 keeps them.
      formula: attrAny(a, 'chemicalFormula')
        || /FORMULA:\s*([^\s<]+)/.exec(notes.text ?? '')?.[1] || '',
      charge: numberOr(attrAny(a, 'charge'), 0),
      notes,
      annotation: extractIdentifiersOrg(block),
    });
  }

  const genes = [];
  const geneBlocks = indexBlocksByAttr(xml, 'geneProduct', 'id');
  for (const tag of openTags(xml, 'geneProduct')) {
    const a = attrs(tag);
    const id = attrAny(a, 'id');
    if (!id) continue;
    const geneBlock = geneBlocks.get(id) ?? '';
    const label = attrAny(a, 'label') || unmangleId(id, 'G_');
    genes.push({ id: label, name: attrAny(a, 'name') || label, annotation: extractIdentifiersOrg(geneBlock), notes: extractNotes(geneBlock) });
  }

  const objective = parseFbcObjective(xml);
  const reactions = [];
  for (const block of elementBlocks(xml, 'reaction')) {
    const a = attrs(block.open);
    if (!a.id) continue;
    const stoich = {};
    const reactants = innerOf(block.body, 'listOfReactants');
    const products = innerOf(block.body, 'listOfProducts');
    addSpeciesRefs(stoich, reactants, -1);
    addSpeciesRefs(stoich, products, 1);

    const reversible = String(a.reversible ?? 'true').toLowerCase() !== 'false';
    const lowerRef = attrAny(a, 'lowerFluxBound');
    const upperRef = attrAny(a, 'upperFluxBound');
    const lower = resolveBound(lowerRef, parameters, reversible ? -1000 : 0);
    const upper = resolveBound(upperRef, parameters, 1000);
    const gprBlock = innerOf(block.body, 'geneProductAssociation');
    const gpr = gprBlock ? parseGeneAssociation(gprBlock) : '';

    reactions.push({
      id: unmangleId(a.id, 'R_'),
      name: a.name || a.id,
      metabolites: stoich,
      lower_bound: lower,
      upper_bound: upper,
      objective_coefficient: Number(objective[unmangleId(a.id, 'R_')] ?? 0),
      gene_reaction_rule: gpr,
      subsystem: '',
      notes: extractNotes(block.body),
      annotation: extractIdentifiersOrg(block.body),
    });
  }

  if (!metabolites.length) throw new Error(`${filename}: SBML contains no species/metabolites.`);
  if (!reactions.length) throw new Error(`${filename}: SBML contains no reactions.`);

  return {
    id: modelAttrs.id ? unmangleId(modelAttrs.id, 'M_') : filename.replace(/\.(xml|sbml)$/i, ''),
    name: modelAttrs.name || modelAttrs.id || filename,
    version: 'SBML L3 FBC',
    compartments,
    metabolites,
    reactions,
    genes,
    objective,
  };
}

function parseFbcObjective(xml) {
  const out = {};
  let objectives = innerOf(xml, 'listOfObjectives') || '';
  if (!objectives) return out;
  const listTag = firstOpenTag(xml, 'listOfObjectives');
  const active = attrAny(attrs(listTag), 'activeObjective');
  let selected = null;
  for (const block of elementBlocks(objectives, 'objective')) {
    const a = attrs(block.open);
    const id = attrAny(a, 'id');
    if (!selected || (active && id === active)) selected = block;
    if (active && id === active) break;
  }
  if (!selected) return out;
  for (const tag of openTags(selected.body, 'fluxObjective')) {
    const a = attrs(tag);
    const rid = attrAny(a, 'reaction');
    const coef = numberOr(attrAny(a, 'coefficient'), 1);
    if (rid) out[unmangleId(rid, 'R_')] = coef;
  }
  return out;
}

function addSpeciesRefs(target, xml, sign) {
  if (!xml) return;
  for (const tag of openTags(xml, 'speciesReference')) {
    const a = attrs(tag);
    if (!a.species) continue;
    const coefficient = numberOr(a.stoichiometry, 1) * sign;
    const id = unmangleId(a.species, 'M_');
    target[id] = (target[id] || 0) + coefficient;
  }
}

function parseGeneAssociation(xml) {
  const tokens = [];
  const re = /<(\/)?(?:\w+:)?(and|or|geneProductRef)\b([^>]*)\/?\s*>/gi;
  const stack = [];
  let m;
  while ((m = re.exec(xml))) {
    const closing = Boolean(m[1]);
    const type = m[2].toLowerCase();
    if (type === 'geneproductref' && !closing) {
      const gene = attrAny(attrs(m[0]), 'geneProduct');
      if (gene) tokens.push({ depth: stack.length, text: unmangleId(gene, 'G_') });
    } else if (!closing) {
      stack.push({ type, start: tokens.length });
    } else {
      const group = stack.pop();
      if (!group) continue;
      const children = tokens.splice(group.start);
      if (children.length) tokens.push({
        depth: stack.length,
        text: children.length === 1 ? children[0].text : `(${children.map(x => x.text).join(` ${group.type} ` )})`,
      });
    }
  }
  return tokens.length ? tokens[tokens.length - 1].text.replace(/^\((.*)\)$/, '$1') : '';
}

function resolveBound(ref, parameters, fallback) {
  if (ref == null || ref === '') return fallback;
  if (parameters.has(ref)) return parameters.get(ref);
  return finiteNumber(ref) ? Number(ref) : fallback;
}

function attrAny(object, localName) {
  if (object[localName] != null) return object[localName];
  const key = Object.keys(object).find((k) => k.split(':').pop() === localName);
  return key ? object[key] : undefined;
}

function attrs(tag) {
  const out = {};
  const re = /([:\w.-]+)\s*=\s*(["'])(.*?)\2/g;
  let m;
  while ((m = re.exec(tag || ''))) out[m[1]] = decodeXML(m[3]);
  return out;
}

function openTags(xml, localName) {
  const re = new RegExp(`<(?:\\w+:)?${localName}\\b[^>]*>`, 'gi');
  return String(xml ?? '').match(re) || [];
}

function firstOpenTag(xml, localName) {
  return openTags(xml, localName)[0] || '';
}

function elementBlocks(xml, localName) {
  const re = new RegExp(`(<(?:\\w+:)?${localName}\\b[^>]*>)([\\s\\S]*?)<\\/(?:\\w+:)?${localName}\\s*>`, 'gi');
  const out = [];
  let m;
  while ((m = re.exec(String(xml ?? '')))) out.push({ open: m[1], body: m[2] });
  return out;
}

function innerOf(xml, localName) {
  return elementBlocks(xml, localName)[0]?.body || '';
}

function decodeXML(value) {
  return String(value ?? '')
    .replace(/&quot;/g, '"').replace(/&apos;/g, "'")
    .replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&amp;/g, '&');
}

function finiteNumber(value) {
  return value !== '' && value != null && Number.isFinite(Number(value));
}

function numberOr(value, fallback) {
  return finiteNumber(value) ? Number(value) : fallback;
}

function indexBlocksByAttr(xml, localName, attr) {
  const out = new Map();
  for (const block of elementBlocks(xml, localName)) {
    const a = attrs(block.open);
    const value = attrAny(a, attr);
    if (value) out.set(value, block.body);
  }
  return out;
}

function blockForId(xml, localName, id) { return blockForAttr(xml, localName, 'id', id); }
function blockForAttr(xml, localName, attr, value) {
  return elementBlocks(xml, localName).find((b) => {
    const a=attrs(b.open); return attrAny(a,attr)===value;
  })?.body || '';
}
function extractNotes(xml) {
  const body=innerOf(xml,'notes'); if(!body)return {};
  const text=decodeXML(body.replace(/<[^>]+>/g,' ').replace(/\s+/g,' ').trim());
  return text ? { text } : {};
}


// COBRA Toolbox MAT and Excel

/**
 * Parses a COBRA Toolbox MATLAB file (MAT v5, or v7 with compression) into a
 * Model: the first structure in it with rxns, mets and S. A v7.3 file is HDF5,
 * which this reader cannot open, and the error says how to save it again.
 */
export async function parseModelMAT(bytes, filename = 'model.mat') {
  const data = asBytes(bytes);
  if (isMatHdf5(data)) {
    throw new Error(`${filename}: this is a MATLAB v7.3 (HDF5) file, which cannot be read here. `
      + "In MATLAB, save the model again as v7 (save('model.mat', 'model', '-v7')) or export it with writeCbModel as SBML.");
  }
  let variables;
  try {
    variables = await readMat5(data);
  } catch (error) {
    throw new Error(`${filename}: ${error.message}`);
  }
  const found = findCobraStruct(variables);
  if (!found) throw new Error(`${filename}: no COBRA model in the file (a structure with rxns, mets and S).`);
  const raw = cobraStructToDocument(found.value, filename, found.name);
  validateRawModel(raw, filename);
  return buildModel(raw, filename);
}

/**
 * Parses a COBRA Toolbox Excel workbook (writeCbModel's "Reaction List" and
 * "Metabolite List" sheets) into a Model. Column names are matched loosely
 * (case, spaces and common synonyms), and a metabolite only named in an
 * equation is still created.
 */
export async function parseModelXLSX(bytes, filename = 'model.xlsx') {
  let workbook;
  try {
    workbook = await readWorkbookAsync(asBytes(bytes));
  } catch (error) {
    throw new Error(`${filename}: not a readable Excel workbook (${error.message}).`);
  }
  const raw = cobraWorkbookToDocument(workbook, filename);
  validateRawModel(raw, filename);
  return buildModel(raw, filename);
}

function buildModel(raw, filename) {
  let model;
  try { model = new Model(raw); } catch (error) { throw new Error(`${filename}: ${error.message}`); }
  if (!model.reactions.size) throw new Error(`${filename}: the model contains no reactions.`);
  if (!model.metabolites.size) throw new Error(`${filename}: the model contains no metabolites.`);
  return model;
}

function asBytes(input) {
  if (input instanceof Uint8Array) return input;
  if (input instanceof ArrayBuffer) return new Uint8Array(input);
  if (ArrayBuffer.isView(input)) return new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
  throw new Error('expected the bytes of the file');
}

/** A v7.3 MAT file: version 0x0200 in its header, or the HDF5 signature after the 512-byte user block. */
function isMatHdf5(bytes) {
  if (bytes.length < 128) return false;
  const header = String.fromCharCode(...bytes.subarray(0, 10));
  const version = bytes[126] === 0x49 ? bytes[124] | (bytes[125] << 8) : (bytes[124] << 8) | bytes[125];
  const hdf5 = bytes.length >= 516 && bytes[512] === 0x89 && String.fromCharCode(...bytes.subarray(513, 516)) === 'HDF';
  return header === 'MATLAB 7.3' || version === 0x0200 || hdf5;
}

const isCobraStruct = (value) => Boolean(value) && typeof value === 'object' && !Array.isArray(value)
  && !ArrayBuffer.isView(value) && 'rxns' in value && 'mets' in value && 'S' in value;

/** The first variable that is a COBRA structure, or the first one inside a cell or structure array. */
function findCobraStruct(variables) {
  for (const [name, value] of Object.entries(variables)) {
    if (isCobraStruct(value)) return { name, value };
    if (Array.isArray(value)) {
      const index = value.findIndex(isCobraStruct);
      if (index >= 0) return { name: `${name}{${index + 1}}`, value: value[index] };
    }
  }
  return null;
}

const list = (value) => (value == null ? [] : Array.isArray(value) || ArrayBuffer.isView(value) ? Array.from(value) : [value]);
/** A MATLAB value as text: a string, a number, or the text of each item joined with `; `. */
const text = (value) => {
  if (typeof value === 'string') return value;
  if (typeof value === 'number') return String(value);
  if (Array.isArray(value) || ArrayBuffer.isView(value)) return Array.from(value).map(text).filter(Boolean).join('; ');
  return '';
};

/**
 * A COBRA Toolbox metabolite id as the other importers write it: the trailing
 * `[c]` becomes `_c`, which is how the Toolbox's own SBML export spells it, so
 * one model reads the same from .mat, .xlsx and .xml.
 */
function cobraMetaboliteId(id) {
  const trimmed = String(id ?? '').trim();
  const match = /^(.+)\[([^\]\s]+)\]$/.exec(trimmed);
  return match ? { id: `${match[1]}_${match[2]}`, compartment: match[2] } : { id: trimmed, compartment: null };
}

/** The converted id of every metabolite; one that would collide with another keeps its own spelling. */
function metaboliteIds(originals) {
  const taken = new Set();
  const listed = new Set(originals);
  return originals.map((original) => {
    const converted = cobraMetaboliteId(original);
    const clash = taken.has(converted.id) || (converted.id !== original && listed.has(converted.id));
    const id = clash ? original : converted.id;
    taken.add(id);
    return { id, compartment: converted.compartment, original };
  });
}

/** A gene rule from grRules, or from `rules` (x(3) & x(5)) and `genes` when grRules is missing or unreadable. */
function structGeneRule(grRule, encoded, genes) {
  const readable = (rule) => { try { evaluateGeneRule(rule); return true; } catch { return false; } };
  if (grRule && readable(grRule)) return grRule;
  if (encoded) {
    const translated = encoded
      .replace(/x\((\d+)\)/g, (_, n) => genes[Number(n) - 1] ?? `x(${n})`)
      .replace(/&+/g, ' and ').replace(/\|+/g, ' or ')
      .replace(/\s+/g, ' ').trim();
    if (readable(translated)) return translated;
  }
  return grRule;
}

function cobraStructToDocument(struct, filename, variable) {
  const rxns = list(struct.rxns).map(text);
  const metIds = list(struct.mets).map(text);
  const m = metIds.length;
  const n = rxns.length;
  const column = sparseColumns(struct.S, m, n, filename);
  const lb = list(struct.lb);
  const ub = list(struct.ub);
  const c = list(struct.c);
  const rev = list(struct.rev);
  const genes = list(struct.genes).map(text);
  const geneNames = list(struct.geneNames).map(text);
  const grRules = list(struct.grRules).map(text);
  const encodedRules = list(struct.rules).map(text);
  const names = list(struct.rxnNames).map(text);
  const subsystems = list(struct.subSystems).map(text);
  const metNames = list(struct.metNames).map(text);
  const formulas = list(struct.metFormulas).map(text);
  const charges = list(struct.metCharges ?? struct.metCharge);
  const comps = list(struct.comps).map(text);
  const compNames = list(struct.compNames).map(text);
  const metComps = list(struct.metComps);

  const compartments = {};
  comps.forEach((id, i) => { if (id) compartments[id] = compNames[i] || id; });
  const ids = metaboliteIds(metIds);
  const metabolites = ids.map((info, i) => {
    const compartment = info.compartment ?? comps[Number(metComps[i]) - 1] ?? inferCompartment(info.id);
    compartments[compartment] ??= compartment;
    const charge = Number(charges[i]);
    return {
      id: info.id, name: metNames[i] || info.original, compartment,
      formula: formulas[i] || '', charge: Number.isFinite(charge) ? charge : 0,
    };
  });

  const reactions = rxns.map((id, j) => {
    const stoich = {};
    for (const [row, value] of column(j)) {
      if (!Number.isFinite(value)) throw new Error(`${filename}: reaction ${id} has a non-numeric coefficient.`);
      if (value) stoich[ids[row].id] = (stoich[ids[row].id] ?? 0) + value;
    }
    const lower = Number(lb[j]);
    const upper = Number(ub[j]);
    return {
      id, name: names[j] || id, metabolites: stoich,
      lower_bound: Number.isFinite(lower) ? lower : (Number(rev[j]) ? -1000 : 0),
      upper_bound: Number.isFinite(upper) ? upper : 1000,
      objective_coefficient: Number(c[j]) || 0,
      gene_reaction_rule: structGeneRule(grRules[j] ?? '', encodedRules[j] ?? '', genes),
      subsystem: subsystems[j] || '',
    };
  });

  const doc = {
    id: text(struct.modelID ?? struct.id) || filename.replace(MODEL_EXTENSION, '') || variable,
    name: text(struct.modelName ?? struct.name) || text(struct.description) || variable,
    version: 'COBRA Toolbox MAT',
    compartments, metabolites, reactions,
    genes: genes.map((gene, i) => ({ id: gene, name: geneNames[i] || gene })),
  };
  // Further met*/rxn* text vectors (KEGG ids, EC numbers, ...) are label
  // sources the Model reads off the document, as it does for any COBRA file.
  const handled = new Set(['mets', 'metNames', 'metFormulas', 'metCharges', 'metCharge', 'metComps', 'rxns', 'rxnNames']);
  for (const [field, value] of Object.entries(struct)) {
    if (handled.has(field) || !/^(met|rxn)/i.test(field)) continue;
    const values = list(value);
    const count = /^met/i.test(field) ? m : n;
    if (values.length === count && values.every((item) => typeof item === 'string' || Array.isArray(item))) doc[field] = values.map(text);
  }
  return doc;
}

/** S sparse ({shape, ir, jc, values}) or dense (column-major) as a function giving reaction j's [row, value] entries. */
function sparseColumns(S, m, n, filename) {
  if (S?.ir && S?.jc && S?.values) {
    if (S.shape?.[0] !== m || S.shape?.[1] !== n) {
      throw new Error(`${filename}: S is ${S.shape?.join('×')}, but there are ${m} metabolites and ${n} reactions.`);
    }
    return (j) => {
      const out = [];
      for (let k = S.jc[j]; k < S.jc[j + 1]; k++) out.push([S.ir[k], S.values[k]]);
      return out;
    };
  }
  if (ArrayBuffer.isView(S) && S.length === m * n) {
    return (j) => {
      const out = [];
      for (let i = 0; i < m; i++) if (S[i + j * m]) out.push([i, S[i + j * m]]);
      return out;
    };
  }
  throw new Error(`${filename}: S is not a ${m}×${n} matrix.`);
}

// Column names are compared lower-cased, with everything but letters and digits taken out.
const REACTION_COLUMNS = {
  id: ['abbreviation', 'id', 'rxn', 'rxns', 'reactionid', 'rxnid', 'reactionabbreviation', 'abbrev'],
  name: ['description', 'name', 'rxnname', 'rxnnames', 'reactionname', 'reactiondescription'],
  formula: ['reaction', 'formula', 'equation', 'reactionformula', 'rxnformula', 'reactionequation'],
  gpr: ['gpr', 'grrule', 'grrules', 'generule', 'generules', 'genereactionrule', 'genereactionassociation'],
  subsystem: ['subsystem', 'subsystems', 'pathway'],
  reversible: ['reversible', 'rev', 'reversibility'],
  lower: ['lowerbound', 'lowerbounds', 'lb', 'lower', 'min'],
  upper: ['upperbound', 'upperbounds', 'ub', 'upper', 'max'],
  objective: ['objective', 'objectivecoefficient', 'c', 'obj'],
  ec: ['ecnumber', 'ecnumbers', 'ec', 'rxnecnumbers'],
};
const METABOLITE_COLUMNS = {
  id: ['abbreviation', 'id', 'met', 'mets', 'metaboliteid', 'metid', 'abbrev'],
  name: ['description', 'name', 'metname', 'metnames', 'metabolitename', 'fullname'],
  charged: ['chargedformula', 'formula', 'metformula', 'metformulas'],
  neutral: ['neutralformula'],
  charge: ['charge', 'metcharge', 'metcharges'],
  compartment: ['compartment', 'comp', 'compartments', 'metcompartment'],
  kegg: ['keggid', 'kegg', 'metkeggid'],
  pubchem: ['pubchemid', 'pubchem'],
  chebi: ['chebiid', 'chebi'],
  hmdb: ['hmdbid', 'hmdb'],
};
const columnKey = (name) => String(name ?? '').toLowerCase().replace(/[^a-z0-9]/g, '');

/**
 * A sheet as {field: value} rows, read under the first of its first ten rows
 * that names an id column and one more known column; null when none does.
 */
function readSheet(workbook, sheet, columns) {
  const rows = [];
  let header = null;
  let seen = 0;
  for (const row of workbook.rows(sheet)) {
    if (!header) {
      if (++seen > 10) break;
      const fields = {};
      for (const [letter, value] of Object.entries(row)) {
        if (letter === '__row') continue;
        const key = columnKey(value);
        const field = Object.keys(columns).find((name) => columns[name].includes(key));
        if (field && !Object.values(fields).includes(field)) fields[letter] = field;
      }
      if (Object.values(fields).includes('id') && Object.keys(fields).length >= 2) header = fields;
      continue;
    }
    const out = {};
    for (const [letter, field] of Object.entries(header)) if (row[letter] !== undefined) out[field] = String(row[letter]).trim();
    if (out.id) rows.push(out);
  }
  return header ? rows : null;
}

/** The rows of the sheet called one of `names`, or else of the first sheet that reads under `columns` (with equations, for reactions). */
function findSheet(workbook, names, columns, needs = null) {
  const named = workbook.sheetNames.find((name) => names.includes(columnKey(name)));
  if (named) return readSheet(workbook, named, columns);
  for (const name of workbook.sheetNames) {
    const rows = readSheet(workbook, name, columns);
    if (rows?.length && (!needs || rows.some((row) => row[needs]))) return rows;
  }
  return null;
}

const ARROW = /\s*(<==>|<=>|<->|<--|<-|-->|->|=>)\s*/;

/**
 * A COBRA formula as {stoichiometry, reversible}: `2 atp[c] + glc[c] -> ...`,
 * `<=>` for reversible, coefficients bare or in parentheses, a backward arrow
 * turned round, and the old `[c] : a + b -> c` form that puts every
 * metabolite in one compartment. Throws for text it cannot read.
 */
export function parseCobraFormula(formula) {
  let rest = String(formula ?? '').trim();
  const prefix = /^\[([^\]]+)\]\s*:\s*/.exec(rest);
  if (prefix) rest = rest.slice(prefix[0].length);
  const arrow = ARROW.exec(rest);
  if (!arrow) throw new Error(`no reaction arrow in "${formula}"`);
  let left = rest.slice(0, arrow.index);
  let right = rest.slice(arrow.index + arrow[0].length);
  if (arrow[1].startsWith('<') && !arrow[1].endsWith('>')) [left, right] = [right, left];
  const stoichiometry = {};
  const side = (part, sign) => {
    for (const term of part.split(/\s+\+\s+/).map((t) => t.trim()).filter(Boolean)) {
      const match = /^(?:\(?\s*(\d*\.?\d+(?:[eE][-+]?\d+)?)\s*\)?\s+)?(\S+)$/.exec(term);
      if (!match) throw new Error(`cannot read "${term}" in "${formula}"`);
      const coefficient = match[1] === undefined ? 1 : Number(match[1]);
      const met = prefix && !/\[[^\]]+\]$/.test(match[2]) ? `${match[2]}[${prefix[1]}]` : match[2];
      stoichiometry[met] = (stoichiometry[met] ?? 0) + sign * coefficient;
    }
  };
  side(left, -1);
  side(right, 1);
  return { stoichiometry, reversible: ['<==>', '<=>', '<->'].includes(arrow[1]) };
}

const truthy = (value) => /^(1|true|yes|y|reversible)$/i.test(String(value ?? '').trim());

function cobraWorkbookToDocument(workbook, filename) {
  const reactionRows = findSheet(workbook, ['reactionlist', 'reactions', 'reaction', 'rxns'], REACTION_COLUMNS, 'formula');
  if (!reactionRows?.length) {
    throw new Error(`${filename}: no reaction sheet with an Abbreviation and a Reaction column (the COBRA Toolbox "Reaction List").`);
  }
  const metaboliteRows = findSheet(workbook, ['metabolitelist', 'metabolites', 'metabolite', 'mets'], METABOLITE_COLUMNS) ?? [];

  const compartments = {};
  const metabolites = [];
  const known = new Set();
  const vectors = { metKEGGID: [], metPubChemID: [], metChEBIID: [], metHMDBID: [] };
  const addMetabolite = (original, row = {}) => {
    const info = cobraMetaboliteId(original);
    if (known.has(info.id)) return info.id;
    known.add(info.id);
    // The Compartment column holds an id or a name; the id in brackets wins.
    const listed = row.compartment ?? '';
    const compartment = info.compartment ?? (/^[A-Za-z0-9_]{1,3}$/.test(listed) ? listed : inferCompartment(info.id));
    if (!compartments[compartment] || compartments[compartment] === compartment) {
      compartments[compartment] = listed && listed !== compartment ? listed : compartment;
    }
    const charge = Number(row.charge);
    metabolites.push({
      id: info.id, name: row.name || String(original).trim(), compartment,
      formula: row.charged || row.neutral || '', charge: Number.isFinite(charge) ? charge : 0,
    });
    vectors.metKEGGID.push(row.kegg ?? '');
    vectors.metPubChemID.push(row.pubchem ?? '');
    vectors.metChEBIID.push(row.chebi ?? '');
    vectors.metHMDBID.push(row.hmdb ?? '');
    return info.id;
  };
  for (const row of metaboliteRows) addMetabolite(row.id, row);

  const reactions = [];
  const ec = [];
  for (const row of reactionRows) {
    let parsed;
    try {
      parsed = parseCobraFormula(row.formula ?? '');
    } catch (error) {
      throw new Error(`${filename}: reaction ${row.id}: ${error.message}.`);
    }
    const stoich = {};
    for (const [met, coefficient] of Object.entries(parsed.stoichiometry)) {
      const id = addMetabolite(met);
      stoich[id] = (stoich[id] ?? 0) + coefficient;
    }
    const reversible = row.reversible !== undefined ? truthy(row.reversible) : parsed.reversible;
    const lower = Number(row.lower);
    const upper = Number(row.upper);
    reactions.push({
      id: row.id, name: row.name || row.id, metabolites: stoich,
      lower_bound: row.lower !== undefined && Number.isFinite(lower) ? lower : (reversible ? -1000 : 0),
      upper_bound: row.upper !== undefined && Number.isFinite(upper) ? upper : 1000,
      objective_coefficient: Number(row.objective) || 0,
      gene_reaction_rule: row.gpr ?? '',
      subsystem: row.subsystem ?? '',
    });
    ec.push(row.ec ?? '');
  }

  const genes = [...new Set(reactions.flatMap((reaction) => geneRuleGenes(reaction.gene_reaction_rule)))];
  const stem = filename.replace(MODEL_EXTENSION, '') || 'model';
  const doc = {
    id: stem, name: stem, version: 'COBRA Toolbox XLSX',
    compartments, metabolites, reactions, genes: genes.map((id) => ({ id, name: id })),
  };
  if (ec.some(Boolean)) doc.rxnECNumbers = ec;
  for (const [field, values] of Object.entries(vectors)) if (values.some(Boolean)) doc[field] = values;
  return doc;
}
