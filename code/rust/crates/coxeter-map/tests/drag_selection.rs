//! Port of `test/drag-selection.test.mjs`: dragging a multi-selection
//! (`drag_selection`). An organelle selected with what is round it moves
//! whole, with everything in it, and a cofactor selected with its reaction is
//! not moved twice.
//!
//! Everything runs on the yeast layout, which has a mitochondrion inside a
//! mitochondrial membrane inside the cytosol.
//!
//! Ported elsewhere: "the store selects compartments by rectangle and its
//! drag moves them" (`coxeter-state/tests/drag_selection.rs`) and "a selected
//! compartment is outlined in the drawing"
//! (`coxeter-render/tests/drag_selection.rs`).

mod common;

use std::sync::Arc;

use coxeter_core::community::{CommunityModel, CommunitySpec, SpeciesSpec};
use coxeter_core::mapdata::{MapCompartment, NodeKind, Point, Rect};
use coxeter_map::EditableMap;
use coxeter_map::layout::{LayoutOptions, layout_community};
use coxeter_map::multi_selection::{
    Selection, SelectionRect, empty_selection, selectable_in_rect, selection_size, translate_selection_with,
};

use common::load_model;

fn yeast() -> EditableMap {
    let species = vec![SpeciesSpec::new("sce", Arc::new(load_model("yeast_core_mc"))).abundance(1.0)];
    let com = CommunityModel::new(CommunitySpec { species, ..Default::default() }).unwrap();
    EditableMap::from_layout(layout_community(&com, &LayoutOptions::default()), None)
}

/// The innermost compartment that still holds one: the mitochondrial
/// membrane round the mitochondrion (the cytosol holds compartments too, but
/// sits higher). Returns the cell id, the organelle and its parent.
fn organelle(map: &EditableMap) -> (String, MapCompartment, MapCompartment) {
    let cell = &map.cells()[0];
    let depth = |c: &MapCompartment| {
        let mut d = 0;
        let mut q = c.parent_id.clone();
        while let Some(pid) = q {
            d += 1;
            q = cell.compartments.iter().find(|k| k.id == pid).and_then(|k| k.parent_id.clone());
        }
        d
    };
    let mut holders: Vec<&MapCompartment> = cell
        .compartments
        .iter()
        .filter(|c| c.parent_id.is_some() && cell.compartments.iter().any(|k| k.parent_id.as_ref() == Some(&c.id)))
        .collect();
    holders.sort_by_key(|c| std::cmp::Reverse(depth(c)));
    let org = (*holders.first().expect("yeast has an organelle with a compartment inside it")).clone();
    assert!(depth(&org) >= 2, "expected the mitochondrial membrane, got {}", org.id);
    let parent = cell.compartments.iter().find(|c| Some(&c.id) == org.parent_id.as_ref()).unwrap().clone();
    (cell.id.clone(), org, parent)
}

fn around(r: &Rect, margin: f64) -> SelectionRect {
    SelectionRect::from_xywh(r.x - margin, r.y - margin, r.w + 2.0 * margin, r.h + 2.0 * margin)
}

fn compartment_rect(map: &EditableMap, cell: &str, id: &str) -> Rect {
    map.cells().iter().find(|c| c.id == cell).unwrap().compartments.iter().find(|c| c.id == id).unwrap().rect
}

/// Nodes the organelle carries: those inside it or in what it holds.
fn inside(map: &EditableMap, cell: &str, org: &MapCompartment) -> Vec<String> {
    let c = map.cells().iter().find(|c| c.id == cell).unwrap();
    let held: Vec<&str> = c
        .compartments
        .iter()
        .filter(|k| k.id == org.id || k.parent_id.as_deref() == Some(org.id.as_str()))
        .map(|k| k.id.as_str())
        .collect();
    map.nodes().values().filter(|n| n.compartment.as_deref().is_some_and(|k| held.contains(&k))).map(|n| n.id.clone()).collect()
}

/// `shiftedBy(a, b, d)`: `b` is `a` moved by `d`.
fn shifted_by(a: Point, b: Point, d: Point) -> bool {
    (b.x - a.x - d.x).abs() < 1e-6 && (b.y - a.y - d.y).abs() < 1e-6
}

