//! test/steadycom.test.mjs, test/steadycom-search.test.mjs,
//! test/steadycom-glpk-status.test.mjs and test/steadycom-runtime.test.mjs,
//! ported, plus parity with the JS answers on the same communities
//! (rust/fixtures/solver/steadycom-small.json).

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use coxeter_core::Model;
use coxeter_core::testkit::read_fixture;
use coxeter_solver::fba::FluxProblem;
use coxeter_solver::lp::{LpBackend, LpProblem, LpSolution, LpStatus, SolveOptions};
use coxeter_solver::steadycom::{
    AtOptions, Member, SteadyComError, SteadyComOptions, SteadyComProgress, SteadyComSpec, SteadyComStatus, SteadyComTarget,
    max_total_biomass_at, parsimonious_at, steady_com, steady_com_fva,
};
use coxeter_solver::steadycom_runtime::{MemberModel, exchange_index, steady_com_spec, to_merged_fluxes};
use common::{close, num};
use indexmap::IndexMap;
use serde_json::{Value, json};

const GLUCOSE_BUDGET: f64 = 10.0;
const EXPECTED_GROWTH: f64 = GLUCOSE_BUDGET / 2.0;

/// One member: `makes` is secreted, `needs` is taken up, both one per biomass.
fn member_doc(id: &str, makes: &str, needs: &str) -> Value {
    json!({
        "id": id, "name": id,
        "compartments": {"e": "Extracelular", "c": "Citosol"},
        "metabolites": [
            {"id": "glc_e", "name": "glucose", "compartment": "e"},
            {"id": "glc_c", "name": "glucose", "compartment": "c"},
            {"id": format!("{makes}_c"), "name": makes, "compartment": "c"},
            {"id": format!("{makes}_e"), "name": makes, "compartment": "e"},
            {"id": format!("{needs}_c"), "name": needs, "compartment": "c"},
            {"id": format!("{needs}_e"), "name": needs, "compartment": "e"}
        ],
        "reactions": [
            {"id": "EX_glc_e", "metabolites": {"glc_e": -1}, "lower_bound": -GLUCOSE_BUDGET, "upper_bound": 1000},
            {"id": "GLCt", "metabolites": {"glc_e": -1, "glc_c": 1}, "lower_bound": 0, "upper_bound": 100},
            {"id": "BIOMASS", "metabolites": {"glc_c": -1, format!("{needs}_c"): -1}, "lower_bound": 0, "upper_bound": 100},
            {"id": format!("MK_{makes}"), "metabolites": {"glc_c": -1, format!("{makes}_c"): 1}, "lower_bound": 0, "upper_bound": 100},
            {"id": format!("T_{makes}"), "metabolites": {format!("{makes}_c"): -1, format!("{makes}_e"): 1}, "lower_bound": 0, "upper_bound": 100},
            {"id": format!("EX_{makes}_e"), "metabolites": {format!("{makes}_e"): -1}, "lower_bound": 0, "upper_bound": 1000},
            {"id": format!("T_{needs}"), "metabolites": {format!("{needs}_e"): -1, format!("{needs}_c"): 1}, "lower_bound": 0, "upper_bound": 100},
            {"id": format!("EX_{needs}_e"), "metabolites": {format!("{needs}_e"): -1}, "lower_bound": -100, "upper_bound": 1000}
        ],
        "objective": {"BIOMASS": 1}
    })
}

fn member(id: &str, makes: &str, needs: &str, doc: Option<Value>) -> Member {
    let model = Model::from_value(&doc.unwrap_or_else(|| member_doc(id, makes, needs))).unwrap();
    Member {
        id: id.into(),
        name: None,
        biomass_reaction: "BIOMASS".into(),
        atp_maintenance: None,
        model: Some(Arc::new(model)),
        exchange_for: [("glc", "EX_glc_e".to_string()), (makes, format!("EX_{makes}_e")), (needs, format!("EX_{needs}_e"))]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    }
}

/// The pair, plus the medium: glucose only. Neither x nor y is supplied.
fn mutualists(total_biomass: f64, block_a: bool) -> SteadyComSpec {
    let mut a_doc = member_doc("A", "x", "y");
    if block_a {
        for r in a_doc["reactions"].as_array_mut().unwrap() {
            if r["id"] == "MK_x" {
                r["lower_bound"] = json!(0);
                r["upper_bound"] = json!(0);
            }
        }
    }
    SteadyComSpec {
        members: vec![member("A", "x", "y", Some(a_doc)), member("B", "y", "x", None)],
        total_biomass,
        biomass_cap: None,
        community_uptake: [("glc".to_string(), GLUCOSE_BUDGET)].into_iter().collect(),
        community_exchange_bounds: IndexMap::new(),
        joint: None,
    }
}

