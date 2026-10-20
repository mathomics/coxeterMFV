//! The Scenario & objective page through the real interface: the community
//! list, the objective controls, a published community's card (downloads,
//! recorded results, the sweep figure, provenance), the Zuñiga case and the
//! SteadyCom controls (`scenarioCard` and what it holds in
//! `src/ui/panels.js`, with the `onSidebarClick` / `onSidebarInput` cases of
//! `src/ui/app.js` for its `data-action`s).
//!
//! Ports `test/steadycom-ui.test.mjs` in full. The JS stubs
//! `store.steadyComSupport` and sets `store.steadyComState` by hand; here the
//! store is real: a two-member teaching community whose LPs solve in
//! milliseconds stands under the published entry the JS fixture builds, and
//! the SteadyCom state is set on the store where the JS test sets it.
//! Ports the panel part of `test/zuniga-store.test.mjs` ("the case controls
//! expose honest source constraints…"; its inspector half is the
//! inspector's) on the same synthetic case as
//! `coxeter-state/tests/js_zuniga_store.rs`. `test/published-communities
//! .test.mjs` reads no panel HTML; what its card shows is tested here on the
//! bundled communities.

mod common;

use std::sync::Arc;

use coxeter_analysis::zuniga::{ZunigaModels, ZunigaSourceModels};
use coxeter_analysis::{ZunigaStudy, create_zuniga_scenario};
use coxeter_app::Shell;
use coxeter_app::files::{ScriptedDialogs, SharedDialogs};
use coxeter_app::ui_state::{SteadyComDraft, Workspace};
use coxeter_core::model::Model;
use coxeter_map::editable_map::EditableMap;
use coxeter_map::layout::LayoutOptions;
use coxeter_map::place_reaction::PlaceOptions;
use coxeter_solver::backends::SolverKind;
use coxeter_solver::steadycom::{Fiber, SteadyComProgress};
use coxeter_state::zuniga::{ZUNIGA_ID, zuniga_community_model};
use coxeter_state::{MemoryAssets, NoFetcher, StatusState, SteadyComState, Store};
use egui::accesskit::{Invalid, Role};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use indexmap::IndexMap;
use serde_json::{Value, json};

type H = Harness<'static, Shell>;

const ECOLI: &str = "steadycom-ecoli-auxotrophs";
const GUT9: &str = "steadycom-gut-nine";
const TEACHING: &str = "teaching-mutualism";

// Helpers

fn open(h: &mut H, ws: Workspace) {
    h.state_mut().open_workspace(ws);
    common::settle(h);
}

/// Pick an option of a select that may be below the fold.
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

/// Empty a field and commit it (select all, delete, Enter).
fn clear_field(h: &mut H, label: &str) {
    common::control(h, label).focus();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.key_press(egui::Key::Backspace);
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    common::settle(h);
}

fn value(h: &H, label: &str) -> String {
    common::control(h, label).value().unwrap_or_default()
}

fn invalid(h: &H, label: &str) -> bool {
    common::control(h, label).accesskit_node().data().invalid() == Some(Invalid::True)
}

fn status(h: &H) -> String {
    h.state().store.status().message.clone()
}

/// `member(id, makes, needs)`: an obligate cross-feeder (as in
/// `coxeter-state/tests/js_steadycom_store.rs`).
fn member(id: &str, makes: &str, needs: &str) -> Model {
    let metabolites: Vec<Value> = ["glc", makes, needs]
        .iter()
        .flat_map(|base| ["e", "c"].map(|compartment| json!({"id": format!("{base}_{compartment}"), "compartment": compartment})))
        .collect();
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
            {"id": "GLCt", "metabolites": {"glc_e": -1, "glc_c": 1}, "lower_bound": 0, "upper_bound": 100},
            {"id": "BIOMASS", "metabolites": biomass, "lower_bound": 0, "upper_bound": 100},
            {"id": format!("MK_{makes}"), "metabolites": make, "lower_bound": 0, "upper_bound": 100},
            {"id": format!("T_{makes}"), "metabolites": {format!("{makes}_c"): -1, format!("{makes}_e"): 1}, "lower_bound": 0, "upper_bound": 100},
            {"id": format!("EX_{makes}_e"), "metabolites": {format!("{makes}_e"): -1}, "lower_bound": 0, "upper_bound": 1000},
            {"id": format!("T_{needs}"), "metabolites": {format!("{needs}_e"): -1, format!("{needs}_c"): 1}, "lower_bound": 0, "upper_bound": 100},
            {"id": format!("EX_{needs}_e"), "metabolites": {format!("{needs}_e"): -1}, "lower_bound": -100, "upper_bound": 1000}
        ],
        "objective": {"BIOMASS": 1}
    }))
    .expect("a valid member model")
}

