//! The Edit Map cards through the real interface: Selection & geometry and
//! the View card (`displayCard`), the arrange block, Reaction colors &
//! pathway ribbons with the flux palette editor, the Legend, the Pathway
//! Manager, Map structure, Canvas and Find reaction — one test per
//! `data-action` they wire (`onSidebarClick` / `onSidebarInput` of
//! `src/ui/app.js`), checked on the Store as the JS changes it.
//!
//! Also here: the parts of `test/workspace-ui.test.mjs` about the shell and
//! the workspace pages (the task panel stays bounded, the Simulate hub's
//! labels, the View menu's state, what each page shows), and the GUI halves
//! the state crate left for the app (`js_ui_audit_fixes.rs`: drawing the
//! full model asks first only when it is too large; `js_flux_color_stops.rs`:
//! an invalid palette edit keeps the previous palette).
//!
//! The full View card (`displayCard(store)` without `workspace`) is no page
//! of the window; it is drawn alone around the real Shell.

mod common;

use coxeter_app::Shell;
use coxeter_app::panels::map_options;
use coxeter_app::ui_state::{LeftTab, Workspace};
use coxeter_canvas::Tool;
use coxeter_render::reaction_colors::{PathwayRibbonMode, ReactionColorMode};
use coxeter_state::StatusState;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

type H = Harness<'static, Shell>;

fn open(h: &mut H, ws: Workspace) {
    h.state_mut().open_workspace(ws);
    common::settle(h);
}

/// One card drawn alone around a booted Shell (what `Shell::frame` does
/// around the panels: fonts first, the store's events after).
fn card_harness(card: fn(&mut egui::Ui, &mut Shell)) -> H {
    let shell = common::shell(None);
    let mut fonts = false;
    let mut h = Harness::builder()
        .with_size(egui::vec2(520.0, 1500.0))
        .with_max_steps(40)
        .build_ui_state(
            move |ui, shell: &mut Shell| {
                if !fonts {
                    coxeter_app::fonts::install(ui.ctx());
                    fonts = true;
                    ui.ctx().request_repaint();
                    return;
                }
                for outcome in shell.store.poll_jobs() {
                    let _ = outcome;
                }
                egui::ScrollArea::vertical().show(ui, |ui| card(ui, shell));
                shell.take_reasons();
            },
            shell,
        );
    common::settle(&mut h);
    h
}

fn view_card(ui: &mut egui::Ui, shell: &mut Shell) {
    map_options::display_card(ui, shell, false);
}

/// A text shown exactly (a label, not a control: egui gives a label's text
/// as its value).
fn label_exact(h: &H, text: &str) -> bool {
    h.query_all_by(|n| n.role() == Role::Label && (n.value().as_deref() == Some(text) || n.label().as_deref() == Some(text))).next().is_some()
}

/// Empty a field and commit it (select all, delete, Enter).
fn clear_field(h: &mut H, label: &str) {
    common::control(h, label).focus();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.key_press(egui::Key::Backspace);
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    common::settle(h);
}

fn focused(h: &H) -> Option<egui::Id> {
    h.ctx.memory(|m| m.focused())
}

/// Movable metabolite nodes of the map, `n` of them, with distinct positions.
fn movable_nodes(h: &H, n: usize) -> Vec<String> {
    let store = &h.state().store;
    let map = store.map.as_ref().expect("a map");
    let mut out: Vec<String> = Vec::new();
    let mut seen: Vec<(f64, f64)> = Vec::new();
    for node in map.nodes().values() {
        if node.kind.as_str() != "metabolite" || !store.can_move_node(&node.id) {
            continue;
        }
        if seen.iter().any(|(x, y)| (x - node.x).abs() < 5.0 || (y - node.y).abs() < 5.0) {
            continue;
        }
        seen.push((node.x, node.y));
        out.push(node.id.clone());
        if out.len() == n {
            break;
        }
    }
    assert_eq!(out.len(), n, "movable nodes");
    out
}

fn node_xy(h: &H, id: &str) -> (f64, f64) {
    let node = h.state().store.map.as_ref().unwrap().nodes().get(id).unwrap().clone();
    (node.x, node.y)
}

fn drawn_reactions(h: &H, n: usize) -> Vec<String> {
    h.state().store.map.as_ref().unwrap().reaction_ids().into_iter().take(n).collect()
}

// Selection & geometry

#[test]
fn selection_card_toggles_geometry_edit_and_multi_select() {
    let mut h = common::harness(None);
    open(&mut h, Workspace::Map);
    assert!(common::shows(&h, "Select a reaction, or at least two objects, to align them"));
    assert!(!common::pressed(&h, "Edit geometry"));

    // toggle-geometry-edit
    let node = movable_nodes(&h, 1).remove(0);
    h.state_mut().store.toggle_multi_selection("node", &node);
    common::settle(&mut h);
    common::click(&mut h, "Edit geometry");
    let v = &h.state().store.view;
    assert!(v.geometry_edit && !v.multi_select && !v.text_tool);
    assert!(common::pressed(&h, "Edit geometry"));
    assert_eq!(h.state().ui.workspace, Some(Workspace::Map), "the page stays open");
    common::click(&mut h, "Edit geometry");
    assert!(!h.state().store.view.geometry_edit);

    // toggle-multi-select: clears the geometry selection, turns the others off.
    common::click(&mut h, "Edit geometry");
    let cell = h.state().store.map.as_ref().unwrap().cells()[0].id.clone();
    h.state_mut().store.select_cell_geometry(&cell);
    common::settle(&mut h);
    assert!(h.state().store.selected_geometry.is_some());
    common::click(&mut h, "Multi-select");
    let store = &h.state().store;
    assert!(store.view.multi_select && !store.view.geometry_edit && !store.view.text_tool);
    assert!(store.selected_geometry.is_none(), "clearGeometrySelection");
    assert!(common::pressed(&h, "Multi-select"));
    common::click(&mut h, "Multi-select");
    assert!(!h.state().store.view.multi_select);
}

