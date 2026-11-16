//! The Edit compartments side window (port of `src/ui/nesting-window.js`).
//!
//! It opens with the Edit Compartments tool, over the map's left edge right
//! beside the tool column, and shows one drawn cell's compartments as a tree:
//! every compartment of the species' model, each under the one it is drawn
//! inside, with a small drawing of the nested boxes underneath. The nesting is
//! changed by dragging a row onto another, by Tab and Shift+Tab, or by
//! starting over from a preset. Every change goes through the store's MAP
//! operations, so it is one undo step and is saved with the map; the model is
//! never touched.
//!
//! The JavaScript renders HTML into a layer of its own and reads the clicks
//! back from `data-` attributes; here [`show`] draws the window as an egui
//! `Area` over the stage and carries out what was clicked once it is drawn.

use coxeter_canvas::Tool;
use coxeter_core::compartments::{NESTING_PRESETS, Names, compartment_colors};
use coxeter_state::Store;
use coxeter_state::types::{NestingCell, NestingRow, NestingView};
use egui::{
    Align2, Color32, CornerRadius, CursorIcon, EventFilter, FontFamily, FontId, Frame, Key, Margin, Modifiers, Pos2, Rect, RichText,
    Sense, Shape, Stroke, StrokeKind, Ui, Vec2,
};

use super::add_reaction::{PanelDrag, PanelPosition, menu_area, menu_head, panel_placement_size, update_panel_drag};
use crate::app::Shell;
use crate::theme::{Palette, css_color};
use crate::widgets::{self, Btn};

/// Width of the window in screen pixels: room for a compartment name, its id
/// and a badge with four levels of indent in front.
const NESTING_WINDOW_WIDTH: f64 = 320.0;

/// The tallest the window grows before its contents scroll.
const NESTING_WINDOW_HEIGHT: f64 = 640.0;

/// How far each nesting level is indented in the tree, in pixels.
const NESTING_INDENT: f32 = 16.0;

/// The share of a row's height, at its top and at its bottom edge, where a
/// drop puts the dragged compartment beside that row instead of inside it.
const DROP_EDGE: f32 = 0.25;

/// Height of one tree row: `.nesting-row` is 12px text in 4px of padding,
/// with room for the badge.
const ROW_HEIGHT: f32 = 24.0;

/// How the presets are named in the Start from list.
fn preset_label(preset: &str) -> &'static str {
    match preset {
        "auto" => "Automatic",
        "yeast9" => "Yeast9",
        "bigg" => "BiGG",
        "empty" => "Empty",
        _ => "",
    }
}

/// The window while it is open (`this.state` of the controller).
#[derive(Debug, Clone, Default)]
pub struct WindowState {
    /// Where it stands on the stage. It starts at the top left of the map,
    /// next to the tools it belongs to.
    pub position: PanelPosition,
    pub species_id: Option<String>,
    pub selected_id: Option<String>,
    /// The compartment whose row is being dragged.
    pub drag_id: Option<String>,
}

/// `NestingWindowController`: whether the window is open, and on what.
#[derive(Debug, Default)]
pub struct NestingWindow {
    pub state: Option<WindowState>,
    /// The drag of the window by its header (`panelDrag`).
    pub drag: Option<PanelDrag>,
    /// Closed by hand while the tool is still on: it stays closed until the
    /// Nesting button opens it again or the tool is picked anew.
    pub dismissed: bool,
    /// The selected row takes the keyboard on the next frame
    /// (`focusSelectedRow`).
    focus_row: bool,
}

impl NestingWindow {
    pub fn is_open(&self) -> bool {
        self.state.is_some()
    }

    /// `follow(toolActive, speciesId)`: keep the window in step with the
    /// active tool: open while Edit Compartments is the tool, unless closed
    /// by hand.
    pub fn follow(&mut self, store: &Store, tool_active: bool, species_id: Option<&str>) {
        if !tool_active {
            self.close();
            self.dismissed = false;
            return;
        }
        if self.state.is_none() && !self.dismissed {
            self.open(store, species_id);
        }
    }

    /// `open(speciesId)`: open the window, on the given cell if it is drawn.
    pub fn open(&mut self, store: &Store, species_id: Option<&str>) {
        self.dismissed = false;
        let cells = store.nesting_cells();
        let state = self.state.get_or_insert_with(WindowState::default);
        if let Some(chosen) = cells.iter().find(|cell| Some(cell.species_id.as_str()) == species_id) {
            state.species_id = Some(chosen.species_id.clone());
            state.selected_id = None;
        }
    }

