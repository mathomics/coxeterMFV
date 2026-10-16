//! Port of the Store tests of `test/uploaded-model-map.test.mjs`, against the
//! real store: a genome-scale model somebody else wrote, uploaded and mapped
//! by hand, and maps saved over one export opened over the next.
//! (`coxeter-map/tests/js_uploaded_model_map.rs` ported the whole file over
//! the map alone, with a transcribed stand-in for the store.)
//!
//! Ported here: every test that builds a `new Store` (through
//! `uploadedStore` / `handBuiltStore`) — the fifteen from "a map still opens
//! after the model file is renamed" to "a small model is still laid out again
//! by a preset, and one undo brings the old map back", "renaming a species"
//! excepted.
//!
//! Not ported here — no store in them, ported in coxeter-map
//! (`tests/js_uploaded_model_map.rs`): "the matrix is inside the intermembrane
//! space, whichever way that is spelt", "ATP synthase crosses the one membrane
//! it sits in", "the shared medium keeps the letter it shares with the
//! eyespot", "a compartment named after its own id is given its real name",
//! "the drawn shell names its compartments and nests them", "sibling
//! compartments get room in proportion to what they hold", "a reaction in the
//! cell`s own `s` is drawn in that compartment, not in the medium", "a real
//! shared-medium metabolite is still a medium node with no cell", "a
//! metabolite is labelled with its own id, not the model`s", "the automatic
//! layout labels cofactors without the model name too" (its `{ community }`
//! is a plain object, not a `Store`), "a map saved with the model name in its
//! labels is mended when it is opened", "renaming a species leaves names and
//! free text alone". "an upload is named and counted as it was written" is
//! `prepareImportedModels` (coxeter-io).
//!
//! One device not ported: "a preset on a large model with nothing drawn lays
//! out the shell alone" replaces `EditableMap.fromCommunity` with a function
//! that throws, to prove the full layout is not run. Nothing can be patched
//! that way here; the assertions on the resulting map are ported (and a full
//! layout of 400 padding reactions in one compartment would not give a map
//! narrower than the shell it replaced).
//!
//! `assert.rejects(p, /re/)` is an `Err` whose message matches.

mod common;

use std::sync::Arc;

use coxeter_core::community::{CommunityModel, CommunitySpec, SpeciesSpec};
use coxeter_core::js;
use coxeter_core::mapdata::{MapCompartment, NodeKind, Rect};
use coxeter_core::model::Model;
use coxeter_io::model_import::ImportFile;
use coxeter_map::map_document::{ContextSpecies, MapContext, ParsedMapDocument, validate_map_context};
use coxeter_map::{EditableMap, LayoutOptions, PlaceOptions};
use coxeter_state::Store;
use regex::Regex;
use serde_json::{Value, json};

use common::store;

// The model

/// `algaDocument({id, big})`: an alga in miniature, shaped like a cobrapy
/// export of an iRC1080 descendant: every compartment is named after its own
/// id, `s` is the eyespot, `i` the mitochondrial intermembrane space, and
/// there is a flagellum beside the cell.
fn alga_document(id: &str, big: usize) -> Value {
    let compartments = ["c", "e", "f", "g", "h", "i", "m", "s", "u"];
    let met = |mid: &str, compartment: &str| json!({ "id": mid, "name": mid, "compartment": compartment, "formula": "", "charge": 0 });
    let rxn = |rid: &str, metabolites: Value| {
        json!({ "id": rid, "name": rid, "metabolites": metabolites, "lower_bound": -1000, "upper_bound": 1000 })
    };
    let mut metabolites: Vec<Value> = [
        ("g6p_c", "c"),
        ("f6p_c", "c"),
        ("atp_c", "c"),
        ("adp_c", "c"),
        ("g3p_c", "c"),
        ("g3p_h", "h"),
        ("pi_c", "c"),
        ("pi_h", "h"),
        ("h_i", "i"),
        ("h_m", "m"),
        ("atp_m", "m"),
        ("adp_m", "m"),
        ("pi_m", "m"),
        ("retinol_s", "s"),
        ("dhretinol_s", "s"),
        ("udpgal_c", "c"),
        ("udpgal_g", "g"),
        ("tub_f", "f"),
        ("tubp_f", "f"),
        ("pq_u", "u"),
        ("pqh2_u", "u"),
        ("glc_e", "e"),
    ]
    .iter()
    .map(|(mid, compartment)| met(mid, compartment))
    .collect();
    let mut reactions = vec![
        rxn("PGI", json!({ "g6p_c": -1, "f6p_c": 1 })),
        rxn("TPT", json!({ "g3p_h": -1, "pi_c": -1, "g3p_c": 1, "pi_h": 1 })),
        rxn("ATPSm", json!({ "adp_m": -1, "pi_m": -1, "h_i": -4, "atp_m": 1, "h_m": 4 })),
        rxn("RETSAT", json!({ "retinol_s": -1, "dhretinol_s": 1 })),
        rxn("UDPGALtg", json!({ "udpgal_c": -1, "udpgal_g": 1 })),
        rxn("TUBK", json!({ "tub_f": -1, "tubp_f": 1 })),
        rxn("PQR", json!({ "pq_u": -1, "pqh2_u": 1 })),
        rxn("GLCt", json!({ "glc_e": -1, "g6p_c": 1 })),
    ];
    // Padding for the chloroplast, to make it a great deal larger than the Golgi.
    for k in 0..big {
        let (x, y) = (format!("x{k}_h"), format!("y{k}_h"));
        metabolites.push(met(&x, "h"));
        metabolites.push(met(&y, "h"));
        reactions.push(rxn(&format!("HR{k}"), json!({ (x): -1, (y): 1 })));
    }
    let names: serde_json::Map<String, Value> = compartments.iter().map(|c| (c.to_string(), json!(c))).collect();
    json!({
        "id": id,
        "name": "A small alga",
        "compartments": names,
        "metabolites": metabolites,
        "reactions": reactions,
        "genes": [],
    })
}

