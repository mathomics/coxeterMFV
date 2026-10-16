import { extractIdentifiersOrg } from './provenance.js';

/**
 * Import ordinary COBRA JSON or SBML Level 3/FBC models supplied by the user.
 *
 * The browser-facing code hands this module {name,text} records. Everything
 * here is pure and independently testable: JSON parsing, structural validation,
 * Model construction, stable species/model identifiers and medium inference.
 */

import { Model } from './model.js';
import { unmangleId } from './sbml.js';

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
 * Turns uploaded {name, text} files into community entries: the parsed model,
 * a unique species id and model key, an equal starting abundance and a short
 * summary. Returns {entries, medium}, with the medium inferred from all models.
 */
export function prepareImportedModels(files) {
  if (!Array.isArray(files) || files.length === 0) {
    throw new Error('Select at least one COBRA JSON or SBML model.');
  }

  const takenSpecies = new Set();
  const takenKeys = new Set();

  const entries = files.map((file, index) => {
    const filename = file.name || `model-${index + 1}.json`;
    const model = parseModelFile(file.text, filename);
    const stem = filename.replace(/\.(json|xml|sbml)$/i, '');
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
    .replace(/\.(json|xml|sbml)$/i, '')
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
