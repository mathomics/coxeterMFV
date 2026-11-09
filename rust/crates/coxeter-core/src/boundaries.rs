//! Boundaries, and which reactions cross them (`src/map/boundaries.js`).
//!
//! A boundary is a membrane: the surface of one compartment, separating what
//! is inside it from whatever it sits in. The outermost compartment of a cell
//! is separated from the shared medium. The shared medium itself is separated
//! from everything outside the system.
//!
//! Transport is decided by where a reaction's metabolites are, not by its
//! name: a reaction crosses the boundary of compartment X when at least one
//! of its metabolites is inside X (or nested within it) and at least one is
//! not. Everything here is pure. Geometry is the map layer's business.

use std::collections::HashSet;

use indexmap::IndexMap;

use crate::community::{CommunityModel, LINK_COMPARTMENT, SHARED_COMPARTMENT};
use crate::compartments::CompartmentTree;
use crate::js;
use crate::model::{Model, Reaction};

/// Boundary kinds, in the order the map draws them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BoundaryKind {
    /// The shared medium's own surface.
    Environment,
    /// A cell's outermost membrane.
    Cell,
    /// An organelle membrane inside a cell.
    Membrane,
}

impl BoundaryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            BoundaryKind::Environment => "environment",
            BoundaryKind::Cell => "cell",
            BoundaryKind::Membrane => "membrane",
        }
    }
}

/// Stand-in cell id for boundaries that belong to no cell.
pub const NO_CELL: &str = "*";

/// Stable identity of a boundary: which cell, and whose surface it is.
pub fn boundary_key(cell_id: Option<&str>, compartment_id: &str) -> String {
    format!("{}/{compartment_id}", cell_id.unwrap_or(NO_CELL))
}

/// The key of the one boundary that encloses the whole system.
pub fn environment_key() -> String {
    boundary_key(None, SHARED_COMPARTMENT)
}

/// For every compartment, the set containing it and everything nested in it,
/// in the post-order the JS walk produces.
pub fn compartment_subtrees(tree: &CompartmentTree) -> IndexMap<String, HashSet<String>> {
    fn walk(tree: &CompartmentTree, id: &str, out: &mut IndexMap<String, HashSet<String>>) -> HashSet<String> {
        let mut set = HashSet::from([id.to_string()]);
        if let Some(node) = tree.get(id) {
            for child in &node.children {
                set.extend(walk(tree, child, out));
            }
        }
        out.insert(id.to_string(), set.clone());
        set
    }
    let mut out = IndexMap::new();
    for root in &tree.roots {
        walk(tree, root, &mut out);
    }
    out
}

/// Where one metabolite of a reaction sits: which cell (None = the shared
/// medium) and which compartment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub cell_id: Option<String>,
    pub compartment: String,
}

/// Where a reaction's metabolites sit, inside a single species model. The
/// virtual growth-coupling compartment is skipped: it is a solver device.
pub fn model_reaction_places(reaction: &Reaction, model: &Model) -> Vec<Place> {
    reaction
        .metabolites
        .keys()
        .filter_map(|mid| model.metabolites.get(mid))
        .filter(|m| m.compartment != LINK_COMPARTMENT)
        .map(|m| Place { cell_id: None, compartment: m.compartment.clone() })
        .collect()
}

/// Where a merged reaction's metabolites sit, in community terms.
pub fn community_reaction_places(community: &CommunityModel, reaction: &Reaction) -> Vec<Place> {
    reaction
        .metabolites
        .keys()
        .filter_map(|mid| community.metabolite_owner.get(mid))
        .filter(|o| o.compartment != LINK_COMPARTMENT)
        .map(|o| Place { cell_id: o.species_id.clone(), compartment: o.compartment.clone() })
        .collect()
}

/// Is this set of places inside-and-outside the given compartment subtree?
pub fn crosses_subtree(places: &[Place], cell_id: Option<&str>, subtree: &HashSet<String>) -> bool {
    let mut inside = false;
    let mut outside = false;
    for place in places {
        if place.cell_id.as_deref() == cell_id && subtree.contains(&place.compartment) {
            inside = true;
        } else {
            outside = true;
        }
        if inside && outside {
            return true;
        }
    }
    false
}

/// Does this reaction cross the surface of the shared medium? An exchange of
/// the merged model is a half-reaction: exactly one metabolite, in the medium.
pub fn crosses_environment(places: &[Place]) -> bool {
    places.len() == 1 && places[0].cell_id.is_none() && places[0].compartment == SHARED_COMPARTMENT
}

/// The compartments whose membranes this reaction crosses, within one species
/// model, sorted.
pub fn reaction_boundaries(
    reaction: &Reaction,
    model: &Model,
    subtrees: Option<&IndexMap<String, HashSet<String>>>,
) -> Vec<String> {
    let owned;
    let sets = match subtrees {
        Some(s) => s,
        None => {
            owned = compartment_subtrees(&model.compartment_tree());
            &owned
        }
    };
    let places = model_reaction_places(reaction, model);
    let mut out: Vec<String> =
        sets.iter().filter(|(_, sub)| crosses_subtree(&places, None, sub)).map(|(id, _)| id.clone()).collect();
    out.sort_by(|a, b| js::code_unit_compare(a, b));
    out
}

