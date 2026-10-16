//! The shell of the window through the real interface: the header, the
//! left panel's tabs and tools, the map toolbar, the right panel, the
//! results dock (the `data-action`s of `index.html` that `onSidebarClick`
//! of `src/ui/app.js` handles).

mod common;

use coxeter_app::ui_state::{AnalysisTab, LeftTab, ResultsTab, RightTab, Workspace};
use coxeter_canvas::Tool;

#[test]
fn left_tabs_switch_the_task_hub() {
    let mut h = common::harness(None);
    assert!(common::shows(&h, "Reaction library"), "Edit Map shows its hub");
    common::click(&mut h, "Edit Model");
    assert_eq!(h.state().ui.left_tab, LeftTab::Model);
    assert!(common::shows(&h, "active species"));
    assert!(common::has(&h, "Scenario & objective"));
    common::click(&mut h, "Simulate");
    assert_eq!(h.state().ui.left_tab, LeftTab::Analysis);
    assert!(common::has(&h, "View simulation results ↗"));
    common::click(&mut h, "Edit Map");
    assert_eq!(h.state().ui.left_tab, LeftTab::Build);
}

#[test]
fn leaving_edit_map_drops_the_drawing_tool() {
    let mut h = common::harness(None);
    common::click(&mut h, "Add Cell");
    assert_eq!(h.state().map.tool(), Tool::Cell);
    assert!(h.state().store.view.geometry_edit, "the drawing tools edit geometry");
    common::click(&mut h, "Simulate");
    assert_eq!(h.state().map.tool(), Tool::Select, "left-tab cancels the drawing mode");
}

#[test]
fn every_tool_of_the_grid() {
    let mut h = common::harness(None);
    for (tool, label) in [
        (Tool::Pan, "Pan Mode"),
        (Tool::Edit, "Edit Control Points"),
        (Tool::Reaction, "Add Reaction"),
        (Tool::Metabolite, "Add Metabolite"),
        (Tool::Text, "Text"),
        (Tool::Compartment, "Add Compartment"),
        (Tool::Cell, "Add Cell"),
        (Tool::Select, "Select"),
    ] {
        common::click(&mut h, label);
        assert_eq!(h.state().map.tool(), tool, "{label}");
        assert!(common::pressed(&h, label), "{label} shows as active");
        assert_eq!(h.state().store.view.text_tool, tool == Tool::Text, "{label}: text tool");
        assert_eq!(h.state().store.view.geometry_edit, matches!(tool, Tool::Cell | Tool::Compartment), "{label}: geometry");
    }
}

#[test]
fn panels_collapse_and_focus_mode() {
    let mut h = common::harness(None);
    common::click(&mut h, "Collapse task panel");
    assert!(h.state().ui.left_collapsed);
    common::click(&mut h, "Tools");
    assert!(!h.state().ui.left_collapsed, "the toolbar's Tools button brings it back");
    common::click(&mut h, "Focus mode");
    assert!(h.state().ui.focus_mode);
    assert!(common::has(&h, "Expand task panel"));
    h.state_mut().ui.focus_mode = false;
    common::settle(&mut h);
    common::click(&mut h, "Toggle inspector");
    assert!(h.state().ui.inspector_pinned, "toggle-inspector pins the panel");
    let collapsed = h.state().ui.inspector_collapsed;
    common::click(&mut h, "Toggle inspector");
    assert_ne!(h.state().ui.inspector_collapsed, collapsed);
}

#[test]
fn right_tabs_and_layout_presets() {
    let mut h = common::harness(None);
    h.state_mut().ui.inspector_collapsed = false;
    common::settle(&mut h);
    common::click(&mut h, "Layout");
    assert_eq!(h.state().ui.right_tab, RightTab::Layout);
    common::click(&mut h, "Inspector");
    assert_eq!(h.state().ui.right_tab, RightTab::Inspector);
    common::click(&mut h, "Compact");
    assert_eq!(h.state().store.layout_settings.preset, "compact");
    assert_eq!(h.state().ui.right_tab, RightTab::Layout, "a preset shows the layout settings");
    assert!(common::pressed(&h, "Compact"));
}

#[test]
fn results_dock() {
    let mut h = common::harness(None);
    assert!(h.state().ui.results_collapsed, "the page starts with the dock collapsed");
    common::click(&mut h, "Table view");
    assert_eq!(h.state().ui.results_tab, ResultsTab::Table);
    assert!(!h.state().ui.results_collapsed, "a results tab opens the dock");
    common::click(&mut h, "Logs");
    assert_eq!(h.state().ui.results_tab, ResultsTab::Logs);
    assert!(common::shows(&h, "History ·"), "the log lists the session's messages");
    common::click(&mut h, "Clear");
    assert!(h.state().store.log.is_empty(), "clear-log");
    common::click(&mut h, "Analysis results");
    assert!(common::shows(&h, "completed successfully"));
    common::click(&mut h, "Maximize results");
    assert!(h.state().ui.results_maximized);
    common::click(&mut h, "Restore results dock");
    assert!(!h.state().ui.results_maximized);
    common::click(&mut h, "Collapse results");
    assert!(h.state().ui.results_collapsed);
}

