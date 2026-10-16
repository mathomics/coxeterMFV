//! The Edit Model pages through the real interface: species and
//! abundance, the shared medium, the model editor, the model overview and
//! the two omics cards (`speciesCard`, `mediumCard`, `modelEditorCard`,
//! `modelOverviewCard`, `communityOmicsCard`, `omicsOverlayCard` of
//! `src/ui/panels.js`, with their `onSidebarClick` / `onSidebarInput`
//! cases of `src/ui/app.js`). Small functional checks: each control acts on
//! the store as the JS handler does.

mod common;

use std::sync::Arc;

use coxeter_app::Shell;
use coxeter_app::files::{ScriptedDialogs, SharedDialogs};
use coxeter_app::ui_state::Workspace;
use coxeter_core::model::Model;
use coxeter_state::{NoFetcher, StatusState, Store};
use egui::accesskit::{Invalid, Role};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

type H = Harness<'static, Shell>;

fn open(h: &mut H, ws: Workspace) {
    h.state_mut().open_workspace(ws);
    common::settle(h);
}

/// Pick an option of a select that may be below the fold, scrolling the
/// popup to the option too.
fn choose(h: &mut H, label: &str, option: &str) {
    common::control(h, label).scroll_to_me();
    let _ = h.try_run();
    common::control(h, label).click();
    let _ = h.try_run();
    let find = |h: &H| h.query_all_by(|n| n.role() == Role::Button && n.label().as_deref() == Some(option)).last().map(|n| n.accesskit_node().id());
    let id = find(h).expect("the option");
    h.get_by(|n| n.id() == id).scroll_to_me();
    let _ = h.try_run();
    h.get_by(|n| n.id() == id).click();
    common::settle(h);
}

fn status(h: &H) -> String {
    h.state().store.status().message.clone()
}

/// The display name of species `i` (`model.cell.name`).
fn species_name(h: &H, i: usize) -> String {
    let store = &h.state().store;
    store.models.get(&store.species[i].file).unwrap().cell.name.clone()
}

/// A harness over `store`, its saves landing in a fresh folder.
fn harness_over(store: Store, name: &str) -> (H, SharedDialogs, std::path::PathBuf) {
    let dir = common::temp_dir(name);
    let dialogs = SharedDialogs::new(ScriptedDialogs::saving_to(&dir));
    let mut shell = Shell::new(store, Box::new(dialogs.clone()));
    shell.boot(None);
    (common::harness_of(shell), dialogs, dir)
}

// Species

#[test]
fn species_are_switched_and_weighted() {
    let mut h = common::harness(None);
    common::click(&mut h, "Edit Model");
    common::click(&mut h, "Species & abundance");
    assert!(common::shows(&h, "4 active"));
    let first = species_name(&h, 0);
    // `species-toggle`.
    common::click(&mut h, &format!("Include {first} in the analysis"));
    assert!(!h.state().store.species[0].enabled);
    assert!(common::shows(&h, "3 active"));
    assert!(common::disabled(&h, &format!("Abundance of {first}")), "a switched-off species has no abundance");
    // `abundance` (debounced).
    let second = species_name(&h, 1);
    common::set_value(&mut h, &format!("Abundance of {second}"), 0.5);
    assert_eq!(h.state().store.species[1].abundance, 0.5);
    assert!(common::shows(&h, "50%"));
}

#[test]
fn models_are_loaded_from_files_and_yeast9_is_downloaded() {
    let store = common::store().with_fetcher(Arc::new(NoFetcher));
    let (mut h, dialogs, _) = harness_over(store, "model-load");
    open(&mut h, Workspace::Community);
    // `import-models`.
    dialogs.with(|d| d.opens.push_back(vec![common::repo_root().join("models").join("ecoli_core_mc.json")]));
    common::click(&mut h, "Load model(s)");
    assert_eq!(h.state().store.species.len(), 1, "{}", status(&h));
    assert!(common::shows(&h, "1 active"));
    // `load-yeast9`: with no network here, the download says it failed.
    common::click(&mut h, "Yeast9 9.1.1");
    assert_eq!(h.state().store.status().state, StatusState::Error, "{}", status(&h));
}

// Medium

