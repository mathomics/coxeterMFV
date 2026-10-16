/**
 * COBRA-compatible metabolic model.
 *
 * Reads plain COBRApy JSON (`cobra.io.save_json_model`) and adds a few optional
 * extension keys that COBRApy ignores on load:
 *
 *   compartment_tree : { "<child>": "<parent>|null" }  explicit organelle nesting
 *   cell             : { name, organism, color, gram, shape }
 *   metabolites[].primary : boolean  force a metabolite to be a map hub
 *   reactions[].layout    : { rank, lane }  coarse placement hints for the map
 *
 * Nothing here is required: a stock BiGG model loads and renders with inferred
 * nesting and an automatic layout.
 */

import { buildCompartmentTree } from './compartments.js';
import { entityLabelSources, modelLabelVectors } from './label-sources.js';

const ELEMENT_RE = /([A-Z][a-z]?)(\d*\.?\d*)/g;

/** Parse "C6H12O6" into { C: 6, H: 12, O: 6 }. Returns null for unusable input. */
export function parseFormula(formula) {
  if (!formula || typeof formula !== 'string') return null;
  const f = formula.trim();
  if (!f || f === 'null' || /[()\[\]]/.test(f)) return null;
  const out = {};
  let consumed = 0;
  ELEMENT_RE.lastIndex = 0;
  let match;
  while ((match = ELEMENT_RE.exec(f)) !== null) {
    if (match[0] === '') { ELEMENT_RE.lastIndex++; continue; }
    consumed += match[0].length;
    const count = match[2] === '' ? 1 : Number(match[2]);
    if (!Number.isFinite(count)) return null;
    out[match[1]] = (out[match[1]] || 0) + count;
  }
  // Reject formulas with characters we silently skipped (R groups, charges...).
  if (consumed !== f.length) return null;
  return out;
}

/** Strip the trailing `_<compartment>` from a BiGG metabolite id. */
export function stripCompartment(metaboliteId, compartmentId) {
  if (compartmentId && metaboliteId.endsWith(`_${compartmentId}`)) {
    return metaboliteId.slice(0, -(compartmentId.length + 1));
  }
  const m = /^(.*)_([a-z][a-z0-9]?)$/.exec(metaboliteId);
  return m ? m[1] : metaboliteId;
}

/** Guess a compartment id from a BiGG metabolite id suffix. */
export function inferCompartment(metaboliteId) {
  const m = /_([a-z][a-z0-9]?)$/.exec(metaboliteId);
  return m ? m[1] : 'c';
}

/** Currency metabolites that are duplicated per reaction instead of shared. */
export const DEFAULT_CURRENCY = new Set([
  'h', 'h2o', 'atp', 'adp', 'amp', 'pi', 'ppi', 'nad', 'nadh', 'nadp', 'nadph',
  'co2', 'o2', 'coa', 'nh4', 'q8', 'q8h2', 'q6', 'q6h2', 'fad', 'fadh2',
  'so4', 'hco3', 'na1', 'k', 'mg2', 'cl', 'accoa', 'fdxo', 'fdxr', 'pqh2', 'pq',
  'h2', 'photon',
]);

/** A reaction of a Model, read from a COBRA JSON reaction (snake_case or camelCase keys). */
export class Reaction {
  constructor(raw, model, labelSources = entityLabelSources(raw, 'reaction')) {
    this.id = raw.id;
    this.name = raw.name || raw.id;
    this.metabolites = { ...(raw.metabolites || {}) };
    this.lowerBound = numOr(raw.lower_bound, raw.lowerBound, -1000);
    this.upperBound = numOr(raw.upper_bound, raw.upperBound, 1000);
    this.objectiveCoefficient = numOr(raw.objective_coefficient, raw.objectiveCoefficient, 0);
    this.subsystem = raw.subsystem || '';
    this.geneReactionRule = raw.gene_reaction_rule || raw.geneReactionRule || '';
    this.layout = raw.layout || null;
    this.notes = raw.notes || {};
    this.annotation = raw.annotation || raw.annotations || {};
    this.labelSources = labelSources;
    // COBRA JSON has a well-established "subsystem" field but no single
    // universal pathway field. Preserve explicit pathway metadata when present
    // and accept the common notes/annotation variants without inventing one.
    this.pathway = firstText(
      raw.pathway,
      this.notes.pathway,
      this.notes.Pathway,
      this.annotation.pathway,
      this.annotation.pathways,
    );
    this.model = model;
    /** Filled in by Model#finalize(). */
    this.compartments = [];
    this.kind = 'metabolic'; // metabolic | transport | exchange | biomass | sink
  }

