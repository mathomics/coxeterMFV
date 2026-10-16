//! The widget on egui, offscreen (`egui_kittest`): egui's own pointer, wheel,
//! touch and key events through [`MapView::show`], on the E. coli map of the
//! browser references, over an [`EditableMapHost`]. No window is opened.

mod common;

use coxeter_canvas::host::{MapHost, SelectKind};
use coxeter_canvas::targets;
use coxeter_canvas::test_host::EditableMapHost;
use coxeter_canvas::{CanvasEvent, MapView, Tool};
use coxeter_core::mapdata::Point;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use common::*;

struct App {
    view: MapView,
    host: EditableMapHost,
    events: Vec<CanvasEvent>,
}

fn app() -> Option<App> {
    let doc = gestures()?;
    let host = host_for(&doc, &json!({ "start": "initial" }));
    Some(App { view: MapView::new(), host, events: Vec::new() })
}

fn harness(app: App) -> Harness<'static, App> {
    let mut h = Harness::builder().with_size(Vec2::new(1100.0, 800.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut App| {
            let revision = app.host.emitted.len() as u64;
            let out = app.view.show(ui, &mut app.host, revision);
            app.events.extend(out.events);
        },
        app,
    );
    h.run_steps(3);
    // A closer look than the fitted one, at the middle of the cell.
    let s = h.state_mut();
    s.view.zoom_by(2.5);
    h.run_steps(2);
    h
}

fn pos(p: Point) -> Pos2 {
    Pos2::new(p.x as f32, p.y as f32)
}

/// A metabolite that is movable, shown, and found under its own centre.
fn grabbable_node(h: &Harness<'static, App>) -> (String, Point, Pos2) {
    let s = h.state();
    let rect = s.view.canvas.rect;
    let map = s.host.map().unwrap();
    for n in map.nodes().values() {
        if !n.is_metabolite_kind() || !s.host.can_move_node(&n.id) || n.kind.as_str() != "metabolite" {
            continue;
        }
        let at = s.view.canvas.map_to_screen(Point::new(n.x, n.y));
        if at.x < rect.left + 60.0 || at.y < rect.top + 60.0 || at.x > rect.left + rect.width - 60.0 || at.y > rect.top + rect.height - 60.0 {
            continue;
        }
        let mut cam = s.view.canvas.camera;
        let stage = coxeter_canvas::gestures::Stage { camera: &mut cam, rect, scene: s.view.canvas.scene.as_ref() };
        let target = stage.target_at(at, s.view.canvas.gestures.hit_filter());
        if targets::draggable_node_at(&target, &s.host).as_deref() == Some(n.id.as_str()) {
            return (n.id.clone(), Point::new(n.x, n.y), pos(at));
        }
    }
    panic!("no grabbable metabolite in view");
}

fn press(h: &mut Harness<'static, App>, at: Pos2, button: PointerButton) {
    h.event(Event::PointerMoved(at));
    h.event(Event::PointerButton { pos: at, button, pressed: true, modifiers: Modifiers::NONE });
    h.step();
}

fn move_to(h: &mut Harness<'static, App>, at: Pos2) {
    h.event(Event::PointerMoved(at));
    h.step();
}

fn release(h: &mut Harness<'static, App>, at: Pos2, button: PointerButton) {
    h.event(Event::PointerButton { pos: at, button, pressed: false, modifiers: Modifiers::NONE });
    h.step();
}

#[test]
fn dragging_a_metabolite_moves_it_and_records_one_edit() {
    let Some(a) = app() else { return };
    let mut h = harness(a);
    let (id, start, at) = grabbable_node(&h);
    let scale = h.state().view.camera().scale;
    press(&mut h, at, PointerButton::Primary);
    move_to(&mut h, at + Vec2::new(12.0, 6.0));
    move_to(&mut h, at + Vec2::new(40.0, 30.0));
    // While it is dragged the drawing marks it.
    {
        let s = h.state();
        let scene = s.view.canvas.scene.as_ref().unwrap();
        let marked = (0..scene.doc.len()).any(|i| {
            let e = scene.doc.element(i);
            e.has_class("is-dragging") && e.get("data-node") == Some(id.as_str())
        });
        assert!(marked, "the dragged circle carries is-dragging");
    }
    release(&mut h, at + Vec2::new(40.0, 30.0), PointerButton::Primary);
    h.run_steps(2);
    let s = h.state();
    let n = s.host.map().unwrap().node(&id).cloned();
    let labels = s.host.history.undo_labels().iter().map(|l| l.to_string()).collect::<Vec<_>>();
    // (Dropped on a copy of itself it would have been merged into it.)
    if let Some(n) = n {
        assert!((n.x - start.x - 40.0 / scale).abs() < 1.5 && (n.y - start.y - 30.0 / scale).abs() < 1.5, "moved to ({}, {})", n.x, n.y);
    }
    assert_eq!(labels, vec!["Move metabolite".to_string()]);
}

