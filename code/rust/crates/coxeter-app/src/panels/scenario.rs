//! The Scenario & objective card (port of `scenarioCard`,
//! `objectiveControls`, `formatPercent`, `looplessProgressLine`,
//! `fvaProgressLine`, `publishedCommunityDetail`, `speciesBlock`,
//! `zunigaCaseBlock`, `createGrowthForCase`, `conditionsBlock`,
//! `resultsBlock`, `steadyComBlock`, `sweepFigure`, `sweepPanel`,
//! `sweepTable` and `communityProvenanceBlock` of `src/ui/panels.js`), with
//! the handlers of `onSidebarClick` / `onSidebarInput` for its controls and
//! `runSteadyCom`, `clearSteadyComError`, `showSteadyComError`,
//! `validateSteadyComFiber` and `patchSteadyComControls` of `src/ui/app.js`.
//!
//! The JS repaints the panel from HTML and keeps the growth rate being typed
//! in `ui.steadyComDraft`; here the panel is drawn each frame from the store
//! and the same draft (`shell.ui.steady_com_draft`), so what
//! `patchSteadyComControls` patches in place is simply drawn as it is.

use coxeter_core::js;
use coxeter_render::format_flux;
use coxeter_state::store::objectives;
use coxeter_state::zuniga::{ZUNIGA_ID, ZUNIGA_SCENARIOS};
use coxeter_state::{JobKind, SteadyComRequest, Store, StoreError};
use egui::text::LayoutJob;
use egui::{Align2, Color32, CornerRadius, FontFamily, FontId, Frame, Margin, Response, RichText, Sense, Stroke, TextFormat, Ui, Vec2, pos2};
use indexmap::IndexMap;
use serde_json::Value;

use crate::app::{Deferred, Shell};
use crate::panels::left::{current_analysis_method, format_percent};
use crate::theme::{Palette, bold, to_hex};
use crate::ui_state::SteadyComDraft;
use crate::widgets::{self, Btn, Field, FieldEvent};

// Helpers

/// A paragraph of runs: plain, bold or coloured text (`<b>`, `<strong>`,
/// `.hint` spans inside one `<p>`).
pub(crate) struct Rich {
    job: LayoutJob,
    size: f32,
    ink: Color32,
    strong: Color32,
}

impl Rich {
    pub(crate) fn new(ui: &Ui, size: f32) -> Rich {
        let p = Palette::current(ui.ctx());
        Rich { job: LayoutJob::default(), size, ink: p.ink_2, strong: p.ink }
    }
    /// The colours of `.hint` (the default) or of body text.
    pub(crate) fn ink(mut self, ink: Color32, strong: Color32) -> Rich {
        self.ink = ink;
        self.strong = strong;
        self
    }
    pub(crate) fn plain(self, s: &str) -> Rich {
        let c = self.ink;
        self.run(s, c, false)
    }
    pub(crate) fn bold(self, s: &str) -> Rich {
        let c = self.strong;
        self.run(s, c, true)
    }
    pub(crate) fn color(self, s: &str, c: Color32) -> Rich {
        self.run(s, c, false)
    }
    pub(crate) fn run(mut self, s: &str, color: Color32, strong: bool) -> Rich {
        let family = if strong { bold() } else { FontFamily::Proportional };
        self.job.append(s, 0.0, TextFormat { font_id: FontId::new(self.size, family), color, ..Default::default() });
        self
    }
    pub(crate) fn show(mut self, ui: &mut Ui) -> Response {
        self.job.wrap.max_width = ui.available_width();
        ui.add(egui::Label::new(self.job).wrap())
    }
}

/// `aria-invalid="true"` on a field, for assistive technology and tests.
pub(crate) fn aria_invalid(response: &Response, invalid: bool) {
    if invalid {
        response.ctx.accesskit_node_builder(response.id, |node| node.set_invalid(egui::accesskit::Invalid::True));
    }
}

/// `<h2>` of a `.sc-col`.
fn column_title(ui: &mut Ui, s: &str) {
    let p = Palette::current(ui.ctx());
    ui.label(RichText::new(s).size(13.0).family(bold()).color(p.ink));
    ui.add_space(4.0);
}

/// `.pc-detail h3`: small capitals over a block.
fn pc_heading(ui: &mut Ui, s: &str) {
    let p = Palette::current(ui.ctx());
    ui.add_space(8.0);
    ui.label(RichText::new(s.to_uppercase()).size(11.0).family(bold()).color(p.ink_2).extra_letter_spacing(0.6));
    ui.add_space(2.0);
}

/// `.sc-label`.
fn sc_label(ui: &mut Ui, s: &str) {
    let p = Palette::current(ui.ctx());
    ui.label(RichText::new(s).size(11.5).color(p.ink_2));
}

/// A `.pc-list` item: a bullet and its runs.
fn bullet(ui: &mut Ui, add: impl FnOnce(Rich) -> Rich) -> Response {
    let p = Palette::current(ui.ctx());
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        ui.label(RichText::new("•").size(12.0).color(p.ink_3));
        add(Rich::new(ui, 12.0).ink(p.ink, p.ink)).show(ui)
    })
    .inner
}

/// `.pc-dl`: the small pill of a download.
fn pc_dl(ui: &mut Ui, label: &str, enabled: bool, title: &str, aria: &str) -> Response {
    let p = Palette::current(ui.ctx());
    let fill = p.accent.gamma_multiply(0.1);
    let line = p.accent.gamma_multiply(0.35);
    let response = ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(label).size(10.0).family(bold()).color(p.accent))
            .fill(fill)
            .stroke(Stroke::new(1.0, line))
            .corner_radius(CornerRadius::same(99))
            .min_size(Vec2::new(0.0, 18.0)),
    );
    widgets::aria_label(&response, aria);
    if title.is_empty() {
        response
    } else if enabled {
        response.on_hover_text(title)
    } else {
        response.on_disabled_hover_text(title)
    }
}

/// `.pc-dl.is-off`: a member without a model file.
fn pc_dl_off(ui: &mut Ui, title: &str) {
    let p = Palette::current(ui.ctx());
    Frame::new()
        .stroke(Stroke::new(1.0, p.line))
        .corner_radius(CornerRadius::same(99))
        .inner_margin(Margin::symmetric(7, 1))
        .show(ui, |ui| ui.label(RichText::new("—").size(10.0).family(bold()).color(p.ink_3)))
        .response
        .on_hover_text(title);
}

/// `String(value)` of a JSON value (`${x}` in a template).
fn text_of(v: Option<&Value>) -> String {
    v.filter(|v| !v.is_null()).map(js::to_string).unwrap_or_default()
}

/// `Number(value)`, NaN for a missing value (`Number(undefined)`).
fn num(v: Option<&Value>) -> f64 {
    v.map_or(f64::NAN, js::to_number)
}

fn list(v: Option<&Value>) -> Vec<Value> {
    v.and_then(Value::as_array).cloned().unwrap_or_default()
}

/// The registry entry of the published community that is open.
fn selected_published(store: &Store) -> Option<Value> {
    let id = store.scenario_published()?;
    store.published_communities.iter().find(|c| c.get("id").and_then(Value::as_str) == Some(id.as_str())).cloned()
}

// `scenarioCard`

/// `.card` without a title: the scenario card has one per column.
fn plain_card(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let p = Palette::current(ui.ctx());
    Frame::new()
        .fill(p.panel)
        .stroke(Stroke::new(1.0, p.line))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        });
}

/// `scenarioCard(store, steadyComUI)`.
pub fn scenario_card(ui: &mut Ui, shell: &mut Shell) {
    plain_card(ui, |ui| {
        let width = ui.available_width();
        let gap = 18.0;
        // `.sc-grid`: two columns (1.15fr | 1fr), stacked on a narrow page.
        if width < 600.0 {
            scenario_column(ui, shell);
            ui.add_space(14.0);
            ui.separator();
            objective_column(ui, shell);
            return;
        }
        let left = (width - gap) * 1.15 / 2.15;
        let right = width - gap - left;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let r = ui
                .allocate_ui_with_layout(Vec2::new(left, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(left);
                    scenario_column(ui, shell);
                })
                .response;
            let p = Palette::current(ui.ctx());
            let x = r.rect.right() + gap / 2.0;
            ui.add_space(gap);
            let column = ui
                .allocate_ui_with_layout(Vec2::new(right, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(right);
                    objective_column(ui, shell);
                })
                .response;
            let bottom = r.rect.bottom().max(column.rect.bottom());
            ui.painter().vline(x, r.rect.top()..=bottom, Stroke::new(1.0, p.line));
        });
    });
}

