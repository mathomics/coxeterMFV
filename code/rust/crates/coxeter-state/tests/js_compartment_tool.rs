//! Port of the Store tests of `test/compartment-tool.test.mjs` against the
//! real store (`coxeter-map/tests/js_compartment_tool.rs` re-expressed them
//! over the map alone).
//!
//! Ported in coxeter-map (no Store): "custom compartment_tree governs ER-like
//! nested compartments without hardcoding names" — `Model.compartmentTree()`
//! only.
//!
//! The JS reads `store.map.cells[0].compartments` once and keeps the objects,
//! which the edits then change in place; the Rust reads them again after each
//! edit.

mod common;

use std::sync::Arc;

use coxeter_core::community::{CommunityModel, CommunitySpec, SpeciesSpec};
use coxeter_core::mapdata::{MapCompartment, Rect};
use coxeter_map::EditableMap;
use coxeter_render::theme::FluxScale;
use coxeter_state::{AvailableCompartment, Species, Store};

use common::{bundled_model, store};

/// `loaded(name, speciesId)`: a blank 1200 × 850 map and the one species,
/// over a one-species community with no medium.
fn loaded(name: &str, species_id: &str) -> Store {
    let model = bundled_model(name);
    let community = CommunityModel::new(CommunitySpec {
        species: vec![SpeciesSpec::new(species_id, Arc::new(model.clone())).abundance(1.0)],
        ..Default::default()
    })
    .unwrap();
    let mut store = store();
    store.set_model("fixture", model);
    store.species = vec![Species::new(species_id, "fixture", 1.0, true)];
    store.community = Some(Arc::new(community));
    store.map = Some(EditableMap::blank(Some(1200.0), Some(850.0)));
    store.scale = Some(FluxScale::automatic([], false));
    store
}

/// `[compartmentId, parentId]` of each entry.
fn id_and_parent(list: &[AvailableCompartment]) -> Vec<(&str, Option<&str>)> {
    list.iter().map(|x| (x.compartment_id.as_str(), x.parent_id.as_deref())).collect()
}

/// `store.map.cells[0].compartments.find((x) => x.id === id)`.
fn compartment<'a>(store: &'a Store, id: &str) -> &'a MapCompartment {
    store.map.as_ref().expect("a map").cells()[0].compartments.iter().find(|x| x.id == id).expect("the compartment")
}

/// "E. coli compartments become drawable in biological containment order e -> p -> c"
#[test]
fn e_coli_compartments_become_drawable_in_biological_containment_order_e_p_c() {
    let mut store = loaded("ecoli_core_mc", "eco");
    store.add_map_cell("eco", Some(Rect::new(80.0, 70.0, 760.0, 560.0)));

    let available = store.available_map_compartments();
    assert_eq!(id_and_parent(&available), [("p", Some("e"))]);

    let p = store.add_map_compartment("eco", "p", Some(Rect::new(130.0, 120.0, 650.0, 450.0)));
    assert!(p.is_some());
    let available = store.available_map_compartments();
    assert_eq!(id_and_parent(&available), [("c", Some("p"))]);

    let c = store.add_map_compartment("eco", "c", Some(Rect::new(175.0, 165.0, 540.0, 350.0)));
    assert!(c.is_some());

    let periplasm = compartment(&store, "p").rect;
    let cytosol = compartment(&store, "c");
    assert_eq!(cytosol.parent_id.as_deref(), Some("p"));
    let cytosol = cytosol.rect;
    assert!(cytosol.x > periplasm.x);
    assert!(cytosol.y > periplasm.y);
    assert!(cytosol.x + cytosol.w < periplasm.x + periplasm.w);
    assert!(cytosol.y + cytosol.h < periplasm.y + periplasm.h);
}

