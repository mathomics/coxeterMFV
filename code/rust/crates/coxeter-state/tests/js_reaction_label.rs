//! Port of `test/reaction-label.test.mjs`: where a reaction's name is
//! written (`reaction_label` of coxeter-core), the offset a drag leaves on the
//! reaction, and the store's half of moving and resetting it.
//!
//! Assertions left out of tests that are ported: the two on the HTML of
//! `renderInspector` (the Reset label button is offered only for a moved
//! name), checked in `coxeter-app/tests/inspector.rs`.

mod common;

use std::f64::consts::PI;
use std::sync::Arc;

use coxeter_core::community::{CommunityModel, CommunitySpec, SpeciesSpec};
use coxeter_core::mapdata::{Axis, Point};
use coxeter_core::metabolite_label::Anchor;
use coxeter_core::reaction_label::{
    LABEL_OFFSET, normalize_reaction_label_offset, reaction_label_offset, reaction_label_placement_in,
    set_reaction_label_offset,
};
use coxeter_map::util::blank_reaction;
use coxeter_map::{EditableMap, LayoutOptions, PlaceOptions};
use coxeter_render::LabelMode;
use coxeter_render::mapview::render_map_svg;
use coxeter_state::operation_boundary::{OperationDomain, operation_domain};
use coxeter_state::{RenderOptions, Species, Store};
use serde_json::{Value, json};

use common::{bundled_model, store};

const RID: &str = "eco@PFK";

fn community() -> Arc<CommunityModel> {
    Arc::new(
        CommunityModel::new(CommunitySpec {
            species: vec![SpeciesSpec::new("eco", Arc::new(bundled_model("ecoli_core_mc"))).abundance(1.0)],
            ..Default::default()
        })
        .unwrap(),
    )
}

/// `storeWithPFK()`: E. coli core as `eco`, an empty map, PFK placed and
/// selected. (`pfkMap()` is its map.)
fn store_with_pfk() -> Store {
    let com = community();
    let mut store = store();
    store.community = Some(com.clone());
    store.species = vec![Species::new("eco", "ecoli_core_mc", 1.0, true)];
    store.set_model("ecoli_core_mc", com.species[0].model.as_ref().clone());
    store.map = Some(EditableMap::empty_from_community(&com, &LayoutOptions::default()));
    assert!(store.add_map_reaction(RID, &PlaceOptions::default()).is_some());
    store.select(Some(RID));
    store
}

fn map(store: &Store) -> &EditableMap {
    store.map.as_ref().expect("a map")
}

fn map_mut(store: &mut Store) -> &mut EditableMap {
    store.map.as_mut().expect("a map")
}

fn near(a: f64, b: f64, eps: f64) -> bool {
    (a - b).abs() <= eps
}

/// The drawn name of a reaction: where, its anchor, and its class list.
struct Drawn {
    x: f64,
    y: f64,
    anchor: String,
    classes: Vec<String>,
}

