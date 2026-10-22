//! The Edit Map cards (port of `displayCard`, `selectionArrangeControls`,
//! `arrangeSelectionBlock`, `mapAppearanceCard`, `fluxColorEditor`,
//! `renderFluxColorPreview`, `legendCard`, `pathwayManagerCard`,
//! `mapStructureCard`, `mapCanvasCard`, `reactionSearchCard` and
//! `renderReactionSearchResults` of `src/ui/panels.js`), with the handlers
//! of `onSidebarClick` / `onSidebarInput` and the flux colour editor of
//! `src/ui/app.js` (`readFluxColorRows`, `fluxColorValues`,
//! `commitFluxColorRows`, `patchFluxColorEditor`, `newFluxColorValue`,
//! `focusFluxColorPoint`, `paintReactionSearchResults`).
//!
//! In the JS the DOM is the flux colour editor's memory between a keystroke
//! and its `change`. Here the rows on screen are `ui.flux_color_draft` when
//! there is one, the store's palette otherwise, and the text being typed
//! lives in the field until it is committed (Enter or leaving it), which is
//! when the JS reads the rows back (`readFluxColorRows`).

use coxeter_canvas::Tool;
use coxeter_core::js;
use coxeter_map::editable_map::ColorChange;
use coxeter_render::reaction_colors::{PathwayRibbonMode, ReactionColorMode};
use coxeter_render::theme::{FluxColorStop, format_flux};
use coxeter_state::pathways::PathwayPatch;
use coxeter_state::{Store, ViewPatch};
use egui::{Color32, CornerRadius, Frame, Margin, RichText, Sense, Stroke, Ui, Vec2};

use crate::app::Shell;
use crate::fonts::glyph;
use crate::theme::{Palette, bold, css_color};
use crate::ui_state::{FluxColorDraft, FluxColorRow, FormValues};
use crate::widgets::{self, Btn, Field, FieldEvent, aria_pressed};

/// The id of the reaction search field (`#reaction-search-input`), which
/// Ctrl+F puts the keyboard in.
pub fn reaction_search_id() -> egui::Id {
    egui::Id::new("reaction-search-input")
}

/// The flux value field of the palette's point `index`
/// (`[data-action="flux-color-value"][data-flux-color-index]`).
pub fn flux_color_value_id(index: usize) -> egui::Id {
    egui::Id::new(("flux-color-value", index))
}

// Display

/// `displayCard(store, { workspace })`.
pub fn display_card(ui: &mut Ui, shell: &mut Shell, workspace: bool) {
    if workspace {
        widgets::card(ui, "Selection & geometry", None, |ui| {
            let v = shell.store.view.clone();
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::splat(6.0);
                if widgets::toggle_button(ui, "Edit geometry", v.geometry_edit, "Move and resize cells and compartments").clicked() {
                    toggle_geometry_edit(shell);
                }
                if widgets::toggle_button(ui, "Multi-select", v.multi_select, "Select and move several metabolites and reactions").clicked() {
                    toggle_multi_select(shell);
                }
            });
            selection_color_block(ui, shell);
        });
        return;
    }

    widgets::card(ui, "View", None, |ui| {
        let v = shell.store.view.clone();
        if v.geometry_edit {
            widgets::hint_rich(ui, &["", "Geometry editor on.", " Click a membrane or compartment."]);
        }
        if v.multi_select {
            let size = shell.store.multi_selection_size();
            let rest = format!(" Selected: {size}.");
            widgets::hint_rich(ui, &["", "Multi-select on.", rest.as_str()]);
        }
        if v.text_tool {
            widgets::hint_rich(ui, &["", "Text tool on.", " Click the map, type, and press Enter."]);
        }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(6.0);
            if widgets::toggle_button(ui, "Cofactors", v.show_currency, "Show secondary metabolites (ATP, NADH, H⁺…)").clicked() {
                let _ = shell.store.set_view(ViewPatch { show_currency: Some(!v.show_currency), ..Default::default() });
            }
            let quiet = shell.store.map.as_ref().and_then(|m| m.cofactor_style()) == Some("quiet");
            if widgets::toggle_button(ui, "Quiet cofactors", quiet, "Draw secondary metabolites small and light").clicked() {
                shell.store.set_map_cofactor_style(if quiet { None } else { Some("quiet") });
            }
            if widgets::toggle_button(ui, "Highlight cross-feeding", v.show_cross_feeding, "Dim the network and highlight transfers between cells").clicked() {
                let _ = shell.store.set_view(ViewPatch { show_cross_feeding: Some(!v.show_cross_feeding), ..Default::default() });
            }
            if widgets::toggle_button(ui, "Dark", v.dark, "Switch the colour scheme").clicked() {
                shell.set_theme(!v.dark);
            }
            if widgets::toggle_button(ui, "Edit geometry", v.geometry_edit, "Move and resize cells and compartments").clicked() {
                toggle_geometry_edit(shell);
            }
            if widgets::toggle_button(ui, "Multi-select", v.multi_select, "Select and move several metabolites and reactions").clicked() {
                toggle_multi_select(shell);
            }
            if widgets::toggle_button(ui, "Text", v.text_tool, "Add free text to the map").clicked() {
                toggle_text_tool(shell);
            }
        });

        selection_color_block(ui, shell);
        let hidden = shell.store.hidden_metabolite_count();
        if hidden > 0 {
            ui.add_space(10.0);
            if Btn::new(format!("Show hidden metabolites ({hidden})")).wide().show(ui).clicked() {
                show_hidden_metabolites(shell);
            }
        }
        ui.add_space(4.0);
        reaction_color_select(ui, shell, "display-reaction-color-mode", "Reaction color");
        ribbon_box(ui, |ui| {
            pathway_ribbon_select(ui, shell, "display-pathway-ribbon-mode", "Pathway ribbons");
            if shell.store.view.pathway_ribbon_mode != PathwayRibbonMode::Off {
                pathway_ribbon_sliders(ui, shell);
            }
        });
        ui.add_space(4.0);
        crate::panels::toolbar::labels_select(ui, shell, "display-labels");
        ui.add_space(6.0);
        let (can_undo, can_redo) = (shell.store.can_undo_map(), shell.store.can_redo_map());
        widgets::grow_row(ui, 2, |ui, i| {
            if i == 0 {
                if Btn::new("↶ Undo").enabled(can_undo).title("Ctrl+Z").wide().show(ui).clicked() {
                    shell.undo_redo(false);
                }
            } else if Btn::new("↷ Redo").enabled(can_redo).title("Ctrl+Y / Ctrl+Shift+Z").wide().show(ui).clicked() {
                shell.undo_redo(true);
            }
        });
        widgets::grow_row(ui, 2, |ui, i| {
            if i == 0 {
                if Btn::new("Empty map").wide().show(ui).clicked() {
                    shell.map_empty();
                }
            } else if Btn::new("Full map").wide().show(ui).clicked() {
                shell.map_full();
            }
        });
    });
}

/// `displayCard(store, { workspace: true })`, as a workspace card.
pub fn selection_card(ui: &mut Ui, shell: &mut Shell) {
    display_card(ui, shell, true);
}

/// `cancelTextEditor()`: drop the text being typed on the map.
fn cancel_text_editor(shell: &mut Shell) {
    shell.map.canvas.gestures.cancel_text_editor();
    shell.drain_canvas_events();
}

/// `toggle-geometry-edit`.
fn toggle_geometry_edit(shell: &mut Shell) {
    shell.close_map_menus();
    cancel_text_editor(shell);
    let on = !shell.store.view.geometry_edit;
    let _ = shell.store.set_view(ViewPatch { geometry_edit: Some(on), multi_select: Some(false), text_tool: Some(false), ..Default::default() });
}

/// `toggle-multi-select`.
fn toggle_multi_select(shell: &mut Shell) {
    shell.close_map_menus();
    cancel_text_editor(shell);
    shell.store.clear_geometry_selection();
    let on = !shell.store.view.multi_select;
    let _ = shell.store.set_view(ViewPatch { multi_select: Some(on), geometry_edit: Some(false), text_tool: Some(false), ..Default::default() });
}

/// `toggle-text-tool`.
fn toggle_text_tool(shell: &mut Shell) {
    shell.close_map_menus();
    cancel_text_editor(shell);
    shell.store.clear_geometry_selection();
    shell.store.clear_multi_selection(true);
    let on = !shell.store.view.text_tool;
    let _ = shell.store.set_view(ViewPatch { text_tool: Some(on), geometry_edit: Some(false), multi_select: Some(false), ..Default::default() });
}

/// `show-hidden-metabolites`.
fn show_hidden_metabolites(shell: &mut Shell) {
    shell.close_map_menus();
    shell.store.show_all_hidden_metabolites();
}

