//! The map's context menus (port of `src/ui/metabolite-menu.js`,
//! `compartment-menu.js`, `boundary-menu.js` and `metabolite-add-menu.js`):
//! floating panels over the stage, opened by the map widget's
//! `CanvasEvent::OpenMenu`.
//!
//! - the metabolite menu: the Add Reaction panel anchored to one drawn
//!   metabolite. The selected metabolite is the sole reuse exception: the new
//!   reaction starts from that existing node. Every other metabolite is
//!   created as a fresh visual instance by the placement engine.
//! - the compartment menu: the Add Reaction panel anchored to free space in
//!   one compartment. Unlike the metabolite-origin workflow it never passes an
//!   `anchor_node_id`, so every metabolite of the inserted reaction is a fresh
//!   visual instance.
//! - the boundary menu: model reactions crossing a membrane that are not
//!   drawn yet.
//! - Add metabolite: the compartment's metabolites, one placed where the
//!   compartment was clicked.
//!
//! The JavaScript keeps one controller per menu; the press that opens one
//! closes the others (the stage's `pointerup`), so here one menu is open at
//! a time. Each keeps its own state as its controller does (`this.state`).

use coxeter_canvas::{MapMenu, gestures::MenuRequest};
use coxeter_core::boundaries::reactions_crossing;
use coxeter_core::js;
use coxeter_core::mapdata::Point;
use coxeter_map::place_reaction::PlaceOptions;
use coxeter_state::{AvailableMetabolite, Store};
use egui::{Color32, CornerRadius, CursorIcon, FontFamily, FontId, Frame, Margin, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2};

use super::add_reaction::{
    self, PanelAction, PanelArgs, PanelDrag, PanelPosition, PanelUi, ReactionRow, Side, menu_area, menu_empty, menu_head, menu_search, menu_summary,
    one_line, primary_metabolites_for_side,
};
use crate::app::Shell;
use crate::theme::{Palette, bold};

/// `.boundary-menu { width: 330px }`.
const BOUNDARY_MENU_WIDTH: f64 = 330.0;
/// `.boundary-menu { max-height: 330px }`.
const BOUNDARY_MENU_HEIGHT: f32 = 330.0;
/// `.add-reaction-list { max-height: 220px }`.
const LIST_MAX_HEIGHT: f32 = 220.0;

/// The open menu, if any.
#[derive(Debug, Clone, Default)]
pub struct MapMenus {
    /// What the open menu was opened for, and the client point of the press
    /// that opened it (`event.clientX/Y`).
    pub open: Option<(MenuRequest, Point)>,
    /// Its controller's state (`this.state`).
    pub state: Option<MenuState>,
}

/// The state of the open menu's controller.
#[derive(Debug, Clone)]
pub struct MenuState {
    /// What it was opened for and where (the pair in [`MapMenus::open`]).
    pub request: MenuRequest,
    pub at: Point,
    /// Whether `position` and `origin` have been worked out against the
    /// stage (on the first frame the menu is drawn, with the stage's box and
    /// the camera of the press).
    pub placed: bool,
    /// `state.left`, `state.top` (stage coordinates) and `state.moved`.
    pub position: PanelPosition,
    /// `state.origin` / `state.anchor`: the map point of the press (all but
    /// the metabolite menu, which starts from its node).
    pub origin: Option<Point>,
    /// `state.label`: the compartment's name.
    pub label: String,
    /// `state.selectedReactionId`.
    pub selected_reaction_id: Option<String>,
    /// `state.terminalMetaboliteId`.
    pub terminal_metabolite_id: Option<String>,
    /// What is typed in the filter.
    pub filter: String,
    /// A drag of the panel by its header.
    pub drag: Option<PanelDrag>,
    /// What the menu lists, as of a store revision.
    cache: Option<(u64, Listing)>,
}

#[derive(Debug, Clone)]
enum Listing {
    Reactions(Vec<String>, Vec<ReactionRow>),
    Metabolites(Vec<AvailableMetabolite>),
}

