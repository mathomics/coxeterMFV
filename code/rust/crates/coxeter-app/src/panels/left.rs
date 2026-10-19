//! The left panel (`aside.left-panel` of `index.html`): the Edit Map / Edit
//! Model / Simulate tabs, the tool grid (`.toolbox`), the task hub of the
//! tab (`renderTaskPanel` of `src/ui/panels.js`) and the panel footer.

use coxeter_canvas::Tool;
use coxeter_render::format_flux;
use coxeter_state::store::objectives;
use egui::{CornerRadius, Frame, Margin, RichText, Stroke, Ui, Vec2};

use crate::app::Shell;
use crate::theme::{Palette, bold};
use crate::ui_state::LeftTab;
use crate::widgets::{self, Btn, Look, aria_label};

/// `--left-w`.
pub const WIDTH: f32 = 232.0;
/// A collapsed side panel.
pub const COLLAPSED: f32 = 42.0;

/// The tools of the grid: tool, icon, label, `title`.
pub const TOOLS: [(Tool, &str, &str, &str); 8] = [
    (
        Tool::Select,
        crate::fonts::glyph::SELECT,
        "Select",
        "Select and move: drag a reaction or metabolite to move it, the background to sweep a selection. Click a compartment's membrane to select it, then drag its centre handle to move it or a corner to resize it",
    ),
    (Tool::Pan, "✥", "Pan Mode", "Pan Mode: drag anywhere to move the map. Nothing can be selected until another tool is chosen"),
    (Tool::Edit, "⌘", "Edit Control Points", "Show and move reaction control points to adjust reaction paths"),
    (Tool::Reaction, "⌁", "Add Reaction", "Add Reaction mode: click a metabolite to list addable reactions"),
    (Tool::Metabolite, "○", "Add Metabolite", ""),
    (Tool::Text, "T", "Text", ""),
    (Tool::Compartment, "▣", "Add Compartment", "Add an existing model compartment to the map"),
    (Tool::Cell, "⬡", "Add Cell", "Add an existing model species as a cell on the map"),
];

/// Draw the left panel.
pub fn show(ui: &mut Ui, shell: &mut Shell) {
    let p = shell.palette();
    let collapsed = (shell.ui.left_collapsed || shell.ui.focus_mode) && !shell.ui.task_overlay;
    let width = if collapsed { COLLAPSED } else { WIDTH };
    egui::Panel::left("left-panel")
        .exact_size(width)
        .resizable(false)
        .frame(Frame::new().fill(p.panel).stroke(Stroke::new(1.0, p.line)))
        .show(ui, |ui| {
            tabs(ui, shell, collapsed, &p);
            if collapsed {
                return;
            }
            egui::Panel::bottom("left-footer")
                .frame(Frame::new().fill(p.panel).inner_margin(Margin::symmetric(12, 8)).stroke(Stroke::new(1.0, p.line)))
                .show(ui, |ui| footer(ui, shell, &p));
            egui::ScrollArea::vertical().id_salt("left-scroll").show(ui, |ui| {
                if shell.ui.left_tab == LeftTab::Build {
                    toolbox(ui, shell, &p);
                }
                Frame::new().inner_margin(Margin::same(12)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    render_task_panel(ui, shell, &p);
                });
            });
        });
}

/// `.panel-tabs`.
fn tabs(ui: &mut Ui, shell: &mut Shell, collapsed: bool, p: &Palette) {
    Frame::new().fill(p.panel_2).inner_margin(Margin::symmetric(if collapsed { 5 } else { 10 }, 0)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.set_height(48.0);
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if !collapsed {
                for tab in [LeftTab::Build, LeftTab::Model, LeftTab::Analysis] {
                    let active = shell.ui.left_tab == tab;
                    let r = tab_button(ui, tab.label(), active, p);
                    if r.clicked() {
                        shell.set_left_tab(tab);
                    }
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = if collapsed { "Expand task panel" } else { "Collapse task panel" };
                let r = Btn::new(RichText::new(if collapsed { "›" } else { "‹" }).size(19.0).color(p.ink_3))
                    .look(Look::Flat)
                    .title(label)
                    .aria(label)
                    .show(ui);
                if r.clicked() {
                    if shell.ui.focus_mode {
                        shell.toggle_focus();
                    } else {
                        shell.toggle_left(false);
                    }
                }
            });
        });
    });
    let rect = ui.min_rect();
    ui.painter().hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, p.line));
}