/// `arrangeSelectionBlock(store)`: align and distribute for a selection, in
/// the inspector's Selection section. With reactions in it, Horizontal and
/// Vertical work as in Escher: each reaction comes with its metabolites, all
/// on one line; one selected reaction is enough for that. The action clicked.
pub(crate) fn arrange_selection_block(ui: &mut Ui, store: &Store) -> Option<&'static str> {
    let size = store.multi_selection_size();
    let with_reactions = store.map.as_ref().is_some_and(|m| m.selection_has_line_reactions(&store.multi_selection));
    let line_title = |axis: &str, same: &str| {
        if with_reactions {
            format!("Put the selected reactions and their metabolites on one {axis} line")
        } else {
            format!("Same {same}: a {axis} line")
        }
    };
    let horizontal = line_title("horizontal", "y");
    let vertical = line_title("vertical", "x");
    let distribute_h = format!("Distribute {}", glyph::DISTRIBUTE_H);
    let distribute_v = format!("Distribute {}", glyph::DISTRIBUTE_V);
    let buttons: [(&str, &str, &str, bool); 8] = [
        ("align-horizontal", "Horizontal", &horizontal, true),
        ("align-vertical", "Vertical", &vertical, true),
        ("align-left", "Left", "Align left edges", size >= 2),
        ("align-right", "Right", "Align right edges", size >= 2),
        ("align-top", "Top", "Align top edges", size >= 2),
        ("align-bottom", "Bottom", "Align bottom edges", size >= 2),
        ("distribute-horizontal", &distribute_h, "Space centres evenly, horizontally", size >= 3),
        ("distribute-vertical", &distribute_v, "Space centres evenly, vertically", size >= 3),
    ];
    let mut clicked = None;
    for pair in buttons.chunks(2) {
        widgets::grow_row(ui, 2, |ui, i| {
            let (action, label, title, enabled) = pair[i];
            if Btn::new(label).enabled(enabled).title(title).wide().show(ui).clicked() {
                clicked = Some(action);
            }
        });
    }
    clicked
}

/// A head line: strong text on the left, a small count on the right
/// (`.selection-arrange-head`, `.pathway-create-head`, `.flux-color-editor-head`).
fn head_row(ui: &mut Ui, title: &str, count: &str, p: &Palette) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).size(11.5).family(bold()).color(p.ink_2));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(count).size(10.5).color(p.ink_3));
        });
    });
}

// Reaction colours and ribbons

/// `.view-label-control`: a label on the left, its select on the right.
fn view_label_control(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    let p = Palette::current(ui.ctx());
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(12.5).color(p.ink_2));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), add);
    });
}

/// `select[data-action="reaction-color-mode"]`.
fn reaction_color_select(ui: &mut Ui, shell: &mut Shell, id: &str, aria: &str) {
    let current = shell.store.view.reaction_color_mode.as_str().to_string();
    let options = [("flux", "Flux"), ("subsystem", "Subsystem"), ("pathway", "Pathway")].map(|(v, l)| (v.to_string(), l.to_string()));
    view_label_control(ui, "Reaction color", |ui| {
        if let Some(value) = widgets::select(ui, id, &current, &options, 160.0, true, aria) {
            // A group chosen here is what "Color lines by flux" goes back to.
            let group = (value != "flux").then(|| value.clone());
            let _ = shell.store.set_view(ViewPatch { reaction_color_mode: Some(value), line_color_group: group, ..Default::default() });
        }
    });
}

/// `select[data-action="pathway-ribbon-mode"]`: a ribbon brings the arrows
/// back to flux colours.
fn pathway_ribbon_select(ui: &mut Ui, shell: &mut Shell, id: &str, aria: &str) {
    let current = shell.store.view.pathway_ribbon_mode.as_str().to_string();
    let options = [("off", "Off"), ("subsystem", "Subsystem"), ("pathway", "Pathway")].map(|(v, l)| (v.to_string(), l.to_string()));
    view_label_control(ui, "Pathway ribbons", |ui| {
        if let Some(value) = widgets::select(ui, id, &current, &options, 160.0, true, aria) {
            let flux = (value != "off").then(|| "flux".to_string());
            let _ = shell.store.set_view(ViewPatch { pathway_ribbon_mode: Some(value), reaction_color_mode: flux, ..Default::default() });
        }
    });
}

/// `.pathway-ribbon-controls`.
fn ribbon_box(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let p = Palette::current(ui.ctx());
    ui.add_space(8.0);
    Frame::new().fill(p.panel).stroke(Stroke::new(1.0, p.line)).corner_radius(CornerRadius::same(9)).inner_margin(Margin::symmetric(9, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui)
    });
}

/// The Width and Opacity sliders of the ribbons (`.pathway-ribbon-slider`):
/// applied live, as the range's `input` events are.
fn pathway_ribbon_sliders(ui: &mut Ui, shell: &mut Shell) {
    let p = shell.palette();
    let width = shell.store.view.pathway_ribbon_width;
    let opacity = shell.store.view.pathway_ribbon_opacity;
    let mut new_width = None;
    slider_row(ui, &p, "Width", format!("{} px", js::number_to_string(js::round(width))), |ui| {
        let (_, event) = widgets::slider(ui, width, 20.0, 120.0, 2.0, true, "Pathway ribbon width");
        new_width = event.value();
    });
    let mut new_opacity = None;
    slider_row(ui, &p, "Opacity", format!("{}%", js::number_to_string(js::round(opacity * 100.0))), |ui| {
        let (_, event) = widgets::slider(ui, opacity, 0.04, 0.45, 0.01, true, "Pathway ribbon opacity");
        new_opacity = event.value();
    });
    if let Some(w) = new_width {
        let _ = shell.store.set_view(ViewPatch { pathway_ribbon_width: Some(w), ..Default::default() });
    }
    if let Some(o) = new_opacity {
        let _ = shell.store.set_view(ViewPatch { pathway_ribbon_opacity: Some(o), ..Default::default() });
    }
}

/// `.pathway-ribbon-slider`: a label, the range, its read-out.
fn slider_row(ui: &mut Ui, p: &Palette, label: &str, readout: String, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(Vec2::new(58.0, 18.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_width(58.0);
            ui.label(RichText::new(label).size(11.0).color(p.ink_2));
        });
        ui.spacing_mut().slider_width = (ui.available_width() - 48.0 - 16.0).max(40.0);
        add(ui);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(readout).size(10.5).color(p.ink_3));
        });
    });
}

/// `mapAppearanceCard(store, fluxColorDraft)`.
pub fn map_appearance_card(ui: &mut Ui, shell: &mut Shell) {
    widgets::card(ui, "Lines, circles & ribbons", None, |ui| {
        let by_flux = shell.store.view.reaction_color_mode == ReactionColorMode::Flux;
        // `color-by-flux`: off colours lines by the group last chosen.
        if let Some(on) = line_toggle(ui, by_flux, "Color lines by flux") {
            let mode = if on { "flux".to_string() } else { shell.store.view.line_color_group.clone() };
            let _ = shell.store.set_view(ViewPatch { reaction_color_mode: Some(mode), ..Default::default() });
        }
        if by_flux {
            flux_color_editor(ui, shell);
        } else {
            let current = shell.store.view.reaction_color_mode.as_str().to_string();
            let options = [("pathway", "Pathway"), ("subsystem", "Subsystem")].map(|(v, l)| (v.to_string(), l.to_string()));
            view_label_control(ui, "Color lines by", |ui| {
                if let Some(value) = widgets::select(ui, "reaction-color-mode", &current, &options, 160.0, true, "Reaction color mode") {
                    let _ = shell.store.set_view(ViewPatch {
                        reaction_color_mode: Some(value.clone()),
                        line_color_group: Some(value),
                        ..Default::default()
                    });
                }
            });
        }
        let v = shell.store.view.clone();
        if let Some(on) = line_toggle(ui, v.width_by_flux, "Line width follows flux") {
            let _ = shell.store.set_view(ViewPatch { width_by_flux: Some(on), ..Default::default() });
        }
        if let Some(on) = line_toggle(ui, v.dash_no_flux, "Dash lines with no flux") {
            let _ = shell.store.set_view(ViewPatch { dash_no_flux: Some(on), ..Default::default() });
        }
        if let Some(on) = line_toggle(ui, v.line_hops, "Gap where lines cross") {
            let _ = shell.store.set_view(ViewPatch { line_hops: Some(on), ..Default::default() });
        }
        node_style_controls(ui, shell);
        pathway_ribbon_select(ui, shell, "pathway-ribbon-mode", "Pathway ribbon mode");
        if shell.store.view.pathway_ribbon_mode != PathwayRibbonMode::Off {
            ribbon_box(ui, |ui| pathway_ribbon_sliders(ui, shell));
        }
    });
}

/// `lineToggle(action, checked, title)`: a switch in the map appearance
/// card, one way of drawing the lines, on or off. The new state when it is
/// clicked.
fn line_toggle(ui: &mut Ui, checked: bool, title: &str) -> Option<bool> {
    let p = Palette::current(ui.ctx());
    let label = RichText::new(title).size(12.0).family(bold()).color(p.ink);
    ui.add_space(4.0);
    widgets::checkbox(ui, checked, label, true, Some(title)).1
}