/// The teaching community: two members that feed each other.
fn pair_store() -> Store {
    let mut store = common::store().with_fetcher(Arc::new(NoFetcher));
    store.set_model("member-A", member("A", "x", "y"));
    store.set_model("member-B", member("B", "y", "x"));
    let species = json!([{"id": "A", "model": "member-A", "abundance": 0.5}, {"id": "B", "model": "member-B", "abundance": 0.5}]);
    store.scenarios = vec![json!({
        "id": TEACHING, "name": "Teaching mutualism", "description": "Two-member teaching model.",
        "objective": "equal-growth", "medium": {"glc": 10}, "species": species,
    })];
    store.use_scenario(TEACHING).expect("the teaching community opens");
    store
}

/// `fixture(id, { published })` of `steadycom-ui.test.mjs`: a recorded
/// published entry over the teaching community, or the teaching community.
fn fixture(id: &str, published: bool) -> H {
    let mut store = pair_store();
    if published {
        store.published_communities = vec![json!({
            "id": id, "name": id, "description": "Published benchmark.", "species": [],
            "paper": { "framework": "SteadyCom", "title": "A recorded community", "authors": "Authors", "year": 2026 },
            "reproduction": { "status": "reproduced", "reproduces": [], "adaptations": [], "needs": [] },
            "computed": { "growthRate": 0.73599, "lpCount": 23, "seconds": 42 },
        })];
        store.scenario = Some(json!({ "id": format!("published:{id}"), "published": id, "name": id }));
    }
    let mut h = common::harness_of(Shell::new(store, Box::new(ScriptedDialogs::new())));
    open(&mut h, Workspace::Scenario);
    h
}

/// A SteadyCom state as the JS tests write it.
fn state(community: &str, key: &str, max: Option<f64>, edit: impl FnOnce(&mut SteadyComState)) -> SteadyComState {
    let mut s = SteadyComState {
        serial: 1,
        community_id: Some(community.into()),
        key: key.into(),
        min: Some(0.0),
        max,
        source: "computed".into(),
        phase: None,
        progress: None,
        mu: None,
        feasible: None,
        abundances: IndexMap::new(),
        upper_bound: None,
        resolution: None,
        iterations: None,
        seconds: None,
        error: None,
        parsimonious: false,
        total_biomass: None,
        original_fluxes: None,
        total_flux: None,
        solve_ms: None,
    };
    edit(&mut s);
    s
}

fn set_state(h: &mut H, s: SteadyComState) {
    h.state_mut().store.steady_com_state = Some(s);
    common::settle(h);
}

// The community and objective

#[test]
fn the_community_list_opens_a_teaching_scenario_and_a_published_one() {
    let mut h = common::harness(None);
    common::click(&mut h, "Edit Model");
    common::click(&mut h, "Scenario & objective");
    assert_eq!(h.state().ui.workspace, Some(Workspace::Scenario));
    assert_eq!(value(&h, "Community"), "Anaerobic consortium (4 species)");
    assert!(common::shows(&h, "Glucose feeds the lactic acid bacterium"), "the scenario's description");

    choose(&mut h, "Community", "Aerobic co-culture (E. coli + yeast)");
    assert_eq!(h.state().store.scenario_id().as_deref(), Some("aerobio-mixto"));
    assert_eq!(h.state().ui.scenario_choice, None, "the choice is the open scenario");
    assert_eq!(value(&h, "Community"), "Aerobic co-culture (E. coli + yeast)");

    // A published one is marked by its reproduction status.
    choose(&mut h, "Community", "— Two E. coli ecotypes (glucose and acetate specialists)");
    assert_eq!(h.state().store.scenario_published().as_deref(), Some("cfba-ecoli-ecotypes"));
    assert!(h.state().ui.steady_com_draft.is_none());
}

#[test]
fn a_blocked_community_says_what_it_still_needs_and_offers_no_download() {
    let mut h = common::harness(None);
    open(&mut h, Workspace::Scenario);
    choose(&mut h, "Community", "— Two E. coli ecotypes (glucose and acetate specialists)");
    assert!(common::shows(&h, "BLOCKED"));
    assert!(common::shows(&h, "CFBA"), "the framework chip");
    assert!(common::shows(&h, "VERIFIED"), "a citation checked against a retrieved file says so");
    assert!(common::shows(&h, "Requirements for reproduction (2)"));
    // The two ecotypes share an organism name: the role tells them apart.
    assert!(common::shows(&h, "Escherichia coli (ecotype) · "));
    assert!(!common::has(&h, "Download glu as COBRA JSON"), "no model file is bundled");
    assert!(!common::shows(&h, "RECORDED PUBLISHED RESULTS"), "nothing was computed");
    assert_eq!(h.state().store.status().state, StatusState::Warn, "no model files are bundled");
}

