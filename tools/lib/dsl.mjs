/**
 * Compact model description language.
 *
 * Models are written as reaction equations rather than raw JSON so that a typo
 * in a metabolite id fails at build time and every stoichiometry can be checked
 * against the shared formula table.
 */
import { MET, CURRENCY } from './metabolites.mjs';

const ARROWS = [
  ['<=>', true],
  ['<==>', true],
  ['-->', false],
  ['->', false],
  ['<--', 'reverse'],
];

/**
 * Parse "2.5 atp_c + f6p_c --> adp_c" into { metabolites, reversible, backwards }.
 * Left-hand terms get negative coefficients. `backwards` marks an equation
 * written with `<--`.
 */
export function parseEquation(eq) {
  let arrow = null;
  let idx = -1;
  for (const [token, rev] of ARROWS) {
    const i = eq.indexOf(token);
    if (i >= 0 && (idx < 0 || i < idx || token.length > arrow?.token.length)) {
      if (idx < 0 || token.length > (arrow?.token.length ?? 0) || i < idx) {
        idx = i; arrow = { token, rev };
      }
    }
  }
  if (!arrow) throw new Error(`No arrow in equation: ${eq}`);

  const left = eq.slice(0, idx).trim();
  const right = eq.slice(idx + arrow.token.length).trim();
  const metabolites = {};

  const consume = (side, sign) => {
    if (!side || side === '∅' || side === '0') return;
    for (const raw of side.split('+')) {
      const term = raw.trim();
      if (!term) continue;
      const m = /^(?:([0-9]*\.?[0-9]+)\s+)?([A-Za-z0-9_]+)$/.exec(term);
      if (!m) throw new Error(`Cannot parse term "${term}" in "${eq}"`);
      const coef = (m[1] ? Number(m[1]) : 1) * sign;
      metabolites[m[2]] = (metabolites[m[2]] || 0) + coef;
    }
  };

  consume(left, -1);
  consume(right, 1);
  for (const k of Object.keys(metabolites)) if (metabolites[k] === 0) delete metabolites[k];

  return { metabolites, reversible: arrow.rev === true, backwards: arrow.rev === 'reverse' };
}

/**
 * Declare a reaction.
 * @param {string} id
 * @param {string} equation
 * @param {object} [opts] name, lb, ub, sub (subsystem), rank, lane, gene
 */
export function r(id, equation, opts = {}) {
  return { id, equation, ...opts };
}

/** Declare an exchange reaction for a base metabolite in compartment `e`. */
export function ex(base, { lb = 0, ub = 1000, rank = 0, lane = 0 } = {}) {
  return r(`EX_${base}_e`, `${base}_e <=>`, {
    name: `${MET[base]?.[2] ?? base} exchange`,
    lb, ub, sub: 'Exchange', rank, lane,
  });
}

const COMPARTMENT_SUFFIX = /_([a-z][a-z0-9]?)$/;

/**
 * Assemble a COBRA JSON document from a compact species description.
 *
 * @param {object} spec
 * @param {string} spec.id
 * @param {string} spec.name
 * @param {Object<string,string>} spec.compartments
 * @param {Object<string,string|null>} [spec.compartmentTree]
 * @param {object} spec.cell
 * @param {Array} spec.reactions
 * @param {string} spec.objective reaction id
 * @param {string[]} [spec.primary] base ids forced to be map hubs
 */
export function buildModel(spec) {
  const metabolites = new Map();
  const reactions = [];

  const touch = (mid) => {
    if (metabolites.has(mid)) return;
    const m = COMPARTMENT_SUFFIX.exec(mid);
    if (!m) throw new Error(`Metabolite ${mid} has no compartment suffix`);
    const compartment = m[1];
    const base = mid.slice(0, -(compartment.length + 1));
    const entry = MET[base];
    if (!entry) throw new Error(`Unknown metabolite base "${base}" (from ${mid}) in model ${spec.id}`);
    if (!spec.compartments[compartment]) {
      throw new Error(`Metabolite ${mid} lives in undeclared compartment "${compartment}" of ${spec.id}`);
    }
    const [formula, charge, name] = entry;
    const forcedPrimary = spec.primary?.includes(base);
    const record = {
      id: mid,
      name: `${name} [${spec.compartments[compartment]}]`,
      compartment,
      formula,
      charge,
    };
    if (forcedPrimary) record.primary = true;
    else if (CURRENCY.has(base)) record.primary = false;
    metabolites.set(mid, record);
  };

  for (const decl of spec.reactions) {
    const { metabolites: stoich, reversible, backwards } = parseEquation(decl.equation);
    for (const mid of Object.keys(stoich)) touch(mid);

    let lb = decl.lb;
    let ub = decl.ub;
    if (lb === undefined) lb = reversible || backwards ? -1000 : 0;
    if (ub === undefined) ub = backwards ? 0 : 1000;

    const rxn = {
      id: decl.id,
      name: decl.name || decl.id,
      metabolites: stoich,
      lower_bound: lb,
      upper_bound: ub,
      gene_reaction_rule: decl.gene || '',
    };
    if (decl.sub) rxn.subsystem = decl.sub;
    if (decl.rank !== undefined || decl.lane !== undefined) {
      rxn.layout = { rank: decl.rank ?? 0, lane: decl.lane ?? 0 };
    }
    if (decl.id === spec.objective) rxn.objective_coefficient = 1;
    reactions.push(rxn);
  }

  const ids = new Set(reactions.map((x) => x.id));
  if (ids.size !== reactions.length) {
    const seen = new Set();
    for (const x of reactions) {
      if (seen.has(x.id)) throw new Error(`Duplicate reaction id ${x.id} in ${spec.id}`);
      seen.add(x.id);
    }
  }
  if (spec.objective && !ids.has(spec.objective)) {
    throw new Error(`Objective ${spec.objective} not defined in ${spec.id}`);
  }

  return {
    id: spec.id,
    name: spec.name,
    version: '1',
    compartments: spec.compartments,
    compartment_tree: spec.compartmentTree || null,
    cell: spec.cell,
    metabolites: [...metabolites.values()].sort((a, b) => a.id.localeCompare(b.id)),
    reactions,
    genes: [],
    objective: spec.objective ? { [spec.objective]: 1 } : null,
  };
}