/// A tab of `.panel-tabs`: underlined when active.
pub fn tab_button(ui: &mut Ui, label: &str, active: bool, p: &Palette) -> egui::Response {
    let hovered = crate::widgets::hovered_next(ui);
    let color = if active { p.accent } else if hovered { p.ink } else { p.ink_2 };
    let r = ui.add(
        egui::Button::new(RichText::new(label).size(12.5).family(if active { bold() } else { egui::FontFamily::Proportional }).color(color))
            .fill(if hovered && !active { p.hover } else { egui::Color32::TRANSPARENT })
            .stroke(Stroke::NONE)
            .min_size(Vec2::new(0.0, 46.0)),
    );
    if active {
        let rect = r.rect;
        ui.painter().hline(rect.x_range(), rect.bottom() - 1.0, Stroke::new(2.0, p.accent));
    }
    widgets::aria_pressed(&r, active);
    r
}

/// `.toolbox`: the editing tools, two to a row.
fn toolbox(ui: &mut Ui, shell: &mut Shell, p: &Palette) {
    Frame::new().inner_margin(Margin::same(12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::section_label(ui, "Editing tools");
        let current = shell.map.tool();
        let w = (ui.available_width() - 5.0) / 2.0;
        for pair in TOOLS.chunks(2) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                for (tool, icon, label, title) in pair {
                    if tool_button(ui, icon, label, title, current == *tool, w, p).clicked() {
                        shell.activate_tool(*tool);
                    }
                }
            });
            ui.add_space(-1.0);
        }
    });
    let rect = ui.min_rect();
    ui.painter().hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, p.line));
}

/// `.tool-button`: an icon and a label that wraps onto two lines.
fn tool_button(ui: &mut Ui, icon: &str, label: &str, title: &str, active: bool, width: f32, p: &Palette) -> egui::Response {
    let (fill, line, ink) = if active { (p.accent_soft, p.accent_line, p.accent) } else { (p.panel, p.line, p.ink) };
    let (rect, r) = ui.allocate_exact_size(Vec2::new(width, 40.0), egui::Sense::click());
    r.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, active, label));
    let hovered = r.hovered() && !active;
    let painter = ui.painter();
    painter.rect(rect, CornerRadius::same(8), if hovered { p.hover } else { fill }, Stroke::new(1.0, if hovered { p.ink_3 } else { line }), egui::StrokeKind::Inside);
    let icon_pos = egui::pos2(rect.min.x + 14.0, rect.center().y);
    painter.text(icon_pos, egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(15.0), if active { p.accent } else { p.ink_2 });
    let font = egui::FontId::new(11.5, if active { bold() } else { egui::FontFamily::Proportional });
    let galley = painter.layout(label.to_string(), font, ink, width - 32.0);
    let text_pos = egui::pos2(rect.min.x + 27.0, rect.center().y - galley.size().y / 2.0);
    painter.galley(text_pos, galley, ink);
    aria_label(&r, label);
    widgets::aria_pressed(&r, active);
    if title.is_empty() { r } else { r.on_hover_text(title) }
}

/// `.panel-footer`.
fn footer(ui: &mut Ui, shell: &mut Shell, p: &Palette) {
    ui.horizontal(|ui| {
        let focused = shell.ui.focus_mode;
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let r = Btn::new(if focused { "Exit focus" } else { "Focus mode" }).pressed(focused).title("Give the map more space").show(ui);
            if r.clicked() {
                shell.toggle_focus();
            }
            ui.add(egui::Label::new(RichText::new("Map edits stay separate from the model.").size(10.0).color(p.ink_3)).wrap());
        });
    });
}

/// `currentAnalysisMethod(store)`.
pub fn current_analysis_method(shell: &Shell) -> String {
    let store = &shell.store;
    if store.solution.as_ref().is_some_and(|s| s.steady_com.is_some()) {
        return "SteadyCom fixed µ".into();
    }
    let gimme = store.solution.as_ref().and_then(|s| s.extra.as_ref()).and_then(|e| e.get("gimme")).is_some_and(coxeter_core::js::truthy)
        || store.omics.gimme_enabled;
    let method = if gimme {
        "GIMME"
    } else if store.loopless {
        "Loopless FBA"
    } else if store.parsimonious {
        "pFBA"
    } else {
        "FBA"
    };
    if store.fva_enabled && !store.loopless { format!("{method} + FVA") } else { method.into() }
}

/// `formatPercent(value)`.
pub fn format_percent(value: f64) -> String {
    if !value.is_finite() {
        return "100".into();
    }
    if value.fract() == 0.0 {
        return coxeter_core::js::number_to_string(value);
    }
    coxeter_core::js::number_to_string(coxeter_core::js::string_to_number(&coxeter_core::js::to_fixed(value, 2)))
}

