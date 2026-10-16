//! Incremental reaction placement tests (port of `test/add-reaction.test.mjs`).
//!
//! One Rust test per JavaScript test, named after it. `assert.equal` and
//! `assert.deepEqual` on numbers are strict in JavaScript (`Object.is`: `-0`
//! is not `0`), so numbers compared that way go through [`is`] here.
//!
//! Not ported (assertions):
//!
//! // Not ported: "adding a reaction to the map does not change FBA" — the
//! //   two `new FluxProblem(com.merged).optimize()` solves and `sameFluxes`
//! //   (crate `coxeter-solver`). Kept: the map edit, and in place of the
//! //   solve a check that the merged model the LP is built from is unchanged.
//! // Not ported: "removing a reaction from the map leaves FBA unchanged" —
//! //   same: the solver part is replaced by a check that the merged model
//! //   is unchanged.
//!
//! Translation notes:
//!
//! - "an orphan metabolite can be removed safely" and "a reaction added from
//!   a selected metabolite reuses only that metabolite as its anchor" write
//!   `map.nodes.set(id, node)` directly; here that is `map.edit_graph`.
//! - `result.anchorNode?.id` is `Placed::anchor_node` (the id itself).

mod common;

use std::sync::Arc;

use common::load_model;
use coxeter_core::boundaries::reaction_boundary_keys;
use coxeter_core::community::{CommunityModel, CommunitySpec, LINK_COMPARTMENT, MediumLimit, SpeciesSpec};
use coxeter_core::js;
use coxeter_core::mapdata::{NodeKind, Point, SegmentKind};
use coxeter_map::place_reaction::is_escher_secondary_metabolite;
use coxeter_map::util::blank_node;
use coxeter_map::{EditableMap, LayoutOptions, PlaceOptions};

/// `Object.is(a, b)` for two numbers: what `assert.equal` / `deepEqual`
/// (strict) compare with.
fn is(a: f64, b: f64) -> bool {
    a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
}

const MEDIUM: [(&str, f64); 7] = [
    ("glc__D", -10.0),
    ("o2", -20.0),
    ("nh4", -1000.0),
    ("pi", -1000.0),
    ("h2o", -1000.0),
    ("h", -1000.0),
    ("co2", -1000.0),
];

fn build_community() -> CommunityModel {
    CommunityModel::new(CommunitySpec {
        species: vec![SpeciesSpec::new("eco", Arc::new(load_model("ecoli_core_mc"))).abundance(1.0)],
        medium: MEDIUM.iter().map(|(base, lower)| (base.to_string(), MediumLimit { lower: Some(*lower), upper: None })).collect(),
        ..Default::default()
    })
    .unwrap()
}

fn empty_map(com: &CommunityModel) -> EditableMap {
    EditableMap::empty_from_community(com, &LayoutOptions::default())
}

fn full_map(com: &CommunityModel) -> EditableMap {
    EditableMap::from_community(com, &LayoutOptions::default())
}

fn add(map: &mut EditableMap, com: &CommunityModel, id: &str) -> bool {
    map.add_reaction(com, id, &PlaceOptions::default()).is_some()
}

fn find_internal_reaction(com: &CommunityModel) -> Option<String> {
    for (id, rxn) in &com.merged.reactions {
        let real = rxn
            .metabolites
            .keys()
            .filter(|mid| com.merged.metabolites.get(*mid).is_some_and(|met| met.compartment != LINK_COMPARTMENT))
            .count();
        if real >= 2 && reaction_boundary_keys(com, id, None).is_empty() {
            return Some(id.clone());
        }
    }
    None
}

struct Transport {
    id: String,
    boundary_id: String,
}

fn find_transport_reaction(com: &CommunityModel) -> Option<Transport> {
    let mut fallback = None;
    for id in com.merged.reactions.keys() {
        let boundaries: Vec<String> =
            reaction_boundary_keys(com, id, None).into_iter().filter(|key| key.starts_with("eco/")).collect();
        if boundaries.len() == 1 {
            return Some(Transport { id: id.clone(), boundary_id: boundaries[0].clone() });
        }
        if !boundaries.is_empty() && fallback.is_none() {
            fallback = Some(Transport { id: id.clone(), boundary_id: boundaries[0].clone() });
        }
    }
    fallback
}

/// `[x, y, w, h]` of a rectangle, as bits (strict comparison).
type RectBits = [u64; 4];

#[derive(Debug, Clone, PartialEq)]
struct Snapshot {
    /// `(id, rect, [(compartment id, rect)])`.
    cells: Vec<(String, RectBits, Vec<(String, RectBits)>)>,
    /// `(id, x, y)`.
    nodes: Vec<(String, u64, u64)>,
    reactions: Vec<(String, u64, u64)>,
}