#[test]
fn arrange_selection_aligns_and_distributes() {
    let mut h = common::harness(None);
    open(&mut h, Workspace::Map);
    let nodes = movable_nodes(&h, 3);

    // One object: no block (a lone metabolite is no line).
    h.state_mut().store.toggle_multi_selection("node", &nodes[0]);
    common::settle(&mut h);
    assert!(!common::has(&h, "Left"));
    // Two: aligned, not distributed.
    h.state_mut().store.toggle_multi_selection("node", &nodes[1]);
    common::settle(&mut h);
    assert!(common::shows(&h, "Arrange selection"));
    assert!(common::shows(&h, "2 objects"));
    assert!(!common::disabled(&h, "Left"));
    let distribute_h = format!("Distribute {}", coxeter_app::fonts::glyph::DISTRIBUTE_H);
    let distribute_v = format!("Distribute {}", coxeter_app::fonts::glyph::DISTRIBUTE_V);
    assert!(common::disabled(&h, &distribute_h), "three objects to distribute");
    assert!(common::disabled(&h, &distribute_v));
    h.state_mut().store.toggle_multi_selection("node", &nodes[2]);
    common::settle(&mut h);
    assert!(common::shows(&h, "3 objects"));
    assert!(!common::disabled(&h, &distribute_h));

    // align-horizontal: one y.
    common::click(&mut h, "Horizontal");
    let ys: Vec<f64> = nodes.iter().map(|n| node_xy(&h, n).1).collect();
    assert!(ys.iter().all(|y| (y - ys[0]).abs() < 1e-6), "{ys:?}");
    // align-vertical: one x.
    h.state_mut().store.undo_map();
    common::settle(&mut h);
    common::click(&mut h, "Vertical");
    let xs: Vec<f64> = nodes.iter().map(|n| node_xy(&h, n).0).collect();
    assert!(xs.iter().all(|x| (x - xs[0]).abs() < 1e-6), "{xs:?}");
    h.state_mut().store.undo_map();
    common::settle(&mut h);

    // distribute-horizontal / -vertical: even gaps between the centres.
    common::click(&mut h, &distribute_h);
    let mut xs: Vec<f64> = nodes.iter().map(|n| node_xy(&h, n).0).collect();
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert!(((xs[1] - xs[0]) - (xs[2] - xs[1])).abs() < 1e-6, "{xs:?}");
    common::click(&mut h, &distribute_v);
    let mut ys: Vec<f64> = nodes.iter().map(|n| node_xy(&h, n).1).collect();
    ys.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert!(((ys[1] - ys[0]) - (ys[2] - ys[1])).abs() < 1e-6, "{ys:?}");

    // align-left / -right / -top / -bottom: each moves something and is one undo step.
    for label in ["Left", "Right", "Top", "Bottom"] {
        let before: Vec<(f64, f64)> = nodes.iter().map(|n| node_xy(&h, n)).collect();
        let undo_before = h.state().store.map_history.undo_len();
        common::click(&mut h, label);
        let after: Vec<(f64, f64)> = nodes.iter().map(|n| node_xy(&h, n)).collect();
        assert_ne!(before, after, "{label}");
        assert_eq!(h.state().store.map_history.undo_len(), undo_before + 1, "{label} is one edit");
        h.state_mut().store.undo_map();
        common::settle(&mut h);
    }
    let lefts: Vec<f64> = {
        common::click(&mut h, "Left");
        let map = h.state().store.map.as_ref().unwrap();
        nodes.iter().map(|n| map.nodes().get(n).map(|node| node.x - node.r).unwrap()).collect()
    };
    assert!(lefts.iter().all(|x| (x - lefts[0]).abs() < 1e-6), "left edges: {lefts:?}");
}

// View card

#[test]
fn view_card_toggles() {
    let mut h = card_harness(view_card);
    assert!(common::shows(&h, "View"));
    // toggle-currency
    let currency = h.state().store.view.show_currency;
    common::click(&mut h, "Cofactors");
    assert_eq!(h.state().store.view.show_currency, !currency);
    assert_eq!(common::pressed(&h, "Cofactors"), !currency);
    // toggle-crossfeed
    common::click(&mut h, "Highlight cross-feeding");
    assert!(h.state().store.view.show_cross_feeding);
    assert!(common::pressed(&h, "Highlight cross-feeding"));
    // toggle-dark
    common::click(&mut h, "Dark");
    assert!(h.state().store.view.dark);
    common::click(&mut h, "Dark");
    assert!(!h.state().store.view.dark);
    // toggle-geometry-edit
    common::click(&mut h, "Edit geometry");
    assert!(h.state().store.view.geometry_edit);
    assert!(common::shows(&h, "Geometry editor on."));
    // toggle-multi-select
    common::click(&mut h, "Multi-select");
    let v = &h.state().store.view;
    assert!(v.multi_select && !v.geometry_edit);
    assert!(common::shows(&h, "Multi-select on."));
    assert!(common::shows(&h, "Selected: 0."));
    // toggle-text-tool: clears the multi-selection too.
    let node = movable_nodes(&h, 1).remove(0);
    h.state_mut().store.toggle_multi_selection("node", &node);
    common::settle(&mut h);
    common::click(&mut h, "Text");
    let store = &h.state().store;
    assert!(store.view.text_tool && !store.view.multi_select && !store.view.geometry_edit);
    assert_eq!(store.multi_selection_size(), 0, "clearMultiSelection");
    assert!(common::shows(&h, "Text tool on."));
    common::click(&mut h, "Text");
    assert!(!h.state().store.view.text_tool);
}

