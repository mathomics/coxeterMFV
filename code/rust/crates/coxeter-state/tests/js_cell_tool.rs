//! Port of `test/cell-tool.test.mjs` against the real store
//! (`coxeter-map/tests/js_cell_tool.rs` re-expressed these tests over the map
//! alone). Every test is ported.
//!
//! "Cell boundary is rendered as an explicit selectable boundary" draws the
//! store's map with what the store hands the renderer
//! (`render_map_svg(map.graph(), &store.map_render_state(..))`); the JS hands
//! `renderMap` the store's map and scale.
//!
//! The JS reads `store.map.cells[0].compartments` once and keeps the objects,
//! which the edits then change in place; the Rust reads them again after each
//! edit.

mod common;

use std::sync::Arc;

use coxeter_core::boundaries::BoundaryKind;
use coxeter_core::community::{CommunityModel, CommunitySpec, MediumLimit, SpeciesSpec};
use coxeter_core::mapdata::Rect;
use coxeter_core::model::Model;
use coxeter_map::EditableMap;
use coxeter_render::mapview::render_map_svg;
use coxeter_render::theme::FluxScale;
use coxeter_state::{RenderOptions, Species, Store};

use common::{bundled_model, store};

/// `fixture()`: the model and a one-species community over it.
fn fixture() -> (Model, Arc<CommunityModel>) {
    let model = bundled_model("ecoli_core_mc");
    let community = CommunityModel::new(CommunitySpec {
        species: vec![SpeciesSpec::new("eco", Arc::new(model.clone())).abundance(1.0)],
        medium: [("glc__D", -10.0), ("o2", -20.0)]
            .into_iter()
            .map(|(base, lower)| (base.to_string(), MediumLimit { lower: Some(lower), upper: None }))
            .collect(),
        ..Default::default()
    })
    .unwrap();
    (model, Arc::new(community))
}

/// `storeFor(model, community)`: a blank 1000 × 700 map and the one species.
fn store_for(model: Model, community: &Arc<CommunityModel>) -> Store {
    let mut store = store();
    store.set_model("eco-file", model);
    store.species = vec![Species::new("eco", "eco-file", 1.0, true)];
    store.community = Some(community.clone());
    store.map = Some(EditableMap::blank(Some(1000.0), Some(700.0)));
    store.scale = Some(FluxScale::automatic([], false));
    store
}

fn map(store: &Store) -> &EditableMap {
    store.map.as_ref().expect("a map")
}

fn map_mut(store: &mut Store) -> &mut EditableMap {
    store.map.as_mut().expect("a map")
}

/// `cell.compartments.find((c) => c.id === id).rect`.
fn rect_of(store: &Store, compartment_id: &str) -> Rect {
    map(store).cells()[0].compartments.iter().find(|c| c.id == compartment_id).expect("the compartment").rect
}

/// "Cell creation draws the species outer extracellular boundary on a blank map"
#[test]
fn cell_creation_draws_the_species_outer_extracellular_boundary_on_a_blank_map() {
    let (model, community) = fixture();
    let mut store = store_for(model, &community);

    assert_eq!(store.available_map_cells().iter().map(|x| x.species_id.as_str()).collect::<Vec<_>>(), ["eco"]);
    assert!(store.available_map_compartments().is_empty());

    let result = store.add_map_cell("eco", Some(Rect::new(120.0, 100.0, 520.0, 360.0)));
    assert!(result.is_some());
    assert_eq!(map(&store).cells().len(), 1);

    let cell = &map(&store).cells()[0];
    assert_eq!(cell.id, "eco");
    assert_eq!(cell.outer_compartment.as_deref(), Some("e"));
    assert_eq!(cell.compartments.len(), 1);
    assert_eq!(cell.compartments[0].id, "e");
    assert_eq!(cell.rect, cell.compartments[0].rect);

    let boundary = map(&store).boundary("eco/e").expect("the cell boundary");
    assert_eq!(boundary.kind, BoundaryKind::Cell);
    assert_eq!(boundary.kind.as_str(), "cell");
    assert_eq!(boundary.inside, "e");
    assert_eq!(boundary.outside.as_deref(), Some("s"));

    assert!(store.available_map_cells().is_empty());
    assert_eq!(store.available_map_compartments().iter().map(|x| x.compartment_id.as_str()).collect::<Vec<_>>(), ["p"]);
}

