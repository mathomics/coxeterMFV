//! The Reactions & bounds explorer on a small community (port of
//! `test/reactions-explorer.test.mjs`): one test per feature.

mod common;

use coxeter_core::model::Model;
use coxeter_render::render_map_svg;
use coxeter_render::state::RenderState;
use coxeter_state::reactions::{BulkAction, ReactionTablePatch};
use coxeter_state::reactions_explorer::{Grouping, ReactionFilter, ReactionType, RowStatus, SortKey, TableItem, classify_reaction};
use coxeter_state::{Bounds, ObjectiveSense, Store, StatusState, UserNumber};
use serde_json::json;

/// One cell: two nutrients in, a transport, an internal step, biomass and a demand.
fn make_store() -> Store {
    let mut store = common::store();
    let model = Model::from_value(&json!({
        "id": "explorer-demo",
        "compartments": { "c": "Cytosol", "e": "Extracellular" },
        "metabolites": [
            { "id": "a_e", "compartment": "e" }, { "id": "a_c", "compartment": "c" },
            { "id": "z_e", "compartment": "e" }, { "id": "b_c", "compartment": "c" },
        ],
        "reactions": [
            { "id": "EX_a_e", "metabolites": { "a_e": -1 }, "lower_bound": -10, "upper_bound": 1000 },
            { "id": "EX_z_e", "metabolites": { "z_e": -1 }, "lower_bound": -5, "upper_bound": 1000 },
            { "id": "At", "metabolites": { "a_e": -1, "a_c": 1 }, "lower_bound": 0, "upper_bound": 1000 },
            { "id": "Zt", "metabolites": { "z_e": -1, "b_c": 1 }, "lower_bound": 0, "upper_bound": 1000 },
            { "id": "R1", "name": "a to b", "metabolites": { "a_c": -1, "b_c": 1 }, "lower_bound": 0, "upper_bound": 1000, "gene_reaction_rule": "g1" },
            { "id": "BIOMASS", "metabolites": { "b_c": -1 }, "lower_bound": 0, "upper_bound": 1000, "objective_coefficient": 1 },
            { "id": "DM_b_c", "metabolites": { "b_c": -1 }, "lower_bound": 0, "upper_bound": 1000 },
        ],
    }))
    .unwrap();
    store.set_model("explorer-demo", model);
    store.scenarios = vec![json!({
        "id": "explorer-demo", "name": "Explorer demo", "objective": "equal-growth",
        "species": [{ "id": "demo", "model": "explorer-demo", "abundance": 1 }],
        "medium": { "a": 10, "z": 5 },
    })];
    store.use_scenario("explorer-demo").unwrap();
    assert!(store.solution.as_ref().unwrap().is_optimal());
    store
}

fn bounds(lower: f64, upper: f64) -> Bounds {
    Bounds { lower, upper }
}

fn objective_value(store: &Store) -> f64 {
    store.solution.as_ref().unwrap().objective_value
}

#[test]
fn reactions_are_classified_from_how_the_community_was_built() {
    let store = make_store();
    let community = store.community.clone().unwrap();
    let kind = |id: &str| classify_reaction(&community, id);
    assert_eq!(kind("EX_a_s"), ReactionType::Exchange);
    assert_eq!(kind("demo@EX_a_e"), ReactionType::Uptake);
    assert_eq!(kind("demo@At"), ReactionType::Transport);
    assert_eq!(kind("demo@R1"), ReactionType::Internal);
    assert_eq!(kind("demo@BIOMASS"), ReactionType::Biomass);
    assert_eq!(kind("COMMUNITY_GROWTH"), ReactionType::Biomass);
    assert_eq!(kind("demo@DM_b_c"), ReactionType::Demand);
}

#[test]
fn exchange_bounds_and_the_medium_card_edit_the_same_medium() {
    let mut store = make_store();
    store.set_reaction_bounds("EX_a_s", -4.0, UserNumber::Empty).unwrap();
    assert_eq!(store.medium["a"], 4.0, "the uptake bound is the medium supply");
    assert!(!store.overrides.contains_key("EX_a_s"));
    store.set_medium_limit("a", 6.0).unwrap();
    assert_eq!(store.current_reaction_bounds("EX_a_s"), bounds(-6.0, 1000.0));
    store.set_reaction_bounds("EX_a_s", UserNumber::Empty, 50.0).unwrap();
    assert_eq!(store.overrides.get("EX_a_s"), Some(&bounds(-6.0, 50.0)), "a secretion cap stays an override");
    assert_eq!(store.medium["a"], 6.0);
    assert_eq!(store.original_reaction_bounds("EX_a_s"), bounds(-10.0, 1000.0));
    store.reset_reaction("EX_a_s").unwrap();
    assert_eq!(store.medium["a"], 10.0);
    assert!(!store.overrides.contains_key("EX_a_s"));
    assert!(store.constraint_changes().is_empty());
}

