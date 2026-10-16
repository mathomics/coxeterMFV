//! FBA, pFBA, loopless FBA, FVA and duals against the JS app
//! (fixtures written by rust/tools/fixtures-solver.mjs).
//!
//! What is compared, and why only that:
//!
//! * Objective values, pFBA total flux and FVA ranges are unique optima, so
//!   they must agree with the JS (GLPK) to 1e-6, relative.
//! * Individual fluxes are compared only where the optimum is unique, that is
//!   where the JS FVA range at 100 % has zero width. Elsewhere HiGHS and GLPK
//!   may return different vertices of the same optimal face, and both are
//!   right.
//! * Duals are compared only where they are unique: when the HiGHS optimum is
//!   primal non-degenerate. Otherwise the test checks that the HiGHS duals are
//!   *an* optimal dual solution (reduced costs consistent, signs right, and
//!   strong duality), which is what makes them correct.
//! * Loopless results are compared where the JS proved its answer; the JS
//!   branch-and-bound gives up after 8 s on the larger ones.

mod common;

use coxeter_core::testkit::{fixture_path, read_fixture};
use coxeter_solver::fba::{FluxProblem, FvaOptions, FvaRange, LooplessOptions, PfbaOptions};
use coxeter_solver::lp::{LpStatus, SolveOptions, solve_lp};
use common::{close, community_from_spec, load_model, num};
use indexmap::IndexMap;
use serde_json::Value;

const TOL: f64 = 1e-6;

fn cases() -> Vec<(String, Value)> {
    let index = read_fixture("solver/index.json").expect("run rust/tools/fixtures-solver.mjs");
    index["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let name = c.as_str().unwrap().to_string();
            let fx = read_fixture(&format!("solver/fba-{name}.json")).unwrap();
            (name, fx)
        })
        .collect()
}

fn problem_for(fx: &Value) -> FluxProblem {
    match fx["kind"].as_str().unwrap() {
        "model" => FluxProblem::new(&load_model(fx["model"].as_str().unwrap())),
        _ => FluxProblem::new(&community_from_spec(&fx["spec"]).merged),
    }
}

fn check_ranges(name: &str, label: &str, rust: &IndexMap<String, FvaRange>, js: &Value) -> usize {
    let js = js.as_object().unwrap();
    assert_eq!(rust.len(), js.len(), "{name} {label}: number of ranges");
    let mut checked = 0;
    for (rid, pair) in js {
        let r = rust.get(rid).unwrap_or_else(|| panic!("{name} {label}: no range for {rid}"));
        let (min, max) = (num(&pair[0]), num(&pair[1]));
        assert!(close(r.min, min, TOL), "{name} {label} {rid}: min {} vs JS {min}", r.min);
        assert!(close(r.max, max, TOL), "{name} {label} {rid}: max {} vs JS {max}", r.max);
        checked += 1;
    }
    checked
}

