//! The Reactions page and its compact twin in the results dock (port of
//! `src/ui/reactions-panel.js`), with the handlers of `onSidebarClick` /
//! `onSidebarInput` for their controls (`rx-*`, `bound-*`).
//!
//! The table lays out only the rows in view (`ScrollArea::show_rows`): a
//! genome-scale community has eleven thousand of them. The tick box and the
//! name stay at the left and the actions at the right while the columns
//! between scroll sideways.

use coxeter_core::js;
use coxeter_render::format_flux;
use coxeter_state::reactions::{BulkAction, ReactionTableModel, ReactionTablePatch, TableColumns};
use coxeter_state::reactions_explorer::{
    GroupItem, Grouping, ReactionFilter, ReactionRow, ReactionType, RowStatus, SIGN_LOG, SortKey, TableItem, round_precision,
    sign_log_position, sign_log_steps, sign_log_value,
};
use coxeter_state::store::objectives;
use coxeter_state::{ObjectiveSense, Store, UserNumber};
use egui::{Align, Color32, CornerRadius, Frame, Id, Layout, Margin, Rect, RichText, Sense, Stroke, Ui, UiBuilder, pos2, vec2};

use crate::app::{Deferred, Shell};
use crate::theme::{Palette, bold};
use crate::ui_state::CompactMode;
use crate::widgets::{self, Btn, Field, FieldEvent, Look, aria_label, aria_pressed};

/// Row heights, fixed so the rows can be laid out by index.
pub const PAGE_ROW: f32 = 34.0;
pub const COMPACT_ROW: f32 = 26.0;
const HEAD: f32 = 26.0;
const SCROLLBAR: f32 = 10.0;

/// `acceptNumber`'s warning (the inspector's, shared).
pub use crate::panels::inspector::NOT_A_NUMBER;

// Shared bound control

/// `formatBound(value)`: a bound as the number box shows it, exact, without
/// float noise.
pub fn format_bound(value: f64) -> String {
    if !value.is_finite() {
        return String::new();
    }
    js::number_to_string(round_precision(value, 10))
}

/// `formatBoundText(value)`: a bound as text in a table cell.
pub fn format_bound_text(value: f64) -> String {
    if !value.is_finite() {
        return "—".into();
    }
    if value.abs() >= 1e5 { coxeter_state::util::to_exponential(value, 0) } else { format_bound(value) }
}

/// What a bound control did.
#[derive(Debug, Clone, PartialEq)]
pub enum BoundEdit {
    /// The slider moved: a value to apply now and solve on the next frame.
    Slider(f64),
    /// The box was committed (Enter or leaving it) with a number.
    Typed(String),
    /// The box was committed holding no number: it shows the value in force again.
    NotANumber,
}

/// `boundControl({ rid, side, value, disabled, label })`: one bound as a
/// sign-log slider and an exact number box (see `SIGN_LOG`). The inspector
/// and the Reactions page share it, so both edit bounds the same way. The
/// box applies on Enter or when it is left; while typing it is only checked.
#[allow(clippy::too_many_arguments)]
pub fn bound_control(ui: &mut Ui, id: Id, lower: bool, value: f64, enabled: bool, label: &str, box_width: f32, focus: bool) -> Option<BoundEdit> {
    let steps = sign_log_steps(&SIGN_LOG) as f64;
    let side = if lower { "Lower" } else { "Upper" };
    let name = if label.is_empty() { format!("{side} bound") } else { format!("{side} bound of {label}") };
    let mut edit = None;
    ui.spacing_mut().slider_width = (ui.available_width() - box_width - ui.spacing().item_spacing.x).max(30.0);
    let position = sign_log_position(value, &SIGN_LOG) as f64;
    let (response, event) = widgets::slider(ui, position, -steps, steps, 1.0, enabled, &format!("{name}, logarithmic slider"));
    let _ = response.on_hover_text(format!("{} (log scale; the middle is 0)", format_bound(value)));
    if let Some(p) = event.value() {
        let v = sign_log_value(p, &SIGN_LOG);
        if v != value {
            edit = Some(BoundEdit::Slider(v));
        }
    }
    let invalid_id = id.with("aria-invalid");
    let invalid = ui.data(|d| d.get_temp::<bool>(invalid_id)).unwrap_or(false);
    let (response, event) =
        widgets::number_field(ui, id, &format_bound(value), Field::new().width(box_width).aria(&name).invalid(invalid).enabled(enabled));
    response.ctx.accesskit_node_builder(response.id, |node| {
        if invalid {
            node.set_invalid(egui::accesskit::Invalid::True);
        }
    });
    if focus {
        response.request_focus();
        response.scroll_to_me(Some(Align::Center));
    }
    match event {
        FieldEvent::None => {}
        FieldEvent::Input(typed) => {
            let ok = widgets::value_as_number(&typed).is_finite();
            ui.data_mut(|d| {
                if ok {
                    d.remove::<bool>(invalid_id);
                } else {
                    d.insert_temp(invalid_id, true);
                }
            });
        }
        FieldEvent::Change(typed) => {
            ui.data_mut(|d| d.remove::<bool>(invalid_id));
            edit = Some(if widgets::value_as_number(&typed).is_finite() { BoundEdit::Typed(typed) } else { BoundEdit::NotANumber });
        }
    }
    edit
}

// What the user did