#[test]
fn view_card_colors_ribbons_and_labels() {
    let mut h = card_harness(view_card);
    // reaction-color-mode
    common::choose(&mut h, "Reaction color", "Pathway");
    assert_eq!(h.state().store.view.reaction_color_mode, ReactionColorMode::Pathway);
    // pathway-ribbon-mode: a ribbon brings the arrows back to flux colours.
    assert!(!common::has(&h, "Pathway ribbon width"));
    common::choose(&mut h, "Pathway ribbons", "Subsystem");
    assert_eq!(h.state().store.view.pathway_ribbon_mode, PathwayRibbonMode::Subsystem);
    assert_eq!(h.state().store.view.reaction_color_mode, ReactionColorMode::Flux);
    // pathway-ribbon-width / -opacity
    common::set_value(&mut h, "Pathway ribbon width", 80.0);
    assert_eq!(h.state().store.view.pathway_ribbon_width, 80.0);
    assert!(common::shows(&h, "80 px"));
    common::set_value(&mut h, "Pathway ribbon opacity", 0.3);
    assert!((h.state().store.view.pathway_ribbon_opacity - 0.3).abs() < 1e-9);
    assert!(common::shows(&h, "30%"));
    // labels
    common::choose(&mut h, "Map labels", "None");
    assert_eq!(h.state().store.view.labels, "none");
}

#[test]
fn view_card_map_buttons() {
    let mut h = card_harness(view_card);
    let drawn = h.state().store.map.as_ref().unwrap().reactions().len();
    assert!(common::disabled(&h, "↶ Undo"));
    // map-empty
    common::click(&mut h, "Empty map");
    assert_eq!(h.state().store.map.as_ref().unwrap().reactions().len(), 0);
    assert!(h.state().store.status().message.contains(&format!("{drawn} drawn reactions removed")));
    // map-undo / map-redo
    assert!(!common::disabled(&h, "↶ Undo"));
    common::click(&mut h, "↶ Undo");
    assert_eq!(h.state().store.map.as_ref().unwrap().reactions().len(), drawn);
    common::click(&mut h, "↷ Redo");
    assert_eq!(h.state().store.map.as_ref().unwrap().reactions().len(), 0);
    // map-full
    common::click(&mut h, "Full map");
    assert_eq!(h.state().store.map_mode, "full");
    assert!(!h.state().store.map.as_ref().unwrap().reactions().is_empty());
    // show-hidden-metabolites
    let node = movable_nodes(&h, 1).remove(0);
    h.state_mut().store.set_map_node_hidden(&node, true);
    common::settle(&mut h);
    common::click(&mut h, "Show hidden metabolites (1)");
    assert_eq!(h.state().store.hidden_metabolite_count(), 0);
    assert!(!common::has(&h, "Show hidden metabolites (1)"));
    // The arrange block appears with two objects.
    for id in movable_nodes(&h, 2) {
        h.state_mut().store.toggle_multi_selection("node", &id);
    }
    common::settle(&mut h);
    assert!(common::shows(&h, "Arrange selection"));
}

// Reaction colours & pathway ribbons

#[test]
fn appearance_card_modes_and_ribbons() {
    let mut h = common::harness(None);
    open(&mut h, Workspace::Map);
    assert!(common::shows(&h, "Flux palette"), "the palette editor shows in Flux mode");
    // color-by-flux (7b5aaa0): off colours lines by pathway, or by the group last chosen.
    common::click(&mut h, "Color lines by flux");
    assert_eq!(h.state().store.view.reaction_color_mode, ReactionColorMode::Pathway);
    common::choose(&mut h, "Reaction color mode", "Subsystem");
    assert_eq!(h.state().store.view.reaction_color_mode, ReactionColorMode::Subsystem);
    assert_eq!(h.state().store.view.line_color_group, "subsystem");
    assert!(common::shows(&h, "A color set on a reaction in the Inspector is used over its pathway's. Your flux palette is retained."));
    assert!(!common::shows(&h, "Flux palette"));
    common::choose(&mut h, "Pathway ribbon mode", "Pathway");
    assert_eq!(h.state().store.view.pathway_ribbon_mode, PathwayRibbonMode::Pathway);
    assert_eq!(h.state().store.view.reaction_color_mode, ReactionColorMode::Flux, "ribbons return the arrows to flux colours");
    common::set_value(&mut h, "Pathway ribbon width", 40.0);
    assert_eq!(h.state().store.view.pathway_ribbon_width, 40.0);
    common::set_value(&mut h, "Pathway ribbon opacity", 0.12);
    assert!((h.state().store.view.pathway_ribbon_opacity - 0.12).abs() < 1e-9);
    assert!(common::shows(&h, "12%"));
    common::click(&mut h, "Color lines by flux");
    assert_eq!(h.state().store.view.reaction_color_mode, ReactionColorMode::Subsystem, "back to the group last chosen");
    common::choose(&mut h, "Pathway ribbon mode", "Off");
    assert_eq!(h.state().store.view.pathway_ribbon_mode, PathwayRibbonMode::Off);
    assert_eq!(h.state().store.view.reaction_color_mode, ReactionColorMode::Subsystem, "Off leaves the colour mode alone");
    assert!(!common::has(&h, "Pathway ribbon width"));
}

