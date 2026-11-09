/**
 * User-facing biological model editing.
 *
 * These helpers mutate a source Model (one species), never the merged
 * CommunityModel directly. CommunityModel/FBA are rebuilt afterwards by Store.
 */

import { Metabolite, Reaction } from './model.js';
import { evaluateGeneRule, geneRuleGenes } from './gene-rules.js';

function inputNumber(value) {
  if (typeof value === 'number') {
    return value;
  }
  if (typeof value === 'string' && value.trim() !== '') {
    return Number(value);
  }
  return NaN;
}

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

  const out = new Map();
  for (const [idRaw, coefficientRaw] of entries) {
    const id = String(idRaw ?? '').trim();
    const coefficient = inputNumber(coefficientRaw);
    if (!id) throw new Error('A metabolite in the stoichiometry has no ID.');
    if (!Number.isFinite(coefficient)) throw new Error(`Non-numeric coefficient for ${id}.`);
    if (Math.abs(coefficient) < 1e-15) throw new Error(`The coefficient of ${id} cannot be zero.`);
    out.set(id, (out.get(id) ?? 0) + coefficient);
  }

  for (const [id, coefficient] of out) {
    if (!Number.isFinite(coefficient)) {
      throw new Error(`The net coefficient of ${id} must be finite.`);
    }
    if (Math.abs(coefficient) < 1e-15) {
      out.delete(id);
    }
  }
  if (!out.size) throw new Error('The net stoichiometry came out empty.');
  return Object.fromEntries(out);
}

/** The gene ids of a rule, using the same operators as knockout evaluation. */
export function geneIdsFromGPR(rule) {
  return geneRuleGenes(rule);
}

/**
 * Adds a metabolite {id, name, compartment, formula?, charge?} to a species
 * model. The id must be new and the compartment one the model has. Returns the
 * Metabolite and throws on invalid input.
 */
export function addMetaboliteToModel(model, spec) {
  if (!model) throw new Error('No model is selected.');
  const id = validId(spec?.id, 'metabolite');
  if (model.metabolites.has(id)) throw new Error(`Metabolite ${id} already exists.`);

  const compartment = String(spec?.compartment ?? '').trim();
  if (!compartment || !model.compartmentIds().includes(compartment)) {
    throw new Error(`Invalid compartment: ${compartment || 'empty'}.`);
  }

  const name = String(spec?.name ?? '').trim();
  if (!name) throw new Error('The metabolite name is required.');

  let charge = 0;
  if (spec?.charge !== null && spec?.charge !== undefined
    && !(typeof spec.charge === 'string' && spec.charge.trim() === '')) {
    charge = inputNumber(spec.charge);
  }
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
    const coefficient = inputNumber(coefficientRaw);
    if (!Number.isFinite(coefficient) || Math.abs(coefficient) < 1e-15) {
      throw new Error(`Invalid coefficient for ${mid}.`);
    }
    metabolites[mid] = coefficient;
  }
  if (!Object.keys(metabolites).length) throw new Error('The reaction must contain at least one metabolite.');

  const lower = inputNumber(spec?.lowerBound);
  const upper = inputNumber(spec?.upperBound);
  if (!Number.isFinite(lower) || !Number.isFinite(upper)) {
    throw new Error('Lower and upper bounds must be numbers.');
  }
  if (lower > upper) throw new Error('The lower bound cannot be greater than the upper bound.');

  const gpr = String(spec?.geneReactionRule ?? spec?.gpr ?? '').trim();
  try {
    evaluateGeneRule(gpr);
  } catch (error) {
    throw new Error(`Invalid gene rule: ${error.message}`);
  }
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
