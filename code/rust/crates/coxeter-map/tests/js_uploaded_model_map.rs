//! Port of `test/uploaded-model-map.test.mjs`: a genome-scale model that
//! somebody else wrote, uploaded and mapped by hand.
//!
//! The bundled core models are written for this tool: their compartments are
//! named, nested in a chain and use only letters the registry knows. A draft
//! reconstruction exported from cobrapy is none of those things, and
//! everything here is a way that difference showed on the map.
//!
//! # How the `Store` is stood in for
//!
//! Sixteen of the JavaScript tests go through `Store` (`src/ui/store.js`),
//! which is another crate's business. [`Store`] below is a transcription of
//! the few methods those tests call, each reduced to what it does to the map:
//!
//! - `importModelFiles` — the model is built from the same COBRA JSON with
//!   `Model::from_json_str` (`prepareImportedModels` is crate `coxeter-io`),
//!   the species id and the model key are derived as
//!   `src/core/model-import.js` derives them, and the map is the full layout
//!   or, past the store's auto-draw limits, the empty shell;
//! - `mapJSON` — `create_map_document` over the store's context;
//! - `loadMapDocument` / `adoptMapSpecies` / `applyMapDocument` — parse,
//!   validate the context, (rename the species), `EditableMap::from_json`,
//!   `prune_map_to_model`, `reconcile_compartments`,
//!   `describe_reconciliation`, one history record;
//! - `addMapReaction`, `setMapReactionLinear`, `moveNode`, `showEmptyMap`,
//!   `undoMap`, `setLayoutPreset` / `relayoutMap` / `relayoutBlocked`.
//!
//! Two things in there are the store's own and are copied here only so that
//! the sequence is whole; an assertion that rests on them says so where it
//! stands: `validateMapDocument` (which refuses a map that shares nothing
//! with the model, and words that refusal) and the sentences the store adds
//! around an error or a load note.
//
// Not ported: "an upload is named and counted as it was written" — it tests
//   `prepareImportedModels` (src/core/model-import.js, crate coxeter-io) and
//   nothing of the map.
// Not ported (one assertion): "the automatic layout labels cofactors without
//   the model name too" — the `primaryMetabolitesForSide` check at its end is
//   a UI module (src/ui/add-reaction-panel.js). The layout half is ported.
// Not ported (one assertion): "a layout preset does not lay a hand-built map
//   of a large model out again" — `store.status.message`, the store's status
//   line.
// Not ported (one device): "a preset on a large model with nothing drawn lays
//   out the shell alone" — the JavaScript replaces `EditableMap.fromCommunity`
//   with a function that throws, to prove the full layout is not run. Nothing
//   can be patched that way here; the transcribed `relayout_map` takes the
//   shell-only branch by the same condition, and the assertions on the
//   resulting map are ported.

mod common;

use std::collections::HashSet;
use std::sync::Arc;

use coxeter_core::boundaries::reaction_boundary_keys;
use coxeter_core::community::{CommunityModel, CommunitySpec, MediumLimit, Objective, ReactionRole, SpeciesSpec};
use coxeter_core::compartments::{
    Membrane, Names, build_compartment_tree, cell_compartment_name, cell_compartment_visual_id, compartment_name,
    compartment_visual_id, membrane_style,
};
use coxeter_core::js;
use coxeter_core::mapdata::{MapCompartment, NodeKind, Rect};
use coxeter_core::model::Model;
use coxeter_map::core_metabolism_overview::{build_core_metabolism_overview, core_overview_support};
use coxeter_map::history::MapHistory;
use coxeter_map::layout_settings::{LayoutSettings, RawLayoutSettings, layout_engine_options, layout_preset};
use coxeter_map::map_document::{
    ContextSpecies, MapContext, ParsedMapDocument, create_map_document, map_species_ids, parse_map_document,
    parse_map_document_str, rename_map_species, validate_map_context,
};
use coxeter_map::place_reaction::Placed;
use coxeter_map::reconcile_map::{describe_reconciliation, prune_map_to_model, reconcile_compartments};
use coxeter_map::{EditableMap, LayoutOptions, PlaceOptions};
use indexmap::IndexMap;
use regex::Regex;
use serde_json::{Value, json};

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

/// A community of the one model, under the model's own id:
/// `new CommunityModel({species: [{id: model.id, model, abundance: 1, enabled: true}], medium: {}})`.
fn community_of(model: Model) -> (Arc<Model>, CommunityModel) {
    let model = Arc::new(model);
    let community = CommunityModel::new(CommunitySpec {
        species: vec![SpeciesSpec {
            id: Some(model.id.clone()),
            model: model.clone(),
            abundance: Some(1.0),
            enabled: Some(true),
            color: None,
        }],
        ..Default::default()
    })
    .unwrap();
    (model, community)
}

/// `alga(options)`.
fn alga(big: usize) -> (Arc<Model>, CommunityModel) {
    community_of(Model::from_value(&alga_document("sbml_model", big)).unwrap())
}

/// The bundled E. coli core as the one species `eco`.
fn eco_community() -> CommunityModel {
    CommunityModel::new(CommunitySpec {
        species: vec![SpeciesSpec::new("eco", Arc::new(common::load_model("ecoli_core_mc"))).abundance(1.0)],
        ..Default::default()
    })
    .unwrap()
}

// Helpers