#[test]
fn the_objective_select_sets_the_community_objective() {
    let mut h = common::harness(None);
    open(&mut h, Workspace::Scenario);
    assert_eq!(value(&h, "Community objective"), "Equal growth (shared µ)");
    choose(&mut h, "Community objective", "Weighted biomass sum");
    assert_eq!(h.state().store.objective, "sum-biomass");
}

#[test]
fn pfba_and_its_biomass_target() {
    let mut h = common::harness(None);
    open(&mut h, Workspace::Scenario);
    assert!(common::shows(&h, "Current method: FBA."));
    common::click(&mut h, "Use Parsimonious Flux Balance Analysis");
    assert!(h.state().store.parsimonious);
    assert!(common::shows(&h, "Current method: pFBA. pFBA fixes 100% of the optimum"));
    assert_eq!(value(&h, "Percentage of the optimal objective fixed for pFBA"), "100");
    common::type_into(&mut h, "Percentage of the optimal objective fixed for pFBA", "90.5");
    assert_eq!(h.state().store.pfba_biomass_percent, 90.5, "the debounced setter ran");
    assert!(common::shows(&h, "pFBA fixes 90.5% of the optimum"));
}

#[test]
fn loopless_and_its_progress_line() {
    let mut h = common::harness(None);
    open(&mut h, Workspace::Scenario);
    assert!(common::shows(&h, "Removes internal cycles with a MILP"));
    common::click(&mut h, "Enable Loopless Flux Balance Analysis");
    assert!(h.state().store.loopless);
    assert!(common::shows(&h, "Loopless uses a MILP"));
    // (The loopless MILP of this consortium is infeasible, as in the JS app.)
    assert!(common::shows(&h, "Loopless MILP optimal") || common::shows(&h, "Loopless: "));
}

#[test]
fn fva_and_its_progress_line() {
    let mut h = common::harness(None);
    open(&mut h, Workspace::Scenario);
    assert!(common::shows(&h, "Computes the minimum and maximum flux"));
    common::click(&mut h, "Enable Flux Variability Analysis");
    assert!(h.state().store.fva_enabled);
    assert!(common::shows(&h, "FVA done: "), "FVA ran on the next solve");
    assert!(common::shows(&h, "Current method: FBA + FVA."));
}

// steadycom-ui.test.mjs

#[test]
fn recorded_benchmarks_never_enable_a_growth_rate_solve_before_calculating_the_current_range() {
    let h = fixture(ECOLI, true);
    assert!(common::shows(&h, "Recorded µ (offline): 0.73599"));
    assert!(common::shows(&h, "Not calculated for the current inputs"));
    assert!(common::disabled(&h, "Growth rate"));
    assert!(common::disabled(&h, "Growth rate, h⁻¹"));
    assert!(common::disabled(&h, "Solve at this µ"));
    assert!(!common::disabled(&h, "Calculate range"));
}

#[test]
fn calculated_bounds_and_target_preserve_their_exact_numeric_values() {
    let mut h = fixture(ECOLI, true);
    let max = 0.7359843211234567;
    set_state(
        &mut h,
        state(ECOLI, "inputs-1", Some(max), |s| {
            s.resolution = Some(1e-7);
            s.upper_bound = Some(max + 1e-7);
            s.mu = Some(max);
        }),
    );
    let slider = common::control(&h, "Growth rate");
    assert_eq!(slider.accesskit_node().data().max_numeric_value(), Some(max));
    assert_eq!(slider.accesskit_node().data().numeric_value_step(), None, "step=\"any\"");
    assert_eq!(value(&h, "Growth rate, h⁻¹"), "0.7359843211234567");
    assert!(!common::disabled(&h, "Solve at this µ"));
    assert!(common::shows(&h, "Search resolution: 1e-7 h⁻¹"));
    assert!(common::has(&h, "Recalculate range"));
}

#[test]
fn worker_progress_disables_duplicate_requests_and_exposes_cancellation() {
    let mut h = fixture(ECOLI, true);
    assert!(!common::has(&h, "Cancel"), "Cancel is hidden while nothing runs");
    set_state(
        &mut h,
        state(ECOLI, "inputs-1", None, |s| {
            s.phase = Some("range".into());
            s.progress = Some(SteadyComProgress { iterations: 9, lo: 0.0, hi: 1.0, mu: 0.5 });
        }),
    );
    assert!(common::disabled(&h, "Calculate range"));
    assert!(common::disabled(&h, "Solve at this µ"));
    assert!(common::has(&h, "Cancel"));
    assert!(common::shows(&h, "Calculating the growth-rate range · iteration 9…"));
    // `steadycom-cancel`.
    common::click(&mut h, "Cancel");
    assert_eq!(h.state().store.steady_com_state.as_ref().unwrap().phase, None);
    assert!(!common::has(&h, "Cancel"));
    assert!(!common::disabled(&h, "Calculate range"));
}

