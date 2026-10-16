mod common;
use coxeter_core::Model;
use coxeter_core::testkit::repo_root;
use coxeter_solver::lp::*;
use coxeter_solver::steadycom::*;
use coxeter_solver::steadycom_runtime::*;
use common::read_json;

fn spec(fiber: Option<Fiber>) -> SteadyComSpec {
    let dir = repo_root().join("models/published/steadycom-gut-nine");
    let manifest = read_json(&dir.join("manifest.json"));
    let members: Vec<MemberModel> = manifest["members"].as_array().unwrap().iter().map(|m| {
        MemberModel::new(m["id"].as_str().unwrap(), Model::from_value(&read_json(&dir.join(m["model"].as_str().unwrap()))).unwrap())
    }).collect();
    let bounds = std::fs::read_to_string(dir.join("bounds.csv")).unwrap();
    joint_steady_com_spec(&members, &bounds, JointSpecOptions { fiber, ..Default::default() }).unwrap()
}

fn solve(name: &str, lp: &LpProblem, basis: Option<&Basis>, log: bool, opts: &[(&str, OptionValue)]) -> LpSolution {
    let mut h = Highs::new();
    h.set_time_limit(Some(60.0));
    if log { h.set_option("output_flag", OptionValue::Bool(true)); h.set_option("log_to_console", OptionValue::Bool(true)); }
    for (k, v) in opts { assert!(h.set_option(k, *v), "{k}"); }
    h.pass_problem(lp).unwrap();
    if let Some(b) = basis { h.set_basis(b).unwrap(); }
    let t = std::time::Instant::now();
    let r = h.run();
    eprintln!("== {name}: {} ({}) iters={} obj={:.10} in {:.2}s", r.status, r.model_status.name(), r.iterations, r.objective, t.elapsed().as_secs_f64());
    r
}

#[test]
#[ignore]
fn probe() {
    let spec = spec(None);
    let warm: &[(&str, OptionValue)] = &[("simplex_dual_edge_weight_strategy", OptionValue::Int(1)), ("simplex_iteration_limit", OptionValue::Int(20000))];
    let warm_primal: &[(&str, OptionValue)] = &[("simplex_strategy", OptionValue::Int(4)), ("simplex_iteration_limit", OptionValue::Int(20000))];
    let edge = 0.07969360351562502f64;
    // E1: the plain fixed LP, cold, away from the edge on both sides.
    for mu in [0.05, 0.09, 0.1] {
        solve(&format!("E1 fixed-eq cold {mu}"), &debug_lp_at(&spec, mu, None, Some(1000.0)).unwrap(), None, false, &[]);
    }
    // E2: two stages: relaxation (cap 2, x1000) cold, then the fixed LP from its basis.
    let mut last: Option<Basis> = None;
    for mu in [0.0, 0.05, 0.079, edge - 1e-5, edge, 0.0798, 0.1] {
        let relax = solve(&format!("E2 relax cold {mu}"), &debug_lp_at(&spec, mu, Some(2.0), Some(1000.0)).unwrap(), None, false, &[]);
        let fixed = solve(&format!("E2   fixed warm {mu}"), &debug_lp_at(&spec, mu, None, Some(1000.0)).unwrap(), relax.basis.as_ref(), false, warm);
        if mu == edge - 1e-5 { last = fixed.basis.clone(); }
    }
    // E3: objective switches on the fixed LP at the verified maximum, from its feasible basis.
    let mu = edge - 1e-5;
    let base = debug_lp_at(&spec, mu, None, Some(1000.0)).unwrap();
    let n = base.cols();
    for (k, name) in ["Bt", "Fp", "St", "Ef"].iter().enumerate() {
        // The biomass variables X are the last 9 columns... use column index of X_k = n - 9 + position
        let pos = ["Bt","Fp","Kp","St","Ba","Ec","Ef","Er","Lc"].iter().position(|s| s == name).unwrap();
        for maximize in [true, false] {
            let mut lp = base.clone();
            lp.cost = vec![0.0; n];
            lp.cost[n - 9 + pos] = 1.0;
            lp.maximize = maximize;
            let a = solve(&format!("E3 X_{name} max={maximize} warm dual"), &lp, last.as_ref(), false, warm);
            let b = solve(&format!("E3 X_{name} max={maximize} warm primal"), &lp, last.as_ref(), false, warm_primal);
            if k == 0 && maximize {
                let c = solve(&format!("E3 X_{name} max={maximize} cold"), &lp, None, false, &[]);
                eprintln!("   values {} {} {}", a.objective, b.objective, c.objective);
            }
        }
    }
}
