/**
 * Stolyar et al. 2007: the joint model of a two-species mutualism.
 *
 * The supplement is four plain-text files in CellNetAnalyzer's project format,
 * not SBML and not COBRA JSON, so this reads that format directly rather than
 * retyping the network out of the paper:
 *
 *   reactions.txt              id, equation, measured rate, bounds, objective,
 *                              map position and layer
 *   metabolites.txt            id, name, and a flag marking the external ones
 *   macromolecules.txt         the seven D. vulgaris macromolecules and the
 *                              fraction of dry weight each one contributes
 *   macromolecule_synthesis.txt what it costs to make one unit of each
 *
 * Two organisms share one stoichiometric matrix. Which reaction belongs to
 * which is in the file: the layer column, 1 for D. vulgaris and 3 for
 * M. maripaludis, with layer 2 holding the methanogen's macromolecule
 * synthesis. The methanogen's metabolites also carry a `ZM_` prefix, and the
 * two agree everywhere. The import checks that rather than trusting it.
 *
 * Four metabolites appear in both organisms' reactions, and they are not
 * declared as shared anywhere: they are found by looking at which side of the
 * model touches them. That is the mutualism, and it is derived, not asserted.
 *
 * Two conventions of this format matter for reading it as a COBRA model:
 *
 *   - The equation's `=` separates substrates from products, and an empty side
 *     is allowed: `1 H2(exch) =` is a drain. `mue`, the D. vulgaris growth
 *     reaction, has no equation at all: its stoichiometry is the macromolecule
 *     fractions, which is what the other two files are for.
 *   - The measured-rate column is a constraint, not a note. `#` means
 *     unmeasured, and a number fixes that flux. Thirteen reactions are fixed at
 *     zero, `D86` fixes the lactate supply at 10, and `H2gen` fixes the
 *     hydrogen leaving the pair at zero, which is the syntrophy, stated as a
 *     constraint rather than described.
 *
 * In macromolecule_synthesis.txt a positive coefficient is consumed and a
 * negative one produced, the opposite of the usual reading. The methanogen's
 * own synthesis reactions are written out in reactions.txt (MC_RNA and friends)
 * and say the same thing the other way round, which is how that sign
 * convention is confirmed here instead of guessed.
 */

/** Which organism each layer in the file belongs to. */
export const SPECIES = [
  {
    id: 'Dv',
    name: 'Desulfovibrio vulgaris Hildenborough',
    short: 'D. vulgaris',
    layers: [1],
    prefix: null,
    biomassReaction: 'mue',
  },
  {
    id: 'Mm',
    name: 'Methanococcus maripaludis S2',
    short: 'M. maripaludis',
    layers: [2, 3],
    prefix: 'ZM_',
    biomassReaction: 'MC_BIO',
  },
];

/** What a correct import produces, so a silent change in the files shows up. */
export const EXPECTED = {
  species: 2,
  reactions: 180,
  metabolites: 163,
  macromolecules: 7,
  externalMetabolites: 8,
  sharedMetabolites: 4,
  measuredReactions: 15,
  objectiveReactions: 3,
};

const trimmedLines = (text) => String(text ?? '').split(/\r?\n/).filter((l) => l.trim());
const fields = (line) => line.split('\t').map((s) => s.trim());

/**
 * Split one side of an equation into terms.
 *
 * The separator is ` + ` with spaces: metabolite ids in this model contain a
 * bare `+` (`H+`, `ZM_H+`), so splitting on the character alone invents two
 * metabolites out of one.
 */
export function parseTerms(side) {
  return String(side ?? '').split(/\s+\+\s+/).map((t) => t.trim()).filter(Boolean)
    .map((term) => {
      const m = /^([0-9]*\.?[0-9]+)\s+(.+)$/.exec(term);
      return m ? { coefficient: Number(m[1]), metabolite: m[2].trim() } : { coefficient: 1, metabolite: term };
    });
}

