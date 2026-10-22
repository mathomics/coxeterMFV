//! Port of the map tests of `test/selection-turn.test.mjs`: a selection
//! spaced evenly along its own line, turned as one rigid piece, pointed with
//! the compass, and the turns that are refused.
//!
//! The tests that go through the Store (one undo step, the status line's
//! warnings, straight pathways of the scenario map) are in
//! `coxeter-state/tests/js_selection_turn.rs`; the inspector's in
//! `coxeter-app/tests/inspector.rs`.
//!
//! Adapted: "a turn is refused … for a transporter, a compartment or a wall"
//! runs on `EditableMap::from_community` of the E. coli core model, the map
//! the JavaScript `ecoliStore()` draws.

mod common;

use std::f64::consts::{FRAC_1_SQRT_2, PI};
use std::sync::Arc;

use common::load_model;
use coxeter_core::community::{CommunityModel, CommunitySpec, SpeciesSpec};
use coxeter_core::mapdata::{NodeKind, Point};
use coxeter_core::metabolite_label::{DEFAULT_METABOLITE_LABEL_ANGLE, metabolite_label_angle};
use coxeter_core::reaction_label::reaction_label_offset;
use coxeter_map::layout::LayoutOptions;
use coxeter_map::multi_selection::{
    Selection, SelectionTurn, TurnRefusal, empty_selection, pointing_angle, rotate_selection, selection_main_axis,
    space_selection_evenly, spacing_items,
};
use coxeter_map::util::blank_node;
use coxeter_map::{EditableMap, PlaceOptions};

const CHAIN: [&str; 3] = ["eco@PGI", "eco@PFK", "eco@FBA"];

fn near(a: f64, b: f64, eps: f64) -> bool {
    (a - b).abs() <= eps
}

fn ecoli() -> CommunityModel {
    CommunityModel::new(CommunitySpec {
        species: vec![SpeciesSpec::new("eco", Arc::new(load_model("ecoli_core_mc"))).abundance(1.0)],
        ..Default::default()
    })
    .unwrap()
}

/// `chainSelection(map, reactions)`: some reactions and every metabolite
/// drawn to them, cofactors included.
fn chain_selection(map: &EditableMap, reactions: &[&str]) -> Selection {
    let mut selection = empty_selection();
    selection.reactions.extend(reactions.iter().map(|id| id.to_string()));
    for seg in map.segments() {
        if selection.reactions.contains(&seg.reaction_id) {
            if let Some(node) = seg.node_id.as_deref().filter(|id| !id.is_empty()) {
                selection.nodes.insert(node.to_string());
            }
        }
    }
    selection
}

/// `placedChain()`: a chain placed reaction by reaction on an empty cell:
/// curved reactions with their own cofactors.
fn placed_chain() -> (EditableMap, Selection) {
    let community = ecoli();
    let mut map = EditableMap::empty_from_community(&community, &LayoutOptions::default());
    for rid in CHAIN {
        assert!(map.add_reaction(&community, rid, &PlaceOptions::default()).is_some(), "{rid} is placed");
    }
    let selection = chain_selection(&map, &CHAIN);
    (map, selection)
}

/// `drawingPoints(map, selection)`: every point a selection's drawing is made
/// of: its metabolites (cofactors of its reactions included), markers, axis
/// ends, and the ends and handles of its reactions' curves.
fn drawing_points(map: &EditableMap, selection: &Selection) -> Vec<(String, f64, f64)> {
    let mut out = Vec::new();
    for node in map.nodes().values() {
        if selection.nodes.contains(&node.id) || node.reaction_id.as_ref().is_some_and(|r| selection.reactions.contains(r)) {
            out.push((format!("node {}", node.id), node.x, node.y));
        }
    }
    for rid in &selection.reactions {
        let rxn = map.reaction(rid).unwrap();
        let axis = rxn.axis.unwrap();
        out.push((format!("marker {rid}"), rxn.x, rxn.y));
        out.push((format!("a1 {rid}"), axis.a1.x, axis.a1.y));
        out.push((format!("a2 {rid}"), axis.a2.x, axis.a2.y));
    }
    for (i, seg) in map.segments().iter().enumerate() {
        if !selection.reactions.contains(&seg.reaction_id) {
            continue;
        }
        for (key, p) in [("from", Some(seg.from)), ("to", Some(seg.to)), ("b1", seg.b1), ("b2", seg.b2)] {
            if let Some(p) = p {
                out.push((format!("seg {i} {key}"), p.x, p.y));
            }
        }
    }
    out
}

