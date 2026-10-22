//! Scripted edit sequences — reactions placed in every direction, moved,
//! turned, flipped, mirrored, aligned, tidied and kept straight; nodes
//! dragged, copied and merged; selections moved, aligned and distributed;
//! cells, compartments, the environment and the canvas reshaped; texts;
//! removal, subsets and a save-and-reopen — replayed on the Rust map and
//! compared with the JavaScript map after every step. Exact.
//!
//! Three kinds of script: a map drawn by hand on an empty shell, edits of a
//! full automatic layout (which shares points with reaction axes the way the
//! layout drew them), and edits of maps saved from the app. The thousand-
//! reaction gut9 map is only replayed with `COXETER_SLOW_TESTS=1`.

mod common;

use coxeter_core::mapdata::MapData;
use coxeter_map::EditableMap;
use coxeter_map::layout::LayoutOptions;
use common::*;

/// Ops whose JavaScript changed after these references were recorded, so
/// they are not compared until the fixtures are written again (`node
/// rust/tools/fixtures-map.mjs edits`). `editGeometryRect` since e9438d7 and
/// its follow-ups: a curve's b1 moves only with its metabolite, a transporter
/// on a resized side rides it across and not along, and a resized side stops
/// at the parent instead of shifting the far one. `tests/js_geometry_edit.rs`
/// and `tests/drag_selection.rs` cover the current behaviour.
const STALE_OPS: &[&str] = &["editGeometryRect"];

#[test]
fn edit_sequences_match_javascript_step_by_step() {
    replay_index("edits");
}

/// Sequence D: a curated scenario map laid out selection by selection (even
/// spacing, turns, compass points, refusals) and reaction names moved.
#[test]
fn selection_turns_and_moved_reaction_names_match_javascript_step_by_step() {
    replay_index("selection-turn");
}

/// Replay every script listed in `map/index-<section>.json`.
fn replay_index(section: &str) {
    let index = fixture(&format!("map/index-{section}.json")).unwrap_or_else(|| panic!("run: node rust/tools/fixtures-map.mjs {section}"));
    let mut checked = 0;
    let mut stale = 0;
    for entry in index.as_array().unwrap() {
        let file = entry["file"].as_str().unwrap();
        if entry["slow"] == true && std::env::var("COXETER_SLOW_TESTS").is_err() {
            eprintln!("skipping {file} (slow: set COXETER_SLOW_TESTS=1 to run it)");
            continue;
        }
        let Some(steps) = fixture(file) else {
            eprintln!("skipping {file} (not generated)");
            continue;
        };
        // A script run on a saved map uses no op that asks the model anything.
        let community = scenario_community(entry["scenario"].as_str().unwrap_or("solo-ecoli"));
        let mut map = if entry["start"] == "file" {
            let Some(input) = input_path(entry) else {
                eprintln!("skipping {file} (its map is not on this machine)");
                continue;
            };
            let parsed = coxeter_map::map_document::parse_map_document_str(&std::fs::read_to_string(input).unwrap()).unwrap();
            EditableMap::from_json(&parsed.map).unwrap()
        } else if entry["start"] == "empty" {
            EditableMap::empty_from_community(&community, &LayoutOptions::default())
        } else {
            EditableMap::from_community(&community, &LayoutOptions::default())
        };
        // The fixture keeps what changed at each step; the expected map is
        // rebuilt as the steps go by.
        let mut expected = serde_json::Value::Null;
        for (i, step) in steps.as_array().unwrap().iter().enumerate() {
            let op = step["op"].as_str().unwrap();
            let result = edits::apply_edit(&mut map, &community, op, &step["args"]);
            apply_delta(&mut expected, &step["delta"]);
            let what = format!("{file} step {i} ({op})");
            if STALE_OPS.contains(&op) {
                // Not compared (see STALE_OPS): carry on from the JavaScript
                // map as it was after this step. Only the drawing is taken
                // over; what the session knows beyond it (which segment ends
                // are axis points, which lane entries are live nodes) is the
                // Rust map's own, as reloading would lose it.
                let data: MapData = serde_json::from_value(expected.clone()).unwrap();
                map.edit_graph(|g| {
                    // `MapGraph::from_data` without its re-linking of segment
                    // ends to axes: those links are the session's.
                    g.canvas = data.canvas_size();
                    g.environment = data.environment.unwrap();
                    g.cells = data.cells;
                    g.medium = data.medium.unwrap_or_default();
                    g.nodes = data.nodes.into_iter().map(|n| (n.id.clone(), n)).collect();
                    g.reactions = data.reactions.into_iter().map(|r| (r.id.clone(), r)).collect();
                    g.segments = data.segments;
                    g.texts = data.texts.into_iter().map(|t| (t.id.clone(), t)).collect();
                    g.label_settings = data.label_settings;
                });
                map.refresh_boundaries();
                stale += 1;
                continue;
            }
            // Exact but for the last bits of boundary projections (see PROJECTION_TOL).
            assert_map_eq(&result, &step["result"], PROJECTION_TOL, &format!("{what} result"));
            assert_map_eq(&map.to_json(), &expected, PROJECTION_TOL, &what);
            checked += 1;
        }
    }
    eprintln!("{section}: {checked} steps compared, {stale} stale ones skipped");
}
