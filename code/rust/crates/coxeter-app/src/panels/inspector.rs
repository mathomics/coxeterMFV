//! The inspector (port of `renderInspector`, `renderLayoutSettings` and the
//! inspectors of `src/ui/panels.js`: `inspectorHeader`, `propertyRow`,
//! `mapLabelControl`, `metaboliteInspector`, `geneInspector`,
//! `provenanceBlock`, `notesBlock`, `formatField`, `genesFromRule`,
//! `localMetabolite`, `metaboliteReactionLinks`, `textInspector`,
//! `textStyleFields`, `colorField`, `cellColorFields`,
//! `compartmentColorFields`, `cellInspector`, `compartmentInspector`,
//! `inspectorCard`, `omicsReactionReadout`, `reactionDrawingControls`,
//! `showsSelectionInspector`, `selectionCounts` and `selectionInspector`),
//! with the handlers of `onSidebarClick` / `onSidebarInput` of
//! `src/ui/app.js` for their controls (`acceptNumber`,
//! `selectModelMetabolite`).
//!
//! The panel is drawn from the store as it is; what the user does is
//! gathered as [`Act`]s and applied once the panel is drawn, as the JS
//! handlers run after the HTML is in place.
//!
//! Two deliberate deviations, both where the JS is broken:
//! - "Connected paths" counts the segments drawn to the node
//!   (`segmentsAt`). The JS calls `store.map.segmentsForNode`, which
//!   `EditableMap` does not have, so it always prints 0; its test
//!   (`ui-redesign.test.mjs`) stubs the method to say what it means.
//! - A metabolite's reaction chips select the reaction itself. The JS
//!   prefixes the species to ids of the merged model that already carry it
//!   (`eco@eco@PGI`), which selects nothing.

use coxeter_core::colors::normalize_hex_color;
use coxeter_core::compartments::{Names, compartment_colors};
use coxeter_core::js;
use coxeter_core::label_sources::readable_name;
use coxeter_core::mapdata::NodeKind;
use coxeter_core::text_style::{DEFAULT_TEXT_FONT, TEXT_FONTS, TEXT_SIZE};
use coxeter_io::provenance::annotation_links;
use coxeter_map::editable_map::{ColorChange, DuplicateOptions};
use coxeter_map::layout_settings::RawLayoutSettings;
use coxeter_map::multi_selection::{Selection, selection_main_axis, spacing_items};
use coxeter_core::reaction_label::reaction_label_offset;
use coxeter_render::format_flux;
use coxeter_state::map_edit::GeometryColors;
use coxeter_state::{SelectedEntity, Store, UserNumber};
use egui::{Align, Align2, Color32, CornerRadius, FontId, Frame, Id, Label, Layout, Margin, Response, RichText, Sense, Stroke, Ui, Vec2};
use serde_json::{Map, Value};

use crate::app::Shell;
use crate::fonts::glyph;
use crate::theme::{Palette, bold};
use crate::widgets::{self, Btn, Field, FieldEvent, Look, aria_label, aria_pressed, text};

/// `acceptNumber`'s warning when a field is left empty or not a number.
pub const NOT_A_NUMBER: &str = "Invalid number; value unchanged.";

/// What the user did in the panel: the `data-action`s of the controls drawn.
#[derive(Debug, Clone)]
enum Act {
    ClearSelection,
    NodePrimary(String, bool),
    DuplicateMetabolite(String),
    HideMetabolite(String),
    RemoveMapNode(String),
    RemoveMapText(String),
    ReactionLinear(String, bool),
    ReactionSwap(String, String, String),
    ReactionLineEnd(String, String),
    ReactionAlign(String, &'static str),
    ReactionAlignAxis(String, &'static str),
    ReactionTidy(String),
    ReactionFlip(String),
    ReactionMirror(String),
    ReactionRotate(String, f64),
    ReactionLabelReset(String),
    Arrange(&'static str),
    SelectionSpaceEvenly,
    SelectionCircularize,
    SelectionTidy,
    SelectionPoint(&'static str),
    SelectionRotate(f64),
    ClearMultiSelection,
    Knockout(String),
    GeneKo(String),
    ResetReaction(String),
    ShowInReactions(String),
    ResetAll,
    RemoveMapReaction(String),
    Bound { rid: String, lower: bool, value: UserNumber },
    NotANumber,
    SelectGene { gene: String, species: String },
    SelectModelReaction(String),
    SelectModelMetabolite(String),
    TextFontSize(String, f64),
    TextFontFamily(String, String),
    GeometryColor { cell: String, compartment: Option<String>, fill: bool, color: String },
    ResetColors { cell: String, compartment: Option<String> },
    EditBoundary(Option<String>),
    CompartmentShape { cell: String, compartment: String, shape: &'static str },
    RemoveMapCompartment { cell: String, compartment: Option<String> },
    Colors { nodes: Vec<String>, reactions: Vec<String>, edit: crate::panels::map_options::ColorEdit },
    LabelSource { kind: String, source: String, id: Option<String> },
    Layout(RawLayoutSettings),
    CanvasSize { width: bool, value: f64 },
    LayoutPreset(&'static str),
    ZoomFit,
}

// Entry points

/// `renderInspector(store)`.
pub fn render_inspector(ui: &mut Ui, shell: &mut Shell) {
    let p = shell.palette();
    // A double click on a reaction asks for its lower bound; the request is
    // answered by the field if it is drawn, and dropped otherwise (the JS
    // `setTimeout(() => …querySelector(…)?.focus())`).
    let focus_lower = std::mem::take(&mut shell.ui.focus_lower_bound);
    let mut acts = Vec::new();
    {
        let store = &shell.store;
        ui.spacing_mut().item_spacing.y = 5.0;
        if store.selected_reaction.is_some() {
            inspector_card(ui, store, &p, focus_lower, &mut acts);
        } else if shows_selection_inspector(store) {
            selection_inspector(ui, store, &p, &mut acts);
        } else {
            match store.selected_entity.as_ref() {
                None => inspector_empty(ui, &p, true, "No selection", "Select something on the map to inspect it."),
                Some(selected) => match selected.kind.as_str() {
                    "metabolite" => metabolite_inspector(ui, store, &p, &selected.id, &mut acts),
                    "gene" => {
                        let species = selected.details.get("speciesId").map(js::to_string).unwrap_or_default();
                        gene_inspector(ui, store, &p, &species, &selected.id, &mut acts);
                    }
                    "text" => text_inspector(ui, store, &p, &selected.id, &mut acts),
                    "cell" => cell_inspector(ui, store, &p, &selected.id, &mut acts),
                    "compartment" => compartment_inspector(ui, store, &p, selected.cell_id.as_deref(), &selected.id, &mut acts),
                    _ => inspector_empty(ui, &p, false, "Selection", "No properties are available."),
                },
            }
        }
    }
    apply(shell, acts);
}

/// `renderLayoutSettings(store)`.
pub fn render_layout_settings(ui: &mut Ui, shell: &mut Shell) {
    let p = shell.palette();
    let mut acts = Vec::new();
    layout_settings(ui, &shell.store, &p, &mut acts);
    apply(shell, acts);
}

/// `mapLabelControl(store, kind, id)`: the label source of every
/// `kind` (`metabolite`, `reaction`) object, or of one (`id`).
pub fn map_label_control(ui: &mut Ui, shell: &mut Shell, kind: &str, id: Option<&str>) {
    let mut acts = Vec::new();
    label_control(ui, &shell.store, kind, id, &mut acts);
    apply(shell, acts);
}

// Actions

/// What `onSidebarClick` / `onSidebarInput` do for the inspector's controls.
fn apply(shell: &mut Shell, acts: Vec<Act>) {
    for act in acts {
        match act {
            Act::ClearSelection => shell.store.select(None),
            Act::NodePrimary(node, primary) => {
                shell.store.set_map_node_primary(&node, primary);
            }
            Act::DuplicateMetabolite(node) => {
                if let Some(copy) = shell.store.duplicate_map_metabolite(&node, &DuplicateOptions::default()) {
                    shell.store.select_entity("metabolite", &copy.id, Map::new());
                }
            }
            Act::HideMetabolite(node) => {
                if shell.store.set_map_node_hidden(&node, true).is_some() {
                    shell.store.select(None);
                }
            }
            Act::RemoveMapNode(node) => {
                shell.close_map_menus();
                shell.store.remove_map_node(&node);
            }
            Act::RemoveMapText(id) => {
                shell.map.canvas.gestures.cancel_text_editor();
                shell.drain_canvas_events();
                shell.store.remove_map_text(&id);
            }
            Act::ReactionLinear(rid, on) => {
                shell.store.set_map_reaction_linear(&rid, on);
            }
            Act::ReactionSwap(rid, a, b) => {
                shell.store.swap_map_reaction_metabolites(&rid, &a, &b);
            }
            Act::ReactionLineEnd(rid, node) => {
                shell.store.set_map_reaction_line_end(&rid, &node);
            }
            Act::ReactionAlign(rid, direction) => {
                shell.store.align_map_reaction(&rid, direction);
            }
            Act::ReactionAlignAxis(rid, axis) => {
                shell.store.align_map_reaction_axis(&rid, axis);
            }
            Act::ReactionTidy(rid) => {
                shell.store.tidy_map_reaction(&rid);
            }
            Act::ReactionFlip(rid) => {
                shell.store.flip_map_reaction(&rid);
            }
            Act::ReactionMirror(rid) => {
                shell.store.mirror_map_reaction(&rid);
            }
            Act::ReactionRotate(rid, degrees) => {
                shell.store.rotate_map_reaction(&rid, (degrees * std::f64::consts::PI) / 180.0);
            }
            Act::ReactionLabelReset(rid) => {
                shell.store.reset_reaction_label(&rid);
            }
            Act::Arrange(action) => {
                shell.close_map_menus();
                shell.store.arrange_multi_selection(action);
            }
            Act::SelectionSpaceEvenly => {
                shell.close_map_menus();
                shell.store.space_multi_selection_evenly();
            }
            Act::SelectionCircularize => {
                shell.close_map_menus();
                shell.store.circularize_multi_selection();
            }
            Act::SelectionTidy => {
                shell.close_map_menus();
                shell.store.tidy_multi_selection();
            }
            Act::SelectionPoint(direction) => {
                shell.close_map_menus();
                shell.store.point_multi_selection(direction);
            }
            Act::SelectionRotate(degrees) => {
                shell.close_map_menus();
                shell.store.rotate_multi_selection((degrees * std::f64::consts::PI) / 180.0);
            }
            Act::ClearMultiSelection => {
                shell.close_map_menus();
                shell.store.clear_multi_selection(true);
            }
            Act::Knockout(rid) => {
                if let Err(error) = shell.store.knockout(&rid) {
                    shell.status_error(error.to_string());
                }
            }
            Act::ResetReaction(rid) => {
                if let Err(error) = shell.store.reset_reaction(&rid) {
                    shell.status_error(error.to_string());
                }
            }
            Act::GeneKo(key) => {
                if let Err(error) = shell.store.toggle_gene_knockout(&key) {
                    shell.status_error(error.to_string());
                }
            }
            Act::ResetAll => {
                if let Err(error) = shell.store.reset_all_reactions() {
                    shell.status_error(error.to_string());
                }
            }
            Act::ShowInReactions(rid) => shell.reveal_in_reactions(&rid),
            Act::RemoveMapReaction(rid) => {
                shell.close_map_menus();
                shell.store.remove_map_reaction(&rid);
            }
            Act::Bound { rid, lower, value } => {
                let result = if lower {
                    shell.store.set_reaction_bounds(&rid, value, UserNumber::Empty)
                } else {
                    shell.store.set_reaction_bounds(&rid, UserNumber::Empty, value)
                };
                match result {
                    Ok(_) => shell.schedule_solve(),
                    Err(error) => shell.status_error(error.to_string()),
                }
            }
            Act::NotANumber => shell.status_warn(NOT_A_NUMBER),
            Act::SelectGene { gene, species } => {
                shell.store.selected_reaction = None;
                let mut entity = SelectedEntity::new("gene", &gene);
                entity.details.insert("speciesId".into(), Value::String(species));
                shell.store.selected_entity = Some(entity);
                shell.store.emit("selection");
            }
            Act::SelectModelReaction(rid) => shell.store.select(Some(&rid)),
            Act::SelectModelMetabolite(merged) => select_model_metabolite(shell, &merged),
            Act::TextFontSize(id, size) => {
                shell.store.style_map_text(&id, Some(size), None);
            }
            Act::TextFontFamily(id, family) => {
                shell.store.style_map_text(&id, None, Some(&family));
            }
            Act::GeometryColor { cell, compartment, fill, color } => {
                let colors = if fill {
                    GeometryColors { fill: ColorChange::Set(color), ..Default::default() }
                } else {
                    GeometryColors { stroke: ColorChange::Set(color), ..Default::default() }
                };
                shell.store.set_map_geometry_colors(&cell, compartment.as_deref(), &colors);
            }
            Act::ResetColors { cell, compartment } => {
                let colors = GeometryColors { fill: ColorChange::Reset, stroke: ColorChange::Reset };
                shell.store.set_map_geometry_colors(&cell, compartment.as_deref(), &colors);
            }
            Act::EditBoundary(cell) => shell.edit_selected_boundary(cell.as_deref()),
            Act::Colors { nodes, reactions, edit } => {
                crate::panels::map_options::apply_color_edit(&mut shell.store, &nodes, &reactions, edit);
            }
            Act::CompartmentShape { cell, compartment, shape } => {
                if let Some(shape) = coxeter_core::geometry::compartment_shape_preset(shape) {
                    shell.store.set_compartment_shape(Some(&cell), &compartment, coxeter_map::geometry_edit::ShapePatch::of(shape));
                }
            }
            Act::RemoveMapCompartment { cell, compartment } => {
                // A cell's own button names no compartment: its outermost one.
                let target = match compartment {
                    Some(c) => Some((cell, c)),
                    None => shell.store.map.as_ref().and_then(|m| m.cell(&cell)).and_then(|c| {
                        c.compartments
                            .iter()
                            .find(|k| Some(&k.id) == c.outer_compartment.as_ref())
                            .or_else(|| c.compartments.iter().find(|k| k.parent_id.is_none()))
                            .map(|k| (cell.clone(), k.id.clone()))
                    }),
                };
                if let Some((cell, compartment)) = target {
                    shell.close_map_menus();
                    shell.store.remove_map_compartment(&cell, &compartment);
                }
            }
            Act::LabelSource { kind, source, id } => {
                shell.store.set_map_label_source(&kind, &source, id.as_deref());
            }
            Act::Layout(patch) => {
                shell.refit();
                shell.store.set_layout_settings(patch);
            }
            Act::CanvasSize { width, value } => {
                let Some(map) = shell.store.map.as_ref() else { continue };
                let (w, h) = if width { (value, map.height()) } else { (map.width(), value) };
                let tx = shell.store.begin_map_edit("Resize canvas from layout settings");
                shell.store.set_map_canvas_size(Some(w), Some(h), 0.0, 0.0);
                shell.store.commit_map_edit(tx);
                shell.refit();
            }
            Act::LayoutPreset(name) => shell.layout_preset(name),
            Act::ZoomFit => shell.zoom_fit(),
        }
    }
}

/// `selectModelMetabolite(mergedId)`: select a drawn instance of a model
/// metabolite, or say that none is drawn.
fn select_model_metabolite(shell: &mut Shell, merged_id: &str) {
    let node = shell
        .store
        .map
        .as_ref()
        .and_then(|m| m.nodes().values().find(|n| n.merged_id.as_deref() == Some(merged_id)).map(|n| n.id.clone()));
    match node {
        Some(id) => {
            shell.store.selected_reaction = None;
            shell.store.selected_entity = Some(SelectedEntity::new("metabolite", &id));
            shell.store.emit("selection");
        }
        None => shell.status_ok(format!("Metabolite {merged_id} is not drawn on the map.")),
    }
}

// Pieces

/// `.inspector-empty`, with the `⌁` icon or without.
fn inspector_empty(ui: &mut Ui, p: &Palette, icon: bool, title: &str, body: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(if icon { 80.0 } else { 100.0 });
        if icon {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(48.0), Sense::hover());
            ui.painter().circle_filled(rect.center(), 24.0, p.accent_soft);
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, "⌁", FontId::proportional(25.0), p.accent);
            ui.add_space(8.0);
        }
        ui.label(RichText::new(title).size(14.0).family(bold()).color(p.ink));
        ui.add_space(2.0);
        ui.allocate_ui_with_layout(Vec2::new(220.0_f32.min(ui.available_width()), 0.0), Layout::top_down(Align::Center), |ui| {
            ui.add(Label::new(text(body, 11.5, p.ink_3)).wrap());
        });
    });
}

/// `inspectorHeader(kind, title, subtitle)`: the kind, the × that clears the
/// selection (`clear-selection`), the title and what it belongs to.
fn inspector_header(ui: &mut Ui, p: &Palette, kind: &str, title: &str, subtitle: &str, acts: &mut Vec<Act>) {
    inspector_header_closing(ui, p, kind, title, subtitle, Act::ClearSelection, acts);
}

/// `inspectorHeader(kind, title, subtitle, closeAction)`: one whose ×
/// does `close`.
fn inspector_header_closing(ui: &mut Ui, p: &Palette, kind: &str, title: &str, subtitle: &str, close: Act, acts: &mut Vec<Act>) {
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(kind.to_uppercase()).size(10.0).family(bold()).color(p.ink_3).extra_letter_spacing(0.5));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let button = Btn::new(RichText::new("×").size(18.0).color(p.ink_2)).look(Look::Flat).title("Close").aria("Clear selection").height(24.0).show(ui);
            if button.clicked() {
                acts.push(close);
            }
        });
    });
    ui.add(Label::new(RichText::new(title).size(17.0).family(bold()).color(p.ink)).wrap());
    if !subtitle.is_empty() {
        ui.add(Label::new(text(subtitle, 11.5, p.ink_3)).wrap());
    }
    ui.add_space(6.0);
    let rect = ui.min_rect();
    ui.painter().hline(rect.x_range(), ui.cursor().top(), Stroke::new(1.0, p.line));
    ui.add_space(4.0);
}