/// `defaultNodeColors(dark)`: the circle colours of the theme, as the
/// renderer draws them.
pub fn default_node_colors(dark: bool) -> (&'static str, &'static str) {
    if dark { ("#f59e0b", "#7c2d12") } else { ("#e67e4c", "#a64613") }
}

/// `nodeStyleControls(store)`: the colours metabolite circles take on this
/// map unless one has its own.
fn node_style_controls(ui: &mut Ui, shell: &mut Shell) {
    let p = Palette::current(ui.ctx());
    let style = shell.store.map.as_ref().map(|m| m.graph().node_style()).unwrap_or_default();
    let (theme_fill, theme_stroke) = default_node_colors(shell.store.view.dark);
    ui.add_space(10.0);
    ui.label(RichText::new("METABOLITE CIRCLES").size(11.0).family(bold()).color(p.ink_2));
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Fill").size(12.0));
        let fill = style.fill.clone().unwrap_or_else(|| theme_fill.to_string());
        if let Some(c) = widgets::color_field(ui, egui::Id::new("node-style-fill"), &fill, "Fill of metabolite circles") {
            shell.store.set_map_node_style(&ColorChange::Set(c), &ColorChange::Keep);
        }
        ui.label(RichText::new("Outline").size(12.0));
        let stroke = style.stroke.clone().unwrap_or_else(|| theme_stroke.to_string());
        if let Some(c) = widgets::color_field(ui, egui::Id::new("node-style-stroke"), &stroke, "Outline of metabolite circles") {
            shell.store.set_map_node_style(&ColorChange::Keep, &ColorChange::Set(c));
        }
        let own = style.fill.is_some() || style.stroke.is_some();
        if Btn::new("Theme colors").enabled(own).show(ui).clicked() {
            shell.store.set_map_node_style(&ColorChange::Reset, &ColorChange::Reset);
        }
    });
}

/// What a colour row asks for (`node-fill`, `node-stroke`,
/// `node-colors-reset`, `reaction-line-color`, `reaction-color-reset`).
#[derive(Debug, Clone, PartialEq)]
pub enum ColorEdit {
    NodeFill(String),
    NodeStroke(String),
    NodeReset,
    ReactionColor(String),
    ReactionReset,
}

/// `nodeColorRow(store, nodeIds, node)`: fill and outline for one metabolite
/// circle or several. Shows what is drawn now: the node's own colour, else
/// the map's, else the theme's.
pub fn node_color_row(ui: &mut Ui, store: &Store, node_ids: &[String], salt: &str) -> Option<ColorEdit> {
    let map = store.map.as_ref()?;
    let style = map.graph().node_style();
    let (theme_fill, theme_stroke) = default_node_colors(store.view.dark);
    let first = node_ids.first().and_then(|id| map.node(id));
    let single = node_ids.len() == 1;
    let own = node_ids.iter().filter_map(|id| map.node(id)).any(|n| n.own_fill().is_some() || n.own_stroke().is_some());
    let fill = single.then(|| first.and_then(|n| n.own_fill()).map(str::to_string)).flatten().or(style.fill.clone()).unwrap_or_else(|| theme_fill.into());
    let stroke =
        single.then(|| first.and_then(|n| n.own_stroke()).map(str::to_string)).flatten().or(style.stroke.clone()).unwrap_or_else(|| theme_stroke.into());
    let p = Palette::current(ui.ctx());
    let mut edit = None;
    ui.label(RichText::new("Circle color").size(12.0).family(bold()).color(p.ink_2));
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Fill").size(12.0));
        if let Some(c) = widgets::color_field(ui, egui::Id::new(("node-fill", salt)), &fill, "Circle fill") {
            edit = Some(ColorEdit::NodeFill(c));
        }
        ui.label(RichText::new("Outline").size(12.0));
        if let Some(c) = widgets::color_field(ui, egui::Id::new(("node-stroke", salt)), &stroke, "Circle outline") {
            edit = Some(ColorEdit::NodeStroke(c));
        }
        if Btn::new("Map colors").enabled(own).show(ui).clicked() {
            edit = Some(ColorEdit::NodeReset);
        }
    });
    edit
}

/// `reactionColorRow(store, reactionIds)`: the colour of one reaction's
/// lines or several, used when lines are not coloured by flux.
pub fn reaction_color_row(ui: &mut Ui, store: &Store, reaction_ids: &[String], salt: &str) -> Option<ColorEdit> {
    let map = store.map.as_ref()?;
    let reactions: Vec<_> = reaction_ids.iter().filter_map(|id| map.reaction(id)).collect();
    if reactions.is_empty() {
        return None;
    }
    let own = reactions.iter().find_map(|r| r.own_color()).map(str::to_string);
    let by_flux = store.view.reaction_color_mode == ReactionColorMode::Flux;
    let p = Palette::current(ui.ctx());
    let mut edit = None;
    ui.label(RichText::new("Line color").size(12.0).family(bold()).color(p.ink_2));
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Color").size(12.0));
        let value = own.clone().unwrap_or_else(|| "#64748b".into());
        if let Some(c) = widgets::color_field(ui, egui::Id::new(("reaction-line-color", salt)), &value, "Reaction line color") {
            edit = Some(ColorEdit::ReactionColor(c));
        }
        if Btn::new("Pathway color").enabled(own.is_some()).show(ui).clicked() {
            edit = Some(ColorEdit::ReactionReset);
        }
    });
    if by_flux {
        widgets::note(ui, "Used when lines are not colored by flux.");
    }
    edit
}

/// Apply what a colour row asked for, as one undoable edit.
pub fn apply_color_edit(store: &mut Store, node_ids: &[String], reaction_ids: &[String], edit: ColorEdit) {
    match edit {
        ColorEdit::NodeFill(c) => {
            store.set_map_node_colors(node_ids, &ColorChange::Set(c), &ColorChange::Keep);
        }
        ColorEdit::NodeStroke(c) => {
            store.set_map_node_colors(node_ids, &ColorChange::Keep, &ColorChange::Set(c));
        }
        ColorEdit::NodeReset => {
            store.set_map_node_colors(node_ids, &ColorChange::Reset, &ColorChange::Reset);
        }
        ColorEdit::ReactionColor(c) => {
            store.set_map_reaction_color(reaction_ids, &ColorChange::Set(c));
        }
        ColorEdit::ReactionReset => {
            store.set_map_reaction_color(reaction_ids, &ColorChange::Reset);
        }
    }
}

/// `selectionColorBlock(store)`: colour what is selected: the circles of its
/// metabolites, the lines of its reactions.
fn selection_color_block(ui: &mut Ui, shell: &mut Shell) {
    let Some(map) = shell.store.map.as_ref() else { return };
    let nodes: Vec<String> = shell
        .store
        .multi_selection
        .nodes
        .iter()
        .filter(|id| map.node(id).is_some_and(|n| n.is_metabolite_kind()))
        .cloned()
        .collect();
    let reactions: Vec<String> = shell.store.multi_selection.reactions.iter().filter(|id| map.has_reaction(id)).cloned().collect();
    if nodes.is_empty() && reactions.is_empty() {
        return;
    }
    let p = shell.palette();
    let mut edit = None;
    ui.add_space(10.0);
    Frame::new().fill(p.panel).stroke(Stroke::new(1.0, p.line)).corner_radius(CornerRadius::same(9)).inner_margin(Margin::same(9)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new("Color selection").size(12.0).family(bold()).color(p.ink));
            let plural = |n: usize, one: &str| format!("{n} {one}{}", if n == 1 { "" } else { "s" });
            ui.label(RichText::new(format!("{} · {}", plural(nodes.len(), "metabolite"), plural(reactions.len(), "reaction"))).size(11.0).color(p.ink_3));
        });
        if !nodes.is_empty() {
            if let Some(e) = node_color_row(ui, &shell.store, &nodes, "selection") {
                edit = Some(e);
            }
        }
        if !reactions.is_empty() {
            if let Some(e) = reaction_color_row(ui, &shell.store, &reactions, "selection") {
                edit = Some(e);
            }
        }
    });
    if let Some(edit) = edit {
        apply_color_edit(&mut shell.store, &nodes, &reactions, edit);
    }
}

// Flux colour editor

/// A row of the editor as `readFluxColorRows` reads it: the text of the
/// value field, the number it holds (`valueAsNumber`) and the colour.
#[derive(Debug, Clone, PartialEq)]
struct RowInput {
    value: String,
    number: f64,
    color: String,
}

impl RowInput {
    fn typed(value: String, color: String) -> RowInput {
        RowInput { number: widgets::value_as_number(&value), value, color }
    }
}

/// The rows on screen: the draft as the user left it, or the palette.
fn shown_rows(shell: &Shell) -> Vec<FluxColorRow> {
    match &shell.ui.flux_color_draft {
        Some(draft) => draft.rows.clone(),
        None => shell
            .store
            .view
            .flux_color_stops
            .iter()
            .flatten()
            .map(|stop| FluxColorRow { value: js::number_to_string(stop.value), color: stop.color.clone() })
            .collect(),
    }
}