fn snapshot(map: &EditableMap) -> Snapshot {
    let bits = |r: &coxeter_core::mapdata::Rect| [r.x.to_bits(), r.y.to_bits(), r.w.to_bits(), r.h.to_bits()];
    Snapshot {
        cells: map
            .cells()
            .iter()
            .map(|c| (c.id.clone(), bits(&c.rect), c.compartments.iter().map(|x| (x.id.clone(), bits(&x.rect))).collect()))
            .collect(),
        nodes: map.nodes().values().map(|n| (n.id.clone(), n.x.to_bits(), n.y.to_bits())).collect(),
        reactions: map.reactions().values().map(|r| (r.id.clone(), r.x.to_bits(), r.y.to_bits())).collect(),
    }
}

/// The centre of the `c` compartment of `eco` (or of its first compartment).
fn cytosol_centre(map: &EditableMap) -> Point {
    let cell = map.cells().iter().find(|item| item.id == "eco");
    let compartment = cell
        .and_then(|c| c.compartments.iter().find(|item| item.id == "c"))
        .or_else(|| cell.and_then(|c| c.compartments.first()))
        .expect("a compartment with a rectangle");
    Point::new(compartment.rect.x + compartment.rect.w / 2.0, compartment.rect.y + compartment.rect.h / 2.0)
}

/// JS: 'an empty map can add one internal reaction'
#[test]
fn an_empty_map_can_add_one_internal_reaction() {
    let com = build_community();
    let mut map = empty_map(&com);
    let id = find_internal_reaction(&com);
    assert!(id.is_some());
    let id = id.unwrap();
    let result = map.add_reaction(&com, &id, &PlaceOptions::default());
    assert!(result.is_some());
    assert_eq!(map.reactions().len(), 1);
    assert!(!map.nodes().is_empty());
    assert!(!map.segments().is_empty());
}

/// JS: 'adding a reaction creates primary metabolites and local Escher-style secondary satellites'
#[test]
fn adding_a_reaction_creates_primary_metabolites_and_local_escher_style_secondary_satellites() {
    let com = build_community();
    let mut map = empty_map(&com);
    let id = find_internal_reaction(&com).unwrap();
    let rxn = &com.merged.reactions[&id];
    map.add_reaction(&com, &id, &PlaceOptions::default());
    let mut checked = 0;
    for mid in rxn.metabolites.keys() {
        let Some(met) = com.merged.metabolites.get(mid) else { continue };
        if met.compartment == LINK_COMPARTMENT {
            continue;
        }
        checked += 1;
        if is_escher_secondary_metabolite(&com, mid, Some(met)) {
            let satellites: Vec<_> = map
                .nodes()
                .values()
                .filter(|node| {
                    node.kind == NodeKind::Currency
                        && node.merged_id.as_deref() == Some(mid.as_str())
                        && node.reaction_id.as_deref() == Some(id.as_str())
                })
                .collect();
            assert_eq!(satellites.len(), 1, "{mid} should be local to {id}");
            assert_eq!(satellites[0].primary, Some(false));
            assert_eq!(satellites[0].visual_role.as_deref(), Some("secondary"));
            assert!(satellites[0].r < 13.0);
            assert!(!map.has_node(&format!("met:{mid}")));
        } else {
            let local: Vec<_> = map
                .nodes()
                .values()
                .filter(|node| {
                    matches!(node.kind, NodeKind::Metabolite | NodeKind::Medium)
                        && node.merged_id.as_deref() == Some(mid.as_str())
                        && node.reaction_local_for.as_deref() == Some(id.as_str())
                })
                .collect();
            assert_eq!(local.len(), 1, "{mid} should have one fresh reaction-local node");
            assert_eq!(local[0].reaction_local, Some(true));
        }
    }
    assert!(checked >= 2);
}

/// JS: 'the added reaction gets one axis and connected metabolite segments'
#[test]
fn the_added_reaction_gets_one_axis_and_connected_metabolite_segments() {
    let com = build_community();
    let mut map = empty_map(&com);
    let id = find_internal_reaction(&com).unwrap();
    map.add_reaction(&com, &id, &PlaceOptions::default());
    assert!(map.reaction(&id).unwrap().axis.is_some());
    let segs = map.segments_of(&id);
    assert!(segs.iter().any(|s| s.kind == SegmentKind::Axis));
    assert!(segs.iter().any(|s| s.node_id.as_deref().is_some_and(|n| !n.is_empty())));
}