fn opts<'a>(upper_guess: f64) -> SteadyComOptions<'a> {
    SteadyComOptions { upper_guess, ..Default::default() }
}

// steadycom.test.mjs

#[test]
fn reaches_the_growth_rate_and_abundances_the_medium_dictates() {
    let r = steady_com(&mutualists(1.0, false), opts(20.0)).unwrap();
    assert_eq!(r.status, SteadyComStatus::Optimal, "{}", r.message);
    assert!((r.growth_rate - EXPECTED_GROWTH).abs() < 1e-4, "got {}", r.growth_rate);
    assert!((r.abundances["A"] - 0.5).abs() < 1e-4);
    assert!((r.abundances["B"] - 0.5).abs() < 1e-4);
    let total: f64 = r.abundances.values().sum();
    assert!((total - 1.0).abs() < 1e-6, "total biomass {total}");
}

#[test]
fn twice_the_biomass_on_the_same_budget_grows_half_as_fast() {
    let one = steady_com(&mutualists(1.0, false), opts(20.0)).unwrap();
    let two = steady_com(&mutualists(2.0, false), opts(20.0)).unwrap();
    assert!((two.growth_rate - one.growth_rate / 2.0).abs() < 1e-4);
}

#[test]
fn neither_member_grows_alone_without_its_open_uptake() {
    for m in mutualists(1.0, false).members {
        let model = m.model.as_ref().unwrap();
        let other = if m.id == "A" { "y" } else { "x" };
        assert!(model.reactions[&format!("EX_{other}_e")].lower_bound < 0.0);
        assert!(FluxProblem::new(model).optimize().objective_value > 0.0, "with uptake open, a lone member does grow");
    }
}

#[test]
fn a_metabolite_the_medium_does_not_supply_can_only_come_from_a_partner() {
    let crippled = steady_com(&mutualists(1.0, true), opts(20.0)).unwrap();
    let healthy = steady_com(&mutualists(1.0, false), opts(20.0)).unwrap();
    assert!(crippled.growth_rate < healthy.growth_rate / 1000.0, "B cannot get x, yet grew at {}", crippled.growth_rate);
}

#[test]
fn cross_feeding_balances_and_mass_balance_and_scaled_bounds_hold() {
    let spec = mutualists(1.0, false);
    let r = steady_com(&spec, opts(20.0)).unwrap();
    for (producer, consumer, base) in [("A", "B", "x"), ("B", "A", "y")] {
        let out = r.fluxes[&format!("{producer}|EX_{base}_e")];
        let into = r.fluxes[&format!("{consumer}|EX_{base}_e")];
        assert!(out > 1e-6 && into < -1e-6, "{base}: {out} / {into}");
        assert!((out + into).abs() < 1e-5);
    }
    for m in &spec.members {
        let model = m.model.as_ref().unwrap();
        let x = r.abundances[&m.id];
        let mut balance: IndexMap<&str, f64> = IndexMap::new();
        for (rid, rxn) in &model.reactions {
            let v = r.fluxes.get(&format!("{}|{rid}", m.id)).copied().unwrap_or(0.0);
            assert!(v <= rxn.upper_bound * x + 1e-6 && v >= rxn.lower_bound * x - 1e-6, "{}|{rid}", m.id);
            for (mid, coef) in &rxn.metabolites {
                *balance.entry(mid).or_default() += coef * v;
            }
        }
        for (mid, net) in balance {
            assert!(net.abs() < 1e-6, "{}: {mid} off by {net}", m.id);
        }
    }
}

#[test]
fn the_answer_is_reproducible_across_runs() {
    let first = steady_com(&mutualists(1.0, false), opts(20.0)).unwrap();
    let second = steady_com(&mutualists(1.0, false), opts(20.0)).unwrap();
    assert_eq!(first.growth_rate, second.growth_rate);
    for (id, x) in &first.abundances {
        assert!((x - second.abundances[id]).abs() < 1e-9);
    }
}