fn names(pairs: &[(&str, &str)]) -> Names {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

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

/// `for (const node of doc.nodes) node.base = `sbml_model@${node.base}``.
fn put_model_name_in_labels(doc: &mut Value) {
    for node in doc["nodes"].as_array_mut().unwrap() {
        let base = node.get("base").and_then(Value::as_str).unwrap_or("undefined").to_string();
        node["base"] = json!(format!("sbml_model@{base}"));
    }
}

/// The bare letters the compartments were stored with, and the eyespot
/// stroked like the open shared medium.
fn store_compartments_by_letter(doc: &mut Value) {
    for comp in doc["cells"][0]["compartments"].as_array_mut().unwrap() {
        let id = comp["id"].clone();
        comp["label"] = id.clone();
        comp.as_object_mut().unwrap().shift_remove("visualId");
        if id == "s" {
            comp["membrane"] = json!("open");
        }
    }
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

fn doc_rect(v: &Value) -> Rect {
    serde_json::from_value(v.clone()).unwrap()
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("{v} is not a number"))
}

// The store stand-in

/// The scenario id an upload is filed under.
const SCENARIO: &str = "uploaded-models";
/// `AUTO_DRAW_REACTION_LIMIT` and `AUTO_DRAW_METABOLITE_LIMIT` of the store.
const AUTO_DRAW_REACTION_LIMIT: usize = 350;
const AUTO_DRAW_METABOLITE_LIMIT: usize = 500;

/// What these tests use of `Store`; see the head of the file.
struct Store {
    /// `store.models.get(store.species[0].file)`.
    model: Arc<Model>,
    community: CommunityModel,
    /// `store.species`: `[{id, file, abundance, enabled}]`.
    species: Vec<ContextSpecies>,
    layout_settings: LayoutSettings,
    map: EditableMap,
    history: MapHistory,
    map_load_note: String,
}

/// `safeId` of `src/core/model-import.js`.
fn safe_id(value: &str) -> String {
    let id = Regex::new(r"(?i)\.(json|xml|sbml)$").unwrap().replace(value.trim(), "").into_owned();
    let id = Regex::new(r"[^A-Za-z0-9_.-]+").unwrap().replace_all(&id, "_").into_owned();
    let id = id.trim_matches('_');
    if id.is_empty() { "model".into() } else { id.to_string() }
}

/// `a || b`.
fn or(a: &str, b: &str) -> String {
    if a.is_empty() { b.to_string() } else { a.to_string() }
}

/// `uploadedStore(document, filename)`: `new Store('.')` then
/// `importModelFiles([{name: filename, text: JSON.stringify(document)}])`.
fn uploaded_store(document: &Value, filename: &str) -> Store {
    // prepareImportedModels, for one file.
    let model = Arc::new(Model::from_json_str(&js::stringify(document, None)).unwrap());
    let stem = Regex::new(r"(?i)\.(json|xml|sbml)$").unwrap().replace(filename, "").into_owned();
    let species_id = safe_id(&or(&model.id, &or(&stem, "sp1")));
    let model_key = format!("upload:{}", safe_id(&or(&stem, &or(&model.id, &species_id))));

    // inferCommunityMedium, then mediumSpec().
    let mut medium: IndexMap<String, MediumLimit> = IndexMap::new();
    for rxn in model.exchange_reactions() {
        if !(rxn.lower_bound < 0.0) || rxn.metabolites.len() != 1 {
            continue;
        }
        let Some(met) = rxn.metabolites.keys().next().and_then(|mid| model.metabolites.get(mid)) else { continue };
        let so_far = medium.get(&met.base).and_then(|limit| limit.lower).map_or(0.0, f64::abs);
        let limit = js::max(so_far, rxn.lower_bound.abs());
        medium.insert(met.base.clone(), MediumLimit { lower: Some(-limit), upper: Some(1000.0) });
    }

    // rebuild({relayout: true, contents: drawable ? 'all' : 'none'}).
    let community = CommunityModel::new(CommunitySpec {
        name: Some(model.name.clone()),
        objective: Some(Objective::EqualGrowth),
        product_target: None,
        species: vec![SpeciesSpec {
            id: Some(species_id.clone()),
            model: model.clone(),
            abundance: Some(1.0),
            enabled: Some(true),
            color: None,
        }],
        medium,
    })
    .unwrap();
    let layout_settings = layout_preset(None);
    let options = layout_engine_options(&RawLayoutSettings::from(&layout_settings));
    // Crowding is per cell, so it is the largest member that decides.
    let drawable = model.reactions.len() <= AUTO_DRAW_REACTION_LIMIT && model.metabolites.len() <= AUTO_DRAW_METABOLITE_LIMIT;
    let mut map = if drawable {
        EditableMap::from_community(&community, &options)
    } else {
        EditableMap::empty_from_community(&community, &options)
    };
    // A model too large to draw in full opens on central carbon metabolism
    // where it has it, and on the empty shell where it has not.
    if !drawable && core_overview_support(&community).supported {
        map = build_core_metabolism_overview(&community, &RawLayoutSettings::from(&layout_settings)).map;
    }
    let mut history = MapHistory::default();
    history.clear();

    Store {
        model,
        community,
        species: vec![ContextSpecies { id: species_id, file: Some(model_key), model: None, enabled: Some(true) }],
        layout_settings,
        map,
        history,
        map_load_note: String::new(),
    }
}

/// `handBuiltStore(document, filename)`: the same, starting from the empty
/// shell a model too large to draw opens on.
fn hand_built_store(document: &Value, filename: &str) -> Store {
    let mut store = uploaded_store(document, filename);
    store.map = EditableMap::empty_from_community(&store.community, &LayoutOptions::default());
    store.history.clear();
    store
}

/// `validateMapDocument(doc, community, species)` of `store.js`: the store's
/// own check, copied so the load sequence is whole.
fn validate_map_document(doc: &Value, community: &CommunityModel, species: &[ContextSpecies]) -> Result<(), String> {
    if !doc.is_object() && !doc.is_array() {
        return Err("The file does not contain a valid map.".into());
    }
    let list = |key: &str| doc.get(key).and_then(Value::as_array);
    let (Some(reactions), Some(nodes), Some(_)) = (list("reactions"), list("nodes"), list("segments")) else {
        return Err("The file does not have the expected EditableMap structure.".into());
    };

    // Every cell the map draws has to be a species of the community.
    let known: HashSet<&str> = species.iter().map(|sp| sp.id.as_str()).collect();
    let cells = list("cells").map(Vec::as_slice).unwrap_or(&[]);
    if cells.iter().any(|cell| !cell.get("id").and_then(Value::as_str).is_some_and(|id| known.contains(id))) {
        return Err("The map was created for a different set of cells/species.".into());
    }

    // Only a map that shares nothing with the active model is a map of
    // something else.
    let ids = |items: &[Value], key: &str| -> Vec<String> {
        items.iter().filter_map(|item| item.get(key).and_then(Value::as_str)).filter(|id| !id.is_empty()).map(String::from).collect()
    };
    let drawn = ids(reactions, "id");
    let has = |id: &String| community.merged.reactions.contains_key(id);
    if let Some(missing) = drawn.iter().find(|id| !has(id)) {
        if !drawn.iter().any(has) {
            return Err(if drawn.len() == 1 {
                format!("Reaction {missing} does not exist in the active model.")
            } else {
                format!("None of the {} reactions on this map exist in the active model (the first is {missing}).", drawn.len())
            });
        }
    }
    if drawn.is_empty() {
        let merged = ids(nodes, "mergedId");
        let has = |id: &String| community.merged.metabolites.contains_key(id);
        if let Some(lost) = merged.iter().find(|id| !has(id)) {
            if !merged.iter().any(has) {
                return Err(format!("Metabolite {lost} does not exist in the active model."));
            }
        }
    }
    Ok(())
}

impl Store {
    /// `store.addMapReaction(reactionId)`.
    fn add_map_reaction(&mut self, reaction_id: &str) -> Option<Placed> {
        let before = self.map.to_json();
        let result = self.map.add_reaction(&self.community, reaction_id, &PlaceOptions::default())?;
        self.history.record(&before, &self.map.to_json(), "Add reaction");
        Some(result)
    }

    /// `store.setMapReactionLinear(reactionId, linear)`, with nothing selected.
    fn set_map_reaction_linear(&mut self, reaction_id: &str, linear: bool) -> bool {
        let before = self.map.to_json();
        if !self.map.set_reaction_linear(reaction_id, linear, None, None) {
            return false;
        }
        let label = if linear { "Make reaction straight" } else { "Release straight reaction" };
        self.history.record(&before, &self.map.to_json(), label);
        true
    }

    /// `store.moveNode(nodeId, x, y)`: no history, as in the store.
    fn move_node(&mut self, node_id: &str, x: f64, y: f64) -> bool {
        self.map.move_node(node_id, x, y).is_some()
    }

    /// `store.showEmptyMap()`.
    fn show_empty_map(&mut self) {
        let before = self.map.to_json();
        let next = EditableMap::blank(Some(self.map.width()), Some(self.map.height()));
        self.history.record(&before, &next.to_json(), "Empty map");
        self.map = next;
    }

    /// `store.mapJSON()`.
    fn map_json(&self) -> String {
        let context = MapContext { species: self.species.clone(), scenario: Some(SCENARIO.into()), ..Default::default() };
        js::stringify(&create_map_document(&self.map, &context).unwrap(), Some(2))
    }

    /// `store.loadMapDocument(text)`.
    fn load_map_document(&mut self, text: &str) -> Result<(), String> {
        let parsed = parse_map_document_str(text).map_err(|e| e.to_string())?;
        self.map_load_note.clear();
        let Err(error) = self.apply_map_document(&parsed) else { return Ok(()) };
        let wanted = parsed.metadata.as_ref().and_then(|m| m.get("scenario")).and_then(Value::as_str).unwrap_or("");
        assert!(wanted.is_empty() || wanted == SCENARIO, "restoreMapContext is not stood in for: the map names {wanted}");
        // The right scenario is already open and the map still does not fit
        // it: the one thing left that is not a real mismatch is the same
        // model under another id.
        if self.adopt_map_species(&parsed)? { Ok(()) } else { Err(error) }
    }

    /// `store.adoptMapSpecies(parsed)`: open a map that was drawn over the
    /// same model under another id. `false` when this is not that case.
    fn adopt_map_species(&mut self, parsed: &ParsedMapDocument) -> Result<bool, String> {
        let saved = map_species_ids(parsed);
        let active: Vec<&ContextSpecies> = self.species.iter().filter(|sp| sp.enabled != Some(false)).collect();
        if saved.len() != 1 || active.len() != 1 || saved[0] == active[0].id {
            return Ok(false);
        }
        let to = active[0].id.clone();
        let mut renamed = rename_map_species(parsed, &saved[0], &to, active[0].file.as_deref().or(active[0].model.as_deref()));
        // The cell is drawn with the name of the model it belongs to now.
        let cell_name = self.model.cell.name.clone();
        if !cell_name.is_empty() {
            for cell in renamed.map.get_mut("cells").and_then(Value::as_array_mut).into_iter().flatten() {
                cell["name"] = json!(cell_name);
            }
        }
        self.apply_map_document(&renamed).map_err(|error| {
            format!("{error} (The map was saved for \"{}\" and was read against \"{to}\", the model now open.)", saved[0])
        })?;
        self.map_load_note = format!(" It was saved for \"{}\" and has been read against \"{to}\".{}", saved[0], self.map_load_note);
        Ok(true)
    }

    /// `store.applyMapDocument(parsed)`.
    fn apply_map_document(&mut self, parsed: &ParsedMapDocument) -> Result<(), String> {
        let context = MapContext { species: self.species.clone(), ..Default::default() };
        validate_map_context(parsed, &context).map_err(|e| e.to_string())?;
        validate_map_document(&parsed.map, &self.community, &self.species)?;

        let before = self.map.to_json();
        let mut next = EditableMap::from_json(&parsed.map).map_err(|e| e.to_string())?;
        // A map from an earlier export of the model, or from an earlier build
        // of this program, is brought up to date rather than refused. What a
        // switched-off species has drawn is kept, not taken as lost.
        let keep_species: HashSet<String> =
            self.species.iter().filter(|sp| sp.enabled == Some(false)).map(|sp| sp.id.clone()).collect();
        let contents = prune_map_to_model(&mut next, &self.community, &keep_species);
        let shell = reconcile_compartments(&mut next, &self.community);
        self.map_load_note.push_str(&describe_reconciliation(Some(&contents), Some(&shell)));
        self.history.record(&before, &next.to_json(), "Load map JSON");
        self.map = next;
        Ok(())
    }

    /// `store.undoMap()`.
    fn undo_map(&mut self) -> bool {
        let Some(entry) = self.history.undo(&self.map.to_json()) else { return false };
        self.map = EditableMap::from_json(&entry.doc).unwrap();
        true
    }

    /// `store.tooLargeToAutoDraw()`.
    fn too_large_to_auto_draw(&self) -> bool {
        self.community.active_species().any(|sp| {
            sp.model.reactions.len() > AUTO_DRAW_REACTION_LIMIT || sp.model.metabolites.len() > AUTO_DRAW_METABOLITE_LIMIT
        })
    }

    /// `store.relayoutBlocked()`: why the automatic layout must not be re-run
    /// over the current drawing, or `None` when it may.
    fn relayout_blocked(&self) -> Option<&'static str> {
        let drawn = self.map.reactions().len();
        if drawn == 0 || drawn >= self.community.merged.reactions.len() {
            return None;
        }
        self.too_large_to_auto_draw().then_some(
            "Layout presets re-run the automatic layout, which would replace this hand-built drawing and take minutes on a model this size. The drawing was kept.",
        )
    }

    /// `store.setLayoutPreset(name)`, outside the core overview.
    fn set_layout_preset(&mut self, name: &str) {
        self.layout_settings = layout_preset(Some(name));
        let label = format!("Layout preset: {}", self.layout_settings.preset);
        self.relayout_map(&label);
    }

    /// `store.relayoutMap(label)`: re-run only the graphical automatic layout.
    /// If the map is a reaction subset, keep that same subset.
    fn relayout_map(&mut self, label: &str) {
        let drawn = self.map.reaction_ids();
        if self.map.cells().is_empty() && drawn.is_empty() {
            return;
        }
        if self.relayout_blocked().is_some() {
            return;
        }
        let before = self.map.to_json();
        let options = layout_engine_options(&RawLayoutSettings::from(&self.layout_settings));
        let next = if drawn.is_empty() {
            // Only the shell is drawn, so only the shell is laid out.
            EditableMap::empty_from_community(&self.community, &options)
        } else {
            let next = EditableMap::from_community(&self.community, &options);
            if drawn.len() < next.reactions().len() {
                let keep: Vec<String> = drawn.into_iter().filter(|id| next.has_reaction(id)).collect();
                next.subset_by_reactions(&keep)
            } else {
                next
            }
        };
        self.history.record(&before, &next.to_json(), label);
        self.map = next;
    }
}

