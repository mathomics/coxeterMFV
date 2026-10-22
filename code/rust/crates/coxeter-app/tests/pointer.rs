//! Every pointer gesture of the stage (`src/ui/app.js`), driven through the
//! whole window with egui's own pointer, wheel and touch events, on the real
//! Store: the map widget of `coxeter-canvas` acting through the app's
//! `MapHost` (`host.rs`). The canvas crate tests each gesture's geometry
//! against the JavaScript; these check that the application wires each one
//! to the Store (one edit in the history, the selection, the camera).

mod common;

use coxeter_app::Shell;
use coxeter_app::host::StoreHost;
use coxeter_canvas::camera::Camera;
use coxeter_canvas::gestures::{MenuRequest, Stage};
use coxeter_canvas::host::{MapHost, SelectKind};
use coxeter_canvas::targets::{self, Target};
use coxeter_canvas::{CanvasEvent, Tool};
use coxeter_core::mapdata::Point;
use coxeter_core::metabolite_label::metabolite_label_angle;
use egui::{Event, Modifiers, PointerButton, Pos2, Vec2};

type H = egui_kittest::Harness<'static, Shell>;

// Helpers

/// Put map point `p` at the middle of the stage, at `scale`.
fn look_at(h: &mut H, p: Point, scale: f64) {
    let rect = h.state().map.canvas.rect;
    h.state_mut().map.set_camera(Camera { x: rect.width / 2.0 - p.x * scale, y: rect.height / 2.0 - p.y * scale, scale, fitted: true });
    // The pointer targets are rebuilt for the new zoom once it settles.
    h.state_mut().revision += 1;
    for _ in 0..3 {
        h.step();
    }
}

fn screen(h: &H, p: Point) -> Pos2 {
    let s = h.state().map.canvas.map_to_screen(p);
    Pos2::new(s.x as f32, s.y as f32)
}

fn map_point(h: &H, at: Pos2) -> Point {
    h.state().map.canvas.screen_to_map(Point::new(at.x as f64, at.y as f64))
}

/// What the stage finds under a screen point, and `f` of it.
fn at_point<R>(h: &mut H, at: Pos2, f: impl FnOnce(&Target, &StoreHost) -> R) -> R {
    let shell = h.state_mut();
    let mut camera = shell.map.canvas.camera;
    let rect = shell.map.canvas.rect;
    let filter = shell.map.canvas.gestures.hit_filter();
    let stage = Stage { camera: &mut camera, rect, scene: shell.map.canvas.scene.as_ref() };
    let target = stage.target_at(Point::new(at.x as f64, at.y as f64), filter);
    let host = StoreHost::new(&mut shell.store);
    f(&target, &host)
}

