//! Manual editing of reaction geometry (port of `test/reaction-edit.test.mjs`).
//!
//! One Rust test per JavaScript test, named after it. `assert.equal` and
//! `assert.deepEqual` on numbers are strict in JavaScript (`Object.is`: `-0`
//! is not `0`), so numbers compared that way go through [`is`] here.
//!
//! Not ported (assertions):
//!
//! // Not ported: "global Edit mode exposes controls for every drawn reaction
//! //   without selecting them one by one" — the four `assert.match(svg, …)`
//! //   on `renderMap(map, {reactionEdit: true})` (crate `coxeter-render`).
//! //   Kept: the two reactions are placed on one map.
//!
//! "cubic handles and backbone anchors survive map JSON save/load" compares
//! the full `toJSON()` after a reload, `terminalMetaboliteId: null` included.
//! `..._null_as_absent` (an extra test, not in the JavaScript file) runs the
//! same body reading `null` and an absent key as one thing, the rule of the
//! parity fixtures.
//!
//! Translation notes:
//!
//! - "old quadratic controls upgrade to two cubic controls" edits a segment
//!   in place (`delete seg.b1; seg.control = {…}`); here that is
//!   `map.edit_graph`, with the legacy `control` in the segment's `extra`.
//! - A control point is `{reactionId, nodeId, handle, role, x, y}` in
//!   JavaScript; `deepEqual` on two of them is [`assert_same_control`].

mod common;

use std::collections::HashSet;
use std::sync::Arc;

use common::load_model;
use coxeter_core::boundaries::reaction_boundary_keys;
use coxeter_core::community::{CommunityModel, CommunitySpec, SpeciesSpec};
use coxeter_core::js;
use coxeter_core::mapdata::Point;
use coxeter_core::testkit::json_diff;
use coxeter_map::reaction_edit::{ControlPoint, Handle};
use coxeter_map::{EditableMap, LayoutOptions, PlaceOptions};
use serde_json::{Value, json};

/// `Object.is(a, b)` for two numbers.
fn is(a: f64, b: f64) -> bool {
    a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
}

/// `assert.deepEqual(a, b)` on two control points.
#[track_caller]
fn assert_same_control(a: &ControlPoint, b: &ControlPoint) {
    assert!(
        a.reaction_id == b.reaction_id && a.node_id == b.node_id && a.handle == b.handle && a.role == b.role && is(a.x, b.x) && is(a.y, b.y),
        "{a:?} is not {b:?}"
    );
}

fn community() -> CommunityModel {
    CommunityModel::new(CommunitySpec {
        species: vec![SpeciesSpec::new("eco", Arc::new(load_model("ecoli_core_mc"))).abundance(1.0)],
        ..Default::default()
    })
    .unwrap()
}

const TALA: &str = "eco@TALA";

fn tala_map() -> (CommunityModel, EditableMap, &'static str) {
    let com = community();
    let mut map = EditableMap::empty_from_community(&com, &LayoutOptions::default());
    let rid = TALA;
    assert!(com.merged.reactions.contains_key(rid));
    assert!(map.add_reaction(&com, rid, &PlaceOptions::default()).is_some());
    (com, map, rid)
}

/// `[n.id, n.x, n.y]` of every node no reaction owns (`!n.reactionId`).
fn free_nodes(map: &EditableMap) -> Vec<(String, u64, u64)> {
    map.nodes()
        .values()
        .filter(|n| n.reaction_id.as_deref().unwrap_or("").is_empty())
        .map(|n| (n.id.clone(), n.x.to_bits(), n.y.to_bits()))
        .collect()
}

fn find<'a>(points: &'a [ControlPoint], node_id: Option<&str>, handle: Handle) -> &'a ControlPoint {
    points
        .iter()
        .find(|p| node_id.is_none_or(|id| p.node_id == id) && p.handle == handle)
        .unwrap_or_else(|| panic!("no {} control point of {node_id:?}", handle.as_str()))
}

