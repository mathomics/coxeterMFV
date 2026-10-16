//! Port of the Store test of `test/compartment-label.test.mjs`.
//!
//! Not ported here, as they drive `EditableMap` (and the renderer) alone,
//! although `fixture()` also builds a store they never touch: "dragging a
//! compartment name follows straight edges and rounded corners without
//! changing map geometry", "a moved compartment name renders on its actual
//! membrane above the reaction targets" and "invalid compartment-name moves
//! are ignored".

mod common;

use std::sync::Arc;

use coxeter_core::community::{CommunityModel, CommunitySpec, SpeciesSpec};
use coxeter_core::mapdata::{Point, Rect};
use coxeter_map::EditableMap;
use coxeter_map::layout::LayoutOptions;
use coxeter_map::place_reaction::PlaceOptions;
use coxeter_state::Store;

use common::{bundled_model, store};

/// `fixture()`: yeast's mitochondrion with the pyruvate transporter put on
/// its membrane, and a store over that map and community.
fn fixture() -> (Store, Rect) {
    let model = bundled_model("yeast_core_mc");
    let community = Arc::new(
        CommunityModel::new(CommunitySpec {
            species: vec![SpeciesSpec::new("yeast", Arc::new(model)).abundance(1.0)],
            ..Default::default()
        })
        .unwrap(),
    );
    let mut map = EditableMap::empty_from_community(&community, &LayoutOptions::default());
    let rect = map.cells()[0].compartments.iter().find(|c| c.id == "m").expect("the mitochondrion").rect;
    map.add_reaction(
        &community,
        "yeast@PYRt2m",
        &PlaceOptions { boundary_id: Some("yeast/m".into()), anchor: Some(Point::new(rect.x, rect.y)), ..PlaceOptions::default() },
    );
    let mut store = store();
    store.map = Some(map);
    store.community = Some(community);
    (store, rect)
}

/// "compartment name positions survive JSON save and load and one drag makes one undo step"
#[test]
fn compartment_name_positions_survive_json_save_and_load_and_one_drag_makes_one_undo_step() {
    let (mut store, rect) = fixture();
    let before = store.map.as_ref().unwrap().to_json();
    let history = store.begin_map_edit("Move compartment name");
    store.move_compartment_label(Some("yeast"), "m", rect.x, rect.y);
    store.move_compartment_label(Some("yeast"), "m", rect.x, rect.y + rect.h / 2.0);
    store.commit_map_edit(history);
    let moved = store.map.as_ref().unwrap().to_json();
    assert_eq!(EditableMap::from_json(&moved).unwrap().to_json(), moved);
    assert_eq!(store.map_history.undo_len(), 1);
    store.undo_map();
    assert_eq!(store.map.as_ref().unwrap().to_json(), before);
    store.redo_map();
    assert_eq!(store.map.as_ref().unwrap().to_json(), moved);
}
