//! Genome-scale layouts against the JavaScript, exactly: one iAF1260
//! (2,383 reactions) and the four-E. coli community of
//! `models/published/steadycom-ecoli-auxotrophs` (9,831 drawn reactions,
//! 27,879 nodes). This is the proof that the neighbour grid of `relax` gives
//! the bits the JavaScript's all-pairs loop gives at the scale it was added
//! for.
//!
//! The references take minutes to make in Node and are not committed
//! (`node rust/tools/fixtures-map.mjs perf perf4`); the test is ignored by
//! default because a debug build needs minutes too:
//!
//!   cargo test --release -p coxeter-map --test parity_big_layout -- --ignored --nocapture

mod common;

use std::sync::Arc;
use std::time::Instant;

use coxeter_core::community::{CommunityModel, CommunitySpec, SpeciesSpec};
use coxeter_core::model::Model;
use coxeter_core::testkit::repo_root;
use coxeter_map::EditableMap;
use coxeter_map::layout::LayoutOptions;
use common::*;

#[test]
#[ignore = "genome-scale: run with --release -- --ignored"]
fn genome_scale_layouts_match_javascript_exactly() {
    let Some(index) = fixture("map/index-perf.json") else {
        eprintln!("big layouts: no references (node rust/tools/fixtures-map.mjs perf perf4)");
        return;
    };
    for entry in index.as_array().unwrap() {
        let name = entry["name"].as_str().unwrap();
        let Some(expected) = fixture(entry["file"].as_str().unwrap()) else {
            eprintln!("big layouts: {name} skipped (reference absent)");
            continue;
        };
        let members: Vec<&str> = entry["members"].as_array().unwrap().iter().map(|m| m.as_str().unwrap()).collect();
        let n = members.len() as f64;
        let species = members
            .iter()
            .map(|id| {
                let path = repo_root().join(format!("models/published/steadycom-ecoli-auxotrophs/model-{id}.json"));
                let model = Model::from_json_str(&std::fs::read_to_string(path).unwrap()).unwrap();
                SpeciesSpec::new(id, Arc::new(model)).abundance(1.0 / n)
            })
            .collect();
        let community = CommunityModel::new(CommunitySpec { species, ..Default::default() }).unwrap();
        let t = Instant::now();
        let map = EditableMap::from_community(&community, &LayoutOptions::default());
        let secs = t.elapsed().as_secs_f64();
        let stats = map.stats();
        eprintln!(
            "big layouts: {name}: {} reactions, {} nodes laid out in {secs:.2} s (Node took {} s)",
            stats.reactions,
            stats.nodes,
            entry["nodeMs"].as_f64().unwrap_or(f64::NAN) / 1000.0
        );
        assert_map_eq(&map.to_json(), &expected, 0.0, name);
    }
}