/// `.sc-scenario`: the community list and what the chosen one shows.
fn scenario_column(ui: &mut Ui, shell: &mut Shell) {
    column_title(ui, "Scenario");
    let store = &shell.store;
    let published: Vec<(String, String)> = store
        .published_communities
        .iter()
        .map(|c| {
            let reproduced = c.pointer("/reproduction/status").and_then(Value::as_str) == Some("reproduced");
            let mark = if reproduced { "✓" } else { "—" };
            (format!("pub:{}", text_of(c.get("id"))), format!("{mark} {}", text_of(c.get("name"))))
        })
        .collect();
    let teaching: Vec<(String, String)> =
        store.scenarios.iter().map(|sc| (format!("sc:{}", text_of(sc.get("id"))), text_of(sc.get("name")))).collect();
    let mut groups = Vec::new();
    if !published.is_empty() {
        groups.push(("Published communities".to_string(), published));
    }
    groups.push(("Teaching scenarios".to_string(), teaching));
    let current = shell.ui.scenario_choice.clone().unwrap_or_else(|| shell.active_scenario_value());
    let width = ui.available_width() - 8.0;
    if let Some(value) = widgets::select_grouped(ui, "scenario-select", &current, &groups, width, "Community") {
        // `scenario`: one asynchronous community rebuild per choice.
        shell.store.cancel_steady_com(false);
        shell.ui.steady_com_draft = None;
        shell.ui.scenario_choice = Some(value.clone());
        shell.queue_scenario_selection(&value);
    }
    match selected_published(&shell.store) {
        Some(c) => published_community_detail(ui, shell, &c),
        None => {
            let description = shell.store.scenario_str("description").unwrap_or_default();
            ui.add_space(6.0);
            widgets::hint(ui, description);
            steady_com_block(ui, shell, None);
        }
    }
}

/// `.sc-objective`.
fn objective_column(ui: &mut Ui, shell: &mut Shell) {
    column_title(ui, "Objective");
    objective_controls(ui, shell);
}

// `objectiveControls`

/// `objectiveControls(store)`.
fn objective_controls(ui: &mut Ui, shell: &mut Shell) {
    let p = Palette::current(ui.ctx());
    if shell.store.is_zuniga_study() {
        let coefficients = shell
            .store
            .problem
            .as_ref()
            .map(|problem| {
                problem.objective().iter().map(|(id, v)| format!("{} × {id}", js::number_to_string(*v))).collect::<Vec<_>>().join(" + ")
            })
            .unwrap_or_default();
        ui.label(widgets::strong("Authors’ original objective").size(12.5).color(p.ink));
        let shown = if coefficients.is_empty() { "Source model objective".to_string() } else { coefficients };
        ui.add(egui::Label::new(widgets::code(shown).color(p.ink_2)).wrap());
        widgets::hint(
            ui,
            "The documented medium, reaction bounds and population coefficients are fixed for this case. Choose a case scenario to compare the WT, mutant and coculture. The drawing remains editable.",
        );
        widgets::hint(ui, "Stationary FBA · GLPK. This comparison does not reproduce dynamic culture trajectories or measured experimental growth rates.");
        return;
    }
    let fixed_mu = shell.store.solution.as_ref().is_some_and(|s| s.steady_com.is_some());

    sc_label(ui, "Community objective");
    let mut objectives = vec![
        (objectives::EQUAL_GROWTH.to_string(), "Equal growth (shared µ)".to_string()),
        (objectives::SUM_BIOMASS.to_string(), "Weighted biomass sum".to_string()),
    ];
    // A select whose value is none of its options shows the first.
    let mut current = if objectives.iter().any(|(v, _)| *v == shell.store.objective) {
        shell.store.objective.clone()
    } else {
        objectives[0].0.clone()
    };
    // An objective edited in the Reactions page is shown as what it is.
    if shell.store.custom_objective.is_some() {
        objectives.insert(0, (String::new(), format!("Edited: {}", shell.store.objective_description())));
        current = String::new();
    }
    let width = ui.available_width() - 8.0;
    // The edited objective is a reading, not a choice.
    let picked = widgets::select(ui, "objective-select", &current, &objectives, width, true, "Community objective").filter(|o| !o.is_empty());
    if let Some(objective) = picked {
        if let Err(error) = shell.store.set_objective(&objective, None) {
            shell.status_error(error.to_string());
        }
    }
    let edit = Btn::new("Edit objective in Reactions ↗").title("Any reaction, any coefficient, maximised or minimised").wide().show(ui);
    if edit.clicked() {
        shell.open_workspace(crate::ui_state::Workspace::Reactions);
    }

    ui.add_space(10.0);
    let label = Rich::new(ui, 12.5).ink(p.ink, p.ink).bold("pFBA").plain(" · Parsimonious Flux Balance Analysis").job;
    let (_, changed) = widgets::checkbox(ui, shell.store.parsimonious, label, true, Some("Use Parsimonious Flux Balance Analysis"));
    if let Some(on) = changed {
        if let Err(error) = shell.store.set_parsimonious(on) {
            shell.status_error(error.to_string());
        }
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new("Target biomass (% of optimum)").size(12.0).color(p.ink_2));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let id = egui::Id::new("pfba-biomass-percent");
            let shown = match shell.deferred.get("pfba-biomass-percent") {
                Some((_, Deferred::PfbaPercent(text))) => text.clone(),
                _ => format_percent(shell.store.pfba_biomass_percent),
            };
            let (_, event) =
                widgets::number_field(ui, id, &shown, Field::new().width(72.0).aria("Percentage of the optimal objective fixed for pFBA"));
            // Both `input` and `change` reach the debounced setter.
            if let Some(text) = event.value() {
                shell.debounce("pfba-biomass-percent", 0.18, Deferred::PfbaPercent(text));
            }
        });
    });
    let method = current_analysis_method(shell);
    let tail = if fixed_mu {
        "The map shows a fixed-µ SteadyCom solution. Run FBA, pFBA or Loopless to return to ordinary analysis.".to_string()
    } else if shell.store.loopless {
        "Loopless uses a MILP to exclude thermodynamically impossible internal cycles.".to_string()
    } else {
        format!("pFBA fixes {}% of the optimum, then minimises total flux.", format_percent(shell.store.pfba_biomass_percent))
    };
    Rich::new(ui, 12.0).plain("Current method: ").bold(&method).plain(&format!(". {tail}")).show(ui);

    ui.add_space(10.0);
    let label = Rich::new(ui, 12.5).ink(p.ink, p.ink).bold("Loopless").plain(" · Loopless FBA").job;
    let (_, changed) = widgets::checkbox(ui, shell.store.loopless, label, true, Some("Enable Loopless Flux Balance Analysis"));
    if let Some(on) = changed {
        if let Err(error) = shell.store.set_loopless(on) {
            shell.status_error(error.to_string());
        }
    }
    loopless_progress_line(ui, shell);

    ui.add_space(10.0);
    let label = Rich::new(ui, 12.5).ink(p.ink, p.ink).bold("FVA").plain(" · Flux Variability Analysis").job;
    let (response, changed) = widgets::checkbox(ui, shell.store.fva_enabled, label, !fixed_mu, Some("Enable Flux Variability Analysis"));
    if fixed_mu {
        response.on_disabled_hover_text("Run an ordinary analysis before enabling FVA");
    }
    if let Some(on) = changed {
        // A fixed-µ result cannot take ordinary FVA.
        if !(fixed_mu && on) {
            if let Err(error) = shell.store.set_fva(on) {
                shell.status_error(error.to_string());
            }
        }
    }
    fva_progress_line(ui, shell);
}

/// `looplessProgressLine(store)`.
fn loopless_progress_line(ui: &mut Ui, shell: &Shell) {
    if !shell.store.loopless {
        widgets::hint(ui, "Removes internal cycles with a MILP. Can be much slower than FBA/pFBA.");
        return;
    }
    let Some(solution) = shell.store.solution.as_ref() else {
        widgets::hint(ui, "Loopless enabled.");
        return;
    };
    if solution.is_optimal() {
        let info = solution.result.as_ref().and_then(|r| r.loopless.as_ref());
        let binaries = info.map_or(0, |i| i.binary_variables).to_string();
        let nodes = info.map_or(0, |i| i.nodes).to_string();
        Rich::new(ui, 12.0)
            .plain("Loopless MILP optimal · ")
            .bold(&binaries)
            .plain(" binaries · ")
            .bold(&nodes)
            .plain(" branch-and-bound nodes.")
            .show(ui);
        return;
    }
    let why = if solution.message.is_empty() { solution.status.as_str().to_string() } else { solution.message.clone() };
    widgets::hint(ui, format!("Loopless: {why}."));
}

