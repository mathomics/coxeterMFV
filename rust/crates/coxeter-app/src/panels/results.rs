//! The results dock (`section.results-panel` of `index.html`): its tabs,
//! the run summary (`paintRunSummary` of `src/ui/app.js`), maximize and
//! collapse, and what `renderResults`, `analysisTabContent`,
//! `reproducibilityReport`, `resultReactionTable` and `renderLogEntries` of
//! `src/ui/panels.js` render. The Table view lists every reaction (see
//! `reactions::table_view`), or the shadow prices in full.

use coxeter_core::js;
use coxeter_render::format_flux;
use coxeter_state::RunPhase;
use egui::{Color32, CornerRadius, Frame, Margin, RichText, Stroke, Ui, Vec2};
use serde_json::Value;

use crate::app::Shell;
use crate::theme::{Palette, bold};
use crate::ui_state::{AnalysisTab, ResultsTab};
use crate::widgets::{self, Btn, Look, aria_label};

/// The tab bar's height.
pub const TABS: f32 = 38.0;

/// The dock: collapsed to its tab bar, docked under the map, or (`fill`)
/// taking the whole column.
pub fn show(ui: &mut Ui, shell: &mut Shell, fill: bool) {
    let p = shell.palette();
    let collapsed = (shell.ui.results_collapsed || shell.ui.focus_mode) && !fill;
    let body = |ui: &mut Ui, shell: &mut Shell| {
        tabs(ui, shell, &p);
        if !collapsed && shell.ui.results_tab == ResultsTab::Table {
            // The Table view scrolls its own rows.
            Frame::new().inner_margin(Margin::same(10)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.set_height(ui.available_height());
                crate::panels::reactions::table_view(ui, shell, |ui, shell| {
                    let state = tab_state(shell);
                    analysis_tab_content(ui, shell, AnalysisTab::Shadow, &state, true, &p);
                });
            });
        } else if !collapsed {
            egui::ScrollArea::vertical().id_salt("results-scroll").auto_shrink([false, false]).show(ui, |ui| {
                Frame::new().inner_margin(Margin::same(10)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    content(ui, shell, &p);
                });
            });
        }
    };
    let frame = Frame::new().fill(p.panel).stroke(Stroke::new(1.0, p.line));
    if fill {
        egui::CentralPanel::default().frame(frame).show(ui, |ui| body(ui, shell));
        return;
    }
    let height = if collapsed { TABS } else { shell.ui.results_height };
    let response = egui::Panel::bottom("results-panel")
        .exact_size(height)
        .resizable(false)
        .frame(frame)
        .show(ui, |ui| body(ui, shell))
        .response;
    if !collapsed {
        // `.results-resize`: drag the top edge to resize the dock.
        let edge = egui::Rect::from_min_size(response.rect.min - Vec2::new(0.0, 3.0), Vec2::new(response.rect.width(), 6.0));
        let drag = ui.interact(edge, egui::Id::new("results-resize"), egui::Sense::drag());
        if drag.hovered() || drag.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }
        if drag.dragged() {
            let available = (ui.max_rect().height() - 320.0).max(140.0);
            shell.ui.results_height = (shell.ui.results_height - drag.drag_delta().y).clamp(140.0, available);
        }
    }
}

/// `.results-tabs`.
fn tabs(ui: &mut Ui, shell: &mut Shell, p: &Palette) {
    Frame::new().inner_margin(Margin::symmetric(10, 0)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.set_height(TABS - 1.0);
        ui.horizontal_centered(|ui| {
            for (tab, label) in [(ResultsTab::Analysis, "Analysis results"), (ResultsTab::Table, "Table view"), (ResultsTab::Logs, "Logs")] {
                let active = shell.ui.results_tab == tab;
                let r = ui.add(
                    egui::Button::new(RichText::new(label).size(12.0).color(if active { p.accent } else { p.ink_3 }))
                        .fill(Color32::TRANSPARENT)
                        .stroke(Stroke::NONE)
                        .min_size(Vec2::new(0.0, TABS - 4.0)),
                );
                if active {
                    ui.painter().hline(r.rect.x_range(), r.rect.bottom(), Stroke::new(2.0, p.accent));
                }
                widgets::aria_pressed(&r, active);
                if r.clicked() {
                    shell.set_results_tab(tab);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let collapse = Btn::new(RichText::new("⌄").size(16.0).color(p.ink_3)).look(Look::Flat).title("Collapse results").aria("Collapse results").show(ui);
                if collapse.clicked() {
                    shell.toggle_results();
                }
                let max_label = if shell.ui.results_maximized { "Restore results dock" } else { "Maximize results" };
                let max = Btn::new(RichText::new("⤢").size(15.0).color(p.ink_3)).look(Look::Flat).title(max_label).aria(max_label).show(ui);
                if max.clicked() {
                    shell.maximize_results();
                }
                run_summary(ui, shell, p);
            });
        });
    });
    let rect = ui.min_rect();
    ui.painter().hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, p.line));
}

/// `RUN_MODE`: what each Simulate button runs, as the mode line names it.
fn run_mode(method: coxeter_state::AnalysisMethod) -> &'static str {
    match method {
        coxeter_state::AnalysisMethod::Fba => "FBA",
        coxeter_state::AnalysisMethod::Pfba => "pFBA",
        coxeter_state::AnalysisMethod::Fva => "FVA",
        coxeter_state::AnalysisMethod::Pfva => "pFVA",
        coxeter_state::AnalysisMethod::Loopless => "Loopless",
        coxeter_state::AnalysisMethod::CycleFree => "CycleFreeFlux",
    }
}

