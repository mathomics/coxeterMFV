/**
 * User-facing biological model editing.
 *
 * These helpers mutate a source Model (one species), never the merged
 * CommunityModel directly. CommunityModel/FBA are rebuilt afterwards by Store.
 */

import { Metabolite, Reaction } from './model.js';

const RESERVED_GPR = new Set(['and', 'or', 'not', 'true', 'false']);

/**
 * Reads a stoichiometry typed by the user, either a JSON object
 * {metabolite: coefficient} or terms `met:coef` / `met coef` separated by
 * newlines, commas or semicolons. Repeated metabolites are summed and terms
 * that cancel out are dropped. Returns {metaboliteId: coefficient} and throws
 * a readable Error for anything it cannot use.
 */
export function parseStoichiometry(text) {
  const raw = String(text ?? '').trim();
  if (!raw) throw new Error('The stoichiometry cannot be empty.');

  let entries;
  if (raw.startsWith('{')) {
    let doc;
    try {
      doc = JSON.parse(raw);
    } catch (error) {
      throw new Error(`Invalid stoichiometry JSON: ${error.message}`);
    }
    if (!doc || typeof doc !== 'object' || Array.isArray(doc)) {
      throw new Error('Stoichiometry JSON must be an object of { metabolite: coefficient }.');
    }
    entries = Object.entries(doc);
  } else {
    entries = raw
      .split(/[\n,;]+/)
      .map((part) => part.trim())
      .filter(Boolean)
      .map((part) => {
        const colon = /^(.+?)\s*:\s*([+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?)$/.exec(part);
        if (colon) return [colon[1].trim(), colon[2]];
        const spaced = /^(\S+)\s+([+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?)$/.exec(part);
        if (spaced) return [spaced[1], spaced[2]];
        throw new Error(`Could not read “${part}”. Use metabolite:coefficient.`);
      });
  }

  if (!entries.length) throw new Error('The stoichiometry has no terms.');

  const out = {};
  for (const [idRaw, coefficientRaw] of entries) {
    const id = String(idRaw ?? '').trim();
    const coefficient = Number(coefficientRaw);
    if (!id) throw new Error('A metabolite in the stoichiometry has no ID.');
    if (!Number.isFinite(coefficient)) throw new Error(`Non-numeric coefficient for ${id}.`);
    if (Math.abs(coefficient) < 1e-15) throw new Error(`The coefficient of ${id} cannot be zero.`);
    out[id] = (out[id] ?? 0) + coefficient;
  }

  for (const [id, coefficient] of Object.entries(out)) {
    if (Math.abs(coefficient) < 1e-15) delete out[id];
  }
  if (!Object.keys(out).length) throw new Error('The net stoichiometry came out empty.');
  return out;
}

/** Unique gene ids of a gene-reaction rule, without the boolean keywords. */
export function geneIdsFromGPR(rule) {
  const text = String(rule ?? '').trim();
  if (!text) return [];
  const tokens = text.match(/[^\s()]+/g) ?? [];
  return [...new Set(tokens
    .map((token) => token.trim())
    .filter((token) => token && !RESERVED_GPR.has(token.toLowerCase())))];
}

/**
 * Adds a metabolite {id, name, compartment, formula?, charge?} to a species
 * model. The id must be new and the compartment one the model has. Returns the
 * Metabolite and throws on invalid input.
 */
export function addMetaboliteToModel(model, spec) {
  if (!model) throw new Error('No model is selected.');
  const id = validId(spec?.id, 'metabolito');
  if (model.metabolites.has(id)) throw new Error(`Metabolite ${id} already exists.`);

  const compartment = String(spec?.compartment ?? '').trim();
  if (!compartment || !model.compartmentIds().includes(compartment)) {
    throw new Error(`Invalid compartment: ${compartment || 'empty'}.`);
  }

  const name = String(spec?.name ?? '').trim();
  if (!name) throw new Error('The metabolite name is required.');

  const charge = spec?.charge === '' || spec?.charge === null || spec?.charge === undefined
    ? 0
    : Number(spec.charge);
  if (!Number.isFinite(charge)) throw new Error('The charge must be numeric.');

  const met = new Metabolite({
    id,
    name,
    compartment,
    formula: String(spec?.formula ?? '').trim(),
    charge,
  }, model);

  model.metabolites.set(id, met);
  model.finalize();
  return met;
}

/**
 * Adds a reaction {id, name?, metabolites, lowerBound, upperBound,
 * geneReactionRule? (or gpr), subsystem?} to a species model. `metabolites` is
 * a stoichiometry object or text for parseStoichiometry, and every metabolite
 * must already exist. Genes named in the rule are added to the model. Returns
 * the Reaction and throws on invalid input.
 */
export function addReactionToModel(model, spec) {
  if (!model) throw new Error('No model is selected.');
  const id = validId(spec?.id, 'reaction');
  if (model.reactions.has(id)) throw new Error(`Reaction ${id} already exists.`);

  const metabolites = typeof spec?.metabolites === 'string'
    ? parseStoichiometry(spec.metabolites)
    : { ...(spec?.metabolites ?? {}) };

  for (const [mid, coefficientRaw] of Object.entries(metabolites)) {
    if (!model.metabolites.has(mid)) {
      throw new Error(`The reaction references a metabolite that does not exist: ${mid}.`);
    }
    const coefficient = Number(coefficientRaw);
    if (!Number.isFinite(coefficient) || Math.abs(coefficient) < 1e-15) {
      throw new Error(`Invalid coefficient for ${mid}.`);
    }
    metabolites[mid] = coefficient;
  }
  if (!Object.keys(metabolites).length) throw new Error('The reaction must contain at least one metabolite.');

  const lower = Number(spec?.lowerBound);
  const upper = Number(spec?.upperBound);
  if (!Number.isFinite(lower) || !Number.isFinite(upper)) {
    throw new Error('Lower and upper bounds must be numbers.');
  }
  if (lower > upper) throw new Error('The lower bound cannot be greater than the upper bound.');

  const gpr = String(spec?.geneReactionRule ?? spec?.gpr ?? '').trim();
  const reaction = new Reaction({
    id,
    name: String(spec?.name ?? '').trim() || id,
    metabolites,
    lower_bound: lower,
    upper_bound: upper,
    gene_reaction_rule: gpr,
    subsystem: String(spec?.subsystem ?? '').trim(),
  }, model);

  model.reactions.set(id, reaction);
  ensureGenes(model, geneIdsFromGPR(gpr));
  model.finalize();
  return reaction;
}

function ensureGenes(model, ids) {
  if (!Array.isArray(model.genes)) model.genes = [];
  const existing = new Set(model.genes.map((gene) =>
    typeof gene === 'string' ? gene : gene?.id
  ).filter(Boolean));

  for (const id of ids) {
    if (existing.has(id)) continue;
    model.genes.push({ id, name: id });
    existing.add(id);
  }
}

function validId(value, kind) {
  const id = String(value ?? '').trim();
  if (!id) throw new Error(`The ${kind} ID is required.`);
  if (/\s/.test(id)) throw new Error(`The ${kind} ID cannot contain spaces.`);
  return id;
}
