//! Port of the map halves of `test/compartment-shape.test.mjs` and
//! `test/compartment-delete.test.mjs`: shaping a compartment (the two shape
//! handles and `editCompartmentShape`) and taking one off the drawing
//! (`removeCompartment`).
//!
//! The rendering half (the diamonds and the elliptical outline) is in
//! `coxeter-render`, the Store half (presets, the compartment list, undo) in
//! `coxeter-state`.

mod common;

use std::collections::HashSet;
use std::sync::Arc;

use common::load_model;
use coxeter_core::community::{CommunityModel, CommunitySpec, SpeciesSpec};
use coxeter_core::geometry::compartment_shape;
use coxeter_core::js;
use coxeter_core::mapdata::{Point, Rect};
use coxeter_map::editable_map::TextOptions;
use coxeter_map::geometry_edit::{ShapePatch, shape_amount_at, shape_handle_point};
use coxeter_map::{EditableMap, LayoutOptions, PlaceOptions};

fn yeast() -> CommunityModel {
    CommunityModel::new(CommunitySpec {
        species: vec![SpeciesSpec::new("yeast", Arc::new(load_model("yeast_core_mc"))).abundance(1.0)],
        ..Default::default()
    })
    .unwrap()
}

/// How far a point is from the ellipse inside `rect`, in units of the ellipse: 0 on it.
fn on_ellipse(rect: &Rect, p: Point) -> f64 {
    js::hypot((p.x - (rect.x + rect.w / 2.0)) / (rect.w / 2.0), (p.y - (rect.y + rect.h / 2.0)) / (rect.h / 2.0)) - 1.0
}

/// JS: "reshaping a compartment moves its transporters onto the new outline, turned to face out"
#[test]
fn reshaping_a_compartment_moves_its_transporters_onto_the_new_outline() {
    let community = yeast();
    let mut map = EditableMap::empty_from_community(&community, &LayoutOptions::default());
    let cell = map.cells()[0].id.clone();
    let m = map.cells()[0].compartments.iter().find(|c| c.id == "m").unwrap().clone();
    let rect = m.rect;
    let id = "yeast@PYRt2m";
    let opts = PlaceOptions { boundary_id: Some(format!("{cell}/m")), anchor: Some(Point::new(rect.x, rect.y)), ..Default::default() };
    assert!(map.add_reaction(&community, id, &opts).is_some());
    let owned = |map: &EditableMap| {
        let rxn = map.reaction(id).unwrap();
        map.nodes().values().filter(|n| n.reaction_id.as_deref() == Some(id)).map(|n| js::hypot(n.x - rxn.x, n.y - rxn.y)).collect::<Vec<_>>()
    };
    let reach = owned(&map);
    let others = |map: &EditableMap| map.nodes().values().filter(|n| n.reaction_id.is_none()).cloned().collect::<Vec<_>>();
    let before = others(&map);

    // Only the amount given changes; the corner it had stays.
    let corner = compartment_shape(&m).corner;
    let half = map.edit_compartment_shape(Some(&cell), "m", ShapePatch { oval: Some(0.5), corner: None }).unwrap();
    assert_eq!((half.shape.corner, half.shape.oval), (corner, 0.5));
    let result = map.edit_compartment_shape(Some(&cell), "m", ShapePatch { corner: Some(1.0), oval: Some(1.0) }).unwrap();
    assert_eq!((result.shape.corner, result.shape.oval), (1.0, 1.0));
    let now = map.cells()[0].compartments.iter().find(|c| c.id == "m").unwrap().clone();
    assert_eq!(now.rect, rect, "the rectangle stays");
    let rxn = map.reaction(id).unwrap().clone();
    assert!(on_ellipse(&rect, Point::new(rxn.x, rxn.y)).abs() < 1e-9, "the transporter sits on the ellipse");
    let gx = (rxn.x - (rect.x + rect.w / 2.0)) / (rect.w / 2.0).powi(2);
    let gy = (rxn.y - (rect.y + rect.h / 2.0)) / (rect.h / 2.0).powi(2);
    let normal = rxn.normal.unwrap();
    assert!((normal.x * gy - normal.y * gx).abs() < 1e-9, "it faces out of the ellipse");
    for (d, r) in owned(&map).iter().zip(&reach) {
        assert!((d - r).abs() < 1e-6, "its metabolites came along");
    }
    assert_eq!(others(&map), before);

    // Saved and opened again, it keeps its shape.
    let restored = EditableMap::from_json(&map.to_json()).unwrap();
    assert_eq!(restored.cells()[0].compartments.iter().find(|c| c.id == "m").unwrap().shape, now.shape);

    // The shared medium has no shape to edit.
    let env = map.environment().id.clone();
    assert!(map.edit_compartment_shape(None, &env, ShapePatch { oval: Some(1.0), corner: None }).is_none());
}

