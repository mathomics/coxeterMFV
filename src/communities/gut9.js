/**
 * Published community: the nine-species human gut microbiota model (SteadyCom).
 *
 *   Chan SHJ, Simons MN, Maranas CD (2017) SteadyCom: Predicting microbial
 *   abundances while ensuring community stability.
 *   PLoS Comput Biol 13(5): e1005539.  doi:10.1371/journal.pcbi.1005539
 *
 * Built from the authors' own distribution of the model, which arrives as a
 * workbook rather than SBML:
 *
 *   model9.xlsx              sheets: rxns, mets, stoich, comEx, comSp
 *   model9_readme.txt        what each field of the COBRA structure holds
 *   model9_rxns_params.txt   rev / lb / ub / c per reaction
 *   model9_stoich.txt        the stoichiometric matrix as triples
 *   simFigure4.m             the Figure 4 experiment
 *
 * Unlike the four-auxotroph community, this model already carries its community
 * structure, so none of it is reconstructed here:
 *
 *   a species metabolite is `x[c_Bt]`, a community metabolite is `x[u]`
 *   EXsp   `EX_x[u]tr_Bt`   -1 x[e_Bt] +1 x[u]   species <-> community medium
 *   EXcom  `UT_x(u)`        +1 x[u]              environment -> community medium
 *          `EX_x(u)`        -1 x[u]              community medium -> environment
 *
 * So a positive EXsp flux is secretion into the shared medium and a negative one
 * is uptake from it, and the diet is the upper bound on the UT reactions. The
 * EXsp names are not derivable: Bt uses `EX_x[u]tr_Bt`, Fp uses `IEX_x[u]tr_Fp`
 * and Ec uses `EX_x(e)_Ec`, so every one is read from the comEx sheet.
 *
 * The diet, and why it comes from the TXT
 *
 * In model9.xlsx all 558 UT reactions are bounded [0, 0]: the workbook export
 * carries no diet, and a community with every uptake closed cannot grow at all.
 * model9_rxns_params.txt, exported from the same model, carries the diet on
 * exactly 88 of those UT reactions and agrees with the workbook on all 10921
 * other reactions, objective coefficients included.
 *
 * The bounds therefore come from the TXT, and that is the one place where the
 * workbook is not the primary source. This is not a medium built here: it is the
 * authors' own export of their own bounds, and the alternative is a model that
 * is structurally complete and biologically dead. The import refuses to proceed
 * silently either way: `validateBounds` reports every difference.
 */

import { readWorkbook } from '../core/xlsx.js';

export const PROVENANCE = {
  paper: {
    title: 'SteadyCom: Predicting microbial abundances while ensuring community stability',
    authors: 'Chan SHJ, Simons MN, Maranas CD',
    year: 2017,
    journal: 'PLoS Computational Biology',
    doi: '10.1371/journal.pcbi.1005539',
    framework: 'SteadyCom',
  },
  files: {
    model: 'model9.xlsx',
    readme: 'model9_readme.txt',
    bounds: 'model9_rxns_params.txt',
    stoichiometry: 'model9_stoich.txt',
    experiment: 'simFigure4.m',
  },
};

/** Sizes the workbook itself declares, used as assertions rather than guesses. */
export const EXPECTED = {
  species: 9,
  reactions: 11009,
  metabolites: 8160,
  stoichiometryEntries: 42420,
  communityMetabolites: 558,
  communityExchangeReactions: 1116,   // 558 uptake + 558 export
  speciesExchangeReactions: 1355,     // the sum of comSp's #exchange column
  dietReactions: 88,                  // UT reactions the TXT opens
};

// Import

/**
 * Read the workbook into a joint community model.
 *
 * @param {Buffer|Uint8Array} xlsx          contents of model9.xlsx
 * @param {object} [opts]
 * @param {string} [opts.boundsText]        contents of model9_rxns_params.txt.
 *        When given, its lb/ub/c replace the workbook's, which is how the diet
 *        gets in. Without it the community is closed and cannot grow.
 * @returns {object} the joint model
 */
