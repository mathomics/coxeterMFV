//! Port of the Store parts of `test/published-communities.test.mjs`.
//!
//! The registry tests (metadata, citations, species, statuses, the six
//! papers, `models/scenarios.json`, the recorded results and the slow
//! four-auxotroph solve) read no Store and are ported in
//! `coxeter-analysis/tests/js_tests.rs`, as are the file checks of "the
//! four-E. coli published community keeps full models separate from map
//! previews". That test also matches three lines of `store.js` with regular
//! expressions; here the behaviour those lines guard is asserted on the
//! Rust store instead (which file a member runs on, and that the curated
//! overview, not the full models, is what is drawn).

mod common;

use std::sync::Arc;

use coxeter_core::testkit::repo_root;
use coxeter_state::{MemoryAssets, Store};
use serde_json::{Value, json};

#[test]
fn the_four_e_coli_published_community_keeps_full_models_separate_from_map_previews() {
    let id = "steadycom-ecoli-auxotrophs";
    let mut store = common::init_store();
    let manifest = store.published_manifest(id).expect("the community ships a manifest");
    let members = manifest["members"].as_array().unwrap().clone();
    // `publishedMemberFileForRuntime(id, m)`: hosted builds must continue to
    // prefer the full biological model (`return full ?? preview`).
    for member in &members {
        assert_eq!(store.published_member_file_for_runtime(id, member), member["model"].as_str().map(str::to_string));
    }
    store.use_published_community(id).unwrap();
    for (sp, member) in store.species.iter().zip(&members) {
        let full = member["model"].as_str().unwrap().trim_end_matches(".json");
        assert_eq!(sp.file, format!("published/{id}/{full}"), "the member runs on its full model");
    }
    let community = store.community.clone().expect("the full models form the community");
    for sp in &store.species {
        assert!(community.merged.reactions.contains_key(&format!("{}@ACALD", sp.id)), "{}: full model is missing ACALD", sp.id);
    }
    // `contents: curatedCoreMap || jointMap ? 'none' : 'all'`: the E. coli
    // preview remains a map concern when the full biological model is
    // available.
    let map = store.map.as_ref().expect("a curated map");
    assert_eq!(store.map_mode, "core-overview");
    assert!(map.reaction_ids().len() < community.merged.reactions.len());
    assert!(!map.reaction_ids().iter().any(|r| r.ends_with("@ACALD")), "ACALD is in the model, not in the overview");
}

#[test]
fn standalone_runtime_loads_an_embedded_published_preview_instead_of_fetching_an_absent_full_model() {
    let preview: Value = serde_json::from_str(&std::fs::read_to_string(repo_root().join("models/ecoli_core_mc.json")).unwrap()).unwrap();
    // `globalThis.__ESCHER_MCMC_DATA__`: only the preview is embedded.
    let assets = MemoryAssets::embedded();
    assets.insert_json("models/published/runtime-preview/preview-Ec.json", &preview);
    let mut store = Store::new(Arc::new(assets));
    store.published_communities = vec![json!({
        "id": "runtime-preview",
        "name": "Runtime preview community",
        "description": "Regression fixture for the standalone file.",
    })];
    // `store.publishedManifest = async () => (…)`: the manifest, remembered.
    store.published_manifests.insert(
        "runtime-preview".into(),
        Some(json!({
            "medium": { "glc__D": 10, "o2": 1000, "nh4": 1000, "pi": 1000, "h2o": 1000, "h": 1000, "co2": 1000 },
            "members": [{ "id": "Ec", "model": "model-Ec.json", "preview": "preview-Ec.json" }],
        })),
    );

    store.use_published_community("runtime-preview").unwrap();

    assert_eq!(store.scenario_id().as_deref(), Some("published:runtime-preview"));
    assert_eq!(store.species[0].file, "published/runtime-preview/preview-Ec");
    assert!(store.community.is_some(), "standalone preview did not become the active community");
    let map = store.map.as_ref().expect("standalone preview did not create a map");
    assert!(!map.graph().nodes.is_empty(), "standalone preview map is empty");
}