#[test]
fn flux_palette_editor() {
    let mut h = common::harness(None);
    open(&mut h, Workspace::Map);
    assert!(common::pressed(&h, "Automatic"));
    assert!(!common::pressed(&h, "Custom colors"));
    assert!(common::shows(&h, "Automatic flux magnitude color gradient, 0 to "));
    let reference = h.state().store.scale.as_ref().unwrap().reference;

    // flux-colors-customize: a signed palette around the scale, the first value focused.
    common::click(&mut h, "Custom colors");
    let stops = h.state().store.view.flux_color_stops.clone().expect("custom stops");
    let values: Vec<f64> = stops.iter().map(|s| s.value).collect();
    assert_eq!(values, vec![-reference, 0.0, reference]);
    assert_eq!(stops.iter().map(|s| s.color.as_str()).collect::<Vec<_>>(), vec!["#2563eb", "#cbd5e1", "#dc2626"]);
    assert!(common::pressed(&h, "Custom colors"));
    assert!(common::shows(&h, "3 points"));
    assert!(common::shows(&h, "Signed flux color gradient"));
    assert_eq!(focused(&h), Some(map_options::flux_color_value_id(0)), "focusFluxColorPoint(0)");

    // flux-color-value
    common::type_into(&mut h, "Flux value at point 3", "5");
    let values: Vec<f64> = h.state().store.view.flux_color_stops.as_ref().unwrap().iter().map(|s| s.value).collect();
    assert_eq!(values, vec![-reference, 0.0, 5.0]);

    // An empty value is refused; the map keeps the last valid palette.
    let before = h.state().store.view.flux_color_stops.clone();
    clear_field(&mut h, "Flux value at point 2");
    assert_eq!(h.state().store.view.flux_color_stops, before);
    let draft = h.state().ui.flux_color_draft.clone().unwrap();
    assert_eq!(draft.error, "Enter a finite flux value for every point. The map keeps the last valid palette.");
    assert_eq!(draft.invalid, vec![1]);
    assert!(common::shows(&h, "Enter a finite flux value for every point."));
    assert_eq!(draft.rows[1].value, "", "the field keeps what was typed");
    common::type_into(&mut h, "Flux value at point 2", "1");
    assert!(h.state().ui.flux_color_draft.as_ref().unwrap().error.is_empty());
    assert!(!common::shows(&h, "Enter a finite flux value for every point."));

    // A duplicate value is refused too.
    let before = h.state().store.view.flux_color_stops.clone();
    common::type_into(&mut h, "Flux value at point 1", "5");
    assert_eq!(h.state().store.view.flux_color_stops, before);
    let draft = h.state().ui.flux_color_draft.clone().unwrap();
    assert_eq!(draft.error, "Each color point needs a unique flux value. The map keeps the last valid palette.");
    assert_eq!(draft.invalid, vec![0, 2]);
    common::type_into(&mut h, "Flux value at point 1", "-2");
    let values: Vec<f64> = h.state().store.view.flux_color_stops.as_ref().unwrap().iter().map(|s| s.value).collect();
    assert_eq!(values, vec![-2.0, 1.0, 5.0]);

    // flux-color-color
    common::type_into(&mut h, "Color at flux point 2", "#00ff00");
    assert_eq!(h.state().store.view.flux_color_stops.as_ref().unwrap()[1].color, "#00ff00");

    // flux-color-add: past the top end, in the top colour, focused.
    common::click(&mut h, "+ Add color point");
    let stops = h.state().store.view.flux_color_stops.clone().unwrap();
    assert_eq!(stops.iter().map(|s| s.value).collect::<Vec<_>>(), vec![-2.0, 1.0, 5.0, 9.0]);
    assert_eq!(stops[3].color, "#dc2626");
    assert!(common::shows(&h, "4 points"));
    assert_eq!(focused(&h), Some(map_options::flux_color_value_id(3)));

    // flux-color-remove
    common::click(&mut h, "Remove flux point 1");
    let values: Vec<f64> = h.state().store.view.flux_color_stops.as_ref().unwrap().iter().map(|s| s.value).collect();
    assert_eq!(values, vec![1.0, 5.0, 9.0]);
    common::click(&mut h, "Remove flux point 1");
    common::click(&mut h, "Remove flux point 1");
    assert_eq!(h.state().store.view.flux_color_stops.as_ref().unwrap().len(), 1);
    assert!(common::disabled(&h, "Remove flux point 1"), "the last point stays");

    // flux-colors-reset
    common::click(&mut h, "Automatic");
    assert_eq!(h.state().store.view.flux_color_stops, None);
    assert_eq!(h.state().ui.flux_color_draft, None);
    assert!(common::pressed(&h, "Automatic"));
    assert!(!common::has(&h, "Flux value at point 1"));
}

#[test]
fn an_imported_palette_replaces_the_draft() {
    let mut h = common::harness(None);
    open(&mut h, Workspace::Map);
    common::click(&mut h, "Custom colors");
    clear_field(&mut h, "Flux value at point 2");
    assert!(!h.state().ui.flux_color_draft.as_ref().unwrap().error.is_empty());
    // Another palette arrives (a map or scenario import).
    let stops = vec![
        coxeter_render::theme::FluxColorStop { value: -3.0, color: "#000000".into() },
        coxeter_render::theme::FluxColorStop { value: 3.0, color: "#ffffff".into() },
    ];
    h.state_mut().store.set_flux_color_stops(Some(stops)).unwrap();
    common::settle(&mut h);
    assert_eq!(h.state().ui.flux_color_draft, None);
    assert!(common::shows(&h, "2 points"));
    assert!(!common::shows(&h, "Enter a finite flux value"));
}