  get reversible() { return this.lowerBound < 0 && this.upperBound > 0; }

  /** Metabolites consumed at positive flux. */
  substrates() { return Object.entries(this.metabolites).filter(([, c]) => c < 0); }
  /** Metabolites produced at positive flux. */
  products() { return Object.entries(this.metabolites).filter(([, c]) => c > 0); }

  /** Human readable equation, e.g. "atp_c + f6p_c --> adp_c + fdp_c + h_c". */
  equation() {
    const fmt = (list) => list
      .map(([id, c]) => (Math.abs(c) === 1 ? id : `${round(Math.abs(c))} ${id}`))
      .join(' + ') || '∅';
    const arrow = this.reversible ? '<=>' : (this.upperBound > 0 ? '-->' : '<--');
    return `${fmt(this.substrates())} ${arrow} ${fmt(this.products())}`;
  }

  toJSON() {
    const out = {
      id: this.id,
      name: this.name,
      metabolites: this.metabolites,
      lower_bound: this.lowerBound,
      upper_bound: this.upperBound,
      gene_reaction_rule: this.geneReactionRule,
    };
    if (this.subsystem) out.subsystem = this.subsystem;
    if (this.pathway) out.pathway = this.pathway;
    if (this.objectiveCoefficient) out.objective_coefficient = this.objectiveCoefficient;
    if (this.layout) out.layout = this.layout;
    if (this.notes && Object.keys(this.notes).length) out.notes = this.notes;
    if (this.annotation && Object.keys(this.annotation).length) out.annotation = this.annotation;
    if (Object.keys(this.labelSources).length > 0) {
      out.labelSources = this.labelSources;
    }
    return out;
  }
}

/** A metabolite of a Model, read from a COBRA JSON metabolite. */
export class Metabolite {
  constructor(raw, model, labelSources = entityLabelSources(raw, 'metabolite')) {
    this.id = raw.id;
    this.name = raw.name || raw.id;
    this.formula = raw.formula || '';
    this.charge = numOr(raw.charge, null, 0);
    this.compartment = raw.compartment || inferCompartment(raw.id);
    // A merged community writes every id as `<species>@<id>`, which the id alone
    // cannot be told apart from. Whoever builds such a model says what the
    // metabolite is called. Otherwise the base is read off the id.
    this.base = typeof raw.base === 'string' && raw.base
      ? raw.base
      : stripCompartment(raw.id, this.compartment);
    this.explicitPrimary = typeof raw.primary === 'boolean' ? raw.primary : null;
    this.notes = raw.notes || {};
    this.annotation = raw.annotation || raw.annotations || {};
    this.labelSources = labelSources;
    this.model = model;
    /** Reaction ids touching this metabolite. Filled in by Model#finalize(). */
    this.reactions = [];
  }

  get elements() { return parseFormula(this.formula); }

  /**
   * Primary metabolites get one shared node on the map. Currency metabolites get
   * one small node per reaction so the map does not collapse into a hairball.
   */
  get isPrimary() {
    if (this.explicitPrimary !== null) return this.explicitPrimary;
    if (DEFAULT_CURRENCY.has(this.base)) return false;
    return this.reactions.length <= 6;
  }

  toJSON() {
    const out = {
      id: this.id, name: this.name, compartment: this.compartment,
      formula: this.formula, charge: this.charge,
    };
    if (this.explicitPrimary !== null) out.primary = this.explicitPrimary;
    if (this.notes && Object.keys(this.notes).length) out.notes = this.notes;
    if (this.annotation && Object.keys(this.annotation).length) out.annotation = this.annotation;
    if (Object.keys(this.labelSources).length > 0) {
      out.labelSources = this.labelSources;
    }
    return out;
  }
}

