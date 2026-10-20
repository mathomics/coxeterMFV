//! The map toolbar (`.canvas-toolbar` of `index.html`): the task panel
//! toggle, undo and redo, zoom, the zoom read-out and active tool, the
//! layout presets, the View popover (`renderViewControls` of
//! `src/ui/panels.js`) and the inspector toggle.

use egui::{Frame, Margin, RichText, Stroke, Ui, Vec2};

use crate::app::Shell;
use crate::theme::{Palette, bold};
use crate::widgets::{self, Btn, Look, aria_label};

/// `--toolbar-h`.
pub const HEIGHT: f32 = 44.0;

/// Draw the map toolbar.
pub fn show(ui: &mut Ui, shell: &mut Shell) {
    let p = shell.palette();
    egui::Panel::top("canvas-toolbar")
        .exact_size(HEIGHT)
        .frame(Frame::new().fill(p.panel).inner_margin(Margin::symmetric(10, 5)).stroke(Stroke::new(1.0, p.line)))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                let expanded = shell.ui.task_overlay || (!shell.ui.left_collapsed && !shell.ui.focus_mode);
                let tools = Btn::new("☷ Tools").look(Look::Secondary).aria("Tools").show(ui);
                widgets::aria_pressed(&tools, expanded);
                if tools.clicked() {
                    let narrow = ui.ctx().content_rect().width() <= 1000.0 || ui.ctx().content_rect().height() <= 620.0;
                    shell.toggle_left(narrow);
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
                ui.label(RichText::new(shell.map.tool().label()).size(11.5).color(p.ink_3));
                layout_presets(ui, shell, &p);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let inspector = Btn::new("Inspector").look(Look::Secondary).title("Show or hide selection properties").aria("Toggle inspector").show(ui);
                    if inspector.clicked() {
                        shell.toggle_inspector();
                    }
                    view_menu(ui, shell, &p);
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

/// `.layout-preset-toolbar`: the presets, refused (and said why on the
/// button) while a re-layout is blocked.
fn layout_presets(ui: &mut Ui, shell: &mut Shell, p: &Palette) {
    let blocked = shell.store.relayout_blocked();
    let current = shell.store.layout_settings.preset.clone();
    Frame::new().stroke(Stroke::new(1.0, p.line)).corner_radius(egui::CornerRadius::same(8)).inner_margin(Margin::symmetric(6, 2)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            ui.label(RichText::new("LAYOUT:").size(10.0).family(bold()).color(p.ink_3));
            for (name, label) in [("compact", "Compact"), ("normal", "Normal"), ("spacious", "Spacious")] {
                let on = current == name;
                let mut b = Btn::new(RichText::new(label).size(11.5)).look(if on { Look::Active } else { Look::Flat }).enabled(blocked.is_none()).height(26.0);
                if let Some(why) = &blocked {
                    b = b.title(why);
                }
                let r = b.show(ui);
                widgets::aria_pressed(&r, on);
                if r.clicked() {
                    shell.layout_preset(name);
                }
            }
        });
    });
}

/// `details.canvas-view` and its popover: an area under the button that
/// stays open while its own selects open their lists (a popup would close),
/// and closes on the button, Escape, or a press elsewhere.
fn view_menu(ui: &mut Ui, shell: &mut Shell, p: &Palette) {
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
                render_view_controls(ui, shell);
            });
        });
    let pressed_elsewhere = ctx.input(|i| i.pointer.any_pressed())
        && ctx.input(|i| i.pointer.interact_pos()).is_some_and(|pos| !area.response.rect.contains(pos) && !response.rect.contains(pos))
        && !egui::Popup::is_any_open(&ctx);
    if pressed_elsewhere || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        shell.ui.view_open = false;
    }
}

/// `renderViewControls(store)`: presentation only; nothing here changes a flux.
pub fn render_view_controls(ui: &mut Ui, shell: &mut Shell) {
    let v = shell.store.view.clone();
    widgets::section_label(ui, "Map view");
    let overview = shell.store.core_metabolism_overview_support().is_some_and(|s| s.supported);
    ui.horizontal_wrapped(|ui| {
        let core = Btn::new("Core metabolism")
            .pressed(shell.store.map_mode == "core-overview")
            .enabled(overview)
            .title("Curated glycolysis, PPP, TCA and amino-acid synthesis layout")
            .show(ui);
        if core.clicked() {
            shell.map_core_overview();
        }
        let full = Btn::new("Full network").pressed(shell.store.map_mode == "full").title("Draw every reaction available in the active map model").show(ui);
        if full.clicked() {
            shell.map_full();
        }
    });
    widgets::section_label(ui, "Map visibility");
    ui.horizontal_wrapped(|ui| {
        if widgets::toggle_button(ui, "Cofactors", v.show_currency, "Show secondary metabolites: cofactors such as ATP, NADH and H⁺, and every node set to Secondary").clicked() {
            let _ = shell.store.set_view(coxeter_state::ViewPatch { show_currency: Some(!v.show_currency), ..Default::default() });
        }
        if widgets::toggle_button(ui, "Cofactor labels", v.currency_labels, "Name the small secondary metabolite nodes. They appear once the map is zoomed in past 34%").clicked() {
            let _ = shell.store.set_view(coxeter_state::ViewPatch { currency_labels: Some(!v.currency_labels), ..Default::default() });
        }
        if widgets::toggle_button(ui, "Highlight cross-feeding", v.show_cross_feeding, "Dim the metabolic network and highlight active metabolite transfers between cells").clicked() {
            let _ = shell.store.set_view(coxeter_state::ViewPatch { show_cross_feeding: Some(!v.show_cross_feeding), ..Default::default() });
        }
    });
    crate::panels::cross_feeding::cross_feeding_top_control(ui, shell);
    crate::panels::cross_feeding::cross_feeding_width_control(ui, shell);
    labels_select(ui, shell, "view-labels");
    crate::panels::inspector::map_label_control(ui, shell, "metabolite", None);
    crate::panels::inspector::map_label_control(ui, shell, "reaction", None);
    widgets::hint(ui, "Missing names or database IDs use the usual label. Individual labels can override these choices in the inspector.");
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
    if Btn::new("Colors & pathways ↗").show(ui).clicked() {
        shell.open_workspace_named("map");
        shell.ui.view_open = false;
    }
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
