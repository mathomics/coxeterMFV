//! The cross-feeding explorer (port of `crossFeedingCard`,
//! `crossFeedingTopControl`, `crossFeedingMetabolitePicker`,
//! `crossFeedingLegend`, `exchangeNames` and `renderCrossFeedingEvents` of
//! `src/ui/panels.js`),
//! with the handlers of `onSidebarClick` / `onSidebarInput`
//! (`open-crossfeeding-explorer`, `close-crossfeeding-explorer`,
//! `crossfeeding-*`, `clear-crossfeeding-filters`,
//! `select-crossfeeding-event`, `crossfeed-*`).

use coxeter_core::js;
use coxeter_core::mapdata::NodeKind;
use coxeter_render::crossfeed_colors::cross_feeding_colors;
use coxeter_render::crossfeed_routes::CROSSFEED_WIDTH_RANGE;
use coxeter_render::theme::format_flux;
use coxeter_state::{Store, ViewPatch};
use coxeter_state::cross_feeding::ExplorerPatch;
use egui::{Color32, CornerRadius, Frame, Margin, RichText, Sense, Stroke, Ui, Vec2};
use indexmap::IndexMap;

use crate::app::Shell;
use crate::theme::{Palette, bold, css_color};
use crate::ui_state::Workspace;
use crate::widgets::{self, Btn, Field, FieldEvent};

/// How many events the list shows (`events.slice(0, 50)`).
const EVENT_LIMIT: usize = 50;

/// The name of a species' cell (`nameOf`): its model's cell name, or its id.
fn name_of(store: &Store, id: &str) -> String {
    store
        .species
        .iter()
        .find(|s| s.id == id)
        .and_then(|s| store.models.get(&s.file))
        .map_or_else(|| id.to_string(), |m| m.cell.name.clone())
}

/// `exchangeNames(store)`: the names the map gives the shared-medium
/// metabolites, by base id.
pub fn exchange_names(store: &Store) -> IndexMap<String, String> {
    let mut names = IndexMap::new();
    let Some(map) = store.layout() else { return names };
    for node in map.nodes().values() {
        if node.kind != NodeKind::Medium {
            continue;
        }
        // `/^M@(.+)_s$/`
        let merged = node.merged_id.as_deref().unwrap_or("");
        let base = match merged.strip_prefix("M@").and_then(|rest| rest.strip_suffix("_s")).filter(|b| !b.is_empty()) {
            Some(b) => Some(b.to_string()),
            None => node.base.clone(),
        };
        if let (Some(base), Some(name)) = (base.filter(|b| !b.is_empty()), node.name.as_ref().filter(|n| !n.is_empty())) {
            names.entry(base).or_insert_with(|| name.clone());
        }
    }
    names
}

/// `crossFeedingCard(store)`.
pub fn cross_feeding_card(ui: &mut Ui, shell: &mut Shell) {
    if !shell.store.cross_feeding_explorer.enabled {
        let n = shell.store.cross_feeding.links.iter().filter(|l| l.from.as_deref().is_some_and(|f| !f.is_empty()) && l.to.as_deref().is_some_and(|t| !t.is_empty())).count();
        widgets::card(ui, "Cross-Feeding Explorer", None, |ui| {
            widgets::hint(ui, format!("{n} active producer→consumer event(s) detected in the current flux solution."));
            ui.add_space(4.0);
            if Btn::new("Explore cross-feeding").wide().show(ui).clicked() {
                // `open-crossfeeding-explorer`.
                shell.open_workspace(Workspace::Crossfeeding);
                shell.store.set_cross_feeding_explorer(ExplorerPatch { enabled: Some(true), ..Default::default() });
            }
            cross_feeding_metabolite_picker(ui, shell);
        });
        return;
    }
    let p = shell.palette();
    Frame::new().fill(p.panel).stroke(Stroke::new(1.0, p.line)).corner_radius(CornerRadius::same(12)).inner_margin(Margin::same(16)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new("Cross-Feeding Explorer").size(13.0).family(bold()).color(p.ink));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if Btn::new("×").aria("Close cross-feeding explorer").show(ui).clicked() {
                    // `close-crossfeeding-explorer`.
                    shell.store.cross_feeding_explorer.selected_id = None;
                    shell.store.set_cross_feeding_explorer(ExplorerPatch { enabled: Some(false), ..Default::default() });
                }
            });
        });
        ui.add_space(6.0);
        if shell.store.cross_feeding_explorer.enabled {
            explorer(ui, shell, &p);
        }
        cross_feeding_metabolite_picker(ui, shell);
    });
}

