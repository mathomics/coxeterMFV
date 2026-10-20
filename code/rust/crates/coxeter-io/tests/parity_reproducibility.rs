//! Reproducibility bundles, digests, replay comparison and the methods report
//! against `src/core/reproducibility.js` (`rust/fixtures/io/reproducibility.json`).

mod common;

use coxeter_core::model::Model;
use coxeter_core::testkit::{assert_json_eq, read_fixture};
use coxeter_io::reproducibility::{
    BundleContext, OmicsDataset, OmicsState, ReproStore, SoftwareIdentity, SolutionState, Tolerances, analysis_provenance,
    canonical_json, compare_replayed_solution, create_reproducibility_bundle_with, methods_provenance_report, sha256,
    software_identity, verify_reproducibility_bundle,
};
use indexmap::IndexMap;
use serde_json::Value;

use common::{assert_same_text, repo};

fn fixture() -> Value {
    read_fixture("io/reproducibility.json").expect("run: node rust/tools/fixtures-io.mjs")
}

fn entries(v: &Value) -> Vec<(String, Value)> {
    v.as_array().unwrap().iter().map(|e| (e[0].as_str().unwrap().to_string(), e[1].clone())).collect()
}

/// `Number(v)` of the description's numbers, `"NaN"`/`"Infinity"` included.
fn number(v: &Value) -> f64 {
    coxeter_core::js::to_number(v)
}

fn numbers(v: &Value) -> Vec<(String, f64)> {
    entries(v).into_iter().map(|(k, v)| (k, number(&v))).collect()
}

fn opt(v: &Value) -> Option<Value> {
    if v.is_null() { None } else { Some(v.clone()) }
}

fn store<'a>(desc: &Value, models: &'a IndexMap<String, Model>) -> ReproStore<'a> {
    let omics = desc["omics"].as_object().map(|o| OmicsState {
        dataset: o["dataset"].as_object().map(|d| OmicsDataset {
            source: d["source"].clone(),
            data_type: d["dataType"].clone(),
            conditions: d["conditions"].clone(),
            values: entries(&d["values"]),
        }),
        enabled: o["enabled"].clone(),
        mode: o["mode"].clone(),
        condition_a: o["conditionA"].clone(),
        condition_b: o["conditionB"].clone(),
        metric: o["metric"].clone(),
        pseudocount: o["pseudocount"].clone(),
        gimme_enabled: o["gimmeEnabled"].as_bool().unwrap(),
        gimme_threshold: o["gimmeThreshold"].clone(),
        gimme_objective_fraction: o["gimmeObjectiveFraction"].clone(),
        gimme_result: opt(&o["gimmeResult"]),
        report: opt(&o["report"]),
    });
    let solution = desc["solution"].as_object().map(|s| SolutionState {
        status: s["status"].clone(),
        objective_value: Some(number(&s["objectiveValue"])),
        iterations: opt(&s["iterations"]),
        fluxes: numbers(&s["fluxes"]),
        reduced_costs: numbers(&s["reducedCosts"]),
        scaled_reduced_costs: numbers(&s["scaledReducedCosts"]),
        reduced_cost_scale: Some(number(&s["reducedCostScale"])),
        shadow_prices: numbers(&s["shadowPrices"]),
        dual_analysis: opt(&s["dualAnalysis"]),
        zuniga: opt(&s["zuniga"]),
    });
    ReproStore {
        species: desc["species"].as_array().unwrap().clone(),
        models: models.iter().map(|(k, m)| (k.clone(), m)).collect(),
        scenario: opt(&desc["scenario"]),
        objective: desc["objective"].clone(),
        product_target: desc.get("productTarget").and_then(opt),
        custom_objective: desc.get("customObjective").and_then(opt),
        medium: desc["medium"].as_object().unwrap().clone(),
        overrides: entries(&desc["overrides"]),
        gene_knockouts: Vec::new(),
        parsimonious: desc["parsimonious"].as_bool().unwrap(),
        pfba_biomass_percent: desc["pfbaBiomassPercent"].clone(),
        loopless: desc["loopless"].as_bool().unwrap(),
        fva_enabled: desc["fvaEnabled"].as_bool().unwrap(),
        fva_ranges: entries(&desc["fvaRanges"]),
        growth: entries(&desc["growth"]),
        map: opt(&desc["map"]),
        omics,
        solution,
        zuniga: None,
    }
}

