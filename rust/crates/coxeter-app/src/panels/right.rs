//! The right panel (`aside.inspector-panel` of `index.html`): the Inspector
//! and Layout tabs, the collapse button, and `paintInspector` (the inspector
//! or the layout settings). It closes itself when nothing is selected
//! (`syncInspectorVisibility`) until the user sets it by hand.

use egui::{Frame, Margin, RichText, Stroke, Ui};

use crate::app::Shell;
use crate::panels::left::{COLLAPSED, tab_button};
use crate::theme::Palette;
use crate::ui_state::RightTab;
use crate::widgets::{Btn, Look};

/// `--inspector-w`.
pub const WIDTH: f32 = 286.0;

/// Draw the right panel.
pub fn show(ui: &mut Ui, shell: &mut Shell) {
    let p = shell.palette();
    let collapsed = shell.ui.inspector_collapsed || shell.ui.focus_mode;
    // `@media (max-width: 1080px)`: a narrow window keeps the map its whole
    // width, and the panel opens over it instead of squeezing the map and its
    // toolbar; closed, it is not drawn at all, and the toolbar's Inspector
    // button opens it.
    if ui.ctx().content_rect().width() <= 1080.0 {
        if collapsed {
            return;
        }
        let room = ui.available_rect_before_wrap();
        let width = WIDTH.min(room.width() - 50.0).max(0.0);
        egui::Area::new(egui::Id::new("inspector-panel-over"))
            .order(egui::Order::Middle)
            .fixed_pos(egui::pos2(room.right() - width, room.top()))
            .show(ui.ctx(), |ui| {
                Frame::new().fill(p.panel).stroke(Stroke::new(1.0, p.line)).shadow(ui.style().visuals.popup_shadow).show(ui, |ui| {
                    ui.set_min_size(egui::vec2(width, room.height()));
                    ui.set_max_width(width);
                    contents(ui, shell, false, &p);
                });
            });
        return;
    }
    egui::Panel::right("inspector-panel")
        .exact_size(if collapsed { COLLAPSED } else { WIDTH })
        .resizable(false)
        .frame(Frame::new().fill(p.panel).stroke(Stroke::new(1.0, p.line)))
        .show(ui, |ui| contents(ui, shell, collapsed, &p));
}

/// The tabs with the collapse button, then the inspector or the layout settings.
fn contents(ui: &mut Ui, shell: &mut Shell, collapsed: bool, p: &Palette) {
    Frame::new().fill(p.panel_2).inner_margin(Margin::symmetric(if collapsed { 5 } else { 10 }, 0)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.set_height(48.0);
        ui.horizontal_centered(|ui| {
            if !collapsed {
                for (tab, label) in [(RightTab::Inspector, "Inspector"), (RightTab::Layout, "Layout")] {
                    if tab_button(ui, label, shell.ui.right_tab == tab, p).clicked() {
                        shell.set_right_tab(tab);
                    }
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = if collapsed { "Expand right panel" } else { "Collapse right panel" };
                let r = Btn::new(RichText::new(if collapsed { "‹" } else { "›" }).size(19.0).color(p.ink_3)).look(Look::Flat).title(label).aria(label).show(ui);
                if r.clicked() {
                    shell.toggle_inspector();
                }
            });
        });
    });
    let rect = ui.min_rect();
    ui.painter().hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, p.line));
    if collapsed {
        return;
    }
    egui::ScrollArea::vertical().id_salt("inspector-scroll").auto_shrink([false, false]).show(ui, |ui| {
        Frame::new().inner_margin(Margin { left: 14, right: 14, top: 0, bottom: 30 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            match shell.ui.right_tab {
                RightTab::Layout => crate::panels::inspector::render_layout_settings(ui, shell),
                RightTab::Inspector => crate::panels::inspector::render_inspector(ui, shell),
            }
        });
    });
}
