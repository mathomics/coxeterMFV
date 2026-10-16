//! Port of `test/editable-map.test.mjs`.
//!
//! The editable map: what is drawn, as opposed to what exists.
//!
//! The invariants worth defending are that a map may legitimately hold every
//! reaction, some of them or none; that an empty one still draws the structure
//! of the community; that the shell it draws is the same shell a full map
//! draws; and that none of this can reach the model or the solver.
//!
//! Not ported: "an empty map renders as valid SVG with no metabolic content"
//!   — every assertion is about the SVG string (crate `coxeter-render`).
//! Not ported, in "a full automatic layout converts to an equivalent map":
//!   `render(map) === render(layout)` (SVG). `map instanceof EditableMap` has
//!   no Rust counterpart (the type says so).
//! Not ported, in "a partial map keeps its reactions, their segments and their
//!   metabolites": the three SVG assertions at the end.
//! Not ported, in "a map with zero reactions is a valid map, not a broken
//!   one": the SVG assertion at the end.
//! Not ported, in "an empty map survives a JSON round trip":
//!   `render(back) === render(empty)` (SVG).
//! Not ported, in "what is drawn has no effect on the simulation": the three
//!   `FluxProblem(...).parsimonious()` comparisons (crate `coxeter-solver`).
//!   The reaction bounds and the reaction count of the merged model are kept.
//!
//! Stand-ins, where a JavaScript test compares two SVG strings to say that two
//! maps are the same map, the maps' own JSON is compared instead (the renderer
//! is a function of the map):
//!   "a map built from a community matches one built from its layout",
//!   "building a map twice gives the same map", and the `render(back) ===
//!   render(map)` line of "a map survives a JSON round trip, axis identity
//!   included".
//!
//! JavaScript object identity (`assert.equal(a, b)` on two objects) has no
//! direct Rust form: the map owns copies. Where the JS says "the same object"
//! the values are compared, and for a segment end that must *be* its
//! reaction's axis point the map's own record of that link
//! (`segment_links()`) is checked as well as the coordinates.

mod common;

use std::sync::Arc;

use common::load_model;
use coxeter_core::boundaries::{BoundaryKind, boundary_key, environment_key};
use coxeter_core::community::{CommunityModel, CommunitySpec, MediumLimit, SpeciesSpec};
use coxeter_core::geometry::signed_distance;
use coxeter_core::js;
use coxeter_core::mapdata::{AxisEnd, NodeKind, Point, Rect, SegmentKind};
use coxeter_core::testkit::json_diff;
use coxeter_map::layout::{LayoutOptions, layout_community};
use coxeter_map::{EditableMap, PlaceOptions};
use indexmap::IndexMap;
use serde_json::Value;

fn medium() -> IndexMap<String, MediumLimit> {
    [
        ("glc__D", -10.0),
        ("o2", -20.0),
        ("nh4", -1000.0),
        ("pi", -1000.0),
        ("h2o", -1000.0),
        ("h", -1000.0),
        ("co2", -1000.0),
        ("photon", -80.0),
    ]
    .into_iter()
    .map(|(base, lower)| (base.to_string(), MediumLimit { lower: Some(lower), upper: None }))
    .collect()
}

/// `buildCommunity(names)`.
fn build_community(names: &[(&str, &str)]) -> CommunityModel {
    let species = names
        .iter()
        .map(|(id, file)| SpeciesSpec::new(id, Arc::new(load_model(file))).abundance(1.0 / names.len() as f64))
        .collect();
    CommunityModel::new(CommunitySpec { species, medium: medium(), ..Default::default() }).unwrap()
}

/// `buildCommunity()` with its default: E. coli and yeast.
fn default_community() -> CommunityModel {
    build_community(&[("eco", "ecoli_core_mc"), ("sce", "yeast_core_mc")])
}

fn solo_ecoli() -> CommunityModel {
    build_community(&[("eco", "ecoli_core_mc")])
}

fn opts() -> LayoutOptions {
    LayoutOptions::default()
}

/// `JSON.parse(JSON.stringify(value))`.
fn through_json(value: &Value) -> Value {
    serde_json::from_str(&js::stringify(value, None)).unwrap()
}

/// `assert.deepEqual` on two JSON values.
#[track_caller]
fn assert_deep_eq(actual: &Value, expected: &Value, what: &str) {
    if let Some(d) = json_diff(actual, expected, 0.0) {
        panic!("{what}: {d}");
    }
}