// Legend

#[test]
fn legend_card_follows_the_colour_modes() {
    let mut h = common::harness(None);
    open(&mut h, Workspace::Map);
    assert!(common::shows(&h, "Legend"));
    assert!(common::shows(&h, "Shared medium"));
    let cell_label = h.state().store.map.as_ref().unwrap().cells()[0].compartments[0].label.clone();
    assert!(common::shows(&h, &cell_label), "compartments of the drawn cells");
    assert!(common::shows(&h, "|flux| mmol·gDW⁻¹·h⁻¹"));
    assert!(!common::shows(&h, "of each species"), "not a SteadyCom solution");
    assert!(common::shows(&h, "Flux colors encode magnitude; arrowheads show the actual direction in the solution."));

    h.state_mut().store.set_view(coxeter_state::ViewPatch { reaction_color_mode: Some("subsystem".into()), ..Default::default() }).unwrap();
    common::settle(&mut h);
    assert!(label_exact(&h, "Subsystems"));
    let groups = h.state().store.reaction_coloring(true).legend;
    assert!(label_exact(&h, &groups.len().to_string()));
    assert!(common::shows(&h, &groups[0].label));
    assert!(common::shows(&h, "Color marks the subsystem; arrowheads show the direction of the solved flux."));

    h.state_mut().store.set_view(coxeter_state::ViewPatch { pathway_ribbon_mode: Some("pathway".into()), ..Default::default() }).unwrap();
    common::settle(&mut h);
    assert!(label_exact(&h, "Pathways"));
    assert!(common::shows(&h, "The translucent ribbon marks the pathway; arrow colors encode flux magnitude."));

    h.state_mut()
        .store
        .set_view(coxeter_state::ViewPatch { pathway_ribbon_mode: Some("off".into()), reaction_color_mode: Some("flux".into()), ..Default::default() })
        .unwrap();
    common::settle(&mut h);
    common::click(&mut h, "Custom colors");
    assert!(common::shows(&h, "Flux colors represent signed values; arrowheads show the solved direction."));
    let previews = h.query_all_by(|n| n.label().is_some_and(|l| l.starts_with("Signed flux color gradient"))).count();
    assert_eq!(previews, 2, "the editor and the legend preview the palette");
}

// Pathway manager

#[test]
fn pathway_manager_rows() {
    let mut h = common::harness(None);
    let reactions = drawn_reactions(&h, 3);
    let pathway = h.state_mut().store.create_manual_pathway("Test path", Some("#ff0000"), Some(&reactions[..2])).unwrap();
    let key = format!("manual:{}", pathway.id);
    open(&mut h, Workspace::Map);
    assert!(common::shows(&h, "Test path"));
    assert!(common::shows(&h, "Manual · 2 drawn reactions"));

    // pathway-visible
    common::click(&mut h, "Show ribbon for Test path");
    assert!(!h.state().store.manual_pathways[&pathway.id].visible);
    common::click(&mut h, "Show ribbon for Test path");
    assert!(h.state().store.manual_pathways[&pathway.id].visible);
    // pathway-color
    common::type_into(&mut h, "Color of Test path", "#00ff00");
    assert_eq!(h.state().store.manual_pathways[&pathway.id].color, "#00ff00");
    // pathway-name: an empty name is refused, with the store's message.
    clear_field(&mut h, "Name of Test path");
    assert_eq!(h.state().store.manual_pathways[&pathway.id].name, "Test path");
    assert_eq!(h.state().store.status().state, StatusState::Error);
    assert_eq!(h.state().store.status().message, "The pathway name cannot be empty.");
    common::type_into(&mut h, "Name of Test path", "Renamed");
    assert_eq!(h.state().store.manual_pathways[&pathway.id].name, "Renamed");

    // pathway-select: closes the page and selects the reactions.
    common::click(&mut h, "Select Renamed");
    assert_eq!(h.state().ui.workspace, None);
    let selected: Vec<String> = h.state().store.multi_selection.reactions.iter().cloned().collect();
    assert_eq!(selected, h.state().store.pathway_reaction_ids(&key));
    assert!(h.state().store.view.multi_select);

    // pathway-focus: selects and centres.
    h.state_mut().store.clear_multi_selection(true);
    open(&mut h, Workspace::Map);
    let camera = h.state().map.camera();
    common::click(&mut h, "Focus Renamed");
    assert_eq!(h.state().ui.workspace, None);
    assert_eq!(h.state().store.multi_selection.reactions.len(), 2);
    assert_ne!(h.state().map.camera(), camera, "centerOnPathway");

    // pathway-delete
    open(&mut h, Workspace::Map);
    common::click(&mut h, "Delete Renamed");
    assert!(h.state().store.manual_pathways.is_empty());
    assert!(!common::shows(&h, "Renamed"));
}