#[test]
fn zero_growth_and_inactive_species_are_described_without_claiming_a_unique_composition() {
    let mut h = fixture(ECOLI, true);
    set_state(
        &mut h,
        state(ECOLI, "inputs-1", Some(0.7), |s| {
            s.mu = Some(0.0);
            s.feasible = Some(true);
            s.abundances = IndexMap::from([("A".to_string(), 1.0), ("B".to_string(), 0.0)]);
            s.solve_ms = Some(12.0);
        }),
    );
    assert!(common::shows(&h, "Composition at µ = 0 h⁻¹"));
    assert!(common::shows(&h, "A 100.0 % · B inactive (X = 0)"));
    assert!(common::shows(&h, "(12 ms)"));
    assert!(common::shows(&h, "may be non-unique, even at the maximum"));
    assert!(common::shows(&h, "At µ = 0, present members still obey their maintenance bounds"));
    assert!(!common::shows(&h, "answer unique") && !common::shows(&h, "community is pinned down"));
}

#[test]
fn an_infeasible_growth_rate_is_reported() {
    let mut h = fixture(ECOLI, true);
    set_state(
        &mut h,
        state(ECOLI, "inputs-1", Some(0.7), |s| {
            s.mu = Some(0.5);
            s.feasible = Some(false);
        }),
    );
    assert!(common::shows(&h, "No feasible community at µ = 0.5 h⁻¹."));
}

#[test]
fn nine_species_conditions_distinguish_baseline_from_optional_figure_4_fiber_limits() {
    let mut h = fixture(GUT9, true);
    assert_eq!(value(&h, "SteadyCom fiber condition"), "Baseline diet (no additional fiber limits)");
    assert!(!common::has(&h, "Bacteroides maximum specific fiber uptake"));
    h.state_mut().store.steady_com_config.fiber = Some(Fiber { bacteroides: 12.5, faecalibacterium: 5.0 });
    common::settle(&mut h);
    assert_eq!(value(&h, "SteadyCom fiber condition"), "Figure 4 fiber limits");
    assert_eq!(value(&h, "Bacteroides maximum specific fiber uptake"), "12.5");
    assert!(common::shows(&h, "F. prausnitzii is held at 5"));
    assert!(common::shows(&h, "invalidates the calculated range"));
}

#[test]
fn the_fiber_condition_and_limit_are_set_and_validated() {
    let mut h = fixture(GUT9, true);
    // `steadycom-fiber-condition`.
    choose(&mut h, "SteadyCom fiber condition", "Figure 4 fiber limits");
    assert_eq!(h.state().store.steady_com_config.fiber.map(|f| f.bacteroides), Some(30.0));
    assert!(status(&h).contains("Calculate the permitted growth-rate range again"));
    // `steadycom-fiber`: out of range is refused under the controls.
    let field = "Bacteroides maximum specific fiber uptake";
    common::type_into(&mut h, field, "50");
    assert!(common::shows(&h, "Enter a Bacteroides fiber limit between 1 and 30 C-mmol/gDW/h."));
    assert!(invalid(&h, field));
    assert_eq!(value(&h, field), "50", "the field keeps what was typed");
    assert_eq!(h.state().store.steady_com_config.fiber.map(|f| f.bacteroides), Some(30.0));
    // `validateSteadyComFiber`: Calculate refuses the same value.
    common::click(&mut h, "Calculate range");
    assert!(h.state().store.steady_com_state.is_none(), "no calculation started");
    assert!(common::shows(&h, "Enter a Bacteroides fiber limit between 1 and 30"));
    common::type_into(&mut h, field, "12");
    assert_eq!(h.state().store.steady_com_config.fiber.map(|f| f.bacteroides), Some(12.0));
    assert!(h.state().ui.steady_com_draft.is_none());
    assert!(!invalid(&h, field));
    assert!(!common::shows(&h, "Enter a Bacteroides fiber limit"));
    // Back to the baseline.
    choose(&mut h, "SteadyCom fiber condition", "Baseline diet (no additional fiber limits)");
    assert_eq!(h.state().store.steady_com_config.fiber, None);
}

#[test]
fn supported_teaching_communities_expose_the_same_fixed_growth_workflow() {
    let h = fixture(TEACHING, false);
    assert!(common::shows(&h, "FIX A GROWTH RATE · STEADYCOM"));
    assert!(common::has(&h, "Calculate range"));
    assert!(!common::shows(&h, "RECORDED PUBLISHED RESULTS"));
    assert!(common::shows(&h, "Two-member teaching model."));
}