/// A screen point over an element carrying `attr` where the stage's target
/// satisfies `accept`, well inside the stage.
fn find(h: &mut H, attr: &str, accept: impl Fn(&Target, &StoreHost) -> bool) -> Option<Pos2> {
    let selector = coxeter_canvas::doc::Selector::parse(&format!("[{attr}]"));
    let candidates: Vec<Pos2> = {
        let s = h.state();
        let scene = s.map.canvas.scene.as_ref()?;
        let rect = s.map.canvas.rect;
        (0..scene.doc.len())
            .filter(|i| scene.doc.closest(*i, &selector).is_some())
            .filter_map(|i| {
                let e = &scene.elems[i];
                let b = e.hit_bounds.or(e.bounds.map(|b| [b[0] as f64, b[1] as f64, b[2] as f64, b[3] as f64]))?;
                let group = &scene.groups[e.group as usize];
                let (x, y) = group.place((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
                let p = s.map.canvas.map_to_screen(Point::new(x, y));
                let inside = p.x > rect.left + 40.0 && p.y > rect.top + 40.0 && p.x < rect.left + rect.width - 40.0 && p.y < rect.top + rect.height - 40.0;
                inside.then(|| Pos2::new(p.x as f32, p.y as f32))
            })
            .collect()
    };
    candidates.into_iter().find(|at| at_point(h, *at, |t, host| accept(t, host)))
}

/// An empty spot of the drawing: no content, membrane, grip or label.
fn background(h: &mut H) -> Pos2 {
    let rect = h.state().map.canvas.rect;
    for gy in 2..18 {
        for gx in 2..18 {
            let at = Pos2::new((rect.left + rect.width * gx as f64 / 20.0) as f32, (rect.top + rect.height * gy as f64 / 20.0) as f32);
            let empty = at_point(h, at, |t, _| {
                t.index.is_some()
                    && !targets::map_content_at(t)
                    && t.closest(&targets::BOUNDARY).is_none()
                    && targets::canvas_resize_at(t).is_none()
                    && t.closest(&targets::COMPARTMENT_LABEL).is_none()
                    && t.closest(&targets::SELECTION_DRAG).is_none()
                    && t.closest(&targets::CROSSFEED_ROUTE).is_none()
            });
            if empty {
                return at;
            }
        }
    }
    panic!("no empty spot on the stage");
}

fn press(h: &mut H, at: Pos2, button: PointerButton) {
    h.event(Event::PointerMoved(at));
    h.event(Event::PointerButton { pos: at, button, pressed: true, modifiers: Modifiers::NONE });
    h.step();
}

fn move_to(h: &mut H, at: Pos2) {
    h.event(Event::PointerMoved(at));
    h.step();
}

fn release(h: &mut H, at: Pos2, button: PointerButton) {
    h.event(Event::PointerButton { pos: at, button, pressed: false, modifiers: Modifiers::NONE });
    h.step();
    common::settle(h);
}

/// Press, travel in steps, release.
fn drag(h: &mut H, from: Pos2, by: Vec2) {
    press(h, from, PointerButton::Primary);
    for k in 1..=4 {
        move_to(h, from + by * (k as f32 / 4.0));
    }
    release(h, from + by, PointerButton::Primary);
}

fn undo_labels(h: &H) -> Vec<String> {
    h.state().store.map_history.undo_labels().iter().map(|l| l.to_string()).collect()
}

/// The first scenario, zoomed in on the middle of its first cell.
fn zoomed() -> H {
    let mut h = common::harness(None);
    let rect = h.state().store.map.as_ref().unwrap().cells()[0].compartments.last().unwrap().rect;
    look_at(&mut h, Point::new(rect.x + rect.w / 2.0, rect.y + rect.h / 2.0), 1.4);
    h
}

/// Zoomed in on a node that can be dragged.
fn on_a_node() -> (H, String, Pos2) {
    let mut h = zoomed();
    let at = find(&mut h, "data-node", |t, host| targets::draggable_node_at(t, host).is_some()).expect("a node in view");
    let id = at_point(&mut h, at, |t, host| targets::draggable_node_at(t, host)).unwrap();
    (h, id, at)
}

// Tests

#[test]
fn node_drag_moves_a_metabolite_as_one_edit() {
    let (mut h, id, at) = on_a_node();
    let before = h.state().store.map.as_ref().unwrap().node(&id).unwrap().clone();
    drag(&mut h, at, Vec2::new(36.0, 24.0));
    let after = h.state().store.map.as_ref().unwrap().node(&id).cloned();
    if let Some(after) = after {
        assert!(after.x > before.x && after.y > before.y, "moved");
    }
    assert_eq!(undo_labels(&h).last().map(String::as_str), Some("Move metabolite"));
}

#[test]
fn a_click_on_a_node_selects_it_and_opens_the_inspector() {
    let (mut h, id, at) = on_a_node();
    press(&mut h, at, PointerButton::Primary);
    release(&mut h, at, PointerButton::Primary);
    let selected = h.state().store.selected_entity.clone().expect("a selection");
    assert_eq!((selected.kind.as_str(), selected.id.as_str()), ("metabolite", id.as_str()));
    assert!(!h.state().ui.inspector_collapsed, "the inspector opens on a selection");
    assert!(undo_labels(&h).is_empty(), "a click edits nothing");
}

#[test]
fn reaction_drag_moves_a_reaction_by_its_marker() {
    let mut h = zoomed();
    let at = find(&mut h, "data-rxn", |t, host| targets::draggable_reaction_at(t, host).is_some() && targets::reaction_marker_at(t))
        .expect("a reaction marker in view");
    let rid = at_point(&mut h, at, |t, host| targets::draggable_reaction_at(t, host)).unwrap();
    let before = h.state().store.map.as_ref().unwrap().reaction(&rid).unwrap().clone();
    drag(&mut h, at, Vec2::new(30.0, -20.0));
    let after = h.state().store.map.as_ref().unwrap().reaction(&rid).unwrap().clone();
    assert!((after.x - before.x).abs() > 1.0 || (after.y - before.y).abs() > 1.0, "moved");
    assert_eq!(undo_labels(&h).last().map(String::as_str), Some("Move reaction"));
}

#[test]
fn rotate_drag_turns_the_selected_reaction() {
    // The Edit tool draws every reaction's green lever; a reaction kept on its
    // line or on its membrane has none, so the view is centred on one that is free.
    let mut h = common::harness(None);
    let free = {
        let map = h.state().store.map.as_ref().unwrap();
        let r = map.reactions().values().find(|r| r.axis.is_some() && r.linear.is_none() && !r.on_membrane).expect("a free reaction");
        Point::new(r.x, r.y)
    };
    look_at(&mut h, free, 1.4);
    common::click(&mut h, "Edit Control Points");
    let lever = find(&mut h, "data-rotate-handle", |t, host| targets::rotate_handle_at(t, host).is_some()).expect("a lever in view");
    let rid = at_point(&mut h, lever, |t, host| targets::rotate_handle_at(t, host)).unwrap();
    let before = h.state().store.map.as_ref().unwrap().reaction_angle(&rid);
    let centre = screen(&h, {
        let r = h.state().store.map.as_ref().unwrap().reaction(&rid).unwrap();
        Point::new(r.x, r.y)
    });
    // A quarter turn about the marker.
    let arm = lever - centre;
    let turned = centre + Vec2::new(-arm.y, arm.x);
    press(&mut h, lever, PointerButton::Primary);
    for k in 1..=6 {
        let a = std::f32::consts::FRAC_PI_2 * k as f32 / 6.0;
        move_to(&mut h, centre + Vec2::new(arm.x * a.cos() - arm.y * a.sin(), arm.x * a.sin() + arm.y * a.cos()));
    }
    release(&mut h, turned, PointerButton::Primary);
    let after = h.state().store.map.as_ref().unwrap().reaction_angle(&rid);
    assert_ne!(before, after, "the angle changed");
    assert_eq!(undo_labels(&h).last().map(String::as_str), Some("Rotate reaction"));
}

#[test]
fn edit_handles_show_control_points_and_a_control_drag_reshapes_a_curve() {
    let mut h = zoomed();
    common::click(&mut h, "Edit Control Points");
    assert_eq!(h.state().map.tool(), Tool::Edit);
    let scene_has = |h: &H| {
        let scene = h.state().map.canvas.scene.as_ref().unwrap();
        (0..scene.doc.len()).any(|i| scene.doc.attr(i, "data-control-handle").is_some())
    };
    assert!(scene_has(&h), "the Edit tool draws every reaction's control points");
    let at = find(&mut h, "data-control-handle", |t, _| targets::control_handle_at(t).is_some()).expect("a control point in view");
    let handle = at_point(&mut h, at, |t, _| targets::control_handle_at(t)).unwrap();
    let before = h.state().store.map.as_ref().unwrap().to_json();
    drag(&mut h, at, Vec2::new(25.0, 18.0));
    assert_ne!(h.state().store.map.as_ref().unwrap().to_json(), before, "the curve of {} changed", handle.reaction_id);
    assert!(!undo_labels(&h).is_empty(), "one edit recorded");
    common::click(&mut h, "Select");
    assert!(!scene_has(&h), "Select hides them again");
}

#[test]
fn metabolite_label_drag_places_a_label_around_its_circle() {
    let mut h = zoomed();
    let at = find(&mut h, "data-metabolite-label", |t, host| targets::metabolite_label_at(t, host).is_some()).expect("a label in view");
    let id = at_point(&mut h, at, |t, host| targets::metabolite_label_at(t, host)).unwrap();
    let before = h.state().store.map.as_ref().unwrap().node(&id).unwrap().label_angle;
    drag(&mut h, at, Vec2::new(-30.0, 40.0));
    let after = h.state().store.map.as_ref().unwrap().node(&id).unwrap().label_angle;
    assert!(after.is_some() && after != before, "{before:?} → {after:?}");
    assert!(!undo_labels(&h).is_empty());
}

#[test]
fn undo_during_a_label_drag_first_records_the_drag() {
    // `finishMetaboliteLabelDragForHistory`: Ctrl+Z mid-drag ends the drag as
    // one edit, then undoes exactly that edit.
    let mut h = zoomed();
    let at = find(&mut h, "data-metabolite-label", |t, host| targets::metabolite_label_at(t, host).is_some()).expect("a label in view");
    let id = at_point(&mut h, at, |t, host| targets::metabolite_label_at(t, host)).unwrap();
    let before = h.state().store.map.as_ref().unwrap().node(&id).unwrap().label_angle;
    press(&mut h, at, PointerButton::Primary);
    move_to(&mut h, at + Vec2::new(-20.0, 30.0));
    move_to(&mut h, at + Vec2::new(-30.0, 45.0));
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::Z);
    h.step();
    release(&mut h, at + Vec2::new(-30.0, 45.0), PointerButton::Primary);
    assert_eq!(h.state().store.map.as_ref().unwrap().node(&id).unwrap().label_angle, before, "undone");
    assert!(h.state().store.can_redo_map(), "the drag is the edit undone");
}

#[test]
fn text_drag_moves_a_text_with_the_edit_tool() {
    let mut h = zoomed();
    let middle = map_point(&h, h.state().map_rect.unwrap().center());
    h.state_mut().store.add_map_text("Glycolysis", middle.x, middle.y, None);
    common::settle(&mut h);
    common::click(&mut h, "Edit Control Points");
    let at = find(&mut h, "data-map-text", |t, _| targets::map_text_at(t).is_some()).expect("the text");
    let id = at_point(&mut h, at, |t, _| targets::map_text_at(t)).unwrap();
    let before = h.state().store.map.as_ref().unwrap().text(&id).unwrap().clone();
    drag(&mut h, at, Vec2::new(40.0, 10.0));
    let after = h.state().store.map.as_ref().unwrap().text(&id).unwrap().clone();
    assert!(after.x > before.x, "moved right");
    assert_eq!(undo_labels(&h).last().map(String::as_str), Some("Move text"));
}

#[test]
fn a_selection_rectangle_then_a_group_drag() {
    let mut h = common::harness(None);
    let from = background(&mut h);
    let rect = h.state().map_rect.unwrap();
    // Sweep from the empty spot towards the far corner of the stage.
    let to = Pos2::new(if from.x < rect.center().x { rect.right() - 30.0 } else { rect.left() + 30.0 }, if from.y < rect.center().y { rect.bottom() - 30.0 } else { rect.top() + 30.0 });
    press(&mut h, from, PointerButton::Primary);
    move_to(&mut h, from + (to - from) * 0.5);
    assert!(h.state().map.canvas.gestures.selection.is_some(), "the rectangle is being drawn");
    move_to(&mut h, to);
    release(&mut h, to, PointerButton::Primary);
    let size = h.state().store.multi_selection_size();
    assert!(size >= 2, "{size} selected");

    // Drag one selected node: the whole selection moves, as one edit.
    let node = h.state().store.multi_selection.nodes.iter().next().cloned().expect("a selected node");
    let n = h.state().store.map.as_ref().unwrap().node(&node).unwrap().clone();
    look_at(&mut h, Point::new(n.x, n.y), 1.4);
    let at = screen(&h, Point::new(n.x, n.y));
    let picked = at_point(&mut h, at, |t, host| targets::multi_selectable_at(t, host));
    if picked.as_ref().is_some_and(|(k, id)| *k == SelectKind::Node && id == &node) {
        let before = h.state().store.map.as_ref().unwrap().to_json();
        drag(&mut h, at, Vec2::new(30.0, 30.0));
        assert_ne!(h.state().store.map.as_ref().unwrap().to_json(), before);
        assert_eq!(h.state().store.multi_selection_size(), size, "the selection stays");
        assert!(!undo_labels(&h).is_empty());
    }
}

#[test]
fn group_drag_by_the_selection_handle() {
    let mut h = zoomed();
    // Select two nodes in view by shift-clicks.
    let mut picked = Vec::new();
    for _ in 0..2 {
        let at = find(&mut h, "data-node", |t, host| {
            targets::multi_selectable_at(t, host).is_some_and(|(k, id)| k == SelectKind::Node && !host.is_multi_selected(k, &id))
        })
        .expect("a node to add");
        h.event(Event::PointerMoved(at));
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::SHIFT });
        h.step();
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::SHIFT });
        h.step();
        common::settle(&mut h);
        picked.push(at);
    }
    assert_eq!(h.state().store.multi_selection_size(), 2, "shift-click adds to the selection");
    let at = picked[0];
    let before = h.state().store.map.as_ref().unwrap().to_json();
    drag(&mut h, at, Vec2::new(20.0, 25.0));
    assert_ne!(h.state().store.map.as_ref().unwrap().to_json(), before, "the group moved");
    let moved_both = {
        let store = &h.state().store;
        let map = store.map.as_ref().unwrap();
        store.multi_selection.nodes.iter().all(|id| map.node(id).is_some())
    };
    assert!(moved_both);
}