#[test]
fn constraint_undo_and_redo_cover_bounds_medium_and_objective_and_leave_the_map_history_alone() {
    let mut store = make_store();
    store.set_reaction_bounds("demo@R1", UserNumber::Empty, 5.0).unwrap();
    // The same slider drag: one step.
    store.set_reaction_bounds("demo@R1", UserNumber::Empty, 3.0).unwrap();
    store.knockout("demo@At").unwrap();
    store.set_medium_limit("z", 1.0).unwrap();
    store.set_objective_coefficient("demo@DM_b_c", 2.0).unwrap();
    assert_eq!(store.constraint_history.undo_len(), 4);
    assert!(!store.map_history.can_undo());

    assert!(store.undo_constraints().unwrap().is_some());
    assert_eq!(store.custom_objective, None, "the objective is back to the preset");
    store.undo_constraints().unwrap();
    assert_eq!(store.medium["z"], 5.0);
    store.undo_constraints().unwrap();
    assert!(!store.reaction_knocked_out("demo@At"));
    store.undo_constraints().unwrap();
    assert_eq!(store.current_reaction_bounds("demo@R1").upper, 1000.0, "the whole drag is undone at once");
    assert!(!store.can_undo_constraints());

    store.redo_constraints().unwrap();
    assert_eq!(store.current_reaction_bounds("demo@R1").upper, 3.0);
    store.redo_constraints().unwrap();
    assert!(store.reaction_knocked_out("demo@At"));
    assert!(store.solution.as_ref().unwrap().is_optimal());
}

#[test]
fn an_edited_objective_is_maximised_or_minimised_and_round_trips_in_the_scenario_file() {
    let mut store = make_store();
    store.set_objective_coefficient("COMMUNITY_GROWTH", 0.0).unwrap();
    store.set_objective_coefficient("demo@DM_b_c", 1.0).unwrap();
    assert_eq!(store.objective_coefficients().into_iter().collect::<Vec<_>>(), vec![("demo@DM_b_c".to_string(), 1.0)]);
    assert!((objective_value(&store) - 15.0).abs() < 1e-6, "both nutrients end in the demand");
    store.set_objective_coefficient("demo@DM_b_c", -1.0).unwrap();
    store.set_objective_sense(ObjectiveSense::Min).unwrap();
    assert!((objective_value(&store) + 15.0).abs() < 1e-6, "a minimum is reported with its own sign, got {}", objective_value(&store));

    let file = store.state_json();
    let doc: serde_json::Value = serde_json::from_str(&file).unwrap();
    assert_eq!(doc["customObjective"], json!({ "sense": "min", "coefficients": { "demo@DM_b_c": -1 } }));
    let mut other = make_store();
    other.load_state(&file).unwrap();
    assert_eq!(other.objective_sense(), ObjectiveSense::Min);
    assert!((objective_value(&other) + 15.0).abs() < 1e-6);
    other.reset_objective().unwrap();
    assert_eq!(other.objective_coefficients().into_iter().collect::<Vec<_>>(), vec![("COMMUNITY_GROWTH".to_string(), 1.0)]);
    let bundle = store.reproducibility_bundle().unwrap();
    assert_eq!(bundle["community"]["customObjective"]["sense"], "min", "the bundle carries the edited objective");
}

#[test]
fn a_change_that_leaves_the_lp_infeasible_flags_its_reaction_until_it_is_reverted() {
    let mut store = make_store();
    // More than the 10 of a on offer.
    store.set_reaction_bounds("demo@R1", 20.0, UserNumber::Empty).unwrap();
    store.solve();
    assert!(!store.solution.as_ref().unwrap().is_optimal());
    let culprits = store.constraint_culprits.clone().expect("flagged");
    assert_eq!(culprits.ids.iter().cloned().collect::<Vec<_>>(), ["demo@R1"]);
    let row = store.reaction_table_rows().into_iter().find(|r| r.id() == "demo@R1").unwrap();
    assert_eq!(row.status, RowStatus::Culprit);
    store.undo_constraints().unwrap();
    assert!(store.solution.as_ref().unwrap().is_optimal());
    assert!(store.constraint_culprits.is_none());
}