#[test]
fn the_medium_takes_a_slider_or_an_exact_number() {
    let mut h = common::harness(None);
    open(&mut h, Workspace::Medium);
    // `medium` (debounced).
    common::set_value(&mut h, "Supply of Glucose", 5.0);
    assert_eq!(h.state().store.medium.get("glc__D").copied(), Some(5.0));
    // `medium-number`.
    common::type_into(&mut h, "Maximum supply of Glucose", "7.5");
    assert_eq!(h.state().store.medium.get("glc__D").copied(), Some(7.5));
    // `acceptNumber`: half-typed is marked, left empty goes back.
    let field = "Maximum supply of Glucose";
    common::control(&h, field).focus();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    common::control(&h, field).type_text("-");
    h.run_steps(2);
    assert_eq!(common::control(&h, field).accesskit_node().data().invalid(), Some(Invalid::True));
    h.key_press(egui::Key::Enter);
    common::settle(&mut h);
    assert_eq!(h.state().store.medium.get("glc__D").copied(), Some(7.5), "nothing applied");
    assert_eq!(h.state().store.status().state, StatusState::Warn);
    assert!(status(&h).starts_with("That field was empty or not a number"));
    assert_eq!(common::control(&h, field).value().as_deref(), Some("7.5"), "it shows the value in force");
}

// Model editor

#[test]
fn a_metabolite_is_created_and_a_bad_one_refused() {
    let mut h = common::harness(None);
    common::click(&mut h, "Edit Model");
    common::click(&mut h, "Biological model");
    assert_eq!(h.state().ui.workspace, Some(Workspace::Model));
    // `new-metabolite`; a refused one says why.
    common::click(&mut h, "+ Metabolite");
    assert_eq!(h.state().store.model_editor_mode.as_deref(), Some("metabolite"));
    common::click(&mut h, "Create metabolite");
    assert!(status(&h).starts_with("Could not edit the model: "), "{}", status(&h));
    assert!(h.state().ui.model_editor_error.is_some());
    // `create-metabolite`.
    common::type_into(&mut h, "New metabolite ID", "test_e");
    common::type_into(&mut h, "New metabolite name", "Test");
    common::click(&mut h, "Create metabolite");
    let store = &h.state().store;
    assert!(store.model_editor_model().unwrap().metabolites.contains_key("test_e"), "{}", status(&h));
    assert_eq!(store.model_editor_mode, None, "the form closes");
    assert!(h.state().ui.drafts.get("model:metabolite").is_none(), "its draft is discarded");
    assert!(h.state().ui.model_editor_error.is_none());
}

#[test]
fn a_reaction_is_created_and_drawn_and_drafts_survive_a_switch() {
    let mut h = common::harness(None);
    open(&mut h, Workspace::Model);
    // What is typed survives switching between the two forms.
    common::click(&mut h, "+ Metabolite");
    common::type_into(&mut h, "New metabolite ID", "kept_c");
    common::click(&mut h, "+ Reaction");
    assert_eq!(h.state().store.model_editor_mode.as_deref(), Some("reaction"));
    common::click(&mut h, "+ Metabolite");
    assert_eq!(common::control(&h, "New metabolite ID").value().as_deref(), Some("kept_c"));
    // `cancel-model-editor` discards it.
    common::click(&mut h, "Cancel");
    assert_eq!(h.state().store.model_editor_mode, None);
    common::click(&mut h, "+ Metabolite");
    assert_eq!(common::control(&h, "New metabolite ID").value().as_deref(), Some(""));
    // `create-reaction`, added to the map.
    common::click(&mut h, "+ Reaction");
    let species = h.state().store.model_editor_species().unwrap().id.clone();
    let (a, b) = {
        let model = h.state().store.model_editor_model().unwrap();
        let mut ids = model.metabolites.keys().filter(|m| m.ends_with("_c"));
        (ids.next().unwrap().clone(), ids.next().unwrap().clone())
    };
    common::type_into(&mut h, "New reaction ID", "TEST_RXN");
    common::type_into(&mut h, "New reaction stoichiometry", &format!("{a}:-1, {b}:1"));
    common::click(&mut h, "Create reaction");
    let store = &h.state().store;
    assert!(store.model_editor_model().unwrap().reactions.contains_key("TEST_RXN"), "{}", status(&h));
    assert_eq!(store.selected_reaction.as_deref(), Some(format!("{species}@TEST_RXN").as_str()), "drawn and selected");
    assert_eq!(store.model_editor_mode, None);
}