/// `new Date(ms).toLocaleTimeString()`.
pub fn local_time(ms: f64) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_millis_opt(ms as i64).single() {
        Some(t) => t.format("%-I:%M:%S %p").to_string(),
        None => String::new(),
    }
}

/// `paintRunSummary(optimal)`: how the last analysis run from the Simulate
/// panel went, the one line that stays on screen at every width.
pub fn run_summary_text(shell: &Shell) -> (String, &'static str) {
    let store = &shell.store;
    let status = store.status();
    if status.state == coxeter_state::StatusState::Error && !status.message.is_empty() {
        return (status.message.clone(), "error");
    }
    if status.state == coxeter_state::StatusState::Warn && !status.message.is_empty() {
        return (status.message.clone(), "warn");
    }
    let optimal = store.solution.as_ref().is_some_and(|s| s.is_optimal());
    let mode = if store.solution.as_ref().is_some_and(|s| s.steady_com.is_some()) {
        "SteadyCom fixed µ"
    } else if store.omics.gimme_enabled || store.solution.as_ref().and_then(|s| s.extra.as_ref()).and_then(|s| s.get("gimme")).is_some_and(js::truthy) {
        "GIMME"
    } else if store.loopless {
        "Loopless"
    } else if store.cycle_free {
        "CycleFreeFlux"
    } else if store.pfva_enabled && store.fva_enabled {
        "pFVA"
    } else if store.parsimonious {
        "pFBA"
    } else if store.fva_enabled {
        "FVA"
    } else {
        "FBA"
    };
    let not_run = if store.solution_cleared { "Solution cleared" } else { "Not run" };
    let status = store.solution.as_ref().map_or_else(|| not_run.to_string(), |s| s.status.as_str().to_string());
    let objective = if optimal { format!(" · {}", format_flux(store.solution.as_ref().map(|s| s.objective_value), 5)) } else { String::new() };
    let mut text = format!("{mode} · {status}{objective}");
    let mut state = "";
    if let Some(run) = &store.last_run {
        if run.phase == RunPhase::Running {
            let fva = if matches!(run.method, coxeter_state::AnalysisMethod::Fva | coxeter_state::AnalysisMethod::Pfva) && store.fva_progress.running {
                format!(" {} / {} reactions", store.fva_progress.done, store.fva_progress.total)
            } else {
                String::new()
            };
            text = format!("Running {}…{fva}", run.label);
            state = "busy";
        } else if run_mode(run.method) == mode {
            let at = local_time(run.finished_at.unwrap_or(0.0));
            match run.phase {
                RunPhase::Done => {
                    let detail = if matches!(run.method, coxeter_state::AnalysisMethod::Fva | coxeter_state::AnalysisMethod::Pfva) {
                        format!("{} flux ranges", store.fva_ranges.len())
                    } else {
                        format!("{} · {}", run.outcome.clone().unwrap_or_default(), format_flux(run.objective, 5))
                    };
                    text = format!("✓ {} completed {at} · {detail} · {}", run.label, run.took.clone().unwrap_or_default());
                    state = "success";
                }
                RunPhase::Failed => {
                    text = format!("✕ {} did not finish {at} · {}", run.label, run.outcome.clone().unwrap_or_default());
                    state = "error";
                }
                RunPhase::Cancelled => text = format!("{} cancelled {at}", run.label),
                RunPhase::Running => {}
            }
        }
    }
    (text, state)
}

fn run_summary(ui: &mut Ui, shell: &mut Shell, p: &Palette) {
    let (text, state) = run_summary_text(shell);
    let finished = shell.store.last_run.as_ref().and_then(|r| r.finished_at);
    if let Some(at) = finished {
        if shell.run_flash.is_none_or(|(prev, _)| prev != at) {
            shell.run_flash = Some((at, std::time::Instant::now()));
        }
    }
    let flashing = shell.run_flash.is_some_and(|(_, t)| t.elapsed().as_secs_f32() < 2.6);
    let color = match state {
        "busy" | "warn" => p.warn,
        "success" => p.ok,
        "error" => p.danger,
        _ => p.ink_3,
    };
    let r = Frame::new()
        .fill(if flashing { p.success_bg } else { Color32::TRANSPARENT })
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.set_max_width((ui.available_width() - 20.0).max(80.0));
            ui.add(egui::Label::new(RichText::new(&text).size(11.5).color(color)).truncate())
        })
        .inner;
    aria_label(&r, &text);
    r.on_hover_text(&text);
    if flashing {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
    }
}

// Content

/// The method line of the results (`renderResults`' `mode`).
fn results_mode(shell: &Shell) -> &'static str {
    let store = &shell.store;
    let solution = store.solution.as_ref();
    if solution.is_some_and(|s| s.steady_com.is_some()) {
        "SteadyCom fixed µ"
    } else if solution.and_then(|s| s.extra.as_ref()).and_then(|e| e.get("gimme")).is_some_and(js::truthy) {
        "GIMME"
    } else if store.loopless {
        "Loopless FBA"
    } else if store.cycle_free {
        "CycleFreeFlux"
    } else if store.pfva_enabled && store.fva_enabled {
        "pFVA"
    } else if store.parsimonious {
        "pFBA"
    } else if store.fva_enabled {
        "FVA"
    } else {
        "FBA"
    }
}

