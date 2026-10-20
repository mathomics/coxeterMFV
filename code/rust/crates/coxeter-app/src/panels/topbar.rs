//! The header (`header.topbar` of `index.html`): the brand, the model's
//! name and kind (`paintShell`, `modelKindLabel`), the save badge
//! (`paintSaveState`), the status line (`paintStatus`), Find reaction, the
//! theme button, and the File and Simulate menus.

use egui::{Color32, CornerRadius, Frame, Margin, RichText, Stroke, Ui, Vec2};

use crate::app::Shell;
use crate::theme::{Palette, bold, size};
use crate::widgets::{self, Btn, Look, aria_label};

/// The header's height.
pub const HEIGHT: f32 = 56.0;

/// Draw the header.
pub fn show(ui: &mut Ui, shell: &mut Shell) {
    let p = shell.palette();
    egui::Panel::top("topbar")
        .exact_size(HEIGHT)
        .show_separator_line(true)
        .frame(Frame::new().fill(p.panel).inner_margin(Margin::symmetric(18, 8)).stroke(Stroke::new(1.0, p.line)))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                brand(ui, &p);
                model_identity(ui, shell, &p);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    simulate_menu(ui, shell, &p);
                    file_menu(ui, shell, &p);
                    theme_button(ui, shell);
                    let find = Btn::new("⌕ Find reaction").look(Look::Secondary).title("Find reactions (Ctrl+F)").aria("Find reaction").height(34.0).show(ui);
                    if find.clicked() {
                        shell.open_workspace_named("library");
                        shell.ui.focus_request = Some(crate::panels::map_options::reaction_search_id());
                    }
                    status(ui, shell, &p);
                    save_state(ui, shell, &p);
                });
            });
        });
}

/// `.brand`: the mark and the name.
fn brand(ui: &mut Ui, p: &Palette) {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(34.0), egui::Sense::hover());
    let at = |x: f32, y: f32| rect.min + Vec2::new(x, y);
    let painter = ui.painter();
    let stroke = Stroke::new(2.5, p.accent);
    painter.line(vec![at(7.0, 25.0), at(14.0, 17.0), at(19.0, 8.0), at(27.0, 13.0)], stroke);
    for (x, y, r) in [(7.0, 25.0, 4.0), (14.0, 17.0, 3.3), (19.0, 8.0, 4.0), (27.0, 13.0, 3.0)] {
        painter.circle_filled(at(x, y), r, p.accent);
    }
    response.on_hover_text("Coxeter Multicell Flux Visualizer");
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.label(RichText::new("Coxeter-MFV").size(15.0).family(bold()).color(p.ink)).on_hover_text("Coxeter Multicell Flux Visualizer");
        ui.label(RichText::new("Edit Map · Edit Model · Simulate").size(11.0).color(p.ink_3));
    });
}

/// `#model-name` and `#model-kind`.
fn model_identity(ui: &mut Ui, shell: &Shell, p: &Palette) {
    let store = &shell.store;
    let name = store
        .scenario_str("name")
        .or_else(|| store.community.as_ref().map(|c| c.name.clone()).filter(|n| !n.is_empty()))
        .or_else(|| store.species.first().and_then(|sp| store.models.get(&sp.file)).map(|m| m.name.clone()))
        .unwrap_or_else(|| "Untitled model".into());
    let (kind, title) = model_kind_label(shell);
    ui.vertical(|ui| {
        ui.set_max_width(280.0);
        ui.spacing_mut().item_spacing.y = 1.0;
        ui.add(egui::Label::new(RichText::new(name).size(12.0).family(bold()).color(p.ink)).truncate());
        let r = ui.label(RichText::new(kind).size(11.0).color(p.ink_3));
        if !title.is_empty() {
            r.on_hover_text(title);
        }
    });
}

/// `modelKindLabel(store)`: the header badge, read from what is loaded
/// (a single-species upload is not a multicell community).
pub fn model_kind_label(shell: &Shell) -> (String, String) {
    let Some(community) = shell.store.community.as_ref() else { return (String::new(), String::new()) };
    let species: Vec<_> = community.active_species().collect();
    if species.is_empty() {
        return (String::new(), String::new());
    }
    let compartments = species.iter().map(|sp| community.species_compartments.get(&sp.id).map_or(1, Vec::len)).max().unwrap_or(1);
    let text = format!(
        "{} · {}",
        if compartments > 1 { "Multi-compartment" } else { "Single compartment" },
        if species.len() > 1 { "Multi-cell" } else { "Single cell" }
    );
    let title = format!(
        "{} active species; up to {compartments} compartment{} per cell",
        species.len(),
        if compartments == 1 { "" } else { "s" }
    );
    (text, title)
}