impl MenuState {
    fn new(request: MenuRequest, at: Point, label: String, selected_reaction_id: Option<String>) -> MenuState {
        MenuState {
            request,
            at,
            placed: false,
            position: PanelPosition::default(),
            origin: None,
            label,
            selected_reaction_id,
            terminal_metabolite_id: None,
            filter: String::new(),
            drag: None,
            cache: None,
        }
    }

    /// The state `<controller>.open(…, event)` starts with, if the map has
    /// what the menu is for.
    fn opened(store: &Store, request: &MenuRequest, at: Point) -> Option<MenuState> {
        Some(match request {
            MenuRequest::Metabolite { node_id } => {
                // `MetaboliteMenuController.open(nodeId, event)`.
                store.map.as_ref()?.node(node_id)?;
                let reactions = store.reactions_available_from_metabolite(node_id);
                let selected = (reactions.len() == 1).then(|| reactions[0].clone());
                MenuState::new(request.clone(), at, String::new(), selected)
            }
            MenuRequest::Compartment { cell_id, compartment_id } => {
                // `CompartmentMenuController.open(cellId, compartmentId, event)`.
                store.community.as_ref()?;
                let label = compartment_label(store, cell_id, compartment_id)?;
                let reactions = store.reactions_available_in_compartment(cell_id, compartment_id);
                let selected = (reactions.len() == 1).then(|| reactions[0].clone());
                MenuState::new(request.clone(), at, label, selected)
            }
            MenuRequest::Boundary { boundary_id } => {
                // `BoundaryMenuController.open(boundaryId, event)`.
                if boundary_id.is_empty() {
                    return None;
                }
                store.community.as_ref()?;
                store.map.as_ref()?.boundary(boundary_id)?;
                MenuState::new(request.clone(), at, String::new(), None)
            }
            MenuRequest::MetaboliteAdd { cell_id, compartment_id } => {
                // `MetaboliteAddMenuController.open(cellId, compartmentId, event)`.
                let label = compartment_label(store, cell_id, compartment_id)?;
                MenuState::new(request.clone(), at, label, None)
            }
        })
    }

    /// Which of the four menus this is.
    pub fn menu(&self) -> MapMenu {
        self.request.menu()
    }
}

impl MapMenus {
    /// `<controller>.open(…, event)`. One menu is open at a time: opening
    /// one replaces the one open.
    pub fn open(&mut self, store: &Store, request: MenuRequest, at: Point) {
        if let MenuRequest::Boundary { boundary_id } = &request {
            // The membrane's menu is already open: it stays where it is.
            let same = matches!(&self.open, Some((MenuRequest::Boundary { boundary_id: b }, _)) if b == boundary_id);
            if same {
                return;
            }
            // A full map often has every transporter already drawn. In that
            // case the membrane still highlights on hover, but no empty menu
            // obscures the map.
            if available_for_boundary(store, boundary_id).is_empty() {
                return;
            }
        }
        let Some(state) = MenuState::opened(store, &request, at) else { return };
        self.open = Some((request, at));
        self.state = Some(state);
    }

    /// `<controller>.close()`.
    pub fn close(&mut self, menu: MapMenu) {
        if self.current() == Some(menu) {
            self.close_all();
        }
    }

    /// `closeMapMenus()`.
    pub fn close_all(&mut self) {
        self.open = None;
        self.state = None;
    }

    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// The menu that is open.
    pub fn current(&self) -> Option<MapMenu> {
        self.open.as_ref().map(|(r, _)| r.menu())
    }
}

/// `compartment.label || compartmentId`, if the map has that compartment.
fn compartment_label(store: &Store, cell_id: &str, compartment_id: &str) -> Option<String> {
    let compartment = store.map.as_ref()?.cell(cell_id)?.compartments.iter().find(|c| c.id == compartment_id)?;
    Some(if compartment.label.is_empty() { compartment_id.to_string() } else { compartment.label.clone() })
}

