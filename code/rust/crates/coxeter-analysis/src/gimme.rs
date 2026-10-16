//! Port of `src/omics/gimme.js`.
//!
//! GIMME: Gene Inactivity Moderated by Metabolism and Expression.
//! Becker & Palsson (2008), adapted to Coxeter's bounded LP solver.
//!
//! 1. Maximise the required metabolic objective.
//! 2. Require a configurable fraction of that optimum.
//! 3. Minimise flux through reactions whose expression is below threshold,
//!    weighted by (threshold - expression).
//!
//! Absolute flux is linearised as v = v+ - v-.
//!
//! The JS solves the GIMME LP with its bundled simplex (`solveLP`). This port
//! solves the same LP, row for row and column for column, with HiGHS through
//! `coxeter_solver::lp`, whose duals follow the simplex.js conventions.

use coxeter_core::js;
use coxeter_solver::fba::{DualAnalysis, FluxProblem, FluxResult};
use coxeter_solver::lp::{LpProblem, LpStatus, SolveOptions, SparseColumns, solve_lp};
use indexmap::{IndexMap, IndexSet};
use serde_json::{Map, Value, json};

use crate::error::{Error, ErrorKind, Result};
use crate::jsutil::{ascending, json_number};

/// What the duals of a GIMME solution refer to.
pub const GIMME_DUAL_CONTEXT: &str = "GIMME penalty-minimization LP";
/// How to read them.
pub const GIMME_DUAL_INTERPRETATION: &str =
    "Duals and reduced costs refer to the contextual GIMME LP, not the unconstrained FBA optimum.";
/// `reason` when the first step, the plain FBA optimum, has no solution.
pub const BASE_OBJECTIVE_FAILED: &str = "base_objective_failed";

/// Options of [`run_gimme`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GimmeOptions {
    /// Expression below this is penalised. `None` (or a value that is not a
    /// finite number) takes the 25th percentile of the mapped expression.
    pub threshold: Option<f64>,
    /// The fraction of the optimum the GIMME solution must keep (0.9),
    /// clamped to 0–1, and NaN counts as 0.
    pub objective_fraction: f64,
}

impl Default for GimmeOptions {
    fn default() -> Self {
        GimmeOptions { threshold: None, objective_fraction: 0.9 }
    }
}

/// The reduced costs of the two halves of a split reaction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DirectionalReducedCost {
    pub forward: f64,
    pub reverse: f64,
}

/// The result of [`run_gimme`].
///
/// `solution` has the shape of every other solve (status, objective value,
/// fluxes, shadow prices, reduced costs), so it can be shown on the map as
/// any FBA result is. Its duals belong to the GIMME LP: see
/// [`GIMME_DUAL_INTERPRETATION`].
#[derive(Debug, Clone)]
pub struct GimmeResult {
    pub solution: FluxResult,
    /// [`BASE_OBJECTIVE_FAILED`] when the plain FBA optimum could not be
    /// found. `solution` is then that failed FBA result and nothing else is set.
    pub reason: Option<&'static str>,
    /// The expression threshold used (given or automatic).
    pub threshold: Option<f64>,
    /// The fraction actually required, after clamping. Only set when the
    /// GIMME LP solved, as in the JS.
    pub objective_fraction: Option<f64>,
    /// `base optimum × fraction`: the least objective the solution keeps.
    pub required_objective: Option<f64>,
    /// The GIMME objective: Σ penalty × |flux| at the optimum.
    pub inconsistency_score: Option<f64>,
    /// Reaction id → `threshold - expression` when below threshold, else 0.
    pub penalties: IndexMap<String, f64>,
    /// Penalised reactions that carry no flux.
    pub inactive: IndexSet<String>,
    /// The reduced costs of `v+` and `v-` for every reaction.
    pub directional_reduced_costs: IndexMap<String, DirectionalReducedCost>,
    /// The dual of the minimum-objective row, not a metabolite shadow price.
    pub objective_constraint_shadow_price: Option<f64>,
}