export function importGut9(xlsx, opts = {}) {
  const wb = readWorkbook(xlsx);

  // Species
  const species = [];
  for (const row of wb.rows('comSp')) {
    if (row.__row === 1 || !row.A) continue;
    species.push({
      id: row.A,
      name: row.B,
      biomassReaction: row.C,
      atpMaintenance: row.D,
      declaredMetabolites: Number(row.E),
      declaredReactions: Number(row.F),
      declaredExchanges: Number(row.G),
    });
  }

  // Metabolites
  const metabolites = new Map();
  for (const row of wb.rows('mets')) {
    if (row.__row === 1 || !row.A) continue;
    metabolites.set(row.A, {
      id: row.A,
      name: row.B ?? row.A,
      formula: row.C ?? '',
      charge: Number(row.D ?? 0),
      species: row.J,            // 'comm' for a community metabolite
    });
  }

  // Reactions
  // The rxns sheet has trailing rows with no identifier; a row without one is
  // not a reaction, and counting it would put the total at 11540.
  const reactions = new Map();
  for (const row of wb.rows('rxns')) {
    if (row.__row === 1 || !row.A) continue;
    reactions.set(row.A, {
      id: row.A,
      name: row.B ?? row.A,
      subsystem: row.C ?? '',
      metabolites: {},
      reversible: Number(row.J ?? 0) !== 0,
      lowerBound: Number(row.K ?? 0),
      upperBound: Number(row.L ?? 0),
      objective: Number(row.M ?? 0),
      species: row.N,            // 'comm' for a community exchange reaction
    });
  }

  // Stoichiometry
  let stoichiometryEntries = 0;
  for (const row of wb.rows('stoich')) {
    if (row.__row === 1 || !row.A) continue;
    const rxn = reactions.get(row.B);
    if (!rxn) throw new Error(`stoich names reaction ${row.B}, which the rxns sheet does not have`);
    if (!metabolites.has(row.A)) {
      throw new Error(`stoich names metabolite ${row.A}, which the mets sheet does not have`);
    }
    rxn.metabolites[row.A] = (rxn.metabolites[row.A] ?? 0) + Number(row.C);
    stoichiometryEntries += 1;
  }

  // The community exchange space
  // Row 2 of comEx holds the species abbreviation for each EXsp column, in the
  // order the paper's EXsp(:,k) uses. It is read rather than assumed, because
  // simFigure4.m addresses those columns by index.
  const communityMetabolites = [];
  const communityUptake = new Map();      // community metabolite -> UT reaction
  const communityExport = new Map();      // community metabolite -> EX reaction
  const speciesExchange = new Map();      // `${metabolite}\u0000${species}` -> EXsp
  let exspColumns = null;

  for (const row of wb.rows('comEx')) {
    if (row.__row === 1) continue;
    if (row.__row === 2) {
      exspColumns = Object.entries(row)
        .filter(([key, value]) => key !== '__row' && value)
        .map(([column, speciesId]) => ({ column, speciesId }));
      continue;
    }
    if (!row.A) continue;
    communityMetabolites.push(row.A);
    if (row.B) communityUptake.set(row.A, row.B);
    if (row.C) communityExport.set(row.A, row.C);
    for (const { column, speciesId } of exspColumns ?? []) {
      if (row[column]) speciesExchange.set(`${row.A}\u0000${speciesId}`, row[column]);
    }
  }

  const model = {
    species,
    metabolites,
    reactions,
    communityMetabolites,
    communityUptake,
    communityExport,
    speciesExchange,
    exspOrder: (exspColumns ?? []).map((c) => c.speciesId),
    stoichiometryEntries,
    boundsFrom: 'model9.xlsx',
  };

  if (opts.boundsText) applyBounds(model, opts.boundsText);
  return model;
}

// The diet

/** Parse model9_rxns_params.txt into id -> { rev, lb, ub, c }. */
export function parseBounds(text) {
  const out = new Map();
  const lines = text.split(/\r?\n/);
  const header = lines[0]?.split('\t') ?? [];
  if (header[0] !== 'rxns') throw new Error('model9_rxns_params.txt: unexpected header');
  for (const line of lines.slice(1)) {
    if (!line.trim()) continue;
    const [id, rev, lb, ub, c] = line.split('\t');
    out.set(id, {
      reversible: Number(rev) !== 0,
      lowerBound: Number(lb),
      upperBound: Number(ub),
      objective: Number(c),
    });
  }
  return out;
}

/**
 * Replace the workbook's bounds with the ones the TXT export carries, which is
 * where the authors' diet is. Every change is recorded, so nothing moves
 * silently: the caller gets the list and the tests assert it is the 88 diet
 * reactions and nothing else.
 */
export function applyBounds(model, boundsText) {
  const bounds = parseBounds(boundsText);
  const changed = [];
  const missing = [];

  for (const [id, rxn] of model.reactions) {
    const b = bounds.get(id);
    if (!b) { missing.push(id); continue; }
    if (b.lowerBound !== rxn.lowerBound || b.upperBound !== rxn.upperBound) {
      changed.push({
        id,
        species: rxn.species,
        from: [rxn.lowerBound, rxn.upperBound],
        to: [b.lowerBound, b.upperBound],
      });
      rxn.lowerBound = b.lowerBound;
      rxn.upperBound = b.upperBound;
    }
  }

  model.boundsFrom = 'model9_rxns_params.txt';
  model.boundChanges = changed;
  model.boundsMissing = missing;
  return changed;
}

