//! The Edit Model cards (port of `speciesCard`, `mediumCard`,
//! `modelEditorCard`, `modelOverviewCard`, `communityOmicsCard` and
//! `omicsOverlayCard` of `src/ui/panels.js`), with the handlers of
//! `onSidebarClick` / `onSidebarInput` for their controls and
//! `acceptNumber`, `createMetaboliteFromEditor`, `createReactionFromEditor`,
//! `showModelEditError`, `draftKey`, `captureWorkspaceDraft`,
//! `restoreWorkspaceDraft` and `discardModelEditorDraft` of `src/ui/app.js`.
//!
//! The model editor's forms keep what is typed in `ui.drafts` under the JS
//! draft key (`model:metabolite`, `model:reaction`) as it is typed, which is
//! what the JS gets by capturing the form before a repaint and restoring it
//! after.

use coxeter_core::compartments::{Names, cell_compartment_name, cell_compartment_visual_id, compartment_colors};
use coxeter_core::js;
use coxeter_render::format_flux;
use coxeter_state::omics::{CommunityOmicsPatch, OmicsSettingsPatch, to_precision};
use egui::{Color32, CornerRadius, Frame, Margin, RichText, Sense, Stroke, Ui, Vec2, pos2};
use serde_json::json;

use crate::app::{Deferred, Shell};
use crate::panels::scenario::{Rich, aria_invalid};
use crate::theme::{Palette, bold, css_color};
use crate::ui_state::Workspace;
use crate::widgets::{self, Btn, Field, FieldEvent};

/// Medium components worth putting a slider on, in the order they matter.
pub const MEDIUM_ORDER: [&str; 16] =
    ["glc__D", "o2", "photon", "co2", "hco3", "ac", "etoh", "lac__L", "lac__D", "for", "h2", "succ", "pyr", "glyc", "nh4", "pi"];

/// `MEDIUM_LABELS`.
pub fn medium_label(base: &str) -> &str {
    match base {
        "glc__D" => "Glucose",
        "o2" => "Oxygen",
        "photon" => "Light (photons)",
        "co2" => "CO₂",
        "hco3" => "Bicarbonate",
        "ac" => "Acetate",
        "etoh" => "Ethanol",
        "lac__L" => "L-lactate",
        "lac__D" => "D-lactate",
        "for" => "Formate",
        "h2" => "H₂",
        "succ" => "Succinate",
        "pyr" => "Pyruvate",
        "glyc" => "Glycerol",
        "nh4" => "Ammonium",
        "pi" => "Phosphate",
        other => other,
    }
}

/// `.species-item`: a bordered box on the panel surface, faded when off.
fn species_item(ui: &mut Ui, off: bool, add: impl FnOnce(&mut Ui)) {
    let p = Palette::current(ui.ctx());
    ui.scope(|ui| {
        if off {
            ui.multiply_opacity(0.5);
        }
        Frame::new()
            .fill(p.panel)
            .stroke(Stroke::new(1.0, p.line))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(10, 9))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                add(ui)
            });
    });
}

/// A `<select>` showing its first option when `current` is none of them.
fn select_or_first(ui: &mut Ui, id: &str, current: &str, options: &[(String, String)], width: f32, enabled: bool, aria: &str) -> Option<String> {
    let current = if options.iter().any(|(v, _)| v == current) {
        current.to_string()
    } else {
        options.first().map(|(v, _)| v.clone()).unwrap_or_default()
    };
    widgets::select(ui, id, &current, options, width, enabled, aria)
}

/// A slider that takes what is left of the row beside `reserve` points.
#[allow(clippy::too_many_arguments)]
fn row_slider(ui: &mut Ui, value: f64, min: f64, max: f64, step: f64, enabled: bool, aria: &str, reserve: f32) -> FieldEvent<f64> {
    ui.scope(|ui| {
        let spacing = ui.spacing().item_spacing.x;
        ui.spacing_mut().slider_width = (ui.available_width() - reserve - spacing).max(40.0);
        widgets::slider(ui, value, min, max, step, enabled, aria).1
    })
    .inner
}

// `speciesCard`

/// `speciesCard(store)`.
pub fn species_card(ui: &mut Ui, shell: &mut Shell) {
    let p = Palette::current(ui.ctx());
    if shell.store.is_zuniga_study() {
        widgets::card(ui, "Case organisms", None, |ui| {
            for id in ["Ec", "Se"] {
                let model = shell.store.species_by_id(id).and_then(|sp| shell.store.models.get(&sp.file));
                let present = id == "Ec" || shell.store.zuniga.scenario_id.as_deref() == Some("mutant-coculture");
                let organism = model.map(|m| m.cell.organism.clone()).unwrap_or_else(|| {
                    if id == "Ec" { "Escherichia coli" } else { "Synechococcus elongatus" }.to_string()
                });
                let growth = shell.store.growth.get(id).copied();
                species_item(ui, !present, |ui| {
                    ui.label(RichText::new(organism).size(13.0).family(bold()).color(p.ink));
                    if present {
                        widgets::hint(ui, format!("Growth: {} h⁻¹", format_flux(growth, 6)));
                    } else {
                        widgets::hint(ui, "Absent from this monoculture calculation; shown faintly to preserve map geometry.");
                    }
                });
                ui.add_space(10.0);
            }
            widgets::hint(
                ui,
                "Population coefficients come from the source community model. Generic abundance sliders would change its stoichiometry and are disabled for this reproducible case.",
            );
        });
        return;
    }
    let active = shell.store.species.iter().filter(|s| s.enabled).count();
    widgets::card(ui, "Species", Some(&format!("{active} active")), |ui| {
        let species = shell.store.species.clone();
        for sp in &species {
            species_row(ui, shell, sp, &p);
            ui.add_space(10.0);
        }
        let mut load = false;
        let mut yeast9 = false;
        widgets::grow_row(ui, 2, |ui, i| {
            if i == 0 {
                load = Btn::new("Load model(s)").primary().wide().show(ui).clicked();
            } else {
                yeast9 = Btn::new("Yeast9 9.1.1").title("Load the official yeast-GEM 9.1.1 SBML model").wide().show(ui).clicked();
            }
        });
        if load {
            shell.import_models();
        }
        if yeast9 {
            shell.load_yeast9();
        }
        widgets::hint(
            ui,
            "Accepts COBRApy/BiGG JSON or SBML. You can pick several files at once: each becomes one species/cell. Imported models start with an empty compartment map. The Yeast9 shortcut loads the official pinned model and preserves all 14 subcellular compartments.",
        );
        widgets::hint(
            ui,
            "Abundances are normalised to 1. They turn each species' per-gDW flux into community per-gDW flux, which is what travels through the shared medium.",
        );
    });
}

