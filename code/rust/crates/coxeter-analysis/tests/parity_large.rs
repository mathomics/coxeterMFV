//! Large and real-data checks. Each skips cleanly when its input is absent.
//!
//! * `rust/fixtures/large/analysis/stolyar-reconstructed/`: the four
//!   CellNetAnalyzer files fixtures-analysis.mjs rebuilds from the bundled
//!   Stolyar joint model (180 real reactions), and what the JS read from them.
//! * `rust/fixtures/large/analysis/yeast9-omics.json`: omics scores on the
//!   user's yeast-GEM.xml (4000 reactions with real gene rules).
//! * `COXETER_ZUNIGA_DIR`: a folder holding the authors' `M-models.mat` and
//!   `CM-modelsKO.mat`. The study is loaded through the real loader (hashes
//!   checked against the bundled manifest) and the three scenarios are
//!   compared with the bundled `results.json` at the manifest's tolerance.
//! * `zuniga_downloads_the_pinned_files` (ignored: it needs the network)
//!   downloads them over HTTPS into a cache under the target folder.

mod common;

use std::sync::Arc;

use coxeter_analysis::overlay::{
    CompareOptions, OmicsCsvOptions, OmicsSpecies, compare_reaction_scores, parse_omics_csv, reaction_omics_scores, scores_to_json,
    sequential_omics_colors,
};
use coxeter_analysis::stolyar::{self, JointOptions, StolyarFiles};
use coxeter_analysis::zuniga::{self, DirectoryCache, FetchResponse, SourceCache, ZunigaLoadOptions, ZunigaStudy};
use coxeter_analysis::{Error, ErrorKind, Result, community_omics_mapping_report};
use coxeter_core::testkit::{assert_json_eq, read_fixture, repo_root};
use coxeter_core::{CommunityModel, CommunitySpec, Model, Objective, SpeciesSpec};
use coxeter_io::sbml::{SbmlReadOptions, parse_cobra_sbml};
use indexmap::IndexMap;
use serde_json::{Value, json};

#[test]
fn the_rebuilt_stolyar_supplement_reads_as_the_js_reads_it() {
    let dir = repo_root().join("rust/fixtures/large/analysis/stolyar-reconstructed");
    let Some(js) = read_fixture("large/analysis/stolyar-reconstructed/outputs.json") else {
        return eprintln!("skipped: run node rust/tools/fixtures-analysis.mjs");
    };
    let read = |f: &str| std::fs::read_to_string(dir.join(f)).unwrap();
    let (r, m, mm, s) = (read("reactions.txt"), read("metabolites.txt"), read("macromolecules.txt"), read("macromolecule_synthesis.txt"));
    let files = StolyarFiles { reactions: &r, metabolites: &m, macromolecules: &mm, synthesis: &s };
    assert_eq!(js["reproducesBundledJoint"], json!(true), "the rebuilt files should give the bundled joint model");
    let model = stolyar::import_stolyar(&files).unwrap();
    assert_json_eq(&model.to_json(), &js["model"], 0.0, "model");
    assert_json_eq(&stolyar::validate_stolyar(&model).to_json(), &js["validation"], 0.0, "validation");
    let exchange: Vec<Value> = stolyar::exchange_structure(&model).iter().map(|x| x.to_json()).collect();
    assert_json_eq(&json!(exchange), &js["exchange"], 0.0, "exchange");
    assert_json_eq(&json!(stolyar::objective_of(&model)), &js["objective"], 0.0, "objective");
    let joint = stolyar::to_joint_cobra_json(&model, &JointOptions::default());
    assert_json_eq(&joint, &js["joint"], 0.0, "joint");
    for sp in stolyar::SPECIES {
        assert_json_eq(&stolyar::to_species_cobra_json(&model, sp.id, None).unwrap(), &js["species"][sp.id], 0.0, sp.id);
    }
    // And the Rust joint model is the one the app ships.
    let bundled: Value = serde_json::from_str(
        &std::fs::read_to_string(repo_root().join("models/published/stolyar-2007-syntrophy/joint-model.json")).unwrap(),
    )
    .unwrap();
    assert_json_eq(&joint, &bundled, 0.0, "the bundled joint-model.json");
}

