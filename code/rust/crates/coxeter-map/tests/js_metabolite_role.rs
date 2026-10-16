//! Port of `test/metabolite-role.test.mjs`: a metabolite node is drawn as
//! primary or secondary, and that role is map-only.
//!
//! Not ported: "renderer distinguishes primary and secondary nodes and labels"
//! — every assertion is on the SVG string `renderMap` returns (crate
//! `coxeter-render`).

mod common;

use std::sync::Arc;

use common::load_model;
use coxeter_core::community::{CommunityModel, CommunitySpec, MediumLimit, SpeciesSpec};
use coxeter_core::js;
use coxeter_core::mapdata::{Node, NodeKind};
use coxeter_map::EditableMap;
use coxeter_map::editable_map::DuplicateOptions;
use coxeter_map::layout::LayoutOptions;
use serde_json::Value;

/// `build()`: the community and `EditableMap.fromCommunity(community)`.
fn build() -> (CommunityModel, EditableMap) {
    let limit = |lower: f64| MediumLimit { lower: Some(lower), upper: None };
    let community = CommunityModel::new(CommunitySpec {
        species: vec![SpeciesSpec::new("eco", Arc::new(load_model("ecoli_core_mc"))).abundance(1.0)],
        medium: [("glc__D", limit(-10.0)), ("o2", limit(-20.0))].into_iter().map(|(base, l)| (base.to_string(), l)).collect(),
        ..Default::default()
    })
    .unwrap();
    let map = EditableMap::from_community(&community, &LayoutOptions::default());
    (community, map)
}

/// `metaboliteNode(map)`: the id of the first node of kind `metabolite`.
fn metabolite_node(map: &EditableMap) -> Option<String> {
    map.nodes().values().find(|node| node.kind == NodeKind::Metabolite).map(|node| node.id.clone())
}

fn node<'a>(map: &'a EditableMap, id: &str) -> &'a Node {
    map.node(id).unwrap()
}

/// `JSON.parse(JSON.stringify(map.toJSON()))`.
fn through_json(map: &EditableMap) -> Value {
    serde_json::from_str(&js::stringify(&map.to_json(), None)).unwrap()
}

/// JS: "existing metabolite nodes default to primary"
#[test]
fn existing_metabolite_nodes_default_to_primary() {
    let (_, map) = build();
    let id = metabolite_node(&map);
    assert!(id.is_some());
    let n = node(&map, &id.unwrap());
    assert_eq!(n.primary, Some(true));
    assert_eq!(n.visual_role.as_deref(), Some("primary"));
}

/// JS: "a metabolite can be changed from primary to secondary and back"
#[test]
fn a_metabolite_can_be_changed_from_primary_to_secondary_and_back() {
    let (_, mut map) = build();
    let id = metabolite_node(&map).unwrap();

    map.set_node_primary(&id, false);
    let n = node(&map, &id);
    assert_eq!(n.primary, Some(false));
    assert_eq!(n.visual_role.as_deref(), Some("secondary"));
    assert_eq!(n.r, 7.0);

    map.set_node_primary(&id, true);
    let n = node(&map, &id);
    assert_eq!(n.primary, Some(true));
    assert_eq!(n.visual_role.as_deref(), Some("primary"));
    assert_eq!(n.r, 13.0);
}

/// JS: "radius requests snap to the two supported visual roles"
#[test]
fn radius_requests_snap_to_the_two_supported_visual_roles() {
    let (_, mut map) = build();
    let id = metabolite_node(&map).unwrap();

    map.set_node_radius(&id, 23.0);
    let n = node(&map, &id);
    assert_eq!(n.primary, Some(true));
    assert_eq!(n.r, 13.0);

    map.set_node_radius(&id, 5.0);
    let n = node(&map, &id);
    assert_eq!(n.primary, Some(false));
    assert_eq!(n.r, 7.0);
}

/// JS: "duplicated metabolite instances can have independent visual roles"
#[test]
fn duplicated_metabolite_instances_can_have_independent_visual_roles() {
    let (_, mut map) = build();
    let original_id = metabolite_node(&map).unwrap();
    let copy_id = map.duplicate_metabolite_node(&original_id, &DuplicateOptions::default()).map(|n| n.id.clone());
    assert!(copy_id.is_some());
    let copy_id = copy_id.unwrap();

    map.set_node_primary(&copy_id, false);

    let original = node(&map, &original_id);
    let copy = node(&map, &copy_id);
    assert_eq!(original.primary, Some(true));
    assert_eq!(original.visual_role.as_deref(), Some("primary"));
    assert_eq!(copy.primary, Some(false));
    assert_eq!(copy.visual_role.as_deref(), Some("secondary"));
    assert_eq!(copy.merged_id, original.merged_id);
}

/// JS: "primary/secondary role survives map JSON round trip"
#[test]
fn primary_secondary_role_survives_map_json_round_trip() {
    let (_, mut map) = build();
    let id = metabolite_node(&map).unwrap();
    map.set_node_primary(&id, false);

    let restored = EditableMap::from_json(&through_json(&map)).unwrap();
    let back = node(&restored, &id);
    assert_eq!(back.primary, Some(false));
    assert_eq!(back.visual_role.as_deref(), Some("secondary"));
    assert_eq!(back.r, 7.0);
}

/// JS: "legacy map nodes without role metadata load as primary"
#[test]
fn legacy_map_nodes_without_role_metadata_load_as_primary() {
    let (_, map) = build();
    let mut doc = through_json(&map);
    let legacy = doc["nodes"].as_array_mut().unwrap().iter_mut().find(|item| item["kind"] == "metabolite").unwrap();
    let id = legacy["id"].as_str().unwrap().to_string();
    let fields = legacy.as_object_mut().unwrap();
    fields.remove("primary");
    fields.remove("visualRole");

    let restored = EditableMap::from_json(&doc).unwrap();
    let back = node(&restored, &id);
    assert_eq!(back.primary, Some(true));
    assert_eq!(back.visual_role.as_deref(), Some("primary"));
}

/// JS: "changing visual role does not change biological model membership"
#[test]
fn changing_visual_role_does_not_change_biological_model_membership() {
    let (community, mut map) = build();
    let id = metabolite_node(&map).unwrap();
    let biological_count = community.merged.metabolites.len();
    let reaction_count = community.merged.reactions.len();

    map.set_node_primary(&id, false);

    assert_eq!(community.merged.metabolites.len(), biological_count);
    assert_eq!(community.merged.reactions.len(), reaction_count);
    assert!(community.merged.metabolites.contains_key(node(&map, &id).merged_id.as_deref().unwrap()));
}
