//! Port of the Store tests of `test/reaction-arrange.test.mjs`, against the
//! real store (`coxeter-map/tests/js_reaction_arrange.rs` ported the whole
//! file over the map alone, with a transcribed stand-in for these five).
//!
//! Ported here (they build a `new Store`): "the store records a drag and a
//! turn as separate undo steps", "the inspector options are undoable edits
//! that leave the model alone", "metabolites picked out on the map are the
//! ones Linear and the compass use", "the inspector offers the compass, the
//! metabolite choice and tidy" and "cofactor labels start enabled and can be
//! turned off from the View menu".
//!
//! Not ported here — pure `EditableMap` / `MapHistory` / `renderMap` tests,
//! ported in coxeter-map (`tests/js_reaction_arrange.rs`): every other test of
//! the file.
//!
//! Not ported: "the reaction inspector offers linear, swap, mirror and rotate
//! as map options" — the store is only its set-up; every assertion is on the
//! `renderInspector` HTML (GUI crate).
//!
//! HTML assertions dropped from the ported tests:
//! - "the inspector options are undoable edits that leave the model alone":
//!   `renderInspector` matching /Kept straight between f6p and fdp/. What that
//!   sentence reads — the two ends of `rxn.linear` — is asserted on the map
//!   instead.
//! - "metabolites picked out on the map are the ones Linear and the compass
//!   use": the `renderInspector` match for the active line-end button.
//! - "the inspector offers the compass, the metabolite choice and tidy": the
//!   `renderInspector` compass, line-end and tidy buttons.
//! - "cofactor labels start enabled and can be turned off from the View
//!   menu": the `renderViewControls` toggle (`aria-pressed`). The SVG half is
//!   ported through `coxeter_render` with what the store hands the renderer
//!   (`map_render_state`): the JS renders the same PFK map with
//!   `labels: 'auto'` and `currencyLabels` false / true, which is the store's
//!   default view with `setView({ currencyLabels })`.
//!
//! `storeWithPFK()` also stubs `store.problem`, `defaultReactionBounds` and
//! `currentReactionBounds`, which only `renderInspector` reads; they are left
//! out. JS reads `reactionSides(..).node` (the live node) after an edit; the
//! Rust reads the node from the map again.

mod common;

use std::sync::Arc;

use coxeter_core::community::{CommunityModel, CommunitySpec, SpeciesSpec};
use coxeter_core::mapdata::Point;
use coxeter_map::reaction_edit::SideEntry;
use coxeter_map::{EditableMap, LayoutOptions, PlaceOptions};
use coxeter_render::mapview::render_map_svg;
use coxeter_state::{RenderOptions, Species, Store, ViewPatch};

use common::{bundled_model, store};

const RID: &str = "eco@PFK";

/// `community()`: E. coli core as the one species `eco`.
fn community() -> Arc<CommunityModel> {
    Arc::new(
        CommunityModel::new(CommunitySpec {
            species: vec![SpeciesSpec::new("eco", Arc::new(bundled_model("ecoli_core_mc"))).abundance(1.0)],
            ..Default::default()
        })
        .unwrap(),
    )
}

fn map(store: &Store) -> &EditableMap {
    store.map.as_ref().expect("a map")
}

/// `store.mapHistory.undoStack.length`.
fn depth(store: &Store) -> usize {
    store.map_history.undo_len()
}

/// `[...map.nodes.values()].find((n) => n.base === base)`: id and position.
fn node_of(map: &EditableMap, base: &str) -> (String, Point) {
    let node = map.nodes().values().find(|n| n.base.as_deref() == Some(base)).unwrap_or_else(|| panic!("no node {base}"));
    (node.id.clone(), node.pos())
}

/// Where a node is now.
fn at(map: &EditableMap, id: &str) -> Point {
    map.node(id).unwrap_or_else(|| panic!("no node {id}")).pos()
}

/// `[rxn.linear.from, rxn.linear.to]`, `None` when not linear.
fn linear_ends(map: &EditableMap, rid: &str) -> Option<(Option<String>, Option<String>)> {
    map.reaction(rid).expect("the reaction is drawn").linear.as_ref().map(|l| (l.from.clone(), l.to.clone()))
}

fn entry<'a>(entries: &'a [SideEntry], label: &str) -> &'a SideEntry {
    entries.iter().find(|e| e.label == label).unwrap_or_else(|| panic!("no {label} on that side"))
}

/// `storeWithPFK()`: E. coli core as `eco`, an empty map, PFK placed and
/// selected.
fn store_with_pfk() -> Store {
    let com = community();
    let mut store = store();
    store.community = Some(com.clone());
    store.species = vec![Species::new("eco", "ecoli_core_mc", 1.0, true)];
    store.set_model("ecoli_core_mc", com.species[0].model.as_ref().clone());
    store.map = Some(EditableMap::empty_from_community(&com, &LayoutOptions::default()));
    store.add_map_reaction(RID, &PlaceOptions::default());
    store.select(Some(RID));
    store
}

// Undo history