/** `1 A + 2 B = 1 C` into a COBRA stoichiometry map. */
export function parseEquation(equation) {
  const stoichiometry = {};
  const [left = '', right = ''] = String(equation ?? '').split('=');
  for (const { coefficient, metabolite } of parseTerms(left)) {
    stoichiometry[metabolite] = (stoichiometry[metabolite] ?? 0) - coefficient;
  }
  for (const { coefficient, metabolite } of parseTerms(right)) {
    stoichiometry[metabolite] = (stoichiometry[metabolite] ?? 0) + coefficient;
  }
  for (const [id, coefficient] of Object.entries(stoichiometry)) {
    if (coefficient === 0) delete stoichiometry[id];
  }
  return stoichiometry;
}

/**
 * One reaction per line. The `|` column is the anchor rather than a fixed field
 * count: `mue` carries an empty equation and shifts everything after it.
 */
export function parseReactions(text) {
  const out = new Map();
  for (const line of trimmedLines(text)) {
    const f = fields(line);
    const bar = f.indexOf('|');
    if (bar < 1) throw new Error(`reactions: no "|" column in ${JSON.stringify(line.slice(0, 40))}`);
    const id = f[0];
    const equation = f.slice(1, bar).join(' ').trim();
    const measured = f[bar + 1];
    const [lowerBound, upperBound, objective] = f[bar + 2].split(/\s+/).map(Number);
    const [x, y, layer] = f[bar + 3].split(/\s+/).map(Number);
    if (!Number.isFinite(lowerBound) || !Number.isFinite(upperBound)) {
      throw new Error(`reactions: ${id} has unreadable bounds ${JSON.stringify(f[bar + 2])}`);
    }
    out.set(id, {
      id,
      equation,
      metabolites: parseEquation(equation),
      measured: measured === '#' ? null : Number(measured),
      lowerBound,
      upperBound,
      objective,
      layer,
      position: { x, y },
    });
  }
  return out;
}

/** id, name, a tolerance this import does not use, and the external flag. */
export function parseMetabolites(text) {
  const out = new Map();
  for (const line of trimmedLines(text)) {
    const f = fields(line);
    out.set(f[0], { id: f[0], name: f[1] || f[0], external: f[3] === '1' });
  }
  return out;
}

/** id, name, mass fraction, then map position. */
export function parseMacromolecules(text) {
  const out = new Map();
  for (const line of trimmedLines(text)) {
    const f = fields(line);
    out.set(f[0], { id: f[0], name: f[1] || f[0], fraction: Number(f[2]) });
  }
  return out;
}

/**
 * `RNA = 6.17 Asp + -3.47 fum + …`, where a positive coefficient is consumed
 * and a negative one produced.
 */
export function parseMacromoleculeSynthesis(text) {
  const out = new Map();
  for (const line of trimmedLines(text)) {
    const [name, body = ''] = line.split('=');
    const id = name.trim();
    if (!id) continue;
    const stoichiometry = {};
    for (const term of body.split('+')) {
      const t = term.trim();
      if (!t) continue;
      const m = /^(-?[0-9]*\.?[0-9]+)\s+(.+)$/.exec(t);
      if (!m) throw new Error(`macromolecule synthesis: unreadable term ${JSON.stringify(t)} in ${id}`);
      // Consumed is positive here, so the sign flips to reach COBRA's reading.
      stoichiometry[m[2].trim()] = (stoichiometry[m[2].trim()] ?? 0) - Number(m[1]);
    }
    out.set(id, stoichiometry);
  }
  return out;
}

/**
 * Read the four files into one model.
 *
 * @param {{reactions:string, metabolites:string, macromolecules:string, synthesis:string}} files
 * @returns the metabolite and reaction maps (each reaction tagged with its
 *   `species`, null for the pair's boundary), the macromolecules, the set of
 *   shared metabolite ids and SPECIES.
 */
