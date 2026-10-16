//! What the analysis functions read from a community.
//!
//! In the JavaScript app `store.community` is whichever object the active
//! scenario built: a `CommunityModel`, the exact source-matrix community of the
//! Zuñiga case, or the projection of a joint SteadyCom model. The explorer and
//! the omics mapper only ever ask it four things, and this trait is those four.

use coxeter_core::CommunityModel;
use coxeter_core::community::{CrossFeeding, ReactionOwner};
use indexmap::IndexMap;

/// One reaction joining a cell to the shared medium.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferRef<'a> {
    pub species_id: &'a str,
    pub reaction_id: &'a str,
}

/// A community as the cross-feeding explorer and the omics overlay see it.
pub trait CommunityView {
    /// Ids of the merged model's reactions, in model order.
    fn merged_reaction_ids(&self) -> Vec<&str>;

    /// Which species a merged reaction belongs to, and its id there.
    fn reaction_owner(&self, reaction_id: &str) -> Option<&ReactionOwner>;

    /// The transfers of one medium metabolite, in the order they were found.
    fn transfers(&self, base: &str) -> Vec<TransferRef<'_>>;

    /// Who feeds whom at these fluxes.
    fn cross_feeding(&self, fluxes: &IndexMap<String, f64>, threshold: f64) -> CrossFeeding;
}

impl CommunityView for CommunityModel {
    fn merged_reaction_ids(&self) -> Vec<&str> {
        self.merged.reactions.keys().map(String::as_str).collect()
    }

    fn reaction_owner(&self, reaction_id: &str) -> Option<&ReactionOwner> {
        self.reaction_owner.get(reaction_id)
    }

    fn transfers(&self, base: &str) -> Vec<TransferRef<'_>> {
        self.transfers_by_base
            .get(base)
            .map(|list| list.iter().map(|t| TransferRef { species_id: &t.species_id, reaction_id: &t.reaction_id }).collect())
            .unwrap_or_default()
    }

    fn cross_feeding(&self, fluxes: &IndexMap<String, f64>, threshold: f64) -> CrossFeeding {
        CommunityModel::cross_feeding(self, fluxes, threshold)
    }
}