/// `BoundaryMenuController.availableFor(boundaryId)`: the model's reactions
/// crossing a membrane that are not drawn yet.
pub fn available_for_boundary(store: &Store, boundary_id: &str) -> Vec<String> {
    let (Some(community), Some(map)) = (store.community.as_ref(), store.map.as_ref()) else { return Vec::new() };
    if boundary_id.is_empty() {
        return Vec::new();
    }
    reactions_crossing(community, boundary_id).into_iter().filter(|r| !map.has_reaction(r)).collect()
}

// Drawing

/// What a menu asked for this frame, carried out once it is drawn.
enum Outcome {
    Keep,
    Close,
    /// `store.addMapReaction(reactionId, opts)`, the menu closed first.
    AddReaction(String, PlaceOptions),
    /// `store.addMapMetabolite(id, {x, y})`.
    AddMetabolite(String, Point),
}

/// Draw the open menu over the map (`<controller>.render()`), and do what
/// it was clicked for (`handleClick`, `handleFilterInput`).
pub fn show(ctx: &egui::Context, shell: &mut Shell) {
    let Some((request, at)) = shell.menus.open.clone() else {
        shell.menus.state = None;
        return;
    };
    // The menus live on the stage: under a workspace page there is none.
    if shell.ui.workspace.is_some() || shell.ui.results_maximized {
        return;
    }
    let Some(stage) = shell.map_rect.filter(|r| r.width() > 0.0 && r.height() > 0.0) else { return };
    // The controller's state, or a fresh one if the open menu was set by hand.
    let state = shell.menus.state.take().filter(|s| s.request == request && s.at == at);
    let Some(mut menu) = state.or_else(|| MenuState::opened(&shell.store, &request, at)) else {
        shell.menus.close_all();
        return;
    };
    if !menu.placed {
        // `left: event.clientX - rect.left + 12`, `origin: screenToMap(…)`.
        menu.position = PanelPosition { left: menu.at.x - stage.min.x as f64 + 12.0, top: menu.at.y - stage.min.y as f64 + 12.0, moved: false };
        if !matches!(menu.request, MenuRequest::Metabolite { .. }) {
            menu.origin = Some(shell.map.canvas.screen_to_map(menu.at));
        }
        menu.placed = true;
    }
    let revision = shell.revision;
    let outcome = match menu.request.clone() {
        MenuRequest::Metabolite { node_id } => metabolite_menu(ctx, &shell.store, &mut menu, stage, revision, &node_id),
        MenuRequest::Compartment { cell_id, compartment_id } => compartment_menu(ctx, &shell.store, &mut menu, stage, revision, &cell_id, &compartment_id),
        MenuRequest::Boundary { boundary_id } => boundary_menu(ctx, &shell.store, &mut menu, stage, revision, &boundary_id),
        MenuRequest::MetaboliteAdd { cell_id, compartment_id } => metabolite_add_menu(ctx, &shell.store, &mut menu, stage, revision, &cell_id, &compartment_id),
    };
    match outcome {
        Outcome::Keep => shell.menus.state = Some(menu),
        Outcome::Close => shell.menus.close_all(),
        Outcome::AddReaction(reaction_id, opts) => {
            // `this.state = null; store.addMapReaction(…)`.
            shell.menus.close_all();
            let _ = shell.store.add_map_reaction(&reaction_id, &opts);
        }
        Outcome::AddMetabolite(id, at) => {
            shell.menus.close_all();
            let _ = shell.store.add_map_metabolite(&id, at.x, at.y);
        }
    }
}

/// The reactions an Add Reaction panel lists, worked out again when the
/// store has changed.
fn reaction_listing<'m>(menu: &'m mut MenuState, store: &Store, revision: u64, list: impl FnOnce() -> Vec<String>) -> (&'m [String], &'m [ReactionRow]) {
    let fresh = matches!(&menu.cache, Some((r, Listing::Reactions(..))) if *r == revision);
    if !fresh {
        let ids = list();
        let rows = add_reaction::reaction_rows(store, &ids);
        menu.cache = Some((revision, Listing::Reactions(ids, rows)));
    }
    match &menu.cache {
        Some((_, Listing::Reactions(ids, rows))) => (ids, rows),
        _ => (&[], &[]),
    }
}