// The store

/// `uploadedStore(document, filename)`: `new Store('.')` and
/// `importModelFiles([{ name: filename, text: JSON.stringify(document) }])`.
fn uploaded_store(document: &Value, filename: &str) -> Store {
    let mut store = store();
    store
        .import_model_files(&[ImportFile { name: filename.into(), text: js::stringify(document, None) }])
        .expect("the upload is accepted");
    store
}

/// `handBuiltStore(document, filename)`: the same, starting from the empty
/// shell a model too large to draw opens on.
fn hand_built_store(document: &Value, filename: &str) -> Store {
    let mut store = uploaded_store(document, filename);
    let community = store.community.clone().expect("a community");
    store.map = Some(EditableMap::empty_from_community(&community, &LayoutOptions::default()));
    store.map_history.clear();
    store
}

fn map(store: &Store) -> &EditableMap {
    store.map.as_ref().expect("a map")
}

/// `store.addMapReaction(id)` is truthy.
fn add(store: &mut Store, rid: &str) -> bool {
    store.add_map_reaction(rid, &PlaceOptions::default()).is_some()
}

fn map_json(store: &Store) -> String {
    store.map_json().expect("the map is saved")
}

// Helpers

/// `map.cells[0].compartments.find((c) => c.id === id)`.
fn comp<'a>(map: &'a EditableMap, id: &str) -> Option<&'a MapCompartment> {
    map.cells()[0].compartments.iter().find(|c| c.id == id)
}

/// `rectOf(map, id)`.
fn rect_of(map: &EditableMap, id: &str) -> Option<Rect> {
    comp(map, id).map(|c| c.rect)
}

/// `within(inner, outer)`: one box wholly inside another, edges included.
fn within(inner: &Rect, outer: &Rect) -> bool {
    inner.x >= outer.x && inner.y >= outer.y && inner.x + inner.w <= outer.x + outer.w && inner.y + inner.h <= outer.y + outer.h
}

/// `inBox(p, r)`: a point in a box, edges included.
fn in_box(x: f64, y: f64, r: &Rect) -> bool {
    x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h
}

/// `JSON.stringify(map.toJSON())`.
fn map_text(map: &EditableMap) -> String {
    js::stringify(&map.to_json(), None)
}

/// `assert.match(text, /pattern/)`.
#[track_caller]
fn assert_matches(text: &str, pattern: &str) {
    assert!(Regex::new(pattern).unwrap().is_match(text), "{text:?} does not match /{pattern}/");
}

/// The label of a node, which every metabolite node has.
#[track_caller]
fn base_of(node: &coxeter_core::mapdata::Node) -> &str {
    node.base.as_deref().unwrap_or_else(|| panic!("{} has no label", node.id))
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("{v} is not a number"))
}

