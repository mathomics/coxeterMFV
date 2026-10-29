//! The map toolbar (`.canvas-toolbar` of `index.html`): the task panel
//! toggle, undo and redo, zoom and its read-out, the display options used
//! most (`renderMapQuickControls` of `src/ui/panels.js`), the Image menu, the
//! View popover (`renderViewControls`) and the inspector toggle.

use coxeter_render::reaction_colors::normalize_reaction_color_mode;
use egui::{Frame, Margin, RichText, Stroke, Ui, Vec2};

use crate::app::Shell;
use crate::theme::Palette;
use crate::widgets::{self, Btn, Look, aria_label};

/// `--toolbar-h`.
pub const HEIGHT: f32 = 44.0;

/// How much of the quick options a toolbar this wide hands over to View
/// (the container queries on `.canvas-toolbar` in `app.css`): line color and
/// width first, then the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickFold {
    None,
    Second,
    All,
}

impl QuickFold {
    pub fn for_width(width: f32) -> Self {
        if width <= 860.0 {
            QuickFold::All
        } else if width <= 1260.0 {
            QuickFold::Second
        } else {
            QuickFold::None
        }
    }
}

/// Draw the map toolbar.
pub fn show(ui: &mut Ui, shell: &mut Shell) {
    let p = shell.palette();
    egui::Panel::top("canvas-toolbar")
        .exact_size(HEIGHT)
        .frame(Frame::new().fill(p.panel).inner_margin(Margin::symmetric(10, 5)).stroke(Stroke::new(1.0, p.line)))
        .show(ui, |ui| {
            let fold = QuickFold::for_width(ui.max_rect().width());
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                let expanded = shell.ui.task_overlay || (!shell.ui.left_collapsed && !shell.ui.focus_mode);
                let tools = icon(ui, "☷", "Tools panel", "Show or hide the tools panel", true, &p);
                widgets::aria_pressed(&tools, expanded);
                if tools.clicked() {
                    shell.toggle_left(crate::panels::left::narrow(ui.ctx()));
                }
                let undo = icon(ui, "↶", "Undo map edit", "Undo map edit (Ctrl+Z)", shell.store.can_undo_map(), &p);
                if undo.clicked() {
                    shell.undo_redo(false);
                }
                let redo = icon(ui, "↷", "Redo map edit", "Redo map edit (Ctrl+Y)", shell.store.can_redo_map(), &p);
                if redo.clicked() {
                    shell.undo_redo(true);
                }
                separator(ui, &p);
                if icon(ui, "−", "Zoom out", "Zoom out", true, &p).clicked() {
                    shell.zoom_by(1.0 / 1.3);
                }
                if icon(ui, "+", "Zoom in", "Zoom in", true, &p).clicked() {
                    shell.zoom_by(1.3);
                }
                if icon(ui, "⤢", "Fit map to screen", "Fit map to screen", true, &p).clicked() {
                    shell.zoom_fit();
                }
                let readout = shell.map.zoom_readout();
                egui::Frame::new().stroke(Stroke::new(1.0, p.line)).corner_radius(egui::CornerRadius::same(7)).inner_margin(Margin::symmetric(8, 3)).show(ui, |ui| {
                    ui.label(RichText::new(readout).size(11.5).color(p.ink_2));
                });
                if fold != QuickFold::All {
                    separator(ui, &p);
                    render_quick_controls(ui, shell, true, fold == QuickFold::None, "toolbar");
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let inspector = Btn::new("Inspector").look(Look::Secondary).title("Show or hide selection properties").aria("Toggle inspector").show(ui);
                    if inspector.clicked() {
                        shell.toggle_inspector();
                    }
                    view_menu(ui, shell, &p, fold);
                    image_menu(ui, shell, &p);
                });
            });
        });
}

/// An icon button of the toolbar.
fn icon(ui: &mut Ui, glyph: &str, aria: &str, title: &str, enabled: bool, p: &Palette) -> egui::Response {
    let hovered = enabled && widgets::hovered_next(ui);
    let r = ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(glyph).size(16.0).color(if hovered { p.ink } else { p.ink_2 }))
            .fill(if hovered { p.hover } else { egui::Color32::TRANSPARENT }).stroke(Stroke::NONE).min_size(Vec2::splat(31.0)),
    );
    aria_label(&r, aria);
    if enabled { r.on_hover_text(title) } else { r.on_disabled_hover_text(title) }
}

fn separator(ui: &mut Ui, p: &Palette) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(9.0, 22.0), egui::Sense::hover());
    ui.painter().vline(rect.center().x, rect.y_range(), Stroke::new(1.0, p.line));
}