/// `.inspector-note` with `<b>` runs: `parts` alternate plain and bold.
fn note_rich(ui: &mut Ui, p: &Palette, parts: &[&str]) -> Response {
    let mut job = egui::text::LayoutJob::default();
    for (i, part) in parts.iter().enumerate() {
        let family = if i % 2 == 1 { bold() } else { egui::FontFamily::Proportional };
        job.append(part, 0.0, egui::TextFormat { font_id: FontId::new(11.0, family), color: p.ink_3, ..Default::default() });
    }
    job.wrap.max_width = ui.available_width();
    ui.add(Label::new(job).wrap())
}

/// A line of text with `<b>` runs, at a size and colour.
fn rich_line(ui: &mut Ui, parts: &[(&str, bool)], size: f32, color: egui::Color32) -> Response {
    let mut job = egui::text::LayoutJob::default();
    for (part, strong) in parts {
        let family = if *strong { bold() } else { egui::FontFamily::Proportional };
        job.append(part, 0.0, egui::TextFormat { font_id: FontId::new(size, family), color, ..Default::default() });
    }
    job.wrap.max_width = ui.available_width();
    ui.add(Label::new(job).wrap())
}

/// `propertyRow(label, value)` with a plain value.
fn property(ui: &mut Ui, label: &str, value: impl Into<String>) {
    let value = value.into();
    widgets::property_row(ui, label, false, |ui| {
        widgets::value_text(ui, value);
    });
}

/// `propertyRow(label, '<code>…</code>')`.
fn property_code(ui: &mut Ui, label: &str, value: &str) {
    widgets::property_row(ui, label, false, |ui| {
        ui.add(Label::new(widgets::code(value).size(10.5)).wrap());
    });
}

/// `.metabolite-inspector-controls`: the boxed group of map controls.
fn controls_box<R>(ui: &mut Ui, p: &Palette, add: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::new()
        .fill(p.panel_2)
        .stroke(Stroke::new(1.0, p.line))
        .corner_radius(CornerRadius::same(9))
        .inner_margin(Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 7.0;
            add(ui)
        })
        .inner
}

/// `.metabolite-role-label`.
fn role_label(ui: &mut Ui, p: &Palette, s: &str) -> Response {
    ui.label(text(s, 11.0, p.ink_3))
}

/// `formatField(v)`.
fn format_field(v: &Value) -> String {
    match v {
        Value::Array(items) => items.iter().map(|x| if x.is_null() { String::new() } else { js::to_string(x) }).collect::<Vec<_>>().join(", "),
        Value::Object(_) => js::stringify(v, None),
        Value::Null => String::new(),
        other => js::to_string(other),
    }
}

/// `genesFromRule(rule)`: the gene ids of a GPR rule, once each.
fn genes_from_rule(rule: &str) -> Vec<String> {
    let cleaned = rule.replace(['(', ')'], " ");
    let mut out: Vec<String> = Vec::new();
    for token in cleaned.split_whitespace() {
        let lower = token.to_ascii_lowercase();
        if lower == "and" || lower == "or" {
            continue;
        }
        if !out.iter().any(|g| g == token) {
            out.push(token.to_string());
        }
    }
    out
}