/// One `.species-item` of the species card.
fn species_row(ui: &mut Ui, shell: &mut Shell, sp: &coxeter_state::Species, p: &Palette) {
    let model = shell.store.models.get(&sp.file);
    let name = model.map_or_else(|| sp.id.clone(), |m| m.cell.name.clone());
    let organism = model.map(|m| m.cell.organism.clone()).unwrap_or_default();
    let color = css_color(model.map_or("#64748b", |m| m.cell.color.as_str()), css_color("#64748b", Color32::GRAY));
    let dark = shell.store.view.dark;
    let chips: Vec<(String, (String, String, String))> = model
        .map(|m| {
            m.compartment_ids()
                .iter()
                .map(|cid| {
                    let c = compartment_colors(&cell_compartment_visual_id(cid, &m.compartment_names), dark, &Names::new());
                    (cell_compartment_name(cid, &m.compartment_names, "en"), (c.fill.to_string(), c.text.to_string(), c.stroke.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();
    let mu = shell.store.growth.get(&sp.id).copied();
    // A drag still waiting for its debounced call shows where it was left.
    let abundance = match shell.deferred.get("abundance") {
        Some((_, Deferred::Abundance { species, value })) if *species == sp.id => js::string_to_number(value),
        _ => sp.abundance,
    };
    species_item(ui, !sp.enabled, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let (_, toggled) = widgets::checkbox(ui, sp.enabled, "", true, Some(&format!("Include {name} in the analysis")));
            if let Some(on) = toggled {
                // A kept drawing keeps its view too; only a fresh layout is refitted.
                if !shell.store.keeps_drawing_on_species_toggle() {
                    shell.refit();
                }
                if let Err(error) = shell.store.set_species_enabled(&sp.id, on) {
                    shell.status_error(error.to_string());
                }
            }
            widgets::swatch(ui, color, 11.0);
            ui.add(egui::Label::new(RichText::new(&name).size(13.0).family(bold()).color(p.ink)).truncate());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let text = mu.map_or_else(|| "—".to_string(), |mu| format!("µ {}", format_flux(Some(mu), 4)));
                ui.label(RichText::new(text).size(12.0).color(p.ink_2));
            });
        });
        ui.label(RichText::new(&organism).size(11.5).color(p.ink_3));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_sized([74.0, 18.0], egui::Label::new(RichText::new("Abundance").size(12.0).color(p.ink_2)));
            let event = row_slider(ui, abundance, 0.0, 1.0, 0.05, sp.enabled, &format!("Abundance of {name}"), 40.0);
            if let Some(v) = event.value() {
                shell.debounce("abundance", 0.14, Deferred::Abundance { species: sp.id.clone(), value: js::number_to_string(v) });
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(format!("{}%", js::to_fixed(abundance * 100.0, 0))).size(12.0).color(p.ink_2));
            });
        });
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
            for (label, (fill, text, stroke)) in &chips {
                widgets::chip(ui, label, Some((fill, text, stroke)));
            }
        });
    });
}

// `mediumCard`

/// `mediumCard(store)`.
pub fn medium_card(ui: &mut Ui, shell: &mut Shell) {
    if shell.store.is_zuniga_study() {
        widgets::card(ui, "Source medium", None, |ui| {
            widgets::hint(
                ui,
                "The complete source model fixes the available nutrients and exchange bounds for each case scenario. Select WT monoculture, mutant monoculture or mutant coculture under Scenario & objective.",
            );
            widgets::hint(ui, "The shared-medium drawing is a visual excerpt. It does not change the medium or the mathematical solution.");
        });
        return;
    }
    let mut bases: indexmap::IndexSet<String> = shell.store.medium.keys().cloned().collect();
    if let Some(community) = &shell.store.community {
        bases.extend(community.transfers_by_base.keys().cloned());
    }
    // Known components lead, in the order they usually matter; anything else
    // a loaded model brings with it still gets a slider rather than
    // disappearing.
    let rank = |b: &str| MEDIUM_ORDER.iter().position(|x| *x == b).unwrap_or(99);
    let mut ordered: Vec<String> = bases.into_iter().collect();
    ordered.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| js::locale_compare(a, b)));
    widgets::card(ui, "Shared medium", None, |ui| {
        if ordered.is_empty() {
            widgets::empty(ui, "No exchangeable medium components.");
        }
        for base in &ordered {
            medium_row(ui, shell, base);
        }
        widgets::hint(
            ui,
            "Maximum supply in mmol · gDW⁻¹ · h⁻¹. Zero means the medium does not supply it: it is then only available if another species secretes it.",
        );
    });
}

