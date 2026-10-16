//! Port of `test/zuniga-store.test.mjs`: the Zuñiga published case in the
//! store, on synthetic small LPs (integration only; they are not the
//! published models).
//!
//! Not ported: "the case controls expose honest source constraints, growth,
//! comparison and editable map tools" reads the HTML of `panels.js`
//! (`renderWorkspace`, `renderInspector`), which belongs to the GUI.
//!
//! The JS fetches the map through the single-file build's embedded table
//! (`globalThis.__ESCHER_MCMC_DATA__`); here the store reads it from
//! in-memory assets standing for that table. The late download of the last
//! test is a loader that waits on a channel, standing for the promise the JS
//! test resolves by hand.

mod common;

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use coxeter_analysis::zuniga::{ZunigaModels, ZunigaSourceModels};
use coxeter_analysis::{ZunigaStudy, create_zuniga_scenario};
use coxeter_core::model::Model;
use coxeter_map::editable_map::EditableMap;
use coxeter_map::layout::LayoutOptions;
use coxeter_map::place_reaction::PlaceOptions;
use coxeter_state::zuniga::{ZUNIGA_ID, ZunigaLoader, zuniga_community_model};
use coxeter_state::{Fetcher, JobKind, MemoryAssets, Store, StoreError};
use indexmap::IndexMap;
use serde_json::{Value, json};

fn rxn(id: &str, metabolites: Value, extra: Value) -> Value {
    let mut r = json!({ "id": id, "metabolites": metabolites, "lower_bound": 0, "upper_bound": 100 });
    if let (Some(o), Value::Object(e)) = (r.as_object_mut(), extra) {
        o.extend(e);
    }
    r
}

fn metabolite(id: &str) -> Value {
    json!({ "id": id, "name": id, "compartment": if id.ends_with("[smp]") { "s" } else { "c" } })
}

/// Synthetic small LPs test integration only. They are not the published
/// models.
fn fixture_study() -> ZunigaStudy {
    let bio = "BIOMASS_Ec_iJO1366_core_53p95M";
    let mono = json!({
        "id": "synthetic-integration-fixture-Ec", "compartments": { "c": "Cytosol" },
        "metabolites": (["glc_c", "phe__L_c"].map(metabolite)),
        "reactions": [
            rxn("SOURCE", json!({ "glc_c": 1 }), json!({ "upper_bound": 10 })),
            rxn("PPNDH", json!({ "glc_c": -1, "phe__L_c": 1 }), json!({ "gene_reaction_rule": "b2599" })),
            rxn(bio, json!({ "glc_c": -1, "phe__L_c": -1 }), json!({ "objective_coefficient": 1 })),
        ],
        "cell": { "name": "E. coli fixture", "organism": "E. coli" },
    });
    let se = json!({
        "id": "synthetic-integration-fixture-Se", "compartments": { "c": "Cytosol" },
        "metabolites": (["glc_c", "phe__L_c"].map(metabolite)),
        "reactions": [
            rxn("SOURCE", json!({ "glc_c": 1 }), json!({})),
            rxn("MAKEPHE", json!({ "glc_c": -1, "phe__L_c": 1 }), json!({})),
            rxn("BOF", json!({ "glc_c": -1 }), json!({ "objective_coefficient": 1 })),
        ],
        "cell": { "name": "S. elongatus fixture", "organism": "S. elongatus" },
    });
    let joint = json!({
        "id": "synthetic-integration-fixture-community", "compartments": { "c": "Cytosol", "s": "Shared medium" },
        "metabolites": (["glc_c_D", "phe__L_c_D", "glc_c_A", "phe__L_c_A", "phe__L[smp]"].map(metabolite)),
        "reactions": [
            rxn("SOURCE_D", json!({ "glc_c_D": 1 }), json!({ "upper_bound": 10 })),
            rxn("SOURCE_A", json!({ "glc_c_A": 1 }), json!({ "upper_bound": 10 })),
            rxn("PPNDH_D", json!({ "glc_c_D": -1, "phe__L_c_D": 1 }), json!({ "gene_reaction_rule": "b2599" })),
            rxn("MAKEPHE_A", json!({ "glc_c_A": -1, "phe__L_c_A": 1 }), json!({})),
            rxn("PHELtex_A", json!({ "phe__L_c_A": 1, "phe__L[smp]": -1 }), json!({ "lower_bound": -100 })),
            rxn("PHEtex_D", json!({ "phe__L[smp]": -1, "phe__L_c_D": 1 }), json!({})),
            rxn(&format!("{bio}_D"), json!({ "glc_c_D": -1, "phe__L_c_D": -1 }), json!({ "objective_coefficient": 1 })),
            rxn("BOF_A", json!({ "glc_c_A": -1 }), json!({ "objective_coefficient": 0.5 })),
        ],
    });
    let models = ZunigaModels {
        ec: Arc::new(Model::from_value(&mono).unwrap()),
        se: Arc::new(Model::from_value(&se).unwrap()),
    };
    let mut source_hashes = IndexMap::new();
    source_hashes.insert("fixture".to_string(), "synthetic-only".to_string());
    ZunigaStudy {
        manifest: json!({ "community": ZUNIGA_ID, "runtime": "zuniga-mat", "map": "map.json", "fluxReference": 10, "members": [{ "id": "Ec" }, { "id": "Se" }] }),
        source_hashes,
        source_models: ZunigaSourceModels { mono, joint },
        models,
    }
}