/// `#save-state` (`paintSaveState`).
fn save_state(ui: &mut Ui, shell: &Shell, p: &Palette) {
    let work = shell.save_state;
    let unsaved = work.map || work.scenario || work.model;
    let (mark, label, color) = if unsaved { ("●", "Unsaved", p.warn) } else { ("✓", "Saved", p.ok) };
    let title = if unsaved {
        [
            work.map.then_some("Map edits not saved (File → Export → Map JSON)"),
            work.scenario.then_some("Scenario changes not saved (File → Export → Scenario)"),
            work.model.then_some("Model edits not saved (File → Export → Model JSON)"),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("\n")
    } else {
        "Matches the last saved or opened map, scenario and model".to_string()
    };
    let r = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            ui.label(RichText::new(label).size(11.5).family(bold()).color(color));
            if unsaved {
                ui.label(RichText::new(mark).size(11.0).color(color));
            } else {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(17.0), egui::Sense::hover());
                ui.painter().circle_filled(rect.center(), 8.5, color);
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, mark, egui::FontId::proportional(11.0), p.panel);
            }
        })
        .response;
    r.on_hover_text(title);
}

/// `#status` (`paintStatus`).
fn status(ui: &mut Ui, shell: &mut Shell, p: &Palette) {
    let line = shell.status_line(ui.ctx());
    let dot = match line.state {
        "error" => p.danger,
        "busy" => p.warn,
        _ => p.ok,
    };
    let r = ui
        .horizontal(|ui| {
            ui.set_max_width(420.0);
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.add(egui::Label::new(RichText::new(&line.text).size(12.5).color(p.ink_2)).truncate());
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
            let alpha = if line.state == "busy" { 0.3 + 0.7 * ((ui.input(|i| i.time) * 3.5).sin().abs() as f32) } else { 1.0 };
            ui.painter().circle_filled(rect.center(), 4.0, dot.gamma_multiply(alpha));
            if line.state == "busy" {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
            }
        })
        .response;
    if !line.title.is_empty() {
        r.on_hover_text(&line.title);
    }
}

/// `.theme-button` (`toggle-theme`).
fn theme_button(ui: &mut Ui, shell: &mut Shell) {
    let dark = shell.store.view.dark;
    let label = if dark { "Switch to light theme" } else { "Switch to dark theme" };
    let r = Btn::new(format!("◐ {}", if dark { "Light" } else { "Dark" })).active(dark).title(label).aria(label).height(34.0).show(ui);
    widgets::aria_pressed(&r, dark);
    if r.clicked() {
        shell.set_theme(!dark);
    }
}

/// One item of a header menu: `<button data-action>`.
fn menu_item(ui: &mut Ui, label: &str, aria: &str, enabled: bool, title: &str) -> bool {
    let p = Palette::current(ui.ctx());
    let fill = if enabled && widgets::hovered_next(ui) { p.hover } else { Color32::TRANSPARENT };
    let button = egui::Button::new(RichText::new(label).size(size::BUTTON).color(p.ink))
        .fill(fill)
        .stroke(Stroke::NONE)
        .min_size(Vec2::new(ui.available_width().max(230.0), 32.0));
    let r = ui.add_enabled(enabled, button);
    aria_label(&r, aria);
    let r = if title.is_empty() { r } else if enabled { r.on_hover_text(title) } else { r.on_disabled_hover_text(title) };
    r.clicked()
}

/// `.top-menu-popover > small`.
fn menu_heading(ui: &mut Ui, s: &str) {
    let p = Palette::current(ui.ctx());
    ui.add_space(4.0);
    ui.label(RichText::new(s).size(9.5).family(bold()).color(p.ink_3).extra_letter_spacing(0.5));
}