/// The explorer's filters, the picked event and the events.
fn explorer(ui: &mut Ui, shell: &mut Shell, p: &Palette) {
    let x = shell.store.cross_feeding_explorer.clone();
    let visible = shell.store.cross_feeding_explorer_events();
    let total = x.index.as_ref().map_or(0, |i| i.events.len());
    let count = visible.len().to_string();
    let rest = format!(" of {total} active events · flux solution evidence.");
    widgets::hint_rich(ui, &["", count.as_str(), rest.as_str()]);
    ui.add_space(4.0);

    // `crossfeeding-query`: live, on every keystroke.
    let (_, event) = widgets::text_field(
        ui,
        egui::Id::new("crossfeeding-query"),
        &x.query,
        Field::new().placeholder("Search metabolite or species").aria("Search metabolite or species"),
    );
    if let Some(text) = event.value() {
        if shell.store.cross_feeding_explorer.query != text {
            shell.store.set_cross_feeding_explorer(ExplorerPatch { query: Some(text), ..Default::default() });
        }
    }

    let species = x.index.as_ref().map(|i| i.species.clone()).unwrap_or_default();
    let with_all = |all: &str, values: Vec<(String, String)>| -> Vec<(String, String)> {
        std::iter::once((String::new(), all.to_string())).chain(values).collect()
    };
    let metabolites = with_all("All metabolites", x.index.as_ref().map(|i| i.metabolites.iter().map(|m| (m.clone(), m.clone())).collect()).unwrap_or_default());
    let producers = with_all("All producers", species.iter().map(|s| (s.clone(), name_of(&shell.store, s))).collect());
    let consumers = with_all("All consumers", species.iter().map(|s| (s.clone(), name_of(&shell.store, s))).collect());
    ui.add_space(8.0);
    let mut patch = ExplorerPatch::default();
    let half = ((ui.available_width() - 6.0) / 2.0).max(80.0);
    egui::Grid::new("crossfeed-filters").num_columns(2).spacing(Vec2::new(6.0, 6.0)).show(ui, |ui| {
        if let Some(v) = widgets::select(ui, "crossfeeding-metabolite", &x.metabolite, &metabolites, half - 8.0, true, "Metabolite filter") {
            patch.metabolite = Some(v);
        }
        if let Some(v) = widgets::select(ui, "crossfeeding-producer", &x.producer, &producers, half - 8.0, true, "Producer filter") {
            patch.producer = Some(v);
        }
        ui.end_row();
        if let Some(v) = widgets::select(ui, "crossfeeding-consumer", &x.consumer, &consumers, half - 8.0, true, "Consumer filter") {
            patch.consumer = Some(v);
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            ui.label(RichText::new("Min flux").size(11.0).color(p.ink_2));
            let width = (half - 60.0).max(48.0);
            let (_, event) = widgets::number_field(
                ui,
                egui::Id::new("crossfeeding-min-flux"),
                &js::number_to_string(x.min_flux),
                Field::new().width(width).aria("Min flux"),
            );
            // `crossfeeding-min-flux`: `Number(el.value) || 0`, live.
            if let Some(text) = event.value() {
                let n = js::string_to_number(&text);
                patch.min_flux = Some(if n.is_nan() || n == 0.0 { 0.0 } else { n });
            }
        });
        ui.end_row();
    });
    if patch != ExplorerPatch::default() {
        shell.store.set_cross_feeding_explorer(patch);
    }

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if Btn::new("Clear filters").show(ui).clicked() {
            // `clear-crossfeeding-filters`.
            shell.store.cross_feeding_explorer.selected_id = None;
            shell.store.set_cross_feeding_explorer(ExplorerPatch {
                query: Some(String::new()),
                metabolite: Some(String::new()),
                producer: Some(String::new()),
                consumer: Some(String::new()),
                min_flux: Some(0.0),
                ..Default::default()
            });
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new("mmol·gDW community⁻¹·h⁻¹").size(12.0).color(p.ink_2));
        });
    });

    let x = shell.store.cross_feeding_explorer.clone();
    let selected = x.index.as_ref().and_then(|i| i.events.iter().find(|e| Some(&e.id) == x.selected_id.as_ref())).cloned();
    if let Some(selected) = selected {
        ui.add_space(8.0);
        Frame::new().stroke(Stroke::new(1.0, p.accent)).corner_radius(CornerRadius::same(8)).inner_margin(Margin::same(8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 3.0;
            let title = format!("{} → {} → {}", name_of(&shell.store, &selected.from), selected.base, name_of(&shell.store, &selected.to));
            ui.add(egui::Label::new(RichText::new(title).family(bold()).color(p.ink)).wrap());
            let small = format!(
                "Producer transfer: {} · Consumer transfer: {}",
                selected.producer_transfer.as_deref().unwrap_or("—"),
                selected.consumer_transfer.as_deref().unwrap_or("—")
            );
            ui.add(egui::Label::new(RichText::new(small).size(11.0).color(p.ink_2)).wrap());
        });
    }
    ui.add_space(8.0);
    render_cross_feeding_events(ui, shell, p);
}