#[test]
fn a_rectangle_round_an_organelle_selects_it_and_what_it_holds() {
    let map = yeast();
    let (cell, org, _) = organelle(&map);
    let selection = selectable_in_rect(&map, around(&org.rect, 4.0), true);
    assert!(selection.compartments.contains(&(cell.clone(), org.id.clone())));
    let child = map.cells()[0].compartments.iter().find(|k| k.parent_id.as_ref() == Some(&org.id)).unwrap();
    assert!(selection.compartments.contains(&(cell.clone(), child.id.clone())));
    assert!(selection_size(&selection) > selection.nodes.len() + selection.reactions.len());
    // A rectangle that leaves part of it out does not take the membrane.
    let half = SelectionRect::from_xywh(org.rect.x - 4.0, org.rect.y - 4.0, org.rect.w / 2.0, org.rect.h + 8.0);
    assert!(!selectable_in_rect(&map, half, true).compartments.contains(&(cell, org.id)));
}

#[test]
fn dragging_a_selection_carries_its_organelle_and_everything_in_it() {
    let mut map = yeast();
    let (cell, org, parent) = organelle(&map);
    let selection = selectable_in_rect(&map, around(&org.rect, 4.0), true);
    let carried = inside(&map, &cell, &org);
    assert!(carried.len() > 5, "the organelle should hold metabolites, got {}", carried.len());
    let before: Vec<Point> = carried.iter().map(|id| map.node(id).unwrap().pos()).collect();

    let room_left = org.rect.x - (parent.rect.x + 10.0);
    let dx = if room_left > 2.0 { -(room_left / 2.0).min(30.0) } else { 10.0 };
    let moved = map.drag_selection(&selection, dx, 0.0).unwrap();

    assert!(moved.compartments.contains(&(cell.clone(), org.id.clone())));
    assert!(moved.dx != 0.0, "the organelle should have moved");
    let r = compartment_rect(&map, &cell, &org.id);
    assert!((r.x - (org.rect.x + moved.dx)).abs() < 1e-9 && (r.y - org.rect.y).abs() < 1e-9);
    for (id, p) in carried.iter().zip(&before) {
        let n = map.node(id).unwrap();
        assert!(
            (n.x - (p.x + moved.dx)).abs() < 1e-9 && (n.y - p.y).abs() < 1e-9,
            "{id} went to {},{} instead of moving with its organelle",
            n.x,
            n.y
        );
    }
}

#[test]
fn the_old_translation_leaves_an_organelle_behind_and_squeezes_what_is_in_it() {
    // What drag_selection fixes: translate_selection on the same selection
    // moves the metabolites but not the membrane, so pushed far enough they
    // pile up at the membrane instead of keeping their shape.
    let mut map = yeast();
    let (cell, org, _) = organelle(&map);
    let selection = selectable_in_rect(&map, around(&org.rect, 4.0), true);
    let carried: Vec<String> = inside(&map, &cell, &org).into_iter().filter(|id| selection.nodes.contains(id)).collect();
    let before: Vec<f64> = carried.iter().map(|id| map.node(id).unwrap().x).collect();
    map.translate_selection(&selection, -100_000.0, 0.0);
    assert_eq!(compartment_rect(&map, &cell, &org.id), org.rect, "the membrane stayed where it was");
    let spread = |xs: &[f64]| xs.iter().cloned().fold(f64::MIN, f64::max) - xs.iter().cloned().fold(f64::MAX, f64::min);
    let after: Vec<f64> = carried.iter().map(|id| map.node(id).unwrap().x).collect();
    assert!(spread(&after) < spread(&before), "the contents were squeezed");
}

#[test]
fn a_selection_stops_together_when_its_organelle_meets_its_parent() {
    let mut map = yeast();
    let (cell, org, parent) = organelle(&map);
    let selection = selectable_in_rect(&map, around(&org.rect, 4.0), true);
    let carried = inside(&map, &cell, &org);
    let before: Vec<Point> = carried.iter().map(|id| map.node(id).unwrap().pos()).collect();

    let moved = map.drag_selection(&selection, -100_000.0, 0.0).unwrap();
    assert!(moved.dx > -100_000.0 && moved.dx <= 0.0);
    let r = compartment_rect(&map, &cell, &org.id);
    assert!(r.x >= parent.rect.x + 10.0 - 1e-6, "the organelle left its parent: {} < {}", r.x, parent.rect.x + 10.0);
    for (id, p) in carried.iter().zip(&before) {
        let n = map.node(id).unwrap();
        assert!((n.x - (p.x + moved.dx)).abs() < 1e-9, "{id} did not stop with its organelle");
    }
}