/// `readFluxColorRows()`.
fn read_flux_color_rows(shell: &Shell) -> Vec<RowInput> {
    shown_rows(shell).into_iter().map(|r| RowInput::typed(r.value, r.color)).collect()
}

/// What is wrong with the rows, and which ones (`error.indices`).
#[derive(Debug, Clone, PartialEq)]
struct RowsError {
    message: String,
    indices: Vec<usize>,
}

/// `fluxColorValues(rows)`: the palette the rows make, or why they make none.
fn flux_color_values(rows: &[RowInput]) -> Result<Vec<FluxColorStop>, RowsError> {
    if rows.is_empty() {
        return Err(RowsError { message: "Keep at least one color point in a custom palette.".into(), indices: Vec::new() });
    }
    let mut invalid = Vec::new();
    // A JS `Map` keyed by number: 0 and -0 are one key.
    let mut by_value: indexmap::IndexMap<u64, Vec<usize>> = indexmap::IndexMap::new();
    let points: Vec<FluxColorStop> = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let value = row.number;
            if row.value.trim().is_empty() || !value.is_finite() {
                invalid.push(index);
            } else {
                let key = if value == 0.0 { 0.0_f64.to_bits() } else { value.to_bits() };
                by_value.entry(key).or_default().push(index);
            }
            FluxColorStop { value, color: row.color.clone() }
        })
        .collect();
    if !invalid.is_empty() {
        return Err(RowsError {
            message: "Enter a finite flux value for every point. The map keeps the last valid palette.".into(),
            indices: invalid,
        });
    }
    let duplicates: Vec<usize> = by_value.into_values().filter(|m| m.len() > 1).flatten().collect();
    if !duplicates.is_empty() {
        return Err(RowsError {
            message: "Each color point needs a unique flux value. The map keeps the last valid palette.".into(),
            indices: duplicates,
        });
    }
    Ok(points)
}

/// `commitFluxColorRows(rows)`: keep the rows as typed and, when they make a
/// palette, give it to the map. Returns whether it did.
fn commit_flux_color_rows(shell: &mut Shell, rows: &[RowInput]) -> bool {
    let mut draft = FluxColorDraft {
        rows: rows.iter().map(|r| FluxColorRow { value: r.value.clone(), color: r.color.clone() }).collect(),
        error: String::new(),
        invalid: Vec::new(),
    };
    let committed = match flux_color_values(rows) {
        Ok(points) => {
            shell.ui.flux_colors_committing = true;
            let result = shell.store.set_flux_color_stops(Some(points));
            // `ui.fluxColorDraft.source = store.view.fluxColorStops`: the
            // palette the draft belongs to, so the change it caused does not
            // drop it.
            shell.ui.flux_color_source = Some(shell.store.view.flux_color_stops.clone());
            shell.ui.flux_colors_committing = false;
            match result {
                Ok(_) => true,
                Err(error) => {
                    draft.error = error.to_string();
                    false
                }
            }
        }
        Err(error) => {
            draft.error = error.message;
            draft.invalid = error.indices;
            false
        }
    };
    shell.ui.flux_color_draft = Some(draft);
    committed
}

/// `newFluxColorValue(points)`: a value no point has yet, past the top end
/// first.
fn new_flux_color_value(points: &[FluxColorStop]) -> Result<f64, String> {
    let mut sorted: Vec<f64> = points.iter().map(|p| p.value).collect();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let first = sorted[0];
    let last = sorted[sorted.len() - 1];
    let step = 1.0_f64.max((last.abs() * 0.1).max(if sorted.len() > 1 { last - sorted[sorted.len() - 2] } else { 0.0 }));
    let mut candidates = vec![last + step, first - 1.0_f64.max(first.abs() * 0.1), 0.0];
    for index in 1..sorted.len() {
        candidates.push(sorted[index - 1] / 2.0 + sorted[index] / 2.0);
    }
    candidates
        .into_iter()
        .find(|c| c.is_finite() && !sorted.contains(c))
        .ok_or_else(|| "Choose a different value before adding another color point.".to_string())
}

/// `focusFluxColorPoint(index)`.
fn focus_flux_color_point(shell: &mut Shell, index: usize) {
    shell.ui.focus_request = Some(flux_color_value_id(index));
}

/// `flux-colors-customize`: start a signed palette around the current scale.
fn flux_colors_customize(shell: &mut Shell) {
    if shell.store.view.flux_color_stops.is_none() {
        let reference = shell.store.scale.as_ref().map_or(1.0, |s| s.reference);
        let limit = if reference.is_finite() && reference > 0.0 { reference } else { 1.0 };
        let row = |v: f64, color: &str| RowInput { value: js::number_to_string(v), number: v, color: color.into() };
        commit_flux_color_rows(shell, &[row(-limit, "#2563eb"), row(0.0, "#cbd5e1"), row(limit, "#dc2626")]);
    }
    focus_flux_color_point(shell, 0);
}

/// `flux-colors-reset`.
fn flux_colors_reset(shell: &mut Shell) {
    shell.ui.flux_color_draft = None;
    let _ = shell.store.set_flux_color_stops(None);
}

/// `flux-color-add`.
fn flux_color_add(shell: &mut Shell) {
    let mut rows = read_flux_color_rows(shell);
    let next = flux_color_values(&rows).map_err(|_| ()).and_then(|points| {
        let value = new_flux_color_value(&points).map_err(|_| ())?;
        let mut sorted = points.clone();
        sorted.sort_by(|a, b| a.value.partial_cmp(&b.value).unwrap_or(std::cmp::Ordering::Equal));
        Ok((value, sorted.last().map(|p| p.color.clone()).unwrap_or_default()))
    });
    match next {
        Ok((value, color)) => {
            rows.push(RowInput { value: js::number_to_string(value), number: value, color });
            if commit_flux_color_rows(shell, &rows) {
                focus_flux_color_point(shell, rows.len() - 1);
            }
        }
        // The JS catch commits the rows as they are, which shows what is
        // wrong with them (or, when only a new value could not be found,
        // keeps them).
        Err(()) => {
            commit_flux_color_rows(shell, &rows);
        }
    }
}

/// `flux-color-remove`.
fn flux_color_remove(shell: &mut Shell, index: usize) {
    let mut rows = read_flux_color_rows(shell);
    if rows.len() <= 1 || index >= rows.len() {
        return;
    }
    rows.remove(index);
    commit_flux_color_rows(shell, &rows);
    focus_flux_color_point(shell, index.min(rows.len() - 1));
}

/// What a row of the editor asked for this frame.
enum RowEdit {
    Value(usize, String),
    Color(usize, String),
    Remove(usize),
}

