//! The Add Reaction panel (port of `src/ui/add-reaction-panel.js`): the
//! reaction list with its stoichiometry, then one of eight directions to
//! place it; shared by the metabolite and compartment menus.
//!
//! The panel deliberately separates reaction choice from placement direction:
//! users inspect the reaction and compact stoichiometry first, then choose one
//! of eight equidistant directions to commit the insertion.
//!
//! The JavaScript renders HTML and reads the clicks back from `data-`
//! attributes; here [`show_panel`] draws the panel as an egui `Area` over the
//! stage and returns what was clicked as a [`PanelAction`], which the menu
//! that owns the panel carries out (`handleClick` of the controllers).

use std::sync::Arc;

use coxeter_state::Store;
use coxeter_core::js;
use coxeter_core::mapdata::Point;
use coxeter_render::theme::format_flux;
use egui::text::{LayoutJob, TextWrapping};
use egui::{Color32, CornerRadius, CursorIcon, FontFamily, FontId, Frame, Galley, Margin, Pos2, Rect, Response, RichText, Sense, Shape, Stroke, StrokeKind, Ui, UiBuilder, Vec2};

use crate::fonts::glyph;
use crate::theme::{Palette, bold};
use crate::widgets::{self, Field, FieldEvent};

// Directions

/// How far from the anchor the new reaction is placed (map units).
pub const REACTION_PLACEMENT_DISTANCE: f64 = 96.0;
const ROOT_HALF: f64 = std::f64::consts::FRAC_1_SQRT_2;

/// One of the eight placement directions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Direction {
    pub id: &'static str,
    pub label: &'static str,
    /// The JavaScript's arrow. The panel draws the matching arrow of
    /// [`glyph::COMPASS`] instead, since some of these have no outline in
    /// the usual system fonts.
    pub glyph: &'static str,
    pub dx: f64,
    pub dy: f64,
}

/// `REACTION_DIRECTIONS`.
pub const REACTION_DIRECTIONS: [Direction; 8] = [
    Direction { id: "n", label: "North", glyph: "↑", dx: 0.0, dy: -1.0 },
    Direction { id: "ne", label: "North-east", glyph: "↗", dx: ROOT_HALF, dy: -ROOT_HALF },
    Direction { id: "e", label: "East", glyph: "→", dx: 1.0, dy: 0.0 },
    Direction { id: "se", label: "South-east", glyph: "↘", dx: ROOT_HALF, dy: ROOT_HALF },
    Direction { id: "s", label: "South", glyph: "↓", dx: 0.0, dy: 1.0 },
    Direction { id: "sw", label: "South-west", glyph: "↙", dx: -ROOT_HALF, dy: ROOT_HALF },
    Direction { id: "w", label: "West", glyph: "←", dx: -1.0, dy: 0.0 },
    Direction { id: "nw", label: "North-west", glyph: "↖", dx: -ROOT_HALF, dy: -ROOT_HALF },
];

/// `directionById(id)`.
pub fn direction_by_id(id: &str) -> Option<&'static Direction> {
    REACTION_DIRECTIONS.iter().find(|d| d.id == id)
}

impl Direction {
    /// The arrow drawn on the wheel.
    pub fn shown_glyph(&self) -> &'static str {
        glyph::COMPASS.iter().find(|(k, _)| k.eq_ignore_ascii_case(self.id)).map_or(self.glyph, |(_, g)| *g)
    }

    /// The direction button's `aria-label`.
    pub fn aria_label(&self) -> String {
        format!("Add selected reaction to the {}", self.label)
    }

    /// The point `REACTION_PLACEMENT_DISTANCE` from `from` this way.
    pub fn anchor_from(&self, from: Point) -> Point {
        Point::new(from.x + self.dx * REACTION_PLACEMENT_DISTANCE, from.y + self.dy * REACTION_PLACEMENT_DISTANCE)
    }
}

// Where the panel sits

/// `ADD_REACTION_PANEL_WIDTH`.
pub const ADD_REACTION_PANEL_WIDTH: f64 = 380.0;
/// The tallest the panel gets, on a stage with room for it.
pub const PANEL_HEIGHT: f64 = 520.0;
/// The gap the panel keeps from the stage's edges.
pub const PANEL_MARGIN: f64 = 8.0;

fn clamp_to(value: f64, low: f64, high: f64) -> f64 {
    js::max(low, js::min(value, js::max(low, high)))
}

/// Where a controller keeps its panel (stage coordinates), and whether the
/// user has moved it by hand.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PanelPosition {
    pub left: f64,
    pub top: f64,
    pub moved: bool,
}

/// What [`panel_placement`] works out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub width: f64,
    pub max_height: f64,
    pub left: f64,
    pub top: f64,
}

