//! Port of `test/reaction-search.test.mjs`.
//!
//! `setup()` puts a one-species E. coli community and an empty map drawn
//! over it straight into the store, as the JS does.

mod common;

use std::sync::Arc;

use coxeter_core::community::{CommunityModel, CommunitySpec, SpeciesSpec};
use coxeter_map::editable_map::EditableMap;
use coxeter_map::layout::LayoutOptions;
use coxeter_map::place_reaction::PlaceOptions;
use coxeter_state::Store;

use common::{bundled_model, store};

/// `searchReactions(query)`'s default limit.
const LIMIT: usize = 30;

/// `setup()`.
fn setup() -> (Store, Arc<CommunityModel>) {
    let com = CommunityModel::new(CommunitySpec {
        species: vec![SpeciesSpec::new("eco", Arc::new(bundled_model("ecoli_core_mc"))).abundance(1.0)],
        ..CommunitySpec::default()
    })
    .expect("a valid community");
    let com = Arc::new(com);
    let mut store = store();
    store.community = Some(com.clone());
    store.map = Some(EditableMap::empty_from_community(&com, &LayoutOptions::default()));
    (store, com)
}

#[test]
fn reaction_search_finds_an_exact_reaction_id_first() {
    let (store, _) = setup();
    let results = store.search_reactions("TALA", LIMIT);
    assert!(!results.is_empty());
    assert_eq!(results[0].local_id, "TALA");
    assert_eq!(results[0].reaction_id, "eco@TALA");
    assert_eq!(results[0].score, 0);
}

#[test]
fn reaction_search_is_case_insensitive_and_searches_reaction_names() {
    let (store, _) = setup();
    let results = store.search_reactions("transaldolase", LIMIT);
    assert!(results.iter().any(|r| r.reaction_id == "eco@TALA"));
}

#[test]
fn reaction_search_searches_subsystem_text() {
    let (store, _) = setup();
    let results = store.search_reactions("pentose phosphate", LIMIT);
    assert!(results.iter().any(|r| r.reaction_id == "eco@TALA"));
}

#[test]
fn reaction_search_reports_whether_a_result_is_already_drawn() {
    let (mut store, com) = setup();
    let tala = store.search_reactions("TALA", LIMIT)[0].clone();
    assert!(!tala.drawn);

    assert!(store.map.as_mut().unwrap().add_reaction(&com, "eco@TALA", &PlaceOptions::default()).is_some());
    let tala = store.search_reactions("TALA", LIMIT)[0].clone();
    assert!(tala.drawn);
}

#[test]
fn reaction_search_respects_its_result_limit() {
    let (store, _) = setup();
    let results = store.search_reactions("a", 5);
    assert!(results.len() <= 5);
}

#[test]
fn empty_search_does_not_enumerate_the_full_model() {
    let (store, _) = setup();
    assert!(store.search_reactions("", LIMIT).is_empty());
    assert!(store.search_reactions("   ", LIMIT).is_empty());
}