    pub fn close(&mut self) {
        self.state = None;
        self.drag = None;
    }
}

/// What a click, a key or a drop in the window asks for, done once the
/// window is drawn (`handleClick`, `handleChange`, `handleKey`, `handleDrop`).
enum Action {
    Close,
    Cell(String),
    Preset(String),
    Select(String),
    Nest(&'static str),
    Drop { id: String, parent_id: String },
}

/// Draw the window over the map while it is open (`render()`), and do what it
/// was used for.
pub fn show(ctx: &egui::Context, shell: &mut Shell) {
    let tool_active = shell.map.tool() == Tool::Compartment;
    let selected_cell = shell.store.selected_geometry.as_ref().and_then(|g| g.cell_id.clone());
    shell.nesting.follow(&shell.store, tool_active, selected_cell.as_deref());
    // The window lives on the stage: under a workspace page there is none.
    if shell.ui.workspace.is_some() || shell.ui.results_maximized {
        return;
    }
    let Some(stage) = shell.map_rect.filter(|r| r.width() > 0.0 && r.height() > 0.0) else { return };
    let Some(mut state) = shell.nesting.state.take() else { return };

    // The cell and the row shown are kept to what the store now holds.
    let cells = shell.store.nesting_cells();
    if !cells.iter().any(|cell| Some(&cell.species_id) == state.species_id.as_ref()) {
        state.species_id = cells.first().map(|cell| cell.species_id.clone());
        state.selected_id = None;
    }
    let view = state.species_id.as_deref().and_then(|id| shell.store.compartment_nesting(id));
    if let Some(view) = &view {
        if !view.rows.iter().any(|row| Some(&row.id) == state.selected_id.as_ref()) {
            let first = view.rows.iter().find(|row| !row.outer).or(view.rows.first());
            state.selected_id = first.map(|row| row.id.clone());
        }
    }

    let id = egui::Id::new("nesting-window");
    let place = panel_placement_size(
        stage.width() as f64, stage.height() as f64, Some(&state.position), NESTING_WINDOW_WIDTH, NESTING_WINDOW_HEIGHT,
    );
    let at = stage.min + Vec2::new(place.left as f32, place.top as f32);
    let mut subtitle = "";
    if cells.len() == 1 {
        subtitle = cells[0].name.as_str();
    }
    let dark = shell.store.view.dark;
    let focus_row = std::mem::take(&mut shell.nesting.focus_row);
    let mut action = None;
    let mut head = None;
    let mut close_rect = Rect::NOTHING;

    menu_area(ctx, id, stage, at, place.width as f32, place.max_height as f32, |ui| {
        let top = ui.cursor().top();
        let (response, closed, close) = menu_head(ui, "Edit compartments", subtitle, "Close", Some("Close"));
        head = Some(response);
        close_rect = close;
        if closed {
            action = Some(Action::Close);
        }
        let Some(view) = &view else {
            Frame::new().inner_margin(Margin::same(12)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                widgets::hint(ui, "Draw a cell first.");
            });
            return;
        };
        let rest = (place.max_height as f32 - (ui.cursor().top() - top) - 2.0).max(60.0);
        // An area offers its contents only last frame's size, so the body is
        // given the height its contents took last frame, up to the room the
        // window has; beyond that it scrolls.
        let height_id = id.with("body-height");
        let last: f32 = ui.data(|d| d.get_temp(height_id)).unwrap_or(rest);
        let height = last.min(rest);
        let scroll = egui::ScrollArea::vertical().id_salt(id.with("body")).max_height(height).min_scrolled_height(height);
        let out = scroll.auto_shrink([false, true]).show(ui, |ui| {
            Frame::new().inner_margin(Margin { left: 11, right: 11, top: 10, bottom: 12 }).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
                if let Some(picked) = body(ui, &cells, view, &mut state, dark, focus_row) {
                    action = Some(picked);
                }
            });
        });
        ui.data_mut(|d| d.insert_temp(height_id, out.content_size.y));
    });

    if let Some(head) = head {
        update_panel_drag(ctx, id, stage, head, close_rect, &mut state.position, &mut shell.nesting.drag);
    }
    shell.nesting.state = Some(state);
    if let Some(action) = action {
        apply(shell, action);
    }
}

