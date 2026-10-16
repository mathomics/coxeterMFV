//! Port of `test/metabolite-visual-edit.test.mjs`: the two sizes of a
//! metabolite circle, and hiding one without taking it out of the map.
//!
//! Not ported: "a hidden metabolite and its attached branches are not
//! rendered" — every assertion after the set-up is on the SVG string
//! `renderMap` returns (crate `coxeter-render`).

mod common;

use std::sync::Arc;

use common::load_model;
use coxeter_core::community::{CommunityModel, CommunitySpec, MediumLimit, SpeciesSpec};
use coxeter_core::js;
use coxeter_core::mapdata::{Node, NodeKind};
use coxeter_map::EditableMap;
use coxeter_map::layout::LayoutOptions;
use serde_json::Value;

/// `map()`: `EditableMap.fromCommunity(com)` for E. coli core.
fn map() -> EditableMap {
    let limit = |lower: f64| MediumLimit { lower: Some(lower), upper: None };
    let com = CommunityModel::new(CommunitySpec {
        species: vec![SpeciesSpec::new("eco", Arc::new(load_model("ecoli_core_mc"))).abundance(1.0)],
        medium: [("glc__D", limit(-10.0)), ("o2", limit(-20.0))].into_iter().map(|(base, l)| (base.to_string(), l)).collect(),
        ..Default::default()
    })
    .unwrap();
    EditableMap::from_community(&com, &LayoutOptions::default())
}

/// `primaryNode(m)`: the id of the first node of kind `metabolite`.
fn primary_node(m: &EditableMap) -> Option<String> {
    m.nodes().values().find(|n| n.kind == NodeKind::Metabolite).map(|n| n.id.clone())
}

/// JS: "metabolite circles have only primary and secondary sizes"
#[test]
fn metabolite_circles_have_only_primary_and_secondary_sizes() {
    let mut m = map();
    let id = primary_node(&m);
    assert!(id.is_some());
    let id = id.unwrap();

    assert_eq!(m.set_node_radius(&id, 7.0).unwrap().r, 7.0);
    assert_eq!(m.node(&id).unwrap().visual_role.as_deref(), Some("secondary"));
    assert_eq!(m.set_node_radius(&id, 31.0).unwrap().r, 13.0);
    assert_eq!(m.node(&id).unwrap().visual_role.as_deref(), Some("primary"));
    assert_eq!(m.set_node_radius(&id, -100.0).unwrap().r, 7.0);
    assert_eq!(m.set_node_radius(&id, 1000.0).unwrap().r, 13.0);
}

/// JS: "hiding a metabolite keeps it in the map data and reaction membership"
#[test]
fn hiding_a_metabolite_keeps_it_in_the_map_data_and_reaction_membership() {
    let mut m = map();
    let id = primary_node(&m).unwrap();
    let reactions: Vec<String> = m.segments_at(&id).iter().map(|s| s.reaction_id.clone()).collect();
    assert!(!reactions.is_empty());

    let before_stats = m.stats();
    // `assert.equal(m.setNodeHidden(node.id, true), node)`: the node itself
    // comes back, not a copy of it.
    let returned = m.set_node_hidden(&id, true).map(|n| n as *const Node);
    assert_eq!(returned, m.node(&id).map(|n| n as *const Node));
    assert!(returned.is_some());
    assert_eq!(m.node(&id).unwrap().hidden, Some(true));
    assert!(m.has_node(&id));
    assert_eq!(m.stats(), before_stats);

    for rid in &reactions {
        assert!(m.has_reaction(rid));
    }
}

/// JS: "showAllHiddenNodes restores hidden metabolites"
#[test]
fn show_all_hidden_nodes_restores_hidden_metabolites() {
    let mut m = map();
    let nodes: Vec<String> = m
        .nodes()
        .values()
        .filter(|n| matches!(n.kind, NodeKind::Metabolite | NodeKind::Medium))
        .take(3)
        .map(|n| n.id.clone())
        .collect();
    for id in &nodes {
        m.set_node_hidden(id, true);
    }
    assert_eq!(m.hidden_node_count(), nodes.len());

    let restored = m.show_all_hidden_nodes();
    assert_eq!(restored.len(), nodes.len());
    assert_eq!(m.hidden_node_count(), 0);
    for id in &nodes {
        assert_eq!(m.node(id).unwrap().hidden, Some(false));
    }
}

/// JS: "visual role size and hidden state survive EditableMap JSON save/load"
#[test]
fn visual_role_size_and_hidden_state_survive_editable_map_json_save_load() {
    let mut m = map();
    let id = primary_node(&m).unwrap();
    m.set_node_primary(&id, false);
    m.set_node_hidden(&id, true);

    // `JSON.parse(JSON.stringify(m.toJSON()))`.
    let doc: Value = serde_json::from_str(&js::stringify(&m.to_json(), None)).unwrap();
    let back = EditableMap::from_json(&doc).unwrap();
    let restored = back.node(&id).unwrap();
    assert_eq!(restored.r, 7.0);
    assert_eq!(restored.visual_role.as_deref(), Some("secondary"));
    assert_eq!(restored.hidden, Some(true));
}
