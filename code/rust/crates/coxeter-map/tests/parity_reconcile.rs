//! Opening a saved map over the model of the day — `pruneMapToModel`,
//! `reconcileCompartments` and `describeReconciliation` — against the
//! JavaScript (`rust/tools/fixtures-map.mjs reconcile`): the user's yeast
//! mitochondrion map over the BiGG-id Yeast9 (every reaction drawn afresh) and
//! over the native-id one (nothing redrawn), and bundled
//! scenarios with what a stale file gets wrong written into them. Exact.

mod common;

use std::collections::HashSet;

use coxeter_core::model::Model;
use coxeter_map::EditableMap;
use coxeter_map::map_document::{map_species_ids, parse_map_document_str, rename_map_species};
use coxeter_map::reconcile_map::{describe_reconciliation, prune_map_to_model, reconcile_compartments};
use common::*;

#[test]
fn reconciliation_matches_javascript() {
    let index = fixture("map/index-reconcile.json").expect("run: node rust/tools/fixtures-map.mjs reconcile");
    let (mut checked, mut skipped) = (0, 0);
    for entry in index.as_array().unwrap() {
        let name = entry["name"].as_str().unwrap();
        let Some(expected) = fixture(entry["file"].as_str().unwrap()) else {
            skipped += 1;
            continue;
        };
        let (community, doc) = if entry["kind"] == "upload" {
            let (Some(model), Some(input)) = (fixture(entry["model"].as_str().unwrap()), input_path(entry)) else {
                skipped += 1;
                continue;
            };
            let species_id = entry["speciesId"].as_str().unwrap();
            let community = single_species_community(species_id, Model::from_value(&model).unwrap());
            let mut parsed = parse_map_document_str(&std::fs::read_to_string(input).unwrap()).unwrap();
            let saved = map_species_ids(&parsed);
            let renamed_from = if saved.len() == 1 && saved[0] != species_id {
                parsed = rename_map_species(&parsed, &saved[0], species_id, entry["modelKey"].as_str());
                Some(saved[0].clone())
            } else {
                None
            };
            assert_eq!(serde_json::json!(renamed_from), expected["renamedFrom"], "{name}: renamed species");
            (community, parsed.map)
        } else {
            let Some(doc) = fixture(entry["inputFixture"].as_str().unwrap()) else {
                skipped += 1;
                continue;
            };
            (scenario_community(entry["scenario"].as_str().unwrap()), doc)
        };
        let keep: HashSet<String> =
            entry["keepSpecies"].as_array().map(|l| l.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();

        let mut map = EditableMap::from_json(&doc).unwrap();
        let contents = prune_map_to_model(&mut map, &community, &keep);
        assert_map_eq(&contents.to_json(), &expected["contents"], 0.0, &format!("{name}: prune report"));
        assert_map_eq(&edits::stats_json(&map), &expected["prunedStats"], 0.0, &format!("{name}: stats after pruning"));
        let shell = reconcile_compartments(&mut map, &community);
        assert_map_eq(&shell.to_json(), &expected["shell"], 0.0, &format!("{name}: shell report"));
        assert_eq!(describe_reconciliation(Some(&contents), Some(&shell)), expected["description"].as_str().unwrap(), "{name}: description");
        assert_map_eq(&map.to_json(), &expected["map"], 0.0, &format!("{name}: reconciled map"));
        checked += 1;
    }
    eprintln!("reconcile: {checked} compared, {skipped} skipped");
}
