//! The stage's side of the compartment move by multi-selection (e9438d7) and
//! its follow-ups in `src/ui/app.js`, on the E. coli map of the browser
//! references, through [`MapCanvas`] over an [`EditableMapHost`]:
//!
//! - the empty inside of a selected compartment grabs the group;
//! - "Select moves a text annotation, as it moves everything else on the map"
//!   (`test/ui-redesign.test.mjs`);
//! - "undo and redo wait for a drag in progress to finish" (same file);
//! - "Escape drops a selected compartment in Select as well as in Geometry
//!   Edit" (same file).
//!
//! The JavaScript tests read `app.js` as text; these press, drag and type.

mod common;

use coxeter_canvas::camera::StageRect;
use coxeter_canvas::gestures::{CanvasKey, KeyMods, PointerInput, PointerType, Stage};
use coxeter_canvas::host::{MapHost, TextStyle};
use coxeter_canvas::targets::{self, Target};
use coxeter_canvas::test_host::EditableMapHost;
use coxeter_canvas::{MapCanvas, Tool};
use coxeter_core::mapdata::Point;
use coxeter_map::multi_selection::text_center;
use serde_json::json;

use common::*;

struct Rig {
    canvas: MapCanvas,
    host: EditableMapHost,
    time: f64,
}

fn rig(tool: Tool) -> Option<Rig> {
    let doc = gestures()?;
    let mut host = host_for(&doc, &json!({ "start": "initial" }));
    let mut canvas = MapCanvas::new();
    canvas.set_rect(StageRect::new(0.0, 0.0, 1100.0, 800.0));
    canvas.activate_tool(tool, &mut host);
    canvas.take_events();
    host.history.clear();
    let mut r = Rig { canvas, host, time: 1000.0 };
    r.sync();
    r.canvas.fit(&r.host);
    r.sync();
    Some(r)
}

fn mouse(p: Point, button: i16) -> PointerInput {
    PointerInput { pointer_id: 1, pointer_type: PointerType::Mouse, button, client: p, ..Default::default() }
}

impl Rig {
    fn sync(&mut self) {
        let revision = self.host.emitted.len() as u64;
        self.canvas.sync(&self.host, revision, self.time);
        self.canvas.take_events();
    }

    fn target(&self, at: Point) -> Target {
        let stage = Stage { camera: &mut self.canvas.camera.clone(), rect: self.canvas.rect, scene: self.canvas.scene.as_ref() };
        stage.target_at(at, self.canvas.gestures.hit_filter())
    }

    fn down(&mut self, at: Point) {
        self.sync();
        self.canvas.pointer_move(&mut self.host, &mouse(at, -1), self.time);
        self.canvas.pointer_down(&mut self.host, &mouse(at, 0));
    }

    fn move_to(&mut self, at: Point) {
        self.time += 16.0;
        self.canvas.pointer_move(&mut self.host, &mouse(at, -1), self.time);
    }

    fn up(&mut self, at: Point) {
        self.time += 16.0;
        self.canvas.pointer_up(&mut self.host, &mouse(at, 0), self.time);
        self.sync();
    }

    fn key(&mut self, key: CanvasKey, command: bool) {
        self.canvas.key_down(&mut self.host, key, KeyMods { command, shift: false });
    }

    fn labels(&self) -> Vec<String> {
        self.host.history.undo_labels().iter().map(|l| l.to_string()).collect()
    }

    /// A free text in the middle of the stage, and where a press finds it.
    fn add_text(&mut self) -> (String, Point) {
        let middle = self.canvas.screen_to_map(Point::new(550.0, 400.0));
        assert!(self.host.add_map_text("A note", middle.x, middle.y, &TextStyle::default()));
        self.sync();
        let map = self.host.map().unwrap();
        let (id, text) = map.texts().iter().last().map(|(id, t)| (id.clone(), t.clone())).unwrap();
        let at = self.canvas.map_to_screen(text_center(&text));
        assert_eq!(targets::map_text_at(&self.target(at)).as_deref(), Some(id.as_str()), "the text is under its centre");
        (id, at)
    }
}

fn text_pos(r: &Rig, id: &str) -> Point {
    let t = r.host.map().unwrap().text(id).unwrap();
    Point::new(t.x, t.y)
}

#[test]
fn select_moves_a_text_annotation_as_it_moves_everything_else() {
    let Some(mut r) = rig(Tool::Select) else { return };
    let (id, at) = r.add_text();
    let start = text_pos(&r, &id);
    let scale = r.canvas.camera.scale;
    r.down(at);
    r.move_to(Point::new(at.x + 15.0, at.y + 5.0));
    r.move_to(Point::new(at.x + 30.0, at.y + 20.0));
    r.up(Point::new(at.x + 30.0, at.y + 20.0));
    let now = text_pos(&r, &id);
    assert!((now.x - start.x - 30.0 / scale).abs() < 1e-6 && (now.y - start.y - 20.0 / scale).abs() < 1e-6, "moved to {now:?}");
    assert_eq!(r.labels(), vec!["Add text".to_string(), "Move text".to_string()]);
    assert!(r.canvas.gestures.text_editor.is_none(), "a drag is not a click: no editor");
}