#[test]
fn the_edited_model_is_chosen_and_exported() {
    let (mut h, dialogs, _) = common::harness_saving("model-export", None);
    open(&mut h, Workspace::Model);
    // `model-editor-species`.
    let (id, name) = {
        let store = &h.state().store;
        let sp = &store.species[1];
        (sp.id.clone(), store.models.get(&sp.file).unwrap().name.clone())
    };
    choose(&mut h, "Model to edit", &format!("{name} [{id}]"));
    assert_eq!(h.state().store.model_editor_species_id.as_deref(), Some(id.as_str()));
    // `export-model-json`, `export-model-sbml`.
    common::click(&mut h, "Export COBRA JSON");
    common::click(&mut h, "Export SBML");
    let saved = common::saved(&dialogs);
    assert_eq!(saved.len(), 2);
    assert!(saved[0].0.ends_with(".json") && saved[1].0.ends_with(".xml"), "{:?}", saved.iter().map(|s| &s.0).collect::<Vec<_>>());
    // The overview beside it.
    assert!(common::shows(&h, "Genes (GPR)"));
}

// Omics

/// A cross-feeder whose reactions have gene rules (`{id}_g1`, `{id}_g2`).
fn member(id: &str, makes: &str, needs: &str) -> Model {
    let met = |base: &str, c: &str| json!({"id": format!("{base}_{c}"), "compartment": c});
    let metabolites: Vec<Value> = ["glc", makes, needs].iter().flat_map(|b| [met(b, "e"), met(b, "c")]).collect();
    let mut biomass = serde_json::Map::new();
    biomass.insert("glc_c".into(), json!(-1));
    biomass.insert(format!("{needs}_c"), json!(-1));
    let mut make = serde_json::Map::new();
    make.insert("glc_c".into(), json!(-1));
    make.insert(format!("{makes}_c"), json!(1));
    Model::from_value(&json!({
        "id": id, "compartments": {"e": "Extracellular", "c": "Cytosol"},
        "metabolites": metabolites,
        "reactions": [
            {"id": "EX_glc_e", "metabolites": {"glc_e": -1}, "lower_bound": -10, "upper_bound": 1000},
            {"id": "GLCt", "metabolites": {"glc_e": -1, "glc_c": 1}, "lower_bound": 0, "upper_bound": 100, "gene_reaction_rule": format!("{id}_g1")},
            {"id": "BIOMASS", "metabolites": biomass, "lower_bound": 0, "upper_bound": 100},
            {"id": format!("MK_{makes}"), "metabolites": make, "lower_bound": 0, "upper_bound": 100, "gene_reaction_rule": format!("{id}_g2")},
            {"id": format!("T_{makes}"), "metabolites": {format!("{makes}_c"): -1, format!("{makes}_e"): 1}, "lower_bound": 0, "upper_bound": 100},
            {"id": format!("EX_{makes}_e"), "metabolites": {format!("{makes}_e"): -1}, "lower_bound": 0, "upper_bound": 1000},
            {"id": format!("T_{needs}"), "metabolites": {format!("{needs}_e"): -1, format!("{needs}_c"): 1}, "lower_bound": 0, "upper_bound": 100},
            {"id": format!("EX_{needs}_e"), "metabolites": {format!("{needs}_e"): -1}, "lower_bound": -100, "upper_bound": 1000}
        ],
        "objective": {"BIOMASS": 1}
    }))
    .expect("a valid member model")
}

/// Two members with gene rules, on the Omics page, saves and opens scripted.
fn gene_harness(name: &str) -> (H, SharedDialogs, std::path::PathBuf) {
    let mut store = common::store().with_fetcher(Arc::new(NoFetcher));
    store.set_model("member-A", member("A", "x", "y"));
    store.set_model("member-B", member("B", "y", "x"));
    store.scenarios = vec![json!({
        "id": "genes", "name": "Genes", "objective": "equal-growth", "medium": {"glc": 10},
        "species": [{"id": "A", "model": "member-A", "abundance": 0.5}, {"id": "B", "model": "member-B", "abundance": 0.5}],
    })];
    store.use_scenario("genes").unwrap();
    let dir = common::temp_dir(name);
    let dialogs = SharedDialogs::new(ScriptedDialogs::saving_to(&dir));
    let mut h = common::harness_of(Shell::new(store, Box::new(dialogs.clone())));
    open(&mut h, Workspace::Omics);
    (h, dialogs, dir)
}