/// `solverLabel(store)`: the solver that produced the solution, with its
/// version when it is the one in use.
fn solver_label(shell: &Shell) -> String {
    match shell.store.solution.as_ref().map(|s| s.solver.as_str()) {
        Some("glpk") => "GLPK (WebAssembly)".into(),
        Some("simplex") => "Local simplex.js".into(),
        Some(other) if !other.is_empty() => shell.store.solution_solver_label().unwrap_or_else(|| other.into()),
        _ if shell.store.loopless => "Local MILP".into(),
        _ => "—".into(),
    }
}

struct TabState {
    optimal: bool,
    dual_available: bool,
    fva_available: bool,
    replay_available: bool,
}

fn tab_state(shell: &Shell) -> TabState {
    let store = &shell.store;
    let solution = store.solution.as_ref();
    let optimal = solution.is_some_and(|s| s.is_optimal());
    let dual_available =
        optimal && solution.and_then(|s| s.result.as_ref()).is_some_and(|r| r.dual_analysis.available) && !store.loopless;
    let fva_available =
        !solution.is_some_and(|s| s.steady_com.is_some()) && (store.fva_enabled || !store.fva_ranges.is_empty() || store.fva_progress.running);
    let replay_available = store.reproducibility_replay.is_some();
    TabState { optimal, dual_available, fva_available, replay_available }
}

fn content(ui: &mut Ui, shell: &mut Shell, p: &Palette) {
    let state = tab_state(shell);
    let tab = shell.ui.analysis_tab;
    match shell.ui.results_tab {
        ResultsTab::Analysis => {
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(Vec2::new(280.0, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(280.0);
                    summary(ui, shell, &state, p);
                });
                ui.vertical(|ui| {
                    subtabs(ui, shell, &state);
                    analysis_tab_content(ui, shell, tab, &state, false, p);
                });
            });
        }
        // Drawn by `show` itself (see `reactions::table_view`).
        ResultsTab::Table => {}
        ResultsTab::Logs => logs(ui, shell, p),
    }
}

/// `.analysis-summary`: the verdict and the four metrics.
fn summary(ui: &mut Ui, shell: &Shell, state: &TabState, p: &Palette) {
    let store = &shell.store;
    let mode = results_mode(shell);
    let status = store.solution.as_ref().map_or_else(|| "Not run".to_string(), |s| s.status.as_str().to_string());
    let status_label = if state.optimal { "Optimal".to_string() } else { status };
    let (fill, line) = if state.optimal { (p.success_bg, p.success_line) } else { (p.danger_bg, p.danger_line) };
    widgets::tinted(ui, fill, line, |ui| {
        ui.horizontal_top(|ui| {
            ui.label(RichText::new(if state.optimal { "✓" } else { "!" }).size(14.0).family(bold()).color(if state.optimal { p.ok } else { p.danger }));
            ui.vertical(|ui| {
                ui.label(RichText::new(format!("{mode} · {status_label}")).size(12.5).family(bold()));
                if !state.optimal {
                    let small = store
                        .solution
                        .as_ref()
                        .map(|s| s.message.clone())
                        .filter(|m| !m.is_empty())
                        .or_else(|| Some(store.status().message.clone()).filter(|m| !m.is_empty()))
                        .unwrap_or_else(|| "No solution available.".into());
                    ui.add(egui::Label::new(RichText::new(small).size(11.0).color(p.ink_2)).wrap());
                }
            });
        });
    });
    ui.add_space(6.0);
    let objective = if state.optimal { format_flux(store.solution.as_ref().map(|s| s.objective_value), 5) } else { "—".into() };
    let metrics = [
        ("Objective value", objective),
        ("Solver", solver_label(shell)),
        ("Time", format!("{} ms", js::number_to_string(store.status().solve_ms))),
        ("Status", status_label),
    ];
    egui::Grid::new("analysis-metrics").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
        for (dt, dd) in metrics {
            ui.label(RichText::new(dt).size(11.0).color(p.ink_3));
            ui.label(RichText::new(dd).size(12.0).family(bold()));
            ui.end_row();
        }
    });
}

/// `.subtabs`.
fn subtabs(ui: &mut Ui, shell: &mut Shell, state: &TabState) {
    let not_returned = "Not returned by the current solver";
    let tabs = [
        (AnalysisTab::Flux, "Flux", true, ""),
        (AnalysisTab::Fva, "FVA", state.fva_available, "Run FVA to fill this tab"),
        (AnalysisTab::Reduced, "Reduced costs", state.dual_available, not_returned),
        (AnalysisTab::Scaled, "Scaled reduced costs", state.dual_available, not_returned),
        (AnalysisTab::Shadow, "Shadow prices", state.dual_available, not_returned),
        (AnalysisTab::Repro, "Reproducibility", state.replay_available, "Import or run a reproducibility replay first"),
    ];
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (tab, label, enabled, reason) in tabs {
            let active = shell.ui.analysis_tab == tab;
            let r = Btn::new(RichText::new(label).size(11.5)).look(if active { Look::Active } else { Look::Normal }).enabled(enabled).title(if enabled { "" } else { reason }).height(24.0).show(ui);
            widgets::aria_pressed(&r, active);
            if r.clicked() {
                shell.ui.analysis_tab = tab;
            }
        }
    });
    ui.add_space(4.0);
}

