//! The File menu: every import and export (the file inputs of `index.html`
//! and the `download()` calls of `src/ui/app.js`), through the menu, with
//! the dialogs answered from a script.

mod common;

use std::path::PathBuf;

use coxeter_app::files::{FileKind, classify};
use coxeter_app::ui_state::AnalysisTab;

/// Open the File menu and pick an item.
fn file_menu(h: &mut egui_kittest::Harness<'static, coxeter_app::Shell>, item: &str) {
    common::press(h, "File menu");
    common::press(h, item);
}

fn queue_open(dialogs: &coxeter_app::files::SharedDialogs, paths: Vec<PathBuf>) {
    dialogs.with(|d| d.opens.push_back(paths));
}

#[test]
fn exports_write_their_files_under_the_names_the_js_gives_them() {
    let (mut h, dialogs, dir) = common::harness_saving("exports", None);
    file_menu(&mut h, "Export SVG map");
    file_menu(&mut h, "Export PNG map");
    file_menu(&mut h, "Export flux table (CSV)");
    file_menu(&mut h, "Export scenario");
    file_menu(&mut h, "Export map JSON");
    file_menu(&mut h, "Export model JSON");
    file_menu(&mut h, "Export reproducibility bundle");
    file_menu(&mut h, "Export methods / provenance report");
    let saved = common::saved(&dialogs);
    let names: Vec<&str> = saved.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(&names[..6], &[
        "consorcio-anaerobio_map.svg",
        "consorcio-anaerobio_map.png",
        "consorcio-anaerobio_fluxes.csv",
        "consorcio-anaerobio_scenario.json",
        "consorcio-anaerobio_map.json",
        "lactobacillus_core_mc_model.json",
    ]);
    assert!(names[6].starts_with("consorcio-anaerobio_repro_") && names[6].ends_with(".json"), "{}", names[6]);
    assert!(names[7].starts_with("consorcio-anaerobio_methods_") && names[7].ends_with(".md"), "{}", names[7]);
    let text = |i: usize| String::from_utf8_lossy(&saved[i].1).into_owned();
    assert!(text(0).starts_with("<svg"), "an SVG");
    assert_eq!(&saved[1].1[..8], b"\x89PNG\r\n\x1a\n", "a PNG");
    assert!(text(2).starts_with("reaction,species,local_id,name,subsystem,flux,lower_bound,upper_bound"));
    assert_eq!(classify(&dir.join(&saved[3].0), &text(3)), Some(FileKind::Scenario));
    assert_eq!(classify(&dir.join(&saved[4].0), &text(4)), Some(FileKind::Map));
    assert_eq!(classify(&dir.join(&saved[5].0), &text(5)), Some(FileKind::Model));
    assert_eq!(classify(&dir.join(&saved[6].0), &text(6)), Some(FileKind::Bundle));
    assert!(text(7).contains("#"), "a Markdown report");
    // What was written is what is now saved: the badge says Saved.
    let work = h.state().store.unsaved_work();
    assert!(!work.map && !work.scenario, "map-save and export-state mark their part saved");
    let log: Vec<String> = h.state().store.log.iter().map(|e| e.message.clone()).collect();
    assert!(log.iter().any(|m| m == "Map saved as consorcio-anaerobio_map.json"), "{log:?}");
    assert!(log.iter().any(|m| m == "Scenario saved as consorcio-anaerobio_scenario.json"));
    assert!(log.iter().any(|m| m == "Flux table exported as consorcio-anaerobio_fluxes.csv"));
    assert!(log.iter().any(|m| m.starts_with("Map exported as consorcio-anaerobio_map.png (")));
    assert!(log.iter().any(|m| m == "Model lla saved as lactobacillus_core_mc_model.json"));
    assert!(h.state().store.status().message.starts_with("Methods / Provenance report exported"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_cancelled_save_writes_nothing_and_marks_nothing_saved() {
    // `common::harness` answers every save dialog with Cancel.
    let mut h = common::harness(None);
    file_menu(&mut h, "Export map JSON");
    file_menu(&mut h, "Export scenario");
    let log: Vec<String> = h.state().store.log.iter().map(|e| e.message.clone()).collect();
    assert!(!log.iter().any(|m| m.contains("saved as")), "{log:?}");
}

#[test]
fn saved_files_open_again_through_the_file_menu() {
    let (mut h, dialogs, dir) = common::harness_saving("roundtrip", None);
    file_menu(&mut h, "Export scenario");
    file_menu(&mut h, "Export map JSON");
    file_menu(&mut h, "Export reproducibility bundle");
    let saved: Vec<PathBuf> = dialogs.with(|d| d.saved.clone());

    // import-state
    h.state_mut().store.use_scenario("solo-ecoli").unwrap();
    common::settle(&mut h);
    queue_open(&dialogs, vec![saved[0].clone()]);
    file_menu(&mut h, "Import scenario");
    assert_eq!(h.state().store.scenario_id().as_deref(), Some("consorcio-anaerobio"));
    assert!(h.state().store.log.iter().any(|e| e.message == "Scenario loaded from consorcio-anaerobio_scenario.json"));

    // map-load
    queue_open(&dialogs, vec![saved[1].clone()]);
    file_menu(&mut h, "Import map JSON");
    assert!(h.state().store.status().message.starts_with("Map loaded: "), "{}", h.state().store.status().message);

    // import-reproducibility
    queue_open(&dialogs, vec![saved[2].clone()]);
    file_menu(&mut h, "Import reproducibility bundle");
    assert!(h.state().store.reproducibility_replay.is_some());
    assert!(h.state().store.status().message.starts_with("Bundle restored"), "{}", h.state().store.status().message);
    common::click(&mut h, "Analysis results");
    assert!(!common::disabled(&h, "Reproducibility"), "the replay fills its tab");
    common::click(&mut h, "Reproducibility");
    assert_eq!(h.state().ui.analysis_tab, AnalysisTab::Repro);
    assert!(common::shows(&h, "Reproducibility verified") || common::shows(&h, "Reproducibility warning"));
    assert!(common::shows(&h, "Bundle integrity"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn models_open_from_the_file_menu() {
    let (mut h, dialogs, dir) = common::harness_saving("models", None);
    queue_open(&dialogs, vec![common::repo_root().join("models").join("ecoli_core_mc.json")]);
    file_menu(&mut h, "Import model (JSON / SBML)");
    let store = &h.state().store;
    assert_eq!(store.species.len(), 1, "one file, one species");
    assert!(store.community.is_some());
    assert_eq!(store.uploaded_model_keys.len(), 1);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn metatranscriptomics_opens_from_the_file_menu() {
    let (mut h, dialogs, dir) = common::harness_saving("metatx", None);
    let csv = dir.join("metatx.csv");
    std::fs::write(&csv, "MAG,gene,condition,TPM\neco,b0001,Control,10\neco,b0001,Treatment,40\nlla,g2,Control,5\n").unwrap();
    queue_open(&dialogs, vec![csv]);
    file_menu(&mut h, "Import metatranscriptomics (CSV / TSV)");
    let omics = &h.state().store.omics;
    assert!(omics.dataset.is_some(), "{}", h.state().store.status().message);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_file_that_cannot_be_read_says_so() {
    let (mut h, dialogs, dir) = common::harness_saving("bad", None);
    let bad = dir.join("bad.json");
    std::fs::write(&bad, "{ not json").unwrap();
    queue_open(&dialogs, vec![bad.clone()]);
    file_menu(&mut h, "Import scenario");
    assert!(h.state().store.status().message.starts_with("Could not load: "), "{}", h.state().store.status().message);
    queue_open(&dialogs, vec![bad.clone()]);
    file_menu(&mut h, "Import map JSON");
    assert!(h.state().store.status().message.starts_with("Could not load the map: "));
    queue_open(&dialogs, vec![bad]);
    file_menu(&mut h, "Import reproducibility bundle");
    assert!(h.state().store.status().message.starts_with("Could not replay the bundle: "));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn files_on_the_command_line_open_as_what_they_are() {
    let (mut h, dialogs, dir) = common::harness_saving("cli", None);
    file_menu(&mut h, "Export map JSON");
    let map = dialogs.with(|d| d.saved[0].clone());
    let model = common::repo_root().join("models").join("yeast_core_mc.json");
    h.state_mut().open_paths(&[model]);
    common::settle(&mut h);
    assert_eq!(h.state().store.species.len(), 1);
    h.state_mut().store.use_scenario("consorcio-anaerobio").unwrap();
    h.state_mut().open_paths(&[map]);
    common::settle(&mut h);
    assert!(h.state().store.status().message.starts_with("Map loaded: "), "{}", h.state().store.status().message);
    let _ = std::fs::remove_dir_all(dir);
}

/// waiting-map: a map whose model is not open is kept, the status line says
/// what it waits for, and File ▸ "Forget waiting map" discards it
/// (`discard-waiting-map`).
#[test]
fn a_map_for_a_model_that_is_not_open_waits_and_can_be_forgotten() {
    let (mut h, dialogs, dir) = common::harness_saving("waiting", None);
    let mut doc: serde_json::Value = serde_json::from_str(&h.state().store.map_json().unwrap()).unwrap();
    doc["metadata"]["scenario"] = serde_json::json!("a-scenario-that-went-away");
    // Drawn over reactions the open model does not have.
    for rxn in doc["map"]["reactions"].as_array_mut().unwrap() {
        let id = rxn["id"].as_str().unwrap().to_string();
        rxn["id"] = serde_json::json!(format!("X_{id}"));
    }
    let path = dir.join("old_map.json");
    std::fs::write(&path, doc.to_string()).unwrap();
    let before = h.state().store.map.as_ref().unwrap().to_json();

    queue_open(&dialogs, vec![path]);
    file_menu(&mut h, "Import map JSON");
    let store = &h.state().store;
    assert!(store.waiting_map.is_some(), "kept, not refused");
    assert_eq!(store.status().state, coxeter_state::StatusState::Warn);
    let message = store.status().message.clone();
    assert!(message.starts_with("“old_map.json” does not fit what is open: "), "{message}");
    assert!(message.contains("a-scenario-that-went-away") && message.contains("The current drawing is unchanged."), "{message}");
    assert_eq!(store.map.as_ref().unwrap().to_json(), before, "the drawing is untouched");

    common::press(&mut h, "File menu");
    common::press(&mut h, "Forget waiting map (old_map.json)");
    assert!(h.state().store.waiting_map.is_none());
    assert_eq!(h.state().store.status().message, "The waiting map was forgotten.");
    common::press(&mut h, "File menu");
    assert!(!common::shows(&h, "Forget waiting map"), "offered only while a map waits");
    let _ = std::fs::remove_dir_all(dir);
}

/// line-styles: an exported SVG carries its title, what drew it, and hover
/// text on each reaction and metabolite.
#[test]
fn an_exported_svg_says_what_it_is_and_what_each_reaction_is() {
    let (mut h, dialogs, dir) = common::harness_saving("svg-about", None);
    file_menu(&mut h, "Export SVG map");
    let saved = common::saved(&dialogs);
    let svg = String::from_utf8_lossy(&saved[0].1).into_owned();
    let name = h.state().store.scenario_str("name").unwrap();
    assert!(svg.contains(&format!("<title>{name}</title><metadata>")), "the title first, then what drew it");
    assert!(svg.contains("Coxeter Multicell Flux Visualizer"));
    assert!(svg.contains("Flux: "), "a reaction's hover text");
    assert!(svg.contains("</title></circle>"), "a metabolite's hover text");
    let _ = std::fs::remove_dir_all(dir);
}