/// Is this reaction transport, i.e. does it cross at least one membrane?
pub fn is_transport_reaction(reaction: &Reaction, model: &Model, subtrees: Option<&IndexMap<String, HashSet<String>>>) -> bool {
    !reaction_boundaries(reaction, model, subtrees).is_empty()
}

/// Every reaction of one model grouped by the compartment membrane it
/// crosses. Compartments nothing crosses are present with an empty list.
pub fn model_boundary_index(model: &Model) -> IndexMap<String, Vec<String>> {
    let subtrees = compartment_subtrees(&model.compartment_tree());
    let mut out: IndexMap<String, Vec<String>> = subtrees.keys().map(|k| (k.clone(), vec![])).collect();
    for reaction in model.reactions.values() {
        for id in reaction_boundaries(reaction, model, Some(&subtrees)) {
            out.get_mut(&id).expect("subtree ids").push(reaction.id.clone());
        }
    }
    for list in out.values_mut() {
        list.sort_by(|a, b| js::code_unit_compare(a, b));
    }
    out
}

/// Identity of one boundary, without geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct BoundaryInfo {
    pub id: String,
    pub kind: BoundaryKind,
    pub cell_id: Option<String>,
    pub inside: String,
    /// None: beyond the system.
    pub outside: Option<String>,
    pub parent_id: Option<String>,
    pub depth: i64,
}

/// Every boundary a community has, from its biology alone.
pub fn community_boundaries(community: &CommunityModel) -> Vec<BoundaryInfo> {
    let mut out = vec![BoundaryInfo {
        id: environment_key(),
        kind: BoundaryKind::Environment,
        cell_id: None,
        inside: SHARED_COMPARTMENT.into(),
        outside: None,
        parent_id: None,
        depth: -1,
    }];
    for sp in community.active_species() {
        let tree = sp.model.compartment_tree();
        for node in tree.by_id.values() {
            out.push(BoundaryInfo {
                id: boundary_key(Some(&sp.id), &node.id),
                kind: if node.parent.is_none() { BoundaryKind::Cell } else { BoundaryKind::Membrane },
                cell_id: Some(sp.id.clone()),
                inside: node.id.clone(),
                // The outermost membrane of a cell faces the shared medium.
                outside: Some(node.parent.clone().unwrap_or_else(|| SHARED_COMPARTMENT.into())),
                parent_id: node.parent.clone(),
                depth: node.depth as i64,
            });
        }
    }
    out
}

/// Per-species compartment subtrees, computed once for bulk callers.
pub type SubtreeCache = IndexMap<String, IndexMap<String, HashSet<String>>>;

/// The boundary keys one merged reaction crosses, sorted.
pub fn reaction_boundary_keys(community: &CommunityModel, reaction_id: &str, cache: Option<&mut SubtreeCache>) -> Vec<String> {
    let Some(reaction) = community.merged.reactions.get(reaction_id) else { return vec![] };
    let places = community_reaction_places(community, reaction);
    let mut out = Vec::new();
    if crosses_environment(&places) {
        out.push(environment_key());
    }
    let mut local = SubtreeCache::new();
    let cache = match cache {
        Some(c) => c,
        None => &mut local,
    };
    for sp in community.active_species() {
        if !cache.contains_key(&sp.id) {
            cache.insert(sp.id.clone(), compartment_subtrees(&sp.model.compartment_tree()));
        }
        for (comp_id, subtree) in &cache[&sp.id] {
            if crosses_subtree(&places, Some(&sp.id), subtree) {
                out.push(boundary_key(Some(&sp.id), comp_id));
            }
        }
    }
    out.sort_by(|a, b| js::code_unit_compare(a, b));
    out
}

/// Every merged reaction grouped by the boundary it crosses. Boundaries with
/// nothing on them are present with an empty list.
pub fn reactions_by_boundary(community: &CommunityModel) -> IndexMap<String, Vec<String>> {
    let mut out: IndexMap<String, Vec<String>> = community_boundaries(community).into_iter().map(|b| (b.id, vec![])).collect();
    let mut cache = SubtreeCache::new();
    for id in community.merged.reactions.keys() {
        for key in reaction_boundary_keys(community, id, Some(&mut cache)) {
            out.entry(key).or_default().push(id.clone());
        }
    }
    for list in out.values_mut() {
        list.sort_by(|a, b| js::code_unit_compare(a, b));
    }
    out
}

/// Reactions the model offers on one boundary, by key.
pub fn reactions_crossing(community: &CommunityModel, key: &str) -> Vec<String> {
    reactions_by_boundary(community).get(key).cloned().unwrap_or_default()
}
