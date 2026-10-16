//! Arranging one reaction as a drawing (port of `test/reaction-arrange.test.mjs`).
//!
//! One Rust test per JavaScript test, named after it. `assert.equal` and
//! `assert.deepEqual` on numbers are strict in JavaScript (`Object.is`: `-0`
//! is not `0`), so numbers compared that way go through [`is`] here.
//!
//! Not ported (whole tests — nothing in them is about the map but its set-up):
//!
//! // Not ported: "selecting a reaction does not put control points over its
//! //   marker" — only `renderMap` output (crate `coxeter-render`).
//! // Not ported: "Edit Control Points shows the two curve handles and a
//! //   lever, and no backbone points" — only `renderMap` output.
//! // Not ported: "a linear reaction offers no lever and no handles on the two
//! //   branches it keeps straight" — only `renderMap` output.
//! // Not ported: "the reaction inspector offers linear, swap, mirror and
//! //   rotate as map options" — only `renderInspector` HTML (UI panels).
//! // Not ported: "cofactor labels start enabled and can be turned off from
//! //   the View menu" — `store.view`, `renderViewControls` and `renderMap`.
//!
//! Not ported (assertions of tests that are otherwise kept):
//!
//! // Not ported: "the inspector options are undoable edits that leave the
//! //   model alone" — `assert.match(renderInspector(store), /Kept straight
//! //   between f6p and fdp/)`.
//! // Not ported: "metabolites picked out on the map are the ones Linear and
//! //   the compass use" — the `renderInspector` match for the active
//! //   line-end button.
//! // Not ported: "the name of a straight reaction lying on its side stands
//! //   clear of its arm" — every assertion on where `renderMap` puts the
//! //   reaction's name. Kept: the alignments it makes are accepted.
//! // Not ported: "the inspector offers the compass, the metabolite choice and
//! //   tidy" — the `renderInspector` assertions (compass buttons, line-end
//! //   buttons, tidy button). Kept: align is one undo step, tidying what is
//! //   tidy is none, undo releases the line.
//! // Not ported: "a symporter shows one ordinary arrow per metabolite,
//! //   pointing the way it carries them" — the lane arrows of `renderMap`
//! //   (`laneArrows`, `emc-lane-head`, `emc-lane`, `emc-seg-transport-lane`).
//! //   Kept: transport mode, equation, boundary, lane segments.
//! // Not ported: "an antiporter shows two arrows pointing opposite ways" —
//! //   the lane arrows of `renderMap`. Kept: the transport mode.
//! // Not ported: "the arrows stay on their lanes when a transporter is
//! //   carried round a corner" — the lane arrows and the glyph transform of
//! //   `renderMap`. Kept: the move and the membrane normal after it.
//!
//! The store. Four tests go through `Store` (`src/ui/store.js`). Its map
//! methods are `editMapReaction(label, edit)`: take `map.toJSON()`, run the
//! `EditableMap` method, and if it succeeded `mapHistory.record(before,
//! map.toJSON(), label)`; `undoMap()` is `EditableMap.fromJSON(
//! mapHistory.undo(map.toJSON()).doc)`. [`MiniStore`] is exactly that over
//! `coxeter_map::history::MapHistory`, with `store.multiSelection.nodes` as a
//! set of picked node ids and `selectedLineEnds` re-written from the store
//! (it is the store's own logic over `reactionSides`, so the assertions on
//! its result test this file's copy of it together with `reaction_sides`).
//! `store.mapHistory.undoStack.length` is `MapHistory::undo_len`.
//!
//! Identity. `assert.equal(axis.from, rxn.axis.a1)` (the same object) is
//! here: the map's segment link says that end is that axis point, and the
//! coordinates are the same.

mod common;

use std::sync::Arc;

use common::load_model;
use coxeter_core::community::{CommunityModel, CommunitySpec, SpeciesSpec};
use coxeter_core::js;
use coxeter_core::mapdata::{AxisEnd, Node, Point, Rect, Segment, SegmentKind};
use coxeter_map::editable_map::DuplicateOptions;
use coxeter_map::history::MapHistory;
use coxeter_map::reaction_edit::{Handle, SideEntry};
use coxeter_map::{EditableMap, LayoutOptions, PlaceOptions};
use indexmap::{IndexMap, IndexSet};
use serde_json::{Value, json};

const RID: &str = "eco@PFK";

/// `Object.is(a, b)` for two numbers.
fn is(a: f64, b: f64) -> bool {
    a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
}

/// `assert.deepEqual({x: a.x, y: a.y}, {x: b.x, y: b.y})`.
fn same_point(a: Point, b: Point) -> bool {
    is(a.x, b.x) && is(a.y, b.y)
}

fn community_of(species_id: &str, model: &str) -> CommunityModel {
    CommunityModel::new(CommunitySpec {
        species: vec![SpeciesSpec::new(species_id, Arc::new(load_model(model))).abundance(1.0)],
        ..Default::default()
    })
    .unwrap()
}

fn community() -> CommunityModel {
    community_of("eco", "ecoli_core_mc")
}

fn add(map: &mut EditableMap, com: &CommunityModel, rid: &str) -> bool {
    map.add_reaction(com, rid, &PlaceOptions::default()).is_some()
}

fn pfk_map(extra: &[&str]) -> (CommunityModel, EditableMap) {
    let com = community();
    let mut map = EditableMap::empty_from_community(&com, &LayoutOptions::default());
    for rid in std::iter::once(RID).chain(extra.iter().copied()) {
        assert!(add(&mut map, &com, rid), "{rid} is placed");
    }
    (com, map)
}

/// `map.segments.filter((seg) => seg.reactionId === rid && seg.nodeId)`, as
/// they are now (JavaScript holds the live segments; here they are read again
/// after every edit).
fn branches(map: &EditableMap, rid: &str) -> Vec<Segment> {
    map.segments()
        .iter()
        .filter(|seg| seg.reaction_id == rid && seg.node_id.as_deref().is_some_and(|n| !n.is_empty()))
        .cloned()
        .collect()
}

fn node_id(seg: &Segment) -> &str {
    seg.node_id.as_deref().expect("a branch has a node")
}

fn metabolite_end(seg: &Segment) -> Point {
    if seg.side.as_deref() == Some("substrate") { seg.from } else { seg.to }
}

fn reaction_end(seg: &Segment) -> Point {
    if seg.side.as_deref() == Some("substrate") { seg.to } else { seg.from }
}

fn near(a: Point, b: Point, eps: f64) -> bool {
    js::hypot(a.x - b.x, a.y - b.y) < eps
}

/// `[...map.nodes.values()].find((n) => n.base === base)`, as it is now.
fn node_of(map: &EditableMap, base: &str) -> Node {
    map.nodes().values().find(|n| n.base.as_deref() == Some(base)).unwrap_or_else(|| panic!("no node {base}")).clone()
}

/// Where a node is now.
fn at(map: &EditableMap, id: &str) -> Point {
    map.node(id).unwrap_or_else(|| panic!("no node {id}")).pos()
}

/// Where a reaction's marker is now.
fn marker(map: &EditableMap, rid: &str) -> Point {
    let rxn = map.reaction(rid).unwrap_or_else(|| panic!("no reaction {rid}"));
    Point::new(rxn.x, rxn.y)
}

