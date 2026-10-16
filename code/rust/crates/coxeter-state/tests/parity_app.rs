//! The store against the JavaScript `Store` (fixtures written by
//! `rust/tools/fixtures-app.mjs`).
//!
//! Each case opens a scenario and replays the fixture's steps; after every
//! step the snapshot is compared with the one the JS store gave. Maps and the
//! flux-independent render state are compared through the SHA-256 of their
//! canonical JSON. The objective value is compared to 1e-6 (GLPK and HiGHS
//! agree on it; individual fluxes are not unique and are not compared).

// Let-chains are not available to the workspace's Rust 1.85.
#![allow(clippy::collapsible_if)]

mod common;

use coxeter_core::testkit::{json_diff, read_fixture};
use coxeter_io::reproducibility::sha256;
use coxeter_state::Store;
use serde_json::{Map, Value, json};

use common::{Harness, entity_json, init_store, step};

fn obj(m: impl IntoIterator<Item = (String, String)>) -> Value {
    Value::Object(m.into_iter().map(|(k, v)| (k, Value::String(v))).collect::<Map<_, _>>())
}

/// A map's JSON with `null` (and the two false-only segment flags) left out:
/// the form every reader of a map sees, as the fixtures digest it.
fn canonical_map(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.iter()
                .filter(|(k, x)| !x.is_null() && !((k.as_str() == "backboneAnchor" || k.as_str() == "transportLane") && **x == Value::Bool(false)))
                .map(|(k, x)| (k.clone(), canonical_map(x)))
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(canonical_map).collect()),
        other => other.clone(),
    }
}

/// The parts of `map_render_state()` that do not depend on the fluxes.
fn render_projection(store: &Store) -> Value {
    let reaction = store.reaction_coloring(false);
    let ribbon = store.pathway_ribbon_coloring(false);
    json!({
        "reactionColors": obj(reaction.colors.clone()),
        "pathwayRibbonColors": obj(ribbon.colors.clone()),
        "pathwayRibbonGroups": obj(ribbon.groups.clone()),
        "metaboliteLabels": obj(store.map_labels("metabolite")),
        "reactionLabels": obj(store.map_labels("reaction")),
    })
}

fn numbers(m: &indexmap::IndexMap<String, f64>) -> Value {
    Value::Object(m.iter().map(|(k, v)| (k.clone(), coxeter_state::util::num_json(*v))).collect())
}

fn strings(m: &indexmap::IndexMap<String, String>) -> Value {
    Value::Object(m.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect())
}

/// What the omics overlay, community omics and the explorer hold.
fn analysis_projection(store: &Store) -> Value {
    let overlay = store.omics_overlay();
    let o = &store.omics;
    json!({
        "overlay": {
            "enabled": overlay.enabled, "matchedReactions": overlay.matched_reactions, "totalDrawn": overlay.total_drawn,
            "mode": overlay.mode, "metric": overlay.metric, "conditionA": overlay.condition_a, "conditionB": overlay.condition_b,
            "values": numbers(&overlay.values), "colors": strings(&overlay.colors),
        },
        "communityOmics": {
            "enabled": o.enabled, "mode": o.mode, "conditionA": o.condition_a, "conditionB": o.condition_b, "metric": o.metric,
            "scores": numbers(&o.scores), "colors": strings(&o.colors), "gimmeEnabled": o.gimme_enabled,
            "report": o.report.as_ref().map(|r| json!({
                "inputRows": r.input_rows, "inputSpecies": r.input_species, "modelSpecies": r.model_species,
                "genesInModels": r.genes_in_models, "measuredGenes": r.measured_genes, "gprReactions": r.gpr_reactions,
                "mappedReactions": r.mapped_reactions, "unmatchedSpecies": r.unmatched_species,
            })),
        },
        "explorer": { "enabled": store.cross_feeding_explorer.enabled, "indexed": store.cross_feeding_explorer.index.is_some() },
    })
}

