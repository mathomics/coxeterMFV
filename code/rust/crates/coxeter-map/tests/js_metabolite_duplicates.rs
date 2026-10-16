//! Port of `test/metabolite-duplicates.test.mjs`: a metabolite may be drawn
//! more than once, and the copies are map furniture, never biology.
//!
//! Not ported: "duplicating a metabolite leaves FBA completely unchanged" —
//! it solves the merged model twice with `FluxProblem` (crate
//! `coxeter-solver`, not available here); the map never holds the model, so
//! there is nothing of the map left to assert once the solver is taken out.

mod common;

use std::sync::Arc;

use common::load_model;
use coxeter_core::community::{CommunityModel, CommunitySpec, LINK_COMPARTMENT, MediumLimit, SpeciesSpec};
use coxeter_core::js;
use coxeter_core::mapdata::{Node, NodeKind, Point};
use coxeter_core::model::ReactionKind;
use coxeter_map::editable_map::DuplicateOptions;
use coxeter_map::layout::LayoutOptions;
use coxeter_map::{EditableMap, PlaceOptions};
use serde_json::Value;

/// `buildCommunity()`.
fn build_community() -> CommunityModel {
    let limit = |lower: f64| MediumLimit { lower: Some(lower), upper: None };
    CommunityModel::new(CommunitySpec {
        species: vec![SpeciesSpec::new("eco", Arc::new(load_model("ecoli_core_mc"))).abundance(1.0)],
        medium: [("glc__D", limit(-10.0)), ("o2", limit(-20.0)), ("nh4", limit(-1000.0)), ("pi", limit(-1000.0))]
            .into_iter()
            .map(|(base, l)| (base.to_string(), l))
            .collect(),
        ..Default::default()
    })
    .unwrap()
}

/// `EditableMap.emptyFromCommunity(com)`.
fn empty_map(com: &CommunityModel) -> EditableMap {
    EditableMap::empty_from_community(com, &LayoutOptions::default())
}

/// `firstInternalReaction(com)`.
fn first_internal_reaction(com: &CommunityModel) -> Option<String> {
    for (rid, rxn) in &com.merged.reactions {
        let real = rxn
            .metabolites
            .keys()
            .filter(|mid| com.merged.metabolites.get(*mid).is_some_and(|met| met.compartment != LINK_COMPARTMENT))
            .count();
        if real >= 2 && rxn.kind == ReactionKind::Metabolic {
            return Some(rid.clone());
        }
    }
    None
}

/// `anotherReactionFor(com, mergedId, excluded)`.
fn another_reaction_for(com: &CommunityModel, merged_id: &str, excluded: &str) -> Option<String> {
    for (rid, rxn) in &com.merged.reactions {
        if rid == excluded {
            continue;
        }
        if rxn.metabolites.contains_key(merged_id) {
            return Some(rid.clone());
        }
    }
    None
}

fn is_metabolite_or_medium(node: &Node) -> bool {
    matches!(node.kind, NodeKind::Metabolite | NodeKind::Medium)
}

/// `[...map.nodes.values()].find((node) => node.kind === 'metabolite')`.
fn first_metabolite(map: &EditableMap) -> Option<Node> {
    map.nodes().values().find(|n| n.kind == NodeKind::Metabolite).cloned()
}

/// `map.duplicateMetaboliteNode(id)`, the copy as it is right after.
fn duplicate(map: &mut EditableMap, id: &str) -> Option<Node> {
    map.duplicate_metabolite_node(id, &DuplicateOptions::default()).cloned()
}

/// `JSON.parse(JSON.stringify(map.toJSON()))`.
fn through_json(map: &EditableMap) -> Value {
    serde_json::from_str(&js::stringify(&map.to_json(), None)).unwrap()
}

/// JS: "duplicating a metabolite creates a second visual instance with the same biological identity"
#[test]
fn duplicating_a_metabolite_creates_a_second_visual_instance_with_the_same_biological_identity() {
    let com = build_community();
    let mut map = empty_map(&com);
    let rid = first_internal_reaction(&com).unwrap();
    map.add_reaction(&com, &rid, &PlaceOptions::default());

    let original = map.nodes().values().find(|n| is_metabolite_or_medium(n)).cloned();
    assert!(original.is_some());
    let original = original.unwrap();

    let biological_count = com.merged.metabolites.len();
    let copy = duplicate(&mut map, &original.id);
    assert!(copy.is_some());
    let copy = copy.unwrap();
    assert_ne!(copy.id, original.id);
    assert_eq!(copy.merged_id, original.merged_id);
    assert_eq!(copy.local_id, original.local_id);
    assert_eq!(copy.manual_duplicate, Some(true));
    assert_eq!(map.nodes_for_metabolite(original.merged_id.as_deref().unwrap()).len(), 2);
    assert_eq!(com.merged.metabolites.len(), biological_count);
}

