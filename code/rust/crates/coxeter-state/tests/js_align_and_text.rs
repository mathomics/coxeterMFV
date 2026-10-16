//! Port of the Store tests of `test/align-and-text.test.mjs` against the real
//! store: Escher-style Align vertical / horizontal for reactions, and the
//! size and font of free map text. (`coxeter-map/tests/js_align_and_text.rs`
//! re-expressed these over the map alone.)
//!
//! Ported in coxeter-map (no Store): "map text keeps a size and a font through
//! saving and drawing" (`EditableMap` and the renderer) and "text boxes follow
//! the font they are drawn in" (`textBox` of `multi-selection.js`).
//! Not ported: "the reaction inspector offers Vertical and Horizontal" and "the
//! text inspector offers a size and a font" — every assertion is on the HTML
//! `renderInspector` returns (UI panels, another crate).
//!
//! Assertions left out of tests that are ported:
//! - "one selected reaction is enough to align, and the panel offers it": the
//!   two `assert.match` on the HTML of `renderWorkspace` (and the
//!   `store.view.multiSelect = true` it needs).
//!
//! Adapted:
//! - "a transporter-only selection is aligned as plain objects, as before"
//!   asserts `'count' in result && !('held' in result)`: the shape of the
//!   result of `map.alignSelection` rather than `map.alignSelectionInLine`.
//!   The Rust `ArrangeResult` has one shape for both, so the test checks that
//!   the store's map is exactly what `align_selection` (the plain alignment of
//!   centres) makes of the same map and selection, and that nothing is held.

mod common;

use coxeter_core::js;
use coxeter_core::mapdata::{Node, NodeKind, Point};
use coxeter_core::text_style::TEXT_SIZE;
use coxeter_map::EditableMap;
use coxeter_map::multi_selection::{empty_selection, text_center};
use coxeter_state::{StatusState, Store, TextStyle};
use serde_json::json;

use common::{bundled_model, store};

const PGI: &str = "eco@PGI";
const PFK: &str = "eco@PFK";
const TPI: &str = "eco@TPI";
const G6P: &str = "met:eco@g6p_c";
const F6P: &str = "met:eco@f6p_c";
const FDP: &str = "met:eco@fdp_c";

/// `near(a, b, eps = 1e-6)`.
fn near(a: f64, b: f64) -> bool {
    near_eps(a, b, 1e-6)
}

fn near_eps(a: f64, b: f64, eps: f64) -> bool {
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

/// `select(store, { reactions, nodes })`.
fn select(store: &mut Store, reactions: &[&str], nodes: &[&str]) {
    store.multi_selection = empty_selection();
    for id in reactions {
        store.multi_selection.reactions.insert(id.to_string());
    }
    for id in nodes {
        store.multi_selection.nodes.insert(id.to_string());
    }
}

fn map(store: &Store) -> &EditableMap {
    store.map.as_ref().expect("a map")
}

fn map_mut(store: &mut Store) -> &mut EditableMap {
    store.map.as_mut().expect("a map")
}

/// `map().nodes.get(id)`.
fn node<'a>(store: &'a Store, id: &str) -> &'a Node {
    map(store).node(id).unwrap_or_else(|| panic!("no node {id}"))
}

/// `map().reactions.get(id)`, its marker.
fn marker(store: &Store, id: &str) -> Point {
    let r = map(store).reaction(id).unwrap_or_else(|| panic!("no reaction {id}"));
    Point::new(r.x, r.y)
}

/// `map().reactionLineEnds(id)`.
fn line_ends(store: &Store, id: &str) -> (String, String) {
    let (from, to) = map(store).reaction_line_ends(id);
    (from.expect("a line start"), to.expect("a line end"))
}

/// `[...map().reactions.values()].find((r) => r.onMembrane)`.
fn transporter(store: &Store) -> Option<String> {
    map(store).reactions().values().find(|r| r.on_membrane).map(|r| r.id.clone())
}

/// `[...ids].sort((a, b) => y(a) - y(b))`.
fn sorted_by_y(store: &Store, ids: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = ids.iter().map(|s| s.to_string()).collect();
    out.sort_by(|a, b| node(store, a).y.partial_cmp(&node(store, b).y).unwrap());
    out
}

// Escher-style line alignment