/// A `.task-link`: icon, title, description, arrow; opens a workspace.
fn task_link(ui: &mut Ui, shell: &mut Shell, workspace: &str, icon: &str, title: &str, description: &str, p: &Palette) {
    let width = ui.available_width();
    let r = Frame::new()
        .fill(p.panel)
        .stroke(Stroke::new(1.0, p.line))
        .corner_radius(CornerRadius::same(9))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(width - 22.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon).size(16.0).color(p.accent));
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(RichText::new(title).size(12.0).family(bold()).color(p.ink));
                    ui.add(egui::Label::new(RichText::new(description).size(10.5).color(p.ink_3)).wrap());
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new("›").size(14.0).color(p.ink_3));
                });
            });
        })
        .response;
    let r = ui.interact(r.rect, ui.id().with(("task-link", workspace)), egui::Sense::click());
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, title));
    if r.hovered() {
        ui.painter().rect_stroke(r.rect, CornerRadius::same(9), Stroke::new(1.0, p.accent_line), egui::StrokeKind::Inside);
    }
    if r.clicked() {
        shell.open_workspace_named(workspace);
    }
}

/// `.task-context`: what the tab is about.
fn task_context(ui: &mut Ui, p: &Palette, add: impl FnOnce(&mut Ui)) {
    Frame::new().fill(p.panel_2).stroke(Stroke::new(1.0, p.line)).corner_radius(CornerRadius::same(9)).inner_margin(Margin::same(10)).show(
        ui,
        |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        },
    );
}

/// `renderTaskPanel(store, ui)`.
pub fn render_task_panel(ui: &mut Ui, shell: &mut Shell, p: &Palette) {
    ui.spacing_mut().item_spacing.y = 8.0;
    match shell.ui.left_tab {
        LeftTab::Model => {
            let count = shell.store.species.iter().filter(|s| s.enabled).count();
            task_context(ui, p, |ui| {
                widgets::section_label(ui, "Community model");
                ui.label(RichText::new(format!("{count} active species")).family(bold()).color(p.ink));
                ui.add(egui::Label::new(RichText::new("Edit the biological inputs used by the analysis.").size(11.5).color(p.ink_2)).wrap());
            });
            task_link(ui, shell, "scenario", "◇", "Scenario & objective", "Setup and published communities", p);
            task_link(ui, shell, "community", "⬡", "Species & abundance", "Models and community composition", p);
            task_link(ui, shell, "medium", "◌", "Shared medium", "Nutrient supply and exchange", p);
            task_link(ui, shell, "reactions", "☰", "Reactions & bounds", "Every reaction’s constraints", p);
            task_link(ui, shell, "model", "⌁", "Biological model", "Create reactions and metabolites", p);
            task_link(ui, shell, "omics", "≋", "Omics & expression", "Visual overlays and GIMME", p);
            footnote(ui, "Model edits affect the analysis. Map edits affect the drawing.", p);
        }
        LeftTab::Analysis => analysis_hub(ui, shell, p),
        LeftTab::Build => {
            let tool = shell.map.tool();
            let (title, help) = tool_help(tool);
            task_context(ui, p, |ui| {
                widgets::section_label(ui, title);
                ui.add(egui::Label::new(RichText::new(help).size(11.5).color(p.ink_2)).wrap());
                if tool == Tool::Compartment {
                    crate::panels::map_options::compartment_tool_controls(ui, shell);
                }
            });
            task_link(ui, shell, "library", "⌕", "Reaction library", "Find and add model reactions", p);
            task_link(ui, shell, "structure", "⬡", "Structure", "Cells and nested compartments", p);
            task_link(ui, shell, "map", "⚙", "Map options", "Appearance, pathways and canvas", p);
            footnote(ui, "Map edits change the drawing. The biological model is preserved.", p);
        }
    }
}

fn footnote(ui: &mut Ui, s: &str, p: &Palette) {
    ui.add(egui::Label::new(RichText::new(s).size(11.0).color(p.ink_3)).wrap());
}

/// The help of each tool in the Edit Map hub.
pub fn tool_help(tool: Tool) -> (&'static str, &'static str) {
    match tool {
        Tool::Select => ("Select & inspect", "Click an object to inspect it. Drag an object to reposition it. Drag a metabolite label to place it around its circle."),
        Tool::Edit => ("Edit Control Points", "Show and move reaction control points to adjust reaction paths. Drag metabolites, reaction centres, or Bézier handles directly."),
        Tool::Reaction => ("Add Reaction mode", "Click a metabolite on the map to see only the model reactions that can be added from it."),
        Tool::Metabolite => (
            "Add Metabolite mode",
            "Click inside a compartment, then pick one of its model metabolites to draw it there. This adds a map instance; the biological model is unchanged.",
        ),
        Tool::Text => ("Add map text", "Click the map, type a label, choose its size and font beside it, and press Enter. Select a text later to restyle it."),
        Tool::Compartment => ("Add Compartment", "Choose a model compartment and a shape, then drag inside its highlighted parent to add it to the map."),
        Tool::Cell => ("Add Cell", "Choose an existing model species, then drag on the map to add its cell boundary."),
        Tool::Pan => ("Edit your map", "Choose an editing tool to work on the map."),
    }
}