/// JS: "multiple duplicates receive unique node IDs"
#[test]
fn multiple_duplicates_receive_unique_node_ids() {
    let com = build_community();
    let mut map = empty_map(&com);
    let rid = first_internal_reaction(&com).unwrap();
    map.add_reaction(&com, &rid, &PlaceOptions::default());
    let original = first_metabolite(&map);
    assert!(original.is_some());
    let original = original.unwrap();

    let a = duplicate(&mut map, &original.id).unwrap();
    let b = duplicate(&mut map, &original.id).unwrap();
    let c = duplicate(&mut map, &a.id).unwrap();

    let ids: std::collections::HashSet<&str> = [&original.id, &a.id, &b.id, &c.id].into_iter().map(String::as_str).collect();
    assert_eq!(ids.len(), 4);
    assert_eq!(a.merged_id, original.merged_id);
    assert_eq!(b.merged_id, original.merged_id);
    assert_eq!(c.merged_id, original.merged_id);
}

/// JS: "moving and styling one visual copy does not alter the original"
#[test]
fn moving_and_styling_one_visual_copy_does_not_alter_the_original() {
    let com = build_community();
    let mut map = empty_map(&com);
    map.add_reaction(&com, &first_internal_reaction(&com).unwrap(), &PlaceOptions::default());
    let original = first_metabolite(&map).unwrap();
    // `{ x, y, r, hidden: Boolean(original.hidden) }`.
    let state = |n: &Node| (n.x, n.y, n.r, n.hidden == Some(true));
    let before = state(&original);

    let copy = duplicate(&mut map, &original.id).unwrap();
    map.move_node(&copy.id, copy.x + 25.0, copy.y + 18.0);
    map.set_node_primary(&copy.id, false);
    map.set_node_hidden(&copy.id, true);

    assert_eq!(state(map.node(&original.id).unwrap()), before);
    let copy = map.node(&copy.id).unwrap();
    assert_eq!(copy.r, 7.0);
    assert_eq!(copy.visual_role.as_deref(), Some("secondary"));
    assert_eq!(copy.hidden, Some(true));
}

/// JS: "adding a reaction without a selected anchor never auto-links to an existing visual instance"
#[test]
fn adding_a_reaction_without_a_selected_anchor_never_auto_links_to_an_existing_visual_instance() {
    let com = build_community();
    let mut map = empty_map(&com);

    let mut source_reaction = None;
    let mut source_mid = None;
    let mut target_reaction = None;

    'reactions: for (rid, rxn) in &com.merged.reactions {
        for mid in rxn.metabolites.keys() {
            let Some(met) = com.merged.metabolites.get(mid) else { continue };
            if met.compartment == LINK_COMPARTMENT {
                continue;
            }
            if let Some(target) = another_reaction_for(&com, mid, rid) {
                source_reaction = Some(rid.clone());
                source_mid = Some(mid.clone());
                target_reaction = Some(target);
                break 'reactions;
            }
        }
    }

    assert!(source_reaction.is_some() && source_mid.is_some() && target_reaction.is_some());
    let (source_reaction, source_mid, target_reaction) =
        (source_reaction.unwrap(), source_mid.unwrap(), target_reaction.unwrap());
    assert!(map.add_reaction(&com, &source_reaction, &PlaceOptions::default()).is_some());

    let original = map.nodes_for_metabolite(&source_mid).first().map(|n| (*n).clone());
    assert!(original.is_some());
    let original = original.unwrap();
    let copy = map
        .duplicate_metabolite_node(
            &original.id,
            &DuplicateOptions { x: Some(original.x + 55.0), y: Some(original.y + 35.0), ..Default::default() },
        )
        .cloned();
    assert!(copy.is_some());
    let copy = copy.unwrap();

    let result = map.add_reaction(
        &com,
        &target_reaction,
        &PlaceOptions { anchor: Some(Point::new(copy.x + 82.0, copy.y)), ..Default::default() },
    );
    assert!(result.is_some());

    let reused = map.segments_of(&target_reaction).into_iter().find(|seg| {
        seg.node_id.as_ref().is_some_and(|id| {
            (id == &copy.id || id == &original.id)
                && map.node(id).and_then(|n| n.merged_id.as_deref()) == Some(source_mid.as_str())
        })
    });
    assert!(reused.is_none(), "new reaction reused a pre-existing metabolite node");

    let fresh = map
        .nodes_for_metabolite(&source_mid)
        .into_iter()
        .find(|node| node.reaction_local_for.as_deref() == Some(target_reaction.as_str()));
    assert!(fresh.is_some(), "new reaction did not create its own metabolite instance");
    let fresh = fresh.unwrap();
    assert_ne!(fresh.id, original.id);
    assert_ne!(fresh.id, copy.id);
    assert!(
        map.segments_of(&target_reaction).iter().any(|seg| seg.node_id.as_deref() == Some(fresh.id.as_str())),
        "new reaction is not connected to its fresh metabolite instance"
    );
}