#[test]
fn maximum_total_biomass_falls_away_as_the_growth_rate_rises() {
    let spec = mutualists(1.0, false);
    let mut previous = f64::INFINITY;
    for mu in [0.5, 1.0, 2.0, 4.0, 5.0] {
        let step = max_total_biomass_at(&spec, mu, &AtOptions::default()).unwrap();
        assert_eq!(step.status, LpStatus::Optimal, "mu={mu}: {}", step.message);
        assert!(step.total <= previous + 1e-6);
        previous = step.total;
    }
}

#[test]
fn an_unreachable_growth_rate_is_infeasible_not_zero_and_capped_it_is_a_verdict() {
    let beyond = max_total_biomass_at(&mutualists(1.0, false), EXPECTED_GROWTH * 2.0, &AtOptions::default()).unwrap();
    assert!(!beyond.feasible);
    assert_eq!(beyond.status, LpStatus::Infeasible, "a solver that gave up must not be recorded as a community that cannot grow");

    let capped = max_total_biomass_at(&mutualists(1.0, false), EXPECTED_GROWTH * 2.0, &AtOptions { biomass_cap: Some(2.0), ..Default::default() }).unwrap();
    assert_eq!(capped.status, LpStatus::Optimal, "with a cap the LP is always feasible");
    assert!(capped.total < 1.0);
    assert!(!capped.feasible);
    let search = steady_com(&mutualists(1.0, false), SteadyComOptions { upper_guess: EXPECTED_GROWTH * 4.0, biomass_cap: Some(2.0), ..Default::default() }).unwrap();
    assert_eq!(search.status, SteadyComStatus::Optimal, "{}", search.message);
    assert!(search.iterations > 1);
    assert!((search.growth_rate - EXPECTED_GROWTH).abs() < 1e-4);
}

#[test]
fn capping_the_total_biomass_and_fixing_it_give_the_same_growth_rate() {
    let fixed = steady_com(&mutualists(1.0, false), opts(20.0)).unwrap();
    let capped = steady_com(&mutualists(1.0, false), SteadyComOptions { upper_guess: 20.0, biomass_cap: Some(2.0), ..Default::default() }).unwrap();
    assert_eq!(capped.status, SteadyComStatus::Optimal, "{}", capped.message);
    assert!((capped.growth_rate - fixed.growth_rate).abs() < 1e-4);
    assert!((capped.growth_rate - EXPECTED_GROWTH).abs() < 1e-4);
    for (id, x) in &fixed.abundances {
        assert!((x - capped.abundances[id]).abs() < 1e-4);
    }
}

#[test]
fn the_parsimonious_solve_drops_a_futile_cycle_and_costs_ten_mu() {
    let mut spec = mutualists(1.0, false);
    let mut doc = spec.members[0].model.as_ref().unwrap().to_json();
    doc["metabolites"].as_array_mut().unwrap().push(json!({"id": "z_c", "name": "z", "compartment": "c"}));
    doc["reactions"].as_array_mut().unwrap().push(json!({"id": "CYC1", "metabolites": {"glc_c": -1, "z_c": 1}, "lower_bound": 0, "upper_bound": 100}));
    doc["reactions"].as_array_mut().unwrap().push(json!({"id": "CYC2", "metabolites": {"z_c": -1, "glc_c": 1}, "lower_bound": 0, "upper_bound": 100}));
    spec.members[0].model = Some(Arc::new(Model::from_value(&doc).unwrap()));
    let mu = EXPECTED_GROWTH / 2.0;
    let lean = parsimonious_at(&spec, mu, &AtOptions::default()).unwrap();
    assert_eq!(lean.status, LpStatus::Optimal, "{}", lean.message);
    assert!(lean.feasible);
    assert!((lean.total_flux.unwrap() - 10.0 * mu).abs() < 1e-6, "least total flux {:?}", lean.total_flux);
    assert!(lean.fluxes["A|CYC1"].abs() < 1e-9 && lean.fluxes["A|CYC2"].abs() < 1e-9, "the cycle should carry nothing");
    let total: f64 = lean.abundances.values().sum();
    assert!((total - 1.0).abs() < 1e-7);
    for (id, x) in &lean.abundances {
        assert!((lean.fluxes[&format!("{id}|BIOMASS")] - mu * x).abs() < 1e-7);
    }
}

// steadycom-search.test.mjs