/// The assets of the single-file build, holding the case's drawing: two
/// reactions over the coculture community.
fn fixture_assets() -> Arc<MemoryAssets> {
    let study = fixture_study();
    let context = zuniga_community_model(&create_zuniga_scenario(&study, "mutant-coculture").unwrap().community);
    let mut map = EditableMap::empty_from_community(
        &context,
        &LayoutOptions { width: Some(1800.0), height: Some(1200.0), ..LayoutOptions::default() },
    );
    map.add_reaction(&context, "Ec@PPNDH", &PlaceOptions::default());
    map.add_reaction(&context, "Se@PHELtex", &PlaceOptions::default());
    let assets = MemoryAssets::embedded();
    assets.insert_json(format!("models/published/{ZUNIGA_ID}/map.json"), &map.to_json());
    Arc::new(assets)
}

fn published_record() -> Value {
    json!({
        "id": ZUNIGA_ID, "name": "Synthetic integration fixture",
        "species": [{ "id": "Ec", "name": "Ec", "organism": "E. coli" }, { "id": "Se", "name": "Se", "organism": "S. elongatus" }],
        "paper": { "title": "Fixture", "year": 2020, "authors": "Fixture", "journal": "Fixture", "doi": "10.0000/fixture", "framework": "FBA" },
        "reproduction": { "status": "reproduced", "needs": [] },
    })
}

fn fixture_store() -> Store {
    let mut store = Store::new(fixture_assets());
    store.published_communities = vec![published_record()];
    store.use_zuniga_study(Some(fixture_study()), None).expect("the fixture case opens");
    store
}

fn growth_of(store: &Store, scenario: &str, species: &str) -> f64 {
    store.zuniga.solutions[scenario].zuniga.as_ref().expect("a case record").growth[species]
}

fn assert_refused(result: Result<impl std::fmt::Debug, StoreError>) {
    let error = result.expect_err("refused");
    assert!(error.to_string().contains("authors’ objective"), "{error}");
}