/// Carry out what the window was used for.
fn apply(shell: &mut Shell, action: Action) {
    let species_id = shell.nesting.state.as_ref().and_then(|s| s.species_id.clone()).unwrap_or_default();
    let selected_id = shell.nesting.state.as_ref().and_then(|s| s.selected_id.clone());
    match action {
        Action::Close => {
            shell.nesting.close();
            shell.nesting.dismissed = true;
        }
        Action::Cell(cell_id) => {
            if let Some(state) = shell.nesting.state.as_mut() {
                state.species_id = Some(cell_id);
                state.selected_id = None;
            }
        }
        Action::Preset(preset) => {
            shell.store.apply_nesting_preset(&species_id, &preset);
        }
        Action::Select(row_id) => {
            if let Some(state) = shell.nesting.state.as_mut() {
                state.selected_id = Some(row_id);
            }
            shell.nesting.focus_row = true;
        }
        Action::Nest(direction) => {
            if let Some(row_id) = selected_id {
                shell.store.nest_compartment(&species_id, &row_id, direction);
            }
            shell.nesting.focus_row = true;
        }
        Action::Drop { id, parent_id } => {
            shell.store.set_compartment_parent(&species_id, &id, &parent_id);
        }
    }
}

/// The window's body: the cell, the preset, the tree, Out and In, the preview.
fn body(ui: &mut Ui, cells: &[NestingCell], view: &NestingView, state: &mut WindowState, dark: bool, focus_row: bool) -> Option<Action> {
    let mut action = None;

    // Which cell, when there is more than one
    if cells.len() >= 2 {
        let options: Vec<(String, String)> = cells.iter().map(|cell| (cell.species_id.clone(), cell.name.clone())).collect();
        let current = state.species_id.clone().unwrap_or_default();
        if let Some(picked) = field(ui, "Cell", |ui, width| widgets::select(ui, "nesting-cell", &current, &options, width, true, "Cell")) {
            action = Some(Action::Cell(picked));
        }
    }

    // A nesting set by hand matches none of the presets, and picking one of
    // them replaces it, so it is listed as what it is.
    let mut options: Vec<(String, String)> = Vec::new();
    if view.custom {
        options.push((String::new(), "Edited".to_string()));
    }
    for preset in NESTING_PRESETS {
        options.push((preset.to_string(), preset_label(preset).to_string()));
    }
    let current = if view.custom { String::new() } else { "auto".to_string() };
    let picked = field(ui, "Start from", |ui, width| widgets::select(ui, "nesting-preset", &current, &options, width, true, "Start from"));
    if let Some(preset) = picked.filter(|p| !p.is_empty()) {
        action = Some(Action::Preset(preset));
    }

    if let Some(picked) = tree(ui, view, state, dark, focus_row) {
        action = Some(picked);
    }

    ui.columns(2, |columns| {
        if Btn::new("⇤ Out").title("Move out (Shift+Tab)").wide().show(&mut columns[0]).clicked() {
            action = Some(Action::Nest("out"));
        }
        if Btn::new("In ⇥").title("Nest under the row above (Tab)").wide().show(&mut columns[1]).clicked() {
            action = Some(Action::Nest("in"));
        }
    });

    preview(ui, view, dark);
    action
}

/// `.nesting-field`: a label and the select beside it.
fn field<T>(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui, f32) -> T) -> T {
    let p = Palette::current(ui.ctx());
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(12.0).color(p.ink_2));
        let width = (ui.available_width() - 8.0).max(60.0);
        add(ui, width)
    })
    .inner
}

