//! Port of `test/rounded-transport.test.mjs`: transporters on the rounded
//! corners of a compartment.
//!
//! Both JavaScript tests are ported; nothing is skipped.
//!
//! "a mitochondrial transporter stays on the rendered arc when placed, dragged
//! and restored" compares the full `toJSON()` after a reload,
//! `terminalMetaboliteId: null` included. `..._null_as_absent` (an extra
//! test, not in the JavaScript file) runs the same body reading `null` and an
//! absent key as one thing, the rule of the parity fixtures.

mod common;

use std::f64::consts::FRAC_1_SQRT_2;
use std::sync::Arc;

use coxeter_core::community::{CommunityModel, CommunitySpec, SpeciesSpec};
use coxeter_core::geometry::{closest_perimeter_point, compartment_corner_radius};
use coxeter_core::js;
use coxeter_core::mapdata::{MapReaction, Node, Point, Rect};
use coxeter_core::testkit::json_diff;
use coxeter_map::{EditableMap, LayoutOptions, PlaceOptions};
use indexmap::IndexMap;

use common::{load_model, map_diff};

/// The JS `boundaryDistance(rect, point, radius)`: how far `point` is from
/// the rounded outline of `rect`, negative inside.
fn boundary_distance(rect: &Rect, point: Point, radius: f64) -> f64 {
    let nearest_x = js::max(rect.x + radius, js::min(rect.x + rect.w - radius, point.x));
    let nearest_y = js::max(rect.y + radius, js::min(rect.y + rect.h - radius, point.y));
    js::hypot(point.x - nearest_x, point.y - nearest_y) - radius
}

/// JS: `rounded projection follows all corners and returns a radial unit normal`
#[test]
fn rounded_projection_follows_all_corners_and_returns_a_radial_unit_normal() {
    let rect = Rect::new(100.0, 200.0, 300.0, 500.0);
    let radius = 108.0;
    for x in [rect.x, rect.x + rect.w] {
        for y in [rect.y, rect.y + rect.h] {
            let point = closest_perimeter_point(&rect, Point::new(x, y), radius);
            assert!(boundary_distance(&rect, point.point(), radius).abs() < 1e-9);
            assert!((js::hypot(point.nx, point.ny) - 1.0).abs() < 1e-9);
            assert!((point.nx.abs() - FRAC_1_SQRT_2).abs() < 1e-9);
            assert!(point.x != x && point.y != y);
        }
    }
}

/// `(point - reaction) · reaction.normal`: how deep a lane point is along the
/// transporter's normal.
fn depth(reaction: &MapReaction, point: Point) -> f64 {
    let normal = reaction.normal.expect("the transporter has a normal");
    (point.x - reaction.x) * normal.x + (point.y - reaction.y) * normal.y
}

/// `JSON.stringify([...map.nodes.values()].filter(node => !node.reactionId))`.
fn free_nodes_json(map: &EditableMap) -> String {
    let nodes: Vec<&Node> =
        map.nodes().values().filter(|node| node.reaction_id.as_deref().is_none_or(str::is_empty)).collect();
    let mut value = serde_json::to_value(&nodes).unwrap();
    js::normalize_json_numbers(&mut value);
    js::stringify(&value, None)
}

/// How `assert.deepEqual(restored.toJSON(), map.toJSON())` is read.
#[derive(Clone, Copy, PartialEq)]
enum Restored {
    /// As JavaScript reads it: a key holding `null` is not an absent key.
    Identical,
    /// The rule of the parity fixtures: `null` and absent are the same.
    NullAsAbsent,
}

/// JS: `a mitochondrial transporter stays on the rendered arc when placed, dragged and restored`
#[test]
fn a_mitochondrial_transporter_stays_on_the_rendered_arc_when_placed_dragged_and_restored() {
    transporter_stays_on_the_arc(Restored::Identical);
}

/// Extra, not in the JavaScript file: the test above with `null` and absent
/// read as the same thing when the restored map is compared, so that
/// everything else it checks stays covered while it is ignored.
#[test]
fn a_mitochondrial_transporter_stays_on_the_rendered_arc_when_placed_dragged_and_restored_null_as_absent() {
    transporter_stays_on_the_arc(Restored::NullAsAbsent);
}