/// `assertRigid(before, after, label)`.
fn assert_rigid(before: &[(String, f64, f64)], after: &[(String, f64, f64)], label: &str) {
    assert_eq!(before.len(), after.len());
    assert!(before.len() > 20, "{label}: enough points to mean something");
    let mut worst: f64 = 0.0;
    for i in 0..before.len() {
        for j in i + 1..before.len() {
            let d0 = (before[i].1 - before[j].1).hypot(before[i].2 - before[j].2);
            let d1 = (after[i].1 - after[j].1).hypot(after[i].2 - after[j].2);
            worst = worst.max((d0 - d1).abs());
        }
    }
    assert!(worst < 1e-6, "{label}: every distance kept (worst change {worst})");
}

// Even spacing

/// `looseNodes(points)`: metabolites on their own on an open map, so nothing
/// holds them back.
fn loose_nodes(points: &[(f64, f64)]) -> (EditableMap, Selection) {
    let mut map = EditableMap::blank(Some(2000.0), Some(2000.0));
    let mut selection = empty_selection();
    map.edit_graph(|g| {
        for (i, &(x, y)) in points.iter().enumerate() {
            let id = format!("n{i}");
            let mut node = blank_node(&id, NodeKind::Metabolite);
            node.primary = Some(true);
            node.x = x;
            node.y = y;
            node.r = 13.0;
            g.nodes.insert(id.clone(), node);
            selection.nodes.insert(id);
        }
    });
    (map, selection)
}

/// JS: "even spacing works along a diagonal, keeping the ends and how far each stands off the line"
#[test]
fn even_spacing_works_along_a_diagonal_keeping_the_ends_and_how_far_each_stands_off_the_line() {
    // A chain running down to the right, unevenly spaced, one of them a little off it.
    let (mut map, selection) = loose_nodes(&[(100.0, 100.0), (130.0, 130.0), (300.0, 300.0), (356.0, 344.0), (500.0, 500.0)]);
    let axis = selection_main_axis(&map, &selection).unwrap();
    assert!(near(axis.x.abs(), FRAC_1_SQRT_2, 0.01) && near(axis.x, axis.y, 0.01), "the line is the diagonal");
    // Off the selection's own line, which is all but the diagonal.
    let across = |p: Point| p.y * axis.x - p.x * axis.y;
    let off_before: Vec<f64> = ["n1", "n2", "n3"].iter().map(|id| across(map.node(id).unwrap().pos())).collect();

    let result = space_selection_evenly(&mut map, &selection);
    assert_eq!(result.moved.len(), 3);
    let (result_axis, step) = (result.axis.unwrap(), result.step.unwrap());
    let along = |map: &EditableMap, id: &str| {
        let p = map.node(id).unwrap();
        p.x * result_axis.x + p.y * result_axis.y
    };
    let steps: Vec<f64> = ["n0", "n1", "n2", "n3", "n4"].iter().map(|id| along(&map, id)).collect();
    for i in 1..steps.len() {
        assert!(near(steps[i] - steps[i - 1], step, 1e-6), "step {i}");
    }
    for (id, x, y) in [("n0", 100.0, 100.0), ("n4", 500.0, 500.0)] {
        let p = map.node(id).unwrap();
        assert_eq!((p.x, p.y), (x, y), "the two ends stay");
    }
    for (i, id) in ["n1", "n2", "n3"].iter().enumerate() {
        assert!(near(across(map.node(id).unwrap().pos()), off_before[i], 1e-6), "{id} stays as far off the line");
    }
}

/// JS: "even spacing needs three items, and leaves two alone"
#[test]
fn even_spacing_needs_three_items_and_leaves_two_alone() {
    let (mut map, selection) = loose_nodes(&[(0.0, 0.0), (10.0, 90.0)]);
    let before = map.to_json();
    let result = space_selection_evenly(&mut map, &selection);
    assert!(result.moved.is_empty());
    assert_eq!(result.count, 2);
    assert_eq!((result.axis, result.step), (None, None));
    assert_eq!(map.to_json(), before);
}