fn single(biomass_lower: f64, unbounded: bool) -> SteadyComSpec {
    let mut model = Model::from_value(&json!({
        "id": "A", "compartments": {"e": "Extracellular"},
        "metabolites": [{"id": "a_e", "compartment": "e"}],
        "reactions": [
            {"id": "EX_a_e", "metabolites": {"a_e": -1}, "lower_bound": -1, "upper_bound": 0},
            {"id": "BIOMASS", "metabolites": if unbounded { json!({}) } else { json!({"a_e": -1}) }, "lower_bound": biomass_lower, "upper_bound": 10}
        ], "objective": {"BIOMASS": 1}
    }))
    .unwrap();
    if unbounded {
        model.reactions.get_mut("BIOMASS").unwrap().upper_bound = f64::INFINITY;
    }
    SteadyComSpec {
        members: vec![Member {
            model: Some(Arc::new(model)),
            exchange_for: [("a".to_string(), "EX_a_e".to_string())].into_iter().collect(),
            ..Member::joint("A", "BIOMASS")
        }],
        total_biomass: 1.0,
        biomass_cap: None,
        community_uptake: [("a".to_string(), 1.0)].into_iter().collect(),
        community_exchange_bounds: IndexMap::new(),
        joint: None,
    }
}

#[test]
fn a_feasible_bracketing_point_retains_fluxes_at_its_growth_rate() {
    let r = steady_com(&single(0.0, false), SteadyComOptions { upper_guess: 1.0, tolerance: 0.001, ..Default::default() }).unwrap();
    assert_eq!(r.status, SteadyComStatus::Optimal);
    assert_eq!(r.growth_rate, 1.0);
    assert!((r.fluxes["A|BIOMASS"] - r.growth_rate * r.abundances["A"]).abs() < 1e-12);
}

