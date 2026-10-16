//! Port of `test/blank-map-canvas.test.mjs`.
//!
//! Not ported: "blank map renderer shows the white canvas but no environment
//!   or compartments" — every assertion is about the SVG string (crate
//!   `coxeter-render`).
//!
//! Through the store: `showEmptyMap`, `availableMapCells`,
//! `availableMapCompartments`, `addMapCell` and `addMapCompartment` are
//! methods of the JavaScript `Store` (`src/ui/store.js`). Each is a thin
//! wrapper over `EditableMap`, re-expressed below (`Store`) with exactly the
//! argument the store builds; the store's own book-keeping (history,
//! selection, events) is left out, and no test here asserts on it.

mod common;

use std::sync::Arc;

use common::load_model;
use coxeter_core::community::{CommunityModel, CommunitySpec, MediumLimit, SpeciesSpec};
use coxeter_core::compartments::cell_compartment_name;
use coxeter_core::js;
use coxeter_core::mapdata::{Canvas, Rect};
use coxeter_core::model::Model;
use coxeter_map::EditableMap;
use coxeter_map::editable_map::{AddedShell, CellSpec, CompartmentSpec};
use coxeter_map::layout::LayoutOptions;
use serde_json::Value;

/// `ecoli()`: the model and a one-species community over it.
fn ecoli() -> (Arc<Model>, CommunityModel) {
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
}

fn compartment_ids(list: &[AvailableCompartment]) -> Vec<&str> {
    list.iter().map(|x| x.compartment_id.as_str()).collect()
}

// Tests

/// "blank map contains only a drawing canvas and no biological geometry"
#[test]
fn blank_map_contains_only_a_drawing_canvas_and_no_biological_geometry() {
    let map = EditableMap::blank(Some(900.0), Some(600.0));
    assert_eq!(map.width(), 900.0);
    assert_eq!(map.height(), 600.0);
    assert_eq!(map.cells().len(), 0);
    assert_eq!(map.nodes().len(), 0);
    assert_eq!(map.reactions().len(), 0);
    assert_eq!(map.segments().len(), 0);
    assert_eq!(map.environment().hidden, Some(true));
    assert!(map.is_empty());
}

/// "showEmptyMap now produces a truly blank map rather than the structural shell"
#[test]
fn show_empty_map_now_produces_a_truly_blank_map_rather_than_the_structural_shell() {
    let (_, community) = ecoli();
    let map = EditableMap::from_community(&community, &LayoutOptions::default());
    let width = map.width();
    let height = map.height();

    // `store.showEmptyMap()`: `EditableMap.blank({ width, height })` of the
    // map on screen, which then replaces it.
    let map = EditableMap::blank(Some(width), Some(height));

    assert_eq!(map.cells().len(), 0);
    assert_eq!(map.nodes().len(), 0);
    assert_eq!(map.reactions().len(), 0);
    assert_eq!(map.environment().hidden, Some(true));
    assert_eq!(map.width(), width);
    assert_eq!(map.height(), height);
}

/// "manual structure follows the biological compartment hierarchy"
#[test]
fn manual_structure_follows_the_biological_compartment_hierarchy() {
    let (model, _community) = ecoli();
    let mut store = Store {
        species: vec![Species { id: "eco".into(), model, abundance: 1.0, enabled: true }],
        map: EditableMap::blank(Some(1000.0), Some(700.0)),
    };

    let available = store.available_map_compartments();
    assert!(available.is_empty());
    assert_eq!(store.available_map_cells().iter().map(|x| x.species_id.as_str()).collect::<Vec<_>>(), ["eco"]);

    let cell = store.add_map_cell("eco", Rect::new(80.0, 70.0, 520.0, 360.0));
    assert!(cell.is_some());
    assert_eq!(store.map.cells().len(), 1);
    assert_eq!(store.map.cells()[0].compartments.len(), 1);
    assert_eq!(store.map.cells()[0].compartments[0].id, "e");

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

/// "adding map compartments does not change the biological model"
#[test]
fn adding_map_compartments_does_not_change_the_biological_model() {
    let (model, community) = ecoli();
    let reaction_count = community.merged.reactions.len();
    let metabolite_count = community.merged.metabolites.len();
    let source_compartments = model.compartment_ids();

    let mut map = EditableMap::blank(Some(1000.0), Some(700.0));
    map.add_cell(&CellSpec {
        species_id: "eco".into(),
        outer_compartment_id: Some("e".into()),
        outer_label: model.compartment_names.get("e").cloned(),
        cell_name: Some(model.cell.name.clone()),
        color: Some(model.cell.color.clone()),
        rect: Some(Rect::new(100.0, 80.0, 520.0, 360.0)),
        ..Default::default()
    });

    assert_eq!(community.merged.reactions.len(), reaction_count);
    assert_eq!(community.merged.metabolites.len(), metabolite_count);
    assert_eq!(model.compartment_ids(), source_compartments);
}

/// "canvas can grow and cannot shrink through existing content"
#[test]
fn canvas_can_grow_and_cannot_shrink_through_existing_content() {
    let mut map = EditableMap::blank(Some(800.0), Some(500.0));
    let size = map.set_canvas_size(Some(1200.0), Some(900.0), 0.0, 0.0);
    assert_eq!(size, Canvas { width: 1200.0, height: 900.0 });

    map.add_cell(&CellSpec {
        species_id: "eco".into(),
        outer_compartment_id: Some("e".into()),
        cell_name: Some("E. coli".into()),
        rect: Some(Rect::new(700.0, 500.0, 350.0, 250.0)),
        ..Default::default()
    });

    let min = map.minimum_canvas_size(40.0);
    let size = map.set_canvas_size(Some(400.0), Some(300.0), 0.0, 0.0);
    assert!(size.width >= min.width);
    assert!(size.height >= min.height);
    assert!(size.width >= 1090.0);
    assert!(size.height >= 790.0);
}

/// "blank-map canvas size and manually added compartments survive JSON round trip"
#[test]
fn blank_map_canvas_size_and_manually_added_compartments_survive_json_round_trip() {
    let mut map = EditableMap::blank(Some(1100.0), Some(750.0));
    map.add_cell(&CellSpec {
        species_id: "eco".into(),
        outer_compartment_id: Some("e".into()),
        outer_label: Some("Extracelular".into()),
        cell_name: Some("E. coli".into()),
        color: Some("#0284c7".into()),
        rect: Some(Rect::new(100.0, 80.0, 520.0, 360.0)),
        ..Default::default()
    });
    map.set_canvas_size(Some(1300.0), Some(900.0), 0.0, 0.0);

    // `JSON.parse(JSON.stringify(map.toJSON()))`.
    let doc: Value = serde_json::from_str(&js::stringify(&map.to_json(), None)).unwrap();
    let back = EditableMap::from_json(&doc).unwrap();
    assert_eq!(back.width(), 1300.0);
    assert_eq!(back.height(), 900.0);
    assert_eq!(back.environment().hidden, Some(true));
    assert_eq!(back.cells().len(), 1);
    assert_eq!(back.cells()[0].compartments[0].id, "e");
}