#[test]
fn pathway_manager_creates_from_the_selection() {
    let mut h = common::harness(None);
    open(&mut h, Workspace::Map);
    assert!(common::shows(&h, "Select one or more reactions to create a manual pathway."));
    assert!(!common::has(&h, "New pathway name"));

    // pathway-enable-selection
    let cell = h.state().store.map.as_ref().unwrap().cells()[0].id.clone();
    h.state_mut().store.select_cell_geometry(&cell);
    common::settle(&mut h);
    common::click(&mut h, "Enable multi-select");
    let store = &h.state().store;
    assert_eq!(h.state().ui.workspace, None);
    assert!(store.view.multi_select && !store.view.geometry_edit && !store.view.text_tool);
    assert!(store.selected_geometry.is_none());

    // pathway-create
    let reactions = drawn_reactions(&h, 2);
    for rid in &reactions {
        h.state_mut().store.toggle_multi_selection("reaction", rid);
    }
    open(&mut h, Workspace::Map);
    assert!(common::shows(&h, "Create pathway from selection"));
    assert!(common::shows(&h, "2 reactions"));
    common::click(&mut h, "Create");
    assert_eq!(h.state().store.status().message, "Enter a pathway name.", "the store's error, in the status line");
    assert!(h.state().store.manual_pathways.is_empty());
    common::type_into(&mut h, "New pathway name", "Glyco");
    common::type_into(&mut h, "New pathway color", "#123456");
    let camera = h.state().map.camera();
    common::click(&mut h, "Create");
    let pathway = h.state().store.manual_pathways.values().next().cloned().expect("created");
    assert_eq!(pathway.name, "Glyco");
    assert_eq!(pathway.color, "#123456");
    assert_eq!(pathway.reaction_ids.iter().cloned().collect::<Vec<_>>(), reactions);
    assert_eq!(h.state().store.view.pathway_ribbon_mode, PathwayRibbonMode::Pathway);
    assert_ne!(h.state().map.camera(), camera, "centred on the new pathway");
    assert!(h.state().ui.drafts.get("map:pathway-create").is_none(), "the form starts empty again");
    assert!(common::shows(&h, "Glyco"));
}

// Map structure

#[test]
fn map_structure_card_places_cells_and_compartments() {
    let mut h = common::harness(None);
    open(&mut h, Workspace::Structure);
    let cells = h.state().store.map.as_ref().unwrap().cells().len();
    assert!(common::shows(&h, &format!("{cells} cells · ")));
    assert!(common::shows(&h, "Every active species already has a cell boundary drawn."));

    h.state_mut().store.show_empty_map();
    common::settle(&mut h);
    let available = h.state().store.available_map_cells();
    assert!(available.len() >= 2);
    assert!(common::shows(&h, "0 cells · 0 compartments"));
    assert!(common::shows(&h, "Nothing can be drawn yet: draw a cell first, or every compartment is already on the map."));

    // cell-placement-species
    let second = &available[1];
    let option = format!("{} · boundary {} ({})", second.species_name, second.outer_compartment_id, second.outer_compartment_name);
    common::choose(&mut h, "Species to draw a cell for", &option);
    assert_eq!(h.state().store.cell_placement_species_id.as_deref(), Some(second.species_id.as_str()));

    // compartment-placement and add-map-compartment ("Place automatically").
    let species = available[0].species_id.clone();
    h.state_mut().store.add_map_cell(&species, None).expect("a cell");
    common::settle(&mut h);
    let compartments = h.state().store.available_map_compartments();
    assert!(!compartments.is_empty());
    let last = compartments.last().unwrap().clone();
    // Every compartment of the model is listed, with why it can be drawn or not (d7b2c0e).
    let choices = h.state().store.map_compartment_choices();
    let many = choices.iter().any(|c| c.species_id != choices[0].species_id);
    let choice = choices.iter().find(|c| c.key() == last.key()).unwrap();
    let who = if many { format!("{} · ", choice.species_name) } else { String::new() };
    let option = format!("{who}{} ({}) — {}", choice.compartment_name, choice.compartment_id, choice.reason);
    common::choose(&mut h, "Compartment to add", &option);
    assert_eq!(h.state().store.compartment_placement_key.as_deref(), Some(last.key().as_str()));
    let before = h.state().store.map.as_ref().unwrap().cell(&last.species_id).unwrap().compartments.len();
    common::click(&mut h, "Place automatically");
    let cell = h.state().store.map.as_ref().unwrap().cell(&last.species_id).unwrap().clone();
    assert_eq!(cell.compartments.len(), before + 1);
    assert!(cell.compartments.iter().any(|c| c.id == last.compartment_id));
    assert_eq!(h.state().ui.workspace, Some(Workspace::Structure), "placing keeps the page");

    // tool-compartment ("Draw compartment")
    h.state_mut().ui.left_tab = LeftTab::Model;
    common::click(&mut h, "Draw compartment");
    assert_eq!(h.state().map.tool(), Tool::Compartment);
    assert_eq!(h.state().ui.workspace, None);
    assert_eq!(h.state().ui.left_tab, LeftTab::Build);
}

// Canvas

#[test]
fn canvas_card_clears_and_draws_the_map() {
    let (mut h, dialogs, _dir) = common::harness_saving("map-options-canvas", None);
    open(&mut h, Workspace::Structure);
    let map = h.state().store.map.as_ref().unwrap();
    let size = format!("{} × {}", coxeter_core::js::round(map.width()), coxeter_core::js::round(map.height()));
    assert!(common::shows(&h, &size));
    let drawn = map.reactions().len();
    // map-empty
    common::click(&mut h, "Clear drawing");
    assert_eq!(h.state().ui.workspace, None);
    assert_eq!(h.state().store.map.as_ref().unwrap().reactions().len(), 0);
    assert_eq!(
        h.state().store.status().message,
        format!("Map cleared — {drawn} drawn reactions removed. The model is unchanged; Ctrl+Z undoes this.")
    );
    // map-full: a core model draws without asking (ui-audit-fixes).
    open(&mut h, Workspace::Map);
    common::click(&mut h, "Draw full model");
    assert_eq!(h.state().ui.workspace, None);
    assert_eq!(h.state().store.map_mode, "full");
    assert!(!h.state().store.map.as_ref().unwrap().reactions().is_empty());
    assert!(h.state().store.full_map_warning().is_none());
    assert!(dialogs.with(|d| d.asked.iter().all(|a| !a.starts_with("confirm"))), "nothing asked");
}