/// `renderCrossFeedingEvents(store)`, with `select-crossfeeding-event`.
fn render_cross_feeding_events(ui: &mut Ui, shell: &mut Shell, p: &Palette) {
    let events = shell.store.cross_feeding_explorer_events();
    let selected_id = shell.store.cross_feeding_explorer.selected_id.clone();
    if events.is_empty() {
        widgets::empty(ui, "No events match these filters.");
    }
    let mut picked = None;
    ui.spacing_mut().item_spacing.y = 4.0;
    for event in events.iter().take(EVENT_LIMIT) {
        let from = name_of(&shell.store, &event.from);
        let to = name_of(&shell.store, &event.to);
        let is_selected = selected_id.as_deref() == Some(event.id.as_str());
        let response = Frame::new()
            .fill(p.panel)
            .stroke(if is_selected { Stroke::new(2.0, p.accent) } else { Stroke::new(1.0, p.line) })
            .corner_radius(CornerRadius::same(7))
            .inner_margin(Margin::symmetric(8, 7))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(format_flux(Some(event.flux), 3)).family(bold()).color(p.ink));
                        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            let mut job = egui::text::LayoutJob::default();
                            let strong = egui::TextFormat { font_id: egui::FontId::new(12.5, bold()), color: p.ink, ..Default::default() };
                            let plain = egui::TextFormat { font_id: egui::FontId::proportional(12.5), color: p.ink, ..Default::default() };
                            job.append(&from, 0.0, strong.clone());
                            job.append(" → ", 0.0, plain);
                            job.append(&to, 0.0, strong);
                            ui.add(egui::Label::new(job).wrap());
                            ui.label(RichText::new(&event.base).size(11.0).color(p.ink.gamma_multiply(0.72)));
                        });
                    });
                });
            })
            .response;
        let response = ui.interact(response.rect, ui.id().with(("crossfeed-event", &event.id)), Sense::click());
        let label = format!("{from} → {} → {to}", event.base);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
        if response.hovered() {
            ui.painter().rect_stroke(response.rect, CornerRadius::same(7), Stroke::new(1.0, p.accent_line), egui::StrokeKind::Inside);
        }
        if response.clicked() {
            picked = Some(event.id.clone());
        }
    }
    if events.len() > EVENT_LIMIT {
        widgets::hint(ui, "Showing first 50 events.");
    }
    if let Some(id) = picked {
        shell.store.cross_feeding_explorer.selected_at = None;
        shell.store.select_cross_feeding_event(&id);
    }
}