/// One row of the medium: the slider and the exact number.
fn medium_row(ui: &mut Ui, shell: &mut Shell, base: &str) {
    let p = Palette::current(ui.ctx());
    let label = medium_label(base).to_string();
    let value = shell.store.medium.get(base).copied().unwrap_or(0.0).abs();
    let value = match shell.deferred.get("medium") {
        Some((_, Deferred::Medium { base: b, value })) if b == base => js::string_to_number(value).abs(),
        _ => value,
    };
    let max = if value > 100.0 { 1000.0 } else { 40.0 };
    let step = if max > 100.0 { 10.0 } else { 0.5 };
    let invalid_id = egui::Id::new(("medium-number-invalid", base));
    let invalid = ui.data(|d| d.get_temp::<bool>(invalid_id)).unwrap_or(false);
    ui.horizontal(|ui| {
        ui.add_sized([96.0, 18.0], egui::Label::new(RichText::new(&label).size(12.0).color(p.ink_2)).truncate());
        let slid = row_slider(ui, js::min(value, max), 0.0, max, step, true, &format!("Supply of {label}"), 72.0);
        if let Some(v) = slid.value() {
            shell.debounce("medium", 0.12, Deferred::Medium { base: base.to_string(), value: js::number_to_string(v) });
        }
        let (response, event) = widgets::number_field(
            ui,
            egui::Id::new(("medium-number", base)),
            &js::number_to_string(value),
            Field::new().width(72.0).aria(&format!("Maximum supply of {label}")).invalid(invalid),
        );
        aria_invalid(&response, invalid);
        // `acceptNumber`: a cleared or half-typed field ("", "-", "1e") is
        // marked while typed and applies nothing; left so, it goes back to
        // the value in force and the header says why.
        let (text, change) = match event {
            FieldEvent::Input(t) => (t, false),
            FieldEvent::Change(t) => (t, true),
            FieldEvent::None => return,
        };
        if widgets::value_as_number(&text).is_finite() {
            ui.data_mut(|d| d.remove::<bool>(invalid_id));
            shell.debounce("medium", 0.12, Deferred::Medium { base: base.to_string(), value: text });
        } else if change {
            ui.data_mut(|d| d.remove::<bool>(invalid_id));
            shell.status_warn("That field was empty or not a number, so nothing changed; it shows the value still in force.");
        } else {
            ui.data_mut(|d| d.insert_temp(invalid_id, true));
        }
    });
}

// `modelEditorCard`

/// `draftKey()`: `${workspace}:${mode}`.
fn draft_key(shell: &Shell) -> String {
    let workspace = shell.ui.workspace.unwrap_or(shell.ui.last_workspace);
    format!("{}:{}", workspace.as_str(), shell.store.model_editor_mode.as_deref().unwrap_or(""))
}

/// `discardModelEditorDraft(mode)`.
fn discard_model_editor_draft(shell: &mut Shell, mode: Option<&str>) {
    let mode = mode.map(str::to_string).or_else(|| shell.store.model_editor_mode.clone()).unwrap_or_default();
    if shell.store.model_editor_mode.as_deref() == Some(mode.as_str()) {
        let key = draft_key(shell);
        shell.ui.drafts.shift_remove(&key);
    }
    shell.ui.drafts.shift_remove(&format!("model:{mode}"));
}

/// `showModelEditError(error)`.
fn show_model_edit_error(shell: &mut Shell, message: String) {
    shell.status_error(format!("Could not edit the model: {message}"));
    shell.ui.model_editor_error = Some(message);
}

/// The value of a named field of the open form, or its default.
fn draft_value(shell: &Shell, key: &str, name: &str, default: &str) -> String {
    shell.ui.drafts.get(key).and_then(|f| f.get(name)).cloned().unwrap_or_else(|| default.to_string())
}

fn set_draft_value(shell: &mut Shell, key: &str, name: &str, value: String) {
    shell.ui.drafts.entry(key.to_string()).or_default().insert(name.to_string(), value);
}

/// A labelled named field of the form (`<label>` over `<input name>`).
#[allow(clippy::too_many_arguments)]
fn form_field(ui: &mut Ui, shell: &mut Shell, key: &str, name: &str, label: (&str, &str), default: &str, placeholder: &str, aria: &str, rows: Option<usize>) {
    let p = Palette::current(ui.ctx());
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(RichText::new(label.0).size(12.0).color(p.ink));
        if !label.1.is_empty() {
            ui.label(RichText::new(label.1).size(10.5).color(p.ink_3));
        }
    });
    let value = draft_value(shell, key, name, default);
    let mut field = Field::new().placeholder(placeholder).aria(aria);
    if let Some(rows) = rows {
        field = field.rows(rows);
    }
    let id = egui::Id::new(("model-editor", key, name));
    let (_, event) = if rows.is_some() {
        ui.scope(|ui| {
            ui.style_mut().override_font_id = Some(egui::FontId::monospace(12.0));
            widgets::text_field(ui, id, &value, field)
        })
        .inner
    } else {
        widgets::text_field(ui, id, &value, field)
    };
    if let Some(text) = event.value() {
        set_draft_value(shell, key, name, text);
    }
}