/// `renderTree(view)`: one row per compartment, indented by depth, with its
/// colour, name, id and whether it is drawn. A row is picked by a click, and
/// moved by dragging it onto another or with the keyboard while it is focused.
fn tree(ui: &mut Ui, view: &NestingView, state: &mut WindowState, dark: bool, focus_row: bool) -> Option<Action> {
    let p = Palette::current(ui.ctx());
    let mut action = None;
    let frame = Frame::new().stroke(Stroke::new(1.0, p.line)).corner_radius(CornerRadius::same(8)).inner_margin(Margin::same(3));
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        let mut placed: Vec<(usize, Rect)> = Vec::new();
        for (index, row) in view.rows.iter().enumerate() {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_HEIGHT), Sense::hover());
            let row_id = egui::Id::new(("nesting-row", &view.species_id, &row.id));
            let sense = if row.outer { Sense::click() } else { Sense::click_and_drag() };
            let response = ui.interact(rect, row_id, sense);
            placed.push((index, rect));
            let selected = state.selected_id.as_deref() == Some(row.id.as_str());
            // `role="treeitem"` with `aria-selected`.
            response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, &row.name));
            paint_row(ui, rect, row, selected, response.hovered(), response.has_focus(), dark, &p);

            if response.clicked() {
                response.request_focus();
                action = Some(Action::Select(row.id.clone()));
            }
            if response.drag_started() && !row.outer {
                state.drag_id = Some(row.id.clone());
                state.selected_id = Some(row.id.clone());
            }
            if !row.outer {
                response.clone().on_hover_cursor(CursorIcon::Grab);
            }
            if selected && focus_row {
                response.request_focus();
            }

            // Keys pressed on the row stay in the window: Delete would
            // otherwise remove the selected compartment from the map, and
            // Escape would leave the tool.
            if response.has_focus() {
                let filter = EventFilter { tab: true, vertical_arrows: true, escape: true, horizontal_arrows: false };
                ui.memory_mut(|m| m.set_focus_lock_filter(row_id, filter));
                if let Some(key_action) = row_keys(ui, view, &row.id) {
                    action = Some(key_action);
                }
            }
        }

        // A row being dragged shows where it would go, and goes there when
        // it is let go.
        if let Some(drag_id) = state.drag_id.clone() {
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            let pointer = ui.ctx().pointer_latest_pos();
            let mut drop = None;
            if let Some(pos) = pointer {
                if let Some((index, rect)) = placed.iter().find(|(_, rect)| rect.contains(pos)) {
                    let share = (pos.y - rect.top()) / rect.height().max(1.0);
                    if let Some(target) = drop_target(view, &drag_id, &view.rows[*index].id, share) {
                        drop = Some((*rect, target));
                    }
                }
            }
            if let Some((rect, (_, zone))) = &drop {
                let line = Stroke::new(2.0, p.accent);
                match *zone {
                    "before" => {
                        ui.painter().hline(rect.x_range(), rect.top() + 1.0, line);
                    }
                    "after" => {
                        ui.painter().hline(rect.x_range(), rect.bottom() - 1.0, line);
                    }
                    _ => {
                        ui.painter().rect_stroke(*rect, CornerRadius::same(6), line, StrokeKind::Inside);
                    }
                }
            }
            let released = ui.input(|i| i.pointer.any_released() || !i.pointer.any_down());
            if released {
                state.drag_id = None;
                if let Some((_, (parent_id, _))) = drop {
                    action = Some(Action::Drop { id: drag_id, parent_id });
                }
            }
        }
    });
    action
}

