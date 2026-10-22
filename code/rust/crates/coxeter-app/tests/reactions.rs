//! The Reactions & bounds page through the real interface: open it, edit a
//! bound from its row, take the edit back, minimise the objective, send the
//! bounds out as CSV and read them back in, and reach the page from the
//! results dock's Table view. The store's half of each feature is tested in
//! `coxeter-state/tests/reactions_explorer.rs`.

mod common;

use coxeter_app::Shell;
use coxeter_app::ui_state::{ResultsTab, Workspace};
use coxeter_state::ObjectiveSense;
use egui_kittest::Harness;

type H = Harness<'static, Shell>;

const PGI: &str = "eco@PGI";

fn objective_value(h: &H) -> f64 {
    h.state().store.solution.as_ref().filter(|s| s.is_optimal()).map_or(f64::NAN, |s| s.objective_value)
}

fn pgi_upper(h: &H) -> f64 {
    h.state().store.current_reaction_bounds(PGI).upper
}

/// What the table calls a reaction (the map's label source), as its controls are named.
fn row_label(h: &mut H, rid: &str) -> String {
    let model = h.state_mut().reaction_table_model();
    model.rows.iter().find(|r| r.id() == rid).map(|r| r.entry.label.clone()).expect("a row for the reaction")
}

#[test]
fn the_reactions_page_edits_bounds_and_the_objective_with_its_own_undo_and_round_trips_a_bounds_csv() {
    let (mut h, dialogs, _dir) = common::harness_saving("reactions-page", Some("solo-ecoli"));
    let max = objective_value(&h);
    let original = pgi_upper(&h);
    assert!(max > 0.0, "the case grows");

    common::click(&mut h, "Edit Model");
    common::click(&mut h, "Reactions & bounds");
    assert_eq!(h.state().ui.workspace, Some(Workspace::Reactions));
    assert!(h.state().reactions_split(), "the top of the map stays in view");
    common::type_into(&mut h, "Search reactions", "PGI");
    assert_eq!(h.state().store.reaction_table.query, "PGI");
    let label = row_label(&mut h, PGI);
    let upper = format!("Upper bound of {label}");

    // A bound typed in its row applies and solves.
    common::type_into(&mut h, &upper, "2");
    assert_eq!(pgi_upper(&h), 2.0);
    let flux = h.state().store.solution.as_ref().unwrap().fluxes.get(PGI).copied().unwrap_or(0.0);
    assert!(objective_value(&h).is_finite() && flux <= 2.0 + 1e-9, "solved with the new bound: {flux}");
    assert_eq!(h.state().store.constraint_changes().len(), 1);

    // The page's undo is the constraints' own; the map's history is untouched.
    common::click(&mut h, "Undo constraint edit");
    assert_eq!(pgi_upper(&h), original);
    assert!((objective_value(&h) - max).abs() < 1e-6, "solved again as opened");
    assert!(!h.state().store.map_history.can_undo());

    // Minimised, growth goes to its least.
    common::choose(&mut h, "Maximise or minimise the objective", "Minimise");
    assert_eq!(h.state().store.objective_sense(), ObjectiveSense::Min);
    assert!(objective_value(&h) < max - 1e-6, "a minimum, solved: {}", objective_value(&h));
    common::click(&mut h, "Back to preset");
    assert_eq!(h.state().store.custom_objective, None);
    assert!((objective_value(&h) - max).abs() < 1e-6);

    // Bounds out as CSV and back in, as one undoable step.
    common::type_into(&mut h, &upper, "2");
    common::click(&mut h, "Export CSV");
    let (name, bytes) = common::saved(&dialogs).pop().expect("a saved file");
    assert!(name.ends_with("_bounds.csv"), "{name}");
    let csv = String::from_utf8(bytes).unwrap();
    assert!(csv.starts_with("id,lower,upper,species,original_id,name"));
    assert!(csv.lines().any(|l| l.starts_with(&format!("{PGI},")) && l.split(',').nth(2) == Some("2")), "PGI's edited bound is written");
    common::click(&mut h, "Undo constraint edit");
    assert_eq!(pgi_upper(&h), original);
    let path = dialogs.with(|d| d.saved.last().cloned()).unwrap();
    dialogs.with(|d| d.opens.push_back(vec![path]));
    common::click(&mut h, "Import CSV");
    assert_eq!(pgi_upper(&h), 2.0, "the file's bound is applied");
    common::click(&mut h, "Undo constraint edit");
    assert_eq!(pgi_upper(&h), original, "the whole import is one step");

    // The dock's Table view lists the same rows and leads back to the page.
    common::click(&mut h, "← Return to map");
    assert_eq!(h.state().ui.workspace, None);
    common::click(&mut h, "Table view");
    assert_eq!(h.state().ui.results_tab, ResultsTab::Table);
    assert!(common::shows(&h, " · filtered"), "the search carries over");
    common::click(&mut h, "Open in Reactions ↗");
    assert_eq!(h.state().ui.workspace, Some(Workspace::Reactions));
}
