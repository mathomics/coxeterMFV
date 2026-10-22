//! The inspector and the Layout tab through the real interface: every
//! `data-action` of `renderInspector` / `renderLayoutSettings` (handled by
//! `onSidebarClick` / `onSidebarInput` of `src/ui/app.js`), clicked or typed
//! as a user would, with the Store checked afterwards; and the inspector
//! assertions of the JS tests (`test/*.test.mjs`) that read what
//! `renderInspector` and `renderLayoutSettings` render:
//!
//! - ui-redesign: "metabolite inspector reports available model and map
//!   properties".
//! - ui-audit-fixes: "a shared-medium metabolite belongs to the shared
//!   medium, not to a cell called M", "colour pickers show the colour drawn
//!   and offer a reset only once customised", "a cofactor helper node
//!   explains its role instead of offering a dead Primary button", "label
//!   options, Add Metabolite help and blocked layout sliders say what they
//!   do" (the layout half), and the inspector halves of "switching a species
//!   off keeps a hand-built map…" and "a zero-abundance joint member is held
//!   out like a switched-off one".
//! - reaction-arrange: "the reaction inspector offers linear, swap, mirror
//!   and rotate as map options", "the inspector options are undoable edits
//!   that leave the model alone" (the "Kept straight between f6p and fdp"
//!   line), "metabolites picked out on the map are the ones Linear and the
//!   compass use" (the active line-end chip), "the inspector offers the
//!   compass, the metabolite choice and tidy".
//! - align-and-text: "the reaction inspector offers Vertical and
//!   Horizontal", "the text inspector offers a size and a font".
//! - label-sources: "View and both inspectors offer the available sources
//!   with an inherit option" (the inspector half).
//! - layout-settings: "layout settings panel renders the same controls shown
//!   in the canvas toolbar".
//!
//! Not ported: the inspector half of zuniga-store "the case controls expose
//! honest source constraints…" (`<fieldset disabled>`): the case needs its
//! source models downloaded.

mod common;

use std::sync::Arc;

use coxeter_app::Shell;
use coxeter_app::ui_state::RightTab;
use coxeter_core::mapdata::NodeKind;
use coxeter_core::model::Model;
use coxeter_map::{EditableMap, LayoutOptions, PlaceOptions};
use coxeter_state::StatusState;
use coxeter_state::reactions_explorer::{SIGN_LOG, sign_log_position, sign_log_value};
use egui::accesskit::{Invalid, Role, Toggled};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Map, Value, json};

type H = Harness<'static, Shell>;

const PFK: &str = "eco@PFK";
const PGI: &str = "eco@PGI";

// Set-ups

/// E. coli core alone (`ecoliStore()` of the JS tests), drawn in full.
fn ecoli() -> H {
    common::harness(Some("solo-ecoli"))
}

/// E. coli core with only PFK drawn, selected (`storeWithPFK()`).
fn pfk_alone() -> H {
    let mut h = ecoli();
    {
        let store = &mut h.state_mut().store;
        let community = store.community.clone().expect("a community");
        store.set_map(Some(EditableMap::empty_from_community(&community, &LayoutOptions::default())));
        assert!(store.add_map_reaction(PFK, &PlaceOptions::default()).is_some(), "PFK is placed");
        store.select(Some(PFK));
    }
    common::settle(&mut h);
    h
}

/// A small model with genes, annotations and a transporter (`makeStore()`
/// of ui-audit-fixes, with genes added).
fn demo_model(id: &str) -> Value {
    json!({
        "id": id,
        "compartments": {"c": "Cytosol", "e": "Extracellular"},
        "metabolites": [
            {"id": "a_e", "compartment": "e", "name": "A outside"},
            {"id": "a_c", "compartment": "c", "name": "A inside", "formula": "C6H12O6",
             "annotation": {"kegg.compound": ["C00031"]}}
        ],
        "reactions": [
            {"id": "EX_a_e", "metabolites": {"a_e": -1}, "lower_bound": -10, "upper_bound": 1000},
            {"id": "At", "name": "A transport", "metabolites": {"a_e": -1, "a_c": 1}, "lower_bound": 0, "upper_bound": 1000,
             "gene_reaction_rule": "g1 and (g2 or g3)", "annotation": {"kegg.reaction": ["R00001"]}},
            {"id": "BIOMASS", "metabolites": {"a_c": -1}, "lower_bound": 0, "upper_bound": 1000, "objective_coefficient": 1}
        ],
        "genes": [
            {"id": "g1", "name": "geneOne", "annotation": {"ncbigene": ["945"]}, "notes": {"source": "test"}},
            {"id": "g2", "name": "geneTwo"},
            {"id": "g3"}
        ]
    })
}

/// The demo model as the one species `demo`, or `demo` and `other`.
fn demo(pair: bool) -> H {
    let mut shell = common::shell(None);
    let model = Model::from_value(&demo_model("audit-demo")).expect("a valid model");
    shell.store.set_model("audit-demo", model);
    let species = if pair {
        json!([{"id": "demo", "model": "audit-demo", "abundance": 0.5}, {"id": "other", "model": "audit-demo", "abundance": 0.5}])
    } else {
        json!([{"id": "demo", "model": "audit-demo", "abundance": 1}])
    };
    shell.store.scenarios.push(json!({
        "id": "audit-demo", "name": "Audit demo", "objective": "equal-growth", "species": species, "medium": {"a": 10}
    }));
    shell.store.use_scenario("audit-demo").expect("the demo opens");
    common::harness_of(shell)
}

// Helpers

fn select_reaction(h: &mut H, rid: &str) {
    h.state_mut().store.select(Some(rid));
    common::settle(h);
}

fn select_entity(h: &mut H, kind: &str, id: &str) {
    h.state_mut().store.select_entity(kind, id, Map::new());
    common::settle(h);
}

/// Whether a text (a label, not a control) reads exactly `text`.
fn label_is(h: &H, text: &str) -> bool {
    h.query_all_by(|n| n.role() == Role::Label && n.value().as_deref() == Some(text)).next().is_some()
}

/// The labels of every control whose label starts with `prefix`.
fn control_labels(h: &H, prefix: &str) -> Vec<String> {
    h.query_all_by(|n| !matches!(n.role(), Role::Label | Role::Unknown | Role::GenericContainer) && n.label().is_some_and(|l| l.starts_with(prefix)))
        .filter_map(|n| n.accesskit_node().label())
        .collect()
}

/// The text a field shows.
fn value_of(h: &H, label: &str) -> String {
    common::control(h, label).value().unwrap_or_default()
}

/// Whether a field is marked `aria-invalid`.
fn invalid(h: &H, label: &str) -> bool {
    common::control(h, label).accesskit_node().data().invalid() == Some(Invalid::True)
}

/// Empty a field (select all, delete) without leaving it.
fn clear_field(h: &mut H, label: &str) {
    common::control(h, label).focus();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.key_press(egui::Key::Backspace);
    h.run_steps(2);
}

/// Pick an option of a select in the right panel (scrolled into view first,
/// since a select opens under the pointer).
fn choose(h: &mut H, label: &str, option: &str) {
    common::control(h, label).scroll_to_me();
    common::settle(h);
    common::choose(h, label, option);
}