/// `rxn.axis.dir`, as it is now.
fn axis_dir(map: &EditableMap, rid: &str) -> Point {
    map.reaction(rid).unwrap().axis.expect("the reaction has an axis").dir
}

/// `[rxn.linear.from, rxn.linear.to]`.
fn linear_ends(map: &EditableMap, rid: &str) -> (String, String) {
    let linear = map.reaction(rid).unwrap().linear.as_ref().expect("the reaction is linear");
    (linear.from.clone().expect("linear.from"), linear.to.clone().expect("linear.to"))
}

fn midpoint(a: Point, b: Point) -> Point {
    Point::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0)
}

/// `[n.id, n.x, n.y]` of every node no reaction owns (`!n.reactionId`).
fn free_nodes(map: &EditableMap) -> Vec<(String, u64, u64)> {
    map.nodes()
        .values()
        .filter(|n| n.reaction_id.as_deref().unwrap_or("").is_empty())
        .map(|n| (n.id.clone(), n.x.to_bits(), n.y.to_bits()))
        .collect()
}

fn labels(entries: &[SideEntry]) -> Vec<&str> {
    entries.iter().map(|s| s.label.as_str()).collect()
}

fn entry<'a>(entries: &'a [SideEntry], label: &str) -> &'a SideEntry {
    entries.iter().find(|e| e.label == label).unwrap_or_else(|| panic!("no {label} on that side"))
}

/// A reaction is still one drawing: every branch ends on its node and on its backbone.
#[track_caller]
fn assert_intact(map: &EditableMap, rid: &str) {
    let rxn = map.reaction(rid).unwrap();
    for seg in branches(map, rid) {
        let node = map.node(node_id(&seg)).unwrap_or_else(|| panic!("no node {}", node_id(&seg)));
        let base = node.base.as_deref().unwrap_or("");
        assert!(near(metabolite_end(&seg), node.pos(), 1e-6), "{base}: the branch ends on its metabolite");
        assert!(
            js::hypot(reaction_end(&seg).x - rxn.x, reaction_end(&seg).y - rxn.y) < 30.0,
            "{base}: the branch leaves from the backbone"
        );
    }
    let i = map.segments().iter().position(|seg| seg.reaction_id == rid && seg.kind == SegmentKind::Axis).expect("an axis segment");
    let axis = &map.segments()[i];
    let ends = rxn.axis.expect("the reaction has an axis");
    // assert.equal(axis.from, rxn.axis.a1); assert.equal(axis.to, rxn.axis.a2);
    assert_eq!(map.segment_links()[i], (Some(AxisEnd::A1), Some(AxisEnd::A2)), "the axis segment is the reaction's axis");
    assert!(same_point(axis.from, ends.a1), "axis.from {:?} is not a1 {:?}", axis.from, ends.a1);
    assert!(same_point(axis.to, ends.a2), "axis.to {:?} is not a2 {:?}", axis.to, ends.a2);
}

// The store

/// What a test uses of `Store` (see the top of the file).
struct MiniStore {
    community: CommunityModel,
    map: EditableMap,
    history: MapHistory,
    /// `store.multiSelection.nodes`.
    picked: IndexSet<String>,
}

/// `{label, before}` of `beginMapEdit`.
struct MapEdit {
    label: String,
    before: Value,
}

impl MiniStore {
    /// `new Store('.')` with `store.community` and an empty map of it.
    fn new(community: CommunityModel) -> MiniStore {
        let map = EditableMap::empty_from_community(&community, &LayoutOptions::default());
        MiniStore { community, map, history: MapHistory::new(100), picked: IndexSet::new() }
    }

    /// `store.mapHistory.undoStack.length`.
    fn depth(&self) -> usize {
        self.history.undo_len()
    }

    fn add_map_reaction(&mut self, rid: &str) -> bool {
        let before = self.map.to_json();
        if self.map.add_reaction(&self.community, rid, &PlaceOptions::default()).is_none() {
            return false;
        }
        self.history.record(&before, &self.map.to_json(), "Add reaction");
        true
    }

    fn begin_map_edit(&self, label: &str) -> MapEdit {
        MapEdit { label: label.to_string(), before: self.map.to_json() }
    }

    fn commit_map_edit(&mut self, transaction: MapEdit) -> bool {
        self.history.record(&transaction.before, &self.map.to_json(), &transaction.label)
    }

    /// One undoable change to how a single reaction is drawn.
    fn edit_map_reaction(&mut self, label: &str, edit: impl FnOnce(&mut EditableMap) -> bool) -> bool {
        let before = self.map.to_json();
        if !edit(&mut self.map) {
            return false;
        }
        self.history.record(&before, &self.map.to_json(), label);
        true
    }

    fn undo_map(&mut self) -> bool {
        let Some(entry) = self.history.undo(&self.map.to_json()) else { return false };
        self.map = EditableMap::from_json(&entry.doc).expect("history holds maps");
        true
    }

    /// `toggleMultiSelection('node', id)`.
    fn toggle_node(&mut self, id: &str) {
        if !self.picked.shift_remove(id) {
            self.picked.insert(id.to_string());
        }
    }

    /// `selectedLineEnds`: the picked metabolites of this reaction as the two
    /// its line should run through — a side with none picked, or with more
    /// than one, is left for the reaction to decide.
    fn selected_line_ends(&self, rid: &str) -> (Option<String>, Option<String>) {
        if self.picked.is_empty() {
            return (None, None);
        }
        let (substrates, products) = self.map.reaction_sides(rid);
        let one = |entries: &[SideEntry]| {
            let chosen: Vec<&SideEntry> = entries.iter().filter(|e| self.picked.contains(&e.node_id)).collect();
            (chosen.len() == 1).then(|| chosen[0].node_id.clone())
        };
        (one(&substrates), one(&products))
    }

    fn rotate_map_reaction(&mut self, rid: &str, angle: f64) -> bool {
        self.edit_map_reaction("Rotate reaction", |map| map.rotate_reaction(rid, angle))
    }

    fn set_map_reaction_linear(&mut self, rid: &str, linear: bool) -> bool {
        let (from, to) = if linear { self.selected_line_ends(rid) } else { (None, None) };
        self.edit_map_reaction(if linear { "Make reaction straight" } else { "Release straight reaction" }, |map| {
            map.set_reaction_linear(rid, linear, from.as_deref(), to.as_deref())
        })
    }

    fn align_map_reaction(&mut self, rid: &str, direction: &str) -> bool {
        let (from, to) = self.selected_line_ends(rid);
        self.edit_map_reaction(&format!("Align reaction {direction}"), |map| {
            map.align_reaction(rid, direction, from.as_deref(), to.as_deref())
        })
    }

    fn tidy_map_reaction(&mut self, rid: &str) -> bool {
        self.edit_map_reaction("Tidy reaction", |map| map.tidy_reaction(rid))
    }

    fn swap_map_reaction_metabolites(&mut self, rid: &str, a: &str, b: &str) -> bool {
        self.edit_map_reaction("Swap metabolites", |map| map.swap_reaction_metabolites(rid, a, b))
    }