export function importStolyar(files) {
  const metabolites = parseMetabolites(files.metabolites);
  const reactions = parseReactions(files.reactions);
  const macromolecules = parseMacromolecules(files.macromolecules);
  const synthesis = parseMacromoleculeSynthesis(files.synthesis);

  // The D. vulgaris biomass, which the reaction file leaves empty
  // Each macromolecule becomes a metabolite, its synthesis a reaction, and the
  // growth reaction consumes them in the fractions macromolecules.txt gives.
  // The methanogen's equivalent (MC_BIO) is already written out in the reaction
  // file, and this mirrors it.
  const growth = reactions.get(SPECIES[0].biomassReaction);
  if (!growth) throw new Error(`reactions: the growth reaction ${SPECIES[0].biomassReaction} is missing`);
  if (Object.keys(growth.metabolites).length) {
    throw new Error(`reactions: ${growth.id} was expected to carry no equation of its own`);
  }

  for (const [id, macro] of macromolecules) {
    const stoichiometry = synthesis.get(id);
    if (!stoichiometry) throw new Error(`macromolecules: ${id} has no synthesis equation`);
    metabolites.set(id, { id, name: macro.name, external: false, macromolecule: true });
    reactions.set(`${id}_syn`, {
      id: `${id}_syn`,
      equation: '',
      metabolites: { ...stoichiometry, [id]: 1 },
      measured: null,
      lowerBound: 0,
      upperBound: 100,
      objective: 0,
      layer: SPECIES[0].layers[0],
      position: null,
      derived: 'macromolecule synthesis, from macromolecule_synthesis.txt',
    });
    growth.metabolites[id] = -macro.fraction;
  }
  growth.derived = 'biomass composition, from macromolecules.txt';

  for (const rxn of reactions.values()) {
    for (const id of Object.keys(rxn.metabolites)) {
      if (!metabolites.has(id)) {
        throw new Error(`reactions: ${rxn.id} uses ${id}, which metabolites.txt does not declare`);
      }
    }
  }

  // Who owns what
  const layerOwner = new Map();
  for (const sp of SPECIES) for (const layer of sp.layers) layerOwner.set(layer, sp.id);

  const touchedBy = new Map(); // metabolite -> Set of species ids
  for (const rxn of reactions.values()) {
    const owner = layerOwner.get(rxn.layer);
    if (!owner) throw new Error(`reactions: ${rxn.id} sits on unknown layer ${rxn.layer}`);
    for (const id of Object.keys(rxn.metabolites)) {
      if (!touchedBy.has(id)) touchedBy.set(id, new Set());
      touchedBy.get(id).add(owner);
    }
  }

  // Shared: used by both organisms. Nothing in the files says which metabolites
  // these are, so they are read off the model.
  const shared = new Set([...touchedBy.entries()]
    .filter(([, owners]) => owners.size > 1).map(([id]) => id));

  for (const rxn of reactions.values()) {
    const ids = Object.keys(rxn.metabolites);
    // A reaction touching nothing but the shared pool belongs to neither
    // organism: it is the pair's boundary with the rest of the world.
    rxn.species = ids.length && ids.every((id) => shared.has(id))
      ? null
      : layerOwner.get(rxn.layer);

    // The prefix and the layer have to agree, or the layer is not what it looks
    // like and every split below would be wrong.
    const prefixed = ids.filter((id) => id.startsWith('ZM_'));
    if (rxn.species === 'Mm' && ids.length && !prefixed.length) {
      throw new Error(`reactions: ${rxn.id} is on a methanogen layer but names no ZM_ metabolite`);
    }
    if (rxn.species === 'Dv' && prefixed.length) {
      throw new Error(`reactions: ${rxn.id} is on the D. vulgaris layer but names ${prefixed[0]}`);
    }
  }

  return { metabolites, reactions, macromolecules, shared, species: SPECIES };
}

/** Metabolites the file marks external: unbalanced, free to enter or leave. */
export function externalMetabolites(model) {
  return [...model.metabolites.values()].filter((m) => m.external).map((m) => m.id);
}