/// One `.nesting-row`: the highlight, then swatch, name, id and badge.
#[allow(clippy::too_many_arguments)]
fn paint_row(ui: &Ui, rect: Rect, row: &NestingRow, selected: bool, hovered: bool, focused: bool, dark: bool, p: &Palette) {
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, CornerRadius::same(6), p.accent_soft);
    } else if hovered {
        painter.rect_filled(rect, CornerRadius::same(6), p.hover);
    }
    if focused {
        painter.rect_stroke(rect, CornerRadius::same(6), Stroke::new(2.0, p.accent_line), StrokeKind::Inside);
    }

    // Swatch
    let colors = compartment_colors(&row.visual_id, dark, &Names::new());
    let mut x = rect.left() + 8.0 + row.depth as f32 * NESTING_INDENT;
    let swatch = Rect::from_min_size(Pos2::new(x, rect.center().y - 5.5), Vec2::splat(11.0));
    painter.rect(
        swatch,
        CornerRadius::same(3),
        css_color(colors.fill, p.panel_2),
        Stroke::new(1.5, css_color(colors.stroke, p.line)),
        StrokeKind::Inside,
    );
    x += 11.0 + 7.0;

    // Badge, against the right edge
    let (badge, badge_ink, badge_bg) = if row.outer {
        ("Outermost", p.ink_3, p.hover)
    } else if row.parent_id.is_none() {
        ("Outside cell", p.warn, p.danger_bg)
    } else if row.drawn {
        ("On map", p.ok, p.success_bg)
    } else {
        ("Not drawn", p.ink_3, p.hover)
    };
    let badge_galley = painter.layout_no_wrap(badge.to_string(), FontId::proportional(10.5), badge_ink);
    let badge_rect = Rect::from_min_size(
        Pos2::new(rect.right() - 8.0 - badge_galley.size().x - 12.0, rect.center().y - badge_galley.size().y / 2.0 - 1.0),
        Vec2::new(badge_galley.size().x + 12.0, badge_galley.size().y + 2.0),
    );
    painter.rect_filled(badge_rect, CornerRadius::same(6), badge_bg);
    painter.galley(badge_rect.min + Vec2::new(6.0, 1.0), badge_galley, badge_ink);

    // Name and id, the name cut short where they would run into the badge
    let ink = if selected { p.accent } else { p.ink };
    let id_galley = painter.layout_no_wrap(row.id.clone(), FontId::new(10.5, FontFamily::Monospace), p.ink_3);
    let name_room = (badge_rect.left() - 7.0 - x - id_galley.size().x - 7.0).max(12.0);
    let mut job = egui::text::LayoutJob::simple_singleline(row.name.clone(), FontId::proportional(12.0), ink);
    job.wrap = egui::text::TextWrapping::truncate_at_width(name_room);
    let name_galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
    let name_width = name_galley.size().x;
    painter.galley(Pos2::new(x, rect.center().y - name_galley.size().y / 2.0), name_galley, ink);
    painter.galley(Pos2::new(x + name_width + 7.0, rect.center().y - id_galley.size().y / 2.0), id_galley, p.ink_3);
}

/// `handleKey`: what a key pressed on the focused row `row_id` does. The
/// keys are taken from the input, so neither the map nor the app's own
/// shortcuts see them.
fn row_keys(ui: &mut Ui, view: &NestingView, row_id: &str) -> Option<Action> {
    let mut action = None;
    ui.input_mut(|i| {
        if i.consume_key(Modifiers::NONE, Key::Escape) {
            action = Some(Action::Close);
        }
        i.consume_key(Modifiers::NONE, Key::Delete);
        i.consume_key(Modifiers::NONE, Key::Backspace);
        // Shift+Tab first: a plain Tab pattern also matches with Shift held.
        if i.consume_key(Modifiers::SHIFT, Key::Tab) {
            action = Some(Action::Nest("out"));
        } else if i.consume_key(Modifiers::NONE, Key::Tab) {
            action = Some(Action::Nest("in"));
        }
        let mut step: isize = 0;
        if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
            step = 1;
        }
        if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
            step = -1;
        }
        if step != 0 {
            let index = view.rows.iter().position(|row| row.id == row_id).unwrap_or(0) as isize;
            let next = index + step;
            if next >= 0 && (next as usize) < view.rows.len() {
                action = Some(Action::Select(view.rows[next as usize].id.clone()));
            }
        }
    });
    action
}

/// `dropTarget(event)`: where a drop on row `target`, at `share` of its
/// height from the top, puts the dragged compartment: its new parent and the
/// zone (`inside`, `before` or `after`). `None` when it cannot go there: on
/// itself, or inside something nested in it.
fn drop_target(view: &NestingView, drag_id: &str, target: &str, share: f32) -> Option<(String, &'static str)> {
    let parent_of = |id: &str| view.rows.iter().find(|row| row.id == id).and_then(|row| row.parent_id.clone());
    let mut at = Some(target.to_string());
    while let Some(id) = at {
        if id == drag_id {
            return None;
        }
        at = parent_of(&id);
    }
    let mut zone = "inside";
    if target != view.outer_id && share < DROP_EDGE {
        zone = "before";
    } else if target != view.outer_id && share > 1.0 - DROP_EDGE {
        zone = "after";
    }
    let parent_id = if zone == "inside" { target.to_string() } else { parent_of(target).unwrap_or_else(|| view.outer_id.clone()) };
    Some((parent_id, zone))
}

/// `renderPreview(view)`: the cell drawn small, each compartment a box inside
/// the box of its parent.
fn preview(ui: &mut Ui, view: &NestingView, dark: bool) {
    let p = Palette::current(ui.ctx());
    let response = Frame::new()
        .inner_margin(Margin::same(8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::splat(6.0);
                for row in view.rows.iter().filter(|row| row.parent_id.is_none()) {
                    preview_box(ui, view, row, dark);
                }
            });
        })
        .response;
    dashed_outline(ui, response.rect, Stroke::new(1.0, p.line));
}