/// JS: "a manual duplicate survives removal of a reaction that used it"
#[test]
fn a_manual_duplicate_survives_removal_of_a_reaction_that_used_it() {
    let com = build_community();
    let mut map = empty_map(&com);

    let source = first_internal_reaction(&com).unwrap();
    map.add_reaction(&com, &source, &PlaceOptions::default());
    let original = first_metabolite(&map).unwrap();
    let merged_id = original.merged_id.clone().unwrap();
    let target = another_reaction_for(&com, &merged_id, &source);
    assert!(target.is_some());
    let target = target.unwrap();

    let copy = duplicate(&mut map, &original.id).unwrap();
    map.add_reaction(
        &com,
        &target,
        &PlaceOptions {
            anchor: Some(Point::new(copy.x + 82.0, copy.y)),
            anchor_node_id: Some(copy.id.clone()),
            ..Default::default()
        },
    );

    assert!(
        map.segments_of(&target).iter().any(|seg| seg.node_id.as_deref() == Some(copy.id.as_str())),
        "selected metabolite was not reused as the reaction anchor"
    );
    let fresh = map
        .nodes_for_metabolite(&merged_id)
        .into_iter()
        .find(|node| node.reaction_local_for.as_deref() == Some(target.as_str()));
    assert!(fresh.is_none(), "selected anchor was duplicated");

    map.remove_reaction(&target);
    assert!(map.has_node(&copy.id), "pre-existing selected anchor was pruned with the reaction");
    assert_eq!(map.segments_at(&copy.id).len(), 0);
}

/// JS: "duplicate visual instances survive map JSON round trip"
#[test]
fn duplicate_visual_instances_survive_map_json_round_trip() {
    let com = build_community();
    let mut map = empty_map(&com);
    map.add_reaction(&com, &first_internal_reaction(&com).unwrap(), &PlaceOptions::default());
    let original = first_metabolite(&map).unwrap();
    let copy = duplicate(&mut map, &original.id).unwrap();
    map.set_node_primary(&copy.id, false);

    let restored = EditableMap::from_json(&through_json(&map)).unwrap();
    let restored_copy = restored.node(&copy.id);

    assert!(restored_copy.is_some());
    let restored_copy = restored_copy.unwrap();
    assert_eq!(restored_copy.merged_id, original.merged_id);
    assert_eq!(restored_copy.manual_duplicate, Some(true));
    assert_eq!(restored_copy.r, 7.0);
    assert_eq!(restored_copy.visual_role.as_deref(), Some("secondary"));
    assert_eq!(restored.nodes_for_metabolite(original.merged_id.as_deref().unwrap()).len(), 2);
}

/// JS: "model JSON still exports one biological metabolite despite multiple visual copies"
#[test]
fn model_json_still_exports_one_biological_metabolite_despite_multiple_visual_copies() {
    let com = build_community();
    let mut map = empty_map(&com);
    map.add_reaction(&com, &first_internal_reaction(&com).unwrap(), &PlaceOptions::default());

    let original = first_metabolite(&map);
    assert!(original.is_some());
    let original = original.unwrap();
    duplicate(&mut map, &original.id);
    duplicate(&mut map, &original.id);

    let owner = com.metabolite_owner.get(original.merged_id.as_deref().unwrap());
    let species = com.active_species().find(|entry| Some(&entry.id) == owner.and_then(|o| o.species_id.as_ref()));
    assert!(species.is_some());

    let local_id = &owner.unwrap().original_id;
    let exported = species.unwrap().model.to_json();
    let count = exported["metabolites"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|met| met["id"].as_str() == Some(local_id.as_str()))
        .count();
    assert_eq!(count, 1);
}