/// `if (selectedReactionId && !reactions.includes(selectedReactionId))`:
/// a reaction that is no longer offered gives way to the only one, or none.
fn revalidate_selection(menu: &mut MenuState, reactions: &[String]) {
    if let Some(selected) = &menu.selected_reaction_id {
        if !reactions.contains(selected) {
            menu.selected_reaction_id = (reactions.len() == 1).then(|| reactions[0].clone());
        }
    }
}

/// The Add Reaction panel of the metabolite and compartment menus, and what
/// its clicks do to the menu's state. Returns the direction committed.
fn add_reaction_panel(
    ctx: &egui::Context,
    store: &Store,
    menu: &mut MenuState,
    stage: Rect,
    revision: u64,
    subtitle: &str,
    list: impl FnOnce() -> Vec<String>,
) -> Result<Option<&'static add_reaction::Direction>, ()> {
    if store.community.is_none() {
        return Ok(None);
    }
    let (ids, rows) = reaction_listing(menu, store, revision, list);
    let ids = ids.to_vec();
    let rows = rows.to_vec();
    revalidate_selection(menu, &ids);
    let selected = menu.selected_reaction_id.clone().filter(|s| ids.contains(s));
    let terminals = selected.as_deref().map_or_else(Vec::new, |s| primary_metabolites_for_side(store, s, Side::Product));
    let id = egui::Id::new(("add-reaction-menu", format!("{:?}", menu.menu())));
    let action = add_reaction::show_panel(
        ctx,
        PanelArgs {
            id,
            stage,
            rows: &rows,
            selected_reaction_id: selected.as_deref(),
            title: "Add reaction",
            subtitle,
            terminals: &terminals,
            terminal_metabolite_id: menu.terminal_metabolite_id.as_deref(),
        },
        PanelUi { position: &mut menu.position, drag: &mut menu.drag, filter: &mut menu.filter },
    );
    match action {
        None => Ok(None),
        Some(PanelAction::Close) => Err(()),
        Some(PanelAction::Select(reaction_id)) => {
            menu.selected_reaction_id = Some(reaction_id);
            menu.terminal_metabolite_id = None;
            Ok(None)
        }
        Some(PanelAction::RotateTerminal(step)) => {
            if let Some(selected) = &menu.selected_reaction_id {
                let terminals = primary_metabolites_for_side(store, selected, Side::Product);
                if !terminals.is_empty() {
                    let n = terminals.len() as i64;
                    let current = terminals.iter().position(|t| Some(&t.id) == menu.terminal_metabolite_id.as_ref());
                    let start = current.unwrap_or(0) as i64;
                    let step = if step < 0 { -1 } else { 1 };
                    let next = ((start + step + n) % n) as usize;
                    menu.terminal_metabolite_id = Some(terminals[next].id.clone());
                }
            }
            Ok(None)
        }
        Some(PanelAction::Direction(direction)) => Ok(menu.selected_reaction_id.is_some().then_some(direction)),
    }
}

