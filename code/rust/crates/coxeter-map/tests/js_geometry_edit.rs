//! Port of `test/geometry-edit.test.mjs`: moving and resizing cells,
//! compartments and the shared medium by hand.
//!
//! Not ported: "shared medium boundary is clickable and gets geometry handles
//! when selected" — every assertion is on the SVG string `renderMap` returns
//! (crate `coxeter-render`).

mod common;

use std::sync::Arc;

use common::load_model;
use coxeter_core::boundaries::environment_key;
use coxeter_core::community::{CommunityModel, CommunitySpec, SpeciesSpec};
use coxeter_core::js;
use coxeter_core::mapdata::{MapCompartment, NodeKind, Point, Rect};
use coxeter_core::testkit::json_diff;
use coxeter_map::EditableMap;
use coxeter_map::editable_map::GeometryTarget;
use coxeter_map::geometry_edit::{nearest_rect_edge, rect_from_handle};
use coxeter_map::layout::LayoutOptions;
use indexmap::IndexMap;
use serde_json::Value;

/// `community(name = 'yeast_core_mc')`.
fn community(name: &str) -> CommunityModel {
    CommunityModel::new(CommunitySpec {
        species: vec![SpeciesSpec::new("sp", Arc::new(load_model(name))).abundance(1.0)],
        ..Default::default()
    })
    .unwrap()
}

/// `EditableMap.emptyFromCommunity(await community())`.
fn empty_yeast_map() -> EditableMap {
    EditableMap::empty_from_community(&community("yeast_core_mc"), &LayoutOptions::default())
}

/// `cell.compartments.find((c) => c.parentId === null)` of the first cell.
fn outer(map: &EditableMap) -> Option<MapCompartment> {
    map.cells()[0].compartments.iter().find(|c| c.parent_id.is_none()).cloned()
}

/// `assert.deepEqual` on two rectangles.
#[track_caller]
fn assert_rect_eq(actual: Rect, expected: Rect) {
    assert_eq!((actual.x, actual.y, actual.w, actual.h), (expected.x, expected.y, expected.w, expected.h));
}

/// JS: "nearest rectangle edge resolves the side of a shared-medium boundary drag"
#[test]
fn nearest_rectangle_edge_resolves_the_side_of_a_shared_medium_boundary_drag() {
    let rect = Rect::new(100.0, 80.0, 500.0, 300.0);
    assert_eq!(nearest_rect_edge(&rect, Point::new(320.0, 82.0)), Some("n"));
    assert_eq!(nearest_rect_edge(&rect, Point::new(598.0, 200.0)), Some("e"));
    assert_eq!(nearest_rect_edge(&rect, Point::new(360.0, 379.0)), Some("s"));
    assert_eq!(nearest_rect_edge(&rect, Point::new(101.0, 180.0)), Some("w"));
}

/// JS: "corner handles produce resized rectangles and the centre handle produces movement"
#[test]
fn corner_handles_produce_resized_rectangles_and_the_centre_handle_produces_movement() {
    let r = Rect::new(100.0, 100.0, 300.0, 200.0);
    assert_rect_eq(
        rect_from_handle(&r, "move", Point::new(0.0, 0.0), Point::new(20.0, -10.0)).unwrap(),
        Rect::new(120.0, 90.0, 300.0, 200.0),
    );
    assert_rect_eq(
        rect_from_handle(&r, "se", Point::new(400.0, 300.0), Point::new(440.0, 330.0)).unwrap(),
        Rect::new(100.0, 100.0, 340.0, 230.0),
    );
}