/// `fvaProgressLine(store)`.
fn fva_progress_line(ui: &mut Ui, shell: &Shell) {
    if !shell.store.fva_enabled {
        widgets::hint(ui, "Computes the minimum and maximum flux of every reaction while holding the objective at its optimum.");
        return;
    }
    let progress = shell.store.fva_progress;
    if progress.running {
        Rich::new(ui, 12.0)
            .plain("FVA running… ")
            .bold(&format!("{}/{}", progress.done, progress.total))
            .plain(" reactions.")
            .show(ui);
    } else if progress.total > 0 {
        Rich::new(ui, 12.0)
            .plain("FVA done: ")
            .bold(&format!("{}/{}", progress.done, progress.total))
            .plain(&format!(" reactions · {} ms.", js::number_to_string(progress.ms)))
            .show(ui);
    } else {
        widgets::hint(ui, "FVA enabled. It runs on the next solve.");
    }
}

// `publishedCommunityDetail`

/// `publishedCommunityDetail(c, store, steadyComUI)`.
///
/// Two different things are on this card and they are kept apart on
/// purpose: what the paper reported, and what this app computed. A
/// community whose sources could not be retrieved says so rather than
/// showing a plausible number, and anything downloadable is a real file
/// generated from the authors' own distribution, never a button that leads
/// nowhere.
fn published_community_detail(ui: &mut Ui, shell: &mut Shell, c: &Value) {
    let p = Palette::current(ui.ctx());
    let id = text_of(c.get("id"));
    let paper = c.get("paper").cloned().unwrap_or(Value::Null);
    let reproduced = c.pointer("/reproduction/status").and_then(Value::as_str) == Some("reproduced");
    ui.add_space(10.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let accent = (to_hex(p.accent_soft), to_hex(p.accent), to_hex(p.accent_line));
        widgets::chip(ui, &text_of(paper.get("framework")), Some((&accent.0, &accent.1, &accent.2)));
        let (fill, ink, line) = if reproduced {
            (to_hex(p.success_bg), to_hex(p.ok), to_hex(p.success_line))
        } else {
            (to_hex(p.danger_bg), to_hex(p.danger), to_hex(p.danger_line))
        };
        widgets::chip(ui, if reproduced { "Reproduced" } else { "Blocked" }, Some((&fill, &ink, &line)));
    });
    ui.add_space(6.0);
    // `.pc-paper`: title, authors (year), citation, the DOI and whether it
    // was checked.
    ui.add(egui::Label::new(RichText::new(text_of(paper.get("title"))).size(12.0).family(bold()).color(p.ink)).wrap());
    let citation = paper.get("citation").filter(|v| !v.is_null()).or(paper.get("journal"));
    ui.add(
        egui::Label::new(
            RichText::new(format!("{} ({}), {}", text_of(paper.get("authors")), text_of(paper.get("year")), text_of(citation)))
                .size(12.0)
                .color(p.ink),
        )
        .wrap(),
    );
    ui.horizontal_wrapped(|ui| {
        let doi = text_of(paper.get("doi"));
        if !doi.is_empty() {
            ui.hyperlink_to(RichText::new(format!("doi:{doi}")).size(12.0).color(p.accent), format!("https://doi.org/{doi}"));
        }
        // A citation nobody could check offline is marked as such rather
        // than left to look as solid as the ones verified against a
        // retrieved file.
        match paper.get("doiVerifiedFrom").filter(|v| js::truthy(v)) {
            Some(from) => {
                ui.label(RichText::new("VERIFIED").size(10.0).family(bold()).color(p.ok))
                    .on_hover_text(format!("Verified against {}", js::to_string(from)));
            }
            None => {
                ui.label(RichText::new("UNVERIFIED").size(10.0).family(bold()).color(p.danger))
                    .on_hover_text("Could not verify against a retrieved source");
            }
        }
    });
    ui.add_space(4.0);
    widgets::hint(ui, text_of(c.get("description")));
    if id == ZUNIGA_ID {
        zuniga_case_block(ui, shell);
    }
    let assets = shell.store.published_manifests.get(&id).cloned().flatten();
    species_block(ui, shell, c, assets.as_ref());
    conditions_block(ui, shell, c, assets.as_ref());
    results_block(ui, c);
    steady_com_block(ui, shell, Some(c));
    community_provenance_block(ui, shell, c);
}

/// `speciesBlock(c, assets, base, store)`: one row per member, with the
/// model itself behind a download.
fn species_block(ui: &mut Ui, shell: &mut Shell, c: &Value, assets: Option<&Value>) {
    let species = list(c.get("species"));
    if species.is_empty() {
        widgets::empty(ui, "The species list could not be retrieved from the paper.");
        return;
    }
    let p = Palette::current(ui.ctx());
    let community = text_of(c.get("id"));
    let members = list(assets.and_then(|a| a.get("members")));
    let by_id = |id: &str| members.iter().find(|m| m.get("id").and_then(Value::as_str) == Some(id)).cloned();
    // The row is the organism's full name, once. The role line survives only
    // where the organism names repeat inside a community: the four
    // auxotrophs are all "Escherichia coli K-12 MG1655", and what tells them
    // apart is precisely the role.
    let organism = |sp: &Value| {
        let o = text_of(sp.get("organism"));
        if o.is_empty() { text_of(sp.get("name")) } else { o }
    };
    let organisms: Vec<String> = species.iter().map(organism).collect();
    let distinct = organisms.iter().collect::<std::collections::HashSet<_>>().len() == organisms.len();
    let zuniga = community == ZUNIGA_ID;
    let loaded = shell.store.is_zuniga_study();
    pc_heading(ui, "Species");
    for sp in &species {
        let sp_id = text_of(sp.get("id"));
        let asset = by_id(&sp_id);
        let abundance = sp.get("abundance").filter(|v| !v.is_null()).map(|a| format!(" · {} %", js::to_fixed(js::to_number(a) * 100.0, 0)));
        let label = if distinct { organism(sp) } else { format!("{} · {}", organism(sp), text_of(sp.get("name"))) };
        let role = text_of(sp.get("role"));
        let frame = Frame::new().inner_margin(Margin { left: 8, right: 0, top: 1, bottom: 1 });
        let response = frame
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.label(RichText::new(&label).size(12.0).family(bold()).color(p.ink));
                    if let Some(a) = &abundance {
                        ui.label(RichText::new(a).size(12.0).color(p.ink));
                    }
                    let json_aria = format!("Download {sp_id} as COBRA JSON");
                    let sbml_aria = format!("Download {sp_id} as SBML");
                    if zuniga {
                        if pc_dl(ui, "JSON", loaded, "Personal export of the privately loaded complete model", &json_aria).clicked() {
                            shell.download_member_json(&community, &sp_id);
                        }
                        if pc_dl(ui, "SBML", loaded, "", &sbml_aria).clicked() {
                            shell.download_member_sbml(&community, &sp_id);
                        }
                    } else if let Some(asset) = &asset {
                        let title = format!("{} reactions, {} metabolites", text_of(asset.get("reactions")), text_of(asset.get("metabolites")));
                        // A plain link in a served build; here a save dialog either way.
                        if pc_dl(ui, "JSON", true, &format!("{title}, COBRA JSON"), &json_aria).clicked() {
                            shell.download_member_json(&community, &sp_id);
                        }
                        if pc_dl(ui, "SBML", true, &format!("{title}, SBML L3V1 fbc-v2"), &sbml_aria).clicked() {
                            shell.download_member_sbml(&community, &sp_id);
                        }
                    } else {
                        pc_dl_off(ui, "No model file is bundled for this member");
                    }
                });
                if !distinct && !role.is_empty() {
                    ui.add(egui::Label::new(RichText::new(&role).size(12.0).color(p.ink.gamma_multiply(0.85))).wrap());
                }
            })
            .response;
        ui.painter().vline(response.rect.left(), response.rect.y_range(), Stroke::new(2.0, p.line));
        ui.add_space(4.0);
    }
}