/// `crossFeedingTopControl(store)`: how many exchanged metabolites the map
/// shows (in the View menu).
pub fn cross_feeding_top_control(ui: &mut Ui, shell: &mut Shell) {
    let all = shell.store.cross_feeding_metabolites();
    if all.is_empty() {
        return;
    }
    let p = shell.palette();
    let custom = shell.store.view.cross_feeding_shown.is_some();
    let shown = shell.store.cross_feeding_visible_bases().len();
    let value = if custom { shown.to_string() } else { js::number_to_string(shell.store.view.cross_feeding_top) };
    ui.add_space(10.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(RichText::new("Exchanged metabolites shown").size(12.5).color(p.ink_2)).on_hover_text(
            "The most important exchanged metabolites: short-chain fatty acids and lactate first, then carbon sources, then the rest by flux. 0 shows them all",
        );
        let (_, event) = widgets::number_field(
            ui,
            egui::Id::new("crossfeed-top"),
            &value,
            Field::new().width(56.0).aria("Number of exchanged metabolites shown"),
        );
        // `crossfeed-top`: only once the number is settled; a repaint on
        // every keystroke would take the field away.
        if let FieldEvent::Change(text) = event {
            shell.store.set_cross_feeding_top(js::string_to_number(&text));
        }
        let hint = format!("of {}{}", all.len(), if custom { " · picked by hand" } else { "" });
        ui.label(RichText::new(hint).size(12.0).color(p.ink_2));
        if Btn::new("Choose…").title("Pick the metabolites one by one").show(ui).clicked() {
            // `crossfeed-choose`.
            shell.open_workspace(Workspace::Crossfeeding);
            ui.close();
        }
    });
}

/// `crossFeedingWidthControl(store)`: the cross-feeding line width slider, as
/// a share of the default width (in the View menu). As in the browser, the
/// width is taken when the slider is let go; while it is dragged only the
/// read-out follows.
pub fn cross_feeding_width_control(ui: &mut Ui, shell: &mut Shell) {
    if shell.store.cross_feeding_metabolites().is_empty() {
        return;
    }
    let p = shell.palette();
    let range = &CROSSFEED_WIDTH_RANGE;
    let draft_id = egui::Id::new("crossfeed-width-draft");
    let value = ui.data(|d| d.get_temp::<f64>(draft_id)).unwrap_or(shell.store.view.cross_feeding_width);
    let mut commit = None;
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(RichText::new("Cross-feeding line width").size(12.5).color(p.ink_2)).on_hover_text(
            "Thickness of the cross-feeding lines. At 100% a line is about as thick as a cell's boundary with the whole map in view; lines side by side keep apart at any width",
        );
        ui.spacing_mut().slider_width = 110.0;
        let (_, event) = widgets::slider(ui, value, range.min, range.max, range.step, true, "Cross-feeding line width");
        match event {
            FieldEvent::Input(v) => ui.data_mut(|d| { d.insert_temp(draft_id, v); }),
            FieldEvent::Change(v) => {
                ui.data_mut(|d| d.remove::<f64>(draft_id));
                commit = Some(v);
            }
            FieldEvent::None => {}
        }
        ui.label(RichText::new(format!("{}%", js::number_to_string(js::round(value * 100.0)))).size(12.0).color(p.ink_2));
    });
    if let Some(v) = commit {
        let _ = shell.store.set_view(ViewPatch { cross_feeding_width: Some(v), ..Default::default() });
    }
}

/// `drawnCrossFeedingColors(store)`: the colour of each metabolite whose
/// cross-feeding lines are drawn, as the map gives them.
pub fn drawn_cross_feeding_colors(store: &Store) -> IndexMap<String, &'static str> {
    let shown = store.cross_feeding_visible_bases();
    let bases = store.cross_feeding_metabolites().into_iter().map(|m| m.base).filter(|base| shown.contains(base));
    cross_feeding_colors(bases, store.view.dark)
}

/// `crossFeedingSwatch(color)`: a short bar in a cross-feeding line's colour;
/// hollow for a metabolite not drawn.
fn cross_feeding_swatch(ui: &mut Ui, color: Option<&str>, p: &Palette) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(12.0, 5.0), Sense::hover());
    let fill = color.map_or(Color32::TRANSPARENT, |c| css_color(c, p.ink_3));
    ui.painter().rect(rect, CornerRadius::same(3), fill, Stroke::new(1.0, p.ink.gamma_multiply(0.3)), egui::StrokeKind::Inside);
}