/// JS: 'a transport reaction can be placed on the boundary it crosses'
#[test]
fn a_transport_reaction_can_be_placed_on_the_boundary_it_crosses() {
    let com = build_community();
    let mut map = empty_map(&com);
    let t = find_transport_reaction(&com);
    assert!(t.is_some());
    let t = t.unwrap();
    let b = map.boundary(&t.boundary_id).cloned();
    assert!(b.is_some());
    let b = b.unwrap();
    let opts = PlaceOptions { boundary_id: Some(t.boundary_id.clone()), ..Default::default() };
    assert!(map.add_reaction(&com, &t.id, &opts).is_some());
    let rxn = map.reaction(&t.id).unwrap();
    assert!(rxn.on_membrane);
    assert_eq!(rxn.membrane.as_deref(), Some(b.inside.as_str()));
    assert!(rxn.normal.is_some());
}

/// JS: 'a boundary reaction honours an explicit placement anchor'
#[test]
fn a_boundary_reaction_honours_an_explicit_placement_anchor() {
    let com = build_community();
    let mut map = empty_map(&com);
    let t = find_transport_reaction(&com).unwrap();
    let rect = map.boundary(&t.boundary_id).unwrap().rect.unwrap();
    let anchor = Point::new(rect.x + rect.w * 0.25, rect.y);
    let opts = PlaceOptions { boundary_id: Some(t.boundary_id.clone()), anchor: Some(anchor), ..Default::default() };
    map.add_reaction(&com, &t.id, &opts);
    let rxn = map.reaction(&t.id).unwrap();
    assert!(js::hypot(rxn.x - anchor.x, rxn.y - anchor.y) < 5.0);
}

/// JS: 'adding the same reaction twice does not duplicate it'
#[test]
fn adding_the_same_reaction_twice_does_not_duplicate_it() {
    let com = build_community();
    let mut map = empty_map(&com);
    let id = find_internal_reaction(&com).unwrap();
    assert!(map.add_reaction(&com, &id, &PlaceOptions::default()).is_some());
    let before = map.stats();
    assert!(map.add_reaction(&com, &id, &PlaceOptions::default()).is_none());
    assert_eq!(map.stats(), before);
}

/// JS: 'adding a second reaction does not move existing geometry'
#[test]
fn adding_a_second_reaction_does_not_move_existing_geometry() {
    let com = build_community();
    let full = full_map(&com);
    let mut map = full.subset_by_reactions(&[full.reaction_ids()[0].clone()]);
    let before = snapshot(&map);
    let mut id = None;
    for rid in com.merged.reactions.keys() {
        if !map.has_reaction(rid) && reaction_boundary_keys(&com, rid, None).is_empty() {
            id = Some(rid.clone());
            break;
        }
    }
    assert!(id.is_some());
    assert!(map.add_reaction(&com, &id.unwrap(), &PlaceOptions::default()).is_some());
    let now = snapshot(&map);
    let after = Snapshot {
        cells: now.cells,
        nodes: now.nodes.into_iter().filter(|n| before.nodes.iter().any(|o| o.0 == n.0)).collect(),
        reactions: now.reactions.into_iter().filter(|r| before.reactions.iter().any(|o| o.0 == r.0)).collect(),
    };
    assert_eq!(after, before);
}

/// JS: 'adding a reaction changes the map but not the biological model'
#[test]
fn adding_a_reaction_changes_the_map_but_not_the_biological_model() {
    let com = build_community();
    let mut map = empty_map(&com);
    let id = find_internal_reaction(&com).unwrap();
    let r = com.merged.reactions.len();
    let m = com.merged.metabolites.len();
    let ids: Vec<String> = com.merged.reactions.keys().cloned().collect();
    map.add_reaction(&com, &id, &PlaceOptions::default());
    assert_eq!(com.merged.reactions.len(), r);
    assert_eq!(com.merged.metabolites.len(), m);
    assert_eq!(com.merged.reactions.keys().cloned().collect::<Vec<_>>(), ids);
}

/// JS: 'adding a reaction to the map does not change FBA'
///
/// The two solves are not ported (see the top of the file); what the LP is
/// built from — the merged model — is compared instead.
#[test]
fn adding_a_reaction_to_the_map_does_not_change_fba() {
    let com = build_community();
    let before = com.merged.clone();
    let mut map = empty_map(&com);
    let id = find_internal_reaction(&com).unwrap();
    map.add_reaction(&com, &id, &PlaceOptions::default());
    assert!(com.merged == before, "the merged model changed");
    assert!(com.merged.stoichiometry() == before.stoichiometry(), "the stoichiometric matrix changed");
}