/// `conditionsBlock(c, assets, base)`: the medium and the bounds, as files
/// rather than prose.
fn conditions_block(ui: &mut Ui, shell: &mut Shell, c: &Value, assets: Option<&Value>) {
    let id = text_of(c.get("id"));
    if id == ZUNIGA_ID {
        pc_heading(ui, "Source conditions");
        widgets::hint(
            ui,
            "Complete author models are downloaded directly from the pinned public source and verified by SHA-256. They are cached privately on this computer. Coxeter distributes the case specification and calculated results; the original models are not redistributed.",
        );
        widgets::hint(
            ui,
            "The reference medium, original reaction bounds, biomass definitions and population coefficients are preserved. Monoculture and coculture have different source models; differences are documented as adaptations.",
        );
        return;
    }
    let Some(assets) = assets else {
        if let Some(conditions) = c.get("conditions").filter(|v| js::truthy(v)) {
            pc_heading(ui, "Conditions");
            widgets::hint(ui, text_of(conditions.get("medium")));
        }
        return;
    };
    pc_heading(ui, "Conditions");
    let rows = [
        ("Medium", "the exchange reactions the diet opens, with their bounds", "media", "Download the medium CSV"),
        ("Bounds", "every reaction's lower and upper bound and objective coefficient", "bounds", "Download the bounds CSV"),
    ];
    for (name, what, key, aria) in rows {
        let file = text_of(assets.get(key));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let p = Palette::current(ui.ctx());
            ui.label(RichText::new("•").size(12.0).color(p.ink_3));
            ui.label(RichText::new(name).size(12.0).family(bold()).color(p.ink));
            ui.label(RichText::new(format!("— {what}")).size(12.0).color(p.ink));
            if pc_dl(ui, "CSV", true, "", aria).clicked() {
                shell.download_community_csv(&id, &file);
            }
        });
    }
}

/// `resultsBlock(c)`: what the paper reported and what was recorded offline.
fn results_block(ui: &mut Ui, c: &Value) {
    let expected = c.pointer("/reproduction/expected").filter(|v| js::truthy(v));
    let computed = c.get("computed").filter(|v| js::truthy(v));
    if expected.is_none() && computed.is_none() {
        return;
    }
    let p = Palette::current(ui.ctx());
    pc_heading(ui, "Recorded published results");
    let e = |k: &str| expected.and_then(|x| x.get(k)).filter(|v| !v.is_null());
    let k = |k: &str| computed.and_then(|x| x.get(k)).filter(|v| !v.is_null());
    if let Some(rate) = e("growthRate") {
        bullet(ui, |r| r.bold("Paper µ:").plain(&format!(" {} h⁻¹", js::to_string(rate))));
    }
    if let Some(rate) = k("growthRate") {
        let note = format!(" ({} LP, {} s)", text_of(k("lpCount")), text_of(k("seconds")));
        bullet(ui, |r| r.bold("Recorded µ (offline):").plain(&format!(" {} h⁻¹", js::to_string(rate))).color(&note, p.ink_2));
    } else if let Some(range) = k("growthRateRange").filter(|v| js::truthy(v)) {
        let joined = list(Some(range)).iter().map(js::to_string).collect::<Vec<_>>().join(" – ");
        let note = format!(" ({} sweep points)", text_of(k("sweepPoints")));
        bullet(ui, |r| r.bold("Recorded sweep µ:").plain(&format!(" {joined} h⁻¹")).color(&note, p.ink_2));
    } else if let Some(Value::Object(rates)) = k("growthRates") {
        // A joint model whose members grow at different rates: one figure
        // each, and no community rate, because there is not one.
        let by_member = rates.iter().map(|(id, rate)| format!("{id} {}", js::to_string(rate))).collect::<Vec<_>>().join(" · ");
        let scenarios = list(k("scenarios"));
        let sampling = if scenarios.is_empty() {
            format!("{} sweep points", text_of(k("sweepPoints")))
        } else {
            format!("{} source scenarios", scenarios.len())
        };
        bullet(ui, |r| r.bold("Recorded growth:").plain(&format!(" {by_member} h⁻¹")).color(&format!(" ({sampling})"), p.ink_2));
    }
    if let Some(methane) = k("methane") {
        bullet(ui, |r| r.bold("Methane:").plain(&format!(" {} mmol gDW⁻¹ h⁻¹", js::to_string(methane))));
    }
    if let Some(note) = e("note").filter(|v| js::truthy(v)) {
        bullet(ui, |r| r.color(&js::to_string(note), p.ink_2));
    }
    if let Some(Value::Object(abundances)) = k("abundances").or(e("abundances")) {
        // A nine-species community leaves most members at essentially zero;
        // listing them all buries the ones that are actually there.
        let mut present: Vec<(&String, f64)> =
            abundances.iter().map(|(id, x)| (id, js::to_number(x))).filter(|(_, x)| *x >= 1e-4).collect();
        present.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let parts = present.iter().map(|(id, x)| format!("{id} {} %", js::to_fixed(x * 100.0, 1))).collect::<Vec<_>>().join(" · ");
        let hidden = abundances.len() - present.len();
        bullet(ui, |r| {
            let r = r.bold("Relative abundances:").plain(&format!(" {parts}"));
            if hidden > 0 { r.color(&format!(" ({hidden} below 0.01%)"), p.ink_2) } else { r }
        });
    }
    // Both reproduced communities record a sweep, and they sweep different
    // things, so what to plot is part of the registry entry.
    let sweep = list(k("sweep"));
    if let (false, Some(figure)) = (sweep.is_empty(), c.get("figure").filter(|v| js::truthy(v))) {
        sweep_figure(ui, &sweep, figure, &text_of(c.get("id")));
    }
}

// The Zuñiga case

/// `createGrowthForCase(store, scenarioId, speciesId)`.
fn create_growth_for_case(store: &Store, scenario_id: &str, species_id: &str) -> Option<f64> {
    let result = store.zuniga.solutions.get(scenario_id);
    if let Some(g) = result.and_then(|r| r.zuniga.as_ref()).and_then(|z| z.growth.get(species_id)).copied().filter(|g| g.is_finite()) {
        return Some(g);
    }
    let biomass = store.zuniga.prepared.as_ref()?.community.biomass_ids.get(species_id)?;
    result?.fluxes.get(biomass).copied()
}

/// `zunigaCaseBlock(store)`.
fn zuniga_case_block(ui: &mut Ui, shell: &mut Shell) {
    if !shell.store.is_zuniga_study() {
        widgets::hint(
            ui,
            "Loading requires an internet connection the first time. The original models remain in a private cache on this computer for subsequent use.",
        );
        return;
    }
    let store = &shell.store;
    let co = store.zuniga.solutions.get("mutant-coculture").and_then(|s| s.zuniga.clone());
    let mono_growth = create_growth_for_case(store, "mutant-monoculture", "Ec");
    let co_growth = co.as_ref().and_then(|z| z.growth.get("Ec").copied()).or_else(|| create_growth_for_case(store, "mutant-coculture", "Ec"));
    let phe_transfer = co.as_ref().and_then(|z| {
        z.cross_feeding
            .links
            .iter()
            .find(|l| l.base == "phe__L" && l.from.as_deref() == Some("Se") && l.to.as_deref() == Some("Ec"))
            .map(|l| l.flux)
    });
    let rescued = co_growth.is_some_and(|g| g.is_finite() && g > 1e-8)
        && mono_growth.is_some_and(|g| g.is_finite() && g.abs() < 1e-8)
        && phe_transfer.is_some_and(|f| f > 1e-8);
    let difference = store.zuniga.comparison == "difference";
    let reference = store.zuniga_state().get("fluxReference").map(js::to_number);
    let rows: Vec<(String, String, String)> = ZUNIGA_SCENARIOS
        .iter()
        .map(|s| {
            let se = if s.coculture { format_flux(create_growth_for_case(store, s.id, "Se"), 6) } else { "Absent".into() };
            (s.name.to_string(), format_flux(create_growth_for_case(store, s.id, "Ec"), 6), se)
        })
        .collect();
    let scenario = store.zuniga.scenario_id.clone().unwrap_or_default();
    let comparison = store.zuniga.comparison.clone();

    pc_heading(ui, "Comparable case scenarios");
    sc_label(ui, "Case scenario");
    let width = ui.available_width() - 8.0;
    let options: Vec<(String, String)> = ZUNIGA_SCENARIOS.iter().map(|s| (s.id.to_string(), s.name.to_string())).collect();
    if let Some(id) = widgets::select(ui, "zuniga-scenario", &scenario, &options, width, true, "Zuñiga case scenario") {
        if let Err(error) = shell.store.use_zuniga_scenario(&id, true) {
            shell.status_error(error.to_string());
        }
    }
    sc_label(ui, "Flux display");
    let options = [("absolute".to_string(), "Solved fluxes".to_string()), ("difference".to_string(), "Δ flux vs WT monoculture".to_string())];
    if let Some(mode) = widgets::select(ui, "zuniga-comparison", &comparison, &options, width, true, "Zuñiga flux comparison") {
        if let Err(error) = shell.store.set_zuniga_comparison(&mode) {
            shell.status_error(error.to_string());
        }
    }
    let shows = if difference {
        "Δv = selected − WT; blue means decrease, red means increase. Arrows retain the direction of the selected solved flux"
    } else {
        "the magnitude of the solved flux; arrows show its direction"
    };
    widgets::hint(
        ui,
        format!(
            "Geometry and the flux scale are shared across the three scenarios. Color and width show {shows}. The reference magnitude is {} in source-model flux units.",
            format_flux(reference, 4)
        ),
    );
    sweep_like_table(ui, "zuniga-growth", &["Scenario", "E. coli (h⁻¹)", "S. elongatus (h⁻¹)"], &rows.iter().map(|(a, b, c)| vec![a.clone(), b.clone(), c.clone()]).collect::<Vec<_>>());
    Rich::new(ui, 12.0).bold("Perturbation:").plain(" Ec PPNDH (pheA) is disabled in both mutant scenarios. The WT has its original bounds.").show(ui);
    Rich::new(ui, 12.0)
        .bold("Calculated phenylalanine transfer:")
        .plain(&format!(" S. elongatus → E. coli {} in source-model shared-pool flux units.", format_flux(phe_transfer, 6)))
        .show(ui);
    widgets::hint(
        ui,
        if rescued {
            "Calculated growth is restored in the mutant coculture relative to mutant monoculture. The displayed phenylalanine transfer identifies the calculated complementing route."
        } else {
            "Growth and exchange values shown here are calculated stationary solutions. Interpret rescue only where the mutant coculture supports growth and the relevant transfer is positive."
        },
    );
    widgets::hint(
        ui,
        "S. elongatus is absent from both monoculture calculations; its faint cell preserves the visual comparison. Δ values for its reactions use a zero baseline. This is a stationary adaptation, not a comparison of experimental monoculture and coculture measurements.",
    );
    ui.horizontal_wrapped(|ui| {
        if widgets::button(ui, "Export SVG").clicked() {
            shell.export_svg();
        }
        if widgets::button(ui, "Export PNG").clicked() {
            shell.export_png();
        }
        if widgets::button(ui, "Private reproducibility bundle").clicked() {
            shell.export_reproducibility();
        }
    });
}

