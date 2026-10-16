//! Port of `test/multi-selection.test.mjs`: rectangle selection, moving,
//! aligning and distributing what is selected, annotations included.
//!
//! Every JS test is ported. One adaptation: the five alignment/distribution
//! tests run in JavaScript on `simpleArrangeMap()`, a duck-typed stand-in with
//! three nodes and one reaction whose `moveNode`/`moveReaction` just write
//! `x`/`y`. The Rust functions take an `EditableMap`, so the same three nodes
//! and one reaction are put on a blank map with no cells: nothing there has a
//! compartment to be held in, a segment or an axis, so `move_node` and
//! `move_reaction` place them exactly where they are asked to, as the
//! stand-in does.

mod common;

use std::sync::Arc;

use common::load_model;
use coxeter_core::community::{CommunityModel, CommunitySpec, SpeciesSpec};
use coxeter_core::mapdata::{Node, NodeKind, Point};
use coxeter_map::editable_map::TextOptions;
use coxeter_map::layout::LayoutOptions;
use coxeter_map::multi_selection::{
    Selection, SelectionRect, align_selection, distribute_selection, empty_selection, selectable_in_rect, selection_size,
    text_center, translate_selection,
};
use coxeter_map::reaction_edit::Handle;
use coxeter_map::util::{blank_node, blank_reaction};
use coxeter_map::{EditableMap, PlaceOptions};

const TALA: &str = "eco@TALA";

/// `setup()`: an empty E. coli shell with `eco@TALA` added to it.
fn setup() -> (CommunityModel, EditableMap) {
    let community = CommunityModel::new(CommunitySpec {
        species: vec![SpeciesSpec::new("eco", Arc::new(load_model("ecoli_core_mc"))).abundance(1.0)],
        ..Default::default()
    })
    .unwrap();
    let mut map = EditableMap::empty_from_community(&community, &LayoutOptions::default());
    map.add_reaction(&community, TALA, &PlaceOptions::default());
    (community, map)
}

/// `{ nodes: new Set([...]), reactions: new Set([...]) }`.
fn selection(nodes: &[&str], reactions: &[&str]) -> Selection {
    let mut s = empty_selection();
    s.nodes.extend(nodes.iter().map(|id| id.to_string()));
    s.reactions.extend(reactions.iter().map(|id| id.to_string()));
    s
}

/// `[...map.nodes.values()].find((n) => n.kind === 'metabolite')`.
fn first_metabolite(map: &EditableMap) -> Option<Node> {
    map.nodes().values().find(|n| n.kind == NodeKind::Metabolite).cloned()
}

/// The first metabolite node a segment of `rid` ends at.
fn first_metabolite_of(map: &EditableMap, rid: &str) -> Option<Node> {
    map.nodes()
        .values()
        .find(|n| n.kind == NodeKind::Metabolite && map.segments_at(&n.id).iter().any(|s| s.reaction_id == rid))
        .cloned()
}

fn font_size(size: f64) -> TextOptions {
    TextOptions { font_size: Some(size), ..Default::default() }
}

/// JS: "empty selection starts with no objects"
#[test]
fn empty_selection_starts_with_no_objects() {
    assert_eq!(selection_size(&empty_selection()), 0);
}

/// JS: "rectangle selection includes metabolite centres and reaction centres"
#[test]
fn rectangle_selection_includes_metabolite_centres_and_reaction_centres() {
    let (_, map) = setup();
    let rxn = map.reaction(TALA).cloned();
    let node = first_metabolite(&map);
    assert!(rxn.is_some() && node.is_some());
    let (rxn, node) = (rxn.unwrap(), node.unwrap());

    let rect = SelectionRect::from_corners(
        rxn.x.min(node.x) - 5.0,
        rxn.y.min(node.y) - 5.0,
        rxn.x.max(node.x) + 5.0,
        rxn.y.max(node.y) + 5.0,
    );
    let selected = selectable_in_rect(&map, rect, true);
    assert!(selected.reactions.contains(TALA));
    assert!(selected.nodes.contains(&node.id));
}