/// JS: 'adding an unknown reaction is a safe no-op'
#[test]
fn adding_an_unknown_reaction_is_a_safe_no_op() {
    let com = build_community();
    let mut map = empty_map(&com);
    let before = map.stats();
    assert!(map.add_reaction(&com, "eco@THIS_REACTION_DOES_NOT_EXIST", &PlaceOptions::default()).is_none());
    assert_eq!(map.stats(), before);
}

/// JS: 'every segment created for an added reaction references valid geometry'
#[test]
fn every_segment_created_for_an_added_reaction_references_valid_geometry() {
    let com = build_community();
    let mut map = empty_map(&com);
    let id = find_internal_reaction(&com).unwrap();
    map.add_reaction(&com, &id, &PlaceOptions::default());
    for seg in map.segments_of(&id) {
        assert!(seg.from.x.is_finite() && seg.from.y.is_finite());
        assert!(seg.to.x.is_finite() && seg.to.y.is_finite());
        if let Some(node_id) = seg.node_id.as_deref().filter(|n| !n.is_empty()) {
            assert!(map.has_node(node_id), "{node_id}");
        }
    }
}

/// JS: 'transporters can be inserted on every side of the same membrane'
#[test]
fn transporters_can_be_inserted_on_every_side_of_the_same_membrane() {
    let com = build_community();
    let t = find_transport_reaction(&com);
    assert!(t.is_some());
    let t = t.unwrap();

    let shell = empty_map(&com);
    let b = shell.boundary(&t.boundary_id);
    assert!(b.is_some_and(|b| b.rect.is_some()));
    let r = b.unwrap().rect.unwrap();
    let cases = [
        ("top", Point::new(r.x + r.w / 2.0, r.y), Point::new(0.0, -1.0)),
        ("right", Point::new(r.x + r.w, r.y + r.h / 2.0), Point::new(1.0, 0.0)),
        ("bottom", Point::new(r.x + r.w / 2.0, r.y + r.h), Point::new(0.0, 1.0)),
        ("left", Point::new(r.x, r.y + r.h / 2.0), Point::new(-1.0, 0.0)),
    ];

    for (side, anchor, normal) in cases {
        let mut map = empty_map(&com);
        let opts = PlaceOptions { boundary_id: Some(t.boundary_id.clone()), anchor: Some(anchor), ..Default::default() };
        let result = map.add_reaction(&com, &t.id, &opts);
        assert!(result.is_some(), "{side}");
        let rxn = map.reaction(&t.id).unwrap();
        assert!(js::hypot(rxn.x - anchor.x, rxn.y - anchor.y) < 1e-9, "{side} moved");
        let got = rxn.normal.unwrap_or(Point::new(f64::NAN, f64::NAN));
        assert!(rxn.normal.is_some() && is(got.x, normal.x) && is(got.y, normal.y), "{side} normal: {got:?} is not {normal:?}");
    }
}

/// JS: 'removing a transporter removes only its graphical reaction and orphaned nodes'
#[test]
fn removing_a_transporter_removes_only_its_graphical_reaction_and_orphaned_nodes() {
    let com = build_community();
    let t = find_transport_reaction(&com).unwrap();
    let mut map = empty_map(&com);
    map.add_reaction(&com, &t.id, &PlaceOptions { boundary_id: Some(t.boundary_id.clone()), ..Default::default() });
    assert!(map.has_reaction(&t.id));
    assert!(!map.segments_of(&t.id).is_empty());

    let result = map.remove_reaction(&t.id);
    assert!(result.is_some());
    assert!(!map.has_reaction(&t.id));
    assert_eq!(map.segments_of(&t.id).len(), 0);
    assert_eq!(map.reactions().len(), 0);
    assert_eq!(map.segments().len(), 0);
    assert_eq!(map.nodes().len(), 0);
    assert!(com.merged.reactions.contains_key(&t.id), "the biological reaction was removed");
}