/// "yeast mitochondrial intermembrane space surrounds mitochondrial matrix when both exist"
#[test]
fn yeast_mitochondrial_intermembrane_space_surrounds_mitochondrial_matrix_when_both_exist() {
    let mut store = loaded("yeast_core_mc", "yeast");
    store.add_map_cell("yeast", Some(Rect::new(80.0, 70.0, 900.0, 650.0)));

    assert_eq!(id_and_parent(&store.available_map_compartments()), [("c", Some("e"))]);

    store.add_map_compartment("yeast", "c", Some(Rect::new(130.0, 120.0, 800.0, 540.0)));
    let after_c = store.available_map_compartments();
    let im_spec = after_c.iter().find(|x| x.compartment_id == "im").expect("im is offered");
    assert_eq!(im_spec.parent_id.as_deref(), Some("c"));

    store.add_map_compartment("yeast", "im", Some(Rect::new(250.0, 220.0, 360.0, 260.0)));
    let after_im = store.available_map_compartments();
    let m_spec = after_im.iter().find(|x| x.compartment_id == "m").expect("m is offered");
    assert_eq!(m_spec.parent_id.as_deref(), Some("im"));

    store.add_map_compartment("yeast", "m", Some(Rect::new(285.0, 255.0, 285.0, 190.0)));
    let im = compartment(&store, "im").rect;
    let m = compartment(&store, "m");

    assert_eq!(m.parent_id.as_deref(), Some("im"));
    let m = m.rect;
    assert!(m.x > im.x);
    assert!(m.y > im.y);
    assert!(m.x + m.w < im.x + im.w);
    assert!(m.y + m.h < im.y + im.h);
}

/// "a requested compartment rectangle is clamped inside its biological parent"
#[test]
fn a_requested_compartment_rectangle_is_clamped_inside_its_biological_parent() {
    let mut store = loaded("ecoli_core_mc", "eco");
    store.add_map_cell("eco", Some(Rect::new(100.0, 100.0, 520.0, 360.0)));

    let result = store.add_map_compartment("eco", "p", Some(Rect::new(-500.0, -400.0, 5000.0, 4000.0)));
    assert!(result.is_some());

    let e = compartment(&store, "e").rect;
    let p = compartment(&store, "p").rect;
    assert!(p.x > e.x);
    assert!(p.y > e.y);
    assert!(p.x + p.w < e.x + e.w);
    assert!(p.y + p.h < e.y + e.h);
}

/// "a compartment cannot be added before its biological parent exists"
#[test]
fn a_compartment_cannot_be_added_before_its_biological_parent_exists() {
    let mut store = loaded("yeast_core_mc", "yeast");
    store.add_map_cell("yeast", Some(Rect::new(80.0, 70.0, 900.0, 650.0)));

    assert!(store.add_map_compartment("yeast", "m", Some(Rect::new(200.0, 200.0, 200.0, 150.0))).is_none());
    assert!(store.add_map_compartment("yeast", "im", Some(Rect::new(200.0, 200.0, 250.0, 180.0))).is_none());
}

/// "editing a child compartment still cannot cross its parent after creation"
#[test]
fn editing_a_child_compartment_still_cannot_cross_its_parent_after_creation() {
    let mut store = loaded("ecoli_core_mc", "eco");
    store.add_map_cell("eco", Some(Rect::new(100.0, 100.0, 700.0, 500.0)));
    store.add_map_compartment("eco", "p", Some(Rect::new(140.0, 140.0, 600.0, 400.0)));
    store.add_map_compartment("eco", "c", Some(Rect::new(190.0, 190.0, 480.0, 290.0)));

    let p = compartment(&store, "p").rect;

    store.map.as_mut().unwrap().edit_geometry_rect(Some("eco"), "c", Rect::new(p.x - 1000.0, p.y - 1000.0, p.w * 5.0, p.h * 5.0));

    let p = compartment(&store, "p").rect;
    let c = compartment(&store, "c").rect;
    assert!(c.x >= p.x + 10.0 - 1e-6);
    assert!(c.y >= p.y + 10.0 - 1e-6);
    assert!(c.x + c.w <= p.x + p.w - 10.0 + 1e-6);
    assert!(c.y + c.h <= p.y + p.h - 10.0 + 1e-6);
}