#[test]
fn only_a_resolved_bracket_supplies_the_verified_range() {
    let progress: Arc<Mutex<Vec<SteadyComProgress>>> = Arc::default();
    let sink = progress.clone();
    let r = steady_com(
        &single(0.0, false),
        SteadyComOptions {
            upper_guess: 2.0,
            biomass_cap: Some(2.0),
            on_progress: Some(Box::new(move |p| sink.lock().unwrap().push(*p))),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(r.status, SteadyComStatus::Optimal, "{}", r.message);
    assert_eq!(r.zero_feasible, Some(true));
    assert_eq!(r.range_lower, Some(0.0));
    assert_eq!(r.range_upper, Some(r.growth_rate));
    let (upper, range_upper) = (r.upper_bound.unwrap(), r.range_upper.unwrap());
    assert!(upper >= range_upper && upper - range_upper <= r.resolution.unwrap());
    assert!((range_upper - 1.0).abs() < 1e-4);
    let progress = progress.lock().unwrap();
    assert_eq!(progress.len() as u32, r.iterations);
    assert_eq!(progress.iter().map(|p| p.iterations).collect::<Vec<_>>(), (1..=progress.len() as u32).collect::<Vec<_>>());
    assert!(progress.iter().all(|p| p.lo.is_finite() && p.hi.is_finite() && p.mu.is_finite()));
}

#[test]
fn an_exhausted_lp_budget_cannot_be_labelled_optimal() {
    let r = steady_com(&single(0.0, false), SteadyComOptions { upper_guess: 2.0, max_iterations: 2, ..Default::default() }).unwrap();
    assert_eq!(r.status, SteadyComStatus::IterationLimit);
    assert_eq!(r.zero_feasible, Some(true));
    assert_eq!(r.iterations, 2);
    assert_eq!(r.range_upper, None);
    assert!(r.fluxes.is_empty());
}

#[test]
fn failure_to_find_an_upper_bracket_cannot_invent_a_finite_optimum() {
    let r = steady_com(&single(0.0, true), opts(2.0)).unwrap();
    assert_eq!(r.status, SteadyComStatus::IterationLimit);
    assert!(r.message.contains("No finite upper"), "{}", r.message);
    assert_eq!(r.range_upper, None);
}

#[test]
fn an_excluded_zero_does_not_imply_that_positive_growth_is_impossible() {
    let spec = single(0.5, false);
    assert_eq!(max_total_biomass_at(&spec, 0.0, &AtOptions::default()).unwrap().status, LpStatus::Infeasible);
    assert!(max_total_biomass_at(&spec, 0.5, &AtOptions::default()).unwrap().feasible);
    let r = steady_com(&spec, opts(2.0)).unwrap();
    assert_eq!(r.status, SteadyComStatus::ZeroInfeasible);
    assert_eq!(r.zero_feasible, Some(false));
    assert!(r.message.contains("positive growth rate may still be feasible"));
    assert_eq!(r.range_lower, None);
}

#[test]
fn invalid_rates_and_search_options_are_refused_before_solving() {
    for mu in [f64::NAN, f64::INFINITY, -1.0] {
        let e = max_total_biomass_at(&single(0.0, false), mu, &AtOptions::default()).unwrap_err();
        assert!(e.to_string().contains("finite number"), "{e}");
    }
    let bad: Vec<SteadyComOptions> = vec![
        SteadyComOptions { tolerance: 0.0, ..Default::default() },
        SteadyComOptions { tolerance: f64::INFINITY, ..Default::default() },
        SteadyComOptions { upper_guess: f64::NAN, ..Default::default() },
        SteadyComOptions { upper_guess: -1.0, ..Default::default() },
        SteadyComOptions { max_iterations: 0, ..Default::default() },
        SteadyComOptions { time_limit_seconds: 0.0, ..Default::default() },
        SteadyComOptions { biomass_cap: Some(0.5), ..Default::default() },
    ];
    for o in bad {
        assert!(matches!(steady_com(&single(0.0, false), o), Err(SteadyComError::Invalid(_))));
    }
    let mut zero = single(0.0, false);
    zero.total_biomass = 0.0;
    let e = steady_com(&zero, SteadyComOptions::default()).unwrap_err();
    assert!(e.to_string().contains("Total community biomass"), "{e}");
}

// steadycom-glpk-status.test.mjs

/// A solver that never finishes: feasible-but-unproven, as GLP_FEAS was.
struct Unfinished;
impl LpBackend for Unfinished {
    fn solve(&self, _: &LpProblem, _: &SolveOptions) -> LpSolution {
        LpSolution::failed(LpStatus::IterationLimit, "HiGHS status time limit")
    }
}

#[test]
fn unfinished_capped_objectives_cannot_generate_a_maximum_or_a_zero_infeasible_verdict() {
    let model = Model::from_value(&json!({"id": "A", "compartments": {}, "metabolites": [], "reactions": [
        {"id": "BIOMASS", "metabolites": {}, "lower_bound": 0, "upper_bound": 1}
    ], "objective": {"BIOMASS": 1}}))
    .unwrap();
    let spec = SteadyComSpec {
        members: vec![Member { model: Some(Arc::new(model)), ..Member::joint("A", "BIOMASS") }],
        total_biomass: 1.0,
        biomass_cap: None,
        community_uptake: IndexMap::new(),
        community_exchange_bounds: IndexMap::new(),
        joint: None,
    };
    let r = steady_com(&spec, SteadyComOptions { biomass_cap: Some(2.0), backend: Some(&Unfinished), ..Default::default() }).unwrap();
    assert_eq!(r.status, SteadyComStatus::SolverError);
    assert_eq!(r.zero_feasible, None);
    assert_eq!(r.range_upper, None);
    assert!(r.fluxes.is_empty());
    assert!(r.message.contains("did not solve (iteration_limit"), "{}", r.message);
}

// steadycom-runtime.test.mjs

fn pair() -> Vec<MemberModel> {
    vec![
        MemberModel::new("A", Model::from_value(&member_doc("A", "x", "y")).unwrap()),
        MemberModel::new("B", Model::from_value(&member_doc("B", "y", "x")).unwrap()),
    ]
}

fn budget() -> IndexMap<String, f64> {
    [("glc".to_string(), GLUCOSE_BUDGET)].into_iter().collect()
}

#[test]
fn the_exchange_index_keys_every_ex_reaction_by_its_metabolite() {
    let index = exchange_index(&pair()[0].model).unwrap();
    assert_eq!(index["glc"], "EX_glc_e");
    assert_eq!(index["x"], "EX_x_e");
    assert_eq!(index["y"], "EX_y_e");
    assert!(!index.contains_key("GLCt"));
}

#[test]
fn a_spec_built_from_member_models_alone_finds_the_same_growth_rate() {
    let spec = steady_com_spec(&pair(), &budget(), 1.0).unwrap();
    assert_eq!(spec.members.iter().map(|m| m.biomass_reaction.as_str()).collect::<Vec<_>>(), ["BIOMASS", "BIOMASS"]);
    let r = steady_com(&spec, opts(20.0)).unwrap();
    assert_eq!(r.status, SteadyComStatus::Optimal);
    assert!((r.growth_rate - EXPECTED_GROWTH).abs() < 1e-4);
}

#[test]
fn a_member_with_no_biomass_reaction_is_refused() {
    let model = Model::from_value(&json!({"id": "C", "compartments": {"c": "Cytosol"}, "metabolites": [{"id": "a_c", "compartment": "c"}],
        "reactions": [{"id": "R", "metabolites": {"a_c": 1}, "lower_bound": 0, "upper_bound": 1}]}))
    .unwrap();
    let e = steady_com_spec(&[MemberModel::new("C", model)], &IndexMap::new(), 1.0).unwrap_err();
    assert!(e.to_string().contains("no biomass reaction"), "{e}");
}

#[test]
fn at_a_fixed_growth_rate_every_member_grows_at_exactly_that_rate() {
    let spec = steady_com_spec(&pair(), &budget(), 1.0).unwrap();
    let max = steady_com(&spec, opts(20.0)).unwrap().growth_rate;
    let mu = max / 2.0;
    let at = max_total_biomass_at(&spec, mu, &AtOptions::default()).unwrap();
    assert!(at.feasible);
    let fluxes = to_merged_fluxes(&at.fluxes, "@", None);
    for id in ["A", "B"] {
        let x = at.abundances[id];
        assert!(x > 0.0);
        assert!((fluxes[&format!("{id}@BIOMASS")] / x - mu).abs() < 1e-6);
    }
    let beyond = max_total_biomass_at(&spec, max * 2.0, &AtOptions::default()).unwrap();
    assert!(!beyond.feasible);
    assert!(beyond.fluxes.is_empty());
    // Below the maximum a fixed rate leaves more than one community.
    let most = max_total_biomass_at(&spec, mu, &AtOptions { biomass_cap: Some(2.0), ..Default::default() }).unwrap();
    assert!(most.total > 1.0, "reached {}", most.total);
}

// SteadyComFVA

#[test]
fn steadycom_fva_holds_the_community_at_its_maximum() {
    let spec = mutualists(1.0, false);
    let targets = vec![
        SteadyComTarget { name: "bio_A".into(), reactions: [("A\u{0}BIOMASS".to_string(), 1.0)].into_iter().collect(), ..Default::default() },
        SteadyComTarget { name: "X_B".into(), biomass: [("B".to_string(), 1.0)].into_iter().collect(), ..Default::default() },
        SteadyComTarget { name: "bio_B".into(), reactions: [("BIOMASS".to_string(), 1.0)].into_iter().collect(), species: Some("B".into()), ..Default::default() },
    ];
    let r = steady_com_fva(&spec, &targets, 1.0, opts(20.0)).unwrap();
    assert_eq!(r.status, SteadyComStatus::Optimal);
    assert!(r.unsolved.is_empty(), "{}", r.message);
    // At the maximum the composition is pinned: X_B = 0.5, biomass flux mu/2.
    assert!((r.ranges[1].min - 0.5).abs() < 1e-4 && (r.ranges[1].max - 0.5).abs() < 1e-4);
    assert!((r.ranges[0].max - EXPECTED_GROWTH / 2.0).abs() < 1e-3);
    assert!(close(r.ranges[2].min, r.ranges[0].min, 1e-4));
    let missing = vec![SteadyComTarget { name: "nope".into(), biomass: [("Z".to_string(), 1.0)].into_iter().collect(), ..Default::default() }];
    let e = steady_com_fva(&spec, &missing, 1.0, opts(20.0)).unwrap_err();
    assert_eq!(e.to_string(), "target nope: no member Z");
}

#[test]
fn a_cancelled_search_stops() {
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let r = steady_com(
        &mutualists(1.0, false),
        SteadyComOptions {
            upper_guess: 20.0,
            cancel: Some(cancel),
            on_progress: Some(Box::new(move |p| {
                if p.iterations == 3 {
                    flag.store(true, Ordering::Relaxed);
                }
            })),
            ..Default::default()
        },
    );
    assert_eq!(r.unwrap_err(), SteadyComError::Cancelled);
}

// Parity with the JS answers

fn check_steady(label: &str, rust: &coxeter_solver::steadycom::SteadyComResult, js: &Value, tol: f64) {
    assert_eq!(rust.status.as_str(), js["status"], "{label}: {}", rust.message);
    assert!(close(rust.growth_rate, num(&js["growthRate"]), tol), "{label}: mu {} vs JS {}", rust.growth_rate, js["growthRate"]);
    for (id, x) in js["abundances"].as_object().unwrap() {
        assert!(close(rust.abundances[id], num(x), 1e-4), "{label}: X_{id} {} vs JS {x}", rust.abundances[id]);
    }
    assert_eq!(rust.zero_feasible, js["zeroFeasible"].as_bool());
}

#[test]
fn small_communities_match_the_javascript() {
    let fx = read_fixture("solver/steadycom-small.json").expect("run rust/tools/fixtures-solver.mjs");
    let spec = steady_com_spec(
        &[
            MemberModel::new("A", Model::from_value(&fx["members"]["A"]).unwrap()),
            MemberModel::new("B", Model::from_value(&fx["members"]["B"]).unwrap()),
        ],
        &budget(),
        1.0,
    )
    .unwrap();
    for (i, m) in spec.members.iter().enumerate() {
        let js: IndexMap<String, String> =
            fx["exchangeFor"][i].as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.as_str().unwrap().into())).collect();
        assert_eq!(m.exchange_for, js);
    }
    // The bisection resolves mu to its tolerance; both land within it.
    check_steady("fixed", &steady_com(&spec, opts(20.0)).unwrap(), &fx["fixed"], 2e-6);
    check_steady("capped", &steady_com(&spec, SteadyComOptions { upper_guess: 20.0, biomass_cap: Some(2.0), ..Default::default() }).unwrap(), &fx["capped"], 2e-5);
    let app = steady_com(
        &spec,
        SteadyComOptions { upper_guess: 1.0, tolerance: 1e-5, biomass_cap: Some(2.0), time_limit_seconds: 30.0, ..Default::default() },
    )
    .unwrap();
    check_steady("app options", &app, &fx["appOptions"], 2e-5);
    assert_eq!(app.range_upper.map(|_| ()), fx["appOptions"]["rangeUpper"].as_f64().map(|_| ()));

    for case in fx["at"].as_array().unwrap() {
        let mu = num(&case["mu"]);
        for (label, cap) in [("fixed", None), ("capped", Some(2.0))] {
            let r = max_total_biomass_at(&spec, mu, &AtOptions { biomass_cap: cap, ..Default::default() }).unwrap();
            let js = &case[label];
            assert_eq!(r.status.as_str(), js["status"], "mu={mu} {label}");
            assert_eq!(r.feasible, js["feasible"].as_bool().unwrap(), "mu={mu} {label}");
            if r.status == LpStatus::Optimal {
                // The fixed total is unique; the capped maximum total is too.
                assert!(close(r.total, num(&js["total"]), 1e-6), "mu={mu} {label}: total {} vs {}", r.total, js["total"]);
            }
        }
    }
    for case in fx["parsimonious"].as_array().unwrap() {
        let mu = num(&case["mu"]);
        let r = parsimonious_at(&spec, mu, &AtOptions::default()).unwrap();
        assert_eq!(r.status.as_str(), case["status"]);
        assert!(close(r.total_flux.unwrap(), num(&case["totalFlux"]), 1e-6), "mu={mu}: least flux {:?} vs {}", r.total_flux, case["totalFlux"]);
    }
    let targets = vec![
        SteadyComTarget { name: "bio_A".into(), reactions: [("A\u{0}BIOMASS".to_string(), 1.0)].into_iter().collect(), ..Default::default() },
        SteadyComTarget { name: "X_B".into(), biomass: [("B".to_string(), 1.0)].into_iter().collect(), ..Default::default() },
        SteadyComTarget {
            name: "glc".into(),
            reactions: [("A\u{0}EX_glc_e".to_string(), 1.0), ("B\u{0}EX_glc_e".to_string(), 1.0)].into_iter().collect(),
            ..Default::default()
        },
    ];
    let fva = steady_com_fva(&spec, &targets, 1.0, opts(20.0)).unwrap();
    for (r, js) in fva.ranges.iter().zip(fx["fva"]["ranges"].as_array().unwrap()) {
        assert_eq!(r.name, js["name"].as_str().unwrap());
        assert!(close(r.min, num(&js["min"]), 1e-4), "{}: min {} vs {}", r.name, r.min, js["min"]);
        assert!(close(r.max, num(&js["max"]), 1e-4), "{}: max {} vs {}", r.name, r.max, js["max"]);
    }
}