/// `panelPlacement(stage, state)`: where an Add Reaction panel goes on the
/// stage (`clientWidth` × `clientHeight`), and how tall it may be.
///
/// It opens beside the click and is kept wholly on the stage, its height
/// included: it is never taller than the stage, and gives up list rows rather
/// than the wheel when it is short of room. Placed as if it were always 520 px
/// tall, on a laptop, where the stage is shorter than that, its lower part (the
/// direction wheel, the one control that actually adds the reaction) would
/// hang off the bottom edge with no way to reach it.
///
/// Once it has been dragged it stays where it was put; only its header is kept
/// within reach, so it can be moved aside to see the map behind it.
pub fn panel_placement(stage_width: f64, stage_height: f64, state: Option<&PanelPosition>) -> Placement {
    let stage_width = if stage_width.is_nan() { 0.0 } else { stage_width };
    let stage_height = if stage_height.is_nan() { 0.0 } else { stage_height };
    let width = if stage_width != 0.0 {
        js::min(ADD_REACTION_PANEL_WIDTH, js::max(240.0, stage_width - 2.0 * PANEL_MARGIN))
    } else {
        ADD_REACTION_PANEL_WIDTH
    };
    let max_height =
        if stage_height != 0.0 { js::min(PANEL_HEIGHT, js::max(220.0, stage_height - 2.0 * PANEL_MARGIN)) } else { PANEL_HEIGHT };
    if let Some(state) = state.filter(|s| s.moved) {
        return Placement {
            width,
            max_height,
            left: clamp_to(state.left, 64.0 - width, stage_width - 64.0),
            top: clamp_to(state.top, 0.0, stage_height - 40.0),
        };
    }
    Placement {
        width,
        max_height,
        left: clamp_to(state.map_or(PANEL_MARGIN, |s| s.left), PANEL_MARGIN, stage_width - width - PANEL_MARGIN),
        top: clamp_to(state.map_or(PANEL_MARGIN, |s| s.top), PANEL_MARGIN, stage_height - max_height - PANEL_MARGIN),
    }
}

/// A drag of the panel by its header: where the press was (client) and
/// where the panel stood then (`offsetLeft`, `offsetTop`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanelDrag {
    pub x: f64,
    pub y: f64,
    pub left: f64,
    pub top: f64,
}

/// `beginPanelDrag(controller, event)`: whether a press starts moving the
/// panel. Only a primary press on the header itself does (a press on a
/// control in the header is that control's), and only while the controller
/// has a panel open.
pub fn begin_panel_drag(open: bool, on_header: bool, on_control: bool, primary: bool, press: Point, placed: &Placement) -> Option<PanelDrag> {
    if !open || !on_header || on_control || !primary {
        return None;
    }
    Some(PanelDrag { x: press.x, y: press.y, left: placed.left, top: placed.top })
}

/// The drag's `pointermove`: the panel follows the pointer, kept within
/// reach, and is marked as moved so it stays where it is put.
pub fn panel_drag_move(stage_width: f64, stage_height: f64, from: &PanelDrag, client: Point, state: &mut PanelPosition) {
    let place = panel_placement(
        stage_width,
        stage_height,
        Some(&PanelPosition { left: from.left + client.x - from.x, top: from.top + client.y - from.y, moved: true }),
    );
    state.left = place.left;
    state.top = place.top;
    state.moved = true;
}

// What the rows show

/// `reactionStoichiometry(store, reactionId, { maxTerms })`:
/// `a + 2 b → c`, `↔` when the reaction runs both ways, its metabolites
/// named as the map names them.
pub fn reaction_stoichiometry(store: &Store, reaction_id: &str, max_terms: usize) -> String {
    let Some(community) = store.community.as_deref() else { return "—".into() };
    let Some(reaction) = community.merged.reactions.get(reaction_id) else { return "—".into() };
    let mut substrates = Vec::new();
    let mut products = Vec::new();
    for (merged_id, &coefficient) in &reaction.metabolites {
        if !coefficient.is_finite() || coefficient == 0.0 {
            continue;
        }
        let label = metabolite_label(store, merged_id);
        let magnitude = coefficient.abs();
        let term = if magnitude == 1.0 { label } else { format!("{} {label}", format_coefficient(magnitude)) };
        if coefficient < 0.0 { substrates.push(term) } else { products.push(term) }
    }
    let reversible = reaction.lower_bound < 0.0 && reaction.upper_bound > 0.0;
    let arrow = if reversible { "↔" } else { "→" };
    format!("{} {arrow} {}", summarize_side(&substrates, max_terms), summarize_side(&products, max_terms))
}

/// `reactionSearchText(store, reactionId)`: what the filter matches: the
/// map's label, the model id and the name, the subsystem and the equation.
pub fn reaction_search_text(store: &Store, reaction_id: &str) -> String {
    let reaction = store.community.as_ref().and_then(|c| c.merged.reactions.get(reaction_id));
    let named = store.entity_label("reaction", reaction_id);
    normalize_search(
        &[
            named.label.as_str(),
            named.id.as_str(),
            named.name.as_str(),
            reaction.map_or("", |r| r.subsystem.as_str()),
            &reaction_stoichiometry(store, reaction_id, 99),
        ]
        .join(" "),
    )
}