#[derive(Debug, Clone)]
enum Act {
    Search(String),
    GroupBy(Grouping),
    Filter(ReactionFilter),
    Sort(SortKey),
    ToggleGroup(String),
    Tick(Vec<String>, bool),
    TickNone,
    Open(String),
    Locate(String),
    Reveal(String),
    Ko(String),
    Reset(String),
    ResetAll,
    Undo,
    Redo,
    ToggleChanges,
    ToggleSplit,
    ImportCsv,
    ExportCsv,
    Bulk(&'static str),
    Bound { rid: String, lower: bool, value: UserNumber, live: bool },
    NotANumber,
    Coefficient(String, UserNumber),
    Sense(ObjectiveSense),
    Preset(&'static str),
    Product(String),
    ResetObjective,
    CompactMode(CompactMode),
    OpenPage,
    Close,
}

/// What `onSidebarClick` / `onSidebarInput` do for the page's controls. An
/// edit a published case refuses, or one that fails, is said in the status line.
fn apply(shell: &mut Shell, acts: Vec<Act>) {
    for act in acts {
        let result = match act {
            Act::Search(query) => {
                shell.debounce("rx-search", 0.14, Deferred::ReactionSearch(query));
                Ok(())
            }
            Act::GroupBy(group_by) => {
                shell.store.set_reaction_table(ReactionTablePatch { group_by: Some(group_by), ..Default::default() });
                Ok(())
            }
            Act::Filter(filter) => {
                shell.store.set_reaction_table(ReactionTablePatch { toggle_filter: Some(filter), ..Default::default() });
                Ok(())
            }
            Act::Sort(key) => {
                shell.store.set_reaction_table(ReactionTablePatch { sort_by: Some(key), ..Default::default() });
                Ok(())
            }
            Act::ToggleGroup(key) => {
                shell.store.set_reaction_table(ReactionTablePatch { toggle_group: Some(key), ..Default::default() });
                Ok(())
            }
            Act::Tick(ids, on) => {
                shell.store.select_reaction_rows(&ids, on);
                Ok(())
            }
            Act::TickNone => {
                shell.store.clear_reaction_row_selection();
                Ok(())
            }
            Act::Open(rid) => {
                shell.store.select(Some(&rid));
                Ok(())
            }
            Act::Locate(rid) => {
                shell.locate_reaction(&rid);
                Ok(())
            }
            Act::Reveal(rid) => {
                shell.reveal_in_reactions(&rid);
                Ok(())
            }
            Act::Ko(rid) => {
                if shell.store.reaction_knocked_out(&rid) {
                    shell.store.reset_reaction(&rid)
                } else {
                    shell.store.knockout(&rid)
                }
            }
            Act::Reset(rid) => shell.store.reset_reaction(&rid),
            Act::ResetAll => shell.store.reset_all_reactions(),
            Act::Undo => shell.store.undo_constraints().map(|_| ()),
            Act::Redo => shell.store.redo_constraints().map(|_| ()),
            Act::ToggleChanges => {
                shell.ui.rx.show_changes = !shell.ui.rx.show_changes;
                Ok(())
            }
            Act::ToggleSplit => {
                shell.ui.rx.split = !shell.ui.rx.split;
                Ok(())
            }
            Act::ImportCsv => {
                shell.import_bounds_csv();
                Ok(())
            }
            Act::ExportCsv => {
                shell.export_bounds_csv();
                Ok(())
            }
            Act::Bulk(kind) => {
                let rx = &shell.ui.rx;
                let action = match kind {
                    "set-bounds" => BulkAction::SetBounds { lower: rx.bulk_lower.as_str().into(), upper: rx.bulk_upper.as_str().into() },
                    "scale" => BulkAction::Scale(rx.bulk_factor.as_str().into()),
                    "knockout" => BulkAction::Knockout,
                    "reset" => BulkAction::Reset,
                    _ => BulkAction::CloseUptakesExcept,
                };
                shell.store.bulk_reaction_action(action, None).map(|_| ())
            }
            Act::Bound { rid, lower, value, live } => {
                let (l, u) = if lower { (value, UserNumber::Empty) } else { (UserNumber::Empty, value) };
                let result = shell.store.set_reaction_bounds(&rid, l, u).map(|_| ());
                if result.is_ok() {
                    // A drag solves once a frame; a typed value at once.
                    if live {
                        shell.schedule_solve();
                    } else {
                        shell.store.solve();
                    }
                }
                result
            }
            Act::NotANumber => {
                shell.status_warn(NOT_A_NUMBER);
                Ok(())
            }
            Act::Coefficient(rid, value) => shell.store.set_objective_coefficient(&rid, value).map(|_| ()),
            Act::Sense(sense) => shell.store.set_objective_sense(sense).map(|_| ()),
            Act::Preset(preset) => shell.store.set_objective(preset, None),
            Act::Product(base) => shell.store.set_objective(objectives::PRODUCT, Some(&base)),
            Act::ResetObjective => shell.store.reset_objective().map(|_| ()),
            Act::CompactMode(mode) => {
                shell.ui.rx.compact_mode = mode;
                Ok(())
            }
            Act::OpenPage => {
                shell.open_workspace(crate::ui_state::Workspace::Reactions);
                Ok(())
            }
            Act::Close => {
                shell.close_workspace();
                Ok(())
            }
        };
        if let Err(error) = result {
            shell.status_error(error.to_string());
        }
    }
}

// Columns

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Col {
    Select,
    Reaction,
    Species,
    Compartment,
    Subsystem,
    Type,
    Lower,
    Upper,
    Flux,
    Fva,
    Rc,
    Objective,
    Status,
    Actions,
}

#[derive(Debug, Clone, Copy)]
struct Column {
    id: Col,
    /// A minimum: a wider table shares the extra out among the middle columns.
    width: f32,
    label: &'static str,
    sort: Option<SortKey>,
}

const fn col(id: Col, width: f32, label: &'static str, sort: Option<SortKey>) -> Column {
    Column { id, width, label, sort }
}

/// `reactionColumns(model, mode)`: FVA and reduced costs only when there is
/// something in them.
fn columns(available: TableColumns, compact: bool) -> Vec<Column> {
    let all: &[Column] = if compact {
        &[
            col(Col::Reaction, 200.0, "Reaction", Some(SortKey::Reaction)),
            col(Col::Species, 80.0, "Species", Some(SortKey::Species)),
            col(Col::Type, 112.0, "Type", Some(SortKey::Type)),
            col(Col::Lower, 84.0, "Lower", Some(SortKey::Lower)),
            col(Col::Upper, 84.0, "Upper", Some(SortKey::Upper)),
            col(Col::Flux, 84.0, "Flux", Some(SortKey::Flux)),
            col(Col::Fva, 136.0, "FVA min–max", Some(SortKey::Fva)),
            col(Col::Rc, 92.0, "Reduced cost", Some(SortKey::Rc)),
            col(Col::Status, 112.0, "Status", Some(SortKey::Status)),
            col(Col::Actions, 78.0, "", None),
        ]
    } else {
        &[
            col(Col::Select, 30.0, "", None),
            col(Col::Reaction, 190.0, "Reaction", Some(SortKey::Reaction)),
            col(Col::Species, 82.0, "Species", Some(SortKey::Species)),
            col(Col::Compartment, 100.0, "Compartment", Some(SortKey::Compartment)),
            col(Col::Subsystem, 112.0, "Subsystem", Some(SortKey::Subsystem)),
            col(Col::Type, 108.0, "Type", Some(SortKey::Type)),
            col(Col::Lower, 176.0, "Lower bound", Some(SortKey::Lower)),
            col(Col::Upper, 176.0, "Upper bound", Some(SortKey::Upper)),
            col(Col::Flux, 76.0, "Flux", Some(SortKey::Flux)),
            col(Col::Fva, 132.0, "FVA min–max", Some(SortKey::Fva)),
            col(Col::Rc, 88.0, "Reduced cost", Some(SortKey::Rc)),
            col(Col::Objective, 74.0, "Objective", Some(SortKey::Objective)),
            col(Col::Status, 110.0, "Status", Some(SortKey::Status)),
            col(Col::Actions, 104.0, "", None),
        ]
    };
    all.iter()
        .copied()
        .filter(|c| match c.id {
            Col::Fva => available.fva,
            Col::Rc => available.rc,
            _ => true,
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pin {
    Left,
    Middle,
    Right,
}

/// Where each column sits in a row `width` wide, with the middle columns
/// scrolled `scroll` to the left.
struct Geometry {
    cells: Vec<(Column, Pin, f32, f32)>,
    left: f32,
    right: f32,
    /// How far the middle columns can scroll.
    overflow: f32,
    width: f32,
    scroll: f32,
}

impl Geometry {
    fn new(columns: &[Column], width: f32, scroll: f32) -> Geometry {
        let pin = |c: &Column| match c.id {
            Col::Select => Pin::Left,
            Col::Reaction => Pin::Left,
            Col::Actions => Pin::Right,
            _ => Pin::Middle,
        };
        let sum = |p: Pin| columns.iter().filter(|c| pin(c) == p).map(|c| c.width).sum::<f32>();
        let (left, right, middle) = (sum(Pin::Left), sum(Pin::Right), sum(Pin::Middle));
        let room = (width - left - right).max(0.0);
        let grow = if middle > 0.0 && room > middle { room / middle } else { 1.0 };
        let overflow = (middle - room).max(0.0);
        let scroll = scroll.clamp(0.0, overflow);
        let mut cells = Vec::new();
        let (mut xl, mut xm, mut xr) = (0.0, left - scroll, width - right);
        for c in columns {
            match pin(c) {
                Pin::Left => {
                    cells.push((*c, Pin::Left, xl, c.width));
                    xl += c.width;
                }
                Pin::Middle => {
                    let w = c.width * grow;
                    cells.push((*c, Pin::Middle, xm, w));
                    xm += w;
                }
                Pin::Right => {
                    cells.push((*c, Pin::Right, xr, c.width));
                    xr += c.width;
                }
            }
        }
        Geometry { cells, left, right, overflow, width, scroll }
    }

    /// The rect of each cell of a row at `origin`, with the rect it is clipped to.
    fn place(&self, origin: egui::Pos2, height: f32) -> Vec<(Column, Rect, Rect)> {
        let middle = Rect::from_min_max(pos2(origin.x + self.left, origin.y), pos2(origin.x + self.width - self.right, origin.y + height));
        self.cells
            .iter()
            .map(|(c, pin, x, w)| {
                let rect = Rect::from_min_size(pos2(origin.x + x, origin.y), vec2(*w, height));
                let clip = if *pin == Pin::Middle { rect.intersect(middle) } else { rect };
                (*c, rect, clip)
            })
            .collect()
    }
}

/// A child ui for one cell, with an id of its own so its controls keep their
/// state while the rows around them come and go.
fn in_cell<R>(ui: &mut Ui, id: Id, rect: Rect, clip: Rect, add: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    if clip.width() < 1.0 {
        return None;
    }
    let mut child = ui.new_child(UiBuilder::new().id(id).max_rect(rect.shrink2(vec2(6.0, 0.0))).layout(Layout::left_to_right(Align::Center)));
    child.set_clip_rect(clip.intersect(ui.clip_rect()));
    Some(add(&mut child))
}

// Colours

fn hue(c: Color32, p: &Palette, ink: f32, fill: f32, line: f32) -> (Color32, Color32, Color32) {
    let mix = |a: Color32, b: Color32, t: f32| {
        let m = |x: u8, y: u8| (x as f32 * t + y as f32 * (1.0 - t)).round() as u8;
        Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
    };
    (mix(c, p.ink, ink), mix(c, p.panel, fill), mix(c, p.panel, line))
}

fn type_hue(t: ReactionType) -> Color32 {
    match t {
        ReactionType::Exchange => Color32::from_rgb(0x25, 0x63, 0xeb),
        ReactionType::Uptake => Color32::from_rgb(0x0d, 0x94, 0x88),
        ReactionType::Transport => Color32::from_rgb(0x7c, 0x3a, 0xed),
        ReactionType::Internal => Color32::from_rgb(0x64, 0x74, 0x8b),
        ReactionType::Biomass => Color32::from_rgb(0x16, 0xa3, 0x4a),
        ReactionType::Demand => Color32::from_rgb(0xc2, 0x41, 0x0c),
    }
}

/// The pink the map draws a knocked-out reaction in.
const KNOCKED_OUT: Color32 = Color32::from_rgb(0xdb, 0x27, 0x77);
const MODIFIED: Color32 = Color32::from_rgb(0xd9, 0x77, 0x06);

/// `.rx-type`, `.rx-status`: a small rounded tag.
fn pill(ui: &mut Ui, text: &str, colors: (Color32, Color32, Color32), title: &str) {
    let r = Frame::new()
        .fill(colors.1)
        .stroke(Stroke::new(1.0, colors.2))
        .corner_radius(CornerRadius::same(99))
        .inner_margin(Margin::symmetric(7, 1))
        .show(ui, |ui| ui.add(egui::Label::new(RichText::new(text).size(10.5).family(bold()).color(colors.0)).truncate()))
        .response;
    if !title.is_empty() {
        r.on_hover_text(title);
    }
}

fn status_colors(status: RowStatus, p: &Palette) -> (Color32, Color32, Color32) {
    match status {
        RowStatus::Culprit => (p.panel, p.danger, p.danger),
        RowStatus::KnockedOut => hue(KNOCKED_OUT, p, 0.8, 0.12, 0.4),
        RowStatus::Modified => hue(MODIFIED, p, 0.8, 0.12, 0.4),
        RowStatus::Blocked => (p.ink_3, p.panel, p.line),
        RowStatus::Original => (p.ink_3, p.panel_2, p.line),
    }
}

fn status_title(row: &ReactionRow, store: &Store) -> String {
    let original = format!("As opened: [{}, {}]", format_flux(Some(row.original_lower), 4), format_flux(Some(row.original_upper), 4));
    match row.status {
        RowStatus::Culprit => format!(
            "This edit left the LP without an optimum ({}). {original}",
            store.constraint_culprits.as_ref().map_or("infeasible", |c| c.status.as_str())
        ),
        RowStatus::KnockedOut | RowStatus::Modified => original,
        RowStatus::Blocked => "FVA: carries no flux at the optimum".into(),
        RowStatus::Original => "The model’s bounds".into(),
    }
}

// Page

/// The search, filters, order and grouping as one text: a new one is a new
/// list, read from the top.
fn listing(store: &Store) -> String {
    let t = &store.reaction_table;
    format!("{}|{:?}|{:?}|{:?}", t.query, t.filters, t.sort, t.group_by)
}

/// The whole Reactions page (`renderReactionsPage`), header included.
pub fn page(ui: &mut Ui, shell: &mut Shell) {
    let p = shell.palette();
    let model = shell.reaction_table_model();
    let mut acts = Vec::new();
    Frame::new().fill(p.panel).stroke(Stroke::new(1.0, p.line)).corner_radius(CornerRadius::same(14)).outer_margin(Margin::same(10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.set_height(ui.available_height());
        // The table keeps to the panel even if a line above it runs over.
        let right = ui.max_rect().max.x;
        Frame::new().inner_margin(Margin { left: 16, right: 16, top: 10, bottom: 6 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            header(ui, shell, &p, &mut acts);
            ui.add_space(4.0);
            toolbar(ui, shell, &model, &p, &mut acts);
            objective_line(ui, &shell.store, &p, &mut acts);
            alert(ui, &shell.store, &p, &mut acts);
            if !shell.store.is_zuniga_study() && !shell.store.reaction_table.selection.is_empty() {
                bulk_bar(ui, shell, &p, &mut acts);
            }
        });
        let rect = ui.available_rect_before_wrap().shrink2(vec2(10.0, 0.0));
        let rect = Rect::from_min_max(rect.min, pos2(rect.max.x.min(right - 10.0), rect.max.y - 8.0));
        let (table_rect, changes_rect) = if shell.ui.rx.show_changes && rect.width() > 640.0 {
            let w = 300.0;
            (Rect::from_min_max(rect.min, pos2(rect.max.x - w - 10.0, rect.max.y)), Some(Rect::from_min_max(pos2(rect.max.x - w, rect.min.y), rect.max)))
        } else {
            (rect, None)
        };
        ui.scope_builder(UiBuilder::new().max_rect(table_rect), |ui| table(ui, shell, &model, false, &p, &mut acts));
        if let Some(r) = changes_rect {
            ui.scope_builder(UiBuilder::new().max_rect(r), |ui| changes_panel(ui, &shell.store, &p, &mut acts));
        }
        ui.allocate_rect(rect, Sense::hover());
    });
    apply(shell, acts);
}

/// The page's title bar: `WORKSPACE`, the title, and the way back to the map.
fn header(ui: &mut Ui, shell: &Shell, p: &Palette, acts: &mut Vec<Act>) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(RichText::new("WORKSPACE").size(10.0).family(bold()).color(p.ink_3).extra_letter_spacing(0.5));
            ui.label(RichText::new("Reactions & bounds").size(17.0).family(bold()).color(p.ink));
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if Btn::new("← Return to map").height(32.0).show(ui).clicked() {
                acts.push(Act::Close);
            }
            let split = shell.ui.rx.split;
            if Btn::new("Map").pressed(split).title("Show the map above the table").height(32.0).show(ui).clicked() {
                acts.push(Act::ToggleSplit);
            }
        });
    });
}

