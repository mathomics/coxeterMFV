//! Port of `src/core/reproducibility.js`: scientific provenance and the
//! reproducibility bundle.
//!
//! Pure data layer: deterministic canonical JSON and SHA-256 digests, so a
//! bundle written by either build verifies in the other.
//!
//! The JS functions that take the `Store` take a [`ReproStore`] here: a plain
//! struct holding exactly the fields they read, which the GUI fills from its
//! own state.

use std::fmt::Write as _;

use indexmap::IndexMap;
use serde::Serialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use coxeter_core::js;
use coxeter_core::model::Model;

use crate::error::{Error, ErrorKind, Result};
use crate::jscompat::{js_order, object_entries};

pub const REPRO_FORMAT: &str = "CoxeterReproducibilityBundle";
pub const REPRO_VERSION: u32 = 2;
pub const COXETER_VERSION: &str = "3.4";

/// Which build produced an analysis: `globalThis.__COXETER_BUILD__` in the JS,
/// compile-time environment variables here (`COXETER_BUILD_VERSION`,
/// `COXETER_GIT_COMMIT`, `COXETER_GIT_BRANCH`, `COXETER_BUILD_TIME`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SoftwareIdentity {
    pub version: String,
    #[serde(rename = "gitCommit")]
    pub git_commit: Option<String>,
    #[serde(rename = "gitBranch")]
    pub git_branch: Option<String>,
    #[serde(rename = "buildTime")]
    pub build_time: Option<String>,
}

impl SoftwareIdentity {
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("plain data serialises")
    }
}

/// The identity of this build, from the compile-time variables listed on
/// [`SoftwareIdentity`]. The version falls back to [`COXETER_VERSION`].
pub fn software_identity() -> SoftwareIdentity {
    SoftwareIdentity {
        version: option_env!("COXETER_BUILD_VERSION").unwrap_or(COXETER_VERSION).to_string(),
        git_commit: option_env!("COXETER_GIT_COMMIT").map(str::to_string),
        git_branch: option_env!("COXETER_GIT_BRANCH").map(str::to_string),
        build_time: option_env!("COXETER_BUILD_TIME").map(str::to_string),
    }
}

// Canonical digests

/// `canonicalJSON(value)`: `JSON.stringify` of the value with every object's
/// keys sorted. JavaScript objects being what they are, that means
/// integer-like keys first in numeric order, then the others in code-unit
/// order. Numbers print as JavaScript prints them.
pub fn canonical_json(value: &Value) -> String {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out
}

fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            let x = n.as_f64().unwrap_or(f64::NAN);
            if x.is_finite() { out.push_str(&js::number_to_string(x)) } else { out.push_str("null") }
        }
        Value::String(s) => out.push_str(&serde_json::to_string(s).expect("strings serialise")),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by(|a, b| js::code_unit_compare(a, b));
            // Object.fromEntries puts array-index keys first, ascending.
            keys.sort_by(|a, b| match (js::is_array_index(a), js::is_array_index(b)) {
                (true, true) => a.parse::<u64>().unwrap_or(0).cmp(&b.parse::<u64>().unwrap_or(0)),
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                (false, false) => std::cmp::Ordering::Equal,
            });
            out.push('{');
            for (i, key) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(key).expect("strings serialise"));
                out.push(':');
                write_canonical(&map[*key], out);
            }
            out.push('}');
        }
    }
}

/// Hex SHA-256 of some bytes.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for b in digest.iter() {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// `sha256(value)`: a string is hashed as its UTF-8 text, anything else as its
/// canonical JSON.
pub fn sha256(value: &Value) -> String {
    match value {
        Value::String(s) => sha256_hex(s.as_bytes()),
        other => sha256_hex(canonical_json(other).as_bytes()),
    }
}

// Store

/// The omics state the bundle records (`store.omics`).
#[derive(Debug, Clone, Default)]
pub struct OmicsState {
    /// `store.omics.dataset`. `None` means no dataset (and no omics payload).
    pub dataset: Option<OmicsDataset>,
    pub enabled: Value,
    pub mode: Value,
    pub condition_a: Value,
    pub condition_b: Value,
    pub metric: Value,
    pub pseudocount: Value,
    pub gimme_enabled: bool,
    pub gimme_threshold: Value,
    pub gimme_objective_fraction: Value,
    /// `{threshold, inconsistencyScore}` of the last GIMME run.
    pub gimme_result: Option<Value>,
    /// The gene-mapping report (`store.omics.report`).
    pub report: Option<Value>,
}

/// `store.omics.dataset`.
#[derive(Debug, Clone, Default)]
pub struct OmicsDataset {
    pub source: Value,
    pub data_type: Value,
    pub conditions: Value,
    /// `dataset.values`, a Map gene → values.
    pub values: Vec<(String, Value)>,
}