/// One `.nesting-box`, with the boxes of the compartments inside it. A
/// compartment the tree leaves outside the cell has a dashed edge.
fn preview_box(ui: &mut Ui, view: &NestingView, row: &NestingRow, dark: bool) {
    let p = Palette::current(ui.ctx());
    let colors = compartment_colors(&row.visual_id, dark, &Names::new());
    let fill = css_color(colors.fill, Color32::TRANSPARENT);
    let edge = Stroke::new(1.5, css_color(colors.stroke, p.line));
    let stray = row.parent_id.is_none() && !row.outer;
    let border = if stray { Stroke::NONE } else { edge };
    let response = Frame::new()
        .fill(fill)
        .stroke(border)
        .corner_radius(CornerRadius::same(9))
        .inner_margin(Margin { left: 5, right: 5, top: 16, bottom: 5 })
        .show(ui, |ui| {
            ui.set_min_size(Vec2::new(30.0, 5.0));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::splat(6.0);
                for child in view.rows.iter().filter(|child| child.parent_id.as_deref() == Some(row.id.as_str())) {
                    preview_box(ui, view, child, dark);
                }
            });
        })
        .response;
    let label = Pos2::new(response.rect.left() + 6.0, response.rect.top() + 1.0);
    ui.painter().text(label, Align2::LEFT_TOP, &row.id, FontId::proportional(10.5), css_color(colors.text, p.ink));
    if stray {
        dashed_outline(ui, response.rect, edge);
    }
}

/// A dashed rectangle (`border-style: dashed`).
fn dashed_outline(ui: &Ui, rect: Rect, stroke: Stroke) {
    let corners = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom(), rect.left_top()];
    ui.painter().extend(Shape::dashed_line(&corners, stroke, 4.0, 3.0));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::ScriptedDialogs;
    use coxeter_core::mapdata::Rect as MapRect;
    use egui_kittest::kittest::Queryable;
    use indexmap::IndexMap;

    #[test]
    fn the_window_opens_with_the_tool_beside_the_tool_column_and_tab_nests_the_selected_compartment() {
        let mut store = Store::with_dir(coxeter_core::testkit::repo_root());
        store.init().expect("bundled models");
        store.use_scenario("solo-levadura").expect("scenario");
        store.show_empty_map();
        store.add_map_cell("sce", Some(MapRect::new(40.0, 40.0, 1180.0, 800.0))).expect("the cell is drawn");
        store.apply_nesting_preset("sce", "empty");
        let names: IndexMap<String, String> =
            store.compartment_nesting("sce").expect("view").rows.into_iter().map(|row| (row.id, row.name)).collect();
        let mut shell = Shell::new(store, Box::new(ScriptedDialogs::new()));
        shell.activate_tool(Tool::Compartment);
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1400.0, 900.0))
            .wgpu()
            .build_eframe(move |_cc| shell);
        harness.run_steps(6);
        assert!(harness.state().nesting.is_open(), "the window opens with the tool");
        let stage = harness.state().map_rect.expect("the map is drawn");
        let window = harness.get_by_label("Edit compartments").rect();
        // The title stands 8px of margin and 12px of header padding in.
        assert!(window.left() - stage.left() <= 24.0, "it stands at the left of the map, next to the tools");

        harness.get_by_label(&names["im"]).click();
        harness.run_steps(2);
        harness.key_press(egui::Key::Tab);
        harness.run_steps(4);
        let parents = |harness: &egui_kittest::Harness<'_, Shell>| -> IndexMap<String, Option<String>> {
            let view = harness.state().store.compartment_nesting("sce").expect("view");
            view.rows.into_iter().map(|row| (row.id, row.parent_id)).collect()
        };
        assert_eq!(parents(&harness)["im"].as_deref(), Some("c"), "Tab nests it under the row above");
        harness.key_press_modifiers(Modifiers::SHIFT, egui::Key::Tab);
        harness.run_steps(4);
        assert_eq!(parents(&harness)["im"].as_deref(), Some("e"), "Shift+Tab moves it back out");
        assert!(harness.state().nesting.is_open(), "the keys stay in the window");
    }
}