/// The Rust snapshot, field for field the JS one.
fn snapshot(store: &Store, search: Option<&str>, analysis: bool) -> Value {
    let map = store.map.as_ref().map(|m| m.to_json());
    let reaction = store.reaction_coloring(false);
    let ribbon = store.pathway_ribbon_coloring(false);
    let render = render_projection(store);
    let unsaved = store.unsaved_work();
    let sel = &store.multi_selection;
    let mut snap = json!({
        "state": store.state_value(),
        "map": map.as_ref().map(|m| json!({
            "digest": sha256(&canonical_map(m)),
            "cells": m["cells"].as_array().map_or(0, Vec::len),
            "nodes": m["nodes"].as_array().map_or(0, Vec::len),
            "reactions": m["reactions"].as_array().map_or(0, Vec::len),
            "segments": m["segments"].as_array().map_or(0, Vec::len),
            "texts": m["texts"].as_array().map_or(0, Vec::len),
            "width": m["canvas"]["width"],
            "height": m["canvas"]["height"],
        })),
        "render": {
            "digest": sha256(&render),
            "reactionColors": reaction.colors.len(),
            "ribbonGroups": ribbon.groups.len(),
            "metaboliteLabels": render["metaboliteLabels"].as_object().map_or(0, Map::len),
            "inactiveSpeciesIds": store.disabled_drawn_species_ids(),
            "currencyLabels": store.view.currency_labels || store.map_mode == "core-overview",
            "showCurrency": store.view.show_currency,
            "showCrossFeeding": store.view.show_cross_feeding,
        },
        "mapMode": store.map_mode,
        "mapLoadNote": store.map_load_note,
        "status": { "state": store.status().state.as_str(), "message": store.status().message },
        "unsaved": { "map": unsaved.map, "scenario": unsaved.scenario, "model": unsaved.model },
        "undoLabels": store.map_history.undo_labels(),
        "redoCount": store.map_history.redo_len(),
        "selectedReaction": store.selected_reaction,
        "selectedEntity": entity_json(store),
        "selectedGeometry": store.selected_geometry.as_ref().map(|g| json!({"cellId": g.cell_id, "compartmentId": g.compartment_id})),
        "multiSelection": { "nodes": sel.nodes, "reactions": sel.reactions, "texts": sel.texts },
        "community": store.community.as_ref().map(|c| json!({
            "reactions": c.merged.reactions.len(),
            "metabolites": c.merged.metabolites.len(),
            "active": c.active_species().map(|s| s.id.clone()).collect::<Vec<_>>(),
        })),
        "solution": store.solution.as_ref().map(|s| json!({"status": s.status.as_str(), "objectiveValue": s.objective_value})),
        "availableCells": store.available_map_cells().iter().map(|c| c.species_id.clone()).collect::<Vec<_>>(),
        "availableCompartments": store.available_map_compartments().iter()
            .map(|c| format!("{}|{}|{}", c.species_id, c.compartment_id, c.parent_id.clone().unwrap_or_else(|| "null".into())))
            .collect::<Vec<_>>(),
        "pathwayEntries": store.pathway_manager_entries().iter()
            .map(|e| json!({"key": e.key, "name": e.name, "color": e.color, "visible": e.visible, "count": e.count}))
            .collect::<Vec<_>>(),
        "exportBaseName": store.export_base_name(),
    });
    if let Some(q) = search {
        snap["search"] = store.search_reactions(q, 30).iter().map(|r| json!([r.reaction_id, r.score, r.drawn])).collect();
    }
    if analysis {
        snap["analysis"] = analysis_projection(store);
    }
    snap
}

/// Differences between a Rust and a JS snapshot: exact everywhere but the
/// objective value.
fn compare(rust: &Value, js: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let (Some(r), Some(j)) = (rust.as_object(), js.as_object()) else { return vec!["not objects".into()] };
    for (k, jv) in j {
        let Some(rv) = r.get(k) else {
            out.push(format!("{k}: missing in Rust"));
            continue;
        };
        if k == "solution" {
            if let Some(d) = json_diff(&rv["status"], &jv["status"], 0.0) {
                out.push(format!("solution.status{d}"));
            }
            let (a, b) = (rv["objectiveValue"].as_f64(), jv["objectiveValue"].as_f64());
            let ok = match (a, b) {
                (Some(a), Some(b)) => (a - b).abs() <= 1e-6 * a.abs().max(b.abs()).max(1.0),
                (a, b) => a.is_none() && b.is_none() || jv.is_null() == rv.is_null(),
            };
            if !ok {
                out.push(format!("solution.objectiveValue: {:?} != {:?}", a, b));
            }
            continue;
        }
        if k == "status" {
            // The solver names itself in a failed solve's message ("GLPK
            // status 4" / "HiGHS status infeasible"); the state must agree.
            let (rm, jm) = (rv["message"].as_str().unwrap_or(""), jv["message"].as_str().unwrap_or(""));
            let solver_message = rm.starts_with("HiGHS status ") && jm.starts_with("GLPK status ");
            if solver_message {
                if rv["state"] != jv["state"] {
                    out.push(format!("status.state: {} != {}", rv["state"], jv["state"]));
                }
                continue;
            }
        }
        if let Some(d) = json_diff(rv, jv, 0.0) {
            out.push(format!("{k}{}", d.trim_start_matches('$')));
        }
    }
    out
}