/// `provenanceBlock(entity, type)`: database links of an entity, or `—`.
fn provenance_block(ui: &mut Ui, p: &Palette, annotation: &Value, notes: &Value, kind: &str) {
    let links = annotation_links(annotation, notes, kind);
    if links.is_empty() {
        widgets::value_text(ui, "—");
        return;
    }
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        for link in links {
            match link.url {
                Some(url) => {
                    let label = format!("{}: {} ↗", link.label, link.value);
                    ui.add(egui::Hyperlink::from_label_and_url(RichText::new(label).size(12.0).color(p.accent), url).open_in_new_tab(true));
                }
                None => {
                    rich_line(ui, &[(&link.label, true), (&format!(": {}", link.value), false)], 12.0, p.ink);
                }
            }
        }
    });
}

/// `notesBlock(notes)`.
fn notes_block(ui: &mut Ui, p: &Palette, notes: &Value) {
    let Some(entries) = notes.as_object().filter(|o| !o.is_empty()) else {
        widgets::value_text(ui, "—");
        return;
    };
    for (key, value) in entries {
        rich_line(ui, &[(key, true), (&format!(": {}", format_field(value)), false)], 12.0, p.ink);
    }
}

/// A wrapped row of clickable chips, or `—` when there are none.
fn chips(ui: &mut Ui, items: &[(String, String)], mut on_click: impl FnMut(&str)) {
    if items.is_empty() {
        widgets::value_text(ui, "—");
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(3.0, 3.0);
        for (label, target) in items {
            if widgets::chip_button(ui, label, false).clicked() {
                on_click(target);
            }
        }
    });
}

/// `pickerValue(...candidates)`: the first `#rrggbb` of the candidates.
fn picker_value(candidates: &[Option<&str>]) -> String {
    candidates.iter().flatten().find_map(|c| normalize_hex_color(c)).unwrap_or_else(|| "#64748b".into())
}

/// `colorField(label, action, ids, value)`: a `.field` with a colour input,
/// applied when the picker closes (`change`).
fn color_field(ui: &mut Ui, p: &Palette, label: &str, id: Id, value: &str) -> Option<String> {
    let mut picked = None;
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(Vec2::new(86.0, 22.0), Layout::left_to_right(Align::Center), |ui| {
            ui.set_min_width(86.0);
            ui.label(text(label, 11.5, p.ink_2));
        });
        picked = widgets::color_field(ui, id, value, label);
    });
    picked
}

/// A `.field` row: its label, then the control.
fn field_row<R>(ui: &mut Ui, p: &Palette, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(Vec2::new(86.0, 22.0), Layout::left_to_right(Align::Center), |ui| {
            ui.set_min_width(86.0);
            ui.label(text(label, 11.5, p.ink_2));
        });
        add(ui)
    })
    .inner
}

/// The theme colours of a compartment (`compartmentColors(id, dark)`).
fn theme_colors(id: &str, dark: bool) -> coxeter_core::compartments::CompartmentColors {
    compartment_colors(id, dark, &Names::new())
}

/// A visual id, or the id when it has none (`comp.visualId ?? comp.id`).
fn visual_or<'a>(visual: &'a str, id: &'a str) -> &'a str {
    if visual.is_empty() { id } else { visual }
}

/// `Math.round(v)` as text.
fn round_text(v: f64) -> String {
    js::number_to_string(js::round(v))
}

/// The value egui shows a slider at: clamped into its range and snapped to
/// its step from the start, as a range input does. egui reports that snap as
/// a change; the browser fires nothing, so neither does the panel.
fn snapped(value: f64, min: f64, max: f64, step: f64) -> f64 {
    let (lo, hi) = if min <= max { (min, max) } else { (max, min) };
    let v = if value.is_nan() { lo } else { value.clamp(lo, hi) };
    if step > 0.0 && step.is_finite() { lo + ((v - lo) / step).round() * step } else { v }
}

/// A slider event the user caused (not egui snapping the shown value).
fn slider_event(response: &Response, event: FieldEvent<f64>, value: f64, min: f64, max: f64, step: f64) -> FieldEvent<f64> {
    let real = |v: f64| response.dragged() || response.drag_stopped() || v != snapped(value, min, max, step);
    match event {
        FieldEvent::Input(v) if real(v) => FieldEvent::Input(v),
        FieldEvent::Change(v) if real(v) => FieldEvent::Change(v),
        _ => FieldEvent::None,
    }
}

// Map label control

/// `mapLabelControl(store, kind, id)`.
fn label_control(ui: &mut Ui, store: &Store, kind: &str, id: Option<&str>, acts: &mut Vec<Act>) {
    let p = Palette::current(ui.ctx());
    let metabolite = kind == "metabolite";
    let global = store.map_label_source(kind);
    let selected = match id {
        Some(id) => {
            let own = store.map.as_ref().and_then(|m| {
                let extra = if metabolite { m.node(id).map(|n| &n.extra) } else { m.reaction(id).map(|r| &r.extra) };
                extra.and_then(|e| e.get("labelSource")).filter(|v| !v.is_null()).map(js::to_string)
            });
            own.unwrap_or_else(|| "inherit".into())
        }
        None => global,
    };
    let mut options: Vec<(String, String)> = Vec::new();
    if id.is_some() {
        options.push(("inherit".into(), "Follow View setting".into()));
    }
    for option in store.map_label_options(kind, id) {
        let preview = id.map(|id| store.map_label_text(kind, id, Some(&option.value))).unwrap_or_default();
        let shown = if preview.is_empty() { option.label.clone() } else { format!("{}: {preview}", option.label) };
        options.push((option.value, shown));
    }
    let label = if id.is_some() {
        "Map label"
    } else if metabolite {
        "Metabolite labels"
    } else {
        "Reaction labels"
    };
    let aria = if metabolite { "Metabolite label source" } else { "Reaction label source" };
    let salt = ("label-source", kind.to_string(), id.map(str::to_string));
    let picked = if id.is_some() {
        // `.inspector-detail .map-label-control`: the select under its label.
        ui.label(text(label, 12.0, p.ink_2));
        let width = ui.available_width() - 8.0;
        widgets::select(ui, salt, &selected, &options, width, true, aria)
    } else {
        // `.view-label-control`: label and select on one line.
        ui.horizontal(|ui| {
            ui.label(text(label, 12.5, p.ink_2));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| widgets::select(ui, salt, &selected, &options, 150.0, true, aria)).inner
        })
        .inner
    };
    if let Some(source) = picked {
        acts.push(Act::LabelSource { kind: kind.to_string(), source, id: id.map(str::to_string) });
    }
}

// Metabolite and gene

/// `metaboliteInspector(store, nodeId)`.
fn metabolite_inspector(ui: &mut Ui, store: &Store, p: &Palette, node_id: &str, acts: &mut Vec<Act>) {
    let Some(node) = store.map.as_ref().and_then(|m| m.node(node_id)) else {
        inspector_empty(ui, p, false, "Metabolite", "The selected metabolite is no longer present.");
        return;
    };
    let merged_id = node.merged_id.clone().unwrap_or_default();
    let metabolite = store.community.as_ref().and_then(|c| c.merged.metabolites.get(&merged_id));
    let owner = merged_id.split('@').next().unwrap_or("").to_string();
    let species = store.species.iter().find(|s| s.id == owner);
    let model = species.and_then(|s| store.models.get(&s.file));
    // Shared-medium ids start with the medium's own prefix (`M@ac`), which is
    // not the id of any cell.
    let shared_medium = node.kind == NodeKind::Medium || (species.is_none() && node.cell_id.is_none() && owner == "M");
    let cell_name = if shared_medium {
        "Shared medium".to_string()
    } else {
        let name = model.map_or(owner.clone(), |m| m.cell.name.clone());
        if name.is_empty() { "—".into() } else { name }
    };
    let connected = store.map.as_ref().map_or(0, |m| m.segments_at(node_id).len());
    let shadow_price = store.solution.as_ref().and_then(|s| s.result.as_ref()).and_then(|r| r.shadow_prices.get(&merged_id)).copied();
    let copies = store.metabolite_copies(node_id).len();
    let removable = store.can_remove_map_node(node_id);

    let title = node.base.clone().or_else(|| metabolite.map(|m| m.id.clone())).unwrap_or_else(|| node.id.clone());
    // The heading gives one readable name; a model name that lists synonyms
    // is shown whole below it.
    let model_name = metabolite.map_or("", |m| m.name.as_str());
    let name = if model_name.is_empty() {
        String::new()
    } else {
        readable_name(model_name, &[metabolite.map_or("", |m| m.id.as_str()), node.base.as_deref().unwrap_or("")])
    };
    inspector_header(ui, p, "Metabolite", &title, &name, acts);
    property_code(ui, "Map ID", &node.id);
    property_code(ui, "Model ID", &metabolite.map_or_else(|| merged_id.clone(), |m| m.id.clone()));
    if name != model_name {
        property(ui, "Model name", model_name);
    }
    property(ui, "Formula", metabolite.map_or_else(|| "—".to_string(), |m| m.formula.clone()));
    property(ui, "Shadow price", match shadow_price {
        Some(v) if v.is_finite() => format_flux(Some(v), 6),
        _ => "—".into(),
    });
    property(ui, "Charge", match metabolite {
        Some(m) if m.charge.is_finite() => js::number_to_string(m.charge),
        _ => "—".into(),
    });
    property(ui, "Compartment", node.compartment.clone().unwrap_or_else(|| "—".into()));
    property(ui, "Cell", cell_name);
    // A cofactor copy beside one reaction is secondary until made primary.
    let cofactor = node.kind == NodeKind::Currency;
    let secondary = cofactor || node.primary == Some(false);
    property(ui, "Visual role", if secondary { "Secondary" } else { "Primary" });
    property(ui, "Visual copies", copies.max(1).to_string());
    property(ui, "Connected paths", connected.to_string());
    property(ui, "Radius", format!("{} px", js::number_to_string(node.r)));
    widgets::property_row(ui, "Reactions", true, |ui| {
        // `metaboliteReactionLinks(store, met, owner)`.
        let items: Vec<(String, String)> = metabolite
            .map(|m| {
                m.reactions
                    .iter()
                    .map(|rid| {
                        let prefixed = !owner.is_empty() && owner != "M" && !rid.starts_with(&format!("{owner}@"));
                        (rid.clone(), if prefixed { format!("{owner}@{rid}") } else { rid.clone() })
                    })
                    .collect()
            })
            .unwrap_or_default();
        chips(ui, &items, |rid| acts.push(Act::SelectModelReaction(rid.to_string())));
    });
    widgets::property_row(ui, "Annotations", true, |ui| {
        // `localMetabolite(store, mergedId) ?? metabolite`.
        let local = store.community.as_ref().and_then(|c| c.metabolite_owner.get(&merged_id)).and_then(|o| {
            let sp = store.species.iter().find(|s| Some(&s.id) == o.species_id.as_ref())?;
            store.models.get(&sp.file)?.metabolites.get(&o.original_id)
        });
        match local.or(metabolite) {
            Some(m) => provenance_block(ui, p, &m.annotation, &m.notes, "metabolite"),
            None => provenance_block(ui, p, &Value::Null, &Value::Null, "metabolite"),
        }
    });

    widgets::section_title(ui, "Visual map instance");
    label_control(ui, store, "metabolite", Some(node_id), acts);
    ui.add_space(6.0);
    controls_box(ui, p, |ui| {
        ui.horizontal(|ui| {
            role_label(ui, p, "Visual role");
            widgets::grow_row(ui, 2, |ui, i| {
                let (label, on) = if i == 0 { ("Primary", !secondary) } else { ("Secondary", secondary) };
                let mut button = Btn::new(label).active(on);
                if i == 0 && cofactor {
                    button = button.title("Draw it as a main metabolite of its reaction");
                }
                let r = button.show(ui);
                aria_pressed(&r, on);
                if r.clicked() {
                    acts.push(Act::NodePrimary(node_id.to_string(), i == 0));
                }
            });
        });
        widgets::note(ui, "Primary = 13 px · Secondary = 7 px");
        let ids = vec![node_id.to_string()];
        if let Some(edit) = crate::panels::map_options::node_color_row(ui, store, &ids, node_id) {
            acts.push(Act::Colors { nodes: ids, reactions: Vec::new(), edit });
        }
        if Btn::new("Duplicate metabolite").wide().show(ui).clicked() {
            acts.push(Act::DuplicateMetabolite(node_id.to_string()));
        }
        if Btn::new("Hide metabolite").danger().wide().show(ui).clicked() {
            acts.push(Act::HideMetabolite(node_id.to_string()));
        }
        if removable {
            if Btn::new("Delete metabolite from map").look(Look::DangerBox).wide().height(34.0).show(ui).clicked() {
                acts.push(Act::RemoveMapNode(node_id.to_string()));
            }
        } else {
            widgets::note(ui, "Remove the drawn reactions that use it before deleting it.");
        }
    });
}