#[test]
fn fba_pfba_and_fva_match_the_javascript() {
    let cases = cases();
    assert!(cases.len() >= 20, "expected every model and scenario, got {}", cases.len());
    let mut ranges = 0;
    let mut unique_fluxes = 0;
    for (name, fx) in &cases {
        let p = problem_for(fx);
        assert_eq!(p.rxn_ids.len() as u64, fx["reactions"].as_u64().unwrap(), "{name}: reactions");
        assert_eq!(p.met_ids.len() as u64, fx["metabolites"].as_u64().unwrap(), "{name}: metabolites");
        let js_objective: IndexMap<String, f64> =
            fx["objective"].as_object().unwrap().iter().map(|(k, v)| (k.clone(), num(v))).collect();
        assert_eq!(p.objective(), &js_objective, "{name}: objective");

        let fba = p.optimize();
        assert_eq!(fba.status.as_str(), fx["fba"]["status"], "{name}: FBA status");
        let js_opt = num(&fx["fba"]["objectiveValue"]);
        assert!(close(fba.objective_value, js_opt, TOL), "{name}: FBA {} vs JS {js_opt}", fba.objective_value);
        assert!(close(fba.objective_value, num(&fx["simplex"]["objectiveValue"]), TOL), "{name}: FBA vs JS simplex");

        let min = p.optimize_with(false);
        assert_eq!(min.status.as_str(), fx["fbaMin"]["status"], "{name}: minimisation status");
        if min.status == LpStatus::Optimal {
            assert!(close(min.objective_value, num(&fx["fbaMin"]["objectiveValue"]), TOL), "{name}: minimum");
        }

        for (label, fraction) in [("pfba", 1.0), ("pfba90", 0.9)] {
            let r = p.parsimonious(PfbaOptions { fraction, ..Default::default() });
            let js = &fx[label];
            assert_eq!(r.status.as_str(), js["status"], "{name} {label}: status");
            assert!(close(r.objective_value, num(&js["objectiveValue"]), TOL), "{name} {label}: objective {} vs {}", r.objective_value, js["objectiveValue"]);
            let total = r.total_flux.unwrap();
            assert!(close(total, num(&js["totalFlux"]), TOL), "{name} {label}: total flux {total} vs JS {}", js["totalFlux"]);
        }

        let fva = p.variability_all(&FvaOptions::default());
        ranges += check_ranges(name, "FVA 100%", &fva, &fx["fva"]);
        let fva90 = p.variability_all(&FvaOptions { fraction: 0.9, ..Default::default() });
        ranges += check_ranges(name, "FVA 90%", &fva90, &fx["fva90"]);

        // Fluxes, where the optimum pins them down.
        for (rid, pair) in fx["fva"].as_object().unwrap() {
            let (lo, hi) = (num(&pair[0]), num(&pair[1]));
            if !(hi - lo).abs().le(&1e-9) {
                continue;
            }
            let js_flux = num(&fx["fba"]["fluxes"][rid]);
            assert!(close(fba.fluxes[rid], js_flux, TOL), "{name} {rid}: unique flux {} vs JS {js_flux}", fba.fluxes[rid]);
            unique_fluxes += 1;
        }
    }
    eprintln!("compared {} cases, {ranges} FVA ranges, {unique_fluxes} unique fluxes", cases.len());
}

#[test]
fn loopless_matches_the_javascript_where_it_proved_its_answer() {
    let mut compared = 0;
    for (name, fx) in cases() {
        let p = problem_for(&fx);
        let js = &fx["loopless"];
        let r = p.loopless(&LooplessOptions { time_limit_ms: 60_000.0, ..Default::default() });
        let info = r.loopless.as_ref().unwrap();
        assert_eq!(info.binary_variables as u64, js["binaryVariables"].as_u64().unwrap(), "{name}: binaries");
        assert_eq!(info.loopless_internal_reactions as u64, js["looplessInternalReactions"].as_u64().unwrap(), "{name}: internal");
        if js["provenOptimal"] != true {
            eprintln!("{name}: JS did not prove its loopless answer ({}); HiGHS says {}", js["status"], r.status);
            continue;
        }
        assert_eq!(r.status.as_str(), js["status"], "{name}: loopless status ({})", r.message);
        assert!(info.proven_optimal, "{name}");
        if r.status == LpStatus::Optimal {
            assert!(close(r.objective_value, num(&js["objectiveValue"]), TOL), "{name}: loopless objective {} vs {}", r.objective_value, js["objectiveValue"]);
        }
        compared += 1;
    }
    assert!(compared > 10, "only {compared} loopless cases were comparable");
}