/** Reaction ids per organism, plus the pair's own boundary reactions. */
export function reactionsBySpecies(model) {
  const out = new Map(SPECIES.map((sp) => [sp.id, []]));
  const community = [];
  for (const rxn of model.reactions.values()) {
    if (rxn.species === null) community.push(rxn.id);
    else out.get(rxn.species).push(rxn.id);
  }
  return { bySpecies: out, community };
}

/**
 * Which organism releases each shared metabolite and which takes it up, read
 * off the stoichiometry rather than written down.
 *
 * This is the structural picture: who *can* exchange what. What actually moves
 * is a question for the fluxes, and the app computes that from a solution.
 */
export function exchangeStructure(model) {
  const out = [];
  for (const id of [...model.shared].sort()) {
    const producers = new Set();
    const consumers = new Set();
    for (const rxn of model.reactions.values()) {
      const coefficient = rxn.metabolites[id];
      if (!coefficient || !rxn.species) continue;
      // A reversible reaction can do either, so it counts as both.
      if (coefficient > 0 || rxn.lowerBound < 0) producers.add(rxn.species);
      if (coefficient < 0 || rxn.lowerBound < 0) consumers.add(rxn.species);
    }
    out.push({
      metabolite: id,
      name: model.metabolites.get(id)?.name ?? id,
      producers: [...producers].sort(),
      consumers: [...consumers].sort(),
    });
  }
  return out;
}

const COMPARTMENT_NAMES = { c: 'Cytosol', e: 'Extracellular' };

/** Shared and external metabolites sit outside the cell, everything else inside it. */
const compartmentOf = (model, id) => (
  model.shared.has(id) || model.metabolites.get(id)?.external ? 'e' : 'c');

/**
 * The whole two-species model as one COBRA document, in the file's own shape.
 *
 * External metabolites are unbalanced in CellNetAnalyzer, which a COBRA model
 * has no notion of, so each one gets an unconstrained exchange reaction. That
 * is the faithful translation of "free to enter or leave", and it is the only
 * reaction this adds.
 */
export function toJointCobraJSON(model, { id = 'stolyar2007', name = 'Stolyar 2007 joint model' } = {}) {
  const reactions = [...model.reactions.values()].map((rxn) => toCobraReaction(rxn));
  for (const met of externalMetabolites(model)) {
    reactions.push({
      id: `EX_${met}`,
      name: `${model.metabolites.get(met)?.name ?? met} exchange`,
      metabolites: { [met]: -1 },
      lower_bound: -1000,
      upper_bound: 1000,
      subsystem: 'Exchange',
    });
  }

  return {
    id,
    name,
    compartments: { ...COMPARTMENT_NAMES },
    metabolites: [...model.metabolites.values()].map((met) => ({
      id: met.id,
      name: met.name,
      compartment: compartmentOf(model, met.id),
      formula: '',
      charge: 0,
    })),
    reactions,
    genes: [],
    objective: objectiveOf(model),
  };
}

/**
 * CellNetAnalyzer minimises its objective and this project maximises, so the
 * coefficients change sign. The file asks for growth of both organisms (the
 * methanogen at a tenth of the weight) against the cost of running ATP
 * maintenance, and that weighting is the paper's, not this project's.
 */
export function objectiveOf(model) {
  const out = {};
  for (const rxn of model.reactions.values()) {
    if (rxn.objective) out[rxn.id] = -rxn.objective;
  }
  return out;
}

function toCobraReaction(rxn) {
  // A measured rate is a constraint in this format, so it narrows the bounds
  // rather than sitting beside them as a comment.
  const lower = rxn.measured === null ? rxn.lowerBound : rxn.measured;
  const upper = rxn.measured === null ? rxn.upperBound : rxn.measured;
  return {
    id: rxn.id,
    name: rxn.id,
    metabolites: { ...rxn.metabolites },
    lower_bound: lower,
    upper_bound: upper,
    subsystem: rxn.derived ?? '',
  };
}

/**
 * One organism on its own, with the shared pool turned into exchanges.
 *
 * The joint model keeps the shared metabolites inside the matrix, where they
 * are neither organism's. Split out, each organism keeps its own copy in its
 * extracellular compartment and gains one exchange reaction per shared
 * metabolite, which is what lets the community layer draw a single medium
 * between the cells and compute the cross-feeding from the fluxes.
 */