#[test]
fn growth_rate_drafts_survive_rerenders_only_for_the_same_input_key() {
    let mut h = fixture(ECOLI, true);
    h.state_mut().store.steady_com_state = Some(state(ECOLI, "inputs-1", Some(0.7), |s| s.mu = Some(0.3)));
    h.state_mut().ui.steady_com_draft = Some(SteadyComDraft {
        community_id: Some(ECOLI.into()),
        key: Some("inputs-1".into()),
        value: String::new(),
        error: "Enter a finite rate.".into(),
        invalid_field: Some("mu".into()),
        fiber_value: None,
    });
    common::settle(&mut h);
    assert_eq!(value(&h, "Growth rate, h⁻¹"), "");
    assert!(invalid(&h, "Growth rate, h⁻¹"));
    assert!(common::shows(&h, "Enter a finite rate."));
    h.state_mut().store.steady_com_state.as_mut().unwrap().key = "inputs-2".into();
    common::settle(&mut h);
    assert_eq!(value(&h, "Growth rate, h⁻¹"), "0.3");
    assert!(!invalid(&h, "Growth rate, h⁻¹"));
    assert!(!common::shows(&h, "Enter a finite rate"));
}

#[test]
fn fixed_growth_results_take_precedence_over_ordinary_mode_flags_and_disable_incompatible_fva() {
    let mut h = fixture(TEACHING, false);
    // `steadycom-recompute`: the range of the current inputs, on HiGHS.
    common::click(&mut h, "Calculate range");
    assert!(!h.state().ui.steady_com_pending, "the shell clears the request when the job ends");
    let max = h.state().store.steady_com_state.as_ref().and_then(|s| s.max).expect("a calculated range");
    assert!(max > 0.0);
    assert!(common::has(&h, "Recalculate range"));
    assert!(common::shows(&h, &format!("0 – {} h⁻¹", coxeter_core::js::number_to_string(max))));
    // `steadycom-mu`: the slider moves the draft, not the store.
    let mu = coxeter_core::js::string_to_number(&coxeter_core::js::to_fixed(max / 2.0, 6));
    common::set_value(&mut h, "Growth rate", mu);
    assert_eq!(h.state().ui.steady_com_draft.as_ref().map(|d| d.value.clone()), Some(coxeter_core::js::number_to_string(mu)));
    assert_eq!(value(&h, "Growth rate, h⁻¹"), coxeter_core::js::number_to_string(mu), "the number follows the slider");
    assert_eq!(h.state().store.steady_com_state.as_ref().unwrap().mu, None, "the store is told only on Solve");
    // `steadycom-solve`.
    {
        let store = &mut h.state_mut().store;
        store.parsimonious = true;
        store.loopless = false;
    }
    common::click(&mut h, "Solve at this µ");
    let solved = h.state().store.solution.as_ref().and_then(|s| s.steady_com.as_ref()).map(|s| s.mu);
    assert_eq!(solved, Some(mu));
    assert!(common::shows(&h, &format!("Composition at µ = {} h⁻¹:", coxeter_core::js::number_to_string(mu))));
    assert!(common::shows(&h, "Current method: SteadyCom fixed µ. The map shows a fixed-µ SteadyCom solution."));
    assert!(common::disabled(&h, "Enable Flux Variability Analysis"));
}

#[test]
fn the_growth_rate_field_says_what_is_wrong_with_it() {
    let mut h = fixture(TEACHING, false);
    common::click(&mut h, "Calculate range");
    let max = h.state().store.steady_com_state.as_ref().and_then(|s| s.max).expect("a calculated range");
    let field = "Growth rate, h⁻¹";
    // `steadycom-mu-number`: an empty field is not µ = 0.
    clear_field(&mut h, field);
    assert!(common::shows(&h, "Enter a finite growth rate; an empty field is not µ = 0."));
    assert!(invalid(&h, field));
    // A finite rate clears it; a negative one is refused by Solve.
    common::type_into(&mut h, field, "-0.1");
    assert!(!invalid(&h, field));
    assert!(!common::shows(&h, "an empty field is not µ = 0"));
    common::click(&mut h, "Solve at this µ");
    assert!(common::shows(&h, "Enter a finite growth rate of zero or more."));
    assert!(invalid(&h, field));
    assert!(h.state().store.solution.as_ref().is_none_or(|s| s.steady_com.is_none()));
    // Outside the range, the store refuses, under the controls and in the header.
    common::type_into(&mut h, field, &coxeter_core::js::number_to_string(max * 2.0));
    common::click(&mut h, "Solve at this µ");
    assert!(common::shows(&h, "Choose μ inside the computed range"));
    assert!(status(&h).starts_with("SteadyCom: Choose μ inside the computed range"));
    assert_eq!(h.state().store.status().state, StatusState::Error);
}