/// `analysisTabContent(store, tab, state)`.
fn analysis_tab_content(ui: &mut Ui, shell: &mut Shell, tab: AnalysisTab, state: &TabState, full: bool, p: &Palette) {
    if tab == AnalysisTab::Repro {
        return reproducibility_report(ui, shell, p);
    }
    if !state.optimal {
        widgets::empty(ui, "Run an analysis to see results.");
        return;
    }
    let limit = if full { usize::MAX } else { 20 };
    let Some(solution) = shell.store.solution.clone() else { return };
    match tab {
        AnalysisTab::Flux => {
            let mut rows: Vec<(String, f64)> = solution.fluxes.iter().map(|(k, v)| (k.clone(), *v)).collect();
            rows.sort_by(|a, b| b.1.abs().partial_cmp(&a.1.abs()).unwrap_or(std::cmp::Ordering::Equal).then_with(|| js::locale_compare(&a.0, &b.0)));
            let rows: Vec<(String, String, Option<String>)> = rows.into_iter().map(|(id, v)| (id, format_flux(Some(v), 6), None)).collect();
            reaction_table(ui, shell, &rows, &["Reaction", "Name", "Flux"], limit, full, p);
        }
        AnalysisTab::Fva => {
            if !state.fva_available {
                widgets::empty(ui, "FVA is not enabled for this solution.");
                return;
            }
            let progress = shell.store.fva_progress;
            if progress.running {
                widgets::hint(ui, format!("Computing FVA: {}/{}", progress.done, progress.total));
            }
            let mut rows: Vec<(String, coxeter_state::FvaRange)> = shell.store.fva_ranges.iter().map(|(k, r)| (k.clone(), *r)).collect();
            rows.sort_by(|a, b| (b.1.max - b.1.min).partial_cmp(&(a.1.max - a.1.min)).unwrap_or(std::cmp::Ordering::Equal));
            let title = if shell.store.pfva_enabled { "pFVA" } else { "FVA" };
            if shell.store.pfva_enabled {
                widgets::hint(ui, "Total absolute flux ≤ 1.00001 × minimum, including exchanges.");
            }
            widgets::hint(ui, format!("Showing {} of {} ranges, widest first.", rows.len().min(limit), rows.len()));
            if !rows.is_empty() {
                flux_range_plot(ui, shell, &rows[..rows.len().min(limit)], title);
            }
            let rows: Vec<(String, String, Option<String>)> = rows
                .into_iter()
                .map(|(id, r)| (id, format!("[{}, {}]", format_flux(Some(r.min), 5), format_flux(Some(r.max), 5)), Some(format_flux(Some(r.max - r.min), 5))))
                .collect();
            if rows.is_empty() {
                widgets::empty(ui, "No FVA ranges available yet.");
            } else {
                reaction_table(ui, shell, &rows, &["Reaction", "Name", "FVA range", "Width"], limit, full, p);
            }
        }
        AnalysisTab::Shadow => {
            if !state.dual_available {
                return dual_unavailable(ui, shell);
            }
            dual_context(ui, shell);
            let mut rows: Vec<(String, f64)> =
                solution.result.as_ref().map(|r| r.shadow_prices.iter().map(|(k, v)| (k.clone(), *v)).collect()).unwrap_or_default();
            rows.sort_by(|a, b| b.1.abs().partial_cmp(&a.1.abs()).unwrap_or(std::cmp::Ordering::Equal));
            if rows.is_empty() {
                widgets::empty(ui, "No shadow prices available.");
                return;
            }
            let shown = rows.len().min(limit);
            let width = ui.available_width();
            table(ui, &["Metabolite", "Shadow price"], shown, full, p, |ui, i| {
                cell(ui, width * 0.6, |ui| {
                    ui.label(widgets::code(&rows[i].0));
                });
                cell(ui, width * 0.4, |ui| {
                    ui.label(RichText::new(format_flux(Some(rows[i].1), 7)).size(12.0));
                });
            });
        }
        AnalysisTab::Reduced | AnalysisTab::Scaled => {
            if !state.dual_available {
                return dual_unavailable(ui, shell);
            }
            dual_context(ui, shell);
            let scaled = tab == AnalysisTab::Scaled;
            let source = solution.result.as_ref().map(|r| if scaled { &r.scaled_reduced_costs } else { &r.reduced_costs });
            let mut rows: Vec<(String, f64)> = source.map(|m| m.iter().map(|(k, v)| (k.clone(), *v)).collect()).unwrap_or_default();
            rows.sort_by(|a, b| b.1.abs().partial_cmp(&a.1.abs()).unwrap_or(std::cmp::Ordering::Equal).then_with(|| js::locale_compare(&a.0, &b.0)));
            let digits = if scaled { 6 } else { 7 };
            let rows: Vec<(String, String, Option<String>)> = rows.into_iter().map(|(id, v)| (id, format_flux(Some(v), digits), None)).collect();
            reaction_table(ui, shell, &rows, &["Reaction", "Name", if scaled { "Scaled RC" } else { "Reduced cost" }], limit, full, p);
        }
        AnalysisTab::Repro => {}
    }
}

