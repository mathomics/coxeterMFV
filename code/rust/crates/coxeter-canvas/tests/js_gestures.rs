//! The gestures against the real JavaScript app: every case of
//! `tests/fixtures/gestures.json` (written by `tools/fixtures-canvas.mjs`,
//! which drives `src/ui/app.js` in headless Firefox with real input) is
//! replayed through the Rust stage at the same client points, with the same
//! camera and stage box, over an [`EditableMapHost`] holding the same map.
//!
//! Compared per case: what the browser found under every press (hit-testing),
//! the map afterwards (exactly, by coxeter-map's rule), the selection, the
//! multi-selection, the undo labels, the camera, the menus asked for, the
//! status line and the text editor.

mod common;

use std::collections::BTreeSet;

use coxeter_canvas::camera::{Camera, StageRect};
use coxeter_canvas::gestures::{CanvasKey, KeyMods, PointerInput, PointerType, Stage};
use coxeter_canvas::host::MapHost;
use coxeter_canvas::test_host::EditableMapHost;
use coxeter_canvas::{CanvasEvent, MapCanvas, MapMenu, MenuRequest, Tool};
use coxeter_core::mapdata::Point;
use coxeter_core::testkit::json_diff;
use serde_json::{Value, json};

use common::*;

/// One case replayed.
struct Replay {
    canvas: MapCanvas,
    host: EditableMapHost,
    time: f64,
    menus: Vec<MenuRequest>,
    status: Option<String>,
    /// The editor's text was replaced by typing (it opens all selected).
    typed: bool,
    /// Where the mouse is: a browser fires `pointermove` only when it moves.
    mouse_at: Option<Point>,
    problems: Vec<String>,
}

fn mouse(p: Point, button: i16) -> PointerInput {
    PointerInput { pointer_id: 1, pointer_type: PointerType::Mouse, button, client: p, ..Default::default() }
}

fn point(v: &Value) -> Point {
    Point::new(v["x"].as_f64().unwrap(), v["y"].as_f64().unwrap())
}

impl Replay {
    fn new(doc: &Value, case: &Value) -> Replay {
        let mut host = host_for(doc, case);
        let mut canvas = MapCanvas::new();
        let st = &case["stage"];
        canvas.set_rect(StageRect::new(
            st["left"].as_f64().unwrap(),
            st["top"].as_f64().unwrap(),
            st["width"].as_f64().unwrap(),
            st["height"].as_f64().unwrap(),
        ));
        let cam = &case["camera"];
        canvas.camera = Camera { x: cam["x"].as_f64().unwrap(), y: cam["y"].as_f64().unwrap(), scale: cam["scale"].as_f64().unwrap(), fitted: true };
        let tool = Tool::parse(case["tool"].as_str().unwrap()).expect("a tool");
        canvas.activate_tool(tool, &mut host);
        canvas.take_events();
        if case["setup"] == "select-first-reaction-and-node" {
            let map = host.map.as_ref().unwrap();
            let rxn = map.reactions().keys().next().unwrap().clone();
            let node = map.nodes().keys().next().unwrap().clone();
            host.select(Some(&rxn));
            host.toggle_multi_selection(coxeter_canvas::host::SelectKind::Node, &node);
        }
        host.history.clear();
        Replay { canvas, host, time: 1000.0, menus: Vec::new(), status: None, typed: false, mouse_at: None, problems: Vec::new() }
    }

    fn sync(&mut self) {
        let revision = self.host.emitted.len() as u64;
        self.canvas.sync(&self.host, revision, self.time);
        let had_editor = self.canvas.gestures.text_editor.is_some();
        for e in self.canvas.take_events() {
            match e {
                CanvasEvent::OpenMenu { menu, .. } => {
                    self.menus.retain(|m| m.menu() != menu.menu());
                    self.menus.push(menu);
                }
                CanvasEvent::CloseMenu(kind) => self.menus.retain(|m| m.menu() != kind),
                CanvasEvent::StatusHint(s) => self.status = Some(s),
                CanvasEvent::TextEditorOpened { .. } => self.typed = false,
                _ => {}
            }
        }
        let _ = had_editor;
    }