/// JS: 'the store records a drag and a turn as separate undo steps'
#[test]
fn the_store_records_a_drag_and_a_turn_as_separate_undo_steps() {
    let com = community();
    let mut store = store();
    store.community = Some(com.clone());
    store.map = Some(EditableMap::empty_from_community(&com, &LayoutOptions::default()));
    store.add_map_reaction(RID, &PlaceOptions::default());
    assert_eq!(depth(&store), 1);

    let (node, pos) = node_of(map(&store), "fdp");
    let drag = store.begin_map_edit("Move metabolite");
    store.move_node(&node, pos.x + 50.0, pos.y + 10.0);
    assert!(store.commit_map_edit(drag), "a drag is an edit");
    assert_eq!(depth(&store), 2);

    let untouched = store.begin_map_edit("Move metabolite");
    assert!(!store.commit_map_edit(untouched), "a press that moved nothing is not");

    assert!(store.rotate_map_reaction(RID, std::f64::consts::PI / 2.0));
    assert_eq!(depth(&store), 3);

    // JS reads `node.x, node.y` of the live node, which the turn has moved.
    let moved = at(map(&store), &node);
    store.undo_map();
    let (_, after) = node_of(map(&store), "fdp");
    assert_eq!((after.x, after.y), (moved.x, moved.y), "undo takes back the turn, not the drag");
    assert_eq!(depth(&store), 2);
}

// Inspector

/// JS: 'the inspector options are undoable edits that leave the model alone'
#[test]
fn the_inspector_options_are_undoable_edits_that_leave_the_model_alone() {
    let mut store = store_with_pfk();
    let equation = |store: &Store| store.community.as_ref().unwrap().merged.reactions[RID].equation();
    let before = equation(&store);
    let start = depth(&store);

    assert!(store.set_map_reaction_linear(RID, true));
    // Stands in for /Kept straight between f6p and fdp/ of the inspector: the
    // two ends of the line it names.
    let (from, to) = linear_ends(map(&store), RID).expect("the reaction is straight");
    let base = |id: Option<String>| map(&store).node(&id.expect("an end")).and_then(|n| n.base.clone());
    assert_eq!((base(from).as_deref(), base(to).as_deref()), (Some("f6p"), Some("fdp")));

    let (_, products) = map(&store).reaction_sides(RID);
    let (a, b) = (products[0].node_id.clone(), products[1].node_id.clone());
    assert!(store.swap_map_reaction_metabolites(RID, &a, &b));
    assert!(store.mirror_map_reaction(RID));
    assert!(store.flip_map_reaction(RID));
    assert_eq!(depth(&store), start + 4);
    assert_eq!(equation(&store), before);

    store.undo_map();
    store.undo_map();
    store.undo_map();
    store.undo_map();
    assert_eq!(linear_ends(map(&store), RID), None);
}

// Compass alignment

/// JS: 'metabolites picked out on the map are the ones Linear and the compass use'
#[test]
fn metabolites_picked_out_on_the_map_are_the_ones_linear_and_the_compass_use() {
    let mut store = store_with_pfk();
    let (substrates, products) = map(&store).reaction_sides(RID);
    let atp = entry(&substrates, "atp").node_id.clone();
    let h = entry(&products, "h").node_id.clone();
    assert_eq!(store.selected_line_ends(RID), (None, None));

    store.toggle_multi_selection("node", &atp);
    store.toggle_multi_selection("node", &h);
    assert_eq!(store.selected_line_ends(RID), (Some(atp.clone()), Some(h.clone())));

    assert!(store.align_map_reaction(RID, "W"));
    assert_eq!(linear_ends(map(&store), RID), Some((Some(atp.clone()), Some(h.clone()))));
    let rxn = map(&store).reaction(RID).unwrap();
    assert!(at(map(&store), &atp).x > rxn.x && at(map(&store), &h).x < rxn.x, "substrate to product runs to the left");

    // Two picked on the same side say nothing about that side.
    store.toggle_multi_selection("node", &entry(&substrates, "f6p").node_id);
    assert_eq!(store.selected_line_ends(RID), (None, Some(h)));
}

/// JS: 'the inspector offers the compass, the metabolite choice and tidy'
#[test]
fn the_inspector_offers_the_compass_the_metabolite_choice_and_tidy() {
    let mut store = store_with_pfk();
    let depth_before = depth(&store);
    assert!(store.align_map_reaction(RID, "NE"));
    // JS: `store.tidyMapReaction(RID) !== undefined` — any answer will do.
    let _ = store.tidy_map_reaction(RID);
    assert_eq!(depth(&store), depth_before + 1, "aligning is one undo step; tidying what is already tidy is none");
    store.undo_map();
    assert_eq!(linear_ends(map(&store), RID), None);
}

// Cofactor labels

/// JS: 'cofactor labels start enabled and can be turned off from the View menu'
#[test]
fn cofactor_labels_start_enabled_and_can_be_turned_off_from_the_view_menu() {
    let com = community();
    let mut store = store();
    assert!(store.view.currency_labels);
    store.set_view(ViewPatch { currency_labels: Some(false), ..Default::default() }).unwrap();
    assert!(!store.view.currency_labels);

    // `pfkMap()`, handed to the renderer with the store's view.
    let mut map = EditableMap::empty_from_community(&com, &LayoutOptions::default());
    assert!(map.add_reaction(&com, RID, &PlaceOptions::default()).is_some(), "{RID} is placed");
    store.map = Some(map);
    assert_eq!(store.view.labels, "auto");
    let named = |store: &Store| {
        render_map_svg(store.map.as_ref().unwrap().graph(), &store.map_render_state(RenderOptions::default()))
            .matches("emc-label-currency")
            .count()
    };
    assert_eq!(named(&store), 0);
    store.set_view(ViewPatch { currency_labels: Some(true), ..Default::default() }).unwrap();
    // atp, adp and h, each beside its own small node.
    assert_eq!(named(&store), 3);
}