/// `[...map.reactions.keys()].sort()`.
fn sorted_reactions(map: &EditableMap) -> Vec<String> {
    let mut ids = map.reaction_ids();
    ids.sort_by(|a, b| js::code_unit_compare(a, b));
    ids
}

// Maps across versions

/// JS: 'a map still opens after the model file is renamed'
#[test]
fn a_map_still_opens_after_the_model_file_is_renamed() {
    let mut store = hand_built_store(&alga_document("sbml_model", 0), "alga_stage16.json");
    assert!(add(&mut store, "sbml_model@PGI"));
    let saved = map_json(&store);

    // The same model, exported again under the next version's file name.
    let mut next = uploaded_store(&alga_document("sbml_model", 0), "alga_stage17.json");
    next.load_map_document(&saved).expect("the map opens");
    assert!(map(&next).has_reaction("sbml_model@PGI"));

    // Bundled models are still held to the model they were drawn over
    // (`validateMapContext` of map-document.js, not the store).
    let parsed = ParsedMapDocument {
        format: "EscherMCMCMap".into(),
        version: 3.0,
        metadata: Some(json!({ "species": [{ "id": "eco", "model": "ecoli_core_mc" }] })),
        pathway_manager: None,
        flux_color_stops: None,
        study: None,
        map: Value::Null,
        legacy: false,
    };
    let current = MapContext {
        species: vec![ContextSpecies { id: "eco".into(), file: Some("yeast_core_mc".into()), model: None, enabled: Some(true) }],
        ..Default::default()
    };
    let refused = validate_map_context(&parsed, &current).expect_err("a different bundled model is refused");
    assert_matches(&refused.to_string(), "different model");
}

/// JS: 'a map follows its model when the model comes back under another id'
#[test]
fn a_map_follows_its_model_when_the_model_comes_back_under_another_id() {
    let mut first = hand_built_store(&alga_document("iAlga_stage15", 0), "alga.json");
    assert!(add(&mut first, "iAlga_stage15@PGI"));
    assert!(add(&mut first, "iAlga_stage15@TPT"));
    assert!(first.set_map_reaction_linear("iAlga_stage15@PGI", true));
    let saved = map_json(&first);
    assert_matches(&saved, "iAlga_stage15@PGI");

    let mut second = uploaded_store(&alga_document("sbml_model", 0), "alga.json");
    second.load_map_document(&saved).expect("the map is read against the model now open");

    assert_eq!(sorted_reactions(map(&second)), ["sbml_model@PGI", "sbml_model@TPT"]);
    assert_eq!(map(&second).cells()[0].id, "sbml_model");
    let text = map_text(map(&second));
    assert!(!text.contains("iAlga_stage15"), "no identifier still names the old model");
    assert_matches(&second.map_load_note, "iAlga_stage15.*sbml_model");
    // The line a straight reaction is kept on still names nodes that exist.
    let linear = map(&second).reaction("sbml_model@PGI").unwrap().linear.as_ref().expect("the reaction is still straight");
    assert!(
        map(&second).has_node(linear.from.as_deref().unwrap_or("")) && map(&second).has_node(linear.to.as_deref().unwrap_or(""))
    );
    // The transporter is still on the membrane it was put on.
    assert_eq!(map(&second).reaction("sbml_model@TPT").unwrap().boundary_id.as_deref(), Some("sbml_model/h"));
}

/// JS: 'a map of a genuinely different model is not drawn over it, and says which reaction'
/// (80e0611: it waits for its own model instead of being refused).
#[test]
fn a_map_of_a_genuinely_different_model_is_not_drawn_over_it_and_says_which_reaction() {
    let mut first = hand_built_store(&alga_document("iAlga_stage15", 0), "alga.json");
    assert!(add(&mut first, "iAlga_stage15@RETSAT"));
    let saved = map_json(&first);

    let mut other = alga_document("sbml_model", 0);
    other["reactions"].as_array_mut().unwrap().retain(|r| r["id"] != "RETSAT");
    let mut second = uploaded_store(&other, "alga.json");
    let open = map_text(map(&second));
    // Kept for the model it belongs to, and drawn when that one is loaded.
    second.load_map_document(&saved).expect("kept, not refused");
    assert_matches(&second.waiting_map.as_ref().expect("it waits").reason, "sbml_model@RETSAT does not exist.*iAlga_stage15");
    assert_eq!(map_text(map(&second)), open);
}