/// "Align vertical puts selected reactions and their metabolites on one straight line"
#[test]
fn align_vertical_puts_selected_reactions_and_their_metabolites_on_one_straight_line() {
    let mut store = ecoli_store();
    let reactions = [PGI, PFK];
    let line = [G6P, F6P, FDP];
    let order_before = sorted_by_y(&store, &line);
    let cofactor = map(&store)
        .nodes()
        .values()
        .find(|n| n.kind == NodeKind::Currency && n.reaction_id.as_deref() == Some(PFK))
        .map(|n| n.id.clone());
    select(&mut store, &reactions, &[]);

    let result = store.arrange_multi_selection("align-vertical");
    assert!(result.is_some_and(|r| r.changed), "the map changed and was recorded");
    let x = node(&store, line[0]).x;
    for id in line {
        assert!(near(node(&store, id).x, x), "{id} on the line");
    }
    for id in reactions {
        assert!(near(marker(&store, id).x, x), "{id} marker on the line");
    }
    let order_after = sorted_by_y(&store, &line);
    assert!(order_before.len() == 3);
    let reversed: Vec<&str> = line.iter().rev().copied().collect();
    assert!(
        order_after == line || order_after == reversed,
        "the pathway reads in its own order along the line, not folded back ({})",
        order_after.join(" → ")
    );
    for i in 1..order_after.len() {
        assert!(
            node(&store, &order_after[i]).y - node(&store, &order_after[i - 1]).y >= 80.0 - 1e-6,
            "neighbours are spread to leave room for a marker between them"
        );
    }
    for id in reactions {
        let (from, to) = line_ends(&store, id);
        let mid = (node(&store, &from).y + node(&store, &to).y) / 2.0;
        assert!(near_eps(marker(&store, id).y, mid, 1e-3), "{id} marker half way between its metabolites");
    }
    for id in reactions {
        assert!(map(&store).reaction(id).unwrap().linear.is_some(), "{id} is kept straight");
        for seg in map(&store)
            .segments()
            .iter()
            .filter(|s| s.reaction_id == id && s.node_id.as_deref().is_some_and(|n| line.contains(&n)))
        {
            for point in [seg.b1, seg.b2].into_iter().flatten() {
                assert!(near_eps(point.x, x, 1e-3), "{id} branch to {:?} is straight", seg.node_id);
            }
        }
    }
    if let Some(cofactor) = &cofactor {
        let placed = node(&store, cofactor);
        let pfk = marker(&store, PFK);
        assert!(!near_eps(placed.x, x, 0.5), "cofactors are placed beside the line, not stacked on it");
        assert!(js::hypot(placed.x - pfk.x, placed.y - pfk.y) < 120.0, "and stay by their reaction");
    }

    store.undo_map();
    assert!(!near(node(&store, line[0]).x, x) || !near(node(&store, line[2]).x, x), "one Undo puts it back");
}

/// "Align horizontal does the same along a horizontal line"
#[test]
fn align_horizontal_does_the_same_along_a_horizontal_line() {
    let mut store = ecoli_store();
    select(&mut store, &[PGI], &[]);
    store.arrange_multi_selection("align-horizontal");
    let y = marker(&store, PGI).y;
    assert!(near(node(&store, G6P).y, y));
    assert!(near(node(&store, F6P).y, y));
}

/// "one selected reaction is enough to align, and the panel offers it"
#[test]
fn one_selected_reaction_is_enough_to_align_and_the_panel_offers_it() {
    let mut store = ecoli_store();
    select(&mut store, &[PGI], &[]);
    assert!(store.can_arrange_selection("align-vertical"));
    assert!(!store.can_arrange_selection("align-left"), "edge alignment still needs two objects");
}

/// "a selection without reactions keeps the plain alignment of centres"
#[test]
fn a_selection_without_reactions_keeps_the_plain_alignment_of_centres() {
    let mut store = ecoli_store();
    select(&mut store, &[], &[G6P, FDP]);
    let pgi_before = map(&store).reaction(PGI).unwrap().clone();
    store.arrange_multi_selection("align-vertical");
    assert!(near(node(&store, G6P).x, node(&store, FDP).x));
    assert_eq!(map(&store).reaction(PGI).unwrap().linear, pgi_before.linear, "no reaction was made straight");
}

/// "transporters stay on their membranes when a selection is aligned"
#[test]
fn transporters_stay_on_their_membranes_when_a_selection_is_aligned() {
    let mut store = ecoli_store();
    let transporter = transporter(&store);
    assert!(transporter.is_some(), "the fixture has a transporter");
    let transporter = transporter.unwrap();
    let before = marker(&store, &transporter);
    select(&mut store, &[&transporter, PGI], &[]);
    store.arrange_multi_selection("align-vertical");
    let after = marker(&store, &transporter);
    assert!(near(after.x, before.x) && near(after.y, before.y));
}