#[test]
fn canonical_json_and_digests_match_javascript() {
    let fx = fixture();
    for case in fx["canonical"].as_array().unwrap() {
        let value = match case.get("model") {
            Some(name) => {
                let path = repo(&format!("models/{}.json", name.as_str().unwrap()));
                Model::from_json_str(&std::fs::read_to_string(path).unwrap()).unwrap().to_json()
            }
            None => case["value"].clone(),
        };
        if !value.is_string() {
            assert_same_text(&canonical_json(&value), case["canonical"].as_str().unwrap(), &format!("canonicalJSON({value})"));
        }
        assert_eq!(sha256(&value), case["sha256"].as_str().unwrap(), "sha256({value})");
    }
}

#[test]
fn bundles_match_javascript() {
    let fx = fixture();
    let software = &fx["software"];
    let identity = SoftwareIdentity {
        version: software["version"].as_str().unwrap().into(),
        git_commit: software["gitCommit"].as_str().map(String::from),
        git_branch: software["gitBranch"].as_str().map(String::from),
        build_time: software["buildTime"].as_str().map(String::from),
    };
    assert_eq!(software_identity().to_json(), identity.to_json(), "softwareIdentity() of a build with nothing injected");
    let ctx = BundleContext { created_at: fx["createdAt"].as_str().unwrap().into(), user_agent: None, software: identity };
    for (name, case) in fx["bundles"].as_object().unwrap() {
        let desc = &case["store"];
        let mut models = IndexMap::new();
        for sp in desc["species"].as_array().unwrap() {
            let file = sp["file"].as_str().unwrap();
            if let Ok(text) = std::fs::read_to_string(repo(&format!("models/{file}.json"))) {
                models.insert(file.to_string(), Model::from_json_str(&text).unwrap());
            }
        }
        let store = store(desc, &models);
        let bundle = create_reproducibility_bundle_with(&store, &ctx);
        assert_json_eq(&bundle, &case["bundle"], 0.0, &format!("bundle {name}"));
        assert_json_eq(&analysis_provenance(&store), &case["analysis"], 0.0, &format!("analysisProvenance {name}"));
        let verify = serde_json::to_value(verify_reproducibility_bundle(&case["bundle"]).unwrap()).unwrap();
        assert_json_eq(&verify, &case["verify"], 0.0, &format!("verify {name}"));
        let tampered = serde_json::to_value(verify_reproducibility_bundle(&case["tampered"]["bundle"]).unwrap()).unwrap();
        assert_json_eq(&tampered, &case["tampered"]["verify"], 0.0, &format!("verify tampered {name}"));
        assert_same_text(&methods_provenance_report(&bundle), case["report"].as_str().unwrap(), &format!("report {name}"));
    }
    for case in fx["verifyErrors"].as_array().unwrap() {
        let err = verify_reproducibility_bundle(&case["bundle"]).unwrap_err();
        assert_eq!(err.message, case["error"].as_str().unwrap());
    }
    for case in fx["verifyOdd"].as_array().unwrap() {
        let got = serde_json::to_value(verify_reproducibility_bundle(&case["bundle"]).unwrap()).unwrap();
        assert_json_eq(&got, &case["out"], 0.0, &format!("verify {}", case["bundle"]));
    }
}

#[test]
fn replay_comparisons_match_javascript() {
    let fx = fixture();
    for case in fx["compare"].as_array().unwrap() {
        let tol = match &case["tolerances"] {
            Value::Null => Tolerances::default(),
            t => Tolerances { absolute: t["absolute"].as_f64().unwrap(), relative: t["relative"].as_f64().unwrap() },
        };
        let got = compare_replayed_solution(&case["archived"], &case["current"], tol).to_json();
        assert_json_eq(&got, &case["out"], 0.0, &format!("compareReplayedSolution({}, {})", case["archived"], case["current"]));
    }
}

#[test]
fn methods_reports_match_javascript() {
    let fx = fixture();
    for case in fx["reports"].as_array().unwrap() {
        assert_same_text(&methods_provenance_report(&case["bundle"]), case["report"].as_str().unwrap(), &format!("report of {}", case["bundle"]));
    }
    // A bundle made here verifies, and its report names its hash.
    let bundle = &fx["bundles"]["pair"]["bundle"];
    assert!(verify_reproducibility_bundle(bundle).unwrap().valid);
    assert!(methods_provenance_report(bundle).contains(bundle["integrity"]["payloadHash"].as_str().unwrap()));
}