/// `reactionListing(store, reactionId)`: how a reaction is listed in a menu:
/// by the map's label for it, and beside that the one thing the label is
/// not (its name, or its model id when the map is labelled by name).
pub fn reaction_listing(store: &Store, reaction_id: &str) -> (String, String) {
    let named = store.entity_label("reaction", reaction_id);
    let detail = if named.label == named.name {
        if named.id == named.label { String::new() } else { named.id.clone() }
    } else {
        named.name.clone()
    };
    (named.label, detail)
}

/// Which side of a reaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Substrate,
    Product,
}

/// A metabolite the reaction's axis can run to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Terminal {
    pub id: String,
    pub label: String,
}

/// Cofactors are never offered as the end of the axis.
const COFACTOR_BASES: [&str; 12] = ["nad", "nadh", "nadp", "nadph", "coa", "h2o", "co2", "h", "atp", "adp", "amp", "pi"];

/// `primaryMetabolitesForSide(store, reactionId, side)`: the metabolites of
/// one side, cofactors left out.
pub fn primary_metabolites_for_side(store: &Store, reaction_id: &str, side: Side) -> Vec<Terminal> {
    let Some(community) = store.community.as_deref() else { return Vec::new() };
    let Some(reaction) = community.merged.reactions.get(reaction_id) else { return Vec::new() };
    let want_product = side == Side::Product;
    let mut out = Vec::new();
    for (merged_id, &coefficient) in &reaction.metabolites {
        if !coefficient.is_finite() || coefficient == 0.0 || (coefficient > 0.0) != want_product {
            continue;
        }
        let Some(metabolite) = community.merged.metabolites.get(merged_id) else { continue };
        let base = metabolite.base.to_lowercase();
        if COFACTOR_BASES.contains(&base.as_str()) {
            continue;
        }
        out.push(Terminal { id: merged_id.clone(), label: metabolite_label(store, merged_id) });
    }
    out
}

/// One `.add-reaction-choice` of the list: what it shows and what the filter
/// matches (`data-search`).
#[derive(Debug, Clone, PartialEq)]
pub struct ReactionRow {
    pub id: String,
    /// The map's label for it (`reactionListing`).
    pub label: String,
    /// Beside the label: its name, or its model id.
    pub detail: String,
    pub equation: String,
    pub search: String,
    /// Its flux in the current solution, shown in the row: set for the panel
    /// of a metabolite, whose list is ordered by these fluxes.
    pub flux: Option<f64>,
}

