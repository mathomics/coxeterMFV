//! Port of the Store tests of `test/label-sources.test.mjs`.
//!
//! "rendered labels use the selected sources and retain flux and FVA values"
//! hands `renderMap` the labels the store gives (`store.mapLabels`) with a
//! flux and an FVA range of its own: here through `coxeter_render`, with the
//! store's render state (`map_render_state`) carrying those same labels and
//! the test's flux, label mode and FVA range put in it.
//!
//! Not ported here:
//! - "model vectors and database annotations survive model and community
//!   round trips" — `Model` and `CommunityModel` only; the store `fixture()`
//!   builds is not used.
//! - "a base label reads as a name where the model id does not" —
//!   `readableBase` of `src/core/label-sources.js` only (crate
//!   `coxeter-core`).
//! - "View and both inspectors offer the available sources with an inherit
//!   option" — every assertion is on the HTML `renderViewControls` and
//!   `renderInspector` return (UI panels, another crate).

mod common;

use std::sync::Arc;

use coxeter_core::community::{CommunityModel, CommunitySpec, SpeciesSpec};
use coxeter_map::EditableMap;
use coxeter_map::layout::LayoutOptions;
use coxeter_map::map_document::parse_map_document_str;
use coxeter_map::place_reaction::PlaceOptions;
use coxeter_render::mapview::render_map_svg;
use coxeter_render::state::{FvaRange, LabelMode};
use coxeter_state::{RenderOptions, Species, Store};
use serde_json::{Value, json};

use common::{model, store};

/// `fixture()`: a model with COBRA name / database vectors and annotations,
/// one reaction of it drawn, and a store over the map and its community.
/// Returns the store and the ids of the nodes of `a_c` and `b_c`.
fn fixture() -> (Store, String, String) {
    let model = model(json!({
        "id": "labels",
        "compartments": { "c": "Cytosol" },
        "metabolites": [
            { "id": "a_c", "name": "Original A", "compartment": "c", "annotation": { "bigg.metabolite": ["a"], "kegg.compound": ["C00001"], "metanetx.chemical": ["MNXM1"] } },
            { "id": "b_c", "name": "Original B", "compartment": "c", "annotation": { "bigg.metabolite": ["b"], "metacyc.compound": ["B-META"], "seed.compound": ["cpd00002"] } },
        ],
        "metNames": ["Alternative A", "Alternative B"],
        "metModelSEEDID": ["cpd00001", "cpd00002"],
        "metFormulas": ["H2O", "CO2"],
        "reactions": [
            { "id": "R", "name": "Original reaction", "metabolites": { "a_c": -1, "b_c": 1 }, "lower_bound": 0, "upper_bound": 1000, "annotation": { "bigg.reaction": ["RXN-A"], "kegg.reaction": ["R00001"], "kegg.pathway": ["path:map00010"] } },
        ],
        "rxnNames": ["Alternative reaction"],
        "rxnMetaNetXID": ["MNXR1"],
        "rxnMetaCycID": ["RXN-META"],
        "rxnModelSEEDID": ["rxn00001"],
    }));
    let community = Arc::new(
        CommunityModel::new(CommunitySpec { species: vec![SpeciesSpec::new("sp", Arc::new(model.clone())).abundance(1.0)], ..Default::default() })
            .unwrap(),
    );
    let mut map = EditableMap::empty_from_community(&community, &LayoutOptions::default());
    map.add_reaction(&community, "sp@R", &PlaceOptions::default()).expect("the reaction is drawn");
    let node_of = |merged: &str| map.nodes().values().find(|n| n.merged_id.as_deref() == Some(merged)).expect("the node").id.clone();
    let (node, other) = (node_of("sp@a_c"), node_of("sp@b_c"));
    let mut store = store();
    store.community = Some(community);
    store.map = Some(map);
    // `{ id: 'sp', file: 'labels' }`: no abundance, and enabled by default.
    store.species = vec![Species::from_json(&json!({ "id": "sp", "file": "labels" }))];
    store.set_model("labels", model);
    (store, node, other)
}

/// `JSON.stringify([model.toJSON(), community.merged.toJSON()])`.
fn biology(store: &Store) -> Value {
    json!([store.models.get("labels").unwrap().to_json(), store.community.as_ref().unwrap().merged.to_json()])
}