/// `fluxColorEditor(store, draft)`.
fn flux_color_editor(ui: &mut Ui, shell: &mut Shell) {
    let p = shell.palette();
    let store = &shell.store;
    let custom = store.view.flux_color_stops.as_ref().is_some_and(|s| !s.is_empty());
    let omics_active = (store.omics_dataset.is_some()
        && store.omics_settings.enabled
        && store.omics_settings.condition_a.as_deref().is_some_and(|c| !c.is_empty())
        && store.community.is_some())
        || (store.omics.enabled && store.omics.dataset.is_some());
    let rows = shown_rows(shell);
    let (error, invalid) = shell.ui.flux_color_draft.as_ref().map_or((String::new(), Vec::new()), |d| (d.error.clone(), d.invalid.clone()));

    ui.add_space(12.0);
    Frame::new().fill(p.panel_2).stroke(Stroke::new(1.0, p.line)).corner_radius(CornerRadius::same(9)).inner_margin(Margin::same(12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new("Flux palette").size(12.0).family(bold()).color(p.ink));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let count = if custom { format!("{} point{}", rows.len(), if rows.len() == 1 { "" } else { "s" }) } else { "Automatic".into() };
                ui.label(RichText::new(count).size(11.0).color(p.ink_3));
            });
        });
        ui.add_space(4.0);
        // `role="group" aria-label="Flux color palette"`.
        let mut mode = None;
        widgets::grow_row(ui, 2, |ui, i| {
            let (label, on) = if i == 0 { ("Automatic", !custom) } else { ("Custom colors", custom) };
            let r = Btn::new(label).active(on).wide().height(30.0).show(ui);
            aria_pressed(&r, on);
            if r.clicked() {
                mode = Some(i);
            }
        });
        match mode {
            Some(0) => flux_colors_reset(shell),
            Some(_) => flux_colors_customize(shell),
            None => {}
        }
        ui.add_space(8.0);
        if omics_active {
            widgets::hint(ui, "An omics overlay is active and hides flux colors.");
        }
        render_flux_color_preview(ui, &shell.store);
        // The rows may have changed with the mode.
        let custom = shell.store.view.flux_color_stops.as_ref().is_some_and(|s| !s.is_empty());
        if !custom {
            return;
        }
        let rows = if mode.is_some() { shown_rows(shell) } else { rows };
        let (error, invalid) =
            if mode.is_some() { shell.ui.flux_color_draft.as_ref().map_or((String::new(), Vec::new()), |d| (d.error.clone(), d.invalid.clone())) } else { (error, invalid) };
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 7.0;
            let small = |s: &str| RichText::new(s).size(10.0).color(p.ink_3);
            ui.allocate_ui_with_layout(Vec2::new(108.0, 14.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.set_width(108.0);
                ui.label(small("Color"));
            });
            ui.label(small("Flux value"));
        });
        let mut edit = None;
        for (index, row) in rows.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 7.0;
                let aria = format!("Color at flux point {}", index + 1);
                if let Some(color) = widgets::color_field(ui, egui::Id::new(("flux-color-color", index)), &row.color, &aria) {
                    edit = Some(RowEdit::Color(index, color));
                }
                let width = (ui.available_width() - 30.0 - 7.0).max(48.0);
                let aria = format!("Flux value at point {}", index + 1);
                let (response, event) = widgets::number_field(
                    ui,
                    flux_color_value_id(index),
                    &row.value,
                    Field::new().width(width).aria(&aria).invalid(invalid.contains(&index)),
                );
                shell.take_focus_request(&response);
                if let FieldEvent::Change(text) = event {
                    edit = Some(RowEdit::Value(index, text));
                }
                let aria = format!("Remove flux point {}", index + 1);
                let remove = Btn::new(RichText::new("×").size(18.0)).aria(&aria).title("Remove point").enabled(rows.len() > 1).height(30.0).show(ui);
                if remove.clicked() {
                    edit = Some(RowEdit::Remove(index));
                }
            });
        }
        if !error.is_empty() {
            ui.add_space(4.0);
            ui.add(egui::Label::new(RichText::new(&error).size(11.0).color(p.danger)).wrap());
        }
        ui.add_space(4.0);
        if Btn::new("+ Add color point").wide().height(34.0).show(ui).clicked() {
            flux_color_add(shell);
        }
        widgets::hint(ui, "Each point needs a unique value.");
        match edit {
            Some(RowEdit::Value(index, text)) => {
                let mut rows = read_flux_color_rows(shell);
                if let Some(row) = rows.get_mut(index) {
                    *row = RowInput::typed(text, row.color.clone());
                    commit_flux_color_rows(shell, &rows);
                }
            }
            Some(RowEdit::Color(index, color)) => {
                let mut rows = read_flux_color_rows(shell);
                if let Some(row) = rows.get_mut(index) {
                    row.color = color;
                    commit_flux_color_rows(shell, &rows);
                }
            }
            Some(RowEdit::Remove(index)) => flux_color_remove(shell, index),
            None => {}
        }
    });
    ui.add_space(4.0);
}

/// `renderFluxColorPreview(store)`: the palette the map uses, never the
/// editor's uncommitted fields.
pub fn render_flux_color_preview(ui: &mut Ui, store: &Store) {
    let p = Palette::current(ui.ctx());
    let custom = store.view.flux_color_stops.as_ref().is_some_and(|s| !s.is_empty());
    let stops: Vec<(f64, f64, String)> = if custom {
        let mut sorted = store.view.flux_color_stops.clone().unwrap_or_default();
        sorted.sort_by(|a, b| a.value.partial_cmp(&b.value).unwrap_or(std::cmp::Ordering::Equal));
        sorted.into_iter().map(|s| (f64::NAN, s.value, s.color)).collect()
    } else {
        store.scale.as_ref().map(|s| s.legend_stops(7)).unwrap_or_default().into_iter().map(|s| (s.t, s.value, s.color)).collect()
    };
    if stops.is_empty() {
        widgets::hint(ui, "The scale appears after an analysis.");
        return;
    }
    let first = stops[0].1;
    let last = stops[stops.len() - 1].1;
    let position = |stop: &(f64, f64, String)| -> f64 {
        if !custom {
            let t = if stop.0.is_nan() { 0.0 } else { stop.0 };
            return (t * 100.0).clamp(0.0, 100.0);
        }
        if first == last {
            return 50.0;
        }
        if stop.1 == first {
            return 0.0;
        }
        if stop.1 == last {
            return 100.0;
        }
        let range = last - first;
        let fraction = if range.is_finite() { (stop.1 - first) / range } else { (stop.1 / 2.0 - first / 2.0) / (last / 2.0 - first / 2.0) };
        js::max(0.0, js::min(100.0, fraction * 100.0))
    };
    let points: Vec<(f64, Color32)> = stops.iter().map(|s| (position(s), css_color(&s.2, p.ink_3))).collect();

    let label = format!(
        "{} color gradient, {} to {}",
        if custom { "Signed flux" } else { "Automatic flux magnitude" },
        format_flux(Some(first), 6),
        format_flux(Some(last), 6)
    );
    ui.add_space(6.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width() - 2.0, 22.0), Sense::hover());
    response.ctx.accesskit_node_builder(response.id, |node| {
        node.set_role(egui::accesskit::Role::Image);
        node.set_label(label);
    });
    paint_gradient(ui, rect.shrink(1.0), &points);
    ui.painter().rect_stroke(rect, CornerRadius::same(4), Stroke::new(1.0, p.line), egui::StrokeKind::Inside);
    for (i, (position, _)) in points.iter().enumerate() {
        let x = rect.left() + rect.width() * (*position as f32) / 100.0;
        let mark = egui::Rect::from_center_size(egui::pos2(x, rect.center().y), Vec2::new(2.0, 26.0));
        ui.painter().rect(mark, CornerRadius::ZERO, p.ink, Stroke::new(1.0, p.panel), egui::StrokeKind::Outside);
        let hover = ui.interact(mark.expand(2.0), response.id.with(("mark", i)), Sense::hover());
        hover.on_hover_text(format_flux(Some(stops[i].1), 6));
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let small = |s: String| RichText::new(s).size(10.0).color(p.ink_2);
        ui.label(small(format_flux(Some(first), 6)));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            if stops.len() > 1 {
                ui.label(small(format_flux(Some(last), 6)));
            }
            ui.with_layout(egui::Layout::centered_and_justified(egui::Direction::LeftToRight), |ui| {
                ui.label(small(format!("{} mmol·gDW⁻¹·h⁻¹", if custom { "Flux" } else { "|flux|" })));
            });
        });
    });
    ui.add_space(6.0);
}

/// `linear-gradient(to right in srgb, …)` over `rect`: `points` are
/// `(position %, colour)` in order; the ends hold the first and last colours.
fn paint_gradient(ui: &Ui, rect: egui::Rect, points: &[(f64, Color32)]) {
    let Some(&(_, first)) = points.first() else { return };
    let &(_, last) = points.last().expect("not empty");
    let mut stops: Vec<(f32, Color32)> = vec![(0.0, first)];
    let mut previous = 0.0_f32;
    for &(position, color) in points {
        // A stop before the previous one is moved up to it (CSS).
        let at = (position as f32 / 100.0).max(previous);
        stops.push((at, color));
        previous = at;
    }
    stops.push((1.0_f32.max(previous), last));
    let mut mesh = egui::Mesh::default();
    for pair in stops.windows(2) {
        let (a, ca) = pair[0];
        let (b, cb) = pair[1];
        if b <= a {
            continue;
        }
        let x0 = rect.left() + rect.width() * a;
        let x1 = rect.left() + rect.width() * b;
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(egui::pos2(x0, rect.top()), ca);
        mesh.colored_vertex(egui::pos2(x1, rect.top()), cb);
        mesh.colored_vertex(egui::pos2(x1, rect.bottom()), cb);
        mesh.colored_vertex(egui::pos2(x0, rect.bottom()), ca);
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    }
    ui.painter().add(egui::Shape::mesh(mesh));
}

// Legend

/// A run of text in a paragraph: plain, `<b>` or `<i>`.
#[derive(Clone, Copy, PartialEq)]
enum Run {
    Plain,
    Bold,
    Italic,
}

/// A wrapped paragraph of runs (`<p>` with `<b>` and `<i>`).
fn paragraph(ui: &mut Ui, size: f32, runs: &[(Run, &str)]) {
    let p = Palette::current(ui.ctx());
    let mut job = egui::text::LayoutJob::default();
    for (run, text) in runs {
        let family = if *run == Run::Bold { bold() } else { egui::FontFamily::Proportional };
        job.append(
            text,
            0.0,
            egui::TextFormat {
                font_id: egui::FontId::new(size, family),
                color: if *run == Run::Bold { p.ink } else { p.ink_2 },
                italics: *run == Run::Italic,
                ..Default::default()
            },
        );
    }
    job.wrap.max_width = ui.available_width();
    ui.add(egui::Label::new(job).wrap());
}

