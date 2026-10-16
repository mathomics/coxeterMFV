/**
 * Boundaries, and which reactions cross them.
 *
 * A boundary is a membrane: the surface of one compartment, separating what is
 * inside it from whatever it sits in. The outermost compartment of a cell is
 * separated from the shared medium; the shared medium itself is separated from
 * everything outside the system.
 *
 * Everything here is pure. It reads a `Model` or a `CommunityModel` and answers
 * questions about them; it never touches geometry, the map, or the solver. The
 * map layer turns these answers into drawable objects.
 *
 * How transport is decided
 * Not by name. `t`, `tex`, `abc`, `EX_` and friends are conventions, not facts,
 * and a user-supplied model is free to ignore them. What makes a reaction cross
 * a membrane is where its metabolites are:
 *
 *   a reaction crosses the boundary of compartment X
 *     <=>  at least one of its metabolites is inside X or inside something
 *          nested within X,
 *     and  at least one is not.
 *
 * That definition falls out of the compartment hierarchy alone, and it handles
 * multi-boundary transport for free. A reaction with metabolites in `c` and `e`,
 * nested c ⊂ p ⊂ w ⊂ e, is inside-and-outside for the surfaces of `c`, `p` and
 * `w` (it really does cross three membranes) and wholly inside `e`, which it
 * does not cross. A reaction whose metabolites all live in one compartment is
 * never inside-and-outside for anything, so it is never transport.
 */

import { LINK_COMPARTMENT, SHARED_COMPARTMENT } from '../core/community.js';

/** Boundary kinds, in the order the map draws them. */
export const BOUNDARY_KIND = {
  ENVIRONMENT: 'environment',  // the shared medium's own surface
  CELL: 'cell',                // a cell's outermost membrane
  MEMBRANE: 'membrane',        // an organelle membrane inside a cell
};

/** Stand-in cell id for boundaries that belong to no cell. */
export const NO_CELL = '*';

/** Stable identity of a boundary: which cell, and whose surface it is. */
export const boundaryKey = (cellId, compartmentId) => `${cellId ?? NO_CELL}/${compartmentId}`;

/** The key of the one boundary that encloses the whole system. */
export const ENVIRONMENT_KEY = boundaryKey(null, SHARED_COMPARTMENT);

// Hierarchy

/**
 * For every compartment, the set containing it and everything nested inside it.
 *
 * This is the only thing the crossing test needs from the hierarchy, and
 * computing it once turns the test into two set lookups per metabolite.
 *
 * @param {{roots: Array, byId: Map}} tree result of `Model#compartmentTree()`
 * @returns {Map<string, Set<string>>}
 */
export function compartmentSubtrees(tree) {
  const out = new Map();
  const walk = (node) => {
    const set = new Set([node.id]);
    for (const child of node.children) {
      for (const id of walk(child)) set.add(id);
    }
    out.set(node.id, set);
    return set;
  };
  for (const root of tree.roots) walk(root);
  return out;
}

// Places

/**
 * @typedef {object} Place
 * @property {string|null} cellId  which cell, or null for the shared medium
 * @property {string} compartment
 */

/**
 * Where a reaction's metabolites sit, inside a single species model.
 *
 * The virtual growth-coupling compartment is skipped: it is a solver device, not
 * a place, and counting it would make every biomass reaction look like transport
 * across every membrane in the cell.
 *
 * @returns {Place[]}
 */
export function modelReactionPlaces(reaction, model) {
  const out = [];
  for (const mid of Object.keys(reaction.metabolites)) {
    const met = model.metabolites.get(mid);
    if (!met || met.compartment === LINK_COMPARTMENT) continue;
    out.push({ cellId: null, compartment: met.compartment });
  }
  return out;
}

/**
 * Where a merged reaction's metabolites sit, in community terms.
 *
 * Compartment ids are local to a species (two cells both have a `c`), so a
 * place is only meaningful together with the species that owns it. Shared-medium
 * metabolites belong to no cell.
 *
 * @returns {Place[]}
 */
export function communityReactionPlaces(community, reaction) {
  const out = [];
  for (const mid of Object.keys(reaction.metabolites)) {
    const owner = community.metaboliteOwner.get(mid);
    if (!owner || owner.compartment === LINK_COMPARTMENT) continue;
    out.push({ cellId: owner.speciesId ?? null, compartment: owner.compartment });
  }
  return out;
}

// Crossing

/**
 * Is this set of places inside-and-outside the given compartment subtree?
 *
 * @param {Place[]} places
 * @param {string|null} cellId    the cell the subtree belongs to
 * @param {Set<string>} subtree   the compartment and everything nested in it
 */
export function crossesSubtree(places, cellId, subtree) {
  let inside = false;
  let outside = false;
  for (const place of places) {
    if (place.cellId === cellId && subtree.has(place.compartment)) inside = true;
    else outside = true;
    if (inside && outside) return true;
  }
  return false;
}

/**
 * Does this reaction cross the surface of the shared medium?
 *
 * The medium has no "outside" written down anywhere: in the merged model an
 * exchange is a half-reaction, a single metabolite whose partner is implicitly
 * beyond the system. That structural shape, exactly one metabolite in the
 * shared medium, is what identifies a system-boundary reaction. No name is
 * consulted.
 *
 * @param {Place[]} places
 */