/// JS: 'an internal reaction marker can be moved without moving primary metabolites'
#[test]
fn an_internal_reaction_marker_can_be_moved_without_moving_primary_metabolites() {
    let (_com, mut map, rid) = tala_map();
    let before = free_nodes(&map);
    let x0 = map.reaction(rid).unwrap().x;
    let y0 = map.reaction(rid).unwrap().y;

    assert!(map.move_reaction(rid, x0 + 35.0, y0 + 25.0).is_some());
    let rxn = map.reaction(rid).unwrap();
    assert!(!is(rxn.x, x0));
    assert!(!is(rxn.y, y0));
    assert_eq!(free_nodes(&map), before);
}

/// JS: 'a transporter drag stays on its original membrane'
#[test]
fn a_transporter_drag_stays_on_its_original_membrane() {
    let com = community();
    let rid = com
        .merged
        .reactions
        .keys()
        .find(|id| reaction_boundary_keys(&com, id, None).iter().any(|k| k.starts_with("eco/")))
        .cloned();
    assert!(rid.is_some());
    let rid = rid.unwrap();

    let key = reaction_boundary_keys(&com, &rid, None).into_iter().find(|k| k.starts_with("eco/")).unwrap();
    let mut map = EditableMap::empty_from_community(&com, &LayoutOptions::default());
    let rect = map.boundary(&key).unwrap().rect.unwrap();
    map.add_reaction(
        &com,
        &rid,
        &PlaceOptions { boundary_id: Some(key.clone()), anchor: Some(Point::new(rect.x, rect.y + 20.0)), ..Default::default() },
    );

    assert!(map.move_reaction(&rid, rect.x + rect.w / 2.0, rect.y - 300.0).is_some());
    let rxn = map.reaction(&rid).unwrap();

    let on_top = (rxn.y - rect.y).abs() < 1e-9;
    let on_bottom = (rxn.y - (rect.y + rect.h)).abs() < 1e-9;
    let on_left = (rxn.x - rect.x).abs() < 1e-9;
    let on_right = (rxn.x - (rect.x + rect.w)).abs() < 1e-9;
    assert!(on_top || on_bottom || on_left || on_right);
    assert_eq!(rxn.boundary_id.as_deref(), Some(key.as_str()));
}

/// JS: 'TALA exposes three Escher-style control points per branch'
#[test]
fn tala_exposes_three_escher_style_control_points_per_branch() {
    let (_com, mut map, rid) = tala_map();
    let controls = map.reaction_control_points(rid);

    assert_eq!(controls.len(), 12);
    assert_eq!(controls.iter().filter(|p| p.handle == Handle::B1).count(), 4);
    assert_eq!(controls.iter().filter(|p| p.handle == Handle::B2).count(), 4);
    assert_eq!(controls.iter().filter(|p| p.handle == Handle::Anchor).count(), 4);
    assert_eq!(controls.iter().map(|p| p.node_id.as_str()).collect::<HashSet<_>>().len(), 4);
}

/// JS: 'TALA branches attach to four distinct reaction-backbone points'
#[test]
fn tala_branches_attach_to_four_distinct_reaction_backbone_points() {
    let (_com, mut map, rid) = tala_map();
    map.reaction_control_points(rid); // upgrades legacy/full geometry when needed

    let segments = map.segments_of(rid);
    let branches: Vec<_> =
        segments.iter().filter(|seg| seg.node_id.as_deref().is_some_and(|n| !n.is_empty()) && seg.backbone_anchor).collect();
    assert_eq!(branches.len(), 4);

    let anchors = branches.iter().map(|seg| if seg.side.as_deref() == Some("substrate") { seg.to } else { seg.from });
    let unique: HashSet<String> = anchors.map(|p| format!("{}:{}", js::to_fixed(p.x, 6), js::to_fixed(p.y, 6))).collect();
    assert_eq!(unique.len(), 4);
}