// Compartments

/// JS: "the matrix is inside the intermembrane space, whichever way that is spelt"
#[test]
fn the_matrix_is_inside_the_intermembrane_space_whichever_way_that_is_spelt() {
    let (model, _) = alga(0);
    let tree = model.compartment_tree();
    assert_eq!(tree.by_id["i"].parent.as_deref(), Some("c"));
    assert_eq!(tree.by_id["m"].parent.as_deref(), Some("i"), "the matrix sits inside `i`");
    assert_eq!(tree.by_id["s"].parent.as_deref(), Some("c"), "a cell`s own `s` is an organelle, not a root");
    assert_eq!(tree.roots, ["e", "f"]);

    // The other spelling, and neither: a model that declares only one still nests.
    let none = Names::new();
    assert_eq!(build_compartment_tree(&["e", "c", "im", "m"], None, &none).by_id["m"].parent.as_deref(), Some("im"));
    assert_eq!(build_compartment_tree(&["e", "c", "im", "m"], None, &none).by_id["im"].parent.as_deref(), Some("c"));
    assert_eq!(build_compartment_tree(&["e", "c", "m"], None, &none).by_id["m"].parent.as_deref(), Some("c"));
}

/// JS: "ATP synthase crosses the one membrane it sits in"
#[test]
fn atp_synthase_crosses_the_one_membrane_it_sits_in() {
    // With `i` drawn beside the cell it was reported as crossing four.
    let (_, community) = alga(0);
    assert_eq!(reaction_boundary_keys(&community, "sbml_model@ATPSm", None), ["sbml_model/m"]);
}