/// `.pc-sweep`: a small numeric table (first column left, numbers right).
fn sweep_like_table(ui: &mut Ui, id: &str, head: &[&str], rows: &[Vec<String>]) {
    let p = Palette::current(ui.ctx());
    ui.add_space(4.0);
    egui::Grid::new(("pc-sweep", id)).striped(false).spacing(Vec2::new(10.0, 3.0)).show(ui, |ui| {
        for h in head {
            ui.label(RichText::new(*h).size(11.0).family(bold()).color(p.ink_2));
        }
        ui.end_row();
        for row in rows {
            for cell in row {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(cell).size(11.0).color(p.ink));
                });
            }
            ui.end_row();
        }
    });
    ui.add_space(4.0);
}

// `steadyComBlock`

/// What the SteadyCom controls show, worked out once per frame (the head
/// of `steadyComBlock` and of `patchSteadyComControls`).
struct ScView {
    community_id: Option<String>,
    key: Option<String>,
    phase: Option<String>,
    parsimonious: bool,
    iteration: Option<u32>,
    computed: bool,
    top: Option<f64>,
    minimum: f64,
    busy: bool,
    draft: Option<SteadyComDraft>,
    mu: String,
    state_mu: Option<f64>,
    feasible: Option<bool>,
    abundances: IndexMap<String, f64>,
    solve_ms: Option<f64>,
    resolution: Option<f64>,
    upper_bound: Option<f64>,
}

/// `support.communityId ?? c?.id ?? store.scenario?.id`: the community the
/// SteadyCom panel is about.
fn panel_community_id(store: &Store, c: Option<&Value>) -> Option<String> {
    let c_id = c.and_then(|c| c.get("id")).and_then(Value::as_str).map(str::to_string);
    store.steady_com_support(c_id.as_deref()).community_id.or(c_id).or_else(|| store.scenario_id())
}

fn sc_view(shell: &Shell, community_id: Option<String>) -> ScView {
    let state = shell.store.steady_com_state.as_ref().filter(|s| s.community_id == community_id);
    let computed = state.is_some_and(|s| s.source == "computed" && s.max.is_some_and(f64::is_finite));
    let top = if computed { state.and_then(|s| s.max) } else { None };
    let minimum = state.and_then(|s| s.min).filter(|m| m.is_finite()).unwrap_or(0.0);
    let busy = state.is_some_and(|s| s.phase.is_some()) || shell.ui.steady_com_pending;
    let key = state.map(|s| s.key.clone());
    let draft = shell.ui.steady_com_draft.clone().filter(|d| d.community_id == community_id && d.key == key);
    let state_mu = state.and_then(|s| s.mu).filter(|m| m.is_finite());
    let mu = match &draft {
        Some(d) => d.value.clone(),
        None => js::number_to_string(state_mu.or(top).unwrap_or(minimum)),
    };
    ScView {
        community_id,
        key,
        phase: state.and_then(|s| s.phase.clone()),
        parsimonious: state.is_some_and(|s| s.parsimonious),
        iteration: state.and_then(|s| s.progress.map(|p| p.iterations).or(s.iterations)),
        computed,
        top,
        minimum,
        busy,
        draft,
        mu,
        state_mu,
        feasible: state.and_then(|s| s.feasible),
        abundances: state.map(|s| s.abundances.clone()).unwrap_or_default(),
        solve_ms: state.and_then(|s| s.solve_ms),
        resolution: state.and_then(|s| s.resolution).filter(|r| r.is_finite()),
        upper_bound: state.and_then(|s| s.upper_bound).filter(|u| u.is_finite()),
    }
}

/// `showSteadyComError(message, invalidField)`: the error under the
/// SteadyCom controls (kept in `ui.steady_com_draft`), with the growth rate
/// and fiber limit as they were typed.
pub fn show_steady_com_error(shell: &mut Shell, message: &str, invalid_field: Option<&str>) {
    show_error_with(shell, message, invalid_field, None);
}

fn show_error_with(shell: &mut Shell, message: &str, invalid_field: Option<&str>, fiber_typed: Option<String>) {
    let c = selected_published(&shell.store);
    let community_id = panel_community_id(&shell.store, c.as_ref());
    let view = sc_view(shell, community_id.clone());
    let fiber_value = fiber_typed.or_else(|| view.draft.as_ref().and_then(|d| d.fiber_value.clone()));
    shell.ui.steady_com_draft = Some(SteadyComDraft {
        community_id,
        key: shell.store.steady_com_state.as_ref().map(|s| s.key.clone()),
        value: view.mu,
        error: message.to_string(),
        invalid_field: invalid_field.map(str::to_string),
        fiber_value,
    });
}

/// `clearSteadyComError(panel)`: the alert hidden and no field marked.
fn clear_steady_com_error(shell: &mut Shell) {
    if let Some(draft) = shell.ui.steady_com_draft.as_mut() {
        draft.error.clear();
        draft.invalid_field = None;
    }
}

/// The fiber limit as the field holds it: what is being typed, else what
/// it shows.
fn fiber_text(ui: &Ui, view: &ScView, store: &Store) -> Option<String> {
    let fiber = store.steady_com_config.fiber?;
    let typed: Option<String> = ui.data(|d| d.get_temp(fiber_live_id()));
    Some(typed.or_else(|| view.draft.as_ref().and_then(|d| d.fiber_value.clone())).unwrap_or_else(|| js::number_to_string(fiber.bacteroides)))
}

fn fiber_live_id() -> egui::Id {
    egui::Id::new("steadycom-fiber-typed")
}

/// `validateSteadyComFiber(panel)`.
fn validate_steady_com_fiber(shell: &mut Shell, fiber: Option<String>, nine: bool) -> bool {
    let Some(text) = fiber.filter(|_| nine) else { return true };
    let value = widgets::value_as_number(&text);
    if !value.is_finite() || !(1.0..=30.0).contains(&value) {
        show_error_with(shell, "Enter a Bacteroides fiber limit between 1 and 30 C-mmol/gDW/h.", Some("fiber"), Some(text));
        return false;
    }
    true
}

fn is_cancellation(error: &StoreError) -> bool {
    let message = error.to_string().to_lowercase();
    error.is_abort() || message.contains("cancel") || message.contains("superseded")
}

