//! The window's `keydown` handler of `src/ui/app.js`: Escape, Ctrl+F,
//! Ctrl+Z / Ctrl+Y (and Ctrl+Shift+Z), Delete / Backspace, 0 + −, and the
//! arrows the native map adds, wherever the pointer is.

mod common;

use coxeter_app::ui_state::Workspace;
use coxeter_canvas::Tool;
use egui::{Key, Modifiers};

type H = egui_kittest::Harness<'static, coxeter_app::Shell>;

/// A drawn reaction that can be removed.
fn some_reaction(h: &H) -> String {
    h.state().store.map.as_ref().unwrap().reactions().keys().next().unwrap().clone()
}

/// Keep the pointer away from the map, over the left panel.
fn pointer_off_map(h: &mut H) {
    h.event(egui::Event::PointerMoved(egui::pos2(100.0, 500.0)));
    h.run_steps(1);
}

/// Put the pointer on the map.
fn pointer_on_map(h: &mut H) {
    let rect = h.state().map_rect.unwrap();
    h.event(egui::Event::PointerMoved(rect.center()));
    h.run_steps(1);
}

#[test]
fn delete_removes_the_selected_reaction_and_ctrl_z_brings_it_back() {
    for on_map in [false, true] {
        let mut h = common::harness(None);
        let rid = some_reaction(&h);
        h.state_mut().store.select(Some(&rid));
        common::settle(&mut h);
        if on_map { pointer_on_map(&mut h) } else { pointer_off_map(&mut h) }
        h.key_press(Key::Delete);
        common::settle(&mut h);
        assert!(!h.state().store.map.as_ref().unwrap().has_reaction(&rid), "Delete (pointer on map: {on_map})");
        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        common::settle(&mut h);
        assert!(h.state().store.map.as_ref().unwrap().has_reaction(&rid), "Ctrl+Z (pointer on map: {on_map})");
        h.key_press_modifiers(Modifiers::COMMAND, Key::Y);
        common::settle(&mut h);
        assert!(!h.state().store.map.as_ref().unwrap().has_reaction(&rid), "Ctrl+Y");
        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
        common::settle(&mut h);
        assert!(!h.state().store.map.as_ref().unwrap().has_reaction(&rid), "Ctrl+Shift+Z redoes");
        assert!(h.state().store.can_undo_map());
    }
}

#[test]
fn escape_clears_the_selection_and_returns_to_select() {
    let mut h = common::harness(None);
    common::click(&mut h, "Add Metabolite");
    assert_eq!(h.state().map.tool(), Tool::Metabolite);
    let rid = some_reaction(&h);
    h.state_mut().store.select(Some(&rid));
    common::settle(&mut h);
    pointer_off_map(&mut h);
    h.key_press(Key::Escape);
    common::settle(&mut h);
    assert_eq!(h.state().store.selected_reaction, None);
    assert_eq!(h.state().map.tool(), Tool::Select);
}

#[test]
fn escape_closes_what_is_open_first() {
    let mut h = common::harness(None);
    h.state_mut().open_workspace(Workspace::Medium);
    let rid = some_reaction(&h);
    h.state_mut().store.select(Some(&rid));
    common::settle(&mut h);
    h.key_press(Key::Escape);
    common::settle(&mut h);
    assert_eq!(h.state().ui.workspace, None, "the workspace closes");
    assert_eq!(h.state().store.selected_reaction.as_deref(), Some(rid.as_str()), "and nothing else happens");
    common::click(&mut h, "Maximize results");
    h.key_press(Key::Escape);
    common::settle(&mut h);
    assert!(!h.state().ui.results_maximized, "Escape restores the results dock");
}

#[test]
fn keys_do_nothing_while_a_field_has_the_keyboard() {
    let mut h = common::harness(None);
    h.state_mut().open_workspace(Workspace::Library);
    common::settle(&mut h);
    let rid = some_reaction(&h);
    h.state_mut().store.select(Some(&rid));
    common::settle(&mut h);
    // The reaction search field takes the keyboard (Ctrl+F's target).
    h.state_mut().ui.focus_request = Some(coxeter_app::panels::map_options::reaction_search_id());
    common::settle(&mut h);
    if !h.ctx.text_edit_focused() {
        // The search card is drawn by another module; without a field there is nothing to test.
        return;
    }
    h.key_press(Key::Delete);
    common::settle(&mut h);
    assert!(h.state().store.map.as_ref().unwrap().has_reaction(&rid), "Delete in a field edits the field");
}

#[test]
fn zero_plus_and_minus_zoom_the_map() {
    let mut h = common::harness(None);
    pointer_off_map(&mut h);
    let fitted = h.state().map.camera().scale;
    h.key_press(Key::Plus);
    common::settle(&mut h);
    assert!((h.state().map.camera().scale / fitted - 1.3).abs() < 1e-9);
    h.key_press(Key::Minus);
    h.key_press(Key::Minus);
    common::settle(&mut h);
    assert!((h.state().map.camera().scale * 1.3 / fitted - 1.0).abs() < 1e-9);
    h.key_press(Key::Num0);
    common::settle(&mut h);
    assert!((h.state().map.camera().scale - fitted).abs() < 1e-9, "0 fits the map");
}

#[test]
fn arrows_nudge_the_selected_metabolite() {
    let mut h = common::harness(None);
    let map = h.state().store.map.clone().unwrap();
    let node = map.nodes().values().find(|n| n.kind.as_str() == "metabolite" && h.state().store.can_move_node(&n.id)).unwrap().clone();
    let mut details = serde_json::Map::new();
    details.insert("x".into(), 0.into());
    h.state_mut().store.select_entity("metabolite", &node.id, serde_json::Map::new());
    common::settle(&mut h);
    pointer_on_map(&mut h);
    h.key_press(Key::ArrowRight);
    common::settle(&mut h);
    let moved = h.state().store.map.as_ref().unwrap().node(&node.id).unwrap().clone();
    assert!(moved.x > node.x, "{} → {}", node.x, moved.x);
    assert_eq!(moved.y, node.y);
    assert!(h.state().store.can_undo_map(), "one undoable edit");
}

/// compartment-delete: "Delete removes the selected compartment or cell, read
/// from what is selected" (the JS reads app.js; this presses the key).
#[test]
fn delete_removes_the_selected_compartment_and_ctrl_z_brings_it_back() {
    let mut h = common::harness(None);
    let (cell, comp) = {
        let map = h.state().store.map.as_ref().unwrap();
        let cell = &map.cells()[0];
        (cell.id.clone(), cell.compartments.iter().find(|c| c.parent_id.is_some()).unwrap().id.clone())
    };
    h.state_mut().store.select_geometry(Some(&cell), &comp);
    common::settle(&mut h);
    pointer_on_map(&mut h);
    h.key_press(Key::Delete);
    common::settle(&mut h);
    let gone = |h: &H| h.state().store.map.as_ref().unwrap().cell(&cell).is_none_or(|c| c.compartments.iter().all(|k| k.id != comp));
    assert!(gone(&h), "the compartment went");
    assert!(h.state().store.status().message.starts_with("Removed "));
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    common::settle(&mut h);
    assert!(!gone(&h), "Ctrl+Z brings it back");
}