/// `geneInspector(store, speciesId, geneId)`.
fn gene_inspector(ui: &mut Ui, store: &Store, p: &Palette, species_id: &str, gene_id: &str, acts: &mut Vec<Act>) {
    let model = store.species.iter().find(|s| s.id == species_id).and_then(|s| store.models.get(&s.file));
    let gene = model.and_then(|m| m.genes.iter().find(|g| g.get("id").map(js::to_string).as_deref() == Some(gene_id)));
    let (Some(model), Some(gene)) = (model, gene) else {
        inspector_empty(ui, p, false, "Gene", &format!("Gene {gene_id} not found."));
        return;
    };
    let reactions: Vec<(String, String)> = model
        .reactions
        .values()
        .filter(|r| genes_from_rule(&r.gene_reaction_rule).iter().any(|g| g == gene_id))
        .map(|r| (r.id.clone(), format!("{species_id}@{}", r.id)))
        .collect();
    let id = gene.get("id").map(js::to_string).unwrap_or_default();
    let name = gene.get("name").filter(|v| !v.is_null()).map(js::to_string).unwrap_or_default();
    inspector_header(ui, p, "Gene", &id, &name, acts);
    if let Some(fields) = gene.as_object() {
        for (key, value) in fields {
            if ["annotation", "annotations", "notes"].contains(&key.as_str()) {
                continue;
            }
            property(ui, key, format_field(value));
        }
    }
    property(ui, "Cell / species", model.cell.name.clone());
    widgets::property_row(ui, "Reactions", true, |ui| {
        chips(ui, &reactions, |rid| acts.push(Act::SelectModelReaction(rid.to_string())));
    });
    let key = if species_id.is_empty() { id.clone() } else { format!("{species_id}@{id}") };
    if !store.is_zuniga_study() && store.gene_index().genes.contains_key(&key) {
        ui.add_space(6.0);
        let on = store.gene_knockouts.contains(&key);
        let button = Btn::new(if on { "Restore gene" } else { "Knock out gene" }).wide();
        if (if on { button } else { button.danger() }).show(ui).clicked() {
            acts.push(Act::GeneKo(key.clone()));
        }
        widgets::hint(ui, gene_effect_text(store, &key));
    }
    let empty = Value::Object(Map::new());
    let annotation = gene.get("annotation").unwrap_or(&empty);
    let notes = gene.get("notes").unwrap_or(&empty);
    widgets::property_row(ui, "Annotations", true, |ui| provenance_block(ui, p, annotation, notes, "gene"));
    widgets::property_row(ui, "Notes", true, |ui| notes_block(ui, p, notes));
}

// Texts

/// `textInspector(store, textId)`.
fn text_inspector(ui: &mut Ui, store: &Store, p: &Palette, text_id: &str, acts: &mut Vec<Act>) {
    let Some(item) = store.map.as_ref().and_then(|m| m.text(text_id)) else {
        inspector_empty(ui, p, false, "Text", "The selected text is no longer present.");
        return;
    };
    inspector_header(ui, p, "Text", &item.text, &item.id, acts);
    property_code(ui, "ID", &item.id);
    property(ui, "Content", item.text.clone());
    property(ui, "X", format_flux(Some(item.x), 2));
    property(ui, "Y", format_flux(Some(item.y), 2));
    ui.add_space(6.0);
    widgets::section_label(ui, "STYLE");
    text_style_fields(ui, p, item.font_size, item.font_family.as_deref(), text_id, acts);
    widgets::note(ui, "Also used for the next text you write.");
    if Btn::new("Delete text").look(Look::DangerBox).wide().height(34.0).show(ui).clicked() {
        acts.push(Act::RemoveMapText(text_id.to_string()));
    }
}

/// `textStyleFields({ fontSize, fontFamily }, ids)`: size and font of a map
/// text. The size is applied when the field is left or Enter pressed; a
/// cleared field keeps the size (it shows it again).
fn text_style_fields(ui: &mut Ui, p: &Palette, font_size: Option<f64>, font_family: Option<&str>, text_id: &str, acts: &mut Vec<Act>) {
    let size = font_size.filter(|v| v.is_finite()).unwrap_or(TEXT_SIZE.default);
    let family = font_family.unwrap_or(DEFAULT_TEXT_FONT).to_string();
    field_row(ui, p, "Size", |ui| {
        let id = Id::new(("text-font-size", text_id.to_string()));
        let (_, event) = widgets::number_field(ui, id, &js::number_to_string(size), Field::new().width(90.0).aria("Text size in pixels"));
        if let FieldEvent::Change(value) = event {
            let n = widgets::value_as_number(&value);
            if n.is_finite() {
                acts.push(Act::TextFontSize(text_id.to_string(), n));
            }
        }
    });
    field_row(ui, p, "Font", |ui| {
        let options: Vec<(String, String)> = TEXT_FONTS.iter().map(|f| (f.id.to_string(), f.label.to_string())).collect();
        let width = ui.available_width() - 8.0;
        if let Some(font) = widgets::select(ui, ("text-font-family", text_id.to_string()), &family, &options, width, true, "Text font") {
            acts.push(Act::TextFontFamily(text_id.to_string(), font));
        }
    });
}

// Cells and compartments

/// `cellInspector(store, cellId)`.
fn cell_inspector(ui: &mut Ui, store: &Store, p: &Palette, cell_id: &str, acts: &mut Vec<Act>) {
    let Some(cell) = store.map.as_ref().and_then(|m| m.cell(cell_id)) else {
        inspector_empty(ui, p, false, "Cell", "The selected cell is no longer present.");
        return;
    };
    let species = store.species.iter().find(|s| s.id == cell.id);
    let model = species.and_then(|s| store.models.get(&s.file));
    let title = if cell.name.is_empty() { cell.id.clone() } else { cell.name.clone() };
    inspector_header(ui, p, "Cell", &title, model.map_or("", |m| m.cell.organism.as_str()), acts);
    property_code(ui, "ID", &cell.id);
    property(ui, "Compartments", cell.compartments.len().to_string());
    property(ui, "Outer compartment", cell.outer_compartment.clone().unwrap_or_else(|| "—".into()));
    property(ui, "Abundance", species.map_or_else(|| "—".to_string(), |s| format!("{}%", round_text(s.abundance * 100.0))));
    property(ui, "Growth rate", store.growth.get(&cell.id).map_or_else(|| "—".to_string(), |g| format!("{} h⁻¹", format_flux(Some(*g), 4))));
    property(ui, "Canvas bounds", format!("{} × {}", round_text(cell.rect.w), round_text(cell.rect.h)));
    ui.add_space(6.0);
    widgets::section_label(ui, "MAP");

    // `cellColorFields(store, cell)`: the fill of a cell is the fill of its body.
    let body = cell
        .compartments
        .iter()
        .find(|c| Some(&c.id) == cell.outer_compartment.as_ref())
        .or_else(|| cell.compartments.iter().find(|c| c.parent_id.is_none()));
    let theme_fill = body.map(|b| theme_colors(visual_or(&b.visual_id, &b.id), store.view.dark).fill);
    let fill = picker_value(&[body.and_then(|b| b.fill.as_deref()), theme_fill]);
    let border = picker_value(&[cell.stroke.as_deref(), Some(cell.color.as_str())]);
    let custom = body.is_some_and(|b| b.fill.is_some()) || cell.stroke.is_some();
    if let Some(color) = color_field(ui, p, "Fill color", Id::new(("cell-fill-color", cell_id.to_string())), &fill) {
        acts.push(Act::GeometryColor { cell: cell_id.to_string(), compartment: None, fill: true, color });
    }
    if let Some(color) = color_field(ui, p, "Border color", Id::new(("cell-border-color", cell_id.to_string())), &border) {
        acts.push(Act::GeometryColor { cell: cell_id.to_string(), compartment: None, fill: false, color });
    }
    if custom && Btn::new("Use theme colors").look(Look::Secondary).wide().show(ui).clicked() {
        acts.push(Act::ResetColors { cell: cell_id.to_string(), compartment: None });
    }
    if Btn::new("Edit Boundary").look(Look::Secondary).wide().show(ui).clicked() {
        acts.push(Act::EditBoundary(Some(cell_id.to_string())));
    }
    if Btn::new("Delete cell").look(Look::DangerBox).wide().show(ui).clicked() {
        acts.push(Act::RemoveMapCompartment { cell: cell_id.to_string(), compartment: None });
    }
    widgets::note(ui, "Delete key also works; Ctrl+Z undoes.");
}