impl ReactionRow {
    /// The row's accessible name: its text, as a screen reader reads the
    /// `<button>` (identifier, name, equation).
    pub fn accessible_name(&self) -> String {
        [self.label.as_str(), self.detail.as_str(), self.equation.as_str()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// The rows of `renderAddReactionPanel` for these reactions.
pub fn reaction_rows(store: &Store, reaction_ids: &[String]) -> Vec<ReactionRow> {
    reaction_ids
        .iter()
        .map(|id| {
            let (label, detail) = reaction_listing(store, id);
            ReactionRow {
                id: id.clone(),
                label,
                detail,
                equation: reaction_stoichiometry(store, id, 3),
                search: reaction_search_text(store, id),
                flux: None,
            }
        })
        .collect()
}

/// `renderAddReactionPanel({ fluxes })`: every row with its reaction's flux
/// in the current optimal solution; the rows as they are without one.
pub fn with_fluxes(store: &Store, mut rows: Vec<ReactionRow>) -> Vec<ReactionRow> {
    if let Some(solution) = store.solution.as_ref().filter(|s| s.is_optimal()) {
        for row in &mut rows {
            row.flux = Some(solution.fluxes.get(&row.id).copied().unwrap_or(0.0));
        }
    }
    rows
}

/// `filterReactionButtons(input)`: which rows the query keeps, and the
/// summary line above the list.
pub fn filter_reaction_buttons(rows: &[ReactionRow], query: &str) -> (Vec<bool>, String) {
    let q = normalize_search(query);
    let shown: Vec<bool> = rows.iter().map(|r| q.is_empty() || normalize_search(&r.search).contains(&q)).collect();
    let visible = shown.iter().filter(|s| **s).count();
    let summary = if q.is_empty() { available_summary(rows.len()) } else { format!("{visible} of {} reactions", rows.len()) };
    (shown, summary)
}

/// `N reactions available`.
pub fn available_summary(n: usize) -> String {
    format!("{n} {}", if n == 1 { "reaction available" } else { "reactions available" })
}

/// `metaboliteLabel(store, mergedId)`: as the map names it (`entityLabel`).
fn metabolite_label(store: &Store, merged_id: &str) -> String {
    store.entity_label("metabolite", merged_id).label
}

fn summarize_side(terms: &[String], max_terms: usize) -> String {
    if terms.is_empty() {
        return "∅".into();
    }
    if terms.len() <= max_terms {
        return terms.join(" + ");
    }
    format!("{} + …", terms[..max_terms].join(" + "))
}

fn format_coefficient(value: f64) -> String {
    if value.fract() == 0.0 {
        return js::number_to_string(value);
    }
    js::number_to_string(js::string_to_number(&js::to_fixed(value, 3)))
}

/// `normalizeSearch(value)`: accents off, trimmed, lower case.
pub fn normalize_search(value: &str) -> String {
    coxeter_state::util::normalize_search(value)
}

// Drawing

/// What a click in the panel asks for (the `data-` attribute it carried).
#[derive(Debug, Clone, PartialEq)]
pub enum PanelAction {
    /// `data-action="close-add-reaction-menu"`.
    Close,
    /// `data-select-add-rxn`.
    Select(String),
    /// `data-rotate-terminal` (−1 or 1).
    RotateTerminal(i32),
    /// `data-add-rxn-direction`.
    Direction(&'static Direction),
}

/// What [`show_panel`] needs to draw one panel (`renderAddReactionPanel`'s
/// arguments, with the controller's state).
pub struct PanelArgs<'a> {
    /// The panel's id (one per controller).
    pub id: egui::Id,
    /// The stage, in client coordinates.
    pub stage: Rect,
    pub rows: &'a [ReactionRow],
    pub selected_reaction_id: Option<&'a str>,
    pub title: &'a str,
    pub subtitle: &'a str,
    /// The product metabolites of the selected reaction.
    pub terminals: &'a [Terminal],
    pub terminal_metabolite_id: Option<&'a str>,
}

/// The parts of a controller's state the panel itself changes: where it is,
/// the drag in progress, the filter's text.
pub struct PanelUi<'a> {
    pub position: &'a mut PanelPosition,
    pub drag: &'a mut Option<PanelDrag>,
    pub filter: &'a mut String,
}

/// `color-mix(in srgb, a p%, b)`.
pub fn mix(a: Color32, p: f32, b: Color32) -> Color32 {
    let f = |x: u8, y: u8| (x as f32 * p + y as f32 * (1.0 - p)).round() as u8;
    Color32::from_rgba_unmultiplied(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()), f(a.a(), b.a()))
}

/// `--shadow`.
pub fn panel_shadow(p: &Palette) -> egui::Shadow {
    if p.dark {
        egui::Shadow { offset: [0, 10], blur: 30, spread: 0, color: Color32::from_black_alpha(90) }
    } else {
        egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_rgba_unmultiplied(15, 23, 42, 20) }
    }
}

/// The floating box every map menu is drawn in (`.add-reaction-menu`,
/// `.boundary-menu`): a panel with a border, rounded, with a shadow, clipped
/// to the stage (`.stage { overflow: hidden }`).
pub fn menu_area<R>(ctx: &egui::Context, id: egui::Id, stage: Rect, at: Pos2, width: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let p = Palette::current(ctx);
    let inner = egui::Area::new(id)
        .order(egui::Order::Middle)
        .fixed_pos(at)
        .constrain(false)
        .movable(false)
        .interactable(true)
        .show(ctx, |ui| {
            ui.set_clip_rect(stage);
            Frame::new()
                .fill(p.panel)
                .stroke(Stroke::new(1.0, p.line))
                .corner_radius(CornerRadius::same(12))
                .shadow(panel_shadow(&p))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    ui.set_width((width - 2.0).max(10.0));
                    add(ui)
                })
                .inner
        });
    inner.inner
}

/// While a panel is dragged, the pointer belongs to it wherever it goes
/// (`setPointerCapture`): a sheet over the whole window keeps the map from
/// reading the release as a click on empty canvas, which would close the
/// very panel that was being moved.
pub fn pointer_capture_sheet(ctx: &egui::Context, id: egui::Id) {
    let screen = ctx.content_rect();
    egui::Area::new(id.with("capture"))
        .order(egui::Order::Background)
        .fixed_pos(screen.min)
        .constrain(false)
        .movable(false)
        .interactable(true)
        .show(ctx, |ui| {
            ui.allocate_space(screen.size());
        });
}

