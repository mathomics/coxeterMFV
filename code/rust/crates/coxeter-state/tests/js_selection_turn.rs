//! Port of the Store tests of `test/selection-turn.test.mjs`: even spacing,
//! turns and compass points of the multi-selection as single undo steps, and
//! the status line's warning when the map refuses a turn.
//!
//! The tests on the map alone are in `coxeter-map/tests/js_selection_turn.rs`;
//! "the inspector turns to a selection of two or more and offers its tools
//! there" (HTML of `renderInspector`) is in `coxeter-app/tests/inspector.rs`.

mod common;

use std::f64::consts::{FRAC_1_SQRT_2, PI};

use coxeter_core::mapdata::Point;
use coxeter_map::EditableMap;
use coxeter_map::multi_selection::{ItemKind, Selection, TurnRefusal, empty_selection, rotate_selection, spacing_items};
use coxeter_state::operation_boundary::{OperationDomain, operation_domain};
use coxeter_state::{StatusState, Store};
use serde_json::json;

use common::{bundled_model, store};

const G6P: &str = "met:eco@g6p_c";
const F6P: &str = "met:eco@f6p_c";
const FDP: &str = "met:eco@fdp_c";

fn near(a: f64, b: f64, eps: f64) -> bool {
    (a - b).abs() <= eps
}

/// `ecoliStore()`: the E. coli core model alone, opened as a scenario.
fn ecoli_store() -> Store {
    let mut store = store();
    store.set_model("ecoli_core_mc", bundled_model("ecoli_core_mc"));
    store.scenarios = vec![json!({
        "id": "ecoli", "name": "E. coli", "objective": "equal-growth",
        "species": [{ "id": "eco", "model": "ecoli_core_mc", "abundance": 1 }],
        "medium": { "glc__D": 10, "o2": 20 },
    })];
    store.use_scenario("ecoli").unwrap();
    store
}