/**
 * A COBRA model: metabolites and reactions keyed by id, genes, compartment
 * names, the optional extension keys and the objective.
 */
export class Model {
  /** @param {object} raw COBRA JSON document */
  constructor(raw) {
    this.id = raw.id || 'model';
    this.name = raw.name || this.id;
    this.version = raw.version || '1';
    this.compartmentNames = { ...(raw.compartments || {}) };
    this.cell = {
      name: raw.cell?.name || this.name,
      organism: raw.cell?.organism || '',
      color: raw.cell?.color || '#334155',
      shape: raw.cell?.shape || 'rounded',
      gram: raw.cell?.gram || null,
      note: raw.cell?.note || '',
    };
    this.explicitTree = raw.compartment_tree || null;

    this.metabolites = new Map();
    const metaboliteVectors = modelLabelVectors(raw, 'metabolite', raw.metabolites?.length ?? 0);
    let metaboliteIndex = 0;
    for (const m of raw.metabolites || []) {
      const sources = entityLabelSources(m, 'metabolite', metaboliteVectors, metaboliteIndex);
      const met = new Metabolite(m, this, sources);
      this.metabolites.set(met.id, met);
      metaboliteIndex += 1;
    }

    this.reactions = new Map();
    const reactionVectors = modelLabelVectors(raw, 'reaction', raw.reactions?.length ?? 0);
    let reactionIndex = 0;
    for (const r of raw.reactions || []) {
      const sources = entityLabelSources(r, 'reaction', reactionVectors, reactionIndex);
      const rxn = new Reaction(r, this, sources);
      this.reactions.set(rxn.id, rxn);
      reactionIndex += 1;
    }

    this.genes = (raw.genes || []).map((g) => typeof g === 'string' ? { id:g, name:g, annotation:{}, notes:{} } : ({ ...g, id:String(g.id), name:g.name || g.id, annotation:g.annotation || g.annotations || {}, notes:g.notes || {} }));
    this.rawObjective = raw.objective || null;
    this.finalize();
  }

  /**
   * Links metabolites to their reactions, sets each reaction's compartments and
   * kind, and names any unnamed compartment after its id. Call it after adding
   * or changing reactions. Throws when a reaction uses an unknown metabolite.
   */
  finalize() {
    for (const met of this.metabolites.values()) met.reactions = [];

    for (const rxn of this.reactions.values()) {
      const comps = new Set();
      for (const mid of Object.keys(rxn.metabolites)) {
        const met = this.metabolites.get(mid);
        if (!met) {
          throw new Error(`Reaction ${rxn.id} references unknown metabolite ${mid}`);
        }
        met.reactions.push(rxn.id);
        comps.add(met.compartment);
      }
      rxn.compartments = [...comps].sort();
      rxn.kind = classify(rxn, comps);
    }

    for (const id of this.compartmentIds()) {
      if (!this.compartmentNames[id]) this.compartmentNames[id] = id;
    }
  }

  /** Declared compartments plus any a metabolite uses, sorted. */
  compartmentIds() {
    const ids = new Set(Object.keys(this.compartmentNames));
    for (const met of this.metabolites.values()) ids.add(met.compartment);
    return [...ids].sort();
  }

  compartmentTree() {
    return buildCompartmentTree(this.compartmentIds(), this.explicitTree, this.compartmentNames);
  }

  /** Reactions whose objective coefficient is non-zero. */
  objectiveReactions() {
    if (this.rawObjective && typeof this.rawObjective === 'object') {
      return Object.entries(this.rawObjective)
        .filter(([id]) => this.reactions.has(id))
        .map(([id, coef]) => ({ id, coefficient: Number(coef) }));
    }
    const out = [];
    for (const r of this.reactions.values()) {
      if (r.objectiveCoefficient) out.push({ id: r.id, coefficient: r.objectiveCoefficient });
    }
    return out;
  }