/// `legendCard(store)`.
pub fn legend_card(ui: &mut Ui, shell: &mut Shell) {
    let store = &shell.store;
    let dark = store.view.dark;
    let mut seen: indexmap::IndexMap<String, String> = indexmap::IndexMap::new();
    for cell in store.layout().map(|m| m.cells()).unwrap_or_default() {
        for comp in &cell.compartments {
            seen.entry(comp.id.clone()).or_insert_with(|| comp.label.clone());
        }
    }
    seen.insert("s".into(), "Shared medium".into());

    let reaction_mode = store.view.reaction_color_mode;
    let ribbon_mode = store.view.pathway_ribbon_mode;
    let mode = if ribbon_mode != PathwayRibbonMode::Off { ribbon_mode.as_str() } else { reaction_mode.as_str() };
    let coloring = if ribbon_mode != PathwayRibbonMode::Off { store.pathway_ribbon_coloring(true) } else { store.reaction_coloring(true) };
    let categorical = mode != "flux";
    let group_title = if mode == "pathway" { "Pathways" } else { "Subsystems" };
    let shown_groups = &coloring.legend[..coloring.legend.len().min(18)];
    let extra_groups = coloring.legend.len() - shown_groups.len();
    let custom_flux_colors = reaction_mode == ReactionColorMode::Flux && store.view.flux_color_stops.as_ref().is_some_and(|s| !s.is_empty());
    // Categorical arrow colours do not use the saved signed palette. Their
    // neutral magnitude guide must continue to start at zero.
    let stops: Vec<(f64, f64, String)> = match &store.scale {
        None => Vec::new(),
        Some(scale) if categorical => (0..7)
            .map(|index| {
                let t = index as f64 / 6.0;
                let value = if t == 0.0 { 0.0 } else { coxeter_render::format::expm1(t * js::log(10.0)) / 9.0 * scale.reference };
                (t, value, String::new())
            })
            .collect(),
        Some(scale) => scale.legend_stops(7).into_iter().map(|s| (s.t, s.value, s.color)).collect(),
    };
    // SteadyCom solves for abundance × flux; the map shows each species per
    // gram of itself, and the cross-feeding lines per gram of the whole
    // community.
    let steady_com = store.solution.as_ref().is_some_and(|s| s.steady_com.is_some());
    let show_cross_feeding = store.cross_feeding_highlighted();

    widgets::card(ui, "Legend", None, |ui| {
        let p = Palette::current(ui.ctx());
        let items: Vec<(String, String)> = seen.into_iter().collect();
        egui::Grid::new("legend-grid").num_columns(2).spacing(Vec2::new(8.0, 5.0)).show(ui, |ui| {
            for (i, (id, label)) in items.iter().enumerate() {
                let c = coxeter_core::compartments::compartment_colors(id, dark, &Default::default());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    widgets::square_swatch(ui, css_color(c.fill, p.panel_2), css_color(c.stroke, p.line));
                    ui.label(RichText::new(label).size(11.5).color(p.ink_2));
                });
                if i % 2 == 1 {
                    ui.end_row();
                }
            }
        });

        if categorical {
            ui.add_space(12.0);
            let top = ui.cursor().top() - 2.0;
            ui.painter().hline(ui.max_rect().x_range(), top, Stroke::new(1.0, p.line));
            ui.add_space(8.0);
            head_row(ui, group_title, &coloring.legend.len().to_string(), &p);
            for item in shown_groups {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 7.0;
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
                    ui.painter().rect_filled(rect, CornerRadius::same(3), css_color(&item.color, p.ink_3));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(item.count.to_string()).size(10.0).family(bold()).color(p.ink_3));
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.add(egui::Label::new(RichText::new(&item.label).size(11.0).color(p.ink_2)).truncate()).on_hover_text(&item.label);
                        });
                    });
                });
            }
            if shown_groups.is_empty() {
                widgets::empty(ui, "No drawn reactions.");
            }
            if extra_groups > 0 {
                widgets::hint(ui, format!("+{extra_groups} more"));
            }
        }

        if custom_flux_colors {
            ui.add_space(12.0);
            render_flux_color_preview(ui, &shell.store);
        } else {
            ui.add_space(12.0);
            let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::hover());
            if !stops.is_empty() {
                let gap = 2.0;
                let n = stops.len() as f32;
                let width = (rect.width() - gap * (n - 1.0)) / n;
                let neutral = css_color(if dark { "#94a3b8" } else { "#64748b" }, p.ink_3);
                for (i, (t, _, color)) in stops.iter().enumerate() {
                    let height = 6.0 + (*t as f32) * 26.0;
                    let x = rect.left() + i as f32 * (width + gap);
                    let bar = egui::Rect::from_min_max(egui::pos2(x, rect.bottom() - height), egui::pos2(x + width, rect.bottom()));
                    let fill = if categorical { neutral } else { css_color(color, p.ink_3) };
                    ui.painter().rect_filled(bar, CornerRadius { nw: 2, ne: 2, sw: 0, se: 0 }, fill);
                }
                let last = stops[stops.len() - 1].1;
                ui.horizontal(|ui| {
                    let small = |s: String| RichText::new(s).size(10.0).color(p.ink_3);
                    ui.label(small("0".into()));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        ui.label(small(format_flux(Some(last), 2)));
                        ui.with_layout(egui::Layout::centered_and_justified(egui::Direction::LeftToRight), |ui| {
                            ui.label(small(format!("|flux| mmol·gDW⁻¹·h⁻¹{}", if steady_com { " of each species" } else { "" })));
                        });
                    });
                });
            }
        }

        if show_cross_feeding {
            super::cross_feeding::cross_feeding_legend(ui, &shell.store);
        }

        if steady_com {
            ui.add_space(12.0);
            ui.label(RichText::new("Units").size(12.0).family(bold()).color(p.ink));
            paragraph(
                ui,
                12.0,
                &[(Run::Bold, "Reaction arrows"), (Run::Plain, ": mmol·gDW⁻¹·h⁻¹ "), (Run::Italic, "of that species"), (Run::Plain, ".")],
            );
            if show_cross_feeding {
                paragraph(
                    ui,
                    12.0,
                    &[
                        (Run::Bold, "Cross-feeding lines"),
                        (Run::Plain, ": mmol·gDW⁻¹·h⁻¹ "),
                        (Run::Italic, "of the whole community"),
                        (Run::Plain, " (abundance × flux)."),
                    ],
                );
            }
        }
    });
}

// Pathway manager

/// The draft key of the Create pathway form (`.pathway-create`).
const PATHWAY_CREATE: &str = "map:pathway-create";

/// `pathwayManagerCard(store)`.
pub fn pathway_manager_card(ui: &mut Ui, shell: &mut Shell) {
    let entries = shell.store.pathway_manager_entries();
    let selected_reaction_count = shell.store.multi_selection.reactions.len();
    let badge = entries.len().to_string();
    widgets::card(ui, "Pathway Manager", Some(&badge), |ui| {
        let p = Palette::current(ui.ctx());
        if selected_reaction_count > 0 {
            pathway_create(ui, shell, selected_reaction_count, &p);
        } else {
            ui.add_space(8.0);
            widgets::hint(ui, "Select one or more reactions to create a manual pathway.");
            if Btn::new("Enable multi-select").show(ui).clicked() {
                // `pathway-enable-selection`.
                shell.close_workspace_quietly();
                shell.close_map_menus();
                cancel_text_editor(shell);
                shell.store.clear_geometry_selection();
                let _ = shell.store.set_view(ViewPatch { multi_select: Some(true), geometry_edit: Some(false), text_tool: Some(false), ..Default::default() });
            }
        }
        ui.add_space(10.0);
        if entries.is_empty() {
            widgets::empty(ui, "No annotated pathways among the drawn reactions.");
        }
        for item in &entries {
            pathway_item(ui, shell, item, &p);
            ui.add_space(6.0);
        }
    });
}

/// `.pathway-create`: name and colour for a pathway made of the selected
/// reactions. What is typed is kept in `ui.drafts` until it is created.
fn pathway_create(ui: &mut Ui, shell: &mut Shell, count: usize, p: &Palette) {
    let values = shell.ui.drafts.get(PATHWAY_CREATE).cloned().unwrap_or_default();
    let name = values.get("pathway-name").cloned().unwrap_or_default();
    let color = values.get("pathway-color").cloned().unwrap_or_else(|| "#2563eb".into());
    let mut next: FormValues = values.clone();
    let mut create = false;
    ui.add_space(10.0);
    Frame::new().fill(p.panel).stroke(Stroke::new(1.0, p.line)).corner_radius(CornerRadius::same(9)).inner_margin(Margin::same(9)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        head_row(ui, "Create pathway from selection", &format!("{count} reaction{}", if count == 1 { "" } else { "s" }), p);
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let width = (ui.available_width() - 110.0 - 70.0 - 12.0).max(80.0);
            let (_, event) = widgets::text_field(
                ui,
                egui::Id::new("pathway-create-name"),
                &name,
                Field::new().width(width).placeholder("Pathway name…").aria("New pathway name"),
            );
            if let Some(text) = event.value() {
                next.insert("pathway-name".into(), text);
            }
            if let Some(c) = widgets::color_field(ui, egui::Id::new("pathway-create-color"), &color, "New pathway color") {
                next.insert("pathway-color".into(), c);
            }
            create = Btn::new("Create").primary().show(ui).clicked();
        });
    });
    if next != values {
        shell.ui.drafts.insert(PATHWAY_CREATE.into(), next.clone());
    }
    if create {
        // `pathway-create`.
        let name = next.get("pathway-name").cloned().unwrap_or_default();
        let color = next.get("pathway-color").cloned().unwrap_or_else(|| "#2563eb".into());
        match shell.store.create_manual_pathway(&name, Some(&color), None) {
            Ok(pathway) => {
                // The panel is drawn afresh after a change: the form is empty again.
                shell.ui.drafts.shift_remove(PATHWAY_CREATE);
                shell.center_on_pathway(&format!("manual:{}", pathway.id));
            }
            Err(error) => shell.status_error(error.to_string()),
        }
    }
}

