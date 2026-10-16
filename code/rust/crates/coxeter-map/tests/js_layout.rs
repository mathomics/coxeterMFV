//! Port of `test/layout.test.mjs`: the geometry helpers and the automatic
//! layout of a community.
//!
//! Not ported: "the renderer emits well-formed SVG containing the whole community"
//!   — it solves the community (`FluxProblem`, crate `coxeter-solver`) and
//!   asserts on the SVG string of `renderMap` (crate `coxeter-render`).
//! Not ported: "flux scale maps magnitude monotonically to width" — `FluxScale`
//!   is `src/render/theme.js`, the render crate's.

mod common;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use coxeter_core::community::{CommunityModel, CommunitySpec, MediumLimit, SpeciesSpec};
use coxeter_core::geometry::{
    closest_perimeter_point, compartment_corner_radius, grid_cells, inset_all, perimeter_point, signed_distance,
};
use coxeter_core::js;
use coxeter_core::mapdata::{MapCompartment, NodeKind, Point, Rect};
use coxeter_map::layout::{Layout, LayoutOptions, layout_community};

use common::load_model;

/// The JS `buildLayout(names)`: a community of the bundled models on the
/// test medium, and its automatic layout at the default options.
fn build_layout(names: &[(&str, &str)]) -> (CommunityModel, Layout) {
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
    let layout = layout_community(&com, &LayoutOptions::default());
    (com, layout)
}

/// `assert.deepEqual` on two numbers (strict: `Object.is`).
fn same(a: f64, b: f64) -> bool {
    (a.is_nan() && b.is_nan()) || a.to_bits() == b.to_bits()
}

/// JS: `geometry helpers behave`
#[test]
fn geometry_helpers_behave() {
    let r = Rect::new(0.0, 0.0, 100.0, 50.0);
    let p = perimeter_point(&r, 0.0);
    assert!(same(p.x, 50.0) && same(p.y, 0.0) && same(p.nx, 0.0) && same(p.ny, -1.0), "{p:?}");
    assert_eq!(closest_perimeter_point(&r, Point::new(200.0, 25.0), 0.0).x, 100.0);
    assert!(signed_distance(&r, Point::new(50.0, 25.0)) < 0.0, "inside is negative");
    assert!(signed_distance(&r, Point::new(150.0, 25.0)) > 0.0, "outside is positive");
    // Four items on a wide canvas should come out 2x2, not 3 + 1.
    // (`gridCells(r, 4, 40)`: the target aspect takes its default, 1.08.)
    let cells = grid_cells(&Rect::new(0.0, 0.0, 1600.0, 1000.0), 4, 40.0, 1.08);
    let ys: HashSet<u64> = cells.iter().map(|c| js::round(c.y).to_bits()).collect();
    assert_eq!(ys.len(), 2, "expected two rows");
}

/// JS: `compartments nest strictly inside their parent`
#[test]
fn compartments_nest_strictly_inside_their_parent() {
    let (_, layout) = build_layout(&[("eco", "ecoli_core_mc"), ("sce", "yeast_core_mc")]);
    for cell in &layout.cells {
        let by_id: HashMap<&str, &MapCompartment> = cell.compartments.iter().map(|c| (c.id.as_str(), c)).collect();
        for comp in &cell.compartments {
            // `if (!comp.parentId) continue;`
            let Some(parent_id) = comp.parent_id.as_deref().filter(|p| !p.is_empty()) else { continue };
            let parent = by_id.get(parent_id).unwrap_or_else(|| panic!("{} has no parent rect", comp.id));
            assert!(comp.rect.x >= parent.rect.x - 0.5, "{} escapes left of {}", comp.id, parent.id);
            assert!(comp.rect.y >= parent.rect.y - 0.5, "{} escapes above {}", comp.id, parent.id);
            assert!(
                comp.rect.x + comp.rect.w <= parent.rect.x + parent.rect.w + 0.5,
                "{} escapes right of {}",
                comp.id,
                parent.id
            );
            assert!(
                comp.rect.y + comp.rect.h <= parent.rect.y + parent.rect.h + 0.5,
                "{} escapes below {}",
                comp.id,
                parent.id
            );
            assert!(comp.rect.w > 20.0 && comp.rect.h > 20.0, "{} collapsed to nothing", comp.id);
        }
    }
}

/// JS: `cells do not overlap each other`
#[test]
fn cells_do_not_overlap_each_other() {
    let (_, layout) = build_layout(&[
        ("eco", "ecoli_core_mc"),
        ("sce", "yeast_core_mc"),
        ("mba", "methanogen_core_mc"),
        ("lla", "lactobacillus_core_mc"),
    ]);
    let rects: Vec<Rect> = layout.cells.iter().map(|c| c.rect).collect();
    for i in 0..rects.len() {
        for j in (i + 1)..rects.len() {
            let a = rects[i];
            let b = rects[j];
            let overlap = a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
            assert!(!overlap, "cells {i} and {j} overlap");
        }
    }
}