/// `.rx-toolbar`: search, group by, the quick filters, the count and the actions.
fn toolbar(ui: &mut Ui, shell: &Shell, model: &ReactionTableModel, p: &Palette, acts: &mut Vec<Act>) {
    let store = &shell.store;
    let table = &store.reaction_table;
    let locked = store.is_zuniga_study();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        let shown = match shell.deferred.get("rx-search") {
            Some((_, Deferred::ReactionSearch(q))) => q.clone(),
            _ => table.query.clone(),
        };
        let (response, event) = widgets::text_field(
            ui,
            search_id(),
            &shown,
            Field::new().width(250.0).placeholder("Search id, name, gene, metabolite, EC…").aria("Search reactions"),
        );
        let _ = response;
        if let Some(q) = event.value() {
            acts.push(Act::Search(q));
        }
        ui.label(RichText::new("Group by").size(12.0).color(p.ink_2));
        let options: Vec<(Grouping, String)> = Grouping::ALL.iter().map(|g| (*g, g.label().to_string())).collect();
        if let Some(g) = widgets::select(ui, "rx-group-by", &table.group_by, &options, 110.0, true, "Group reactions by") {
            acts.push(Act::GroupBy(g));
        }
        ui.add_space(6.0);
        for filter in ReactionFilter::ALL {
            let on = table.filters.contains(&filter);
            if Btn::new(RichText::new(filter.label()).size(11.5)).pressed(on).title(filter.title()).height(24.0).show(ui).clicked() {
                acts.push(Act::Filter(filter));
            }
        }
    });
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        let mut job = egui::text::LayoutJob::default();
        let fmt = |color: Color32, b: bool| egui::TextFormat {
            font_id: egui::FontId::new(12.0, if b { bold() } else { egui::FontFamily::Proportional }),
            color,
            ..Default::default()
        };
        job.append(&model.shown.to_string(), 0.0, fmt(p.ink, true));
        job.append(&format!(" of {} reactions", model.total), 0.0, fmt(p.ink_2, false));
        if model.modified > 0 {
            job.append(&format!(" · {} modified", model.modified), 0.0, fmt(MODIFIED, true));
        }
        ui.label(job);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            if Btn::new("Export CSV").title("Every reaction's bounds in force, as CSV").height(26.0).show(ui).clicked() {
                acts.push(Act::ExportCsv);
            }
            if Btn::new("Import CSV").title("Apply a CSV of id, lower, upper").enabled(!locked).height(26.0).show(ui).clicked() {
                acts.push(Act::ImportCsv);
            }
            let changes = store.constraint_changes().len() + usize::from(store.custom_objective.is_some());
            let r = Btn::new(format!("Changes ({changes})"))
                .pressed(shell.ui.rx.show_changes)
                .title("Every reaction whose bounds differ from the model's")
                .height(26.0)
                .show(ui);
            if r.clicked() {
                acts.push(Act::ToggleChanges);
            }
            let history = &store.constraint_history;
            let redo_title = if history.can_redo() && !locked { format!("Redo: {}", history.redo_label()) } else { "No constraint edit to redo".into() };
            if Btn::new("↷ Redo").enabled(history.can_redo() && !locked).title(&redo_title).aria("Redo constraint edit").height(26.0).show(ui).clicked() {
                acts.push(Act::Redo);
            }
            let undo_title = if history.can_undo() && !locked { format!("Undo: {}", history.undo_label()) } else { "No constraint edit to undo".into() };
            if Btn::new("↶ Undo").enabled(history.can_undo() && !locked).title(&undo_title).aria("Undo constraint edit").height(26.0).show(ui).clicked() {
                acts.push(Act::Undo);
            }
        });
    });
    ui.add_space(4.0);
}