/// JS: 'removing one reaction preserves metabolite nodes still used by another drawn reaction'
#[test]
fn removing_one_reaction_preserves_metabolite_nodes_still_used_by_another_drawn_reaction() {
    let com = build_community();
    let full = full_map(&com);
    let mut pair = None;
    let ids = full.reaction_ids();
    let node_ids = |rid: &str| -> Vec<String> {
        full.segments_of(rid).iter().filter_map(|s| s.node_id.clone()).filter(|n| !n.is_empty()).collect()
    };
    let mut i = 0;
    while i < ids.len() && pair.is_none() {
        let a_nodes: std::collections::HashSet<String> = node_ids(&ids[i]).into_iter().collect();
        for j in i + 1..ids.len() {
            let shared = node_ids(&ids[j]).into_iter().find(|id| a_nodes.contains(id));
            if let Some(shared) = shared {
                pair = Some((ids[i].clone(), ids[j].clone(), shared));
                break;
            }
        }
        i += 1;
    }
    assert!(pair.is_some(), "no reactions with a shared drawn metabolite");
    let (a, b, shared_node) = pair.unwrap();
    let mut map = full.subset_by_reactions(&[a.clone(), b.clone()]);
    assert!(map.has_node(&shared_node));
    map.remove_reaction(&a);
    assert!(map.has_reaction(&b));
    assert!(map.has_node(&shared_node), "shared node was pruned while still referenced");
}

/// JS: 'a removed transporter becomes available to add again on its membrane'
#[test]
fn a_removed_transporter_becomes_available_to_add_again_on_its_membrane() {
    let com = build_community();
    let t = find_transport_reaction(&com).unwrap();
    let mut map = full_map(&com);
    assert!(map.has_reaction(&t.id));
    map.remove_reaction(&t.id);
    assert!(!map.has_reaction(&t.id));
    assert!(reaction_boundary_keys(&com, &t.id, None).contains(&t.boundary_id));
    let opts = PlaceOptions { boundary_id: Some(t.boundary_id.clone()), ..Default::default() };
    assert!(map.add_reaction(&com, &t.id, &opts).is_some());
    assert!(map.has_reaction(&t.id));
}

/// JS: 'removing a reaction from the map leaves FBA unchanged'
///
/// The two solves are not ported (see the top of the file); what the LP is
/// built from — the merged model — is compared instead.
#[test]
fn removing_a_reaction_from_the_map_leaves_fba_unchanged() {
    let com = build_community();
    let t = find_transport_reaction(&com).unwrap();
    let before = com.merged.clone();
    let mut map = full_map(&com);
    map.remove_reaction(&t.id);
    assert!(com.merged == before, "the merged model changed");
    assert!(com.merged.stoichiometry() == before.stoichiometry(), "the stoichiometric matrix changed");
}

/// JS: 'a membrane transporter chooses the lower-crossing orientation'
#[test]
fn a_membrane_transporter_chooses_the_lower_crossing_orientation() {
    let com = build_community();
    let t = find_transport_reaction(&com);
    assert!(t.is_some());
    let t = t.unwrap();
    let mut map = empty_map(&com);
    let rect = map.boundary(&t.boundary_id).unwrap().rect.unwrap();
    let anchor = Point::new(rect.x, rect.y + rect.h / 2.0);
    let opts = PlaceOptions { boundary_id: Some(t.boundary_id.clone()), anchor: Some(anchor), ..Default::default() };
    assert!(map.add_reaction(&com, &t.id, &opts).is_some());

    let rxn = map.reaction(&t.id).unwrap();
    let model_rxn = &com.merged.reactions[&t.id];
    let half = 17.0;
    let cost = |dir: Point| {
        let a1 = Point::new(rxn.x - dir.x * half, rxn.y - dir.y * half);
        let a2 = Point::new(rxn.x + dir.x * half, rxn.y + dir.y * half);
        let mut total = 0.0;
        for (mid, &coef) in &model_rxn.metabolites {
            let Some(node) = map.node(&format!("met:{mid}")) else { continue };
            let end = if coef < 0.0 { a1 } else { a2 };
            total += js::pow(node.x - end.x, 2.0) + js::pow(node.y - end.y, 2.0);
        }
        total
    };

    let chosen = rxn.axis.unwrap().dir;
    let flipped = Point::new(-chosen.x, -chosen.y);
    assert!(cost(chosen) <= cost(flipped) + 1e-9, "transporter chose the orientation with longer/crossing connections");
}