/// JS: 'a saved map can be opened onto a blank canvas'
#[test]
fn a_saved_map_can_be_opened_onto_a_blank_canvas() {
    let mut store = hand_built_store(&alga_document("sbml_model", 0), "alga.json");
    assert!(add(&mut store, "sbml_model@PGI"));
    let saved = map_json(&store);

    // Clear drawing leaves a canvas with no cells, and a map must still open on
    // it rather than be turned away as "created for a different set of cells".
    store.show_empty_map();
    assert_eq!(map(&store).cells().len(), 0);
    store.load_map_document(&saved).expect("the map opens onto the blank canvas");
    assert!(map(&store).has_reaction("sbml_model@PGI"));
    assert_eq!(map(&store).cells().len(), 1);
}

// Maps saved by earlier builds

/// What `mapFromBefore` hands back: the map as the file holds it, and the
/// file's text.
struct Before {
    doc: Value,
    text: String,
}

/// The reactions `mapFromBefore` draws when it is not told which.
const DRAWN_BEFORE: [&str; 4] = ["PGI", "TPT", "ATPSm", "RETSAT"];

/// `mapFromBefore({reactions, without})`: a map as an earlier build saved it.
///
/// The nesting those builds read off this same model (`OLD_TREE`) — `i` and
/// `s` as boxes of their own beside the cell, and the matrix straight in the
/// cytosol — the model's name in front of every label, compartments labelled
/// with their letters, and the reaction of the cell's own `s` compartment
/// drawn out in the shared medium with its metabolites filed as medium nodes.
fn map_from_before(reactions: &[&str], without: &[&str]) -> Before {
    let mut raw = alga_document("sbml_model", 0);
    raw["compartment_tree"] = json!({ "e": null, "f": null, "i": null, "s": null, "c": "e", "g": "c", "h": "c", "m": "c", "u": "h" });
    let model = Arc::new(Model::from_value(&raw).unwrap());
    let community = CommunityModel::new(CommunitySpec {
        species: vec![SpeciesSpec { id: Some(model.id.clone()), model, abundance: Some(1.0), enabled: Some(true), color: None }],
        ..Default::default()
    })
    .unwrap();
    let mut map = EditableMap::empty_from_community(&community, &LayoutOptions::default());
    for id in reactions {
        assert!(map.add_reaction(&community, &format!("sbml_model@{id}"), &PlaceOptions::default()).is_some(), "{id} is drawn");
    }
    let mut doc = map.to_json();

    fn shift(p: &mut Value, dx: f64, dy: f64) {
        if !p.is_object() {
            return;
        }
        p["x"] = json!(num(&p["x"]) + dx);
        p["y"] = json!(num(&p["y"]) + dy);
    }
    let stray = "sbml_model@RETSAT";
    let at = doc["reactions"].as_array().unwrap().iter().position(|r| r["id"] == stray);
    if let Some(at) = at {
        let to = (num(&doc["environment"]["rect"]["x"]) + 70.0, num(&doc["environment"]["rect"]["y"]) + 70.0);
        let dx = to.0 - num(&doc["reactions"][at]["x"]);
        let dy = to.1 - num(&doc["reactions"][at]["y"]);
        let rxn = &mut doc["reactions"][at];
        shift(rxn, dx, dy);
        shift(&mut rxn["axis"]["a1"], dx, dy);
        shift(&mut rxn["axis"]["a2"], dx, dy);
        let mut medium_nodes = Vec::new();
        for node in doc["nodes"].as_array_mut().unwrap() {
            let of_stray = |key: &str| node.get(key).and_then(Value::as_str) == Some(stray);
            if !of_stray("reactionLocalFor") && !of_stray("reactionId") {
                continue;
            }
            shift(node, dx, dy);
            if node["kind"] == "metabolite" {
                node["kind"] = json!("medium");
                medium_nodes.push(node["id"].clone());
            }
        }
        doc["medium"]["nodes"] = Value::Array(medium_nodes);
        for seg in doc["segments"].as_array_mut().unwrap() {
            if seg["reactionId"] != stray {
                continue;
            }
            for end in ["from", "to", "b1", "b2"] {
                if let Some(p) = seg.get_mut(end) {
                    shift(p, dx, dy);
                }
            }
        }
    }
    for node in doc["nodes"].as_array_mut().unwrap() {
        let base = node.get("base").and_then(Value::as_str).unwrap_or("undefined").to_string();
        node["base"] = json!(format!("sbml_model@{base}"));
    }
    doc["cells"][0]["compartments"].as_array_mut().unwrap().retain(|c| !without.contains(&c["id"].as_str().unwrap_or("")));
    for comp in doc["cells"][0]["compartments"].as_array_mut().unwrap() {
        let id = comp["id"].clone();
        comp["label"] = id.clone();
        comp.as_object_mut().unwrap().shift_remove("visualId");
        if id == "s" {
            comp["membrane"] = json!("open");
        }
    }
    let text = js::stringify(
        &json!({
            "format": "EscherMCMCMap",
            "version": 3,
            "metadata": { "scenario": "uploaded-models", "species": [{ "id": "sbml_model", "model": "upload:alga" }] },
            "pathwayManager": null,
            "fluxColorStops": null,
            "map": doc,
        }),
        None,
    );
    Before { doc, text }
}

