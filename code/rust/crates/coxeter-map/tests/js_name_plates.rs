//! Port of `test/name-plates.test.mjs`: the names of compartments, cells and
//! the medium — the geometry parts only.
//!
//! A name plate (its pill, its text, its `--fit` limit and where it is drawn
//! at a zoom) is computed by `renderMap` and `frameLabelScale` in
//! `src/render/mapview.js`, which is the render crate's. Six of the seven
//! JavaScript tests assert on nothing else, so only the one that also edits
//! the map is ported here, as far as the map goes.
//!
//! Not ported: "every compartment and the medium is named, each text cut to its own pill"
//!   — every assertion is on the plates parsed out of the rendered SVG.
//! Not ported: "a name plate stays inside its own compartment at every zoom"
//!   — rendered plates and `frameLabelScale`.
//! Not ported: "plates are all one size on screen until a compartment is too small for that"
//!   — rendered plates and `frameLabelScale`.
//! Not ported: "a plate keeps out of the compartments nested in its own"
//!   — rendered plates and `frameLabelScale`.
//! Not ported (one assertion of): "a compartment is named even when one nested in it reaches its top edge"
//!   — the last assertion, that the rendering still holds a plate `c` reading
//!   "Cytosol", is on the rendered SVG. The edit and its geometry are ported.
//! Not ported: "two plates astride closely nested top edges share the band between them"
//!   — rendered plates and `frameLabelScale` (the band it measures them
//!   against is map geometry, but nothing is asserted about it alone).
//! Not ported: "the cell is named on a plate as wide as its name, held to the cell and the room above it"
//!   — the cell plate parsed out of the rendered SVG.

use std::sync::Arc;

use coxeter_core::community::{CommunityModel, CommunitySpec, SpeciesSpec};
use coxeter_core::mapdata::Rect;
use coxeter_core::model::Model;
use coxeter_map::EditableMap;
use coxeter_map::layout::LayoutOptions;
use indexmap::IndexMap;
use serde_json::{Value, json};

/// The JS `cell()` (its `tree` option is never given by these tests).
fn cell() -> CommunityModel {
    let met = |id: &str, compartment: &str| -> Value {
        json!({ "id": id, "name": id, "compartment": compartment, "formula": "", "charge": 0 })
    };
    let model = Model::from_value(&json!({
        "id": "alga",
        "name": "A cell with a long descriptive name",
        "compartments": { "e": "e", "c": "c", "h": "h", "u": "u", "i": "i", "m": "m", "g": "g" },
        "metabolites": [
            met("a_e", "e"), met("a_c", "c"), met("a_h", "h"), met("a_u", "u"),
            met("a_i", "i"), met("a_m", "m"), met("a_g", "g"),
        ],
        "reactions": [
            { "id": "T", "name": "T", "metabolites": { "a_e": -1, "a_c": 1 }, "lower_bound": -1000, "upper_bound": 1000 },
        ],
    }))
    .unwrap();
    CommunityModel::new(CommunitySpec {
        species: vec![SpeciesSpec::new("alga", Arc::new(model)).abundance(1.0)],
        medium: IndexMap::new(),
        ..Default::default()
    })
    .unwrap()
}

/// `map.cells[0].compartments.find((c) => c.id === id).rect`.
fn rect_of(map: &EditableMap, id: &str) -> Rect {
    map.cells()[0].compartments.iter().find(|c| c.id == id).unwrap_or_else(|| panic!("no compartment {id}")).rect
}

/// JS: `a compartment is named even when one nested in it reaches its top edge`
///
/// The geometry part: the chloroplast is moved to within a plate's height of
/// the top of the cytosol. That the cytosol is still named there is asserted
/// on the rendering, in the render crate.
#[test]
fn a_compartment_is_named_even_when_one_nested_in_it_reaches_its_top_edge() {
    // The cytosol keeps its name when an organelle stands within a plate's
    // height of its top, wherever along the top that organelle is.
    let mut map = EditableMap::empty_from_community(&cell(), &LayoutOptions::default());
    let cytosol = rect_of(&map, "c");
    let chloroplast = rect_of(&map, "h");
    map.edit_geometry_rect(Some("alga"), "h", Rect { y: cytosol.y + 10.0, ..chloroplast });
    // The JS `chloroplast` and `cytosol` are the live compartments: read them again.
    let cytosol = rect_of(&map, "c");
    let chloroplast = rect_of(&map, "h");
    assert!(chloroplast.y - cytosol.y < 27.0);
}