    fn flip_map_reaction(&mut self, rid: &str) -> bool {
        self.edit_map_reaction("Swap substrates and products", |map| map.flip_reaction(rid))
    }

    fn mirror_map_reaction(&mut self, rid: &str) -> bool {
        self.edit_map_reaction("Mirror reaction", |map| map.mirror_reaction(rid))
    }
}

/// `storeWithPFK()`: the parts of it that are the map and its history.
fn store_with_pfk() -> MiniStore {
    let mut store = MiniStore::new(community());
    store.add_map_reaction(RID);
    store
}

// Undo history

/// JS: 'edits made in place are recorded, so one undo takes back one edit'
#[test]
fn edits_made_in_place_are_recorded_so_one_undo_takes_back_one_edit() {
    /*
     * The history is given the document from before an edit and compares it with
     * the one after, so that document has to be a copy. Were it the live nodes
     * and segments, a drag (which changes them where they stand) would compare
     * equal to itself and be dropped. Only edits that add or remove something
     * would be kept, and the first undo would go straight back past every move
     * to the last of those.
     */
    let (_com, mut map) = pfk_map(&[]);
    let mut history = MapHistory::default();
    fn edit(map: &mut EditableMap, history: &mut MapHistory, label: &str, change: impl FnOnce(&mut EditableMap)) -> bool {
        let before = map.to_json();
        change(map);
        history.record(&before, &map.to_json(), label)
    }

    let node = node_of(&map, "f6p");
    let start = node.pos();
    assert!(edit(&mut map, &mut history, "move metabolite", |map| {
        map.move_node(&node.id, node.x + 40.0, node.y + 25.0);
    }));
    let rxn = marker(&map, RID);
    assert!(edit(&mut map, &mut history, "move reaction", |map| {
        map.move_reaction(RID, rxn.x + 30.0, rxn.y);
    }));
    let seg = branches(&map, RID)[0].clone();
    let b1 = seg.b1.expect("the branch has a b1 handle");
    assert!(edit(&mut map, &mut history, "move control point", |map| {
        map.move_branch_control(RID, node_id(&seg), Handle::B1, b1.x + 9.0, b1.y + 9.0);
    }));
    assert!(edit(&mut map, &mut history, "make secondary", |map| {
        map.set_node_primary(&node.id, false);
    }));
    assert_eq!(history.undo_len(), 4);

    // Undoing the last one leaves the three before it standing.
    let undone = EditableMap::from_json(&history.undo(&map.to_json()).expect("an edit to undo").doc).unwrap();
    let again = node_of(&undone, "f6p");
    assert_ne!(again.primary, Some(false), "the role change is undone");
    assert!((again.x - (start.x + 40.0)).abs() < 1e-9, "and the move before it is not");
}

/// JS: 'a saved document does not change when the map it came from is edited'
#[test]
fn a_saved_document_does_not_change_when_the_map_it_came_from_is_edited() {
    let (_com, mut map) = pfk_map(&[]);
    let doc = map.to_json();
    let node = node_of(&map, "f6p");
    let saved = |doc: &Value| -> (f64, f64) {
        let n = doc["nodes"].as_array().unwrap().iter().find(|n| n["id"] == node.id.as_str()).expect("the node is in the document");
        (n["x"].as_f64().unwrap(), n["y"].as_f64().unwrap())
    };
    let was = saved(&doc);
    map.move_node(&node.id, node.x + 80.0, node.y + 80.0);
    let now = saved(&doc);
    assert!(is(now.0, was.0) && is(now.1, was.1));
}

/// JS: 'the store records a drag and a turn as separate undo steps'
#[test]
fn the_store_records_a_drag_and_a_turn_as_separate_undo_steps() {
    let mut store = MiniStore::new(community());
    store.add_map_reaction(RID);
    assert_eq!(store.depth(), 1);

    let node = node_of(&store.map, "fdp");
    let drag = store.begin_map_edit("Move metabolite");
    store.map.move_node(&node.id, node.x + 50.0, node.y + 10.0);
    assert!(store.commit_map_edit(drag), "a drag is an edit");
    assert_eq!(store.depth(), 2);

    let untouched = store.begin_map_edit("Move metabolite");
    assert!(!store.commit_map_edit(untouched), "a press that moved nothing is not");

    assert!(store.rotate_map_reaction(RID, std::f64::consts::PI / 2.0));
    assert_eq!(store.depth(), 3);

    let moved = at(&store.map, &node.id);
    store.undo_map();
    let after = node_of(&store.map, "fdp");
    assert!(same_point(after.pos(), moved), "undo takes back the turn, not the drag");
    assert_eq!(store.depth(), 2);
}

// Rotation

/// JS: 'turning a reaction turns its backbone and what it owns, and nothing else'
#[test]
fn turning_a_reaction_turns_its_backbone_and_what_it_owns_and_nothing_else() {
    let (_com, mut map) = pfk_map(&[]);
    let dir = axis_dir(&map, RID);
    let centre = marker(&map, RID);
    let own: Vec<Node> = map.nodes().values().filter(|n| n.reaction_id.as_deref() == Some(RID)).cloned().collect();
    let free = free_nodes(&map);
    let radius: Vec<f64> = own.iter().map(|n| js::hypot(n.x - centre.x, n.y - centre.y)).collect();

    assert!(map.rotate_reaction(RID, std::f64::consts::PI / 2.0));

    let rxn = marker(&map, RID);
    assert!(same_point(rxn, centre), "it turns about its own centre");
    let now = axis_dir(&map, RID);
    assert!((now.x - -dir.y).abs() < 1e-9 && (now.y - dir.x).abs() < 1e-9, "the backbone has made a quarter turn");
    assert_eq!(free_nodes(&map), free, "metabolites that stand on their own stay where they are");
    for (i, n) in own.iter().enumerate() {
        let p = at(&map, &n.id);
        assert!(
            (js::hypot(p.x - rxn.x, p.y - rxn.y) - radius[i]).abs() < 1e-6,
            "{} went round with the backbone",
            n.base.as_deref().unwrap_or("")
        );
    }
    assert_intact(&map, RID);
}

/// JS: 'a backbone can be set to an angle, and a transporter keeps the one its membrane gives it'
#[test]
fn a_backbone_can_be_set_to_an_angle_and_a_transporter_keeps_the_one_its_membrane_gives_it() {
    let (com, mut map) = pfk_map(&[]);
    assert!(map.set_reaction_angle(RID, 0.0));
    assert!(map.reaction_angle(RID).expect("the reaction has an angle").abs() < 1e-9);
    assert!((axis_dir(&map, RID).x - 1.0).abs() < 1e-9);

    assert!(add(&mut map, &com, "eco@PIt2rpp"));
    assert!(map.reaction("eco@PIt2rpp").unwrap().on_membrane);
    assert!(!map.can_rotate_reaction("eco@PIt2rpp"));
    assert!(!map.rotate_reaction("eco@PIt2rpp", 1.0));
}

// Straight reactions