/// A compartment of the first cell of a map document.
fn doc_comp<'a>(doc: &'a Value, id: &str) -> &'a Value {
    doc["cells"][0]["compartments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == id)
        .unwrap_or_else(|| panic!("the file has no compartment {id}"))
}

/// JS: 'a map drawn before opens with its compartments where the model nests them'
#[test]
fn a_map_drawn_before_opens_with_its_compartments_where_the_model_nests_them() {
    let before = map_from_before(&DRAWN_BEFORE, &[]);
    let mut roots: Vec<&str> = before.doc["cells"][0]["compartments"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c.get("parentId").is_none_or(Value::is_null))
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    roots.sort_by(|a, b| js::code_unit_compare(a, b));
    assert_eq!(roots, ["e", "f", "i", "s"], "the file really has the old nesting");
    let matrix_was: Rect = serde_json::from_value(doc_comp(&before.doc, "m")["rect"].clone()).unwrap();

    let mut store = uploaded_store(&alga_document("sbml_model", 0), "alga.json");
    store.load_map_document(&before.text).expect("the old map opens");
    let map = map(&store);

    let tree = store.models.get(&store.species[0].file).expect("the uploaded model").compartment_tree();
    for comp in &map.cells()[0].compartments {
        let parent = comp.parent_id.as_deref().filter(|p| !p.is_empty());
        assert_eq!(parent, tree.by_id[&comp.id].parent.as_deref(), "{} is nested as the model has it", comp.id);
        if let Some(parent) = parent {
            let outer = rect_of(map, parent).unwrap_or_else(|| panic!("{parent} is drawn"));
            assert!(within(&comp.rect, &outer), "{} is drawn inside {parent}", comp.id);
        }
    }

    // The intermembrane space takes the matrix's place and the matrix is drawn
    // in a little, so nothing beside them had to move. (A matrix as small as
    // this one cannot give up the whole band, and the space takes the rest
    // outward.)
    let i = rect_of(map, "i").expect("the intermembrane space is drawn");
    let m = rect_of(map, "m").expect("the matrix is drawn");
    assert!(within(&matrix_was, &i), "the space covers where the matrix stood");
    assert!(i.w * i.h < matrix_was.w * matrix_was.h * 1.2, "and hardly more than that");
    assert!(m.w < matrix_was.w && m.h < matrix_was.h, "the matrix is drawn in");
    for band in [m.x - i.x, m.y - i.y, i.x + i.w - (m.x + m.w), i.y + i.h - (m.y + m.h)] {
        assert!(band >= 44.0, "a metabolite fits between the two membranes ({band})");
    }
    assert_matches(&store.map_load_note, "i around m");
    assert_matches(&store.map_load_note, "s into c");
}

/// JS: 'what was drawn in the right place stays exactly where it was put'
#[test]
fn what_was_drawn_in_the_right_place_stays_exactly_where_it_was_put() {
    let before = map_from_before(&DRAWN_BEFORE, &[]);
    let mut store = uploaded_store(&alga_document("sbml_model", 0), "alga.json");
    store.load_map_document(&before.text).expect("the old map opens");

    for rid in ["sbml_model@PGI", "sbml_model@TPT"] {
        let was = before.doc["reactions"].as_array().unwrap().iter().find(|r| r["id"] == rid).expect("the file draws it");
        let now = map(&store).reaction(rid).unwrap_or_else(|| panic!("{rid} is still drawn"));
        assert_eq!((now.x, now.y), (num(&was["x"]), num(&was["y"])), "{rid} has not moved");
        for node in before.doc["nodes"].as_array().unwrap() {
            // `(n.reactionLocalFor ?? n.reactionId) === rid`
            let owner = node.get("reactionLocalFor").filter(|v| !v.is_null()).or_else(|| node.get("reactionId"));
            if owner.and_then(Value::as_str) != Some(rid) {
                continue;
            }
            let id = node["id"].as_str().unwrap();
            let after = map(&store).node(id).unwrap_or_else(|| panic!("{id} is still drawn"));
            assert_eq!((after.x, after.y), (num(&node["x"]), num(&node["y"])), "{id} has not moved");
        }
    }
    // Labels and names are mended on the way, as for any earlier file.
    for node in map(&store).nodes().values() {
        assert!(!base_of(node).contains('@'));
    }
    assert_eq!(comp(map(&store), "c").unwrap().label, "Cytosol");
}