/// "a reaction aligned on its own keeps the way it runs"
#[test]
fn a_reaction_aligned_on_its_own_keeps_the_way_it_runs() {
    let mut store = ecoli_store();
    let (from, to) = line_ends(&store, PGI);
    let was_below = node(&store, &to).y >= node(&store, &from).y;
    store.align_map_reaction_axis(PGI, "vertical");
    let marker_at = marker(&store, PGI);
    assert!(
        near(node(&store, &from).x, marker_at.x) && near(node(&store, &to).x, marker_at.x),
        "on one vertical line through the marker"
    );
    assert_eq!(node(&store, &to).y >= node(&store, &from).y, was_below, "the product stays on the side it was");

    store.align_map_reaction_axis(PGI, "horizontal");
    // The JS keeps the marker object, which the edit moves in place: read it
    // again.
    let marker_at = marker(&store, PGI);
    assert!(near(node(&store, &from).y, marker_at.y) && near(node(&store, &to).y, marker_at.y));
}

// Text size and font

/// "restyling a text is one Undo step and becomes the style of the next text"
#[test]
fn restyling_a_text_is_one_undo_step_and_becomes_the_style_of_the_next_text() {
    let mut store = ecoli_store();
    let first = store.add_map_text("Pathway", 50.0, 50.0, None).unwrap();
    assert_eq!(first.font_size, Some(TEXT_SIZE.default));
    store.style_map_text(&first.id, Some(40.0), Some("times"));
    assert_eq!(map(&store).text(&first.id).unwrap().font_family.as_deref(), Some("times"));
    assert_eq!(store.view.text_style, TextStyle { font_size: 40.0, font_family: "times".into() });

    let second = store.add_map_text("Next", 60.0, 60.0, None).unwrap();
    assert_eq!(second.font_size, Some(40.0), "the next text uses the last size");
    assert_eq!(second.font_family.as_deref(), Some("times"), "and the last font");

    store.undo_map(); // undoes adding the second text
    store.undo_map(); // undoes the restyle
    assert_eq!(map(&store).text(&first.id).unwrap().font_size, Some(TEXT_SIZE.default));

    store.update_map_text(&first.id, "Renamed", Some((Some(18.0), Some("courier"))));
    let edited = map(&store).text(&first.id).unwrap();
    assert_eq!(edited.text, "Renamed");
    assert_eq!(edited.font_family.as_deref(), Some("courier"));
    store.undo_map();
    assert_eq!(map(&store).text(&first.id).unwrap().text, "Pathway", "words and style are one step");
}

/// "a pathway drawn across the line is not folded back on itself"
#[test]
fn a_pathway_drawn_across_the_line_is_not_folded_back_on_itself() {
    let mut store = ecoli_store();
    // Lay g6p, f6p and fdp side by side, f6p a little below g6p: projected onto a
    // vertical line by position alone, g6p would land between f6p and fdp.
    map_mut(&mut store).move_node(FDP, 600.0, 300.0);
    map_mut(&mut store).move_node(G6P, 400.0, 500.0);
    map_mut(&mut store).move_node(F6P, 500.0, 520.0);
    select(&mut store, &[PGI, PFK], &[]);
    store.arrange_multi_selection("align-vertical");
    let y = |id: &str| node(&store, id).y;
    let ordered = sorted_by_y(&store, &[G6P, F6P, FDP]);
    assert_eq!(ordered[1], F6P, "f6p stays between g6p and fdp");
    for id in [PGI, PFK] {
        let (from, to) = line_ends(&store, id);
        let (low, high) = if y(&from) <= y(&to) { (y(&from), y(&to)) } else { (y(&to), y(&from)) };
        let marker_y = marker(&store, id).y;
        assert!(marker_y > low && marker_y < high, "{id} sits between its own metabolites");
        for other in [G6P, F6P, FDP].into_iter().filter(|n| *n != from && *n != to) {
            assert!(!(y(other) > low && y(other) < high), "{id}'s line does not run over {other}");
        }
    }
}

// Alignment edge cases

/// "a metabolite a membrane stops is reported, and spacing follows where it landed"
#[test]
fn a_metabolite_a_membrane_stops_is_reported_and_spacing_follows_where_it_landed() {
    let mut store = ecoli_store();
    let cell = &map(&store).cells()[0];
    let cytosol = cell.compartments.iter().find(|c| c.id == "c").map(|c| c.rect);
    let periplasm = cell.compartments.iter().find(|c| c.id == "p").map(|c| c.rect);
    let outside = map(&store)
        .nodes()
        .values()
        .find(|n| n.compartment.as_deref() == Some("p") && n.kind == NodeKind::Metabolite && n.primary != Some(false))
        .map(|n| n.id.clone());
    assert!(
        cytosol.is_some() && periplasm.is_some() && outside.is_some(),
        "fixture has a cytosol, a periplasm and a periplasmic metabolite"
    );
    let (cytosol, periplasm, outside) = (cytosol.unwrap(), periplasm.unwrap(), outside.unwrap());
    // Beside the cytosol, inside the periplasm band, at the cytosol's height.
    map_mut(&mut store).move_node(&outside, (periplasm.x + cytosol.x) / 2.0, cytosol.y + cytosol.h / 2.0);
    select(&mut store, &[PGI], &[&outside]);
    let result = store.arrange_multi_selection("align-vertical").unwrap();
    assert!(result.held.contains(&outside), "the periplasmic metabolite could not cross into the cytosol");
    assert_eq!(store.status().state, StatusState::Warn);
    assert!(store.status().message.contains("could not reach the line"), "{}", store.status().message);
    let landed = node(&store, &outside);
    assert!(landed.x < cytosol.x, "it stayed outside the cytosol");
}

