//! The map toolbar: undo and redo, and the View popover
//! (`renderViewControls` of `src/ui/panels.js`) with what `onSidebarClick` /
//! `onSidebarInput` do for its controls.

mod common;

type H = egui_kittest::Harness<'static, coxeter_app::Shell>;

fn open_view(h: &mut H) {
    if !h.state().ui.view_open {
        common::press(h, "View menu");
    }
    assert!(h.state().ui.view_open);
}

#[test]
fn undo_and_redo_buttons() {
    let mut h = common::harness(None);
    assert!(common::disabled(&h, "Undo map edit"), "nothing to undo yet");
    let rid = h.state().store.map.as_ref().unwrap().reactions().keys().next().unwrap().clone();
    h.state_mut().store.remove_map_reaction(&rid);
    common::settle(&mut h);
    common::click(&mut h, "Undo map edit");
    assert!(h.state().store.map.as_ref().unwrap().has_reaction(&rid), "map-undo");
    common::click(&mut h, "Redo map edit");
    assert!(!h.state().store.map.as_ref().unwrap().has_reaction(&rid), "map-redo");
}

#[test]
fn view_toggles_change_the_view_only() {
    let mut h = common::harness(None);
    let fluxes = h.state().store.solution.as_ref().unwrap().fluxes.clone();
    for (label, read) in [
        ("Cofactors", (|s: &coxeter_state::View| s.show_currency) as fn(&coxeter_state::View) -> bool),
        ("Cofactor labels", |s| s.currency_labels),
        ("Highlight cross-feeding", |s| s.show_cross_feeding),
    ] {
        open_view(&mut h);
        let before = read(&h.state().store.view);
        assert_eq!(common::pressed(&h, label), before, "{label} shows its state");
        common::click(&mut h, label);
        assert_eq!(read(&h.state().store.view), !before, "{label}");
    }
    assert_eq!(h.state().store.solution.as_ref().unwrap().fluxes, fluxes, "presentation only: no flux changed");
}

#[test]
fn labels_and_scrolling_selects() {
    let mut h = common::harness(None);
    open_view(&mut h);
    common::choose(&mut h, "Map labels", "None");
    assert_eq!(h.state().store.view.labels, "none");
    open_view(&mut h);
    common::choose(&mut h, "Scrolling input", "Touchpad: pan");
    assert_eq!(h.state().store.view.scroll_input, "touchpad");
}

#[test]
fn map_modes_core_overview_and_full_network() {
    let mut h = common::harness(None);
    open_view(&mut h);
    if common::disabled(&h, "Core metabolism") {
        // This community has no curated overview; the button says so by being off.
        assert!(!h.state().store.core_metabolism_overview_support().is_some_and(|s| s.supported));
    } else {
        common::click(&mut h, "Core metabolism");
        assert_eq!(h.state().store.map_mode, "core-overview");
    }
    open_view(&mut h);
    let before = h.state().store.map.as_ref().unwrap().reactions().len();
    common::click(&mut h, "Full network");
    assert_eq!(h.state().store.map_mode, "full");
    assert!(h.state().store.map.as_ref().unwrap().reactions().len() >= before);
}

#[test]
fn hidden_metabolites_come_back_from_the_view_menu() {
    let mut h = common::harness(None);
    let node = h.state().store.map.as_ref().unwrap().nodes().values().find(|n| n.kind.as_str() == "metabolite").unwrap().id.clone();
    h.state_mut().store.set_map_node_hidden(&node, true);
    common::settle(&mut h);
    assert_eq!(h.state().store.hidden_metabolite_count(), 1);
    open_view(&mut h);
    common::click(&mut h, "Show hidden metabolites (1)");
    assert_eq!(h.state().store.hidden_metabolite_count(), 0);
}

#[test]
fn colors_and_pathways_opens_the_map_options_page() {
    let mut h = common::harness(None);
    open_view(&mut h);
    common::press(&mut h, "Colors & pathways ↗");
    assert_eq!(h.state().ui.workspace, Some(coxeter_app::ui_state::Workspace::Map));
    assert!(!h.state().ui.view_open, "the popover closes");
}