/// `details.file-menu`.
fn file_menu(ui: &mut Ui, shell: &mut Shell, p: &Palette) {
    let hovered = widgets::hovered_next(ui);
    let button = egui::Button::new(RichText::new("File ⌄").size(size::BUTTON).family(bold()).color(p.ink))
        .fill(if hovered { p.hover } else { p.panel })
        .stroke(Stroke::new(1.0, if hovered { p.ink_3 } else { p.line }))
        .corner_radius(CornerRadius::same(8))
        .min_size(Vec2::new(0.0, 34.0));
    let (response, _) = egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
        ui.set_min_width(250.0);
        menu_heading(ui, "IMPORT");
        if menu_item(ui, "Model (JSON / SBML / MAT / XLSX)", "Import model (JSON / SBML / MAT / XLSX)", true, "") {
            shell.import_models();
        }
        if menu_item(ui, "Metatranscriptomics (CSV / TSV)", "Import metatranscriptomics (CSV / TSV)", true, "") {
            shell.import_metatranscriptomics();
        }
        if menu_item(ui, "Map JSON", "Import map JSON", true, "") {
            shell.map_load();
        }
        // A map opened before its model is offered for forgetting while it waits.
        if let Some(waiting) = shell.store.waiting_map.clone() {
            let label = match &waiting.name {
                Some(name) => format!("Forget waiting map ({name})"),
                None => "Forget waiting map".to_string(),
            };
            let title = format!("Will be drawn when {} is loaded", waiting.needs);
            if menu_item(ui, &label, &label, true, &title) {
                shell.discard_waiting_map();
            }
        }
        if menu_item(ui, "Scenario", "Import scenario", true, "") {
            shell.import_state();
        }
        if menu_item(ui, "Reproducibility bundle", "Import reproducibility bundle", true, "") {
            shell.import_reproducibility();
        }
        menu_heading(ui, "EXPORT / SAVE A LOCAL COPY");
        if menu_item(ui, "Map JSON", "Export map JSON", true, "") {
            shell.map_save();
        }
        if menu_item(ui, "Model JSON", "Export model JSON", true, "") {
            shell.export_model(false);
        }
        if menu_item(ui, "Model SBML", "Export model SBML", true, "") {
            shell.export_model(true);
        }
        if menu_item(ui, "SVG map", "Export SVG map", true, "") {
            shell.export_svg();
        }
        if menu_item(ui, "PNG map", "Export PNG map", true, "") {
            shell.export_png();
        }
        if menu_item(ui, "Flux table (CSV)", "Export flux table (CSV)", true, "") {
            shell.export_csv();
        }
        if menu_item(ui, "Scenario", "Export scenario", true, "") {
            shell.export_state();
        }
        if menu_item(ui, "Reproducibility bundle", "Export reproducibility bundle", true, "") {
            shell.export_reproducibility();
        }
        if menu_item(ui, "Methods / provenance report", "Export methods / provenance report", true, "") {
            shell.export_methods_provenance();
        }
    });
    aria_label(&response, "File menu");
}

/// The analysis buttons' state (`paintShell`): FVA needs an ordinary
/// solution, and the Zuñiga case keeps its authors' method.
pub fn run_button_state(shell: &Shell, method: &str) -> (bool, &'static str) {
    let zuniga = shell.store.is_zuniga_study();
    let steady = shell.store.solution.as_ref().is_some_and(|s| s.steady_com.is_some());
    const ZUNIGA: &str = "The Zuñiga case preserves the original source-model FBA method";
    match method {
        "fva" if zuniga => (false, ZUNIGA),
        "fva" if steady => (false, "Run FBA, pFBA or Loopless before FVA"),
        "pfba" | "loopless" if zuniga => (false, ZUNIGA),
        _ => (true, ""),
    }
}

/// `details.analysis-menu`.
fn simulate_menu(ui: &mut Ui, shell: &mut Shell, p: &Palette) {
    let button = egui::Button::new(RichText::new(format!("{} Simulate ⌄", crate::fonts::glyph::RUN)).size(size::BUTTON).family(bold()).color(p.accent_ink))
        .fill(p.accent)
        .stroke(Stroke::new(1.0, p.accent))
        .corner_radius(CornerRadius::same(8))
        .min_size(Vec2::new(0.0, 34.0));
    let (response, _) = egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
        ui.set_min_width(250.0);
        menu_heading(ui, "FLUX ANALYSIS");
        for (method, name, description) in [
            ("fba", "FBA", "Flux balance analysis"),
            ("pfba", "pFBA", "Parsimonious FBA"),
            ("fva", "FVA", "Flux variability analysis"),
            ("loopless", "Loopless FBA", "Exclude internal cycles"),
        ] {
            let (enabled, title) = run_button_state(shell, method);
            let mut job = egui::text::LayoutJob::default();
            let pal = Palette::current(ui.ctx());
            job.append(name, 0.0, egui::TextFormat { font_id: egui::FontId::new(12.5, bold()), color: pal.ink, ..Default::default() });
            job.append(&format!("   {description}"), 0.0, egui::TextFormat { font_id: egui::FontId::proportional(11.5), color: pal.ink_3, ..Default::default() });
            let fill = if enabled && widgets::hovered_next(ui) { pal.hover } else { Color32::TRANSPARENT };
            let button = egui::Button::new(job).fill(fill).stroke(Stroke::NONE).min_size(Vec2::new(ui.available_width().max(230.0), 34.0));
            let r = ui.add_enabled(enabled, button);
            aria_label(&r, &format!("Run {name}"));
            let r = if title.is_empty() { r } else { r.on_disabled_hover_text(title) };
            if r.clicked() {
                shell.run_analysis(method);
            }
        }
        if menu_item(ui, "Simulation settings", "Simulation settings", true, "") {
            shell.open_workspace_named("analysis");
        }
    });
    aria_label(&response, "Simulate menu");
}