/// `store.solution`: the last numerical result.
#[derive(Debug, Clone, Default)]
pub struct SolutionState {
    pub status: Value,
    pub objective_value: Option<f64>,
    pub iterations: Option<Value>,
    pub fluxes: Vec<(String, f64)>,
    pub reduced_costs: Vec<(String, f64)>,
    pub scaled_reduced_costs: Vec<(String, f64)>,
    pub reduced_cost_scale: Option<f64>,
    pub shadow_prices: Vec<(String, f64)>,
    pub dual_analysis: Option<Value>,
    /// `solution.zuniga`, archived as `caseResult` when present.
    pub zuniga: Option<Value>,
}

/// The Zuñiga published-case state (only when `store.isZunigaStudy`).
#[derive(Debug, Clone, Default)]
pub struct ZunigaStudyState {
    /// `store.zunigaState()`.
    pub state: Value,
    pub manifest: Value,
    pub source_models: Value,
    /// `store.community.merged.toJSON()`.
    pub active_model: Value,
}

/// What `createReproducibilityBundle(store)` and `analysisProvenance(store)`
/// read from the JS `Store`, field for field.
#[derive(Debug, Clone, Default)]
pub struct ReproStore<'a> {
    /// `store.species`: `{id, file, abundance, enabled, …}` per member.
    pub species: Vec<Value>,
    /// `store.models`: file → model. A species whose file is absent is skipped.
    pub models: IndexMap<String, &'a Model>,
    /// `store.scenario` (`{id, name, …}`), if any.
    pub scenario: Option<Value>,
    pub objective: Value,
    pub product_target: Option<Value>,
    /// `store.customObjective` (`{sense, coefficients}`), written only when set.
    pub custom_objective: Option<Value>,
    /// `store.medium`: base metabolite → uptake limit.
    pub medium: Map<String, Value>,
    /// `store.overrides`: merged reaction id → `{lower, upper}`.
    pub overrides: Vec<(String, Value)>,
    pub parsimonious: bool,
    pub pfba_biomass_percent: Value,
    pub loopless: bool,
    pub fva_enabled: bool,
    /// `store.fvaRanges`: reaction id → `{min, max}`.
    pub fva_ranges: Vec<(String, Value)>,
    /// `store.growth`: species id → growth rate.
    pub growth: Vec<(String, Value)>,
    /// `store.map.toJSON()`.
    pub map: Option<Value>,
    pub omics: Option<OmicsState>,
    pub solution: Option<SolutionState>,
    /// Set when the store shows the Zuñiga published case.
    pub zuniga: Option<ZunigaStudyState>,
}

/// When and where a bundle is made (`new Date()`, `navigator.userAgent`).
#[derive(Debug, Clone)]
pub struct BundleContext {
    /// ISO 8601 UTC with milliseconds, as `Date#toISOString` writes it.
    pub created_at: String,
    pub user_agent: Option<String>,
    pub software: SoftwareIdentity,
}

impl BundleContext {
    /// Now, this build, no browser.
    pub fn now() -> BundleContext {
        BundleContext {
            created_at: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
            user_agent: None,
            software: software_identity(),
        }
    }
}

fn finite(v: Option<f64>) -> Value {
    match v {
        Some(x) if x.is_finite() => json!(x),
        _ => Value::Null,
    }
}

fn entries_object(entries: &[(String, Value)]) -> Value {
    let map: Map<String, Value> = entries.iter().cloned().collect();
    Value::Object(js_order(map))
}

fn finite_object(entries: &[(String, f64)]) -> Value {
    let map: Map<String, Value> = entries.iter().map(|(k, v)| (k.clone(), finite(Some(*v)))).collect();
    Value::Object(js_order(map))
}

fn or_null(v: &Option<Value>) -> Value {
    v.clone().unwrap_or(Value::Null)
}

/// `analysisProvenance(store)`: the method, solver and settings of the analysis.
pub fn analysis_provenance(store: &ReproStore) -> Value {
    if let Some(z) = &store.zuniga {
        return json!({
            "method": "Stationary source-model FBA",
            "solver": "GLPK",
            "objective": "authors’ original objective",
            "study": z.state,
            "parsimonious": false,
            "loopless": false,
            "fvaEnabled": false,
            "sourceConstraints": "Exact author bounds and stoichiometry; documented Ec PPNDH knockout only.",
            "limitations": "Stationary FBA comparison; not reproduction of dynamic culture trajectories or experimental growth measurements.",
        });
    }
    let gimme_enabled = store.omics.as_ref().is_some_and(|o| o.gimme_enabled);
    let method = if gimme_enabled {
        "GIMME"
    } else if store.loopless {
        "loopless FBA"
    } else if store.parsimonious {
        "pFBA"
    } else {
        "FBA"
    };
    let gimme = match &store.omics {
        Some(o) if o.gimme_enabled => {
            let result = o.gimme_result.as_ref().filter(|r| !r.is_null());
            let threshold = result.and_then(|r| r.get("threshold")).filter(|v| !v.is_null()).cloned().unwrap_or_else(|| o.gimme_threshold.clone());
            let score = result.and_then(|r| r.get("inconsistencyScore")).filter(|v| !v.is_null()).cloned().unwrap_or(Value::Null);
            json!({
                "condition": o.condition_a,
                "threshold": threshold,
                "objectiveFraction": o.gimme_objective_fraction,
                "inconsistencyScore": score,
            })
        }
        _ => Value::Null,
    };
    json!({
        "method": method,
        "solver": "Coxeter bounded-variable revised simplex / MILP",
        "objective": store.objective,
        "productTarget": or_null(&store.product_target),
        "parsimonious": store.parsimonious,
        "pfbaBiomassPercent": store.pfba_biomass_percent,
        "loopless": store.loopless,
        "fvaEnabled": store.fva_enabled,
        "gimme": gimme,
    })
}

