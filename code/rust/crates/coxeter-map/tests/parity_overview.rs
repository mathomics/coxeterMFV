//! The core-metabolism overview, the Yeast9 preview and the reactions inside
//! a compartment, against the JavaScript
//! (`rust/tools/fixtures-map.mjs overview yeast9`). Exact.

mod common;

use coxeter_core::model::Model;
use coxeter_map::EditableMap;
use coxeter_map::compartment_reactions::reactions_in_compartment;
use coxeter_map::core_metabolism_overview::{CORE_OVERVIEW_GROUPS, build_core_metabolism_overview, core_overview_support};
use coxeter_map::layout::LayoutOptions;
use coxeter_map::layout_settings::{RawLayoutSettings, layout_engine_options, layout_preset};
use coxeter_map::yeast9::{populate_yeast9_preview, yeast9_preview_specs, yeast_preview_anchors, YEAST9_CANVAS};
use common::*;
use serde_json::{Value, json};

fn settings(v: &Value) -> RawLayoutSettings {
    RawLayoutSettings {
        preset: v["preset"].as_str().map(String::from),
        cell_spacing: v["cellSpacing"].as_f64(),
        minimum_compartment_size: v["minimumCompartmentSize"].as_f64(),
        node_spacing: v["nodeSpacing"].as_f64(),
        cell_area_multiplier: v["cellAreaMultiplier"].as_f64(),
    }
}

fn support_json(s: coxeter_map::core_metabolism_overview::OverviewSupport) -> Value {
    json!({ "supported": s.supported, "coverage": s.coverage, "groups": s.groups })
}

#[test]
fn core_overview_matches_javascript() {
    let index = fixture("map/index-overview.json").expect("run: node rust/tools/fixtures-map.mjs overview");
    let mut checked = 0;
    for entry in index.as_array().unwrap() {
        let file = entry["file"].as_str().unwrap();
        let Some(expected) = fixture(file) else { continue };
        let community = scenario_community(entry["scenario"].as_str().unwrap());
        let built = build_core_metabolism_overview(&community, &settings(&entry["settings"]));
        assert_map_eq(&built.map.to_json(), &expected["map"], 0.0, file);
        let groups: Vec<Value> = built
            .pathway_groups
            .iter()
            .map(|g| json!({ "id": g.id, "speciesId": g.species_id, "groupId": g.group_id, "name": g.name, "reactionIds": g.reaction_ids }))
            .collect();
        assert_map_eq(&json!(groups), &expected["pathwayGroups"], 0.0, &format!("{file} pathway groups"));
        assert_eq!(json!(built.placed), expected["placed"]);
        let per_cell: Vec<Value> = built
            .per_cell
            .iter()
            .map(|(sp, g)| json!({ "speciesId": sp, "groups": g.iter().map(|(id, n)| json!({ "id": id, "count": n })).collect::<Vec<_>>() }))
            .collect();
        assert_map_eq(&json!(per_cell), &expected["perCell"], 0.0, &format!("{file} per cell"));
        assert_map_eq(&support_json(built.support), &expected["support"], 0.0, &format!("{file} support"));
        checked += 1;
    }
    assert!(checked >= 1);

    let fx = fixture("map/overview-support.json").unwrap();
    let groups: Vec<Value> = CORE_OVERVIEW_GROUPS.iter().map(|g| json!({ "id": g.id, "label": g.label, "reactions": g.reactions })).collect();
    assert_eq!(json!(groups), fx["groups"]);
    let mut communities = std::collections::HashMap::new();
    for case in fx["support"].as_array().unwrap() {
        let id = case["scenario"].as_str().unwrap();
        let c = communities.entry(id.to_string()).or_insert_with(|| scenario_community(id));
        assert_map_eq(&support_json(core_overview_support(c)), &case["support"], 0.0, &format!("support of {id}"));
    }
    for case in fx["inCompartment"].as_array().unwrap() {
        let c = &communities[case["scenario"].as_str().unwrap()];
        let got = reactions_in_compartment(c, case["cellId"].as_str().unwrap(), case["compartmentId"].as_str().unwrap());
        assert_eq!(json!(got), case["reactions"], "reactionsInCompartment {case}");
    }
}

#[test]
fn yeast9_preview_matches_javascript() {
    let Some(index) = fixture("map/index-yeast9.json") else { return };
    for entry in index.as_array().unwrap() {
        let (Some(model), Some(expected)) = (fixture(entry["model"].as_str().unwrap()), fixture(entry["file"].as_str().unwrap())) else {
            eprintln!("yeast9: skipped (large fixtures absent)");
            continue;
        };
        let model = Model::from_value(&model).unwrap();
        let specs: Vec<Value> = yeast9_preview_specs(&model)
            .iter()
            .map(|s| json!({ "reactionId": s.reaction_id, "compartment": s.compartment, "group": s.group, "index": s.index, "count": s.count }))
            .collect();
        assert_eq!(json!(specs), expected["specs"]);

        let community = single_species_community("sce", model.clone());
        let engine = layout_engine_options(&RawLayoutSettings::from(&layout_preset(Some("spacious"))));
        let opts = LayoutOptions { width: Some(YEAST9_CANVAS.0), height: Some(YEAST9_CANVAS.1), ..engine };
        let mut map = EditableMap::empty_from_community(&community, &opts);
        assert_map_eq(&map.to_json(), &expected["shell"], 0.0, "the Yeast9 shell");
        for (compartment, anchors) in expected["anchors"].as_object().unwrap() {
            let got: Vec<Value> = yeast_preview_anchors(&map, "sce", compartment, anchors.as_array().unwrap().len())
                .iter()
                .map(|p| json!({ "x": p.x, "y": p.y }))
                .collect();
            assert_map_eq(&json!(got), anchors, 0.0, &format!("anchors in {compartment}"));
        }
        let preview = populate_yeast9_preview(&mut map, &community, &model, "sce");
        assert_eq!(json!(preview.placed), expected["placed"]);
        assert_eq!(json!(preview.covered_compartments), expected["coveredCompartments"]);
        assert_eq!(json!(preview.missing), expected["missing"]);
        assert_map_eq(&map.to_json(), &expected["map"], 0.0, "the Yeast9 preview map");
    }
}