/// "global label choices allow per-instance overrides and fall back for missing annotations"
#[test]
fn global_label_choices_allow_per_instance_overrides_and_fall_back_for_missing_annotations() {
    let (mut store, node, other) = fixture();
    let biology_before = biology(&store);
    assert_eq!(store.map_label_text("metabolite", &node, None), "a");
    store.set_map_label_source("metabolite", "kegg", None);
    assert_eq!(store.map_label_text("metabolite", &node, None), "C00001");
    assert_eq!(store.map_label_text("metabolite", &other, None), "b");
    store.set_map_label_source("metabolite", "name", Some(&node));
    assert_eq!(store.map_label_text("metabolite", &node, None), "Alternative A");
    store.set_map_label_source("metabolite", "inherit", Some(&node));
    assert_eq!(store.map_label_text("metabolite", &node, None), "C00001");
    store.set_map_label_source("reaction", "metanetx", None);
    assert_eq!(store.map_label_text("reaction", "sp@R", None), "MNXR1");
    store.set_map_label_source("reaction", "name", Some("sp@R"));
    assert_eq!(store.map_label_text("reaction", "sp@R", None), "Alternative reaction");
    assert_eq!(biology(&store), biology_before);
}

/// "global and individual label choices persist in map JSON and undo independently"
#[test]
fn global_and_individual_label_choices_persist_in_map_json_and_undo_independently() {
    let (mut store, node, _) = fixture();
    store.set_map_label_source("metabolite", "bigg", None);
    store.set_map_label_source("metabolite", "name", Some(&node));
    store.set_map_label_source("reaction", "modelseed", Some("sp@R"));
    let parsed = parse_map_document_str(&store.map_json().unwrap()).unwrap();
    let restored = EditableMap::from_json(&parsed.map).unwrap();
    assert_eq!(restored.label_settings().and_then(|s| s.get("metabolite")).map(String::as_str), Some("bigg"));
    assert_eq!(restored.node(&node).unwrap().extra.get("labelSource"), Some(&json!("name")));
    assert_eq!(restored.reaction("sp@R").unwrap().extra.get("labelSource"), Some(&json!("modelseed")));
    store.undo_map();
    assert_eq!(store.map.as_ref().unwrap().reaction("sp@R").unwrap().extra.get("labelSource"), None);
    assert_eq!(store.map_label_text("metabolite", &node, None), "Alternative A");
    store.redo_map();
    assert_eq!(store.map_label_text("reaction", "sp@R", None), "rxn00001");
}

/// "rendered labels use the selected sources and retain flux and FVA values"
#[test]
fn rendered_labels_use_the_selected_sources_and_retain_flux_and_fva_values() {
    let (mut store, node, _) = fixture();
    store.set_map_label_source("metabolite", "name", None);
    store.set_map_label_source("reaction", "kegg", None);
    let mut state = store.map_render_state(RenderOptions::default());
    state.labels = LabelMode::All;
    state.fluxes = [("sp@R".to_string(), 2.0)].into_iter().collect();
    let metabolite_labels = store.map_labels("metabolite");
    assert_eq!(metabolite_labels.get(&node).map(String::as_str), Some("Alternative A"));
    assert_eq!(state.metabolite_labels.get(&node).map(String::as_str), Some("Alternative A"), "the store hands the renderer the same labels");
    let graph = store.map.as_ref().unwrap().graph();
    let svg = render_map_svg(graph, &state);
    assert!(svg.contains(">Alternative A</text>"), "the metabolite is labelled by its model name");
    assert!(svg.contains(">R00001  2</text>"), "the reaction by its KEGG id and its flux");
    state.fva_enabled = true;
    state.fva_ranges = [("sp@R".to_string(), FvaRange { min: 1.0, max: 3.0 })].into_iter().collect();
    let fva = render_map_svg(graph, &state);
    assert!(fva.contains(">R00001  [1, 3]</text>"), "and by its FVA range");
}

/// "cofactors and their labels start enabled while cross-feeding starts disabled"
#[test]
fn cofactors_and_their_labels_start_enabled_while_cross_feeding_starts_disabled() {
    let store = store();
    assert!(store.view.show_currency);
    assert!(store.view.currency_labels);
    assert!(!store.view.show_cross_feeding);
}