#[test]
fn a_wheel_notch_zooms_about_the_pointer() {
    let Some(a) = app() else { return };
    let mut h = harness(a);
    let at = Pos2::new(500.0, 400.0);
    move_to(&mut h, at);
    let (before, scale) = {
        let s = h.state();
        (s.view.canvas.screen_to_map(Point::new(at.x as f64, at.y as f64)), s.view.camera().scale)
    };
    // One notch up (a mouse wheel reports lines): the DOM's deltaY < 0, zoom in.
    h.event(Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta: Vec2::new(0.0, 1.0), phase: egui::TouchPhase::Move, modifiers: Modifiers::NONE });
    h.step();
    let s = h.state();
    let after = s.view.canvas.screen_to_map(Point::new(at.x as f64, at.y as f64));
    assert!((s.view.camera().scale / scale - 1.12).abs() < 1e-9);
    assert!((after.x - before.x).abs() < 1e-6 && (after.y - before.y).abs() < 1e-6);
}

#[test]
fn the_text_tool_writes_a_text_where_the_map_is_clicked() {
    let Some(a) = app() else { return };
    let mut h = harness(a);
    {
        let s = h.state_mut();
        s.view.activate_tool(Tool::Text, &mut s.host);
    }
    h.run_steps(2);
    let at = Pos2::new(600.0, 420.0);
    press(&mut h, at, PointerButton::Primary);
    release(&mut h, at, PointerButton::Primary);
    h.run_steps(3);
    assert!(h.state().events.iter().any(|e| matches!(e, CanvasEvent::TextEditorOpened { id: None })));
    h.event(Event::Text("Glycolysis".into()));
    h.step();
    h.key_press(Key::Enter);
    h.run_steps(3);
    let s = h.state();
    assert!(s.view.canvas.gestures.text_editor.is_none(), "Enter closes the editor");
    let texts: Vec<String> = s.host.map().unwrap().texts().values().map(|t| t.text.clone()).collect();
    assert!(texts.contains(&"Glycolysis".to_string()), "{texts:?}");
    assert_eq!(s.host.history.undo_labels(), vec!["Add text"]);
}

#[test]
fn escape_returns_to_select_and_delete_removes_the_selection() {
    let Some(a) = app() else { return };
    let mut h = harness(a);
    {
        let s = h.state_mut();
        s.view.activate_tool(Tool::Edit, &mut s.host);
    }
    move_to(&mut h, Pos2::new(500.0, 400.0));
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state().view.tool(), Tool::Select);
    assert!(h.state().events.contains(&CanvasEvent::ToolChanged(Tool::Select)));

    let rid = h.state().host.map().unwrap().reactions().keys().next().unwrap().clone();
    h.state_mut().host.select(Some(&rid));
    h.run_steps(1);
    h.key_press(Key::Delete);
    h.run_steps(2);
    let s = h.state();
    assert!(s.host.map().unwrap().reaction(&rid).is_none(), "Delete removed the selected reaction");
    assert_eq!(s.host.history.undo_labels(), vec!["Remove reaction"]);
}

#[test]
fn arrows_nudge_the_selected_metabolite() {
    let Some(a) = app() else { return };
    let mut h = harness(a);
    let (id, start, _) = grabbable_node(&h);
    h.state_mut().host.select_entity("metabolite", &id, None);
    move_to(&mut h, Pos2::new(500.0, 400.0));
    h.key_press_modifiers(Modifiers::SHIFT, Key::ArrowRight);
    h.key_press(Key::ArrowUp);
    h.run_steps(2);
    let s = h.state();
    let n = s.host.map().unwrap().node(&id).unwrap();
    assert_eq!((n.x, n.y), (start.x + 10.0, start.y - 1.0));
    assert_eq!(s.host.history.undo_labels(), vec!["Move metabolite", "Move metabolite"]);
}