/// JS: "moving the outer cell moves all of its compartment rectangles"
#[test]
fn moving_the_outer_cell_moves_all_of_its_compartment_rectangles() {
    let mut map = empty_yeast_map();
    let cell_id = map.cells()[0].id.clone();
    let outer = outer(&map);
    assert!(outer.is_some());
    let outer = outer.unwrap();

    let before: IndexMap<String, Rect> = map.cells()[0].compartments.iter().map(|c| (c.id.clone(), c.rect)).collect();
    let target = Rect { x: outer.rect.x + 25.0, y: outer.rect.y + 18.0, ..outer.rect };
    assert!(map.edit_geometry_rect(Some(&cell_id), &outer.id, target).is_some());

    let cell = &map.cells()[0];
    for comp in &cell.compartments {
        let old = before[&comp.id];
        assert!((comp.rect.x - (old.x + 25.0)).abs() < 1e-6, "{}", comp.id);
        assert!((comp.rect.y - (old.y + 18.0)).abs() < 1e-6, "{}", comp.id);
    }
    let outer_now = cell.compartments.iter().find(|c| c.id == outer.id).unwrap();
    assert_rect_eq(cell.rect, outer_now.rect);
}

/// JS: "resizing a parent keeps nested compartments inside it"
#[test]
fn resizing_a_parent_keeps_nested_compartments_inside_it() {
    let mut map = empty_yeast_map();
    let cell_id = map.cells()[0].id.clone();
    let outer = outer(&map);
    assert!(outer.is_some());
    let outer = outer.unwrap();

    let target = Rect::new(outer.rect.x, outer.rect.y, outer.rect.w * 0.82, outer.rect.h * 0.84);
    map.edit_geometry_rect(Some(&cell_id), &outer.id, target);

    let cell = &map.cells()[0];
    for comp in cell.compartments.iter().filter(|c| c.parent_id.as_deref().is_some_and(|p| !p.is_empty())) {
        let parent = cell.compartments.iter().find(|p| Some(&p.id) == comp.parent_id.as_ref()).unwrap();
        assert!(comp.rect.x >= parent.rect.x - 1e-6, "{}", comp.id);
        assert!(comp.rect.y >= parent.rect.y - 1e-6, "{}", comp.id);
        assert!(comp.rect.x + comp.rect.w <= parent.rect.x + parent.rect.w + 1e-6, "{}", comp.id);
        assert!(comp.rect.y + comp.rect.h <= parent.rect.y + parent.rect.h + 1e-6, "{}", comp.id);
    }
}

/// JS: "editing geometry on a populated map transforms cellular content"
#[test]
fn editing_geometry_on_a_populated_map_transforms_cellular_content() {
    let com = community("ecoli_core_mc");
    let mut map = EditableMap::from_community(&com, &LayoutOptions::default());
    let cell_id = map.cells()[0].id.clone();
    let outer = outer(&map).unwrap();
    let node = map
        .nodes()
        .values()
        .find(|n| n.cell_id.as_deref() == Some(cell_id.as_str()) && n.kind == NodeKind::Metabolite)
        .cloned();
    let rxn = map.reactions().values().find(|r| r.cell_id.as_deref() == Some(cell_id.as_str())).cloned();
    assert!(node.is_some() && rxn.is_some());
    let (before_node, before_rxn) = (node.unwrap(), rxn.unwrap());

    map.edit_geometry_rect(
        Some(&cell_id),
        &outer.id,
        Rect { x: outer.rect.x + 30.0, y: outer.rect.y + 20.0, ..outer.rect },
    );

    let node = map.node(&before_node.id).unwrap();
    let rxn = map.reaction(&before_rxn.id).unwrap();
    assert!((node.x - (before_node.x + 30.0)).abs() < 1e-6);
    assert!((node.y - (before_node.y + 20.0)).abs() < 1e-6);
    assert!((rxn.x - (before_rxn.x + 30.0)).abs() < 1e-6);
    assert!((rxn.y - (before_rxn.y + 20.0)).abs() < 1e-6);
}