/// A line of text cut with `…` to `width` (`text-overflow: ellipsis`).
pub fn one_line(ui: &Ui, text: &str, font: FontId, color: Color32, width: f32) -> Arc<Galley> {
    let mut job = LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap = TextWrapping { max_width: width.max(1.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    ui.ctx().fonts_mut(|f| f.layout_job(job))
}

/// The header of a menu (`.add-reaction-head`): title and subtitle, and the
/// close button. Returns the header's response (it senses drags) and whether
/// the close button was clicked, with its box.
pub fn menu_head(ui: &mut Ui, title: &str, subtitle: &str, close_aria: &str, close_title: Option<&str>) -> (Response, bool, Rect) {
    let p = Palette::current(ui.ctx());
    let mut closed = false;
    let mut close_rect = Rect::NOTHING;
    let head = ui.scope_builder(UiBuilder::new().id_salt("menu-head").sense(Sense::drag()), |ui| {
        Frame::new().inner_margin(Margin::same(11)).show(ui, |ui| {
            let width = ui.available_width();
            ui.set_width(width);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                let text_w = (width - 28.0 - 10.0).max(20.0);
                ui.allocate_ui_with_layout(Vec2::new(text_w, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(text_w);
                    ui.add(egui::Label::new(RichText::new(title).size(13.0).family(bold()).color(p.ink)).wrap().selectable(false));
                    ui.add_space(2.0);
                    ui.add(egui::Label::new(RichText::new(subtitle).size(11.0).color(p.ink_3)).wrap().selectable(false));
                });
                ui.add_space(10.0);
                let close = ui.add(
                    egui::Button::new(RichText::new("×").size(18.0).color(p.ink))
                        .fill(p.panel)
                        .stroke(Stroke::new(1.0, p.line))
                        .corner_radius(CornerRadius::same(8))
                        .min_size(Vec2::splat(28.0)),
                );
                widgets::aria_label(&close, close_aria);
                let close = match close_title {
                    Some(t) => close.on_hover_text(t),
                    None => close,
                };
                close_rect = close.rect;
                closed = close.clicked();
            });
        });
    });
    let rect = head.response.rect;
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, p.line));
    (head.response, closed, close_rect)
}

/// A strip of the secondary surface with a line of small text (`.add-reaction-summary`).
pub fn menu_summary(ui: &mut Ui, text: &str) {
    let p = Palette::current(ui.ctx());
    Frame::new().fill(p.panel_2).inner_margin(Margin::symmetric(11, 7)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.add(egui::Label::new(RichText::new(text).size(11.0).color(p.ink_3)).selectable(false));
    });
}

/// `.add-reaction-empty`.
pub fn menu_empty(ui: &mut Ui, text: &str) {
    let p = Palette::current(ui.ctx());
    Frame::new().inner_margin(Margin::same(12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.add(egui::Label::new(RichText::new(text).size(12.0).color(p.ink_3)).wrap());
    });
}

/// `.add-reaction-search`: the filter field. Returns its text when the user
/// changed it (the `input` event).
pub fn menu_search(ui: &mut Ui, id: egui::Id, value: &str, placeholder: &str, aria: &str) -> Option<String> {
    let mut out = None;
    Frame::new().inner_margin(Margin { left: 8, right: 8, top: 7, bottom: 0 }).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let (_, event) = widgets::text_field(ui, id, value, Field::new().placeholder(placeholder).aria(aria));
        if let FieldEvent::Input(text) | FieldEvent::Change(text) = event {
            out = Some(text);
        }
    });
    out
}