#[test]
fn bulk_actions_act_on_the_ticked_rows_as_one_undoable_edit() {
    let mut store = make_store();
    store.select_reaction_rows(&["demo@R1", "demo@At"], true);
    store.bulk_reaction_action(BulkAction::Knockout, None).unwrap();
    assert!(store.reaction_knocked_out("demo@R1") && store.reaction_knocked_out("demo@At"));
    store.bulk_reaction_action(BulkAction::Reset, None).unwrap();
    assert!(store.constraint_changes().is_empty());

    store.bulk_reaction_action(BulkAction::Scale(0.5.into()), Some(vec!["EX_a_s".into()])).unwrap();
    assert_eq!(store.medium["a"], 5.0, "the finite uptake is scaled, the 1000 left as it is");
    assert_eq!(store.current_reaction_bounds("EX_a_s").upper, 1000.0);
    // Ticking the species' own uptake row keeps its nutrient open as well.
    store.bulk_reaction_action(BulkAction::CloseUptakesExcept, Some(vec!["demo@EX_z_e".into()])).unwrap();
    assert_eq!(store.medium["a"], 0.0);
    assert_eq!(store.medium["z"], 5.0);
    store.undo_constraints().unwrap();
    assert_eq!(store.medium["a"], 5.0);
}

#[test]
fn bounds_csv_export_and_import_with_unknown_ids_reported() {
    let mut store = make_store();
    let csv = store.bounds_csv(None);
    assert!(csv.starts_with("id,lower,upper,species,original_id,name\n"));
    assert!(csv.contains("\ndemo@R1,0,1000,demo,R1,a to b\n"));
    let text = ["id,lower,upper,species,original_id", "demo@R1,0,7,,", ",,9,demo,At", "Zt,,4,,", "nope,0,1,,"].join("\n");
    let report = store.import_bounds_csv(&text, "test.csv").unwrap();
    assert_eq!(report.applied, 3);
    assert_eq!(report.unknown, ["nope"]);
    assert_eq!(store.current_reaction_bounds("demo@R1").upper, 7.0);
    assert_eq!(store.current_reaction_bounds("demo@At").upper, 9.0);
    assert_eq!(store.current_reaction_bounds("demo@Zt").upper, 4.0, "an original id only one species has");
    assert_eq!(store.status().state, StatusState::Warn);
    store.undo_constraints().unwrap();
    assert!(store.constraint_changes().is_empty(), "the whole import is one step");
}

#[test]
fn the_table_filters_groups_and_sorts_its_rows() {
    let mut store = make_store();
    store.set_reaction_bounds("demo@R1", UserNumber::Empty, 3.0).unwrap();
    store.set_reaction_table(ReactionTablePatch { toggle_filter: Some(ReactionFilter::Modified), ..Default::default() });
    let model = store.reaction_table_model();
    assert_eq!(model.visible.iter().map(|r| r.id()).collect::<Vec<_>>(), ["demo@R1"]);
    match &model.items[..] {
        [TableItem::Group(group), TableItem::Row(_)] => assert_eq!(group.label, "Internal"),
        other => panic!("one group and its row, not {} items", other.len()),
    }

    store.set_reaction_table(ReactionTablePatch { toggle_filter: Some(ReactionFilter::Modified), query: Some("g1".into()), ..Default::default() });
    assert_eq!(store.reaction_table_model().visible.iter().map(|r| r.id().to_string()).collect::<Vec<_>>(), ["demo@R1"], "genes are searched");
    store.set_reaction_table(ReactionTablePatch {
        query: Some(String::new()),
        group_by: Some(Grouping::None),
        sort_by: Some(SortKey::Upper),
        ..Default::default()
    });
    let model = store.reaction_table_model();
    assert_eq!(model.items.len(), model.total);
    assert_eq!(model.visible.last().unwrap().id(), "demo@R1", "descending by upper bound puts the capped one last");
}

#[test]
fn the_map_badges_edited_bounds_and_draws_knockouts_disabled_but_not_in_exports() {
    let mut store = make_store();
    store.knockout("demo@At").unwrap();
    store.set_reaction_bounds("demo@R1", UserNumber::Empty, 3.0).unwrap();
    store.solve();
    let state = store.map_render_state(Default::default());
    assert!(state.disabled_reaction_ids.contains("demo@At"));
    assert!(state.modified_reaction_ids.contains("demo@R1") && !state.modified_reaction_ids.contains("demo@At"));
    let map = store.map.as_ref().unwrap().graph();
    let svg = render_map_svg(map, &state);
    assert!(svg.contains("data-disabled-reaction=\"demo@At\""));
    assert!(svg.contains("data-modified-reaction=\"demo@R1\""));
    assert!(!render_map_svg(map, &RenderState { for_export: true, ..state }).contains("emc-modified-badge"));
}