fn solution_payload(store: &ReproStore) -> Value {
    let Some(s) = &store.solution else { return Value::Null };
    let mut out = Map::new();
    out.insert("status".into(), s.status.clone());
    out.insert("objectiveValue".into(), finite(s.objective_value));
    out.insert("iterations".into(), s.iterations.clone().filter(|v| !v.is_null()).unwrap_or(Value::Null));
    out.insert("fluxes".into(), finite_object(&s.fluxes));
    out.insert("reducedCosts".into(), finite_object(&s.reduced_costs));
    out.insert("scaledReducedCosts".into(), finite_object(&s.scaled_reduced_costs));
    out.insert("reducedCostScale".into(), finite(s.reduced_cost_scale));
    out.insert("shadowPrices".into(), finite_object(&s.shadow_prices));
    out.insert("dualAnalysis".into(), s.dual_analysis.clone().filter(|v| !v.is_null()).unwrap_or(Value::Null));
    if let Some(z) = s.zuniga.as_ref().filter(|z| js::truthy(z)) {
        out.insert("caseResult".into(), z.clone());
    }
    out.insert("fva".into(), if store.fva_enabled { entries_object(&store.fva_ranges) } else { Value::Null });
    out.insert("growth".into(), entries_object(&store.growth));
    Value::Object(out)
}

fn omics_payload(omics: &Option<OmicsState>) -> Value {
    let Some(o) = omics else { return Value::Null };
    let Some(d) = &o.dataset else { return Value::Null };
    json!({
        "source": d.source,
        "dataType": d.data_type,
        "conditions": d.conditions,
        "values": entries_object(&d.values),
        "settings": {
            "enabled": o.enabled,
            "mode": o.mode,
            "conditionA": o.condition_a,
            "conditionB": o.condition_b,
            "metric": o.metric,
            "pseudocount": o.pseudocount,
            "gimmeEnabled": o.gimme_enabled,
            "gimmeThreshold": o.gimme_threshold,
            "gimmeObjectiveFraction": o.gimme_objective_fraction,
        },
        "mappingReport": or_null(&o.report),
    })
}

/// `createReproducibilityBundle(store)`, made now by this build.
pub fn create_reproducibility_bundle(store: &ReproStore) -> Value {
    create_reproducibility_bundle_with(store, &BundleContext::now())
}

/// The bundle with an explicit creation time and build (for tests and for
/// re-creating a bundle exactly).
pub fn create_reproducibility_bundle_with(store: &ReproStore, ctx: &BundleContext) -> Value {
    let mut models = Map::new();
    let mut model_hashes = Map::new();
    for sp in &store.species {
        let file = sp.get("file").map(js::to_string).unwrap_or_else(|| "undefined".into());
        let Some(model) = store.models.get(&file) else { continue };
        let id = sp.get("id").map(js::to_string).unwrap_or_else(|| "undefined".into());
        let doc = model.to_json();
        model_hashes.insert(id.clone(), json!(sha256(&doc)));
        models.insert(id, doc);
    }
    let scenario_field = |key: &str| store.scenario.as_ref().and_then(|s| s.get(key)).filter(|v| !v.is_null()).cloned().unwrap_or(Value::Null);
    let mut payload = Map::new();
    payload.insert("format".into(), json!(REPRO_FORMAT));
    payload.insert("version".into(), json!(REPRO_VERSION));
    payload.insert(
        "metadata".into(),
        json!({
            "createdAt": ctx.created_at,
            "generator": "Coxeter Multicell Flux Visualizer",
            "software": ctx.software.to_json(),
            "coxeterVersion": ctx.software.version,
            "userAgent": ctx.user_agent,
        }),
    );
    payload.insert("models".into(), Value::Object(js_order(models)));
    payload.insert("modelHashes".into(), Value::Object(js_order(model_hashes)));
    let mut community = json!({
        "scenario": scenario_field("id"),
        "name": scenario_field("name"),
        "species": store.species,
        "objective": store.objective,
        "productTarget": or_null(&store.product_target),
    });
    if let (Some(custom), Some(c)) = (&store.custom_objective, community.as_object_mut()) {
        c.insert("customObjective".into(), custom.clone());
    }
    payload.insert("community".into(), community);
    payload.insert(
        "constraints".into(),
        json!({
            "medium": Value::Object(js_order(store.medium.clone())),
            "reactionOverrides": entries_object(&store.overrides),
        }),
    );
    payload.insert("analysis".into(), analysis_provenance(store));
    payload.insert("omics".into(), omics_payload(&store.omics));
    payload.insert("map".into(), store.map.clone().unwrap_or(Value::Null));
    payload.insert("solution".into(), solution_payload(store));
    if let Some(z) = &store.zuniga {
        let mut study = match &z.state {
            Value::Object(m) => m.clone(),
            _ => Map::new(),
        };
        study.insert("manifest".into(), z.manifest.clone());
        study.insert("sourceModels".into(), z.source_models.clone());
        study.insert("activeModel".into(), z.active_model.clone());
        study.insert(
            "redistribution".into(),
            json!("Personal reproducibility export. Original models were downloaded privately; review their authors’ terms before sharing."),
        );
        payload.insert("study".into(), Value::Object(study));
    }
    payload.insert("replay".into(), json!({"tolerances": {"absolute": 1e-7, "relative": 1e-6}}));
    let hash = sha256(&Value::Object(payload.clone()));
    payload.insert("integrity".into(), json!({"algorithm": "SHA-256", "payloadHash": hash}));
    Value::Object(payload)
}