/// The id of the page's search field (focused when the page opens).
pub fn search_id() -> Id {
    Id::new("rx-search")
}

/// `.rx-objective-line`: the objective in force, its sense, the presets.
fn objective_line(ui: &mut Ui, store: &Store, p: &Palette, acts: &mut Vec<Act>) {
    let locked = store.is_zuniga_study();
    let coefficients = store.objective_coefficients();
    let custom = store.custom_objective.is_some();
    widgets::boxed(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
            ui.label(RichText::new("Objective").size(12.0).family(bold()).color(p.ink))
                .on_hover_text("Type a coefficient in any row’s Objective column to add that reaction; 0 takes it out");
            if !locked {
                let sense = store.objective_sense();
                let options = [(ObjectiveSense::Max, "Maximise".to_string()), (ObjectiveSense::Min, "Minimise".to_string())];
                if let Some(s) = widgets::select(ui, "rx-objective-sense", &sense, &options, 90.0, true, "Maximise or minimise the objective") {
                    acts.push(Act::Sense(s));
                }
            }
            if coefficients.is_empty() {
                ui.label(RichText::new(if locked { "—" } else { "no reaction: any feasible flux" }).size(12.0).italics().color(p.ink_3));
            }
            for (id, c) in coefficients.iter().take(6) {
                ui.label(RichText::new(format!("{} ×", format_bound(*c))).size(12.0).family(bold()).color(p.ink));
                let title = store.reaction_title(id);
                if widgets::link_button(ui, &title).clicked() {
                    acts.push(Act::Open(id.clone()));
                }
                if !locked {
                    let r = Btn::new("×").look(Look::Flat).aria(&format!("Take {id} out of the objective")).height(20.0).show(ui);
                    if r.clicked() {
                        acts.push(Act::Coefficient(id.clone(), UserNumber::Number(0.0)));
                    }
                }
            }
            if coefficients.len() > 6 {
                ui.label(RichText::new(format!("+ {} more", coefficients.len() - 6)).size(11.5).color(p.ink_3));
            }
            if locked {
                ui.label(RichText::new("The authors’ objective, medium and bounds are fixed for this case.").size(11.0).color(p.ink_3));
                return;
            }
            ui.label(RichText::new("add any reaction from its Objective column").size(11.0).color(p.ink_3));
            if custom {
                pill(ui, "Edited", hue(MODIFIED, p, 0.8, 0.12, 0.4), "Edited here, on top of the preset");
            }
            ui.add_space(8.0);
            ui.label(RichText::new("Presets").size(11.0).color(p.ink_3));
            for (preset, label, title) in [
                (objectives::EQUAL_GROWTH, "Equal growth", "Every species grows at one shared rate"),
                (objectives::SUM_BIOMASS, "Biomass sum", "The abundance-weighted sum of the biomass reactions"),
            ] {
                let on = !custom && store.objective == preset;
                if Btn::new(RichText::new(label).size(11.5)).pressed(on).title(title).height(24.0).show(ui).clicked() {
                    acts.push(Act::Preset(preset));
                }
            }
            let mut bases: Vec<String> = store.medium_reactions().by_base.keys().cloned().collect();
            bases.sort_by(|a, b| js::locale_compare(a, b));
            let product = if !custom && store.objective == objectives::PRODUCT { store.product_target.clone().unwrap_or_default() } else { String::new() };
            let mut options = vec![(String::new(), "Product…".to_string())];
            options.extend(bases.into_iter().map(|b| (b.clone(), b)));
            // A select is a box of its own and does not wrap: it starts the
            // next line when the rest of this one is too short for it.
            if ui.available_size_before_wrap().x < 130.0 {
                ui.end_row();
            }
            if let Some(base) = widgets::select(ui, "rx-objective-product", &product, &options, 110.0, true, "Maximise the export of a product") {
                if !base.is_empty() {
                    acts.push(Act::Product(base));
                }
            }
            if custom && Btn::new(RichText::new("Back to preset").size(11.5)).height(24.0).show(ui).clicked() {
                acts.push(Act::ResetObjective);
            }
        });
    });
    ui.add_space(4.0);
}