/// JS: "hidden metabolites are excluded from rectangle selection"
#[test]
fn hidden_metabolites_are_excluded_from_rectangle_selection() {
    let (_, mut map) = setup();
    let node = first_metabolite(&map).unwrap();
    map.set_node_hidden(&node.id, true);
    let selected =
        selectable_in_rect(&map, SelectionRect::from_corners(node.x - 2.0, node.y - 2.0, node.x + 2.0, node.y + 2.0), true);
    assert!(!selected.nodes.contains(&node.id));
}

/// JS: "group translation moves a selected metabolite and reaction together"
#[test]
fn group_translation_moves_a_selected_metabolite_and_reaction_together() {
    let (_, mut map) = setup();
    let rxn = map.reaction(TALA).cloned();
    let node = first_metabolite_of(&map, TALA);
    assert!(rxn.is_some() && node.is_some());
    let (before_rxn, before_node) = (rxn.unwrap(), node.unwrap());
    let id = before_node.id.clone();

    let moved = translate_selection(&mut map, &selection(&[&id], &[TALA]), 12.0, 8.0).unwrap();
    assert!(moved.nodes.contains(&id));
    assert!(moved.reactions.iter().any(|r| r == TALA));
    let node = map.node(&id).unwrap();
    let rxn = map.reaction(TALA).unwrap();
    assert!(((node.x - before_node.x) - 12.0).abs() < 1e-6);
    assert!(((node.y - before_node.y) - 8.0).abs() < 1e-6);
    assert!(((rxn.x - before_rxn.x) - 12.0).abs() < 1e-6);
    assert!(((rxn.y - before_rxn.y) - 8.0).abs() < 1e-6);
}

/// JS: "Delete-style group removal removes selected map instances but not the biological model"
#[test]
fn delete_style_group_removal_removes_selected_map_instances_but_not_the_biological_model() {
    let (community, mut map) = setup();
    let node = first_metabolite_of(&map, TALA);
    assert!(node.is_some());
    let node = node.unwrap();

    let model_reaction_count = community.merged.reactions.len();
    let model_metabolite_count = community.merged.metabolites.len();
    let selection = selection(&[&node.id], &[TALA]);

    // Mirror Store.removeMultiSelection ordering: reactions first, then selected
    // node instances that still exist and are removable.
    for reaction_id in &selection.reactions {
        map.remove_reaction(reaction_id);
    }
    for node_id in &selection.nodes {
        if map.nodes().contains_key(node_id) && map.can_remove_node(node_id) {
            map.remove_node(node_id);
        }
    }

    assert!(!map.reactions().contains_key(TALA));
    assert!(community.merged.reactions.contains_key(TALA));
    assert_eq!(community.merged.reactions.len(), model_reaction_count);
    assert_eq!(community.merged.metabolites.len(), model_metabolite_count);
}

/// JS: "when both ends of a TALA branch move, both cubic handles translate"
#[test]
fn when_both_ends_of_a_tala_branch_move_both_cubic_handles_translate() {
    let (_, mut map) = setup();
    let controls = map.reaction_control_points(TALA);
    let b1 = controls.iter().find(|p| p.handle == Handle::B1).unwrap().clone();
    let b2 = controls.iter().find(|p| p.node_id == b1.node_id && p.handle == Handle::B2).unwrap().clone();

    translate_selection(&mut map, &selection(&[&b1.node_id], &[TALA]), 10.0, 6.0);

    let after = map.reaction_control_points(TALA);
    let a1 = after.iter().find(|p| p.node_id == b1.node_id && p.handle == Handle::B1).unwrap();
    let a2 = after.iter().find(|p| p.node_id == b1.node_id && p.handle == Handle::B2).unwrap();

    assert!((a1.x - (b1.x + 10.0)).abs() < 1e-6);
    assert!((a1.y - (b1.y + 6.0)).abs() < 1e-6);
    assert!((a2.x - (b2.x + 10.0)).abs() < 1e-6);
    assert!((a2.y - (b2.y + 6.0)).abs() < 1e-6);
}