/// JS: "geometry edits refresh membrane boundary geometry"
#[test]
fn geometry_edits_refresh_membrane_boundary_geometry() {
    let mut map = empty_yeast_map();
    let cell_id = map.cells()[0].id.clone();
    let outer = outer(&map).unwrap();
    let boundary_id = format!("{}/{}", cell_id, outer.id);
    let before = map.boundary(&boundary_id).unwrap().rect.unwrap();

    map.edit_geometry_rect(
        Some(&cell_id),
        &outer.id,
        Rect { x: outer.rect.x + 15.0, y: outer.rect.y + 12.0, ..outer.rect },
    );

    let after = map.boundary(&boundary_id).unwrap().rect.unwrap();
    assert_ne!((after.x, after.y, after.w, after.h), (before.x, before.y, before.w, before.h));
    let outer_now = map.cells()[0].compartments.iter().find(|c| c.id == outer.id).unwrap();
    assert_rect_eq(after, outer_now.rect);
}

/// JS: "geometry survives EditableMap JSON save/load"
#[test]
fn geometry_survives_editable_map_json_save_load() {
    let mut map = empty_yeast_map();
    let cell_id = map.cells()[0].id.clone();
    let outer = outer(&map).unwrap();
    map.edit_geometry_rect(
        Some(&cell_id),
        &outer.id,
        Rect::new(outer.rect.x + 10.0, outer.rect.y + 10.0, outer.rect.w - 20.0, outer.rect.h - 20.0),
    );

    // `EditableMap.fromJSON(JSON.parse(JSON.stringify(map.toJSON())))`.
    let doc: Value = serde_json::from_str(&js::stringify(&map.to_json(), None)).unwrap();
    let restored = EditableMap::from_json(&doc).unwrap();
    assert_eq!(json_diff(&restored.to_json(), &map.to_json(), 0.0), None);
}

/// JS: "shared medium is a first-class geometry target that can move the whole map"
#[test]
fn shared_medium_is_a_first_class_geometry_target_that_can_move_the_whole_map() {
    let mut map = empty_yeast_map();
    let before_env = map.environment().rect;
    let before_cell = map.cells()[0].rect;
    let env_id = map.environment().id.clone();
    // `target.environment` is true and `target.compartment` is the environment.
    assert_eq!(map.geometry_target(None, &env_id), Some(GeometryTarget::Environment));

    let moved = map.edit_geometry_rect(None, &env_id, Rect { x: before_env.x + 35.0, y: before_env.y + 24.0, ..before_env });
    assert!(moved.is_some());
    assert_eq!(map.environment().rect.x, before_env.x + 35.0);
    assert_eq!(map.environment().rect.y, before_env.y + 24.0);
    assert_eq!(map.cells()[0].rect.x, before_cell.x + 35.0);
    assert_eq!(map.cells()[0].rect.y, before_cell.y + 24.0);
    assert_rect_eq(map.boundary(&environment_key()).unwrap().rect.unwrap(), map.environment().rect);
}

/// JS: "shared medium boundary can be expanded without moving the cell"
#[test]
fn shared_medium_boundary_can_be_expanded_without_moving_the_cell() {
    let mut map = empty_yeast_map();
    let before_cell = map.cells()[0].rect;
    let before = map.environment().rect;
    let env_id = map.environment().id.clone();

    let resized = map.edit_geometry_rect(None, &env_id, Rect::new(before.x, before.y, before.w + 260.0, before.h + 180.0));

    assert!(resized.is_some());
    let env = map.environment().rect;
    assert!(env.w >= before.w + 260.0);
    assert!(env.h >= before.h + 180.0);
    assert_rect_eq(map.cells()[0].rect, before_cell);
    assert!(map.canvas().width >= env.x + env.w);
    assert!(map.canvas().height >= env.y + env.h);
}

// Resizing with transporters

/// `dragCorner(map, cellId, compartmentId, handle, dx, dy, steps = 20)`:
/// resize as a corner handle does, every pointer move asking for the rect the
/// pointer now implies, worked out from the rect the drag started with.
fn drag_corner(map: &mut EditableMap, cell_id: &str, compartment_id: &str, handle: &str, dx: f64, dy: f64) -> Rect {
    let steps = 20;
    let start = compartment_rect(map, compartment_id);
    let corner = Point::new(
        start.x + if handle.contains('e') { start.w } else { 0.0 },
        start.y + if handle.contains('s') { start.h } else { 0.0 },
    );
    for i in 1..=steps {
        let at = Point::new(corner.x + (dx * i as f64) / steps as f64, corner.y + (dy * i as f64) / steps as f64);
        let rect = rect_from_handle(&start, handle, corner, at).unwrap();
        map.edit_geometry_rect(Some(cell_id), compartment_id, rect);
    }
    start
}