/// `.rx-alert`: what the last solve says, and the way back from an edit that
/// lost the optimum.
fn alert(ui: &mut Ui, store: &Store, p: &Palette, acts: &mut Vec<Act>) {
    if store.is_zuniga_study() {
        widgets::hint(ui, "Zuñiga case: the source bounds are fixed. Choose a documented case scenario to apply its perturbation.");
        return;
    }
    if let Some(culprits) = &store.constraint_culprits {
        widgets::tinted(ui, p.danger_bg, p.danger_line, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.label(RichText::new(format!("No optimum ({})", culprits.status)).size(12.0).family(bold()).color(p.danger));
                ui.label(RichText::new(format!("after “{}”.", culprits.label)).size(12.0).color(p.ink));
                if !culprits.ids.is_empty() {
                    ui.label(RichText::new("Flagged:").size(12.0).color(p.ink));
                    for id in culprits.ids.iter().take(6) {
                        if widgets::link_button(ui, &store.reaction_title(id)).clicked() {
                            acts.push(Act::Reveal(id.clone()));
                        }
                    }
                    if culprits.ids.len() > 6 {
                        ui.label("…");
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if Btn::new("Revert last change").primary().height(26.0).show(ui).clicked() {
                        acts.push(Act::Undo);
                    }
                });
            });
        });
        ui.add_space(4.0);
        return;
    }
    match store.solution.as_ref() {
        Some(solution) if !solution.is_optimal() => {
            let message = if solution.message.is_empty() { "Check the bounds of the flagged or modified reactions." } else { solution.message.as_str() };
            widgets::tinted(ui, p.danger_bg, p.danger_line, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(format!("No optimum ({}).", solution.status.as_str())).size(12.0).family(bold()).color(p.danger));
                    ui.label(RichText::new(message).size(12.0).color(p.ink));
                });
            });
            ui.add_space(4.0);
        }
        None if store.community.is_some() => {
            widgets::hint(ui, "Not solved yet: run an analysis to see fluxes. Bounds can be edited now.");
        }
        _ => {}
    }
}

/// `.rx-bulk-bar`: one action on every ticked row.
fn bulk_bar(ui: &mut Ui, shell: &mut Shell, p: &Palette, acts: &mut Vec<Act>) {
    let count = shell.store.reaction_table.selection.len();
    let rx = &mut shell.ui.rx;
    widgets::tinted(ui, p.accent_soft, p.accent_line, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
            ui.label(RichText::new(format!("{count} ticked")).size(12.0).family(bold()).color(p.ink));
            let field = |ui: &mut Ui, id: &str, value: &mut String, placeholder: &str, aria: &str| {
                let (_, event) = widgets::number_field(ui, Id::new(("rx-bulk", id.to_string())), value, Field::new().width(64.0).placeholder(placeholder).aria(aria));
                if let Some(v) = event.value() {
                    *value = v;
                }
            };
            field(ui, "lower", &mut rx.bulk_lower, "lower", "Lower bound for the ticked reactions");
            field(ui, "upper", &mut rx.bulk_upper, "upper", "Upper bound for the ticked reactions");
            if Btn::new("Set bounds").title("An empty side is left as it is").height(26.0).show(ui).clicked() {
                acts.push(Act::Bulk("set-bounds"));
            }
            ui.add_space(6.0);
            ui.label("×");
            field(ui, "factor", &mut rx.bulk_factor, "0.5", "Scale factor");
            if Btn::new("Scale").title("Multiply the finite bounds; ±1000 (unbounded) stays").height(26.0).show(ui).clicked() {
                acts.push(Act::Bulk("scale"));
            }
            ui.add_space(6.0);
            if Btn::new("Knock out").danger().height(26.0).show(ui).clicked() {
                acts.push(Act::Bulk("knockout"));
            }
            if Btn::new("Reset").aria("Reset the ticked reactions").height(26.0).show(ui).clicked() {
                acts.push(Act::Bulk("reset"));
            }
            if Btn::new("Close all uptakes except ticked").title("Close the uptake of every exchange except the ticked ones").height(26.0).show(ui).clicked() {
                acts.push(Act::Bulk("close-uptakes-except"));
            }
            if Btn::new("Untick all").height(26.0).show(ui).clicked() {
                acts.push(Act::TickNone);
            }
        });
    });
    ui.add_space(4.0);
}

/// `.rx-changes`: every modified reaction, original → current, each with
/// its reset, and Reset all.
fn changes_panel(ui: &mut Ui, store: &Store, p: &Palette, acts: &mut Vec<Act>) {
    let locked = store.is_zuniga_study();
    let changes = store.constraint_changes();
    Frame::new().fill(p.panel_2).stroke(Stroke::new(1.0, p.line)).corner_radius(CornerRadius::same(10)).inner_margin(Margin::same(10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.set_height(ui.available_height());
        ui.horizontal(|ui| {
            ui.label(RichText::new("Changes").size(13.0).family(bold()).color(p.ink));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let r = Btn::new("Reset all")
                    .enabled(!changes.is_empty() && !locked)
                    .title("Every reaction back to the bounds it was opened with")
                    .height(24.0)
                    .show(ui);
                if r.clicked() {
                    acts.push(Act::ResetAll);
                }
            });
        });
        ui.separator();
        if changes.is_empty() && store.custom_objective.is_none() {
            widgets::empty(ui, "No reaction differs from the bounds it was opened with.");
            return;
        }
        let bound = |b: &coxeter_state::Bounds| format!("[{}, {}]", format_bound_text(b.lower), format_bound_text(b.upper));
        egui::ScrollArea::vertical().id_salt("rx-changes").auto_shrink([false, false]).show(ui, |ui| {
            if store.custom_objective.is_some() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("Objective").size(12.0).family(bold()).color(p.ink));
                    ui.label(RichText::new(store.objective_description()).size(11.5).color(p.ink_2));
                    if !locked && Btn::new("↺").title("Back to the preset objective").aria("Objective back to the preset").height(22.0).show(ui).clicked() {
                        acts.push(Act::ResetObjective);
                    }
                });
                ui.separator();
            }
            for change in &changes {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let color = if change.knocked_out { KNOCKED_OUT } else { p.accent };
                    let r = ui.add(egui::Button::new(RichText::new(&change.label).size(12.0).family(bold()).color(color)).frame(false));
                    aria_label(&r, &change.label);
                    if r.clicked() {
                        acts.push(Act::Reveal(change.id.clone()));
                    }
                    if let Some(sp) = &change.species_id {
                        ui.label(RichText::new(sp).size(10.5).color(p.ink_3));
                    }
                    if change.medium.is_some() {
                        ui.label(RichText::new("medium").size(10.5).color(p.ink_3)).on_hover_text("Its uptake is the medium supply");
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if !locked && Btn::new("↺").title("Reset to the bounds as opened").aria(&format!("Reset {}", change.label)).height(22.0).show(ui).clicked() {
                            acts.push(Act::Reset(change.id.clone()));
                        }
                    });
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new(bound(&change.original)).size(11.0).strikethrough().color(p.ink_3)).on_hover_text("As opened");
                    ui.label(RichText::new("→").size(11.0).color(p.ink_3));
                    ui.label(RichText::new(bound(&change.current)).size(11.0).family(bold()).color(p.ink));
                });
                ui.add_space(4.0);
            }
        });
    });
}