    /// The mouse goes to `at`: a `pointermove` if that is somewhere new.
    fn move_mouse(&mut self, at: Point) {
        if self.mouse_at == Some(at) {
            return;
        }
        self.mouse_at = Some(at);
        self.canvas.pointer_move(&mut self.host, &mouse(at, -1), self.time);
    }

    fn tick(&mut self) {
        self.time += 400.0;
    }


    fn check_target(&mut self, case: &str, k: usize, step: &Value, at: Point) {
        let filter = self.canvas.gestures.hit_filter();
        if self.canvas.tool() == Tool::Pan {
            // `.stage[data-tool="pan"] svg { pointer-events: none }` is
            // inherited; the browser still hands the pointer to elements that
            // set their own pointer-events (the halos). The native stage gives
            // Pan mode nothing, as the stylesheet means; a press only pans.
            let stage = Stage { camera: &mut self.canvas.camera.clone(), rect: self.canvas.rect, scene: self.canvas.scene.as_ref() };
            assert!(stage.target_at(at, filter).is_stage(), "{case}: Pan mode takes nothing");
            return;
        }
        let rust = {
            let stage = Stage { camera: &mut self.canvas.camera.clone(), rect: self.canvas.rect, scene: self.canvas.scene.as_ref() };
            describe(&stage.target_at(at, filter))
        };
        if !same_target(&rust, &step["target"]) {
            self.problems.push(format!("{case} step {k}: under ({}, {}) the browser hit {} and Rust {}", at.x, at.y, step["target"], rust));
        }
    }

    fn step(&mut self, case: &str, k: usize, step: &Value) {
        self.sync();
        let kind = step["type"].as_str().unwrap();
        let button = step["button"].as_i64().unwrap_or(0) as i16;
        let shift = step["shift"].as_bool().unwrap_or(false);
        match kind {
            "down" => {
                let at = point(step);
                self.move_mouse(at);
                self.sync();
                self.check_target(case, k, step, at);
                let ev = PointerInput { shift, ..mouse(at, button) };
                self.canvas.pointer_down(&mut self.host, &ev);
            }
            "move" => {
                let at = point(step);
                self.move_mouse(at);
            }
            "up" => {
                let at = point(step);
                self.move_mouse(at);
                self.sync();
                self.canvas.pointer_up(&mut self.host, &mouse(at, button), self.time);
            }
            "click" => {
                let at = point(step);
                self.move_mouse(at);
                self.sync();
                self.check_target(case, k, step, at);
                let ev = PointerInput { shift, ..mouse(at, button) };
                self.canvas.pointer_down(&mut self.host, &ev);
                self.sync();
                self.tick();
                self.canvas.pointer_up(&mut self.host, &ev, self.time);
            }
            "wheel" => {
                let at = point(step);
                let input = coxeter_canvas::wheel::WheelInput {
                    delta_x: step["wx"].as_f64().unwrap_or(0.0),
                    delta_y: step["wy"].as_f64().unwrap_or(0.0),
                    ..Default::default()
                };
                self.canvas.wheel(&self.host, &input, at, self.time);
            }
            "key" | "ctrl" => {
                let key = step["key"].as_str().unwrap();
                let editor = self.canvas.gestures.text_editor.is_some();
                match (key, kind) {
                    // The editor's own keys: Enter writes, Escape drops.
                    ("\u{E007}", _) if editor => self.canvas.gestures.commit_text_editor(&mut self.host),
                    ("\u{E00C}", _) if editor => self.canvas.gestures.cancel_text_editor(),
                    (k, "ctrl") => {
                        let key = if k == "z" { CanvasKey::Z } else { CanvasKey::Y };
                        self.canvas.key_down(&mut self.host, key, KeyMods { command: true, shift: false });
                    }
                    ("\u{E00C}", _) => {
                        self.canvas.key_down(&mut self.host, CanvasKey::Escape, KeyMods::default());
                    }
                    ("\u{E017}", _) => {
                        self.canvas.key_down(&mut self.host, CanvasKey::Delete, KeyMods::default());
                    }
                    (other, _) => panic!("{case}: key {other:?}"),
                }
            }
            "type" => {
                let text = step["text"].as_str().unwrap();
                if let Some(editor) = &mut self.canvas.gestures.text_editor {
                    if self.typed {
                        editor.value.push_str(text);
                    } else {
                        editor.value = text.to_string();
                        self.typed = true;
                    }
                }
            }
            "touch" => {
                let points: Vec<Point> = step["points"].as_array().unwrap().iter().map(point).collect();
                let touch = |p: Point, button: i16| PointerInput { pointer_id: 7, pointer_type: PointerType::Touch, button, client: p, ..Default::default() };
                self.canvas.pointer_down(&mut self.host, &touch(points[0], 0));
                for p in &points[1..] {
                    self.sync();
                    self.canvas.pointer_move(&mut self.host, &touch(*p, -1), self.time);
                }
                self.sync();
                self.canvas.pointer_up(&mut self.host, &touch(*points.last().unwrap(), 0), self.time);
            }
            "pinch" => {
                let finger = |id: u64, p: Point, button: i16| PointerInput { pointer_id: id, pointer_type: PointerType::Touch, button, client: p, ..Default::default() };
                let (a, b, a2, b2) = (point(&step["a"]), point(&step["b"]), point(&step["a2"]), point(&step["b2"]));
                self.canvas.pointer_down(&mut self.host, &finger(2, a, 0));
                self.canvas.pointer_down(&mut self.host, &finger(3, b, 0));
                self.canvas.pointer_move(&mut self.host, &finger(2, a2, -1), self.time);
                self.canvas.pointer_move(&mut self.host, &finger(3, b2, -1), self.time);
                self.canvas.pointer_up(&mut self.host, &finger(2, a2, 0), self.time);
                self.canvas.pointer_up(&mut self.host, &finger(3, b2, 0), self.time);
            }
            other => panic!("{case}: step {other}"),
        }
        self.tick();
        self.sync();
    }