/// `compartmentShapeFields(cellId, comp)`: ready-made shapes for a
/// compartment, and how to get any shape in between.
fn compartment_shape_fields(ui: &mut Ui, p: &Palette, cell_id: &str, comp: &coxeter_core::mapdata::MapCompartment, acts: &mut Vec<Act>) {
    let now = coxeter_core::geometry::compartment_shape(comp);
    widgets::section_label(ui, "SHAPE");
    ui.horizontal_wrapped(|ui| {
        for (name, shape) in coxeter_core::geometry::COMPARTMENT_SHAPES {
            let on = (shape.corner - now.corner).abs() < 0.01 && (shape.oval - now.oval).abs() < 0.01;
            let r = Btn::new(crate::panels::map_options::shape_label(name)).active(on).show(ui);
            aria_pressed(&r, on);
            if r.clicked() {
                acts.push(Act::CompartmentShape { cell: cell_id.to_string(), compartment: comp.id.clone(), shape: name });
            }
        }
    });
    note_rich(ui, p, &["Or drag the ", "◆ orange diamond", " to round the corners and the ", "◆ yellow diamond", " to make it oval."]);
}

/// `compartmentInspector(store, cellId, compartmentId)`, the shared medium
/// included.
fn compartment_inspector(ui: &mut Ui, store: &Store, p: &Palette, cell_id: Option<&str>, compartment_id: &str, acts: &mut Vec<Act>) {
    let Some(map) = store.map.as_ref() else {
        inspector_empty(ui, p, false, "Compartment", "The selected compartment is no longer present.");
        return;
    };
    if cell_id.is_none() && compartment_id == map.environment().id {
        let env = map.environment();
        let label = if env.label.is_empty() { "Shared medium" } else { env.label.as_str() };
        inspector_header(ui, p, "Shared medium", label, "System boundary", acts);
        property_code(ui, "ID", &env.id);
        property(ui, "Role", "Shared extracellular medium");
        property(ui, "Bounds", format!("{} × {}", round_text(env.rect.w), round_text(env.rect.h)));
        property(ui, "Position", format!("x {}, y {}", round_text(env.rect.x), round_text(env.rect.y)));
        ui.add_space(6.0);
        widgets::section_label(ui, "MAP");
        if Btn::new("Edit Boundary").look(Look::Secondary).wide().show(ui).clicked() {
            acts.push(Act::EditBoundary(None));
        }
        return;
    }
    let cell = cell_id.and_then(|c| map.cell(c));
    let Some(comp) = cell.and_then(|c| c.compartments.iter().find(|x| x.id == compartment_id)) else {
        inspector_empty(ui, p, false, "Compartment", "The selected compartment is no longer present.");
        return;
    };
    let cell_name = cell.map(|c| c.name.clone()).filter(|n| !n.is_empty()).or_else(|| cell_id.map(str::to_string)).unwrap_or_default();
    let title = if comp.label.is_empty() { comp.id.clone() } else { comp.label.clone() };
    inspector_header(ui, p, "Compartment", &title, &cell_name, acts);
    property_code(ui, "ID", &comp.id);
    property(ui, "Cell", cell_name.clone());
    property(ui, "Parent", comp.parent_id.clone().unwrap_or_else(|| "Cell root".into()));
    property(ui, "Depth", js::number_to_string(comp.depth));
    property(ui, "Membrane", if comp.membrane.is_empty() { "—".to_string() } else { comp.membrane.clone() });
    property(ui, "Bounds", format!("{} × {}", round_text(comp.rect.w), round_text(comp.rect.h)));
    note_rich(ui, p, &["Drag the ", "centre", " to move it, a ", "corner", " or ", "side", " to resize it."]);
    ui.add_space(6.0);
    if let Some(cell) = cell_id {
        compartment_shape_fields(ui, p, cell, comp, acts);
        ui.add_space(6.0);
    }
    widgets::section_label(ui, "MAP");

    // `compartmentColorFields(store, cellId, comp)`.
    let cell_key = cell_id.unwrap_or_default().to_string();
    let theme = theme_colors(visual_or(&comp.visual_id, &comp.id), store.view.dark);
    let fill = picker_value(&[comp.fill.as_deref(), Some(theme.fill)]);
    let border = picker_value(&[comp.stroke.as_deref(), Some(theme.stroke)]);
    let ids = (cell_key.clone(), comp.id.clone());
    if let Some(color) = color_field(ui, p, "Fill color", Id::new(("compartment-fill-color", ids.clone())), &fill) {
        acts.push(Act::GeometryColor { cell: cell_key.clone(), compartment: Some(comp.id.clone()), fill: true, color });
    }
    if let Some(color) = color_field(ui, p, "Border color", Id::new(("compartment-border-color", ids)), &border) {
        acts.push(Act::GeometryColor { cell: cell_key.clone(), compartment: Some(comp.id.clone()), fill: false, color });
    }
    if (comp.fill.is_some() || comp.stroke.is_some()) && Btn::new("Use theme colors").look(Look::Secondary).wide().show(ui).clicked() {
        acts.push(Act::ResetColors { cell: cell_key.clone(), compartment: Some(comp.id.clone()) });
    }
    if Btn::new("Edit Boundary").look(Look::Secondary).wide().show(ui).clicked() {
        acts.push(Act::EditBoundary(None));
    }
    let is_cell = cell.and_then(coxeter_map::geometry_edit::cell_root_id) == Some(comp.id.as_str());
    if Btn::new(if is_cell { "Delete cell" } else { "Delete compartment" }).look(Look::DangerBox).wide().show(ui).clicked() {
        acts.push(Act::RemoveMapCompartment { cell: cell_key.clone(), compartment: Some(comp.id.clone()) });
    }
    widgets::note(ui, "Delete key also works; Ctrl+Z undoes.");
}

// Reactions