#[test]
fn the_parsimonious_choice_applies_to_the_next_fixed_growth_solve() {
    let mut h = fixture(TEACHING, false);
    assert!(!h.state().store.steady_com_config.parsimonious);
    common::click(&mut h, "Parsimonious: least total flux at this µ");
    assert!(h.state().store.steady_com_config.parsimonious);
    common::click(&mut h, "Parsimonious: least total flux at this µ");
    assert!(!h.state().store.steady_com_config.parsimonious);
}

#[test]
fn a_steadycom_job_that_fails_shows_its_error_under_the_controls() {
    let mut h = fixture(TEACHING, false);
    // What the shell does when a SteadyCom job fails (`runSteadyCom`'s catch).
    coxeter_app::panels::scenario::show_steady_com_error(h.state_mut(), "The LP failed.", None);
    common::settle(&mut h);
    assert!(common::shows(&h, "The LP failed."));
    let draft = h.state().ui.steady_com_draft.clone().unwrap();
    assert_eq!(draft.community_id.as_deref(), Some(TEACHING));
    assert_eq!(draft.value, "0", "the rate shown is kept");
}

// A published community

fn open_stolyar(name: &str) -> (H, SharedDialogs, std::path::PathBuf) {
    let (mut h, dialogs, dir) = common::harness_saving(name, None);
    open(&mut h, Workspace::Scenario);
    choose(&mut h, "Community", "✓ Desulfovibrio vulgaris + Methanococcus maripaludis");
    assert_eq!(h.state().store.scenario_published().as_deref(), Some("stolyar-2007-syntrophy"));
    (h, dialogs, dir)
}

fn saved_names(dialogs: &SharedDialogs) -> Vec<String> {
    common::saved(dialogs).into_iter().map(|(name, _)| name).collect()
}

#[test]
fn a_reproduced_community_shows_its_paper_results_and_figure() {
    let (mut h, _dialogs, _dir) = open_stolyar("stolyar-card");
    assert!(common::shows(&h, "REPRODUCED"));
    assert!(common::shows(&h, "UNVERIFIED"), "an unchecked DOI looks unchecked");
    assert!(common::shows(&h, "Desulfovibrio vulgaris Hildenborough"));
    assert!(common::shows(&h, "Recorded growth: Dv 0.05317 · Mm 0.02608 h⁻¹"));
    assert!(common::shows(&h, "Methane: 4.5115 mmol gDW⁻¹ h⁻¹"));
    // The sweep, in two panels with the table behind a disclosure.
    assert!(common::shows(&h, "Growth of both partners: flux through biomass"));
    assert!(common::shows(&h, "What crosses between them: mmol/gDW/h"));
    assert!(!common::shows(&h, "Sulfate available to D. vulgaris"), "the table is closed");
    common::click(&mut h, "Table view");
    assert!(common::shows(&h, "Sulfate available to D. vulgaris"), "the x column, without its unit");
    assert!(common::shows(&h, "0.0532"), "format(0.05317)");
    // Provenance: what it would still need is open, the rest closed.
    assert!(common::shows(&h, "Model provenance (1)"));
    assert!(!common::has(&h, "Calculate range"), "a joint-FBA example offers no SteadyCom block");
}

#[test]
fn member_models_media_and_findings_download() {
    let (mut h, dialogs, _dir) = open_stolyar("stolyar-downloads");
    // `download-member-json` (a link in the served build) and `download-member-sbml`.
    common::click(&mut h, "Download Dv as COBRA JSON");
    common::click(&mut h, "Download Mm as SBML");
    // `download-community-csv`: the medium and the bounds.
    common::click(&mut h, "Download the medium CSV");
    common::click(&mut h, "Download the bounds CSV");
    // `download-findings`.
    common::click(&mut h, "Download");
    let names = saved_names(&dialogs);
    assert_eq!(names.len(), 5, "{names:?}");
    assert!(names[0].contains("stolyar") && names[0].contains("Dv") && names[0].ends_with(".json"), "{names:?}");
    assert!(names[1].contains("Mm") && names[1].ends_with(".xml"), "{names:?}");
    assert!(names[2].contains("media") && names[2].ends_with(".csv"), "{names:?}");
    assert!(names[3].contains("bounds") && names[3].ends_with(".csv"), "{names:?}");
    assert!(names[4].ends_with(".md"), "{names:?}");
    let saved = common::saved(&dialogs);
    let model: Value = serde_json::from_slice(&saved[0].1).expect("a COBRA JSON model");
    assert!(model["reactions"].as_array().is_some_and(|r| r.len() > 50));
    assert!(String::from_utf8_lossy(&saved[1].1).contains("<sbml"));
    assert!(String::from_utf8_lossy(&saved[4].1).contains("## Reproduced paper findings"));
}