/// JS: 'what the old nesting had put in the wrong place is drawn where it belongs'
#[test]
fn what_the_old_nesting_had_put_in_the_wrong_place_is_drawn_where_it_belongs() {
    let before = map_from_before(&DRAWN_BEFORE, &[]);
    let mut store = uploaded_store(&alga_document("sbml_model", 0), "alga.json");
    store.load_map_document(&before.text).expect("the old map opens");
    let map = map(&store);
    let rect = |id: &str| rect_of(map, id).unwrap_or_else(|| panic!("{id} is drawn"));

    // The eyespot reaction is in the eyespot, as ordinary metabolites.
    let retsat = map.reaction("sbml_model@RETSAT").expect("the eyespot reaction is drawn");
    assert!(in_box(retsat.x, retsat.y, &rect("s")), "the reaction is inside the eyespot");
    let eyespot: Vec<_> = map.nodes().values().filter(|n| n.compartment.as_deref() == Some("s")).collect();
    assert_eq!(eyespot.len(), 2);
    for node in eyespot {
        assert_eq!(node.kind, NodeKind::Metabolite);
        assert!(in_box(node.x, node.y, &rect("s")), "{} is inside the eyespot", base_of(node));
    }
    assert!(map.medium().nodes.is_empty());
    assert!(!map.nodes().values().any(|n| n.kind == NodeKind::Medium));

    // ATP synthase is on the membrane it crosses, with its protons on either side.
    let atps = map.reaction("sbml_model@ATPSm").expect("ATP synthase is drawn");
    assert_eq!(atps.boundary_id.as_deref(), Some("sbml_model/m"));
    let proton = map.nodes().values().find(|n| n.merged_id.as_deref() == Some("sbml_model@h_i")).expect("the outer proton is drawn");
    assert!(
        in_box(proton.x, proton.y, &rect("i")) && !in_box(proton.x, proton.y, &rect("m")),
        "the proton outside the matrix is in the intermembrane space"
    );
    for node in map.nodes().values() {
        if node.compartment.as_deref() == Some("m") {
            assert!(in_box(node.x, node.y, &rect("m")), "{} is in the matrix", base_of(node));
        }
    }
    // Every curve still ends on the metabolite it is drawn to.
    for seg in map.segments() {
        let Some(node_id) = seg.node_id.as_deref().filter(|id| !id.is_empty()) else { continue };
        let node = map.node(node_id).unwrap_or_else(|| panic!("a curve ends on {node_id}, which is not drawn"));
        let end = if seg.side.as_deref() == Some("substrate") { seg.from } else { seg.to };
        assert!(js::hypot(end.x - node.x, end.y - node.y) < 1e-6, "a curve of {} ends off {node_id}", seg.reaction_id);
    }
}

/// JS: 'opening an old map is one undo step, and saving it again leaves nothing to mend'
#[test]
fn opening_an_old_map_is_one_undo_step_and_saving_it_again_leaves_nothing_to_mend() {
    let before = map_from_before(&DRAWN_BEFORE, &[]);
    let mut store = uploaded_store(&alga_document("sbml_model", 0), "alga.json");
    let full = map(&store).reactions().len();
    store.load_map_document(&before.text).expect("the old map opens");
    assert_eq!(store.map_history.undo_len(), 1);

    let mended = map_json(&store);
    let shell = map_text(map(&store));
    store.load_map_document(&mended).expect("the mended map opens");
    assert_eq!(store.map_load_note, "", "a map saved now needs nothing done to it");
    assert_eq!(map_text(map(&store)), shell);

    store.undo_map();
    store.undo_map();
    assert_eq!(map(&store).reactions().len(), full, "undo brings back the map that was open before");
}