/// `renderMapQuickControls(store)`: the display options used most, in two
/// groups, what is labelled and drawn (`quick-a`) and how lines are colored
/// (`quick-b`), so that a narrow toolbar can hand the second and then both
/// over to View. `salt` keeps the selects of the toolbar and of View apart.
pub fn render_quick_controls(ui: &mut Ui, shell: &mut Shell, first: bool, second: bool, salt: &str) {
    let v = shell.store.view.clone();
    let p = shell.palette();
    let patch = |shell: &mut Shell, patch: coxeter_state::ViewPatch| {
        let _ = shell.store.set_view(patch);
    };
    if first {
        let options = [("auto", "Labels: major fluxes"), ("active", "Labels: all active"), ("all", "Labels: all"), ("none", "Labels: none")]
            .map(|(value, label)| (value.to_string(), label.to_string()));
        if let Some(value) = widgets::select(ui, (salt, "labels"), &v.labels, &options, 140.0, true, "Map labels") {
            patch(shell, coxeter_state::ViewPatch { labels: Some(value), ..Default::default() });
        }
        if widgets::toggle_button(ui, "Cofactors", v.show_currency, "Show cofactors such as ATP, NADH and H⁺").clicked() {
            patch(shell, coxeter_state::ViewPatch { show_currency: Some(!v.show_currency), ..Default::default() });
        }
        let multicell = shell.store.community.as_ref().is_some_and(|c| c.active_species().count() > 1);
        if multicell && widgets::toggle_button(ui, "⇄ Cross-feeding", v.show_cross_feeding, "Highlight transfers between cells").clicked() {
            patch(shell, coxeter_state::ViewPatch { show_cross_feeding: Some(!v.show_cross_feeding), ..Default::default() });
        }
    }
    if second {
        if first {
            separator(ui, &p);
        }
        let current = v.reaction_color_mode.as_str().to_string();
        let options = [("flux", "Color: flux"), ("absflux", "Color: absolute flux"), ("pathway", "Color: pathway"), ("subsystem", "Color: subsystem")]
            .map(|(value, label)| (value.to_string(), label.to_string()));
        if let Some(value) = widgets::select(ui, (salt, "line-color"), &current, &options, 120.0, true, "Line color") {
            // A group chosen here is what "Color lines by flux" goes back to.
            let group = (!normalize_reaction_color_mode(&value).is_flux()).then(|| value.clone());
            patch(shell, coxeter_state::ViewPatch { reaction_color_mode: Some(value), line_color_group: group, ..Default::default() });
        }
        if widgets::toggle_button(ui, "Width by flux", v.width_by_flux, "Line width follows flux").clicked() {
            patch(shell, coxeter_state::ViewPatch { width_by_flux: Some(!v.width_by_flux), ..Default::default() });
        }
        if Btn::new("Colors & pathways ↗").show(ui).clicked() {
            shell.open_workspace_named("map");
            shell.ui.view_open = false;
        }
    }
}

/// `details.canvas-export`: the map as a figure, SVG or PNG.
fn image_menu(ui: &mut Ui, shell: &mut Shell, p: &Palette) {
    let hovered = widgets::hovered_next(ui);
    let button = egui::Button::new(RichText::new("⤓ Image ⌄").size(12.5).color(p.ink))
        .fill(if hovered { p.hover } else { p.panel })
        .stroke(Stroke::new(1.0, p.line));
    let (response, _) = egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
        ui.set_min_width(110.0);
        if Btn::new("SVG").look(Look::Flat).aria("Export SVG image").show(ui).clicked() {
            ui.close();
            shell.export_svg();
        }
        if Btn::new("PNG").look(Look::Flat).aria("Export PNG image").show(ui).clicked() {
            ui.close();
            shell.export_png();
        }
    });
    aria_label(&response, "Image menu");
    response.on_hover_text("Export the map as a figure");
}

/// `details.canvas-view` and its popover: an area under the button that
/// stays open while its own selects open their lists (a popup would close),
/// and closes on the button, Escape, or a press elsewhere.
fn view_menu(ui: &mut Ui, shell: &mut Shell, p: &Palette, fold: QuickFold) {
    let hovered = widgets::hovered_next(ui);
    let button = egui::Button::new(RichText::new("View ⌄").size(12.5).color(p.ink))
        .fill(if shell.ui.view_open { p.accent_soft } else if hovered { p.hover } else { p.panel })
        .stroke(Stroke::new(1.0, if shell.ui.view_open { p.accent_line } else { p.line }));
    let response = ui.add(button);
    aria_label(&response, "View menu");
    widgets::aria_pressed(&response, shell.ui.view_open);
    if response.clicked() {
        shell.ui.view_open = !shell.ui.view_open;
        return;
    }
    if !shell.ui.view_open {
        return;
    }
    let ctx = ui.ctx().clone();
    let area = egui::Area::new(egui::Id::new("view-popover"))
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::RIGHT_TOP)
        .fixed_pos(response.rect.right_bottom() + Vec2::new(0.0, 6.0))
        .show(&ctx, |ui| {
            Frame::popup(ui.style()).fill(p.panel).stroke(Stroke::new(1.0, p.line)).inner_margin(Margin::same(12)).show(ui, |ui| {
                ui.set_width(300.0);
                render_view_controls(ui, shell, fold);
            });
        });
    let pressed_elsewhere = ctx.input(|i| i.pointer.any_pressed())
        && ctx.input(|i| i.pointer.interact_pos()).is_some_and(|pos| !area.response.rect.contains(pos) && !response.rect.contains(pos))
        && !egui::Popup::is_any_open(&ctx);
    if pressed_elsewhere || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        shell.ui.view_open = false;
    }
}