fn compartment_rect(map: &EditableMap, compartment_id: &str) -> Rect {
    map.cells()[0].compartments.iter().find(|c| c.id == compartment_id).unwrap().rect
}

/// `sideOf(rect, p)`: the side of `rect` nearest to `p`.
fn side_of(rect: &Rect, p: Point) -> &'static str {
    let mut d = [
        ("left", (p.x - rect.x).abs()),
        ("right", (p.x - (rect.x + rect.w)).abs()),
        ("top", (p.y - rect.y).abs()),
        ("bottom", (p.y - (rect.y + rect.h)).abs()),
    ];
    d.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    d[0].0
}

/// `yeastCytosol()`: the cytosol of the yeast layout, with a transporter on
/// each of three of its sides, and every curve bent so it shows if a handle
/// is left behind or taken twice. Returns the map, the cell id, the cytosol
/// id and its transporters.
fn yeast_cytosol() -> (EditableMap, String, String, Vec<String>) {
    let mut map = EditableMap::from_community(&community("yeast_core_mc"), &LayoutOptions::default());
    let cell = &map.cells()[0];
    let root = cell.compartments.iter().find(|k| k.parent_id.is_none()).unwrap().id.clone();
    let cytosol = cell.compartments.iter().find(|c| c.parent_id.as_deref() == Some(root.as_str())).unwrap().clone();
    let cell_id = cell.id.clone();
    let transporters: Vec<String> =
        map.reactions().iter().filter(|(_, r)| r.membrane.as_deref() == Some(cytosol.id.as_str())).map(|(id, _)| id.clone()).collect();
    let mut sides: Vec<&str> = transporters
        .iter()
        .map(|id| {
            let r = map.reaction(id).unwrap();
            side_of(&cytosol.rect, Point::new(r.x, r.y))
        })
        .collect();
    sides.sort();
    sides.dedup();
    assert_eq!(sides, vec!["bottom", "left", "top"], "yeast has cytosol transporters on three sides");

    map.edit_graph(|g| {
        for seg in &mut g.segments {
            let node = seg.node_id.as_ref().and_then(|id| g.nodes.get(id));
            let (Some(node), Some(rxn)) = (node, g.reactions.get(&seg.reaction_id)) else { continue };
            seg.b1 = Some(Point::new(node.x + 7.0, node.y - 11.0));
            seg.b2 = Some(Point::new(rxn.x - 13.0, rxn.y + 5.0));
        }
    });
    (map, cell_id, cytosol.id, transporters)
}

/// `positions(map)`.
struct Positions {
    nodes: IndexMap<String, Point>,
    reactions: IndexMap<String, Point>,
    segments: Vec<(Option<Point>, Option<Point>)>,
}

fn positions(map: &EditableMap) -> Positions {
    Positions {
        nodes: map.nodes().values().map(|n| (n.id.clone(), n.pos())).collect(),
        reactions: map.reactions().iter().map(|(id, r)| (id.clone(), Point::new(r.x, r.y))).collect(),
        segments: map.segments().iter().map(|s| (s.b1, s.b2)).collect(),
    }
}