impl GimmeResult {
    /// The JS result object, Maps written as objects and the `inactive` Set
    /// as an array.
    pub fn to_json(&self) -> Value {
        let s = &self.solution;
        let mut out = Map::new();
        if self.reason.is_some() {
            // `{...base, gimme: true, reason}`
            out.extend(flux_result_json(s));
            out.insert("gimme".into(), json!(true));
            out.insert("reason".into(), json!(self.reason));
            return Value::Object(out);
        }
        out.insert("status".into(), json!(s.status.as_str()));
        out.insert("objectiveValue".into(), json_number(s.objective_value));
        out.insert("fluxes".into(), number_map(&s.fluxes));
        if s.status != LpStatus::Optimal {
            out.insert("gimme".into(), json!(true));
            out.insert("threshold".into(), opt_number(self.threshold));
            out.insert("requiredObjective".into(), opt_number(self.required_objective));
            out.insert("penalties".into(), number_map(&self.penalties));
            out.insert("message".into(), json!(s.message));
            return Value::Object(out);
        }
        out.insert("shadowPrices".into(), number_map(&s.shadow_prices));
        out.insert("reducedCosts".into(), number_map(&s.reduced_costs));
        out.insert("scaledReducedCosts".into(), number_map(&s.scaled_reduced_costs));
        out.insert("reducedCostScale".into(), opt_number(s.reduced_cost_scale));
        out.insert(
            "directionalReducedCosts".into(),
            Value::Object(
                self.directional_reduced_costs
                    .iter()
                    .map(|(k, d)| (k.clone(), json!({"forward": json_number(d.forward), "reverse": json_number(d.reverse)})))
                    .collect(),
            ),
        );
        out.insert("objectiveConstraintShadowPrice".into(), opt_number(self.objective_constraint_shadow_price));
        out.insert("iterations".into(), json!(s.iterations));
        out.insert(
            "dualAnalysis".into(),
            json!({"available": true, "context": GIMME_DUAL_CONTEXT, "interpretation": GIMME_DUAL_INTERPRETATION}),
        );
        out.insert("gimme".into(), json!(true));
        out.insert("threshold".into(), opt_number(self.threshold));
        out.insert("objectiveFraction".into(), opt_number(self.objective_fraction));
        out.insert("requiredObjective".into(), opt_number(self.required_objective));
        out.insert("inconsistencyScore".into(), opt_number(self.inconsistency_score));
        out.insert("penalties".into(), number_map(&self.penalties));
        out.insert("inactive".into(), json!(self.inactive.iter().collect::<Vec<_>>()));
        out.insert("message".into(), json!(""));
        Value::Object(out)
    }
}

fn number_map(m: &IndexMap<String, f64>) -> Value {
    Value::Object(m.iter().map(|(k, v)| (k.clone(), json_number(*v))).collect())
}

fn opt_number(v: Option<f64>) -> Value {
    v.map_or(Value::Null, json_number)
}

/// The fields of a JS FBA result (`_wrap` in fba.js).
fn flux_result_json(s: &FluxResult) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("status".into(), json!(s.status.as_str()));
    out.insert("objectiveValue".into(), json_number(s.objective_value));
    out.insert("fluxes".into(), number_map(&s.fluxes));
    out.insert("shadowPrices".into(), number_map(&s.shadow_prices));
    out.insert("reducedCosts".into(), number_map(&s.reduced_costs));
    out.insert("scaledReducedCosts".into(), number_map(&s.scaled_reduced_costs));
    out.insert("reducedCostScale".into(), opt_number(s.reduced_cost_scale));
    let d = &s.dual_analysis;
    let mut dual = Map::new();
    dual.insert("available".into(), json!(d.available));
    if let Some(v) = &d.source {
        dual.insert("source".into(), json!(v));
    }
    if let Some(v) = &d.interpretation {
        dual.insert("interpretation".into(), json!(v));
    }
    out.insert("dualAnalysis".into(), Value::Object(dual));
    out.insert("iterations".into(), json!(s.iterations));
    out.insert("message".into(), json!(s.message));
    out
}