/// `drawnName(map)`, with every label drawn.
fn drawn_name(store: &Store) -> Drawn {
    let mut state = store.map_render_state(RenderOptions::default());
    state.labels = LabelMode::All;
    state.show_currency = true;
    let svg = render_map_svg(map(store).graph(), &state);
    let text = regex::Regex::new(r"<text\b([^>]*)>[^<]*</text>").unwrap();
    let attr = regex::Regex::new(r#"([\w-]+)="([^"]*)""#).unwrap();
    let found = text
        .captures_iter(&svg)
        .map(|c| c[1].to_string())
        .find(|attrs| attrs.contains(&format!(r#"data-rxn="{RID}""#)))
        .expect("the name is drawn");
    let get = |name: &str| attr.captures_iter(&found).find(|c| &c[1] == name).map(|c| c[2].to_string()).unwrap_or_default();
    Drawn {
        x: get("x").parse().unwrap(),
        y: get("y").parse().unwrap(),
        anchor: get("text-anchor"),
        classes: get("class").split(' ').map(String::from).collect(),
    }
}

/// JS: "without an offset the name stands where it always has, in each of the three default places"
#[test]
fn without_an_offset_the_name_stands_where_it_always_has_in_each_of_the_three_default_places() {
    let mut store = store_with_pfk();
    let check = |store: &Store, placement: &str, anchor: Anchor| {
        let m = map(store);
        let place = reaction_label_placement_in(m.graph(), m.reaction(RID).unwrap());
        assert_eq!(place.placement, placement);
        assert_eq!(place.anchor, anchor);
        let drawn = drawn_name(store);
        assert!(near(drawn.x, place.x, 0.006) && near(drawn.y, place.y, 0.006), "the renderer draws it there");
        assert_eq!(drawn.anchor, anchor.as_str());
        assert!(drawn.classes.iter().any(|c| c == placement));
        place
    };

    // Beside the marker: 8 past its right edge, which for a free internal
    // reaction is 3 |cos| + 9 |sin| of its angle.
    assert!(map_mut(&mut store).set_reaction_angle(RID, PI / 2.0));
    let beside = check(&store, "emc-label-rxn-adjacent", Anchor::Start);
    let rxn = map(&store).reaction(RID).unwrap().clone();
    assert!((beside.x - (rxn.x + 9.0 + 8.0)).abs() < 1e-9 && beside.y == rxn.y);

    // Straight and on its side: over or under the bar.
    assert!(map_mut(&mut store).align_reaction(RID, "E", None, None));
    let over = check(&store, "emc-label-rxn-above", Anchor::Middle);
    let rxn = map(&store).reaction(RID).unwrap().clone();
    assert!(over.x == rxn.x && over.y < rxn.y);
    assert!(map_mut(&mut store).align_reaction(RID, "W", None, None));
    let under = check(&store, "emc-label-rxn-below", Anchor::Middle);
    let rxn = map(&store).reaction(RID).unwrap().clone();
    assert!(under.x == rxn.x && under.y > rxn.y);
    assert!(!rxn.extra.contains_key(LABEL_OFFSET), "nothing was stored");
}

/// JS: "a moved name is written from the marker plus its offset, aligned away from the marker"
#[test]
fn a_moved_name_is_written_from_the_marker_plus_its_offset_aligned_away_from_the_marker() {
    let mut rxn = blank_reaction("r");
    rxn.x = 100.0;
    rxn.y = 50.0;
    rxn.axis = Some(Axis { a1: Point::new(100.0, 40.0), a2: Point::new(100.0, 60.0), dir: Point::new(0.0, 1.0) });
    let quarter = 0.25_f64.acos();
    let cases = [
        ((30.0, 0.0), Anchor::Start, "emc-label-rxn-adjacent"),
        ((-30.0, 0.0), Anchor::End, "emc-label-rxn-adjacent"),
        ((0.0, -30.0), Anchor::Middle, "emc-label-rxn-above"),
        ((0.0, 30.0), Anchor::Middle, "emc-label-rxn-below"),
        ((20.0, -20.0), Anchor::Start, "emc-label-rxn-above"),
        ((-20.0, 20.0), Anchor::End, "emc-label-rxn-below"),
        // Within 0.3 of square across or along, the name is centred that way.
        ((quarter.cos(), quarter.sin()), Anchor::Middle, "emc-label-rxn-below"),
        ((1.0, -0.25), Anchor::Start, "emc-label-rxn-adjacent"),
        ((0.0, 0.0), Anchor::Middle, "emc-label-rxn-adjacent"),
    ];
    let graph = EditableMap::blank(None, None);
    for ((x, y), anchor, placement) in cases {
        set_reaction_label_offset(&mut rxn, Some(Point::new(x, y)));
        let place = reaction_label_placement_in(graph.graph(), &rxn);
        assert_eq!((place.x, place.y, place.anchor, place.placement), (100.0 + x, 50.0 + y, anchor, placement), "{x}, {y}");
    }
}

/// JS: "each default place read as an offset gives the same place, so a picked-up name does not jump"
#[test]
fn each_default_place_read_as_an_offset_gives_the_same_place_so_a_picked_up_name_does_not_jump() {
    let mut store = store_with_pfk();
    let lays: [&dyn Fn(&mut EditableMap) -> bool; 3] = [
        &|m| m.set_reaction_angle(RID, 0.4),
        &|m| m.align_reaction(RID, "E", None, None),
        &|m| m.align_reaction(RID, "W", None, None),
    ];
    for lay in lays {
        assert!(lay(map_mut(&mut store)));
        map_mut(&mut store).reset_reaction_label(RID);
        let m = map(&store);
        let rxn = m.reaction(RID).unwrap();
        let before = reaction_label_placement_in(m.graph(), rxn);
        let (x, y) = (rxn.x, rxn.y);
        assert!(map_mut(&mut store).set_reaction_label_offset(RID, Point::new(before.x - x, before.y - y)).is_some());
        let m = map(&store);
        let after = reaction_label_placement_in(m.graph(), m.reaction(RID).unwrap());
        assert!(near(after.x, before.x, 1e-9) && near(after.y, before.y, 1e-9));
        assert_eq!(after.anchor, before.anchor);
        assert_eq!(after.placement, before.placement);
    }
}

/// JS: "the drawn name follows its offset, and the offset follows the reaction"
#[test]
fn the_drawn_name_follows_its_offset_and_the_offset_follows_the_reaction() {
    let mut store = store_with_pfk();
    assert!(map_mut(&mut store).set_reaction_label_offset(RID, Point::new(-40.0, 25.0)).is_some());
    let rxn = map(&store).reaction(RID).unwrap().clone();
    let drawn = drawn_name(&store);
    assert!(near(drawn.x, rxn.x - 40.0, 0.006) && near(drawn.y, rxn.y + 25.0, 0.006));
    assert_eq!(drawn.anchor, "end");
    assert!(drawn.classes.iter().any(|c| c == "emc-label-rxn-below"));

    // Moved, the reaction takes its name along; straightened, it leaves it where it was put.
    map_mut(&mut store).move_reaction(RID, rxn.x + 30.0, rxn.y - 20.0);
    let rxn = map(&store).reaction(RID).unwrap().clone();
    let drawn = drawn_name(&store);
    assert!(near(drawn.x, rxn.x - 40.0, 0.006) && near(drawn.y, rxn.y + 25.0, 0.006));
    assert!(map_mut(&mut store).set_reaction_linear(RID, true, None, None));
    assert_eq!(reaction_label_offset(map(&store).reaction(RID).unwrap()), Some(Point::new(-40.0, 25.0)));
}

/// JS: "turning a reaction turns its moved name with it"
#[test]
fn turning_a_reaction_turns_its_moved_name_with_it() {
    let mut store = store_with_pfk();
    assert!(map_mut(&mut store).set_reaction_label_offset(RID, Point::new(30.0, 0.0)).is_some());
    assert!(map_mut(&mut store).rotate_reaction(RID, PI / 2.0));
    let m = map(&store);
    let rxn = m.reaction(RID).unwrap();
    let offset = reaction_label_offset(rxn).unwrap();
    assert!(near(offset.x, 0.0, 1e-9) && near(offset.y, 30.0, 1e-9), "a quarter turn clockwise on screen");
    assert_eq!(reaction_label_placement_in(m.graph(), rxn).placement, "emc-label-rxn-below");
}

/// JS: "offsets are saved and loaded, and a damaged one puts the name back in its place"
#[test]
fn offsets_are_saved_and_loaded_and_a_damaged_one_puts_the_name_back_in_its_place() {
    let mut store = store_with_pfk();
    map_mut(&mut store).set_reaction_label_offset(RID, Point::new(12.5, -7.0));
    let doc = map(&store).to_json();
    let index = doc["reactions"].as_array().unwrap().iter().position(|r| r["id"] == RID).unwrap();
    assert_eq!(doc["reactions"][index]["labelOffset"], json!({ "x": 12.5, "y": -7 }));
    let loaded = EditableMap::from_json(&doc).unwrap();
    assert_eq!(reaction_label_offset(loaded.reaction(RID).unwrap()), Some(Point::new(12.5, -7.0)));

    // (NaN and Infinity cannot be written in JSON; `normalize_reaction_label_offset` is checked with them below.)
    for bad in [Value::Null, json!(3), json!("x"), json!([]), json!({ "x": 1 }), json!({ "x": "a", "y": 2 })] {
        let mut copy = doc.clone();
        copy["reactions"][index]["labelOffset"] = bad.clone();
        let loaded = EditableMap::from_json(&copy).unwrap();
        assert!(!loaded.reaction(RID).unwrap().extra.contains_key(LABEL_OFFSET), "{bad}");
    }
    let mut extra = doc.clone();
    extra["reactions"][index]["labelOffset"] = json!({ "x": 1, "y": 2, "z": 3 });
    let loaded = EditableMap::from_json(&extra).unwrap();
    assert_eq!(loaded.reaction(RID).unwrap().extra[LABEL_OFFSET], json!({ "x": 1.0, "y": 2.0 }));

    // A subset of the map is a copy: moving the name there leaves this one alone.
    let mut subset = map(&store).subset_by_reactions(&[RID.to_string()]);
    subset.set_reaction_label_offset(RID, Point::new(0.0, 40.0));
    assert_eq!(reaction_label_offset(map(&store).reaction(RID).unwrap()), Some(Point::new(12.5, -7.0)));

    assert_eq!(normalize_reaction_label_offset(&json!({ "x": -0.5, "y": 2 })).map(|p| p.x), Some(-0.5));
    assert!(map_mut(&mut store).set_reaction_label_offset(RID, Point::new(f64::NAN, 1.0)).is_none());
    assert!(map_mut(&mut store).set_reaction_label_offset(RID, Point::new(f64::INFINITY, 1.0)).is_none());
    assert!(map_mut(&mut store).set_reaction_label_offset("missing", Point::new(1.0, 1.0)).is_none());
}

/// JS: "the store moves a name without history, records the drag once, and resets it as one step"
#[test]
fn the_store_moves_a_name_without_history_records_the_drag_once_and_resets_it_as_one_step() {
    let mut store = store_with_pfk();
    let offset = |store: &Store| reaction_label_offset(map(store).reaction(RID).unwrap());
    let start = store.map_history.undo_len();

    let drag = store.begin_map_edit("Move reaction label");
    for x in [5.0, 15.0, 25.0] {
        assert!(store.set_reaction_label_offset(RID, Point::new(x, -10.0)));
    }
    assert_eq!(store.map_history.undo_len(), start, "moving it records nothing");
    assert!(store.commit_map_edit(drag));
    assert_eq!(store.map_history.undo_len(), start + 1);

    assert_eq!(store.undo_map().as_deref(), Some("Move reaction label"));
    assert_eq!(offset(&store), None, "undo puts it back in its default place");
    assert_eq!(store.redo_map().as_deref(), Some("Move reaction label"));
    assert_eq!(offset(&store), Some(Point::new(25.0, -10.0)));

    store.select(Some(RID));
    assert!(store.reset_reaction_label(RID));
    assert_eq!(offset(&store), None);
    assert_eq!(store.map_history.undo_len(), start + 2);
    assert!(!store.reset_reaction_label(RID), "nothing to reset");
    assert_eq!(store.map_history.undo_len(), start + 2);
    store.undo_map();
    assert_eq!(offset(&store), Some(Point::new(25.0, -10.0)));

    for name in ["setReactionLabelOffset", "resetReactionLabel"] {
        assert_eq!(operation_domain(name), Some(OperationDomain::Map));
    }
}