/// JS: `every metabolite node sits inside the compartment it belongs to`
#[test]
fn every_metabolite_node_sits_inside_the_compartment_it_belongs_to() {
    let (_, layout) = build_layout(&[("eco", "ecoli_core_mc"), ("sce", "yeast_core_mc")]);
    let mut rects: HashMap<String, Rect> = HashMap::new();
    for cell in &layout.cells {
        for comp in &cell.compartments {
            rects.insert(format!("{}/{}", cell.id, comp.id), comp.rect);
        }
    }
    let mut checked = 0;
    for node in layout.nodes.values() {
        if node.kind != NodeKind::Metabolite {
            continue;
        }
        let key = format!(
            "{}/{}",
            node.cell_id.as_deref().unwrap_or("null"),
            node.compartment.as_deref().unwrap_or("undefined")
        );
        let r = rects.get(&key).unwrap_or_else(|| panic!("no rect for {key}"));
        assert!(
            signed_distance(r, node.pos()) < 1.0,
            "{} outside {}",
            node.merged_id.as_deref().unwrap_or("?"),
            node.compartment.as_deref().unwrap_or("?")
        );
        checked += 1;
    }
    assert!(checked > 50, "only checked {checked} nodes");
}

/// JS: `medium metabolites stay outside every cell`
#[test]
fn medium_metabolites_stay_outside_every_cell() {
    let (_, layout) = build_layout(&[("eco", "ecoli_core_mc"), ("sce", "yeast_core_mc")]);
    let mut checked = 0;
    for node in layout.nodes.values() {
        if node.kind != NodeKind::Medium {
            continue;
        }
        for cell in &layout.cells {
            assert!(
                signed_distance(&cell.rect, node.pos()) > -1.0,
                "medium node {} sits inside cell {}",
                node.base.as_deref().unwrap_or("?"),
                cell.id
            );
        }
        checked += 1;
    }
    assert!(checked > 3, "only checked {checked} medium nodes");
}

/// JS: `transport reactions land on the membrane they cross`
#[test]
fn transport_reactions_land_on_the_membrane_they_cross() {
    let (_, layout) = build_layout(&[("sce", "yeast_core_mc")]);
    let cell = &layout.cells[0];
    let by_id: HashMap<&str, &MapCompartment> = cell.compartments.iter().map(|c| (c.id.as_str(), c)).collect();
    let mut checked = 0;
    for rxn in layout.reactions.values() {
        if !rxn.on_membrane {
            continue;
        }
        let comp = rxn
            .membrane
            .as_deref()
            .and_then(|m| by_id.get(m))
            .unwrap_or_else(|| panic!("{} on unknown membrane {:?}", rxn.id, rxn.membrane));
        let radius = compartment_corner_radius(comp);
        let inner = inset_all(&comp.rect, radius);
        let distance = (signed_distance(&inner, Point::new(rxn.x, rxn.y)) - radius).abs();
        assert!(
            distance < 2.0,
            "{} is {}px off its membrane",
            rxn.local_id.as_deref().unwrap_or(&rxn.id),
            js::to_fixed(distance, 1)
        );
        checked += 1;
    }
    assert!(checked > 5, "only checked {checked} transporters");
}

/// JS: `every reaction in the merged model is drawn exactly once`
#[test]
fn every_reaction_in_the_merged_model_is_drawn_exactly_once() {
    let (com, layout) = build_layout(&[("eco", "ecoli_core_mc"), ("mba", "methanogen_core_mc")]);
    // The layout's reactions are keyed by id (a JS `Map`), so each is drawn at
    // most once; these are the JS `new Set(layout.reactions.keys())`.
    let drawn: HashSet<&str> = layout.reactions.keys().map(String::as_str).collect();
    for rid in com.merged.reactions.keys() {
        if rid == "COMMUNITY_GROWTH" {
            continue; // a virtual coupling variable
        }
        assert!(drawn.contains(rid.as_str()), "{rid} is missing from the map");
    }
    // And nothing is invented.
    for rid in &drawn {
        assert!(com.merged.reactions.contains_key(*rid), "{rid} is not in the model");
    }
}

/// JS: `the layout is deterministic`
#[test]
fn the_layout_is_deterministic() {
    let a = build_layout(&[("eco", "ecoli_core_mc")]);
    let b = build_layout(&[("eco", "ecoli_core_mc")]);
    let pick = |l: &(CommunityModel, Layout)| -> Vec<String> {
        l.1.nodes.values().take(40).map(|n| format!("{}:{},{}", n.id, js::to_fixed(n.x, 4), js::to_fixed(n.y, 4))).collect()
    };
    assert_eq!(pick(&a), pick(&b));
}