/// `simpleArrangeMap()`: three free metabolite nodes and one reaction marker
/// (see the note at the top of the file).
fn simple_arrange_map() -> EditableMap {
    let mut map = EditableMap::blank(None, None);
    map.edit_graph(|g| {
        for (id, x, y, r) in [("n1", 10.0, 20.0, 5.0), ("n2", 40.0, 50.0, 10.0), ("n3", 100.0, 80.0, 5.0)] {
            let mut node = blank_node(id, NodeKind::Metabolite);
            node.x = x;
            node.y = y;
            node.r = r;
            g.nodes.insert(id.to_string(), node);
        }
        let mut reaction = blank_reaction("r1");
        reaction.x = 70.0;
        reaction.y = 35.0;
        reaction.on_membrane = false;
        g.reactions.insert("r1".to_string(), reaction);
    });
    map
}

/// JS: "horizontal alignment puts selected centres on the same y coordinate"
#[test]
fn horizontal_alignment_puts_selected_centres_on_the_same_y_coordinate() {
    let mut map = simple_arrange_map();
    let selection = selection(&["n1", "n2", "n3"], &[]);

    let result = align_selection(&mut map, &selection, "horizontal");
    assert_eq!(result.moved.len(), 3);
    assert_eq!(map.node("n1").unwrap().y, 50.0);
    assert_eq!(map.node("n2").unwrap().y, 50.0);
    assert_eq!(map.node("n3").unwrap().y, 50.0);
}

/// JS: "vertical alignment works across metabolite and reaction objects"
#[test]
fn vertical_alignment_works_across_metabolite_and_reaction_objects() {
    let mut map = simple_arrange_map();
    let selection = selection(&["n1", "n2"], &["r1"]);

    align_selection(&mut map, &selection, "vertical");
    let expected = (10.0 + 40.0 + 70.0) / 3.0;
    assert_eq!(map.node("n1").unwrap().x, expected);
    assert_eq!(map.node("n2").unwrap().x, expected);
    assert_eq!(map.reaction("r1").unwrap().x, expected);
}

/// JS: "left alignment uses visual edges rather than only object centres"
#[test]
fn left_alignment_uses_visual_edges_rather_than_only_object_centres() {
    let mut map = simple_arrange_map();
    let selection = selection(&["n1", "n2"], &["r1"]);

    align_selection(&mut map, &selection, "left");
    let left_edges = [
        map.node("n1").unwrap().x - map.node("n1").unwrap().r,
        map.node("n2").unwrap().x - map.node("n2").unwrap().r,
        map.reaction("r1").unwrap().x - 9.0,
    ];
    assert!(left_edges.iter().all(|x| (x - left_edges[0]).abs() < 1e-9));
}

/// JS: "horizontal distribution keeps endpoints and spaces intermediate centres equally"
#[test]
fn horizontal_distribution_keeps_endpoints_and_spaces_intermediate_centres_equally() {
    let mut map = simple_arrange_map();
    let selection = selection(&["n1", "n2", "n3"], &["r1"]);

    distribute_selection(&mut map, &selection, "horizontal");
    let mut xs = [
        map.node("n1").unwrap().x,
        map.node("n2").unwrap().x,
        map.reaction("r1").unwrap().x,
        map.node("n3").unwrap().x,
    ];
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let gaps = [xs[1] - xs[0], xs[2] - xs[1], xs[3] - xs[2]];
    assert!(gaps.iter().all(|gap| (gap - gaps[0]).abs() < 1e-9));
    assert_eq!(xs[0], 10.0);
    assert_eq!(xs[3], 100.0);
}

/// JS: "distribution requires at least three selected objects"
#[test]
fn distribution_requires_at_least_three_selected_objects() {
    let mut map = simple_arrange_map();
    let before = map.node("n1").unwrap().clone();
    let result = distribute_selection(&mut map, &selection(&["n1", "n2"], &[]), "vertical");
    assert_eq!(result.moved.len(), 0);
    assert_eq!(map.node("n1").unwrap(), &before);
}