#[test]
fn the_case_solves_complete_source_lps_and_excludes_se_from_monoculture_growth() {
    let mut store = fixture_store();
    assert_eq!(store.models.get(&format!("published/{ZUNIGA_ID}/model-Ec")).unwrap().reactions.len(), 3);
    assert_eq!(store.problem.as_ref().unwrap().rxn_ids.len(), 8);
    assert_eq!(store.map.as_ref().unwrap().reaction_ids().len(), 2, "the visual excerpt is independent of the LP");
    assert_eq!(growth_of(&store, "wt-monoculture", "Ec"), 5.0);
    assert_eq!(growth_of(&store, "mutant-monoculture", "Ec"), 0.0);
    assert!(store.growth["Ec"] > 0.0);
    assert!(store
        .cross_feeding
        .links
        .iter()
        .any(|l| l.base == "phe__L" && l.from.as_deref() == Some("Se") && l.to.as_deref() == Some("Ec") && l.flux > 0.0));
    store.use_zuniga_scenario("mutant-monoculture", true).unwrap();
    assert_eq!(store.growth.keys().cloned().collect::<Vec<_>>(), ["Ec"]);
    assert_eq!(store.current_reaction_bounds("Ec@PPNDH").upper, 0.0);
}

#[test]
fn switching_scenarios_preserves_edited_geometry_history_while_changing_the_exact_model_and_knockout() {
    let mut store = fixture_store();
    let node = store.map.as_ref().unwrap().graph().nodes.values().find(|n| n.cell_id.as_deref() == Some("Ec")).cloned().unwrap();
    let edit = store.begin_map_edit("Move fixture node");
    store.move_node(&node.id, node.x + 15.0, node.y + 10.0);
    store.commit_map_edit(edit);
    let drawing = store.map.as_ref().unwrap().to_json();
    // `assert.equal(store.mapHistory, history)`: the same history, which the
    // Rust store owns by value; its entries are compared instead.
    let history = (store.map_history.undo_labels().into_iter().map(str::to_string).collect::<Vec<_>>(), store.map_history.redo_len());
    store.use_zuniga_scenario("wt-monoculture", true).unwrap();
    assert!(store.current_reaction_bounds("Ec@PPNDH").upper > 0.0);
    store.use_zuniga_scenario("mutant-coculture", true).unwrap();
    assert_eq!(store.map.as_ref().unwrap().to_json(), drawing);
    assert_eq!(
        (store.map_history.undo_labels().into_iter().map(str::to_string).collect::<Vec<_>>(), store.map_history.redo_len()),
        history
    );
    assert_eq!(store.problem.as_ref().unwrap().rxn_ids.len(), 8);
}

#[test]
fn comparison_uses_a_fixed_scale_and_leaves_actual_fluxes_bounds_and_growth_unchanged() {
    let mut store = fixture_store();
    let solution = store.solution.clone().unwrap();
    let fluxes = solution.fluxes.clone();
    let bounds = store.problem.as_ref().unwrap().snapshot_bounds();
    let growth = store.growth.clone();
    let baseline = store.zuniga.solutions["wt-monoculture"].fluxes.clone();
    store.set_zuniga_comparison("difference").unwrap();
    let shown = store.display_fluxes()["Ec@PPNDH"];
    assert_eq!(shown, fluxes["Ec@PPNDH"] - baseline.get("Ec@PPNDH").copied().unwrap_or(0.0));
    assert_eq!(store.scale.as_ref().unwrap().reference, 10.0);
    assert!(Arc::ptr_eq(store.solution.as_ref().unwrap(), &solution));
    assert_eq!(store.solution.as_ref().unwrap().fluxes, fluxes);
    assert_eq!(store.problem.as_ref().unwrap().snapshot_bounds(), bounds);
    assert_eq!(store.growth, growth);
    store.use_zuniga_scenario("mutant-monoculture", true).unwrap();
    assert_eq!(store.scale.as_ref().unwrap().reference, 10.0);
    assert_eq!(store.zuniga.comparison, "difference");
}