    /// The stage's state in the fixture's terms.
    fn state(&self) -> Value {
        let h = &self.host;
        let entity = h.selected_entity.as_ref().map(|e| {
            let mut v = json!({ "type": e.kind, "id": e.id });
            if e.kind == "compartment" {
                v["cellId"] = e.cell_id.clone().map(Value::String).unwrap_or(Value::Null);
            }
            v
        });
        let geometry = h.selected_geometry.as_ref().map(|g| json!({ "cellId": g.cell_id, "compartmentId": g.compartment_id }));
        let set = |s: &indexmap::IndexSet<String>| s.iter().cloned().collect::<BTreeSet<_>>();
        let mut menus: Vec<&str> = self
            .menus
            .iter()
            .map(|m| match m.menu() {
                MapMenu::Boundary => "boundary",
                MapMenu::Metabolite | MapMenu::Compartment => "reaction-panel",
                MapMenu::MetaboliteAdd => "metabolite-add",
            })
            .collect();
        menus.sort();
        json!({
            "selectedReaction": h.selected_reaction,
            "selectedEntity": entity,
            "selectedGeometry": geometry,
            "multiSelection": {
                "nodes": set(&h.multi_selection.nodes),
                "reactions": set(&h.multi_selection.reactions),
                "texts": set(&h.multi_selection.texts),
            },
            "undo": h.history.undo_labels(),
            "menus": menus,
            "status": self.status,
            "editor": self.canvas.gestures.text_editor.as_ref().map(|e| e.value.clone()),
            "view": { "geometryEdit": h.view.geometry_edit, "textTool": h.view.text_tool, "multiSelect": h.view.multi_select },
        })
    }
}