/// JS: "the shared medium keeps the letter it shares with the eyespot"
#[test]
fn the_shared_medium_keeps_the_letter_it_shares_with_the_eyespot() {
    let (_, community) = alga(0);
    let none = Names::new();
    // The merged model is the one that really contains the medium, and says so.
    let tree = community.merged.compartment_tree();
    assert_eq!(tree.by_id["s"].parent, None);
    assert_eq!(compartment_name("s", &names(&[("s", "Shared medium")]), "en"), "Shared medium");
    assert_eq!(compartment_visual_id("s", &none), "s");
    assert_eq!(membrane_style("s", &none), Membrane::Open);

    let own = names(&[("s", "s")]);
    assert_eq!(cell_compartment_visual_id("s", &own), "es");
    assert_eq!(cell_compartment_name("s", &own, "en"), "Eyespot");
    assert_eq!(membrane_style(&cell_compartment_visual_id("s", &own), &none), Membrane::Bilayer);
    assert_eq!(cell_compartment_visual_id("s", &community.merged.compartment_names), "s");
}

/// JS: "a compartment named after its own id is given its real name"
#[test]
fn a_compartment_named_after_its_own_id_is_given_its_real_name() {
    let given = names(&[("c", "c"), ("h", "h"), ("i", "i"), ("u", ""), ("x", "Glyoxysome")]);
    assert_eq!(compartment_name("c", &given, "en"), "Cytosol");
    assert_eq!(compartment_name("h", &given, "en"), "Chloroplast stroma");
    assert_eq!(compartment_name("i", &given, "en"), "Intermembrane space");
    assert_eq!(compartment_name("u", &given, "en"), "Thylakoid lumen");
    assert_eq!(compartment_name("x", &given, "en"), "Glyoxysome", "a name the model does supply is kept");
    assert_eq!(compartment_name("zz", &names(&[("zz", "zz")]), "en"), "zz", "and an unknown letter stays as it is");
}

/// JS: "the drawn shell names its compartments and nests them"
#[test]
fn the_drawn_shell_names_its_compartments_and_nests_them() {
    let (_, community) = alga(0);
    let map = EditableMap::empty_from_community(&community, &LayoutOptions::default());
    let comp = |id: &str| comp(&map, id).unwrap_or_else(|| panic!("{id} is drawn"));

    assert_eq!(comp("c").label, "Cytosol");
    assert_eq!(comp("s").label, "Eyespot");
    assert_eq!(comp("s").membrane, "bilayer");
    assert_eq!(comp("s").visual_id, "es");
    assert_eq!(comp("i").visual_id, "im");
    assert_eq!(comp("m").parent_id.as_deref(), Some("i"));

    assert!(within(&comp("m").rect, &comp("i").rect));
    assert!(within(&comp("i").rect, &comp("c").rect));
    assert!(within(&comp("s").rect, &comp("c").rect));
}

/// JS: "sibling compartments get room in proportion to what they hold"
#[test]
fn sibling_compartments_get_room_in_proportion_to_what_they_hold() {
    let (_, community) = alga(120);
    let map = EditableMap::empty_from_community(&community, &LayoutOptions::default());
    let area = |id: &str| {
        let r = comp(&map, id).unwrap_or_else(|| panic!("{id} is drawn")).rect;
        r.w * r.h
    };
    // 240 metabolites against one: from equal grid cells the Golgi came out
    // three quarters the size of the chloroplast.
    assert!(area("h") > area("g") * 8.0, "chloroplast {} vs Golgi {}", js::round(area("h")), js::round(area("g")));
    // A flagellum declared beside the cell took half of it.
    assert!(area("e") > area("f") * 8.0, "cell {} vs flagellum {}", js::round(area("e")), js::round(area("f")));

    for c in &map.cells()[0].compartments {
        assert!(c.rect.w >= 60.0 && c.rect.h >= 60.0, "{} is still a box something fits in", c.id);
        let aspect = js::max(c.rect.w / c.rect.h, c.rect.h / c.rect.w);
        if c.parent_id.is_some() {
            assert!(aspect < 3.2, "{} is not a sliver ({}:1)", c.id, js::to_fixed(aspect, 1));
        }
    }
    // Siblings do not overlap.
    let siblings: Vec<&MapCompartment> = map.cells()[0].compartments.iter().filter(|c| c.parent_id.as_deref() == Some("c")).collect();
    for a in &siblings {
        for b in &siblings {
            if a.id == b.id {
                continue;
            }
            let apart = a.rect.x + a.rect.w <= b.rect.x
                || b.rect.x + b.rect.w <= a.rect.x
                || a.rect.y + a.rect.h <= b.rect.y
                || b.rect.y + b.rect.h <= a.rect.y;
            assert!(apart, "{} and {} overlap", a.id, b.id);
        }
    }
}

