//! `EditableMap.fromCommunity(...).toJSON()` for every bundled scenario, at
//! the default settings, each preset and with contents 'none', against the
//! JavaScript (`rust/tools/fixtures-map.mjs layouts`). Exact but for the last
//! bits of boundary projections ([`common::PROJECTION_TOL`]).

mod common;

use coxeter_map::EditableMap;
use common::*;

#[test]
fn layouts_match_javascript_exactly() {
    let index = fixture("map/index-layouts.json").expect("run: node rust/tools/fixtures-map.mjs layouts");
    let mut checked = 0;
    let mut skipped = 0;
    let mut communities = std::collections::HashMap::new();
    for entry in index.as_array().unwrap() {
        let file = entry["file"].as_str().unwrap();
        let Some(expected) = fixture(file) else {
            skipped += 1;
            continue;
        };
        let scenario = entry["scenario"].as_str().unwrap();
        let community = communities.entry(scenario.to_string()).or_insert_with(|| scenario_community(scenario));
        let opts = layout_options(&entry["options"]);
        let map = if entry["settings"] == "none" {
            EditableMap::empty_from_community(community, &opts)
        } else {
            EditableMap::from_community(community, &opts)
        };
        assert_map_eq(&map.to_json(), &expected, PROJECTION_TOL, file);
        checked += 1;
    }
    eprintln!("layouts: {checked} compared, {skipped} skipped (large fixtures absent)");
    assert!(checked >= 7);
}
