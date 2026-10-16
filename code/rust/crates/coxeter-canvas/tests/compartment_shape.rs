//! The stage's side of d7b2c0e (`src/ui/app.js`): the side handles resize one
//! side, a shape diamond dragged along its edge reshapes the compartment as
//! one undo step, and Delete takes a selected compartment or cell off the map
//! (`cellRootOf`), read from what is selected. On the E. coli map of the
//! browser references, through [`MapCanvas`] over an [`EditableMapHost`].

mod common;

use coxeter_canvas::camera::{MIN_HIT_PX, StageRect};
use coxeter_canvas::gestures::{CanvasKey, KeyMods, PointerInput, PointerType};
use coxeter_canvas::host::MapHost;
use coxeter_canvas::test_host::EditableMapHost;
use coxeter_canvas::{MapCanvas, Tool};
use coxeter_core::geometry::{compartment_shape, shape_handle_point};
use coxeter_core::mapdata::{MapCompartment, Point};
use serde_json::json;

use common::*;

struct Rig {
    canvas: MapCanvas,
    host: EditableMapHost,
    time: f64,
}

fn rig() -> Option<Rig> {
    let doc = gestures()?;
    let mut host = host_for(&doc, &json!({ "start": "initial" }));
    let mut canvas = MapCanvas::new();
    canvas.set_rect(StageRect::new(0.0, 0.0, 1100.0, 800.0));
    canvas.activate_tool(Tool::Select, &mut host);
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

    fn drag(&mut self, from: Point, to: Point) {
        self.sync();
        self.canvas.pointer_move(&mut self.host, &mouse(from, -1), self.time);
        self.canvas.pointer_down(&mut self.host, &mouse(from, 0));
        for i in 1..=5 {
            self.time += 16.0;
            let t = i as f64 / 5.0;
            let at = Point::new(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t);
            self.canvas.pointer_move(&mut self.host, &mouse(at, -1), self.time);
        }
        self.time += 16.0;
        self.canvas.pointer_up(&mut self.host, &mouse(to, 0), self.time);
        self.sync();
    }

    /// A nested compartment of the first cell, selected so its handles show.
    fn select_inner(&mut self) -> (String, MapCompartment) {
        let cell = self.host.map().unwrap().cells()[0].clone();
        let comp = cell.compartments.iter().filter(|c| c.parent_id.is_some()).max_by(|a, b| (a.rect.w * a.rect.h).total_cmp(&(b.rect.w * b.rect.h))).unwrap().clone();
        self.host.select_geometry(Some(&cell.id), &comp.id);
        self.sync();
        (cell.id, comp)
    }

    fn compartment(&self, cell: &str, id: &str) -> Option<MapCompartment> {
        self.host.map()?.cell(cell)?.compartments.iter().find(|c| c.id == id).cloned()
    }

    fn labels(&self) -> Vec<String> {
        self.host.history.undo_labels().iter().map(|l| l.to_string()).collect()
    }
}

#[test]
fn a_shape_diamond_dragged_along_its_edge_reshapes_the_compartment_in_one_step() {
    let Some(mut r) = rig() else { return };
    let (cell, comp) = r.select_inner();
    let side = coxeter_render::geometry_handle_side(MIN_HIT_PX / r.canvas.camera.scale);
    let before = compartment_shape(&comp);
    let at = r.canvas.map_to_screen(shape_handle_point(&comp.rect, "oval", before.oval, side));
    // Down the left side, most of the way to the middle.
    let to = r.canvas.map_to_screen(shape_handle_point(&comp.rect, "oval", 0.8, side));
    r.drag(at, Point::new(at.x + 3.0, to.y));
    let now = r.compartment(&cell, &comp.id).unwrap();
    let shape = compartment_shape(&now);
    assert!((shape.oval - 0.8).abs() < 0.02, "the oval amount follows the diamond: {shape:?}");
    assert_eq!(shape.corner, before.corner, "the other amount stays");
    assert_eq!(now.rect, comp.rect, "the rectangle stays");
    assert_eq!(r.labels().last().map(String::as_str), Some("Make compartment oval"));
    let steps = r.labels().len();
    r.canvas.key_down(&mut r.host, CanvasKey::Z, KeyMods { command: true, shift: false });
    assert_eq!(r.host.history.undo_labels().len(), steps - 1, "one undo step");
    assert_eq!(compartment_shape(&r.compartment(&cell, &comp.id).unwrap()), before);
}

#[test]
fn a_side_handle_moves_that_side_alone() {
    let Some(mut r) = rig() else { return };
    let (cell, comp) = r.select_inner();
    let rect = comp.rect;
    let at = r.canvas.map_to_screen(Point::new(rect.x + rect.w, rect.y + rect.h / 2.0));
    r.drag(at, Point::new(at.x - 30.0, at.y + 40.0));
    let now = r.compartment(&cell, &comp.id).unwrap().rect;
    assert_eq!((now.x, now.y, now.h), (rect.x, rect.y, rect.h), "only the right side moves");
    assert!(now.w < rect.w, "the right side came in");
    assert_eq!(r.labels().last().map(String::as_str), Some("Resize compartment"));
}

#[test]
fn delete_removes_the_selected_compartment_or_cell() {
    let Some(mut r) = rig() else { return };
    let (cell, comp) = r.select_inner();
    r.canvas.key_down(&mut r.host, CanvasKey::Delete, KeyMods::default());
    assert!(r.compartment(&cell, &comp.id).is_none(), "the compartment is gone");
    assert!(r.host.map().unwrap().cell(&cell).is_some(), "its cell stays");
    assert_eq!(r.host.selected_geometry, None);

    // A selected cell goes whole, with its outermost compartment.
    r.host.select_cell_geometry(&cell);
    r.canvas.key_down(&mut r.host, CanvasKey::Delete, KeyMods::default());
    assert!(r.host.map().unwrap().cell(&cell).is_none(), "the cell is gone");
    assert_eq!(r.labels().last().map(String::as_str), Some("Remove cell"));
}