/// `array.sort()` with no comparator.
fn sorted(mut ids: Vec<String>) -> Vec<String> {
    ids.sort_by(|a, b| js::code_unit_compare(a, b));
    ids
}

fn xy(map: &EditableMap, node_id: &str) -> (f64, f64) {
    let n = map.node(node_id).unwrap();
    (n.x, n.y)
}

// Empty maps

/// "an empty one-species map draws the cell and its compartments only"
#[test]
fn an_empty_one_species_map_draws_the_cell_and_its_compartments_only() {
    let com = solo_ecoli();
    let map = EditableMap::empty_from_community(&com, &opts());

    assert_eq!(map.cells().len(), 1);
    let cell = &map.cells()[0];
    assert_eq!(cell.id, "eco");
    assert!(cell.rect.w > 0.0 && cell.rect.h > 0.0, "the cell has no area");
    // e > p > c, every one of them drawn.
    assert_eq!(sorted(cell.compartments.iter().map(|c| c.id.clone()).collect()), ["c", "e", "p"]);
    assert!(cell.outer_compartment.as_deref().is_some_and(|id| !id.is_empty()), "no outer compartment");
    assert!(map.environment().rect.w > 0.0, "no shared medium");

    // And nothing inside them.
    assert_eq!(map.reactions().len(), 0, "an empty map drew a reaction");
    assert_eq!(map.nodes().len(), 0, "an empty map drew a metabolite");
    assert_eq!(map.segments().len(), 0, "an empty map drew a segment");
    assert!(map.medium().nodes.is_empty());
    assert!(map.medium().exchanges.is_empty());
    assert!(map.is_empty());
}

/// "an empty multi-species map draws every cell"
#[test]
fn an_empty_multi_species_map_draws_every_cell() {
    let com = build_community(&[
        ("eco", "ecoli_core_mc"),
        ("sce", "yeast_core_mc"),
        ("mba", "methanogen_core_mc"),
        ("lla", "lactobacillus_core_mc"),
    ]);
    let map = EditableMap::empty_from_community(&com, &opts());

    assert_eq!(sorted(map.cells().iter().map(|c| c.id.clone()).collect()), ["eco", "lla", "mba", "sce"]);
    for cell in map.cells() {
        let model = &com.species.iter().find(|s| s.id == cell.id).unwrap().model;
        assert_eq!(
            sorted(cell.compartments.iter().map(|c| c.id.clone()).collect()),
            sorted(model.compartment_ids()),
            "{} is missing compartments",
            cell.id
        );
    }
    // Cells still do not overlap, and still sit inside the medium.
    let rects: Vec<Rect> = map.cells().iter().map(|c| c.rect).collect();
    for i in 0..rects.len() {
        for j in i + 1..rects.len() {
            let (a, b) = (rects[i], rects[j]);
            assert!(
                !(a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h),
                "cells {i} and {j} overlap"
            );
        }
        assert!(
            signed_distance(&map.environment().rect, Point::new(rects[i].x, rects[i].y)) < 0.0,
            "a cell escaped the shared medium"
        );
    }
    assert_eq!(map.reactions().len(), 0);
    assert_eq!(map.nodes().len(), 0);
    assert_eq!(map.segments().len(), 0);
}

/// "the compartment hierarchy of an empty map still nests strictly"
#[test]
fn the_compartment_hierarchy_of_an_empty_map_still_nests_strictly() {
    let com = default_community();
    let map = EditableMap::empty_from_community(&com, &opts());
    let mut checked = 0;
    for cell in map.cells() {
        for comp in &cell.compartments {
            let Some(parent_id) = comp.parent_id.as_deref().filter(|p| !p.is_empty()) else { continue };
            let parent = cell
                .compartments
                .iter()
                .find(|c| c.id == parent_id)
                .unwrap_or_else(|| panic!("{} has no parent rect", comp.id));
            assert!(comp.rect.x >= parent.rect.x - 0.5, "{} escapes left of {}", comp.id, parent.id);
            assert!(comp.rect.y >= parent.rect.y - 0.5, "{} escapes above {}", comp.id, parent.id);
            assert!(comp.rect.x + comp.rect.w <= parent.rect.x + parent.rect.w + 0.5, "{} escapes right", comp.id);
            assert!(comp.rect.y + comp.rect.h <= parent.rect.y + parent.rect.h + 0.5, "{} escapes below", comp.id);
            checked += 1;
        }
    }
    assert!(checked >= 4, "only checked {checked} nested compartments");
}

