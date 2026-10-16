//! Port of the Store tests of `test/compartment-delete.test.mjs` and
//! `test/compartment-shape.test.mjs` (the map and render halves are in
//! `coxeter-map` and `coxeter-render`).
//!
//! Not ported here: "Delete removes the selected compartment or cell, read
//! from what is selected", a check of the source of `app.js` (the native
//! Delete key is tested in `coxeter-app/tests/keys.rs`).

mod common;

use std::sync::Arc;

use coxeter_core::community::{CommunityModel, CommunitySpec, SpeciesSpec};
use coxeter_core::geometry::{compartment_shape, compartment_shape_preset};
use coxeter_core::mapdata::Rect;
use coxeter_map::geometry_edit::ShapePatch;
use coxeter_map::multi_selection::empty_selection;
use coxeter_map::{EditableMap, LayoutOptions};
use coxeter_render::theme::FluxScale;
use coxeter_state::{Species, Store};

use common::{bundled_model, store};

/// The yeast core model over one species; `full` lays it out in full,
/// otherwise a blank map with the yeast cell drawn by hand.
fn yeast_store(full: bool) -> Store {
    let model = bundled_model("yeast_core_mc");
    let community = CommunityModel::new(CommunitySpec {
        species: vec![SpeciesSpec::new("yeast", Arc::new(model.clone())).abundance(1.0)],
        ..Default::default()
    })
    .unwrap();
    let mut store = store();
    store.set_model("fixture", model);
    store.species = vec![Species::new("yeast", "fixture", 1.0, true)];
    store.map = Some(if full {
        EditableMap::from_community(&community, &LayoutOptions::default())
    } else {
        EditableMap::blank(Some(1200.0), Some(850.0))
    });
    store.community = Some(Arc::new(community));
    store.scale = Some(FluxScale::automatic([], false));
    if !full {
        store.add_map_cell("yeast", Some(Rect::new(80.0, 70.0, 900.0, 650.0)));
    }
    store
}

fn ids(store: &Store) -> Vec<String> {
    store.map.as_ref().unwrap().cells()[0].compartments.iter().map(|c| c.id.clone()).collect()
}

/// JS: "deleting a compartment takes what is nested in it and drawn there, and nothing else"
#[test]
fn deleting_a_compartment_takes_what_is_nested_in_it_and_is_one_undo_step() {
    let mut store = yeast_store(true);
    let cell = store.map.as_ref().unwrap().cells()[0].id.clone();
    assert!(["im", "m"].iter().all(|id| ids(&store).contains(&id.to_string())), "the mitochondrion is drawn");
    let im = store.map.as_ref().unwrap().cells()[0].compartments.iter().find(|c| c.id == "im").unwrap().rect;
    store.add_map_text("TCA cycle", im.x + im.w / 2.0, im.y + im.h / 2.0, None);
    let json = store.map.as_ref().unwrap().to_json();

    store.select_geometry(Some(&cell), "im");
    let result = store.remove_map_compartment(&cell, "im").unwrap();
    let mut removed = result.compartment_ids.clone();
    removed.sort();
    assert_eq!(removed, ["im", "m"]);
    assert!(!result.cell_removed);
    assert!(ids(&store).iter().all(|id| id != "im" && id != "m"));
    assert_eq!(store.selected_geometry, None);
    assert!(store.selected_entity.is_none());
    let message = &store.status().message;
    let pattern = regex::Regex::new(r"^Removed .+ from the map, with 1 nested compartment, \d+ reactions?, \d+ metabolites?, 1 text\. The model is unchanged").unwrap();
    assert!(pattern.is_match(message), "{message}");

    // It can be drawn again, and Ctrl+Z puts everything back as it was.
    assert!(store.available_map_compartments().iter().any(|c| c.compartment_id == "im"));
    store.undo_map();
    assert_eq!(store.map.as_ref().unwrap().to_json(), json);
}