/// Run GIMME on a flux problem with reaction expression scores (reaction id →
/// expression, as [`crate::overlay::reaction_omics_scores`] gives them).
///
/// Fails only when no reaction has a finite expression value, and only once
/// the base optimum has been found: a model with no optimum is reported as a
/// result with [`BASE_OBJECTIVE_FAILED`], as the JS reports it.
pub fn run_gimme(problem: &FluxProblem, expression_scores: &IndexMap<String, f64>, options: &GimmeOptions) -> Result<GimmeResult> {
    let base = problem.optimize();
    if base.status != LpStatus::Optimal {
        return Ok(GimmeResult {
            solution: base,
            reason: Some(BASE_OBJECTIVE_FAILED),
            threshold: None,
            objective_fraction: None,
            required_objective: None,
            inconsistency_score: None,
            penalties: IndexMap::new(),
            inactive: IndexSet::new(),
            directional_reduced_costs: IndexMap::new(),
            objective_constraint_shadow_price: None,
        });
    }
    let measured: Vec<f64> = expression_scores.values().copied().filter(|v| v.is_finite()).collect();
    if measured.is_empty() {
        return Err(Error::new(ErrorKind::Gimme, "GIMME needs expression mapped to at least one reaction."));
    }
    // Number(null) is 0 and Number.isFinite(0) is true, so testing the coerced
    // value alone would turn every automatic threshold into 0, and with a cutoff
    // of 0 no reaction is ever below it, so GIMME would do nothing. The absence
    // of a threshold has to be checked before coercing it.
    let cutoff = match options.threshold {
        Some(t) if t.is_finite() => t,
        _ => quantile(&measured, 0.25),
    };
    let fraction = {
        let f = options.objective_fraction;
        // `Number(objectiveFraction) || 0`
        let f = if f.is_nan() { 0.0 } else { f };
        js::min(1.0, js::max(0.0, f))
    };

    let (n, m) = (problem.rxn_ids.len(), problem.met_ids.len());
    let stoichiometry = problem.stoichiometry();
    let objective = problem.objective_vector();
    let mut a = SparseColumns::new(m + 1, 2 * n + 1);
    let mut b = vec![0.0; m + 1];
    let mut lb = vec![0.0; 2 * n + 1];
    let mut ub = vec![0.0; 2 * n + 1];
    let mut c = vec![0.0; 2 * n + 1];
    let mut penalties: IndexMap<String, f64> = IndexMap::new();

    for j in 0..n {
        for &(row, value) in stoichiometry.column(j) {
            a.add(row, j, value);
            a.add(row, n + j, -value);
        }
        let rid = &problem.rxn_ids[j];
        let (plb, pub_) = problem.get_bounds(rid).expect("every listed reaction has bounds");
        lb[j] = js::max(0.0, plb);
        ub[j] = js::max(0.0, pub_);
        lb[n + j] = js::max(0.0, -pub_);
        ub[n + j] = js::max(0.0, -plb);
        let penalty = match expression_scores.get(rid) {
            Some(&expr) if expr.is_finite() && expr < cutoff => cutoff - expr,
            _ => 0.0,
        };
        penalties.insert(rid.clone(), penalty);
        c[j] = penalty;
        c[n + j] = penalty;
        if objective[j] != 0.0 {
            a.add(m, j, objective[j]);
            a.add(m, n + j, -objective[j]);
        }
    }
    // objective - slack = required objective, slack >= 0 => objective >= required
    let slack = 2 * n;
    a.add(m, slack, -1.0);
    lb[slack] = 0.0;
    ub[slack] = f64::INFINITY;
    let required = base.objective_value * fraction;
    b[m] = required;
    let res = solve_lp(&LpProblem::equality(a, &b, c, lb, ub, false), &SolveOptions::default());

    if res.status != LpStatus::Optimal {
        return Ok(GimmeResult {
            solution: FluxResult {
                status: res.status,
                objective_value: f64::NAN,
                fluxes: IndexMap::new(),
                shadow_prices: IndexMap::new(),
                reduced_costs: IndexMap::new(),
                scaled_reduced_costs: IndexMap::new(),
                reduced_cost_scale: None,
                dual_analysis: DualAnalysis::default(),
                iterations: res.iterations,
                message: res.message,
                total_flux: None,
                loopless: None,
            },
            reason: None,
            threshold: Some(cutoff),
            objective_fraction: None,
            required_objective: Some(required),
            inconsistency_score: None,
            penalties,
            inactive: IndexSet::new(),
            directional_reduced_costs: IndexMap::new(),
            objective_constraint_shadow_price: None,
        });
    }

    let x = res.x.as_deref().unwrap_or(&[]);
    let value_at = |k: usize| x.get(k).copied().unwrap_or(0.0);
    let mut fluxes: IndexMap<String, f64> = IndexMap::new();
    let mut achieved = 0.0;
    for (j, rid) in problem.rxn_ids.iter().enumerate() {
        let v = value_at(j) - value_at(n + j);
        fluxes.insert(rid.clone(), if v.abs() < 1e-9 { 0.0 } else { v });
        achieved += objective[j] * v;
    }
    let inactive: IndexSet<String> = penalties
        .iter()
        .filter(|(rid, p)| **p > 0.0 && fluxes.get(*rid).copied().unwrap_or(0.0).abs() < 1e-9)
        .map(|(rid, _)| rid.clone())
        .collect();

    // The first m duals belong to the original metabolite mass balances. The
    // final row is GIMME's minimum-objective constraint and is reported
    // separately: it is not a metabolite shadow price.
    let duals = res.duals.as_deref();
    let shadow_prices: IndexMap<String, f64> = problem
        .met_ids
        .iter()
        .enumerate()
        .map(|(i, mid)| (mid.clone(), duals.and_then(|d| d.get(i).copied()).unwrap_or(0.0)))
        .collect();
    let objective_constraint_shadow_price = duals.and_then(|d| d.get(m).copied());

    // GIMME solves a split-variable LP (v = v+ - v-). Preserve both directional
    // reduced costs for auditability and expose one reaction-level value for the
    // currently active direction. At zero flux there is no unique signed
    // reaction-level derivative, so the aggregate value is deliberately 0 and
    // callers can inspect the directional pair instead.
    let rc = res.reduced_costs.as_deref();
    let rc_at = |k: usize| rc.and_then(|r| r.get(k).copied()).unwrap_or(0.0);
    let mut directional_reduced_costs = IndexMap::new();
    let mut reduced_costs: IndexMap<String, f64> = IndexMap::new();
    for (j, rid) in problem.rxn_ids.iter().enumerate() {
        let (forward, reverse) = (rc_at(j), rc_at(n + j));
        directional_reduced_costs.insert(rid.clone(), DirectionalReducedCost { forward, reverse });
        let v = fluxes.get(rid).copied().unwrap_or(0.0);
        reduced_costs.insert(rid.clone(), if v > 1e-9 { forward } else if v < -1e-9 { -reverse } else { 0.0 });
    }
    let reduced_cost_scale = js::max_n(std::iter::once(0.0).chain(reduced_costs.values().map(|v| v.abs())));
    let scaled_reduced_costs: IndexMap<String, f64> = reduced_costs
        .iter()
        .map(|(rid, v)| (rid.clone(), if reduced_cost_scale > 0.0 { v / reduced_cost_scale } else { 0.0 }))
        .collect();

    Ok(GimmeResult {
        solution: FluxResult {
            status: LpStatus::Optimal,
            objective_value: achieved,
            fluxes,
            shadow_prices,
            reduced_costs,
            scaled_reduced_costs,
            reduced_cost_scale: Some(reduced_cost_scale),
            dual_analysis: DualAnalysis {
                available: true,
                source: Some(GIMME_DUAL_CONTEXT.into()),
                interpretation: Some(GIMME_DUAL_INTERPRETATION.into()),
                reason: None,
            },
            iterations: res.iterations,
            message: String::new(),
            total_flux: None,
            loopless: None,
        },
        reason: None,
        threshold: Some(cutoff),
        objective_fraction: Some(fraction),
        required_objective: Some(required),
        inconsistency_score: Some(res.objective),
        penalties,
        inactive,
        directional_reduced_costs,
        objective_constraint_shadow_price,
    })
}

/// The `q` quantile with linear interpolation, written as gimme.js writes it
/// (`x[l]*(h-p) + x[h]*(p-l)`, which rounds differently from overlay.js).
fn quantile(values: &[f64], q: f64) -> f64 {
    let mut x = values.to_vec();
    x.sort_by(ascending);
    let p = (x.len() as f64 - 1.0) * q;
    let (l, h) = (p.floor(), p.ceil());
    if l == h { x[l as usize] } else { x[l as usize] * (h - p) + x[h as usize] * (p - l) }
}

#[cfg(test)]
mod tests {
    use super::quantile;

    #[test]
    fn quantile_interpolates_as_gimme_js_does() {
        assert_eq!(quantile(&[8.0, 4.0, 0.0], 0.25), 2.0);
        assert_eq!(quantile(&[3.0], 0.25), 3.0);
        assert_eq!(quantile(&[1.0, 2.0, 3.0, 4.0, 5.0], 0.25), 2.0);
        assert_eq!(quantile(&[0.1, 0.7], 0.25), 0.1 * 0.75 + 0.7 * 0.25);
    }
}