/// `inspectorCard(store)`: the selected reaction.
fn inspector_card(ui: &mut Ui, store: &Store, p: &Palette, focus_lower: bool, acts: &mut Vec<Act>) {
    let (Some(rid), Some(map)) = (store.selected_reaction.clone(), store.layout()) else {
        inspector_empty(ui, p, true, "No reaction selected", "Select a reaction on the map.");
        return;
    };
    let info = map.reaction(&rid);
    let community = store.community.as_ref();
    let merged = community.and_then(|c| c.merged.reactions.get(&rid));
    // The reaction of a species held out of the simulation: switched off
    // (kept dimmed on a hand-built map, and gone from the community) or, in a
    // joint community, at zero abundance (still in the problem, held at
    // zero). It has no flux or bounds to show; offering them would edit what
    // is not applied.
    let owner = community.and_then(|c| c.reaction_owner.get(&rid));
    let species_id = owner
        .and_then(|o| o.species_id.clone())
        .unwrap_or_else(|| rid.split('@').next().unwrap_or_default().to_string());
    let held_out = store.species_held_out(&species_id);
    let Some(merged) = merged.filter(|_| !held_out) else {
        let sp = store.species.iter().find(|s| s.id == species_id);
        let name = sp.and_then(|s| store.models.get(&s.file)).map_or_else(|| species_id.clone(), |m| m.cell.name.clone());
        let title = info.and_then(|r| r.local_id.clone()).unwrap_or_else(|| rid.clone());
        inspector_header(ui, p, "Reaction", &title, if sp.is_some() { &name } else { "" }, acts);
        match sp {
            Some(sp) if !sp.enabled => note_rich(ui, p, &["", &name, " is switched off, so this reaction is not simulated."]),
            Some(_) if held_out => note_rich(ui, p, &["", &name, " has zero abundance, so this reaction is held at zero."]),
            _ => widgets::note(ui, "Drawn, but not in the active model."),
        };
        return;
    };

    let owner_species = owner.and_then(|o| o.species_id.clone());
    let flux = store.solution.as_ref().and_then(|s| s.fluxes.get(&rid)).copied().unwrap_or(0.0);
    let bounds = store.current_reaction_bounds(&rid);
    // "Original" is as the scenario was opened, its medium included.
    let defaults = store.original_reaction_bounds(&rid);
    let knocked_out = store.reaction_knocked_out(&rid);
    let modified = (bounds.lower - defaults.lower).abs() > 1e-9 || (bounds.upper - defaults.upper).abs() > 1e-9;
    let fva = if store.fva_enabled { store.fva_ranges.get(&rid).copied() } else { None };
    let result = store.solution.as_ref().and_then(|s| s.result.as_ref());
    let reduced_cost = result.and_then(|r| r.reduced_costs.get(&rid)).copied();
    let scaled_reduced_cost = result.and_then(|r| r.scaled_reduced_costs.get(&rid)).copied();
    let objective_coefficient = store.objective_coefficients().get(&rid).copied().unwrap_or(0.0);

    // The FVA bar's scale.
    let span = js::max_n([defaults.lower.abs(), defaults.upper.abs(), 10.0]);
    let lo = if defaults.lower < 0.0 { -span } else { 0.0 };
    let hi = span;

    let local_model = owner_species.as_ref().and_then(|sid| store.species.iter().find(|s| &s.id == sid)).and_then(|s| store.models.get(&s.file));
    let cell_name = match &owner_species {
        Some(_) => local_model.map(|m| m.cell.name.clone()),
        None => Some("Shared medium".to_string()),
    };
    let compartments: Vec<String> = info.map(|r| r.compartments.clone()).unwrap_or_default();
    let local_reaction = owner.and_then(|o| local_model.and_then(|m| m.reactions.get(&o.original_id)));
    let rule = local_reaction.map_or(merged.gene_reaction_rule.as_str(), |r| r.gene_reaction_rule.as_str());
    let genes: Vec<(String, String)> = genes_from_rule(rule).into_iter().map(|g| (g.clone(), g)).collect();
    let metabolites: Vec<(String, String)> = local_reaction
        .map(|r| {
            r.metabolites
                .keys()
                .map(|mid| (mid.clone(), owner_species.as_ref().map_or_else(|| mid.clone(), |sp| format!("{sp}@{mid}"))))
                .collect()
        })
        .unwrap_or_default();
    let shown_id = owner.map_or_else(|| rid.clone(), |o| o.original_id.clone());

    inspector_header(ui, p, "Reaction", &shown_id, &merged.name, acts);
    let compartment_chips = |ui: &mut Ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(3.0, 3.0);
            for cid in &compartments {
                let c = theme_colors(cid, store.view.dark);
                widgets::chip(ui, cid, Some((c.fill, c.text, c.stroke)));
            }
        });
    };
    if !compartments.is_empty() {
        compartment_chips(ui);
    }
    property_code(ui, "ID", &shown_id);
    property(ui, "Name", merged.name.clone());
    widgets::property_row(ui, "Equation", true, |ui| {
        // `.equation-inline`.
        Frame::new()
            .fill(p.panel_2)
            .stroke(Stroke::new(1.0, p.line))
            .corner_radius(CornerRadius::same(7))
            .inner_margin(Margin::same(8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(Label::new(widgets::code(merged.equation()).size(10.5)).wrap());
            });
    });
    let minimised = if store.objective_sense() == coxeter_state::ObjectiveSense::Min { " (minimised)" } else { "" };
    property(ui, "Objective coefficient", format!("{}{minimised}", format_flux(Some(objective_coefficient), 4)));
    property(ui, "Reduced cost", reduced_cost.filter(|v| v.is_finite()).map_or_else(|| "—".to_string(), |v| format_flux(Some(v), 6)));
    property(ui, "Scaled reduced cost", scaled_reduced_cost.filter(|v| v.is_finite()).map_or_else(|| "—".to_string(), |v| format_flux(Some(v), 6)));
    property(ui, "GPR", if merged.gene_reaction_rule.is_empty() { "—".to_string() } else { merged.gene_reaction_rule.clone() });
    widgets::property_row(ui, "Genes", true, |ui| {
        let species = owner_species.clone().unwrap_or_default();
        if genes.is_empty() {
            widgets::value_text(ui, "—");
            return;
        }
        let index = store.gene_index();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(3.0, 3.0);
            for (gene, _) in &genes {
                if widgets::chip_button(ui, gene, false).clicked() {
                    acts.push(Act::SelectGene { gene: gene.clone(), species: species.clone() });
                }
                // The KO toggle beside each gene the community's rules know.
                let key = if species.is_empty() { gene.clone() } else { format!("{species}@{gene}") };
                if store.is_zuniga_study() || !index.genes.contains_key(&key) {
                    continue;
                }
                let on = store.gene_knockouts.contains(&key);
                let title = gene_effect_text(store, &key);
                let aria = format!("{} gene {gene}", if on { "Restore" } else { "Knock out" });
                let label = RichText::new("KO").size(10.0).family(bold()).color(if on { Color32::WHITE } else { p.ink_3 });
                let r = Btn::new(label).look(if on { Look::Pressed } else { Look::Unpressed }).title(&title).aria(&aria).height(20.0).show(ui);
                if on {
                    ui.painter().rect_filled(r.rect, CornerRadius::same(8), GENE_KNOCKED_OUT);
                    ui.painter().text(r.rect.center(), egui::Align2::CENTER_CENTER, "KO", egui::FontId::new(10.0, bold()), Color32::WHITE);
                }
                if r.clicked() {
                    acts.push(Act::GeneKo(key));
                }
                ui.add_space(4.0);
            }
        });
    });
    widgets::property_row(ui, "Metabolites", true, |ui| {
        chips(ui, &metabolites, |merged_id| acts.push(Act::SelectModelMetabolite(merged_id.to_string())));
    });
    widgets::property_row(ui, "Compartment", false, |ui| {
        if compartments.is_empty() {
            widgets::value_text(ui, "—");
        } else {
            compartment_chips(ui);
        }
    });
    property(ui, "Cell", cell_name.unwrap_or_else(|| "—".into()));
    property(ui, "Subsystem", info.and_then(|r| r.subsystem.clone()).unwrap_or_else(|| merged.subsystem.clone()));
    widgets::property_row(ui, "Annotations", true, |ui| {
        let entity = local_reaction.unwrap_or(merged);
        provenance_block(ui, p, &entity.annotation, &entity.notes, "reaction");
    });

    ui.add_space(8.0);
    let gene_closed_by = store.gene_closed_reactions().get(&rid).cloned().unwrap_or_default();
    let state = if !gene_closed_by.is_empty() {
        "Gene knockout"
    } else if knocked_out {
        "Knocked out"
    } else if modified {
        "Modified"
    } else {
        "Original"
    };
    widgets::readout(ui, &[("Flux", format_flux(Some(flux), 4)), ("State", state.to_string())]);
    omics_reaction_readout(ui, store, p, &rid);

    // The FVA range at the optimal objective, on the slider's scale.
    let fva_range = fva.filter(|f| store.fva_enabled && f.min.is_finite() && f.max.is_finite());
    match fva_range {
        Some(f) => {
            let range = js::max(hi - lo, 1e-9);
            let clamp = |v: f64| v.clamp(0.0, 100.0);
            let left = clamp(((f.min - lo) / range) * 100.0);
            let width = clamp(((f.max - f.min) / range) * 100.0);
            let mark = clamp(((flux - lo) / range) * 100.0);
            ui.add_space(4.0);
            widgets::range_bar(ui, (left / 100.0) as f32, (width / 100.0) as f32, (mark / 100.0) as f32);
            ui.columns(3, |cols| {
                cols[0].label(text(format_flux(Some(f.min), 3), 10.0, p.ink_3));
                cols[1].vertical_centered(|ui| ui.label(text("FVA at the optimal objective", 10.0, p.ink_3)));
                cols[2].with_layout(Layout::right_to_left(Align::Min), |ui| ui.label(text(format_flux(Some(f.max), 3), 10.0, p.ink_3)));
            });
        }
        None if store.fva_enabled => {
            widgets::hint(ui, "Computing the FVA range…");
        }
        None => {}
    }

    if map.has_reaction(&rid) {
        ui.add_space(6.0);
        label_control(ui, store, "reaction", Some(&rid), acts);
        let ids = vec![rid.clone()];
        if let Some(edit) = crate::panels::map_options::reaction_color_row(ui, store, &ids, &rid) {
            acts.push(Act::Colors { nodes: Vec::new(), reactions: ids, edit });
        }
    }
    widgets::section_title(ui, "Bounds");
    let zuniga = store.is_zuniga_study();
    if zuniga {
        widgets::hint(ui, "Source bounds are fixed for this case.");
    }
    // `<fieldset disabled>` around the bounds of the Zuñiga case.
    if !gene_closed_by.is_empty() {
        let names: Vec<String> = gene_closed_by.iter().map(|k| store.gene_title(k)).collect();
        let plural = if names.len() == 1 { "" } else { "s" };
        widgets::hint(ui, format!("Closed by the knockout of gene{plural} {}.", names.join(", ")));
    }
    ui.add_enabled_ui(!zuniga, |ui| {
        ui.add_enabled_ui(gene_closed_by.is_empty(), |ui| {
            bound_row(ui, p, &rid, &shown_id, true, bounds.lower, focus_lower && !zuniga, acts);
            bound_row(ui, p, &rid, &shown_id, false, bounds.upper, false, acts);
        });
        ui.add(egui::Label::new(text("Log sliders, 0 in the middle.", 10.5, p.ink_3)).wrap());
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if knocked_out {
                if Btn::new("Restore").title("Back to the bounds as opened").show(ui).clicked() {
                    acts.push(Act::ResetReaction(rid.clone()));
                }
            } else if Btn::new("Knockout").danger().show(ui).clicked() {
                acts.push(Act::Knockout(rid.clone()));
            }
            if Btn::new("Reset").show(ui).clicked() {
                acts.push(Act::ResetReaction(rid.clone()));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if Btn::new("All").title("Reset all modified reactions").show(ui).clicked() {
                    acts.push(Act::ResetAll);
                }
            });
        });
    });
    ui.add_space(6.0);
    if Btn::new("Show in Reactions ↗").wide().show(ui).clicked() {
        acts.push(Act::ShowInReactions(rid.clone()));
    }
    reaction_drawing_controls(ui, store, p, &rid, acts);
    ui.add_space(8.0);
    if Btn::new("Delete reaction from map").look(Look::DangerBox).wide().height(34.0).show(ui).clicked() {
        acts.push(Act::RemoveMapReaction(rid.clone()));
    }
    widgets::hint(ui, format!("Original bounds: [{}, {}]", format_flux(Some(defaults.lower), 3), format_flux(Some(defaults.upper), 3)));
}

/// What knocking out a gene does, or does now that it is out, in words.
fn gene_effect_text(store: &Store, key: &str) -> String {
    let on = store.gene_knockouts.contains(key);
    let closes: Vec<String> = store.gene_knockout_effect(key).iter().map(|rid| store.reaction_title(rid)).collect();
    if closes.is_empty() {
        return if on { "Knocked out; other genes keep its reactions open." } else { "Knocking it out closes nothing: other genes keep its reactions open." }
            .to_string();
    }
    let list = if closes.len() > 6 { format!("{} +{} more", closes[..6].join(", "), closes.len() - 6) } else { closes.join(", ") };
    if on { format!("Knocked out: closes {list}.") } else { format!("Knocking it out closes {list}.") }
}

/// The colour of a knocked-out gene's KO toggle (`.chip.gene-ko.is-on`).
const GENE_KNOCKED_OUT: Color32 = Color32::from_rgb(0xdb, 0x27, 0x77);

/// One `.bound-row`: the label and the shared bound control
/// (`boundControl`): the sign-log slider (`bound-lower` / `bound-upper`) and
/// the exact number box (`bound-lower-number` / `bound-upper-number`), which
/// applies on Enter or when it is left. A box holding no number is marked
/// `aria-invalid` while it is typed and, once left, shows the value in force
/// again and says so.
#[allow(clippy::too_many_arguments)]
fn bound_row(ui: &mut Ui, p: &Palette, rid: &str, label: &str, lower: bool, value: f64, focus: bool, acts: &mut Vec<Act>) {
    use crate::panels::reactions::{BoundEdit, bound_control};
    let name = if lower { "Lower" } else { "Upper" };
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(Vec2::new(38.0, 20.0), Layout::left_to_right(Align::Center), |ui| {
            ui.set_min_width(38.0);
            ui.label(text(name, 11.0, p.ink_3));
        });
        let id = Id::new(("bound-number", lower, rid.to_string()));
        match bound_control(ui, id, lower, value, true, label, 65.0, focus) {
            Some(BoundEdit::Slider(v)) => acts.push(Act::Bound { rid: rid.to_string(), lower, value: UserNumber::Number(v) }),
            Some(BoundEdit::Typed(typed)) => acts.push(Act::Bound { rid: rid.to_string(), lower, value: UserNumber::Text(typed) }),
            Some(BoundEdit::NotANumber) => acts.push(Act::NotANumber),
            None => {}
        }
    });
}

/// `omicsReactionReadout(store, reactionId)`.
fn omics_reaction_readout(ui: &mut Ui, store: &Store, p: &Palette, rid: &str) {
    if store.omics_dataset.is_none() || !store.omics_settings.enabled {
        return;
    }
    let overlay = store.omics_overlay();
    let a = overlay.scores_a.get(rid).copied().filter(|v| v.is_finite());
    let b = overlay.scores_b.get(rid).copied().filter(|v| v.is_finite());
    let value = overlay.values.get(rid).copied().filter(|v| v.is_finite());
    let condition_a = overlay.condition_a.clone().unwrap_or_default();
    ui.add_space(6.0);
    if overlay.mode == "compare" {
        let (Some(a), Some(b), Some(value)) = (a, b, value) else {
            widgets::hint(ui, "Omics: no value for both conditions.");
            return;
        };
        let metric = if overlay.metric == "log2fc" { "log₂FC" } else { "Δ" };
        widgets::boxed(ui, |ui| {
            omics_head(ui, p, metric);
            let condition_b = overlay.condition_b.clone().unwrap_or_default();
            widgets::readout(ui, &[(condition_a.as_str(), format_flux(Some(a), 4)), (condition_b.as_str(), format_flux(Some(b), 4))]);
            rich_line(ui, &[(&format!("{metric} = "), false), (&format_flux(Some(value), 4), true)], 12.0, p.ink_2);
        });
        return;
    }
    let Some(value) = value else {
        widgets::hint(ui, "Omics: no value for this reaction.");
        return;
    };
    widgets::boxed(ui, |ui| {
        omics_head(ui, p, &condition_a);
        rich_line(ui, &[("Reaction expression/abundance = ", false), (&format_flux(Some(value), 4), true)], 12.0, p.ink_2);
    });
}