/// JS: 'a linear reaction lies on the line between its substrate and its product'
#[test]
fn a_linear_reaction_lies_on_the_line_between_its_substrate_and_its_product() {
    let (_com, mut map) = pfk_map(&[]);
    assert!(map.can_linearize_reaction(RID));
    assert!(map.set_reaction_linear(RID, true, None, None));

    let (from_id, to_id) = linear_ends(&map, RID);
    let from = map.node(&from_id).unwrap();
    let to = map.node(&to_id).unwrap();
    // It runs between the two metabolites the reaction is about, not its cofactors.
    assert_eq!([from.base.as_deref(), to.base.as_deref()], [Some("f6p"), Some("fdp")]);
    assert!(near(marker(&map, RID), midpoint(from.pos(), to.pos()), 1e-6), "the marker is midway");

    let length = js::hypot(to.x - from.x, to.y - from.y);
    let along = Point::new((to.x - from.x) / length, (to.y - from.y) / length);
    assert!(near(axis_dir(&map, RID), along, 1e-6), "and turned along the line");

    // Both branches are that line: every point of them is on it.
    let off = |p: Point| ((p.x - from.x) * -along.y + (p.y - from.y) * along.x).abs();
    for seg in branches(&map, RID).iter().filter(|s| node_id(s) == from_id || node_id(s) == to_id) {
        for p in [seg.from, seg.b1.expect("b1"), seg.b2.expect("b2"), seg.to] {
            assert!(off(p) < 1e-6);
        }
    }
    assert_intact(&map, RID);
}

/// JS: 'moving a metabolite takes a linear reaction with it'
#[test]
fn moving_a_metabolite_takes_a_linear_reaction_with_it() {
    let (_com, mut map) = pfk_map(&[]);
    map.set_reaction_linear(RID, true, None, None);
    let (from_id, to_id) = linear_ends(&map, RID);
    let cofactor = map.nodes().values().find(|n| n.reaction_id.as_deref() == Some(RID)).expect("a cofactor").id.clone();
    let rxn = marker(&map, RID);
    let c = at(&map, &cofactor);
    let reach = js::hypot(c.x - rxn.x, c.y - rxn.y);

    let from = at(&map, &from_id);
    map.move_node(&from_id, from.x + 130.0, from.y + 70.0);

    let rxn = marker(&map, RID);
    assert!(
        near(rxn, midpoint(at(&map, &from_id), at(&map, &to_id)), 1e-6),
        "the marker is midway between where the two are now"
    );
    let c = at(&map, &cofactor);
    assert!((js::hypot(c.x - rxn.x, c.y - rxn.y) - reach).abs() < 1e-6, "and its cofactors came along at the same distance");
    assert_intact(&map, RID);

    // The free reaction it is compared with: the marker stays behind.
    let (_com, mut other) = pfk_map(&[]);
    let still = marker(&other, RID);
    let node = node_of(&other, "f6p");
    other.move_node(&node.id, node.x + 130.0, node.y + 70.0);
    assert!(same_point(marker(&other, RID), still));
}

/// JS: 'a linear reaction slides along its line, cannot be turned, and can be released'
#[test]
fn a_linear_reaction_slides_along_its_line_cannot_be_turned_and_can_be_released() {
    let (_com, mut map) = pfk_map(&[]);
    map.set_reaction_linear(RID, true, None, None);
    let (from_id, to_id) = linear_ends(&map, RID);
    let from = at(&map, &from_id);

    // Dragged far off to one side, the marker goes as far as the line allows.
    map.move_reaction(RID, from.x + 900.0, from.y - 900.0);
    let rxn = marker(&map, RID);
    let from = at(&map, &from_id);
    let to = at(&map, &to_id);
    let t = js::hypot(rxn.x - from.x, rxn.y - from.y) / js::hypot(to.x - from.x, to.y - from.y);
    assert!(t > 0.1 && t < 0.9, "the marker stays between its ends (t = {})", js::to_fixed(t, 2));
    let cross = (rxn.x - from.x) * (to.y - from.y) - (rxn.y - from.y) * (to.x - from.x);
    assert!(cross.abs() < 1e-6, "and on the line");

    assert!(!map.can_rotate_reaction(RID));
    assert!(!map.rotate_reaction(RID, 1.0));

    assert!(map.set_reaction_linear(RID, false, None, None));
    assert!(map.reaction(RID).unwrap().linear.is_none());
    assert!(map.can_rotate_reaction(RID));
}

/// JS: 'linear survives saving, and a merge hands the line on to the node that took over'
#[test]
fn linear_survives_saving_and_a_merge_hands_the_line_on_to_the_node_that_took_over() {
    let (_com, mut map) = pfk_map(&[]);
    map.set_reaction_linear(RID, true, None, None);
    let saved: Value = serde_json::from_str(&js::stringify(&map.to_json(), None)).unwrap();
    let mut loaded = EditableMap::from_json(&saved).unwrap();
    assert!(loaded.reaction(RID).unwrap().linear.is_some());
    assert_eq!(loaded.reaction(RID).unwrap().linear, map.reaction(RID).unwrap().linear);

    let (_, end) = linear_ends(&loaded, RID);
    let copy = loaded.duplicate_metabolite_node(&end, &DuplicateOptions::default()).expect("the end is duplicated").id.clone();
    assert!(loaded.merge_metabolite_nodes(&end, &copy).is_some(), "the end is merged into its copy");
    let (from, to) = linear_ends(&loaded, RID);
    assert_eq!(to, copy);
    assert!(
        near(marker(&loaded, RID), midpoint(at(&loaded, &from), at(&loaded, &copy)), 1e-6),
        "and the reaction runs to where the copy is"
    );
}

// Swap and mirror

