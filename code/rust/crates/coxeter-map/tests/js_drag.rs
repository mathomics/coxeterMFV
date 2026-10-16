//! Port of `test/drag.test.mjs`: manual node placement.
//!
//! Everything here runs on a real layout built from the bundled models: the
//! drag module is a pure mutation of the map, so the parts that can actually
//! be wrong — the coordinate conversion, the compartment constraint and the
//! segment bookkeeping — are all testable outside the application.
//!
//! The JavaScript tests drag on the raw object `layoutCommunity` returns; the
//! Rust drag functions take an `EditableMap`, so each test adopts the same
//! layout with `EditableMap::from_layout(layout_community(..), None)`.
//! Where JavaScript gets the moved node back (or `null`), Rust gets `true`
//! (or `false`).
//!
//! Not ported: "the moved position survives a repaint and lands in the SVG"
//!   — every assertion is on the SVG string of `renderMap` (crate
//!   `coxeter-render`); the move it makes first is covered by "moving a
//!   metabolite changes its x and y".

mod common;

use std::collections::HashSet;
use std::sync::Arc;

use coxeter_core::community::{CommunityModel, CommunitySpec, MediumLimit, SpeciesSpec};
use coxeter_core::geometry::{CLEARANCE, center, inset_all, signed_distance};
use coxeter_core::js;
use coxeter_core::mapdata::{Node, NodeKind, Point, Segment};
use coxeter_map::EditableMap;
use coxeter_map::drag::{
    Camera, DRAGGABLE_NODE_KINDS, Placeable, clamp_node_position, is_draggable_node, move_layout_node,
    node_constraint, stage_to_map, update_connected_segments,
};
use coxeter_map::layout::{LayoutOptions, layout_community};
use indexmap::IndexMap;

use common::load_model;

const DEFAULT_NAMES: [(&str, &str); 2] = [("eco", "ecoli_core_mc"), ("sce", "yeast_core_mc")];