#[test]
fn yeast9_omics_scores_match() {
    let Some(js) = read_fixture("large/analysis/yeast9-omics.json") else {
        return eprintln!("skipped: yeast9-omics.json absent (needs the user's yeast-GEM.xml)");
    };
    let Ok(xml) = std::fs::read_to_string(js["input"].as_str().unwrap()) else {
        return eprintln!("skipped: {} absent", js["input"]);
    };
    let model = Arc::new(Model::from_value(&parse_cobra_sbml(&xml, &SbmlReadOptions::default())).unwrap());
    let community = CommunityModel::new(CommunitySpec {
        name: Some("yeast".into()),
        species: vec![SpeciesSpec { id: Some("sce".into()), model: model.clone(), abundance: Some(1.0), enabled: Some(true), color: None }],
        medium: IndexMap::new(),
        objective: Objective::parse("equal-growth"),
        product_target: None,
    })
    .unwrap();
    let dataset = parse_omics_csv(js["csv"].as_str().unwrap(), &OmicsCsvOptions::default()).unwrap();
    let species = [OmicsSpecies::new("sce", "yeast")];
    let models: IndexMap<String, Arc<Model>> = [("yeast".to_string(), model)].into_iter().collect();
    let a = reaction_omics_scores(&community, &species, &models, Some(&dataset), "A");
    let b = reaction_omics_scores(&community, &species, &models, Some(&dataset), "B");
    assert_json_eq(&scores_to_json(&a), &js["scores"]["A"], 0.0, "scores A");
    assert_json_eq(&scores_to_json(&b), &js["scores"]["B"], 0.0, "scores B");
    assert_json_eq(&scores_to_json(&compare_reaction_scores(&a, &b, &CompareOptions::default())), &js["log2fc"], 0.0, "log2fc");
    assert_json_eq(&sequential_omics_colors(&a, false).to_json(), &js["sequential"], 0.0, "sequential");
    let report = community_omics_mapping_report(&community, &species, &models, &dataset);
    assert_json_eq(&report.to_json(), &js["report"], 0.0, "report");
}

fn manifest() -> Value {
    serde_json::from_str(&std::fs::read_to_string(repo_root().join("models/published/zuniga-2020-phea-rescue/manifest.json")).unwrap()).unwrap()
}

/// The three scenarios against the bundled results.json.
fn check_against_results(study: &ZunigaStudy) {
    let results: Value = serde_json::from_str(
        &std::fs::read_to_string(repo_root().join("models/published/zuniga-2020-phea-rescue/results.json")).unwrap(),
    )
    .unwrap();
    let tolerance = results["tolerance"].as_f64().unwrap();
    for (file, hash) in results["sourceHashes"].as_object().unwrap() {
        assert_eq!(study.source_hashes.get(file).map(String::as_str), hash.as_str(), "{file}");
    }
    for js in results["scenarios"].as_array().unwrap() {
        let id = js["id"].as_str().unwrap();
        let prepared = zuniga::create_zuniga_scenario(study, id).unwrap();
        let solved = zuniga::solve_zuniga_scenario(&prepared);
        assert_eq!(solved.result.status.as_str(), js["status"].as_str().unwrap(), "{id}");
        assert_eq!(json!(prepared.knockouts), js["knockouts"], "{id}");
        assert_eq!(solved.zuniga.source, js["source"].as_str().unwrap());
        let objective = js["objectiveValue"].as_f64().unwrap();
        assert!((solved.result.objective_value - objective).abs() <= tolerance, "{id}: objective {} vs {objective}", solved.result.objective_value);
        let growth = prepared.community.growth_rates(&solved.result.fluxes);
        for (member, rate) in js["growth"].as_object().unwrap() {
            assert!((growth[member] - rate.as_f64().unwrap()).abs() <= tolerance, "{id} {member}: {} vs {rate}", growth[member]);
        }
        assert_eq!(solved.result.fluxes.len(), js["fluxes"].as_object().unwrap().len(), "{id}: every source reaction is solved");
    }
    let reference = &results["reference"];
    let coculture = zuniga::solve_zuniga_scenario(&zuniga::create_zuniga_scenario(study, "mutant-coculture").unwrap());
    for key in ["PHEtex_D", "PHELtex_A"] {
        let got = coculture.zuniga.original_fluxes[key];
        let want = reference[key].as_f64().unwrap();
        assert!((got - want).abs() <= tolerance, "{key}: {got} vs published {want}");
    }
}