/// "an empty map has exactly the shell a full map has"
#[test]
fn an_empty_map_has_exactly_the_shell_a_full_map_has() {
    // The shell must not depend on what is drawn inside it, or a user who
    // starts empty and fills the map in would watch the cells move under them.
    let com = default_community();
    let full = EditableMap::from_community(&com, &opts());
    let empty = EditableMap::empty_from_community(&com, &opts());

    assert_eq!(empty.canvas(), full.canvas());
    assert_eq!(empty.environment().rect, full.environment().rect);
    assert_eq!(empty.cells().len(), full.cells().len());
    for i in 0..full.cells().len() {
        assert_eq!(empty.cells()[i].rect, full.cells()[i].rect, "cell {i} moved");
        let shell = |map: &EditableMap| -> Vec<(String, Rect, Option<String>)> {
            map.cells()[i].compartments.iter().map(|c| (c.id.clone(), c.rect, c.parent_id.clone())).collect()
        };
        assert_eq!(shell(&empty), shell(&full), "compartments of cell {i} moved");
    }
    assert_eq!(
        sorted(empty.boundaries().iter().map(|b| b.id.clone()).collect()),
        sorted(full.boundaries().iter().map(|b| b.id.clone()).collect()),
        "the boundaries differ"
    );
}

// Full conversion

/// "a full automatic layout converts to an equivalent map"
#[test]
fn a_full_automatic_layout_converts_to_an_equivalent_map() {
    let com = default_community();
    let layout = layout_community(&com, &opts());
    let map = EditableMap::from_layout(layout.clone(), None);

    assert_eq!(map.width(), layout.width);
    assert_eq!(map.height(), layout.height);
    assert_eq!((map.canvas().width, map.canvas().height), (layout.width, layout.height));
    // JS: the very same objects.
    assert_eq!(map.environment(), &layout.environment);
    assert_eq!(map.cells(), layout.cells.as_slice());
    assert_eq!(map.nodes().len(), layout.nodes.len());
    assert_eq!(map.reactions().len(), layout.reactions.len());
    assert_eq!(map.segments().len(), layout.segments.len());
    assert!(map.reactions().len() > 100, "only {} reactions", map.reactions().len());
    assert!(!map.is_empty());

    // Membership answers agree with the layout it came from.
    for id in layout.reactions.keys() {
        assert!(map.has_reaction(id), "{id}");
    }
    for id in layout.nodes.keys() {
        assert!(map.has_node(id), "{id}");
    }
    assert!(!map.has_reaction("not-a-reaction"));
    assert!(!map.has_node("not-a-node"));
}

/// "a map built from a community matches one built from its layout"
#[test]
fn a_map_built_from_a_community_matches_one_built_from_its_layout() {
    let com = solo_ecoli();
    let direct = EditableMap::from_community(&com, &opts());
    let via_layout = EditableMap::from_layout(layout_community(&com, &opts()), None);
    // JS compares the two SVG strings; see the header.
    assert_deep_eq(&direct.to_json(), &via_layout.to_json(), "the two maps");
}

// Boundaries

