//! "the store selects compartments by rectangle and its drag moves them", of
//! `test/drag-selection.test.mjs`: `selectInRect` takes the compartments a
//! rectangle encloses into the multi-selection, and `moveMultiSelection`
//! (the map's group drag) carries them. The map side of that test file is
//! `coxeter-map/tests/drag_selection.rs`.

mod common;

use std::sync::Arc;

use coxeter_core::community::{CommunityModel, CommunitySpec, SpeciesSpec};
use coxeter_core::mapdata::Rect;
use coxeter_map::EditableMap;
use coxeter_map::layout::{LayoutOptions, layout_community};
use coxeter_map::multi_selection::SelectionRect;

use common::{bundled_model, store};

/// The yeast layout, which has a mitochondrion inside a mitochondrial
/// membrane inside the cytosol.
fn yeast() -> EditableMap {
    let species = vec![SpeciesSpec::new("sce", Arc::new(bundled_model("yeast_core_mc"))).abundance(1.0)];
    let com = CommunityModel::new(CommunitySpec { species, ..Default::default() }).unwrap();
    EditableMap::from_layout(layout_community(&com, &LayoutOptions::default()), None)
}

/// The innermost compartment that still holds one (the mitochondrial
/// membrane): its cell id, id and rectangle.
fn organelle(map: &EditableMap) -> (String, String, Rect) {
    let cell = &map.cells()[0];
    let depth = |id: &str| {
        let mut d = 0;
        let mut q = cell.compartments.iter().find(|k| k.id == id).and_then(|k| k.parent_id.clone());
        while let Some(pid) = q {
            d += 1;
            q = cell.compartments.iter().find(|k| k.id == pid).and_then(|k| k.parent_id.clone());
        }
        d
    };
    let mut holders: Vec<_> = cell
        .compartments
        .iter()
        .filter(|c| c.parent_id.is_some() && cell.compartments.iter().any(|k| k.parent_id.as_ref() == Some(&c.id)))
        .collect();
    holders.sort_by_key(|c| std::cmp::Reverse(depth(&c.id)));
    let org = holders[0];
    assert!(depth(&org.id) >= 2, "expected the mitochondrial membrane, got {}", org.id);
    (cell.id.clone(), org.id.clone(), org.rect)
}

fn compartment_rect(map: &EditableMap, cell: &str, id: &str) -> Rect {
    map.cells().iter().find(|c| c.id == cell).unwrap().compartments.iter().find(|c| c.id == id).unwrap().rect
}

#[test]
fn the_store_selects_compartments_by_rectangle_and_its_drag_moves_them() {
    let mut store = store();
    store.map = Some(yeast());
    let (cell, org, rect) = organelle(store.map.as_ref().unwrap());
    store.select_in_rect(SelectionRect::from_xywh(rect.x - 4.0, rect.y - 4.0, rect.w + 8.0, rect.h + 8.0), true);
    assert!(store.multi_selection.compartments.contains(&(cell.clone(), org.clone())));
    let moved = store.move_multi_selection(0.0, 5.0).unwrap();
    assert!(!moved.compartments.is_empty());
    assert_eq!(compartment_rect(store.map.as_ref().unwrap(), &cell, &org).y, rect.y + moved.dy);
}
