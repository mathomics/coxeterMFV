/**
 * Reactions that live entirely inside one concrete cellular compartment.
 *
 * Transport reactions are deliberately excluded because at least one of their
 * metabolites belongs somewhere else. Those are exposed by the membrane UI.
 */
import { LINK_COMPARTMENT } from '../core/community.js';

/**
 * Merged reaction ids whose metabolites all sit in one compartment of one cell.
 * Reactions touching the virtual growth-coupling compartment are skipped.
 *
 * @param {import('../core/community.js').CommunityModel} community
 * @param {string} cellId         species id of the cell
 * @param {string} compartmentId  compartment id local to that species
 * @returns {string[]} merged reaction ids, sorted (empty when an argument is missing)
 */
export function reactionsInCompartment(community, cellId, compartmentId) {
  if (!community || !cellId || !compartmentId) return [];

  const out = [];
  for (const [reactionId, reaction] of community.merged.reactions) {
    let hasRealMetabolite = false;
    let belongsHere = true;
    let hasVirtualLink = false;

    for (const mid of Object.keys(reaction.metabolites)) {
      const owner = community.metaboliteOwner?.get(mid);
      if (!owner) continue;
      if (owner.compartment === LINK_COMPARTMENT) {
        hasVirtualLink = true;
        break;
      }

      hasRealMetabolite = true;
      if (owner.speciesId !== cellId || owner.compartment !== compartmentId) {
        belongsHere = false;
        break;
      }
    }

    if (!hasVirtualLink && hasRealMetabolite && belongsHere) out.push(reactionId);
  }

  return out.sort();
}