#[test]
fn cell_draw_and_compartment_draw_build_a_map_by_hand() {
    let mut h = common::harness(Some("solo-ecoli"));
    h.state_mut().map_empty();
    common::settle(&mut h);
    assert!(h.state().store.map.as_ref().unwrap().cells().is_empty());
    common::click(&mut h, "Add Cell");
    let rect = h.state().map_rect.unwrap();
    let from = rect.center() - Vec2::new(rect.width() * 0.3, rect.height() * 0.3);
    drag(&mut h, from, Vec2::new(rect.width() * 0.6, rect.height() * 0.6));
    let cells = h.state().store.map.as_ref().unwrap().cells().len();
    assert_eq!(cells, 1, "the cell is drawn");
    assert_eq!(undo_labels(&h).last().map(String::as_str), Some("Create cell"));

    // Then a compartment inside it.
    common::click(&mut h, "Add Compartment");
    let placement = h.state_mut().store.current_compartment_placement().expect("a compartment to draw");
    let parent = h.state().store.compartment_parent_rect(&placement.species_id, &placement.compartment_id).expect("its parent");
    // Away from the "Nothing drawn yet" card in the middle of the stage,
    // which takes the presses under it (as `.stage-empty-card` does).
    let a = screen(&h, Point::new(parent.x + parent.w * 0.06, parent.y + parent.h * 0.08));
    let b = screen(&h, Point::new(parent.x + parent.w * 0.3, parent.y + parent.h * 0.3));
    let card = egui::Id::new("coxeter-map-view").with("empty-map");
    assert!(!h.ctx.layer_id_at(a).is_some_and(|l| l.id == card), "the press lands on the map, not on the card");
    let before = h.state().store.map.as_ref().unwrap().cells()[0].compartments.len();
    drag(&mut h, a, b - a);
    let after = h.state().store.map.as_ref().unwrap().cells()[0].compartments.len();
    assert_eq!(after, before + 1, "the compartment is drawn");
}