/// JS: "deleting a cell's outermost compartment deletes the cell"
#[test]
fn deleting_a_cells_outermost_compartment_deletes_the_cell() {
    let mut store = yeast_store(true);
    let cell = store.map.as_ref().unwrap().cells()[0].clone();
    let outer = cell.compartments.iter().find(|c| c.parent_id.is_none()).unwrap().id.clone();
    let result = store.remove_map_compartment(&cell.id, &outer).unwrap();
    assert!(result.cell_removed);
    assert!(store.map.as_ref().unwrap().cells().is_empty());
    assert!(store.status().message.starts_with("Removed the cell "));
    assert!(store.available_map_cells().iter().any(|c| c.species_id == cell.id), "it can be drawn again");
    store.undo_map();
    assert_eq!(store.map.as_ref().unwrap().cells().len(), 1);
}

/// JS: "the shared medium is not a compartment that can be deleted"
#[test]
fn the_shared_medium_is_not_a_compartment_that_can_be_deleted() {
    let mut store = yeast_store(true);
    let env = store.map.as_ref().unwrap().environment().id.clone();
    assert!(store.remove_map_compartment("", &env).is_none());
}

/// JS: "a swept selection that holds a whole compartment deletes that compartment too"
#[test]
fn a_swept_selection_that_holds_a_whole_compartment_deletes_it_too() {
    let mut store = yeast_store(true);
    let cell = store.map.as_ref().unwrap().cells()[0].id.clone();
    store.multi_selection = empty_selection();
    store.multi_selection.compartments.insert((cell, "m".into()));
    let removed = store.remove_multi_selection().unwrap();
    assert_eq!(removed.compartments, ["m"]);
    assert!(!ids(&store).contains(&"m".to_string()));
    assert!(ids(&store).contains(&"im".to_string()), "its parent stays");
}

/// JS: "the compartment list names every compartment, and why the rest cannot be drawn yet"
#[test]
fn the_compartment_list_names_every_compartment_and_why_the_rest_cannot_be_drawn_yet() {
    let mut store = yeast_store(false);
    let choice = |store: &Store, id: &str| store.map_compartment_choices().into_iter().find(|c| c.compartment_id == id).unwrap();
    assert_eq!(choice(&store, "e").status, "drawn");
    assert_eq!(choice(&store, "c").status, "ready");
    assert_eq!(choice(&store, "m").status, "waiting");
    assert!(choice(&store, "m").reason.contains("draw that first"));
    let ready: Vec<String> = store.map_compartment_choices().into_iter().filter(|c| c.status == "ready").map(|c| c.compartment_id).collect();
    let available: Vec<String> = store.available_map_compartments().into_iter().map(|c| c.compartment_id).collect();
    assert_eq!(available, ready);

    store.add_map_compartment("yeast", "c", Some(Rect::new(130.0, 120.0, 800.0, 540.0)));
    assert_eq!(choice(&store, "c").status, "drawn");
    assert_eq!(choice(&store, "im").status, "ready");
    assert!(choice(&store, "im").reason.starts_with("goes inside "));
}

/// JS: "a new compartment takes the chosen shape, and a preset reshapes one in a single undo step"
#[test]
fn a_new_compartment_takes_the_chosen_shape_and_a_preset_is_one_undo_step() {
    let mut store = yeast_store(false);
    let shape_of = |store: &Store, id: &str| store.map.as_ref().unwrap().cells()[0].compartments.iter().find(|c| c.id == id).unwrap().clone();
    store.set_compartment_shape_preset("oval");
    store.add_map_compartment("yeast", "c", Some(Rect::new(130.0, 120.0, 800.0, 540.0)));
    assert_eq!(compartment_shape(&shape_of(&store, "c")), compartment_shape_preset("oval").unwrap());

    store.set_compartment_shape_preset("nonsense");
    assert_eq!(store.compartment_shape_preset, "auto");
    store.add_map_compartment("yeast", "im", Some(Rect::new(250.0, 220.0, 360.0, 260.0)));
    assert_eq!(shape_of(&store, "im").shape, None, "auto leaves it as its kind");

    store.set_compartment_shape(Some("yeast"), "c", ShapePatch::of(compartment_shape_preset("rectangle").unwrap()));
    assert_eq!(compartment_shape(&shape_of(&store, "c")), compartment_shape_preset("rectangle").unwrap());
    store.undo_map();
    assert_eq!(compartment_shape(&shape_of(&store, "c")), compartment_shape_preset("oval").unwrap());
}
