//! Port of `test/quiet-cofactors.test.mjs` (the store's half): a teaching
//! scenario opens on its quiet map; View ▸ Quiet cofactors switches the map's
//! style back and forth, as an undoable map edit that is saved with the map.

mod common;

use common::init_store;

#[test]
fn a_teaching_map_opens_quiet_and_view_switches_it_as_an_undoable_map_edit() {
    let mut store = init_store();
    store.use_scenario("solo-ecoli").unwrap();
    assert_eq!(store.map.as_ref().unwrap().cofactor_style(), Some("quiet"));
    assert!(!store.map_history.can_undo());

    assert_eq!(store.set_map_cofactor_style(None), None);
    assert_eq!(store.map.as_ref().unwrap().cofactor_style(), None);
    let full = store.map_json().unwrap();
    assert!(!full.contains("cofactorStyle"), "a full-size map does not mention it");

    store.undo_map();
    assert_eq!(store.map.as_ref().unwrap().cofactor_style(), Some("quiet"), "undone like any map edit");
    let saved = store.map_json().unwrap();
    store.load_map_json(&full).unwrap();
    assert_eq!(store.map.as_ref().unwrap().cofactor_style(), None);
    store.load_map_json(&saved).unwrap();
    assert_eq!(store.map.as_ref().unwrap().cofactor_style(), Some("quiet"), "and comes back with the file");
}
