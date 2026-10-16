//! GIMME against the JS app (fixtures written by rust/tools/fixtures-analysis.mjs).
//!
//! What is compared, and why only that:
//!
//! * The threshold, the required objective, the clamped fraction and every
//!   penalty are arithmetic on the inputs and the FBA optimum: exact, or
//!   within 1e-9 where the FBA optimum enters (HiGHS and GLPK agree on it to
//!   the last bits or nearly).
//! * The inconsistency score is the GIMME LP's optimal value, which is unique:
//!   1e-6, relative.
//! * Fluxes, the achieved objective and the inactive set are compared on the
//!   toy models, where the GIMME optimum is a single point. On the core and
//!   community models the optimal face can hold several flux vectors, so the
//!   Rust solution is checked to be *an* optimum instead: it balances every
//!   metabolite, respects every bound, keeps the required objective and
//!   reaches the JS inconsistency score.
//! * Duals are not unique at these degenerate optima; their shape and the
//!   relations between them (scaled within ±1, the aggregate reduced cost
//!   taken from the active direction) are checked instead.

mod common;

use coxeter_analysis::gimme::{BASE_OBJECTIVE_FAILED, GimmeOptions, GimmeResult, run_gimme};
use coxeter_core::Model;
use coxeter_core::testkit::assert_json_eq;
use coxeter_solver::fba::FluxProblem;
use coxeter_solver::lp::LpStatus;
use common::{close, community_from_spec, fixture, model_doc, num, number_map, option_number, spec_models};
use serde_json::{Value, json};

fn problem_for(case: &Value) -> FluxProblem {
    if let Some(doc) = case.get("model") {
        return FluxProblem::new(&Model::from_value(doc).unwrap());
    }
    if let Some(file) = case.get("modelFile").and_then(Value::as_str) {
        return FluxProblem::new(&Model::from_value(&model_doc(file)).unwrap());
    }
    let models = spec_models(&case["spec"], Some(&case["gprs"]));
    FluxProblem::new(&community_from_spec(&case["spec"], &models).merged)
}

fn options_of(o: &Value) -> GimmeOptions {
    GimmeOptions {
        threshold: option_number(o.get("threshold")),
        objective_fraction: option_number(o.get("objectiveFraction")).unwrap_or(0.9),
    }
}

/// The solution is an optimum of the GIMME LP the JS solved.
fn check_optimal(name: &str, problem: &FluxProblem, r: &GimmeResult, js: &Value) {
    let s = &r.solution;
    let stoichiometry = problem.stoichiometry();
    let v: Vec<f64> = problem.rxn_ids.iter().map(|id| s.fluxes[id]).collect();
    let mut balance = vec![0.0; problem.met_ids.len()];
    for (j, x) in v.iter().enumerate() {
        for &(i, c) in stoichiometry.column(j) {
            balance[i] += c * x;
        }
        let (lb, ub) = problem.get_bounds(&problem.rxn_ids[j]).unwrap();
        assert!(*x >= lb - 1e-6 && *x <= ub + 1e-6, "{name}: {} = {x} outside [{lb}, {ub}]", problem.rxn_ids[j]);
    }
    for (i, b) in balance.iter().enumerate() {
        assert!(b.abs() < 1e-6, "{name}: {} unbalanced by {b}", problem.met_ids[i]);
    }
    let required = r.required_objective.unwrap();
    assert!(s.objective_value >= required - 1e-6 * required.abs().max(1.0), "{name}: objective {} below {required}", s.objective_value);
    let score: f64 = problem.rxn_ids.iter().zip(&v).map(|(id, x)| r.penalties[id] * x.abs()).sum();
    let js_score = num(&js["inconsistencyScore"]);
    assert!(close(score, js_score, 1e-6), "{name}: penalty of the Rust fluxes {score}, JS optimum {js_score}");
    for id in &r.inactive {
        assert!(r.penalties[id] > 0.0 && s.fluxes[id] == 0.0, "{name}: {id} is not an inactive penalised reaction");
    }
}