#[test]
fn zuniga_reproduces_the_published_predictions_from_local_source_files() {
    let Ok(dir) = std::env::var("COXETER_ZUNIGA_DIR") else {
        return eprintln!("skipped: set COXETER_ZUNIGA_DIR to a folder holding M-models.mat and CM-modelsKO.mat");
    };
    let dir = std::path::PathBuf::from(dir);
    let fetch_local = |url: &str| -> Result<FetchResponse> {
        let file = url.rsplit('/').next().unwrap_or(url);
        let body = std::fs::read(dir.join(file)).map_err(|e| Error::new(ErrorKind::Download, format!("{file}: {e}")))?;
        Ok(FetchResponse { status: 200, body })
    };
    let study = zuniga::load_zuniga_study(&manifest(), ZunigaLoadOptions { fetcher: &fetch_local, cache: None, on_progress: None }).unwrap();
    check_against_results(&study);
}

#[cfg(feature = "download")]
#[test]
#[ignore = "downloads the authors' files from GitHub"]
fn zuniga_downloads_the_pinned_files() {
    let cache = DirectoryCache::new(repo_root().join("rust/target/zuniga-cache"));
    let fetcher = coxeter_analysis::HttpsFetcher::default();
    let mut log = |m: &str| eprintln!("{m}");
    let study = zuniga::load_zuniga_study(&manifest(), ZunigaLoadOptions { fetcher: &fetcher, cache: Some(&cache), on_progress: Some(&mut log) }).unwrap();
    check_against_results(&study);
}

#[test]
fn a_directory_cache_serves_verified_bytes_without_fetching_again() {
    let dir = std::env::temp_dir().join(format!("coxeter-analysis-cache-{}", std::process::id()));
    let cache = DirectoryCache::new(&dir);
    assert_eq!(cache.get("k"), None);
    cache.put("k", b"bytes");
    assert_eq!(cache.get("k").as_deref(), Some(&b"bytes"[..]));

    // A manifest whose one source is the toy M-models.mat: the first load
    // fetches and caches it, the second never asks the fetcher.
    let bytes = std::fs::read(repo_root().join("rust/fixtures/analysis/zuniga-M-models.mat")).unwrap();
    let sha = coxeter_io::reproducibility::sha256_hex(&bytes);
    let manifest = json!({"sources": [{"file": "M-models.mat", "url": "https://example.invalid/M-models.mat", "sha256": sha}]});
    let calls = std::cell::Cell::new(0);
    let fetch = |_: &str| -> Result<FetchResponse> {
        calls.set(calls.get() + 1);
        Ok(FetchResponse { status: 200, body: bytes.clone() })
    };
    for _ in 0..2 {
        // The toy has no joint matrix, so the load stops after reading the file.
        let err = zuniga::load_zuniga_study(&manifest, ZunigaLoadOptions { fetcher: &fetch, cache: Some(&cache), on_progress: None }).unwrap_err();
        assert_eq!(err.message, "The original model cell arrays are incomplete.");
    }
    assert_eq!(calls.get(), 1, "the second load reads the cache");
    // Corrupt cached bytes are refused, not refetched (as in the JS).
    cache.put(&format!("https://example.invalid/M-models.mat#{sha}"), b"tampered");
    let err = zuniga::load_zuniga_study(&manifest, ZunigaLoadOptions { fetcher: &fetch, cache: Some(&cache), on_progress: None }).unwrap_err();
    assert_eq!(err.message, "Source integrity check failed for M-models.mat.");
    let _ = std::fs::remove_dir_all(&dir);
}