fn transporter_stays_on_the_arc(rule: Restored) {
    let community = CommunityModel::new(CommunitySpec {
        species: vec![SpeciesSpec::new("yeast", Arc::new(load_model("yeast_core_mc"))).abundance(1.0)],
        medium: IndexMap::new(),
        ..Default::default()
    })
    .unwrap();
    let mut map = EditableMap::empty_from_community(&community, &LayoutOptions::default());
    let cell = &map.cells()[0];
    let compartment = cell.compartments.iter().find(|compartment| compartment.id == "m").unwrap();
    let rect = compartment.rect;
    let radius = compartment_corner_radius(compartment);
    let boundary_id = format!("{}/m", cell.id);
    let reaction_id = "yeast@PYRt2m";
    assert!(
        map.add_reaction(
            &community,
            reaction_id,
            &PlaceOptions {
                boundary_id: Some(boundary_id),
                anchor: Some(Point::new(rect.x, rect.y)),
                ..Default::default()
            },
        )
        .is_some()
    );
    let reaction = map.reaction(reaction_id).unwrap();
    assert!(boundary_distance(&rect, Point::new(reaction.x, reaction.y), radius).abs() < 1e-9);
    let before_nodes = free_nodes_json(&map);
    // `reaction.transportLanes.map(...)`: an array in JavaScript (it would throw otherwise).
    let lane_depths: Vec<(f64, f64)> = reaction
        .transport_lanes
        .as_ref()
        .expect("transportLanes is an array")
        .iter()
        .map(|lane| {
            (
                depth(reaction, lane.inside_point.expect("insidePoint")),
                depth(reaction, lane.outside_point.expect("outsidePoint")),
            )
        })
        .collect();
    for x in [rect.x, rect.x + rect.w] {
        for y in [rect.y, rect.y + rect.h] {
            assert!(map.move_reaction(reaction_id, x, y).is_some());
            let reaction = map.reaction(reaction_id).unwrap();
            assert!(boundary_distance(&rect, Point::new(reaction.x, reaction.y), radius).abs() < 1e-9);
            let normal = reaction.normal.expect("the transporter has a normal");
            assert!((js::hypot(normal.x, normal.y) - 1.0).abs() < 1e-9);
            let lanes = reaction.transport_lanes.as_ref().expect("transportLanes is an array");
            assert_eq!(lanes.len(), lane_depths.len());
            for (index, lane) in lanes.iter().enumerate() {
                let inside_point = lane.inside_point.expect("insidePoint");
                let outside_point = lane.outside_point.expect("outsidePoint");
                let inside = depth(reaction, inside_point);
                let outside = depth(reaction, outside_point);
                assert!(boundary_distance(&rect, inside_point, radius) < 0.0);
                assert!(boundary_distance(&rect, outside_point, radius) > 0.0);
                assert!((inside - lane_depths[index].0).abs() < 1e-6);
                assert!((outside - lane_depths[index].1).abs() < 1e-6);
            }
        }
    }
    assert_eq!(free_nodes_json(&map), before_nodes);
    let mut restored = EditableMap::from_json(&map.to_json()).unwrap();
    let (restored_json, map_json) = (restored.to_json(), map.to_json());
    match rule {
        Restored::Identical => assert!(
            restored_json == map_json,
            "the restored map's JSON differs: {:?}",
            json_diff(&restored_json, &map_json, 0.0)
        ),
        Restored::NullAsAbsent => {
            let difference = map_diff(&restored_json, &map_json, 0.0);
            assert!(difference.is_none(), "the restored map's JSON differs: {difference:?}");
        }
    }
    assert!(restored.move_reaction(reaction_id, rect.x, rect.y).is_some());
    let reaction = restored.reaction(reaction_id).unwrap();
    assert!(boundary_distance(&rect, Point::new(reaction.x, reaction.y), radius).abs() < 1e-9);
}