// Free text annotations

/// JS: "a rectangle catches the annotations under it, by their middle"
#[test]
fn a_rectangle_catches_the_annotations_under_it_by_their_middle() {
    let (_, mut map) = setup();
    let inside = map.add_text("G6P", 400.0, 400.0, &font_size(16.0)).cloned();
    let outside = map.add_text("PEP", 4000.0, 4000.0, &font_size(16.0)).cloned();
    assert!(inside.is_some() && outside.is_some());
    let (inside, outside) = (inside.unwrap(), outside.unwrap());

    let centre = text_center(&inside);
    let found = selectable_in_rect(
        &map,
        SelectionRect::from_corners(centre.x - 4.0, centre.y - 4.0, centre.x + 4.0, centre.y + 4.0),
        true,
    );

    assert!(found.texts.contains(&inside.id), "the annotation under the rectangle is caught");
    assert!(!found.texts.contains(&outside.id), "one far away is not");
}

/// JS: "deleting a selection takes its metabolite names with it"
///
/// The case this covers: a whole pathway is selected and deleted, and its
/// metabolite names must not stay behind, labelling reactions that are no
/// longer there. In the core-metabolism overview those names are map
/// annotations, not node labels, so a rectangle selection has to take
/// annotations too.
#[test]
fn deleting_a_selection_takes_its_metabolite_names_with_it() {
    let (_, mut map) = setup();
    let rxn = Point::new(map.reaction(TALA).unwrap().x, map.reaction(TALA).unwrap().y);
    let label = map.add_text("G6P", rxn.x, rxn.y, &font_size(16.0)).cloned().unwrap();

    let centre = text_center(&label);
    let rect = SelectionRect::from_corners(
        rxn.x.min(centre.x) - 40.0,
        rxn.y.min(centre.y) - 40.0,
        rxn.x.max(centre.x) + 40.0,
        rxn.y.max(centre.y) + 40.0,
    );
    let selection = selectable_in_rect(&map, rect, true);

    assert!(selection.reactions.contains(TALA));
    assert!(selection.texts.contains(&label.id));
    assert_eq!(selection_size(&selection), selection.nodes.len() + selection.reactions.len() + 1);
}

/// JS: "moving a selection carries its annotations along"
#[test]
fn moving_a_selection_carries_its_annotations_along() {
    let (_, mut map) = setup();
    let label = map.add_text("PEP", 300.0, 300.0, &font_size(20.0)).cloned().unwrap();
    let mut selection = empty_selection();
    selection.texts.insert(label.id.clone());

    let moved = translate_selection(&mut map, &selection, 25.0, -40.0).unwrap();

    assert_eq!(moved.texts, vec![label.id.clone()]);
    assert_eq!(map.text(&label.id).unwrap().x, 325.0);
    assert_eq!(map.text(&label.id).unwrap().y, 260.0);
}

/// JS: "aligning mixes annotations and nodes by their middles, not their anchors"
///
/// A node sits at its centre and a text hangs off its baseline corner, so
/// aligning the two has to compare like with like or the text lands low.
#[test]
fn aligning_mixes_annotations_and_nodes_by_their_middles_not_their_anchors() {
    let (_, mut map) = setup();
    let node_id = first_metabolite(&map).unwrap().id;
    let label = map.add_text("ALIGN", 900.0, 900.0, &font_size(20.0)).cloned().unwrap();

    let mut selection = empty_selection();
    selection.nodes.insert(node_id.clone());
    selection.texts.insert(label.id.clone());
    align_selection(&mut map, &selection, "horizontal");

    let after = map.text(&label.id).unwrap();
    let node = map.node(&node_id).unwrap();
    let centre_y = after.y - 20.0 / 2.0;
    assert!((centre_y - node.y).abs() < 1e-6, "text middle {centre_y} should meet the node at {}", node.y);
}