// Eyespot is not medium

/// JS: "a reaction in the cell`s own `s` is drawn in that compartment, not in the medium"
#[test]
fn a_reaction_in_the_cells_own_s_is_drawn_in_that_compartment_not_in_the_medium() {
    let (_, community) = alga(0);
    let mut map = EditableMap::empty_from_community(&community, &LayoutOptions::default());
    let placed = map.add_reaction(&community, "sbml_model@RETSAT", &PlaceOptions::default()).expect("the reaction is placed");

    let bx = comp(&map, "s").expect("the eyespot is drawn").rect;
    let within = |x: f64, y: f64| x > bx.x && x < bx.x + bx.w && y > bx.y && y < bx.y + bx.h;
    let marker = map.reaction(&placed.reaction_id).expect("the placed reaction is on the map");
    assert!(within(marker.x, marker.y), "the marker is inside the eyespot");
    for id in &placed.nodes {
        let node = map.node(id).expect("a placed node is on the map");
        assert_eq!(node.kind, NodeKind::Metabolite, "{} is an ordinary metabolite", base_of(node));
        assert_eq!(node.cell_id.as_deref(), Some("sbml_model"));
        assert!(within(node.x, node.y), "{} is inside the eyespot", base_of(node));
    }
    assert!(map.medium().nodes.is_empty(), "nothing was registered with the shared medium");

    // And it can be dragged about in there, rather than being held out of every cell.
    let id = placed.nodes[0].clone();
    let moved = map.move_node(&id, bx.x + bx.w / 2.0, bx.y + bx.h / 2.0).expect("the node moves");
    assert!(within(moved.x, moved.y));
}

/// JS: "a real shared-medium metabolite is still a medium node with no cell"
#[test]
fn a_real_shared_medium_metabolite_is_still_a_medium_node_with_no_cell() {
    let community = eco_community();
    let mut map = EditableMap::empty_from_community(&community, &LayoutOptions::default());
    let transfer = community
        .reaction_owner
        .iter()
        .find(|(_, owner)| owner.role == ReactionRole::Transfer)
        .map(|(id, _)| id.clone())
        .expect("the model has a transfer");
    let placed = map.add_reaction(&community, &transfer, &PlaceOptions::default()).expect("the transfer is placed");
    let medium = placed
        .nodes
        .iter()
        .map(|id| map.node(id).expect("a placed node is on the map"))
        .find(|n| n.kind == NodeKind::Medium)
        .expect("the transfer has an end in the shared medium");
    assert_eq!(medium.cell_id, None);
    assert!(map.medium().nodes.contains(&medium.id));
    assert!(!base_of(medium).contains('@'));
}

// Labels

/// JS: "a metabolite is labelled with its own id, not the model`s"
#[test]
fn a_metabolite_is_labelled_with_its_own_id_not_the_models() {
    let (_, community) = alga(0);
    let mut map = EditableMap::empty_from_community(&community, &LayoutOptions::default());
    map.add_reaction(&community, "sbml_model@PGI", &PlaceOptions::default());
    map.add_reaction(&community, "sbml_model@TPT", &PlaceOptions::default());
    map.add_metabolite(&community, "sbml_model@udpgal_c", 400.0, 300.0);

    let labels: Vec<&str> = map.nodes().values().map(base_of).collect();
    assert!(labels.len() >= 7);
    for label in &labels {
        assert!(!label.contains('@'), "{label} carries the model name");
    }
    assert!(labels.contains(&"g6p") && labels.contains(&"udpgal"));

    // The merged model knows the plain name too, which is what keeps the
    // cofactor list from being bypassed by `sbml_model@atp`.
    assert_eq!(community.merged.metabolites["sbml_model@atp_c"].base, "atp");
    assert_eq!(community.merged.metabolites["sbml_model@g6p_c"].base, "g6p");
}

/// JS: "the automatic layout labels cofactors without the model name too"
///
/// The `primaryMetabolitesForSide` assertion at its end is not ported (a UI
/// module).
#[test]
fn the_automatic_layout_labels_cofactors_without_the_model_name_too() {
    let community = eco_community();
    let map = EditableMap::from_community(&community, &LayoutOptions::default());
    let cofactors: Vec<_> = map.nodes().values().filter(|n| n.kind == NodeKind::Currency).collect();
    assert!(cofactors.len() > 50);
    for node in cofactors {
        assert!(!base_of(node).contains('@'), "{} carries the model name", base_of(node));
    }
}

/// JS: "a map saved with the model name in its labels is mended when it is opened"
#[test]
fn a_map_saved_with_the_model_name_in_its_labels_is_mended_when_it_is_opened() {
    let (_, community) = alga(0);
    let mut map = EditableMap::empty_from_community(&community, &LayoutOptions::default());
    map.add_reaction(&community, "sbml_model@PGI", &PlaceOptions::default());
    let mut doc = map.to_json();
    put_model_name_in_labels(&mut doc);
    // And the bare letters the compartments were stored with.
    store_compartments_by_letter(&mut doc);

    let loaded = EditableMap::from_json(&doc).unwrap();
    let mut bases: Vec<&str> = loaded.nodes().values().map(base_of).collect();
    bases.sort_by(|a, b| js::code_unit_compare(a, b));
    assert_eq!(bases, ["f6p", "g6p"]);
    let comp = |id: &str| comp(&loaded, id).unwrap_or_else(|| panic!("{id} is drawn"));
    assert_eq!(comp("c").label, "Cytosol");
    assert_eq!(comp("s").label, "Eyespot");
    assert_eq!(comp("s").membrane, "bilayer");
}

// Maps across versions