#[test]
fn escape_cancels_a_drag_and_its_release_does_nothing() {
    let Some(a) = app() else { return };
    let mut h = harness(a);
    let (id, start, at) = grabbable_node(&h);
    press(&mut h, at, PointerButton::Primary);
    move_to(&mut h, at + Vec2::new(50.0, 35.0));
    assert_ne!(h.state().host.map().unwrap().node(&id).map(|n| (n.x, n.y)), Some((start.x, start.y)));
    h.key_press(Key::Escape);
    h.step();
    release(&mut h, at + Vec2::new(50.0, 35.0), PointerButton::Primary);
    h.run_steps(2);
    let s = h.state();
    let n = s.host.map().unwrap().node(&id).unwrap();
    assert_eq!((n.x, n.y), (start.x, start.y), "the node is back");
    assert!(s.host.history.undo_labels().is_empty(), "nothing recorded");
    assert!(s.host.selected_entity.is_none(), "the release selected nothing");
}

#[test]
fn space_and_the_middle_button_pan() {
    let Some(a) = app() else { return };
    let mut h = harness(a);
    let at = Pos2::new(500.0, 400.0);
    move_to(&mut h, at);
    let before = h.state().view.camera();
    let map_before = h.state().host.map().unwrap().to_json();
    h.key_down(Key::Space);
    h.step();
    press(&mut h, at, PointerButton::Primary);
    move_to(&mut h, at + Vec2::new(70.0, -25.0));
    release(&mut h, at + Vec2::new(70.0, -25.0), PointerButton::Primary);
    h.key_up(Key::Space);
    h.run_steps(2);
    let mid = h.state().view.camera();
    assert_eq!((mid.x - before.x, mid.y - before.y), (70.0, -25.0));
    press(&mut h, at, PointerButton::Middle);
    move_to(&mut h, at + Vec2::new(-30.0, 15.0));
    release(&mut h, at + Vec2::new(-30.0, 15.0), PointerButton::Middle);
    h.run_steps(2);
    let after = h.state().view.camera();
    assert_eq!((after.x - mid.x, after.y - mid.y), (-30.0, 15.0));
    assert_eq!(h.state().host.map().unwrap().to_json(), map_before, "panning changes no map");
}

#[test]
fn a_selection_rectangle_selects_what_it_covers() {
    let Some(a) = app() else { return };
    let mut h = harness(a);
    {
        let s = h.state_mut();
        s.view.set_camera(coxeter_canvas::camera::Camera { x: -12.0, y: 8.0, scale: 0.27, fitted: true });
    }
    h.run_steps(2);
    // From an empty spot near the top left to most of the stage.
    let from = Pos2::new(120.0, 110.0);
    press(&mut h, from, PointerButton::Primary);
    move_to(&mut h, Pos2::new(600.0, 450.0));
    assert!(h.state().view.canvas.gestures.selection.is_some(), "the rectangle is being drawn");
    move_to(&mut h, Pos2::new(900.0, 700.0));
    release(&mut h, Pos2::new(900.0, 700.0), PointerButton::Primary);
    h.run_steps(2);
    let s = h.state();
    assert!(s.host.multi_selection_size() > 10, "{} selected", s.host.multi_selection_size());
    assert!(s.host.is_multi_selected(SelectKind::Node, s.host.multi_selection.nodes.first().unwrap()));
}

/// An empty spot inside a compartment of the multi-selection: its space, and
/// nothing drawn there.
fn inside_selected_compartment(h: &Harness<'static, App>) -> Option<Pos2> {
    let s = h.state();
    let rect = s.view.canvas.rect;
    let filter = s.view.canvas.gestures.hit_filter();
    let mut cam = s.view.canvas.camera;
    let stage = coxeter_canvas::gestures::Stage { camera: &mut cam, rect, scene: s.view.canvas.scene.as_ref() };
    let (mut x, step) = (rect.left + 80.0, 17.0);
    while x < rect.left + rect.width - 80.0 {
        let mut y = rect.top + 80.0;
        while y < rect.top + rect.height - 80.0 {
            let target = stage.target_at(Point::new(x, y), filter);
            if !targets::map_content_at(&target)
                && let Some(space) = target.closest(&targets::COMPARTMENT_SPACE)
            {
                let key = (
                    targets::data(space, "data-cell-space").unwrap_or_default(),
                    targets::data(space, "data-compartment-space").unwrap_or_default(),
                );
                if s.host.multi_selection.compartments.contains(&key) {
                    return Some(Pos2::new(x as f32, y as f32));
                }
            }
            y += step;
        }
        x += step;
    }
    None
}