/// JS: 'an orphan metabolite can be removed safely'
#[test]
fn an_orphan_metabolite_can_be_removed_safely() {
    let com = build_community();
    let mut map = empty_map(&com);
    let id = find_internal_reaction(&com).unwrap();
    map.add_reaction(&com, &id, &PlaceOptions::default());
    let node_id = map.segments_of(&id).iter().find_map(|s| s.node_id.clone().filter(|n| !n.is_empty()));
    assert!(node_id.is_some());
    let node_id = node_id.unwrap();
    assert!(!map.can_remove_node(&node_id), "connected node should not be removable");
    map.remove_reaction(&id);
    // The normal reaction cleanup removes orphaned nodes. Add one harmless orphan
    // copy to exercise the explicit metabolite deletion API.
    let original = com.merged.metabolites.values().next().unwrap();
    let orphan_id = "met:test-orphan";
    let mut orphan = blank_node(orphan_id, NodeKind::Metabolite);
    orphan.merged_id = Some(original.id.clone());
    orphan.local_id = Some(original.id.clone());
    orphan.base = Some(original.base.clone());
    orphan.name = Some(original.name.clone());
    orphan.compartment = Some(original.compartment.clone());
    orphan.cell_id = Some("eco".into());
    orphan.x = 10.0;
    orphan.y = 10.0;
    orphan.r = 13.0;
    orphan.primary = Some(true);
    map.edit_graph(|g| g.nodes.insert(orphan_id.to_string(), orphan));
    assert!(map.can_remove_node(orphan_id));
    assert!(map.remove_node(orphan_id).is_some());
    assert!(!map.has_node(orphan_id));
}

/// JS: 'a connected metabolite cannot be removed directly'
#[test]
fn a_connected_metabolite_cannot_be_removed_directly() {
    let com = build_community();
    let mut map = empty_map(&com);
    let id = find_internal_reaction(&com).unwrap();
    map.add_reaction(&com, &id, &PlaceOptions::default());
    let node_id = map.segments_of(&id).iter().find_map(|s| s.node_id.clone().filter(|n| !n.is_empty()));
    assert!(node_id.is_some());
    let before = map.stats();
    assert!(map.remove_node(&node_id.unwrap()).is_none());
    assert_eq!(map.stats(), before);
}

/// JS: 'an internal reaction honors an explicit eight-way placement direction'
#[test]
fn an_internal_reaction_honors_an_explicit_eight_way_placement_direction() {
    let com = build_community();
    let mut map = empty_map(&com);
    let id = find_internal_reaction(&com).unwrap();
    let anchor = cytosol_centre(&map);

    let opts = PlaceOptions { anchor: Some(anchor), placement_direction: Some(Point::new(1.0, 0.0)), ..Default::default() };
    assert!(map.add_reaction(&com, &id, &opts).is_some());

    let reaction = map.reaction(&id).unwrap();
    assert!(reaction.axis.is_some());
    let dir = reaction.axis.unwrap().dir;
    assert!(dir.x > 0.99);
    assert!(dir.y.abs() < 1e-9);
    // assert.deepEqual(reaction.placementDirection, { x: 1, y: 0 })
    let placement = reaction.placement_direction.as_ref().and_then(|v| v.as_object()).expect("placementDirection is an object");
    assert_eq!(placement.len(), 2, "{placement:?}");
    assert!(is(placement["x"].as_f64().unwrap(), 1.0), "{placement:?}");
    assert!(is(placement["y"].as_f64().unwrap(), 0.0), "{placement:?}");
}