#[test]
fn curves_keep_their_shape_b1_goes_with_its_metabolite_b2_with_its_reaction() {
    // Moving an organelle takes its transporters along. A curve from one of
    // them to a cytosolic metabolite that stays must stretch, not loop: its
    // metabolite end handle (b1) stays with the metabolite. One whose
    // metabolite comes too moves whole.
    let mut map = yeast();
    let (_, org, _) = organelle(&map);
    // A drawn layout leaves handles to the renderer; a map saved after
    // editing has them on every branch, and those are what a move can bend.
    let lerp = |a: Point, b: Point, t: f64| Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t);
    map.edit_graph(|g| {
        for seg in &mut g.segments {
            if seg.node_id.is_none() || seg.b1.is_some() {
                continue;
            }
            let (m, r) = if seg.is_product() { (seg.to, seg.from) } else { (seg.from, seg.to) };
            seg.b1 = Some(lerp(m, r, 1.0 / 3.0));
            seg.b2 = Some(lerp(m, r, 2.0 / 3.0));
        }
    });
    // Tight round the organelle, so most cytosolic partners are left behind.
    let selection = selectable_in_rect(&map, around(&org.rect, 1.0), true);
    let before: Vec<_> = map.segments().iter().map(|s| (s.node_id.clone(), s.reaction_id.clone(), s.b1, s.b2)).collect();
    let nodes: Vec<(String, Point)> = map.nodes().values().map(|n| (n.id.clone(), n.pos())).collect();
    let reactions: Vec<(String, Point)> = map.reactions().iter().map(|(id, r)| (id.clone(), Point::new(r.x, r.y))).collect();
    let node_was = |id: &str| nodes.iter().find(|(k, _)| k == id).map(|(_, p)| *p);
    let reaction_was = |id: &str| reactions.iter().find(|(k, _)| k == id).map(|(_, p)| *p);

    let moved = map.drag_selection(&selection, -15.0, 10.0).unwrap();
    let d = Point::new(moved.dx, moved.dy);
    assert!(d.x != 0.0 || d.y != 0.0);
    let zero = Point::new(0.0, 0.0);

    let (mut stretched, mut whole) = (0, 0);
    for (seg, (node_id, reaction_id, b1, b2)) in map.segments().iter().zip(&before) {
        let (Some(node_id), Some(b1), Some(b2)) = (node_id, b1, b2) else { continue };
        let (Some(node), Some(rxn)) = (map.node(node_id), map.reaction(reaction_id)) else { continue };
        let (Some(n0), Some(r0)) = (node_was(node_id), reaction_was(reaction_id)) else { continue };
        let node_moved = shifted_by(n0, node.pos(), d);
        let node_stayed = shifted_by(n0, node.pos(), zero);
        let rxn_moved = shifted_by(r0, Point::new(rxn.x, rxn.y), d);
        let (nb1, nb2) = (seg.b1.unwrap(), seg.b2.unwrap());
        if rxn_moved && node_stayed {
            stretched += 1;
            assert!(shifted_by(*b1, nb1, zero), "b1 of {reaction_id} -> {node_id} left its metabolite");
            assert!(shifted_by(*b2, nb2, d), "b2 of {reaction_id} -> {node_id} did not follow its reaction");
        } else if rxn_moved && node_moved {
            whole += 1;
            assert!(shifted_by(*b1, nb1, d) && shifted_by(*b2, nb2, d), "curve {reaction_id} -> {node_id} did not move whole");
        }
    }
    assert!(stretched > 0, "some curve reaches a metabolite left behind");
    assert!(whole > 0, "some curve moves whole");
}

#[test]
fn a_cytosolic_metabolite_drawn_inside_the_moving_organelle_rides_with_it() {
    // Inner-membrane transporters are drawn with their cytosolic side in the
    // intermembrane band: inside the organelle's outline, but not of it.
    // Moved the ordinary way such a node is pushed out of the organelle and
    // left behind.
    let mut map = yeast();
    let (cell, org, _) = organelle(&map);
    let inner = map.cells()[0].compartments.iter().find(|k| k.parent_id.as_ref() == Some(&org.id)).unwrap().rect;
    let cytosolic = map
        .nodes()
        .values()
        .find(|n| n.compartment == org.parent_id && n.kind == NodeKind::Metabolite && n.reaction_id.as_deref().is_none_or(str::is_empty))
        .expect("a cytosolic metabolite to place")
        .id
        .clone();
    let before = Point::new((org.rect.x + inner.x) / 2.0, inner.y + inner.h / 2.0);
    map.edit_graph(|g| {
        let n = g.nodes.get_mut(&cytosolic).unwrap();
        n.x = before.x;
        n.y = before.y;
    });

    let selection = selectable_in_rect(&map, around(&org.rect, 4.0), true);
    assert!(selection.nodes.contains(&cytosolic));
    let moved = map.drag_selection(&selection, 12.0, 9.0).unwrap();
    assert!(moved.dx != 0.0 || moved.dy != 0.0);
    assert!(moved.nodes.contains(&cytosolic));
    let n = map.node(&cytosolic).unwrap();
    assert!(
        (n.x - before.x - moved.dx).abs() < 1e-9 && (n.y - before.y - moved.dy).abs() < 1e-9,
        "it went to {},{} instead of riding along from {},{} in {cell}",
        n.x,
        n.y,
        before.x,
        before.y
    );
}