#[test]
fn undo_and_redo_wait_for_a_drag_in_progress_to_finish() {
    // Undo replaces the map the drag is editing, and the drag then committed
    // against the snapshot it started from: undoing it brought back the edit
    // just undone.
    let Some(mut r) = rig(Tool::Select) else { return };
    let (id, at) = r.add_text();
    let start = text_pos(&r, &id);
    r.down(at);
    r.move_to(Point::new(at.x + 40.0, at.y));
    assert!(r.canvas.gestures.map_drag_in_progress());
    let during = text_pos(&r, &id);
    r.key(CanvasKey::Z, true);
    r.key(CanvasKey::Y, true);
    assert_eq!(text_pos(&r, &id), during, "Ctrl+Z and Ctrl+Y did nothing during the drag");
    assert_eq!(r.labels(), vec!["Add text".to_string()]);
    r.up(Point::new(at.x + 40.0, at.y));
    assert_eq!(r.labels(), vec!["Add text".to_string(), "Move text".to_string()]);

    // Once it is over, undo takes back the drag and only the drag.
    r.key(CanvasKey::Z, true);
    assert_eq!(text_pos(&r, &id), start);
    assert_eq!(r.labels(), vec!["Add text".to_string()]);
}

#[test]
fn escape_drops_a_selected_compartment_in_select_as_well_as_in_geometry_edit() {
    let Some(mut r) = rig(Tool::Select) else { return };
    let cell = r.host.map().unwrap().cells()[0].clone();
    let comp = cell.compartments.last().unwrap().id.clone();
    r.host.select_geometry(Some(&cell.id), &comp);
    assert!(!r.host.geometry_edit());
    assert!(r.host.selected_geometry().is_some());
    r.key(CanvasKey::Escape, false);
    assert!(r.host.selected_geometry().is_none(), "its handles stayed after Escape in Select");
    assert!(r.host.selected_entity().is_none());
}

#[test]
fn the_empty_inside_of_a_selected_compartment_grabs_the_group() {
    let Some(mut r) = rig(Tool::Select) else { return };
    let cell = r.host.map().unwrap().cells()[0].clone();
    // The innermost compartment: the one with no compartment inside it.
    let comp = cell.compartments.iter().find(|c| !cell.compartments.iter().any(|k| k.parent_id.as_ref() == Some(&c.id))).unwrap().clone();
    let rect = comp.rect;
    r.host.select_in_rect(rect.x - 4.0, rect.y - 4.0, rect.x + rect.w + 4.0, rect.y + rect.h + 4.0, true);
    assert!(r.host.multi_selection().compartments.contains(&(cell.id.clone(), comp.id.clone())));
    assert!(r.host.multi_selection_size() >= 2);
    r.sync();

    // A point inside it with nothing drawn there.
    let mut found = None;
    'scan: for i in 1..40 {
        for j in 1..40 {
            let p = Point::new(rect.x + rect.w * i as f64 / 40.0, rect.y + rect.h * j as f64 / 40.0);
            let at = r.canvas.map_to_screen(p);
            if at.x < 20.0 || at.y < 20.0 || at.x > 1080.0 || at.y > 780.0 {
                continue;
            }
            let t = r.target(at);
            let space = t.closest(&targets::COMPARTMENT_SPACE).and_then(|e| e.get("data-compartment-space")).map(str::to_string);
            if space.as_deref() == Some(comp.id.as_str()) && !targets::map_content_at(&t) && t.closest(&targets::SELECTION_DRAG).is_none() {
                found = Some(at);
                break 'scan;
            }
        }
    }
    let at = found.expect("an empty point inside the compartment");
    let scale = r.canvas.camera.scale;
    r.down(at);
    assert!(r.canvas.gestures.group_drag.is_some(), "the press grabbed the group");
    r.move_to(Point::new(at.x, at.y + 6.0));
    r.up(Point::new(at.x, at.y + 6.0));
    let now = r.host.map().unwrap().cells()[0].compartments.iter().find(|c| c.id == comp.id).unwrap().rect;
    assert_eq!(now.x, rect.x);
    assert!((now.y - rect.y - 6.0 / scale).abs() < 1e-6, "the compartment went to {now:?} from {rect:?}");
    assert_eq!(r.labels(), vec!["Move selection".to_string()]);
}