fn map(store: &Store) -> &EditableMap {
    store.map.as_ref().expect("a map")
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

/// Upper glycolysis (PGI, PFK) laid on one vertical line, and selected.
fn vertical_chain() -> Store {
    let mut store = ecoli_store();
    store.multi_selection = chain_selection(map(&store), &["eco@PGI", "eco@PFK"]);
    assert!(store.arrange_multi_selection("align-vertical").unwrap().changed);
    store.multi_selection = chain_selection(map(&store), &["eco@PGI", "eco@PFK"]);
    store
}

/// `drawingPoints(map, selection)`.
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

/// JS: "even spacing counts metabolites and markers, not cofactors or straight reactions on a selected end"
#[test]
fn even_spacing_counts_metabolites_and_markers_not_cofactors_or_straight_reactions_on_a_selected_end() {
    let mut store = vertical_chain();
    let items = spacing_items(map(&store), &store.multi_selection);
    assert!(items.iter().all(|(kind, _)| *kind == ItemKind::Node), "the two straight reactions follow their ends");
    let mut ids: Vec<&str> = items.iter().map(|(_, id)| id.as_str()).collect();
    ids.sort();
    assert_eq!(ids, vec![F6P, FDP, G6P]);

    // Pull the middle one off its place, then space them again: one undo step.
    let f6p = map(&store).node(F6P).unwrap().pos();
    store.map.as_mut().unwrap().move_node(F6P, f6p.x, f6p.y - 25.0);
    let depth = store.map_history.undo_len();
    let result = store.space_multi_selection_evenly().unwrap();
    assert!(result.changed);
    assert_eq!(store.map_history.undo_len(), depth + 1);
    let at = |id: &str| map(&store).node(id).unwrap().pos();
    let (g6p, mid, fdp) = (at(G6P), at(F6P), at(FDP));
    assert!(near(mid.y - g6p.y, fdp.y - mid.y, 1e-6), "equal steps");
    let pfk = map(&store).reaction("eco@PFK").unwrap();
    assert!(
        near(pfk.x, mid.x, 1e-6) && pfk.y > mid.y.min(fdp.y) && pfk.y < mid.y.max(fdp.y),
        "the straight reaction stayed on its two metabolites"
    );
    assert_eq!(operation_domain("spaceMultiSelectionEvenly"), Some(OperationDomain::Map));
}

/// JS: "a straight pathway turns rigidly too, kept straight on its metabolites"
#[test]
fn a_straight_pathway_turns_rigidly_too_kept_straight_on_its_metabolites() {
    let mut store = vertical_chain();
    let selection = store.multi_selection.clone();
    let before = drawing_points(map(&store), &selection);
    let depth = store.map_history.undo_len();

    let result = store.rotate_multi_selection(PI / 4.0).unwrap();
    assert!(result.turned() && result.changed);
    assert_eq!(store.map_history.undo_len(), depth + 1, "one undo step");
    assert_rigid(&before, &drawing_points(map(&store), &selection), "straight chain");
    for rid in ["eco@PGI", "eco@PFK"] {
        assert!(map(&store).reaction(rid).unwrap().linear.is_some(), "{rid} is still straight");
    }
    store.undo_map();
    let marker = before.iter().find(|(name, ..)| name == "marker eco@PGI").unwrap().1;
    assert!(near(map(&store).reaction("eco@PGI").unwrap().x, marker, 1e-6));
    assert_eq!(operation_domain("rotateMultiSelection"), Some(OperationDomain::Map));
}

/// JS: "the compass turns the selection to point the way its pathway runs"
#[test]
fn the_compass_turns_the_selection_to_point_the_way_its_pathway_runs() {
    let mut store = vertical_chain();
    let way = |store: &Store| {
        let g6p = map(store).node(G6P).unwrap().pos();
        let fdp = map(store).node(FDP).unwrap().pos();
        let length = (fdp.x - g6p.x).hypot(fdp.y - g6p.y);
        Point::new((fdp.x - g6p.x) / length, (fdp.y - g6p.y) / length)
    };
    let d = FRAC_1_SQRT_2;
    for (direction, x, y) in [("E", 1.0, 0.0), ("SE", d, d), ("N", 0.0, -1.0), ("SW", -d, d), ("W", -1.0, 0.0)] {
        let result = store.point_multi_selection(direction).unwrap();
        assert!(result.turned(), "{direction}");
        let w = way(&store);
        assert!(
            near(w.x, x, 1e-6) && near(w.y, y, 1e-6),
            "{direction}: glucose 6-phosphate to fructose bisphosphate runs {direction} ({}, {})",
            w.x,
            w.y
        );
    }
    // Already pointing that way: nothing to do, nothing recorded.
    let depth = store.map_history.undo_len();
    assert!(!store.point_multi_selection("W").unwrap().changed);
    assert_eq!(store.map_history.undo_len(), depth);
    assert_eq!(operation_domain("pointMultiSelection"), Some(OperationDomain::Map));
}

/// JS: "a turn is refused, leaving the map as it was, for a transporter, a compartment or a wall"
/// (the store's part: a short warning, and no undo step).
#[test]
fn a_refused_turn_says_why_and_records_nothing() {
    let mut store = ecoli_store();
    let transporter = map(&store).reactions().values().find(|r| r.on_membrane).unwrap().id.clone();
    let before = map(&store).to_json();
    let depth = store.map_history.undo_len();

    store.multi_selection = chain_selection(map(&store), &["eco@PGI", &transporter]);
    let result = store.rotate_multi_selection(PI / 4.0).unwrap();
    assert_eq!(result.refused, Some(TurnRefusal::Transporter));
    assert_eq!(store.status().state, StatusState::Warn);
    assert_eq!(store.status().message, "Transporters stay on their membranes; leave them out to turn the selection.");

    let mut with_compartment = chain_selection(map(&store), &["eco@PGI", "eco@PFK"]);
    with_compartment.compartments.insert((map(&store).cells()[0].id.clone(), "c".to_string()));
    store.multi_selection = with_compartment;
    assert_eq!(store.point_multi_selection("NE").unwrap().refused, Some(TurnRefusal::Compartment));
    assert_eq!(store.status().message, "Compartments cannot be turned; leave them out.");

    // The wall: the map refuses, so the store would say "No room to turn the selection here."
    let chain = chain_selection(map(&store), &["eco@PGI", "eco@PFK"]);
    let cell = map(&store).cells()[0].rect;
    let far = Point::new(cell.x - 400.0, cell.y - 400.0);
    let refused = rotate_selection(store.map.as_mut().unwrap(), &chain, PI, Some(far));
    assert_eq!(refused, Some(coxeter_map::multi_selection::SelectionTurn::Refused(TurnRefusal::Room)));
    assert_eq!(coxeter_state::map_edit::turn_warning(TurnRefusal::Room), "No room to turn the selection here.");

    assert_eq!(store.map_history.undo_len(), depth);
    assert_eq!(map(&store).to_json(), before);
}