// Turning

/// JS: "turning a selection is rigid: every distance in its drawing is kept, cofactors and handles included"
#[test]
fn turning_a_selection_is_rigid_every_distance_in_its_drawing_is_kept_cofactors_and_handles_included() {
    let (mut map, selection) = placed_chain();
    // Labels and a moved name, to see them turn too.
    let node_id = selection.nodes[0].clone();
    let cofactor = map.nodes().values().find(|n| n.reaction_id.as_deref() == Some("eco@PFK")).map(|n| n.id.clone());
    let cofactor = cofactor.expect("PFK has cofactors of its own");
    assert!(map.set_reaction_label_offset("eco@PFK", Point::new(20.0, 0.0)).is_some());
    let before = drawing_points(&map, &selection);

    let result = rotate_selection(&mut map, &selection, PI / 4.0, None);
    let Some(SelectionTurn::Turned { pivot, .. }) = result else { panic!("turned: {result:?}") };
    assert_rigid(&before, &drawing_points(&map, &selection), "curved chain");
    let turned = DEFAULT_METABOLITE_LABEL_ANGLE + PI / 4.0;
    assert!(near(metabolite_label_angle(map.node(&node_id).unwrap().label_angle), turned, 1e-6), "a label turns round its metabolite");
    assert!(near(metabolite_label_angle(map.node(&cofactor).unwrap().label_angle), turned, 1e-6), "and a cofactor's with it");
    let offset = reaction_label_offset(map.reaction("eco@PFK").unwrap()).unwrap();
    assert!(near(offset.x, 20.0 * FRAC_1_SQRT_2, 1e-6) && near(offset.y, 20.0 * FRAC_1_SQRT_2, 1e-6), "the moved name turns with its reaction");

    // A turn and its opposite bring everything back.
    rotate_selection(&mut map, &selection, -PI / 4.0, Some(pivot));
    for (i, (name, x, y)) in drawing_points(&map, &selection).into_iter().enumerate() {
        assert!(near(x, before[i].1, 1e-6) && near(y, before[i].2, 1e-6), "{name} is back");
    }
}

/// JS: "a pathway picked by its reactions turns and spaces as one straight line, the metabolites at its ends included"
#[test]
fn a_pathway_picked_by_its_reactions_turns_and_spaces_as_one_straight_line_the_metabolites_at_its_ends_included() {
    // Lower glycolysis of E. coli on the curated anaerobic consortium map, a
    // straight vertical chain g3p - GAPD - 13dpg - PGK - 3pg - PGM - 2pg.
    let text = std::fs::read_to_string(coxeter_core::testkit::repo_root().join("models/maps/consorcio-anaerobio.json")).unwrap();
    let curated = coxeter_map::map_document::parse_map_document_str(&text).unwrap();
    let mut map = EditableMap::from_json(&curated.map).unwrap();
    let chain = ["eco@GAPD", "eco@PGK", "eco@PGM"];
    // As a rectangle round the reactions catches it: the inner metabolites, not g3p and 2pg at the ends.
    let picked = || {
        let mut selection = empty_selection();
        selection.reactions.extend(chain.iter().map(|id| id.to_string()));
        selection.nodes.extend(["met:eco@13dpg_c", "met:eco@3pg_c"].map(String::from));
        selection
    };
    let line = ["met:eco@g3p_c", "met:eco@13dpg_c", "met:eco@3pg_c", "met:eco@2pg_c"];
    let at = |map: &EditableMap, id: &str| map.node(id).unwrap().pos();
    let off_line = |map: &EditableMap| {
        let (a, b) = (at(map, line[0]), at(map, line[3]));
        let length = (b.x - a.x).hypot(b.y - a.y);
        line.iter().map(|id| {
            let p = at(map, id);
            ((p.x - a.x) * (b.y - a.y) - (p.y - a.y) * (b.x - a.x)).abs() / length
        }).fold(0.0, f64::max)
    };
    assert!(chain.iter().all(|rid| map.reaction(rid).unwrap().linear.is_some()) && off_line(&map) < 1e-9, "a straight chain to begin with");
    let mut items: Vec<String> = spacing_items(&map, &picked()).into_iter().map(|(_, id)| id).collect();
    items.sort();
    let mut expected: Vec<String> = line.iter().map(|s| s.to_string()).collect();
    expected.sort();
    assert_eq!(items, expected, "the end metabolites are spaced too, and the straight reactions follow them");

    assert!(matches!(rotate_selection(&mut map, &picked(), PI / 4.0, None), Some(SelectionTurn::Turned { .. })));
    assert!(off_line(&map) < 1e-6, "still one straight line ({} off it)", off_line(&map));
    let (first, last) = (at(&map, line[0]), at(&map, line[3]));
    assert!(near(last.x - first.x, -(last.y - first.y), 1e-6) && last.y > first.y, "turned whole, now down to the left");
    for rid in chain {
        assert!(map.reaction(rid).unwrap().linear.is_some(), "{rid} is still straight");
    }

    assert!(matches!(map.point_selection(&picked(), "E"), Some(SelectionTurn::Turned { .. })));
    assert!(off_line(&map) < 1e-6 && near(at(&map, line[0]).y, at(&map, line[3]).y, 1e-6), "pointed east as a line");
    let p = at(&map, "met:eco@13dpg_c");
    map.move_node("met:eco@13dpg_c", p.x + 30.0, p.y);
    assert_eq!(space_selection_evenly(&mut map, &picked()).moved.len(), 2);
    let xs: Vec<f64> = line.iter().map(|id| at(&map, id).x).collect();
    assert!(near(xs[1] - xs[0], xs[2] - xs[1], 1e-6) && near(xs[2] - xs[1], xs[3] - xs[2], 1e-6), "equal steps from end to end");
}