fn check_duals(name: &str, problem: &FluxProblem, r: &GimmeResult) {
    let s = &r.solution;
    assert_eq!(s.shadow_prices.len(), problem.met_ids.len(), "{name}: shadow prices");
    assert_eq!(s.reduced_costs.len(), problem.rxn_ids.len(), "{name}: reduced costs");
    assert_eq!(s.scaled_reduced_costs.len(), problem.rxn_ids.len(), "{name}: scaled reduced costs");
    assert_eq!(r.directional_reduced_costs.len(), problem.rxn_ids.len(), "{name}: directional reduced costs");
    assert!(r.objective_constraint_shadow_price.is_some_and(f64::is_finite), "{name}: objective-row dual");
    for value in s.scaled_reduced_costs.values() {
        assert!(value.abs() <= 1.0 + 1e-12, "{name}: scaled reduced cost {value}");
    }
    for (id, d) in &r.directional_reduced_costs {
        let v = s.fluxes[id];
        let expected = if v > 1e-9 { d.forward } else if v < -1e-9 { -d.reverse } else { 0.0 };
        assert_eq!(s.reduced_costs[id], expected, "{name}: {id} aggregate reduced cost");
    }
}

#[test]
fn gimme_matches_the_js_on_toy_core_and_community_models() {
    let fx = fixture("gimme.json");
    let cases = fx["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 13);
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let problem = problem_for(case);
        let expression = number_map(&case["expression"]);
        let result = run_gimme(&problem, &expression, &options_of(&case["options"]));
        let r = match (result, case.get("error")) {
            (Err(e), Some(msg)) => {
                assert_eq!(e.message, msg.as_str().unwrap(), "{name}");
                continue;
            }
            (Ok(r), None) => r,
            (r, e) => panic!("{name}: Rust {r:?}, JS error {e:?}"),
        };
        let js = &case["result"];
        let rust = r.to_json();
        assert_eq!(rust["status"], js["status"], "{name}: status");
        assert_eq!(rust["gimme"], json!(true), "{name}");
        if r.reason.is_some() {
            assert_eq!(js["reason"], json!(BASE_OBJECTIVE_FAILED), "{name}");
            assert_json_eq(&rust["fluxes"], &js["fluxes"], 0.0, name);
            continue;
        }
        assert_eq!(r.solution.status, LpStatus::Optimal, "{name}");
        for key in ["threshold", "objectiveFraction"] {
            assert_json_eq(&rust[key], &js[key], 0.0, &format!("{name}: {key}"));
        }
        assert_json_eq(&rust["requiredObjective"], &js["requiredObjective"], 1e-9, &format!("{name}: requiredObjective"));
        assert_json_eq(&rust["penalties"], &js["penalties"], 0.0, &format!("{name}: penalties"));
        assert_json_eq(&rust["inconsistencyScore"], &js["inconsistencyScore"], 1e-6, &format!("{name}: inconsistencyScore"));
        assert_json_eq(&rust["dualAnalysis"], &js["dualAnalysis"], 0.0, &format!("{name}: dualAnalysis"));
        assert_eq!(rust["message"], js["message"], "{name}: message");
        // The same fields, nothing more and nothing less.
        let keys = |v: &Value| v.as_object().unwrap().keys().cloned().collect::<std::collections::BTreeSet<_>>();
        assert_eq!(keys(&rust), keys(js), "{name}: fields");
        if case.get("model").is_some() {
            assert_json_eq(&rust["fluxes"], &js["fluxes"], 1e-9, &format!("{name}: fluxes"));
            assert_json_eq(&rust["objectiveValue"], &js["objectiveValue"], 1e-9, &format!("{name}: objectiveValue"));
            assert_json_eq(&rust["inactive"], &js["inactive"], 0.0, &format!("{name}: inactive"));
        }
        check_optimal(name, &problem, &r, js);
        check_duals(name, &problem, &r);
    }
}