/// `assertHandlesFollow(map, before)`: each b1 keeps its offset from its
/// metabolite and each b2 from its reaction.
#[track_caller]
fn assert_handles_follow(map: &EditableMap, before: &Positions) {
    let near = |a: Point, b: Point| (a.x - b.x).abs() < 1e-6 && (a.y - b.y).abs() < 1e-6;
    for (i, seg) in map.segments().iter().enumerate() {
        let (Some(old_b1), old_b2) = before.segments[i] else { continue };
        let Some(node_id) = seg.node_id.as_deref() else { continue };
        let (Some(n0), Some(r0)) = (before.nodes.get(node_id), before.reactions.get(&seg.reaction_id)) else { continue };
        let n1 = map.node(node_id).unwrap().pos();
        let r1 = map.reaction(&seg.reaction_id).unwrap();
        let (b1, b2, old_b2) = (seg.b1.unwrap(), seg.b2.unwrap(), old_b2.unwrap());
        assert!(
            near(Point::new(b1.x - n1.x, b1.y - n1.y), Point::new(old_b1.x - n0.x, old_b1.y - n0.y)),
            "b1 of {} at {node_id} left its metabolite",
            seg.reaction_id
        );
        assert!(
            near(Point::new(b2.x - r1.x, b2.y - r1.y), Point::new(old_b2.x - r0.x, old_b2.y - r0.y)),
            "b2 of {} at {node_id} left its reaction",
            seg.reaction_id
        );
    }
}

/// JS: "dragging one corner of a compartment leaves its other sides where they are"
///
/// The yeast cytosol has room to come in at the top and on either side; its
/// bottom has metabolites right against it.
#[test]
fn dragging_one_corner_of_a_compartment_leaves_its_other_sides_where_they_are() {
    let (mut map, cell, cytosol, _) = yeast_cytosol();
    let start = drag_corner(&mut map, &cell, &cytosol, "ne", -40.0, 20.0);
    let r = compartment_rect(&map, &cytosol);
    // Transporters must not hold every side out a margin beyond them: they
    // follow the side there, so each step of the drag would push all four out.
    assert_eq!(r.x, start.x);
    assert_eq!(r.y + r.h, start.y + start.h);
    assert!(((r.x + r.w) - (start.x + start.w - 40.0)).abs() < 1e-6, "the right side came in");
    assert!((r.y - (start.y + 20.0)).abs() < 1e-6, "the top came down");
}

/// JS: "a transporter on a resized side moves only across it, by as far as the side moved"
#[test]
fn a_transporter_on_a_resized_side_moves_only_across_it_by_as_far_as_the_side_moved() {
    let (mut map, cell, cytosol, transporters) = yeast_cytosol();
    let before = positions(&map);
    let start = drag_corner(&mut map, &cell, &cytosol, "nw", 30.0, 20.0);
    let r = compartment_rect(&map, &cytosol);
    let moved = |side: &str| match side {
        "left" => Point::new(r.x - start.x, 0.0),
        "right" => Point::new((r.x + r.w) - (start.x + start.w), 0.0),
        "top" => Point::new(0.0, r.y - start.y),
        _ => Point::new(0.0, (r.y + r.h) - (start.y + start.h)),
    };
    assert!(moved("left").x > 0.0 && moved("top").y > 0.0, "the left side and the top both came in");

    let mut slid = 0;
    for id in &transporters {
        let old = before.reactions[id];
        let rxn = map.reaction(id).unwrap();
        let now = Point::new(rxn.x, rxn.y);
        let side = side_of(&start, old);
        let horizontal = side == "top" || side == "bottom";
        // (along, across) coordinates of a point.
        let split = |p: Point| if horizontal { (p.x, p.y) } else { (p.y, p.x) };
        let (old_along, old_across) = split(old);
        let (now_along, now_across) = split(now);
        let (_, want_across) = split(moved(side));
        assert!(
            (now_across - old_across - want_across).abs() < 1e-6,
            "{id} on the {side} moved across it by {}",
            now_across - old_across
        );

        // Along it, nowhere: spread in proportion to its new length, as
        // before, every transporter slid away from the metabolites drawn
        // against it. Only one the next side came in past moves, to keep
        // clear of the new corner.
        let (lo, len) = if horizontal { (r.x, r.w) } else { (r.y, r.h) };
        let hi = lo + len;
        let want = js::min(js::max(old_along, lo + 18.0), hi - 18.0);
        assert!((now_along - want).abs() < 1e-6, "{id} on the {side} slid along it by {}", now_along - old_along);
        if want != old_along {
            slid += 1;
        }
    }
    assert_eq!(slid, 1, "O2t, near the corner the left side came in past");
    assert_handles_follow(&map, &before);
}