/// JS: 'explicit eight-way placement keeps the terminal primary metabolite exactly on axis'
#[test]
fn explicit_eight_way_placement_keeps_the_terminal_primary_metabolite_exactly_on_axis() {
    let com = build_community();
    let rid = "eco@PDH";
    assert!(com.merged.reactions.contains_key(rid));
    let directions = [
        [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [-1.0, 1.0],
        [-1.0, 0.0], [-1.0, -1.0], [0.0, -1.0], [1.0, -1.0],
    ];

    for [x, y] in directions {
        let mut map = empty_map(&com);
        let anchor = cytosol_centre(&map);
        let opts = PlaceOptions { anchor: Some(anchor), placement_direction: Some(Point::new(x, y)), ..Default::default() };
        assert!(map.add_reaction(&com, rid, &opts).is_some());

        let reaction = map.reaction(rid).unwrap();
        let product = map.nodes_for_metabolite("eco@accoa_c").into_iter().find(|node| node.reaction_local_for.as_deref() == Some(rid));
        assert!(product.is_some(), "missing acetyl-CoA for direction {x},{y}");
        let product = product.unwrap();

        let dir = reaction.axis.unwrap().dir;
        let dx = product.x - reaction.x;
        let dy = product.y - reaction.y;
        let cross = dx * dir.y - dy * dir.x;
        let dot = dx * dir.x + dy * dir.y;
        assert!(cross.abs() < 1e-7, "acetyl-CoA is off-axis for direction {x},{y}: cross={cross}");
        assert!(dot > 0.0, "acetyl-CoA is not on the product side for direction {x},{y}");
    }
}

/// JS: 'a preferred terminal primary metabolite of an internal reaction is placed exactly on the selected axis'
#[test]
fn a_preferred_terminal_primary_metabolite_of_an_internal_reaction_is_placed_exactly_on_the_selected_axis() {
    let com = build_community();
    let rid = "eco@FBA";
    let preferred = "eco@g3p_c";
    let reaction = com.merged.reactions.get(rid);
    assert!(reaction.is_some(), "FBA fixture reaction is missing");
    let reaction = reaction.unwrap();
    assert_eq!(reaction_boundary_keys(&com, rid, None).len(), 0, "FBA must remain an internal reaction");
    assert!(reaction.metabolites.get(preferred).copied().unwrap_or(f64::NAN) > 0.0, "g3p_c must remain an FBA product");

    let mut map = empty_map(&com);
    let opts = PlaceOptions {
        placement_direction: Some(Point::new(1.0, 0.0)),
        terminal_metabolite_id: Some(preferred.to_string()),
        ..Default::default()
    };
    assert!(map.add_reaction(&com, rid, &opts).is_some());
    let rxn = map.reaction(rid).unwrap();
    let node = map.nodes_for_metabolite(preferred).into_iter().find(|item| item.reaction_local_for.as_deref() == Some(rid));
    assert!(node.is_some());
    let node = node.unwrap();
    assert!((node.y - rxn.y).abs() < 1e-7, "preferred terminal is not on the horizontal axis");
    assert!(node.x > rxn.x, "preferred terminal is not on the product side");
}

/// JS: 'a reaction added from a selected metabolite reuses only that metabolite as its anchor'
#[test]
fn a_reaction_added_from_a_selected_metabolite_reuses_only_that_metabolite_as_its_anchor() {
    let com = build_community();
    let full = full_map(&com);
    let mut source = None;
    let mut target_reaction = None;
    'nodes: for node in full.nodes().values() {
        if !matches!(node.kind, NodeKind::Metabolite | NodeKind::Medium) {
            continue;
        }
        for (rid, rxn) in &com.merged.reactions {
            if full.has_reaction(rid) && node.merged_id.as_deref().is_some_and(|m| rxn.metabolites.contains_key(m)) {
                source = Some(node.clone());
                target_reaction = Some(rid.clone());
                break 'nodes;
            }
        }
    }
    assert!(source.is_some() && target_reaction.is_some());
    let source = source.unwrap();
    let target_reaction = target_reaction.unwrap();
    let source_merged = source.merged_id.clone().unwrap();

    let mut map = full.subset_by_reactions(&[]);
    // Keep exactly one pre-existing visual metabolite as the Add Reaction origin.
    map.edit_graph(|g| g.nodes.insert(source.id.clone(), source.clone()));
    let anchor = Point::new(source.x + 96.0, source.y);
    let result = map.add_reaction(
        &com,
        &target_reaction,
        &PlaceOptions {
            anchor: Some(anchor),
            anchor_node_id: Some(source.id.clone()),
            placement_direction: Some(Point::new(1.0, 0.0)),
            ..Default::default()
        },
    );
    assert!(result.is_some());
    assert_eq!(result.unwrap().anchor_node.as_deref(), Some(source.id.as_str()));
    assert!(
        map.segments_of(&target_reaction).iter().any(|seg| seg.node_id.as_deref() == Some(source.id.as_str())),
        "new reaction does not start from the selected metabolite",
    );
    assert_eq!(
        map.nodes_for_metabolite(&source_merged)
            .into_iter()
            .filter(|node| node.reaction_local_for.as_deref() == Some(target_reaction.as_str()))
            .count(),
        0,
        "selected metabolite was duplicated",
    );

    let target = &com.merged.reactions[&target_reaction];
    for mid in target.metabolites.keys() {
        if *mid == source_merged {
            continue;
        }
        let Some(met) = com.merged.metabolites.get(mid) else { continue };
        if met.compartment == LINK_COMPARTMENT {
            continue;
        }
        if is_escher_secondary_metabolite(&com, mid, Some(met)) {
            assert!(
                map.nodes().values().any(|node| {
                    node.kind == NodeKind::Currency
                        && node.merged_id.as_deref() == Some(mid.as_str())
                        && node.reaction_id.as_deref() == Some(target_reaction.as_str())
                }),
                "{mid} should still be duplicated as a reaction-local secondary metabolite",
            );
        } else {
            assert!(
                map.nodes_for_metabolite(mid).into_iter().any(|node| node.reaction_local_for.as_deref() == Some(target_reaction.as_str())),
                "{mid} should still get a fresh reaction-local instance",
            );
        }
    }

    let rxn = map.reaction(&target_reaction).unwrap();
    assert!(rxn.x.is_finite() && rxn.y.is_finite());
}

