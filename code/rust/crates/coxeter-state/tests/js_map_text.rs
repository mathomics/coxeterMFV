//! Port of the Store tests of `test/map-text.test.mjs`.
//!
//! "a selected annotation is drawn as selected" draws the store's map with
//! the store's multi-selection: here through `coxeter_render`
//! (`render_map_svg(map.graph(), &store.map_render_state(..))`), which hands
//! the renderer that same selection.
//!
//! Not ported here, as they test `EditableMap` or the renderer alone (no
//! Store): "free text can be created, edited, moved and removed", "text IDs
//! increment without colliding", "blank text is rejected", "free text
//! survives EditableMap JSON round trip" and "renderer emits editable map
//! text and escapes its content".

mod common;

use coxeter_map::EditableMap;
use coxeter_map::multi_selection::SelectionRect;
use coxeter_render::mapview::render_map_svg;
use coxeter_state::{RenderOptions, Store};
use serde_json::json;

use common::store;

/// `emptyMap()`: a 600 × 400 canvas with a medium and nothing in it.
fn empty_map() -> EditableMap {
    EditableMap::from_json(&json!({
        "canvas": { "width": 600, "height": 400 },
        "environment": { "id": "u", "label": "Medium", "rect": { "x": 0, "y": 0, "w": 600, "h": 400 } },
        "cells": [],
        "medium": { "nodes": [], "exchanges": [] },
        "nodes": [],
        "reactions": [],
        "segments": [],
    }))
    .expect("a valid empty map")
}

/// `new Store('.')` with `store.map = emptyMap()`.
fn store_with_empty_map() -> Store {
    let mut store = store();
    store.map = Some(empty_map());
    store
}

fn map(store: &Store) -> &EditableMap {
    store.map.as_ref().expect("a map")
}

/// "store records add/edit/delete text in map history"
#[test]
fn store_records_add_edit_delete_text_in_map_history() {
    let mut store = store_with_empty_map();

    let item = store.add_map_text("Glicólisis", 100.0, 100.0, None).expect("the text is added");
    assert!(store.can_undo_map());

    store.update_map_text(&item.id, "Glicólisis / PPP", None);
    assert_eq!(map(&store).text(&item.id).unwrap().text, "Glicólisis / PPP");

    store.remove_map_text(&item.id);
    assert!(map(&store).text(&item.id).is_none());

    store.undo_map();
    assert_eq!(map(&store).text(&item.id).unwrap().text, "Glicólisis / PPP");
}

/// "deleting a selection removes the annotations caught in it"
#[test]
fn deleting_a_selection_removes_the_annotations_caught_in_it() {
    // The report: a whole pathway was selected and deleted, and the
    // metabolite names persisted. In the core-metabolism overview those names
    // are map annotations, so a selection that did not hold annotations could
    // not take them, and Delete left them labelling reactions that had gone.
    let mut store = store_with_empty_map();
    let kept = store.add_map_text("Far away", 580.0, 380.0, None).unwrap();
    let doomed = store.add_map_text("G6P", 100.0, 100.0, None).unwrap();

    store.select_in_rect(SelectionRect::from_corners(60.0, 40.0, 300.0, 140.0), true);
    assert!(store.is_multi_selected("text", &doomed.id), "the rectangle caught it");
    assert!(!store.is_multi_selected("text", &kept.id), "and left the other alone");

    let removed = store.remove_multi_selection().expect("something was removed");

    assert_eq!(removed.texts, vec![doomed.id.clone()]);
    assert!(map(&store).text(&doomed.id).is_none());
    assert!(map(&store).text(&kept.id).is_some());

    store.undo_map();
    assert_eq!(map(&store).text(&doomed.id).unwrap().text, "G6P");
}

/// "a selected annotation is drawn as selected"
#[test]
fn a_selected_annotation_is_drawn_as_selected() {
    let mut store = store_with_empty_map();
    let item = store.add_map_text("G6P", 100.0, 100.0, None).unwrap();
    store.toggle_multi_selection("text", &item.id);

    let svg = render_map_svg(map(&store).graph(), &store.map_render_state(RenderOptions::default()));

    assert!(svg.contains("emc-map-text-selected"), "the plate behind it is drawn");
    assert!(svg.contains("emc-map-text is-multi"), "and the text itself is marked");
}