/// `.omics-inspector-head`.
fn omics_head(ui: &mut Ui, p: &Palette, right: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Omics").size(11.0).family(bold()).color(p.ink));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| ui.label(text(right, 10.0, p.ink_3)));
    });
}

/// `reactionDrawingControls(store, rid)`: how one reaction is drawn
/// (straight or free, which metabolite stands where, and which way round it
/// is). Like the metabolite inspector's "Visual map instance", none of this
/// reaches the model.
fn reaction_drawing_controls(ui: &mut Ui, store: &Store, p: &Palette, rid: &str, acts: &mut Vec<Act>) {
    let Some(map) = store.map.as_ref() else { return };
    let Some(rxn) = map.reaction(rid) else { return };
    let (substrates, products) = map.reaction_sides(rid);
    let linear = rxn.linear.is_some();
    let can_linear = linear || map.can_linearize_reaction(rid);
    let ends: Vec<String> = rxn
        .linear
        .as_ref()
        .map(|l| [&l.from, &l.to].iter().filter_map(|id| id.as_ref().and_then(|id| map.node(id)).and_then(|n| n.base.clone())).filter(|b| !b.is_empty()).collect())
        .unwrap_or_default();
    let membrane = rxn.on_membrane;
    let line = if membrane { (None, None) } else { map.reaction_line_ends(rid) };
    // What a press on Linear or on the compass will use: two metabolites
    // picked out on the map win over the ones marked here.
    let picked = store.selected_line_ends(rid);
    let from_end = picked.0.or(line.0);
    let to_end = picked.1.or(line.1);
    let can_rotate = map.can_rotate_reaction(rid);

    widgets::section_title(ui, "Map drawing");
    controls_box(ui, p, |ui| {
        ui.horizontal(|ui| {
            role_label(ui, p, "Shape");
            widgets::grow_row(ui, 2, |ui, i| {
                if i == 0 {
                    let r = Btn::new("Free").active(!linear).show(ui);
                    aria_pressed(&r, !linear);
                    if r.clicked() {
                        acts.push(Act::ReactionLinear(rid.to_string(), false));
                    }
                } else {
                    let title = if membrane { "A transporter stays on its membrane" } else { "Straight line between the two marked metabolites" };
                    let r = Btn::new("Linear").active(linear).enabled(can_linear).title(title).show(ui);
                    aria_pressed(&r, linear);
                    if r.clicked() {
                        acts.push(Act::ReactionLinear(rid.to_string(), true));
                    }
                }
            });
        });
        if linear {
            widgets::note(ui, format!("Kept straight between {}.", ends.join(" and ")));
        }
        // Each metabolite is a button that runs the reaction's line through
        // it, and between each pair of neighbours a ⇄ exchanges their places:
        // enough to reach any order, and it says which two a press exchanges.
        for (label, entries, end) in [("Substrates", &substrates, &from_end), ("Products", &products, &to_end)] {
            if entries.is_empty() {
                continue;
            }
            role_label(ui, p, label);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                for (i, entry) in entries.iter().enumerate() {
                    let on = end.as_deref() == Some(entry.node_id.as_str());
                    let title = if membrane { entry.label.clone() } else { format!("Run the line through {}", entry.label) };
                    let r = line_end_chip(ui, p, &entry.label, on, !membrane, &title);
                    if r.clicked() {
                        acts.push(Act::ReactionLineEnd(rid.to_string(), entry.node_id.clone()));
                    }
                    let Some(next) = entries.get(i + 1) else { continue };
                    let allowed = entry.group == next.group;
                    let title = if allowed {
                        format!("Swap {} and {}", entry.label, next.label)
                    } else {
                        "In different compartments; cannot swap".to_string()
                    };
                    let aria = format!("Swap {} and {}", entry.label, next.label);
                    let r = Btn::new(RichText::new("⇄").size(12.0)).enabled(allowed).title(&title).aria(&aria).height(22.0).show(ui);
                    if r.clicked() {
                        acts.push(Act::ReactionSwap(rid.to_string(), entry.node_id.clone(), next.node_id.clone()));
                    }
                }
            });
        }
        // Substrate to product, as on a compass with north at the top of the map.
        ui.horizontal(|ui| {
            role_label(ui, p, "Align");
            ui.add_space(6.0);
            egui::Grid::new(("reaction-compass", rid.to_string())).spacing([3.0, 3.0]).show(ui, |ui| {
                let ways = [
                    ("NW", "up and to the left"),
                    ("N", "upwards"),
                    ("NE", "up and to the right"),
                    ("W", "to the left"),
                    ("", ""),
                    ("E", "to the right"),
                    ("SW", "down and to the left"),
                    ("S", "downwards"),
                    ("SE", "down and to the right"),
                ];
                for (i, (direction, way)) in ways.iter().enumerate() {
                    if direction.is_empty() {
                        ui.allocate_ui_with_layout(Vec2::new(30.0, 26.0), Layout::centered_and_justified(egui::Direction::TopDown), |ui| {
                            ui.label(text("●", 9.0, p.ink_3));
                        });
                    } else {
                        let arrow = glyph::COMPASS.iter().find(|(d, _)| d == direction).map_or("?", |(_, a)| *a);
                        let title = format!("Lay the reaction out {way}");
                        let aria = format!("Align {direction}");
                        let r = ui.add_enabled(
                            can_linear,
                            egui::Button::new(RichText::new(arrow).size(14.0).color(p.ink))
                                .fill(p.panel)
                                .stroke(Stroke::new(1.0, p.line))
                                .corner_radius(CornerRadius::same(7))
                                .min_size(Vec2::new(30.0, 26.0)),
                        );
                        aria_label(&r, &aria);
                        let r = if can_linear { r.on_hover_text(&title) } else { r.on_disabled_hover_text(&title) };
                        if r.clicked() {
                            let direction: &'static str = ["NW", "N", "NE", "W", "", "E", "SW", "S", "SE"][i];
                            acts.push(Act::ReactionAlign(rid.to_string(), direction));
                        }
                    }
                    if i % 3 == 2 {
                        ui.end_row();
                    }
                }
            });
        });
        widgets::grow_row(ui, 2, |ui, i| {
            let (label, axis, title) = if i == 0 {
                ("Vertical", "vertical", "Reaction and marked metabolites on one vertical line")
            } else {
                ("Horizontal", "horizontal", "Reaction and marked metabolites on one horizontal line")
            };
            if Btn::new(label).enabled(can_linear).title(title).show(ui).clicked() {
                acts.push(Act::ReactionAlignAxis(rid.to_string(), axis));
            }
        });
        let flip = Btn::new("Swap substrates ⇄ products")
            .enabled(!membrane)
            .title("Swap the places of substrates and products")
            .wide()
            .show(ui);
        if flip.clicked() {
            acts.push(Act::ReactionFlip(rid.to_string()));
        }
        widgets::grow_row(ui, 3, |ui, i| {
            let r = match i {
                0 => Btn::new("Mirror").title("Mirror the reaction across its own line").show(ui),
                1 => Btn::new("⟲ 90°").enabled(can_rotate).title("Rotate 90° anticlockwise").show(ui),
                _ => Btn::new("⟳ 90°").enabled(can_rotate).title("Rotate 90° clockwise").show(ui),
            };
            if r.clicked() {
                acts.push(match i {
                    0 => Act::ReactionMirror(rid.to_string()),
                    1 => Act::ReactionRotate(rid.to_string(), -90.0),
                    _ => Act::ReactionRotate(rid.to_string(), 90.0),
                });
            }
        });
        let tidy = Btn::new("Tidy curves").title("Reset cofactor places and curve shapes").wide().show(ui);
        if tidy.clicked() {
            acts.push(Act::ReactionTidy(rid.to_string()));
        }
        if reaction_label_offset(rxn).is_some() {
            let reset = Btn::new("Reset label").title("Put the name back beside the marker").wide().show(ui);
            if reset.clicked() {
                acts.push(Act::ReactionLabelReset(rid.to_string()));
            }
        }
    });
    widgets::note(ui, "Shift-click two metabolites to pick them.");
}

// Selection

/// `showsSelectionInspector(store)`: whether the inspector is about the
/// multi-selection: nothing is selected on its own (a selected reaction keeps
/// its inspector, which takes the line ends from shift-clicked metabolites),
/// and there is something to arrange, two objects or one reaction to lay on a
/// line.
pub fn shows_selection_inspector(store: &Store) -> bool {
    if store.selected_reaction.is_some() || store.selected_entity.is_some() || store.map.is_none() {
        return false;
    }
    store.multi_selection_size() >= 2 || store.can_arrange_selection("align-vertical")
}

/// `selectionCounts(selection)`: "3 reactions · 4 metabolites", kinds with
/// none left out.
fn selection_counts(selection: &Selection) -> String {
    [
        (selection.reactions.len(), "reaction"),
        (selection.nodes.len(), "metabolite"),
        (selection.texts.len(), "text"),
        (selection.compartments.len(), "compartment"),
    ]
    .into_iter()
    .filter(|(count, _)| *count > 0)
    .map(|(count, noun)| format!("{count} {noun}{}", if count == 1 { "" } else { "s" }))
    .collect::<Vec<_>>()
    .join(" · ")
}