/// `renderFluxRanges` of `src/render/flux-ranges.js`, drawn with the native painter.
fn flux_range_plot(ui: &mut Ui, shell: &Shell, rows: &[(String, coxeter_state::FvaRange)], title: &str) {
    let mut min = 0.0_f64;
    let mut max = 0.0_f64;
    for (_, range) in rows {
        if range.min.is_finite() && range.max.is_finite() {
            min = min.min(range.min).min(range.max);
            max = max.max(range.min).max(range.max);
        }
    }
    if min == max {
        min = -1.0;
        max = 1.0;
    }
    let width = ui.available_width().max(320.0);
    let height = 150.0 + rows.len() as f32 * 30.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::WHITE);
    let ink = Color32::from_rgb(31, 41, 55);
    let blue = Color32::from_rgb(37, 99, 235);
    let orange = Color32::from_rgb(194, 65, 12);
    let font = egui::FontId::proportional(12.0);
    let text = |x: f32, y: f32, label: &str, align| {
        painter.text(rect.min + Vec2::new(x, y), align, label, font.clone(), ink);
    };
    let left = width * 250.0 / 960.0;
    let right = width - 40.0;
    let multiple_species = shell.store.species.iter().filter(|species| species.enabled).count() > 1;
    let x = |value: f64| left + (right - left) * ((value - min) / (max - min)) as f32;
    text(12.0, 24.0, &format!("{title} flux intervals"), egui::Align2::LEFT_BOTTOM);
    text(12.0, 48.0, "Endpoints: minimum and maximum · orange diamond: current solution", egui::Align2::LEFT_BOTTOM);
    for i in 0..=4 {
        let value = min + (max - min) * f64::from(i) / 4.0;
        let position = x(value);
        painter.line_segment(
            [rect.min + Vec2::new(position, 62.0), rect.min + Vec2::new(position, height - 64.0)],
            Stroke::new(1.0, Color32::from_rgb(229, 231, 235)),
        );
        text(position, height - 44.0, &format_flux(Some(value), 3), egui::Align2::CENTER_BOTTOM);
    }
    painter.line_segment(
        [rect.min + Vec2::new(x(0.0), 62.0), rect.min + Vec2::new(x(0.0), height - 64.0)],
        Stroke::new(1.0, Color32::from_rgb(100, 116, 139)),
    );
    for (i, (rid, range)) in rows.iter().enumerate() {
        let y = 74.0 + i as f32 * 30.0;
        let mut label = shell.store.community.as_ref().and_then(|c| c.reaction_owner.get(rid)).map(|o| o.original_id.as_str()).unwrap_or(rid);
        if multiple_species {
            label = rid;
        }
        text(12.0, y + 4.0, label, egui::Align2::LEFT_BOTTOM);
        if !range.min.is_finite() || !range.max.is_finite() {
            text(left, y + 4.0, "Unsolved", egui::Align2::LEFT_BOTTOM);
            continue;
        }
        let lo = rect.min + Vec2::new(x(range.min), y);
        let hi = rect.min + Vec2::new(x(range.max), y);
        painter.line_segment([lo, hi], Stroke::new(4.0, blue));
        painter.circle_filled(lo, 4.0, blue);
        painter.circle_filled(hi, 4.0, blue);
        let flux = shell.store.solution.as_ref().and_then(|s| s.fluxes.get(rid)).copied();
        if let Some(value) = flux.filter(|value| value.is_finite() && *value >= min && *value <= max) {
            let centre = rect.min + Vec2::new(x(value), y);
            let points = vec![centre + Vec2::new(0.0, -5.0), centre + Vec2::new(5.0, 0.0), centre + Vec2::new(0.0, 5.0), centre + Vec2::new(-5.0, 0.0)];
            painter.add(egui::Shape::convex_polygon(points, orange, Stroke::NONE));
        }
    }
    text((left + right) / 2.0, height - 12.0, "Flux (model units) · linear scale", egui::Align2::CENTER_BOTTOM);
}

/// `dualContext(store)`.
fn dual_context(ui: &mut Ui, shell: &Shell) {
    let text = shell
        .store
        .solution
        .as_ref()
        .and_then(|s| s.result.as_ref())
        .and_then(|r| r.dual_analysis.interpretation.clone())
        .unwrap_or_else(|| "Optimal LP basis".into());
    widgets::hint(ui, text);
}

/// `dualUnavailable(store)`.
fn dual_unavailable(ui: &mut Ui, shell: &Shell) {
    widgets::empty(
        ui,
        if shell.store.loopless {
            "Dual values are not reported for the Loopless MILP solution."
        } else {
            "Dual values are not available for this solution."
        },
    );
}

/// A two-column table (`Metabolite`, `Shadow price`): only the visible rows
/// are laid out in the full table view.
fn table(ui: &mut Ui, headers: &[&str], n: usize, full: bool, p: &Palette, mut row: impl FnMut(&mut Ui, usize)) {
    let width = ui.available_width();
    let widths = [width * 0.6, width * 0.4];
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (i, h) in headers.iter().enumerate().take(2) {
            cell(ui, widths[i], |ui| {
                ui.label(RichText::new(h.to_uppercase()).size(10.0).family(bold()).color(p.ink_3));
            });
        }
    });
    ui.separator();
    let mut body = |ui: &mut Ui, range: std::ops::Range<usize>| {
        ui.spacing_mut().interact_size.y = 18.0;
        ui.spacing_mut().item_spacing.y = 2.0;
        for i in range {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.set_min_width(width);
                row(ui, i);
            });
        }
    };
    if full {
        egui::ScrollArea::vertical().id_salt("results-full-table").auto_shrink([false, true]).show_rows(ui, 22.0, n, |ui, range| body(ui, range));
    } else {
        body(ui, 0..n);
    }
}