/// One `.pathway-manager-item`.
fn pathway_item(ui: &mut Ui, shell: &mut Shell, item: &coxeter_state::PathwayEntry, p: &Palette) {
    let manual = item.source == "manual";
    let fill = if item.visible { p.panel } else { p.panel_2 };
    Frame::new().fill(fill).stroke(Stroke::new(1.0, p.line)).corner_radius(CornerRadius::same(8)).inner_margin(Margin::same(7)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let aria = format!("Show ribbon for {}", item.name);
            let (r, toggled) = widgets::checkbox(ui, item.visible, "", true, Some(aria.as_str()));
            r.on_hover_text("Show or hide ribbon");
            if let Some(on) = toggled {
                if let Err(error) = shell.store.set_pathway_visible(&item.key, on) {
                    shell.status_error(error.to_string());
                }
            }
            if let Some(color) = widgets::color_field(ui, egui::Id::new(("pathway-color", &item.key)), &item.color, &format!("Color of {}", item.name)) {
                if let Err(error) = shell.store.set_pathway_color(&item.key, &color) {
                    shell.status_error(error.to_string());
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if manual {
                    let delete = Btn::new(RichText::new("×").size(15.0).color(p.danger))
                        .aria(&format!("Delete {}", item.name))
                        .title("Delete manual pathway")
                        .height(26.0)
                        .show(ui);
                    if delete.clicked() {
                        if let Some(id) = item.key.strip_prefix("manual:") {
                            shell.store.delete_manual_pathway(id);
                        }
                    }
                }
                let focus = Btn::new(RichText::new("⌖").size(13.0)).aria(&format!("Focus {}", item.name)).title("Center this pathway on the map").height(26.0).show(ui);
                if focus.clicked() {
                    shell.close_workspace_quietly();
                    shell.close_map_menus();
                    shell.store.select_pathway(&item.key);
                    shell.center_on_pathway(&item.key);
                }
                let select = Btn::new(RichText::new("Select").size(10.5)).aria(&format!("Select {}", item.name)).title("Select all pathway reactions").height(26.0).show(ui);
                if select.clicked() {
                    shell.close_workspace_quietly();
                    shell.close_map_menus();
                    shell.store.select_pathway(&item.key);
                }
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    let ink = if item.visible { p.ink } else { p.ink_3 };
                    if manual {
                        let (_, event) = widgets::text_field(
                            ui,
                            egui::Id::new(("pathway-name", &item.key)),
                            &item.name,
                            Field::new().width(ui.available_width()).aria(&format!("Name of {}", item.name)),
                        );
                        if let FieldEvent::Change(name) = event {
                            if let Some(id) = item.key.strip_prefix("manual:") {
                                if let Err(error) = shell.store.update_manual_pathway(id, PathwayPatch { name: Some(name), ..Default::default() }) {
                                    shell.status_error(error.to_string());
                                }
                            }
                        }
                    } else {
                        ui.add(egui::Label::new(RichText::new(&item.name).size(11.5).family(bold()).color(ink)).truncate()).on_hover_text(&item.name);
                    }
                    let small = format!(
                        "{} · {} drawn reaction{}",
                        if manual { "Manual" } else { "Model" },
                        item.count,
                        if item.count == 1 { "" } else { "s" }
                    );
                    ui.add(egui::Label::new(RichText::new(small).size(11.0).color(p.ink_3)).truncate());
                });
            });
        });
    });
}

// Map structure

/// `SHAPE_LABELS`: the shape presets of Add Compartment and the inspector.
pub const SHAPE_LABELS: [(&str, &str); 5] = [
    ("auto", "Auto"),
    ("rectangle", "Rectangle"),
    ("rounded", "Rounded corners"),
    ("capsule", "Capsule"),
    ("oval", "Oval"),
];

/// The words for a shape preset.
pub fn shape_label(name: &str) -> &'static str {
    SHAPE_LABELS.iter().find(|(n, _)| *n == name).map_or("", |(_, l)| l)
}

/// `compartmentPlacementControls(store, idPrefix)`: what Add Compartment
/// draws next, and in what shape. Every compartment of the model is listed,
/// and the ones that cannot be drawn yet are greyed out with the reason: the
/// model's nesting decides what goes where, and a list of only the drawable
/// ones would give no clue why most organelles are missing.
pub fn compartment_placement_controls(ui: &mut Ui, shell: &mut Shell, id_prefix: &str) {
    let p = Palette::current(ui.ctx());
    let choices = shell.store.map_compartment_choices();
    let current_key = shell.store.current_compartment_placement().map(|c| c.key()).unwrap_or_default();
    let mut species: Vec<&str> = choices.iter().map(|c| c.species_id.as_str()).collect();
    species.dedup();
    let many_species = species.len() > 1;
    let groups: Vec<(String, Vec<(String, String, bool, String)>)> = [
        ("ready", "Ready to draw"),
        ("waiting", "Not yet"),
        ("cell", "Drawn with Add Cell"),
        ("drawn", "On the map"),
    ]
    .iter()
    .filter_map(|(status, label)| {
        let items: Vec<(String, String, bool, String)> = choices
            .iter()
            .filter(|c| c.status == *status)
            .map(|c| {
                let who = if many_species { format!("{} · ", c.species_name) } else { String::new() };
                (c.key(), format!("{who}{} ({})", c.compartment_name, c.compartment_id), c.status == "ready", c.reason.clone())
            })
            .collect();
        (!items.is_empty()).then(|| (label.to_string(), items))
    })
    .collect();
    let width = ui.available_width() - 8.0;
    ui.label(RichText::new("Compartment to draw").size(12.0).family(bold()).color(p.ink_2));
    if let Some(key) =
        widgets::select_grouped_enabled(ui, (id_prefix, "compartment-select"), &current_key, &groups, width, !choices.is_empty(), "Compartment to add")
    {
        shell.store.set_compartment_placement(&key);
    }
    ui.label(RichText::new("Shape").size(12.0).family(bold()).color(p.ink_2));
    let shapes: Vec<(String, String, String)> = SHAPE_LABELS
        .iter()
        .map(|(n, l)| (n.to_string(), l.to_string(), if *n == "auto" { "Its usual shape; organelles are round".to_string() } else { String::new() }))
        .collect();
    let current_shape = shell.store.compartment_shape_preset.clone();
    if let Some(name) = widgets::select_titled(ui, (id_prefix, "compartment-shape"), &current_shape, &shapes, width, true, "Shape of the new compartment") {
        shell.store.set_compartment_shape_preset(&name);
    }
}

/// `compartmentToolControls(store)`: the left panel while Add Compartment is
/// the tool: choose, then drag.
pub fn compartment_tool_controls(ui: &mut Ui, shell: &mut Shell) {
    ui.add_space(4.0);
    compartment_placement_controls(ui, shell, "tool");
    let current = shell.store.current_compartment_placement();
    match current {
        Some(current) => {
            let parent = shell
                .store
                .map_compartment_choices()
                .into_iter()
                .find(|c| c.species_id == current.species_id && c.compartment_id == current.compartment_id)
                .and_then(|c| c.parent_name)
                .or(current.parent_id.clone())
                .unwrap_or_default();
            widgets::hint_rich(ui, &["Drag inside ", parent.as_str(), ", highlighted on the map."]);
        }
        None => {
            widgets::hint(ui, "Nothing can be drawn yet.");
        }
    }
}