// Validation

/**
 * Check the imported bounds against model9_rxns_params.txt.
 * @returns {{identical:number, differing:object[], onlyInModel:string[], onlyInText:string[]}}
 */
export function validateBounds(model, boundsText) {
  const bounds = parseBounds(boundsText);
  let identical = 0;
  const differing = [];
  const onlyInModel = [];

  for (const [id, rxn] of model.reactions) {
    const b = bounds.get(id);
    if (!b) { onlyInModel.push(id); continue; }
    if (b.lowerBound === rxn.lowerBound && b.upperBound === rxn.upperBound
      && b.objective === rxn.objective) {
      identical += 1;
    } else {
      differing.push({
        id,
        model: [rxn.lowerBound, rxn.upperBound, rxn.objective],
        text: [b.lowerBound, b.upperBound, b.objective],
      });
    }
  }

  const onlyInText = [...bounds.keys()].filter((id) => !model.reactions.has(id));
  return { identical, differing, onlyInModel, onlyInText };
}

/**
 * Check the imported stoichiometric matrix against model9_stoich.txt, entry by
 * entry and in both directions, so neither a missing nor an extra coefficient
 * can pass.
 * @returns {{entries:number, identical:number, differing:object[], onlyInText:string[], onlyInModel:string[]}}
 */
export function validateStoichiometry(model, stoichText) {
  const lines = stoichText.split(/\r?\n/);
  if (lines[0]?.split('\t')[0] !== 'mets') throw new Error('model9_stoich.txt: unexpected header');

  let entries = 0;
  let identical = 0;
  const differing = [];
  const onlyInText = [];
  const seen = new Set();

  for (const line of lines.slice(1)) {
    if (!line.trim()) continue;
    const [met, rxnId, coef] = line.split('\t');
    entries += 1;
    const rxn = model.reactions.get(rxnId);
    const value = rxn?.metabolites[met];
    if (value === undefined) { onlyInText.push(`${rxnId}:${met}`); continue; }
    seen.add(`${rxnId}\u0000${met}`);
    // The TXT prints twelve decimals; compare on value, not on spelling.
    if (Math.abs(value - Number(coef)) < 1e-9) identical += 1;
    else differing.push({ reaction: rxnId, metabolite: met, model: value, text: Number(coef) });
  }

  const onlyInModel = [];
  for (const [id, rxn] of model.reactions) {
    for (const met of Object.keys(rxn.metabolites)) {
      if (!seen.has(`${id}\u0000${met}`)) onlyInModel.push(`${id}:${met}`);
    }
  }

  return { entries, identical, differing, onlyInText, onlyInModel };
}

// Derived accessors

/** Reactions that belong to a species, as `species id -> reaction ids`. */
export function reactionsBySpecies(model) {
  const out = new Map(model.species.map((s) => [s.id, []]));
  for (const [id, rxn] of model.reactions) {
    if (rxn.species === 'comm') continue;
    if (!out.has(rxn.species)) throw new Error(`reaction ${id} belongs to unknown species ${rxn.species}`);
    out.get(rxn.species).push(id);
  }
  return out;
}

/** The 1116 community exchange reactions, which belong to no species. */
export function communityReactionIds(model) {
  return [...model.reactions.values()].filter((r) => r.species === 'comm').map((r) => r.id);
}

/** What the diet actually supplies: UT reaction -> upper bound, when positive. */
export function diet(model) {
  const out = new Map();
  for (const [met, rxnId] of model.communityUptake) {
    const ub = model.reactions.get(rxnId)?.upperBound ?? 0;
    if (ub > 0) out.set(met, ub);
  }
  return out;
}

/**
 * Turn the imported model into the spec steadyCom() takes.
 *
 * Nothing is reshaped: the reactions, metabolites and bounds go through as they
 * were read, and the species tag on each reaction is what tells the solver which
 * fluxes scale with abundance and which are absolute community rates.
 *
 * @param {object} model              from importGut9()
 * @param {object} [opts]
 * @param {number} [opts.totalBiomass=1]
 * @param {object[]} [opts.constraints] extra linear constraints, as
 *        buildJointProblem documents them
 */
export function toSteadyComSpec(model, opts = {}) {
  return {
    members: model.species.map((sp) => ({
      id: sp.id,
      name: sp.name,
      biomassReaction: sp.biomassReaction,
      atpMaintenance: sp.atpMaintenance,
    })),
    reactions: model.reactions,
    metabolites: model.metabolites,
    communityTag: 'comm',
    totalBiomass: opts.totalBiomass ?? 1,
    constraints: opts.constraints ?? [],
  };
}