/// `runSteadyCom(request)`: one request at a time; the shell clears
/// `steady_com_pending` when the job it started finishes.
fn run_steady_com(shell: &mut Shell, request: impl FnOnce(&mut Store) -> coxeter_state::Result<SteadyComRequest>) {
    if shell.ui.steady_com_pending || shell.store.steady_com_state.as_ref().is_some_and(|s| s.phase.is_some()) {
        return;
    }
    match request(&mut shell.store) {
        Ok(SteadyComRequest::Started) => shell.ui.steady_com_pending = true,
        Ok(SteadyComRequest::Cached) => {}
        Err(error) if is_cancellation(&error) => {}
        Err(error) => {
            let message = error.to_string();
            show_steady_com_error(shell, &message, None);
            shell.status_error(format!("SteadyCom: {message}"));
        }
    }
}

/// `steadyComBlock(c, store, steadyComUI)`: current-input analysis, kept
/// separate from recorded published benchmarks.
fn steady_com_block(ui: &mut Ui, shell: &mut Shell, c: Option<&Value>) {
    let c_id = c.and_then(|c| c.get("id")).and_then(Value::as_str);
    let support = shell.store.steady_com_support(c_id);
    let p = Palette::current(ui.ctx());
    // A request whose job is no longer running is no longer pending (the
    // shell also clears it when it sees the job finish).
    if shell.ui.steady_com_pending && !shell.store.job_running(JobKind::SteadyCom) {
        shell.ui.steady_com_pending = false;
    }
    if !support.supported {
        if c.is_some_and(|c| c.pointer("/paper/framework").and_then(Value::as_str) != Some("SteadyCom")) {
            return;
        }
        ui.add_space(10.0);
        pc_heading(ui, "Fix a growth rate");
        widgets::hint(ui, &support.reason);
        return;
    }
    let community_id = support.community_id.clone().or(c_id.map(str::to_string)).or_else(|| shell.store.scenario_id());
    let view = sc_view(shell, community_id.clone());
    let nine = community_id.as_deref() == Some("steadycom-gut-nine");
    let fiber = shell.store.steady_com_config.fiber;

    ui.add_space(12.0);
    let top_y = ui.cursor().top();
    ui.painter().hline(ui.max_rect().x_range(), top_y - 4.0, Stroke::new(1.0, p.line));
    pc_heading(ui, "Fix a growth rate · SteadyCom");
    if c.is_some_and(|c| js::truthy(c.get("computed").unwrap_or(&Value::Null))) {
        widgets::hint(
            ui,
            "The published results above are historical benchmarks. Calculate the range for the current model and conditions before choosing µ.",
        );
    }
    widgets::hint(ui, "Fixing µ solves a community LP at total biomass 1; abundances are outputs of this analysis.");

    // `.sc-steadycom-condition`: the diet of the nine-species example.
    if nine {
        ui.add_space(6.0);
        sc_label(ui, "Diet / fiber condition");
        let options = [
            ("baseline".to_string(), "Baseline diet (no additional fiber limits)".to_string()),
            ("figure4".to_string(), "Figure 4 fiber limits".to_string()),
        ];
        let current = if fiber.is_some() { "figure4" } else { "baseline" }.to_string();
        let width = ui.available_width() - 8.0;
        if let Some(choice) = widgets::select(ui, "steadycom-fiber-condition", &current, &options, width, !view.busy, "SteadyCom fiber condition") {
            shell.ui.steady_com_draft = None;
            if let Err(error) = shell.store.set_steady_com_fiber(if choice == "figure4" { Some(30.0) } else { None }) {
                shell.status_error(error.to_string());
            }
        }
        if let Some(fiber) = fiber {
            let shown = view.draft.as_ref().and_then(|d| d.fiber_value.clone()).unwrap_or_else(|| js::number_to_string(fiber.bacteroides));
            let invalid = view.draft.as_ref().is_some_and(|d| d.invalid_field.as_deref() == Some("fiber"));
            ui.horizontal(|ui| {
                ui.label(RichText::new("B. thetaiotaomicron").size(11.0).color(p.ink_2));
                let (response, event) = widgets::number_field(
                    ui,
                    egui::Id::new("steadycom-fiber"),
                    &shown,
                    Field::new().width(85.0).aria("Bacteroides maximum specific fiber uptake").enabled(!view.busy).invalid(invalid),
                );
                aria_invalid(&response, invalid);
                ui.label(RichText::new("C-mmol/gDW/h").size(10.0).color(p.ink_3));
                match event {
                    FieldEvent::Input(text) => {
                        ui.data_mut(|d| d.insert_temp(fiber_live_id(), text));
                    }
                    FieldEvent::Change(text) => {
                        ui.data_mut(|d| d.remove::<String>(fiber_live_id()));
                        let value = widgets::value_as_number(&text);
                        if !value.is_finite() || !(1.0..=30.0).contains(&value) {
                            show_error_with(shell, "Enter a Bacteroides fiber limit between 1 and 30 C-mmol/gDW/h.", Some("fiber"), Some(text));
                        } else {
                            shell.ui.steady_com_draft = None;
                            if let Err(error) = shell.store.set_steady_com_fiber(Some(value)) {
                                shell.status_error(error.to_string());
                            }
                        }
                    }
                    FieldEvent::None => {}
                }
            });
            widgets::hint(ui, "F. prausnitzii is held at 5 C-mmol/gDW/h. Changing these limits invalidates the calculated range.");
        }
    }

    // `.pc-list`: the range, the search resolution, the composition.
    ui.add_space(4.0);
    let range = match view.top {
        None => "Not calculated for the current inputs".to_string(),
        Some(top) => format!("{} – {} h⁻¹", js::number_to_string(view.minimum), js::number_to_string(top)),
    };
    bullet(ui, |r| r.bold("Calculated µ range:").plain(&format!(" {range}")));
    if let (true, Some(resolution)) = (view.computed, view.resolution) {
        let upper = view.upper_bound.map(|u| format!("; upper search bound: {} h⁻¹", js::number_to_string(u))).unwrap_or_default();
        bullet(ui, |r| r.color(&format!("Search resolution: {} h⁻¹{upper}.", js::number_to_string(resolution)), p.ink_2));
    }
    if let (Some(true), false, Some(mu)) = (view.feasible, view.abundances.is_empty(), view.state_mu) {
        let mut members: Vec<(&String, f64)> = view.abundances.iter().map(|(id, x)| (id, *x)).collect();
        members.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let parts = members
            .iter()
            .map(|(id, x)| if *x > 1e-9 { format!("{id} {} %", js::to_fixed(100.0 * x, 1)) } else { format!("{id} inactive (X = 0)") })
            .collect::<Vec<_>>()
            .join(" · ");
        let ms = js::number_to_string(view.solve_ms.unwrap_or(0.0));
        bullet(ui, |r| {
            r.bold(&format!("Composition at µ = {} h⁻¹:", js::number_to_string(mu))).plain(&format!(" {parts} ")).color(&format!("({ms} ms)"), p.ink_2)
        });
    }
    if let (Some(false), Some(mu)) = (view.feasible, view.state_mu) {
        bullet(ui, |r| r.color(&format!("No feasible community at µ = {} h⁻¹.", js::number_to_string(mu)), p.danger));
    }

    // `.sc-steadycom-progress` (`role="status"`).
    if view.busy {
        let what = if view.phase.as_deref() == Some("fixed") {
            if view.parsimonious { "Solving at fixed µ for the least total flux" } else { "Solving at fixed µ" }
        } else {
            "Calculating the growth-rate range"
        };
        let iteration = view.iteration.map(|i| format!(" · iteration {i}")).unwrap_or_default();
        Frame::new().fill(p.accent_soft).inner_margin(Margin { left: 10, right: 10, top: 8, bottom: 8 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let r = ui.add(egui::Label::new(RichText::new(format!("{what}{iteration}…")).size(11.0).color(p.accent)).wrap());
            ui.painter().vline(r.rect.left() - 10.0, (r.rect.top() - 8.0)..=(r.rect.bottom() + 8.0), Stroke::new(3.0, p.accent));
        });
    }

    // `.sc-mu`: the slider and the number, kept in step in the draft; the
    // store hears of the rate only when Solve is pressed.
    let enabled = view.top.is_some() && !view.busy;
    let max = view.top.unwrap_or(1.0);
    let typed = widgets::value_as_number(&view.mu);
    let position = if typed.is_finite() { js::min(js::max(typed, view.minimum), max) } else { view.minimum };
    ui.add_space(8.0);
    let (_, slid) = ui
        .scope(|ui| {
            ui.spacing_mut().slider_width = ui.available_width();
            widgets::slider(ui, position, view.minimum, max, 0.0, enabled, "Growth rate")
        })
        .inner;
    if let Some(v) = slid.value() {
        shell.ui.steady_com_draft = Some(SteadyComDraft {
            community_id: view.community_id.clone(),
            key: shell.store.steady_com_state.as_ref().map(|s| s.key.clone()),
            value: js::number_to_string(v),
            ..SteadyComDraft::default()
        });
    }
    let mu_invalid = view.draft.as_ref().is_some_and(|d| d.invalid_field.as_deref() == Some("mu"));
    ui.horizontal(|ui| {
        let (response, event) = widgets::number_field(
            ui,
            egui::Id::new("steadycom-mu-number"),
            &view.mu,
            Field::new().width((ui.available_width() - 40.0).max(60.0)).aria("Growth rate, h⁻¹").enabled(enabled).invalid(mu_invalid),
        );
        aria_invalid(&response, mu_invalid);
        ui.label(RichText::new("h⁻¹").size(12.0).color(p.ink_2));
        let (text, change) = match event {
            FieldEvent::Input(t) => (t, false),
            FieldEvent::Change(t) => (t, true),
            FieldEvent::None => return,
        };
        let finite = widgets::value_as_number(&text).is_finite();
        let previous = shell.ui.steady_com_draft.clone().filter(|_| view.draft.is_some());
        shell.ui.steady_com_draft = Some(SteadyComDraft {
            community_id: view.community_id.clone(),
            key: shell.store.steady_com_state.as_ref().map(|s| s.key.clone()),
            value: text,
            // While a half-typed number is not finite the field stays as
            // it was marked; a finite one clears the error.
            error: if finite { String::new() } else { previous.as_ref().map(|d| d.error.clone()).unwrap_or_default() },
            invalid_field: if finite { None } else { previous.and_then(|d| d.invalid_field) },
            fiber_value: None,
        });
        if !finite && change {
            show_steady_com_error(shell, "Enter a finite growth rate; an empty field is not µ = 0.", Some("mu"));
        }
    });

    // `.sc-steadycom-error` (`role="alert"`).
    if let Some(error) = view.draft.as_ref().map(|d| d.error.clone()).filter(|e| !e.is_empty()) {
        ui.add(egui::Label::new(RichText::new(error).size(11.0).color(p.danger)).wrap());
    }

    let (_, changed) = widgets::checkbox(
        ui,
        shell.store.steady_com_config.parsimonious,
        RichText::new("Parsimonious: least total flux at this µ").size(11.0).color(p.ink_2),
        !view.busy,
        None,
    );
    if let Some(on) = changed {
        shell.store.set_steady_com_parsimonious(on);
    }

    // `.sc-mu-actions`.
    let fiber_now = fiber_text(ui, &view, &shell.store);
    ui.horizontal_wrapped(|ui| {
        let label = if view.computed { "Recalculate range" } else { "Calculate range" };
        if Btn::new(label).primary().enabled(!view.busy).show(ui).clicked() {
            if validate_steady_com_fiber(shell, fiber_now.clone(), nine) {
                shell.ui.steady_com_draft = None;
                run_steady_com(shell, |store| store.steady_com_range(true));
            }
        }
        if Btn::new("Solve at this µ").enabled(enabled).show(ui).clicked() && validate_steady_com_fiber(shell, fiber_now.clone(), nine) {
            let mu = widgets::value_as_number(&view.mu);
            if !mu.is_finite() || mu < 0.0 {
                show_steady_com_error(shell, "Enter a finite growth rate of zero or more.", Some("mu"));
            } else {
                clear_steady_com_error(shell);
                run_steady_com(shell, |store| store.solve_steady_com_at(mu));
            }
        }
        if view.busy && Btn::new("Cancel").show(ui).clicked() {
            shell.store.cancel_steady_com(false);
        }
    });
    widgets::hint(ui, "Range calculation runs several LPs and can take time for genome-scale models. Cancel stops the pending calculation.");
    widgets::hint(
        ui,
        "The composition is one feasible solution and may be non-unique, even at the maximum. Species with X = 0 are inactive. At µ = 0, present members still obey their maintenance bounds.",
    );
    widgets::hint(
        ui,
        "Parsimonious picks, among all solutions at this µ, the one with the least summed member flux (abundance × flux). It drops the cycles the community does not need, loops between two members included, and takes longer.",
    );
    widgets::hint(ui, "The map displays per-gDW fluxes for present members. Abundance sliders adopt the returned composition.");
    let _ = view.key;
}