/// `mapStructureCard(store)`.
pub fn map_structure_card(ui: &mut Ui, shell: &mut Shell) {
    let available_cells = shell.store.available_map_cells();
    let current_cell_species = available_cells
        .iter()
        .find(|c| Some(&c.species_id) == shell.store.cell_placement_species_id.as_ref())
        .or(available_cells.first())
        .map(|c| c.species_id.clone())
        .unwrap_or_default();
    let cell_options: Vec<(String, String)> = available_cells
        .iter()
        .map(|c| (c.species_id.clone(), format!("{} · boundary {} ({})", c.species_name, c.outer_compartment_id, c.outer_compartment_name)))
        .collect();

    let available = shell.store.available_map_compartments();
    let current_key = shell.store.current_compartment_placement().or_else(|| available.first().cloned()).map(|c| c.key()).unwrap_or_default();

    let (cells, compartments) = match shell.store.map.as_ref() {
        Some(m) => (m.cells().len(), m.cells().iter().map(|c| c.compartments.len()).sum::<usize>()),
        None => (0, 0),
    };
    let badge = format!("{cells} cells · {compartments} compartments");

    widgets::card(ui, "Map structure", Some(&badge), |ui| {
        let p = Palette::current(ui.ctx());
        let block_title = |ui: &mut Ui, s: &str| {
            ui.label(RichText::new(s).size(12.0).family(bold()).color(p.ink_2));
        };
        if !available_cells.is_empty() {
            ui.add_space(9.0);
            block_title(ui, "Cell boundary");
            ui.label(RichText::new("Species to draw").size(12.0).color(p.ink_2));
            let width = ui.available_width() - 8.0;
            if let Some(species) = widgets::select(ui, "cell-species-select", &current_cell_species, &cell_options, width, true, "Species to draw a cell for") {
                shell.store.set_cell_placement_species(&species);
            }
            widgets::hint_rich(ui, &["Pick the ", "Cell", " tool and drag on the map."]);
        } else {
            widgets::hint(ui, "Every active species already has a cell boundary drawn.");
        }
        ui.add_space(9.0);
        ui.separator();
        ui.add_space(4.0);
        block_title(ui, "Nested compartments");
        compartment_placement_controls(ui, shell, "map");
        if !available.is_empty() {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let place_width = 150.0;
                let draw_width = (ui.available_width() - place_width - ui.spacing().item_spacing.x).max(80.0);
                ui.allocate_ui_with_layout(Vec2::new(draw_width, 28.0), egui::Layout::top_down_justified(egui::Align::Center), |ui| {
                    ui.set_width(draw_width);
                    if Btn::new("Draw compartment").primary().wide().show(ui).clicked() {
                        shell.activate_tool(Tool::Compartment);
                    }
                });
                if Btn::new("Place automatically").title("Skip drawing and fit it inside the parent automatically").show(ui).clicked() {
                    // `add-map-compartment`: what the list shows.
                    let key = shell.store.current_compartment_placement().map(|c| c.key()).unwrap_or(current_key.clone());
                    if let Some((species_id, compartment_id)) = key.split_once('|') {
                        if !species_id.is_empty() && !compartment_id.is_empty() {
                            shell.close_map_menus();
                            shell.store.set_compartment_placement(&key);
                            shell.store.add_map_compartment(species_id, compartment_id, None);
                        }
                    }
                }
            });
        } else {
            widgets::empty(ui, "Nothing to draw: draw a cell first, or all compartments are on the map.");
        }
        widgets::hint(ui, "Drag inside the highlighted parent.");
    });
}

// Canvas

/// `mapCanvasCard(store)`.
pub fn map_canvas_card(ui: &mut Ui, shell: &mut Shell) {
    let (width, height) = shell.store.map.as_ref().map_or((0.0, 0.0), |m| (js::round(m.width()), js::round(m.height())));
    widgets::card(ui, "Canvas", None, |ui| {
        let p = Palette::current(ui.ctx());
        ui.horizontal(|ui| {
            ui.label(RichText::new("Dimensions").size(12.0).color(p.ink_3));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(format!("{} × {}", js::number_to_string(width), js::number_to_string(height))).size(12.0).color(p.ink_2));
            });
        });
        widgets::hint(ui, "Drag the canvas edge or corner to resize it.");
        ui.add_space(4.0);
        widgets::grow_row(ui, 2, |ui, i| {
            if i == 0 {
                if Btn::new("Clear drawing").wide().show(ui).clicked() {
                    shell.map_empty();
                }
            } else if Btn::new("Draw full model").wide().show(ui).clicked() {
                shell.map_full();
            }
        });
        widgets::hint(ui, "Replaces the drawing; Undo restores it.");
    });
}

// Reaction search

/// The accessible name of the search field: its placeholder, as a browser
/// names an unlabelled input.
pub const REACTION_SEARCH_LABEL: &str = "ID, name or subsystem…";

/// `reactionSearchCard(store)`.
pub fn reaction_search_card(ui: &mut Ui, shell: &mut Shell) {
    widgets::card(ui, "Find reaction", None, |ui| {
        let query = shell.store.reaction_search_query.clone();
        let response = ui
            .scope(|ui| {
                ui.style_mut().override_font_id = Some(egui::FontId::monospace(12.5));
                let (response, event) = widgets::text_field(ui, reaction_search_id(), &query, Field::new().placeholder(REACTION_SEARCH_LABEL).aria(REACTION_SEARCH_LABEL));
                // `reaction-search`: live; a blur's `change` with the same
                // text does nothing (replacing a result button between
                // pointerdown and click would swallow the click).
                if let Some(text) = event.value() {
                    if shell.store.reaction_search_query != text {
                        shell.store.reaction_search_query = text;
                    }
                }
                response
            })
            .inner;
        shell.take_focus_request(&response);
        ui.add_space(8.0);
        let query = shell.store.reaction_search_query.clone();
        render_reaction_search_results(ui, shell, &query);
    });
}

/// `renderReactionSearchResults(store, query)`, with `search-add-reaction`
/// and `search-select-reaction`.
fn render_reaction_search_results(ui: &mut Ui, shell: &mut Shell, query: &str) {
    let p = Palette::current(ui.ctx());
    let q = query.trim();
    if q.is_empty() {
        widgets::empty(ui, "Search every reaction in the model.");
        return;
    }
    let results = shell.store.search_reactions(q, 30);
    if results.is_empty() {
        widgets::empty(ui, format!("No reactions found for “{q}”."));
        return;
    }
    let mut picked = None;
    for item in &results {
        Frame::new().fill(p.panel).stroke(Stroke::new(1.0, p.line)).corner_radius(CornerRadius::same(8)).inner_margin(Margin::same(10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (label, aria) = if item.drawn {
                        ("Select", format!("Select {}", item.reaction_id))
                    } else {
                        ("Add to map", format!("Add {} to map", item.reaction_id))
                    };
                    let mut button = Btn::new(label).aria(&aria);
                    if !item.drawn {
                        button = button.primary();
                    }
                    if button.show(ui).clicked() {
                        picked = Some((item.reaction_id.clone(), item.drawn));
                    }
                    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        // Named as the map names it, with the name or id beside.
                        ui.add(egui::Label::new(RichText::new(&item.label).monospace().size(11.5).strong().color(p.ink)).truncate());
                        let meta = [item.detail.as_str(), item.subsystem.as_str(), item.species_id.as_str()]
                            .into_iter()
                            .filter(|s| !s.is_empty())
                            .collect::<Vec<_>>()
                            .join(" · ");
                        ui.add(egui::Label::new(RichText::new(meta).size(11.0).color(p.ink_2)).truncate());
                        ui.add(egui::Label::new(RichText::new(if item.drawn { "On map" } else { "Not drawn" }).size(10.0).color(p.ink_3)).truncate());
                    });
                });
            });
        });
        ui.add_space(6.0);
    }
    match picked {
        // `search-add-reaction`.
        Some((rid, false)) => {
            shell.close_workspace_quietly();
            shell.close_map_menus();
            if shell.store.add_map_reaction(&rid, &Default::default()).is_some() {
                shell.select_and_center(&rid);
            }
        }
        // `search-select-reaction`.
        Some((rid, true)) => {
            shell.close_workspace_quietly();
            shell.close_map_menus();
            shell.select_and_center(&rid);
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(value: &str) -> RowInput {
        RowInput::typed(value.into(), "#000000".into())
    }

    #[test]
    fn flux_color_values_validate_like_the_js() {
        let empty = flux_color_values(&[]).unwrap_err();
        assert_eq!(empty.message, "Keep at least one color point in a custom palette.");
        assert!(empty.indices.is_empty());
        let invalid = flux_color_values(&[row("1"), row(""), row("abc"), row(" ")]).unwrap_err();
        assert_eq!(invalid.message, "Enter a finite flux value for every point. The map keeps the last valid palette.");
        assert_eq!(invalid.indices, vec![1, 2, 3]);
        let duplicate = flux_color_values(&[row("1"), row("0"), row("2"), row("-0"), row("1")]).unwrap_err();
        assert_eq!(duplicate.message, "Each color point needs a unique flux value. The map keeps the last valid palette.");
        assert_eq!(duplicate.indices, vec![0, 4, 1, 3], "grouped by the value first seen, as the JS Map");
        let ok = flux_color_values(&[row("-1"), row("1e1")]).unwrap();
        assert_eq!(ok.iter().map(|s| s.value).collect::<Vec<_>>(), vec![-1.0, 10.0]);
    }

    #[test]
    fn new_flux_color_value_goes_past_the_top_first() {
        let stop = |v: f64| FluxColorStop { value: v, color: "#000000".into() };
        assert_eq!(new_flux_color_value(&[stop(-5.0), stop(0.0), stop(5.0)]), Ok(10.0));
        assert_eq!(new_flux_color_value(&[stop(0.0)]), Ok(1.0));
        assert_eq!(new_flux_color_value(&[stop(100.0), stop(1.0)]), Ok(199.0));
        assert_eq!(new_flux_color_value(&[stop(f64::MAX)]), Ok(f64::MAX - 1.0_f64.max(f64::MAX * 0.1)));
    }
}