/// What [`verify_reproducibility_bundle`] found.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BundleCheck {
    pub valid: bool,
    #[serde(rename = "payloadValid")]
    pub payload_valid: bool,
    /// Species id → whether its embedded model still matches its hash.
    #[serde(rename = "modelChecks")]
    pub model_checks: IndexMap<String, bool>,
    #[serde(rename = "payloadHash")]
    pub payload_hash: String,
}

/// Check a bundle's model hashes and its payload hash.
pub fn verify_reproducibility_bundle(bundle: &Value) -> Result<BundleCheck> {
    let refuse = |m: String| Error::new(ErrorKind::Reproducibility, m);
    if bundle.get("format").and_then(Value::as_str) != Some(REPRO_FORMAT) {
        return Err(refuse("Not a Coxeter Reproducibility Bundle.".into()));
    }
    if let Some(version) = bundle.get("version") {
        if js::to_number(version) > REPRO_VERSION as f64 {
            return Err(refuse(format!("Bundle v{} is not supported.", js::to_string(version))));
        }
    }
    let mut model_checks = IndexMap::new();
    let hashes = bundle.get("modelHashes");
    for (id, doc) in object_entries(bundle.get("models").unwrap_or(&Value::Null)) {
        let expected = hashes.and_then(|h| h.get(&id)).and_then(Value::as_str);
        let ok = expected == Some(sha256(&doc).as_str());
        model_checks.insert(id, ok);
    }
    let mut copy = bundle.as_object().cloned().unwrap_or_default();
    copy.remove("integrity");
    let payload_hash = sha256(&Value::Object(copy));
    let recorded = bundle.get("integrity").and_then(|i| i.get("payloadHash")).and_then(Value::as_str);
    let payload_valid = recorded == Some(payload_hash.as_str());
    Ok(BundleCheck { valid: payload_valid && model_checks.values().all(|ok| *ok), payload_valid, model_checks, payload_hash })
}

// Replay

/// Replay tolerances (`{absolute: 1e-7, relative: 1e-6}` by default).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Tolerances {
    pub absolute: f64,
    pub relative: f64,
}

impl Default for Tolerances {
    fn default() -> Self {
        Tolerances { absolute: 1e-7, relative: 1e-6 }
    }
}

/// The comparison of one numeric family (fluxes, reduced costs…).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MapComparison {
    pub pass: bool,
    pub count: usize,
    pub missing: usize,
    #[serde(rename = "maxAbsError")]
    pub max_abs_error: f64,
    #[serde(rename = "maxRelativeError")]
    pub max_relative_error: f64,
    #[serde(rename = "worstKey")]
    pub worst_key: Option<String>,
}

/// The comparison of the objective value. `archived`/`current` are absent
/// when the solution had none (JS `undefined`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NumberComparison {
    pub pass: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<Value>,
    #[serde(rename = "absError")]
    pub abs_error: Option<f64>,
    #[serde(rename = "relativeError")]
    pub relative_error: Option<f64>,
}

/// Which families agree.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReplayChecks {
    pub status: bool,
    pub objective: bool,
    pub fluxes: bool,
    #[serde(rename = "reducedCosts")]
    pub reduced_costs: bool,
    #[serde(rename = "scaledReducedCosts")]
    pub scaled_reduced_costs: bool,
    #[serde(rename = "shadowPrices")]
    pub shadow_prices: bool,
}

/// `compareReplayedSolution`'s report.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReplayComparison {
    pub pass: bool,
    pub checks: ReplayChecks,
    pub tolerances: Tolerances,
    pub objective: NumberComparison,
    pub fluxes: MapComparison,
    #[serde(rename = "reducedCosts")]
    pub reduced_costs: MapComparison,
    #[serde(rename = "scaledReducedCosts")]
    pub scaled_reduced_costs: MapComparison,
    #[serde(rename = "shadowPrices")]
    pub shadow_prices: MapComparison,
}