// The sweep figure

/// `format(value)`: enough digits to tell two points apart, without a
/// column of noise.
fn format_sweep(value: f64) -> String {
    if !value.is_finite() {
        return "—".into();
    }
    let magnitude = value.abs();
    if magnitude == 0.0 {
        return "0".into();
    }
    if magnitude < 0.01 { to_exponential(value, 2) } else { js::to_fixed(value, if magnitude < 1.0 { 4 } else { 3 }) }
}

/// `x.toExponential(digits)`.
fn to_exponential(value: f64, digits: usize) -> String {
    let s = format!("{value:.digits$e}");
    match s.split_once('e') {
        Some((mantissa, exponent)) if !exponent.starts_with('-') => format!("{mantissa}e+{exponent}"),
        _ => s,
    }
}

/// `fmtTick(v)`.
fn fmt_tick(v: f64) -> String {
    let text = if v.abs() >= 1.0 || v == 0.0 { js::to_fixed(v, 2) } else { coxeter_state::omics::to_precision(v, 2) };
    js::number_to_string(js::string_to_number(&text))
}

struct SweepSeries {
    values: Vec<f64>,
    color: Color32,
    label: String,
}

/// `sweepFigure(sweep, figure)`: the sweep as the paper's figure draws it,
/// two panels sharing an x axis.
///
/// Two panels and not one chart with two y axes: the growth rate runs to
/// about 0.08 h⁻¹ and the aggregate exchange to about 6 C-mmol/h, and two
/// scales on one frame let the reader infer a crossing that is an artefact
/// of where the axes were placed. The table below the panels is the same
/// numbers, for reading exactly and for anyone the colours do not reach.
fn sweep_figure(ui: &mut Ui, sweep: &[Value], figure: &Value, id: &str) {
    let p = Palette::current(ui.ctx());
    let x_key = text_of(figure.pointer("/x/key"));
    let x_label = text_of(figure.pointer("/x/label"));
    let xs: Vec<f64> = sweep.iter().map(|point| num(point.get(&x_key))).collect();
    // Series colours follow the series' place in the panel, not its value,
    // so a curve keeps its colour whatever the numbers do.
    let colours = [p.series_1, p.series_2];
    let x_name = x_label.split('(').next().unwrap_or("").trim().to_string();
    ui.add_space(10.0);
    for (n, panel) in list(figure.get("panels")).iter().enumerate() {
        let series: Vec<SweepSeries> = list(panel.get("series"))
            .iter()
            .enumerate()
            .map(|(i, spec)| {
                let key = text_of(spec.get("key"));
                SweepSeries {
                    values: sweep.iter().map(|point| num(point.get(&key))).collect(),
                    color: colours[i % colours.len()],
                    label: text_of(spec.get("label")),
                }
            })
            .collect();
        let tip = |i: usize| {
            format!("{x_name} {} · ", js::number_to_string(xs[i]))
                + &series.iter().map(|s| format!("{} {}", s.label, format_sweep(s.values[i]))).collect::<Vec<_>>().join(" · ")
        };
        sweep_panel(ui, (id, n), &xs, &series, &text_of(panel.get("yLabel")), &text_of(panel.get("title")), series.len() > 1, &tip);
    }
    ui.add(egui::Label::new(RichText::new(text_of(figure.get("caption"))).size(10.5).color(p.ink_3)).wrap());
    egui::CollapsingHeader::new(RichText::new("Table view").size(12.0)).id_salt(("pc-figure-table", id)).default_open(false).show(ui, |ui| {
        sweep_table(ui, sweep, figure, id);
    });
}

