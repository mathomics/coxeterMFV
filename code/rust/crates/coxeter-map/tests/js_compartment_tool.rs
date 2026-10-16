//! Port of `test/compartment-tool.test.mjs`. Every test is ported.
//!
//! Through the store: `availableMapCells`, `availableMapCompartments`,
//! `addMapCell` and `addMapCompartment` are methods of the JavaScript `Store`
//! (`src/ui/store.js`). Each is a thin wrapper over `EditableMap`,
//! re-expressed below (`Store`) with exactly the argument the store builds;
//! the store's own book-keeping (history, selection, events) is left out, and
//! no test here asserts on it.

mod common;

use std::sync::Arc;

use common::load_model;
use coxeter_core::community::{CommunityModel, CommunitySpec, SpeciesSpec};
use coxeter_core::compartments::cell_compartment_name;
use coxeter_core::js;
use coxeter_core::mapdata::Rect;
use coxeter_core::model::Model;
use coxeter_map::EditableMap;
use coxeter_map::editable_map::{AddedShell, CellSpec, CompartmentSpec};
use serde_json::json;

/// `loaded(name, speciesId)`: a blank 1200 × 850 map and the one species.
///
/// The JavaScript also hands the store a `CommunityModel` over the model. It
/// is built here as well, though none of the store methods used reads it.
fn loaded(name: &str, species_id: &str) -> Store {
    let model = Arc::new(load_model(name));
    let _community = CommunityModel::new(CommunitySpec {
        species: vec![SpeciesSpec::new(species_id, model.clone()).abundance(1.0)],
        ..Default::default()
    })
    .unwrap();
    Store {
        species: vec![Species { id: species_id.into(), model, abundance: 1.0, enabled: true }],
        map: EditableMap::blank(Some(1200.0), Some(850.0)),
    }
}

// The store, over the map only

/// One entry of the store's `species`, with the model its `file` names.
struct Species {
    id: String,
    model: Arc<Model>,
    abundance: f64,
    enabled: bool,
}

/// The part of the JavaScript `Store` these tests drive.
struct Store {
    species: Vec<Species>,
    map: EditableMap,
}

struct AvailableCell {
    species_id: String,
    species_name: String,
    outer_compartment_id: String,
    outer_compartment_name: String,
}

struct AvailableCompartment {
    species_name: String,
    compartment_id: String,
    parent_id: Option<String>,
    depth: usize,
}

impl Store {
    /// `availableMapCells()`: species/cells that are not yet drawn.
    fn available_map_cells(&self) -> Vec<AvailableCell> {
        let mut out = Vec::new();
        for sp in self.species.iter().filter(|sp| sp.enabled && self.map.cell(&sp.id).is_none()) {
            let tree = sp.model.compartment_tree();
            let outer = match tree.by_id.get("e") {
                Some(e) if e.parent.is_none() => Some(e.id.clone()),
                _ => tree.roots.first().cloned(),
            };
            let Some(outer) = outer else { continue };
            out.push(AvailableCell {
                species_id: sp.id.clone(),
                species_name: sp.model.cell.name.clone(),
                outer_compartment_name: cell_compartment_name(&outer, &sp.model.compartment_names, "en"),
                outer_compartment_id: outer,
            });
        }
        out.sort_by(|a, b| js::locale_compare(&a.species_name, &b.species_name));
        out
    }

    /// `addMapCell(speciesId, rect)`.
    fn add_map_cell(&mut self, species_id: &str, rect: Rect) -> Option<AddedShell> {
        let available = self.available_map_cells();
        // `cellPlacementSpeciesId` is never set by these tests, so the store's
        // second choice does not apply.
        let spec = available.iter().find(|item| item.species_id == species_id).or(available.first())?;
        let sp = self.species.iter().find(|sp| sp.id == spec.species_id)?;
        let cell = &sp.model.cell;
        self.map.add_cell(&CellSpec {
            species_id: sp.id.clone(),
            outer_compartment_id: Some(spec.outer_compartment_id.clone()),
            outer_label: Some(spec.outer_compartment_name.clone()),
            cell_name: Some(cell.name.clone()),
            organism: Some(cell.organism.clone()),
            note: Some(cell.note.clone()),
            color: Some(cell.color.clone()),
            abundance: Some(sp.abundance),
            rect: Some(rect),
            ..Default::default()
        })
    }

