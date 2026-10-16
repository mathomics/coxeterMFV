//! Port of `test/pathway-ribbons.test.mjs` (the Store parts).
//!
//! Not ported here:
//! - "pathway ribbon mode validates supported values":
//!   `normalizePathwayRibbonMode` only, no Store; a unit test in
//!   `coxeter-render/src/reaction_colors.rs`.
//! - "ribbons render behind reaction segments and preserve flux colour" and
//!   "unannotated reactions do not receive a ribbon": `renderMap` with a
//!   hand-built state, no Store; ported in `coxeter-render/tests/js_tests.rs`.

mod common;

use coxeter_render::reaction_colors::PathwayRibbonMode;
use serde_json::json;

use common::{store, view_patch};

#[test]
fn store_clamps_pathway_ribbon_width_and_opacity() {
    let mut store = store();
    store
        .set_view(view_patch(&json!({"pathwayRibbonMode": "pathway", "pathwayRibbonWidth": 500, "pathwayRibbonOpacity": 1})))
        .unwrap();
    assert_eq!(store.view.pathway_ribbon_mode, PathwayRibbonMode::Pathway);
    assert_eq!(store.view.pathway_ribbon_width, 120.0);
    assert_eq!(store.view.pathway_ribbon_opacity, 0.45);
    store.set_view(view_patch(&json!({"pathwayRibbonWidth": 1, "pathwayRibbonOpacity": 0}))).unwrap();
    assert_eq!(store.view.pathway_ribbon_width, 20.0);
    assert_eq!(store.view.pathway_ribbon_opacity, 0.04);
}

#[test]
fn scenario_state_persists_pathway_ribbon_settings() {
    let mut store = store();
    store.view.pathway_ribbon_mode = PathwayRibbonMode::Subsystem;
    store.view.pathway_ribbon_width = 72.0;
    store.view.pathway_ribbon_opacity = 0.21;
    let doc: serde_json::Value = serde_json::from_str(&store.state_json()).unwrap();
    assert_eq!(doc["pathwayRibbonMode"], json!("subsystem"));
    assert_eq!(doc["pathwayRibbonWidth"], json!(72));
    assert_eq!(doc["pathwayRibbonOpacity"], json!(0.21));
}
