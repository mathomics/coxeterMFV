//! The map's floating menus through the real interface: the Add Reaction
//! panel of a metabolite (`metabolite-menu.js`) and of a compartment
//! (`compartment-menu.js`), the boundary menu (`boundary-menu.js`) and Add
//! metabolite (`metabolite-add-menu.js`), with the panel helpers of
//! `add-reaction-panel.js`.
//!
//! Ports the Add Reaction panel tests of `test/ui-redesign.test.mjs` the state
//! agent left for the app crate (see the header of
//! `crates/coxeter-state/tests/js_ui_redesign.rs`), and the
//! `primaryMetabolitesForSide` assertion of `test/uploaded-model-map.test.mjs`.
//! Where the JavaScript drives a controller over a plain-object store, these
//! drive the window over the bundled first scenario (four species, E. coli
//! core among them), with the reaction taken off the map first so that the
//! menu has it to offer.

mod common;

use coxeter_app::Shell;
use coxeter_app::panels::add_reaction::{
    self, PanelDrag, PanelPosition, Placement, REACTION_DIRECTIONS, Side, begin_panel_drag, panel_drag_move, panel_placement,
    primary_metabolites_for_side, reaction_stoichiometry,
};
use coxeter_app::panels::menus::available_for_boundary;
use coxeter_canvas::camera::Camera;
use coxeter_canvas::gestures::Stage;
use coxeter_canvas::targets;
use coxeter_canvas::{CanvasEvent, MapMenu, MenuRequest, Tool};
use coxeter_core::mapdata::Point;
use egui::accesskit::Role;
use egui::{Event, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

type H = Harness<'static, Shell>;

// Helpers

fn direction_label(label: &str) -> String {
    format!("Add selected reaction to the {label}")
}

/// The first metabolite node drawn for a merged metabolite.
fn node_of(h: &H, merged_id: &str) -> (String, Point) {
    let map = h.state().store.map.as_ref().expect("a map");
    let node = map
        .nodes()
        .values()
        .find(|n| n.merged_id.as_deref() == Some(merged_id) && n.kind.as_str() == "metabolite")
        .unwrap_or_else(|| panic!("no node drawn for {merged_id}"));
    (node.id.clone(), Point::new(node.x, node.y))
}

/// Take reactions off the map, so the menus have them to offer.
fn undraw(h: &mut H, reaction_ids: &[&str]) {
    for id in reaction_ids {
        assert!(h.state_mut().store.remove_map_reaction(id).is_some(), "{id} was drawn");
    }
    common::settle(h);
}

/// The stage's box and middle, in client coordinates.
fn stage(h: &H) -> egui::Rect {
    h.state().map_rect.expect("the map was drawn")
}

/// Where the map draws a map point.
fn screen_of(h: &H, p: Point) -> Point {
    h.state().map.canvas.map_to_screen(p)
}

/// Open a menu as the map widget asks for it (`CanvasEvent::OpenMenu`).
fn open(h: &mut H, menu: MenuRequest, at: Point) {
    h.state_mut().on_canvas_event(CanvasEvent::OpenMenu { menu, at });
    common::settle(h);
}

/// The cell and compartment of the map an internal reaction of E. coli's
/// cytosol is offered in.
fn compartment_offering(h: &H, reaction_id: &str) -> (String, String) {
    let store = &h.state().store;
    for cell in store.map.as_ref().unwrap().cells() {
        for comp in &cell.compartments {
            if store.reactions_available_in_compartment(&cell.id, &comp.id).iter().any(|r| r == reaction_id) {
                return (cell.id.clone(), comp.id.clone());
            }
        }
    }
    panic!("no compartment offers {reaction_id}");
}

/// A client point well inside a compartment of the map.
fn inside_compartment(h: &H, cell_id: &str, compartment_id: &str) -> Point {
    let map = h.state().store.map.as_ref().unwrap();
    let comp = map.cell(cell_id).unwrap().compartments.iter().find(|c| c.id == compartment_id).unwrap();
    let r = comp.rect;
    screen_of(h, Point::new(r.x + r.w * 0.3, r.y + r.h * 0.3))
}

/// The controls whose accessible name starts with `prefix` and a space (a
/// row of a menu list reads its identifier first).
fn rows<'h>(h: &'h H, prefix: &'h str) -> Vec<egui_kittest::Node<'h>> {
    h.query_all_by(move |n| n.role() == Role::Button && n.label().is_some_and(|l| l == prefix || l.starts_with(&format!("{prefix} ")))).collect()
}

fn row_label(h: &H, prefix: &str) -> String {
    let found = rows(h, prefix);
    assert_eq!(found.len(), 1, "rows starting {prefix:?}");
    found[0].accesskit_node().label().unwrap_or_default()
}

fn click_row(h: &mut H, prefix: &str) {
    let found = rows(h, prefix);
    assert_eq!(found.len(), 1, "rows starting {prefix:?}");
    found[0].click_accesskit();
    common::settle(h);
}

/// Empty a text field (select all, delete, Enter).
fn clear_field(h: &mut H, label: &str) {
    common::control(h, label).focus();
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.key_press(egui::Key::Backspace);
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    common::settle(h);
}

fn current(h: &H) -> Option<MapMenu> {
    h.state().menus.current()
}

fn pointer_press(h: &mut H, at: Pos2) {
    h.event(Event::PointerMoved(at));
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
}

fn pointer_move(h: &mut H, at: Pos2) {
    h.event(Event::PointerMoved(at));
    h.step();
}

fn pointer_release(h: &mut H, at: Pos2) {
    h.event(Event::PointerMoved(at));
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.step();
}

fn pos(p: Point) -> Pos2 {
    Pos2::new(p.x as f32, p.y as f32)
}

/// Put a map point in the middle of the stage, at a close zoom.
fn look_at(h: &mut H, p: Point, scale: f64) {
    let r = stage(h);
    let camera = Camera { x: r.width() as f64 / 2.0 - p.x * scale, y: r.height() as f64 / 2.0 - p.y * scale, scale, fitted: true };
    h.state_mut().map.set_camera(camera);
    common::settle(h);
}

fn segment_nodes(h: &H, reaction_id: &str) -> Vec<String> {
    let map = h.state().store.map.as_ref().unwrap();
    map.segments_of(reaction_id).iter().filter_map(|s| s.node_id.clone()).collect()
}

// The panel's helpers

/// JS: 'the Add Reaction panel is never taller than the stage, so its
/// direction wheel is always in reach' (its `panelPlacement` half).
#[test]
fn panel_placement_keeps_the_panel_on_the_stage() {
    // A laptop: the stage is shorter than the panel at its full height.
    let (w, h) = (820.0, 430.0);
    let place = panel_placement(w, h, Some(&PanelPosition { left: 600.0, top: 380.0, moved: false }));
    assert!(place.max_height <= h - 16.0, "it is cut to the stage");
    assert!(place.top >= 8.0 && place.top + place.max_height <= h - 8.0, "and wholly on it");
    assert!(place.left >= 8.0 && place.left + place.width <= w - 8.0);
    // A roomy stage changes nothing: it opens beside the click at its full height.
    assert_eq!(
        panel_placement(1400.0, 900.0, Some(&PanelPosition { left: 212.0, top: 172.0, moved: false })),
        Placement { width: 380.0, max_height: 520.0, left: 212.0, top: 172.0 }
    );
}

/// JS: 'the Add Reaction panel can be dragged by its header and stays where
/// it is put' (the `beginPanelDrag` half).
#[test]
fn panel_drag_follows_the_pointer_within_reach() {
    let (w, h) = (900.0, 500.0);
    let mut state = PanelPosition { left: 100.0, top: 60.0, moved: false };
    let placed = Placement { width: 380.0, max_height: 484.0, left: 100.0, top: 60.0 };
    let press = Point::new(300.0, 200.0);
    // A press on a control in the header is that control's, not a drag.
    assert_eq!(begin_panel_drag(true, true, true, true, press, &placed), None);
    // Nor is a press anywhere when no panel of this controller is open.
    assert_eq!(begin_panel_drag(false, true, false, true, press, &placed), None);
    let from = begin_panel_drag(true, true, false, true, press, &placed).expect("a drag");
    assert_eq!(from, PanelDrag { x: 300.0, y: 200.0, left: 100.0, top: 60.0 });

    panel_drag_move(w, h, &from, Point::new(420.0, 330.0), &mut state);
    assert_eq!((state.left, state.top, state.moved), (220.0, 190.0, true));
    // It may be pushed part of the way off the stage, but never out of reach.
    panel_drag_move(w, h, &from, Point::new(5000.0, 5000.0), &mut state);
    assert!(state.left <= w - 64.0 && state.top <= h - 40.0);
    panel_drag_move(w, h, &from, Point::new(-5000.0, -5000.0), &mut state);
    assert!(state.left + 380.0 >= 64.0 && state.top >= 0.0);
    panel_drag_move(w, h, &from, Point::new(380.0, 260.0), &mut state);
    // The next repaint draws it where it was left, not back beside the click.
    let place = panel_placement(w, h, Some(&state));
    assert_eq!((place.left, place.top), (180.0, 120.0));
}

/// JS (uploaded-model-map): 'the automatic layout labels cofactors without
/// the model name too' — Add Reaction leaves cofactors out of the
/// metabolites to draw towards.
#[test]
fn primary_metabolites_leave_cofactors_out() {
    let h = common::harness(None);
    let community = h.state().store.community.clone().expect("a community");
    let offered: Vec<String> = primary_metabolites_for_side(&h.state().store, "eco@PFK", Side::Product).into_iter().map(|t| t.label).collect();
    // Named as the map names them (80e0611): E. coli's metabolites by their base.
    assert_eq!(offered, vec!["fdp".to_string()]);
    let substrates: Vec<String> = primary_metabolites_for_side(&h.state().store, "eco@PFK", Side::Substrate).into_iter().map(|t| t.label).collect();
    assert_eq!(substrates, vec!["f6p".to_string()]);
    // The compact equation: identifiers, `2 x` for a coefficient, `+ …` past three terms.
    let pfk = reaction_stoichiometry(&h.state().store, "eco@PFK", 3);
    assert!(pfk.starts_with("atp + f6p → ") || pfk.starts_with("f6p + atp → "), "{pfk}");
    let biomass = community.merged.reactions.keys().find(|k| k.contains("BIOMASS") && k.starts_with("eco@")).cloned().expect("biomass");
    let terms = reaction_stoichiometry(&h.state().store, &biomass, 3);
    assert!(terms.contains(" + …"), "{terms}");
    assert!(terms.split(' ').any(|t| t.parse::<f64>().is_ok()), "coefficients print before their metabolite: {terms}");
    assert_eq!(reaction_stoichiometry(&h.state().store, "eco@NOT_A_REACTION", 3), "—");
}

// Metabolite menu

/// JS: 'Add Reaction panel shows reaction stoichiometry before eight
/// in-panel directions and reuses the selected metabolite'.
#[test]
fn metabolite_menu_shows_stoichiometry_then_eight_directions_and_reuses_the_metabolite() {
    let mut h = common::harness(None);
    undraw(&mut h, &["eco@G6PDH2r"]);
    let (node_id, node_at) = node_of(&h, "eco@g6p_c");
    assert_eq!(h.state().store.reactions_available_from_metabolite(&node_id), vec!["eco@G6PDH2r".to_string()]);

    let r = stage(&h);
    open(&mut h, MenuRequest::Metabolite { node_id: node_id.clone() }, Point::new(r.min.x as f64 + 200.0, r.min.y as f64 + 160.0));
    assert_eq!(current(&h), Some(MapMenu::Metabolite));
    // `left: clientX - rect.left + 12`.
    let menu = h.state().menus.state.clone().unwrap();
    assert_eq!((menu.position.left, menu.position.top), (212.0, 172.0));
    assert!(common::shows(&h, "Add reaction"));
    assert!(common::shows(&h, "selected metabolite will be reused"));
    assert!(common::shows(&h, "1 reaction available"));

    // The reaction, with its compact stoichiometry, is listed and — the only
    // one — already chosen.
    let equation = reaction_stoichiometry(&h.state().store, "eco@G6PDH2r", 3);
    // In the map's labels (80e0611).
    assert!(equation.starts_with("g6p + nadp ") && equation.contains("6pgl"), "{equation}");
    let label = row_label(&h, "G6PDH2r");
    assert!(label.contains(&equation), "the row shows {equation:?}: {label:?}");
    assert_eq!(rows(&h, "G6PDH2r")[0].accesskit_node().toggled(), Some(egui::accesskit::Toggled::True));

    // Eight directions, ready since a reaction is chosen.
    for d in &REACTION_DIRECTIONS {
        assert!(common::has(&h, &direction_label(d.label)), "{}", d.label);
        assert!(!common::disabled(&h, &direction_label(d.label)), "{}", d.label);
    }
    // The terminal metabolite: the one primary product.
    assert!(common::shows(&h, "Primary product on the reaction axis."));

    common::click(&mut h, &direction_label("East"));
    assert_eq!(current(&h), None, "placing the reaction closes the panel");
    let map = h.state().store.map.as_ref().unwrap();
    let rxn = map.reaction("eco@G6PDH2r").expect("the reaction is on the map");
    assert_eq!((rxn.x, rxn.y), (node_at.x + 96.0, node_at.y), "placed 96 units east of the metabolite");
    assert_eq!(rxn.placement_direction, Some(serde_json::json!({ "x": 1.0, "y": 0.0 })));
    assert!(segment_nodes(&h, "eco@G6PDH2r").contains(&node_id), "it starts from the clicked metabolite");
    assert!(h.state().store.can_undo_map(), "one undoable edit");
}

/// A press with the Add Reaction tool on a drawn metabolite opens its menu
/// (the real gesture: `finishReactionPick`), and presses on the panel are the
/// panel's, not the map's.
#[test]
fn add_reaction_tool_press_on_a_metabolite_opens_its_menu() {
    let mut h = common::harness(None);
    undraw(&mut h, &["eco@G6PDH2r", "eco@PGI"]);
    let (node_id, node_at) = node_of(&h, "eco@g6p_c");
    common::click(&mut h, "Add Reaction");
    assert_eq!(h.state().map.tool(), Tool::Reaction);
    look_at(&mut h, node_at, 2.0);

    let at = pos(screen_of(&h, node_at));
    pointer_press(&mut h, at);
    pointer_release(&mut h, at);
    common::settle(&mut h);
    let menu = h.state().menus.state.clone().expect("a menu opened");
    assert_eq!(menu.request, MenuRequest::Metabolite { node_id: node_id.clone() });
    assert_eq!((menu.position.left, menu.position.top), (at.x as f64 - stage(&h).min.x as f64 + 12.0, at.y as f64 - stage(&h).min.y as f64 + 12.0));
    let selected = h.state().store.selected_entity.clone().expect("the metabolite is selected");
    assert_eq!((selected.kind.as_str(), selected.id.as_str()), ("metabolite", node_id.as_str()));
    assert!(common::shows(&h, "2 reactions available"));
    assert!(common::disabled(&h, &direction_label("North")), "no reaction chosen yet");
    assert!(common::shows(&h, "Select a reaction first."));

    // A real click on a row picks it; the map underneath does not take it.
    let row = rows(&h, "PGI")[0].rect().center();
    pointer_press(&mut h, row);
    pointer_release(&mut h, row);
    common::settle(&mut h);
    assert_eq!(h.state().menus.state.as_ref().and_then(|m| m.selected_reaction_id.clone()).as_deref(), Some("eco@PGI"));
    assert_eq!(h.state().store.selected_entity.as_ref().map(|e| e.id.clone()), Some(node_id.clone()), "the selection stays");
    assert!(!common::disabled(&h, &direction_label("North")));

    // And a real click on a direction places it.
    let north = common::control(&h, &direction_label("North")).rect().center();
    pointer_press(&mut h, north);
    pointer_release(&mut h, north);
    common::settle(&mut h);
    assert_eq!(current(&h), None);
    let map = h.state().store.map.as_ref().unwrap();
    let rxn = map.reaction("eco@PGI").expect("PGI is drawn again");
    assert!((rxn.y - (node_at.y - 96.0)).abs() < 1e-9 && (rxn.x - node_at.x).abs() < 1e-9, "96 units north: {} {}", rxn.x, rxn.y);
    assert!(segment_nodes(&h, "eco@PGI").contains(&node_id));
}

/// Filtering the list (`filterReactionButtons`): only matching rows remain,
/// and the summary counts them.
#[test]
fn the_filter_narrows_the_list() {
    let mut h = common::harness(None);
    undraw(&mut h, &["eco@G6PDH2r", "eco@PGI"]);
    let (node_id, _) = node_of(&h, "eco@g6p_c");
    let r = stage(&h);
    open(&mut h, MenuRequest::Metabolite { node_id }, Point::new(r.min.x as f64 + 100.0, r.min.y as f64 + 100.0));
    assert!(common::shows(&h, "2 reactions available"));
    assert_eq!(rows(&h, "PGI").len(), 1);
    assert_eq!(rows(&h, "G6PDH2r").len(), 1);

    // By name, case and accents aside.
    common::type_into(&mut h, "Filter reactions", "DEHYDROGENASE");
    assert!(common::shows(&h, "1 of 2 reactions"));
    assert_eq!(rows(&h, "PGI").len(), 0);
    assert_eq!(rows(&h, "G6PDH2r").len(), 1);
    // By a metabolite of the equation.
    common::type_into(&mut h, "Filter reactions", "f6p");
    assert_eq!(rows(&h, "PGI").len(), 1);
    assert_eq!(rows(&h, "G6PDH2r").len(), 0);
    // Choosing a row keeps the filter.
    click_row(&mut h, "PGI");
    assert!(common::shows(&h, "1 of 2 reactions"));
    clear_field(&mut h, "Filter reactions");
    assert!(common::shows(&h, "2 reactions available"));
    assert_eq!(rows(&h, "G6PDH2r").len(), 1);
}

// Compartment menu

/// JS: 'compartment Add Reaction panel uses the same reaction preview and
/// directions without linking an existing metabolite'.
#[test]
fn compartment_menu_places_at_the_clicked_point_without_linking_a_metabolite() {
    let mut h = common::harness(None);
    let before: std::collections::HashSet<String> = h.state().store.map.as_ref().unwrap().nodes().keys().cloned().collect();
    undraw(&mut h, &["eco@PGI", "eco@TPI"]);
    let (cell_id, compartment_id) = compartment_offering(&h, "eco@PGI");
    let at = inside_compartment(&h, &cell_id, &compartment_id);
    open(&mut h, MenuRequest::Compartment { cell_id: cell_id.clone(), compartment_id: compartment_id.clone() }, at);
    assert_eq!(current(&h), Some(MapMenu::Compartment));
    let origin = h.state().menus.state.as_ref().and_then(|m| m.origin).expect("the clicked map point");
    let expected = h.state().map.canvas.screen_to_map(at);
    assert!((origin.x - expected.x).abs() < 1e-9 && (origin.y - expected.y).abs() < 1e-9);

    assert!(common::shows(&h, "Add reaction"));
    assert!(common::shows(&h, "all metabolites will be new visual instances"));
    let equation = reaction_stoichiometry(&h.state().store, "eco@PGI", 3);
    assert!(row_label(&h, "PGI").contains(&equation));
    assert_eq!(REACTION_DIRECTIONS.iter().filter(|d| common::has(&h, &direction_label(d.label))).count(), 8);
    assert!(common::disabled(&h, &direction_label("East")), "two reactions: none chosen yet");

    click_row(&mut h, "PGI");
    assert_eq!(rows(&h, "PGI")[0].accesskit_node().toggled(), Some(egui::accesskit::Toggled::True));
    common::click(&mut h, &direction_label("East"));
    assert_eq!(current(&h), None);
    let map = h.state().store.map.as_ref().unwrap();
    let rxn = map.reaction("eco@PGI").expect("PGI is on the map");
    assert!((rxn.x - (origin.x + 96.0)).abs() < 1e-9 && (rxn.y - origin.y).abs() < 1e-9, "96 units east of the click");
    // Intentionally no anchor node: every metabolite is a new visual instance.
    let nodes = segment_nodes(&h, "eco@PGI");
    assert!(!nodes.is_empty());
    assert!(nodes.iter().all(|n| !before.contains(n)), "no existing metabolite reused: {nodes:?}");
}

/// The terminal metabolite: rotated with ‹ ›, and passed to the placement.
#[test]
fn the_terminal_metabolite_is_chosen_and_placed_on_the_axis() {
    let mut h = common::harness(None);
    undraw(&mut h, &["eco@TKT1"]);
    let (cell_id, compartment_id) = compartment_offering(&h, "eco@TKT1");
    let at = inside_compartment(&h, &cell_id, &compartment_id);
    open(&mut h, MenuRequest::Compartment { cell_id, compartment_id }, at);
    let terminals = primary_metabolites_for_side(&h.state().store, "eco@TKT1", Side::Product);
    assert_eq!(terminals.len(), 2, "{terminals:?}");
    assert!(common::shows(&h, "Choose which primary product lies on the reaction axis."));
    assert!(common::shows(&h, &terminals[0].label));

    common::click(&mut h, "Next terminal metabolite");
    assert_eq!(h.state().menus.state.as_ref().and_then(|m| m.terminal_metabolite_id.clone()), Some(terminals[1].id.clone()));
    common::click(&mut h, "Next terminal metabolite");
    assert_eq!(h.state().menus.state.as_ref().and_then(|m| m.terminal_metabolite_id.clone()), Some(terminals[0].id.clone()), "it wraps");
    common::click(&mut h, "Previous terminal metabolite");
    assert_eq!(h.state().menus.state.as_ref().and_then(|m| m.terminal_metabolite_id.clone()), Some(terminals[1].id.clone()));

    common::click(&mut h, &direction_label("South-west"));
    let map = h.state().store.map.as_ref().unwrap();
    let rxn = map.reaction("eco@TKT1").expect("TKT1 is on the map");
    assert_eq!(rxn.terminal_metabolite_id, Some(serde_json::Value::String(terminals[1].id.clone())));
    // The direction, made a unit vector by the placement.
    let sw = add_reaction::direction_by_id("sw").unwrap();
    let placed = rxn.placement_direction.clone().expect("a placement direction");
    let (x, y) = (placed["x"].as_f64().unwrap(), placed["y"].as_f64().unwrap());
    assert!((x - sw.dx).abs() < 1e-12 && (y - sw.dy).abs() < 1e-12, "{placed}");
}

/// A point of a compartment where a press finds nothing but the
/// compartment's own space (no metabolite, reaction, membrane or label).
fn free_space_in(h: &H, cell_id: &str, compartment_id: &str) -> Pos2 {
    let s = h.state();
    let canvas = &s.map.canvas;
    let stage_box = stage(h);
    let map = s.store.map.as_ref().unwrap();
    let r = map.cell(cell_id).unwrap().compartments.iter().find(|c| c.id == compartment_id).unwrap().rect;
    let mut camera = canvas.camera;
    let stage = Stage { camera: &mut camera, rect: canvas.rect, scene: canvas.scene.as_ref() };
    let filter = canvas.gestures.hit_filter();
    let others = [
        &targets::ANY_RXN,
        &targets::ANY_NODE,
        &targets::BOUNDARY,
        &targets::CELL_PLATE,
        &targets::CROSSFEED_ROUTE,
        &targets::MAP_TEXT,
        &targets::METABOLITE_LABEL,
        &targets::COMPARTMENT_LABEL,
        &targets::GEOMETRY_HANDLE_ANY,
    ];
    for i in 1..24 {
        for j in 1..24 {
            let p = Point::new(r.x + r.w * i as f64 / 24.0, r.y + r.h * j as f64 / 24.0);
            let at = pos(canvas.map_to_screen(p));
            if !stage_box.shrink(40.0).contains(at) {
                continue;
            }
            let client = Point::new(at.x as f64, at.y as f64);
            let t = stage.target_at(client, filter);
            let Some(space) = t.closest(&targets::COMPARTMENT_SPACE) else { continue };
            let ids = (targets::data(space, "data-cell-space"), targets::data(space, "data-compartment-space"));
            if ids != (Some(cell_id.to_string()), Some(compartment_id.to_string())) {
                continue;
            }
            if others.iter().any(|sel| t.closest(sel).is_some()) {
                continue;
            }
            return at;
        }
    }
    panic!("no free space in {cell_id}/{compartment_id}");
}

/// A press with Add Reaction on free space in a compartment opens its menu
/// at that point, and with Add Metabolite opens Add metabolite there (the
/// real gesture, the stage's `pointerup`).
#[test]
fn tool_presses_on_free_compartment_space_open_the_compartment_menus() {
    let mut h = common::harness(None);
    undraw(&mut h, &["eco@PGI"]);
    let (cell_id, compartment_id) = compartment_offering(&h, "eco@PGI");
    common::click(&mut h, "Add Reaction");
    let at = free_space_in(&h, &cell_id, &compartment_id);
    // Where the press lands on the map (selecting the compartment then opens
    // the inspector, and the stage's new width moves the camera).
    let expected = h.state().map.canvas.screen_to_map(Point::new(at.x as f64, at.y as f64));
    pointer_press(&mut h, at);
    pointer_release(&mut h, at);
    common::settle(&mut h);
    let menu = h.state().menus.state.clone().expect("a menu opened");
    assert_eq!(menu.request, MenuRequest::Compartment { cell_id: cell_id.clone(), compartment_id: compartment_id.clone() });
    assert_eq!(menu.origin, Some(expected), "the clicked map point");
    let selected = h.state().store.selected_entity.clone().expect("the compartment is selected");
    assert_eq!((selected.kind.as_str(), selected.id.as_str()), ("compartment", compartment_id.as_str()));
    assert!(common::has(&h, "Close Add Reaction panel"));

    // Add Metabolite on the same spot: its own menu, the other one closed.
    common::click(&mut h, "Add Metabolite");
    assert_eq!(current(&h), None, "choosing a tool closes the menu");
    let at = free_space_in(&h, &cell_id, &compartment_id);
    let expected = h.state().map.canvas.screen_to_map(Point::new(at.x as f64, at.y as f64));
    pointer_press(&mut h, at);
    pointer_release(&mut h, at);
    common::settle(&mut h);
    let menu = h.state().menus.state.clone().expect("Add metabolite opened");
    assert_eq!(menu.request, MenuRequest::MetaboliteAdd { cell_id: cell_id.clone(), compartment_id: compartment_id.clone() });
    assert!(!common::has(&h, "Close Add Reaction panel"));
    let before = h.state().store.map.as_ref().unwrap().nodes_for_metabolite("eco@f6p_c").len();
    common::type_into(&mut h, "Search metabolites", "f6p");
    let row = rows(&h, "f6p")[0].rect().center();
    pointer_press(&mut h, row);
    pointer_release(&mut h, row);
    common::settle(&mut h);
    assert_eq!(current(&h), None);
    let map = h.state().store.map.as_ref().unwrap();
    let nodes = map.nodes_for_metabolite("eco@f6p_c");
    assert_eq!(nodes.len(), before + 1);
    assert!(nodes.iter().any(|n| n.x == expected.x && n.y == expected.y), "placed where the compartment was pressed");
}

// Boundary menu

/// The membrane's transport reactions not drawn yet; one click places one.
#[test]
fn boundary_menu_lists_undrawn_transporters_and_places_one() {
    let mut h = common::harness(None);
    // A membrane of E. coli with a transporter on it.
    let (boundary_id, reaction_id) = {
        let store = &h.state().store;
        let community = store.community.clone().unwrap();
        let map = store.map.as_ref().unwrap();
        map.boundaries()
            .iter()
            .filter(|b| b.cell_id.as_deref() == Some("eco"))
            .find_map(|b| {
                coxeter_core::boundaries::reactions_crossing(&community, &b.id).into_iter().find(|r| map.has_reaction(r)).map(|r| (b.id.clone(), r))
            })
            .expect("a drawn transporter")
    };
    // With every transporter drawn there is nothing to offer: no menu.
    let r = stage(&h);
    let at = Point::new(r.center().x as f64, r.center().y as f64);
    if available_for_boundary(&h.state().store, &boundary_id).is_empty() {
        open(&mut h, MenuRequest::Boundary { boundary_id: boundary_id.clone() }, at);
        assert_eq!(current(&h), None, "an empty menu would only hide the map");
    }
    let before = available_for_boundary(&h.state().store, &boundary_id).len();
    undraw(&mut h, &[&reaction_id]);
    let offered = available_for_boundary(&h.state().store, &boundary_id);
    assert_eq!(offered.len(), before + 1);
    open(&mut h, MenuRequest::Boundary { boundary_id: boundary_id.clone() }, at);
    assert_eq!(current(&h), Some(MapMenu::Boundary));
    let summary = if offered.len() == 1 { "1 reaction available".to_string() } else { format!("{} reactions available", offered.len()) };
    assert!(common::shows(&h, &summary));
    let local = h.state().store.community.as_ref().unwrap().reaction_owner.get(&reaction_id).unwrap().original_id.clone();
    assert!(h.state().menus.state.as_ref().and_then(|m| m.origin).is_some(), "the press is the anchor");

    click_row(&mut h, &local);
    assert_eq!(current(&h), None);
    let map = h.state().store.map.as_ref().unwrap();
    let rxn = map.reaction(&reaction_id).expect("the transporter is on the map");
    assert_eq!(rxn.boundary_id.as_deref(), Some(boundary_id.as_str()), "on the membrane it was offered for");
    assert!(rxn.on_membrane);
}

// Add metabolite

/// Add metabolite: the compartment's metabolites, filtered, one placed at
/// the clicked point.
#[test]
fn add_metabolite_places_a_node_at_the_clicked_point() {
    let mut h = common::harness(None);
    let (cell_id, compartment_id) = {
        let store = &h.state().store;
        let map = store.map.as_ref().unwrap();
        let cell = map.cell("eco").expect("E. coli's cell");
        let comp = cell
            .compartments
            .iter()
            .find(|c| store.metabolites_available_in_compartment("eco", &c.id).iter().any(|m| m.id == "eco@g6p_c"))
            .expect("the cytosol");
        (cell.id.clone(), comp.id.clone())
    };
    let at = inside_compartment(&h, &cell_id, &compartment_id);
    let count_before = h.state().store.map.as_ref().unwrap().nodes_for_metabolite("eco@g6p_c").len();
    open(&mut h, MenuRequest::MetaboliteAdd { cell_id: cell_id.clone(), compartment_id: compartment_id.clone() }, at);
    assert_eq!(current(&h), Some(MapMenu::MetaboliteAdd));
    assert!(common::shows(&h, "Add metabolite"));
    assert!(common::shows(&h, "new visual instance"));
    assert!(common::has(&h, "Search metabolites"));
    let origin = h.state().map.canvas.screen_to_map(at);

    common::type_into(&mut h, "Search metabolites", "g6p");
    // Listed in the map's labels (80e0611).
    assert_eq!(rows(&h, "f6p").len(), 0, "the filter hides the rest");
    let label = row_label(&h, "g6p");
    assert!(label.ends_with(&format!("on map ({count_before})")), "{label}");
    click_row(&mut h, "g6p");
    assert_eq!(current(&h), None);
    let map = h.state().store.map.as_ref().unwrap();
    let nodes = map.nodes_for_metabolite("eco@g6p_c");
    assert_eq!(nodes.len(), count_before + 1, "a new instance");
    let added = nodes.iter().find(|n| (n.x - origin.x).abs() < 1e-9 && (n.y - origin.y).abs() < 1e-9);
    assert!(added.is_some(), "placed where the compartment was clicked");
}

// Closing

/// × closes, Escape closes, and opening another menu closes the one open.
#[test]
fn menus_close_by_button_escape_and_each_other() {
    let mut h = common::harness(None);
    undraw(&mut h, &["eco@G6PDH2r"]);
    let (node_id, _) = node_of(&h, "eco@g6p_c");
    let r = stage(&h);
    let at = Point::new(r.min.x as f64 + 150.0, r.min.y as f64 + 120.0);

    open(&mut h, MenuRequest::Metabolite { node_id: node_id.clone() }, at);
    assert_eq!(current(&h), Some(MapMenu::Metabolite));
    common::click(&mut h, "Close Add Reaction panel");
    assert_eq!(current(&h), None);
    assert!(!common::has(&h, "Close Add Reaction panel"), "and it is gone");

    open(&mut h, MenuRequest::Metabolite { node_id: node_id.clone() }, at);
    h.key_press(egui::Key::Escape);
    common::settle(&mut h);
    assert_eq!(current(&h), None, "Escape closes it");

    // Escape while typing in the filter closes it too.
    open(&mut h, MenuRequest::Metabolite { node_id: node_id.clone() }, at);
    common::control(&h, "Filter reactions").focus();
    h.run_steps(1);
    h.key_press(egui::Key::Escape);
    common::settle(&mut h);
    assert_eq!(current(&h), None);

    // Opening another menu replaces the open one.
    open(&mut h, MenuRequest::Metabolite { node_id: node_id.clone() }, at);
    open(&mut h, MenuRequest::MetaboliteAdd { cell_id: "eco".into(), compartment_id: "c".into() }, at);
    assert_eq!(current(&h), Some(MapMenu::MetaboliteAdd));
    assert!(!common::has(&h, "Close Add Reaction panel"));
    common::click(&mut h, "Close Add metabolite panel");
    assert_eq!(current(&h), None);

    // The map asks a menu it did not open to close: the open one stays.
    open(&mut h, MenuRequest::Metabolite { node_id }, at);
    h.state_mut().on_canvas_event(CanvasEvent::CloseMenu(MapMenu::Boundary));
    common::settle(&mut h);
    assert_eq!(current(&h), Some(MapMenu::Metabolite));
    h.state_mut().on_canvas_event(CanvasEvent::CloseMenu(MapMenu::Metabolite));
    common::settle(&mut h);
    assert_eq!(current(&h), None);
}

/// Choosing another tool closes the menus (`activateTool` → `closeMapMenus`).
#[test]
fn choosing_a_tool_closes_the_menu() {
    let mut h = common::harness(None);
    undraw(&mut h, &["eco@G6PDH2r"]);
    let (node_id, _) = node_of(&h, "eco@g6p_c");
    let r = stage(&h);
    open(&mut h, MenuRequest::Metabolite { node_id }, Point::new(r.min.x as f64 + 150.0, r.min.y as f64 + 120.0));
    assert!(h.state().menus.is_open());
    common::click(&mut h, "Pan Mode");
    assert_eq!(current(&h), None);
}

// Size and dragging

/// JS: 'the Add Reaction panel is never taller than the stage, so its
/// direction wheel is always in reach' — in a short window, opened low on
/// the stage, the whole wheel is on it.
#[test]
fn on_a_short_stage_the_direction_wheel_stays_in_reach() {
    let shell = common::shell(None);
    let mut h = Harness::builder().with_size(egui::vec2(1100.0, 560.0)).with_max_steps(40).build_eframe(move |_cc| shell);
    common::settle(&mut h);
    undraw(&mut h, &["eco@PGI", "eco@TPI", "eco@FBA", "eco@ENO", "eco@PGM"]);
    let (cell_id, compartment_id) = compartment_offering(&h, "eco@PGI");
    let r = stage(&h);
    assert!(r.height() < 520.0, "a stage shorter than the panel: {}", r.height());
    // A press near the bottom right corner.
    open(&mut h, MenuRequest::Compartment { cell_id, compartment_id }, Point::new(r.max.x as f64 - 20.0, r.max.y as f64 - 20.0));
    let offered = h.state().menus.state.as_ref().map(|m| m.request.clone()).and_then(|r| match r {
        MenuRequest::Compartment { cell_id, compartment_id } => Some(h.state().store.reactions_available_in_compartment(&cell_id, &compartment_id).len()),
        _ => None,
    });
    assert!(offered.is_some_and(|n| n >= 5));
    assert!(common::shows(&h, &format!("{} reactions available", offered.unwrap())));
    click_row(&mut h, "PGI");
    for d in &REACTION_DIRECTIONS {
        let rect = common::control(&h, &direction_label(d.label)).rect();
        assert!(r.contains_rect(rect), "{} at {rect:?} is on the stage {r:?}", d.label);
    }
    let close = common::control(&h, "Close Add Reaction panel").rect();
    assert!(r.contains_rect(close));
    common::click(&mut h, &direction_label("West"));
    assert!(h.state().store.map.as_ref().unwrap().has_reaction("eco@PGI"));
}

/// JS: 'the Add Reaction panel can be dragged by its header and stays where
/// it is put' — through the pointer, and a release far off the panel does
/// not reach the map.
#[test]
fn the_panel_is_dragged_by_its_header_and_stays_where_it_is_put() {
    let mut h = common::harness(None);
    undraw(&mut h, &["eco@G6PDH2r"]);
    let (node_id, _) = node_of(&h, "eco@g6p_c");
    let r = stage(&h);
    open(&mut h, MenuRequest::Metabolite { node_id: node_id.clone() }, Point::new(r.min.x as f64 + 100.0, r.min.y as f64 + 60.0));
    let start = h.state().menus.state.as_ref().unwrap().position;
    assert_eq!((start.left, start.top, start.moved), (112.0, 72.0, false));

    // A press on the header, left of the close button, moved by (120, 90).
    let close = common::control(&h, "Close Add Reaction panel").rect();
    let grab = Pos2::new(close.min.x - 80.0, close.center().y);
    pointer_press(&mut h, grab);
    for i in 1..=6 {
        pointer_move(&mut h, grab + egui::vec2(20.0 * i as f32, 15.0 * i as f32));
    }
    pointer_release(&mut h, grab + egui::vec2(120.0, 90.0));
    common::settle(&mut h);
    let moved = h.state().menus.state.as_ref().expect("still open").position;
    assert!(moved.moved);
    assert!((moved.left - (start.left + 120.0)).abs() < 1.0 && (moved.top - (start.top + 90.0)).abs() < 1.0, "{moved:?}");
    let close_after = common::control(&h, "Close Add Reaction panel").rect();
    assert!((close_after.min.x - close.min.x - 120.0).abs() < 1.5 && (close_after.min.y - close.min.y - 90.0).abs() < 1.5, "drawn where it was put");

    // Flung far past the stage's edge: kept within reach, and the release —
    // off the panel, over the map — closes nothing and selects nothing.
    let selected = h.state().store.selected_entity.clone();
    let grab = Pos2::new(close_after.min.x - 80.0, close_after.center().y);
    pointer_press(&mut h, grab);
    pointer_move(&mut h, grab + egui::vec2(10.0, 10.0));
    pointer_move(&mut h, Pos2::new(r.max.x + 400.0, r.max.y - 5.0));
    pointer_release(&mut h, Pos2::new(r.center().x, r.max.y - 5.0));
    common::settle(&mut h);
    let flung = h.state().menus.state.as_ref().expect("the release did not close it").position;
    assert!(flung.left <= r.width() as f64 - 64.0 && flung.top <= r.height() as f64 - 40.0, "{flung:?}");
    assert_eq!(h.state().store.selected_entity, selected, "the map did not take the release as a click");

    // A press on the close button is the button's, not a drag.
    let close = common::control(&h, "Close Add Reaction panel").rect();
    pointer_press(&mut h, close.center());
    pointer_release(&mut h, close.center());
    common::settle(&mut h);
    assert_eq!(current(&h), None);
}

// Screenshots

/// The menus as drawn (only with `COXETER_SHOT_DIR` set: renders through wgpu).
#[test]
fn screenshots_of_the_menus() {
    if std::env::var("COXETER_SHOT_DIR").is_err() {
        return;
    }
    let mut h = common::rendering_harness_of(common::shell(None));
    undraw(&mut h, &["eco@G6PDH2r", "eco@PGI", "eco@TKT1"]);
    let (node_id, _) = node_of(&h, "eco@g6p_c");
    let r = stage(&h);
    open(&mut h, MenuRequest::Metabolite { node_id }, Point::new(r.min.x as f64 + 220.0, r.min.y as f64 + 80.0));
    click_row(&mut h, "G6PDH2r");
    common::shot(&mut h, "menu-metabolite");
    let (cell_id, compartment_id) = compartment_offering(&h, "eco@TKT1");
    open(&mut h, MenuRequest::Compartment { cell_id: cell_id.clone(), compartment_id: compartment_id.clone() }, Point::new(r.min.x as f64 + 500.0, r.min.y as f64 + 60.0));
    click_row(&mut h, "TKT1");
    common::shot(&mut h, "menu-compartment");
    open(&mut h, MenuRequest::MetaboliteAdd { cell_id, compartment_id }, Point::new(r.min.x as f64 + 300.0, r.min.y as f64 + 60.0));
    common::shot(&mut h, "menu-add-metabolite");
    h.state_mut().set_theme(true);
    common::settle(&mut h);
    common::shot(&mut h, "menu-add-metabolite-dark");
}

// add-menu-labels

/// add-menu-labels: the Add Reaction panel's rows and equations, and the
/// membrane menu's rows (`reactionListing`), in the map's labels.
#[test]
fn the_add_menus_name_reactions_and_metabolites_as_the_map_does() {
    use coxeter_app::panels::add_reaction::{reaction_listing, reaction_rows};
    let mut shell = common::shell(None);
    let doc = serde_json::json!({
        "id": "ymini",
        "compartments": { "e": "extracellular", "c": "cytoplasm" },
        "metabolites": [
            { "id": "s_0563", "name": "D-glucose", "compartment": "e", "annotation": { "bigg.metabolite": "glc__D" } },
            { "id": "s_0565", "name": "D-glucose", "compartment": "c", "annotation": { "bigg.metabolite": "glc__D" } },
            { "id": "s_0434", "name": "ATP", "compartment": "c", "annotation": { "bigg.metabolite": "atp" } },
            { "id": "s_0394", "name": "ADP", "compartment": "c", "annotation": { "bigg.metabolite": "adp" } },
            { "id": "s_0568", "name": "D-glucose 6-phosphate", "compartment": "c", "annotation": { "bigg.metabolite": "g6p" } }
        ],
        "reactions": [
            { "id": "r_1714", "name": "D-glucose exchange", "metabolites": { "s_0563": -1 }, "lower_bound": -10, "upper_bound": 1000 },
            { "id": "r_1166", "name": "glucose transport", "metabolites": { "s_0563": -1, "s_0565": 1 }, "lower_bound": 0, "upper_bound": 1000, "annotation": { "bigg.reaction": "GLCt1" } },
            { "id": "r_0534", "name": "hexokinase", "metabolites": { "s_0565": -1, "s_0434": -1, "s_0568": 1, "s_0394": 1 }, "lower_bound": 0, "upper_bound": 1000, "annotation": { "bigg.reaction": "HEX1" } },
            { "id": "r_bio", "name": "growth", "metabolites": { "s_0568": -1 }, "lower_bound": 0, "upper_bound": 1000, "objective_coefficient": 1 }
        ]
    });
    shell.store.set_model("ymini", coxeter_core::model::Model::from_value(&doc).unwrap());
    shell.store.scenarios.push(serde_json::json!({
        "id": "ymini", "name": "Yeast in miniature", "objective": "equal-growth",
        "species": [{ "id": "sce", "model": "ymini", "abundance": 1 }], "medium": { "s_0563": 10 }
    }));
    shell.store.use_scenario("ymini").unwrap();
    let store = &mut shell.store;
    let hex = store.community.as_ref().unwrap().reaction_owner.iter().find(|(_, o)| o.original_id == "r_0534").unwrap().0.clone();
    // By default the panel still lists the model id with the name beside it.
    assert_eq!(reaction_listing(store, &hex), ("r_0534".to_string(), "hexokinase".to_string()));

    store.set_map_label_source("metabolite", "name", None);
    let row = reaction_rows(store, std::slice::from_ref(&hex)).remove(0);
    assert_eq!((row.label.as_str(), row.detail.as_str()), ("r_0534", "hexokinase"), "reactions keep their own label setting");
    assert!(row.equation.contains("D-glucose + ATP → D-glucose 6-phosphate + ADP"), "{}", row.equation);
    assert!(!row.equation.contains("s_0434"));
    let terminals = primary_metabolites_for_side(store, &hex, Side::Product);
    assert!(terminals.iter().any(|t| t.label == "D-glucose 6-phosphate"), "the terminal metabolite too");

    store.set_map_label_source("reaction", "name", None);
    assert_eq!(reaction_listing(store, &hex), ("hexokinase".to_string(), "r_0534".to_string()), "labelled by name, the id goes beside it");

    store.set_map_label_source("metabolite", "bigg", None);
    store.set_map_label_source("reaction", "bigg", None);
    let row = reaction_rows(store, std::slice::from_ref(&hex)).remove(0);
    assert_eq!((row.label.as_str(), row.detail.as_str()), ("HEX1", "hexokinase"));
    assert!(row.equation.contains("glc__D + atp → g6p + adp"), "{}", row.equation);
    // The filter matches the label, the id and the name.
    assert!(row.search.contains("hex1") && row.search.contains("r_0534") && row.search.contains("hexokinase"), "{}", row.search);

    // The membrane menu: transport reactions by the map's label.
    store.set_map_label_source("reaction", "name", None);
    let transport = store.community.as_ref().unwrap().reaction_owner.iter().find(|(_, o)| o.original_id == "r_1166").unwrap().0.clone();
    assert_eq!(reaction_listing(store, &transport), ("glucose transport".to_string(), "r_1166".to_string()));
}