    /// `availableMapCompartments()`: compartments that may be added next
    /// without violating the source model's containment tree. A child becomes
    /// available after its parent is drawn.
    fn available_map_compartments(&self) -> Vec<AvailableCompartment> {
        let mut out = Vec::new();
        for sp in self.species.iter().filter(|sp| sp.enabled) {
            let Some(cell) = self.map.cell(&sp.id) else { continue };
            let drawn = |id: &str| cell.compartments.iter().any(|c| c.id == id);
            for node in sp.model.compartment_tree().by_id.values() {
                if drawn(&node.id) {
                    continue;
                }
                // Root/extracellular geometry is created by the Cell tool.
                // Manual compartment creation starts below that cell boundary.
                let Some(parent) = node.parent.as_deref().filter(|p| !p.is_empty()) else { continue };
                if !drawn(parent) {
                    continue;
                }
                out.push(AvailableCompartment {
                    species_name: sp.model.cell.name.clone(),
                    compartment_id: node.id.clone(),
                    parent_id: Some(parent.to_string()),
                    depth: node.depth,
                });
            }
        }
        out.sort_by(|a, b| {
            js::locale_compare(&a.species_name, &b.species_name)
                .then(a.depth.cmp(&b.depth))
                .then_with(|| js::locale_compare(&a.compartment_id, &b.compartment_id))
        });
        out
    }

    /// `addMapCompartment(speciesId, compartmentId, rect = null)`.
    fn add_map_compartment(&mut self, species_id: &str, compartment_id: &str, rect: Option<Rect>) -> Option<AddedShell> {
        let sp = self.species.iter().find(|sp| sp.id == species_id && sp.enabled)?;
        let tree = sp.model.compartment_tree();
        let parent = tree.by_id.get(compartment_id)?.parent.clone().filter(|p| !p.is_empty())?;
        self.map.cell(species_id)?;
        let cell = &sp.model.cell;
        self.map.add_compartment(&CompartmentSpec {
            species_id: species_id.to_string(),
            compartment_id: compartment_id.to_string(),
            parent_id: Some(parent),
            label: Some(cell_compartment_name(compartment_id, &sp.model.compartment_names, "en")),
            cell_name: Some(cell.name.clone()),
            organism: Some(cell.organism.clone()),
            note: Some(cell.note.clone()),
            color: Some(cell.color.clone()),
            abundance: Some(sp.abundance),
            rect,
            ..Default::default()
        })
    }

    /// The rectangle of one drawn compartment of the first cell, as it is now
    /// (the JavaScript tests hold the live object).
    fn rect_of(&self, compartment_id: &str) -> Rect {
        self.map.cells()[0].compartments.iter().find(|c| c.id == compartment_id).unwrap().rect
    }

    fn parent_of(&self, compartment_id: &str) -> Option<&str> {
        self.map.cells()[0].compartments.iter().find(|c| c.id == compartment_id).unwrap().parent_id.as_deref()
    }
}

/// `available.map((x) => [x.compartmentId, x.parentId])`.
fn id_and_parent(list: &[AvailableCompartment]) -> Vec<(&str, Option<&str>)> {
    list.iter().map(|x| (x.compartment_id.as_str(), x.parent_id.as_deref())).collect()
}

// Tests

/// "E. coli compartments become drawable in biological containment order e -> p -> c"
#[test]
fn e_coli_compartments_become_drawable_in_biological_containment_order_e_p_c() {
    let mut store = loaded("ecoli_core_mc", "eco");
    store.add_map_cell("eco", Rect::new(80.0, 70.0, 760.0, 560.0));

    let available = store.available_map_compartments();
    assert_eq!(id_and_parent(&available), [("p", Some("e"))]);

    let p = store.add_map_compartment("eco", "p", Some(Rect::new(130.0, 120.0, 650.0, 450.0)));
    assert!(p.is_some());
    let available = store.available_map_compartments();
    assert_eq!(id_and_parent(&available), [("c", Some("p"))]);

    let c = store.add_map_compartment("eco", "c", Some(Rect::new(175.0, 165.0, 540.0, 350.0)));
    assert!(c.is_some());

    let periplasm = store.rect_of("p");
    let cytosol = store.rect_of("c");
    assert_eq!(store.parent_of("c"), Some("p"));
    assert!(cytosol.x > periplasm.x);
    assert!(cytosol.y > periplasm.y);
    assert!(cytosol.x + cytosol.w < periplasm.x + periplasm.w);
    assert!(cytosol.y + cytosol.h < periplasm.y + periplasm.h);
}