// Reaction search

#[test]
fn reaction_search_adds_and_selects() {
    let mut h = common::harness(None);
    open(&mut h, Workspace::Library);
    assert!(common::shows(&h, "Type to search every reaction in the model, drawn or not."));
    let rid = drawn_reactions(&h, 1).remove(0);
    let local = h.state().store.search_reactions(&rid, 30).into_iter().find(|r| r.reaction_id == rid).unwrap().local_id;

    // reaction-search: live.
    common::type_into(&mut h, map_options::REACTION_SEARCH_LABEL, "zzzz-no-such");
    assert_eq!(h.state().store.reaction_search_query, "zzzz-no-such");
    assert!(common::shows(&h, "No reactions found for “zzzz-no-such”."));
    common::type_into(&mut h, map_options::REACTION_SEARCH_LABEL, &local);
    assert!(common::shows(&h, "On map"));

    // search-select-reaction
    common::click(&mut h, &format!("Select {rid}"));
    assert_eq!(h.state().ui.workspace, None);
    assert_eq!(h.state().store.selected_reaction.as_deref(), Some(rid.as_str()));

    // search-add-reaction, on an empty map.
    h.state_mut().store.show_empty_map();
    h.state_mut().store.select(None);
    open(&mut h, Workspace::Library);
    assert!(common::shows(&h, "Not drawn"));
    let camera = h.state().map.camera();
    common::click(&mut h, &format!("Add {rid} to map"));
    assert_eq!(h.state().ui.workspace, None);
    assert!(h.state().store.map.as_ref().unwrap().has_reaction(&rid));
    assert_eq!(h.state().store.selected_reaction.as_deref(), Some(rid.as_str()));
    assert_ne!(h.state().map.camera(), camera, "centerOnReaction");
}

#[test]
fn find_reaction_puts_the_keyboard_in_the_search_field() {
    let mut h = common::harness(None);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::F);
    common::settle(&mut h);
    assert_eq!(h.state().ui.workspace, Some(Workspace::Library));
    assert_eq!(focused(&h), Some(map_options::reaction_search_id()));
    assert_eq!(h.state().ui.focus_request, None, "the request was taken");
    // Typing goes straight into the search.
    h.event(egui::Event::Text("pgi".into()));
    common::settle(&mut h);
    assert_eq!(h.state().store.reaction_search_query, "pgi");
}

// workspace-ui.test.mjs: the shell side

/// "task navigation stays bounded as a community and its medium grow".
#[test]
fn task_navigation_stays_bounded_as_the_community_grows() {
    let mut h = common::harness(None);
    let controls = |h: &H| -> Vec<String> {
        let mut labels: Vec<String> = h
            .query_all_by(|n| !matches!(n.role(), Role::Label | Role::Unknown | Role::GenericContainer))
            .filter_map(|n| n.accesskit_node().label())
            .collect();
        labels.sort();
        labels
    };
    let tabs = [(LeftTab::Build, "Edit Map"), (LeftTab::Model, "Edit Model"), (LeftTab::Analysis, "Simulate")];
    let mut before = Vec::new();
    for (_, label) in tabs {
        common::click(&mut h, label);
        before.push(controls(&h));
    }
    {
        let store = &mut h.state_mut().store;
        store.species = (0..1000).map(|i| coxeter_state::Species::new(&format!("species_{i}"), "none", 1.0, i % 2 == 0)).collect();
        store.medium = (0..500).map(|i| (format!("nutrient_{i}"), i as f64)).collect();
    }
    common::settle(&mut h);
    for (i, (_, label)) in tabs.into_iter().enumerate() {
        common::click(&mut h, label);
        assert_eq!(controls(&h), before[i], "{label} controls grow with model data");
        assert!(!common::shows(&h, "nutrient_499") && !common::shows(&h, "species_999"), "{label}: no per-item controls");
    }
    common::click(&mut h, "Edit Model");
    assert!(common::shows(&h, "500 active species"));
}

/// "current analysis labels reflect actual method and biomass target without changing state".
#[test]
fn simulate_hub_labels_reflect_the_method_without_changing_state() {
    let mut h = common::harness(None);
    common::click(&mut h, "Simulate");
    type Setup = fn(&mut coxeter_state::Store);
    let variants: [(Setup, &str); 5] = [
        (|_| {}, "FBA"),
        (
            |s| {
                s.parsimonious = true;
                s.pfba_biomass_percent = 87.5;
            },
            "Biomass target: 87.5% of optimum",
        ),
        (
            |s| {
                s.parsimonious = true;
                s.fva_enabled = true;
            },
            "pFBA + FVA",
        ),
        (|s| s.loopless = true, "Loopless FBA"),
        (
            |s| {
                s.omics.gimme_enabled = true;
                s.omics.gimme_objective_fraction = 0.8;
            },
            "Objective target: 80%",
        ),
    ];
    for (setup, label) in variants {
        {
            let s = &mut h.state_mut().store;
            s.parsimonious = false;
            s.pfba_biomass_percent = 100.0;
            s.fva_enabled = false;
            s.loopless = false;
            s.omics.gimme_enabled = false;
            s.omics.gimme_objective_fraction = 0.9;
            setup(s);
        }
        let snapshot = |h: &H| {
            let s = &h.state().store;
            (s.parsimonious, s.pfba_biomass_percent, s.loopless, s.fva_enabled, s.solution.as_ref().map(|x| x.objective_value), s.omics.gimme_enabled)
        };
        let before = snapshot(&h);
        common::settle(&mut h);
        assert!(label_exact(&h, label), "{label}");
        assert_eq!(snapshot(&h), before, "{label}: drawing changed nothing");
    }
}