/// Draw the Add Reaction panel (`renderAddReactionPanel`) and answer its
/// header drag (`beginPanelDrag`) and filter (`filterReactionButtons`).
/// Returns what was clicked.
pub fn show_panel(ctx: &egui::Context, args: PanelArgs<'_>, state: PanelUi<'_>) -> Option<PanelAction> {
    let p = Palette::current(ctx);
    let stage = args.stage;
    let (stage_w, stage_h) = (stage.width() as f64, stage.height() as f64);
    let place = panel_placement(stage_w, stage_h, Some(state.position));
    let selected = args.selected_reaction_id.filter(|s| args.rows.iter().any(|r| r.id == *s));
    // The wheel stands beside its caption rather than under it, and the panel is
    // cut to the height it is given: what gives way is the list, which scrolls.
    let compact = place.max_height < 380.0;
    let at = stage.min + Vec2::new(place.left as f32, place.top as f32);
    let dragging = state.drag.is_some();
    let mut action = None;
    let mut head_response = None;
    let mut close_rect = Rect::NOTHING;

    menu_area(ctx, args.id, stage, at, place.width as f32, |ui| {
        if dragging {
            // `.add-reaction-menu.is-dragging { opacity: .92 }`.
            ui.multiply_opacity(0.92);
        }
        let top = ui.cursor().top();
        let (head, closed, close) = menu_head(ui, args.title, args.subtitle, "Close Add Reaction panel", None);
        close_rect = close;
        if closed {
            action = Some(PanelAction::Close);
        }
        head_response = Some(head);

        let (shown, summary) = filter_reaction_buttons(args.rows, state.filter);
        menu_summary(ui, &summary);
        if !args.rows.is_empty() {
            if let Some(text) = menu_search(ui, args.id.with("filter"), state.filter, "Filter reactions…", "Filter reactions") {
                *state.filter = text;
            }
        }

        // The list takes what the head and the direction section leave, between
        // its CSS bounds (`min-height: 58px; max-height: 220px`).
        let used = ui.cursor().top() - top;
        let section_id = args.id.with("direction-section-height");
        let section_h: f32 = ui.data(|d| d.get_temp(section_id)).unwrap_or(if compact { 124.0 } else { 146.0 });
        let list_max = (place.max_height as f32 - used - section_h - 16.0 - 2.0).clamp(58.0 - 16.0, 220.0 - 16.0);
        Frame::new().inner_margin(Margin::same(8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            if args.rows.is_empty() {
                ui.set_min_height(58.0 - 16.0);
                ui.add(egui::Label::new(RichText::new("No reactions are available to add here.").size(12.0).color(p.ink_3)).wrap());
                return;
            }
            egui::ScrollArea::vertical().id_salt(args.id.with("list")).max_height(list_max).auto_shrink([false, true]).show(ui, |ui| {
                ui.set_min_height(58.0 - 16.0);
                ui.spacing_mut().item_spacing = Vec2::new(0.0, 6.0);
                for (row, visible) in args.rows.iter().zip(&shown) {
                    if !visible {
                        continue;
                    }
                    let is_selected = selected == Some(row.id.as_str());
                    if choice_row(ui, row, is_selected, &p).clicked() {
                        action = Some(PanelAction::Select(row.id.clone()));
                    }
                }
            });
        });

        let section_top = ui.cursor().top();
        if let Some(a) = direction_section(ui, &args, selected, compact, &p) {
            action = Some(a);
        }
        let section_h = ui.cursor().top() - section_top;
        ui.data_mut(|d| d.insert_temp(section_id, section_h));
    });

    // `beginPanelDrag`: the header moves the panel; a press on its close
    // button is the button's.
    if let Some(head) = head_response {
        let head = head.on_hover_text("Drag to move this panel").on_hover_cursor(CursorIcon::Move);
        if head.drag_started() {
            let press = ctx.input(|i| i.pointer.press_origin()).unwrap_or(head.interact_pointer_pos().unwrap_or_default());
            let on_control = close_rect.contains(press);
            let primary = head.dragged_by(egui::PointerButton::Primary);
            let client = Point::new(press.x as f64, press.y as f64);
            // `offsetLeft` / `offsetTop`: where the panel stands now.
            let placed = Placement { left: place.left, top: place.top, ..place };
            *state.drag = begin_panel_drag(true, true, on_control, primary, client, &placed);
        }
        if let Some(from) = *state.drag {
            if head.dragged() {
                if let Some(pos) = ctx.input(|i| i.pointer.interact_pos()) {
                    panel_drag_move(stage_w, stage_h, &from, Point::new(pos.x as f64, pos.y as f64), state.position);
                }
            }
            if head.drag_stopped() || !ctx.input(|i| i.pointer.any_down()) {
                *state.drag = None;
            }
        }
    }
    if state.drag.is_some() {
        pointer_capture_sheet(ctx, args.id);
        ctx.set_cursor_icon(CursorIcon::Move);
    }
    action
}

/// One `.add-reaction-choice`: identifier and name, the equation under them.
fn choice_row(ui: &mut Ui, row: &ReactionRow, selected: bool, p: &Palette) -> Response {
    let width = ui.available_width();
    let pad = Vec2::new(9.0, 8.0);
    let inner_w = (width - 2.0 * pad.x - 2.0).max(10.0);
    let mono = FontId::new(11.5, FontFamily::Monospace);
    let id_galley = one_line(ui, &row.label, mono.clone(), p.ink, inner_w);
    // `.has-flux`: `grid-template-columns: minmax(90px, auto) 1fr auto`.
    let flux_galley = row.flux.map(|v| one_line(ui, &format_flux(Some(v), 4), mono.clone(), p.ink, inner_w / 3.0));
    let flux_w = flux_galley.as_ref().map_or(0.0, |g| g.size().x + 8.0);
    // `grid-template-columns: minmax(90px, auto) 1fr; gap: 8px`.
    let id_col = id_galley.size().x.max(90.0).min(inner_w - flux_w);
    let name_w = (inner_w - id_col - 8.0 - flux_w).max(1.0);
    let name_galley = one_line(ui, &row.detail, FontId::proportional(11.5), p.ink_2, name_w);
    let eq_galley = one_line(ui, &row.equation, FontId::new(10.5, FontFamily::Monospace), p.ink_3, inner_w);
    let line_h = id_galley.size().y.max(name_galley.size().y);
    let height = pad.y * 2.0 + line_h + 5.0 + eq_galley.size().y + 2.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let label = row.accessible_name();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
    widgets::aria_pressed(&response, selected);
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let hovered = response.hovered();
        let (fill, line) = if selected {
            (mix(p.accent, 0.09, p.panel), p.accent)
        } else if hovered {
            (p.hover, p.accent)
        } else {
            (p.panel, p.line)
        };
        let r = CornerRadius::same(8);
        painter.rect(rect, r, fill, Stroke::new(1.0, line), StrokeKind::Inside);
        if selected {
            painter.rect_stroke(rect.shrink(1.0), r, Stroke::new(1.0, mix(p.accent, 0.55, Color32::TRANSPARENT)), StrokeKind::Inside);
        }
        let origin = rect.min + pad + Vec2::splat(1.0);
        let center_y = origin.y + line_h / 2.0;
        painter.galley(Pos2::new(origin.x, center_y - id_galley.size().y / 2.0), id_galley, p.ink);
        painter.galley(Pos2::new(origin.x + id_col + 8.0, center_y - name_galley.size().y / 2.0), name_galley, p.ink_2);
        if let Some(g) = flux_galley {
            painter.galley(Pos2::new(origin.x + inner_w - g.size().x, center_y - g.size().y / 2.0), g, p.ink);
        }
        painter.galley(Pos2::new(origin.x, origin.y + line_h + 5.0), eq_galley, p.ink_3);
    }
    if !selected { response.on_hover_cursor(CursorIcon::PointingHand) } else { response }
}