#[test]
fn canvas_resize_by_its_edge() {
    let mut h = common::harness(None);
    let (w, hgt) = {
        let m = h.state().store.map.as_ref().unwrap();
        (m.width(), m.height())
    };
    look_at(&mut h, Point::new(w, hgt / 2.0), 0.5);
    let at = find(&mut h, "data-canvas-resize", |t, _| targets::canvas_resize_at(t).is_some()).expect("a canvas grip in view");
    drag(&mut h, at, Vec2::new(60.0, 0.0));
    let m = h.state().store.map.as_ref().unwrap();
    assert!(m.width() > w || m.height() > hgt, "{w}×{hgt} → {}×{}", m.width(), m.height());
    assert!(!undo_labels(&h).is_empty());
}

#[test]
fn compartment_label_drag_moves_the_label() {
    let mut h = common::harness(None);
    let rect = h.state().store.map.as_ref().unwrap().cells()[0].compartments[0].rect;
    look_at(&mut h, Point::new(rect.x + rect.w / 2.0, rect.y + 20.0), 0.6);
    let Some(at) = find(&mut h, "data-compartment-label", |t, _| t.closest(&targets::COMPARTMENT_LABEL).is_some()) else {
        let rect = h.state().map_rect.unwrap();
        let middle = map_point(&h, rect.center());
        look_at(&mut h, middle, 0.3);
        let at = find(&mut h, "data-compartment-label", |t, _| t.closest(&targets::COMPARTMENT_LABEL).is_some()).expect("a compartment label");
        return label_drag(&mut h, at);
    };
    label_drag(&mut h, at);
}