/// The options a select offers (opened, read, closed).
fn options_of(h: &mut H, label: &str) -> Vec<String> {
    common::control(h, label).scroll_to_me();
    common::settle(h);
    common::control(h, label).click();
    let _ = h.try_run();
    let options: Vec<String> = h
        .query_all_by(|n| n.role() == Role::Button && n.label().is_some())
        .filter_map(|n| n.accesskit_node().label())
        .collect();
    // A second press closes it (Escape would also reach the map and clear
    // the selection).
    common::control(h, label).click();
    common::settle(h);
    assert!(!egui::Popup::is_any_open(&h.ctx));
    options
}

fn undo_depth(h: &H) -> usize {
    h.state().store.map_history.undo_len()
}

fn node_pos(h: &H, id: &str) -> (f64, f64) {
    let n = h.state().store.map.as_ref().unwrap().node(id).unwrap_or_else(|| panic!("no node {id}"));
    (n.x, n.y)
}

fn marker(h: &H, rid: &str) -> (f64, f64) {
    let r = h.state().store.map.as_ref().unwrap().reaction(rid).unwrap();
    (r.x, r.y)
}

/// The node drawn for a base metabolite label on one side of a reaction.
fn side_node(h: &H, rid: &str, label: &str) -> String {
    let (subs, prods) = h.state().store.map.as_ref().unwrap().reaction_sides(rid);
    subs.iter().chain(prods.iter()).find(|e| e.label == label).map(|e| e.node_id.clone()).unwrap_or_else(|| panic!("no {label} on {rid}"))
}