/// `[metaboliteEnd(s), s.b1, s.b2, reactionEnd(s)]` of every substrate branch,
/// each point as `x.toFixed(4),y.toFixed(4)`.
fn substrate_curves(map: &EditableMap) -> Vec<String> {
    branches(map, RID)
        .iter()
        .filter(|s| s.side.as_deref() == Some("substrate"))
        .map(|s| {
            [metabolite_end(s), s.b1.expect("b1"), s.b2.expect("b2"), reaction_end(s)]
                .iter()
                .map(|p| format!("{},{}", js::to_fixed(p.x, 4), js::to_fixed(p.y, 4)))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

/// `[...list].sort()`.
fn sorted(list: &[String]) -> Vec<String> {
    let mut out = list.to_vec();
    out.sort_by(|a, b| js::code_unit_compare(a, b));
    out
}

/// JS: 'two substrates trade places and the drawing stays as it was'
#[test]
fn two_substrates_trade_places_and_the_drawing_stays_as_it_was() {
    let (_com, mut map) = pfk_map(&[]);
    let (substrates, products) = map.reaction_sides(RID);
    assert_eq!(labels(&substrates), ["atp", "f6p"]);
    assert_eq!(labels(&products), ["adp", "fdp", "h"]);

    let (a, b) = (&substrates[0], &substrates[1]);
    let place_a = at(&map, &a.node_id);
    let place_b = at(&map, &b.node_id);
    let curves = substrate_curves(&map);

    assert!(map.swap_reaction_metabolites(RID, &a.node_id, &b.node_id));

    assert!(same_point(at(&map, &a.node_id), place_b));
    assert!(same_point(at(&map, &b.node_id), place_a));
    // The same two curves are still drawn; they have only changed owner.
    let after = substrate_curves(&map);
    assert_eq!(sorted(&after), sorted(&curves));
    assert_ne!(after, curves);
    assert_intact(&map, RID);
}

/// JS: 'a swap is refused across the two sides of a reaction'
#[test]
fn a_swap_is_refused_across_the_two_sides_of_a_reaction() {
    let (_com, mut map) = pfk_map(&[]);
    let (substrates, products) = map.reaction_sides(RID);
    assert!(!map.swap_reaction_metabolites(RID, &substrates[0].node_id, &products[0].node_id));
    assert!(!map.swap_reaction_metabolites(RID, &substrates[0].node_id, &substrates[0].node_id));
}

/// JS: 'a swap on a linear reaction hands the line to the metabolite now standing on it'
#[test]
fn a_swap_on_a_linear_reaction_hands_the_line_to_the_metabolite_now_standing_on_it() {
    let (_com, mut map) = pfk_map(&[]);
    map.set_reaction_linear(RID, true, None, None);
    let (substrates, _) = map.reaction_sides(RID);
    let (atp, f6p) = (&substrates[0], &substrates[1]);
    assert_eq!(linear_ends(&map, RID).0, f6p.node_id);
    assert!(map.swap_reaction_metabolites(RID, &atp.node_id, &f6p.node_id));
    let (from, to) = linear_ends(&map, RID);
    assert_eq!(from, atp.node_id);
    assert!(near(marker(&map, RID), midpoint(at(&map, &atp.node_id), at(&map, &to)), 1e-6));
}

/// JS: 'another reaction drawn to a swapped metabolite follows it'
#[test]
fn another_reaction_drawn_to_a_swapped_metabolite_follows_it() {
    let (com, mut map) = pfk_map(&[]);
    // FBA shares fdp with PFK once it is added from that node.
    let fdp = node_of(&map, "fdp").id;
    assert!(map.add_reaction(&com, "eco@FBA", &PlaceOptions { anchor_node_id: Some(fdp.clone()), ..Default::default() }).is_some());
    let other = |map: &EditableMap| branches(map, "eco@FBA").into_iter().find(|s| node_id(s) == fdp);
    assert!(other(&map).is_some(), "FBA is drawn from the same fdp node");

    let (_, products) = map.reaction_sides(RID);
    let adp = entry(&products, "adp");
    assert!(map.swap_reaction_metabolites(RID, &fdp, &adp.node_id));
    let other = other(&map).expect("FBA is still drawn to fdp");
    assert!(near(metabolite_end(&other), at(&map, &fdp), 1e-6), "the other reaction still ends on fdp");
}

/// `(along, across)` of a node, measured from the reaction's marker as it is
/// now, against the direction the backbone had at the start.
fn along_across(map: &EditableMap, dir: Point, node: &Node) -> (f64, f64) {
    let rxn = marker(map, RID);
    (
        (node.x - rxn.x) * dir.x + (node.y - rxn.y) * dir.y,
        (node.x - rxn.x) * -dir.y + (node.y - rxn.y) * dir.x,
    )
}

/// JS: 'swapping substrates and products turns the reaction end for end'
#[test]
fn swapping_substrates_and_products_turns_the_reaction_end_for_end() {
    let (_com, mut map) = pfk_map(&[]);
    let dir = axis_dir(&map, RID);
    let before: IndexMap<String, (f64, f64)> = map.nodes().values().map(|n| (n.id.clone(), along_across(&map, dir, n))).collect();

    assert!(map.flip_reaction(RID));

    assert!(near(axis_dir(&map, RID), Point::new(-dir.x, -dir.y), 1e-6), "the backbone points the other way");
    for node in map.nodes().values() {
        let was = before[&node.id];
        let (along, across) = along_across(&map, dir, node);
        let base = node.base.as_deref().unwrap_or("");
        assert!((along + was.0).abs() < 1e-6, "{base} is on the other end");
        assert!((across - was.1).abs() < 1e-6, "{base} is on the same side as before");
    }
    assert_intact(&map, RID);
}

/// JS: 'mirroring moves what hangs off one side to the other'
#[test]
fn mirroring_moves_what_hangs_off_one_side_to_the_other() {
    let (_com, mut map) = pfk_map(&[]);
    let dir = axis_dir(&map, RID);
    let before: IndexMap<String, (f64, f64)> = map.nodes().values().map(|n| (n.id.clone(), along_across(&map, dir, n))).collect();

    assert!(map.mirror_reaction(RID));

    assert!(near(axis_dir(&map, RID), dir, 1e-6), "the backbone does not move");
    for node in map.nodes().values() {
        let was = before[&node.id];
        let (along, across) = along_across(&map, dir, node);
        assert!((along - was.0).abs() < 1e-6);
        assert!((across + was.1).abs() < 1e-6, "{} changed sides", node.base.as_deref().unwrap_or(""));
    }
    assert_intact(&map, RID);
}

/// JS: 'a transporter can be mirrored but its two sides stay on their sides of the membrane'
#[test]
fn a_transporter_can_be_mirrored_but_its_two_sides_stay_on_their_sides_of_the_membrane() {
    let (com, mut map) = pfk_map(&[]);
    let rid = "eco@PIt2rpp";
    assert!(add(&mut map, &com, rid));
    let compartments: IndexMap<String, Option<String>> = map.nodes().values().map(|n| (n.id.clone(), n.compartment.clone())).collect();
    assert!(!map.flip_reaction(rid), "the sides of a transporter are the sides of a membrane");
    assert!(!map.can_linearize_reaction(rid));
    assert!(map.mirror_reaction(rid));
    for node in map.nodes().values() {
        assert_eq!(Some(&node.compartment), compartments.get(&node.id));
    }
    for seg in branches(&map, rid) {
        assert!(near(metabolite_end(&seg), at(&map, node_id(&seg)), 1e-6));
    }
}

// Inspector

/// JS: 'the inspector options are undoable edits that leave the model alone'
///
/// Without the `renderInspector` assertion (see the top of the file).
#[test]
fn the_inspector_options_are_undoable_edits_that_leave_the_model_alone() {
    let mut store = store_with_pfk();
    let equation = store.community.merged.reactions[RID].equation();
    let start = store.depth();

    assert!(store.set_map_reaction_linear(RID, true));
    let (_, products) = store.map.reaction_sides(RID);
    let (a, b) = (products[0].node_id.clone(), products[1].node_id.clone());
    assert!(store.swap_map_reaction_metabolites(RID, &a, &b));
    assert!(store.mirror_map_reaction(RID));
    assert!(store.flip_map_reaction(RID));
    assert_eq!(store.depth(), start + 4);
    assert_eq!(store.community.merged.reactions[RID].equation(), equation);

    store.undo_map();
    store.undo_map();
    store.undo_map();
    store.undo_map();
    assert!(store.map.reaction(RID).unwrap().linear.is_none());
}

// Compass alignment

const COMPASS: [(&str, [f64; 2]); 8] = [
    ("N", [0.0, -1.0]),
    ("NE", [1.0, -1.0]),
    ("E", [1.0, 0.0]),
    ("SE", [1.0, 1.0]),
    ("S", [0.0, 1.0]),
    ("SW", [-1.0, 1.0]),
    ("W", [-1.0, 0.0]),
    ("NW", [-1.0, -1.0]),
];

/// JS: 'a reaction can be laid out in each of the eight directions, substrate to product'
#[test]
fn a_reaction_can_be_laid_out_in_each_of_the_eight_directions_substrate_to_product() {
    for (name, [dx, dy]) in COMPASS {
        let (_com, mut map) = pfk_map(&[]);
        let centre = marker(&map, RID);
        assert!(map.align_reaction(RID, name, None, None), "{name} is accepted");

        let (from_id, to_id) = linear_ends(&map, RID);
        let from = map.node(&from_id).unwrap();
        let to = map.node(&to_id).unwrap();
        assert_eq!([from.base.as_deref(), to.base.as_deref()], [Some("f6p"), Some("fdp")]);
        let length = js::hypot(dx, dy);
        let way = Point::new(dx / length, dy / length);

        // The marker has not moved, and both metabolites are in line with it.
        assert!(near(marker(&map, RID), centre, 1e-6), "{name}: the marker stays where it was");
        let off = |p: Point| ((p.x - centre.x) * -way.y + (p.y - centre.y) * way.x).abs();
        let along = |p: Point| (p.x - centre.x) * way.x + (p.y - centre.y) * way.y;
        assert!(off(from.pos()) < 1e-6 && off(to.pos()) < 1e-6, "{name}: both are on the line through the marker");
        assert!(
            along(from.pos()) < 0.0 && along(to.pos()) > 0.0,
            "{name}: the substrate is behind the marker and the product ahead"
        );
        assert!(near(axis_dir(&map, RID), way, 1e-9), "{name}: the backbone points that way");
        assert_intact(&map, RID);
    }
    let (_com, mut map) = pfk_map(&[]);
    assert!(!map.align_reaction(RID, "NNE", None, None), "only the eight compass points exist");
}

/// JS: 'aligning keeps each metabolite as far from the marker as it was'
#[test]
fn aligning_keeps_each_metabolite_as_far_from_the_marker_as_it_was() {
    let (_com, mut map) = pfk_map(&[]);
    let f6p = node_of(&map, "f6p").id;
    let fdp = node_of(&map, "fdp").id;
    let rxn = marker(&map, RID);
    map.move_node(&f6p, rxn.x - 150.0, rxn.y - 90.0);
    let rxn = marker(&map, RID);
    let arms: Vec<f64> = [&f6p, &fdp].iter().map(|id| at(&map, id)).map(|n| js::hypot(n.x - rxn.x, n.y - rxn.y)).collect();

    assert!(map.align_reaction(RID, "E", None, None));
    let rxn = marker(&map, RID);
    let (p, q) = (at(&map, &f6p), at(&map, &fdp));
    assert!((js::hypot(p.x - rxn.x, p.y - rxn.y) - arms[0]).abs() < 1e-6);
    assert!((js::hypot(q.x - rxn.x, q.y - rxn.y) - arms[1]).abs() < 1e-6);
    assert!(p.x < rxn.x && q.x > rxn.x && (p.y - rxn.y).abs() < 1e-6 && (q.y - rxn.y).abs() < 1e-6);
    // And from then on the reaction is kept straight between them.
    map.move_node(&fdp, q.x + 60.0, q.y + 80.0);
    let t = map.reaction(RID).unwrap().linear.as_ref().expect("the reaction is linear").t.expect("linear.t");
    let (p, q) = (at(&map, &f6p), at(&map, &fdp));
    assert!(near(marker(&map, RID), Point::new(p.x + (q.x - p.x) * t, p.y + (q.y - p.y) * t), 1e-6));
}

/// JS: 'the two metabolites a reaction is aligned through can be chosen'
#[test]
fn the_two_metabolites_a_reaction_is_aligned_through_can_be_chosen() {
    let (_com, mut map) = pfk_map(&[]);
    let (substrates, products) = map.reaction_sides(RID);
    let atp = entry(&substrates, "atp");
    let adp = entry(&products, "adp");

    // Left to itself the line runs through the two the reaction is about.
    let own = map.reaction_line_ends(RID);
    let base = |id: &Option<String>| map.node(id.as_deref().expect("a line end")).unwrap().base.clone();
    assert_eq!([base(&own.0), base(&own.1)], [Some("f6p".to_string()), Some("fdp".to_string())]);

    // Chosen one at a time, as in the inspector: nothing moves until it is used.
    let nodes_json = |map: &EditableMap| js::stringify(&map.to_json()["nodes"], None);
    let before = nodes_json(&map);
    assert!(map.set_reaction_line_end(RID, &atp.node_id));
    assert_eq!(nodes_json(&map), before);
    assert_eq!(map.reaction_line_ends(RID).0.as_deref(), Some(atp.node_id.as_str()));
    assert_eq!(map.reaction_line_ends(RID).1, own.1, "the other end is left alone");

    // Or both at once, as from a selection on the map.
    assert!(map.align_reaction(RID, "S", Some(&atp.node_id), Some(&adp.node_id)));
    assert_eq!(linear_ends(&map, RID), (atp.node_id.clone(), adp.node_id.clone()));
    let rxn = marker(&map, RID);
    let (p, q) = (at(&map, &atp.node_id), at(&map, &adp.node_id));
    assert!((p.x - rxn.x).abs() < 1e-6 && p.y < rxn.y, "atp is straight above the marker");
    assert!((q.x - rxn.x).abs() < 1e-6 && q.y > rxn.y, "adp is straight below it");

    // A metabolite of the wrong side is not an end of that side.
    assert!(!map.set_reaction_line_end(RID, "no-such-node"));
}

/// JS: 'metabolites picked out on the map are the ones Linear and the compass use'
///
/// Without the `renderInspector` assertion (see the top of the file).
#[test]
fn metabolites_picked_out_on_the_map_are_the_ones_linear_and_the_compass_use() {
    let mut store = store_with_pfk();
    let (substrates, products) = store.map.reaction_sides(RID);
    let atp = entry(&substrates, "atp").node_id.clone();
    let h = entry(&products, "h").node_id.clone();
    assert_eq!(store.selected_line_ends(RID), (None, None));

    store.toggle_node(&atp);
    store.toggle_node(&h);
    assert_eq!(store.selected_line_ends(RID), (Some(atp.clone()), Some(h.clone())));

    assert!(store.align_map_reaction(RID, "W"));
    assert_eq!(linear_ends(&store.map, RID), (atp.clone(), h.clone()));
    let rxn = marker(&store.map, RID);
    assert!(at(&store.map, &atp).x > rxn.x && at(&store.map, &h).x < rxn.x, "substrate to product runs to the left");

    // Two picked on the same side say nothing about that side.
    store.toggle_node(&entry(&substrates, "f6p").node_id);
    assert_eq!(store.selected_line_ends(RID), (None, Some(h)));
}

// Base shape

/// JS: 'a newly placed reaction is straight, with its cofactors close beside the stem'
#[test]
fn a_newly_placed_reaction_is_straight_with_its_cofactors_close_beside_the_stem() {
    let (_com, map) = pfk_map(&[]);
    let rxn = marker(&map, RID);
    let dir = axis_dir(&map, RID);
    assert!(is(dir.x, 0.0) && is(dir.y, 1.0), "drawn down the page: {dir:?}");
    let along = |p: Point| (p.x - rxn.x) * dir.x + (p.y - rxn.y) * dir.y;
    let across = |p: Point| (p.x - rxn.x) * -dir.y + (p.y - rxn.y) * dir.x;
    // (+ 0 turns the -0 that rounding a tiny negative leaves into a plain 0.)
    let place = |base: &str| {
        let n = node_of(&map, base).pos();
        [js::round(along(n)) + 0.0, js::round(across(n)) + 0.0]
    };
    #[track_caller]
    fn assert_place(got: [f64; 2], want: [f64; 2]) {
        assert!(is(got[0], want[0]) && is(got[1], want[1]), "{got:?} is not {want:?}");
    }

    // Substrate, marker and product in one line.
    assert_place(place("f6p"), [-72.0, 0.0]);
    assert_place(place("fdp"), [72.0, 0.0]);
    // One substrate cofactor and two product ones: the first on each side stands
    // on the same hand of the stem, the second across from it. The end that
    // points up the page has them further out than the end that points down,
    // where the name of the metabolite below has to fit above it.
    assert_place(place("atp"), [-40.0, 26.0]);
    assert_place(place("adp"), [28.0, 26.0]);
    assert_place(place("h"), [28.0, -26.0]);
    assert_intact(&map, RID);
}

/// JS: 'a branch to a metabolite beside the stem is a C: along the stem, then square across to it'
#[test]
fn a_branch_to_a_metabolite_beside_the_stem_is_a_c_along_the_stem_then_square_across_to_it() {
    let (_com, map) = pfk_map(&[]);
    let rxn = marker(&map, RID);
    let dir = axis_dir(&map, RID);
    let along = |p: Point| (p.x - rxn.x) * dir.x + (p.y - rxn.y) * dir.y;
    let across = |p: Point| (p.x - rxn.x) * -dir.y + (p.y - rxn.y) * dir.x;

    for seg in branches(&map, RID) {
        let node = map.node(node_id(&seg)).unwrap();
        let base = node.base.as_deref().unwrap_or("");
        let node = node.pos();
        let end = reaction_end(&seg);
        let b1 = seg.b1.expect("b1");
        let b2 = seg.b2.expect("b2");
        let out = js::sign(along(node));
        // At the reaction it runs along the stem, away from the marker.
        assert!(across(b2).abs() < 1e-6, "{base}: the reaction-side handle is on the stem");
        assert!((along(b2) - along(end)) * out > 0.0, "{base}: it sets out away from the marker");
        if across(node).abs() < 1e-6 {
            continue; // the one the stem runs straight to
        }

        // At the metabolite it comes in from the stem's side: the handle is level
        // with the metabolite and between it and the stem, not above or below it.
        assert!((along(b1) - along(node)).abs() < 1e-6, "{base}: the metabolite-side handle is level with it");
        assert!(
            across(b1).abs() < across(node).abs() && across(b1) * across(node) > 0.0,
            "{base}: and lies between it and the stem"
        );
        // The stem is followed nearly out to the metabolite's level before the turn.
        assert!((along(node) - along(b2)) * out > 0.0, "{base}: the turn comes before its level is reached");
        assert!((along(b2) - along(end)).abs() > 0.6 * (along(node) - along(end)).abs());

        // The curve never crosses to the other hand of the stem, and never doubles
        // back along it: no S.
        let curve = |t: f64| {
            let [p0, p1, p2, p3] =
                if seg.side.as_deref() == Some("substrate") { [seg.from, b1, b2, seg.to] } else { [seg.to, b1, b2, seg.from] };
            let k = |a: f64, b: f64, c: f64, d: f64| {
                js::pow(1.0 - t, 3.0) * a + 3.0 * js::pow(1.0 - t, 2.0) * t * b + 3.0 * (1.0 - t) * js::pow(t, 2.0) * c + js::pow(t, 3.0) * d
            };
            Point::new(k(p0.x, p1.x, p2.x, p3.x), k(p0.y, p1.y, p2.y, p3.y))
        };
        let mut last = curve(0.0);
        for i in 1..=20 {
            let p = curve(f64::from(i) / 20.0);
            assert!(across(p) * across(node) >= -1e-9, "{base}: the curve stays on its own hand of the stem");
            assert!(across(p).abs() <= across(last).abs() + 1e-9, "{base}: it only ever closes in on the stem");
            assert!((along(last) - along(p)) * out >= -1e-9, "{base}: and only ever heads for the marker");
            last = p;
        }
    }
}

/// JS: 'cofactors stand the same distance out on both ends of a reaction lying on its side'
#[test]
fn cofactors_stand_the_same_distance_out_on_both_ends_of_a_reaction_lying_on_its_side() {
    let (_com, mut map) = pfk_map(&[]);
    assert!(map.align_reaction(RID, "E", None, None));
    let along = |map: &EditableMap, base: &str| {
        let rxn = marker(map, RID);
        let dir = axis_dir(map, RID);
        let n = node_of(map, base);
        js::round((n.x - rxn.x) * dir.x + (n.y - rxn.y) * dir.y)
    };
    let got = [along(&map, "atp"), along(&map, "adp"), along(&map, "h")];
    assert!(is(got[0], -34.0) && is(got[1], 34.0) && is(got[2], 34.0), "{got:?} is not [-34, 34, 34]");
    // Turned upside down, the further end is again the one that points up.
    assert!(map.align_reaction(RID, "N", None, None));
    let got = [along(&map, "atp"), along(&map, "adp")];
    assert!(is(got[0], -28.0) && is(got[1], 40.0), "{got:?} is not [-28, 40]");
}

/// JS: 'the name of a straight reaction lying on its side stands clear of its arm'
///
/// Only the map side: each alignment the test makes is accepted. Where
/// `renderMap` then puts the name is not ported (see the top of the file).
#[test]
fn the_name_of_a_straight_reaction_lying_on_its_side_stands_clear_of_its_arm() {
    let (_com, mut map) = pfk_map(&[]);

    // Lying east the product arm is there. Two cofactors are below and one above.
    assert!(map.align_reaction(RID, "E", None, None));

    // Turned end for end the two are above, so the name goes below.
    assert!(map.align_reaction(RID, "W", None, None));

    // On a diagonal the arm clears the name, and a reaction not kept straight is left as it was.
    assert!(map.align_reaction(RID, "NE", None, None));
    assert!(map.align_reaction(RID, "E", None, None));
    map.set_reaction_linear(RID, false, None, None);
}

/// `JSON.stringify(branches(map).map((seg) => [seg.nodeId, seg.from, seg.to, seg.b1, seg.b2]))`.
fn shape(map: &EditableMap) -> String {
    let point = |p: Point| json!({ "x": p.x, "y": p.y });
    let handle = |p: Option<Point>| p.map(point).unwrap_or(Value::Null);
    let list: Vec<Value> =
        branches(map, RID).iter().map(|seg| json!([node_id(seg), point(seg.from), point(seg.to), handle(seg.b1), handle(seg.b2)])).collect();
    js::stringify(&Value::Array(list), None)
}

/// JS: 'tidy puts a pulled-about reaction back in its base shape without moving its metabolites'
#[test]
fn tidy_puts_a_pulled_about_reaction_back_in_its_base_shape_without_moving_its_metabolites() {
    let (_com, mut map) = pfk_map(&[]);
    let base = shape(&map);
    let own = free_nodes(&map);

    // Cofactors dragged off, a handle pulled, a branch taken off the backbone.
    let atp = node_of(&map, "atp");
    map.move_node(&atp.id, atp.x + 70.0, atp.y - 45.0);
    let seg = branches(&map, RID)
        .into_iter()
        .find(|x| map.node(node_id(x)).unwrap().base.as_deref() == Some("fdp"))
        .expect("the branch to fdp");
    let b2 = seg.b2.expect("b2");
    map.move_branch_control(RID, node_id(&seg), Handle::B2, b2.x + 40.0, b2.y + 30.0);
    let rxn = marker(&map, RID);
    map.move_branch_control(RID, node_id(&seg), Handle::Anchor, rxn.x + 25.0, rxn.y + 25.0);
    assert_ne!(shape(&map), base);

    assert!(map.tidy_reaction(RID));
    assert_eq!(shape(&map), base);
    assert_eq!(free_nodes(&map), own);
}

/// JS: 'the inspector offers the compass, the metabolite choice and tidy'
///
/// Without the `renderInspector` assertions (see the top of the file).
/// `assert.ok(store.tidyMapReaction(RID) !== undefined)` holds for any result
/// in JavaScript (the store returns the result or `null`), so the call is
/// made and nothing is asserted on what it returns.
#[test]
fn the_inspector_offers_the_compass_the_metabolite_choice_and_tidy() {
    let mut store = store_with_pfk();

    let depth = store.depth();
    assert!(store.align_map_reaction(RID, "NE"));
    store.tidy_map_reaction(RID);
    assert_eq!(store.depth(), depth + 1, "aligning is one undo step; tidying what is already tidy is none");
    store.undo_map();
    assert!(store.map.reaction(RID).unwrap().linear.is_none());
}

// Transporter arrows

fn yeast() -> CommunityModel {
    community_of("sce", "yeast_core_mc")
}

/// JS: 'a symporter shows one ordinary arrow per metabolite, pointing the way it carries them'
///
/// Without the lane arrows `renderMap` draws (see the top of the file).
#[test]
fn a_symporter_shows_one_ordinary_arrow_per_metabolite_pointing_the_way_it_carries_them() {
    /*
     * PIt2rpp takes a phosphate and a proton from the periplasm into the cytosol.
     * The glyph is not read against the reaction's axis, which points either
     * way across a membrane and for this one would point both arrows out.
     */
    let (com, mut map) = pfk_map(&[]);
    let rid = "eco@PIt2rpp";
    assert!(add(&mut map, &com, rid));
    assert_eq!(map.reaction(rid).unwrap().transport_mode.as_deref(), Some("symport"));
    assert_eq!(com.merged.reactions[rid].equation(), "eco@h_p + eco@pi_p <=> eco@h_c + eco@pi_c");
    assert_eq!(map.reaction(rid).unwrap().boundary_id.as_deref(), Some("eco/c"));

    assert!(map.segments().iter().any(|seg| seg.kind == SegmentKind::TransportLane), "the lane geometry itself is kept");
}

/// JS: 'an antiporter shows two arrows pointing opposite ways'
///
/// Without the lane arrows `renderMap` draws (see the top of the file).
#[test]
fn an_antiporter_shows_two_arrows_pointing_opposite_ways() {
    let com = yeast();
    let mut map = EditableMap::empty_from_community(&com, &LayoutOptions::default());
    let rid = "sce@ATPtm";
    assert!(add(&mut map, &com, rid));
    assert_eq!(map.reaction(rid).unwrap().transport_mode.as_deref(), Some("antiport"));
}

/// JS: 'the arrows stay on their lanes when a transporter is carried round a corner'
///
/// Without the lane arrows and the glyph transform `renderMap` draws (see
/// the top of the file).
#[test]
fn the_arrows_stay_on_their_lanes_when_a_transporter_is_carried_round_a_corner() {
    let (com, mut map) = pfk_map(&[]);
    let rid = "eco@PIt2rpp";
    assert!(add(&mut map, &com, rid));
    let boundary_id = map.reaction(rid).unwrap().boundary_id.clone().expect("the transporter is on a boundary");
    let rect = map.boundary(&boundary_id).expect("its boundary is drawn").rect.expect("with a rectangle");
    // From wherever it was put to the middle of the left-hand side.
    assert!(map.move_reaction(rid, rect.x - 50.0, rect.y + rect.h / 2.0).is_some());
    let normal = map.reaction(rid).unwrap().normal.expect("a transporter has a normal");
    assert!((normal.x + 1.0).abs() < 1e-9, "it is on the left membrane now");
}

/// JS: "editing a compartment leaves the bends of straight reactions it did not pull out of line"
#[test]
fn editing_a_compartment_leaves_the_bends_of_straight_reactions_it_did_not_pull_out_of_line() {
    let (_, mut map) = pfk_map(&[]);
    assert!(map.set_reaction_linear(RID, true, None, None));
    let from = map.reaction(RID).unwrap().linear.as_ref().unwrap().from.clone().unwrap();
    let cell = map.cells()[0].id.clone();
    // A bend drawn by hand into one branch of the line.
    map.edit_graph(|g| {
        let seg = g.segments.iter_mut().find(|s| s.reaction_id == RID && s.node_id.as_deref() == Some(from.as_str())).unwrap();
        let b1 = seg.b1.unwrap();
        seg.b1 = Some(Point::new(b1.x + 40.0, b1.y - 25.0));
    });
    let drawn = |map: &EditableMap| branches(map, RID).iter().map(|s| (s.b1, s.b2)).collect::<Vec<_>>();
    let bend = |map: &EditableMap| branches(map, RID).into_iter().find(|s| s.node_id.as_deref() == Some(from.as_str())).unwrap().b1.unwrap();

    // Another compartment resized: nothing of the line moves, and it keeps its bend.
    let before = drawn(&map);
    let p = map.cells()[0].compartments.iter().find(|c| c.id == "p").unwrap().rect;
    assert!(map.edit_geometry_rect(Some(&cell), "p", Rect::new(p.x, p.y, p.w + 60.0, p.h)).is_some());
    assert_eq!(drawn(&map), before, "the bend survived an edit somewhere else");

    // Its own compartment moved, the whole line with it: the bend moves along, unchanged.
    let c = map.cells()[0].compartments.iter().find(|k| k.id == "c").unwrap().rect;
    let b1 = bend(&map);
    assert!(map.edit_geometry_rect(Some(&cell), "c", Rect::new(c.x + 12.0, c.y + 9.0, c.w, c.h)).is_some());
    let now = map.cells()[0].compartments.iter().find(|k| k.id == "c").unwrap().rect;
    let d = Point::new(now.x - c.x, now.y - c.y);
    assert!(d.x != 0.0 || d.y != 0.0, "the compartment moved");
    assert!(near(bend(&map), Point::new(b1.x + d.x, b1.y + d.y), 1e-6), "the bend came along with it, unchanged");
}