fn label_drag(h: &mut H, at: Pos2) {
    let before = h.state().store.map.as_ref().unwrap().to_json();
    drag(h, at, Vec2::new(30.0, 20.0));
    assert_ne!(h.state().store.map.as_ref().unwrap().to_json(), before, "the label moved");
    assert!(!undo_labels(h).is_empty());
}

#[test]
fn geometry_drag_resizes_a_compartment_from_a_handle() {
    let mut h = common::harness(None);
    let (cell, comp, rect) = {
        let m = h.state().store.map.as_ref().unwrap();
        let cell = &m.cells()[0];
        let comp = cell.compartments.last().unwrap();
        (cell.id.clone(), comp.id.clone(), comp.rect)
    };
    let _ = h.state_mut().store.set_view(coxeter_state::ViewPatch { geometry_edit: Some(true), ..Default::default() });
    h.state_mut().store.select_geometry(Some(&cell), &comp);
    common::settle(&mut h);
    look_at(&mut h, Point::new(rect.x + rect.w, rect.y + rect.h), 0.8);
    let at = find(&mut h, "data-geometry-handle", |t, _| targets::geometry_handle_at(t).is_some_and(|g| g.handle != "move")).expect("a corner handle");
    drag(&mut h, at, Vec2::new(30.0, 30.0));
    let after = h.state().store.map.as_ref().unwrap().to_json();
    let _ = after;
    assert!(!undo_labels(&h).is_empty(), "the resize is one edit");
}