/// `crossFeedingLegend(store)`: the key to the cross-feeding lines on the
/// map, each metabolite drawn with its colour and how many lines it has.
pub fn cross_feeding_legend(ui: &mut Ui, store: &Store) {
    let colors = drawn_cross_feeding_colors(store);
    if colors.is_empty() {
        return;
    }
    let p = Palette::current(ui.ctx());
    let names = exchange_names(store);
    ui.add_space(12.0);
    let top = ui.cursor().top() - 2.0;
    ui.painter().hline(ui.max_rect().x_range(), top, Stroke::new(1.0, p.line));
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Cross-feeding lines").size(11.5).family(bold()).color(p.ink_2));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(colors.len().to_string()).size(10.5).color(p.ink_3));
        });
    });
    for m in store.cross_feeding_metabolites().iter().filter(|m| colors.contains_key(&m.base)) {
        let label = match names.get(&m.base) {
            Some(name) if name != &m.base => format!("{name} ({})", m.base),
            _ => m.base.clone(),
        };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 7.0;
            cross_feeding_swatch(ui, colors.get(&m.base).copied(), &p);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(m.lines.to_string()).size(10.0).family(bold()).color(p.ink_3));
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(egui::Label::new(RichText::new(&label).size(11.0).color(p.ink_2)).truncate()).on_hover_text(&label);
                });
            });
        });
    }
}

/// `crossFeedingMetabolitePicker(store)`: one checkbox per exchanged
/// metabolite, grouped, with its line colour, lines and flux.
fn cross_feeding_metabolite_picker(ui: &mut Ui, shell: &mut Shell) {
    let all = shell.store.cross_feeding_metabolites();
    if all.is_empty() {
        return;
    }
    let p = shell.palette();
    let shown = shell.store.cross_feeding_visible_bases();
    let colors = drawn_cross_feeding_colors(&shell.store);
    let names = exchange_names(&shell.store);
    ui.add_space(12.0);
    ui.separator();
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Metabolites on the map").size(13.0).family(bold()).color(p.ink));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(format!("{} of {}", shown.len(), all.len())).size(12.0).color(p.ink_2));
        });
    });
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if Btn::new("Top 5").show(ui).clicked() {
            // `crossfeed-show-top` (`data-top="5"`).
            shell.store.set_cross_feeding_top(5.0);
        }
        if Btn::new("All").show(ui).clicked() {
            // `crossfeed-show-all`.
            shell.store.set_cross_feeding_top(0.0);
        }
        if Btn::new("None").show(ui).clicked() {
            // `crossfeed-show-none`.
            let _ = shell.store.set_view(coxeter_state::ViewPatch { cross_feeding_shown: Some(Some(Vec::new())), ..Default::default() });
        }
    });
    let mut group = "";
    let mut toggled = None;
    for m in &all {
        if m.group != group {
            ui.add_space(6.0);
            ui.label(RichText::new(m.group.to_uppercase()).size(11.0).family(bold()).color(p.ink.gamma_multiply(0.7)));
        }
        group = m.group;
        let label = match names.get(&m.base) {
            Some(name) if name != &m.base => format!("{name} ({})", m.base),
            _ => m.base.clone(),
        };
        ui.horizontal(|ui| {
            // The whole row is the checkbox's <label>: the swatch sits between
            // the box and the name, and the name toggles it too.
            let checked = shown.contains(&m.base);
            let (_, changed) = widgets::checkbox(ui, checked, "", true, Some(&label));
            cross_feeding_swatch(ui, colors.get(&m.base).copied(), &p);
            let name = ui.add(egui::Label::new(RichText::new(&label).size(12.0).color(p.ink)).sense(Sense::click()));
            if let Some(on) = changed.or_else(|| name.clicked().then_some(!checked)) {
                toggled = Some((m.base.clone(), on));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let small = format!("{} line{} · {}", m.lines, if m.lines == 1 { "" } else { "s" }, format_flux(Some(m.flux), 3));
                ui.label(RichText::new(small).size(11.0).color(p.ink_2));
            });
        });
    }
    if let Some((base, on)) = toggled {
        // `crossfeed-metabolite`.
        shell.store.toggle_cross_feeding_metabolite(&base, on);
    }
    widgets::hint(
        ui,
        "A line between two species splits a metabolite in proportion to what each produces and takes up; FBA does not say which cell feeds which.",
    );
}