// The Zuñiga case

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

/// Synthetic small LPs, as in `zuniga-store.test.mjs`: integration only,
/// not the published models.
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
    let models = ZunigaModels { ec: Arc::new(Model::from_value(&mono).unwrap()), se: Arc::new(Model::from_value(&se).unwrap()) };
    let mut source_hashes = IndexMap::new();
    source_hashes.insert("fixture".to_string(), "synthetic-only".to_string());
    ZunigaStudy {
        manifest: json!({ "community": ZUNIGA_ID, "runtime": "zuniga-mat", "map": "map.json", "fluxReference": 10, "members": [{ "id": "Ec" }, { "id": "Se" }] }),
        source_hashes,
        source_models: ZunigaSourceModels { mono, joint },
        models,
    }
}

/// The case opened on the synthetic study, its drawing in memory as the
/// single-file build embeds it.
fn zuniga_harness(name: &str) -> (H, SharedDialogs) {
    let study = fixture_study();
    let context = zuniga_community_model(&create_zuniga_scenario(&study, "mutant-coculture").unwrap().community);
    let mut map = EditableMap::empty_from_community(&context, &LayoutOptions { width: Some(1800.0), height: Some(1200.0), ..LayoutOptions::default() });
    map.add_reaction(&context, "Ec@PPNDH", &PlaceOptions::default());
    map.add_reaction(&context, "Se@PHELtex", &PlaceOptions::default());
    let assets = MemoryAssets::embedded();
    assets.insert_json(format!("models/published/{ZUNIGA_ID}/map.json"), &map.to_json());
    let mut store = Store::new(Arc::new(assets)).with_fetcher(Arc::new(NoFetcher));
    store.published_communities = vec![json!({
        "id": ZUNIGA_ID, "name": "Synthetic integration fixture", "description": "A synthetic stand-in for the case.",
        "species": [{ "id": "Ec", "name": "Ec", "organism": "E. coli" }, { "id": "Se", "name": "Se", "organism": "S. elongatus" }],
        "paper": { "title": "Fixture", "year": 2020, "authors": "Fixture", "journal": "Fixture", "doi": "10.0000/fixture", "framework": "FBA" },
        "reproduction": { "status": "reproduced", "needs": [] },
    })];
    store.use_zuniga_study(Some(study), None).expect("the fixture case opens");
    let dialogs = SharedDialogs::new(ScriptedDialogs::saving_to(common::temp_dir(name)));
    let mut h = common::harness_of(Shell::new(store, Box::new(dialogs.clone())));
    open(&mut h, Workspace::Scenario);
    (h, dialogs)
}

fn any_control_starting(h: &H, prefix: &str) -> bool {
    h.query_all_by(|n| !matches!(n.role(), Role::Label | Role::Unknown | Role::GenericContainer) && n.label().is_some_and(|l| l.starts_with(prefix)))
        .next()
        .is_some()
}

#[test]
fn the_case_controls_expose_honest_source_constraints_growth_comparison_and_editable_map_tools() {
    let (mut h, _) = zuniga_harness("zuniga-controls");
    assert!(common::has(&h, "Zuñiga case scenario"));
    assert!(common::has(&h, "Zuñiga flux comparison"));
    assert!(common::shows(&h, "Ec PPNDH (pheA)"));
    assert!(common::shows(&h, "Calculated phenylalanine transfer"));
    assert!(common::shows(&h, "not a comparison of experimental"));
    assert!(!common::has(&h, "Community objective"), "the authors' objective is fixed");
    assert!(common::shows(&h, "Authors’ original objective"));
    assert!(common::has(&h, "Export PNG"));
    // Growth per case scenario: the WT grows, the mutant alone does not.
    assert!(common::shows(&h, "WT monoculture"));
    assert!(common::shows(&h, "Absent"), "S. elongatus is absent from the monocultures");
    open(&mut h, Workspace::Medium);
    assert!(common::shows(&h, "Source medium"));
    assert!(!any_control_starting(&h, "Supply of"), "no medium sliders");
    open(&mut h, Workspace::Community);
    assert!(common::shows(&h, "Case organisms"));
    assert!(!any_control_starting(&h, "Abundance of"), "no abundance sliders");
    assert!(common::shows(&h, "Growth: "));
    open(&mut h, Workspace::Model);
    assert!(common::shows(&h, "Complete source models"));
    assert!(!common::has(&h, "+ Reaction"), "source biology is read-only");
}