/// JS: "drag-over merge rewires both reactions to one metabolite instance"
#[test]
fn drag_over_merge_rewires_both_reactions_to_one_metabolite_instance() {
    let com = build_community();
    let mut map = empty_map(&com);

    let source_reaction = first_internal_reaction(&com).unwrap();
    assert!(map.add_reaction(&com, &source_reaction, &PlaceOptions::default()).is_some());
    let source_node = map
        .nodes()
        .values()
        .find(|node| {
            is_metabolite_or_medium(node)
                && another_reaction_for(&com, node.merged_id.as_deref().unwrap_or(""), &source_reaction).is_some()
        })
        .cloned();
    assert!(source_node.is_some(), "no shared primary metabolite found");
    let source_node = source_node.unwrap();
    let merged_id = source_node.merged_id.clone().unwrap();

    let target_reaction = another_reaction_for(&com, &merged_id, &source_reaction);
    assert!(target_reaction.is_some());
    let target_reaction = target_reaction.unwrap();
    assert!(map.add_reaction(&com, &target_reaction, &PlaceOptions::default()).is_some());

    let copies = map.nodes_for_metabolite(&merged_id);
    let first = copies.iter().find(|node| node.reaction_local_for.as_deref() == Some(source_reaction.as_str())).map(|n| (*n).clone());
    let second = copies.iter().find(|node| node.reaction_local_for.as_deref() == Some(target_reaction.as_str())).map(|n| (*n).clone());
    assert!(first.is_some() && second.is_some());
    let (first, second) = (first.unwrap(), second.unwrap());
    assert_ne!(first.id, second.id, "reactions were auto-linked before the drag");

    assert!(map.move_node(&second.id, first.x, first.y).is_some());
    let merged = map.merge_overlapping_metabolite(&second.id);
    assert!(merged.is_some());
    assert_eq!(merged.unwrap().target.id, first.id);
    assert!(!map.has_node(&second.id));
    assert!(map.has_node(&first.id));

    for rid in [&source_reaction, &target_reaction] {
        let segments = map.segments_of(rid);
        let branch = segments.iter().find(|seg| {
            seg.node_id.as_deref().is_some_and(|id| {
                !id.is_empty() && map.node(id).and_then(|n| n.merged_id.as_deref()) == Some(merged_id.as_str())
            })
        });
        assert!(branch.is_some(), "{rid} lost the merged metabolite branch");
        assert_eq!(branch.unwrap().node_id.as_deref(), Some(first.id.as_str()), "{rid} did not rewire to the merge target");
    }

    map.remove_reaction(&source_reaction);
    assert!(map.has_node(&first.id), "shared merged node was deleted with its original reaction");
    assert_eq!(map.node(&first.id).and_then(|n| n.reaction_local_for.clone()), None);
    assert!(map.segments_of(&target_reaction).iter().any(|seg| seg.node_id.as_deref() == Some(first.id.as_str())));
}

/// JS: "overlapping different metabolites never merge"
#[test]
fn overlapping_different_metabolites_never_merge() {
    let com = build_community();
    let mut map = empty_map(&com);
    assert!(map.add_reaction(&com, &first_internal_reaction(&com).unwrap(), &PlaceOptions::default()).is_some());

    let nodes: Vec<Node> = map.nodes().values().filter(|node| is_metabolite_or_medium(node)).cloned().collect();
    let a = nodes.first().cloned();
    let b = nodes
        .iter()
        .find(|node| {
            a.as_ref().is_some_and(|a| {
                node.merged_id != a.merged_id && node.cell_id == a.cell_id && node.compartment == a.compartment
            })
        })
        .cloned();
    assert!(a.is_some() && b.is_some());
    let (a, b) = (a.unwrap(), b.unwrap());

    assert!(map.move_node(&b.id, a.x, a.y).is_some());
    assert!(map.merge_overlapping_metabolite(&b.id).is_none());
    assert!(map.has_node(&a.id));
    assert!(map.has_node(&b.id));
}