/// `.add-reaction-direction-section`: the caption and the terminal
/// metabolite on the left, the wheel of eight directions beside them.
fn direction_section(ui: &mut Ui, args: &PanelArgs<'_>, selected: Option<&str>, compact: bool, p: &Palette) -> Option<PanelAction> {
    let mut action = None;
    let full = ui.available_width();
    // The panel's lower corners are round: so is the section that ends it.
    let corners = CornerRadius { nw: 0, ne: 0, sw: 11, se: 11 };
    let frame = Frame::new().fill(p.panel_2).corner_radius(corners).inner_margin(Margin { left: 11, right: 10, top: 6, bottom: 6 });
    let shown = frame.show(ui, |ui| {
        ui.set_width(full - 21.0);
        let wheel = if compact { 112.0 } else { 134.0 };
        let text_w = (ui.available_width() - wheel - 10.0).max(40.0);
        let text_id = args.id.with("direction-text-height");
        let text_h: f32 = ui.data(|d| d.get_temp(text_id)).unwrap_or(wheel);
        let row_h = text_h.max(wheel);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            ui.allocate_ui_with_layout(Vec2::new(text_w, row_h), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(text_w);
                ui.add_space(((row_h - text_h) / 2.0).max(0.0));
                let block = ui.vertical(|ui| {
                    ui.set_width(text_w);
                    ui.spacing_mut().item_spacing = Vec2::new(0.0, 2.0);
                    ui.add(egui::Label::new(RichText::new("Direction").size(11.5).family(bold()).color(p.ink)).selectable(false));
                    let caption = if selected.is_some() { "Where to place the reaction." } else { "Select a reaction first." };
                    ui.add(egui::Label::new(RichText::new(caption).size(10.5).color(p.ink_3)).wrap());
                    if selected.is_some() {
                        ui.add_space(8.0);
                        if let Some(a) = terminal_control(ui, args, p) {
                            action = Some(a);
                        }
                    }
                });
                let h = block.response.rect.height();
                ui.data_mut(|d| d.insert_temp(text_id, h));
            });
            ui.add_space(10.0);
            ui.allocate_ui_with_layout(Vec2::new(wheel, row_h), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.add_space(((row_h - wheel) / 2.0).max(0.0));
                if let Some(a) = direction_wheel(ui, wheel, compact, selected.is_some(), p) {
                    action = Some(a);
                }
            });
        });
    });
    // `border-top: 1px solid var(--line)`.
    let rect = shown.response.rect;
    ui.painter().hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0, p.line));
    action
}

/// `.add-reaction-terminal`: which product the reaction's axis runs to.
fn terminal_control(ui: &mut Ui, args: &PanelArgs<'_>, p: &Palette) -> Option<PanelAction> {
    let terminals = args.terminals;
    if terminals.is_empty() {
        return None;
    }
    let index = terminals.iter().position(|t| Some(t.id.as_str()) == args.terminal_metabolite_id).unwrap_or(0);
    let terminal = &terminals[index];
    let mut action = None;
    ui.add(egui::Label::new(RichText::new("Terminal metabolite").size(11.5).family(bold()).color(p.ink)).selectable(false));
    ui.add(egui::Label::new(RichText::new("Product on the reaction axis.").size(10.5).color(p.ink_3)).wrap());
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(5.0, 0.0);
        let arrow = |ui: &mut Ui, glyph: &str, aria: &str| {
            let r = ui.add(
                egui::Button::new(RichText::new(glyph).size(13.0).color(p.ink))
                    .fill(p.panel)
                    .stroke(Stroke::new(1.0, p.line))
                    .corner_radius(CornerRadius::same(6))
                    .min_size(Vec2::splat(26.0)),
            );
            widgets::aria_label(&r, aria);
            r.clicked()
        };
        if terminals.len() > 1 && arrow(ui, "‹", "Previous terminal metabolite") {
            action = Some(PanelAction::RotateTerminal(-1));
        }
        let code_w = if terminals.len() > 1 { 120.0 } else { ui.available_width().min(160.0) };
        let galley = one_line(ui, &terminal.label, FontId::new(11.5, FontFamily::Monospace), p.ink, code_w);
        let w = galley.size().x.max(if terminals.len() > 1 { 72.0 } else { 0.0 });
        let (rect, _) = ui.allocate_exact_size(Vec2::new(w + 8.0, 22.0), Sense::hover());
        ui.painter().rect_filled(rect, CornerRadius::same(4), p.panel);
        ui.painter().galley(Pos2::new(rect.center().x - galley.size().x / 2.0, rect.center().y - galley.size().y / 2.0), galley, p.ink);
        if terminals.len() > 1 && arrow(ui, "›", "Next terminal metabolite") {
            action = Some(PanelAction::RotateTerminal(1));
        }
    });
    action
}

