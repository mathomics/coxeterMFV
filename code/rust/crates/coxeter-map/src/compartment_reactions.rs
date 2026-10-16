//! Reactions that live entirely inside one concrete cellular compartment
//! (port of `src/map/compartment-reactions.js`).
//!
//! Transport reactions are deliberately excluded because at least one of
//! their metabolites belongs somewhere else. The membrane UI offers those.

use coxeter_core::community::{CommunityModel, LINK_COMPARTMENT};
use coxeter_core::js;

/// `reactionsInCompartment(community, cellId, compartmentId)`: the merged
/// reaction ids whose every real metabolite is in that compartment of that
/// cell, sorted.
pub fn reactions_in_compartment(community: &CommunityModel, cell_id: &str, compartment_id: &str) -> Vec<String> {
    if cell_id.is_empty() || compartment_id.is_empty() {
        return vec![];
    }
    let mut out = Vec::new();
    for (reaction_id, reaction) in &community.merged.reactions {
        let mut has_real = false;
        let mut belongs_here = true;
        let mut has_virtual_link = false;
        for mid in reaction.metabolites.keys() {
            let Some(owner) = community.metabolite_owner.get(mid) else { continue };
            if owner.compartment == LINK_COMPARTMENT {
                has_virtual_link = true;
                break;
            }
            has_real = true;
            if owner.species_id.as_deref() != Some(cell_id) || owner.compartment != compartment_id {
                belongs_here = false;
                break;
            }
        }
        if !has_virtual_link && has_real && belongs_here {
            out.push(reaction_id.clone());
        }
    }
    out.sort_by(|a, b| js::code_unit_compare(a, b));
    out
}