/// "every compartment of every cell becomes a boundary, plus the environment"
#[test]
fn every_compartment_of_every_cell_becomes_a_boundary_plus_the_environment() {
    let com = default_community();
    for map in [EditableMap::from_community(&com, &opts()), EditableMap::empty_from_community(&com, &opts())] {
        let env = map.boundary(&environment_key()).expect("no environment boundary");
        assert_eq!(env.kind, BoundaryKind::Environment);
        assert_eq!(env.rect, Some(map.environment().rect), "the environment boundary has no geometry");

        let mut expected = 1;
        for cell in map.cells() {
            for comp in &cell.compartments {
                let b = map
                    .boundary(&boundary_key(Some(&cell.id), &comp.id))
                    .unwrap_or_else(|| panic!("no boundary for {}/{}", cell.id, comp.id));
                // Identity: whose it is, and which two compartments it lies between.
                assert_eq!(b.cell_id.as_deref(), Some(cell.id.as_str()));
                assert_eq!(b.inside, comp.id);
                assert_eq!(b.outside.as_deref(), Some(comp.parent_id.as_deref().unwrap_or("s")));
                assert_eq!(b.kind, if comp.parent_id.is_none() { BoundaryKind::Cell } else { BoundaryKind::Membrane });
                // Geometry, for a later phase to hit-test against.
                assert_eq!(b.rect, Some(comp.rect));
                assert!(!b.membrane.is_empty(), "{} has no membrane style", b.id);
                assert!(b.depth.is_finite());
                expected += 1;
            }
            assert_eq!(map.boundaries_of(Some(&cell.id)).len(), cell.compartments.len());
        }
        assert_eq!(map.boundaries().len(), expected);
        assert!(map.has_boundary(&environment_key()));
        assert!(!map.has_boundary("nope/x"));
        assert!(map.boundary("nope/x").is_none());
    }
}

// Partial maps

/// "a partial map keeps its reactions, their segments and their metabolites"
#[test]
fn a_partial_map_keeps_its_reactions_their_segments_and_their_metabolites() {
    let com = solo_ecoli();
    let full = EditableMap::from_community(&com, &opts());
    let keep: Vec<String> = full.reaction_ids().into_iter().take(12).collect();
    let partial = full.subset_by_reactions(&keep);

    assert_eq!(sorted(partial.reaction_ids()), sorted(keep.clone()));
    assert!(partial.reactions().len() < full.reactions().len());
    assert!(!partial.is_empty());

    // Every segment belongs to a kept reaction, and every kept reaction keeps
    // all of the segments it had.
    for seg in partial.segments() {
        assert!(partial.has_reaction(&seg.reaction_id), "{} has an orphan segment", seg.reaction_id);
    }
    for id in &keep {
        assert_eq!(partial.segments_of(id).len(), full.segments_of(id).len(), "{id} lost segments");
    }
    // Every node still has something attached to it, and every segment's node exists.
    for seg in partial.segments() {
        let Some(node_id) = seg.node_id.as_deref().filter(|n| !n.is_empty()) else { continue };
        assert!(partial.has_node(node_id), "segment points at missing node {node_id}");
    }
    for id in partial.node_ids() {
        assert!(!partial.segments_at(&id).is_empty(), "{id} is an orphan node");
    }
    // The shell is untouched.
    assert_eq!(partial.canvas(), full.canvas());
    assert_eq!(partial.cells().len(), full.cells().len());
    assert_eq!(partial.boundaries().len(), full.boundaries().len());
}

/// "a map with zero reactions is a valid map, not a broken one"
#[test]
fn a_map_with_zero_reactions_is_a_valid_map_not_a_broken_one() {
    let com = solo_ecoli();
    let full = EditableMap::from_community(&com, &opts());
    let cleared = full.cleared();

    assert_eq!(cleared.reactions().len(), 0);
    assert_eq!(cleared.nodes().len(), 0);
    assert_eq!(cleared.segments().len(), 0);
    assert!(cleared.is_empty());
    assert_eq!(cleared.subset_by_reactions(&[]).stats(), cleared.stats());
    // Same shell as the full map it came from.
    assert_eq!(cleared.cells().len(), full.cells().len());
    assert_eq!(cleared.boundaries().len(), full.boundaries().len());
}

/// "a subset is independent of the map it came from"
#[test]
fn a_subset_is_independent_of_the_map_it_came_from() {
    let com = solo_ecoli();
    let full = EditableMap::from_community(&com, &opts());
    let with_node = full
        .segments()
        .iter()
        .find(|seg| {
            seg.node_id.as_deref().filter(|n| !n.is_empty()).and_then(|n| full.node(n)).is_some_and(|n| n.kind == NodeKind::Metabolite)
        })
        .unwrap()
        .clone();
    let mut partial = full.subset_by_reactions(&[with_node.reaction_id.clone()]);

    let id = with_node.node_id.clone().unwrap();
    let before = xy(&full, &id);
    assert!(partial.move_node(&id, before.0 + 25.0, before.1 + 25.0).is_some(), "the subset refused to move its node");

    assert_eq!(xy(&full, &id), before, "moving a node in the subset moved it in the original");
    assert_ne!(xy(&partial, &id), before);
    // The axis of the copied reaction is its own object too.
    let copied = partial.reaction(&with_node.reaction_id).unwrap().axis.as_ref().unwrap();
    let original = full.reaction(&with_node.reaction_id).unwrap().axis.as_ref().unwrap();
    assert!(!std::ptr::eq(copied, original));
}