/// `modelEditorCard(store)`.
pub fn model_editor_card(ui: &mut Ui, shell: &mut Shell) {
    let p = Palette::current(ui.ctx());
    if shell.store.is_zuniga_study() {
        widgets::card(ui, "Complete source models", None, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Model").size(12.0).color(p.ink));
                let options: Vec<(String, String)> = shell.store.species.iter().map(|sp| (sp.id.clone(), sp.id.clone())).collect();
                let current = shell.store.model_editor_species_id.clone().unwrap_or_default();
                let width = ui.available_width() - 8.0;
                if let Some(id) = select_or_first(ui, "model-editor-species", &current, &options, width, true, "Model") {
                    shell.store.set_model_editor_species(&id);
                }
            });
            widgets::hint(
                ui,
                "All reactions and metabolites are retained for the calculation and model inspection. Only selected complementing pathways appear on the editable map.",
            );
            widgets::hint(
                ui,
                "Source biology is read-only in this reproducible case. Choose a documented scenario to apply its perturbation and conditions; use the reaction library to add any existing model reaction to the drawing.",
            );
        });
        return;
    }
    widgets::card(ui, "Model editor", None, |ui| {
        ui.label(RichText::new("Model / species").size(12.0).color(p.ink));
        let options: Vec<(String, String)> = shell
            .store
            .species
            .iter()
            .map(|sp| {
                let name = shell.store.models.get(&sp.file).map_or_else(|| sp.id.clone(), |m| m.name.clone());
                (sp.id.clone(), format!("{name} [{}]", sp.id))
            })
            .collect();
        let current = shell.store.model_editor_species_id.clone().unwrap_or_default();
        let width = ui.available_width() - 8.0;
        if let Some(id) = select_or_first(ui, "model-editor-species", &current, &options, width, true, "Model to edit") {
            shell.ui.model_editor_error = None;
            shell.store.set_model_editor_species(&id);
        }
        let summary = shell.store.model_editor_model().map(|m| (m.metabolites.len(), m.reactions.len(), m.compartment_ids()));
        match &summary {
            Some((metabolites, reactions, compartments)) => {
                Rich::new(ui, 12.0)
                    .bold(&metabolites.to_string())
                    .plain(" metabolites · ")
                    .bold(&reactions.to_string())
                    .plain(" reactions · ")
                    .bold(&compartments.len().to_string())
                    .plain(" compartments.")
                    .show(ui);
            }
            None => {
                widgets::empty(ui, "No editable model.");
            }
        }
        ui.add_space(6.0);
        let mut clicked = [false; 4];
        widgets::grow_row(ui, 2, |ui, i| {
            clicked[i] = Btn::new(["+ Metabolite", "+ Reaction"][i]).wide().show(ui).clicked();
        });
        widgets::grow_row(ui, 2, |ui, i| {
            clicked[2 + i] = Btn::new(["Export COBRA JSON", "Export SBML"][i]).wide().show(ui).clicked();
        });
        for (i, kind) in ["metabolite", "reaction"].into_iter().enumerate() {
            if clicked[i] {
                // `new-metabolite` / `new-reaction`: toggle the form, on the
                // Biological model page.
                if shell.ui.workspace != Some(Workspace::Model) {
                    shell.open_workspace(Workspace::Model);
                }
                let next = if shell.store.model_editor_mode.as_deref() == Some(kind) { None } else { Some(kind) };
                shell.ui.model_editor_error = None;
                if let Err(error) = shell.store.set_model_editor_mode(next) {
                    shell.status_error(error.to_string());
                }
            }
        }
        if clicked[2] {
            shell.export_model(false);
        }
        if clicked[3] {
            shell.export_model(true);
        }
        widgets::hint(
            ui,
            "Additions change the selected biological model and travel with both exports. The SBML is Level 3 Version 1 with the fbc package, Version 2 — what cobrapy and the COBRA Toolbox read.",
        );
        let mode = shell.store.model_editor_mode.clone();
        if let (Some(mode), Some((_, _, compartments))) = (mode, summary) {
            ui.add_space(10.0);
            widgets::boxed(ui, |ui| match mode.as_str() {
                "metabolite" => metabolite_form(ui, shell, &compartments),
                "reaction" => reaction_form(ui, shell),
                _ => {}
            });
        }
        // `.model-editor-error`.
        if let Some(error) = shell.ui.model_editor_error.clone() {
            ui.add_space(9.0);
            widgets::tinted(ui, p.danger_bg, p.danger_line, |ui| {
                ui.add(egui::Label::new(RichText::new(error).size(11.5).color(p.danger)).wrap());
            });
        }
    });
}

/// `.model-editor-form[data-editor-kind="metabolite"]`.
fn metabolite_form(ui: &mut Ui, shell: &mut Shell, compartments: &[String]) {
    let p = Palette::current(ui.ctx());
    let key = draft_key(shell);
    ui.label(RichText::new("New metabolite").size(12.0).family(bold()).color(p.ink_2));
    form_field(ui, shell, &key, "id", ("ID *", ""), "", "lac__L_c", "New metabolite ID", None);
    form_field(ui, shell, &key, "name", ("Name *", ""), "", "L-lactate", "New metabolite name", None);
    ui.add_space(6.0);
    ui.label(RichText::new("Compartment *").size(12.0).color(p.ink));
    let names = shell.store.model_editor_model().map(|m| m.compartment_names.clone()).unwrap_or_default();
    let options: Vec<(String, String)> =
        compartments.iter().map(|cid| (cid.clone(), format!("{cid} · {}", cell_compartment_name(cid, &names, "en")))).collect();
    let first = compartments.first().cloned().unwrap_or_default();
    let current = draft_value(shell, &key, "compartment", &first);
    let width = ui.available_width() - 8.0;
    if let Some(cid) = select_or_first(ui, "model-editor-compartment", &current, &options, width, true, "New metabolite compartment") {
        set_draft_value(shell, &key, "compartment", cid);
    }
    ui.columns(2, |cols| {
        form_field(&mut cols[0], shell, &key, "formula", ("Formula", "(optional)"), "", "C3H5O3", "New metabolite formula", None);
        form_field(&mut cols[1], shell, &key, "charge", ("Charge", "(optional)"), "0", "", "New metabolite charge", None);
    });
    ui.add_space(10.0);
    let (cancel, create) = action_row(ui, "Create metabolite");
    if cancel {
        cancel_model_editor(shell);
    }
    if create {
        // `createMetaboliteFromEditor`.
        let spec = json!({
            "id": draft_value(shell, &key, "id", ""),
            "name": draft_value(shell, &key, "name", ""),
            "compartment": draft_value(shell, &key, "compartment", &first),
            "formula": draft_value(shell, &key, "formula", ""),
            "charge": draft_value(shell, &key, "charge", "0"),
        });
        match shell.store.create_model_metabolite(&spec) {
            Ok(_) => {
                discard_model_editor_draft(shell, Some("metabolite"));
                shell.ui.model_editor_error = None;
                if let Err(error) = shell.store.set_model_editor_mode(None) {
                    shell.status_error(error.to_string());
                }
            }
            Err(error) => show_model_edit_error(shell, error.to_string()),
        }
    }
}