#[test]
fn analysis_subtabs() {
    let mut h = common::harness(None);
    common::click(&mut h, "Analysis results");
    assert!(common::has(&h, "Flux"));
    common::click(&mut h, "Reduced costs");
    assert_eq!(h.state().ui.analysis_tab, AnalysisTab::Reduced);
    assert!(common::shows(&h, "REDUCED COST"));
    common::click(&mut h, "Shadow prices");
    assert_eq!(h.state().ui.analysis_tab, AnalysisTab::Shadow);
    assert!(common::disabled(&h, "Reproducibility"), "nothing to replay yet");
    assert!(common::disabled(&h, "FVA"), "FVA is off");
}

#[test]
fn theme_toggle() {
    let mut h = common::harness(None);
    assert!(!h.state().store.view.dark);
    common::click(&mut h, "Switch to dark theme");
    assert!(h.state().store.view.dark);
    assert!(h.ctx.theme() == egui::Theme::Dark);
    common::click(&mut h, "Switch to light theme");
    assert!(!h.state().store.view.dark);
}

#[test]
fn workspaces_open_and_close() {
    let mut h = common::harness(None);
    common::click(&mut h, "Reaction library");
    assert_eq!(h.state().ui.workspace, Some(Workspace::Library));
    common::click(&mut h, "← Return to map");
    assert_eq!(h.state().ui.workspace, None);
    common::click(&mut h, "Find reaction");
    assert_eq!(h.state().ui.workspace, Some(Workspace::Library));
    h.key_press(egui::Key::Escape);
    common::settle(&mut h);
    assert_eq!(h.state().ui.workspace, None, "Escape closes a workspace");
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::F);
    common::settle(&mut h);
    assert_eq!(h.state().ui.workspace, Some(Workspace::Library), "Ctrl+F finds a reaction");
}

#[test]
fn zoom_buttons() {
    let mut h = common::harness(None);
    let before = h.state().map.camera().scale;
    common::click(&mut h, "Zoom in");
    let zoomed = h.state().map.camera().scale;
    assert!((zoomed / before - 1.3).abs() < 1e-9, "{before} → {zoomed}");
    common::click(&mut h, "Zoom out");
    assert!((h.state().map.camera().scale - before).abs() < 1e-9);
    common::click(&mut h, "Zoom in");
    common::click(&mut h, "Fit map to screen");
    assert!((h.state().map.camera().scale - before).abs() < 1e-9, "fit returns to the fitted view");
}

#[test]
fn simulate_menu_runs_each_analysis() {
    let mut h = common::harness(None);
    for (label, method) in [("Run pFBA", "pfba"), ("Run Loopless FBA", "loopless"), ("Run FBA", "fba")] {
        common::press(&mut h, "Simulate menu");
        common::press(&mut h, label);
        assert!(!egui::Popup::is_any_open(&h.ctx), "a menu item closes its menu");
        let run = h.state().store.last_run.clone().expect("a run");
        assert_eq!(run.method.as_str(), method, "{label}");
        // (The loopless MILP of this consortium is infeasible, as in the JS app.)
        assert_ne!(run.phase, coxeter_state::RunPhase::Running, "{label} finished");
        assert!(h.state().store.solution.is_some());
    }
}

#[test]
fn the_simulate_hub_runs_fva_and_shows_results() {
    let mut h = common::harness(None);
    common::click(&mut h, "Simulate");
    common::click(&mut h, "FVA");
    assert_eq!(h.state().store.last_run.as_ref().unwrap().method.as_str(), "fva");
    assert!(!h.state().store.fva_ranges.is_empty(), "FVA filled its ranges");
    common::click(&mut h, "View simulation results ↗");
    assert!(!h.state().ui.results_collapsed, "show-results opens the dock");
}

#[test]
fn a_reaction_of_the_results_table_is_selected_and_shown() {
    let mut h = common::harness(None);
    common::click(&mut h, "Table view");
    // The largest flux first: its row's link names the reaction by its model id.
    let rid = {
        let s = h.state().store.solution.clone().unwrap();
        let mut rows: Vec<(String, f64)> = s.fluxes.iter().map(|(k, v)| (k.clone(), *v)).collect();
        rows.sort_by(|a, b| b.1.abs().partial_cmp(&a.1.abs()).unwrap());
        rows[0].0.clone()
    };
    let local = h.state().store.community.as_ref().unwrap().reaction_owner.get(&rid).map(|o| o.original_id.clone()).unwrap();
    common::click_nth(&mut h, &local, 0);
    assert!(h.state().store.selected_reaction.is_some(), "select-result-reaction");
    assert!(!h.state().ui.inspector_collapsed, "the inspector shows it");
}