/// The Simulate hub.
fn analysis_hub(ui: &mut Ui, shell: &mut Shell, p: &Palette) {
    let store = &shell.store;
    let fixed_mu = store.solution.as_ref().and_then(|s| s.steady_com.as_ref()).map(|sc| sc.mu);
    let method = current_analysis_method(shell);
    let objective = match fixed_mu {
        Some(mu) => format!("µ = {} h⁻¹", coxeter_core::js::number_to_string(mu)),
        None if store.custom_objective.is_some() => format!(
            "Edited objective ({})",
            if store.objective_sense() == coxeter_state::ObjectiveSense::Min { "minimised" } else { "maximised" }
        ),
        None if store.objective == objectives::SUM_BIOMASS => "Weighted biomass sum".into(),
        None => "Equal growth".into(),
    };
    let status = if store.status().state == coxeter_state::StatusState::Busy {
        "Computing…".to_string()
    } else {
        match store.solution.as_ref() {
            Some(s) if s.is_optimal() => "Optimal solution".into(),
            Some(s) => s.status.as_str().to_string(),
            None => "Ready to analyse".into(),
        }
    };
    let parameters = if fixed_mu.is_some() {
        "Abundances are outputs of this solve".to_string()
    } else if store.omics.gimme_enabled {
        format!("Objective target: {}%", coxeter_core::js::round(100.0 * store.omics.gimme_objective_fraction))
    } else if store.loopless {
        "Internal cycles excluded".into()
    } else if store.parsimonious {
        format!("Biomass target: {}% of optimum", format_percent(store.pfba_biomass_percent))
    } else {
        "Community objective maximisation".into()
    };
    task_context(ui, p, |ui| {
        widgets::section_label(ui, "Current simulation");
        ui.label(RichText::new(&method).family(bold()).color(p.ink));
        ui.add(egui::Label::new(RichText::new(format!("{objective} · {status}")).size(11.5).color(p.ink_2)).wrap());
        ui.add(egui::Label::new(RichText::new(&parameters).size(11.0).color(p.ink_3)).wrap());
    });
    let methods = [("fba", "FBA", "Flux balance"), ("pfba", "pFBA", "Minimum total flux"), ("fva", "FVA", "Flux ranges"), ("loopless", "Loopless", "Exclude internal cycles")];
    for pair in methods.chunks(2) {
        widgets::grow_row(ui, pair.len(), |ui, i| {
            let (method, name, small) = pair[i];
            let (mut enabled, mut title) = crate::panels::topbar::run_button_state(shell, method);
            if method == "fva" && fixed_mu.is_some() {
                enabled = false;
                title = "Run an ordinary analysis before FVA";
            }
            let mut job = egui::text::LayoutJob::default();
            job.append(name, 0.0, egui::TextFormat { font_id: egui::FontId::new(12.0, bold()), color: p.ink, ..Default::default() });
            job.append(&format!("\n{small}"), 0.0, egui::TextFormat { font_id: egui::FontId::proportional(10.0), color: p.ink_3, ..Default::default() });
            let hovered = enabled && crate::widgets::hovered_next(ui);
            let r = ui.add_enabled(
                enabled,
                egui::Button::new(job)
                    .fill(if hovered { p.hover } else { p.panel })
                    .stroke(Stroke::new(1.0, if hovered { p.ink_3 } else { p.line })).corner_radius(CornerRadius::same(8)).min_size(Vec2::new(ui.available_width(), 46.0)),
            );
            aria_label(&r, name);
            let r = if title.is_empty() { r } else { r.on_disabled_hover_text(title) };
            if r.clicked() {
                shell.run_analysis(method);
            }
        });
    }
    task_link(ui, shell, "analysis", "⚙", "Simulation settings", "Objective and solver options", p);
    task_link(ui, shell, "reactions", "☰", "Reactions & bounds", "Bounds, objective and fluxes", p);
    task_link(ui, shell, "crossfeeding", "⇄", "Cross-feeding explorer", "Producer and consumer transfers", p);
    if Btn::new("View simulation results ↗").wide().height(34.0).show(ui).clicked() {
        let stage = shell.map_rect.map_or(600.0, |r| r.height());
        shell.show_results(stage);
    }
    footnote(ui, "Input changes recalculate the current method automatically.", p);
    let _ = format_flux;
}