/// `MetaboliteMenuController`: `render()` and `handleClick`.
fn metabolite_menu(ctx: &egui::Context, store: &Store, menu: &mut MenuState, stage: Rect, revision: u64, node_id: &str) -> Outcome {
    if store.community.is_none() {
        return Outcome::Keep;
    }
    let Some(node) = store.map.as_ref().and_then(|m| m.node(node_id)).cloned() else { return Outcome::Keep };
    // Named as the map names it.
    let label = Some(store.map_label_text("metabolite", &node.id, None))
        .filter(|l| !l.is_empty())
        .or_else(|| node.base.clone().filter(|l| !l.is_empty()))
        .or_else(|| node.local_id.clone().filter(|l| !l.is_empty()))
        .or_else(|| node.merged_id.clone().filter(|l| !l.is_empty()))
        .unwrap_or_else(|| "Selected metabolite".into());
    let subtitle = format!("From {label} · selected metabolite will be reused");
    match add_reaction_panel(ctx, store, menu, stage, revision, &subtitle, || store.reactions_available_from_metabolite(node_id)) {
        Err(()) => Outcome::Close,
        Ok(None) => Outcome::Keep,
        Ok(Some(direction)) => {
            let Some(reaction_id) = menu.selected_reaction_id.clone() else { return Outcome::Keep };
            let opts = PlaceOptions {
                anchor: Some(direction.anchor_from(Point::new(node.x, node.y))),
                anchor_node_id: Some(node.id.clone()),
                placement_direction: Some(Point::new(direction.dx, direction.dy)),
                terminal_metabolite_id: menu.terminal_metabolite_id.clone(),
                ..Default::default()
            };
            Outcome::AddReaction(reaction_id, opts)
        }
    }
}

/// `CompartmentMenuController`: `render()` and `handleClick`.
fn compartment_menu(ctx: &egui::Context, store: &Store, menu: &mut MenuState, stage: Rect, revision: u64, cell_id: &str, compartment_id: &str) -> Outcome {
    let subtitle = format!("{} · all metabolites will be new visual instances", menu.label);
    match add_reaction_panel(ctx, store, menu, stage, revision, &subtitle, || store.reactions_available_in_compartment(cell_id, compartment_id)) {
        Err(()) => Outcome::Close,
        Ok(None) => Outcome::Keep,
        Ok(Some(direction)) => {
            let (Some(reaction_id), Some(origin)) = (menu.selected_reaction_id.clone(), menu.origin) else { return Outcome::Keep };
            // Intentionally no anchor_node_id: compartment-origin insertion
            // duplicates every metabolite even if matching nodes already exist
            // elsewhere.
            let opts = PlaceOptions {
                anchor: Some(direction.anchor_from(origin)),
                placement_direction: Some(Point::new(direction.dx, direction.dy)),
                terminal_metabolite_id: menu.terminal_metabolite_id.clone(),
                ..Default::default()
            };
            Outcome::AddReaction(reaction_id, opts)
        }
    }
}

/// `BoundaryMenuController`: `render()` and `handleClick`.
fn boundary_menu(ctx: &egui::Context, store: &Store, menu: &mut MenuState, stage: Rect, revision: u64, boundary_id: &str) -> Outcome {
    let (Some(map), Some(_)) = (store.map.as_ref(), store.community.as_ref()) else { return Outcome::Keep };
    let Some(boundary) = map.boundary(boundary_id) else { return Outcome::Keep };
    let (ids, _) = reaction_listing(menu, store, revision, || available_for_boundary(store, boundary_id));
    let ids = ids.to_vec();
    let p = Palette::current(ctx);
    let (stage_w, stage_h) = (stage.width() as f64, stage.height() as f64);
    let left = js::max(8.0, js::min(menu.position.left, stage_w - BOUNDARY_MENU_WIDTH - 8.0));
    let top = js::max(8.0, js::min(menu.position.top, stage_h - 300.0));
    let title = [boundary.label.as_str(), boundary.inside.as_str()].into_iter().find(|s| !s.is_empty()).unwrap_or("Membrana").to_string();
    let subtitle = format!("{} ↔ {}", boundary.outside.as_deref().unwrap_or("exterior"), boundary.inside);
    let at = stage.min + Vec2::new(left as f32, top as f32);
    let mut outcome = Outcome::Keep;
    menu_area(ctx, egui::Id::new("boundary-menu"), stage, at, BOUNDARY_MENU_WIDTH as f32, |ui| {
        let top_y = ui.cursor().top();
        let (_, closed, _) = menu_head(ui, &title, &subtitle, "Close boundary menu", Some("Close"));
        if closed {
            outcome = Outcome::Close;
        }
        menu_summary(ui, &add_reaction::available_summary(ids.len()));
        let used = ui.cursor().top() - top_y;
        if ids.is_empty() {
            menu_empty(ui, "No undrawn transport reactions are left on this membrane.");
            return;
        }
        Frame::new().inner_margin(Margin::same(8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let max = (BOUNDARY_MENU_HEIGHT - used - 16.0 - 2.0).max(40.0);
            egui::ScrollArea::vertical().id_salt("boundary-menu-list").max_height(max).auto_shrink([false, true]).show(ui, |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(0.0, 5.0);
                for reaction_id in &ids {
                    let (label, detail) = add_reaction::reaction_listing(store, reaction_id);
                    if boundary_row(ui, &label, &detail, &p).clicked() {
                        if let Some(anchor) = menu.origin {
                            // `addMapReaction(reactionId, { boundaryId, anchor })`.
                            let opts = PlaceOptions { boundary_id: Some(boundary_id.to_string()), anchor: Some(anchor), ..Default::default() };
                            outcome = Outcome::AddReaction(reaction_id.clone(), opts);
                        }
                    }
                }
            });
        });
    });
    outcome
}