/// JS: 'moving b1 changes only the metabolite-side handle of that branch'
#[test]
fn moving_b1_changes_only_the_metabolite_side_handle_of_that_branch() {
    let (_com, mut map, rid) = tala_map();
    let before = map.reaction_control_points(rid);
    let chosen = find(&before, None, Handle::B1);
    let partner = find(&before, Some(&chosen.node_id), Handle::B2);

    map.move_branch_control(rid, &chosen.node_id, Handle::B1, chosen.x + 25.0, chosen.y - 18.0);
    let after = map.reaction_control_points(rid);
    let moved = find(&after, Some(&chosen.node_id), Handle::B1);
    let same_partner = find(&after, Some(&chosen.node_id), Handle::B2);

    assert!(is(moved.x, chosen.x + 25.0));
    assert!(is(moved.y, chosen.y - 18.0));
    assert_same_control(same_partner, partner);
}

/// JS: 'moving a metabolite carries b1 but leaves b2 attached to the reaction'
#[test]
fn moving_a_metabolite_carries_b1_but_leaves_b2_attached_to_the_reaction() {
    let (_com, mut map, rid) = tala_map();
    let controls = map.reaction_control_points(rid);
    let c1 = find(&controls, None, Handle::B1);
    let c2 = find(&controls, Some(&c1.node_id), Handle::B2);
    let node_id = c1.node_id.clone();
    let old_node = map.node(&node_id).unwrap().pos();

    map.move_node(&node_id, old_node.x + 18.0, old_node.y + 11.0);
    let node = map.node(&node_id).unwrap().pos();
    let dx = node.x - old_node.x;
    let dy = node.y - old_node.y;
    let after = map.reaction_control_points(rid);
    let b1 = find(&after, Some(&node_id), Handle::B1);
    let b2 = find(&after, Some(&node_id), Handle::B2);

    assert!((b1.x - (c1.x + dx)).abs() < 1e-9);
    assert!((b1.y - (c1.y + dy)).abs() < 1e-9);
    assert_same_control(b2, c2);
}

/// JS: 'moving the reaction carries b2/backbone but leaves b1 with the metabolite'
#[test]
fn moving_the_reaction_carries_b2_backbone_but_leaves_b1_with_the_metabolite() {
    let (_com, mut map, rid) = tala_map();
    let before = map.reaction_control_points(rid);
    let c1 = find(&before, None, Handle::B1);
    let c2 = find(&before, Some(&c1.node_id), Handle::B2);
    let old = Point::new(map.reaction(rid).unwrap().x, map.reaction(rid).unwrap().y);

    map.move_reaction(rid, old.x + 20.0, old.y + 15.0);
    let rxn = map.reaction(rid).unwrap();
    let dx = rxn.x - old.x;
    let dy = rxn.y - old.y;
    let after = map.reaction_control_points(rid);
    let b1 = find(&after, Some(&c1.node_id), Handle::B1);
    let b2 = find(&after, Some(&c1.node_id), Handle::B2);

    assert_same_control(b1, c1);
    assert!((b2.x - (c2.x + dx)).abs() < 1e-9);
    assert!((b2.y - (c2.y + dy)).abs() < 1e-9);
}

/// JS: 'old quadratic controls upgrade to two cubic controls'
#[test]
fn old_quadratic_controls_upgrade_to_two_cubic_controls() {
    let (_com, mut map, rid) = tala_map();
    let node_id = map.edit_graph(|g| {
        let seg = g
            .segments
            .iter_mut()
            .find(|x| x.reaction_id == rid && x.node_id.as_deref().is_some_and(|n| !n.is_empty()))
            .expect("a segment with a node");
        seg.b1 = None;
        seg.b2 = None;
        let control = json!({
            "x": (seg.from.x + seg.to.x) / 2.0 + 10.0,
            "y": (seg.from.y + seg.to.y) / 2.0 - 7.0,
        });
        seg.extra.insert("control".into(), control);
        seg.node_id.clone().unwrap()
    });

    let handles: Vec<ControlPoint> = map.reaction_control_points(rid).into_iter().filter(|p| p.node_id == node_id).collect();
    assert_eq!(handles.len(), 3);
    assert!(handles.iter().any(|p| p.handle == Handle::B1));
    assert!(handles.iter().any(|p| p.handle == Handle::B2));
    assert!(handles.iter().any(|p| p.handle == Handle::Anchor));
}