/// `.add-reaction-direction-wheel`: the eight buttons on a dashed ring.
fn direction_wheel(ui: &mut Ui, size: f32, compact: bool, enabled: bool, p: &Palette) -> Option<PanelAction> {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let center = rect.center();
    let painter = ui.painter().clone();
    // The dashed ring (`::before`) and the dot in the middle.
    let ring_r = if compact { 40.0 } else { 48.0 };
    let ring: Vec<Pos2> = (0..=72)
        .map(|i| {
            let a = i as f32 / 72.0 * std::f32::consts::TAU;
            center + Vec2::new(a.cos(), a.sin()) * ring_r
        })
        .collect();
    painter.extend(Shape::dashed_line(&ring, Stroke::new(1.0, mix(p.accent, 0.45, p.line)), 4.0, 3.0));
    painter.circle(center, 4.0, mix(p.accent, 0.35, p.panel), Stroke::new(1.0, p.accent));
    let point = if compact { 26.0 } else { 30.0 };
    let font = if compact { 15.0 } else { 18.0 };
    let mut action = None;
    for direction in &REACTION_DIRECTIONS {
        // `left: 50 + dx·35.82 %; top: 50 + dy·35.82 %`.
        let c = center + Vec2::new(direction.dx as f32, direction.dy as f32) * (size * 0.3582);
        let hit = Rect::from_center_size(c, Vec2::splat(point));
        let id = ui.id().with(("add-rxn-direction", direction.id));
        let response = ui.interact(hit, id, if enabled { Sense::click() } else { Sense::hover() });
        let label = direction.aria_label();
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &label));
        let hot = enabled && (response.hovered() || response.has_focus());
        let (fill, ink, line, r) = if hot {
            (p.accent, p.panel, p.accent, point / 2.0 * 1.1)
        } else {
            (p.panel, p.accent, mix(p.accent, 0.65, p.line), point / 2.0)
        };
        let alpha = if enabled { 1.0 } else { 0.28 };
        painter.circle(c, r, fill.gamma_multiply(alpha), Stroke::new(1.0, line.gamma_multiply(alpha)));
        painter.text(c, egui::Align2::CENTER_CENTER, direction.shown_glyph(), FontId::proportional(font * if hot { 1.1 } else { 1.0 }), ink.gamma_multiply(alpha));
        let response = if enabled {
            response.on_hover_text(direction.label).on_hover_cursor(CursorIcon::PointingHand)
        } else {
            response.on_hover_text(direction.label).on_hover_cursor(CursorIcon::NotAllowed)
        };
        if enabled && response.clicked() {
            action = Some(PanelAction::Direction(direction));
        }
    }
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_roomy_stage_changes_nothing() {
        let place = panel_placement(1400.0, 900.0, Some(&PanelPosition { left: 212.0, top: 172.0, moved: false }));
        assert_eq!(place, Placement { width: 380.0, max_height: 520.0, left: 212.0, top: 172.0 });
    }

    #[test]
    fn directions_are_unit_vectors() {
        for d in &REACTION_DIRECTIONS {
            assert!((js::hypot(d.dx, d.dy) - 1.0).abs() < 1e-12, "{}", d.id);
            assert!(!d.shown_glyph().is_empty());
        }
        assert_eq!(direction_by_id("e").map(|d| d.label), Some("East"));
        assert!(direction_by_id("x").is_none());
    }

    #[test]
    fn coefficients_print_as_javascript_does() {
        assert_eq!(format_coefficient(2.0), "2");
        assert_eq!(format_coefficient(0.5), "0.5");
        assert_eq!(format_coefficient(1.23456), "1.235");
        assert_eq!(summarize_side(&[], 3), "∅");
        let terms: Vec<String> = ["a", "b", "c", "d"].iter().map(|s| s.to_string()).collect();
        assert_eq!(summarize_side(&terms, 3), "a + b + c + …");
        assert_eq!(summarize_side(&terms[..2], 3), "a + b");
    }
}