/// The JS `buildLayout(names)`: a community of the bundled models on the
/// test medium, and its automatic layout at the default options.
fn build_layout(names: &[(&str, &str)]) -> (CommunityModel, EditableMap) {
    let n = names.len() as f64;
    let species =
        names.iter().map(|(id, file)| SpeciesSpec::new(id, Arc::new(load_model(file))).abundance(1.0 / n)).collect();
    let medium = [
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
    .collect();
    let com = CommunityModel::new(CommunitySpec { species, medium, ..Default::default() }).unwrap();
    let layout = EditableMap::from_layout(layout_community(&com, &LayoutOptions::default()), None);
    (com, layout)
}

/// `nodesOfKind(layout, kind)`, as copies (the tests go on to edit the map).
fn nodes_of_kind(layout: &EditableMap, kind: NodeKind) -> Vec<Node> {
    layout.nodes().values().filter(|node| node.kind == kind).cloned().collect()
}

/// A node with at least `room` of slack on every side, so a test move is not clamped.
fn roomy_node(layout: &EditableMap, kind: NodeKind, room: f64) -> Option<Node> {
    nodes_of_kind(layout, kind).into_iter().find(|node| {
        let Some(c) = node_constraint(layout, Placeable::from(node)) else { return false };
        if signed_distance(&c.bounds, node.pos()) > -(c.pad + room) {
            return false;
        }
        c.holes.iter().all(|hole| signed_distance(hole, node.pos()) > c.clearance + room)
    })
}

/// `assert.deepEqual` on two numbers (strict: `Object.is`).
fn same(a: f64, b: f64) -> bool {
    (a.is_nan() && b.is_nan()) || a.to_bits() == b.to_bits()
}

/// `assert.deepEqual({ x: a.x, y: a.y }, { x: b.x, y: b.y })`.
fn same_point(a: Point, b: Point) -> bool {
    same(a.x, b.x) && same(a.y, b.y)
}

/// The end of a segment that belongs to its metabolite, and the other one:
/// substrates run metabolite -> axis, products axis -> metabolite.
fn ends(seg: &Segment) -> (Point, Point) {
    if seg.side.as_deref() == Some("substrate") { (seg.from, seg.to) } else { (seg.to, seg.from) }
}

// Pointer conversion

/// JS: `stage coordinates convert to map coordinates at any zoom and pan`
#[test]
fn stage_coordinates_convert_to_map_coordinates_at_any_zoom_and_pan() {
    let (left, top) = (64.0, 120.0);
    for camera in [
        Camera { x: 0.0, y: 0.0, scale: 1.0 },
        Camera { x: 37.0, y: -210.0, scale: 0.2718 }, // fitted on a big canvas
        Camera { x: -1450.0, y: -880.0, scale: 3.4 }, // zoomed in, panned away from origin
        Camera { x: 12.0, y: 8.0, scale: 0.05 },      // zoomed all the way out
    ] {
        for map in [Point::new(0.0, 0.0), Point::new(512.5, 91.25), Point::new(2400.0, 1600.0)] {
            // Forward: the camera transform the renderer applies to the SVG element.
            let client = Point::new(map.x * camera.scale + camera.x + left, map.y * camera.scale + camera.y + top);
            let back = stage_to_map(client, camera, left, top);
            assert!((back.x - map.x).abs() < 1e-6, "x {} != {} at {}", back.x, map.x, camera.scale);
            assert!((back.y - map.y).abs() < 1e-6, "y {} != {} at {}", back.y, map.y, camera.scale);
        }
    }
}

/// JS: `a zero scale does not produce a non-finite map coordinate`
#[test]
fn a_zero_scale_does_not_produce_a_non_finite_map_coordinate() {
    let p = stage_to_map(Point::new(10.0, 10.0), Camera { x: 0.0, y: 0.0, scale: 0.0 }, 0.0, 0.0);
    assert!(p.x.is_finite() && p.y.is_finite(), "{p:?}");
}

// Eligibility

/// JS: `primary, shared-medium and secondary metabolites are draggable`
#[test]
fn primary_shared_medium_and_secondary_metabolites_are_draggable() {
    let (_, layout) = build_layout(&DEFAULT_NAMES);
    let mut kinds = DRAGGABLE_NODE_KINDS.to_vec();
    kinds.sort_by(|a, b| js::code_unit_compare(a, b));
    assert_eq!(kinds, ["currency", "medium", "metabolite"]);

    let mut seen: HashSet<String> = HashSet::new();
    for node in layout.nodes().values() {
        seen.insert(node.kind.as_str().to_string());
        assert_eq!(
            is_draggable_node(Some(node)),
            ["metabolite", "medium", "currency"].contains(&node.kind.as_str()),
            "{} ({}) has the wrong eligibility",
            node.id,
            node.kind.as_str()
        );
    }
    for kind in ["metabolite", "medium", "currency"] {
        assert!(seen.contains(kind), "no {kind} nodes on the map");
    }
    // JS: `isDraggableNode(null)` and `isDraggableNode(undefined)`; both are `None` here.
    assert!(!is_draggable_node(None));
}

/// JS: `a secondary currency metabolite can be moved by the layout API`
#[test]
fn a_secondary_currency_metabolite_can_be_moved_by_the_layout_api() {
    let (_, mut layout) = build_layout(&DEFAULT_NAMES);
    let node = nodes_of_kind(&layout, NodeKind::Currency).into_iter().next().expect("no currency nodes on the map");
    let before = node.pos();

    let moved = move_layout_node(&mut layout, &node.id, before.x + 35.0, before.y + 25.0);
    assert!(moved);
    let after = layout.node(&node.id).unwrap().pos();
    assert!(!same_point(after, before));
}

/// JS: `a reaction marker is not a node and cannot be moved`
#[test]
fn a_reaction_marker_is_not_a_node_and_cannot_be_moved() {
    let (_, mut layout) = build_layout(&DEFAULT_NAMES);
    let rid = layout.reactions().keys().next().unwrap().clone();
    let rxn = layout.reaction(&rid).unwrap();
    let before = Point::new(rxn.x, rxn.y);
    assert!(!move_layout_node(&mut layout, &rid, before.x + 90.0, before.y + 90.0));
    let rxn = layout.reaction(&rid).unwrap();
    assert!(same_point(Point::new(rxn.x, rxn.y), before));
}

// Moves

/// JS: `moving a metabolite changes its x and y`
#[test]
fn moving_a_metabolite_changes_its_x_and_y() {
    let (_, mut layout) = build_layout(&DEFAULT_NAMES);
    let node = roomy_node(&layout, NodeKind::Metabolite, 60.0).expect("no metabolite with room to move");
    let before = node.pos();

    let moved = move_layout_node(&mut layout, &node.id, before.x + 40.0, before.y - 25.0);
    assert!(moved, "the moved node was not reported back");
    let node = layout.node(&node.id).unwrap();
    assert!((node.x - (before.x + 40.0)).abs() < 1e-6, "x is {}", node.x);
    assert!((node.y - (before.y - 25.0)).abs() < 1e-6, "y is {}", node.y);
}

/// JS: `a move is rejected rather than writing NaN into the layout`
#[test]
fn a_move_is_rejected_rather_than_writing_nan_into_the_layout() {
    let (_, mut layout) = build_layout(&DEFAULT_NAMES);
    let node = roomy_node(&layout, NodeKind::Metabolite, 60.0).unwrap();
    let before = node.pos();
    assert!(!move_layout_node(&mut layout, &node.id, f64::NAN, 10.0));
    assert!(!move_layout_node(&mut layout, &node.id, 10.0, f64::INFINITY));
    assert!(same_point(layout.node(&node.id).unwrap().pos(), before));
}

/// JS: `an unknown node id is a no-op`
#[test]
fn an_unknown_node_id_is_a_no_op() {
    let (_, mut layout) = build_layout(&DEFAULT_NAMES);
    assert!(!move_layout_node(&mut layout, "met:does-not-exist", 10.0, 10.0));
    assert_eq!(update_connected_segments(&mut layout, "met:does-not-exist", None), 0);
}

// Segments

/// JS: `connected segment endpoints follow the metabolite`
#[test]
fn connected_segment_endpoints_follow_the_metabolite() {
    let (_, mut layout) = build_layout(&DEFAULT_NAMES);
    let node = roomy_node(&layout, NodeKind::Metabolite, 60.0).unwrap();
    let attached: Vec<usize> = layout
        .segments()
        .iter()
        .enumerate()
        .filter(|(_, seg)| seg.node_id.as_deref() == Some(node.id.as_str()))
        .map(|(i, _)| i)
        .collect();
    assert!(!attached.is_empty(), "{} has no segments", node.id);

    let target = Point::new(node.x + 55.0, node.y + 30.0);
    move_layout_node(&mut layout, &node.id, target.x, target.y);

    for i in attached {
        let seg = &layout.segments()[i];
        // Substrates run metabolite -> axis, products axis -> metabolite.
        let (endpoint, other) = ends(seg);
        assert!(
            (endpoint.x - target.x).abs() < 1e-6 && (endpoint.y - target.y).abs() < 1e-6,
            "{} {:?} endpoint is at {},{}",
            seg.reaction_id,
            seg.side,
            endpoint.x,
            endpoint.y
        );
        // The reaction axis is deliberately left where it was.
        let axis = layout.reaction(&seg.reaction_id).unwrap().axis.expect("the reaction has an axis");
        let anchor = if seg.side.as_deref() == Some("substrate") { axis.a1 } else { axis.a2 };
        assert!(same_point(other, anchor), "{} axis endpoint moved", seg.reaction_id);
    }
}

/// JS: `a hub metabolite updates the segments of every reaction it touches`
#[test]
fn a_hub_metabolite_updates_the_segments_of_every_reaction_it_touches() {
    let (_, mut layout) = build_layout(&DEFAULT_NAMES);
    // The busiest movable node, so more than one reaction is involved.
    let mut counts: IndexMap<String, usize> = IndexMap::new();
    for seg in layout.segments() {
        // `if (!seg.nodeId) continue;`
        let Some(node_id) = seg.node_id.as_deref().filter(|id| !id.is_empty()) else { continue };
        *counts.entry(node_id.to_string()).or_insert(0) += 1;
    }
    let mut best: Option<(String, usize)> = None;
    for (id, count) in &counts {
        if !is_draggable_node(layout.node(id)) {
            continue;
        }
        if best.as_ref().is_none_or(|(_, best_count)| count > best_count) {
            best = Some((id.clone(), *count));
        }
    }
    assert!(best.as_ref().is_some_and(|(_, count)| *count > 2), "no well-connected movable node ({best:?})");
    let (best_id, _) = best.unwrap();

    let attached: Vec<usize> = layout
        .segments()
        .iter()
        .enumerate()
        .filter(|(_, seg)| seg.node_id.as_deref() == Some(best_id.as_str()))
        .map(|(i, _)| i)
        .collect();
    let reactions: HashSet<&str> = attached.iter().map(|&i| layout.segments()[i].reaction_id.as_str()).collect();
    assert!(reactions.len() > 1, "only {} reaction(s) on the hub", reactions.len());

    let node = layout.node(&best_id).unwrap().pos();
    move_layout_node(&mut layout, &best_id, node.x + 12.0, node.y + 12.0);
    // The JS `node` is the live object: where the hub is now.
    let node = layout.node(&best_id).unwrap().pos();
    for i in attached {
        let seg = &layout.segments()[i];
        let (endpoint, _) = ends(seg);
        assert!(
            (endpoint.x - node.x).abs() < 1e-6 && (endpoint.y - node.y).abs() < 1e-6,
            "{} did not follow the hub",
            seg.reaction_id
        );
    }
}

/// JS: `unrelated segments are left untouched`
#[test]
fn unrelated_segments_are_left_untouched() {
    let (_, mut layout) = build_layout(&DEFAULT_NAMES);
    let node = roomy_node(&layout, NodeKind::Metabolite, 60.0).unwrap();

    let snapshot: Vec<(usize, Point, Point)> = layout
        .segments()
        .iter()
        .enumerate()
        .filter(|(_, seg)| seg.node_id.as_deref() != Some(node.id.as_str()))
        .map(|(i, seg)| (i, seg.from, seg.to))
        .collect();
    assert!(snapshot.len() > 100, "only {} other segments", snapshot.len());

    move_layout_node(&mut layout, &node.id, node.x + 45.0, node.y + 45.0);

    for (i, from, to) in snapshot {
        let seg = &layout.segments()[i];
        assert!(same_point(seg.from, from), "{}/{} 'from' moved", seg.reaction_id, seg.kind.as_str());
        assert!(same_point(seg.to, to), "{}/{} 'to' moved", seg.reaction_id, seg.kind.as_str());
    }
}

/// JS: `other nodes and every reaction marker stay where they were`
#[test]
fn other_nodes_and_every_reaction_marker_stay_where_they_were() {
    let (_, mut layout) = build_layout(&DEFAULT_NAMES);
    let node = roomy_node(&layout, NodeKind::Metabolite, 60.0).unwrap();
    let others: Vec<(String, Point)> =
        layout.nodes().values().filter(|other| other.id != node.id).map(|other| (other.id.clone(), other.pos())).collect();
    let markers: Vec<(String, Point)> =
        layout.reactions().values().map(|rxn| (rxn.id.clone(), Point::new(rxn.x, rxn.y))).collect();

    move_layout_node(&mut layout, &node.id, node.x + 30.0, node.y + 30.0);

    for (id, at) in others {
        assert!(same_point(layout.node(&id).unwrap().pos(), at), "{id} moved");
    }
    for (id, at) in markers {
        let rxn = layout.reaction(&id).unwrap();
        assert!(same_point(Point::new(rxn.x, rxn.y), at), "marker {id} moved");
    }
}

// Constraints

/// JS: `a cellular metabolite is clamped to its own compartment`
#[test]
fn a_cellular_metabolite_is_clamped_to_its_own_compartment() {
    let (_, layout) = build_layout(&DEFAULT_NAMES);
    let mut checked = 0;

    for node in nodes_of_kind(&layout, NodeKind::Metabolite) {
        let c = node_constraint(&layout, Placeable::from(&node)).unwrap_or_else(|| panic!("no constraint for {}", node.id));

        // Aim far outside the cell, in all four directions.
        for (dx, dy) in [(-9e4, 0.0), (9e4, 0.0), (0.0, -9e4), (0.0, 9e4), (9e4, 9e4)] {
            let p = clamp_node_position(&layout, Placeable::from(&node), node.x + dx, node.y + dy);
            assert!(
                signed_distance(&c.bounds, p) <= -(c.pad - 1e-6),
                "{} escaped {:?} towards {dx},{dy}",
                node.id,
                node.compartment
            );
            for hole in &c.holes {
                assert!(
                    signed_distance(hole, p) >= c.clearance - 1e-6,
                    "{} entered a nested compartment towards {dx},{dy}",
                    node.id
                );
            }
        }
        checked += 1;
    }
    assert!(checked > 50, "only checked {checked} nodes");
}

/// JS: `a metabolite cannot be dragged into a nested child compartment`
#[test]
fn a_metabolite_cannot_be_dragged_into_a_nested_child_compartment() {
    // Yeast has a mitochondrion inside a mitochondrial membrane inside the cytosol,
    // so its cytosolic metabolites have real holes to be kept out of.
    let (_, layout) = build_layout(&[("sce", "yeast_core_mc")]);
    let cell = &layout.cells()[0];
    // `cell.compartments.filter((comp) => comp.parentId)`
    let nested: Vec<_> =
        cell.compartments.iter().filter(|comp| comp.parent_id.as_deref().is_some_and(|p| !p.is_empty())).collect();
    assert!(nested.len() >= 2, "expected nested compartments, got {}", nested.len());

    let mut checked = 0;
    for child in nested {
        let parent_nodes: Vec<Node> = nodes_of_kind(&layout, NodeKind::Metabolite)
            .into_iter()
            .filter(|node| node.compartment.as_deref() == child.parent_id.as_deref())
            .collect();
        if parent_nodes.is_empty() {
            continue;
        }
        let aim = center(&child.rect);
        for node in parent_nodes {
            // Drop it straight onto the middle of the organelle.
            let p = clamp_node_position(&layout, Placeable::from(&node), aim.x, aim.y);
            assert!(
                signed_distance(&child.rect, p) >= node.r + CLEARANCE.hole - 1e-6,
                "{} landed inside {} at {},{}",
                node.id,
                child.id,
                js::to_fixed(p.x, 1),
                js::to_fixed(p.y, 1)
            );
            checked += 1;
        }
    }
    assert!(checked > 10, "only checked {checked} placements");
}

/// JS: `a shared-medium metabolite cannot be dragged inside a cell`
#[test]
fn a_shared_medium_metabolite_cannot_be_dragged_inside_a_cell() {
    let (_, layout) = build_layout(&DEFAULT_NAMES);
    let mediums = nodes_of_kind(&layout, NodeKind::Medium);
    assert!(mediums.len() > 3, "only {} medium nodes", mediums.len());

    let mut checked = 0;
    for node in &mediums {
        for cell in layout.cells() {
            // Aim at the middle of the cell, and at each of its corners.
            let targets = [
                center(&cell.rect),
                Point::new(cell.rect.x + 4.0, cell.rect.y + 4.0),
                Point::new(cell.rect.x + cell.rect.w - 4.0, cell.rect.y + cell.rect.h - 4.0),
            ];
            for target in targets {
                let p = clamp_node_position(&layout, Placeable::from(node), target.x, target.y);
                assert!(
                    signed_distance(&cell.rect, p) > 0.0,
                    "{:?} landed inside cell {} at {},{}",
                    node.base,
                    cell.id,
                    js::to_fixed(p.x, 1),
                    js::to_fixed(p.y, 1)
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 20, "only checked {checked} placements");
}

/// JS: `a shared-medium metabolite stays inside the shared environment`
#[test]
fn a_shared_medium_metabolite_stays_inside_the_shared_environment() {
    let (_, layout) = build_layout(&DEFAULT_NAMES);
    let bounds = inset_all(&layout.environment().rect, CLEARANCE.environment);

    for node in nodes_of_kind(&layout, NodeKind::Medium) {
        for (dx, dy) in [(-9e4, -9e4), (9e4, -9e4), (-9e4, 9e4), (9e4, 9e4)] {
            let p = clamp_node_position(&layout, Placeable::from(&node), node.x + dx, node.y + dy);
            assert!(
                signed_distance(&bounds, p) <= -(node.r + CLEARANCE.bounds - 1e-6),
                "{:?} left the environment towards {dx},{dy}",
                node.base
            );
        }
    }
}

/// JS: `a clamped move still only ever moves the node it was asked about`
#[test]
fn a_clamped_move_still_only_ever_moves_the_node_it_was_asked_about() {
    let (_, mut layout) = build_layout(&DEFAULT_NAMES);
    let node = nodes_of_kind(&layout, NodeKind::Metabolite).into_iter().next().unwrap();
    let others: Vec<(String, Point)> =
        layout.nodes().values().filter(|other| other.id != node.id).map(|other| (other.id.clone(), other.pos())).collect();

    move_layout_node(&mut layout, &node.id, -5e4, -5e4); // far outside everything
    let node = layout.node(&node.id).unwrap();
    assert!(node.x.is_finite() && node.y.is_finite(), "{},{}", node.x, node.y);
    for (id, at) in others {
        assert!(same_point(layout.node(&id).unwrap().pos(), at), "{id} moved");
    }
}

/// JS: `a node with no compartment on the map is left unconstrained`
#[test]
fn a_node_with_no_compartment_on_the_map_is_left_unconstrained() {
    let (_, layout) = build_layout(&DEFAULT_NAMES);
    // { id: 'met:ghost', kind: 'metabolite', cellId: 'nope', compartment: 'c', r: 13, x: 0, y: 0 }
    let kind = NodeKind::Metabolite;
    let orphan = Placeable { kind: &kind, r: 13.0, cell_id: Some("nope"), compartment: Some("c"), x: 0.0, y: 0.0 };
    assert_eq!(node_constraint(&layout, orphan), None);
    let p = clamp_node_position(&layout, orphan, 77.0, 88.0);
    assert!(same_point(p, Point::new(77.0, 88.0)), "{p:?}");
}

// Integration

/// JS: `dragging never disturbs the model, the bounds or the reaction set`
#[test]
fn dragging_never_disturbs_the_model_the_bounds_or_the_reaction_set() {
    let (com, mut layout) = build_layout(&DEFAULT_NAMES);
    let bounds_of = |com: &CommunityModel| -> Vec<String> {
        com.merged
            .reactions
            .values()
            .map(|r| format!("{}:{}:{}", r.id, js::number_to_string(r.lower_bound), js::number_to_string(r.upper_bound)))
            .collect()
    };
    let drawn_of = |layout: &EditableMap| -> Vec<String> {
        let mut ids: Vec<String> = layout.reactions().keys().cloned().collect();
        ids.sort_by(|a, b| js::code_unit_compare(a, b));
        ids
    };
    let before_reactions = com.merged.reactions.len();
    let before_metabolites = com.merged.metabolites.len();
    let before_bounds = bounds_of(&com);
    let before_drawn = drawn_of(&layout);
    let before_segments = layout.segments().len();
    let before_nodes = layout.nodes().len();

    let movable: Vec<String> =
        layout.nodes().values().filter(|node| is_draggable_node(Some(node))).take(25).map(|node| node.id.clone()).collect();
    for id in movable {
        let at = layout.node(&id).unwrap().pos();
        move_layout_node(&mut layout, &id, at.x + 33.0, at.y - 19.0);
    }

    assert_eq!(com.merged.reactions.len(), before_reactions);
    assert_eq!(com.merged.metabolites.len(), before_metabolites);
    assert_eq!(bounds_of(&com), before_bounds, "a reaction bound changed");
    assert_eq!(drawn_of(&layout), before_drawn);
    assert_eq!(layout.segments().len(), before_segments, "segments were added or removed");
    assert_eq!(layout.nodes().len(), before_nodes, "nodes were added or removed");
}

/// JS: `repeated moves are deterministic`
#[test]
fn repeated_moves_are_deterministic() {
    let mut runs: Vec<Vec<String>> = Vec::new();
    for _ in 0..2 {
        let (_, mut layout) = build_layout(&[("eco", "ecoli_core_mc")]);
        let movable: Vec<String> =
            layout.nodes().values().filter(|node| is_draggable_node(Some(node))).map(|node| node.id.clone()).collect();
        for id in &movable {
            let at = layout.node(id).unwrap().pos();
            move_layout_node(&mut layout, id, at.x + 400.0, at.y + 400.0);
        }
        runs.push(
            movable
                .iter()
                .map(|id| {
                    let node = layout.node(id).unwrap();
                    format!("{}:{},{}", node.id, js::to_fixed(node.x, 6), js::to_fixed(node.y, 6))
                })
                .collect(),
        );
    }
    assert_eq!(runs[0], runs[1]);
}
