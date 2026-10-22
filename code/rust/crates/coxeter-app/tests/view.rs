//! The map toolbar: undo and redo, the display options used most
//! (`renderMapQuickControls`), the Image menu, and the View popover
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
fn toolbar_and_view_toggles_change_the_view_only() {
    let mut h = common::harness(None);
    let fluxes = h.state().store.solution.as_ref().unwrap().fluxes.clone();
    for (label, in_view, read) in [
        ("Cofactors", false, (|s: &coxeter_state::View| s.show_currency) as fn(&coxeter_state::View) -> bool),
        ("⇄ Cross-feeding", false, |s| s.show_cross_feeding),
        ("Width by flux", false, |s| s.width_by_flux),
        ("Cofactor labels", true, |s| s.currency_labels),
    ] {
        if in_view {
            open_view(&mut h);
        }
        let before = read(&h.state().store.view);
        assert_eq!(common::pressed(&h, label), before, "{label} shows its state");
        common::click(&mut h, label);
        assert_eq!(read(&h.state().store.view), !before, "{label}");
    }
    assert_eq!(h.state().store.solution.as_ref().unwrap().fluxes, fluxes, "presentation only: no flux changed");
}

#[test]
fn cross_feeding_is_offered_only_between_cells() {
    let h = common::harness(Some("solo-ecoli"));
    assert!(!common::has(&h, "⇄ Cross-feeding"), "one cell feeds no other");
    assert!(common::has(&h, "Cofactors"));
}

#[test]
fn the_toolbar_folds_as_the_stylesheet_does() {
    use coxeter_app::panels::toolbar::QuickFold;
    // `@container toolbar (max-width: 1090px)` and `(max-width: 860px)` of app.css.
    assert_eq!(QuickFold::for_width(1091.0), QuickFold::None);
    assert_eq!(QuickFold::for_width(1090.0), QuickFold::Second);
    assert_eq!(QuickFold::for_width(861.0), QuickFold::Second);
    assert_eq!(QuickFold::for_width(860.0), QuickFold::All);
}

#[test]
fn a_narrow_toolbar_hands_its_options_over_to_view() {
    let mut h = common::harness(None);
    assert!(common::has(&h, "Map labels") && common::has(&h, "Line color"), "a wide toolbar has both groups");
    h.set_size(egui::vec2(700.0, 900.0));
    common::settle(&mut h);
    assert!(!common::has(&h, "Map labels") && !common::has(&h, "Line color"), "a narrow one has neither");
    open_view(&mut h);
    for label in ["Map labels", "Cofactors", "⇄ Cross-feeding", "Line color", "Width by flux"] {
        assert!(common::has(&h, label), "{label} is in View instead");
    }
    common::click(&mut h, "Width by flux");
    assert!(h.state().store.view.width_by_flux, "and works there");
}

#[test]
fn the_image_menu_exports_the_figure() {
    let (mut h, dialogs, dir) = common::harness_saving("image-menu", None);
    common::press(&mut h, "Image menu");
    common::press(&mut h, "Export SVG image");
    common::press(&mut h, "Image menu");
    common::press(&mut h, "Export PNG image");
    let saved = common::saved(&dialogs);
    let names: Vec<&str> = saved.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["consorcio-anaerobio_map.svg", "consorcio-anaerobio_map.png"]);
    // The names of the cells and compartments are written at the size of the
    // fitted view, and say so outright for editors that ignore the stylesheet.
    let svg = String::from_utf8_lossy(&saved[0].1).into_owned();
    let k = coxeter_render::export_frame_label_scale(h.state().store.map.as_ref().unwrap().graph());
    assert!(k > 1.0, "a community map is enlarged ({k}×)");
    assert!(svg.contains(&format!(r#"style="--frame-label-scale:{k}""#)));
    assert!(svg.contains(r#"class="emc-frame-label" style="--fit:"#) && svg.contains(&format!(r#" transform="scale({k})""#)));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn escape_that_closes_a_menu_keeps_the_selection() {
    let mut h = common::harness(None);
    let rid = h.state().store.map.as_ref().unwrap().reactions().keys().next().unwrap().clone();
    h.state_mut().store.select(Some(&rid));
    common::settle(&mut h);
    for menu in ["View menu", "File menu"] {
        common::press(&mut h, menu);
        h.key_press(egui::Key::Escape);
        common::settle(&mut h);
        assert_eq!(h.state().store.selected_reaction.as_deref(), Some(rid.as_str()), "Escape closed the {menu} and nothing else");
        assert!(!h.state().ui.view_open);
    }
    h.key_press(egui::Key::Escape);
    common::settle(&mut h);
    assert!(h.state().store.selected_reaction.is_none(), "with nothing open, Escape clears the selection");
}

#[test]
fn quiet_cofactors_is_switched_in_view_as_a_map_edit() {
    let mut h = common::harness(None);
    let style = |h: &H| h.state().store.map.as_ref().unwrap().cofactor_style();
    let before = style(&h);
    open_view(&mut h);
    assert_eq!(common::pressed(&h, "Quiet cofactors"), before == Some("quiet"), "it shows the map's style");
    common::click(&mut h, "Quiet cofactors");
    assert_eq!(style(&h).is_some(), before.is_none(), "switched");
    assert!(h.state().store.map_history.can_undo(), "an undoable map edit");
    open_view(&mut h);
    common::click(&mut h, "Quiet cofactors");
    assert_eq!(style(&h), before, "and back");
}

#[test]
fn labels_color_and_scrolling_selects() {
    let mut h = common::harness(None);
    common::choose(&mut h, "Map labels", "Labels: none");
    assert_eq!(h.state().store.view.labels, "none");
    common::choose(&mut h, "Line color", "Color: pathway");
    assert_eq!(h.state().store.view.reaction_color_mode.as_str(), "pathway");
    assert_eq!(h.state().store.view.line_color_group, "pathway", "what Color lines by flux goes back to");
    common::choose(&mut h, "Line color", "Color: flux");
    assert_eq!(h.state().store.view.reaction_color_mode.as_str(), "flux");
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