/// JS: "a map still opens after the model file is renamed"
#[test]
fn a_map_still_opens_after_the_model_file_is_renamed() {
    let mut store = hand_built_store(&alga_document("sbml_model", 0), "alga_stage16.json");
    assert!(store.add_map_reaction("sbml_model@PGI").is_some());
    let saved = store.map_json();

    // The same model, exported again under the next version's file name.
    let mut next = uploaded_store(&alga_document("sbml_model", 0), "alga_stage17.json");
    assert_ne!(store.species[0].file, next.species[0].file, "the two uploads are known by different file names");
    next.load_map_document(&saved).unwrap();
    assert!(next.map.has_reaction("sbml_model@PGI"));

    // Bundled models are still held to the model they were drawn over.
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

/// JS: "a map follows its model when the model comes back under another id"
#[test]
fn a_map_follows_its_model_when_the_model_comes_back_under_another_id() {
    let mut first = hand_built_store(&alga_document("iAlga_stage15", 0), "alga.json");
    assert!(first.add_map_reaction("iAlga_stage15@PGI").is_some());
    assert!(first.add_map_reaction("iAlga_stage15@TPT").is_some());
    assert!(first.set_map_reaction_linear("iAlga_stage15@PGI", true));
    let saved = first.map_json();
    assert_matches(&saved, "iAlga_stage15@PGI");

    let mut second = uploaded_store(&alga_document("sbml_model", 0), "alga.json");
    second.load_map_document(&saved).unwrap();

    let mut drawn = second.map.reaction_ids();
    drawn.sort_by(|a, b| js::code_unit_compare(a, b));
    assert_eq!(drawn, ["sbml_model@PGI", "sbml_model@TPT"]);
    assert_eq!(second.map.cells()[0].id, "sbml_model");
    let text = map_text(&second.map);
    assert!(!text.contains("iAlga_stage15"), "no identifier still names the old model");
    // The store's own sentence, as transcribed in `adopt_map_species`.
    assert_matches(&second.map_load_note, "iAlga_stage15.*sbml_model");
    // The line a straight reaction is kept on still names nodes that exist.
    let linear = second.map.reaction("sbml_model@PGI").unwrap().linear.as_ref().expect("the reaction is still straight");
    assert!(second.map.has_node(linear.from.as_deref().unwrap_or("")) && second.map.has_node(linear.to.as_deref().unwrap_or("")));
    // The transporter is still on the membrane it was put on.
    assert_eq!(second.map.reaction("sbml_model@TPT").unwrap().boundary_id.as_deref(), Some("sbml_model/h"));
}

/// JS: "a map of a genuinely different model is still refused, and says which reaction"
///
/// The refusal and its wording are `validateMapDocument` and
/// `adoptMapSpecies` of the store, transcribed above; what the map crate
/// contributes is the renamed id the message names.
#[test]
fn a_map_of_a_genuinely_different_model_is_still_refused_and_says_which_reaction() {
    let mut first = hand_built_store(&alga_document("iAlga_stage15", 0), "alga.json");
    assert!(first.add_map_reaction("iAlga_stage15@RETSAT").is_some());
    let saved = first.map_json();

    let mut other = alga_document("sbml_model", 0);
    other["reactions"].as_array_mut().unwrap().retain(|r| r["id"] != "RETSAT");
    let mut second = uploaded_store(&other, "alga.json");
    let refused = second.load_map_document(&saved).expect_err("the map is refused");
    assert_matches(&refused, "sbml_model@RETSAT does not exist.*iAlga_stage15");
}

/// JS: "renaming a species leaves names and free text alone"
#[test]
fn renaming_a_species_leaves_names_and_free_text_alone() {
    let parsed = parse_map_document(&json!({
        "format": "EscherMCMCMap", "version": 3,
        "metadata": { "scenario": "uploaded-models", "species": [{ "id": "old", "model": "upload:a" }] },
        "map": {
            "cells": [{ "id": "old", "name": "old@name stays", "compartments": [] }],
            "nodes": [{ "id": "met:old@R:old@a_c", "mergedId": "old@a_c", "cellId": "old", "name": "old@ in a name", "base": "old@a" }],
            "reactions": [{ "id": "old@R", "cellId": "old", "boundaryId": "old/h", "equation": "old@a_c <=> old@b_c", "name": "old@R name" }],
            "segments": [{ "reactionId": "old@R", "nodeId": "met:old@R:old@a_c" }],
            "texts": [{ "id": "text-1", "text": "old@note", "x": 0, "y": 0 }],
        },
    }))
    .unwrap();
    let renamed = rename_map_species(&parsed, "old", "new", None);
    assert_eq!(renamed.map["nodes"][0]["id"], "met:new@R:new@a_c");
    assert_eq!(renamed.map["reactions"][0]["boundaryId"], "new/h");
    assert_eq!(renamed.map["reactions"][0]["equation"], "new@a_c <=> new@b_c");
    assert_eq!(renamed.map["cells"][0]["name"], "old@name stays");
    assert_eq!(renamed.map["nodes"][0]["name"], "old@ in a name");
    assert_eq!(renamed.map["texts"][0]["text"], "old@note");
    // An id that merely ends the same way is another species.
    let other = rename_map_species(
        &parse_map_document(&json!({
            "format": "EscherMCMCMap", "version": 3, "metadata": null,
            "map": { "cells": [], "nodes": [{ "id": "met:bold@R:bold@a_c" }], "reactions": [], "segments": [] },
        }))
        .unwrap(),
        "old",
        "new",
        None,
    );
    assert_eq!(other.map["nodes"][0]["id"], "met:bold@R:bold@a_c");
}

/// JS: "a saved map can be opened onto a blank canvas"
#[test]
fn a_saved_map_can_be_opened_onto_a_blank_canvas() {
    let mut store = hand_built_store(&alga_document("sbml_model", 0), "alga.json");
    assert!(store.add_map_reaction("sbml_model@PGI").is_some());
    let saved = store.map_json();

    // Clear drawing leaves a canvas with no cells, and a map must still open on
    // it rather than be turned away as "created for a different set of cells".
    store.show_empty_map();
    assert_eq!(store.map.cells().len(), 0);
    store.load_map_document(&saved).unwrap();
    assert!(store.map.has_reaction("sbml_model@PGI"));
    assert_eq!(store.map.cells().len(), 1);
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
/// The nesting those builds read off this same model — `i` and `s` as boxes
/// of their own beside the cell, and the matrix straight in the cytosol —
/// the model's name in front of every label, compartments labelled with their
/// letters, and the reaction of the cell's own `s` compartment drawn out in
/// the shared medium with its metabolites filed as medium nodes.
fn map_from_before(reactions: &[&str], without: &[&str]) -> Before {
    let mut raw = alga_document("sbml_model", 0);
    // OLD_TREE.
    raw["compartment_tree"] = json!({ "e": null, "f": null, "i": null, "s": null, "c": "e", "g": "c", "h": "c", "m": "c", "u": "h" });
    let (_, community) = community_of(Model::from_value(&raw).unwrap());
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
        assert!(rxn["axis"]["a1"].is_object() && rxn["axis"]["a2"].is_object(), "the stray reaction has an axis");
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
    put_model_name_in_labels(&mut doc);
    doc["cells"][0]["compartments"].as_array_mut().unwrap().retain(|c| !without.contains(&c["id"].as_str().unwrap_or("")));
    store_compartments_by_letter(&mut doc);
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

/// JS: "a map drawn before opens with its compartments where the model nests them"
#[test]
fn a_map_drawn_before_opens_with_its_compartments_where_the_model_nests_them() {
    let before = map_from_before(&DRAWN_BEFORE, &[]);
    let mut roots: Vec<&str> = before.doc["cells"][0]["compartments"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c.get("parentId") == Some(&Value::Null))
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    roots.sort_by(|a, b| js::code_unit_compare(a, b));
    assert_eq!(roots, ["e", "f", "i", "s"], "the file really has the old nesting");
    let matrix_was = doc_rect(&doc_comp(&before.doc, "m")["rect"]);

    let mut store = uploaded_store(&alga_document("sbml_model", 0), "alga.json");
    store.load_map_document(&before.text).unwrap();
    let map = &store.map;

    let tree = store.model.compartment_tree();
    for comp in &map.cells()[0].compartments {
        assert_eq!(comp.parent_id, tree.by_id[&comp.id].parent, "{} is nested as the model has it", comp.id);
        if let Some(parent) = comp.parent_id.as_deref().filter(|p| !p.is_empty()) {
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

/// JS: "what was drawn in the right place stays exactly where it was put"
#[test]
fn what_was_drawn_in_the_right_place_stays_exactly_where_it_was_put() {
    let before = map_from_before(&DRAWN_BEFORE, &[]);
    let mut store = uploaded_store(&alga_document("sbml_model", 0), "alga.json");
    store.load_map_document(&before.text).unwrap();

    for rid in ["sbml_model@PGI", "sbml_model@TPT"] {
        let was = before.doc["reactions"].as_array().unwrap().iter().find(|r| r["id"] == rid).expect("the file draws it");
        let now = store.map.reaction(rid).unwrap_or_else(|| panic!("{rid} is still drawn"));
        assert_eq!((now.x, now.y), (num(&was["x"]), num(&was["y"])), "{rid} has not moved");
        for node in before.doc["nodes"].as_array().unwrap() {
            let owner = node.get("reactionLocalFor").filter(|v| !v.is_null()).or_else(|| node.get("reactionId"));
            if owner.and_then(Value::as_str) != Some(rid) {
                continue;
            }
            let id = node["id"].as_str().unwrap();
            let after = store.map.node(id).unwrap_or_else(|| panic!("{id} is still drawn"));
            assert_eq!((after.x, after.y), (num(&node["x"]), num(&node["y"])), "{id} has not moved");
        }
    }
    // Labels and names are mended on the way, as for any earlier file.
    for node in store.map.nodes().values() {
        assert!(!base_of(node).contains('@'));
    }
    assert_eq!(comp(&store.map, "c").unwrap().label, "Cytosol");
}

/// JS: "what the old nesting had put in the wrong place is drawn where it belongs"
#[test]
fn what_the_old_nesting_had_put_in_the_wrong_place_is_drawn_where_it_belongs() {
    let before = map_from_before(&DRAWN_BEFORE, &[]);
    let mut store = uploaded_store(&alga_document("sbml_model", 0), "alga.json");
    store.load_map_document(&before.text).unwrap();
    let map = &store.map;
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

/// JS: "opening an old map is one undo step, and saving it again leaves nothing to mend"
#[test]
fn opening_an_old_map_is_one_undo_step_and_saving_it_again_leaves_nothing_to_mend() {
    let before = map_from_before(&DRAWN_BEFORE, &[]);
    let mut store = uploaded_store(&alga_document("sbml_model", 0), "alga.json");
    let full = store.map.reactions().len();
    store.load_map_document(&before.text).unwrap();
    assert_eq!(store.history.undo_len(), 1);

    let mended = store.map_json();
    let shell = map_text(&store.map);
    store.load_map_document(&mended).unwrap();
    assert_eq!(store.map_load_note, "", "a map saved now needs nothing done to it");
    assert_eq!(map_text(&store.map), shell);

    store.undo_map();
    store.undo_map();
    assert_eq!(store.map.reactions().len(), full, "undo brings back the map that was open before");
}

/// JS: "a compartment the model has put in between is drawn if the map never had it"
#[test]
fn a_compartment_the_model_has_put_in_between_is_drawn_if_the_map_never_had_it() {
    // Cell, cytosol and matrix drawn by hand under the old nesting; no `i` at all.
    let before = map_from_before(&["PGI"], &["i", "s", "f"]);
    let mut store = uploaded_store(&alga_document("sbml_model", 0), "alga.json");
    store.load_map_document(&before.text).unwrap();
    let map = &store.map;

    let i = comp(map, "i").expect("the intermembrane space is drawn");
    assert_eq!(i.parent_id.as_deref(), Some("c"));
    assert_eq!(i.label, "Intermembrane space");
    assert_eq!(comp(map, "m").expect("the matrix is drawn").parent_id.as_deref(), Some("i"));
    assert!(within(&rect_of(map, "m").unwrap(), &i.rect));
    assert!(map.boundary("sbml_model/i").is_some(), "and it has a membrane to put reactions on");

    // The reaction that needs both can now be added and lands where it should.
    let placed = store.add_map_reaction("sbml_model@ATPSm").expect("ATP synthase can be added");
    assert_eq!(store.map.reaction(&placed.reaction_id).unwrap().boundary_id.as_deref(), Some("sbml_model/m"));
}

/// JS: "a map that needs nothing comes through untouched"
#[test]
fn a_map_that_needs_nothing_comes_through_untouched() {
    let mut store = hand_built_store(&alga_document("sbml_model", 0), "alga.json");
    for id in ["PGI", "TPT", "ATPSm", "RETSAT"] {
        assert!(store.add_map_reaction(&format!("sbml_model@{id}")).is_some());
    }
    let (id, x, y) = store.map.nodes().values().next().map(|n| (n.id.clone(), n.x, n.y)).expect("something is drawn");
    store.move_node(&id, x + 31.0, y - 17.0);
    let saved = store.map_json();
    let drawn = map_text(&store.map);

    let mut next = uploaded_store(&alga_document("sbml_model", 0), "alga.json");
    next.load_map_document(&saved).unwrap();
    assert_eq!(map_text(&next.map), drawn);
    assert_eq!(next.map_load_note, "");
}

/// JS: "reactions the model has since lost are left out and the rest of the map is kept"
#[test]
fn reactions_the_model_has_since_lost_are_left_out_and_the_rest_of_the_map_is_kept() {
    let mut first = hand_built_store(&alga_document("sbml_model", 0), "alga.json");
    for id in ["PGI", "TPT", "RETSAT", "PQR"] {
        assert!(first.add_map_reaction(&format!("sbml_model@{id}")).is_some());
    }
    let saved = first.map_json();
    let at = |map: &EditableMap, id: &str| map.reaction(id).map(|r| (r.x, r.y)).unwrap_or_else(|| panic!("{id} is drawn"));
    let tpt = at(&first.map, "sbml_model@TPT");
    let pqr = at(&first.map, "sbml_model@PQR");

    // The next export of the model: one reaction gone, one metabolite renamed,
    // one reaction given another metabolite.
    let mut next = alga_document("sbml_model", 0);
    next["reactions"].as_array_mut().unwrap().retain(|r| r["id"] != "RETSAT");
    let renamed = next["metabolites"].as_array_mut().unwrap().iter_mut().find(|m| m["id"] == "f6p_c").unwrap();
    renamed["id"] = json!("f6p_B_c");
    let reaction = |doc: &mut Value, id: &str| -> usize { doc["reactions"].as_array().unwrap().iter().position(|r| r["id"] == id).unwrap() };
    let pgi = reaction(&mut next, "PGI");
    next["reactions"][pgi]["metabolites"] = json!({ "g6p_c": -1, "f6p_B_c": 1 });
    let pqr_at = reaction(&mut next, "PQR");
    next["reactions"][pqr_at]["metabolites"]["atp_c"] = json!(-1);

    let mut second = uploaded_store(&next, "alga.json");
    second.load_map_document(&saved).unwrap();
    let map = &second.map;

    let mut drawn = map.reaction_ids();
    drawn.sort_by(|a, b| js::code_unit_compare(a, b));
    assert_eq!(drawn, ["sbml_model@PGI", "sbml_model@PQR", "sbml_model@TPT"]);
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

/// JS: "a map that shares nothing with the open model is still turned away"
///
/// The refusal is `validateMapDocument` of the store, transcribed above: this
/// keeps the sequence honest (nothing is applied before the check) rather
/// than testing the map crate.
#[test]
fn a_map_that_shares_nothing_with_the_open_model_is_still_turned_away() {
    let mut first = hand_built_store(&alga_document("sbml_model", 0), "alga.json");
    for id in ["PGI", "TPT"] {
        assert!(first.add_map_reaction(&format!("sbml_model@{id}")).is_some());
    }
    let saved = first.map_json();

    let mut other = alga_document("sbml_model", 0);
    for rxn in other["reactions"].as_array_mut().unwrap() {
        rxn["id"] = json!(format!("X_{}", rxn["id"].as_str().unwrap()));
    }
    let mut second = uploaded_store(&other, "alga.json");
    let open = map_text(&second.map);
    let refused = second.load_map_document(&saved).expect_err("the map is turned away");
    assert_matches(&refused, "None of the 2 reactions on this map exist in the active model");
    assert_eq!(map_text(&second.map), open, "and the map that was open is still open");
}

// Layout presets

/// JS: "a layout preset does not lay a hand-built map of a large model out again"
///
/// That the preset is held back is `relayoutBlocked` of the store,
/// transcribed above; the map crate's part is that a model this size has no
/// overview to fall back on and opens as an empty shell. `store.status.message`
/// is not ported.
#[test]
fn a_layout_preset_does_not_lay_a_hand_built_map_of_a_large_model_out_again() {
    let mut store = uploaded_store(&alga_document("sbml_model", 400), "alga.json");
    assert_eq!(store.map.reactions().len(), 0, "too large to draw automatically: the shell is empty");
    store.add_map_reaction("sbml_model@PGI");
    let (id, x, y) = store.map.nodes().values().next().map(|n| (n.id.clone(), n.x, n.y)).expect("something is drawn");
    store.move_node(&id, x + 77.0, y + 33.0);
    let placed = store.map.node(&id).map(|n| (n.x, n.y)).unwrap();

    assert_matches(store.relayout_blocked().unwrap_or(""), "hand-built drawing");
    store.set_layout_preset("spacious");

    let after = store.map.node(&id).expect("the node is still drawn");
    assert_eq!((after.x, after.y), placed, "what was placed by hand is where it was put");
}

/// JS: "a preset on a large model with nothing drawn lays out the shell alone"
///
/// The JavaScript proves the full layout is not run by replacing
/// `EditableMap.fromCommunity` with a function that throws; here the
/// transcribed `relayout_map` takes the shell-only branch by the same
/// condition.
#[test]
fn a_preset_on_a_large_model_with_nothing_drawn_lays_out_the_shell_alone() {
    let mut store = uploaded_store(&alga_document("sbml_model", 400), "alga.json");
    assert_eq!(store.map.reactions().len(), 0);
    let width = store.map.width();
    store.set_layout_preset("compact");
    assert!(store.map.width() < width, "the shell was laid out again, smaller");
    assert_eq!(store.map.reactions().len(), 0);
    assert_eq!(store.map.cells().len(), 1);
}

/// JS: "a small model is still laid out again by a preset, and one undo brings the old map back"
#[test]
fn a_small_model_is_still_laid_out_again_by_a_preset_and_one_undo_brings_the_old_map_back() {
    let mut store = uploaded_store(&alga_document("sbml_model", 0), "alga.json");
    assert_eq!(store.relayout_blocked(), None);
    let width = store.map.width();
    let before = map_text(&store.map);
    store.set_layout_preset("compact");
    assert!(store.map.width() < width, "the compact preset draws a smaller map");
    store.undo_map();
    assert!(map_text(&store.map) == before, "undo restores the map as it was");
}
