/**
 * The merged community as one COBRA JSON model, and that file read back into
 * the species it was made from.
 *
 * The file is an ordinary COBRA JSON model, which cobrapy loads as it stands.
 * It holds the community problem the app solves: every species' reactions,
 * metabolites, compartments and genes under the species' own namespace
 * (`eco@PGI`, `eco@g6p_c`, compartment `eco@c`, gene `eco:b4025`), the shared
 * medium `s` with its exchanges, and the community objective written as
 * objective coefficients. Its `compartment_tree` nests every compartment, with
 * the shared medium at the top and each cell inside it as the map draws it, so
 * a nesting set in the Edit compartments window travels with the model.
 *
 * A `community` block, which cobrapy ignores, keeps what cannot be read back
 * from the merge: each species' id, abundance, model id, name, version, cell
 * and own objective, the medium as supply rates, and the community objective.
 * Opening the file splits it back into its species, so a map saved over them
 * opens on it as it does on the separate model files.
 *
 * Reaction-bound overrides, gene knockouts and analysis settings are left to
 * the Scenario file, as they are for a single model.
 */

import { LINK_COMPARTMENT, OBJECTIVES, SHARED_COMPARTMENT, namespaced } from './community.js';
import { treeNesting } from './compartments.js';

/** The `community.format` that marks a merged community model file. */
export const COMMUNITY_MODEL_FORMAT = 'CoxeterCommunityModel';

/**
 * The `community.version` this build writes. Version 1 is the first layout of
 * the block; a later version is refused rather than half read.
 */
export const COMMUNITY_MODEL_VERSION = 1;

/**
 * What joins a species id to a gene id in the file (`eco:b4025`). Reactions
 * and metabolites take the app's `@`, but cobrapy reads a gene rule as a
 * Python expression, where `@` is an operator and the rule fails to load. A
 * `:` is one of the characters cobrapy escapes in a rule and gives back in
 * the gene id.
 */
const GENE_SEPARATOR = ':';

/** Gene ids in a gene rule: every token that is not a parenthesis or an operator. */
const RULE_TOKEN = /[^\s()]+/g;
const RULE_OPERATOR = /^(and|or)$/i;

/** A gene rule with every gene id passed through `rename`, spacing and operators kept as written. */
function renameRuleGenes(rule, rename) {
  return String(rule ?? '').replace(RULE_TOKEN, (token) => (RULE_OPERATOR.test(token) ? token : rename(token)));
}

/** A model's objective as {reaction id: coefficient}. */
function objectiveMap(model) {
  const out = {};
  for (const { id, coefficient } of model.objectiveReactions()) {
    out[id] = coefficient;
  }
  return out;
}

/**
 * A species' own objective, written both ways a COBRA JSON file can hold it:
 * the top-level `objective` its file declares, if any, and the reactions'
 * objective coefficients, which the merge replaces with the community's.
 */
function ownObjective(model) {
  const coefficients = {};
  for (const rxn of model.reactions.values()) {
    if (rxn.objectiveCoefficient) {
      coefficients[rxn.id] = rxn.objectiveCoefficient;
    }
  }
  const declared = model.rawObjective && typeof model.rawObjective === 'object' ? { ...model.rawObjective } : null;
  return { objective: declared, objective_coefficients: coefficients };
}

/**
 * The community as one COBRA JSON document, ready for JSON.stringify.
 *
 * @param {import('./community.js').CommunityModel} community
 * @param {object} [context]
 * @param {Object<string, number>} [context.medium] supply rate by base metabolite id
 * @param {Array<{id: string, outerCompartment?: string, nesting?: object}>} [context.cells]
 *   the map's cells, whose outer compartment and nesting set the compartment tree
 * @returns {object} the document
 */