#[test]
fn pressing_inside_a_selected_compartment_drags_the_whole_selection_with_it() {
    let Some(a) = app() else { return };
    let mut h = harness(a);
    {
        let s = h.state_mut();
        s.view.set_camera(coxeter_canvas::camera::Camera { x: -12.0, y: 8.0, scale: 0.27, fitted: true });
    }
    h.run_steps(2);
    // Everything, cells and compartments included.
    {
        let s = h.state_mut();
        let env = s.host.map().unwrap().environment().rect;
        s.host.select_in_rect(env.x - 10.0, env.y - 10.0, env.x + env.w + 10.0, env.y + env.h + 10.0, true);
        assert!(!s.host.multi_selection.compartments.is_empty(), "a rectangle round everything takes the compartments");
    }
    h.run_steps(2);
    let at = inside_selected_compartment(&h).expect("an empty spot inside a selected compartment");
    let (cell, comp) = {
        let s = h.state();
        s.host.multi_selection.compartments.first().cloned().unwrap()
    };
    let rect_of = |h: &Harness<'static, App>| {
        let map = h.state().host.map().unwrap();
        map.cells().iter().find(|c| c.id == cell).unwrap().compartments.iter().find(|c| c.id == comp).unwrap().rect
    };
    let before = rect_of(&h);
    let nodes_before: Vec<(String, Point)> =
        h.state().host.map().unwrap().nodes().values().filter(|n| n.cell_id.as_deref() == Some(cell.as_str())).map(|n| (n.id.clone(), n.pos())).collect();

    press(&mut h, at, PointerButton::Primary);
    move_to(&mut h, at + Vec2::new(4.0, 3.0));
    assert!(h.state().view.canvas.gestures.group_drag.is_some(), "the press grabbed the selection, not a new rectangle");
    move_to(&mut h, at + Vec2::new(10.0, 8.0));
    release(&mut h, at + Vec2::new(10.0, 8.0), PointerButton::Primary);
    h.run_steps(2);

    let after = rect_of(&h);
    let (dx, dy) = (after.x - before.x, after.y - before.y);
    assert!(dx.abs() + dy.abs() > 1.0, "the compartment moved");
    // Everything in the cell went with it, by the same amount.
    let s = h.state();
    let map = s.host.map().unwrap();
    for (id, p) in nodes_before {
        let n = map.node(&id).unwrap();
        assert!((n.x - p.x - dx).abs() < 1e-6 && (n.y - p.y - dy).abs() < 1e-6, "{id} did not move with its cell");
    }
    assert!(s.host.multi_selection_size() > 10, "the selection is kept");
    let labels = s.host.history.undo_labels().iter().map(|l| l.to_string()).collect::<Vec<_>>();
    assert_eq!(labels, vec!["Move selection".to_string()]);
}

#[test]
fn two_fingers_pinch_zoom() {
    let Some(a) = app() else { return };
    let mut h = harness(a);
    let scale = h.state().view.camera().scale;
    let touch = |id: u64, phase, x: f32, y: f32| Event::Touch {
        device_id: egui::TouchDeviceId(1),
        id: egui::TouchId(id),
        phase,
        pos: Pos2::new(x, y),
        force: None,
    };
    h.event(touch(1, egui::TouchPhase::Start, 450.0, 400.0));
    h.event(touch(2, egui::TouchPhase::Start, 550.0, 400.0));
    h.step();
    h.event(touch(1, egui::TouchPhase::Move, 400.0, 400.0));
    h.event(touch(2, egui::TouchPhase::Move, 600.0, 400.0));
    h.step();
    h.event(touch(1, egui::TouchPhase::End, 400.0, 400.0));
    h.event(touch(2, egui::TouchPhase::End, 600.0, 400.0));
    h.run_steps(2);
    let s = h.state();
    assert!((s.view.camera().scale / scale - 2.0).abs() < 1e-9, "fingers twice as far apart, twice the zoom");
    assert!(s.host.history.undo_labels().is_empty());
}

