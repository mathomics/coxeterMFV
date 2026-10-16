//! Port of the Store tests of `test/blank-map-canvas.test.mjs`, against the
//! real store (`coxeter-map/tests/js_blank_map_canvas.rs` re-expressed these
//! two over the map alone).
//!
//! Ported in coxeter-map (`EditableMap` only, no Store): "blank map contains
//! only a drawing canvas and no biological geometry", "adding map
//! compartments does not change the biological model", "canvas can grow and
//! cannot shrink through existing content" and "blank-map canvas size and
//! manually added compartments survive JSON round trip".
//! Not ported: "blank map renderer shows the white canvas but no environment
//! or compartments" — the renderer alone on `EditableMap.blank`, no Store
//! (crate `coxeter-render`).

mod common;

use std::sync::Arc;

use coxeter_core::community::{CommunityModel, CommunitySpec, MediumLimit, SpeciesSpec};
use coxeter_core::mapdata::Rect;
use coxeter_core::model::Model;
use coxeter_map::EditableMap;
use coxeter_map::layout::LayoutOptions;
use coxeter_render::theme::FluxScale;
use coxeter_state::{AvailableCompartment, Species, Store};

use common::{bundled_model, store};

/// `ecoli()`: the model and a one-species community over it.
fn ecoli() -> (Model, Arc<CommunityModel>) {
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

fn map(store: &Store) -> &EditableMap {
    store.map.as_ref().expect("a map")
}

fn compartment_ids(list: &[AvailableCompartment]) -> Vec<&str> {
    list.iter().map(|x| x.compartment_id.as_str()).collect()
}

/// "showEmptyMap now produces a truly blank map rather than the structural shell"
#[test]
fn show_empty_map_now_produces_a_truly_blank_map_rather_than_the_structural_shell() {
    let (_, community) = ecoli();
    let mut store = store();
    store.map = Some(EditableMap::from_community(&community, &LayoutOptions::default()));
    store.community = Some(community);
    store.scale = Some(FluxScale::automatic([], false));
    let width = map(&store).width();
    let height = map(&store).height();

    store.show_empty_map();

    assert_eq!(map(&store).cells().len(), 0);
    assert_eq!(map(&store).nodes().len(), 0);
    assert_eq!(map(&store).reactions().len(), 0);
    assert_eq!(map(&store).environment().hidden, Some(true));
    assert_eq!(map(&store).width(), width);
    assert_eq!(map(&store).height(), height);
}

/// "manual structure follows the biological compartment hierarchy"
#[test]
fn manual_structure_follows_the_biological_compartment_hierarchy() {
    let (model, community) = ecoli();
    let mut store = store();
    store.set_model("eco-file", model);
    store.species = vec![Species::new("eco", "eco-file", 1.0, true)];
    store.community = Some(community);
    store.map = Some(EditableMap::blank(Some(1000.0), Some(700.0)));

    let available = store.available_map_compartments();
    assert!(available.is_empty());
    assert_eq!(store.available_map_cells().iter().map(|x| x.species_id.as_str()).collect::<Vec<_>>(), ["eco"]);

    let cell = store.add_map_cell("eco", Some(Rect::new(80.0, 70.0, 520.0, 360.0)));
    assert!(cell.is_some());
    assert_eq!(map(&store).cells().len(), 1);
    assert_eq!(map(&store).cells()[0].compartments.len(), 1);
    assert_eq!(map(&store).cells()[0].compartments[0].id, "e");

    let available = store.available_map_compartments();
    assert_eq!(compartment_ids(&available), ["p"]);

    let p = store.add_map_compartment("eco", "p", None).expect("the periplasm");
    assert_eq!(p.compartment.parent_id.as_deref(), Some("e"));

    let available = store.available_map_compartments();
    assert_eq!(compartment_ids(&available), ["c"]);

    let c = store.add_map_compartment("eco", "c", None).expect("the cytosol");
    assert_eq!(c.compartment.parent_id.as_deref(), Some("p"));
    assert_eq!(store.available_map_compartments().len(), 0);
}
