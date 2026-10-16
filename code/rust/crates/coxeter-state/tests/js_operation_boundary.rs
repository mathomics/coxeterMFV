//! Port of `test/operation-boundary.test.mjs` (`src/ui/operation-boundary.js`,
//! ported as `coxeter_state::operation_boundary`).
//!
//! The JS imports `EDIT_OPERATION_DOMAIN`; here the two registries
//! (`MAP_OPERATIONS`, `MODEL_OPERATIONS`) are what it is built from.

use std::sync::Arc;

use coxeter_core::community::{CommunityModel, CommunitySpec, SpeciesSpec};
use coxeter_core::model::Model;
use coxeter_core::testkit::repo_root;
use coxeter_map::editable_map::{DuplicateOptions, EditableMap};
use coxeter_map::layout::LayoutOptions;
use coxeter_map::place_reaction::PlaceOptions;
use coxeter_state::operation_boundary::{
    MAP_OPERATIONS, MODEL_OPERATIONS, OperationDomain, assert_operation_domain, is_map_operation, is_model_operation, operation_domain,
};
use indexmap::IndexMap;
use serde_json::json;

fn fixture() -> (Arc<Model>, CommunityModel, EditableMap) {
    let text = std::fs::read_to_string(repo_root().join("models/ecoli_core_mc.json")).unwrap();
    let model = Arc::new(Model::from_json_str(&text).unwrap());
    let community = CommunityModel::new(CommunitySpec {
        species: vec![SpeciesSpec { abundance: Some(1.0), ..SpeciesSpec::new("eco", model.clone()) }],
        medium: IndexMap::new(),
        ..CommunitySpec::default()
    })
    .unwrap();
    let map = EditableMap::empty_from_community(&community, &LayoutOptions::default());
    (model, community, map)
}

fn biological_snapshot(model: &Model, community: &CommunityModel) -> String {
    json!({
        "model": model.to_json(),
        "reactions": community.merged.reactions.keys().collect::<Vec<_>>(),
        "merged": community.merged.to_json(),
    })
    .to_string()
}

#[test]
fn editing_operations_have_an_explicit_map_or_model_domain() {
    assert_eq!(operation_domain("addMapReaction"), Some(OperationDomain::Map));
    assert_eq!(operation_domain("moveNode"), Some(OperationDomain::Map));
    assert_eq!(operation_domain("duplicateMapMetabolite"), Some(OperationDomain::Map));
    assert_eq!(operation_domain("createModelMetabolite"), Some(OperationDomain::Model));
    assert_eq!(operation_domain("createModelReaction"), Some(OperationDomain::Model));
    assert_eq!(operation_domain("unknownOperation"), None);

    for name in MAP_OPERATIONS.iter().chain(MODEL_OPERATIONS.iter()) {
        assert!(matches!(operation_domain(name), Some(OperationDomain::Map | OperationDomain::Model)));
    }
}

#[test]
fn domain_guards_reject_crossing_the_build_model_boundary() {
    assert!(is_map_operation("removeMapNode"));
    assert!(is_model_operation("createModelReaction"));
    assert!(assert_operation_domain("addMapReaction", OperationDomain::Map).unwrap());
    let crossing = assert_operation_domain("createModelReaction", OperationDomain::Map).unwrap_err().to_string();
    assert!(crossing.contains("belongs to MODEL, not MAP"), "{crossing}");
    let unknown = assert_operation_domain("futureEdit", OperationDomain::Map).unwrap_err().to_string();
    assert!(unknown.contains("has no MAP/MODEL domain"), "{unknown}");
}

#[test]
fn representative_map_edits_cannot_mutate_the_biological_model() {
    let (model, community, mut map) = fixture();
    let before = biological_snapshot(&model, &community);
    let rid = community.merged.reactions.keys().find(|id| !map.has_reaction(id)).cloned().unwrap();

    let added = map.add_reaction(&community, &rid, &PlaceOptions::default());
    assert!(added.is_some());

    let movable = map.graph().nodes.values().find(|n| map.can_move_node(&n.id)).map(|n| (n.id.clone(), n.x, n.y));
    if let Some((id, x, y)) = movable {
        map.move_node(&id, x + 20.0, y + 10.0);
    }
    let source = map.graph().nodes.values().find(|n| n.merged_id.is_some()).map(|n| n.id.clone());
    if let Some(id) = source {
        map.duplicate_metabolite_node(&id, &DuplicateOptions::default());
    }
    map.remove_reaction(&rid);

    assert_eq!(biological_snapshot(&model, &community), before);
}

#[test]
fn map_serialization_contains_drawing_state_not_a_rewritten_biological_model() {
    let (model, community, mut map) = fixture();
    let before_model = model.to_json().to_string();
    let rid = community.merged.reactions.keys().next().cloned().unwrap();
    map.add_reaction(&community, &rid, &PlaceOptions::default());
    let document = map.to_json();
    assert!(document.is_object());
    assert_eq!(model.to_json().to_string(), before_model);
}