/// The map with one b1 handle moved, and the same map after
/// `EditableMap.fromJSON(JSON.parse(JSON.stringify(map.toJSON())))`.
fn saved_and_loaded() -> (EditableMap, EditableMap, &'static str) {
    let (_com, mut map, rid) = tala_map();
    let controls = map.reaction_control_points(rid);
    let c = find(&controls, None, Handle::B1);
    map.move_branch_control(rid, &c.node_id, Handle::B1, c.x + 11.0, c.y + 13.0);

    let text = js::stringify(&map.to_json(), None);
    let parsed: Value = serde_json::from_str(&text).unwrap();
    let back = EditableMap::from_json(&parsed).unwrap();
    (map, back, rid)
}

/// JS: 'cubic handles and backbone anchors survive map JSON save/load'
#[test]
fn cubic_handles_and_backbone_anchors_survive_map_json_save_load() {
    let (map, mut back, rid) = saved_and_loaded();
    let (a, b) = (back.to_json(), map.to_json());
    assert!(a == b, "the reloaded map differs: {:?}", json_diff(&a, &b, 0.0));
    assert_eq!(back.reaction_control_points(rid).len(), 12);
}

/// Extra, not in the JavaScript file: the test above with `null` and absent
/// read as the same thing (the rule of the parity fixtures), so that the
/// handles and anchors themselves stay covered while it is ignored.
#[test]
fn cubic_handles_and_backbone_anchors_survive_map_json_save_load_null_as_absent() {
    let (map, mut back, rid) = saved_and_loaded();
    let diff = json_diff(&common::canonical(&back.to_json()), &common::canonical(&map.to_json()), 0.0);
    assert!(diff.is_none(), "the reloaded map differs: {diff:?}");
    assert_eq!(back.reaction_control_points(rid).len(), 12);
}

/// JS: 'moving the third anchor control moves the branch attachment and carries b2'
#[test]
fn moving_the_third_anchor_control_moves_the_branch_attachment_and_carries_b2() {
    let (_com, mut map, rid) = tala_map();
    let before = map.reaction_control_points(rid);
    let anchor = find(&before, None, Handle::Anchor);
    let b1 = find(&before, Some(&anchor.node_id), Handle::B1);
    let b2 = find(&before, Some(&anchor.node_id), Handle::B2);

    let dx = 14.0;
    let dy = -9.0;
    map.move_branch_control(rid, &anchor.node_id, Handle::Anchor, anchor.x + dx, anchor.y + dy);

    let after = map.reaction_control_points(rid);
    let moved_anchor = find(&after, Some(&anchor.node_id), Handle::Anchor);
    let moved_b1 = find(&after, Some(&anchor.node_id), Handle::B1);
    let moved_b2 = find(&after, Some(&anchor.node_id), Handle::B2);

    assert!(is(moved_anchor.x, anchor.x + dx));
    assert!(is(moved_anchor.y, anchor.y + dy));
    assert_same_control(moved_b1, b1);
    assert!(is(moved_b2.x, b2.x + dx));
    assert!(is(moved_b2.y, b2.y + dy));
}

/// JS: 'global Edit mode exposes controls for every drawn reaction without selecting them one by one'
///
/// Only the map side: both reactions are placed. What `renderMap` writes for
/// them is not ported (see the top of the file).
#[test]
fn global_edit_mode_exposes_controls_for_every_drawn_reaction_without_selecting_them_one_by_one() {
    let com = community();
    let mut map = EditableMap::empty_from_community(&com, &LayoutOptions::default());
    for rid in ["eco@TALA", "eco@GAPD"] {
        assert!(map.add_reaction(&com, rid, &PlaceOptions::default()).is_some());
    }
}