#[test]
fn generic_biological_edits_cannot_silently_replace_the_original_formulation() {
    let mut store = fixture_store();
    let problem = store.problem.clone().unwrap();
    let snapshot = problem.snapshot_bounds();
    assert_refused(store.set_medium_limit("glc", 2.0));
    assert_refused(store.set_reaction_bounds("Ec@PPNDH", 0.0, 20.0));
    assert_refused(store.set_parsimonious(true));
    assert_refused(store.set_abundance("Ec", 0.1));
    assert_refused(store.set_objective("equal-growth", None));
    assert_refused(store.set_species_enabled("Se", false));
    assert_refused(store.create_model_metabolite(&json!({ "id": "new_c", "compartment": "c" })));
    assert!(Arc::ptr_eq(store.problem.as_ref().unwrap(), &problem));
    assert_eq!(store.problem.as_ref().unwrap().snapshot_bounds(), snapshot);
}

#[test]
fn map_and_scenario_json_restore_the_selected_case_and_comparison_without_removing_the_ghost_cell() {
    let mut store = fixture_store();
    store.use_zuniga_scenario("mutant-monoculture", true).unwrap();
    store.set_zuniga_comparison("difference").unwrap();
    let document = store.map_json().unwrap();
    let drawing = store.map.as_ref().unwrap().to_json();
    let state = store.state_json();
    store.use_zuniga_scenario("wt-monoculture", true).unwrap();
    store.load_map_json(&document).unwrap();
    assert_eq!(store.zuniga.scenario_id.as_deref(), Some("mutant-monoculture"));
    assert_eq!(store.zuniga.comparison, "difference");
    assert_eq!(store.map.as_ref().unwrap().to_json(), drawing);
    assert_eq!(store.map.as_ref().unwrap().cells().len(), 2);
    store.use_zuniga_scenario("mutant-coculture", true).unwrap();
    store.load_state(&state).unwrap();
    assert_eq!(store.zuniga.scenario_id.as_deref(), Some("mutant-monoculture"));
    assert_eq!(store.zuniga.comparison, "difference");
    assert_eq!(store.map.as_ref().unwrap().to_json(), drawing);
}

#[test]
fn a_private_bundle_replays_the_complete_source_models_without_the_generic_community_merger() {
    let mut store = fixture_store();
    store.use_zuniga_scenario("mutant-monoculture", true).unwrap();
    store.set_zuniga_comparison("difference").unwrap();
    let bundle = store.reproducibility_bundle().unwrap();
    assert_eq!(bundle["study"]["sourceModels"]["joint"]["reactions"].as_array().unwrap().len(), 8);
    assert_eq!(bundle["study"]["activeModel"]["reactions"].as_array().unwrap().len(), 3);
    // `new Store('.')`: the embedded table is global in JS, so the new store
    // sees the same drawing.
    let mut restored = Store::new(fixture_assets());
    let result = restored.import_reproducibility_bundle(&bundle).unwrap();
    assert_eq!(result["valid"], true);
    assert_eq!(result["replay"]["pass"], true, "{}", result["replay"]);
    assert_eq!(restored.zuniga.scenario_id.as_deref(), Some("mutant-monoculture"));
    assert_eq!(restored.map.as_ref().unwrap().to_json(), store.map.as_ref().unwrap().to_json());
    assert_eq!(restored.map.as_ref().unwrap().cells().len(), 2);
    restored.use_zuniga_scenario("mutant-coculture", true).unwrap();
    assert!(restored.growth["Ec"] > 0.0);
}

#[test]
fn invalid_saved_case_settings_are_rejected_before_replacing_the_active_scenario() {
    let mut store = fixture_store();
    let solution = store.solution.clone().unwrap();
    let drawing = store.map.as_ref().unwrap().to_json();
    let mut document = store.map_document().unwrap();
    document["study"]["scenarioId"] = json!("invented-case");
    let error = store.load_map_json(&document.to_string()).unwrap_err();
    assert!(error.to_string().contains("scenario is invalid"), "{error}");
    assert!(Arc::ptr_eq(store.solution.as_ref().unwrap(), &solution));
    assert_eq!(store.map.as_ref().unwrap().to_json(), drawing);
    let mut state: Value = serde_json::from_str(&store.state_json()).unwrap();
    state["study"]["fluxReference"] = json!(999);
    let error = store.load_state(&state.to_string()).unwrap_err();
    assert!(error.to_string().contains("different flux scale"), "{error}");
    assert!(Arc::ptr_eq(store.solution.as_ref().unwrap(), &solution));
}