/// `resultReactionTable` / `reactionRow`: reaction (a link that selects and
/// shows it), name, value(s).
fn reaction_table(ui: &mut Ui, shell: &mut Shell, rows: &[(String, String, Option<String>)], headers: &[&str], limit: usize, full: bool, p: &Palette) {
    if rows.is_empty() {
        widgets::empty(ui, "No results available.");
        return;
    }
    let shown = rows.len().min(limit);
    let width = ui.available_width();
    let four = headers.len() == 4;
    let widths: Vec<f32> = if four { vec![0.24, 0.36, 0.24, 0.16] } else { vec![0.32, 0.43, 0.25] }.into_iter().map(|f| f * width).collect();
    let header = |ui: &mut Ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for (i, h) in headers.iter().enumerate() {
                cell(ui, widths[i], |ui| {
                    ui.label(RichText::new(h.to_uppercase()).size(10.0).family(bold()).color(p.ink_3));
                });
            }
        });
        ui.separator();
    };
    header(ui);
    let mut clicked = None;
    let mut body = |ui: &mut Ui, range: std::ops::Range<usize>| {
        ui.spacing_mut().interact_size.y = 18.0;
        ui.spacing_mut().item_spacing.y = 2.0;
        for i in range {
            let (rid, value, extra) = &rows[i];
            let owner = shell.store.community.as_ref().and_then(|c| c.reaction_owner.get(rid)).map(|o| o.original_id.clone());
            let name = shell.store.community.as_ref().and_then(|c| c.merged.reactions.get(rid)).map(|r| r.name.clone()).unwrap_or_default();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                cell(ui, widths[0], |ui| {
                    let r = widgets::link_button(ui, owner.as_deref().unwrap_or(rid));
                    aria_label(&r, owner.as_deref().unwrap_or(rid));
                    if r.clicked() {
                        clicked = Some(rid.clone());
                    }
                });
                cell(ui, widths[1], |ui| {
                    ui.add(egui::Label::new(RichText::new(&name).size(11.5).color(p.ink_2)).truncate());
                });
                cell(ui, widths[2], |ui| {
                    ui.label(RichText::new(value).size(12.0));
                });
                if let Some(extra) = extra {
                    cell(ui, widths[3], |ui| {
                        ui.label(RichText::new(extra).size(12.0));
                    });
                }
            });
        }
    };
    if full {
        egui::ScrollArea::vertical().id_salt("results-full-table").auto_shrink([false, true]).show_rows(ui, 22.0, shown, |ui, range| body(ui, range));
    } else {
        body(ui, 0..shown);
    }
    if let Some(rid) = clicked {
        // `select-result-reaction`.
        shell.select_and_center(&rid);
    }
}

fn cell(ui: &mut Ui, width: f32, add: impl FnOnce(&mut Ui)) {
    ui.allocate_ui_with_layout(Vec2::new(width, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_width(width);
        ui.set_max_width(width);
        add(ui);
    });
}

/// `reproducibilityReport(store)`.
fn reproducibility_report(ui: &mut Ui, shell: &Shell, p: &Palette) {
    let Some(state) = shell.store.reproducibility_replay.as_ref() else {
        widgets::empty(ui, "Import a reproducibility bundle to replay it.");
        return;
    };
    let r = &state["replay"];
    let has_replay = !r.is_null();
    let integrity = state.get("integrity").cloned().unwrap_or(Value::Null);
    let truthy = |v: &Value| js::truthy(v);
    let check = |key: &str| truthy(r.get("checks").and_then(|c| c.get(key)).unwrap_or(&Value::Null));
    let num = |v: Option<&Value>| v.map_or(0.0, js::to_number);
    let exp = |x: f64, d: usize| coxeter_state::util::to_exponential(x, d);
    let err = |x: Option<&Value>| -> String {
        let Some(x) = x.filter(|x| truthy(x)) else { return String::new() };
        let mut s = format!("max |Δ| {} · max rel {}", exp(num(x.get("maxAbsError")), 2), exp(num(x.get("maxRelativeError")), 2));
        if let Some(w) = x.get("worstKey").filter(|w| truthy(w)) {
            s.push_str(&format!(" · worst: {}", js::to_string(w)));
        }
        if let Some(m) = x.get("missing").filter(|m| truthy(m)) {
            s.push_str(&format!(" · missing: {}", js::to_string(m)));
        }
        s
    };
    let models: Vec<bool> = integrity.get("modelChecks").and_then(Value::as_object).map(|m| m.values().map(js::truthy).collect()).unwrap_or_default();
    let models_ok = models.iter().all(|b| *b);
    let restored = state.get("restored");
    let map_ok = restored.and_then(|x| x.get("map")).is_some_and(js::truthy);
    let omics_expected = restored.and_then(|x| x.get("omicsExpected")).is_some_and(js::truthy);
    let omics_ok = !omics_expected || restored.and_then(|x| x.get("omics")).is_some_and(js::truthy);
    let payload_valid = integrity.get("payloadValid").is_some_and(js::truthy);
    let archived_status = r.get("status").and_then(|s| s.get("archived")).and_then(Value::as_str).unwrap_or("not recorded");
    let current_status = r.get("status").and_then(|s| s.get("current")).and_then(Value::as_str)
        .unwrap_or_else(|| shell.store.solution.as_ref().map_or("not recorded", |s| s.status.as_str()));
    let objective_number = |key: &str| r.get("objective").and_then(|o| o.get(key)).and_then(Value::as_f64).filter(|v| v.is_finite());
    let has_numbers = has_replay && (objective_number("archived").is_some() || objective_number("current").is_some()
        || r.get("fluxes").and_then(|f| f.get("count")).and_then(Value::as_u64).unwrap_or(0) > 0);
    let pass = (!has_replay || r.get("pass").is_some_and(js::truthy)) && integrity.get("valid").is_some_and(js::truthy);
    let heading = if !pass {
        "Reproducibility warning"
    } else if !has_replay {
        "Inputs restored"
    } else if current_status != "optimal" {
        "Analysis status reproduced"
    } else {
        "Reproducibility verified"
    };
    let (fill, line) = if pass { (p.success_bg, p.success_line) } else { (p.danger_bg, p.danger_line) };
    widgets::tinted(ui, fill, line, |ui| {
        ui.label(RichText::new(format!("{} {heading}", if pass { "✓" } else { "✕" })).family(bold()));
    });
    let objective = objective_number("absError").map(|value| format!("|Δ| {}", exp(value, 2))).unwrap_or_else(|| "Unavailable for comparison".into());
    let mut rows: Vec<(&str, bool, String)> = vec![
        ("Bundle integrity", payload_valid, if payload_valid { "SHA-256 payload verified".into() } else { "SHA-256 payload mismatch".into() }),
        ("Models", models_ok, format!("{} embedded model hash(es) verified", models.len())),
        ("Map restored", map_ok, if map_ok { "Exact archived EditableMap restored".into() } else { "No archived map restored".into() }),
        (
            "Omics restored",
            omics_ok,
            if omics_expected {
                if omics_ok { "Archived omics dataset restored".into() } else { "Expected omics dataset missing".into() }
            } else {
                "No omics dataset archived".into()
            },
        ),
    ];
    if has_replay {
        rows.push(("Solver status", check("status"), format!("Archived: {archived_status} · Replayed: {current_status}")));
    }
    if has_numbers {
        rows.extend([
            ("Objective", check("objective"), objective),
            ("Fluxes", check("fluxes"), err(r.get("fluxes"))),
            ("Reduced costs", check("reducedCosts"), err(r.get("reducedCosts"))),
            ("Scaled RC", check("scaledReducedCosts"), err(r.get("scaledReducedCosts"))),
            ("Shadow prices", check("shadowPrices"), err(r.get("shadowPrices"))),
        ]);
    }
    for (label, ok, detail) in rows {
        ui.horizontal(|ui| {
            ui.label(RichText::new(if ok { "✓" } else { "✕" }).family(bold()).color(if ok { p.ok } else { p.danger }));
            ui.vertical(|ui| {
                ui.label(RichText::new(label).size(12.0).family(bold()));
                if !detail.is_empty() {
                    ui.label(RichText::new(detail).size(11.0).color(p.ink_3));
                }
            });
        });
    }
    if !has_replay {
        widgets::hint(ui, "No numerical solution was archived. Restored inputs were recalculated.");
    } else if current_status != "optimal" {
        widgets::hint(ui, "No optimal solution was obtained.");
    }
    if has_numbers {
        let tolerances = r.get("tolerances");
        widgets::hint(
            ui,
            format!(
                "Tolerance: absolute ≤ {} or relative ≤ {}.",
                exp(num(tolerances.and_then(|t| t.get("absolute"))), 1),
                exp(num(tolerances.and_then(|t| t.get("relative"))), 1)
            ),
        );
    }
}