impl ReplayComparison {
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("plain data serialises")
    }
}

/// `Number.isFinite(v)` (no coercion).
fn finite_value(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64).filter(|x| x.is_finite())
}

/// `a !== b` for two JS values that may be absent (`undefined`). Objects and
/// arrays are never identical: they come from two different solutions.
fn strictly_different(a: Option<&Value>, b: Option<&Value>) -> bool {
    match (a, b) {
        (None, None) => false,
        (Some(Value::Object(_) | Value::Array(_)), _) | (_, Some(Value::Object(_) | Value::Array(_))) => true,
        (Some(Value::Number(x)), Some(Value::Number(y))) => x.as_f64() != y.as_f64(),
        (Some(x), Some(y)) => x != y,
        _ => true,
    }
}

fn compare_numeric_maps(archived: Option<&Value>, current: Option<&Value>, tol: Tolerances) -> MapComparison {
    let a: IndexMap<String, Value> = archived.map(object_entries).unwrap_or_default().into_iter().collect();
    let c: IndexMap<String, Value> = current.map(object_entries).unwrap_or_default().into_iter().collect();
    let mut keys: Vec<String> = a.keys().chain(c.keys()).cloned().collect::<indexmap::IndexSet<_>>().into_iter().collect();
    keys.sort_by(|x, y| js::code_unit_compare(x, y));
    let (mut max_abs, mut max_rel, mut worst, mut missing) = (0.0f64, 0.0f64, None::<String>, 0usize);
    for key in &keys {
        let (av, cv) = (a.get(key), c.get(key));
        let (Some(x), Some(y)) = (finite_value(av), finite_value(cv)) else {
            if strictly_different(av, cv) {
                missing += 1;
                worst.get_or_insert_with(|| key.clone());
            }
            continue;
        };
        let abs = (x - y).abs();
        let rel = abs / js::max_n([1.0, x.abs(), y.abs()]);
        if abs > max_abs {
            max_abs = abs;
            worst = Some(key.clone());
        }
        max_rel = js::max(max_rel, rel);
    }
    MapComparison {
        pass: missing == 0 && (max_abs <= tol.absolute || max_rel <= tol.relative),
        count: keys.len(),
        missing,
        max_abs_error: max_abs,
        max_relative_error: max_rel,
        worst_key: worst,
    }
}

fn compare_number(a: Option<&Value>, b: Option<&Value>, tol: Tolerances) -> NumberComparison {
    let (Some(x), Some(y)) = (finite_value(a), finite_value(b)) else {
        return NumberComparison {
            pass: !strictly_different(a, b),
            archived: a.cloned(),
            current: b.cloned(),
            abs_error: None,
            relative_error: None,
        };
    };
    let abs = (x - y).abs();
    let rel = abs / js::max_n([1.0, x.abs(), y.abs()]);
    NumberComparison {
        pass: abs <= tol.absolute || rel <= tol.relative,
        archived: a.cloned(),
        current: b.cloned(),
        abs_error: Some(abs),
        relative_error: Some(rel),
    }
}

/// Compare a replayed solution with the archived one, family by family. Both
/// are `{status, objectiveValue, fluxes, reducedCosts, scaledReducedCosts,
/// shadowPrices}` with the families as objects (a JS `Map` given as an object).
pub fn compare_replayed_solution(archived: &Value, current: &Value, tol: Tolerances) -> ReplayComparison {
    let objective = compare_number(archived.get("objectiveValue"), current.get("objectiveValue"), tol);
    let family = |k: &str| compare_numeric_maps(archived.get(k), current.get(k), tol);
    let fluxes = family("fluxes");
    let reduced_costs = family("reducedCosts");
    let shadow_prices = family("shadowPrices");
    let scaled_reduced_costs = family("scaledReducedCosts");
    let status_of = |v: &Value| v.get("status").filter(|s| !s.is_null()).cloned().unwrap_or(Value::Null);
    let status = !strictly_different(Some(&status_of(archived)), Some(&status_of(current)));
    let checks = ReplayChecks {
        status,
        objective: objective.pass,
        fluxes: fluxes.pass,
        reduced_costs: reduced_costs.pass,
        scaled_reduced_costs: scaled_reduced_costs.pass,
        shadow_prices: shadow_prices.pass,
    };
    let pass = checks.status
        && checks.objective
        && checks.fluxes
        && checks.reduced_costs
        && checks.scaled_reduced_costs
        && checks.shadow_prices;
    ReplayComparison { pass, checks, tolerances: tol, objective, fluxes, reduced_costs, scaled_reduced_costs, shadow_prices }
}

// Report

fn get<'a>(v: Option<&'a Value>, key: &str) -> Option<&'a Value> {
    v.and_then(|x| x.get(key))
}

/// `a ?? b` with JS nullishness (absent or null).
fn nullish(v: Option<&Value>) -> Option<&Value> {
    v.filter(|x| !x.is_null())
}