/// `sweepPanel({ xs, series, yLabel, title, legend, tip })`: one panel,
/// drawn in the 260 × 96 frame of the JS SVG and scaled to the width.
#[allow(clippy::too_many_arguments)]
fn sweep_panel(
    ui: &mut Ui,
    id: (&str, usize),
    xs: &[f64],
    series: &[SweepSeries],
    y_label: &str,
    title: &str,
    legend: bool,
    tip: &dyn Fn(usize) -> String,
) {
    const W: f32 = 260.0;
    const H: f32 = 96.0;
    const PAD_L: f32 = 40.0;
    const PAD_R: f32 = 10.0;
    const PAD_T: f32 = 16.0;
    const PAD_B: f32 = 20.0;
    let p = Palette::current(ui.ctx());

    // `.pc-panel-title`, with the key when there are two series.
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(title).size(11.0).family(bold()).color(p.ink_2));
        if legend {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                for s in series.iter().rev() {
                    ui.label(RichText::new(&s.label).size(10.0).color(p.ink_2));
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(10.0, 2.0), Sense::hover());
                    ui.painter().rect_filled(rect, CornerRadius::same(1), s.color);
                }
            });
        }
    });

    let all: Vec<f64> = series.iter().flat_map(|s| s.values.iter().copied()).collect();
    let x_min = js::min_n(xs.iter().copied());
    let x_max = js::max_n(xs.iter().copied());
    let y_min = js::min_n(std::iter::once(0.0).chain(all.iter().copied()));
    let y_max = js::max_n(all.iter().copied());
    let or_one = |v: f64| if v == 0.0 || v.is_nan() { 1.0 } else { v };
    let span = or_one(y_max - y_min);
    let x_span = or_one(x_max - x_min);

    let width = ui.available_width();
    let s = width / W;
    let (response, painter) = ui.allocate_painter(Vec2::new(width, H * s), Sense::hover());
    widgets::aria_label(&response, &format!("{title}: {y_label}"));
    let origin = response.rect.min;
    let at = |x: f32, y: f32| pos2(origin.x + x * s, origin.y + y * s);
    let px = |v: f64| PAD_L + (((v - x_min) / x_span) as f32) * (W - PAD_L - PAD_R);
    let py = |v: f64| H - PAD_B - (((v - y_min) / span) as f32) * (H - PAD_T - PAD_B);
    let font = |size: f32, strong: bool| FontId::new(size * s, if strong { bold() } else { FontFamily::Proportional });

    // Grid lines and y ticks at the two ends of the scale.
    for v in [y_min, y_max] {
        let y = py(v);
        if !y.is_finite() {
            continue;
        }
        painter.line_segment([at(PAD_L, y), at(W - PAD_R, y)], Stroke::new(1.0, p.line));
        painter.text(at(PAD_L - 5.0, y), Align2::RIGHT_CENTER, fmt_tick(v), font(8.0, false), p.ink_3);
    }
    for v in [x_min, x_max] {
        let x = px(v);
        if x.is_finite() {
            painter.text(at(x, H - 6.0), Align2::CENTER_BOTTOM, js::number_to_string(v), font(8.0, false), p.ink_3);
        }
    }
    // The lines, and a direct label at the end of each, so identity never
    // rests on colour.
    for sr in series {
        let points: Vec<egui::Pos2> = sr
            .values
            .iter()
            .zip(xs)
            .map(|(v, x)| (px(*x), py(*v)))
            .filter(|(x, y)| x.is_finite() && y.is_finite())
            .map(|(x, y)| at(x, y))
            .collect();
        painter.add(egui::Shape::line(points, Stroke::new(2.0 * s.max(0.6), sr.color)));
        if let (Some(v), Some(x)) = (sr.values.last(), xs.last()) {
            let (x, y) = (px(*x), py(*v));
            if x.is_finite() && y.is_finite() {
                painter.text(at(x - 2.0, y - 5.0), Align2::RIGHT_BOTTOM, &sr.label, font(8.5, true), sr.color);
            }
        }
    }
    // The axis title, turned up the left edge.
    let galley = painter.layout_no_wrap(y_label.to_string(), font(8.0, false), p.ink_3);
    let anchor = at(10.0 - 8.0, H / 2.0 + galley.size().x / s / 2.0);
    painter.add(egui::epaint::TextShape::new(anchor, galley, p.ink_3).with_angle(-std::f32::consts::FRAC_PI_2));

    // One wide target per x, carrying the tooltip: the hover layer a line
    // chart owes the reader.
    if let (Some(pointer), false) = (response.hover_pos(), xs.is_empty()) {
        let step = (W - PAD_L - PAD_R) / (xs.len().max(2) - 1) as f32;
        let x = (pointer.x - origin.x) / s;
        let y = (pointer.y - origin.y) / s;
        let hit = xs.iter().position(|v| {
            let cx = px(*v);
            x >= cx - step / 2.0 && x < cx + step / 2.0
        });
        if let (Some(i), true) = (hit, (PAD_T..=H - PAD_B).contains(&y)) {
            let guide = px(xs[i]);
            painter.line_segment([at(guide, PAD_T), at(guide, H - PAD_B)], Stroke::new(1.0, p.ink_3.gamma_multiply(0.5)));
            response.on_hover_text_at_pointer(tip(i));
        }
    }
    let _ = id;
    ui.add_space(6.0);
}

/// `sweepTable(sweep, figure)`: the same numbers as the figure.
fn sweep_table(ui: &mut Ui, sweep: &[Value], figure: &Value, id: &str) {
    let x_label = text_of(figure.pointer("/x/label"));
    // `label.replace(/\s*\(.*\)$/, '')`.
    let x_label = match (x_label.ends_with(')'), x_label.find('(')) {
        (true, Some(i)) => x_label[..i].trim_end().to_string(),
        _ => x_label,
    };
    let mut columns = vec![(text_of(figure.pointer("/x/key")), x_label)];
    for panel in list(figure.get("panels")) {
        for spec in list(panel.get("series")) {
            columns.push((text_of(spec.get("key")), text_of(spec.get("label"))));
        }
    }
    let head: Vec<&str> = columns.iter().map(|(_, l)| l.as_str()).collect();
    let rows: Vec<Vec<String>> =
        sweep.iter().map(|point| columns.iter().map(|(key, _)| format_sweep(num(point.get(key)))).collect()).collect();
    sweep_like_table(ui, id, &head, &rows);
}

// `communityProvenanceBlock`

/// `communityProvenanceBlock(c)`: what was reproduced, what was adapted,
/// and where the models came from. The findings and the adaptations
/// download together, because they only mean anything together.
fn community_provenance_block(ui: &mut Ui, shell: &mut Shell, c: &Value) {
    let p = Palette::current(ui.ctx());
    let id = text_of(c.get("id"));
    let rep = c.get("reproduction").cloned().unwrap_or(Value::Null);
    let reproduces = list(rep.get("reproduces"));
    let adaptations = list(rep.get("adaptations"));
    let needs = list(rep.get("needs"));
    let items = |ui: &mut Ui, values: &[Value]| {
        for v in values {
            bullet(ui, |r| r.plain(&js::to_string(v)));
        }
    };
    let sub = |ui: &mut Ui, s: &str| {
        ui.add_space(4.0);
        ui.label(RichText::new(s.to_uppercase()).size(10.0).family(bold()).color(p.ink_3).extra_letter_spacing(0.5));
    };
    ui.add_space(8.0);
    if !reproduces.is_empty() || !adaptations.is_empty() {
        let state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), ui.make_persistent_id(("pc-findings", &id)), false);
        state
            .show_header(ui, |ui| {
                ui.label(RichText::new("Reproduced paper findings").size(12.0).family(bold()).color(p.ink));
                if pc_dl(ui, "Download", true, "Findings and adaptations as Markdown", "Download").clicked() {
                    shell.download_findings(&id);
                }
            })
            .body(|ui| {
                if !reproduces.is_empty() {
                    sub(ui, "Reproduced");
                    items(ui, &reproduces);
                }
                if !adaptations.is_empty() {
                    sub(ui, "Adaptations and assumptions");
                    items(ui, &adaptations);
                }
            });
    }
    if !needs.is_empty() {
        egui::CollapsingHeader::new(RichText::new(format!("Requirements for reproduction ({})", needs.len())).size(12.0).family(bold()))
            .id_salt(("pc-needs", &id))
            .default_open(true)
            .show(ui, |ui| items(ui, &needs));
    }
    let models: Vec<Value> = match c.get("models").filter(|m| !m.is_null()) {
        Some(models) => list(Some(models)),
        None => list(c.get("species"))
            .iter()
            .filter_map(|sp| sp.get("model").filter(|m| m.get("id").is_some_and(js::truthy)).cloned())
            .collect(),
    };
    if !models.is_empty() {
        egui::CollapsingHeader::new(RichText::new(format!("Model provenance ({})", models.len())).size(12.0).family(bold()))
            .id_salt(("pc-models", &id))
            .default_open(false)
            .show(ui, |ui| {
                for m in &models {
                    bullet(ui, |r| r.bold(&text_of(m.get("id"))).plain(&format!(" — {}", text_of(m.get("reference")))));
                    let note = text_of(m.get("note"));
                    let source = text_of(m.get("source"));
                    let line = if note.is_empty() { source } else { format!("{source} · {note}") };
                    ui.indent(("pc-model-source", text_of(m.get("id"))), |ui| widgets::hint(ui, line));
                }
            });
    }
}
