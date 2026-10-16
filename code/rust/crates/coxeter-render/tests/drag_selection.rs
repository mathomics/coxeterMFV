//! "a selected compartment is outlined in the drawing", of
//! `test/drag-selection.test.mjs`: a compartment in the multi-selection gets
//! a dashed outline round it, which moves with the selection, membrane and
//! all. (The map side of that test file is `coxeter-map/tests/drag_selection.rs`.)
//!
//! On the GAPD map of the render references rather than the yeast layout:
//! the renderer only reads the compartment rectangles.

mod common;

use coxeter_render::format::n;
use coxeter_render::{MultiSelection, render_map_svg};

#[test]
fn a_selected_compartment_is_outlined_in_the_drawing() {
    let map = common::load_map(&common::dir(false).join("maps/gapd.json"));
    let spec = common::cases(false).unwrap().into_iter().find(|c| c.0 == "gapd-default").unwrap().2;
    let mut state = common::state_from_spec(&spec);
    let cell = &map.cells[0];
    let comp = &cell.compartments[0];

    state.multi_selection = Some(MultiSelection {
        compartments: [(cell.id.clone(), comp.id.clone())].into_iter().collect(),
        ..Default::default()
    });
    let svg = render_map_svg(&map, &state);
    let r = comp.rect;
    let outline = format!(
        r##"<rect class="emc-multi-compartment" x="{}" y="{}" width="{}" height="{}" rx="18" fill="none" stroke="#2563eb" stroke-width="3" stroke-dasharray="10 6"/>"##,
        n(r.x - 4.0),
        n(r.y - 4.0),
        n(r.w + 8.0),
        n(r.h + 8.0)
    );
    assert!(svg.contains(&outline), "no outline round {}/{}", cell.id, comp.id);
    // A compartment alone is enough for the shaded area under it.
    assert!(svg.contains(r#"class="emc-multi-area""#));
    assert_eq!(svg.matches("emc-multi-compartment").count(), 1);

    state.multi_selection = Some(MultiSelection::default());
    assert!(!render_map_svg(&map, &state).contains("emc-multi-compartment"));
}