/// A download that waits until the test hands it the study.
struct LateDownload {
    called: Mutex<Sender<()>>,
    release: Mutex<Receiver<ZunigaStudy>>,
}

impl ZunigaLoader for LateDownload {
    fn load(&self, _manifest: &Value, _fetcher: &dyn Fetcher, on_progress: &mut dyn FnMut(&str)) -> coxeter_state::Result<ZunigaStudy> {
        let _ = self.called.lock().unwrap().send(());
        let study = self.release.lock().unwrap().recv().map_err(|_| StoreError::msg("released without a study"))?;
        on_progress("Stale source download status");
        Ok(study)
    }
}

#[test]
fn another_scenario_supersedes_a_late_model_download_without_installing_its_stale_biology() {
    let mut store = fixture_store();
    let study = fixture_study();
    let (called_tx, called_rx) = channel();
    let (release_tx, release_rx) = channel();
    store.zuniga_loader = Arc::new(LateDownload { called: Mutex::new(called_tx), release: Mutex::new(release_rx) });
    store.use_zuniga_study(None, None).unwrap();
    called_rx.recv_timeout(Duration::from_secs(30)).expect("the download started");
    store.models.insert_arc("ordinary-fixture", study.models.ec.clone());
    store.scenarios = vec![json!({ "id": "ordinary", "name": "Ordinary fixture", "species": [{ "id": "Ec", "model": "ordinary-fixture" }], "medium": {} })];
    store.use_scenario("ordinary").unwrap();
    let ordinary_problem = store.problem.clone().unwrap();
    let status = store.status().clone();
    release_tx.send(study).unwrap();
    let outcomes = store.wait_for_kind(JobKind::Zuniga, Duration::from_secs(60));
    let outcome = outcomes.iter().find(|o| o.kind == JobKind::Zuniga).expect("the download finished");
    assert!(matches!(&outcome.result, Err(e) if e.is_abort()), "{:?}", outcome.result);
    assert!(Arc::ptr_eq(store.problem.as_ref().unwrap(), &ordinary_problem));
    assert_eq!(store.status(), &status);
    assert_eq!(store.scenario_id().as_deref(), Some("ordinary"));
    assert!(!store.is_zuniga_study());
    assert_eq!(store.zuniga.comparison, "absolute");
}

#[test]
fn a_re_solve_leaves_the_header_to_its_summary_and_records_itself_in_the_log() {
    let mut store = fixture_store();
    store.log.clear();
    store.solve();
    store.solve();
    assert_eq!(store.status().state.as_str(), "ok");
    assert_eq!(store.status().message, "", "a notice on every re-solve hid the objective for six seconds");
    assert_eq!(store.log.len(), 1);
    assert!(store.log[0].message.contains("exact source constraints solved: optimal"), "{}", store.log[0].message);
    assert_eq!(store.log[0].count, 2);
}

#[test]
fn opening_the_case_reads_saved() {
    let store = fixture_store();
    let unsaved = store.unsaved_work();
    assert!(!unsaved.map && !unsaved.scenario && !unsaved.model, "{unsaved:?}");
}

/// Extra, not in the JavaScript file: without a study and without a
/// manifest the case says so and leaves nothing half-open.
#[test]
fn a_case_without_its_manifest_fails_cleanly() {
    let mut store = Store::new(Arc::new(MemoryAssets::embedded()));
    let error = store.use_zuniga_study(None, None).unwrap_err();
    assert_eq!(error.to_string(), "The Zuñiga case manifest is unavailable.");
    assert!(!store.is_zuniga_study());
    assert!(store.map.is_none() && store.problem.is_none());
    assert_eq!(store.status().state.as_str(), "error");
}