/// The Logs tab.
fn logs(ui: &mut Ui, shell: &mut Shell, p: &Palette) {
    let store = &shell.store;
    let mode = results_mode(shell);
    let status = store.solution.as_ref().map_or_else(|| "Not run".to_string(), |s| if s.is_optimal() { "Optimal".into() } else { s.status.as_str().to_string() });
    let line = |ui: &mut Ui, tag: &str, text: String| {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(tag).size(11.0).family(bold()).color(p.ink_3));
            ui.label(RichText::new(text).size(12.0));
        });
    };
    line(ui, "Now", format!("{mode} · {status} · {} ms", js::number_to_string(store.status().solve_ms)));
    if store.fva_enabled {
        let progress = store.fva_progress;
        line(
            ui,
            "FVA",
            if progress.running { format!("Computing {}/{}", progress.done, progress.total) } else { format!("Available for {} reactions", store.fva_ranges.len()) },
        );
    }
    if let Some(d) = store.solution.as_ref().and_then(|s| s.result.as_ref()).map(|r| &r.dual_analysis).filter(|d| d.available) {
        line(ui, "Duals", d.interpretation.clone().unwrap_or_else(|| "optimal LP basis".into()));
    }
    render_log_entries(ui, shell, p);
}

/// `renderLogEntries(log)`: newest first, each with when it last happened.
fn render_log_entries(ui: &mut Ui, shell: &mut Shell, p: &Palette) {
    if shell.store.log.is_empty() {
        widgets::note(ui, "No messages yet in this session.");
        return;
    }
    let n = shell.store.log.len();
    let mut clear = false;
    Frame::new().stroke(Stroke::new(1.0, p.line)).corner_radius(CornerRadius::same(8)).inner_margin(Margin::same(8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("History · {n} message{}", if n == 1 { "" } else { "s" })).size(11.5).family(bold()).color(p.ink_2));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if Btn::new("Clear").look(Look::Secondary).height(22.0).show(ui).clicked() {
                    clear = true;
                }
            });
        });
        for entry in shell.store.log.iter().rev() {
            let color = match entry.level {
                coxeter_state::StatusState::Error => p.danger,
                coxeter_state::StatusState::Warn => p.warn,
                coxeter_state::StatusState::Ok => p.ok,
                _ => p.ink_3,
            };
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(RichText::new(local_time(entry.time)).size(11.0).color(p.ink_3));
                ui.label(RichText::new(entry.level.as_str()).size(10.5).family(bold()).color(color));
                ui.label(RichText::new(&entry.message).size(12.0));
                if entry.count > 1 {
                    ui.label(RichText::new(format!("×{}", entry.count)).size(10.5).color(p.ink_3));
                }
            });
        }
    });
    if clear {
        shell.store.clear_log();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coxeter_core::model::Model;
    use coxeter_state::omics::CommunityOmicsPatch;
    use crate::files::ScriptedDialogs;

    #[test]
    fn gimme_is_named_in_the_persistent_results_bar() {
        let mut store = coxeter_state::Store::with_dir(coxeter_core::testkit::repo_root());
        store.init().expect("bundled models");
        let mut raw: Value = serde_json::from_str(include_str!("../../../../../models/ecoli_core_mc.json")).expect("model");
        for reaction in raw["reactions"].as_array_mut().expect("reactions") {
            if reaction["id"] == "GAPD" {
                reaction["gene_reaction_rule"] = serde_json::json!("test_gap");
            }
        }
        store.models.insert("ecoli_core_mc", Model::from_value(&raw).expect("annotated model"));
        store.use_scenario("solo-ecoli").expect("scenario");
        store.import_metatranscriptomics("MAG,gene,condition,TPM\neco,test_gap,Control,1\n", "test-genes.csv").expect("expression");
        store.run_community_gimme(CommunityOmicsPatch::default()).expect("GIMME");
        let mut shell = Shell::new(store, Box::new(ScriptedDialogs::new()));
        assert!(run_summary_text(&shell).0.starts_with("GIMME · optimal"));
        shell.store.run_analysis_blocking("fba", std::time::Duration::from_secs(20)).expect("FBA");
        assert!(run_summary_text(&shell).0.contains("FBA completed"));
    }

    #[test]
    fn a_secondary_optimization_failure_stays_visible_in_the_results_bar() {
        let mut store = coxeter_state::Store::with_dir(coxeter_core::testkit::repo_root());
        store.init().expect("bundled models");
        store.use_scenario("solo-ecoli").expect("scenario");
        store.set_reaction_bounds("eco@BIOMASS_Ec_core", 0.5, 0.5).expect("fixed biomass");
        store.set_pfba_biomass_percent(90.0).expect("target");
        store.set_parsimonious(true).expect("pFBA");
        let mut shell = Shell::new(store, Box::new(ScriptedDialogs::new()));
        let (text, state) = run_summary_text(&shell);
        assert_eq!(state, "warn");
        assert!(text.contains("FBA solution is shown"));
        shell.store.run_analysis_blocking("pfba", std::time::Duration::from_secs(20)).expect("pFBA request");
        let (text, state) = run_summary_text(&shell);
        assert_eq!(state, "error");
        assert!(text.starts_with("pFBA did not finish:"));
        assert!(text.contains("FBA solution is shown"));
    }
}

