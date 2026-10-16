//! The saved map document — `createMapDocument`, `parseMapDocument`,
//! `validateMapContext`, `mapSpeciesIds`, `renameMapSpecies` and the flux
//! colour stops they carry — against the JavaScript
//! (`rust/tools/fixtures-map.mjs documents`).

mod common;

use coxeter_map::EditableMap;
use coxeter_map::editable_map::TextOptions;
use coxeter_map::layout::LayoutOptions;
use coxeter_map::map_document::*;
use common::*;
use serde_json::{Value, json};

fn context(v: &Value) -> MapContext {
    MapContext {
        species: v["species"]
            .as_array()
            .map(|l| {
                l.iter()
                    .map(|sp| ContextSpecies {
                        id: sp["id"].as_str().unwrap_or("").into(),
                        file: sp["file"].as_str().map(String::from),
                        model: sp["model"].as_str().map(String::from),
                        enabled: sp["enabled"].as_bool(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        scenario: v["scenario"].as_str().map(String::from),
        pathway_manager: v.get("pathwayManager").cloned(),
        flux_color_stops: v.get("fluxColorStops").cloned(),
        study: v.get("study").cloned(),
    }
}

fn outcome<T>(r: Result<T, MapDocumentError>, show: impl Fn(T) -> Value) -> Value {
    match r {
        Ok(v) => json!({ "ok": show(v) }),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

fn stops_json(s: Option<Vec<FluxColorStop>>) -> Value {
    match s {
        None => Value::Null,
        Some(l) => Value::Array(l.iter().map(|s| json!({ "value": s.value, "color": s.color })).collect()),
    }
}

#[test]
fn map_documents_match_javascript() {
    let fx = fixture("map/documents.json").expect("run: node rust/tools/fixtures-map.mjs documents");
    let community = scenario_community("solo-ecoli");
    let reactions: Vec<String> = ["eco@PGI", "eco@PFK", "eco@GLCptspp", "EX_glc__D_s"].iter().map(|s| s.to_string()).collect();
    let mut map = EditableMap::from_community(&community, &LayoutOptions { reactions: Some(reactions), ..Default::default() });
    map.add_text("Title", 100.0, 100.0, &TextOptions::default());
    map.set_label_source("metabolite", "name");
    assert_map_eq(&map.to_json(), &fx["map"], 0.0, "the map the documents are made from");

    // createMapDocument, with the time the JavaScript stamped.
    for (key, ctx) in [("created", context(&fx["context"])), ("plain", MapContext::default())] {
        let at = fx[key]["metadata"]["createdAt"].as_str().unwrap();
        let doc = create_map_document_at(&map, &ctx, at).unwrap();
        assert_map_eq(&doc, &fx[key], 0.0, &format!("createMapDocument ({key})"));
    }
    let stamped = create_map_document(&map, &MapContext::default()).unwrap();
    let at = stamped["metadata"]["createdAt"].as_str().unwrap();
    assert!(at.len() == 24 && at.ends_with('Z') && &at[10..11] == "T", "ISO timestamp, got {at}");

    // parseMapDocument, on documents and on everything that is not one.
    for (i, case) in fx["parses"].as_array().unwrap().iter().enumerate() {
        let input = &case["input"];
        if input.is_string() {
            // JavaScript reads a string as JSON text and throws the engine's
            // own SyntaxError; only that it fails is comparable.
            assert!(case["result"].get("error").is_some());
            assert!(parse_map_document_str(input.as_str().unwrap()).is_err());
            continue;
        }
        let got = outcome(parse_map_document(input), |p| p.to_json());
        assert_map_eq(&got, &case["result"], 0.0, &format!("parseMapDocument case {i}"));
    }
    for case in fx["texts"].as_array().unwrap() {
        let text = case["text"].as_str().unwrap();
        let got = parse_map_document_str(text);
        match case["result"].get("ok") {
            Some(ok) => assert_map_eq(&got.unwrap().to_json(), ok, 0.0, "parseMapDocument(text)"),
            None => assert!(got.is_err()),
        }
    }

    for (i, case) in fx["validations"].as_array().unwrap().iter().enumerate() {
        let got = outcome(validate_map_context(&parsed_from_json(&case["parsed"]), &context(&case["current"])), |b| json!(b));
        assert_map_eq(&got, &case["result"], 0.0, &format!("validateMapContext case {i}"));
    }
    for (i, case) in fx["speciesIds"].as_array().unwrap().iter().enumerate() {
        assert_eq!(json!(map_species_ids(&parsed_from_json(&case["parsed"]))), case["result"], "mapSpeciesIds case {i}");
    }
    for (i, case) in fx["renames"].as_array().unwrap().iter().enumerate() {
        let parsed = parsed_from_json(&case["parsed"]);
        let renamed = rename_map_species(&parsed, case["from"].as_str().unwrap(), case["to"].as_str().unwrap(), case["options"]["model"].as_str());
        assert_map_eq(&renamed.to_json(), &case["result"], 0.0, &format!("renameMapSpecies case {i}"));
    }
    for (i, case) in fx["stops"].as_array().unwrap().iter().enumerate() {
        let got = outcome(normalize_flux_color_stops(&case["input"]), stops_json);
        assert_map_eq(&got, &case["result"], 0.0, &format!("normalizeFluxColorStops case {i}"));
    }
}