/// `MetaboliteAddMenuController`: `render()`, `input` and `click`.
fn metabolite_add_menu(ctx: &egui::Context, store: &Store, menu: &mut MenuState, stage: Rect, revision: u64, cell_id: &str, compartment_id: &str) -> Outcome {
    let fresh = matches!(&menu.cache, Some((r, Listing::Metabolites(_))) if *r == revision);
    if !fresh {
        menu.cache = Some((revision, Listing::Metabolites(store.metabolites_available_in_compartment(cell_id, compartment_id))));
    }
    let metabolites = match &menu.cache {
        Some((_, Listing::Metabolites(m))) => m.clone(),
        _ => Vec::new(),
    };
    let p = Palette::current(ctx);
    let (stage_w, stage_h) = (stage.width() as f64, stage.height() as f64);
    let left = js::max(8.0, js::min(menu.position.left, stage_w - 388.0));
    let top = js::max(8.0, js::min(menu.position.top, stage_h - 500.0));
    let at = stage.min + Vec2::new(left as f32, top as f32);
    let subtitle = format!("{} · new visual instance", menu.label);
    let mut outcome = Outcome::Keep;
    let filter_id = egui::Id::new("add-metabolite-filter");
    menu_area(ctx, egui::Id::new("add-metabolite-menu"), stage, at, add_reaction::ADD_REACTION_PANEL_WIDTH as f32, |ui| {
        let (_, closed, _) = menu_head(ui, "Add metabolite", &subtitle, "Close Add metabolite panel", None);
        if closed {
            outcome = Outcome::Close;
        }
        if let Some(text) = menu_search(ui, filter_id, &menu.filter, "Search metabolites…", "Search metabolites") {
            menu.filter = text;
        }
        let q = coxeter_state::util::js_trim(&menu.filter).to_lowercase();
        Frame::new().inner_margin(Margin::same(8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            if metabolites.is_empty() {
                ui.set_min_height(58.0 - 16.0);
                ui.add(egui::Label::new(egui::RichText::new("No metabolites are available in this compartment.").size(12.0).color(p.ink_3)).wrap());
                return;
            }
            egui::ScrollArea::vertical().id_salt("add-metabolite-list").max_height(LIST_MAX_HEIGHT - 16.0).auto_shrink([false, true]).show(ui, |ui| {
                ui.set_min_height(58.0 - 16.0);
                ui.spacing_mut().item_spacing = Vec2::new(0.0, 6.0);
                for m in &metabolites {
                    let search = format!("{} {} {}", m.label, m.name, m.model_id).to_lowercase();
                    if !q.is_empty() && !search.contains(&q) {
                        continue;
                    }
                    if metabolite_row(ui, m, &p).clicked() {
                        if let Some(origin) = menu.origin {
                            outcome = Outcome::AddMetabolite(m.id.clone(), origin);
                        }
                    }
                }
            });
        });
    });
    outcome
}