/// JS: 'without compartments or cofactors a drag is one rigid move by the step it takes'
#[test]
fn without_compartments_or_cofactors_a_drag_is_one_rigid_move_by_the_step_it_takes() {
    let mut a = yeast();
    let mut b = yeast();
    let (_, org, _) = organelle(&a);
    let found = selectable_in_rect(&a, around(&org.rect, 4.0), true);
    let selection = Selection {
        nodes: found.nodes.iter().filter(|id| a.node(id).unwrap().reaction_id.as_deref().is_none_or(str::is_empty)).cloned().collect(),
        reactions: found.reactions,
        texts: found.texts,
        ..Default::default()
    };
    // Every member goes by the one step the whole selection can take; none is
    // held back, or pulled in, on its own.
    let step = a.drag_selection(&selection, 7.0, -5.0).unwrap();
    translate_selection_with(&mut b, &selection, step.dx, step.dy, true);
    assert_eq!(a.to_json(), b.to_json());
}

/// JS: 'reactions dragged into a wall stop there together, keeping their shape'
///
/// Held back by the wall one member at a time, the ones that reached it
/// would stop while the rest went on, and the curves between them would be
/// dragged across the map.
#[test]
fn reactions_dragged_into_a_wall_stop_there_together_keeping_their_shape() {
    let mut map = yeast();
    let cytosol = map.graph().cells[0].compartments.iter().find(|c| c.parent_id.is_none()).unwrap().rect;
    let (x, y) = (cytosol.x + cytosol.w * 0.05, cytosol.y + cytosol.h * 0.2);
    let block = SelectionRect { x1: x, y1: y, x2: x + cytosol.w * 0.35, y2: y + cytosol.h * 0.3 };
    let mut selection = selectable_in_rect(&map, block, true);
    selection.compartments.clear();
    assert!(selection.reactions.len() >= 5, "the block holds several reactions");
    map.drag_selection(&selection, -8.0, 0.0); // the first move settles every backbone
    let place = |map: &EditableMap| -> Vec<(String, Point)> {
        let nodes = selection.nodes.iter().map(|id| (id.clone(), map.node(id).unwrap().pos()));
        let reactions = selection.reactions.iter().map(|id| {
            let r = map.reaction(id).unwrap();
            (id.clone(), Point::new(r.x, r.y))
        });
        nodes.chain(reactions).collect()
    };
    let start = place(&map);
    let mut total = 0.0;
    for _ in 0..120 {
        total += map.drag_selection(&selection, -8.0, 0.0).unwrap().dx;
    }
    assert!(total > -960.0, "the wall stopped the selection");
    for ((id, p), (_, q)) in place(&map).iter().zip(&start) {
        assert!(
            (p.x - q.x - total).abs() < 1e-6 && (p.y - q.y).abs() < 1e-6,
            "{id} moved by {},{}, the selection by {total},0",
            p.x - q.x,
            p.y - q.y
        );
    }
}

#[test]
fn a_cofactor_selected_with_its_reaction_moves_once_with_its_reaction() {
    let mut map = yeast();
    let picked = map
        .reactions()
        .iter()
        .filter(|(_, r)| !r.on_membrane)
        .find_map(|(rid, _)| map.nodes().values().find(|n| n.reaction_id.as_deref() == Some(rid.as_str())).map(|n| (rid.clone(), n.id.clone())));
    let (rid, cofactor) = picked.expect("some reaction draws its own cofactor");
    let mut selection: Selection = empty_selection();
    selection.reactions.insert(rid.clone());
    selection.nodes.insert(cofactor.clone());

    let offset = |m: &EditableMap| {
        let (n, r) = (m.node(&cofactor).unwrap().pos(), m.reaction(&rid).unwrap());
        Point::new(n.x - r.x, n.y - r.y)
    };

    let mut port = map.clone();
    let start = offset(&map);
    map.drag_selection(&selection, 3.0, 2.0);
    let after = offset(&map);
    assert!(
        (after.x - start.x).abs() < 1e-9 && (after.y - start.y).abs() < 1e-9,
        "the cofactor drifted off its reaction: {start:?} -> {after:?}"
    );

    // translate_selection moves it, and then the reaction carries it: twice
    // the drag.
    port.translate_selection(&selection, 3.0, 2.0);
    let doubled = offset(&port);
    assert!((doubled.x - start.x).abs() > 1.0 || (doubled.y - start.y).abs() > 1.0);
}