// Table

/// The table (page or `compact`): a header row, the rows in view, and a
/// sideways scrollbar for the middle columns when they do not fit.
fn table(ui: &mut Ui, shell: &mut Shell, model: &ReactionTableModel, compact: bool, p: &Palette, acts: &mut Vec<Act>) {
    let store = &shell.store;
    let cols = columns(model.columns, compact);
    let rect = ui.available_rect_before_wrap();
    let width = rect.width();
    let scroll_id = Id::new(("rx-h-scroll", compact));
    let mut h_scroll: f32 = if compact { ui.data(|d| d.get_temp(scroll_id)).unwrap_or(0.0) } else { shell.ui.rx.h_scroll };
    let mut geo = Geometry::new(&cols, width, h_scroll);
    let row_h = if compact { COMPACT_ROW } else { PAGE_ROW };

    // The header row: tick-all and the sortable column titles.
    let (head_rect, _) = ui.allocate_exact_size(vec2(width, HEAD), Sense::hover());
    ui.painter().rect_filled(head_rect, CornerRadius::ZERO, p.panel_2);
    ui.painter().hline(head_rect.x_range(), head_rect.bottom(), Stroke::new(1.0, p.line));
    let visible_ids: Vec<String> = model.visible.iter().map(|r| r.id().to_string()).collect();
    let selection = &store.reaction_table.selection;
    let locked = store.is_zuniga_study();
    for (c, cell, clip) in geo.place(head_rect.min, HEAD) {
        in_cell(ui, Id::new(("rx-head", compact, c.id)), cell, clip, |ui| {
            if c.id == Col::Select {
                let all = !visible_ids.is_empty() && visible_ids.iter().all(|id| selection.contains(id));
                let (_, changed) = widgets::checkbox(ui, all, "", !locked, Some("Tick every listed reaction"));
                if let Some(on) = changed {
                    acts.push(Act::Tick(visible_ids.clone(), on));
                }
                return;
            }
            let Some(key) = c.sort else { return };
            let sort = store.reaction_table.sort;
            let active = sort.key == key;
            let arrow = if active { if sort.dir < 0 { " ▾" } else { " ▴" } } else { "" };
            let r = ui.add(egui::Button::new(RichText::new(format!("{}{arrow}", c.label.to_uppercase())).size(10.0).family(bold()).color(if active { p.ink } else { p.ink_3 })).frame(false));
            aria_label(&r, &format!("Sort by {}", c.label));
            if r.clicked() {
                acts.push(Act::Sort(key));
            }
        });
    }

    if model.items.is_empty() {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(if model.total > 0 { "No reaction matches the search and filters." } else { "No model is loaded." }).color(p.ink_3));
        });
        return;
    }

    // A new list is read from the top, in the page and the dock alike; a
    // revealed row is scrolled into view.
    let body_h = (ui.available_height() - if geo.overflow > 0.0 { SCROLLBAR + 2.0 } else { 0.0 }).max(row_h * 2.0);
    let state_id = Id::new(("rx-scroll-state", compact));
    let (offset, viewport): (f32, f32) = ui.data(|d| d.get_temp(state_id)).unwrap_or((0.0, body_h));
    let mut set_offset: Option<f32> = None;
    let listing = listing(store);
    let listing_id = Id::new(("rx-listing", compact));
    if ui.data(|d| d.get_temp::<String>(listing_id)).as_ref() != Some(&listing) {
        set_offset = Some(0.0);
    }
    if let Some(rid) = shell.ui.rx.reveal.take().as_deref() {
        if let Some(index) = model.items.iter().position(|i| matches!(i, TableItem::Row(r) if r.id() == rid)) {
            let top = index as f32 * row_h;
            let offset = set_offset.unwrap_or(offset);
            if top < offset || top + row_h > offset + viewport {
                set_offset = Some((top - viewport / 3.0).max(0.0));
            }
        }
    }
    let ctx = RowContext { store, compact, locked, p, current: store.selected_reaction.as_deref() };
    let mut hover = None;
    let mut area = egui::ScrollArea::vertical().id_salt(("rx-table", compact)).auto_shrink([false, false]).max_height(body_h);
    if let Some(y) = set_offset {
        area = area.vertical_scroll_offset(y);
    }
    let out = ui
        .scope(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            area.show_rows(ui, row_h, model.items.len(), |ui, range| {
                for i in range {
                    let (rect, _) = ui.allocate_exact_size(vec2(width, row_h), Sense::hover());
                    if !ui.is_rect_visible(rect) {
                        continue;
                    }
                    match &model.items[i] {
                        TableItem::Group(group) => group_row(ui, rect, &geo, group, &ctx, acts),
                        TableItem::Row(row) => {
                            if ui.rect_contains_pointer(rect) {
                                hover = Some(row.id().to_string());
                            }
                            reaction_row(ui, rect, &geo, row, &ctx, acts);
                        }
                    }
                }
            })
        })
        .inner;
    ui.data_mut(|d| {
        d.insert_temp(state_id, (out.state.offset.y, out.inner_rect.height()));
        d.insert_temp(listing_id, listing);
    });

    // Sideways: a bar under the middle columns, and the wheel's sideways
    // turn (or Shift) over the table.
    if geo.overflow > 0.0 {
        let (bar, response) = ui.allocate_exact_size(vec2(width, SCROLLBAR), Sense::click_and_drag());
        let track = Rect::from_min_max(pos2(bar.min.x + geo.left, bar.min.y + 2.0), pos2(bar.max.x - geo.right, bar.max.y - 2.0));
        let view = track.width();
        let content = view + geo.overflow;
        let thumb_w = (view * view / content).max(24.0);
        let thumb_x = track.min.x + (view - thumb_w) * (geo.scroll / geo.overflow);
        ui.painter().rect_filled(track, CornerRadius::same(4), p.panel_2);
        let thumb = Rect::from_min_size(pos2(thumb_x, track.min.y), vec2(thumb_w, track.height()));
        ui.painter().rect_filled(thumb, CornerRadius::same(4), if response.hovered() || response.dragged() { p.ink_3 } else { p.line });
        if response.dragged() {
            h_scroll = geo.scroll + response.drag_delta().x * content / view;
        } else if response.clicked() {
            if let Some(at) = response.interact_pointer_pos() {
                h_scroll = ((at.x - track.min.x - thumb_w / 2.0) / (view - thumb_w).max(1.0)) * geo.overflow;
            }
        }
        if ui.rect_contains_pointer(Rect::from_min_max(rect.min, bar.max)) {
            let delta = ui.input(|i| if i.modifiers.shift { i.smooth_scroll_delta.y } else { 0.0 } + i.smooth_scroll_delta.x);
            if delta != 0.0 {
                h_scroll -= delta;
            }
        }
        geo.scroll = h_scroll.clamp(0.0, geo.overflow);
    } else {
        geo.scroll = 0.0;
    }
    if compact {
        ui.data_mut(|d| d.insert_temp(scroll_id, geo.scroll));
    } else {
        shell.ui.rx.h_scroll = geo.scroll;
    }
    if hover.is_some() {
        shell.ui.rx.hover = hover;
    }
}

