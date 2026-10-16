//! `EditableMap.fromJSON(...).toJSON()` of every saved map — the gut9
//! snapshots and the user's own maps — against the JavaScript
//! (`rust/tools/fixtures-map.mjs maps`). Exact.

mod common;

use coxeter_map::EditableMap;
use coxeter_map::map_document::parse_map_document_str;
use common::*;

#[test]
fn saved_maps_round_trip_like_javascript() {
    let index = fixture("map/index-maps.json").expect("run: node rust/tools/fixtures-map.mjs maps");
    let (mut checked, mut skipped) = (0, 0);
    for entry in index.as_array().unwrap() {
        let file = entry["file"].as_str().unwrap();
        let (Some(input), Some(expected)) = (input_path(entry), fixture(file)) else {
            skipped += 1;
            continue;
        };
        let text = std::fs::read_to_string(&input).unwrap();
        let parsed = parse_map_document_str(&text).unwrap_or_else(|e| panic!("{}: {e}", input.display()));
        let map = EditableMap::from_json(&parsed.map).unwrap_or_else(|e| panic!("{}: {e}", input.display()));
        assert_map_eq(&map.to_json(), &expected, 0.0, file);
        checked += 1;
    }
    eprintln!("maps: {checked} compared, {skipped} skipped");
}