/// `.model-editor-form[data-editor-kind="reaction"]`.
fn reaction_form(ui: &mut Ui, shell: &mut Shell) {
    let p = Palette::current(ui.ctx());
    let key = draft_key(shell);
    ui.label(RichText::new("New reaction").size(12.0).family(bold()).color(p.ink_2));
    form_field(ui, shell, &key, "id", ("ID *", ""), "", "LDH_NEW", "New reaction ID", None);
    form_field(ui, shell, &key, "name", ("Name", ""), "", "Lactate dehydrogenase", "New reaction name", None);
    form_field(
        ui,
        shell,
        &key,
        "stoichiometry",
        ("Stoichiometry *", ""),
        "",
        "pyr_c:-1, nadh_c:-1, h_c:-1, lac__L_c:1, nad_c:1",
        "New reaction stoichiometry",
        Some(5),
    );
    widgets::hint(ui, "Negative coefficient = substrate, positive = product. JSON is accepted too.");
    ui.columns(2, |cols| {
        form_field(&mut cols[0], shell, &key, "lower", ("Lower bound *", ""), "0", "", "New reaction lower bound", None);
        form_field(&mut cols[1], shell, &key, "upper", ("Upper bound *", ""), "1000", "", "New reaction upper bound", None);
    });
    form_field(ui, shell, &key, "gpr", ("GPR", ""), "", "geneA and (geneB or geneC)", "New reaction GPR", None);
    form_field(ui, shell, &key, "subsystem", ("Subsystem", "(optional)"), "", "Glycolysis", "New reaction subsystem", None);
    ui.add_space(9.0);
    let add_to_map = draft_value(shell, &key, "addToMap", "true") == "true";
    let (_, toggled) = widgets::checkbox(ui, add_to_map, RichText::new("Add to the map after creating").size(12.0), true, None);
    if let Some(on) = toggled {
        set_draft_value(shell, &key, "addToMap", on.to_string());
    }
    ui.add_space(10.0);
    let (cancel, create) = action_row(ui, "Create reaction");
    if cancel {
        cancel_model_editor(shell);
    }
    if create {
        // `createReactionFromEditor`.
        let spec = json!({
            "id": draft_value(shell, &key, "id", ""),
            "name": draft_value(shell, &key, "name", ""),
            "metabolites": draft_value(shell, &key, "stoichiometry", ""),
            "lowerBound": draft_value(shell, &key, "lower", "0"),
            "upperBound": draft_value(shell, &key, "upper", "1000"),
            "geneReactionRule": draft_value(shell, &key, "gpr", ""),
            "subsystem": draft_value(shell, &key, "subsystem", ""),
        });
        match shell.store.create_model_reaction(&spec, add_to_map) {
            Ok(created) => {
                discard_model_editor_draft(shell, Some("reaction"));
                shell.ui.model_editor_error = None;
                if let Err(error) = shell.store.set_model_editor_mode(None) {
                    shell.status_error(error.to_string());
                }
                if created.map_result.is_some() {
                    shell.center_on_reaction(&created.merged_id);
                }
            }
            Err(error) => show_model_edit_error(shell, error.to_string()),
        }
    }
}

/// `.model-editor-actions`: Cancel and the primary Create, which grows.
fn action_row(ui: &mut Ui, create: &str) -> (bool, bool) {
    ui.horizontal(|ui| {
        let cancel = widgets::button(ui, "Cancel").clicked();
        let create = Btn::new(create).primary().wide().show(ui).clicked();
        (cancel, create)
    })
    .inner
}

/// `cancel-model-editor`.
fn cancel_model_editor(shell: &mut Shell) {
    discard_model_editor_draft(shell, None);
    shell.ui.model_editor_error = None;
    if let Err(error) = shell.store.set_model_editor_mode(None) {
        shell.status_error(error.to_string());
    }
}

// `modelOverviewCard`

/// `modelOverviewCard(store)`.
pub fn model_overview_card(ui: &mut Ui, shell: &mut Shell) {
    let p = Palette::current(ui.ctx());
    let store = &shell.store;
    let map = store.map.as_ref();
    let merged = store.community.as_ref().map(|c| &c.merged);
    let rows = [
        ("Cells", map.map_or(0, |m| m.cells().len())),
        ("Compartments", map.map_or(0, |m| m.stats().compartments)),
        ("Reactions", merged.map_or(0, |m| m.reactions.len())),
        ("Metabolites", merged.map_or(0, |m| m.metabolites.len())),
        ("Genes (GPR)", merged.map_or(0, |m| m.genes.len())),
    ];
    widgets::card(ui, "Model overview", None, |ui| {
        for (label, value) in rows {
            ui.horizontal(|ui| {
                ui.label(RichText::new(label).size(12.5).color(p.ink_3));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(value.to_string()).size(12.5).color(p.ink_2));
                });
            });
            ui.add_space(4.0);
        }
    });
}

// `communityOmicsCard`

/// A row label of a form (`<label>` beside a control).
fn row_label(ui: &mut Ui, s: &str) {
    let p = Palette::current(ui.ctx());
    ui.label(RichText::new(s).size(12.0).color(p.ink_2));
}