export function toSpeciesCobraJSON(model, speciesId, { cell } = {}) {
  const sp = SPECIES.find((s) => s.id === speciesId);
  if (!sp) throw new Error(`Unknown species ${speciesId}`);
  const { bySpecies } = reactionsBySpecies(model);
  const ids = bySpecies.get(speciesId);

  const reactions = ids.map((rid) => toCobraReaction(model.reactions.get(rid)));
  const used = new Set();
  for (const rxn of reactions) for (const id of Object.keys(rxn.metabolites)) used.add(id);

  // One exchange per shared or external metabolite this organism touches.
  for (const met of [...used].sort()) {
    if (!model.shared.has(met) && !model.metabolites.get(met)?.external) continue;
    reactions.push({
      id: `EX_${met}_${speciesId}`,
      name: `${model.metabolites.get(met)?.name ?? met} exchange`,
      metabolites: { [met]: -1 },
      lower_bound: -1000,
      upper_bound: 1000,
      subsystem: 'Exchange',
    });
  }

  const objective = {};
  if (model.reactions.has(sp.biomassReaction)) objective[sp.biomassReaction] = 1;

  return {
    id: speciesId,
    name: sp.name,
    cell: cell ?? { name: sp.id, organism: sp.name, shape: 'rounded', color: '#334155' },
    compartments: { ...COMPARTMENT_NAMES },
    metabolites: [...used].sort().map((id) => ({
      id,
      name: model.metabolites.get(id)?.name ?? id,
      compartment: compartmentOf(model, id),
      formula: '',
      charge: 0,
    })),
    reactions,
    genes: [],
    objective,
  };
}

/** Counts and invariants, checked against EXPECTED. */
export function validateStolyar(model) {
  const { bySpecies, community } = reactionsBySpecies(model);
  const measured = [...model.reactions.values()].filter((r) => r.measured !== null);
  const objective = [...model.reactions.values()].filter((r) => r.objective);

  const problems = [];
  const expect = (label, got, want) => {
    if (got !== want) problems.push(`${label}: ${got}, expected ${want}`);
  };
  // The macromolecule synthesis reactions are derived here, so they are not
  // part of the file's own count.
  expect('reactions in the file', model.reactions.size - model.macromolecules.size, EXPECTED.reactions);
  expect('metabolites in the file', model.metabolites.size - model.macromolecules.size, EXPECTED.metabolites);
  expect('macromolecules', model.macromolecules.size, EXPECTED.macromolecules);
  expect('external metabolites', externalMetabolites(model).length, EXPECTED.externalMetabolites);
  expect('shared metabolites', model.shared.size, EXPECTED.sharedMetabolites);
  expect('measured reactions', measured.length, EXPECTED.measuredReactions);
  expect('objective reactions', objective.length, EXPECTED.objectiveReactions);
  expect('species', SPECIES.length, EXPECTED.species);

  const fractions = [...model.macromolecules.values()].reduce((a, m) => a + m.fraction, 0);
  if (Math.abs(fractions - 1) > 1e-6) {
    problems.push(`macromolecule fractions sum to ${fractions}, expected 1`);
  }

  for (const sp of SPECIES) {
    if (!bySpecies.get(sp.id).length) problems.push(`${sp.id} has no reactions`);
    if (!model.reactions.has(sp.biomassReaction)) {
      problems.push(`${sp.id} has no biomass reaction ${sp.biomassReaction}`);
    }
  }

  return {
    ok: problems.length === 0,
    problems,
    counts: {
      reactions: model.reactions.size,
      metabolites: model.metabolites.size,
      bySpecies: Object.fromEntries([...bySpecies].map(([k, v]) => [k, v.length])),
      community: community.length,
      shared: [...model.shared].sort(),
      measured: measured.map((r) => `${r.id}=${r.measured}`),
    },
  };
}