/// "yeast mitochondrial intermembrane space surrounds mitochondrial matrix when both exist"
#[test]
fn yeast_mitochondrial_intermembrane_space_surrounds_mitochondrial_matrix_when_both_exist() {
    let mut store = loaded("yeast_core_mc", "yeast");
    store.add_map_cell("yeast", Rect::new(80.0, 70.0, 900.0, 650.0));

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
    let im = store.rect_of("im");
    let m = store.rect_of("m");

    assert_eq!(store.parent_of("m"), Some("im"));
    assert!(m.x > im.x);
    assert!(m.y > im.y);
    assert!(m.x + m.w < im.x + im.w);
    assert!(m.y + m.h < im.y + im.h);
}

/// "custom compartment_tree governs ER-like nested compartments without hardcoding names"
#[test]
fn custom_compartment_tree_governs_er_like_nested_compartments_without_hardcoding_names() {
    let model = Model::from_value(&json!({
        "id": "custom",
        "name": "custom",
        "compartments": {
            "e": "Extracellular",
            "c": "Cytosol",
            "r": "Endoplasmic reticulum",
            "rl": "ER lumen",
        },
        "compartment_tree": { "e": null, "c": "e", "r": "c", "rl": "r" },
        "metabolites": [
            { "id": "a_e", "name": "a", "compartment": "e" },
            { "id": "a_c", "name": "a", "compartment": "c" },
            { "id": "a_r", "name": "a", "compartment": "r" },
            { "id": "a_rl", "name": "a", "compartment": "rl" },
        ],
        "reactions": [],
        "genes": [],
        "cell": { "name": "Custom cell", "color": "#64748b" },
    }))
    .unwrap();
    let tree = model.compartment_tree();
    assert_eq!(tree.by_id.get("c").unwrap().parent.as_deref(), Some("e"));
    assert_eq!(tree.by_id.get("r").unwrap().parent.as_deref(), Some("c"));
    assert_eq!(tree.by_id.get("rl").unwrap().parent.as_deref(), Some("r"));
}

/// "a requested compartment rectangle is clamped inside its biological parent"
#[test]
fn a_requested_compartment_rectangle_is_clamped_inside_its_biological_parent() {
    let mut store = loaded("ecoli_core_mc", "eco");
    store.add_map_cell("eco", Rect::new(100.0, 100.0, 520.0, 360.0));

    let result = store.add_map_compartment("eco", "p", Some(Rect::new(-500.0, -400.0, 5000.0, 4000.0)));
    assert!(result.is_some());

    let e = store.rect_of("e");
    let p = store.rect_of("p");
    assert!(p.x > e.x);
    assert!(p.y > e.y);
    assert!(p.x + p.w < e.x + e.w);
    assert!(p.y + p.h < e.y + e.h);
}

/// "a compartment cannot be added before its biological parent exists"
#[test]
fn a_compartment_cannot_be_added_before_its_biological_parent_exists() {
    let mut store = loaded("yeast_core_mc", "yeast");
    store.add_map_cell("yeast", Rect::new(80.0, 70.0, 900.0, 650.0));

    assert!(store.add_map_compartment("yeast", "m", Some(Rect::new(200.0, 200.0, 200.0, 150.0))).is_none());
    assert!(store.add_map_compartment("yeast", "im", Some(Rect::new(200.0, 200.0, 250.0, 180.0))).is_none());
}

/// "editing a child compartment still cannot cross its parent after creation"
#[test]
fn editing_a_child_compartment_still_cannot_cross_its_parent_after_creation() {
    let mut store = loaded("ecoli_core_mc", "eco");
    store.add_map_cell("eco", Rect::new(100.0, 100.0, 700.0, 500.0));
    store.add_map_compartment("eco", "p", Some(Rect::new(140.0, 140.0, 600.0, 400.0)));
    store.add_map_compartment("eco", "c", Some(Rect::new(190.0, 190.0, 480.0, 290.0)));

    let p = store.rect_of("p");

    store.map.edit_geometry_rect(Some("eco"), "c", Rect::new(p.x - 1000.0, p.y - 1000.0, p.w * 5.0, p.h * 5.0));

    let p = store.rect_of("p");
    let c = store.rect_of("c");
    assert!(c.x >= p.x + 10.0 - 1e-6);
    assert!(c.y >= p.y + 10.0 - 1e-6);
    assert!(c.x + c.w <= p.x + p.w - 10.0 + 1e-6);
    assert!(c.y + c.h <= p.y + p.h - 10.0 + 1e-6);
}