/// JS: 'a compartment the model has put in between is drawn if the map never had it'
#[test]
fn a_compartment_the_model_has_put_in_between_is_drawn_if_the_map_never_had_it() {
    // Cell, cytosol and matrix drawn by hand under the old nesting; no `i` at all.
    let before = map_from_before(&["PGI"], &["i", "s", "f"]);
    let mut store = uploaded_store(&alga_document("sbml_model", 0), "alga.json");
    store.load_map_document(&before.text).expect("the old map opens");
    {
        let map = map(&store);
        let i = comp(map, "i").expect("the intermembrane space is drawn");
        assert_eq!(i.parent_id.as_deref(), Some("c"));
        assert_eq!(i.label, "Intermembrane space");
        assert_eq!(comp(map, "m").expect("the matrix is drawn").parent_id.as_deref(), Some("i"));
        assert!(within(&rect_of(map, "m").unwrap(), &i.rect));
        assert!(map.boundary("sbml_model/i").is_some(), "and it has a membrane to put reactions on");
    }

    // The reaction that needs both can now be added and lands where it should.
    let placed = store.add_map_reaction("sbml_model@ATPSm", &PlaceOptions::default()).expect("ATP synthase can be added");
    assert_eq!(map(&store).reaction(&placed.reaction_id).unwrap().boundary_id.as_deref(), Some("sbml_model/m"));
}

/// JS: 'a map that needs nothing comes through untouched'
#[test]
fn a_map_that_needs_nothing_comes_through_untouched() {
    let mut store = hand_built_store(&alga_document("sbml_model", 0), "alga.json");
    for id in ["PGI", "TPT", "ATPSm", "RETSAT"] {
        assert!(add(&mut store, &format!("sbml_model@{id}")));
    }
    let (id, x, y) = map(&store).nodes().values().next().map(|n| (n.id.clone(), n.x, n.y)).expect("something is drawn");
    store.move_node(&id, x + 31.0, y - 17.0);
    let saved = map_json(&store);
    let drawn = map_text(map(&store));

    let mut next = uploaded_store(&alga_document("sbml_model", 0), "alga.json");
    next.load_map_document(&saved).expect("the map opens");
    assert_eq!(map_text(map(&next)), drawn);
    assert_eq!(next.map_load_note, "");
}

/// JS: 'reactions the model has since lost are left out and the rest of the map is kept'
#[test]
fn reactions_the_model_has_since_lost_are_left_out_and_the_rest_of_the_map_is_kept() {
    let mut first = hand_built_store(&alga_document("sbml_model", 0), "alga.json");
    for id in ["PGI", "TPT", "RETSAT", "PQR"] {
        assert!(add(&mut first, &format!("sbml_model@{id}")));
    }
    let saved = map_json(&first);
    let at = |map: &EditableMap, id: &str| map.reaction(id).map(|r| (r.x, r.y)).unwrap_or_else(|| panic!("{id} is drawn"));
    let tpt = at(map(&first), "sbml_model@TPT");
    let pqr = at(map(&first), "sbml_model@PQR");

    // The next export of the model: one reaction gone, one metabolite renamed,
    // one reaction given another metabolite.
    let mut next = alga_document("sbml_model", 0);
    next["reactions"].as_array_mut().unwrap().retain(|r| r["id"] != "RETSAT");
    let renamed = next["metabolites"].as_array_mut().unwrap().iter_mut().find(|m| m["id"] == "f6p_c").unwrap();
    renamed["id"] = json!("f6p_B_c");
    let reaction = |doc: &Value, id: &str| -> usize { doc["reactions"].as_array().unwrap().iter().position(|r| r["id"] == id).unwrap() };
    let pgi = reaction(&next, "PGI");
    next["reactions"][pgi]["metabolites"] = json!({ "g6p_c": -1, "f6p_B_c": 1 });
    let pqr_at = reaction(&next, "PQR");
    next["reactions"][pqr_at]["metabolites"]["atp_c"] = json!(-1);

    let mut second = uploaded_store(&next, "alga.json");
    second.load_map_document(&saved).expect("the map opens");
    let map = map(&second);

    assert_eq!(sorted_reactions(map), ["sbml_model@PGI", "sbml_model@PQR", "sbml_model@TPT"]);
    assert!(
        !map.nodes().values().any(|n| n.compartment.as_deref() == Some("s")),
        "the lost reaction took its metabolites with it"
    );
    // The reaction whose metabolite was renamed is drawn again, with the new one.
    assert!(map.nodes().values().any(|n| n.merged_id.as_deref() == Some("sbml_model@f6p_B_c")));
    assert!(!map.nodes().values().any(|n| n.merged_id.as_deref() == Some("sbml_model@f6p_c")));
    // The ones the change did not reach are where they were.
    assert_eq!(at(map, "sbml_model@TPT"), tpt);
    assert_eq!(at(map, "sbml_model@PQR"), pqr);

    assert_matches(&second.map_load_note, r"1 reaction the model no longer has was left out \(RETSAT\)");
    assert_matches(&second.map_load_note, r"1 drawn afresh because the model has changed their metabolites \(PGI\)");
    assert_matches(&second.map_load_note, r"1 still drawn with metabolites the model has since changed \(PQR\)");
    for seg in map.segments() {
        assert!(map.has_reaction(&seg.reaction_id), "no curve is left without its reaction");
    }
}