/// `renderViewControls(store)`: presentation only; nothing here changes a
/// flux. It starts with the quick options the toolbar had no room for.
pub fn render_view_controls(ui: &mut Ui, shell: &mut Shell, fold: QuickFold) {
    let v = shell.store.view.clone();
    if fold == QuickFold::All {
        ui.horizontal_wrapped(|ui| render_quick_controls(ui, shell, true, false, "view"));
        ui.add_space(6.0);
    }
    if fold != QuickFold::None {
        ui.horizontal_wrapped(|ui| render_quick_controls(ui, shell, false, true, "view"));
        ui.add_space(6.0);
    }
    widgets::section_label(ui, "Map view");
    let overview = shell.store.core_metabolism_overview_support().is_some_and(|s| s.supported);
    ui.horizontal_wrapped(|ui| {
        let core = Btn::new("Core metabolism")
            .pressed(shell.store.map_mode == "core-overview")
            .enabled(overview)
            .title("Glycolysis, PPP, TCA and amino-acid synthesis")
            .show(ui);
        if core.clicked() {
            shell.map_core_overview();
        }
        let full = Btn::new("Full network").pressed(shell.store.map_mode == "full").title("Every reaction in the map's model").show(ui);
        if full.clicked() {
            shell.map_full();
        }
    });
    widgets::section_label(ui, "Cofactors");
    ui.horizontal_wrapped(|ui| {
        if widgets::toggle_button(ui, "Cofactor labels", v.currency_labels, "Shown past 34% zoom").clicked() {
            let _ = shell.store.set_view(coxeter_state::ViewPatch { currency_labels: Some(!v.currency_labels), ..Default::default() });
        }
        let quiet = shell.store.map.as_ref().and_then(|m| m.cofactor_style()) == Some("quiet");
        if widgets::toggle_button(ui, "Quiet cofactors", quiet, "Small and light; saved with the map").clicked() {
            shell.store.set_map_cofactor_style(if quiet { None } else { Some("quiet") });
        }
    });
    crate::panels::cross_feeding::cross_feeding_top_control(ui, shell);
    crate::panels::cross_feeding::cross_feeding_width_control(ui, shell);
    crate::panels::inspector::map_label_control(ui, shell, "metabolite", None);
    crate::panels::inspector::map_label_control(ui, shell, "reaction", None);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Scrolling").size(12.5).color(p_ink2(ui)));
        let options = [
            ("auto".to_string(), "Automatic".to_string()),
            ("mouse".to_string(), "Mouse wheel: zoom".to_string()),
            ("touchpad".to_string(), "Touchpad: pan".to_string()),
        ];
        if let Some(value) = widgets::select(ui, "scroll-input", &v.scroll_input, &options, 170.0, true, "Scrolling input") {
            let _ = shell.store.set_view(coxeter_state::ViewPatch { scroll_input: Some(value), ..Default::default() });
        }
    });
    let hidden = shell.store.hidden_metabolite_count();
    if hidden > 0 && Btn::new(format!("Show hidden metabolites ({hidden})")).show(ui).clicked() {
        shell.close_map_menus();
        shell.store.show_all_hidden_metabolites();
    }
}

fn p_ink2(ui: &Ui) -> egui::Color32 {
    Palette::current(ui.ctx()).ink_2
}

/// The `labels` select (`Major fluxes`, `All active`, `All`, `None`).
pub fn labels_select(ui: &mut Ui, shell: &mut Shell, id: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Labels").size(12.5).color(p_ink2(ui)));
        let options = [
            ("auto".to_string(), "Major fluxes".to_string()),
            ("active".to_string(), "All active".to_string()),
            ("all".to_string(), "All".to_string()),
            ("none".to_string(), "None".to_string()),
        ];
        let current = shell.store.view.labels.clone();
        if let Some(value) = widgets::select(ui, id, &current, &options, 150.0, true, "Map labels") {
            let _ = shell.store.set_view(coxeter_state::ViewPatch { labels: Some(value), ..Default::default() });
        }
    });
}