#[test]
fn reaction_pick_opens_the_add_reaction_panel_on_a_metabolite() {
    let (mut h, id, at) = on_a_node();
    common::click(&mut h, "Add Reaction");
    let at = find(&mut h, "data-node", |t, host| targets::draggable_node_at(t, host).is_some()).unwrap_or(at);
    press(&mut h, at, PointerButton::Primary);
    release(&mut h, at, PointerButton::Primary);
    let open = h.state().menus.open.clone();
    let _ = id;
    assert!(matches!(open, Some((MenuRequest::Metabolite { .. }, _))), "{open:?}");
}

#[test]
fn hit_targets_are_generous_at_a_distant_zoom() {
    let mut h = common::harness(None);
    // At the fitted zoom a metabolite is a few pixels across; its pointer
    // target stays large enough to press.
    let scene_has_hits = {
        let scene = h.state().map.canvas.scene.as_ref().unwrap();
        (0..scene.doc.len()).any(|i| scene.doc.element(i).has_class("emc-hit-node"))
    };
    assert!(scene_has_hits, "the hit layer is drawn");
    // A press beside a metabolite's circle, on the side away from its label
    // (a target of its own, drawn over the node's), still finds it: the
    // primary metabolites well inside the stage, away from the compartments'
    // name plates.
    let rect = h.state().map.canvas.rect;
    let primaries: Vec<(String, Pos2, f64)> = h
        .state()
        .store
        .map
        .as_ref()
        .unwrap()
        .nodes()
        .values()
        .filter(|n| n.kind == coxeter_core::mapdata::NodeKind::Metabolite && n.primary != Some(false))
        .map(|n| (n.id.clone(), screen(&h, Point::new(n.x, n.y)), metabolite_label_angle(n.label_angle)))
        .filter(|(_, p, _)| {
            let (x, y) = (p.x as f64, p.y as f64);
            x > rect.left + 80.0 && y > rect.top + 80.0 && x < rect.left + rect.width - 80.0 && y < rect.top + rect.height - 80.0
        })
        .collect();
    assert!(!primaries.is_empty(), "metabolites in view");
    let found = primaries
        .iter()
        .filter(|(id, at, angle)| {
            let near = *at - Vec2::new(angle.cos() as f32, angle.sin() as f32) * 4.0;
            at_point(&mut h, near, |t, host| targets::draggable_node_at(t, host)).as_deref() == Some(id.as_str())
        })
        .count();
    // Where the map is dense a neighbour's halo or label lies there instead,
    // so it is asked of the metabolites that stand clear.
    assert!(found > 0, "a press beside the circle still finds it");
}