/// JS: 'a map that shares nothing with the open model is not drawn over it'
/// (80e0611: it waits for its own model instead of being turned away).
#[test]
fn a_map_that_shares_nothing_with_the_open_model_is_not_drawn_over_it() {
    let mut first = hand_built_store(&alga_document("sbml_model", 0), "alga.json");
    for id in ["PGI", "TPT"] {
        assert!(add(&mut first, &format!("sbml_model@{id}")));
    }
    let saved = map_json(&first);

    let mut other = alga_document("sbml_model", 0);
    for rxn in other["reactions"].as_array_mut().unwrap() {
        rxn["id"] = json!(format!("X_{}", rxn["id"].as_str().unwrap()));
    }
    let mut second = uploaded_store(&other, "alga.json");
    let open = map_text(map(&second));
    second.load_map_document(&saved).expect("it waits for its own model");
    assert_matches(&second.waiting_map.as_ref().expect("it waits").reason, "None of the 2 reactions on this map exist in the active model");
    assert_eq!(map_text(map(&second)), open, "and the map that was open is still open");
}

// Layout presets

/// JS: 'a layout preset does not lay a hand-built map of a large model out again'
#[test]
fn a_layout_preset_does_not_lay_a_hand_built_map_of_a_large_model_out_again() {
    let mut store = uploaded_store(&alga_document("sbml_model", 400), "alga.json");
    assert_eq!(map(&store).reactions().len(), 0, "too large to draw automatically: the shell is empty");
    add(&mut store, "sbml_model@PGI");
    let (id, x, y) = map(&store).nodes().values().next().map(|n| (n.id.clone(), n.x, n.y)).expect("something is drawn");
    store.move_node(&id, x + 77.0, y + 33.0);
    let placed = map(&store).node(&id).map(|n| (n.x, n.y)).unwrap();

    assert_matches(store.relayout_blocked().as_deref().unwrap_or(""), "hand-built drawing");
    store.set_layout_preset("spacious");

    let after = map(&store).node(&id).expect("the node is still drawn");
    assert_eq!((after.x, after.y), placed, "what was placed by hand is where it was put");
    assert_matches(&store.status().message, "The drawing was kept");
}

/// JS: 'a preset on a large model with nothing drawn lays out the shell alone'
///
/// The JS also makes `EditableMap.fromCommunity` throw for the duration, to
/// prove the full layout is not run; see the head of the file.
#[test]
fn a_preset_on_a_large_model_with_nothing_drawn_lays_out_the_shell_alone() {
    let mut store = uploaded_store(&alga_document("sbml_model", 400), "alga.json");
    assert_eq!(map(&store).reactions().len(), 0);
    let width = map(&store).width();
    // The full layout of a model this size is what the upload itself avoids.
    store.set_layout_preset("compact");
    assert!(map(&store).width() < width, "the shell was laid out again, smaller");
    assert_eq!(map(&store).reactions().len(), 0);
    assert_eq!(map(&store).cells().len(), 1);
}

/// JS: 'a small model is still laid out again by a preset, and one undo brings the old map back'
#[test]
fn a_small_model_is_still_laid_out_again_by_a_preset_and_one_undo_brings_the_old_map_back() {
    let mut store = uploaded_store(&alga_document("sbml_model", 0), "alga.json");
    assert_eq!(store.relayout_blocked(), None);
    let width = map(&store).width();
    let before = map_text(map(&store));
    store.set_layout_preset("compact");
    assert!(map(&store).width() < width, "the compact preset draws a smaller map");
    store.undo_map();
    assert!(map_text(map(&store)) == before, "undo restores the map as it was");
}