/// `.boundary-reaction`: identifier and name in a row.
fn boundary_row(ui: &mut Ui, local_id: &str, name: &str, p: &Palette) -> Response {
    let width = ui.available_width();
    let pad = Vec2::new(9.0, 7.0);
    let inner_w = (width - 2.0 * pad.x - 2.0).max(10.0);
    let id_galley = one_line(ui, local_id, FontId::new(11.5, FontFamily::Monospace), p.ink, inner_w);
    let id_col = id_galley.size().x.max(90.0).min(inner_w);
    let name_galley = one_line(ui, name, FontId::proportional(11.5), p.ink_2, (inner_w - id_col - 8.0).max(1.0));
    let line_h = id_galley.size().y.max(name_galley.size().y);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, line_h + pad.y * 2.0 + 2.0), Sense::click());
    let label = [local_id, name].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        let painter = ui.painter();
        painter.rect(rect, CornerRadius::same(8), if hovered { p.hover } else { p.panel }, Stroke::new(1.0, if hovered { p.accent } else { p.line }), StrokeKind::Inside);
        let y = rect.center().y;
        let x = rect.min.x + pad.x + 1.0;
        painter.galley(Pos2::new(x, y - id_galley.size().y / 2.0), id_galley, p.ink);
        painter.galley(Pos2::new(x + id_col + 8.0, y - name_galley.size().y / 2.0), name_galley, p.ink_2);
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// `.add-metabolite-choice`: label over name, how often it is drawn beside.
fn metabolite_row(ui: &mut Ui, m: &AvailableMetabolite, p: &Palette) -> Response {
    let width = ui.available_width();
    let pad = 8.0;
    let count = if m.count > 0 { format!("on map ({})", m.count) } else { String::new() };
    let em = one_line(ui, &count, FontId::proportional(10.0), p.ink_3, width);
    let text_w = (width - 2.0 * pad - em.size().x - 8.0 - 2.0).max(10.0);
    let label_galley = one_line(ui, &m.label, FontId::new(12.5, bold()), p.ink, text_w);
    let name_galley = one_line(ui, &m.name, FontId::proportional(10.0), p.ink_3, text_w);
    let text_h = label_galley.size().y + name_galley.size().y;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, text_h + 2.0 * pad + 2.0), Sense::click());
    let label = [m.label.as_str(), m.name.as_str(), count.as_str()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        let painter = ui.painter();
        let line = if hovered { p.accent } else { Color32::TRANSPARENT };
        painter.rect(rect, CornerRadius::same(8), if hovered { p.hover } else { Color32::TRANSPARENT }, Stroke::new(1.0, line), StrokeKind::Inside);
        let x = rect.min.x + pad + 1.0;
        let top = rect.center().y - text_h / 2.0;
        let label_h = label_galley.size().y;
        painter.galley(Pos2::new(x, top), label_galley, p.ink);
        painter.galley(Pos2::new(x, top + label_h), name_galley, p.ink_3);
        painter.galley(Pos2::new(rect.max.x - pad - 1.0 - em.size().x, rect.center().y - em.size().y / 2.0), em, p.ink_3);
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_a_menu_that_is_not_open_leaves_the_open_one() {
        let at = Point::new(0.0, 0.0);
        let request = MenuRequest::Boundary { boundary_id: "b".into() };
        let mut menus = MapMenus { open: Some((request.clone(), at)), state: Some(MenuState::new(request, at, String::new(), None)) };
        menus.close(MapMenu::Metabolite);
        assert_eq!(menus.current(), Some(MapMenu::Boundary));
        menus.close(MapMenu::Boundary);
        assert!(!menus.is_open());
        assert!(menus.state.is_none());
    }
}