// V0.2 drag, via map

/// "metabolite dragging still works through the map"
#[test]
fn metabolite_dragging_still_works_through_the_map() {
    let com = default_community();
    let mut map = EditableMap::from_community(&com, &opts());
    let node = map.nodes().values().find(|n| n.kind == NodeKind::Metabolite).unwrap().clone();
    assert!(!map.segments_at(&node.id).is_empty(), "{} has no segments", node.id);

    assert!(map.can_move_node(&node.id));
    let moved = map.move_node(&node.id, node.x + 20.0, node.y + 14.0).cloned();
    // JS: `moved === node`, the live node itself.
    assert_eq!(moved.as_ref(), map.node(&node.id));
    assert!(moved.is_some());
    // The clamp may refuse part of the move; what must hold is that the
    // segments followed wherever it ended up.
    let (nx, ny) = xy(&map, &node.id);
    for seg in map.segments_at(&node.id) {
        let end = if seg.is_substrate() { seg.from } else { seg.to };
        assert!((end.x - nx).abs() < 1e-6 && (end.y - ny).abs() < 1e-6, "{} did not follow", seg.reaction_id);
    }

    // The V0.2 eligibility policy is unchanged.
    let currency = map.nodes().values().find(|n| n.kind == NodeKind::Currency).unwrap().clone();
    let was = (currency.x, currency.y);
    assert!(map.can_move_node(&currency.id));
    let moved = map.move_node(&currency.id, was.0 + 35.0, was.1 + 25.0).cloned();
    assert!(moved.is_some());
    assert_eq!(moved.as_ref(), map.node(&currency.id));
    assert_ne!(xy(&map, &currency.id), was);

    let medium = map.nodes().values().find(|n| n.kind == NodeKind::Medium).unwrap();
    assert!(map.can_move_node(&medium.id), "shared-medium nodes must stay draggable");
}

// I/O

/// "a map survives a JSON round trip, axis identity included"
#[test]
fn a_map_survives_a_json_round_trip_axis_identity_included() {
    let com = solo_ecoli();
    let map = EditableMap::from_community(&com, &opts());
    let mut back = EditableMap::from_json(&through_json(&map.to_json())).unwrap();

    assert_eq!(back.stats(), map.stats());
    // JS compares the two SVG strings; see the header.
    assert_deep_eq(&back.to_json(), &map.to_json(), "the round trip changed the drawing");
    assert_eq!(
        back.boundaries().iter().map(|b| &b.id).collect::<Vec<_>>(),
        map.boundaries().iter().map(|b| &b.id).collect::<Vec<_>>()
    );

    // The axis segment must share objects with its reaction again, or moving
    // an axis later would leave the drawing behind.
    let mut checked = 0;
    for (seg, links) in back.segments().iter().zip(back.segment_links()) {
        let Some(axis) = back.reaction(&seg.reaction_id).and_then(|r| r.axis.as_ref()) else { continue };
        let rid = &seg.reaction_id;
        if seg.kind == SegmentKind::Axis {
            assert_eq!(links.0, Some(AxisEnd::A1), "{rid} axis start not re-linked");
            assert_eq!(seg.from, axis.a1, "{rid} axis start not re-linked");
            assert_eq!(links.1, Some(AxisEnd::A2), "{rid} axis end not re-linked");
            assert_eq!(seg.to, axis.a2, "{rid} axis end not re-linked");
            checked += 1;
        } else if seg.is_substrate() {
            assert_eq!(links.1, Some(AxisEnd::A1), "{rid} substrate not re-linked");
            assert_eq!(seg.to, axis.a1, "{rid} substrate not re-linked");
        } else if seg.is_product() {
            assert_eq!(links.0, Some(AxisEnd::A2), "{rid} product not re-linked");
            assert_eq!(seg.from, axis.a2, "{rid} product not re-linked");
        }
    }
    assert!(checked > 50, "only checked {checked} axes");

    // Dragging still behaves on the restored map.
    let node = back.nodes().values().find(|n| n.kind == NodeKind::Metabolite).unwrap().clone();
    assert!(back.move_node(&node.id, node.x + 5.0, node.y + 5.0).is_some());
}