#[test]
fn the_store_matches_the_javascript_store_step_by_step() {
    let index = read_fixture("app/index.json").expect("run rust/tools/fixtures-app.mjs");
    let mut failures: Vec<String> = Vec::new();
    let mut compared = 0;
    for name in index["cases"].as_array().unwrap() {
        let case = read_fixture(&format!("app/{}", name.as_str().unwrap())).unwrap();
        // `node rust/tools/fixtures-app.mjs --full` also writes every map and
        // render state in full: with them, a digest mismatch names the path.
        let large = read_fixture(&format!("large/app/{}", name.as_str().unwrap()));
        let full_of = |at: usize| large.as_ref().map(|l| l["snapshots"][at]["full"].clone());
        let mut store = init_store();
        let mut harness = Harness::default();
        let search = case["search"].as_str();
        let analysis = case["analysis"].as_bool().unwrap_or(false);
        let snapshots = case["snapshots"].as_array().unwrap();
        let mut at = 0;
        let check = |store: &Store, at: usize, label: &str, result: Option<Result<(), coxeter_state::StoreError>>, failures: &mut Vec<String>| {
            let expected = &snapshots[at];
            if let Some(error) = expected.get("error") {
                match result {
                    Some(Err(e)) if Value::String(e.to_string()) == *error => {}
                    // The fixtures predate 80e0611: a map that does not fit is kept
                    // waiting for its model, for the same reason, instead of
                    // refused (tests/js_waiting_map.rs).
                    Some(Ok(())) if store.waiting_map.as_ref().is_some_and(|w| Value::String(w.reason.clone()) == *error) => {}
                    Some(Err(e)) => failures.push(format!("{name} {label}: error {e:?} != {error}")),
                    _ => failures.push(format!("{name} {label}: expected the error {error}")),
                }
                return;
            }
            if let Some(Err(e)) = result {
                failures.push(format!("{name} {label}: unexpected error {e}"));
                return;
            }
            let snap = snapshot(store, search, analysis);
            // The fixtures predate d7b2c0e: boundary projections go through the
            // ellipse form of closestPerimeterPoint, which moves layout
            // coordinates by a last bit or two, and the digest is exact. The
            // map's counts and size are still compared (coxeter-map's parity
            // tests compare the maps themselves, to 1e-9).
            let mut expected = expected.clone();
            if expected["map"].is_object() && snap["map"].is_object() {
                expected["map"]["digest"] = snap["map"]["digest"].clone();
            }
            // The fixtures predate 7b5aaa0: the Scenario JSON keeps the three line
            // switches (tests/js_line_styles.rs).
            let mut snap = snap;
            if let Some(state) = snap["state"].as_object_mut() {
                for key in ["widthByFlux", "dashNoFlux", "lineHops"] {
                    if expected["state"].get(key).is_none() {
                        state.remove(key);
                    }
                }
            }
            let expected = &expected;
            for d in compare(&snap, expected) {
                failures.push(format!("{name} {label}: {d}"));
            }
            if let Some(full) = full_of(at) {
                if snap["map"]["digest"] != snapshots[at]["map"]["digest"] {
                    // To the last bits of a boundary projection (see above).
                    let map = store.map.as_ref().map(|m| canonical_map(&m.to_json())).unwrap_or(Value::Null);
                    if let Some(d) = json_diff(&map, &canonical_map(&full["map"]), 1e-9) {
                        failures.push(format!("{name} {label}: map{}", d.trim_start_matches('$')));
                    }
                }
                if snap["render"]["digest"] != expected["render"]["digest"] {
                    if let Some(d) = json_diff(&render_projection(store), &full["render"], 0.0) {
                        failures.push(format!("{name} {label}: render{}", d.trim_start_matches('$')));
                    }
                }
            }
        };
        if let Some(scenario) = case["scenario"].as_str() {
            let r = store.use_scenario(scenario);
            check(&store, at, "open", Some(r), &mut failures);
            at += 1;
            compared += 1;
        }
        for (i, s) in case["steps"].as_array().unwrap().iter().enumerate() {
            let s = s.as_array().unwrap();
            let r = step(&mut store, &mut harness, s);
            let label = format!("step {i} {}", s[0]);
            check(&store, at, &label, Some(r), &mut failures);
            at += 1;
            compared += 1;
        }
        assert_eq!(at, snapshots.len(), "{name}: snapshots");
    }
    eprintln!("compared {compared} snapshots");
    if !failures.is_empty() {
        let shown: Vec<&String> = failures.iter().take(200).collect();
        panic!("{} differences from the JS store:\n{}", failures.len(), shown.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n"));
    }
}