export function communityModelDocument(community, context = {}) {
  const merged = community.merged;
  const objective = objectiveMap(merged);
  const cells = new Map((context.cells ?? []).map((cell) => [cell.id, cell]));

  const compartments = {};
  const tree = {};
  const metabolites = [];
  const reactions = [];
  const genes = [];
  const members = [];
  const written = new Set();

  compartments[SHARED_COMPARTMENT] = merged.compartmentNames[SHARED_COMPARTMENT] ?? 'Shared medium';
  tree[SHARED_COMPARTMENT] = null;

  // Each species under its own namespace
  for (const sp of community.activeSpecies) {
    const model = sp.model;
    const own = (id) => namespaced(sp.id, id);
    const ownGene = (id) => `${sp.id}${GENE_SEPARATOR}${id}`;

    for (const id of model.compartmentIds()) {
      compartments[own(id)] = model.compartmentNames[id] ?? id;
    }

    // The nesting as the map draws it: the cell's own nesting where it has
    // one, everything inside the cell's outer compartment, and the outer
    // compartment inside the shared medium.
    const cell = cells.get(sp.id);
    const drawn = model.compartmentTree(cell?.nesting ?? null);
    let nesting = {};
    if (cell?.outerCompartment && drawn.byId.has(cell.outerCompartment)) {
      nesting = treeNesting(drawn, cell.outerCompartment);
    } else {
      for (const node of drawn.byId.values()) {
        nesting[node.id] = node.parent ?? null;
      }
    }
    for (const id of model.compartmentIds()) {
      const parent = nesting[id] ?? null;
      tree[own(id)] = parent === null ? SHARED_COMPARTMENT : own(parent);
    }

    for (const met of model.metabolites.values()) {
      const out = met.toJSON();
      out.id = own(met.id);
      out.compartment = own(met.compartment);
      metabolites.push(out);
      written.add(out.id);
    }

    for (const rxn of model.reactions.values()) {
      const out = rxn.toJSON();
      out.id = own(rxn.id);
      // The merged stoichiometry, which adds the transfer to the shared medium
      // and the growth link to what the species' own reaction says.
      out.metabolites = { ...merged.reactions.get(out.id).metabolites };
      out.gene_reaction_rule = renameRuleGenes(rxn.geneReactionRule, ownGene);
      delete out.objective_coefficient;
      if (objective[out.id]) {
        out.objective_coefficient = objective[out.id];
      }
      reactions.push(out);
      written.add(out.id);
    }

    for (const gene of model.genes) {
      genes.push({ ...gene, id: ownGene(gene.id) });
    }

    members.push({
      id: sp.id,
      abundance: sp.abundance,
      model: {
        id: model.id,
        name: model.name,
        version: model.version,
        cell: model.cell,
        ...ownObjective(model),
      },
    });
  }

  // The shared medium, its exchanges and the growth link
  for (const met of merged.metabolites.values()) {
    if (written.has(met.id)) continue;
    metabolites.push(met.toJSON());
    if (met.compartment === LINK_COMPARTMENT) {
      compartments[LINK_COMPARTMENT] = merged.compartmentNames[LINK_COMPARTMENT] ?? LINK_COMPARTMENT;
      tree[LINK_COMPARTMENT] = null;
    }
  }
  for (const rxn of merged.reactions.values()) {
    if (written.has(rxn.id)) continue;
    const out = rxn.toJSON();
    delete out.objective_coefficient;
    if (objective[out.id]) {
      out.objective_coefficient = objective[out.id];
    }
    reactions.push(out);
  }

  return {
    id: 'community',
    name: community.name,
    version: '1',
    compartments,
    compartment_tree: tree,
    metabolites,
    reactions,
    genes,
    community: {
      format: COMMUNITY_MODEL_FORMAT,
      version: COMMUNITY_MODEL_VERSION,
      objective: community.objectiveType,
      productTarget: community.productTarget ?? null,
      medium: { ...(context.medium ?? {}) },
      members,
    },
  };
}

/** Whether a parsed JSON document is a merged community model. */
export function isCommunityModelDocument(raw) {
  return raw?.community?.format === COMMUNITY_MODEL_FORMAT;
}

/**
 * Split a merged community model back into its species.
 *
 * Each species gets back what is under its namespace, with the namespace
 * removed: its compartments and their nesting, metabolites, reactions without
 * the transfer and growth-link terms the merge added, genes, and its own
 * objective. The shared medium, its exchanges and the growth link are left
 * behind, because a community built from the species makes them again.
 *
 * @param {object} raw the parsed file
 * @param {string} [filename] for error messages
 * @returns {{name: string, objective: string, productTarget: string|null,
 *   medium: Object<string, number>, members: Array<{id: string, abundance: number, model: object}>}}
 *   `model` is each species' COBRA JSON document
 */