  /** Best guess at the growth/biomass reaction id. */
  biomassReactionId() {
    const objective = this.objectiveReactions();
    if (objective.length) return objective[0].id;
    for (const r of this.reactions.values()) {
      if (r.kind === 'biomass') return r.id;
    }
    return null;
  }

  /** Exchange reactions (single metabolite, boundary). */
  exchangeReactions() {
    return [...this.reactions.values()].filter((r) => r.kind === 'exchange');
  }

  metabolitesIn(compartmentId) {
    return [...this.metabolites.values()].filter((m) => m.compartment === compartmentId);
  }

  /**
   * Elemental + charge balance check. Exchanges, sinks and lumped biomass
   * pseudo-reactions are skipped because they are unbalanced by construction.
   * @returns {Array<{id:string, imbalance:Object<string,number>}>}
   */
  checkMassBalance(tolerance = 1e-6) {
    const problems = [];
    for (const rxn of this.reactions.values()) {
      if (rxn.kind === 'exchange' || rxn.kind === 'biomass' || rxn.kind === 'sink') continue;
      const totals = {};
      let ok = true;
      for (const [mid, coef] of Object.entries(rxn.metabolites)) {
        const met = this.metabolites.get(mid);
        const els = met.elements;
        if (!els) { ok = false; break; }
        for (const [el, n] of Object.entries(els)) totals[el] = (totals[el] || 0) + n * coef;
        totals.charge = (totals.charge || 0) + met.charge * coef;
      }
      if (!ok) continue; // Unparseable formula: cannot judge, do not cry wolf.
      const imbalance = {};
      for (const [el, v] of Object.entries(totals)) {
        if (Math.abs(v) > tolerance) imbalance[el] = round(v);
      }
      if (Object.keys(imbalance).length) problems.push({ id: rxn.id, imbalance });
    }
    return problems;
  }

  /** Dense-ish stoichiometric matrix in triplet form, ordered for the solver. */
  stoichiometry() {
    const metIds = [...this.metabolites.keys()];
    const rxnIds = [...this.reactions.keys()];
    const metIndex = new Map(metIds.map((id, i) => [id, i]));
    const triplets = [];
    rxnIds.forEach((rid, j) => {
      const rxn = this.reactions.get(rid);
      for (const [mid, coef] of Object.entries(rxn.metabolites)) {
        if (coef === 0) continue;
        triplets.push([metIndex.get(mid), j, coef]);
      }
    });
    return { metIds, rxnIds, triplets };
  }

  toJSON() {
    const out = {
      id: this.id,
      name: this.name,
      version: this.version,
      compartments: this.compartmentNames,
      cell: this.cell,
      compartment_tree: this.explicitTree,
      metabolites: [...this.metabolites.values()].map((m) => m.toJSON()),
      reactions: [...this.reactions.values()].map((r) => r.toJSON()),
      genes: this.genes,
    };
    if (this.rawObjective) out.objective = this.rawObjective;
    return out;
  }
}

function classify(rxn, comps) {
  const n = Object.keys(rxn.metabolites).length;
  const id = rxn.id.toUpperCase();
  if (/BIOMASS|GROWTH/.test(id)) return 'biomass';
  if (n === 1) return id.startsWith('EX_') ? 'exchange' : 'sink';
  if (id.startsWith('EX_')) return 'exchange';
  if (id.startsWith('DM_') || id.startsWith('SK_')) return 'sink';
  if (comps.size > 1) return 'transport';
  return 'metabolic';
}

function firstText(...values) {
  for (const value of values) {
    if (Array.isArray(value)) {
      const item = value.find((x) => typeof x === 'string' && x.trim());
      if (item) return item.trim();
      continue;
    }
    if (typeof value === 'string' && value.trim()) return value.trim();
  }
  return '';
}

function numOr(...values) {
  for (const v of values) {
    if (v === null || v === undefined) continue;
    const n = Number(v);
    if (Number.isFinite(n)) return n;
  }
  return 0;
}

/** Rounds `v` to `digits` decimal places (default 6). */
function round(v, digits = 6) {
  const f = 10 ** digits;
  return Math.round(v * f) / f;
}

export { round };