/// "a cofactor helper is never put on the line"
#[test]
fn a_cofactor_helper_is_never_put_on_the_line() {
    let mut store = ecoli_store();
    let (from, to) = line_ends(&store, TPI);
    assert_eq!(node(&store, &to).kind, NodeKind::Currency, "TPI runs to its own g3p helper in this map");
    select(&mut store, &[TPI], &[]);
    store.arrange_multi_selection("align-vertical");
    let x = marker(&store, TPI).x;
    assert!(near(node(&store, &from).x, x), "dhap is on the line");
    assert!(!near_eps(node(&store, &to).x, x, 0.5), "the helper is placed beside it");
    assert!(map(&store).reaction(TPI).unwrap().linear.is_none(), "no straight line is drawn to a helper");
}

/// "a transporter-only selection is aligned as plain objects, as before"
#[test]
fn a_transporter_only_selection_is_aligned_as_plain_objects_as_before() {
    let mut store = ecoli_store();
    let transporter = transporter(&store).expect("the fixture has a transporter");
    select(&mut store, &[&transporter], &[G6P]);
    assert!(!map(&store).selection_has_line_reactions(&store.multi_selection));
    // What the plain centre alignment makes of this map and selection.
    let mut plain = map(&store).clone();
    plain.align_selection(&store.multi_selection, "vertical");
    let result = store.arrange_multi_selection("align-vertical").expect("two objects can be aligned");
    assert!(result.held.is_empty() && map(&store).to_json() == plain.to_json(), "the plain centre alignment ran");
    select(&mut store, &[&transporter], &[]);
    assert!(!store.can_arrange_selection("align-vertical"), "one transporter alone offers nothing to align");
}

/// "a selected title joins the line instead of pulling the pathway to it"
#[test]
fn a_selected_title_joins_the_line_instead_of_pulling_the_pathway_to_it() {
    let mut store = ecoli_store();
    let (from, to) = line_ends(&store, PGI);
    let anchors = [node(&store, &from).x, node(&store, &to).x, marker(&store, PGI).x];
    let expected = anchors.iter().fold(0.0, |sum, x| sum + x) / anchors.len() as f64;
    let title = store.add_map_text("Glycolysis", expected - 2000.0, 100.0, None).unwrap();
    select(&mut store, &[PGI], &[]);
    store.multi_selection.texts.insert(title.id.clone());
    store.arrange_multi_selection("align-vertical");
    assert!(near_eps(marker(&store, PGI).x, expected, 1e-6), "the line is where the pathway was");
    assert!(
        near_eps(text_center(map(&store).text(&title.id).unwrap()).x, expected, 1e-6),
        "and the title is centred on it"
    );
}

/// "unselected reactions drawn to a moved metabolite are put in order where they stand"
#[test]
fn unselected_reactions_drawn_to_a_moved_metabolite_are_put_in_order_where_they_stand() {
    let mut store = ecoli_store();
    let neighbour = "eco@G6PDH2r";
    // A curve shaped by hand earlier, its handles now far from both its ends.
    map_mut(&mut store).edit_graph(|g| {
        for seg in g.segments.iter_mut().filter(|s| s.reaction_id == neighbour && s.node_id.as_deref() == Some(G6P)) {
            seg.b1 = Some(Point::new(0.0, 0.0));
            seg.b2 = Some(Point::new(5.0, 5.0));
        }
    });
    select(&mut store, &[PGI], &[]);
    store.arrange_multi_selection("align-vertical");
    let rxn = marker(&store, neighbour);
    let g6p = node(&store, G6P);
    let (left, right) = (js::min(rxn.x, g6p.x) - 90.0, js::max(rxn.x, g6p.x) + 90.0);
    let (top, bottom) = (js::min(rxn.y, g6p.y) - 90.0, js::max(rxn.y, g6p.y) + 90.0);
    for seg in map(&store).segments().iter().filter(|s| s.reaction_id == neighbour && s.node_id.as_deref() == Some(G6P)) {
        for p in [seg.b1, seg.b2].into_iter().flatten() {
            assert!(
                p.x >= left && p.x <= right && p.y >= top && p.y <= bottom,
                "{neighbour}'s curve to g6p stays between its marker and g6p"
            );
        }
    }
}