#[test]
fn the_omics_overlay_is_loaded_set_and_removed() {
    let (mut h, dialogs, dir) = gene_harness("omics-overlay");
    // `omics-type`.
    choose(&mut h, "Omics overlay data type", "Proteomics");
    assert_eq!(h.state().store.omics_settings.data_type, "protein");
    // `import-omics`.
    let csv = dir.join("overlay.csv");
    std::fs::write(&csv, "id,ctrl,heat\nA_g1,1,4\nA_g2,2,1\nB_g1,3,3\nB_g2,5,0.5\n").unwrap();
    dialogs.with(|d| d.opens.push_back(vec![csv]));
    common::click(&mut h, "Load CSV");
    assert!(h.state().store.omics_dataset.is_some(), "{}", status(&h));
    assert_eq!(h.state().store.omics_settings.mode, "compare", "two conditions open compared");
    // `omics-pseudocount`, `omics-metric`, `omics-condition-b`, `omics-mode`.
    common::type_into(&mut h, "Omics overlay pseudocount", "0.5");
    assert_eq!(h.state().store.omics_settings.pseudocount, 0.5);
    choose(&mut h, "Omics overlay comparison metric", "B − A");
    assert_eq!(h.state().store.omics_settings.metric, "difference");
    choose(&mut h, "Omics overlay condition B", "ctrl");
    assert_eq!(h.state().store.omics_settings.condition_b.as_deref(), Some("ctrl"));
    choose(&mut h, "Omics overlay view", "One condition");
    assert_eq!(h.state().store.omics_settings.mode, "single");
    // `omics-condition-a`.
    choose(&mut h, "Omics overlay condition A", "heat");
    assert_eq!(h.state().store.omics_settings.condition_a.as_deref(), Some("heat"));
    // `omics-enabled`.
    let enabled = h.state().store.omics_settings.enabled;
    common::click(&mut h, "Show overlay");
    assert_eq!(h.state().store.omics_settings.enabled, !enabled);
    // `clear-omics`.
    common::click(&mut h, "Remove");
    assert!(h.state().store.omics_dataset.is_none());
    assert!(common::has(&h, "Load CSV"));
}

#[test]
fn metatranscriptomics_maps_compares_and_drives_gimme() {
    let (mut h, dialogs, dir) = gene_harness("metatranscriptomics");
    // `import-metatranscriptomics`.
    let csv = dir.join("meta.csv");
    let mut text = String::from("species,gene,condition,expression\n");
    for (sp, g, c, v) in [("A", "g1", "c1", 10), ("A", "g2", "c1", 1), ("B", "g1", "c1", 8), ("B", "g2", "c1", 2), ("A", "g1", "c2", 3), ("A", "g2", "c2", 9), ("B", "g1", "c2", 4), ("B", "g2", "c2", 6)] {
        text.push_str(&format!("{sp},{sp}_{g},{c},{v}\n"));
    }
    std::fs::write(&csv, text).unwrap();
    dialogs.with(|d| d.opens.push_back(vec![csv]));
    common::click(&mut h, "Import CSV/TSV");
    assert!(h.state().store.omics.dataset.is_some(), "{}", status(&h));
    assert!(common::shows(&h, "Genes: 4/4"), "the mapping report");
    // `omics-toggle`.
    let enabled = h.state().store.omics.enabled;
    common::click(&mut h, if enabled { "Hide" } else { "Show" });
    assert_eq!(h.state().store.omics.enabled, !enabled);
    // `omics-compare`, `community-omics-condition-b`, `-metric`, `-pseudocount`, `omics-single`.
    common::click(&mut h, "A vs B");
    assert_eq!(h.state().store.omics.mode, "compare");
    choose(&mut h, "Community expression condition B", "c2");
    assert_eq!(h.state().store.omics.condition_b.as_deref(), Some("c2"));
    choose(&mut h, "Community expression comparison", "B − A");
    assert_eq!(h.state().store.omics.metric, "difference");
    common::type_into(&mut h, "Community expression pseudocount", "2");
    assert_eq!(h.state().store.omics.pseudocount, 2.0);
    common::click(&mut h, "Condition");
    assert_eq!(h.state().store.omics.mode, "single");
    // `community-omics-condition-a`.
    choose(&mut h, "Community expression condition A", "c2");
    assert_eq!(h.state().store.omics.condition_a.as_deref(), Some("c2"));
    // `gimme-objective-fraction`, `gimme-threshold`.
    common::type_into(&mut h, "GIMME objective fraction", "80");
    assert_eq!(h.state().store.omics.gimme_objective_fraction, 0.8);
    common::type_into(&mut h, "GIMME threshold", "5");
    assert_eq!(h.state().store.omics.gimme_threshold, Some(5.0));
    // `run-gimme` (a background job), then `disable-gimme`.
    common::click(&mut h, "Run GIMME · c2");
    assert!(h.state().store.omics.gimme_result.is_some(), "{}", status(&h));
    assert!(h.state().store.omics.gimme_enabled, "{}", status(&h));
    assert!(common::shows(&h, "Threshold 5.000 · inconsistency"));
    common::click(&mut h, "Return FBA");
    assert!(!h.state().store.omics.gimme_enabled);
}