/// "Cell boundary is rendered as an explicit selectable boundary"
#[test]
fn cell_boundary_is_rendered_as_an_explicit_selectable_boundary() {
    let (model, community) = fixture();
    let mut store = store_for(model, &community);
    store.add_map_cell("eco", Some(Rect::new(120.0, 100.0, 520.0, 360.0)));

    let svg = render_map_svg(map(&store).graph(), &store.map_render_state(RenderOptions::default()));

    assert!(svg.contains(r#"class="emc-cell-boundary""#));
    assert!(svg.contains(r#"data-cell-boundary="eco""#));
    assert!(svg.contains(r#"data-boundary="eco/e""#));
}

/// "resizing a Cell creates free space without scaling child compartments"
#[test]
fn resizing_a_cell_creates_free_space_without_scaling_child_compartments() {
    let (model, community) = fixture();
    let mut store = store_for(model, &community);
    store.add_map_cell("eco", Some(Rect::new(100.0, 90.0, 520.0, 360.0)));
    store.add_map_compartment("eco", "p", None);

    let outer = rect_of(&store, "e");
    let child_before = rect_of(&store, "p");

    let edited = map_mut(&mut store).edit_geometry_rect(Some("eco"), "e", Rect::new(outer.x, outer.y, outer.w + 180.0, outer.h + 120.0));

    assert!(edited.is_some());
    let outer = rect_of(&store, "e");
    assert_eq!(rect_of(&store, "p"), child_before);
    assert_eq!(map(&store).cells()[0].rect, outer);
    assert!(outer.w > 520.0);
    assert!(outer.h > 360.0);
}

/// "moving a Cell moves its child compartments by the same translation"
#[test]
fn moving_a_cell_moves_its_child_compartments_by_the_same_translation() {
    let (model, community) = fixture();
    let mut store = store_for(model, &community);
    store.add_map_cell("eco", Some(Rect::new(100.0, 90.0, 520.0, 360.0)));
    store.add_map_compartment("eco", "p", None);

    let outer = rect_of(&store, "e");
    let before = rect_of(&store, "p");

    map_mut(&mut store).edit_geometry_rect(Some("eco"), "e", Rect { x: outer.x + 40.0, y: outer.y + 25.0, ..outer });

    let child = rect_of(&store, "p");
    assert_eq!(child.x, before.x + 40.0);
    assert_eq!(child.y, before.y + 25.0);
    assert_eq!(child.w, before.w);
    assert_eq!(child.h, before.h);
}

/// "a child compartment cannot be resized beyond its parent boundary"
#[test]
fn a_child_compartment_cannot_be_resized_beyond_its_parent_boundary() {
    let (model, community) = fixture();
    let mut store = store_for(model, &community);
    store.add_map_cell("eco", Some(Rect::new(100.0, 90.0, 520.0, 360.0)));
    store.add_map_compartment("eco", "p", None);

    let outer = rect_of(&store, "e");

    map_mut(&mut store).edit_geometry_rect(Some("eco"), "p", Rect::new(outer.x - 500.0, outer.y - 500.0, outer.w * 4.0, outer.h * 4.0));

    let outer = rect_of(&store, "e");
    let p = rect_of(&store, "p");
    assert!(p.x >= outer.x + 10.0 - 1e-6);
    assert!(p.y >= outer.y + 10.0 - 1e-6);
    assert!(p.x + p.w <= outer.x + outer.w - 10.0 + 1e-6);
    assert!(p.y + p.h <= outer.y + outer.h - 10.0 + 1e-6);
}

/// "Cell creation is map-only and does not mutate the biological model or solver"
#[test]
fn cell_creation_is_map_only_and_does_not_mutate_the_biological_model_or_solver() {
    let (model, community) = fixture();
    let mut store = store_for(model, &community);
    let reactions = community.merged.reactions.len();
    let metabolites = community.merged.metabolites.len();

    store.add_map_cell("eco", Some(Rect::new(100.0, 90.0, 520.0, 360.0)));

    // The store still holds that very community, unchanged.
    let held = store.community.as_ref().expect("the community");
    assert!(Arc::ptr_eq(held, &community), "the store's community is the one it was given");
    assert_eq!(held.merged.reactions.len(), reactions);
    assert_eq!(held.merged.metabolites.len(), metabolites);
    assert!(store.models.get("eco-file").unwrap().compartment_ids().iter().any(|id| id == "e"));
}