export function splitCommunityModel(raw, filename = 'community.json') {
  const block = raw?.community;
  if (block?.format !== COMMUNITY_MODEL_FORMAT) {
    throw new Error(`${filename}: this is not a merged community model.`);
  }
  if (!Number.isInteger(block.version) || block.version < 1 || block.version > COMMUNITY_MODEL_VERSION) {
    throw new Error(`${filename}: merged model version ${block.version} is not supported by this build.`);
  }
  if (!Array.isArray(block.members) || block.members.length === 0) {
    throw new Error(`${filename}: the merged model lists no species.`);
  }
  if (!Array.isArray(raw.metabolites) || !Array.isArray(raw.reactions)) {
    throw new Error(`${filename}: the merged model has no metabolites or reactions array.`);
  }
  if (!Object.values(OBJECTIVES).includes(block.objective)) {
    throw new Error(`${filename}: unknown community objective "${block.objective}".`);
  }
  const medium = {};
  for (const [base, rate] of Object.entries(block.medium ?? {})) {
    if (!Number.isFinite(Number(rate))) {
      throw new Error(`${filename}: the medium supply of ${base} is not a number.`);
    }
    medium[base] = Number(rate);
  }

  const members = [];
  const seen = new Set();
  for (const member of block.members) {
    const id = member?.id;
    if (typeof id !== 'string' || !id || id.includes('@') || id.includes(GENE_SEPARATOR) || seen.has(id)) {
      throw new Error(`${filename}: a species in the merged model has a missing, repeated or invalid id.`);
    }
    seen.add(id);
    const prefix = namespaced(id, '');
    const mine = (value) => typeof value === 'string' && value.startsWith(prefix);
    const local = (value) => value.slice(prefix.length);
    const genePrefix = `${id}${GENE_SEPARATOR}`;
    const mineGene = (value) => typeof value === 'string' && value.startsWith(genePrefix);
    const localGene = (value) => value.slice(genePrefix.length);
    const saved = member.model ?? {};

    const compartments = {};
    for (const [compartmentId, name] of Object.entries(raw.compartments ?? {})) {
      if (mine(compartmentId)) {
        compartments[local(compartmentId)] = name;
      }
    }

    const tree = {};
    for (const [child, parent] of Object.entries(raw.compartment_tree ?? {})) {
      if (!mine(child)) continue;
      tree[local(child)] = mine(parent) ? local(parent) : null;
    }

    const metabolites = [];
    for (const met of raw.metabolites) {
      if (!mine(met?.id)) continue;
      const out = { ...met, id: local(met.id) };
      if (mine(met.compartment)) {
        out.compartment = local(met.compartment);
      }
      metabolites.push(out);
    }

    const coefficients = saved.objective_coefficients ?? {};
    const reactions = [];
    for (const rxn of raw.reactions) {
      if (!mine(rxn?.id)) continue;
      const stoichiometry = {};
      for (const [metaboliteId, coefficient] of Object.entries(rxn.metabolites ?? {})) {
        if (mine(metaboliteId)) {
          stoichiometry[local(metaboliteId)] = coefficient;
        }
      }
      const out = { ...rxn, id: local(rxn.id), metabolites: stoichiometry };
      out.gene_reaction_rule = renameRuleGenes(rxn.gene_reaction_rule, (gene) => (mineGene(gene) ? localGene(gene) : gene));
      delete out.objective_coefficient;
      if (coefficients[out.id]) {
        out.objective_coefficient = coefficients[out.id];
      }
      reactions.push(out);
    }

    const genes = [];
    for (const gene of raw.genes ?? []) {
      const geneId = typeof gene === 'string' ? gene : gene?.id;
      if (!mineGene(geneId)) continue;
      genes.push(typeof gene === 'string' ? localGene(gene) : { ...gene, id: localGene(geneId) });
    }

    const model = {
      id: saved.id || id,
      name: saved.name || saved.id || id,
      version: saved.version || '1',
      compartments,
      cell: saved.cell ?? null,
      compartment_tree: tree,
      metabolites,
      reactions,
      genes,
    };
    if (saved.objective && typeof saved.objective === 'object') {
      model.objective = { ...saved.objective };
    }
    members.push({ id, abundance: Number(member.abundance), model });
  }

  return {
    name: typeof raw.name === 'string' && raw.name ? raw.name : 'Imported community',
    objective: block.objective,
    productTarget: typeof block.productTarget === 'string' && block.productTarget ? block.productTarget : null,
    medium,
    members,
  };
}
