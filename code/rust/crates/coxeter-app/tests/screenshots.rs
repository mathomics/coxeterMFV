//! Pictures of the main states of the window, drawn offscreen through wgpu
//! (kept when `COXETER_SHOT_DIR` names a folder), for comparison with the
//! JavaScript app. Each also checks that every symbol of the interface has
//! a font to draw it (no missing-glyph boxes).

mod common;

use coxeter_app::ui_state::Workspace;

#[test]
fn main_states_render() {
    let mut h = common::rendering_harness_of(common::shell(None));
    let missing = coxeter_app::fonts::missing_glyphs(&h.ctx);
    assert!(missing.is_empty(), "no installed font draws {missing:?}");
    common::shot(&mut h, "solved");

    // The inspector, on a reaction.
    let rid = h.state().store.map.as_ref().unwrap().reactions().keys().next().unwrap().clone();
    h.state_mut().store.select(Some(&rid));
    common::settle(&mut h);
    assert!(!h.state().ui.inspector_collapsed, "a selection opens the inspector");
    common::shot(&mut h, "inspector");

    // The results dock, open.
    common::click(&mut h, "Analysis results");
    common::shot(&mut h, "results");

    // A workspace page.
    h.state_mut().open_workspace(Workspace::Scenario);
    common::settle(&mut h);
    common::shot(&mut h, "workspace-scenario");
    h.state_mut().close_workspace();

    // Dark.
    common::click(&mut h, "Switch to dark theme");
    common::shot(&mut h, "dark");

    // The empty map's card.
    h.state_mut().map_empty();
    common::settle(&mut h);
    common::shot(&mut h, "empty-map");
}

#[test]
fn a_single_cell_and_the_simulate_tab_render() {
    let mut h = common::rendering_harness_of(common::shell(Some("solo-ecoli")));
    common::click(&mut h, "Simulate");
    common::shot(&mut h, "solo-ecoli-simulate");
    common::click(&mut h, "Edit Model");
    common::shot(&mut h, "solo-ecoli-model");
}