#[cfg(test)]
mod archive_outcome_ui_tests {
    use super::*;
    use crate::files::ScriptedDialogs;

    fn make_shell() -> Shell {
        let mut store = coxeter_state::Store::with_dir(coxeter_core::testkit::repo_root());
        store.init().expect("bundled models");
        store.use_scenario("solo-ecoli").expect("scenario");
        Shell::new(store, Box::new(ScriptedDialogs::new()))
    }

    fn report_text(shell: &Shell) -> String {
        let context = egui::Context::default();
        crate::fonts::install(&context);
        let mut output = context.run_ui(egui::RawInput::default(), |ui| {
            let palette = Palette::current(ui.ctx());
            reproducibility_report(ui, shell, &palette);
        });
        output.textures_delta.clear();
        let mut text = Vec::new();
        for clipped in output.shapes {
            if let egui::Shape::Text(shape) = clipped.shape {
                text.push(shape.galley.job.text.clone());
            }
        }
        text.join("\n")
    }

    #[test]
    fn an_input_only_report_verifies_restoration_without_claiming_a_numerical_replay() {
        let mut shell = make_shell();
        shell.store.clear_solution();
        let bundle = shell.store.reproducibility_bundle().expect("bundle");
        shell.store.import_reproducibility_bundle(&bundle).expect("restore inputs");
        assert!(tab_state(&shell).replay_available);
        let text = report_text(&shell);
        assert!(text.contains("Inputs restored"), "{text}");
        assert!(text.contains("No numerical solution was archived"), "{text}");
        assert!(!text.contains("Reproducibility verified"));
        assert!(!text.contains("Reproducibility warning"));
    }

    #[test]
    fn the_replay_view_names_an_infeasible_result_without_verifying_nonexistent_numerical_values() {
        let mut shell = make_shell();
        shell.store.set_medium_limit("glc__D", 0.0).expect("no glucose");
        shell.store.set_medium_limit("o2", 0.0).expect("no oxygen");
        let bundle = shell.store.reproducibility_bundle().expect("bundle");
        shell.store.import_reproducibility_bundle(&bundle).expect("replay failed result");
        let text = report_text(&shell);
        assert!(text.contains("Analysis status reproduced"), "{text}");
        assert!(text.contains("Solver status"), "{text}");
        assert!(text.contains("infeasible"));
        assert!(text.contains("No optimal solution was obtained"));
        assert!(!text.contains("Reproducibility verified"));
        assert!(!text.lines().any(|line| line == "Fluxes"));
    }

    #[test]
    fn changing_the_medium_removes_the_imported_verdict_and_disables_its_tab() {
        let mut shell = make_shell();
        let bundle = shell.store.reproducibility_bundle().expect("bundle");
        shell.store.import_reproducibility_bundle(&bundle).expect("replay");
        assert!(report_text(&shell).contains("Reproducibility verified"));
        shell.store.set_medium_limit("glc__D", 5.0).expect("medium edit");
        assert!(!tab_state(&shell).replay_available);
        let text = report_text(&shell);
        assert!(text.contains("Import a reproducibility bundle to replay it."));
        assert!(!text.contains("Reproducibility verified"));
    }
}