#[test]
fn an_empty_map_shows_its_card_and_its_button_asks_for_the_full_map() {
    let Some(mut a) = app() else { return };
    a.host.map = Some(coxeter_map::EditableMap::blank(Some(1200.0), Some(800.0)));
    let mut h = harness(a);
    h.run_steps(2);
    h.get_by_label("Nothing drawn yet");
    h.get_by_label("Or draw the full map").click();
    h.run_steps(2);
    assert!(h.state().events.contains(&CanvasEvent::Action("map-full")));
}

/// A reaction's marker, found under its own centre.
fn clickable_reaction(h: &Harness<'static, App>) -> (String, Pos2) {
    let s = h.state();
    let rect = s.view.canvas.rect;
    for r in s.host.map().unwrap().reactions().values() {
        let at = s.view.canvas.map_to_screen(Point::new(r.x, r.y));
        if at.x < rect.left + 60.0 || at.y < rect.top + 60.0 || at.x > rect.left + rect.width - 60.0 || at.y > rect.top + rect.height - 60.0 {
            continue;
        }
        let mut cam = s.view.canvas.camera;
        let stage = coxeter_canvas::gestures::Stage { camera: &mut cam, rect, scene: s.view.canvas.scene.as_ref() };
        let target = stage.target_at(at, s.view.canvas.gestures.hit_filter());
        if targets::reaction_at(&target).as_deref() == Some(r.id.as_str()) && targets::reaction_marker_at(&target) {
            return (r.id.clone(), pos(at));
        }
    }
    panic!("no reaction marker in view");
}

#[test]
fn a_double_click_on_a_reaction_selects_it_for_its_bounds() {
    let Some(a) = app() else { return };
    let mut h = harness(a);
    let (id, at) = clickable_reaction(&h);
    for _ in 0..2 {
        press(&mut h, at, PointerButton::Primary);
        release(&mut h, at, PointerButton::Primary);
    }
    h.run_steps(2);
    let s = h.state();
    assert_eq!(s.host.selected_reaction.as_deref(), Some(id.as_str()));
    assert!(s.events.contains(&CanvasEvent::FocusLowerBound));
}

#[test]
fn tools_switch_the_view_and_drawing_tools_set_the_halos_aside() {
    let Some(mut a) = app() else { return };
    a.host.multi_selection.nodes.insert("x".into());
    let (view, host) = (&mut a.view, &mut a.host);
    view.activate_tool(Tool::Cell, host);
    assert!(host.view.geometry_edit && !host.view.text_tool && !host.view.multi_select);
    assert_eq!(host.multi_selection_size(), 0, "a tool change clears the multi-selection");
    assert!(view.canvas.gestures.hit_filter().drawing_tool);
    view.activate_tool(Tool::Text, host);
    assert!(host.view.text_tool && !host.view.geometry_edit);
    assert!(!view.canvas.gestures.hit_filter().drawing_tool);
    view.activate_tool(Tool::Pan, host);
    assert!(!host.view.text_tool && view.canvas.gestures.hit_filter().pan_tool);
}

#[test]
fn the_map_turns_sharp_once_still_and_back_when_it_moves() {
    let Some(app) = app() else { return };
    let mut h = harness(app);
    // Still: after the settle delay a raster of the view arrives (from a
    // thread of its own) and is shown.
    let mut shown = false;
    for _ in 0..400 {
        h.step();
        if h.state().view.crisp_shown() {
            shown = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(shown, "no sharp raster after the camera settled");
    // A pan hides it at once.
    h.state_mut().view.zoom_by(1.0);
    let mut cam = h.state().view.camera();
    cam.x += 30.0;
    h.state_mut().view.set_camera(cam);
    h.step();
    assert!(!h.state().view.crisp_shown(), "a raster of the old view still shown");
    // Switched off: never shown.
    h.state_mut().view.crisp = false;
    for _ in 0..30 {
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(!h.state().view.crisp_shown());
}