/// The number a number field holds, as the browser reports it: `''` for a
/// value that is not one, so `Number(el.value)` is 0 there.
fn number_value(text: &str) -> Option<f64> {
    let n = widgets::value_as_number(text);
    n.is_finite().then_some(n)
}

/// `communityOmicsCard(store)`.
pub fn community_omics_card(ui: &mut Ui, shell: &mut Shell) {
    let p = Palette::current(ui.ctx());
    widgets::card(ui, "Metatranscriptomics", None, |ui| {
        let has_dataset = shell.store.omics.dataset.is_some();
        let enabled = shell.store.omics.enabled;
        ui.horizontal(|ui| {
            let reserve = if has_dataset { 70.0 } else { 0.0 };
            let label = if has_dataset { "Replace CSV/TSV" } else { "Import CSV/TSV" };
            let width = ui.available_width() - reserve;
            if ui.add_sized([width, 28.0], egui::Button::new(RichText::new(label).size(12.5))).clicked() {
                shell.import_metatranscriptomics();
            }
            if has_dataset && widgets::button(ui, if enabled { "Hide" } else { "Show" }).clicked() {
                shell.store.set_community_omics(CommunityOmicsPatch { enabled: Some(!enabled), ..Default::default() });
            }
        });
        let Some(dataset) = shell.store.omics.dataset.clone() else {
            widgets::hint(ui, "Long format: species/MAG, gene, condition, expression (TPM, FPKM, RPKM, CPM or counts also work).");
            return;
        };
        let o = shell.store.omics.clone();
        Rich::new(ui, 12.0)
            .bold(&dataset.source)
            .plain(&format!(" · {} genes/MAG · {} condition(s)", dataset.row_count, dataset.conditions.len()))
            .show(ui);
        ui.horizontal(|ui| {
            if Btn::new("Condition").pressed(o.mode == "single").show(ui).clicked() {
                shell.store.set_community_omics(CommunityOmicsPatch { mode: Some("single".into()), ..Default::default() });
            }
            if Btn::new("A vs B").pressed(o.mode == "compare").show(ui).clicked() {
                shell.store.set_community_omics(CommunityOmicsPatch { mode: Some("compare".into()), ..Default::default() });
            }
        });
        let options: Vec<(String, String)> = dataset.conditions.iter().map(|c| (c.clone(), c.clone())).collect();
        ui.horizontal(|ui| {
            row_label(ui, "A");
            let width = ui.available_width() - 8.0;
            let current = o.condition_a.clone().unwrap_or_default();
            if let Some(c) = select_or_first(ui, "community-omics-condition-a", &current, &options, width, true, "Community expression condition A") {
                shell.store.set_community_omics(CommunityOmicsPatch { condition_a: Some(Some(c)), ..Default::default() });
            }
        });
        if o.mode == "compare" {
            ui.horizontal(|ui| {
                row_label(ui, "B");
                let width = ui.available_width() - 8.0;
                let current = o.condition_b.clone().unwrap_or_default();
                if let Some(c) = select_or_first(ui, "community-omics-condition-b", &current, &options, width, true, "Community expression condition B") {
                    shell.store.set_community_omics(CommunityOmicsPatch { condition_b: Some(Some(c)), ..Default::default() });
                }
            });
            ui.horizontal(|ui| {
                let metrics = [("log2fc".to_string(), "log₂(B/A)".to_string()), ("difference".to_string(), "B − A".to_string())];
                let width = ui.available_width() - 90.0;
                if let Some(m) = select_or_first(ui, "community-omics-metric", &o.metric, &metrics, width, true, "Community expression comparison") {
                    shell.store.set_community_omics(CommunityOmicsPatch { metric: Some(m), ..Default::default() });
                }
                let (_, event) = widgets::number_field(
                    ui,
                    egui::Id::new("community-omics-pseudocount"),
                    &js::number_to_string(o.pseudocount),
                    Field::new().width(70.0).title("Pseudocount").aria("Community expression pseudocount"),
                );
                if let Some(text) = event.value() {
                    let pseudocount = number_value(&text).unwrap_or(0.0);
                    shell.store.set_community_omics(CommunityOmicsPatch { pseudocount: Some(pseudocount), ..Default::default() });
                }
            });
        }
        widgets::section_title(ui, "GIMME context-specific model");
        widgets::hint(ui, "Penalises flux through reactions below the expression threshold while holding a minimum fraction of the metabolic objective.");
        ui.horizontal(|ui| {
            row_label(ui, "Objective");
            let (_, event) = widgets::number_field(
                ui,
                egui::Id::new("gimme-objective-fraction"),
                &js::number_to_string(js::round(100.0 * o.gimme_objective_fraction)),
                Field::new().width(75.0).aria("GIMME objective fraction"),
            );
            ui.label(RichText::new("%").size(12.0).color(p.ink_2));
            if let Some(text) = event.value() {
                let percent = number_value(&text).unwrap_or(0.0);
                shell.store.omics.gimme_objective_fraction = js::max(0.0, js::min(1.0, percent / 100.0));
            }
        });
        ui.horizontal(|ui| {
            row_label(ui, "Threshold");
            let shown = o.gimme_threshold.map(js::number_to_string).unwrap_or_default();
            let (_, event) = widgets::number_field(
                ui,
                egui::Id::new("gimme-threshold"),
                &shown,
                Field::new().width(ui.available_width()).placeholder("auto: P25").aria("GIMME threshold"),
            );
            if let Some(text) = event.value() {
                shell.store.omics.gimme_threshold = number_value(&text);
            }
        });
        let fixed_mu = shell.store.solution.as_ref().is_some_and(|s| s.steady_com.is_some());
        ui.horizontal(|ui| {
            let label = format!("Run GIMME · {}", o.condition_a.clone().unwrap_or_default());
            let run = Btn::new(label.as_str()).enabled(!fixed_mu).title(if fixed_mu { "Run an ordinary analysis before GIMME" } else { "" }).show(ui);
            if run.clicked() && !fixed_mu {
                if let Err(error) = shell.store.start_community_gimme(CommunityOmicsPatch::default()) {
                    shell.status_error(error.to_string());
                }
            }
            if o.gimme_enabled && widgets::button(ui, "Return FBA").clicked() {
                shell.store.disable_gimme();
            }
        });
        if fixed_mu {
            widgets::hint(ui, "Run FBA, pFBA or Loopless before using GIMME with this model.");
        }
        if let Some(result) = o.gimme_result.as_ref().filter(|r| r.solution.status.is_optimal()) {
            Rich::new(ui, 12.0)
                .plain("Threshold ")
                .bold(&to_precision(result.threshold.unwrap_or(f64::NAN), 4))
                .plain(" · inconsistency ")
                .bold(&to_precision(result.inconsistency_score.unwrap_or(f64::NAN), 4))
                .plain(" · inactive penalized ")
                .bold(&result.inactive.len().to_string())
                .show(ui);
        }
        widgets::section_title(ui, "Mapping report");
        let r = o.report.as_ref();
        let percent = |x: f64| js::number_to_string(js::round(100.0 * x));
        Rich::new(ui, 12.0)
            .plain("Genes: ")
            .bold(&format!("{}/{}", r.map_or(0, |r| r.measured_genes), r.map_or(0, |r| r.genes_in_models)))
            .plain(&format!(" ({}%) · GPR reactions: ", percent(r.map_or(0.0, |r| r.gene_coverage))))
            .bold(&format!("{}/{}", r.map_or(0, |r| r.mapped_reactions), r.map_or(0, |r| r.gpr_reactions)))
            .plain(&format!(" ({}%)", percent(r.map_or(0.0, |r| r.reaction_coverage))))
            .show(ui);
        if let Some(r) = r.filter(|r| !r.unmatched_species.is_empty()) {
            widgets::hint(ui, format!("MAG/species not found: {}", r.unmatched_species.join(", ")));
        }
        for x in r.map(|r| r.species.iter().take(12).collect::<Vec<_>>()).unwrap_or_default() {
            let share = if x.gpr_reactions > 0 { js::round(100.0 * x.mapped_reactions as f64 / x.gpr_reactions as f64) } else { 0.0 };
            ui.horizontal(|ui| {
                Rich::new(ui, 12.0).ink(p.ink, p.ink).bold(&x.species).plain(&format!(" · genes {}/{}", x.measured_genes, x.genes)).show(ui);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(format!("{}% rxn", js::number_to_string(share))).size(11.5).color(p.ink_2));
                });
            });
        }
    });
}

