//! Port of `test/cell-tool.test.mjs`.
//!
//! Not ported: "Cell boundary is rendered as an explicit selectable boundary"
//!   — every assertion is about the SVG string (crate `coxeter-render`).
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
use coxeter_core::boundaries::BoundaryKind;
use coxeter_core::community::{CommunityModel, CommunitySpec, MediumLimit, SpeciesSpec};
use coxeter_core::compartments::cell_compartment_name;
use coxeter_core::js;
use coxeter_core::mapdata::Rect;
use coxeter_core::model::Model;
use coxeter_map::EditableMap;
use coxeter_map::editable_map::{AddedShell, CellSpec, CompartmentSpec};

/// `fixture()`: the model and a one-species community over it.
fn fixture() -> (Arc<Model>, CommunityModel) {
    let model = Arc::new(load_model("ecoli_core_mc"));
    let community = CommunityModel::new(CommunitySpec {
        species: vec![SpeciesSpec::new("eco", model.clone()).abundance(1.0)],
        medium: [("glc__D", -10.0), ("o2", -20.0)]
            .into_iter()
            .map(|(base, lower)| (base.to_string(), MediumLimit { lower: Some(lower), upper: None }))
            .collect(),
        ..Default::default()
    })
    .unwrap();
    (model, community)
}

/// `storeFor(model, community)`: a blank 1000 × 700 map and the one species.
fn store_for(model: &Arc<Model>) -> Store {
    Store {
        species: vec![Species { id: "eco".into(), model: model.clone(), abundance: 1.0, enabled: true }],
        map: EditableMap::blank(Some(1000.0), Some(700.0)),
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
}

// Tests

/// "Cell creation draws the species outer extracellular boundary on a blank map"
#[test]
fn cell_creation_draws_the_species_outer_extracellular_boundary_on_a_blank_map() {
    let (model, _community) = fixture();
    let mut store = store_for(&model);

    assert_eq!(store.available_map_cells().iter().map(|x| x.species_id.as_str()).collect::<Vec<_>>(), ["eco"]);
    assert!(store.available_map_compartments().is_empty());

    let result = store.add_map_cell("eco", Rect::new(120.0, 100.0, 520.0, 360.0));
    assert!(result.is_some());
    assert_eq!(store.map.cells().len(), 1);

    let cell = &store.map.cells()[0];
    assert_eq!(cell.id, "eco");
    assert_eq!(cell.outer_compartment.as_deref(), Some("e"));
    assert_eq!(cell.compartments.len(), 1);
    assert_eq!(cell.compartments[0].id, "e");
    assert_eq!(cell.rect, cell.compartments[0].rect);

    let boundary = store.map.boundary("eco/e").expect("the cell boundary");
    assert_eq!(boundary.kind, BoundaryKind::Cell);
    assert_eq!(boundary.kind.as_str(), "cell");
    assert_eq!(boundary.inside, "e");
    assert_eq!(boundary.outside.as_deref(), Some("s"));

    assert!(store.available_map_cells().is_empty());
    assert_eq!(store.available_map_compartments().iter().map(|x| x.compartment_id.as_str()).collect::<Vec<_>>(), ["p"]);
}

/// "resizing a Cell creates free space without scaling child compartments"
#[test]
fn resizing_a_cell_creates_free_space_without_scaling_child_compartments() {
    let (model, _community) = fixture();
    let mut store = store_for(&model);
    store.add_map_cell("eco", Rect::new(100.0, 90.0, 520.0, 360.0));
    store.add_map_compartment("eco", "p", None);

    let outer = store.rect_of("e");
    let child_before = store.rect_of("p");

    let edited = store.map.edit_geometry_rect(Some("eco"), "e", Rect::new(outer.x, outer.y, outer.w + 180.0, outer.h + 120.0));

    assert!(edited.is_some());
    let outer = store.rect_of("e");
    assert_eq!(store.rect_of("p"), child_before);
    assert_eq!(store.map.cells()[0].rect, outer);
    assert!(outer.w > 520.0);
    assert!(outer.h > 360.0);
}

/// "moving a Cell moves its child compartments by the same translation"
#[test]
fn moving_a_cell_moves_its_child_compartments_by_the_same_translation() {
    let (model, _community) = fixture();
    let mut store = store_for(&model);
    store.add_map_cell("eco", Rect::new(100.0, 90.0, 520.0, 360.0));
    store.add_map_compartment("eco", "p", None);

    let outer = store.rect_of("e");
    let before = store.rect_of("p");

    store.map.edit_geometry_rect(Some("eco"), "e", Rect { x: outer.x + 40.0, y: outer.y + 25.0, ..outer });

    let child = store.rect_of("p");
    assert_eq!(child.x, before.x + 40.0);
    assert_eq!(child.y, before.y + 25.0);
    assert_eq!(child.w, before.w);
    assert_eq!(child.h, before.h);
}

/// "a child compartment cannot be resized beyond its parent boundary"
#[test]
fn a_child_compartment_cannot_be_resized_beyond_its_parent_boundary() {
    let (model, _community) = fixture();
    let mut store = store_for(&model);
    store.add_map_cell("eco", Rect::new(100.0, 90.0, 520.0, 360.0));
    store.add_map_compartment("eco", "p", None);

    let outer = store.rect_of("e");

    store.map.edit_geometry_rect(Some("eco"), "p", Rect::new(outer.x - 500.0, outer.y - 500.0, outer.w * 4.0, outer.h * 4.0));

    let outer = store.rect_of("e");
    let p = store.rect_of("p");
    assert!(p.x >= outer.x + 10.0 - 1e-6);
    assert!(p.y >= outer.y + 10.0 - 1e-6);
    assert!(p.x + p.w <= outer.x + outer.w - 10.0 + 1e-6);
    assert!(p.y + p.h <= outer.y + outer.h - 10.0 + 1e-6);
}

/// "Cell creation is map-only and does not mutate the biological model or solver"
#[test]
fn cell_creation_is_map_only_and_does_not_mutate_the_biological_model_or_solver() {
    let (model, community) = fixture();
    let mut store = store_for(&model);
    let reactions = community.merged.reactions.len();
    let metabolites = community.merged.metabolites.len();

    store.add_map_cell("eco", Rect::new(100.0, 90.0, 520.0, 360.0));

    assert_eq!(community.merged.reactions.len(), reactions);
    assert_eq!(community.merged.metabolites.len(), metabolites);
    assert!(model.compartment_ids().iter().any(|id| id == "e"));
}