/// JS: "with no reactions to say which way it runs, the compass makes the smaller turn"
#[test]
fn with_no_reactions_to_say_which_way_it_runs_the_compass_makes_the_smaller_turn() {
    let (map, selection) = loose_nodes(&[(100.0, 100.0), (200.0, 110.0), (300.0, 120.0)]);
    // Nearly level: E and W are each a small turn one way round or the other.
    assert!(pointing_angle(&map, &selection, "E").unwrap().abs() < 0.2);
    assert!(pointing_angle(&map, &selection, "W").unwrap().abs() < 0.2);
    assert!(near(pointing_angle(&map, &selection, "S").unwrap().abs(), PI / 2.0, 0.2));
    let (one, single) = loose_nodes(&[(0.0, 0.0)]);
    assert_eq!(pointing_angle(&one, &single, "E"), None, "one item gives no line");
    assert_eq!(pointing_angle(&map, &selection, "UP"), None, "not a compass direction");
}

/// JS: "a turn is refused, leaving the map as it was, for a transporter, a compartment or a wall"
/// (the map's part; the store's warnings are in coxeter-state).
#[test]
fn a_turn_is_refused_leaving_the_map_as_it_was_for_a_transporter_a_compartment_or_a_wall() {
    let mut map = EditableMap::from_community(&ecoli(), &LayoutOptions::default());
    let transporter = map.reactions().values().find(|r| r.on_membrane).unwrap().id.clone();
    let before = map.to_json();

    let with_transporter = chain_selection(&map, &["eco@PGI", &transporter]);
    assert_eq!(rotate_selection(&mut map, &with_transporter, 1.0, None), Some(SelectionTurn::Refused(TurnRefusal::Transporter)));

    let mut with_compartment = chain_selection(&map, &["eco@PGI", "eco@PFK"]);
    with_compartment.compartments.insert((map.cells()[0].id.clone(), "c".to_string()));
    assert_eq!(rotate_selection(&mut map, &with_compartment, 1.0, None), Some(SelectionTurn::Refused(TurnRefusal::Compartment)));

    // Swung half round a point far outside the cell, the pathway would leave its cytosol.
    let chain = chain_selection(&map, &["eco@PGI", "eco@PFK"]);
    let cell = map.cells()[0].rect;
    let far = Point::new(cell.x - 400.0, cell.y - 400.0);
    assert_eq!(rotate_selection(&mut map, &chain, PI, Some(far)), Some(SelectionTurn::Refused(TurnRefusal::Room)));
    assert_eq!(map.to_json(), before, "nothing moved");
}