// `omicsOverlayCard`

/// `.omics-scale`: the colour ramp of the overlay.
fn gradient_bar(ui: &mut Ui, stops: &[(f32, &str)]) {
    let p = Palette::current(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 12.0), Sense::hover());
    let mut mesh = egui::Mesh::default();
    for pair in stops.windows(2) {
        let (a, ca) = (pair[0].0, css_color(pair[0].1, p.line));
        let (b, cb) = (pair[1].0, css_color(pair[1].1, p.line));
        let (x0, x1) = (rect.left() + rect.width() * a, rect.left() + rect.width() * b);
        let i = mesh.vertices.len() as u32;
        mesh.colored_vertex(pos2(x0, rect.top()), ca);
        mesh.colored_vertex(pos2(x0, rect.bottom()), ca);
        mesh.colored_vertex(pos2(x1, rect.top()), cb);
        mesh.colored_vertex(pos2(x1, rect.bottom()), cb);
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i + 1, i + 3, i + 2);
    }
    ui.painter().add(egui::Shape::mesh(mesh));
    ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, p.line), egui::StrokeKind::Inside);
}

/// `omicsOverlayCard(store)`.
pub fn omics_overlay_card(ui: &mut Ui, shell: &mut Shell) {
    let p = Palette::current(ui.ctx());
    let settings = shell.store.omics_settings.clone();
    let badge = shell.store.omics_dataset.as_ref().map(|d| if d.data_type == "protein" { "proteomics" } else { "genes" });
    widgets::card(ui, "Omics Overlay", badge, |ui| {
        ui.horizontal(|ui| {
            row_label(ui, "Data type");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let types = [("gene".to_string(), "Gene expression".to_string()), ("protein".to_string(), "Proteomics".to_string())];
                if let Some(t) = select_or_first(ui, "omics-type", &settings.data_type, &types, 130.0, true, "Omics overlay data type") {
                    shell.store.set_omics_settings(OmicsSettingsPatch { data_type: Some(t), ..Default::default() });
                }
            });
        });
        let Some(dataset) = shell.store.omics_dataset.clone() else {
            ui.add_space(9.0);
            if Btn::new("Load CSV").primary().wide().show(ui).clicked() {
                shell.import_omics();
            }
            Rich::new(ui, 12.0)
                .plain("Wide format: ")
                .bold("id,Condition_A,Condition_B")
                .plain(". For communities, add an optional ")
                .bold("species")
                .plain(" column.")
                .show(ui);
            widgets::hint(
                ui,
                "IDs map to each reaction's GPR. AND = minimum across subunits; OR = maximum across isoenzymes. This visual overlay preserves the FBA.",
            );
            return;
        };
        let overlay = shell.store.omics_overlay();
        let conditions = dataset.conditions.clone();
        let comparison = settings.mode == "compare" && conditions.len() >= 2;

        // `.omics-file-meta`.
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add(egui::Label::new(RichText::new(&dataset.source).size(12.0).family(bold()).color(p.ink)).truncate()).on_hover_text(&dataset.source);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let plural = if conditions.len() == 1 { "" } else { "s" };
                ui.label(RichText::new(format!("{} IDs · {} condition{plural}", dataset.row_count, conditions.len())).size(10.0).color(p.ink_3));
            });
        });
        ui.add_space(6.0);
        let (_, toggled) = widgets::checkbox(ui, settings.enabled, widgets::strong("Show overlay").size(12.5), true, None);
        if let Some(on) = toggled {
            shell.store.set_omics_settings(OmicsSettingsPatch { enabled: Some(on), ..Default::default() });
        }
        ui.horizontal(|ui| {
            row_label(ui, "View");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let modes = [("single".to_string(), "One condition".to_string()), ("compare".to_string(), "Compare A vs B".to_string())];
                if let Some(m) = select_or_first(ui, "omics-mode", &settings.mode, &modes, 150.0, conditions.len() >= 2, "Omics overlay view") {
                    shell.store.set_omics_settings(OmicsSettingsPatch { mode: Some(m), ..Default::default() });
                }
            });
        });
        let options: Vec<(String, String)> = conditions.iter().map(|c| (c.clone(), c.clone())).collect();
        // `.omics-condition-grid`.
        egui::Grid::new("omics-condition-grid").num_columns(2).spacing(Vec2::new(8.0, 6.0)).show(ui, |ui| {
            ui.add_sized([82.0, 18.0], egui::Label::new(RichText::new(if comparison { "Condition A" } else { "Condition" }).size(11.0).color(p.ink_2)));
            let width = (ui.available_width() - 8.0).max(80.0);
            let current = settings.condition_a.clone().unwrap_or_default();
            if let Some(c) = select_or_first(ui, "omics-condition-a", &current, &options, width, true, "Omics overlay condition A") {
                shell.store.set_omics_settings(OmicsSettingsPatch { condition_a: Some(Some(c)), ..Default::default() });
            }
            ui.end_row();
            if comparison {
                ui.add_sized([82.0, 18.0], egui::Label::new(RichText::new("Condition B").size(11.0).color(p.ink_2)));
                let current = settings.condition_b.clone().unwrap_or_default();
                if let Some(c) = select_or_first(ui, "omics-condition-b", &current, &options, width, true, "Omics overlay condition B") {
                    shell.store.set_omics_settings(OmicsSettingsPatch { condition_b: Some(Some(c)), ..Default::default() });
                }
                ui.end_row();
            }
        });
        if comparison {
            ui.horizontal(|ui| {
                row_label(ui, "Comparison");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let metrics = [("log2fc".to_string(), "log₂(B/A)".to_string()), ("difference".to_string(), "B − A".to_string())];
                    if let Some(m) = select_or_first(ui, "omics-metric", &settings.metric, &metrics, 150.0, true, "Omics overlay comparison metric") {
                        shell.store.set_omics_settings(OmicsSettingsPatch { metric: Some(m), ..Default::default() });
                    }
                });
            });
            if settings.metric == "log2fc" {
                ui.horizontal(|ui| {
                    row_label(ui, "Pseudocount");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (_, event) = widgets::number_field(
                            ui,
                            egui::Id::new("omics-pseudocount"),
                            &js::number_to_string(settings.pseudocount),
                            Field::new().width(90.0).aria("Omics overlay pseudocount"),
                        );
                        if let Some(text) = event.value() {
                            // `Math.max(0, Number(p) || 0)` in the store.
                            let value = number_value(&text).unwrap_or(0.0);
                            shell.store.set_omics_settings(OmicsSettingsPatch { pseudocount: Some(value), ..Default::default() });
                        }
                    });
                });
            }
        }
        ui.add_space(10.0);
        if comparison {
            gradient_bar(ui, &[(0.0, "#2563eb"), (0.5, "#f8fafc"), (1.0, "#dc2626")]);
        } else {
            gradient_bar(ui, &[(0.0, "#e2e8f0"), (0.55, "#fca5a5"), (1.0, "#b91c1c")]);
        }
        let scale = overlay.scale.as_ref();
        let (left, middle, right) = if comparison {
            (
                format!("{} ↑", settings.condition_a.clone().unwrap_or_default()),
                if settings.metric == "log2fc" { "0 log₂FC" } else { "0 Δ" }.to_string(),
                format!("{} ↑", settings.condition_b.clone().unwrap_or_default()),
            )
        } else {
            (
                format_flux(scale.and_then(|s| s.min).filter(|v| v.is_finite()), 3),
                "expression / abundance".to_string(),
                format_flux(scale.and_then(|s| s.max).filter(|v| v.is_finite()), 3),
            )
        };
        ui.columns(3, |cols| {
            cols[0].label(RichText::new(left).size(10.0).color(p.ink_3));
            cols[1].vertical_centered(|ui| ui.label(RichText::new(middle).size(10.0).color(p.ink_3)));
            cols[2].with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| ui.label(RichText::new(right).size(10.0).color(p.ink_3)));
        });
        if overlay.enabled {
            Rich::new(ui, 12.0)
                .bold(&format!("{}/{}", overlay.matched_reactions, overlay.total_drawn))
                .plain(" drawn reactions have a GPR-derived omics value.")
                .show(ui);
        } else {
            widgets::hint(ui, "Overlay is hidden. Enable Show overlay to display GPR-derived values.");
        }
        widgets::hint(
            ui,
            "When both overlays are enabled, this overlay takes precedence for its matches; community expression fills other matches. Unmatched reactions appear in gray.",
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let width = ui.available_width() - 80.0;
            if ui.add_sized([width, 28.0], egui::Button::new(RichText::new("Replace CSV").size(12.5))).clicked() {
                shell.import_omics();
            }
            if widgets::button(ui, "Remove").clicked() {
                shell.store.clear_omics(true);
            }
        });
        widgets::hint(ui, "In comparisons, blue = higher in A and red = higher in B. Arrowheads show the direction of the solved flux.");
    });
}