/// "View menu exposes presentation controls with their actual selected state"
/// (and, from ui-audit-fixes, "label options … say what they do": the
/// options themselves cannot be listed here, as opening the select closes
/// the View menu in the harness).
#[test]
fn view_menu_shows_the_actual_state() {
    let mut h = common::harness(None);
    {
        let v = &mut h.state_mut().store.view;
        v.show_currency = false;
        v.show_cross_feeding = true;
        v.labels = "none".into();
    }
    common::settle(&mut h);
    common::press(&mut h, "View menu");
    assert!(!common::pressed(&h, "Cofactors"));
    assert!(common::pressed(&h, "Highlight cross-feeding"));
    assert_eq!(common::control(&h, "Map labels").value().as_deref(), Some("None"));
    assert!(!common::has(&h, "Dark"), "no theme toggle in the View menu");
    assert!(!common::shows(&h, "Primary only"));
}

/// What each workspace page shows (`renderWorkspace`), for the map's pages.
#[test]
fn map_workspace_pages_show_their_cards() {
    let mut h = common::harness(None);
    open(&mut h, Workspace::Library);
    assert!(common::has(&h, map_options::REACTION_SEARCH_LABEL));
    assert!(!common::shows(&h, "Pathway Manager") && !common::shows(&h, "Map structure"));
    open(&mut h, Workspace::Structure);
    assert!(common::shows(&h, "Map structure") && common::has(&h, "Clear drawing"));
    assert!(!common::has(&h, map_options::REACTION_SEARCH_LABEL));
    open(&mut h, Workspace::Map);
    for title in ["Selection & geometry", "Reaction lines, metabolites & pathway ribbons", "Pathway Manager", "Canvas", "Legend"] {
        assert!(common::shows(&h, title), "{title}");
    }
    assert!(!common::shows(&h, "Nested compartments"));
    assert!(common::has(&h, "Edit geometry") && common::has(&h, "Color lines by flux"));
}

// Line styles (7b5aaa0)

/// line-styles: the line switches (`width-by-flux`, `dash-no-flux`,
/// `line-hops`) and the map's circle colours (`node-style-fill`,
/// `node-style-stroke`, `node-style-reset`).
#[test]
fn line_switches_and_circle_colours_of_the_appearance_card() {
    let mut h = common::harness(None);
    open(&mut h, Workspace::Map);
    let v = h.state().store.view.clone();
    assert_eq!((v.width_by_flux, v.dash_no_flux, v.line_hops), (false, true, true));
    common::click(&mut h, "Line width follows flux");
    common::click(&mut h, "Grey dashed lines");
    common::click(&mut h, "Hop where lines cross");
    let v = h.state().store.view.clone();
    assert_eq!((v.width_by_flux, v.dash_no_flux, v.line_hops), (true, false, false));
    let state = h.state().store.map_render_state(Default::default());
    assert_eq!(state.reaction_width_mode, coxeter_render::ReactionWidthMode::Flux);
    assert!(!state.off_style && !state.line_hops);

    assert!(common::disabled(&h, "Theme colors"), "nothing to reset yet");
    common::type_into(&mut h, "Fill of metabolite circles", "#aabbcc");
    common::type_into(&mut h, "Outline of metabolite circles", "#112233");
    let style = h.state().store.map.as_ref().unwrap().graph().node_style();
    assert_eq!((style.fill.as_deref(), style.stroke.as_deref()), (Some("#aabbcc"), Some("#112233")));
    common::click(&mut h, "Theme colors");
    assert_eq!(h.state().store.map.as_ref().unwrap().graph().node_style(), Default::default());
}

/// line-styles: a swept selection is coloured from the View card
/// (`selectionColorBlock`: `node-fill`, `reaction-line-color`).
#[test]
fn a_selection_is_coloured_from_the_view_card() {
    let mut h = common::harness(None);
    let (node, rid) = {
        let map = h.state().store.map.as_ref().unwrap();
        let node = map.nodes().values().find(|n| n.is_metabolite_kind()).unwrap().id.clone();
        (node, map.reactions().keys().next().unwrap().clone())
    };
    {
        let store = &mut h.state_mut().store;
        store.toggle_multi_selection("node", &node);
        store.toggle_multi_selection("reaction", &rid);
    }
    open(&mut h, Workspace::Map);
    assert!(common::shows(&h, "Color selection"));
    common::type_into(&mut h, "Circle fill", "#ff0000");
    common::type_into(&mut h, "Reaction line color", "#123456");
    let map = h.state().store.map.as_ref().unwrap();
    assert_eq!(map.node(&node).unwrap().own_fill(), Some("#ff0000"));
    assert_eq!(map.reaction(&rid).unwrap().own_color(), Some("#123456"));
}