/// `${v}` in a template literal (`undefined` for an absent value).
fn show(v: Option<&Value>) -> String {
    match v {
        None => "undefined".into(),
        Some(v) => js::to_string(v),
    }
}

fn show_or(v: Option<&Value>, fallback: &str) -> String {
    nullish(v).map_or_else(|| fallback.to_string(), js::to_string)
}

fn truthy(v: Option<&Value>) -> bool {
    v.is_some_and(js::truthy)
}

/// `Object.keys(v ?? {}).length`.
fn key_count(v: Option<&Value>) -> usize {
    match nullish(v) {
        Some(Value::String(s)) => crate::jscompat::utf16_len(s),
        Some(other) => object_entries(other).len(),
        None => 0,
    }
}

fn pct(x: Option<&Value>) -> String {
    match finite_value(x) {
        Some(x) => format!("{}%", js::to_fixed(100.0 * x, 1)),
        None => "not available".into(),
    }
}

/// The Markdown methods and provenance report of a bundle.
pub fn methods_provenance_report(bundle: &Value) -> String {
    let b = Some(bundle);
    let m = nullish(get(b, "metadata"));
    let fallback_sw = json!({"version": get(m, "coxeterVersion").cloned().unwrap_or(Value::Null)});
    let sw_value = nullish(get(m, "software")).cloned();
    let sw_has_version = sw_value.is_some();
    let sw_owned = sw_value.unwrap_or(fallback_sw);
    // `{version: m.coxeterVersion}` leaves the version undefined when absent.
    let sw = Some(&sw_owned);
    let sw_version = if sw_has_version { get(sw, "version") } else { get(m, "coxeterVersion") };
    let a = nullish(get(b, "analysis"));
    let c = nullish(get(b, "community"));
    let cons = nullish(get(b, "constraints"));
    let species: Vec<Value> = match nullish(get(c, "species")) {
        Some(Value::Array(items)) => items.clone(),
        _ => Vec::new(),
    };
    let hashes = nullish(get(b, "modelHashes"));
    let overrides = nullish(get(cons, "reactionOverrides"));
    let om = get(b, "omics").filter(|v| js::truthy(v));
    let solution = get(b, "solution").filter(|v| js::truthy(v));

    let commit = if truthy(get(sw, "gitCommit")) { format!(" (Git commit {})", show(get(sw, "gitCommit"))) } else { String::new() };
    let mut lines: Vec<String> = vec![
        "# Coxeter Methods & Provenance Report".into(),
        String::new(),
        "## Software".into(),
        format!("Analysis generated with Coxeter Multicell Flux Visualizer version {}{commit}.", show_or(sw_version, "unknown")),
        if truthy(get(sw, "gitBranch")) { format!("Git branch: {}.", show(get(sw, "gitBranch"))) } else { String::new() },
        if truthy(get(sw, "buildTime")) { format!("Build timestamp: {}.", show(get(sw, "buildTime"))) } else { String::new() },
        format!("Bundle created: {}.", show_or(get(m, "createdAt"), "unknown")),
        String::new(),
        "## Metabolic models".into(),
        format!("The community contained {} model(s).", species.len()),
    ];
    for sp in &species {
        let sp = Some(sp);
        let file = nullish(get(sp, "file")).or_else(|| nullish(get(sp, "model")));
        let id_key = show(get(sp, "id"));
        lines.push(format!(
            "- {}: model {}; abundance {}; SHA-256 {}.",
            show(get(sp, "id")),
            show_or(file, "embedded"),
            show_or(get(sp, "abundance"), "not specified"),
            show_or(get(hashes, &id_key), "not available"),
        ));
    }
    lines.push(String::new());
    lines.push("## Community and constraints".into());
    let objective = nullish(get(c, "objective")).or_else(|| nullish(get(a, "objective")));
    let target = if truthy(get(c, "productTarget")) { format!("; product target: {}", show(get(c, "productTarget"))) } else { String::new() };
    lines.push(format!("Community objective: {}{target}.", show_or(objective, "not specified")));
    lines.push(format!(
        "Medium contained {} explicitly recorded component(s); {} reaction-bound override(s)/knockout(s) were applied.",
        key_count(get(cons, "medium")),
        key_count(overrides)
    ));
    lines.push(String::new());
    if let Some(study) = get(b, "study").filter(|v| js::truthy(v)) {
        let s = Some(study);
        lines.push("## Published case adaptation".into());
        lines.push(format!(
            "Case: {}; scenario: {}; display: {}; fixed flux reference: {}.",
            show(get(s, "id")),
            show(get(s, "scenarioId")),
            show(get(s, "comparison")),
            show(get(s, "fluxReference"))
        ));
        lines.push("The complete original monoculture and joint community source models are embedded in this personal bundle, together with the exact canonical active LP. Identifier namespacing preserves stoichiometry, source bounds and objective coefficients. The documented pheA deletion disables Ec PPNDH in mutant scenarios.".into());
        lines.push("The source objective and population coefficients are retained. S. elongatus is absent from monoculture calculations even when its faint cell remains visible for map comparison. Displaying fewer reactions or rearranging their geometry does not change the LP.".into());
        lines.push("These stationary FBA solutions are adaptations of the published model conditions. They do not reproduce dynamic/spatial culture trajectories or experimental growth measurements.".into());
        for (file, hash) in object_entries(nullish(get(s, "sourceHashes")).unwrap_or(&Value::Null)) {
            lines.push(format!("- Original source {file}: SHA-256 {}.", js::to_string(&hash)));
        }
        lines.push("Original source files were downloaded privately. Review the authors’ terms before redistributing a personal bundle.".into());
        lines.push(String::new());
    }
    lines.push("## Flux analysis".into());
    lines.push(format!(
        "Method: {}. Solver: {}.",
        show_or(get(a, "method"), "not specified"),
        show_or(get(a, "solver"), "not specified")
    ));
    lines.push(if get(a, "method").and_then(Value::as_str) == Some("pFBA") {
        format!("pFBA biomass target: {}% of the primary optimum.", show_or(get(a, "pfbaBiomassPercent"), "100"))
    } else {
        String::new()
    });
    lines.push(if truthy(get(a, "fvaEnabled")) { "Flux variability analysis (FVA) was enabled.".into() } else { String::new() });
    lines.push(if truthy(get(a, "loopless")) { "Loopless constraints were enabled.".into() } else { String::new() });
    lines.push(match get(a, "gimme").filter(|v| js::truthy(v)) {
        Some(g) => {
            let g = Some(g);
            let fraction = nullish(get(g, "objectiveFraction")).map_or(0.9, js::to_number);
            format!(
                "GIMME used condition \"{}\", expression threshold {}, and required {}% of the unconstrained metabolic objective.",
                show_or(get(g, "condition"), "unspecified"),
                show_or(get(g, "threshold"), "auto"),
                js::number_to_string(js::round(100.0 * fraction))
            )
        }
        None => String::new(),
    });
    lines.push(String::new());
    lines.push("## Omics".into());
    lines.push(match om {
        Some(o) => {
            let o = Some(o);
            let conditions = match nullish(get(o, "conditions")) {
                Some(Value::Array(items)) => {
                    items.iter().map(|x| if x.is_null() { String::new() } else { js::to_string(x) }).collect::<Vec<_>>().join(", ")
                }
                Some(other) => js::to_string(other),
                None => String::new(),
            };
            format!(
                "A {} dataset ({}) was integrated. Conditions: {}.",
                show_or(get(o, "dataType"), "omics"),
                show_or(get(o, "source"), "source not recorded"),
                if conditions.is_empty() { "not recorded".to_string() } else { conditions }
            )
        }
        None => "No omics dataset was integrated.".into(),
    });
    lines.push(match get(om, "mappingReport").filter(|v| js::truthy(v)) {
        Some(r) => format!(
            "Mapping coverage: {} of model genes and {} of GPR-associated reactions.",
            pct(r.get("geneCoverage")),
            pct(r.get("reactionCoverage"))
        ),
        None => String::new(),
    });
    lines.push(String::new());
    lines.push("## Integrity and reproducibility".into());
    let integrity = get(b, "integrity");
    lines.push(format!("Bundle integrity algorithm: {}.", show_or(get(integrity, "algorithm"), "not recorded")));
    lines.push(format!("Bundle SHA-256: {}.", show_or(get(integrity, "payloadHash"), "not recorded")));
    lines.push("The bundle embeds the metabolic models, constraints, analysis settings, map state, omics dataset and archived numerical solution. Re-importing restores these inputs, reruns the archived analysis, and numerically compares the replayed result with the archived solution using recorded tolerances.".into());
    lines.push(String::new());
    lines.push("## Numerical result".into());
    lines.push(match solution {
        Some(s) => format!(
            "Archived solver status: {}; objective value: {}.",
            show(s.get("status")),
            show_or(s.get("objectiveValue"), "not available")
        ),
        None => "No numerical solution was archived.".into(),
    });
    lines.push(if truthy(get(get(solution, "dualAnalysis"), "available")) {
        format!(
            "LP dual analysis archived: {} reduced costs and {} shadow prices. Scaled reduced costs use RC/max|RC| with scale {}.",
            key_count(get(solution, "reducedCosts")),
            key_count(get(solution, "shadowPrices")),
            show_or(get(solution, "reducedCostScale"), "not available")
        )
    } else {
        String::new()
    });
    lines.push(String::new());
    lines.push("### Suggested Methods text".into());
    let commit = if truthy(get(sw, "gitCommit")) { format!(", commit {}", show(get(sw, "gitCommit"))) } else { String::new() };
    lines.push(format!(
        "Metabolic simulations were performed with Coxeter Multicell Flux Visualizer (version {}{commit}) using {}. Model inputs and analysis parameters were archived in a Coxeter Reproducibility Bundle and integrity-checked using SHA-256 hashes.",
        show_or(sw_version, "unknown"),
        show_or(get(a, "method"), "flux balance analysis")
    ));

    let kept: Vec<&str> = lines
        .iter()
        .enumerate()
        .filter(|(i, x)| !x.is_empty() || *i == 0 || !lines[i - 1].is_empty())
        .map(|(_, x)| x.as_str())
        .collect();
    format!("{}\n", crate::jscompat::js_trim(&kept.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;

    // From test/reproducibility.test.mjs.
    #[test]
    fn canonical_json_and_sha256_are_independent_of_object_key_order() {
        assert_eq!(canonical_json(&json!({"b": 2, "a": {"d": 4, "c": 3}})), canonical_json(&json!({"a": {"c": 3, "d": 4}, "b": 2})));
        assert_eq!(sha256(&json!({"b": 2, "a": 1})), sha256(&json!({"a": 1, "b": 2})));
        assert_eq!(canonical_json(&json!({"b": 1e21, "10": 0.1, "9": [1.0, -0.0], "a": "é\n"})), r#"{"9":[1,0],"10":0.1,"a":"é\n","b":1e+21}"#);
    }

    #[test]
    fn bundle_verification_detects_model_tampering() {
        let model = json!({"id": "m", "reactions": [], "metabolites": []});
        let mut bundle = json!({"format": REPRO_FORMAT, "version": 1, "metadata": {}, "models": {"sp": model}, "modelHashes": {"sp": sha256(&model)},
            "community": {}, "constraints": {}, "analysis": {}, "omics": null, "map": null, "solution": null});
        let hash = sha256(&bundle);
        bundle["integrity"] = json!({"algorithm": "SHA-256", "payloadHash": hash});
        assert!(verify_reproducibility_bundle(&bundle).unwrap().valid);
        bundle["models"]["sp"]["id"] = json!("changed");
        let check = verify_reproducibility_bundle(&bundle).unwrap();
        assert!(!check.valid);
        assert!(!check.model_checks["sp"]);
    }

    #[test]
    fn bundle_verification_detects_manifest_tampering() {
        let mut bundle = json!({"format": REPRO_FORMAT, "version": 1, "metadata": {}, "models": {}, "modelHashes": {},
            "community": {"objective": "growth"}, "constraints": {}, "analysis": {"method": "FBA"}, "omics": null, "map": null, "solution": null});
        let hash = sha256(&bundle);
        bundle["integrity"] = json!({"algorithm": "SHA-256", "payloadHash": hash});
        bundle["analysis"]["method"] = json!("pFBA");
        assert!(!verify_reproducibility_bundle(&bundle).unwrap().payload_valid);
        assert!(verify_reproducibility_bundle(&json!({"format": "x"})).is_err());
    }

    #[test]
    fn numerical_replay_comparison_accepts_values_within_tolerance() {
        let archived = json!({"status": "optimal", "objectiveValue": 1, "fluxes": {"R1": 2, "R2": -3}, "reducedCosts": {"R1": 0, "R2": -0.2},
            "scaledReducedCosts": {"R1": 0, "R2": -1}, "shadowPrices": {"M1": 0.5}});
        let current = json!({"status": "optimal", "objectiveValue": 1.0 + 1e-9, "fluxes": {"R1": 2.0 + 1e-9, "R2": -3}, "reducedCosts": {"R1": 0, "R2": -0.2},
            "scaledReducedCosts": {"R1": 0, "R2": -1}, "shadowPrices": {"M1": 0.5}});
        let r = compare_replayed_solution(&archived, &current, Tolerances::default());
        assert!(r.pass);
        assert_eq!(r.fluxes.count, 2);
        assert!(r.fluxes.max_abs_error > 0.0);
    }

    #[test]
    fn numerical_replay_comparison_reports_divergent_and_missing_values() {
        let archived = json!({"status": "optimal", "objectiveValue": 1, "fluxes": {"R1": 2, "R2": 3}, "reducedCosts": {}, "scaledReducedCosts": {}, "shadowPrices": {}});
        let current = json!({"status": "optimal", "objectiveValue": 1.1, "fluxes": {"R1": 2.2}});
        let r = compare_replayed_solution(&archived, &current, Tolerances::default());
        assert!(!r.pass);
        assert!(!r.checks.objective);
        assert!(!r.checks.fluxes);
        assert_eq!(r.fluxes.missing, 1);
    }

    #[test]
    fn replay_comparison_exposes_per_family_checks_for_the_visual_report() {
        let archived = json!({"status": "optimal", "objectiveValue": 2, "fluxes": {"R": 1}, "reducedCosts": {"R": 0}, "scaledReducedCosts": {"R": 0}, "shadowPrices": {"M": 0.25}});
        let current = archived.clone();
        let r = compare_replayed_solution(&archived, &current, Tolerances::default());
        assert_eq!(
            serde_json::to_value(&r.checks).unwrap(),
            json!({"status": true, "objective": true, "fluxes": true, "reducedCosts": true, "scaledReducedCosts": true, "shadowPrices": true})
        );
        assert!(r.pass);
    }
}