/// JS: "a shape handle stands on its edge where its amount says, clear of the other handles"
#[test]
fn a_shape_handle_stands_on_its_edge_where_its_amount_says() {
    let rect = Rect::new(0.0, 0.0, 800.0, 500.0);
    let side = 24.0;
    for which in ["corner", "oval"] {
        for amount in [0.0, 0.25, 0.5, 1.0] {
            let p = shape_handle_point(&rect, which, amount, side);
            assert!((shape_amount_at(&rect, which, p, side) - amount).abs() < 1e-9);
        }
        let along = |p: Point| if which == "corner" { p.x } else { p.y };
        let across = |p: Point| if which == "corner" { p.y } else { p.x };
        let half = (if which == "corner" { rect.w } else { rect.h }) / 2.0;
        assert_eq!(across(shape_handle_point(&rect, which, 0.5, side)), 0.0, "on the edge");
        assert!(along(shape_handle_point(&rect, which, 0.0, side)) >= side * 1.2 - 1e-9, "clear of the corner handle");
        assert!(along(shape_handle_point(&rect, which, 1.0, side)) <= half - side * 1.6 + 1e-9, "clear of the side handle");
    }
    // Dragged past either end, it stops there.
    assert_eq!(shape_amount_at(&rect, "corner", Point::new(-50.0, 0.0), side), 0.0);
    assert_eq!(shape_amount_at(&rect, "oval", Point::new(0.0, 900.0), side), 1.0);
}

/// JS: "deleting a compartment takes what is nested in it and drawn there, and nothing else"
/// (the map's part; the Store's is in `coxeter-state`).
#[test]
fn removing_a_compartment_takes_what_is_nested_and_drawn_in_it() {
    let community = yeast();
    let mut map = EditableMap::from_community(&community, &LayoutOptions::default());
    let cell = map.cells()[0].id.clone();
    let inside: HashSet<&str> = HashSet::from(["im", "m"]);
    let touches = |r: &coxeter_core::mapdata::MapReaction| {
        r.compartments.iter().any(|c| inside.contains(c.as_str())) || r.membrane.as_deref().is_some_and(|m| inside.contains(m))
    };
    let doomed: Vec<String> = map.reactions().values().filter(|r| touches(r)).map(|r| r.id.clone()).collect();
    let kept: Vec<String> = map.reactions().values().filter(|r| !touches(r)).map(|r| r.id.clone()).collect();
    assert!(!doomed.is_empty() && !kept.is_empty());
    let im = map.cells()[0].compartments.iter().find(|c| c.id == "im").unwrap().rect;
    let c = map.cells()[0].compartments.iter().find(|k| k.id == "c").unwrap().rect;
    let tca = map.add_text("TCA cycle", im.x + im.w / 2.0, im.y + im.h / 2.0, &TextOptions::default()).unwrap().id.clone();
    let glycolysis = map.add_text("Glycolysis", c.x + 20.0, c.y + 20.0, &TextOptions::default()).unwrap().id.clone();

    let result = map.remove_compartment(&cell, "im").unwrap();
    let mut ids = result.compartment_ids.clone();
    ids.sort();
    assert_eq!(ids, ["im", "m"]);
    assert!(!result.cell_removed);
    assert!(map.cells()[0].compartments.iter().all(|c| !inside.contains(c.id.as_str())));
    assert!(doomed.iter().all(|id| !map.has_reaction(id)), "its reactions are gone");
    assert!(kept.iter().all(|id| map.has_reaction(id)), "the rest are all still there");
    assert!(map.nodes().values().all(|n| !n.compartment.as_deref().is_some_and(|c| inside.contains(c))), "and its metabolites");
    assert!(map.segments().iter().all(|s| s.node_id.as_deref().is_none_or(|n| map.has_node(n))), "no curve points at a removed metabolite");
    assert!(!map.boundaries().iter().any(|b| b.cell_id.as_deref() == Some(cell.as_str()) && inside.contains(b.inside.as_str())), "its membranes are gone");
    assert_eq!(result.texts, [tca.clone()], "the name written inside it went with it");
    assert!(map.text(&glycolysis).is_some(), "the one outside stayed");

    // Its outermost compartment is the cell.
    let outer = map.cells()[0].compartments.iter().find(|c| c.parent_id.is_none()).unwrap().id.clone();
    let result = map.remove_compartment(&cell, &outer).unwrap();
    assert!(result.cell_removed);
    assert!(map.cells().is_empty());
    assert!(map.reactions().values().all(|r| r.cell_id.as_deref() != Some(cell.as_str())));

    // The shared medium is not a compartment that can be deleted.
    let env = map.environment().id.clone();
    assert!(map.remove_compartment("", &env).is_none());
}
