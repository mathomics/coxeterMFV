//! The cross-feeding highlight (`crossFeedingHighlighted()` of
//! `src/ui/store.js`): it dims all but the transfers between cells, so a map of
//! one cell is never drawn with it, and the view setting waits for the next
//! community.

mod common;

use common::init_store;
use coxeter_state::{RenderOptions, ViewPatch};

#[test]
fn a_single_cell_map_is_never_dimmed_by_the_cross_feeding_highlight() {
    let mut store = init_store();
    store.use_scenario("consorcio-anaerobio").unwrap();
    store.set_view(ViewPatch { show_cross_feeding: Some(true), ..Default::default() }).unwrap();
    assert!(store.cross_feeding_highlighted());
    assert!(store.map_render_state(RenderOptions::default()).show_cross_feeding);

    store.use_scenario("solo-levadura").unwrap();
    assert!(store.view.show_cross_feeding, "the view setting is kept");
    assert!(!store.cross_feeding_highlighted(), "one cell has no one to feed");
    let state = store.map_render_state(RenderOptions::default());
    assert!(!state.show_cross_feeding && state.cross_feeding_bases.is_none(), "so the map is drawn undimmed");

    store.use_scenario("aerobio-mixto").unwrap();
    assert!(store.cross_feeding_metabolites().is_empty(), "E. coli and yeast pass nothing in this solution");
    assert!(!store.cross_feeding_highlighted(), "two cells that pass nothing are not dimmed either");

    store.use_scenario("consorcio-anaerobio").unwrap();
    assert!(store.map_render_state(RenderOptions::default()).show_cross_feeding, "and the next community that does has it again");
}