/// The first node of a kind with this merged id.
fn node_of(h: &H, merged: &str, kind: NodeKind) -> String {
    h.state()
        .store
        .map
        .as_ref()
        .unwrap()
        .nodes()
        .values()
        .find(|n| n.merged_id.as_deref() == Some(merged) && n.kind == kind)
        .map(|n| n.id.clone())
        .unwrap_or_else(|| panic!("no {merged} node"))
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

// Empty and the close

#[test]
fn nothing_selected_reads_no_selection_and_a_selection_opens_the_panel() {
    let mut h = ecoli();
    assert!(h.state().ui.inspector_collapsed, "no selection: the map gets the column");
    select_reaction(&mut h, PGI);
    assert!(!h.state().ui.inspector_collapsed, "selecting something opens it");
    assert!(common::shows(&h, "Glucose-6-phosphate isomerase"));
    h.state_mut().ui.inspector_pinned = true;
    // clear-selection
    common::click(&mut h, "Clear selection");
    assert_eq!(h.state().store.selected_reaction, None);
    assert_eq!(h.state().store.selected_entity, None);
    assert!(label_is(&h, "No selection"));
    assert!(common::shows(&h, "Select something on the map to inspect it."));
}

#[test]
fn an_unknown_selection_has_no_properties() {
    let mut h = ecoli();
    select_entity(&mut h, "pathway", "x");
    assert!(label_is(&h, "Selection"));
    assert!(common::shows(&h, "No properties are available."));
}

// Metabolites

#[test]
fn metabolite_inspector_reports_available_model_and_map_properties() {
    let mut h = ecoli();
    let node = node_of(&h, "eco@glc__D_e", NodeKind::Metabolite);
    select_entity(&mut h, "metabolite", &node);
    assert!(common::shows(&h, "D-Glucose [Extracellular]"));
    assert!(label_is(&h, "C6H12O6"));
    assert!(label_is(&h, "E. coli"));
    let connected = h.state().store.map.as_ref().unwrap().segments_at(&node).len();
    assert!(connected > 0);
    assert!(label_is(&h, "Connected paths") && label_is(&h, &connected.to_string()));
    for control in ["Primary", "Secondary", "Duplicate metabolite", "Hide metabolite", "Metabolite label source"] {
        assert!(common::has(&h, control), "{control}");
    }
    assert!(!common::shows(&h, "Visual changes affect the map only"), "no note that the map is not the model");
}

#[test]
fn metabolite_primary_and_secondary() {
    let mut h = ecoli();
    let node = node_of(&h, "eco@g6p_c", NodeKind::Metabolite);
    select_entity(&mut h, "metabolite", &node);
    assert!(common::pressed(&h, "Primary"));
    common::click(&mut h, "Secondary");
    assert_eq!(h.state().store.map.as_ref().unwrap().node(&node).unwrap().primary, Some(false));
    assert!(common::pressed(&h, "Secondary"));
    assert!(label_is(&h, "Secondary"), "the Visual role row follows");
    common::click(&mut h, "Primary");
    assert_eq!(h.state().store.map.as_ref().unwrap().node(&node).unwrap().primary, Some(true));
}

#[test]
fn a_cofactor_helper_node_explains_its_role_instead_of_offering_a_dead_primary_button() {
    let mut h = ecoli();
    let helper = h.state().store.map.as_ref().unwrap().nodes().values().find(|n| n.kind == NodeKind::Currency).map(|n| n.id.clone()).expect("a cofactor helper");
    select_entity(&mut h, "metabolite", &helper);
    assert!(!common::has(&h, "Primary"));
    assert!(common::shows(&h, "always secondary"));
    let node = node_of(&h, "eco@g6p_c", NodeKind::Metabolite);
    select_entity(&mut h, "metabolite", &node);
    assert!(common::has(&h, "Primary"), "ordinary metabolites keep the buttons");
}

#[test]
fn duplicate_metabolite_selects_the_copy() {
    let mut h = ecoli();
    let node = node_of(&h, "eco@g6p_c", NodeKind::Metabolite);
    select_entity(&mut h, "metabolite", &node);
    let before = h.state().store.metabolite_copies(&node).len();
    common::click(&mut h, "Duplicate metabolite");
    let store = &h.state().store;
    assert_eq!(store.metabolite_copies(&node).len(), before + 1);
    let selected = store.selected_entity.clone().expect("the copy is selected");
    assert_eq!(selected.kind, "metabolite");
    assert_ne!(selected.id, node);
    assert!(store.map.as_ref().unwrap().node(&selected.id).is_some());
    assert!(label_is(&h, &(before + 1).to_string()), "Visual copies counts it");
}

#[test]
fn hide_metabolite_hides_it_and_clears_the_selection() {
    let mut h = ecoli();
    let node = node_of(&h, "eco@g6p_c", NodeKind::Metabolite);
    select_entity(&mut h, "metabolite", &node);
    common::click(&mut h, "Hide metabolite");
    assert_eq!(h.state().store.map.as_ref().unwrap().node(&node).unwrap().hidden, Some(true));
    assert_eq!(h.state().store.selected_entity, None);
}

#[test]
fn remove_map_node_deletes_a_free_instance() {
    let mut h = ecoli();
    let node = node_of(&h, "eco@g6p_c", NodeKind::Metabolite);
    select_entity(&mut h, "metabolite", &node);
    assert!(!common::has(&h, "Delete metabolite from map"), "drawn reactions use it");
    assert!(common::shows(&h, "Remove the drawn reactions that use it before deleting it."));
    common::click(&mut h, "Duplicate metabolite");
    let copy = h.state().store.selected_entity.clone().unwrap().id;
    assert!(h.state().store.can_remove_map_node(&copy));
    common::click(&mut h, "Delete metabolite from map");
    assert!(h.state().store.map.as_ref().unwrap().node(&copy).is_none());
    assert_eq!(h.state().store.selected_entity, None);
}

#[test]
fn a_shared_medium_metabolite_belongs_to_the_shared_medium_not_to_a_cell_called_m() {
    let mut h = demo(false);
    select_entity(&mut h, "metabolite", "met:M@a");
    assert!(label_is(&h, "Shared medium"), "Cell: Shared medium");
    select_entity(&mut h, "metabolite", "met:demo@a_c");
    assert!(!label_is(&h, "Shared medium"));
    assert!(label_is(&h, "audit-demo"), "the cell is the species' model");
}

#[test]
fn a_metabolite_lists_its_reactions_and_annotations() {
    let mut h = demo(false);
    select_entity(&mut h, "metabolite", "met:demo@a_c");
    assert!(common::shows(&h, "KEGG"), "the annotation is linked");
    assert!(common::shows(&h, "C00031"));
    // select-model-reaction
    common::click(&mut h, "demo@At");
    assert_eq!(h.state().store.selected_reaction.as_deref(), Some("demo@At"));
}

// Label sources

#[test]
fn metabolite_label_source_offers_inherit_and_sets_one_label() {
    let mut h = ecoli();
    let node = node_of(&h, "eco@g6p_c", NodeKind::Metabolite);
    select_entity(&mut h, "metabolite", &node);
    let options = options_of(&mut h, "Metabolite label source");
    assert!(options.iter().any(|o| o == "Follow View setting"), "{options:?}");
    let name = options.iter().find(|o| o.starts_with("Model name: ")).cloned().expect("a model-name option");
    assert!(name.contains("D-Glucose 6-phosphate"), "{name}");
    choose(&mut h, "Metabolite label source", &name);
    let source = h.state().store.map.as_ref().unwrap().node(&node).unwrap().extra.get("labelSource").cloned();
    assert_eq!(source, Some(json!("name")));
    assert_eq!(h.state().store.map_label_text("metabolite", &node, None), name.trim_start_matches("Model name: "));
    choose(&mut h, "Metabolite label source", "Follow View setting");
    assert!(h.state().store.map.as_ref().unwrap().node(&node).unwrap().extra.get("labelSource").is_none_or(Value::is_null));
}

#[test]
fn reaction_label_source_sets_one_reaction_label() {
    let mut h = ecoli();
    select_reaction(&mut h, PGI);
    let options = options_of(&mut h, "Reaction label source");
    assert!(options.iter().any(|o| o == "Follow View setting"), "{options:?}");
    let name = options.iter().find(|o| o.starts_with("Model name: ")).cloned().expect("a model-name option");
    choose(&mut h, "Reaction label source", &name);
    let source = h.state().store.map.as_ref().unwrap().reaction(PGI).unwrap().extra.get("labelSource").cloned();
    assert_eq!(source, Some(json!("name")));
    assert_eq!(h.state().store.map_label_text("reaction", PGI, None), "Glucose-6-phosphate isomerase");
}

// Texts

#[test]
fn the_text_inspector_offers_a_size_and_a_font() {
    let mut h = ecoli();
    let style = coxeter_state::TextStyle { font_size: 26.0, font_family: "serif".into() };
    let text = h.state_mut().store.add_map_text("Label", 50.0, 50.0, Some(&style)).expect("a text");
    select_entity(&mut h, "text", &text.id);
    assert_eq!(value_of(&h, "Text size in pixels"), "26");
    assert!(common::has(&h, "Text font"));
    assert!(common::shows(&h, "Also used for the next text you write."));
}

#[test]
fn text_font_size_applies_on_change_and_a_cleared_field_keeps_the_size() {
    let mut h = ecoli();
    let text = h.state_mut().store.add_map_text("Label", 50.0, 50.0, None).expect("a text");
    select_entity(&mut h, "text", &text.id);
    common::type_into(&mut h, "Text size in pixels", "30");
    assert_eq!(h.state().store.map.as_ref().unwrap().text(&text.id).unwrap().font_size, Some(30.0));
    assert_eq!(h.state().store.view.text_style.font_size, 30.0, "the next text uses it");
    clear_field(&mut h, "Text size in pixels");
    assert_eq!(h.state().store.map.as_ref().unwrap().text(&text.id).unwrap().font_size, Some(30.0), "not while typing");
    h.key_press(egui::Key::Enter);
    common::settle(&mut h);
    assert_eq!(h.state().store.map.as_ref().unwrap().text(&text.id).unwrap().font_size, Some(30.0));
    assert_eq!(value_of(&h, "Text size in pixels"), "30", "the cleared field shows the size again");
}

#[test]
fn text_font_family() {
    let mut h = ecoli();
    let text = h.state_mut().store.add_map_text("Label", 50.0, 50.0, None).expect("a text");
    select_entity(&mut h, "text", &text.id);
    choose(&mut h, "Text font", "Serif");
    assert_eq!(h.state().store.map.as_ref().unwrap().text(&text.id).unwrap().font_family.as_deref(), Some("serif"));
}

#[test]
fn remove_map_text() {
    let mut h = ecoli();
    let text = h.state_mut().store.add_map_text("Label", 50.0, 50.0, None).expect("a text");
    select_entity(&mut h, "text", &text.id);
    assert!(label_is(&h, "Label"));
    common::click(&mut h, "Delete text");
    assert!(h.state().store.map.as_ref().unwrap().text(&text.id).is_none());
    assert_eq!(h.state().store.selected_entity, None);
}

// Cells and colours

#[test]
fn colour_pickers_show_the_colour_drawn_and_offer_a_reset_only_once_customised() {
    let mut h = demo(false);
    select_entity(&mut h, "cell", "demo");
    let cell = h.state().store.map.as_ref().unwrap().cell("demo").unwrap().clone();
    let fill = value_of(&h, "Fill color");
    assert!(fill.len() == 7 && fill.starts_with('#'), "{fill}");
    assert_eq!(value_of(&h, "Border color"), cell.color, "border shows the species colour");
    assert!(!common::has(&h, "Use theme colors"));
    let colors = coxeter_state::map_edit::GeometryColors {
        stroke: coxeter_map::editable_map::ColorChange::Set("#aa0000".into()),
        ..Default::default()
    };
    h.state_mut().store.set_map_geometry_colors("demo", None, &colors);
    common::settle(&mut h);
    assert_eq!(value_of(&h, "Border color"), "#aa0000");
    assert!(common::has(&h, "Use theme colors"));

    let comp = cell.compartments[0].id.clone();
    h.state_mut().store.select_geometry(Some("demo"), &comp);
    common::settle(&mut h);
    let fill = value_of(&h, "Fill color");
    assert!(fill.len() == 7 && fill.starts_with('#'), "{fill}");
    assert_ne!(fill, "#dbeafe", "no stale hard-coded default");
}

#[test]
fn cell_colors_are_set_and_reset() {
    let mut h = demo(false);
    select_entity(&mut h, "cell", "demo");
    // cell-fill-color: the fill of a cell is the fill of its body.
    common::type_into(&mut h, "Fill color", "#12AB34");
    let cell = h.state().store.map.as_ref().unwrap().cell("demo").unwrap().clone();
    let body = cell.compartments.iter().find(|c| Some(&c.id) == cell.outer_compartment.as_ref()).unwrap();
    assert_eq!(body.fill.as_deref(), Some("#12ab34"));
    // cell-border-color
    common::type_into(&mut h, "Border color", "#aa0000");
    assert_eq!(h.state().store.map.as_ref().unwrap().cell("demo").unwrap().stroke.as_deref(), Some("#aa0000"));
    assert!(h.state().store.can_undo_map());
    // cell-reset-colors
    common::click(&mut h, "Use theme colors");
    let cell = h.state().store.map.as_ref().unwrap().cell("demo").unwrap().clone();
    assert_eq!(cell.stroke, None);
    assert!(cell.compartments.iter().all(|c| c.fill.is_none()));
    assert!(!common::has(&h, "Use theme colors"));
}

#[test]
fn compartment_colors_are_set_and_reset() {
    let mut h = demo(false);
    let comp = h.state().store.map.as_ref().unwrap().cell("demo").unwrap().compartments.last().unwrap().id.clone();
    h.state_mut().store.select_geometry(Some("demo"), &comp);
    common::settle(&mut h);
    assert!(label_is(&h, &comp) || common::shows(&h, &comp));
    let find = |h: &H| h.state().store.map.as_ref().unwrap().cell("demo").unwrap().compartments.iter().find(|c| c.id == comp).unwrap().clone();
    // compartment-fill-color / compartment-border-color
    common::type_into(&mut h, "Fill color", "#0000ff");
    common::type_into(&mut h, "Border color", "#00ff00");
    assert_eq!(find(&h).fill.as_deref(), Some("#0000ff"));
    assert_eq!(find(&h).stroke.as_deref(), Some("#00ff00"));
    // compartment-reset-colors
    common::click(&mut h, "Use theme colors");
    assert_eq!(find(&h).fill, None);
    assert_eq!(find(&h).stroke, None);
}

#[test]
fn cell_inspector_lists_the_cell_and_edits_its_boundary() {
    let mut h = demo(false);
    select_entity(&mut h, "cell", "demo");
    assert!(label_is(&h, "100%"), "abundance");
    assert!(common::shows(&h, "h⁻¹"), "growth rate");
    // edit-selected-cell-boundary
    common::click(&mut h, "Edit Boundary");
    let store = &h.state().store;
    assert!(store.view.geometry_edit);
    assert_eq!(store.selected_geometry.as_ref().map(|g| g.cell_id.clone()), Some(Some("demo".to_string())));
}

#[test]
fn compartment_inspector_edits_its_boundary_and_the_shared_medium_is_one() {
    let mut h = demo(false);
    let comp = h.state().store.map.as_ref().unwrap().cell("demo").unwrap().compartments.last().unwrap().id.clone();
    h.state_mut().store.select_geometry(Some("demo"), &comp);
    common::settle(&mut h);
    assert!(common::shows(&h, "Drag the centre to move it, a corner or side to resize it."));
    assert!(label_is(&h, "Cell root") || common::shows(&h, "Parent"));
    // edit-selected-compartment-boundary
    common::click(&mut h, "Edit Boundary");
    assert!(h.state().store.view.geometry_edit);

    let env = h.state().store.map.as_ref().unwrap().environment().id.clone();
    h.state_mut().store.select_geometry(None, &env);
    common::settle(&mut h);
    assert!(label_is(&h, "Shared extracellular medium"));
    assert!(label_is(&h, "System boundary"));
    let _ = h.state_mut().store.set_view(coxeter_state::ViewPatch { geometry_edit: Some(false), ..Default::default() });
    common::settle(&mut h);
    common::click(&mut h, "Edit Boundary");
    assert!(h.state().store.view.geometry_edit);
}

// Genes

#[test]
fn genes_of_a_reaction_open_the_gene_inspector_and_lead_back() {
    let mut h = demo(false);
    select_reaction(&mut h, "demo@At");
    assert!(label_is(&h, "g1 and (g2 or g3)"), "GPR");
    // select-gene
    common::click(&mut h, "g1");
    let store = &h.state().store;
    assert_eq!(store.selected_reaction, None);
    let entity = store.selected_entity.clone().expect("the gene");
    assert_eq!((entity.kind.as_str(), entity.id.as_str()), ("gene", "g1"));
    assert_eq!(entity.details.get("speciesId"), Some(&json!("demo")));
    assert!(label_is(&h, "geneOne"));
    assert!(common::shows(&h, "945"), "its annotation");
    assert!(common::shows(&h, "source: test"), "its notes");
    // select-model-reaction
    common::click(&mut h, "At");
    assert_eq!(h.state().store.selected_reaction.as_deref(), Some("demo@At"));
}

#[test]
fn a_missing_gene_says_so() {
    let mut h = demo(false);
    let mut details = Map::new();
    details.insert("speciesId".into(), json!("demo"));
    h.state_mut().store.select_entity("gene", "nope", details);
    common::settle(&mut h);
    assert!(common::shows(&h, "Gene nope not found."));
}

// The reaction card

#[test]
fn the_reaction_card_lists_the_reaction() {
    let mut h = demo(false);
    select_reaction(&mut h, "demo@At");
    for text in ["A transport", "demo@a_e --> demo@a_c", "Original", "audit-demo"] {
        assert!(common::shows(&h, text), "{text}");
    }
    assert!(common::shows(&h, "R00001"), "the reaction's annotation");
    assert!(!common::shows(&h, "Enable FVA") && !common::shows(&h, "Computing the FVA range"), "nothing on FVA while it is off");
    assert!(common::shows(&h, "Original bounds: [0, 1.00e+3]"), "formatFlux writes 1000 as 1.00e+3");
    // select-model-metabolite: a drawn metabolite is selected…
    common::click(&mut h, "a_c");
    let entity = h.state().store.selected_entity.clone().expect("the metabolite");
    assert_eq!(entity.kind, "metabolite");
    let node = h.state().store.map.as_ref().unwrap().node(&entity.id).unwrap().clone();
    assert_eq!(node.merged_id.as_deref(), Some("demo@a_c"));
    assert_eq!(h.state().store.selected_reaction, None);
}

#[test]
fn select_model_metabolite_says_when_it_is_not_drawn() {
    let mut h = pfk_alone();
    // PGI is in the model but not drawn, and neither is g6p.
    select_reaction(&mut h, PGI);
    common::click(&mut h, "g6p_c");
    let status = h.state().store.status().clone();
    assert_eq!(status.state, StatusState::Ok);
    assert_eq!(status.message, "Metabolite eco@g6p_c is not drawn on the map.");
    assert_eq!(h.state().store.selected_reaction.as_deref(), Some(PGI), "the selection stays");
}

#[test]
fn fva_range_is_drawn_once_computed() {
    let mut h = demo(false);
    h.state_mut().store.set_fva(true).expect("FVA on");
    common::settle(&mut h);
    select_reaction(&mut h, "demo@At");
    assert!(common::shows(&h, "FVA at the optimal objective"));
}

#[test]
fn knockout_reset_and_reset_all() {
    let mut h = ecoli();
    select_reaction(&mut h, PGI);
    common::click(&mut h, "Knockout");
    let store = &h.state().store;
    let bounds = store.current_reaction_bounds(PGI);
    assert_eq!((bounds.lower, bounds.upper), (0.0, 0.0));
    assert_eq!(store.solution.as_ref().unwrap().fluxes.get(PGI).copied().unwrap_or(0.0), 0.0);
    assert!(label_is(&h, "Knocked out"));
    assert!(common::has(&h, "Restore"), "a knocked-out reaction offers its way back");
    // reset-reaction
    common::click(&mut h, "Reset");
    assert!(!h.state().store.overrides.contains_key(PGI));
    assert!(label_is(&h, "Original"));
    // reset-all
    h.state_mut().store.set_reaction_bounds(PGI, -5.0, coxeter_state::UserNumber::Empty).unwrap();
    h.state_mut().store.set_reaction_bounds(PFK, coxeter_state::UserNumber::Empty, 7.0).unwrap();
    common::settle(&mut h);
    common::click(&mut h, "All");
    assert!(h.state().store.overrides.is_empty());
}

#[test]
fn bound_sliders_apply_and_solve() {
    let mut h = ecoli();
    select_reaction(&mut h, PGI);
    let before = h.state().store.current_reaction_bounds(PGI);
    // Selecting a reaction applies nothing by itself.
    assert!(!h.state().store.overrides.contains_key(PGI));
    // The sliders are sign-log: a position, read back as a bound by `sign_log_value`.
    let at = |v: f64| sign_log_position(v, &SIGN_LOG) as f64;
    let five = sign_log_value(at(5.0), &SIGN_LOG);
    assert!((five - 5.0).abs() < 0.05, "a position lands on a number worth reading: {five}");
    // bound-lower
    common::set_value(&mut h, "Lower bound of PGI, logarithmic slider", at(-5.0));
    assert_eq!(h.state().store.current_reaction_bounds(PGI).lower, -five);
    assert_eq!(h.state().store.current_reaction_bounds(PGI).upper, before.upper);
    // bound-upper
    common::set_value(&mut h, "Upper bound of PGI, logarithmic slider", at(5.0));
    assert_eq!(h.state().store.current_reaction_bounds(PGI).upper, five);
    let flux = h.state().store.solution.as_ref().unwrap().fluxes.get(PGI).copied().unwrap_or(0.0);
    assert!(flux <= five + 1e-9, "solved with the new bound: {flux}");
    assert_eq!(value_of(&h, "Upper bound of PGI"), coxeter_core::js::number_to_string(five), "the number field follows");
}

#[test]
fn bound_number_fields_apply_and_solve() {
    let mut h = ecoli();
    select_reaction(&mut h, PGI);
    // bound-lower-number
    common::type_into(&mut h, "Lower bound of PGI", "-3");
    assert_eq!(h.state().store.current_reaction_bounds(PGI).lower, -3.0);
    // bound-upper-number
    common::type_into(&mut h, "Upper bound of PGI", "2.5");
    assert_eq!(h.state().store.current_reaction_bounds(PGI).upper, 2.5);
    let flux = h.state().store.solution.as_ref().unwrap().fluxes.get(PGI).copied().unwrap_or(0.0);
    assert!(flux <= 2.5 + 1e-9, "solved with the new bound: {flux}");
}

#[test]
fn a_cleared_bound_field_is_marked_and_changes_nothing() {
    let mut h = ecoli();
    select_reaction(&mut h, PGI);
    let before = h.state().store.current_reaction_bounds(PGI);
    clear_field(&mut h, "Upper bound of PGI");
    assert!(invalid(&h, "Upper bound of PGI"), "aria-invalid while it holds no number");
    assert_eq!(h.state().store.current_reaction_bounds(PGI).upper, before.upper, "nothing applied");
    h.key_press(egui::Key::Enter);
    common::settle(&mut h);
    assert!(!invalid(&h, "Upper bound of PGI"));
    assert_eq!(h.state().store.current_reaction_bounds(PGI).upper, before.upper);
    assert!(!h.state().store.overrides.contains_key(PGI), "no override from an ignored value");
    let status = h.state().store.status().clone();
    assert_eq!(status.state, StatusState::Warn);
    assert_eq!(status.message, coxeter_app::panels::inspector::NOT_A_NUMBER);
    assert_eq!(value_of(&h, "Upper bound of PGI"), coxeter_core::js::number_to_string(before.upper), "it shows the value in force");
}

#[test]
fn a_double_click_gives_the_keyboard_to_the_lower_bound() {
    let mut h = ecoli();
    select_reaction(&mut h, PGI);
    h.state_mut().on_canvas_event(coxeter_canvas::CanvasEvent::FocusLowerBound);
    common::settle(&mut h);
    assert!(common::control(&h, "Lower bound of PGI").is_focused());
    assert!(!h.state().ui.focus_lower_bound, "the request is taken");
}

#[test]
fn remove_map_reaction_keeps_it_in_the_model() {
    let mut h = ecoli();
    select_reaction(&mut h, PGI);
    assert!(!common::shows(&h, "Removing it from the map is not a knockout"));
    common::click(&mut h, "Delete reaction from map");
    let store = &h.state().store;
    assert!(!store.map.as_ref().unwrap().has_reaction(PGI));
    assert!(store.community.as_ref().unwrap().merged.reactions.contains_key(PGI), "not a knockout");
    assert_eq!(store.selected_reaction, None);
}

#[test]
fn switching_a_species_off_leaves_its_reaction_without_bounds() {
    let mut h = demo(true);
    h.state_mut().store.set_species_enabled("other", false).expect("switched off");
    common::settle(&mut h);
    select_reaction(&mut h, "other@At");
    assert!(common::shows(&h, "is switched off, so this reaction is not simulated"));
    assert!(!common::has(&h, "Upper bound of At"), "no bounds to edit for a reaction the analysis cannot see");
}

#[test]
fn a_zero_abundance_joint_member_is_held_out_like_a_switched_off_one() {
    let mut h = demo(true);
    {
        let store = &mut h.state_mut().store;
        store.joint_projection_spec = Some(Arc::new(coxeter_solver::steadycom::SteadyComSpec {
            members: Vec::new(),
            total_biomass: 1.0,
            biomass_cap: None,
            community_uptake: Default::default(),
            community_exchange_bounds: Default::default(),
            joint: None,
        }));
        let mut community = (**store.community.as_ref().unwrap()).clone();
        community.abundances = indexmap::IndexMap::from([("demo".to_string(), 1.0), ("other".to_string(), 0.0)]);
        store.community = Some(Arc::new(community));
        store.select(Some("other@At"));
    }
    common::settle(&mut h);
    assert!(common::shows(&h, "has zero abundance, so this reaction is held at zero"));
    assert!(!common::has(&h, "Upper bound of At"));
}

// Reaction drawing

#[test]
fn the_reaction_inspector_offers_linear_swap_mirror_and_rotate_as_map_options() {
    let h = pfk_alone();
    for control in ["Free", "Linear", "Swap substrates ⇄ products", "Mirror", "⟲ 90°", "⟳ 90°", "Vertical", "Horizontal"] {
        assert!(common::has(&h, control), "{control} is offered");
    }
    // One swap between each pair of neighbours: 1 for two substrates, 2 for three products.
    let swaps: Vec<String> = control_labels(&h, "Swap ").into_iter().filter(|l| l != "Swap substrates ⇄ products").collect();
    assert_eq!(swaps.len(), 3, "{swaps:?}");
    assert!(common::shows(&h, "Shift-click two metabolites to pick them."));
}

#[test]
fn the_inspector_offers_the_compass_the_metabolite_choice_and_tidy() {
    let h = pfk_alone();
    for direction in ["N", "NE", "E", "SE", "S", "SW", "W", "NW"] {
        assert!(common::has(&h, &format!("Align {direction}")), "{direction}");
    }
    let (subs, prods) = h.state().store.map.as_ref().unwrap().reaction_sides(PFK);
    let chips: Vec<_> = subs.iter().chain(prods.iter()).map(|e| common::control(&h, &e.label).accesskit_node().toggled()).collect();
    assert_eq!(chips.len(), 5, "one per drawn metabolite");
    assert_eq!(chips.iter().filter(|t| **t == Some(Toggled::True)).count(), 2, "the two the line would run through are marked");
    assert!(common::has(&h, "Tidy curves"));
}

/// The reaction's name moved off its default place: Reset label is offered,
/// and puts it back as one undo step (`test/reaction-label.test.mjs`).
#[test]
fn reset_label_is_offered_for_a_moved_name_and_puts_it_back() {
    let mut h = pfk_alone();
    assert!(!common::has(&h, "Reset label"), "offered only for a moved name");
    h.state_mut().store.set_reaction_label_offset(PFK, coxeter_core::mapdata::Point::new(25.0, -10.0));
    h.state_mut().store.emit("map");
    common::settle(&mut h);
    let depth = undo_depth(&h);
    common::click(&mut h, "Reset label");
    let rxn = h.state().store.map.as_ref().unwrap().reaction(PFK).unwrap().clone();
    assert_eq!(coxeter_core::reaction_label::reaction_label_offset(&rxn), None);
    assert_eq!(undo_depth(&h), depth + 1);
    assert!(!common::has(&h, "Reset label"));
}

// Selection

/// `chainSelection(map, reactions)`: some reactions and every metabolite
/// drawn to them, cofactors included.
fn select_chain(h: &mut H, reactions: &[&str]) {
    let store = &mut h.state_mut().store;
    store.select(None);
    let mut selection = coxeter_map::multi_selection::empty_selection();
    selection.reactions.extend(reactions.iter().map(|id| id.to_string()));
    for seg in store.map.as_ref().unwrap().segments() {
        if selection.reactions.contains(&seg.reaction_id) {
            if let Some(node) = seg.node_id.as_deref().filter(|id| !id.is_empty()) {
                selection.nodes.insert(node.to_string());
            }
        }
    }
    store.multi_selection = selection;
    store.emit("multi-selection");
}

/// JS (selection-turn.test.mjs): "the inspector turns to a selection of two or more and offers its tools there"
#[test]
fn the_inspector_turns_to_a_selection_of_two_or_more_and_offers_its_tools_there() {
    let mut h = ecoli();
    h.state_mut().store.select(None);
    common::settle(&mut h);
    assert!(h.state().ui.inspector_collapsed, "nothing selected");
    select_chain(&mut h, &[PGI, PFK]);
    common::settle(&mut h);
    let nodes = h.state().store.multi_selection.nodes.len();
    assert!(!h.state().ui.inspector_collapsed, "the inspector opens for it");
    assert!(common::shows(&h, "Selection"));
    assert!(common::shows(&h, &format!("2 reactions · {nodes} metabolites")));
    let distribute_v = format!("Distribute {}", coxeter_app::fonts::glyph::DISTRIBUTE_V);
    for control in ["Horizontal", "Vertical", "Left", distribute_v.as_str(), "Even spacing", "⟲ 45°", "⟳ 45°", "Clear selection"] {
        assert!(common::has(&h, control), "{control} is offered");
    }
    for direction in ["N", "NE", "E", "SE", "S", "SW", "W", "NW"] {
        assert!(common::has(&h, &format!("Point {direction}")), "{direction}");
    }
    assert!(common::shows(&h, "Turns as one piece; cofactors follow."));

    // A reaction picked on its own keeps its inspector: it reads line ends from the selection.
    h.state_mut().store.select(Some(PGI));
    common::settle(&mut h);
    assert!(!common::has(&h, "Point N"));
    assert!(common::has(&h, "Align N"));
}

#[test]
fn the_selection_turns_points_and_is_cleared_from_the_inspector() {
    let mut h = ecoli();
    select_chain(&mut h, &[PGI, PFK]);
    common::settle(&mut h);
    common::click(&mut h, "Vertical");
    select_chain(&mut h, &[PGI, PFK]);
    common::settle(&mut h);
    let labels = |h: &H| h.state().store.map_history.undo_labels().iter().map(|l| l.to_string()).collect::<Vec<_>>();

    common::click(&mut h, "Point E");
    let (g6p, fdp) = (node_pos(&h, "met:eco@g6p_c"), node_pos(&h, "met:eco@fdp_c"));
    assert!((g6p.1 - fdp.1).abs() < 1e-6 && fdp.0 > g6p.0, "glucose 6-phosphate to fructose bisphosphate runs east");
    assert_eq!(labels(&h).last().map(String::as_str), Some("Point selection E"));
    common::click(&mut h, "⟳ 45°");
    assert_eq!(labels(&h).last().map(String::as_str), Some("Rotate selection"));
    let (g6p, fdp) = (node_pos(&h, "met:eco@g6p_c"), node_pos(&h, "met:eco@fdp_c"));
    assert!(((fdp.1 - g6p.1) - (fdp.0 - g6p.0)).abs() < 1e-6 && fdp.0 > g6p.0, "and now down to the right");
    let depth = undo_depth(&h);
    common::click(&mut h, "Even spacing");
    assert!(undo_depth(&h) <= depth + 1);

    common::click(&mut h, "Clear selection");
    assert_eq!(h.state().store.multi_selection_size(), 0);
    assert!(h.state().ui.inspector_collapsed, "the inspector gives the map its column back");
}

#[test]
fn reaction_linear_and_free() {
    let mut h = pfk_alone();
    let depth = undo_depth(&h);
    common::click(&mut h, "Linear");
    assert!(h.state().store.map.as_ref().unwrap().reaction(PFK).unwrap().linear.is_some());
    assert!(common::shows(&h, "Kept straight between f6p and fdp"));
    assert!(common::pressed(&h, "Linear"));
    common::click(&mut h, "Free");
    assert!(h.state().store.map.as_ref().unwrap().reaction(PFK).unwrap().linear.is_none());
    assert_eq!(undo_depth(&h), depth + 2, "two undoable edits");
}

#[test]
fn reaction_swap_exchanges_two_metabolites() {
    let mut h = pfk_alone();
    let (subs, _) = h.state().store.map.as_ref().unwrap().reaction_sides(PFK);
    let (a, b) = (subs[0].clone(), subs[1].clone());
    let (pa, pb) = (node_pos(&h, &a.node_id), node_pos(&h, &b.node_id));
    common::click(&mut h, &format!("Swap {} and {}", a.label, b.label));
    let (na, nb) = (node_pos(&h, &a.node_id), node_pos(&h, &b.node_id));
    assert!(near(na.0, pb.0) && near(na.1, pb.1), "{} stands where {} stood", a.label, b.label);
    assert!(near(nb.0, pa.0) && near(nb.1, pa.1));
}

#[test]
fn reaction_line_end_and_align_and_picked_metabolites() {
    let mut h = pfk_alone();
    let atp = side_node(&h, PFK, "atp");
    // reaction-line-end
    common::click(&mut h, "atp");
    assert_eq!(h.state().store.map.as_ref().unwrap().reaction_line_ends(PFK).0.as_deref(), Some(atp.as_str()));
    assert!(common::pressed(&h, "atp"));
    // reaction-align
    common::click(&mut h, "Align E");
    let rxn = marker(&h, PFK);
    let (from, to) = h.state().store.map.as_ref().unwrap().reaction_line_ends(PFK);
    let (from, to) = (node_pos(&h, &from.unwrap()), node_pos(&h, &to.unwrap()));
    assert!(near(from.1, rxn.1) && near(to.1, rxn.1), "in line with the marker");
    assert!(from.0 < rxn.0 && to.0 > rxn.0, "substrate to product runs to the right");
}

#[test]
fn metabolites_picked_out_on_the_map_are_the_ones_linear_and_the_compass_use() {
    let mut h = pfk_alone();
    let atp = side_node(&h, PFK, "atp");
    let hh = side_node(&h, PFK, "h");
    h.state_mut().store.toggle_multi_selection("node", &atp);
    h.state_mut().store.toggle_multi_selection("node", &hh);
    common::settle(&mut h);
    assert!(common::pressed(&h, "atp"), "the picked substrate is the marked one");
    assert!(common::pressed(&h, "h"));
    common::click(&mut h, "Align W");
    let linear = h.state().store.map.as_ref().unwrap().reaction(PFK).unwrap().linear.clone().unwrap();
    assert_eq!((linear.from.as_deref(), linear.to.as_deref()), (Some(atp.as_str()), Some(hh.as_str())));
}

#[test]
fn reaction_align_axis() {
    let mut h = ecoli();
    select_reaction(&mut h, PGI);
    common::click(&mut h, "Vertical");
    let rxn = marker(&h, PGI);
    let (from, to) = h.state().store.map.as_ref().unwrap().reaction_line_ends(PGI);
    let (from, to) = (from.unwrap(), to.unwrap());
    assert!(near(node_pos(&h, &from).0, rxn.0) && near(node_pos(&h, &to).0, rxn.0), "one vertical line through the marker");
    common::click(&mut h, "Horizontal");
    assert!(near(node_pos(&h, &from).1, rxn.1) && near(node_pos(&h, &to).1, rxn.1), "one horizontal line");
}

#[test]
fn reaction_tidy_puts_a_cofactor_back() {
    let mut h = pfk_alone();
    let map = h.state().store.map.as_ref().unwrap();
    let helper = map.nodes().values().find(|n| n.kind == NodeKind::Currency && n.reaction_id.as_deref() == Some(PFK)).map(|n| n.id.clone()).expect("a cofactor of PFK");
    let home = node_pos(&h, &helper);
    {
        let store = &mut h.state_mut().store;
        let tx = store.begin_map_edit("drag");
        store.map.as_mut().unwrap().move_node(&helper, home.0 + 90.0, home.1 + 70.0);
        store.commit_map_edit(tx);
    }
    common::settle(&mut h);
    let depth = undo_depth(&h);
    common::click(&mut h, "Tidy curves");
    assert_eq!(undo_depth(&h), depth + 1, "tidying is one undo step");
    let back = node_pos(&h, &helper);
    assert!((back.0 - home.0).abs() < 1.0 && (back.1 - home.1).abs() < 1.0, "{home:?} → {back:?}");
}

#[test]
fn reaction_flip_and_mirror_are_undoable_and_leave_the_model_alone() {
    let mut h = pfk_alone();
    let equation = h.state().store.community.as_ref().unwrap().merged.reactions.get(PFK).unwrap().equation();
    let f6p = side_node(&h, PFK, "f6p");
    let fdp = side_node(&h, PFK, "fdp");
    let (a, b) = (node_pos(&h, &f6p), node_pos(&h, &fdp));
    let depth = undo_depth(&h);
    common::click(&mut h, "Swap substrates ⇄ products");
    let (na, nb) = (node_pos(&h, &f6p), node_pos(&h, &fdp));
    assert!((na.0 - b.0).abs() < 1.0 && (na.1 - b.1).abs() < 1.0 && (nb.0 - a.0).abs() < 1.0 && (nb.1 - a.1).abs() < 1.0, "the two ends trade places");
    common::click(&mut h, "Mirror");
    assert_eq!(undo_depth(&h), depth + 2);
    assert_eq!(h.state().store.community.as_ref().unwrap().merged.reactions.get(PFK).unwrap().equation(), equation);
}

#[test]
fn reaction_rotate_turns_a_quarter() {
    let mut h = pfk_alone();
    let angle = |h: &H| h.state().store.map.as_ref().unwrap().reaction_angle(PFK).unwrap();
    let start = angle(&h);
    common::click(&mut h, "⟳ 90°");
    let turned = angle(&h);
    let diff = (turned - start).rem_euclid(std::f64::consts::TAU);
    assert!((diff - std::f64::consts::FRAC_PI_2).abs() < 1e-6, "{start} → {turned}");
    common::click(&mut h, "⟲ 90°");
    let back = (angle(&h) - start).rem_euclid(std::f64::consts::TAU);
    assert!(back < 1e-6 || (back - std::f64::consts::TAU).abs() < 1e-6);
}

// Layout settings

fn layout_tab(h: &mut H) {
    h.state_mut().ui.inspector_collapsed = false;
    common::settle(h);
    common::click(h, "Layout");
    assert_eq!(h.state().ui.right_tab, RightTab::Layout);
}

#[test]
fn layout_settings_panel_renders_the_same_controls_shown_in_the_canvas_toolbar() {
    let mut h = ecoli();
    layout_tab(&mut h);
    for control in ["Compact layout preset", "Normal layout preset", "Spacious layout preset", "Cell area", "Cell spacing", "Minimum compartment size", "Node spacing", "⤢ Fit to screen"] {
        assert!(common::has(&h, control), "{control}");
    }
    assert!(common::pressed(&h, "Spacious layout preset"), "spacious is the default");
    assert!(label_is(&h, "6.0×"));
    let map = h.state().store.map.as_ref().unwrap();
    let (w, hh) = (coxeter_core::js::round(map.width()), coxeter_core::js::round(map.height()));
    assert_eq!(value_of(&h, "Canvas width"), coxeter_core::js::number_to_string(w));
    assert_eq!(value_of(&h, "Canvas height"), coxeter_core::js::number_to_string(hh));
    assert!(!common::disabled(&h, "Cell area"), "nothing blocks the layout");
}

#[test]
fn layout_sliders_lay_the_map_out_again() {
    let mut h = ecoli();
    layout_tab(&mut h);
    common::set_value(&mut h, "Cell area", 4.0);
    assert_eq!(h.state().store.layout_settings.cell_area_multiplier, 4.0);
    assert_eq!(h.state().store.layout_settings.preset, "custom");
    assert!(label_is(&h, "4.0×"));
    assert!(common::shows(&h, "Custom spacing"));
    common::set_value(&mut h, "Cell spacing", 300.0);
    assert_eq!(h.state().store.layout_settings.cell_spacing, 300.0);
    assert!(label_is(&h, "300 px"));
    common::set_value(&mut h, "Minimum compartment size", 80.0);
    assert_eq!(h.state().store.layout_settings.minimum_compartment_size, 80.0);
    common::set_value(&mut h, "Node spacing", 1.2);
    assert!((h.state().store.layout_settings.node_spacing - 1.2).abs() < 1e-9);
    assert!(h.state().store.can_undo_map(), "Undo restores the previous map");
}

#[test]
fn opening_the_layout_tab_changes_nothing() {
    // Spacious keeps a node spacing (1.42) off the slider's 0.05 grid: the
    // slider shows 1.40 and must not report that as a change.
    let mut h = ecoli();
    let before = h.state().store.layout_settings.clone();
    let depth = undo_depth(&h);
    layout_tab(&mut h);
    common::settle(&mut h);
    assert_eq!(h.state().store.layout_settings, before);
    assert_eq!(undo_depth(&h), depth);
}

#[test]
fn layout_preset_and_fit_in_the_layout_tab() {
    let mut h = ecoli();
    layout_tab(&mut h);
    common::click(&mut h, "Compact layout preset");
    assert_eq!(h.state().store.layout_settings.preset, "compact");
    assert!(common::pressed(&h, "Compact layout preset"));
    common::settle(&mut h);
    let fitted = h.state().map.camera().scale;
    common::click(&mut h, "Zoom in");
    assert!((h.state().map.camera().scale - fitted).abs() > 1e-9);
    // zoom-fit
    common::click(&mut h, "⤢ Fit to screen");
    assert!((h.state().map.camera().scale - fitted).abs() < 1e-9);
}

#[test]
fn layout_canvas_size_is_one_undoable_edit() {
    let mut h = ecoli();
    layout_tab(&mut h);
    let depth = undo_depth(&h);
    let height = h.state().store.map.as_ref().unwrap().height();
    // The other side is set again as it is, which the drawing may hold a
    // little larger (`setMapCanvasSize` keeps the drawing inside).
    let expected = h.state().store.preview_canvas_size(5000.0, height).unwrap();
    common::type_into(&mut h, "Canvas width", "5000");
    assert_eq!(h.state().store.map.as_ref().unwrap().width(), 5000.0);
    assert_eq!(h.state().store.map.as_ref().unwrap().height(), expected.height);
    assert_eq!(undo_depth(&h), depth + 1);
    common::type_into(&mut h, "Canvas height", "4000");
    assert_eq!(h.state().store.map.as_ref().unwrap().height(), 4000.0);
    assert_eq!(value_of(&h, "Canvas height"), "4000");
}

#[test]
fn blocked_layout_sliders_say_why() {
    let mut shell = common::shell(None);
    let mets: Vec<Value> = (0..60).map(|i| json!({"id": format!("m{i}_c"), "compartment": "c"})).collect();
    let rxns: Vec<Value> = (0..400)
        .map(|i| {
            let mut stoich = Map::new();
            stoich.insert(format!("m{}_c", i % 60), json!(-1));
            stoich.insert(format!("m{}_c", (i + 1) % 60), json!(1));
            json!({"id": format!("R{i}"), "metabolites": stoich, "lower_bound": 0, "upper_bound": 1000})
        })
        .collect();
    let big = Model::from_value(&json!({"id": "big", "compartments": {"c": "Cytosol"}, "metabolites": mets, "reactions": rxns})).unwrap();
    shell.store.set_model("big", big);
    shell.store.scenarios.push(json!({"id": "big", "name": "Big", "objective": "equal-growth", "species": [{"id": "big", "model": "big", "abundance": 1}], "medium": {}}));
    shell.store.use_scenario("big").expect("the big model opens");
    let community = shell.store.community.clone().expect("a community");
    shell.store.set_map(Some(EditableMap::empty_from_community(&community, &LayoutOptions::default())));
    shell.store.add_map_reaction("big@R0", &PlaceOptions::default());
    let why = shell.store.relayout_blocked().expect("a hand-built drawing of a large model");
    let mut h = common::harness_of(shell);
    layout_tab(&mut h);
    for slider in ["Cell area", "Cell spacing", "Minimum compartment size", "Node spacing"] {
        assert!(common::disabled(&h, slider), "{slider}");
    }
    assert!(common::disabled(&h, "Compact layout preset"));
    assert!(common::shows(&h, &why), "the note says why");
}

// Compartment shape

/// compartment-shape: the inspector's presets (`compartment-shape`) reshape in
/// one undo step; `remove-map-compartment` deletes from the inspector.
#[test]
fn compartment_inspector_offers_shape_presets_and_delete() {
    let mut h = demo(false);
    let comp = h.state().store.map.as_ref().unwrap().cell("demo").unwrap().compartments.iter().find(|c| c.parent_id.is_some()).unwrap().id.clone();
    h.state_mut().store.select_geometry(Some("demo"), &comp);
    common::settle(&mut h);
    assert!(common::shows(&h, "orange diamond"));
    assert!(common::shows(&h, "a corner or side to resize it"));
    let shape = |h: &H| {
        let map = h.state().store.map.as_ref().unwrap();
        coxeter_core::geometry::compartment_shape(map.cell("demo").unwrap().compartments.iter().find(|c| c.id == comp).unwrap())
    };
    common::click(&mut h, "Oval");
    assert_eq!(shape(&h), coxeter_core::geometry::compartment_shape_preset("oval").unwrap());
    assert!(common::pressed(&h, "Oval"));
    assert!(!common::pressed(&h, "Rectangle"));
    h.state_mut().store.undo_map();
    common::settle(&mut h);
    assert_ne!(shape(&h), coxeter_core::geometry::compartment_shape_preset("oval").unwrap(), "one undo step");

    h.state_mut().store.select_geometry(Some("demo"), &comp);
    common::settle(&mut h);
    common::click(&mut h, "Delete compartment");
    assert!(h.state().store.map.as_ref().unwrap().cell("demo").unwrap().compartments.iter().all(|c| c.id != comp));
}

#[test]
fn cell_inspector_deletes_the_cell() {
    let mut h = demo(false);
    select_entity(&mut h, "cell", "demo");
    common::click(&mut h, "Delete cell");
    assert!(h.state().store.map.as_ref().unwrap().cell("demo").is_none());
    assert!(h.state().store.status().message.starts_with("Removed the cell "));
}

/// compartment-shape: the Add Compartment tool lists the model's
/// compartments and a shape (`compartment-placement`,
/// `compartment-shape-preset`).
#[test]
fn the_compartment_tool_offers_the_compartment_and_its_shape() {
    let mut h = demo(false);
    // A blank map with only the cell drawn: the cytosol is ready to draw.
    {
        let store = &mut h.state_mut().store;
        store.set_map(Some(EditableMap::blank(Some(1200.0), Some(850.0))));
        store.add_map_cell("demo", Some(coxeter_core::mapdata::Rect::new(80.0, 70.0, 900.0, 650.0)));
    }
    common::settle(&mut h);
    common::click(&mut h, "Add Compartment");
    assert!(common::has(&h, "Compartment to add"));
    assert!(common::shows(&h, "highlighted on the map"));
    common::choose(&mut h, "Shape of the new compartment", "Capsule");
    assert_eq!(h.state().store.compartment_shape_preset, "capsule");
    let key = h.state_mut().store.current_compartment_placement().unwrap();
    assert_eq!(key.compartment_id, "c");
}

// Colours

/// line-styles: a metabolite's own circle colours (`node-fill`,
/// `node-stroke`, `node-colors-reset`) and a reaction's own line colour
/// (`reaction-line-color`, `reaction-color-reset`), from the inspector.
#[test]
fn the_inspector_colours_a_metabolite_circle_and_a_reaction_line() {
    let mut h = ecoli();
    let node = h.state().store.map.as_ref().unwrap().nodes().values().find(|n| n.is_metabolite_kind()).unwrap().id.clone();
    select_entity(&mut h, "metabolite", &node);
    assert!(common::disabled(&h, "Map colors"));
    common::type_into(&mut h, "Circle fill", "#ff0000");
    common::type_into(&mut h, "Circle outline", "#00ff00");
    let own = |h: &H| {
        let n = h.state().store.map.as_ref().unwrap().node(&node).unwrap().clone();
        (n.own_fill().map(str::to_string), n.own_stroke().map(str::to_string))
    };
    assert_eq!(own(&h), (Some("#ff0000".into()), Some("#00ff00".into())));
    common::click(&mut h, "Map colors");
    assert_eq!(own(&h), (None, None));

    select_reaction(&mut h, PFK);
    assert!(common::shows(&h, "Used when lines are not colored by flux."));
    common::type_into(&mut h, "Reaction line color", "#123456");
    let color = |h: &H| h.state().store.map.as_ref().unwrap().reaction(PFK).unwrap().own_color().map(str::to_string);
    assert_eq!(color(&h).as_deref(), Some("#123456"));
    common::click(&mut h, "Pathway color");
    assert_eq!(color(&h), None);
}