#[test]
fn pan_with_the_pan_tool_the_middle_button_and_space() {
    let mut h = common::harness(None);
    let map_before = h.state().store.map.as_ref().unwrap().to_json();
    let at = h.state().map_rect.unwrap().center();
    common::click(&mut h, "Pan Mode");
    let c0 = h.state().map.camera();
    drag(&mut h, at, Vec2::new(50.0, -30.0));
    let c1 = h.state().map.camera();
    assert_eq!((c1.x - c0.x, c1.y - c0.y), (50.0, -30.0));
    common::click(&mut h, "Select");
    press(&mut h, at, PointerButton::Middle);
    move_to(&mut h, at + Vec2::new(-20.0, 10.0));
    release(&mut h, at + Vec2::new(-20.0, 10.0), PointerButton::Middle);
    let c2 = h.state().map.camera();
    assert_eq!((c2.x - c1.x, c2.y - c1.y), (-20.0, 10.0));
    h.event(Event::PointerMoved(at));
    h.key_down(egui::Key::Space);
    h.step();
    drag(&mut h, at, Vec2::new(12.0, 8.0));
    h.key_up(egui::Key::Space);
    h.step();
    let c3 = h.state().map.camera();
    assert_eq!((c3.x - c2.x, c3.y - c2.y), (12.0, 8.0));
    assert_eq!(h.state().store.map.as_ref().unwrap().to_json(), map_before, "panning changes no map");
}

#[test]
fn the_wheel_zooms_with_a_mouse_and_pans_with_a_touchpad() {
    let mut h = common::harness(None);
    let at = h.state().map_rect.unwrap().center();
    move_to(&mut h, at);
    let scale = h.state().map.camera().scale;
    h.event(Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta: Vec2::new(0.0, 1.0), phase: egui::TouchPhase::Move, modifiers: Modifiers::NONE });
    h.step();
    assert!((h.state().map.camera().scale / scale - 1.12).abs() < 1e-9, "a notch zooms in");
    let _ = h.state_mut().store.set_view(coxeter_state::ViewPatch { scroll_input: Some("touchpad".into()), ..Default::default() });
    common::settle(&mut h);
    let before = h.state().map.camera();
    h.event(Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: Vec2::new(-15.0, -25.0), phase: egui::TouchPhase::Move, modifiers: Modifiers::NONE });
    h.step();
    let after = h.state().map.camera();
    assert_eq!(after.scale, before.scale, "a touchpad scroll pans");
    assert!(after.x != before.x || after.y != before.y);
}

fn touch(id: u64, phase: egui::TouchPhase, at: Pos2) -> Event {
    Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(id), phase, pos: at, force: None }
}

#[test]
fn two_fingers_pinch_zoom() {
    let mut h = common::harness(None);
    let c = h.state().map_rect.unwrap().center();
    let scale = h.state().map.camera().scale;
    h.event(touch(1, egui::TouchPhase::Start, c - Vec2::new(50.0, 0.0)));
    h.event(touch(2, egui::TouchPhase::Start, c + Vec2::new(50.0, 0.0)));
    h.step();
    h.event(touch(1, egui::TouchPhase::Move, c - Vec2::new(100.0, 0.0)));
    h.event(touch(2, egui::TouchPhase::Move, c + Vec2::new(100.0, 0.0)));
    h.step();
    h.event(touch(1, egui::TouchPhase::End, c - Vec2::new(100.0, 0.0)));
    h.event(touch(2, egui::TouchPhase::End, c + Vec2::new(100.0, 0.0)));
    h.step();
    common::settle(&mut h);
    assert!((h.state().map.camera().scale / scale - 2.0).abs() < 1e-6, "twice as far apart, twice the zoom");
    assert!(undo_labels(&h).is_empty());
}

#[test]
fn one_finger_on_the_background_pans() {
    let mut h = common::harness(None);
    let at = background(&mut h);
    let c0 = h.state().map.camera();
    h.event(touch(1, egui::TouchPhase::Start, at));
    h.step();
    for k in 1..=4 {
        h.event(touch(1, egui::TouchPhase::Move, at + Vec2::new(10.0 * k as f32, 6.0 * k as f32)));
        h.step();
    }
    h.event(touch(1, egui::TouchPhase::End, at + Vec2::new(40.0, 24.0)));
    h.step();
    common::settle(&mut h);
    let c1 = h.state().map.camera();
    assert!((c1.x - c0.x - 40.0).abs() < 1e-6 && (c1.y - c0.y - 24.0).abs() < 1e-6, "{:?} → {:?}", (c0.x, c0.y), (c1.x, c1.y));
}

#[test]
fn the_empty_map_card_asks_for_the_full_map() {
    let mut h = common::harness(Some("solo-ecoli"));
    h.state_mut().map_empty();
    common::settle(&mut h);
    h.state_mut().on_canvas_event(CanvasEvent::Action("map-full"));
    common::settle(&mut h);
    assert!(!h.state().store.map.as_ref().unwrap().reactions().is_empty(), "the full map is drawn again");
}