/// "an empty map survives a JSON round trip"
#[test]
fn an_empty_map_survives_a_json_round_trip() {
    let com = solo_ecoli();
    let empty = EditableMap::empty_from_community(&com, &opts());
    let back = EditableMap::from_json(&through_json(&empty.to_json())).unwrap();
    assert!(back.is_empty());
    assert_eq!(back.stats(), empty.stats());
}

// The map cannot reach the LP

/// "what is drawn has no effect on the simulation"
#[test]
fn what_is_drawn_has_no_effect_on_the_simulation() {
    let com = default_community();
    let bounds_of = |com: &CommunityModel| -> Vec<String> {
        com.merged
            .reactions
            .values()
            .map(|r| format!("{}:{}:{}", r.id, js::number_to_string(r.lower_bound), js::number_to_string(r.upper_bound)))
            .collect()
    };
    let bounds = bounds_of(&com);

    // Build every kind of map this phase allows, in turn.
    let full = EditableMap::from_community(&com, &opts());
    let empty = EditableMap::empty_from_community(&com, &opts());
    let partial = full.subset_by_reactions(&full.reaction_ids().into_iter().take(25).collect::<Vec<_>>());
    let cleared = full.cleared();
    for mut map in [full, empty, partial, cleared] {
        let nodes: Vec<(String, f64, f64)> = map.nodes().values().take(20).map(|n| (n.id.clone(), n.x, n.y)).collect();
        for (id, x, y) in nodes {
            map.move_node(&id, x + 40.0, y - 30.0);
        }
    }

    assert_eq!(bounds_of(&com), bounds, "a reaction bound moved");
    assert_eq!(com.merged.reactions.len(), bounds.len());
}

/// "the model keeps every reaction a partial map leaves out"
#[test]
fn the_model_keeps_every_reaction_a_partial_map_leaves_out() {
    let com = solo_ecoli();
    let full = EditableMap::from_community(&com, &opts());
    let partial = full.subset_by_reactions(&full.reaction_ids().into_iter().take(5).collect::<Vec<_>>());

    let hidden: Vec<String> = full.reaction_ids().into_iter().filter(|id| !partial.has_reaction(id)).collect();
    assert!(hidden.len() > 50, "only {} reactions hidden", hidden.len());
    for id in &hidden {
        assert!(com.merged.reactions.contains_key(id), "{id} vanished from the model when it left the map");
    }
}

// Determinism

/// "building a map twice gives the same map"
#[test]
fn building_a_map_twice_gives_the_same_map() {
    let mut runs = Vec::new();
    for _ in 0..2 {
        let com = solo_ecoli();
        runs.push((EditableMap::from_community(&com, &opts()).to_json(), EditableMap::empty_from_community(&com, &opts()).to_json()));
    }
    // JS compares the SVG strings of the two runs; see the header.
    assert_deep_eq(&runs[0].0, &runs[1].0, "the full map");
    assert_deep_eq(&runs[0].1, &runs[1].1, "the empty map");
}

/// "saved incremental edits survive a JSON round trip"
///
/// Includes the `"terminalMetaboliteId": null` a reaction placed by hand is
/// written with, which must survive the reload as it does in JavaScript.
#[test]
fn saved_incremental_edits_survive_a_json_round_trip() {
    let com = solo_ecoli();
    let mut map = EditableMap::empty_from_community(&com, &opts());

    let reaction_id = com.merged.reactions.keys().find(|id| !id.contains("COMMUNITY_GROWTH")).cloned();
    let reaction_id = reaction_id.expect("a reaction other than the community growth");
    assert!(map.add_reaction(&com, &reaction_id, &PlaceOptions::default()).is_some());

    let movable = map.nodes().values().find(|n| map.can_move_node(&n.id)).cloned();
    let movable = movable.expect("a movable node");
    map.move_node(&movable.id, movable.x + 9.0, movable.y + 7.0);

    let json = js::stringify(&map.to_json(), None);
    let back = EditableMap::from_json(&serde_json::from_str(&json).unwrap()).unwrap();

    assert_deep_eq(&back.to_json(), &map.to_json(), "the round trip");
    assert_eq!(back.stats(), map.stats());
}