export function crossesEnvironment(places) {
  return places.length === 1
    && places[0].cellId === null
    && places[0].compartment === SHARED_COMPARTMENT;
}

// Single model (one species)

/**
 * The compartments whose membranes this reaction crosses, within one species
 * model. Returns the ids sorted, so the answer is stable.
 *
 * @param {import('../core/model.js').Reaction} reaction
 * @param {import('../core/model.js').Model} model
 * @param {Map<string, Set<string>>} [subtrees] precomputed, for bulk callers
 * @returns {string[]}
 */
export function reactionBoundaries(reaction, model, subtrees = null) {
  const sets = subtrees ?? compartmentSubtrees(model.compartmentTree());
  const places = modelReactionPlaces(reaction, model);
  const out = [];
  for (const [id, subtree] of sets) {
    if (crossesSubtree(places, null, subtree)) out.push(id);
  }
  return out.sort();
}

/** Is this reaction transport, i.e. does it cross at least one membrane? */
export function isTransportReaction(reaction, model, subtrees = null) {
  return reactionBoundaries(reaction, model, subtrees).length > 0;
}

/**
 * Every reaction of one model, grouped by the compartment membrane it crosses.
 * Compartments no reaction crosses are present with an empty list, so a caller
 * can tell "no transport here" from "unknown compartment".
 *
 * @returns {Map<string, string[]>} compartment id -> reaction ids, sorted
 */
export function modelBoundaryIndex(model) {
  const subtrees = compartmentSubtrees(model.compartmentTree());
  const out = new Map();
  for (const id of subtrees.keys()) out.set(id, []);
  for (const reaction of model.reactions.values()) {
    for (const id of reactionBoundaries(reaction, model, subtrees)) {
      out.get(id).push(reaction.id);
    }
  }
  for (const list of out.values()) list.sort();
  return out;
}

// Community

/**
 * Identity of every boundary a community has, without any geometry.
 *
 * The map layer adds the rectangles; this is the part that depends only on the
 * biology, so it can be computed before anything is drawn, including for a map
 * that draws no reactions at all.
 *
 * @returns {Array<{id:string, kind:string, cellId:string|null, inside:string,
 *   outside:string|null, parentId:string|null, depth:number}>}
 */
export function communityBoundaries(community) {
  const out = [{
    id: ENVIRONMENT_KEY,
    kind: BOUNDARY_KIND.ENVIRONMENT,
    cellId: null,
    inside: SHARED_COMPARTMENT,
    outside: null,              // beyond the system
    parentId: null,
    depth: -1,
  }];

  for (const sp of community.activeSpecies) {
    const { byId } = sp.model.compartmentTree();
    for (const node of byId.values()) {
      out.push({
        id: boundaryKey(sp.id, node.id),
        kind: node.parent === null ? BOUNDARY_KIND.CELL : BOUNDARY_KIND.MEMBRANE,
        cellId: sp.id,
        inside: node.id,
        // The outermost membrane of a cell faces the shared medium.
        outside: node.parent ?? SHARED_COMPARTMENT,
        parentId: node.parent,
        depth: node.depth,
      });
    }
  }
  return out;
}

/**
 * The boundaries one merged reaction crosses.
 *
 * A reaction may cross several: a cytosol-to-medium transporter leaves the
 * cytosol, the periplasm and the wall before it reaches the medium.
 *
 * @param {import('../core/community.js').CommunityModel} community
 * @param {string} reactionId  id in the merged model
 * @param {Map<string, Map<string, Set<string>>>} [cache] species id -> subtrees
 * @returns {string[]} boundary keys, sorted
 */
export function reactionBoundaryKeys(community, reactionId, cache = null) {
  const reaction = community.merged.reactions.get(reactionId);
  if (!reaction) return [];
  const places = communityReactionPlaces(community, reaction);
  const out = [];

  if (crossesEnvironment(places)) out.push(ENVIRONMENT_KEY);

  for (const sp of community.activeSpecies) {
    let subtrees = cache?.get(sp.id);
    if (!subtrees) {
      subtrees = compartmentSubtrees(sp.model.compartmentTree());
      cache?.set(sp.id, subtrees);
    }
    for (const [compId, subtree] of subtrees) {
      if (crossesSubtree(places, sp.id, subtree)) out.push(boundaryKey(sp.id, compId));
    }
  }
  return out.sort();
}

/**
 * Every merged reaction grouped by the boundary it crosses.
 *
 * This is what a future "what can I add here?" interaction needs: given a
 * membrane the user pointed at, the reactions the model makes available on it.
 * Boundaries with nothing on them are present with an empty list.
 *
 * @returns {Map<string, string[]>} boundary key -> merged reaction ids, sorted
 */
export function reactionsByBoundary(community) {
  const out = new Map();
  for (const b of communityBoundaries(community)) out.set(b.id, []);

  const cache = new Map();
  for (const id of community.merged.reactions.keys()) {
    for (const key of reactionBoundaryKeys(community, id, cache)) {
      if (!out.has(key)) out.set(key, []);
      out.get(key).push(id);
    }
  }
  for (const list of out.values()) list.sort();
  return out;
}

/** Reactions the model offers on one boundary, by key. */
export function reactionsCrossing(community, key) {
  return reactionsByBoundary(community).get(key) ?? [];
}