/// JS: "a resized side pulled past the parent stops there instead of shifting the far side"
#[test]
fn a_resized_side_pulled_past_the_parent_stops_there_instead_of_shifting_the_far_side() {
    let (mut map, cell, cytosol, _) = yeast_cytosol();
    let parent_id = map.cells()[0].compartments.iter().find(|c| c.id == cytosol).unwrap().parent_id.clone().unwrap();
    let parent = compartment_rect(&map, &parent_id);
    let start = drag_corner(&mut map, &cell, &cytosol, "se", 400.0, 300.0);
    let r = compartment_rect(&map, &cytosol);
    assert_eq!(r.x, start.x);
    assert_eq!(r.y, start.y);
    assert!(((r.x + r.w) - (parent.x + parent.w - 10.0)).abs() < 1e-6);
    assert!(((r.y + r.h) - (parent.y + parent.h - 10.0)).abs() < 1e-6);
}

/// JS: "a metabolite carried by a resized boundary takes its handle on other curves too"
#[test]
fn a_metabolite_carried_by_a_resized_boundary_takes_its_handle_on_other_curves_too() {
    let (mut map, cell, cytosol, transporters) = yeast_cytosol();
    // A transporter on the top, with a metabolite of its own drawn into a
    // second reaction that stays where it is.
    let rect = compartment_rect(&map, &cytosol);
    let rxn = transporters
        .iter()
        .find(|id| {
            let t = map.reaction(id).unwrap();
            side_of(&rect, Point::new(t.x, t.y)) == "top" && map.nodes().values().any(|n| n.reaction_id.as_deref() == Some(id.as_str()))
        })
        .unwrap()
        .clone();
    let own = map.nodes().values().find(|n| n.reaction_id.as_deref() == Some(rxn.as_str())).unwrap().clone();
    let other = map
        .reactions()
        .values()
        .find(|r| r.cell_id.as_deref() == Some(cell.as_str()) && r.membrane.as_deref().is_none_or(str::is_empty))
        .unwrap()
        .clone();
    let mut shared = map.segments().iter().find(|s| s.node_id.as_deref() == Some(own.id.as_str())).unwrap().clone();
    shared.reaction_id = other.id.clone();
    shared.b1 = Some(Point::new(own.x + 7.0, own.y - 11.0));
    shared.b2 = Some(Point::new(other.x - 13.0, other.y + 5.0));
    let before_b1 = shared.b1.unwrap();
    map.edit_graph(|g| g.segments.push(shared));

    drag_corner(&mut map, &cell, &cytosol, "ne", 0.0, 20.0);
    let dy = map.node(&own.id).unwrap().y - own.y;
    assert!(dy > 0.0, "the transporter came down with the top");
    let b1 = map.segments().last().unwrap().b1.unwrap();
    assert_eq!((b1.x, b1.y), (before_b1.x, before_b1.y + dy));
}

/// JS: "a side handle changes only the width or only the height, however the pointer wanders"
#[test]
fn a_side_handle_changes_only_the_width_or_only_the_height() {
    let (mut map, cell, cytosol, _) = yeast_cytosol();
    let before = positions(&map);
    let start = drag_corner(&mut map, &cell, &cytosol, "e", -40.0, 25.0);
    let r = compartment_rect(&map, &cytosol);
    assert_eq!((r.x, r.y, r.h), (start.x, start.y, start.h));
    assert!((r.w - (start.w - 40.0)).abs() < 1e-6, "the right side came in");

    let start = drag_corner(&mut map, &cell, &cytosol, "n", -30.0, 20.0);
    let r = compartment_rect(&map, &cytosol);
    assert_eq!((r.x, r.w, r.y + r.h), (start.x, start.w, start.y + start.h));
    assert!((r.y - (start.y + 20.0)).abs() < 1e-6, "the top came down");
    assert_handles_follow(&map, &before);
}