/// `selectionInspector(store)`: arrange the multi-selection as a whole: align
/// and distribute it, space it evenly along its own line, and turn it,
/// rigidly, to a compass direction or by 45°. All map-only.
fn selection_inspector(ui: &mut Ui, store: &Store, p: &Palette, acts: &mut Vec<Act>) {
    let Some(map) = store.map.as_ref() else { return };
    let selection = &store.multi_selection;
    let spaced = spacing_items(map, selection).len();
    let has_line = selection_main_axis(map, selection).is_some();
    inspector_header_closing(ui, p, "Map", "Selection", &selection_counts(selection), Act::ClearMultiSelection, acts);

    widgets::section_title(ui, "Align & distribute");
    controls_box(ui, p, |ui| {
        if let Some(action) = crate::panels::map_options::arrange_selection_block(ui, store) {
            acts.push(Act::Arrange(action));
        }
    });

    widgets::section_title(ui, "Spacing");
    controls_box(ui, p, |ui| {
        let even = Btn::new("Even spacing").enabled(spaced >= 3).title("Equal steps along the selection's own line").wide().show(ui);
        if even.clicked() {
            acts.push(Act::SelectionSpaceEvenly);
        }
        let circle = store.can_circularize_selection();
        let title = if circle { "Lay the selected cycle out as a circle" } else { "The selected reactions do not close a cycle" };
        if Btn::new("Circularize").enabled(circle).title(title).wide().show(ui).clicked() {
            acts.push(Act::SelectionCircularize);
        }
    });

    widgets::section_title(ui, "Rotate");
    controls_box(ui, p, |ui| {
        // The way the selection runs, as on a compass with north at the top of the map.
        ui.horizontal(|ui| {
            role_label(ui, p, "Point");
            ui.add_space(6.0);
            egui::Grid::new("selection-compass").spacing([3.0, 3.0]).show(ui, |ui| {
                let ways = [
                    ("NW", "up-left"),
                    ("N", "up"),
                    ("NE", "up-right"),
                    ("W", "left"),
                    ("", ""),
                    ("E", "right"),
                    ("SW", "down-left"),
                    ("S", "down"),
                    ("SE", "down-right"),
                ];
                for (i, (direction, way)) in ways.iter().enumerate() {
                    if direction.is_empty() {
                        ui.allocate_ui_with_layout(Vec2::new(30.0, 26.0), Layout::centered_and_justified(egui::Direction::TopDown), |ui| {
                            ui.label(text("●", 9.0, p.ink_3));
                        });
                    } else {
                        let arrow = glyph::COMPASS.iter().find(|(d, _)| d == direction).map_or("?", |(_, a)| *a);
                        let title = format!("Point {way}");
                        let aria = format!("Point {direction}");
                        let r = ui.add_enabled(
                            has_line,
                            egui::Button::new(RichText::new(arrow).size(14.0).color(p.ink))
                                .fill(p.panel)
                                .stroke(Stroke::new(1.0, p.line))
                                .corner_radius(CornerRadius::same(7))
                                .min_size(Vec2::new(30.0, 26.0)),
                        );
                        aria_label(&r, &aria);
                        let r = if has_line { r.on_hover_text(&title) } else { r.on_disabled_hover_text(&title) };
                        if r.clicked() {
                            let direction: &'static str = ["NW", "N", "NE", "W", "", "E", "SW", "S", "SE"][i];
                            acts.push(Act::SelectionPoint(direction));
                        }
                    }
                    if i % 3 == 2 {
                        ui.end_row();
                    }
                }
            });
        });
        widgets::grow_row(ui, 2, |ui, i| {
            let (label, degrees, title) =
                if i == 0 { ("⟲ 45°", -45.0, "Rotate 45° anticlockwise") } else { ("⟳ 45°", 45.0, "Rotate 45° clockwise") };
            if Btn::new(label).title(title).wide().show(ui).clicked() {
                acts.push(Act::SelectionRotate(degrees));
            }
        });
    });
    widgets::note(ui, "Turns as one piece; cofactors follow.");

    widgets::section_title(ui, "Curves");
    controls_box(ui, p, |ui| {
        let tidy = Btn::new("Tidy curves")
            .enabled(!selection.reactions.is_empty())
            .title("Reset cofactor places and curve shapes of every selected reaction")
            .wide()
            .show(ui);
        if tidy.clicked() {
            acts.push(Act::SelectionTidy);
        }
    });
}

/// `.chip.reaction-line-end`: a metabolite the reaction's line may run
/// through, pressed when it does.
fn line_end_chip(ui: &mut Ui, p: &Palette, label: &str, on: bool, enabled: bool, title: &str) -> Response {
    let hovered = enabled && crate::widgets::hovered_next(ui);
    let (fill, ink, line) = match (on, hovered) {
        (true, _) => (p.accent_soft, p.accent, p.accent),
        (false, true) => (p.hover, p.ink, p.ink_3),
        (false, false) => (p.panel_2, p.ink_2, p.line),
    };
    let mut rich = RichText::new(label).size(11.0).color(ink);
    if on {
        rich = rich.family(bold());
    }
    let r = ui.add_enabled(
        enabled,
        egui::Button::new(rich).fill(fill).stroke(Stroke::new(if on { 1.5 } else { 1.0 }, line)).corner_radius(CornerRadius::same(99)).min_size(Vec2::new(0.0, 20.0)),
    );
    aria_pressed(&r, on);
    if enabled { r.on_hover_text(title) } else { r.on_disabled_hover_text(title) }
}

// Layout settings

/// One `.layout-control`: label, read-out and slider. While the slider is
/// dragged only the read-out follows it (`input`); letting go lays the map
/// out again (`change`).
#[allow(clippy::too_many_arguments)]
fn layout_slider(
    ui: &mut Ui,
    p: &Palette,
    label: &str,
    value: f64,
    (min, max, step): (f64, f64, f64),
    blocked: Option<&str>,
    format: fn(f64) -> String,
    patch: fn(f64) -> RawLayoutSettings,
    acts: &mut Vec<Act>,
) {
    let draft_id = Id::new(("layout-output", label.to_string()));
    let draft = ui.data(|d| d.get_temp::<f64>(draft_id));
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(11.5).family(bold()).color(p.ink));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(text(format(draft.unwrap_or(value)), 10.5, p.ink_3));
        });
    });
    ui.spacing_mut().slider_width = ui.available_width();
    let (response, event) = widgets::slider(ui, value, min, max, step, blocked.is_none(), label);
    if let Some(why) = blocked {
        let _ = response.clone().on_disabled_hover_text(why);
    }
    match slider_event(&response, event, value, min, max, step) {
        FieldEvent::None => {}
        FieldEvent::Input(v) => {
            ui.data_mut(|d| d.insert_temp(draft_id, v));
        }
        FieldEvent::Change(v) => {
            ui.data_mut(|d| d.remove::<f64>(draft_id));
            acts.push(Act::Layout(patch(v)));
        }
    }
    ui.add_space(10.0);
}

/// `.layout-settings-section`: a block with a rule under it.
fn layout_section<R>(ui: &mut Ui, p: &Palette, add: impl FnOnce(&mut Ui) -> R) -> R {
    let inner = Frame::new().inner_margin(Margin::symmetric(4, 13)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui)
    });
    let rect = inner.response.rect;
    ui.painter().hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, p.line));
    inner.inner
}

/// `renderLayoutSettings(store)`.
fn layout_settings(ui: &mut Ui, store: &Store, p: &Palette, acts: &mut Vec<Act>) {
    let settings = store.layout_settings.clone();
    // The spacing sliders re-run the same automatic layout as the presets, so
    // they are refused for the same reason, and say so where the pointer is.
    let blocked = store.relayout_blocked();
    let width = round_text(store.map.as_ref().map_or(1200.0, |m| m.width()));
    let height = round_text(store.map.as_ref().map_or(800.0, |m| m.height()));
    ui.spacing_mut().item_spacing.y = 5.0;

    layout_section(ui, p, |ui| {
        widgets::section_label(ui, "Automatic layout");
        ui.label(RichText::new("Layout settings").size(17.0).family(bold()).color(p.ink));
    });

    layout_section(ui, p, |ui| {
        ui.label(RichText::new("Preset").size(11.0).family(bold()).color(p.ink));
        ui.add_space(4.0);
        widgets::grow_row(ui, 3, |ui, i| {
            let (name, label, aria) = [
                ("compact", "Compact", "Compact layout preset"),
                ("normal", "Normal", "Normal layout preset"),
                ("spacious", "Spacious", "Spacious layout preset"),
            ][i];
            let mut b = Btn::new(label).pressed(settings.preset == name).enabled(blocked.is_none()).aria(aria).height(34.0);
            if let Some(why) = &blocked {
                b = b.title(why);
            }
            if b.show(ui).clicked() {
                acts.push(Act::LayoutPreset(name));
            }
        });
        if settings.preset == "custom" {
            widgets::note(ui, "Custom spacing");
        }
    });

    layout_section(ui, p, |ui| {
        let why = blocked.as_deref();
        layout_slider(
            ui,
            p,
            "Cell area",
            settings.cell_area_multiplier,
            (1.0, 8.0, 0.5),
            why,
            |v| format!("{}×", js::to_fixed(v, 1)),
            |v| RawLayoutSettings { cell_area_multiplier: Some(v), ..Default::default() },
            acts,
        );
        layout_slider(
            ui,
            p,
            "Cell spacing",
            settings.cell_spacing,
            (80.0, 420.0, 10.0),
            why,
            |v| format!("{} px", round_text(v)),
            |v| RawLayoutSettings { cell_spacing: Some(v), ..Default::default() },
            acts,
        );
        layout_slider(
            ui,
            p,
            "Minimum compartment size",
            settings.minimum_compartment_size,
            (28.0, 110.0, 2.0),
            why,
            |v| format!("{} px", round_text(v)),
            |v| RawLayoutSettings { minimum_compartment_size: Some(v), ..Default::default() },
            acts,
        );
        layout_slider(
            ui,
            p,
            "Node spacing",
            settings.node_spacing,
            (0.65, 1.90, 0.05),
            why,
            |v| format!("{}×", js::to_fixed(v, 2)),
            |v| RawLayoutSettings { node_spacing: Some(v), ..Default::default() },
            acts,
        );
        if let Some(why) = why {
            widgets::hint(ui, why);
        }
        widgets::hint(ui, "Undo restores the previous layout.");
    });

    layout_section(ui, p, |ui| {
        ui.label(RichText::new("Canvas size").size(11.0).family(bold()).color(p.ink));
        ui.add_space(4.0);
        widgets::grow_row(ui, 2, |ui, i| {
            let (label, value, aria, is_width) =
                if i == 0 { ("Width", &width, "Canvas width", true) } else { ("Height", &height, "Canvas height", false) };
            ui.vertical(|ui| {
                ui.label(text(label, 10.5, p.ink_3));
                ui.horizontal(|ui| {
                    let id = Id::new(("layout-canvas", is_width));
                    let w = (ui.available_width() - 24.0).max(40.0);
                    let (_, event) = widgets::number_field(ui, id, value, Field::new().width(w).aria(aria));
                    ui.label(text("px", 10.0, p.ink_3));
                    if let FieldEvent::Change(typed) = event {
                        if store.map.is_some() {
                            acts.push(Act::CanvasSize { width: is_width, value: js::string_to_number(&typed) });
                        }
                    }
                });
            });
        });
        ui.add_space(6.0);
        if Btn::new("⤢ Fit to screen").wide().height(34.0).show(ui).clicked() {
            acts.push(Act::ZoomFit);
        }
    });
}