/// JS: 'ACt2rpp is rendered as a two-lane symporter without crossed solute lanes'
#[test]
fn act2rpp_is_rendered_as_a_two_lane_symporter_without_crossed_solute_lanes() {
    let com = build_community();
    let rid = "eco@ACt2rpp";
    assert!(com.merged.reactions.contains_key(rid));

    let boundaries: Vec<String> = reaction_boundary_keys(&com, rid, None).into_iter().filter(|key| key.starts_with("eco/")).collect();
    assert!(!boundaries.is_empty());

    let mut map = empty_map(&com);
    let rect = map.boundary(&boundaries[0]).unwrap().rect.unwrap();
    let anchor = Point::new(rect.x, rect.y + rect.h / 2.0);
    let opts = PlaceOptions { boundary_id: Some(boundaries[0].clone()), anchor: Some(anchor), ..Default::default() };
    assert!(map.add_reaction(&com, rid, &opts).is_some());

    let rxn = map.reaction(rid).unwrap();
    assert_eq!(rxn.transport_mode.as_deref(), Some("symport"));
    let lanes = rxn.transport_lanes.as_ref().expect("transportLanes");
    assert_eq!(lanes.len(), 2);

    let bases: std::collections::HashSet<&str> = lanes.iter().map(|lane| lane.base.as_str()).collect();
    assert!(bases.contains("ac"));
    assert!(bases.contains("h"));

    let lane_segments = map.segments_of(rid).into_iter().filter(|seg| seg.kind == SegmentKind::TransportLane).count();
    assert_eq!(lane_segments, 2);

    for base in ["ac", "h"] {
        let branch_segments = map
            .segments_of(rid)
            .into_iter()
            .filter(|seg| seg.transport_base.as_deref() == Some(base) && seg.kind == SegmentKind::Primary)
            .count();
        assert_eq!(branch_segments, 2, "{base}");
    }
}

/// JS: 'GAPD duplicates NAD, NADH and H as small reaction-local secondary metabolites'
#[test]
fn gapd_duplicates_nad_nadh_and_h_as_small_reaction_local_secondary_metabolites() {
    let com = build_community();
    let rid = "eco@GAPD";
    assert!(com.merged.reactions.contains_key(rid));
    let mut map = empty_map(&com);
    assert!(add(&mut map, &com, rid));

    for base in ["nad", "nadh", "h"] {
        let nodes: Vec<_> = map
            .nodes()
            .values()
            .filter(|node| node.kind == NodeKind::Currency && node.reaction_id.as_deref() == Some(rid) && node.base.as_deref() == Some(base))
            .collect();
        assert_eq!(nodes.len(), 1, "expected one local {base} satellite");
        assert!(nodes[0].r < 13.0, "{base} should be visually secondary");
        assert_eq!(nodes[0].escher_secondary, Some(true));
    }
}

/// JS: 'Pi is treated as a small reaction-local Escher secondary metabolite'
#[test]
fn pi_is_treated_as_a_small_reaction_local_escher_secondary_metabolite() {
    let com = build_community();
    let rid = "eco@GAPD";
    assert!(com.merged.reactions.contains_key(rid));
    let mut map = empty_map(&com);
    assert!(add(&mut map, &com, rid));

    let pi: Vec<_> = map
        .nodes()
        .values()
        .filter(|node| node.kind == NodeKind::Currency && node.reaction_id.as_deref() == Some(rid) && node.base.as_deref() == Some("pi"))
        .collect();
    assert_eq!(pi.len(), 1);
    assert_eq!(pi[0].primary, Some(false));
    assert_eq!(pi[0].visual_role.as_deref(), Some("secondary"));
    assert_eq!(pi[0].escher_secondary, Some(true));
    assert!(pi[0].r < 13.0);
}

/// JS: 'secondary metabolites are duplicated per reaction rather than shared between reactions'
#[test]
fn secondary_metabolites_are_duplicated_per_reaction_rather_than_shared_between_reactions() {
    let com = build_community();
    let mut map = empty_map(&com);
    for rid in ["eco@GAPD", "eco@ACALD"] {
        if com.merged.reactions.contains_key(rid) {
            assert!(add(&mut map, &com, rid));
        }
    }

    let nad: Vec<_> = map.nodes().values().filter(|node| node.kind == NodeKind::Currency && node.base.as_deref() == Some("nad")).collect();
    assert!(nad.len() >= 2);
    let owners: std::collections::HashSet<Option<&str>> = nad.iter().map(|node| node.reaction_id.as_deref()).collect();
    assert_eq!(owners.len(), nad.len());
    assert!(nad.iter().all(|node| node.primary == Some(false) && node.r < 13.0));
}