#[test]
fn duals_are_optimal_and_match_the_javascript_where_unique() {
    let mut unique = 0;
    for (name, fx) in cases() {
        let p = problem_for(&fx);
        let lp = p.lp(true);
        let sol = solve_lp(&lp, &SolveOptions::default());
        assert_eq!(sol.status, LpStatus::Optimal, "{name}");
        let x = sol.x.as_ref().unwrap();
        let y = sol.duals.as_ref().unwrap();
        let rc = sol.reduced_costs.as_ref().unwrap();
        let basis = sol.basis.as_ref().unwrap();
        // An optimal dual solution: rc = c − Aᵀy, the right signs at the bounds,
        // and cᵀx = Σ rc_j x_j (S x = 0, so yᵀ S x vanishes).
        let mut dual_objective = 0.0;
        for j in 0..lp.cols() {
            let expected = lp.cost[j] - lp.a.dot(j, y);
            if !basis.is_basic_col(j) {
                assert!((rc[j] - expected).abs() <= 1e-6 * expected.abs().max(1.0), "{name}: rc of {}", p.rxn_ids[j]);
            } else {
                assert!(expected.abs() <= 1e-6, "{name}: basic {} has reduced cost {expected}", p.rxn_ids[j]);
            }
            let at_lower = (x[j] - lp.col_lower[j]).abs() <= 1e-7;
            let at_upper = (x[j] - lp.col_upper[j]).abs() <= 1e-7;
            if !at_lower && !at_upper {
                assert!(expected.abs() <= 1e-6, "{name}: {} between bounds with rc {expected}", p.rxn_ids[j]);
            } else if at_lower && !at_upper {
                assert!(expected <= 1e-6, "{name}: {} at its lower bound with rc {expected}", p.rxn_ids[j]);
            } else if at_upper && !at_lower {
                assert!(expected >= -1e-6, "{name}: {} at its upper bound with rc {expected}", p.rxn_ids[j]);
            }
            dual_objective += expected * x[j];
        }
        assert!(close(dual_objective, sol.objective, 1e-6), "{name}: strong duality {dual_objective} vs {}", sol.objective);

        // Unique duals: no basic variable at a bound.
        let degenerate = (0..lp.cols()).any(|j| {
            basis.is_basic_col(j) && ((x[j] - lp.col_lower[j]).abs() <= 1e-9 || (x[j] - lp.col_upper[j]).abs() <= 1e-9)
        }) || (0..lp.rows()).any(|i| basis.is_basic_row(i) && lp.a.rows > 0 && lp.row_lower[i] == lp.row_upper[i]);
        if degenerate {
            continue;
        }
        let r = p.optimize();
        for (mid, v) in fx["simplex"]["shadowPrices"].as_object().unwrap() {
            assert!(close(r.shadow_prices[mid], num(v), TOL), "{name} {mid}: shadow price");
        }
        for (rid, v) in fx["simplex"]["reducedCosts"].as_object().unwrap() {
            assert!(close(r.reduced_costs[rid], num(v), TOL), "{name} {rid}: reduced cost");
        }
        assert!(close(r.reduced_cost_scale.unwrap(), num(&fx["simplex"]["reducedCostScale"]), TOL));
        unique += 1;
    }
    eprintln!("{unique} cases had unique duals to compare");
}

#[test]
fn parallel_fva_is_identical_to_a_serial_run() {
    let fx = read_fixture("solver/fba-scenario-todas-equal-growth.json").unwrap();
    let p = problem_for(&fx);
    let serial = p.variability_all(&FvaOptions { threads: Some(1), ..Default::default() });
    for threads in [2, 4, 8] {
        let parallel = p.variability_all(&FvaOptions { threads: Some(threads), ..Default::default() });
        assert_eq!(serial.len(), parallel.len());
        for (rid, a) in &serial {
            let b = parallel[rid];
            assert!(
                a.min.to_bits() == b.min.to_bits() && a.max.to_bits() == b.max.to_bits(),
                "{rid}: {a:?} serial vs {b:?} with {threads} threads"
            );
        }
        assert!(serial.keys().eq(parallel.keys()), "order");
    }
    // Reactions in a different order give the same ranges too.
    let mut reversed: Vec<String> = p.rxn_ids.clone();
    reversed.reverse();
    let backwards = p.variability(&reversed, &FvaOptions::default());
    for (rid, a) in &serial {
        assert_eq!(a.min.to_bits(), backwards[rid].min.to_bits(), "{rid}");
        assert_eq!(a.max.to_bits(), backwards[rid].max.to_bits(), "{rid}");
    }
    assert!(fixture_path("solver/index.json").exists());
}
