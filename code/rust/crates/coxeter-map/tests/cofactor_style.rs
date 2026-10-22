//! The cofactor style a map carries (`cofactorStyle`, src/map/editable-map.js):
//! read, written back, switched, and dropped when unknown.

mod common;

use coxeter_core::testkit::repo_root;
use coxeter_map::EditableMap;
use serde_json::Value;

fn teaching_map(id: &str) -> Value {
    let path = repo_root().join(format!("models/maps/{id}.json"));
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    doc["map"].clone()
}

#[test]
fn a_teaching_map_reads_as_quiet_and_writes_back_as_its_builder_wrote_it() {
    for id in ["solo-ecoli", "metanogenesis"] {
        let saved = teaching_map(id);
        let map = EditableMap::from_json(&saved).unwrap();
        assert_eq!(map.cofactor_style(), Some("quiet"), "{id}");
        let diff = common::map_diff(&map.to_json(), &saved, 0.0);
        assert!(diff.is_none(), "{id}: the JSON round trip changes {diff:?}");
    }
}

#[test]
fn the_style_is_switched_off_and_on_and_an_unknown_one_is_the_default() {
    let mut map = EditableMap::from_json(&teaching_map("solo-ecoli")).unwrap();
    assert_eq!(map.set_cofactor_style(None), None);
    assert!(map.to_json().get("cofactorStyle").is_none(), "a map without it does not mention it");
    assert_eq!(map.set_cofactor_style(Some("quiet")), Some("quiet"));
    assert_eq!(map.to_json()["cofactorStyle"], "quiet");
    assert_eq!(map.set_cofactor_style(Some("loud")), None);
    assert_eq!(map.cofactor_style(), None);

    let mut saved = teaching_map("solo-ecoli");
    saved["cofactorStyle"] = Value::from("loud");
    assert_eq!(EditableMap::from_json(&saved).unwrap().cofactor_style(), None);
}
