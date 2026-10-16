//! The workspace editor (`section.workspace-editor` of `index.html`): one
//! full editor page over the map, `renderWorkspace` and `getWorkspaceTitle`
//! of `src/ui/panels.js`, and its Return to map button (`close-workspace`).

use egui::{CornerRadius, Frame, Margin, RichText, Stroke, Ui};

use crate::app::Shell;
use crate::panels::{CardFn, cross_feeding, map_options, model, scenario};
use crate::theme::bold;
use crate::ui_state::Workspace;
use crate::widgets::Btn;

/// `renderWorkspace(store, workspace)`: the cards of a page, in order.
pub fn cards(ws: Workspace) -> Vec<CardFn> {
    match ws {
        Workspace::Library => vec![map_options::reaction_search_card],
        Workspace::Structure => vec![map_options::map_structure_card, map_options::map_canvas_card],
        Workspace::Map => vec![
            map_options::selection_card,
            map_options::map_appearance_card,
            map_options::pathway_manager_card,
            map_options::map_canvas_card,
            map_options::legend_card,
        ],
        Workspace::Scenario | Workspace::Analysis | Workspace::Settings => vec![scenario::scenario_card],
        Workspace::Community => vec![model::species_card],
        Workspace::Medium => vec![model::medium_card],
        Workspace::Model => vec![model::model_editor_card, model::model_overview_card],
        Workspace::Omics => vec![model::omics_overlay_card, model::community_omics_card],
        Workspace::Crossfeeding => vec![cross_feeding::cross_feeding_card],
        Workspace::Overview => vec![model::model_overview_card],
    }
}

/// The page, laid over the stage and the results dock.
pub fn show(ui: &mut Ui, shell: &mut Shell, ws: Workspace) {
    let p = shell.palette();
    egui::CentralPanel::no_frame().show(ui, |ui| {
        ui.painter().rect_filled(ui.max_rect(), CornerRadius::ZERO, p.bg);
        Frame::new()
            .fill(p.panel)
            .stroke(Stroke::new(1.0, p.line))
            .corner_radius(CornerRadius::same(14))
            .outer_margin(Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.set_height(ui.available_height());
                Frame::new().inner_margin(Margin::symmetric(20, 13)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 3.0;
                            ui.label(RichText::new("WORKSPACE").size(10.0).family(bold()).color(p.ink_3).extra_letter_spacing(0.5));
                            ui.label(RichText::new(ws.title()).size(18.0).family(bold()).color(p.ink));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if Btn::new("← Return to map").height(34.0).show(ui).clicked() {
                                shell.close_workspace();
                            }
                        });
                    });
                });
                let rect = ui.min_rect();
                ui.painter().hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, p.line));
                Frame::new().fill(p.panel_2).corner_radius(CornerRadius { nw: 0, ne: 0, sw: 14, se: 14 }).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.set_height(ui.available_height());
                    egui::ScrollArea::vertical().id_salt(("workspace", ws.as_str())).auto_shrink([false, false]).show(ui, |ui| {
                        Frame::new().inner_margin(Margin::same(20)).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            card_grid(ui, shell, &cards(ws));
                        });
                    });
                });
            });
    });
}

/// `.workspace-cards`: columns of at least 340 px, cards dealt across them
/// in order.
fn card_grid(ui: &mut Ui, shell: &mut Shell, cards: &[CardFn]) {
    let width = ui.available_width();
    let gap = 16.0;
    let columns = (((width + gap) / (340.0 + gap)).floor() as usize).clamp(1, cards.len().max(1));
    if columns == 1 {
        for card in cards {
            card(ui, shell);
            ui.add_space(gap);
        }
        return;
    }
    let column_width = (width - gap * (columns as f32 - 1.0)) / columns as f32;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for c in 0..columns {
            ui.allocate_ui_with_layout(egui::vec2(column_width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(column_width);
                for card in cards.iter().skip(c).step_by(columns) {
                    card(ui, shell);
                    ui.add_space(gap);
                }
            });
        }
    });
}