/// The browser's state after a case, in the same terms.
fn js_state(after: &Value) -> Value {
    let set = |v: &Value| v.as_array().into_iter().flatten().filter_map(|s| s.as_str().map(str::to_string)).collect::<BTreeSet<_>>();
    let mut menus: Vec<&str> = after["menus"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    menus.sort();
    let status = after["status"].as_str().filter(|s| !s.is_empty()).map(str::to_string);
    let mut entity = after["selectedEntity"].clone();
    if let Some(m) = entity.as_object_mut() {
        if m.get("type").and_then(Value::as_str) != Some("compartment") {
            m.remove("cellId");
        } else if !m.contains_key("cellId") {
            m.insert("cellId".into(), Value::Null);
        }
    }
    let geometry = match &after["selectedGeometry"] {
        Value::Object(g) => json!({ "cellId": g.get("cellId").cloned().unwrap_or(Value::Null), "compartmentId": g["compartmentId"] }),
        _ => Value::Null,
    };
    json!({
        "selectedReaction": after["selectedReaction"],
        "selectedEntity": entity,
        "selectedGeometry": geometry,
        "multiSelection": {
            "nodes": set(&after["multiSelection"]["nodes"]),
            "reactions": set(&after["multiSelection"]["reactions"]),
            "texts": set(&after["multiSelection"]["texts"]),
        },
        "undo": after["undo"],
        "menus": menus,
        "status": status,
        "editor": after["editor"],
        "view": after["view"],
    })
}

/// Cases where the native stage deliberately differs from the browser, and how.
fn deviation(case: &str, field: &str) -> bool {
    match (case, field) {
        // Escape cancels the label drag in both; JavaScript then reads the
        // release as a click on what is under it (here a compartment), the
        // native stage swallows the release of a cancelled press.
        ("escape-cancels-label-drag", "selectedEntity" | "selectedGeometry") => true,
        _ => false,
    }
}

/// Cases whose references predate the e9438d7 follow-ups in `src/ui/app.js`,
/// and what the JavaScript gives in place of the reference.
fn updated(case: &str, field: &str) -> Option<Value> {
    match (case, field) {
        // Select moves a text annotation as Edit does, so a click on one
        // selects it and opens its editor there too, as in
        // "edit-text-click-opens-editor".
        ("select-text-click-opens-nothing", "selectedEntity") => Some(json!({ "type": "text", "id": "text-1" })),
        ("select-text-click-opens-nothing", "editor") => Some(json!("Free text")),
        _ => None,
    }
}

/// Cases whose map the references hold as `editGeometryRect` resized it
/// before e9438d7 and its follow-ups, which keep a side not pulled in place
/// and let a transporter on a resized side ride it across and not along.
/// Their maps are not compared until `tools/fixtures-canvas.mjs` is run again,
/// the rest of each case still is. `coxeter-map/tests/js_geometry_edit.rs`
/// covers the current resize.
const STALE_MAPS: &[&str] = &["membrane-drag-resizes", "geometry-handle-resize", "geometry-handle-resize-wide", "edit-membrane-drag-shared-medium"];

#[test]
fn gestures_match_the_javascript_app() {
    let Some(doc) = gestures() else {
        eprintln!("skipped: tests/fixtures/gestures.json not written (tools/fixtures-canvas.mjs)");
        return;
    };
    let initial_rust = EditableMapHostExt::roundtrip(&doc["initial"]);
    assert_eq!(json_diff(&canonical(&initial_rust), &canonical(&doc["initial"]), 0.0), None, "the start map reads back as written");

    let mut problems: Vec<String> = Vec::new();
    let mut known: Vec<String> = Vec::new();
    let mut passed = 0;
    let cases = doc["cases"].as_array().unwrap();
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let mut run = Replay::new(&doc, case);
        for (k, step) in case["steps"].as_array().unwrap().iter().enumerate() {
            run.step(name, k, step);
        }
        let before = problems.len();
        problems.append(&mut run.problems);

        // The map.
        let start = if case["start"] == "own" { &case["map"] } else { &doc["initial"] };
        let expected = canonical(&apply_edits(start, &case["mapEdits"]));
        let actual = canonical(&run.host.map.as_ref().unwrap().to_json());
        let mut diffs = Vec::new();
        if !STALE_MAPS.contains(&name) {
            differences(&actual, &expected, &mut String::from("$"), &mut diffs);
        }
        for (path, a, b) in diffs {
            // coxeter-map stores a translated reaction's direction from its
            // final axis points; JavaScript from points its aliases move again
            // afterwards, which can differ in the last bit or two.
            let last_bits = path.contains(".axis.dir.") && (a - b).abs() <= 4.0 * f64::EPSILON;
            if last_bits {
                known.push(format!("{name}: {path}: {a} vs {b}"));
            } else {
                problems.push(format!("{name}: map differs at {path}: Rust {a}, browser {b}"));
            }
        }
        // The rest of the stage.
        let rust = run.state();
        let js = js_state(&case["after"]);
        for field in ["selectedReaction", "selectedEntity", "selectedGeometry", "multiSelection", "undo", "menus", "status", "editor", "view"] {
            let now = updated(name, field);
            let (r, j) = (&rust[field], now.as_ref().unwrap_or(&js[field]));
            let same = match field {
                // The boundary menu declines a membrane with nothing left to add.
                "menus" => r == j || (r == &json!(["boundary"]) && j == &json!([])),
                _ => r == j,
            };
            if !same && !deviation(name, field) {
                problems.push(format!("{name}: {field}: Rust {r}, browser {j}"));
            }
        }
        let cam = &case["after"]["camera"];
        let c = run.canvas.camera;
        let close = |a: f64, b: &Value| (a - b.as_f64().unwrap()).abs() <= 1e-9 * (1.0 + a.abs());
        if !(close(c.x, &cam["x"]) && close(c.y, &cam["y"]) && close(c.scale, &cam["scale"])) {
            problems.push(format!("{name}: camera Rust ({}, {}, {}), browser {cam}", c.x, c.y, c.scale));
        }
        if problems.len() == before {
            passed += 1;
        }
    }
    eprintln!("{passed} of {} cases match the browser", cases.len());
    if !known.is_empty() {
        eprintln!("last-bit differences of coxeter-map's translation (tolerated):
  {}", known.join("
  "));
    }
    assert!(problems.is_empty(), "{} problems:\n{}", problems.len(), problems.join("\n"));
}

/// Every difference between two JSON values: numbers as `(path, a, b)`;
/// anything else as NaNs with the path saying what.
fn differences(a: &Value, b: &Value, path: &mut String, out: &mut Vec<(String, f64, f64)>) {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (x.as_f64().unwrap(), y.as_f64().unwrap());
            if x != y {
                out.push((path.clone(), x, y));
            }
        }
        (Value::Object(x), Value::Object(y)) => {
            for k in x.keys().chain(y.keys().filter(|k| !x.contains_key(*k))) {
                let len = path.len();
                path.push('.');
                path.push_str(k);
                match (x.get(k), y.get(k)) {
                    (Some(p), Some(q)) => differences(p, q, path, out),
                    (p, _) => out.push((format!("{path} (only {})", if p.is_some() { "Rust" } else { "browser" }), f64::NAN, f64::NAN)),
                }
                path.truncate(len);
            }
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            for (i, (p, q)) in x.iter().zip(y).enumerate() {
                let len = path.len();
                path.push_str(&format!("[{i}]"));
                differences(p, q, path, out);
                path.truncate(len);
            }
        }
        _ if a == b => {}
        _ => out.push((format!("{path} ({} vs {})", short(a), short(b)), f64::NAN, f64::NAN)),
    }
}

fn short(v: &Value) -> String {
    let s = v.to_string();
    if s.len() > 80 { format!("{}…", &s[..80]) } else { s }
}

/// The start map through the Rust reader and writer.
struct EditableMapHostExt;

impl EditableMapHostExt {
    fn roundtrip(v: &Value) -> Value {
        coxeter_map::EditableMap::from_json(v).unwrap().to_json()
    }
}