struct RowContext<'a> {
    store: &'a Store,
    compact: bool,
    locked: bool,
    p: &'a Palette,
    current: Option<&'a str>,
}

/// `groupRow`: a header with its count, how many are modified and ticked,
/// folding and ticking the whole group.
fn group_row(ui: &mut Ui, rect: Rect, geo: &Geometry, group: &GroupItem, ctx: &RowContext, acts: &mut Vec<Act>) {
    let p = ctx.p;
    ui.painter().rect_filled(rect, CornerRadius::ZERO, p.panel_2);
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, p.line));
    let mut x = rect.min.x;
    if !ctx.compact {
        let cell = Rect::from_min_size(rect.min, vec2(30.0, rect.height()));
        in_cell(ui, Id::new(("rx-group-tick", ctx.compact, &group.key)), cell, cell, |ui| {
            let all = group.count > 0 && group.selected == group.count;
            let (_, changed) = widgets::checkbox(ui, all, "", !ctx.locked, Some(&format!("Tick every reaction in {}", group.label)));
            if let Some(on) = changed {
                acts.push(Act::Tick(group.ids.clone(), on));
            }
        });
        x += 30.0;
    }
    let cell = Rect::from_min_max(pos2(x, rect.min.y), pos2(rect.min.x + geo.width, rect.max.y));
    in_cell(ui, Id::new(("rx-group", ctx.compact, &group.key)), cell, cell, |ui| {
        let arrow = if group.collapsed { "▸" } else { "▾" };
        let r = ui.add(egui::Button::new(RichText::new(format!("{arrow} {}", group.label)).size(12.5).family(bold()).color(p.ink)).frame(false));
        aria_label(&r, &group.label);
        r.ctx.accesskit_node_builder(r.id, |node| node.set_expanded(!group.collapsed));
        if r.clicked() {
            acts.push(Act::ToggleGroup(group.key.clone()));
        }
        let mut job = egui::text::LayoutJob::default();
        let fmt = |color: Color32, b: bool| egui::TextFormat {
            font_id: egui::FontId::new(11.5, if b { bold() } else { egui::FontFamily::Proportional }),
            color,
            ..Default::default()
        };
        job.append(&format!("{} reaction{}", group.count, if group.count == 1 { "" } else { "s" }), 0.0, fmt(p.ink_3, false));
        if group.modified > 0 {
            job.append(" · ", 0.0, fmt(p.ink_3, false));
            job.append(&format!("{} modified", group.modified), 0.0, fmt(p.warn, true));
        }
        if group.selected > 0 {
            job.append(&format!(" · {} ticked", group.selected), 0.0, fmt(p.ink_3, false));
        }
        ui.label(job);
    });
}

/// `reactionRow` / `reactionCell`.
fn reaction_row(ui: &mut Ui, rect: Rect, geo: &Geometry, row: &ReactionRow, ctx: &RowContext, acts: &mut Vec<Act>) {
    let p = ctx.p;
    let store = ctx.store;
    let id = row.id();
    let ticked = store.reaction_table.selection.contains(id);
    let current = ctx.current == Some(id);
    let fill = if row.status == RowStatus::Culprit {
        p.danger_bg
    } else if ticked {
        p.accent_soft
    } else if ui.rect_contains_pointer(rect) {
        p.hover
    } else {
        p.panel
    };
    ui.painter().rect_filled(rect, CornerRadius::ZERO, fill);
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, p.line));
    if current {
        ui.painter().rect_stroke(rect.shrink(0.5), CornerRadius::ZERO, Stroke::new(1.0, p.accent), egui::StrokeKind::Inside);
        ui.painter().rect_filled(Rect::from_min_size(rect.min, vec2(3.0, rect.height())), CornerRadius::ZERO, p.accent);
    }
    let editable = !ctx.locked && !row.held_out;
    let e = &row.entry;
    for (c, cell, clip) in geo.place(rect.min, rect.height()) {
        // The pinned cells hide what scrolls under them.
        if c.id == Col::Reaction || c.id == Col::Actions || c.id == Col::Select {
            ui.painter().rect_filled(cell, CornerRadius::ZERO, fill);
        }
        let cell_id = Id::new(("rx-cell", ctx.compact, id, c.id));
        in_cell(ui, cell_id, cell, clip, |ui| match c.id {
            Col::Select => {
                let (_, changed) = widgets::checkbox(ui, ticked, "", !ctx.locked, Some(&format!("Tick {}", e.label)));
                if let Some(on) = changed {
                    acts.push(Act::Tick(vec![id.to_string()], on));
                }
            }
            Col::Reaction => {
                let title = [
                    if e.label != e.local_id { format!("{} — {}", e.local_id, e.label) } else { e.local_id.clone() },
                    e.name.clone(),
                    e.equation.clone(),
                ]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
                ui.spacing_mut().item_spacing.x = 6.0;
                let r = ui.add(egui::Button::new(RichText::new(&e.label).size(12.0).family(bold()).color(p.accent)).frame(false).truncate());
                aria_label(&r, &e.label);
                let r = r.on_hover_text(&title);
                if r.clicked() {
                    acts.push(if ctx.compact { Act::Locate(id.to_string()) } else { Act::Open(id.to_string()) });
                }
                if !e.name.is_empty() && e.name != e.label {
                    ui.add(egui::Label::new(RichText::new(&e.name).size(11.0).color(p.ink_3)).truncate());
                }
            }
            Col::Species => {
                ui.add(egui::Label::new(RichText::new(&e.species).size(11.5).color(p.ink_2)).truncate()).on_hover_text(&e.species_name);
            }
            Col::Compartment => {
                let text = if e.compartment_label.is_empty() { "—" } else { e.compartment_label.as_str() };
                ui.add(egui::Label::new(RichText::new(text).size(11.5).color(p.ink_2)).truncate()).on_hover_text(&e.compartment_label);
            }
            Col::Subsystem => {
                let text = if e.subsystem.is_empty() { "—" } else { e.subsystem.as_str() };
                ui.add(egui::Label::new(RichText::new(text).size(11.5).color(p.ink_2)).truncate()).on_hover_text(&e.subsystem);
            }
            Col::Type => pill(ui, e.kind.label(), hue(type_hue(e.kind), p, 0.78, 0.13, 0.32), e.kind.title()),
            Col::Lower | Col::Upper => {
                let lower = c.id == Col::Lower;
                let value = if lower { row.lower } else { row.upper };
                if ctx.compact {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| ui.label(RichText::new(format_bound_text(value)).size(11.5)));
                    return;
                }
                if row.held_out {
                    ui.interact(cell, cell_id.with("held-out"), Sense::hover()).on_hover_text("Its species is held out of the simulation");
                }
                if let Some(edit) = bound_control(ui, Id::new(("rx-bound", lower, id)), lower, value, editable, &e.label, 60.0, false) {
                    push_bound(acts, id, lower, edit);
                }
            }
            Col::Flux => {
                let text = if row.flux.is_finite() { format_flux(Some(row.flux), 4) } else { "—".into() };
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| ui.label(RichText::new(text).size(11.5)));
            }
            Col::Fva => fva_cell(ui, row, p),
            Col::Rc => {
                let text = if row.reduced_cost.is_finite() { format_flux(Some(row.reduced_cost), 5) } else { "—".into() };
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| ui.label(RichText::new(text).size(11.5)));
            }
            Col::Objective => {
                let shown = if row.objective != 0.0 { format_bound(row.objective) } else { String::new() };
                let invalid_id = cell_id.with("aria-invalid");
                let invalid = ui.data(|d| d.get_temp::<bool>(invalid_id)).unwrap_or(false);
                let (_, event) = widgets::number_field(
                    ui,
                    Id::new(("rx-objective", id)),
                    &shown,
                    Field::new().width(ui.available_width()).placeholder("0").aria(&format!("Objective coefficient of {}", e.label)).enabled(!ctx.locked).invalid(invalid),
                );
                match event {
                    FieldEvent::Input(t) => {
                        let ok = t.trim().is_empty() || widgets::value_as_number(&t).is_finite();
                        ui.data_mut(|d| {
                if ok {
                    d.remove::<bool>(invalid_id);
                } else {
                    d.insert_temp(invalid_id, true);
                }
            });
                    }
                    FieldEvent::Change(t) => {
                        ui.data_mut(|d| d.remove::<bool>(invalid_id));
                        // An emptied box takes the reaction out of the objective.
                        if t.trim().is_empty() {
                            acts.push(Act::Coefficient(id.to_string(), UserNumber::Number(0.0)));
                        } else if widgets::value_as_number(&t).is_finite() {
                            acts.push(Act::Coefficient(id.to_string(), UserNumber::Text(t)));
                        } else {
                            acts.push(Act::NotANumber);
                        }
                    }
                    FieldEvent::None => {}
                }
            }
            Col::Status => pill(ui, row.status.label(), status_colors(row.status, p), &status_title(row, store)),
            Col::Actions => {
                ui.spacing_mut().item_spacing.x = 3.0;
                let ko_aria = format!("Knock out {}", e.label);
                let ko = Btn::new(RichText::new("KO").size(11.0).family(bold()).color(if row.knocked_out { Color32::WHITE } else { p.ink }))
                    .look(if row.knocked_out { Look::Pressed } else { Look::Unpressed })
                    .enabled(editable)
                    .title(if row.knocked_out { "Restore: undo the knockout" } else { "Knock out: both bounds 0" })
                    .aria(&ko_aria)
                    .height(22.0);
                let r = ko.show(ui);
                if row.knocked_out {
                    ui.painter().rect_filled(r.rect, CornerRadius::same(8), KNOCKED_OUT);
                    ui.painter().text(r.rect.center(), egui::Align2::CENTER_CENTER, "KO", egui::FontId::new(11.0, bold()), Color32::WHITE);
                }
                if r.clicked() {
                    acts.push(Act::Ko(id.to_string()));
                }
                if !ctx.compact {
                    let r = Btn::new("↺")
                        .enabled(row.modified && editable)
                        .title("Reset to the bounds as opened")
                        .aria(&format!("Reset {}", e.label))
                        .height(22.0)
                        .show(ui);
                    if r.clicked() {
                        acts.push(Act::Reset(id.to_string()));
                    }
                }
                let r = Btn::new("⌖")
                    .enabled(row.on_map)
                    .title(if row.on_map { "Locate on the map" } else { "Not drawn on the map" })
                    .aria(&format!("Locate {} on the map", e.label))
                    .height(22.0)
                    .show(ui);
                if r.clicked() {
                    acts.push(Act::Locate(id.to_string()));
                }
            }
        });
    }
}