#[test]
fn the_case_scenario_and_flux_comparison_are_chosen() {
    let (mut h, _) = zuniga_harness("zuniga-choices");
    assert_eq!(value(&h, "Zuñiga case scenario"), "Mutant coculture");
    choose(&mut h, "Zuñiga case scenario", "Mutant monoculture");
    assert_eq!(h.state().store.zuniga.scenario_id.as_deref(), Some("mutant-monoculture"));
    choose(&mut h, "Zuñiga flux comparison", "Δ flux vs WT monoculture");
    assert_eq!(h.state().store.zuniga.comparison, "difference");
    assert!(common::shows(&h, "Δv = selected − WT"));
}

#[test]
fn the_case_exports_its_figure_bundle_and_private_models() {
    let (mut h, dialogs) = zuniga_harness("zuniga-exports");
    common::click(&mut h, "Export SVG");
    common::click(&mut h, "Export PNG");
    common::click(&mut h, "Private reproducibility bundle");
    assert!(!common::disabled(&h, "Download Ec as COBRA JSON"), "the privately loaded model can be exported");
    common::click(&mut h, "Download Ec as COBRA JSON");
    common::click(&mut h, "Download Se as SBML");
    let names = saved_names(&dialogs);
    assert_eq!(names.len(), 5, "{names:?}");
    assert!(names[0].ends_with(".svg") && names[1].ends_with(".png") && names[2].ends_with(".json"), "{names:?}");
    assert!(names[3].contains("Ec") && names[3].ends_with(".json"), "{names:?}");
    assert!(names[4].contains("Se") && names[4].ends_with(".xml"), "{names:?}");
    let svg = String::from_utf8_lossy(&common::saved(&dialogs)[0].1).into_owned();
    assert!(svg.contains("Stationary source-model FBA"), "the case travels in the SVG's metadata");
}

#[test]
fn the_case_before_its_sources_are_downloaded_says_so() {
    let mut shell = common::shell(None);
    let name = shell
        .store
        .published_communities
        .iter()
        .find(|c| c["id"] == ZUNIGA_ID)
        .map(|c| c["name"].clone())
        .expect("the case is registered");
    // Selected, its sources not yet in: what the card shows meanwhile.
    shell.store.scenario = Some(json!({ "id": format!("published:{ZUNIGA_ID}"), "published": ZUNIGA_ID, "name": name }));
    let mut h = common::harness_of(shell);
    open(&mut h, Workspace::Scenario);
    assert!(common::shows(&h, "Loading requires an internet connection the first time."));
    assert!(common::shows(&h, "SOURCE CONDITIONS"));
    assert!(common::disabled(&h, "Download Ec as COBRA JSON"));
    assert!(common::disabled(&h, "Download Se as SBML"));
    assert!(!common::has(&h, "Zuñiga case scenario"));
}

// The Solver block (the native app's own)

/// Frames until the background check of the solvers has finished.
fn solvers_checked(h: &mut H) {
    for _ in 0..1200 {
        let _ = h.try_run();
        if h.state().solvers.checked && !h.state().solvers.checking() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    common::settle(h);
}

#[test]
fn the_solver_block_lists_the_solvers_and_switches_every_analysis() {
    let mut h = common::harness(None);
    open(&mut h, Workspace::Analysis);
    solvers_checked(&mut h);
    let highs = format!("{} · built in", h.state().store.solver.label());
    assert_eq!(value(&h, "Solver"), highs);
    assert!(common::shows(&h, "Every analysis runs on it"));
    assert!(common::has(&h, "Check the solvers again"));
    assert!(common::has(&h, "Locate the Gurobi library"));
    assert!(common::has(&h, "Locate the CPLEX library"));
    // Each solver this computer can use takes over the analyses: the
    // solution, the select and the summary of the Simulate tab say so.
    let usable: Vec<SolverKind> =
        h.state().solvers.infos.iter().filter(|i| i.usable() && i.kind != SolverKind::Highs).map(|i| i.kind).collect();
    for kind in usable {
        h.state_mut().choose_solver(kind);
        common::settle(&mut h);
        assert_eq!(h.state().store.solver.kind(), kind);
        assert_eq!(h.state().store.solution.as_ref().unwrap().solver, kind.id(), "{} solved it", kind.name());
        assert!(value(&h, "Solver").starts_with(kind.name()), "{}", value(&h, "Solver"));
    }
    choose(&mut h, "Solver", &highs);
    assert_eq!(h.state().store.solver.kind(), SolverKind::Highs);
    assert_eq!(h.state().store.solution.as_ref().unwrap().solver, "highs");
}