fn push_bound(acts: &mut Vec<Act>, rid: &str, lower: bool, edit: BoundEdit) {
    match edit {
        BoundEdit::Slider(v) => acts.push(Act::Bound { rid: rid.to_string(), lower, value: UserNumber::Number(v), live: true }),
        BoundEdit::Typed(t) => acts.push(Act::Bound { rid: rid.to_string(), lower, value: UserNumber::Text(t), live: false }),
        BoundEdit::NotANumber => acts.push(Act::NotANumber),
    }
}

/// `fvaCell(row)`: the FVA range on the sliders' sign-log scale, with the
/// flux marked on it.
fn fva_cell(ui: &mut Ui, row: &ReactionRow, p: &Palette) {
    if !row.fva_min.is_finite() || !row.fva_max.is_finite() {
        ui.label(RichText::new("—").color(p.ink_3));
        return;
    }
    let steps = sign_log_steps(&SIGN_LOG) as f32;
    let at = |v: f64| (sign_log_position(v, &SIGN_LOG) as f32 + steps) / (2.0 * steps);
    let (bar, _) = ui.allocate_exact_size(vec2(44.0, 12.0), Sense::hover());
    let track = Rect::from_min_size(pos2(bar.min.x, bar.center().y - 3.0), vec2(bar.width(), 6.0));
    ui.painter().rect_filled(track, CornerRadius::same(3), p.line);
    let left = at(row.fva_min);
    let width = (at(row.fva_max) - left).max(0.015);
    let range = Rect::from_min_size(pos2(track.min.x + track.width() * left, track.min.y), vec2(track.width() * width, 6.0));
    ui.painter().rect_filled(range, CornerRadius::same(3), p.accent.gamma_multiply(0.65));
    if row.flux.is_finite() {
        let x = track.min.x + track.width() * at(row.flux);
        ui.painter().rect_filled(Rect::from_min_size(pos2(x - 1.0, bar.min.y), vec2(2.0, 12.0)), CornerRadius::ZERO, p.ink);
    }
    ui.add(
        egui::Label::new(RichText::new(format!("{} – {}", format_flux(Some(row.fva_min), 3), format_flux(Some(row.fva_max), 3))).size(11.0).color(p.ink_2))
            .truncate(),
    );
}

// Results dock

/// The results dock's Table view (`tableViewPane`): the same rows, filters
/// and sorting as the Reactions page, read-mostly (knock out and locate
/// only), with the way to the page itself; or the metabolites' shadow prices.
pub fn table_view(ui: &mut Ui, shell: &mut Shell, shadow: impl FnOnce(&mut Ui, &mut Shell)) {
    let p = shell.palette();
    let mut acts = Vec::new();
    let mode = shell.ui.rx.compact_mode;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (m, label) in [(CompactMode::Reactions, "Reactions"), (CompactMode::Shadow, "Shadow prices")] {
            let r = Btn::new(RichText::new(label).size(11.5)).look(if mode == m { Look::Active } else { Look::Normal }).height(24.0).show(ui);
            aria_pressed(&r, mode == m);
            if r.clicked() {
                acts.push(Act::CompactMode(m));
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if Btn::new("Open in Reactions ↗").height(24.0).show(ui).clicked() {
                acts.push(Act::OpenPage);
            }
        });
    });
    ui.add_space(4.0);
    if mode == CompactMode::Shadow {
        apply(shell, acts);
        shadow(ui, shell);
        return;
    }
    let model = shell.reaction_table_model();
    let count_h = 20.0;
    let rect = ui.available_rect_before_wrap();
    let table_rect = Rect::from_min_max(rect.min, pos2(rect.max.x, rect.max.y - count_h));
    ui.scope_builder(UiBuilder::new().max_rect(table_rect), |ui| table(ui, shell, &model, true, &p, &mut acts));
    let count_rect = Rect::from_min_max(pos2(rect.min.x, rect.max.y - count_h), rect.max);
    ui.scope_builder(UiBuilder::new().max_rect(count_rect), |ui| {
        let table = &shell.store.reaction_table;
        let filtered = if !table.filters.is_empty() || !table.query.is_empty() { " · filtered as on the Reactions page" } else { "" };
        let modified = if model.modified > 0 { format!(" · {} modified", model.modified) } else { String::new() };
        ui.label(RichText::new(format!("{} of {} reactions{modified}{filtered}", model.shown, model.total)).size(11.5).color(p.ink_3));
    });
    ui.allocate_rect(rect, Sense::hover());
    apply(shell, acts);
}
